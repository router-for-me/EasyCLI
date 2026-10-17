use super::support::*;
use super::*;

#[test]
fn installation_detection_separates_executables_from_version_metadata() {
    for client in [
        AgentClient::ClaudeCode,
        AgentClient::Codex,
        AgentClient::OpenCode,
        AgentClient::OpenClaw,
        AgentClient::Hermes,
        AgentClient::DeepSeekHarness,
        AgentClient::KimiCode,
        AgentClient::GrokBuild,
        AgentClient::Omp,
    ] {
        assert!(agent_installation_detected(client, None, true, false));
        assert!(!agent_installation_detected(client, None, false, false));
        assert!(!agent_installation_detected(
            client,
            Some("1.0.0"),
            false,
            false
        ));
    }
    assert!(agent_installation_detected(
        AgentClient::Codex,
        None,
        false,
        true
    ));
    assert!(agent_installation_detected(
        AgentClient::ClaudeDesktop,
        Some("1.0.0"),
        false,
        false,
    ));
}

#[test]
fn executable_directories_preserve_path_precedence_and_package_manager_locations() {
    let home = agent_test_home("agent-search-environment");
    let inherited = home.join("inherited");
    let registered = home.join("registered");
    let npm = home.join("custom-npm");
    let uv = home.join("custom-uv");
    let volta = home.join("custom-volta");
    let directories = agent_executable_directories_from_environment(
        &home,
        |name| match name {
            "PATH" => Some(env::join_paths([&inherited, &inherited]).unwrap()),
            "NPM_CONFIG_PREFIX" => Some(npm.clone().into_os_string()),
            "UV_TOOL_BIN_DIR" => Some(uv.clone().into_os_string()),
            "VOLTA_HOME" => Some(volta.clone().into_os_string()),
            "BUN_INSTALL" => Some(std::ffi::OsString::new()),
            _ => None,
        },
        &[registered.clone(), inherited.clone()],
    );
    assert_eq!(&directories[..2], &[inherited.clone(), registered]);
    assert_eq!(
        directories
            .iter()
            .filter(|path| **path == inherited)
            .count(),
        1
    );
    let npm_bin = if cfg!(target_os = "windows") {
        npm
    } else {
        npm.join("bin")
    };
    assert!(directories.contains(&npm_bin));
    assert!(directories.contains(&uv));
    assert!(directories.contains(&volta.join("bin")));
    assert!(directories.iter().all(|path| path.is_absolute()));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn manual_executable_overrides_satisfy_the_agent_enable_check() {
    let home = agent_test_home("manual-executable-enable");
    let (cli_name, desktop_name) = if cfg!(target_os = "windows") {
        ("opencode.exe", "OpenCode-2.0.26.exe")
    } else {
        ("opencode", "opencode-desktop-2.0.26")
    };
    let cli = home.join(cli_name);
    let desktop = home.join(desktop_name);
    fs::write(&cli, "cli fixture").unwrap();
    fs::write(&desktop, "desktop fixture").unwrap();

    let overrides = HashMap::from([
        ("opencode".to_string(), path_to_string(&cli)),
        ("opencode:app".to_string(), path_to_string(&desktop)),
    ]);
    let (cli_override, desktop_override) =
        agent_executable_overrides(&overrides, AgentClient::OpenCode);
    assert_eq!(cli_override, Some(cli.as_path()));
    assert_eq!(desktop_override, Some(desktop.as_path()));

    let desktop_only =
        HashMap::from([("opencode:app".to_string(), path_to_string(&desktop))]);
    assert!(
        validate_agent_can_enable(AgentClient::OpenCode, &home, 8317, "test-key", Some(&desktop_only))
            .is_ok(),
        "a manually selected desktop application must satisfy the enable check"
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn manual_pi_executable_override_wins_over_automatic_detection() {
    let home = agent_test_home("pi-manual-executable");
    let cli_name = if cfg!(target_os = "windows") { "pi.exe" } else { "pi" };
    let cli = home.join(cli_name);
    fs::write(&cli, "pi fixture").unwrap();
    let overrides = HashMap::from([(PI_AGENT_ID.to_string(), path_to_string(&cli))]);

    assert_eq!(resolve_pi_executable(&home, Some(&overrides)), Some(cli.clone()));
    assert_eq!(pi_executable_override(Some(&overrides)), Some(cli.as_path()));
    assert!(pi_executable_override(Some(&HashMap::new())).is_none());

    let missing = home.join("missing-pi");
    let stale = HashMap::from([(PI_AGENT_ID.to_string(), path_to_string(&missing))]);
    assert_ne!(resolve_pi_executable(&home, Some(&stale)), Some(missing.clone()));
    // The raw override is passed through unfiltered; consumers ignore it when it is not a file.
    assert_eq!(pi_executable_override(Some(&stale)), Some(missing.as_path()));
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn windows_cli_search_skips_shell_stubs_and_finds_runnable_shims() {
    let home = agent_test_home("agent-search-shims");
    let first = home.join("first");
    let second = home.join("second");
    fs::create_dir_all(first.join("agent.exe")).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("agent"), "shell stub").unwrap();
    fs::write(first.join("agent.ps1"), "powershell stub").unwrap();
    let executable = second.join("agent.cmd");
    fs::write(&executable, "@exit /b 1\r\n").unwrap();
    assert_eq!(
        find_named_agent_executable_in_directories(&[first, second], &["agent"]),
        Some(executable.clone()),
    );
    let version = read_agent_version(&executable, &home);
    assert_eq!(version, None);
    assert!(agent_installation_detected(
        AgentClient::ClaudeCode,
        version.as_deref(),
        agent_cli_executable(&executable),
        false,
    ));
    fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[test]
fn unix_cli_search_requires_executable_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let home = agent_test_home("agent-search-permissions");
    let executable = home.join("agent");
    fs::write(&executable, "exit 1\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        find_named_agent_executable_in_directories(&[home.clone()], &["agent"]),
        None
    );
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        find_named_agent_executable_in_directories(&[home.clone()], &["agent"]),
        Some(executable)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn configuration_paths_ignore_inherited_environment() {
    let home = agent_test_home("isolated-paths");
    for client in [
        "claude-code",
        "claude-desktop",
        "codex",
        "opencode",
        "openclaw",
        "hermes",
        "deepseek-harness",
        "zcode",
        "workbuddy",
        "antigravity-cli",
        "kimi-code",
        "grok-build",
        "pi",
    ] {
        let paths = config_paths(client, &home).unwrap();
        assert!(!paths.is_empty(), "{client}");
        assert!(
            paths.iter().all(|path| path.starts_with(&home)),
            "{client}: {paths:?}"
        );
    }
    if env::var_os("CPA_PATH_ISOLATION_CHILD").is_none() {
        let outside = home.join("inherited-environment");
        let mut command = Command::new(env::current_exe().unwrap());
        command.args([
            "--exact",
            "tests::agent_paths::configuration_paths_ignore_inherited_environment",
            "--nocapture",
        ]);
        command.env("CPA_PATH_ISOLATION_CHILD", "1");
        for variable in [
            "LOCALAPPDATA",
            "APPDATA",
            "XDG_CONFIG_HOME",
            "CODEX_HOME",
            "HERMES_HOME",
            "OPENCODE_CONFIG",
            "KIMI_CODE_HOME",
            "GROK_HOME",
            "DSH_HOME",
            "WORKBUDDY_CONFIG_DIR",
            "CODEBUDDY_CONFIG_DIR",
            "WORKBUDDY_INSTALL_DIR",
            "PI_CODING_AGENT_DIR",
        ] {
            command.env(variable, &outside);
        }
        configure_background_command(&mut command);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!outside.exists());
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn zcode_installation_does_not_require_version_metadata() {
    assert!(agent_installation_detected(
        AgentClient::ZCode,
        None,
        true,
        false,
    ));
    assert!(!agent_installation_detected(
        AgentClient::ZCode,
        None,
        false,
        false,
    ));
    let targets = agent_launch_targets(
        AgentClient::ZCode,
        Some(Path::new("ZCode.exe")),
        None,
        false,
    );
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "app");
}

#[cfg(target_os = "windows")]
#[test]
fn zcode_windows_finds_custom_installations_from_registered_paths() {
    let home = agent_test_home("zcode-custom-installation");
    let directory = home.join("自定义应用 [桌面]/ZCode's directory");
    let executable = directory.join("ZCode.exe");
    fs::create_dir_all(&directory).unwrap();
    fs::write(&executable, []).unwrap();
    let icon = directory.join("uninstallerIcon.ico");
    let uninstaller = directory.join("Uninstall ZCode.exe");
    for (kind, value) in [
        ("executable", path_to_string(&executable)),
        ("executable", format!("\"{}\"", executable.display())),
        ("directory", path_to_string(&directory)),
        ("icon", path_to_string(&icon)),
        ("icon", format!("{},0", executable.display())),
        ("icon", format!("\"{}\",-123", executable.display())),
        (
            "uninstaller",
            format!("\"{}\" /currentuser", uninstaller.display()),
        ),
        (
            "uninstaller",
            format!("{} /allusers", uninstaller.display()),
        ),
    ] {
        assert_eq!(
            parse_windows_zcode_registration(kind, &value),
            Some(executable.clone()),
            "kind={kind}, value={value}",
        );
    }
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn zcode_windows_display_name_matches_installer_variants() {
    assert!(windows_display_name_matches_zcode("ZCode"));
    assert!(windows_display_name_matches_zcode("ZCode (64-bit)"));
    assert!(windows_display_name_matches_zcode("ZCode Desktop"));
    assert!(!windows_display_name_matches_zcode("ZCodeHelper"));
    assert!(!windows_display_name_matches_zcode("MyZCode"));
}

#[cfg(target_os = "windows")]
#[test]
fn zcode_windows_skips_stale_and_unrelated_registration_entries() {
    let home = agent_test_home("zcode-stale-registration");
    let unrelated = home.join("Uninstall ZCode.exe");
    fs::write(&unrelated, []).unwrap();
    let executable = home.join("valid/ZCode.exe");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, []).unwrap();
    for (kind, value) in [
        ("directory", path_to_string(&home)),
        ("executable", path_to_string(&unrelated)),
        (
            "executable",
            path_to_string(&home.join("missing/ZCode.exe")),
        ),
        ("executable", "ZCode.exe".to_string()),
        ("executable", "\"unterminated".to_string()),
        ("unknown", path_to_string(&executable)),
    ] {
        assert_eq!(
            parse_windows_zcode_registration(kind, &value),
            None,
            "kind={kind}, value={value}",
        );
    }
    assert_eq!(
        parse_windows_zcode_registration("executable", &path_to_string(&executable)),
        Some(executable),
    );
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn zcode_windows_status_refresh_does_not_launch_path_candidates() {
    let home = agent_test_home("zcode-status-no-launch");
    let executable = home.join("ZCode.cmd");
    let marker = home.join("launched.txt");
    fs::write(
        &executable,
        format!("@echo off\r\necho launched > \"{}\"\r\necho 3.15.3\r\n", marker.display()),
    )
    .unwrap();

    if env::var_os("CPA_ZCODE_STATUS_CHILD").is_none() {
        // Isolate PATH in a child so parallel tests and real installations are unaffected.
        let mut command = Command::new(env::current_exe().unwrap());
        command.args([
            "--exact",
            "tests::agent_paths::zcode_windows_status_refresh_does_not_launch_path_candidates",
            "--nocapture",
        ]);
        command.env("CPA_ZCODE_STATUS_CHILD", "1");
        configure_background_command(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
    } else {
        env::set_var("PATH", &home);
        assert_eq!(find_named_agent_executable(&home, &["zcode"]), Some(home.join("zcode.cmd")));
        let other = home.join("OtherZCode.cmd");
        fs::copy(&executable, &other).unwrap();
        // Cover both case differences for the same file and a separate installation.
        for selected in [&executable, &other] {
            let status = inspect_agent_config_with_executables(
                AgentClient::ZCode, &home, 8317, "test-key", Some(selected), None,
            );
            assert!(!marker.exists(), "status refresh launched the application");
            assert!(status.installed);
            assert_eq!(status.cli_version, None);
            assert_eq!(status.app_version, None);
            assert_eq!(status.launch_targets.len(), 1);
            assert_eq!(status.launch_targets[0].id, "app");
        }
    }
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn zcode_windows_version_discovery_does_not_launch_the_application() {
    let home = agent_test_home("zcode-version-no-launch");
    let executable = home.join("zcode.cmd");
    let marker = home.join("launched.txt");
    fs::write(
        &executable,
        format!(
            "@echo off\r\necho launched > \"{}\"\r\necho 1.0.0\r\n",
            marker.display()
        ),
    )
    .unwrap();
    assert_eq!(read_zcode_app_version(&executable), None);
    assert!(
        !marker.exists(),
        "version discovery launched the application"
    );
    fs::remove_dir_all(home).unwrap();
}
