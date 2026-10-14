use super::support::*;
use super::*;

#[test]
fn release_atom_parser_reads_first_release_tag() {
    let xml = r#"
          <feed>
            <entry>
              <link href="https://github.com/router-for-me/CLIProxyAPI/releases/tag/v7.2.80"/>
              <title>v7.2.80</title>
            </entry>
            <entry><title>v7.2.79</title></entry>
          </feed>
        "#;

    assert_eq!(release_tag_from_atom(xml).as_deref(), Some("v7.2.80"));
}

#[test]
fn app_update_comparison_uses_semantic_versions() {
    assert!(is_app_update_available("v0.1.9", "v0.2.0").unwrap());
    assert!(is_app_update_available("v0.2.0-beta.1", "v0.2.0").unwrap());
    assert!(!is_app_update_available("v0.2.0", "v0.2.0").unwrap());
    assert!(!is_app_update_available("v0.2.0", "v0.1.9").unwrap());
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[test]
fn portable_update_helper_writes_a_distinct_startup_ack() {
    let work_dir = agent_test_home("portable-update-helper-ack");
    acknowledge_portable_update_helper_start(&work_dir).unwrap();

    let ack_path = portable_update_helper_ack_path(&work_dir);
    assert_eq!(
        ack_path.file_name().and_then(|value| value.to_str()),
        Some(PORTABLE_UPDATE_HELPER_ACK_FILE)
    );
    assert_eq!(
        fs::read_to_string(&ack_path).unwrap(),
        std::process::id().to_string()
    );
    assert_ne!(ack_path.file_name().unwrap(), "update-started.ack");
    fs::remove_dir_all(work_dir).unwrap();
}

#[test]
fn macos_update_helper_stays_inside_the_signed_app_bundle() {
    let current_exe =
        PathBuf::from("/Applications/EasyCLIProxyAPI.app/Contents/MacOS/EasyCLIProxyAPI");

    assert_eq!(macos_update_helper_path(&current_exe), current_exe);
}

#[test]
fn macos_dmg_eject_retries_busy_mounts_and_stops_after_success() {
    let mount_dir = Path::new("/private/tmp/update with spaces/mount");
    for busy_attempts in 0..3 {
        let mut attempts = 0;
        let mut waits = Vec::new();
        eject_macos_update_dmg_with_commands(
            mount_dir,
            |command, _| {
                assert_eq!(command.get_program(), "diskutil");
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    [std::ffi::OsStr::new("eject"), mount_dir.as_os_str()]
                );
                attempts += 1;
                if attempts <= busy_attempts {
                    Err("Resource busy".to_string())
                } else {
                    Ok(())
                }
            },
            |duration| waits.push(duration),
        )
        .unwrap();
        assert_eq!(attempts, busy_attempts + 1);
        assert_eq!(waits, vec![Duration::from_millis(500); busy_attempts]);
    }
}

#[test]
fn macos_dmg_eject_falls_back_only_for_the_update_mount() {
    let mount_dir = Path::new("/private/tmp/update with spaces/mount");
    for force_succeeds in [true, false] {
        let mut eject_attempts = 0;
        let mut force_attempts = 0;
        let result = eject_macos_update_dmg_with_commands(
            mount_dir,
            |command, _| {
                if command.get_program() == "diskutil" {
                    eject_attempts += 1;
                    return Err("diskutil: Resource busy".to_string());
                }
                assert_eq!(command.get_program(), "hdiutil");
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    [
                        std::ffi::OsStr::new("detach"),
                        std::ffi::OsStr::new("-force"),
                        mount_dir.as_os_str(),
                    ]
                );
                force_attempts += 1;
                if force_succeeds {
                    Ok(())
                } else {
                    Err("hdiutil: Resource busy".to_string())
                }
            },
            |_| {},
        );
        assert_eq!(eject_attempts, 3);
        assert_eq!(force_attempts, 1);
        if force_succeeds {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("diskutil: Resource busy"));
            assert!(error.contains("hdiutil: Resource busy"));
        }
    }
}

#[test]
fn macos_dmg_staging_survives_cleanup_failure_without_bypassing_verification() {
    // Exercise successful staging and each fatal preparation step, even when
    // every eject attempt fails with the error reported in issue #301.
    for failure in [None, Some("ditto"), Some("codesign"), Some("spctl")] {
        let root = agent_test_home("macos-dmg-stage-busy");
        let source_app = root.join("mount").join("EasyCLIProxyAPI.app");
        let staged_app = root.join("staging/EasyCLIProxyAPI.app");
        fs::create_dir_all(&source_app).unwrap();
        fs::write(source_app.join("payload"), b"application").unwrap();
        let mut preparation = Vec::new();
        let mut cleanup_attempts = 0;
        let result = stage_macos_application_from_dmg_with_commands(
            &root.join("update.dmg"),
            &root,
            &staged_app,
            |command, _| {
                let program = command.get_program().to_str().unwrap();
                let args = command.get_args().collect::<Vec<_>>();
                if program == "diskutil" || args.first().is_some_and(|arg| *arg == "detach") {
                    cleanup_attempts += 1;
                    return Err("Resource busy".to_string());
                }
                preparation.push(program.to_string());
                if failure == Some(program) {
                    return Err(format!("{program} failed"));
                }
                if program == "ditto" {
                    assert_eq!(args, [source_app.as_os_str(), staged_app.as_os_str()]);
                    fs::create_dir_all(&staged_app).unwrap();
                    fs::copy(source_app.join("payload"), staged_app.join("payload")).unwrap();
                } else if program == "codesign" || program == "spctl" {
                    assert_eq!(args.last().copied(), Some(staged_app.as_os_str()));
                }
                Ok(())
            },
            |_| {},
        );
        assert_eq!(cleanup_attempts, 4);
        if let Some(failure) = failure {
            assert_eq!(result.unwrap_err(), format!("{failure} failed"));
            assert_eq!(preparation.last().map(String::as_str), Some(failure));
        } else {
            result.unwrap();
            assert_eq!(preparation, ["hdiutil", "ditto", "codesign", "spctl"]);
            assert_eq!(fs::read(staged_app.join("payload")).unwrap(), b"application");
        }
        assert!(source_app.join("payload").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn macos_dmg_missing_app_is_rejected_and_still_ejected() {
    let root = agent_test_home("macos-dmg-missing-app");
    let mut commands = Vec::new();
    let error = stage_macos_application_from_dmg_with_commands(
        &root.join("update.dmg"),
        &root,
        &root.join("staging/EasyCLIProxyAPI.app"),
        |command, _| {
            commands.push(command.get_program().to_string_lossy().into_owned());
            Ok(())
        },
        |_| panic!("Successful ejection should not wait"),
    )
    .unwrap_err();
    assert!(error.contains("missing EasyCLIProxyAPI.app"));
    assert_eq!(commands, ["hdiutil", "diskutil"]);
    assert!(!root.join("mount").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn macos_dmg_cleanup_preserves_busy_images_and_retries_later() {
    let root = agent_test_home("macos-dmg-cleanup");
    let mount_dir = root.join("mount");
    let archive = root.join("update.dmg");
    let mounted_file = mount_dir.join("mounted-file");
    fs::create_dir_all(&mount_dir).unwrap();
    fs::write(&archive, b"disk image").unwrap();
    fs::write(&mounted_file, b"mounted contents").unwrap();

    cleanup_macos_update_dmg_with_commands(
        &root,
        |_, _| Err("Resource busy".to_string()),
        |_| {},
    );
    assert_eq!(fs::read(&archive).unwrap(), b"disk image");
    assert_eq!(fs::read(&mounted_file).unwrap(), b"mounted contents");

    cleanup_macos_update_dmg_with_commands(
        &root,
        |command, _| {
            assert_eq!(command.get_program(), "diskutil");
            // Simulate the mounted contents disappearing after a successful eject.
            fs::remove_file(&mounted_file).unwrap();
            Ok(())
        },
        |_| panic!("Successful ejection should not wait"),
    );
    assert!(!archive.exists());
    assert!(!mount_dir.exists());

    cleanup_macos_update_dmg_with_commands(
        &root,
        |_, _| panic!("An already cleaned image should not be ejected again"),
        |_| panic!("An already cleaned image should not wait"),
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn macos_dmg_eject_skips_remaining_diskutil_retries_on_timeout() {
    let mount_dir = Path::new("/private/tmp/update with spaces/mount");
    let mut eject_attempts = 0;
    let mut force_attempts = 0;
    let mut waits = Vec::new();
    eject_macos_update_dmg_with_commands(
        mount_dir,
        |command, _| {
            if command.get_program() == "diskutil" {
                eject_attempts += 1;
                return Err("Failed to Eject application update DMG: timed out after 10s".to_string());
            }
            assert_eq!(command.get_program(), "hdiutil");
            force_attempts += 1;
            Ok(())
        },
        |duration| waits.push(duration),
    )
    .unwrap();
    assert_eq!(eject_attempts, 1);
    assert_eq!(force_attempts, 1);
    assert!(waits.is_empty());
}

#[test]
fn macos_dmg_cleanup_removes_archive_when_mount_dir_was_already_unmounted() {
    let root = agent_test_home("macos-dmg-cleanup-already-unmounted");
    let mount_dir = root.join("mount");
    let archive = root.join("update.dmg");
    fs::create_dir_all(&mount_dir).unwrap();
    fs::write(&archive, b"disk image").unwrap();

    cleanup_macos_update_dmg_with_commands(
        &root,
        |_, _| Err("Unable to find disk for mount".to_string()),
        |_| panic!("Already unmounted directory should not wait between diskutil retries"),
    );
    assert!(!mount_dir.exists());
    assert!(!archive.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn eject_macos_update_dmg_does_not_retry_when_volume_already_unmounted() {
    let mount_dir = Path::new("/private/tmp/update with spaces/mount");
    for message in ["Unable to find disk", "Failed to find disk"] {
        let mut eject_attempts = 0;
        let mut force_attempts = 0;
        let mut waits = Vec::new();
        eject_macos_update_dmg_with_commands(
            mount_dir,
            |command, _| {
                if command.get_program() == "diskutil" {
                    eject_attempts += 1;
                    return Err(format!(
                        "Failed to Eject application update DMG: {message} for {}",
                        mount_dir.display()
                    ));
                }
                force_attempts += 1;
                panic!("hdiutil detach -force must not be called when volume is already unmounted");
            },
            |duration| waits.push(duration),
        )
        .unwrap();
        assert_eq!(eject_attempts, 1);
        assert_eq!(force_attempts, 0);
        assert!(waits.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn macos_update_command_runner_terminates_hung_processes_on_timeout() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30; true"]);
    let started = Instant::now();
    let error = run_macos_update_command_with_timeout(
        &mut command,
        "Eject application update DMG",
        Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(error.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn macos_update_command_deadlines_distinguish_detach_from_other_commands() {
    assert_eq!(
        macos_update_command_timeout(&Command::new("diskutil")),
        Duration::from_secs(10)
    );
    let mut detach = Command::new("hdiutil");
    detach.args(["detach", "-force", "/tmp/mount"]);
    assert_eq!(
        macos_update_command_timeout(&detach),
        Duration::from_secs(15)
    );
    let mut attach = Command::new("hdiutil");
    attach.arg("attach");
    assert_eq!(
        macos_update_command_timeout(&attach),
        Duration::from_secs(120)
    );
    for program in ["ditto", "codesign", "spctl"] {
        assert_eq!(
            macos_update_command_timeout(&Command::new(program)),
            Duration::from_secs(120)
        );
    }
}

#[test]
fn macos_update_output_drain_bounds_memory_and_work_per_poll() {
    let mut pipe = std::io::Cursor::new(vec![b'x'; 256 * 1024]);
    let mut output = Vec::new();
    assert!(!drain_macos_update_output(&mut pipe, &mut output).unwrap());
    assert_eq!(pipe.position(), 64 * 1024);
    assert_eq!(output.len(), 64 * 1024);
    for _ in 0..3 {
        assert!(!drain_macos_update_output(&mut pipe, &mut output).unwrap());
        assert_eq!(output.len(), 64 * 1024);
    }
    assert!(drain_macos_update_output(&mut pipe, &mut output).unwrap());
}

#[cfg(unix)]
#[test]
fn macos_update_command_timeout_also_covers_inherited_pipes_after_parent_exit() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & exit 0"]);
    let started = Instant::now();
    let error = run_macos_update_command_with_timeout(
        &mut command,
        "inherited pipe test",
        Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn macos_update_command_captures_failure_details_from_both_pipes() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf stdout-detail; printf stderr-detail >&2; exit 7",
    ]);
    let error = run_macos_update_command_with_timeout(
        &mut command,
        "diagnostic test",
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(error.contains("stdout-detail"), "{error}");
    assert!(error.contains("stderr-detail"), "{error}");
    assert!(error.contains("7"), "{error}");
    assert!(!error.contains("timed out"), "{error}");
}

#[cfg(unix)]
#[test]
fn macos_update_command_drains_large_output_without_deadlocking() {
    let mut command = Command::new("sh");
    command.args(["-c", "dd if=/dev/zero bs=4096 count=64 2>/dev/null; (dd if=/dev/zero bs=4096 count=64 2>/dev/null) >&2"]);
    run_macos_update_command_with_timeout(
        &mut command,
        "large output test",
        Duration::from_secs(5),
    )
    .unwrap();
}

fn portable_update_test_asset(version: &str, arch: &str) -> PortableUpdateAsset {
    let (_, display, suffix) = portable_update_asset_platform().unwrap();
    let name = format!("EasyCLIProxyAPI-v{version}-{display}-{arch}.{suffix}");
    PortableUpdateAsset {
        url: format!("{APP_RELEASE_DOWNLOAD_PREFIX}v{version}/{name}"),
        fallback_urls: Vec::new(),
        sha256: "ab".repeat(32),
        size_bytes: 1024,
    }
}

#[cfg(windows)]
fn portable_update_test_legacy_asset(version: &str, arch: &str) -> PortableUpdateAsset {
    let name = format!("EasyCLIProxyAPI-update-v{version}-Windows-{arch}.zip");
    PortableUpdateAsset {
        url: format!("{APP_RELEASE_DOWNLOAD_PREFIX}v{version}/{name}"),
        fallback_urls: Vec::new(),
        sha256: "ab".repeat(32),
        size_bytes: 1024,
    }
}

fn portable_update_test_manifest(version: &str) -> PortableUpdateManifest {
    let (platform, _, _) = portable_update_asset_platform().unwrap();
    PortableUpdateManifest {
        schema_version: 1,
        version: version.to_string(),
        published_at: "2026-07-24T00:00:00.000Z".to_string(),
        release_notes: HashMap::new(),
        release_url: format!(
            "https://github.com/router-for-me/EasyCLIProxyAPI/releases/tag/v{version}"
        ),
        assets: [
            (
                format!("{platform}-amd64"),
                portable_update_test_asset(version, "amd64"),
            ),
            (
                format!("{platform}-aarch64"),
                portable_update_test_asset(version, "aarch64"),
            ),
        ]
        .into_iter()
        .collect(),
        full_assets: None,
    }
}

#[test]
fn portable_update_manifest_uses_only_explicit_locale_release_notes() {
    let mut value = serde_json::json!({
        "schemaVersion": 1,
        "version": "1.2.3",
        "publishedAt": "2026-09-19T00:00:00Z",
        "releaseUrl": "https://github.com/router-for-me/EasyCLIProxyAPI/releases/tag/v1.2.3",
        "assets": {}
    });
    let legacy: PortableUpdateManifest = serde_json::from_value(value.clone()).unwrap();
    assert!(legacy.release_notes.is_empty());
    value["releaseNotes"] = serde_json::json!({
        "zh-CN": "## 新增\n\n- 更新说明。\n",
        "en": "## Added\n\n- Release notes.\n",
        "ja": null,
        "zh-TW": "  ",
        "fr": "Unsupported locale"
    });
    let with_notes: PortableUpdateManifest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        with_notes.release_notes.get("zh-CN").map(String::as_str),
        Some("## 新增\n\n- 更新说明。\n")
    );
    assert_eq!(with_notes.release_notes.len(), 2);
    assert!(with_notes.release_notes.contains_key("en"));
    assert!(!with_notes.release_notes.contains_key("ja"));
    assert!(!with_notes.release_notes.contains_key("zh-TW"));
    for unlocalized in [serde_json::Value::Null, serde_json::json!("Legacy notes")] {
        value["releaseNotes"] = unlocalized;
        let unavailable: PortableUpdateManifest = serde_json::from_value(value.clone()).unwrap();
        assert!(unavailable.release_notes.is_empty());
    }
}

#[test]
fn portable_update_manifest_requires_both_matching_github_assets() {
    let manifest = portable_update_test_manifest("1.2.3");
    assert!(validate_portable_update_manifest(&manifest).is_ok());

    #[cfg(windows)]
    {
        let mut legacy_manifest = portable_update_test_manifest("1.2.3");
        legacy_manifest.assets.insert(
            "windows-amd64".to_string(),
            portable_update_test_legacy_asset("1.2.3", "amd64"),
        );
        legacy_manifest.assets.insert(
            "windows-aarch64".to_string(),
            portable_update_test_legacy_asset("1.2.3", "aarch64"),
        );
        assert!(validate_portable_update_manifest(&legacy_manifest).is_ok());

        let mut dual_manifest = portable_update_test_manifest("1.2.3");
        dual_manifest.assets = [
            (
                "windows-amd64".to_string(),
                portable_update_test_legacy_asset("1.2.3", "amd64"),
            ),
            (
                "windows-aarch64".to_string(),
                portable_update_test_legacy_asset("1.2.3", "aarch64"),
            ),
        ]
        .into_iter()
        .collect();
        dual_manifest.full_assets = Some(
            [
                (
                    "windows-amd64".to_string(),
                    portable_update_test_asset("1.2.3", "amd64"),
                ),
                (
                    "windows-aarch64".to_string(),
                    portable_update_test_asset("1.2.3", "aarch64"),
                ),
            ]
            .into_iter()
            .collect(),
        );
        assert!(validate_portable_update_manifest(&dual_manifest).is_ok());
    }

    let (platform, _, _) = portable_update_asset_platform().unwrap();
    let mut missing_arch = portable_update_test_manifest("1.2.3");
    missing_arch.assets.remove(&format!("{platform}-aarch64"));
    assert!(validate_portable_update_manifest(&missing_arch).is_err());

    let mut invalid_timestamp = portable_update_test_manifest("1.2.3");
    invalid_timestamp.published_at = "not-a-timestamp".to_string();
    assert!(validate_portable_update_manifest(&invalid_timestamp).is_err());

    let mut foreign_host = portable_update_test_manifest("1.2.3");
    foreign_host
            .assets
            .get_mut(&format!("{platform}-amd64"))
            .unwrap()
            .url = "https://github.com.example.invalid/router-for-me/EasyCLIProxyAPI/releases/download/v1.2.3/update.zip".to_string();
    assert!(validate_portable_update_manifest(&foreign_host).is_err());

    let mut mismatched_tag = portable_update_test_manifest("1.2.3");
    let (_, display, suffix) = portable_update_asset_platform().unwrap();
    mismatched_tag
        .assets
        .get_mut(&format!("{platform}-amd64"))
        .unwrap()
        .url = format!(
        "{APP_RELEASE_DOWNLOAD_PREFIX}v9.9.9/EasyCLIProxyAPI-v1.2.3-{display}-amd64.{suffix}"
    );
    assert!(validate_portable_update_manifest(&mismatched_tag).is_err());
}

#[test]
fn portable_update_asset_accepts_only_the_configured_gitcode_fallback() {
    let mut asset = portable_update_test_asset("1.2.3", "amd64");
    let (_, display, suffix) = portable_update_asset_platform().unwrap();
    let filename = format!("EasyCLIProxyAPI-v1.2.3-{display}-amd64.{suffix}");
    asset.fallback_urls = vec![gitcode_release_attachment_url(
        "mirror-owner/EasyCLIProxyAPI",
        "v1.2.3",
        &filename,
    )];
    assert!(validate_portable_update_asset_fallbacks_for_repository(
        &asset,
        "v1.2.3",
        &[&filename],
        Some("mirror-owner/EasyCLIProxyAPI"),
    )
    .is_ok());
    assert!(validate_portable_update_asset_fallbacks_for_repository(
        &asset,
        "v1.2.3",
        &[&filename],
        Some("another-owner/EasyCLIProxyAPI"),
    )
    .is_err());
}

#[test]
fn portable_update_download_order_respects_gitcode_preference() {
    let mut asset = portable_update_test_asset("1.2.3", "amd64");
    let (_, display, suffix) = portable_update_asset_platform().unwrap();
    let filename = format!("EasyCLIProxyAPI-v1.2.3-{display}-amd64.{suffix}");
    asset.fallback_urls = vec![gitcode_release_attachment_url(
        "mirror-owner/EasyCLIProxyAPI",
        "v1.2.3",
        &filename,
    )];

    let github_first = portable_update_download_urls(
        &asset,
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
    );
    assert_eq!(update_download_source_name(&github_first[0]), "GitHub");
    assert_eq!(update_download_source_name(&github_first[1]), "GitCode");

    let gitcode_first = portable_update_download_urls(
        &asset,
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Gitcode),
    );
    assert_eq!(update_download_source_name(&gitcode_first[0]), "GitCode");
    assert_eq!(update_download_source_name(&gitcode_first[1]), "GitHub");

    let mirror_first = portable_update_download_urls(
        &asset,
        &VersionDownloadCandidate::builtin(VersionDownloadSource::GhProxy),
    );
    assert_eq!(
        update_download_source_name(&mirror_first[0]),
        "gh-proxy.com"
    );
    assert_eq!(update_download_source_name(&mirror_first[1]), "GitHub");
    assert_eq!(update_download_source_name(&mirror_first[2]), "GitCode");
}

#[test]
fn version_detection_candidates_try_every_available_source_once() {
    assert_eq!(
        version_download_source_candidates(
            VersionDownloadCandidate::builtin(VersionDownloadSource::GhFast),
            true,
            &[],
        ),
        [
            VersionDownloadCandidate::builtin(VersionDownloadSource::GhFast),
            VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
            VersionDownloadCandidate::builtin(VersionDownloadSource::Gitcode),
            VersionDownloadCandidate::builtin(VersionDownloadSource::GhProxy),
        ]
    );
    assert_eq!(
        version_download_source_candidates(
            VersionDownloadCandidate::builtin(VersionDownloadSource::Gitcode),
            false,
            &[],
        ),
        [
            VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
            VersionDownloadCandidate::builtin(VersionDownloadSource::GhProxy),
            VersionDownloadCandidate::builtin(VersionDownloadSource::GhFast),
        ]
    );
}

#[test]
fn custom_mirror_urls_are_normalized_and_join_the_fallback_chain() {
    assert_eq!(
        normalize_custom_download_mirror_url(" https://mirror.example.com/base ").unwrap(),
        "https://mirror.example.com/base/"
    );
    assert!(normalize_custom_download_mirror_url("http://mirror.example.com/").is_err());
    assert!(normalize_custom_download_mirror_url("https://mirror.example.com/?token=x").is_err());

    let mirrors = vec![
        "https://first.example.com/".to_string(),
        "https://second.example.com/".to_string(),
    ];
    let candidates = version_download_source_candidates(
        VersionDownloadCandidate::custom(&mirrors[1]),
        true,
        &mirrors,
    );
    assert_eq!(candidates[0], VersionDownloadCandidate::custom(&mirrors[1]));
    assert_eq!(
        candidates.last(),
        Some(&VersionDownloadCandidate::custom(&mirrors[0]))
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate.custom_url.is_some())
            .count(),
        2
    );
}

#[test]
fn custom_mirror_is_used_for_portable_update_downloads() {
    let asset = portable_update_test_asset("1.2.3", "amd64");
    let source = VersionDownloadCandidate::custom("https://mirror.example.com/");
    let urls = portable_update_download_urls(&asset, &source);
    assert!(urls[0].starts_with("https://mirror.example.com/https://github.com/"));
    assert_eq!(update_download_source_name(&urls[0]), "mirror.example.com");
    assert_eq!(update_download_source_name(&urls[1]), "GitHub");
}

#[test]
fn portable_update_state_supports_cancellation_and_snapshot_recovery() {
    let state = AppUpdateState::default();
    let pending = PendingAppUpdate {
        version: "1.2.3".to_string(),
        asset: portable_update_test_asset("1.2.3", "amd64"),
        arch: "amd64".to_string(),
    };
    state.set_available(AppUpdateTask {
        phase: "available".to_string(),
        target_version: Some("1.2.3".to_string()),
        ..AppUpdateTask::default()
    });

    let token = CancellationToken::new();
    state.start(token.clone()).unwrap();
    assert_eq!(state.snapshot().phase, "checking");
    assert!(state.start(CancellationToken::new()).is_err());
    state.start_download(&pending).unwrap();
    let recovered = state.snapshot();
    assert!(recovered.running);
    assert!(recovered.cancellable);
    assert_eq!(recovered.phase, "downloading");

    state.cancel();
    assert!(token.is_cancelled());
    let finished = state.finish("cancelled", Some("cancelled".to_string()));
    assert!(!finished.running);
    assert!(!finished.cancellable);
    assert_eq!(state.snapshot().phase, "cancelled");
}

#[tokio::test]
async fn installing_refreshes_an_old_offer_and_bypasses_cached_manifest_responses() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/releases/latest/manifest.json",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
        let fresh = request.contains("manifest.json?_=")
            && request.contains("cache-control: no-cache")
            && request.contains("pragma: no-cache");
        let manifest = portable_update_test_manifest(if fresh { "0.3.3" } else { "0.3.2" });
        let assets: serde_json::Map<String, serde_json::Value> = manifest
            .assets
            .iter()
            .map(|(key, asset)| {
                (
                    key.clone(),
                    serde_json::json!({
                        "url": asset.url, "sha256": asset.sha256, "sizeBytes": asset.size_bytes,
                    }),
                )
            })
            .collect();
        let body = serde_json::json!({
            "schemaVersion": 1, "version": manifest.version,
            "publishedAt": manifest.published_at, "releaseUrl": manifest.release_url,
            "assets": assets,
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let state = AppUpdateState::default();
    state.set_available(AppUpdateTask {
        phase: "available".to_string(),
        target_version: Some("v0.3.2".to_string()),
        total_bytes: Some(1),
        ..AppUpdateTask::default()
    });
    let token = CancellationToken::new();
    state.start(token.clone()).unwrap();
    let pending = refresh_app_update_for_install(&state, &token, async {
        let manifest = fetch_portable_update_manifest_url(&client, &url).await?;
        let (key, arch) = portable_update_target().unwrap();
        Ok(PendingAppUpdate {
            version: normalize_version(&manifest.version),
            asset: manifest.assets.get(key).unwrap().clone(),
            arch: arch.to_string(),
        })
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(pending.version, "v0.3.3");
    assert!(pending.asset.url.contains("/v0.3.3/"));
    assert_eq!(state.snapshot().target_version.as_deref(), Some("v0.3.3"));
    assert_eq!(state.snapshot().total_bytes, Some(pending.asset.size_bytes));
    assert_eq!(state.snapshot().phase, "downloading");
    state.set_available(AppUpdateTask::default());
    assert_eq!(state.snapshot().target_version.as_deref(), Some("v0.3.3"));
}

#[tokio::test]
async fn failed_or_cancelled_refresh_never_downloads_the_previous_offer() {
    let state = AppUpdateState::default();
    state.set_available(AppUpdateTask {
        phase: "available".to_string(),
        target_version: Some("v0.3.2".to_string()),
        ..AppUpdateTask::default()
    });
    let token = CancellationToken::new();
    state.start(token.clone()).unwrap();
    let result = refresh_app_update_for_install(&state, &token, async {
        Err("version check failed".to_string())
    })
    .await;
    assert_eq!(result.err().as_deref(), Some("version check failed"));
    assert_eq!(state.snapshot().phase, "checking");
    assert!(state.snapshot().target_version.is_none());
    state.finish("failed", None);

    let token = CancellationToken::new();
    state.start(token.clone()).unwrap();
    let refresh = refresh_app_update_for_install(
        &state,
        &token,
        std::future::pending::<Result<PendingAppUpdate, String>>(),
    );
    let (result, _) = tokio::join!(refresh, async {
        state.cancel();
    });
    assert_eq!(result.err().as_deref(), Some("Application update download canceled"));
    assert!(state.snapshot().target_version.is_none());
    let pending = PendingAppUpdate {
        version: "v0.3.3".to_string(),
        asset: portable_update_test_asset("0.3.3", "amd64"),
        arch: "amd64".to_string(),
    };
    assert!(state.start_download(&pending).is_err());
}

#[test]
fn sha256_file_hashes_exact_portable_asset_bytes() {
    let root = agent_test_home("portable-sha256");
    let asset = root.join("update.zip");
    fs::write(&asset, b"EasyCLIProxyAPI portable update").unwrap();

    assert_eq!(
        sha256_file(&asset).unwrap(),
        "ade7a05bacf7c9144319c0f0cf431700a8883d3f6effd3613c60749dfba1eb52"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
fn write_portable_update_zip(path: &Path, entries: &[(&str, &[u8], Option<u32>)]) {
    use zip::write::SimpleFileOptions;

    let file = File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    for (name, contents, unix_mode) in entries {
        let mut options = SimpleFileOptions::default();
        if let Some(mode) = unix_mode {
            options = options.unix_permissions(*mode);
        }
        archive.start_file(*name, options).unwrap();
        archive.write_all(contents).unwrap();
    }
    archive.finish().unwrap();
}

#[cfg(windows)]
#[test]
fn portable_update_zip_accepts_the_complete_release_package() {
    let root = agent_test_home("portable-zip");
    let valid_zip = root.join("valid.zip");
    let manifest = br#"{"schemaVersion":1,"application":"EasyCLIProxyAPI","version":"1.2.3","platform":"windows","arch":"amd64","autoUpdate":true}"#;
    write_portable_update_zip(
            &valid_zip,
            &[
                (
                    "EasyCLIProxyAPI-v1.2.3-Windows-amd64/EasyCLIProxyAPI.exe",
                    b"new executable",
                    None,
                ),
                (
                    "EasyCLIProxyAPI-v1.2.3-Windows-amd64/portable-app.json",
                    manifest,
                    None,
                ),
                (
                    "EasyCLIProxyAPI-v1.2.3-Windows-amd64/core-version.txt",
                    b"7.2.109\n",
                    None,
                ),
                (
                    "EasyCLIProxyAPI-v1.2.3-Windows-amd64/cpa-core/CLIProxyAPI_7.2.109_windows_amd64.zip",
                    b"core archive",
                    None,
                ),
            ],
        );
    let extracted = extract_portable_update_archive(&valid_zip, &root.join("valid")).unwrap();
    assert_eq!(extracted.manifest.version, "1.2.3");
    assert!(extracted.manifest.auto_update);
    assert_eq!(
        extracted.core_archive_name,
        Some("CLIProxyAPI_7.2.109_windows_amd64.zip".to_string())
    );

    let legacy_zip = root.join("legacy.zip");
    write_portable_update_zip(
        &legacy_zip,
        &[
            (PORTABLE_APP_BINARY, b"legacy executable", None),
            (PORTABLE_APP_MANIFEST_FILE, manifest, None),
        ],
    );
    let legacy = extract_portable_update_archive(&legacy_zip, &root.join("legacy")).unwrap();
    assert_eq!(legacy.manifest.version, "1.2.3");
    assert!(legacy.core_archive_name.is_none());

    let traversal_zip = root.join("traversal.zip");
    write_portable_update_zip(
        &traversal_zip,
        &[
            ("../EasyCLIProxyAPI.exe", b"malicious", None),
            (
                "EasyCLIProxyAPI-v1.2.3-Windows-amd64/portable-app.json",
                manifest,
                None,
            ),
        ],
    );
    assert!(extract_portable_update_archive(&traversal_zip, &root.join("traversal")).is_err());

    let symlink_zip = root.join("symlink.zip");
    {
        use zip::write::SimpleFileOptions;

        let file = File::create(&symlink_zip).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .add_symlink(
                "EasyCLIProxyAPI-v1.2.3-Windows-amd64/EasyCLIProxyAPI.exe",
                "target.exe",
                SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .start_file(
                "EasyCLIProxyAPI-v1.2.3-Windows-amd64/portable-app.json",
                SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(manifest).unwrap();
        archive.finish().unwrap();
    }
    assert!(extract_portable_update_archive(&symlink_zip, &root.join("symlink")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn portable_update_replacement_preserves_user_data_and_can_roll_back() {
    let root = agent_test_home("portable-replace");
    let platform = current_core_platform().unwrap();
    let old_archive = core_release_asset_name("v7.2.100", &platform);
    let new_archive = core_release_asset_name("v7.2.109", &platform);
    let app_dir = root.join("app");
    let work_dir = root.join("work");
    let staging = work_dir.join("staging");
    fs::create_dir_all(&app_dir).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::create_dir_all(app_dir.join("cpa-core")).unwrap();
    fs::create_dir_all(staging.join("cpa-core")).unwrap();
    fs::write(app_dir.join(PORTABLE_APP_BINARY), b"old exe").unwrap();
    fs::write(app_dir.join(PORTABLE_APP_MANIFEST_FILE), b"old manifest").unwrap();
    fs::write(app_dir.join(CORE_VERSION_FILE), b"7.2.100").unwrap();
    fs::write(
        app_dir
            .join("cpa-core")
            .join(&old_archive),
        b"old core archive",
    )
    .unwrap();
    fs::write(app_dir.join(GUI_CONFIG_FILE), b"user config").unwrap();
    fs::create_dir_all(app_dir.join(OAUTH_DIR_NAME)).unwrap();
    fs::write(app_dir.join(OAUTH_DIR_NAME).join("account.json"), b"oauth").unwrap();
    let preserved_paths = [
        "usage-view-preferences.json",
        "cpa-core/config.yaml",
        "cpa-core/oauth/account.json",
        "cpa-core/custom-auth/account.json",
        "cpa-core/logs/main.log",
        "cpa-core/logs/archive/requests.log",
        "oauth/logs/oauth.log",
    ];
    for path in preserved_paths {
        let path = app_dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"existing user data").unwrap();
    }
    fs::write(staging.join(PORTABLE_APP_BINARY), b"new exe").unwrap();
    fs::write(staging.join(PORTABLE_APP_MANIFEST_FILE), b"new manifest").unwrap();
    fs::write(staging.join(CORE_VERSION_FILE), b"7.2.109").unwrap();
    fs::write(
        staging
            .join("cpa-core")
            .join(&new_archive),
        b"new core archive",
    )
    .unwrap();
    let descriptor = PortableUpdateDescriptor {
        parent_pid: 1,
        current_exe: app_dir.join(PORTABLE_APP_BINARY),
        staged_exe: staging.join(PORTABLE_APP_BINARY),
        current_manifest: app_dir.join(PORTABLE_APP_MANIFEST_FILE),
        staged_manifest: staging.join(PORTABLE_APP_MANIFEST_FILE),
        backup_exe: app_dir.join(format!(".{PORTABLE_APP_BINARY}.update-backup")),
        backup_manifest: app_dir.join(".portable-app.json.update-backup"),
        current_core_version: app_dir.join(CORE_VERSION_FILE),
        staged_core_version: staging.join(CORE_VERSION_FILE),
        backup_core_version: app_dir.join(".core-version.txt.update-backup"),
        staged_core_archive: staging
            .join("cpa-core")
            .join(&new_archive),
        target_core_archive: app_dir
            .join("cpa-core")
            .join(&new_archive),
        install_core_archive: true,
        ack_path: work_dir.join("update-started.ack"),
        work_dir,
        target_version: "1.2.3".to_string(),
    };

    replace_portable_update_files(&descriptor).unwrap();
    for path in preserved_paths {
        assert_eq!(fs::read(app_dir.join(path)).unwrap(), b"existing user data");
    }
    assert_eq!(fs::read(&descriptor.current_exe).unwrap(), b"new exe");
    assert_eq!(
        fs::read(&descriptor.current_manifest).unwrap(),
        b"new manifest"
    );
    assert_eq!(
        fs::read(&descriptor.current_core_version).unwrap(),
        b"7.2.109"
    );
    assert_eq!(
        fs::read(&descriptor.target_core_archive).unwrap(),
        b"new core archive"
    );
    assert_eq!(
        fs::read(app_dir.join(GUI_CONFIG_FILE)).unwrap(),
        b"user config"
    );
    assert_eq!(
        fs::read(app_dir.join(OAUTH_DIR_NAME).join("account.json")).unwrap(),
        b"oauth"
    );

    restore_portable_update_backup(&descriptor).unwrap();
    for path in preserved_paths {
        assert_eq!(fs::read(app_dir.join(path)).unwrap(), b"existing user data");
    }
    assert_eq!(fs::read(&descriptor.current_exe).unwrap(), b"old exe");
    assert_eq!(
        fs::read(&descriptor.current_manifest).unwrap(),
        b"old manifest"
    );
    assert_eq!(
        fs::read(&descriptor.current_core_version).unwrap(),
        b"7.2.100"
    );
    assert!(!descriptor.target_core_archive.exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn portable_update_tar_gz_accepts_the_complete_linux_release() {
    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, Header};

    let root = agent_test_home("portable-linux-tar");
    let archive_path = root.join("update.tar.gz");
    let file = File::create(&archive_path).unwrap();
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = Builder::new(encoder);
    let package_root = "EasyCLIProxyAPI-v1.2.3-Linux-amd64";
    let manifest = br#"{"schemaVersion":1,"application":"EasyCLIProxyAPI","version":"1.2.3","platform":"linux","arch":"amd64","autoUpdate":true}"#;
    for (path, contents, mode) in [
        (
            format!("{package_root}/EasyCLIProxyAPI"),
            b"binary".as_slice(),
            0o755,
        ),
        (
            format!("{package_root}/portable-app.json"),
            manifest.as_slice(),
            0o644,
        ),
        (
            format!("{package_root}/core-version.txt"),
            b"7.2.109\n".as_slice(),
            0o644,
        ),
        (
            format!("{package_root}/cpa-core/CLIProxyAPI_7.2.109_linux_amd64.tar.gz"),
            b"core archive".as_slice(),
            0o644,
        ),
    ] {
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        archive.append_data(&mut header, path, contents).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();

    let staging = root.join("staging");
    let package = extract_portable_update_tar_gz(&archive_path, &staging).unwrap();
    assert_eq!(package.manifest.platform, "linux");
    assert_eq!(package.manifest.arch, "amd64");
    assert_eq!(
        package.core_archive_name.as_deref(),
        Some("CLIProxyAPI_7.2.109_linux_amd64.tar.gz")
    );
    assert_eq!(
        fs::read(staging.join(PORTABLE_APP_BINARY)).unwrap(),
        b"binary"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn macos_update_descriptor_is_confined_to_the_app_and_temp_directories() {
    let root = agent_test_home("portable-macos-descriptor");
    let work_dir = root.join("EasyCLIProxyAPI-update-1.2.3-1-1");
    let current_app = root.join("Applications").join("EasyCLIProxyAPI.app");
    let executable_relative_path = PathBuf::from("Contents/MacOS/cpa-gui");
    let current_exe = current_app.join(&executable_relative_path);
    let staged_app = work_dir.join("staging").join("EasyCLIProxyAPI.app");
    let staged_exe = staged_app.join(&executable_relative_path);
    fs::create_dir_all(current_exe.parent().unwrap()).unwrap();
    fs::create_dir_all(staged_exe.parent().unwrap()).unwrap();
    fs::write(&current_exe, b"old").unwrap();
    fs::write(&staged_exe, b"new").unwrap();
    let descriptor = MacosUpdateDescriptor {
        parent_pid: 1,
        current_app: current_app.clone(),
        staged_app,
        backup_app: current_app
            .parent()
            .unwrap()
            .join(".EasyCLIProxyAPI.app.update-backup"),
        executable_relative_path,
        ack_path: work_dir.join("update-started.ack"),
        work_dir: work_dir.clone(),
        target_version: "1.2.3".to_string(),
    };
    fs::create_dir_all(&work_dir).unwrap();
    let descriptor_path = work_dir.join("update-descriptor.json");
    fs::write(
        &descriptor_path,
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();
    assert!(validate_macos_update_descriptor(&descriptor_path, &descriptor).is_ok());
    assert_eq!(
        macos_application_bundle_from_executable(&current_exe).unwrap(),
        current_app
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn synthetic_release_uses_official_asset_names_and_urls() {
    let release = release_from_tag("7.2.80");
    let platform = CorePlatform {
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        asset_os: "linux".to_string(),
        asset_arch: "amd64".to_string(),
        archive_kind: "tar.gz".to_string(),
    };
    let asset = select_release_asset(&release, &platform).unwrap();

    assert_eq!(release.tag_name, "v7.2.80");
    assert_eq!(asset.name, "CLIProxyAPI_7.2.80_linux_amd64.tar.gz");
    assert_eq!(
            asset.browser_download_url,
            "https://github.com/router-for-me/CLIProxyAPI/releases/download/v7.2.80/CLIProxyAPI_7.2.80_linux_amd64.tar.gz"
        );
}

#[test]
fn synthetic_core_release_uses_gitcode_as_download_fallback() {
    let repository = "lzt404/CLIProxyAPI";
    let release = release_from_tag_for_repositories(
        "7.2.80",
        Some(repository),
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
    );
    let platform = CorePlatform {
        os: "windows".to_string(),
        arch: "x86_64".to_string(),
        asset_os: "windows".to_string(),
        asset_arch: "amd64".to_string(),
        archive_kind: "zip".to_string(),
    };
    let asset = select_release_asset(&release, &platform).unwrap();

    assert_eq!(
            asset.browser_download_url,
            "https://github.com/router-for-me/CLIProxyAPI/releases/download/v7.2.80/CLIProxyAPI_7.2.80_windows_amd64.zip"
        );
    assert_eq!(
            asset.fallback_download_urls,
            ["https://api.gitcode.com/api/v5/repos/lzt404/CLIProxyAPI/releases/v7.2.80/attach_files/CLIProxyAPI_7.2.80_windows_amd64.zip/download"]
        );
}

#[test]
fn gitcode_discovered_core_release_downloads_from_gitcode_first() {
    let release = release_from_gitcode_tag("v7.2.80", "lzt404/CLIProxyAPI");
    let platform = CorePlatform {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
        asset_os: "linux".to_string(),
        asset_arch: "aarch64".to_string(),
        archive_kind: "tar.gz".to_string(),
    };
    let asset = select_release_asset(&release, &platform).unwrap();

    assert_eq!(
            asset.browser_download_url,
            "https://api.gitcode.com/api/v5/repos/lzt404/CLIProxyAPI/releases/v7.2.80/attach_files/CLIProxyAPI_7.2.80_linux_aarch64.tar.gz/download"
        );
    assert_eq!(
        asset.fallback_download_urls,
        ["https://github.com/router-for-me/CLIProxyAPI/releases/download/v7.2.80/CLIProxyAPI_7.2.80_linux_aarch64.tar.gz"]
    );
}

#[test]
fn github_proxy_core_release_uses_proxy_then_official_and_gitcode() {
    let release = release_from_tag_for_repositories(
        "v7.2.80",
        Some("lzt404/CLIProxyAPI"),
        &VersionDownloadCandidate::builtin(VersionDownloadSource::GhFast),
    );
    let platform = CorePlatform {
        os: "windows".to_string(),
        arch: "x86_64".to_string(),
        asset_os: "windows".to_string(),
        asset_arch: "amd64".to_string(),
        archive_kind: "zip".to_string(),
    };
    let asset = select_release_asset(&release, &platform).unwrap();

    assert_eq!(
        core_download_source_name(&asset.browser_download_url),
        "ghfast.top"
    );
    assert_eq!(
        core_download_source_name(&asset.fallback_download_urls[0]),
        "GitHub"
    );
    assert_eq!(
        core_download_source_name(&asset.fallback_download_urls[1]),
        "GitCode"
    );
}

#[test]
fn core_download_candidates_follow_the_complete_fallback_chain() {
    let release = release_from_tag_for_repositories(
        "v7.2.80",
        Some("lzt404/CLIProxyAPI"),
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
    );
    let platform = CorePlatform {
        os: "windows".to_string(),
        arch: "x86_64".to_string(),
        asset_os: "windows".to_string(),
        asset_arch: "amd64".to_string(),
        archive_kind: "zip".to_string(),
    };
    let asset = select_release_asset(&release, &platform).unwrap();
    let custom_mirrors = vec!["https://mirror.example.com/".to_string()];
    let candidates = core_download_candidates(
        &release.tag_name,
        asset,
        VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
        Some("lzt404/CLIProxyAPI"),
        &custom_mirrors,
    );

    assert_eq!(
        candidates
            .iter()
            .map(|(candidate, _)| candidate.key())
            .collect::<Vec<_>>(),
        [
            "github",
            "gitcode",
            "gh-proxy",
            "gh-fast",
            "custom:https://mirror.example.com/",
        ]
    );
    assert_eq!(
        candidates
            .iter()
            .map(|(_, url)| core_download_source_name(url))
            .collect::<Vec<_>>(),
        [
            "GitHub",
            "GitCode",
            "gh-proxy.com",
            "ghfast.top",
            "mirror.example.com",
        ]
    );
}

#[test]
fn release_asset_names_cover_the_six_supported_gui_targets() {
    let targets = [
        ("linux", "amd64", "tar.gz"),
        ("linux", "aarch64", "tar.gz"),
        ("darwin", "amd64", "tar.gz"),
        ("darwin", "aarch64", "tar.gz"),
        ("windows", "amd64", "zip"),
        ("windows", "aarch64", "zip"),
    ];
    for (os, arch, archive_kind) in targets {
        let platform = CorePlatform {
            os: os.to_string(),
            arch: arch.to_string(),
            asset_os: os.to_string(),
            asset_arch: arch.to_string(),
            archive_kind: archive_kind.to_string(),
        };
        assert_eq!(
            core_release_asset_name("v7.2.83", &platform),
            format!("CLIProxyAPI_7.2.83_{os}_{arch}.{archive_kind}")
        );
    }
}
