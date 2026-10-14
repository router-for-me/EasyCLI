use super::support::*;
use super::*;

#[test]
fn image_generation_preference_round_trips_without_leaking_into_kernel_yaml() {
    let home = agent_test_home("image-generation-preference");
    let path = home.join("config.toml");
    let mut config: GuiConfigFile = toml::from_str("").unwrap();
    assert!(!config.image_generation_user_selected);
    write_gui_config_to_path(&config, &path).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("image-generation-user-selected"));
    config.image_generation_user_selected = true;
    write_gui_config_to_path(&config, &path).unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("image-generation-user-selected = true"));
    let restored: GuiConfigFile = toml::from_str(&content).unwrap();
    assert!(restored.image_generation_user_selected);
    let template = "config-version: 8\nmultimedia: {disable-image-generation: false}\napi-keys: {codex: [{keys: [{api-key: test-key}]}]}\n";
    let merged = merge_core_config_yaml(template, Some(template), &restored).unwrap();
    let document: serde_norway::Value = serde_norway::from_str(&merged).unwrap();
    assert_eq!(document["multimedia"]["disable-image-generation"], false);
    assert!(!merged.contains("image-generation-user-selected"));
    config.image_generation_user_selected = false;
    write_gui_config_to_path(&config, &path).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("image-generation-user-selected"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn usage_statistics_opt_out_is_removed_when_reenabled() {
    let home = agent_test_home("usage-opt-out");
    let path = home.join("config.toml");
    let mut config = GuiConfigFile::default();
    config.usage_statistics_disabled = Some(true);
    write_gui_config_to_path(&config, &path).unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("usage_statistics_disabled = true"));
    let restored: GuiConfigFile = toml::from_str(&content).unwrap();
    assert_eq!(restored.usage_statistics_disabled, Some(true));
    config.usage_statistics_disabled = None;
    write_gui_config_to_path(&config, &path).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("usage_statistics_disabled"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn usage_statistics_startup_repairs_stale_values_unless_explicitly_disabled() {
    for preference in ["", "usage_statistics_disabled = false\n", "usage_statistics_disabled = true\n"] {
        let config: GuiConfigFile = toml::from_str(&format!("usage-statistics-enabled = false\n{preference}")).unwrap();
        for template in ["usage-statistics-enabled: false\n", "config-version: 8\nobservability: {usage: {usage-statistics-enabled: false}}\n"] {
            for current in [None, Some("usage-statistics-enabled: false\n"), Some(template)] {
                let merged = merge_core_config_yaml(template, current, &config).unwrap();
                let document = serde_norway::from_str(&merged).unwrap();
                let settings = core_config_settings_from_value(&document).unwrap();
                assert_eq!(settings.usage_statistics_enabled, config.usage_statistics_disabled != Some(true));
                assert!(!merged.contains("usage_statistics_disabled"));
            }
        }
    }
}

#[test]
fn gui_field_edit_preserves_comments_and_unknown_configuration() {
    let home = agent_test_home("gui-field-edit");
    let path = home.join("config.toml");
    fs::write(
            &path,
            "# keep this comment\ncustom-option = \"keep\"\ncodex-session-repair-on-launch = true\nclaude-code-working-directory = \"legacy\"\nclaude-code-working-directory-prompt-disabled = true\nport = 7000\n\n[third-party]\nenabled = true\n",
        )
        .unwrap();
    let config = GuiConfigFile {
        port: 9527,
        host: "0.0.0.0".to_string(),
        allow_lan: true,
        silent_start: true,
        auth_dir: path_to_string(&home.join("custom-auth")),
        management_secret_key: "custom-secret".to_string(),
        usage_statistics_enabled: false,
        disable_cooling: true,
        download_source: VersionDownloadSource::Gitcode,
        prefer_gitcode_downloads: true,
        ..GuiConfigFile::default()
    };

    write_gui_config_to_path(&config, &path).unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("# keep this comment"));
    assert!(content.contains("custom-option = \"keep\""));
    assert!(content.contains("[third-party]"));
    assert!(content.contains("port = 9527"));
    assert!(content.contains("host = \"0.0.0.0\""));
    assert!(content.contains("silent-start = true"));
    assert!(content.contains("management-secret-key = \"custom-secret\""));
    assert!(content.contains("usage-statistics-enabled = false"));
    assert!(content.contains("disable-cooling = true"));
    assert!(content.contains("download-source = \"gitcode\""));
    assert!(content.contains("prefer-gitcode-downloads = true"));
    assert!(!content.contains("codex-session-repair-on-launch"));
    assert!(!content.contains("claude-code-working-directory"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn software_write_hash_suppresses_only_the_matching_file_content() {
    let home = agent_test_home("write-hash");
    let path = home.join("config.toml");
    write_bytes_directly(&path, b"port = 8317\n").unwrap();
    assert!(consume_software_write(&path));
    assert!(!consume_software_write(&path));
    write_bytes_directly(&path, b"port = 8317\n").unwrap();
    fs::write(&path, b"port = 9000\n").unwrap();
    assert!(!consume_software_write(&path));
    fs::write(&path, b"port = 8317\n").unwrap();
    assert!(!consume_software_write(&path));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn configuration_watcher_uses_the_nearest_existing_parent() {
    let home = agent_test_home("watch-parent");
    let target = home.join("nested/client/config.json");

    assert_eq!(
        nearest_existing_watch_directory(&target),
        Some(home.clone())
    );
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    assert_eq!(
        nearest_existing_watch_directory(&target),
        target.parent().map(Path::to_path_buf)
    );

    fs::remove_dir_all(home).unwrap();
}

#[test]
fn gui_config_defaults_are_stable() {
    let config = GuiConfigFile::default();
    let content = toml::to_string_pretty(&config).unwrap();

    assert!(content.contains("port = 8317"));
    assert!(content.contains("allow-lan = false"));
    assert!(content.contains("run-on-startup = false"));
    assert!(content.contains("start-core-on-launch = true"));
    assert!(content.contains("silent-start = false"));
    assert!(content.contains("close-behavior = \"ask\""));
    assert!(content.contains("default-terminal = \"auto\""));
    assert_eq!(config.default_terminal, DEFAULT_AGENT_TERMINAL);
    assert!(content.contains("window-width = 1280"));
    assert!(content.contains("window-height = 800"));
    assert!(content.contains("auth-dir = \"../oauth\""));
    assert!(content.contains("[[api-keys]]"));
    assert!(content.contains("key = \"123456\""));
    assert!(content.contains("remark = \"Default key\""));
    assert!(content.contains(&format!("management-secret-key = \"{LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY}\"")));
    assert!(content.contains("plugins-enabled = false"));
    assert!(content.contains("routing-strategy = \"round-robin\""));
    assert!(content.contains("download-source = \"github\""));
    assert!(content.contains("prefer-gitcode-downloads = false"));
    assert!(content.contains("request-retry = 3"));
    assert!(content.contains("max-retry-credentials = 0"));
    assert!(content.contains("max-retry-interval = 30"));
    assert!(content.contains("streaming-bootstrap-retries = 0"));
}

#[test]
fn gui_window_size_round_trips_through_portable_config() {
    let config = GuiConfigFile {
        window_width: Some(1440),
        window_height: Some(900),
        ..GuiConfigFile::default()
    };

    let content = toml::to_string_pretty(&config).unwrap();
    let restored = toml::from_str::<GuiConfigFile>(&content).unwrap();

    assert!(content.contains("window-width = 1440"));
    assert!(content.contains("window-height = 900"));
    assert_eq!(
        configured_window_size(&restored),
        Some(SavedWindowSize {
            width: 1440,
            height: 900,
        })
    );
}

#[test]
fn silent_start_defaults_off_and_requires_tray_support() {
    let legacy = toml::from_str::<GuiConfigFile>("port = 8317\n").unwrap();
    assert!(legacy.start_core_on_launch);
    assert!(!legacy.silent_start);
    assert!(!should_start_hidden(&legacy));

    let enabled = GuiConfigFile {
        silent_start: true,
        ..GuiConfigFile::default()
    };
    assert_eq!(
        should_start_hidden(&enabled),
        cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        ))
    );
}

#[test]
fn core_start_on_launch_defaults_on_and_can_be_disabled() {
    let legacy = toml::from_str::<GuiConfigFile>("port = 8317\n").unwrap();
    assert!(should_start_core_on_launch(&legacy));

    let disabled = GuiConfigFile {
        start_core_on_launch: false,
        ..GuiConfigFile::default()
    };
    assert!(!should_start_core_on_launch(&disabled));
}

#[test]
fn gui_window_size_is_clamped_and_requires_both_dimensions() {
    let mut config = GuiConfigFile {
        window_width: Some(320),
        window_height: Some(30_000),
        ..GuiConfigFile::default()
    };

    assert!(sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.window_width, Some(MIN_MAIN_WINDOW_WIDTH));
    assert_eq!(config.window_height, Some(MAX_SAVED_WINDOW_DIMENSION));

    config.window_height = None;
    assert!(sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.window_width, None);
    assert_eq!(config.window_height, None);
}

#[test]
fn legacy_default_window_size_migrates_to_current_default() {
    let mut config = GuiConfigFile {
        window_width: Some(LEGACY_DEFAULT_MAIN_WINDOW_WIDTH),
        window_height: Some(LEGACY_DEFAULT_MAIN_WINDOW_HEIGHT),
        ..GuiConfigFile::default()
    };

    assert!(sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.window_width, Some(DEFAULT_MAIN_WINDOW_WIDTH));
    assert_eq!(config.window_height, Some(DEFAULT_MAIN_WINDOW_HEIGHT));
}

#[test]
fn physical_window_size_uses_display_scale_and_ignores_minimized_sizes() {
    let physical_size = tauri::PhysicalSize::new(1500, 1000);
    assert_eq!(
        logical_window_size_from_physical(&physical_size, 1.25),
        Some(SavedWindowSize {
            width: 1200,
            height: 800,
        })
    );
    assert!(logical_window_size_from_physical(&tauri::PhysicalSize::new(0, 0), 1.0).is_none());
    assert!(logical_window_size_from_physical(&physical_size, 0.0).is_none());
}
