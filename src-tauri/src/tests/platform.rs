#[cfg(target_os = "windows")]
use super::support::agent_test_home;
use super::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn windows_tray_presentation_tracks_core_state_and_busy_actions() {
    let mut status = CoreStatus {
        installed: false,
        running: false,
        ready: false,
        starting: false,
        managed: false,
        process_id: None,
        current_version: None,
        install_dir: String::new(),
        binary_path: None,
        message: String::new(),
    };

    let missing = windows_tray_presentation(&status, false, "zh-CN");
    assert_eq!(missing.status_text, "内核状态：未安装");
    assert!(!missing.toggle_enabled);
    assert!(!missing.restart_enabled);

    status.installed = true;
    let stopped = windows_tray_presentation(&status, false, "zh-CN");
    assert_eq!(stopped.toggle_text, "启动内核");
    assert!(stopped.toggle_enabled);
    assert!(!stopped.restart_enabled);

    status.running = true;
    let running = windows_tray_presentation(&status, false, "zh-CN");
    assert_eq!(running.status_text, "内核状态：运行中");
    assert_eq!(running.toggle_text, "停止内核");
    assert!(running.toggle_enabled);
    assert!(running.restart_enabled);

    let busy = windows_tray_presentation(&status, true, "zh-CN");
    assert_eq!(busy.status_text, "内核状态：处理中");
    assert!(!busy.toggle_enabled);
    assert!(!busy.restart_enabled);

    let english = windows_tray_presentation(&status, false, "en-US");
    assert_eq!(english.status_text, "Core status: Running");
    assert_eq!(english.toggle_text, "Stop Core");

    let japanese = windows_tray_presentation(&status, false, "ja-JP");
    assert_eq!(japanese.status_text, "コア状態：実行中");
    assert_eq!(japanese.toggle_text, "コアを停止");
}

#[test]
fn app_locale_normalization_has_a_stable_english_fallback() {
    assert_eq!(normalize_app_locale("en"), "en");
    assert_eq!(normalize_app_locale("en-US"), "en");
    assert_eq!(normalize_app_locale("ja-JP"), "ja");
    assert_eq!(normalize_app_locale("zh-TW"), "zh-TW");
    assert_eq!(normalize_app_locale("zh-Hant-HK"), "zh-TW");
    assert_eq!(normalize_app_locale("unsupported"), "en");
    assert_eq!(GuiConfigFile::default().locale, "en");
}

#[cfg(target_os = "windows")]
#[test]
fn codex_owl_version_uses_app_metadata_not_package_or_runtime_version() {
    assert_eq!(
        parse_codex_owl_app_version(
            "\u{feff}[Owl]\r\nUserDataDirectoryName=Codex\r\nAppVersion=26.901.51231\r\n"
        )
        .as_deref(),
        Some("26.901.51231")
    );
    assert_eq!(
        parse_codex_owl_app_version("[owl]\n AppVersion = 26.901.51231 \n").as_deref(),
        Some("26.901.51231")
    );
    for content in [
        "AppVersion=26.901.51231",
        "[Other]\nAppVersion=26.901.51231",
        "[Owl]\n[Other]\nAppVersion=26.901.51231",
        "[Owl]\nAppVersion=unknown",
        "[Owl]\nAppVersion=",
        "[Owl]\nAppVersion=1.2.3\0invalid",
    ] {
        assert!(
            parse_codex_owl_app_version(content).is_none(),
            "{content:?}"
        );
    }
}

#[cfg(target_os = "windows")]
fn codex_version_test_asar(package: &[u8], offset: &str, size: u64) -> Vec<u8> {
    let header = serde_json::to_vec(&serde_json::json!({
        "files": { "package.json": { "offset": offset, "size": size } }
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
    archive.extend_from_slice(package);
    archive
}

#[cfg(target_os = "windows")]
#[test]
fn codex_desktop_version_prefers_owl_then_asar_without_launching_the_app() {
    let home = agent_test_home("codex-desktop-version");
    let resources = home.join("resources");
    fs::create_dir_all(&resources).unwrap();
    let executable = home.join("ChatGPT.exe");
    fs::write(&executable, b"not an executable").unwrap();
    let package = br#"{"name":"openai-codex-electron","version":"26.901.51231"}"#;
    let mut data = b"other archive data".to_vec();
    let offset = data.len().to_string();
    data.extend_from_slice(package);
    let asar = resources.join("app.asar");
    fs::write(
        &asar,
        codex_version_test_asar(&data, &offset, package.len() as u64),
    )
    .unwrap();
    assert_eq!(
        read_codex_asar_version(&asar).as_deref(),
        Some("26.901.51231")
    );
    assert_eq!(
        read_windows_codex_desktop_version(&executable).as_deref(),
        Some("26.901.51231")
    );

    let ini = resources.join("owl-app.ini");
    fs::write(&ini, "[Owl]\nAppVersion=26.902.12345\n").unwrap();
    assert_eq!(
        read_windows_codex_desktop_version(&executable).as_deref(),
        Some("26.902.12345")
    );
    fs::write(&ini, "[Owl]\nAppVersion=unknown\n").unwrap();
    assert_eq!(
        read_windows_codex_desktop_version(&executable).as_deref(),
        Some("26.901.51231")
    );
    fs::write(&asar, b"broken archive").unwrap();
    assert!(read_windows_codex_desktop_version(&executable).is_none());
    fs::remove_file(&asar).unwrap();
    assert!(read_windows_codex_desktop_version(&executable).is_none());
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn codex_asar_version_rejects_invalid_sizes_offsets_and_metadata() {
    let home = agent_test_home("codex-invalid-asar-version");
    let asar = home.join("app.asar");
    let package = br#"{"version":"26.901.51231"}"#;
    let valid = codex_version_test_asar(package, "0", package.len() as u64);
    let mut oversized_header = valid.clone();
    oversized_header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut oversized_json = valid.clone();
    oversized_json[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    for archive in [
        Vec::new(),
        valid[..15].to_vec(),
        valid[..valid.len() - 1].to_vec(),
        oversized_header,
        oversized_json,
        codex_version_test_asar(package, "18446744073709551615", package.len() as u64),
        codex_version_test_asar(package, "-1", package.len() as u64),
        codex_version_test_asar(package, "0", 1024 * 1024 + 1),
        codex_version_test_asar(b"{}", "0", 2),
        codex_version_test_asar(b"invalid", "0", 7),
    ] {
        fs::write(&asar, archive).unwrap();
        assert!(read_codex_asar_version(&asar).is_none());
    }
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn codex_store_version_resolves_only_the_selected_family_and_application() {
    let home = agent_test_home("codex-store-version-target");
    let executable = home.join("ChatGPT.exe");
    fs::write(&executable, b"test").unwrap();
    fs::write(
        home.join("AppxManifest.xml"),
        r#"<Package><Applications>
            <Application Id="Other" Executable="wrong.exe" />
            <Application Id="App" Executable="ChatGPT.exe" />
        </Applications></Package>"#,
    )
    .unwrap();
    let package = WindowsAppxPackage {
        full_name: "OpenAI.Codex_26.901.6511.0_x64__publisher".to_string(),
        family_name: "OpenAI.Codex_publisher".to_string(),
        version: "26.901.6511.0".to_string(),
        install_location: home.clone(),
    };
    assert_eq!(
        windows_appx_executable(&package, Some("App")).as_deref(),
        Some(executable.as_path())
    );
    assert!(windows_appx_executable(&package, Some("Missing")).is_none());
    assert_eq!(
        windows_appx_app_id(&package).as_deref(),
        Some("OpenAI.Codex_publisher!Other")
    );
    assert!(windows_codex_store_app_id_parts("invalid").is_none());
    assert!(windows_codex_store_app_id_parts("!App").is_none());
    assert!(windows_codex_store_app_id_parts("OpenAI.Codex_publisher!").is_none());
    assert_eq!(
        windows_codex_store_app_id_parts("OpenAI.Codex_o'brien!App"),
        Some(("OpenAI.Codex_o'brien", "App"))
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn main_window_is_found_after_a_child_webview_is_added() {
    let app = tauri::test::mock_app();
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    window
        .as_ref()
        .window()
        .add_child(
            tauri::webview::WebviewBuilder::new("plugin", Default::default()),
            tauri::LogicalPosition::new(0.0, 0.0),
            tauri::LogicalSize::new(1.0, 1.0),
        )
        .unwrap();

    assert!(app.get_webview_window("main").is_none());
    assert_eq!(main_window(&app).unwrap().label(), "main");
}
