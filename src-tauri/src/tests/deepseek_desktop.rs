#[cfg(any(target_os = "windows", target_os = "macos"))]
use super::support::agent_test_home;
use super::*;

#[cfg(target_os = "macos")]
fn deepseek_desktop_test_asar(version: &str) -> Vec<u8> {
    let package = format!(r#"{{"name":"deepseek-harness","version":"{version}"}}"#);
    let header = serde_json::to_vec(&serde_json::json!({
        "files": { "package.json": { "offset": "0", "size": package.len() } }
    }))
    .unwrap();
    let payload_size = (4 + header.len()).next_multiple_of(4) as u32;
    let header_size = 4 + payload_size;
    let mut archive = Vec::new();
    for value in [4, header_size, payload_size, header.len() as u32] {
        archive.extend_from_slice(&value.to_le_bytes());
    }
    archive.extend(header);
    archive.resize(8 + header_size as usize, 0);
    archive.extend_from_slice(package.as_bytes());
    archive
}

#[cfg(target_os = "macos")]
#[test]
fn deepseek_desktop_macos_version_reads_bundle_and_executable_paths() {
    let home = agent_test_home("deepseek-desktop-macos-version");
    let bundle = home.join("Custom/DeepSeek Harness.app");
    let resources = bundle.join("Contents/Resources");
    let executable = bundle.join("Contents/MacOS/DeepSeek Harness");
    fs::create_dir_all(&resources).unwrap();
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, []).unwrap();
    fs::write(
        resources.join("app.asar"),
        deepseek_desktop_test_asar("1.2.3-beta.1"),
    )
    .unwrap();
    assert_eq!(
        read_deepseek_harness_desktop_version(&bundle).as_deref(),
        Some("1.2.3-beta.1")
    );
    assert_eq!(
        read_deepseek_harness_desktop_version(&executable).as_deref(),
        Some("1.2.3-beta.1")
    );
    // A bundle without Resources/app.asar must not fall back to a sibling archive.
    let empty = home.join("Custom/Empty.app");
    fs::create_dir_all(empty.join("Contents")).unwrap();
    fs::create_dir_all(home.join("Resources")).unwrap();
    fs::write(
        home.join("Resources/app.asar"),
        deepseek_desktop_test_asar("9.9.9"),
    )
    .unwrap();
    assert!(read_deepseek_harness_desktop_version(&empty).is_none());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn deepseek_desktop_launch_targets_are_independent_of_cli_and_version() {
    let client = AgentClient::DeepSeekHarness;
    assert!(agent_installation_detected(client, None, false, true));
    let desktop = agent_launch_targets(client, None, None, true);
    assert_eq!(desktop.len(), 1);
    assert_eq!(desktop[0].id, "app");
    let both = agent_launch_targets(client, Some(Path::new("dsh")), None, true);
    assert_eq!(
        both.iter()
            .map(|target| target.id.as_str())
            .collect::<Vec<_>>(),
        ["cli", "app"]
    );
    // An installed profile alone must not create a Desktop launch target.
    assert!(agent_launch_targets(client, None, Some("0.2.0"), false).is_empty());
    assert!(!agent_installation_detected(client, None, false, false));
}

#[cfg(target_os = "windows")]
#[test]
fn deepseek_desktop_windows_finds_custom_and_default_installations() {
    let home = agent_test_home("deepseek-desktop-discovery");
    let local = home.join("local");
    let custom = home.join("custom/DeepSeek Harness.exe");
    let default = local.join("Programs/DeepSeek Harness/DeepSeek Harness.exe");
    fs::create_dir_all(custom.parent().unwrap()).unwrap();
    fs::create_dir_all(default.parent().unwrap()).unwrap();
    fs::write(&custom, []).unwrap();
    for (kind, value) in [
        ("directory", path_to_string(custom.parent().unwrap())),
        ("icon", format!("\"{}\",0", path_to_string(&custom))),
        (
            "uninstaller",
            format!(
                "\"{}\" /S",
                path_to_string(
                    &custom
                        .parent()
                        .unwrap()
                        .join("Uninstall DeepSeek Harness.exe")
                )
            ),
        ),
    ] {
        let entries = [WindowsZcodeRegistration { kind, value }];
        assert_eq!(
            find_windows_deepseek_harness_desktop(&local, &[], &entries),
            Some(custom.clone())
        );
    }
    fs::remove_file(&custom).unwrap();
    let stale = [WindowsZcodeRegistration {
        kind: "executable",
        value: path_to_string(&custom),
    }];
    assert_eq!(
        find_windows_deepseek_harness_desktop(&local, &[], &stale),
        None
    );
    fs::write(&default, []).unwrap();
    assert_eq!(
        find_windows_deepseek_harness_desktop(&local, &[], &stale),
        Some(default)
    );
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn deepseek_desktop_windows_registration_name_is_specific() {
    for name in [
        "DeepSeek Harness",
        "DeepSeek Harness 0.2.0-rc.2",
        "deepseek harness (user)",
    ] {
        assert!(windows_display_name_matches_deepseek_harness(name));
    }
    for name in [
        "DeepSeek",
        "DeepSeek HarnessHelper",
        "Other DeepSeek Harness",
    ] {
        assert!(!windows_display_name_matches_deepseek_harness(name));
    }
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "Requires DeepSeek Harness Desktop installed on the host"]
fn deepseek_desktop_installed_host_probe() {
    let registrations = collect_windows_desktop_registrations(
        "DeepSeek Harness.exe",
        windows_display_name_matches_deepseek_harness,
    );
    let local = PathBuf::from(env::var_os("LOCALAPPDATA").unwrap());
    let app = find_windows_deepseek_harness_desktop(&local, &[], &registrations)
        .expect("installed Desktop should be detected");
    let version = read_deepseek_harness_desktop_version(&app).expect("Desktop version");
    println!("DeepSeek Harness Desktop: {} ({version})", app.display());
}
