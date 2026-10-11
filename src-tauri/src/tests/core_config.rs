use super::support::*;
use super::*;

#[test]
fn legacy_string_api_keys_keep_custom_keys_without_special_protection() {
    let legacy = "port = 8317\nallow-lan = false\nrun-on-startup = false\nauth-dir = \"/tmp/oauth\"\napi-keys = [\"123456\", \"custom-key\"]\nmanagement-secret-key = \"123456\"\nplugins-enabled = false\nrouting-strategy = \"round-robin\"\n";
    let mut config = toml::from_str::<GuiConfigFile>(legacy).unwrap();

    assert!(!sanitize_gui_config(&mut config).unwrap());
    assert_eq!(
        gui_api_key_values(&config.api_keys),
        vec!["123456", "custom-key"]
    );
    assert!(config.api_keys[0].remark.is_empty());
    assert!(config.api_keys[1].remark.is_empty());

    let serialized = toml::to_string_pretty(&config).unwrap();
    assert!(serialized.contains("[[api-keys]]"));
    let reparsed = toml::from_str::<GuiConfigFile>(&serialized).unwrap();
    assert_eq!(reparsed.api_keys, config.api_keys);
}

#[test]
fn api_key_remarks_follow_matching_core_keys() {
    let existing = vec![
        default_api_key_entry(),
        GuiApiKeyEntry {
            key: "custom-key".to_string(),
            remark: "Development environment".to_string(),
        },
    ];
    let core_keys = vec!["custom-key".to_string(), "new-key".to_string()];

    let merged = merge_core_api_keys_with_gui_metadata(&existing, &core_keys, None);

    assert_eq!(gui_api_key_values(&merged), vec!["custom-key", "new-key"]);
    assert_eq!(merged[0].remark, "Development environment");
    assert!(merged[1].remark.is_empty());
}

#[test]
fn explicit_empty_api_key_list_stays_empty() {
    let existing = vec![default_api_key_entry()];
    assert!(merge_core_api_keys_with_gui_metadata(&existing, &[], None).is_empty());

    let mut config = GuiConfigFile {
        api_keys: Vec::new(),
        ..GuiConfigFile::default()
    };
    sanitize_gui_config(&mut config).unwrap();
    assert!(config.api_keys.is_empty());

    let content = toml::to_string_pretty(&config).unwrap();
    let restored = toml::from_str::<GuiConfigFile>(&content).unwrap();
    assert!(restored.api_keys.is_empty());
}

#[test]
fn fresh_config_keeps_initial_default_unless_core_has_real_keys() {
    assert!(!should_import_core_api_keys(false, &[]));
    assert!(should_import_core_api_keys(
        false,
        &["existing-key".to_string()]
    ));
    assert!(should_import_core_api_keys(true, &[]));
}

#[test]
fn initial_default_api_key_can_be_edited_and_deleted() {
    let mut api_keys = vec![DEFAULT_API_KEY.to_string()];

    replace_core_api_key_value(&mut api_keys, DEFAULT_API_KEY, "custom-key".to_string()).unwrap();
    assert_eq!(api_keys, vec!["custom-key"]);

    remove_core_api_key_value(&mut api_keys, "custom-key").unwrap();
    assert!(api_keys.is_empty());
}

#[test]
fn core_config_view_exposes_api_key_metadata_for_the_webview() {
    let mut config = GuiConfigFile::default();
    ensure_strong_management_secret(&mut config).unwrap();
    let view = serde_json::to_value(CoreConfigView::from(&config)).unwrap();

    assert_eq!(view["apiKeys"][0]["apiKey"], DEFAULT_API_KEY);
    assert_eq!(view["apiKeys"][0]["remark"], DEFAULT_API_KEY_INITIAL_REMARK);
    assert!(view["apiKeys"][0].get("builtIn").is_none());
    assert_eq!(view["managementSecretConfigured"], true);
    assert_eq!(view["debug"], false);
    assert_eq!(view["commercialMode"], false);
    assert_eq!(view["loggingToFile"], false);
    assert_eq!(view["logsMaxTotalSizeMb"], 0);
    assert_eq!(view["errorLogsMaxFiles"], 10);
    assert_eq!(view["usageStatisticsEnabled"], true);
    assert_eq!(view["redisUsageQueueRetentionSeconds"], 60);
    assert_eq!(view["requestLog"], false);
    assert!(view.get("managementSecretKey").is_none());
}

#[test]
fn webui_management_secret_requires_a_non_empty_plaintext_value() {
    assert_eq!(
        normalize_management_secret_key("  new-webui-secret  ".to_string()).unwrap(),
        "new-webui-secret"
    );
    assert!(normalize_management_secret_key("   ".to_string()).is_err());
    assert!(normalize_management_secret_key(
        "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu".to_string()
    )
    .is_err());
    assert!(normalize_management_secret_key("bad\nsecret".to_string()).is_err());
    assert_eq!(
        normalize_management_secret_key("  123456  ".to_string()).unwrap(),
        "123456"
    );
}

#[test]
fn management_secret_rotation_preserves_disabled_and_custom_values() {
    let fresh = GuiConfigFile::default();
    assert_eq!(
        fresh.management_secret_key,
        LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY
    );
    assert!(!management_secret_requires_rotation(
        &fresh.management_secret_key
    ));

    let mut empty = GuiConfigFile {
        management_secret_key: String::new(),
        ..GuiConfigFile::default()
    };
    assert!(!ensure_strong_management_secret(&mut empty).unwrap());
    assert!(empty.management_secret_key.is_empty());
    assert!(validate_gui_config(&empty).is_ok());

    let mut legacy = GuiConfigFile {
        management_secret_key: LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string(),
        ..GuiConfigFile::default()
    };
    assert!(!ensure_strong_management_secret(&mut legacy).unwrap());
    assert_eq!(
        legacy.management_secret_key,
        LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY
    );

    let mut hashed = GuiConfigFile {
        management_secret_key: "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu"
            .to_string(),
        ..GuiConfigFile::default()
    };
    assert!(ensure_strong_management_secret(&mut hashed).unwrap());
    assert_eq!(hashed.management_secret_key, "123456");

    let mut custom = GuiConfigFile {
        management_secret_key: "user-selected-secret".to_string(),
        ..GuiConfigFile::default()
    };
    assert!(!ensure_strong_management_secret(&mut custom).unwrap());
    assert_eq!(custom.management_secret_key, "user-selected-secret");
}

#[test]
fn management_secret_recovery_synchronizes_kernel_and_request_credentials() {
    let hash = "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu";
    for (imported, saved, kernel_key, expected) in [
        ("", None, None, "123456"),
        (hash, None, None, "123456"),
        (hash, Some(hash), None, "123456"),
        ("bad\nkey", None, None, "123456"),
        (hash, Some("known-secret"), None, "known-secret"),
        ("", Some("known-secret"), None, "known-secret"),
        ("", Some(""), None, ""),
        ("", Some(""), Some(""), ""),
        ("", Some(""), Some(hash), "123456"),
        ("", Some(""), Some("kernel-secret"), "kernel-secret"),
        ("new-secret", Some("old-secret"), Some(hash), "new-secret"),
    ] {
        let config = GuiConfigFile {
            management_secret_key: recover_management_secret(imported, saved, kernel_key),
            ..GuiConfigFile::default()
        };
        assert_eq!(config.management_secret_key, expected);
        for (template, section) in [
            ("remote-management: {secret-key: stale}\n", "remote-management"),
            ("config-version: 8\nmanagement: {secret-key: stale}\n", "management"),
        ] {
            let updated = apply_gui_managed_settings(template, &config).unwrap();
            let document: serde_norway::Value = serde_norway::from_str(&updated).unwrap();
            assert_eq!(document[section]["secret-key"], expected);
        }
        if !expected.is_empty() {
            assert_eq!(management_api::management_authorization(&config).unwrap(), format!("Bearer {expected}"));
        }
    }
}

#[test]
fn management_secret_key_is_preserved_and_written_to_core() {
    let mut config = GuiConfigFile {
        management_secret_key: "old-management-secret".to_string(),
        ..GuiConfigFile::default()
    };

    assert!(!sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.management_secret_key, "old-management-secret");

    let template = "remote-management:\n  secret-key: stale-secret\n";
    let merged = merge_core_config_yaml(template, None, &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&merged).unwrap();
    assert_eq!(
        document["remote-management"]["secret-key"],
        "old-management-secret"
    );
}

#[test]
fn custom_auth_directory_is_preserved_and_written_to_core_config() {
    let mut config = GuiConfigFile {
        auth_dir: "/tmp/user-selected-auth".to_string(),
        ..GuiConfigFile::default()
    };
    ensure_strong_management_secret(&mut config).unwrap();

    assert!(validate_gui_config(&config).is_ok());
    assert!(!sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.auth_dir, "/tmp/user-selected-auth");

    let merged = merge_core_config_yaml("auth-dir: ~/.cli-proxy-api\n", None, &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&merged).unwrap();
    assert_eq!(document["auth-dir"], config.auth_dir);
}

#[test]
fn default_auth_directory_is_relative_and_legacy_absolute_value_is_migrated() {
    let base_dir = agent_test_home("relative-default-auth-dir");
    let install_dir = base_dir.join("cpa-core");
    assert_eq!(
        auth_dir_path_for_core(DEFAULT_AUTH_DIR, &install_dir).unwrap(),
        base_dir.join(OAUTH_DIR_NAME)
    );

    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&fixed_oauth_dir().unwrap()),
        auth_dir_user_selected: false,
        ..GuiConfigFile::default()
    };
    assert!(sanitize_gui_config(&mut config).unwrap());
    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    fs::remove_dir_all(base_dir).unwrap();
}

#[test]
fn path_configuration_expands_auth_home() {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let user_home = PathBuf::from(env::var_os(variable).unwrap());
    let install_dir = agent_test_home("auth-home-path").join("cpa-core");
    assert_eq!(
        auth_dir_path_for_core("~/.cli-proxy-api", &install_dir).unwrap(),
        user_home.join(".cli-proxy-api")
    );
    fs::remove_dir_all(install_dir.parent().unwrap()).unwrap();
}

#[test]
fn path_configuration_uses_existing_core_logs() {
    let root = agent_test_home("existing-core-logs");
    let install_dir = root.join("cpa-core");
    let logs_dir = install_dir.join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    fs::write(logs_dir.join("main.log"), b"existing-log").unwrap();
    assert_eq!(core_logs_dir_path_with_base(DEFAULT_AUTH_DIR, &install_dir, None).unwrap(), logs_dir);
    assert_eq!(fs::read(logs_dir.join("main.log")).unwrap(), b"existing-log");
    assert_eq!(fs::read_dir(&logs_dir).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_configuration_respects_log_base_and_fallbacks() {
    let root = agent_test_home("core-log-base");
    let install_dir = root.join("cpa-core");
    fs::create_dir_all(&install_dir).unwrap();
    let fallback = root.join("oauth").join("logs");
    assert_eq!(
        core_logs_dir_path_with_base(DEFAULT_AUTH_DIR, &install_dir, None).unwrap(),
        fallback
    );
    fs::write(install_dir.join("logs"), b"not a directory").unwrap();
    assert_eq!(
        core_logs_dir_path_with_base(DEFAULT_AUTH_DIR, &install_dir, None).unwrap(),
        fallback
    );
    fs::remove_file(install_dir.join("logs")).unwrap();
    fs::create_dir(install_dir.join("logs")).unwrap();
    for base in [" ../shared logs/./ ".to_string(), path_to_string(&root.join("shared logs"))] {
        assert_eq!(
            core_logs_dir_path_with_base(DEFAULT_AUTH_DIR, &install_dir, Some(&base)).unwrap(),
            root.join("shared logs").join("logs")
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_configuration_normalizes_auth_paths_and_rejects_missing_home() {
    let root = agent_test_home("core-auth-normalization");
    let install_dir = root.join("cpa-core");
    let user_home = root.join("home");
    for auth_dir in ["~/.cli-proxy-api", r"~\.cli-proxy-api", ""] {
        assert_eq!(
            auth_dir_path_for_core_with_home(auth_dir, &install_dir, Some(&user_home)).unwrap(),
            user_home.join(".cli-proxy-api")
        );
    }
    assert_eq!(
        auth_dir_path_for_core_with_home("~", &install_dir, Some(&user_home)).unwrap(),
        user_home
    );
    assert_eq!(
        auth_dir_path_for_core("missing/../oauth/./", &install_dir).unwrap(),
        install_dir.join("oauth")
    );
    assert!(auth_dir_path_for_core_with_home("~/oauth", &install_dir, None).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_configuration_writes_effective_tls_fields_without_version_marker() {
    let settings = CoreTlsSettings {
        enabled: true,
        cert: "certs/new.crt".to_string(),
        key: "certs/new.key".to_string(),
    };
    for input in [
        "server:\n  tls: {enable: true, cert: old.crt, key: old.key}\n",
        "server: {tls: {enable: false}}\n",
        "tls: {enable: false, cert: legacy.crt, key: legacy.key}\nserver: {tls: {enable: false}}\n",
    ] {
        let patched = patch_core_tls_settings_yaml(input, &settings).unwrap().unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&patched).unwrap();
        let effective = core_tls_settings_from_value(&document).unwrap();
        assert!(effective.enabled);
        assert_eq!(effective.cert, settings.cert);
        assert_eq!(effective.key, settings.key);
        if let Some(legacy) = document.get("tls") {
            assert_eq!(legacy["cert"].as_str(), Some(settings.cert.as_str()));
        }
        assert!(patch_core_tls_settings_yaml(&patched, &settings).unwrap().is_none());
    }
}

#[test]
fn tls_patch_preserves_null_legacy_fields_with_v8_settings() {
    let settings = CoreTlsSettings {
        enabled: false,
        cert: "certs/new.crt".to_string(),
        key: "certs/new.key".to_string(),
    };
    for version_marker in ["config-version: 8\n", ""] {
        let input = format!(
            "{version_marker}tls: null\nserver:\n  tls:\n    enable: true\n    cert: old.crt\n    key: old.key\n"
        );
        let patched = patch_core_tls_settings_yaml(&input, &settings)
            .unwrap()
            .expect("TLS settings should change");
        let document: serde_norway::Value = serde_norway::from_str(&patched).unwrap();
        assert_eq!(document.get("tls"), Some(&serde_norway::Value::Null));
        let effective = core_tls_settings_from_value(&document).unwrap();
        assert_eq!(effective.enabled, settings.enabled);
        assert_eq!(effective.cert, settings.cert);
        assert_eq!(effective.key, settings.key);
        assert!(patch_core_tls_settings_yaml(&patched, &settings)
            .unwrap()
            .is_none());
    }
}

#[test]
fn path_configuration_writes_effective_logging_fields_without_version_marker() {
    let config = GuiConfigFile {
        logging_to_file: true,
        logs_max_total_size_mb: 123,
        ..GuiConfigFile::default()
    };
    let settings = CoreConfigSettings::from(&config);
    for input in [
        "observability:\n  logs: {logging-to-file: false, logs-max-total-size-mb: 0}\n",
        "logging-to-file: false\nobservability:\n  logs: {logging-to-file: false}\n",
    ] {
        for patched in [
            apply_gui_managed_settings(input, &config).unwrap(),
            patch_core_yaml_document(input, |document| apply_core_logging_settings(document, &settings))
                .unwrap().unwrap(),
        ] {
            let document: serde_norway::Value = serde_norway::from_str(&patched).unwrap();
            let effective = core_config_settings_from_value(&document).unwrap();
            assert!(effective.logging_to_file);
            assert_eq!(effective.logs_max_total_size_mb, 123);
            if let Some(legacy) = document.get("logging-to-file") {
                assert_eq!(legacy.as_bool(), Some(true));
            }
        }
    }
}

#[test]
fn relative_oauth_dir_recovers_existing_persistent_credentials() {
    let root = agent_test_home("relative-oauth-dir");
    let install_dir = root.join("cpa-core");
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    let misplaced_auth_dir = install_dir.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    fs::create_dir_all(&misplaced_auth_dir).unwrap();
    fs::write(
        persistent_auth_dir.join("account.json"),
        b"existing-credential",
    )
    .unwrap();
    fs::write(misplaced_auth_dir.join("core-start-output.log"), b"log").unwrap();
    let mut config = GuiConfigFile {
        auth_dir: OAUTH_DIR_NAME.to_string(),
        ..GuiConfigFile::default()
    };

    assert!(
        recover_relative_oauth_dir(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    assert_eq!(
        auth_dir_path_for_core(&config.auth_dir, &install_dir).unwrap(),
        persistent_auth_dir
    );
    assert_eq!(
        fs::read(persistent_auth_dir.join("account.json")).unwrap(),
        b"existing-credential"
    );
    assert!(!misplaced_auth_dir.join("account.json").exists());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oauth_recovery_accepts_equivalent_directory_paths() {
    let root = agent_test_home("oauth-equivalent-paths");
    let install_dir = root.join("cpa-core");
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    fs::write(persistent_auth_dir.join("account.json"), b"credential").unwrap();
    for auth_dir in [
        "./oauth".to_string(),
        "oauth/./".to_string(),
        "../cpa-core/oauth".to_string(),
        path_to_string(&install_dir.join(OAUTH_DIR_NAME)),
        #[cfg(windows)]
        r".\oauth".to_string(),
        #[cfg(windows)]
        r"..\cpa-core\oauth".to_string(),
    ] {
        let mut config = GuiConfigFile {
            auth_dir: auth_dir.clone(),
            ..GuiConfigFile::default()
        };
        assert!(
            recover_relative_oauth_dir(&mut config, &install_dir, &persistent_auth_dir).unwrap(),
            "failed to recover {auth_dir}"
        );
        assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oauth_recovery_preserves_credentials_behind_parent_components() {
    let root = agent_test_home("oauth-parent-components");
    let install_dir = root.join("cpa-core");
    let active_auth_dir = install_dir.join(OAUTH_DIR_NAME);
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&active_auth_dir).unwrap();
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    fs::write(active_auth_dir.join("current.json"), b"current-credential").unwrap();
    fs::write(persistent_auth_dir.join("old.json"), b"old-credential").unwrap();
    assert!(!install_dir.join("missing").exists());

    for auth_dir in [
        "missing/../oauth".to_string(),
        path_to_string(&install_dir.join("missing").join("..").join(OAUTH_DIR_NAME)),
        #[cfg(windows)]
        r"missing\..\oauth".to_string(),
    ] {
        #[cfg(unix)]
        assert_eq!(
            fs::read_dir(install_dir.join(&auth_dir))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        let mut config = GuiConfigFile {
            auth_dir: auth_dir.clone(),
            ..GuiConfigFile::default()
        };
        assert!(
            !migrate_auth_dir_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
        );
        assert_eq!(config.auth_dir, auth_dir);
    }
    assert_eq!(
        fs::read(active_auth_dir.join("current.json")).unwrap(),
        b"current-credential"
    );
    assert_eq!(
        fs::read(persistent_auth_dir.join("old.json")).unwrap(),
        b"old-credential"
    );
    assert!(!install_dir.join("missing").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oauth_directory_write_updates_the_effective_v8_field_without_version_marker() {
    for input in [
        "oauth: {auth-dir: oauth}\n",
        "auth-dir: ../oauth\noauth: {auth-dir: oauth}\n",
    ] {
        let config = GuiConfigFile::default();
        let updated = apply_gui_managed_settings(input, &config).unwrap();
        let document = serde_norway::from_str(&updated).unwrap();
        assert_eq!(
            core_config_settings_from_value(&document).unwrap().auth_dir,
            DEFAULT_AUTH_DIR
        );
    }
}

#[test]
fn oauth_recovery_survives_config_reload_and_v8_upgrade() {
    for input in [
        "auth-dir: oauth\n",
        "config-version: 8\noauth:\n  auth-dir: oauth\n",
        "oauth:\n  auth-dir: ./oauth\n",
        "auth-dir: ../oauth\noauth: {auth-dir: oauth}\n",
    ] {
        let root = agent_test_home("oauth-recovery-reload");
        let install_dir = root.join("cpa-core");
        let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
        let core_config_path = install_dir.join(CORE_CONFIG_FILE);
        let gui_config_path = root.join(GUI_CONFIG_FILE);
        let credential_path = persistent_auth_dir.join("account.json");
        fs::create_dir_all(&install_dir).unwrap();
        fs::create_dir_all(&persistent_auth_dir).unwrap();
        fs::write(&credential_path, b"existing-credential").unwrap();
        fs::write(&core_config_path, format!("# user comment\n{input}")).unwrap();
        let original_document = serde_norway::from_str(input).unwrap();
        let mut config = GuiConfigFile {
            auth_dir: core_config_settings_from_value(&original_document)
                .unwrap()
                .auth_dir,
            ..GuiConfigFile::default()
        };
        ensure_strong_management_secret(&mut config).unwrap();
        write_gui_config_to_path(&config, &gui_config_path).unwrap();

        assert!(
            sanitize_gui_config_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
        );
        let patched = fs::read_to_string(&core_config_path).unwrap();
        assert!(patched.contains("# user comment"));
        let patched_document: serde_norway::Value = serde_norway::from_str(&patched).unwrap();
        assert_eq!(
            core_config_settings_from_value(&patched_document).unwrap().auth_dir,
            DEFAULT_AUTH_DIR
        );
        if let Some(legacy) = patched_document.get("auth-dir") {
            assert_eq!(legacy.as_str(), Some(DEFAULT_AUTH_DIR));
        }

        write_gui_config_to_path(&config, &gui_config_path).unwrap();
        let mut reloaded: GuiConfigFile =
            toml::from_str(&fs::read_to_string(&gui_config_path).unwrap()).unwrap();
        assert_eq!(reloaded.auth_dir, DEFAULT_AUTH_DIR);
        assert!(
            !sanitize_gui_config_at(&mut reloaded, &install_dir, &persistent_auth_dir).unwrap()
        );
        assert_eq!(fs::read_to_string(&core_config_path).unwrap(), patched);

        let staging_dir = root.join("next-core");
        fs::create_dir_all(&staging_dir).unwrap();
        fs::write(
            staging_dir.join(CORE_EXAMPLE_CONFIG_FILE),
            "config-version: 8\noauth:\n  auth-dir: oauth\n",
        )
        .unwrap();
        migrate_core_config_for_update(&install_dir, &staging_dir).unwrap();
        let upgraded = fs::read_to_string(staging_dir.join(CORE_CONFIG_FILE)).unwrap();
        let started = apply_gui_managed_settings(&upgraded, &reloaded).unwrap();
        let started_document = serde_norway::from_str(&started).unwrap();
        let effective = core_config_settings_from_value(&started_document).unwrap();
        assert_eq!(effective.auth_dir, DEFAULT_AUTH_DIR);
        assert_eq!(
            auth_dir_path_for_core(&effective.auth_dir, &install_dir).unwrap(),
            persistent_auth_dir
        );
        assert_eq!(fs::read(&credential_path).unwrap(), b"existing-credential");
        assert!(!install_dir.join(OAUTH_DIR_NAME).join("account.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn oauth_recovery_preserves_config_when_inspection_or_patch_fails() {
    for failure in ["invalid-yaml", "unreadable-config", "invalid-auth-directory"] {
        let root = agent_test_home("oauth-recovery-failure");
        let install_dir = root.join("cpa-core");
        let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
        let core_config_path = install_dir.join(CORE_CONFIG_FILE);
        fs::create_dir_all(&install_dir).unwrap();
        fs::create_dir_all(&persistent_auth_dir).unwrap();
        fs::write(persistent_auth_dir.join("account.json"), b"existing-credential").unwrap();
        match failure {
            "invalid-yaml" => fs::write(&core_config_path, "oauth: [\n").unwrap(),
            "unreadable-config" => fs::create_dir(&core_config_path).unwrap(),
            _ => {
                fs::write(&core_config_path, "auth-dir: oauth\n").unwrap();
                fs::write(install_dir.join(OAUTH_DIR_NAME), b"not a directory").unwrap();
            }
        }
        let mut config = GuiConfigFile {
            auth_dir: OAUTH_DIR_NAME.to_string(),
            port: 9123,
            ..GuiConfigFile::default()
        };
        let original_config = toml::to_string(&config).unwrap();
        let original_yaml = fs::read(&core_config_path).ok();

        assert!(migrate_auth_dir_at(&mut config, &install_dir, &persistent_auth_dir).is_err());
        assert_eq!(toml::to_string(&config).unwrap(), original_config);
        assert_eq!(fs::read(&core_config_path).ok(), original_yaml);
        assert!(
            !sanitize_gui_config_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
        );
        assert_eq!(toml::to_string(&config).unwrap(), original_config);
        assert_eq!(fs::read(&core_config_path).ok(), original_yaml);
        assert_eq!(
            fs::read(persistent_auth_dir.join("account.json")).unwrap(),
            b"existing-credential"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn oauth_recovery_requires_existing_persistent_credentials() {
    let root = agent_test_home("oauth-recovery-empty");
    let install_dir = root.join("cpa-core");
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    let mut config = GuiConfigFile {
        auth_dir: OAUTH_DIR_NAME.to_string(),
        ..GuiConfigFile::default()
    };
    assert!(
        !migrate_auth_dir_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    fs::create_dir_all(persistent_auth_dir.join("nested")).unwrap();
    fs::write(persistent_auth_dir.join("core.log"), b"log").unwrap();
    fs::write(persistent_auth_dir.join("nested").join("account.json"), b"nested").unwrap();
    assert!(
        !migrate_auth_dir_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, OAUTH_DIR_NAME);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn oauth_recovery_preserves_active_linked_credentials() {
    let root = agent_test_home("oauth-recovery-symlink");
    let install_dir = root.join("cpa-core");
    let active_auth_dir = install_dir.join(OAUTH_DIR_NAME);
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&active_auth_dir).unwrap();
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    let credential_path = persistent_auth_dir.join("account.json");
    fs::write(&credential_path, b"existing-credential").unwrap();
    std::os::unix::fs::symlink(&credential_path, active_auth_dir.join("linked.json")).unwrap();
    let mut config = GuiConfigFile {
        auth_dir: OAUTH_DIR_NAME.to_string(),
        ..GuiConfigFile::default()
    };
    assert!(
        !migrate_auth_dir_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, OAUTH_DIR_NAME);
    assert_eq!(fs::read(active_auth_dir.join("linked.json")).unwrap(), b"existing-credential");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_oauth_dir_preserves_populated_or_custom_paths() {
    let root = agent_test_home("relative-oauth-dir-custom");
    let install_dir = root.join("cpa-core");
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    let active_auth_dir = install_dir.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    fs::create_dir_all(&active_auth_dir).unwrap();
    fs::write(persistent_auth_dir.join("old.json"), b"old-credential").unwrap();
    fs::write(active_auth_dir.join("current.json"), b"current-credential").unwrap();
    let mut config = GuiConfigFile {
        auth_dir: OAUTH_DIR_NAME.to_string(),
        ..GuiConfigFile::default()
    };

    assert!(
        !recover_relative_oauth_dir(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, OAUTH_DIR_NAME);
    fs::remove_file(active_auth_dir.join("current.json")).unwrap();
    config.auth_dir = "other/oauth".to_string();
    assert!(
        !recover_relative_oauth_dir(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, "other/oauth");

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(not(target_os = "macos"))]
fn oauth_recovery_preserves_app_named_custom_directories_on_other_platforms() {
    let root = agent_test_home("oauth-custom-app-directory");
    let install_dir = root.join("cpa-core");
    let persistent_auth_dir = root.join(OAUTH_DIR_NAME);
    let custom_auth_dir = root.join("Custom.app").join("Contents").join(OAUTH_DIR_NAME);
    fs::create_dir_all(&custom_auth_dir).unwrap();
    fs::create_dir_all(&persistent_auth_dir).unwrap();
    fs::write(custom_auth_dir.join("current.json"), b"current-credential").unwrap();
    fs::write(persistent_auth_dir.join("old.json"), b"old-credential").unwrap();
    let configured_auth_dir = path_to_string(&custom_auth_dir);
    let mut config = GuiConfigFile {
        auth_dir: configured_auth_dir.clone(),
        ..GuiConfigFile::default()
    };

    assert!(
        !sanitize_gui_config_at(&mut config, &install_dir, &persistent_auth_dir).unwrap()
    );
    assert_eq!(config.auth_dir, configured_auth_dir);
    assert!(!persistent_auth_dir.join("current.json").exists());
    assert_eq!(
        fs::read(custom_auth_dir.join("current.json")).unwrap(),
        b"current-credential"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packaged_macos_auth_directory_is_copied_before_config_is_repointed() {
    let root = agent_test_home("packaged-macos-auth-migration");
    let contents_dir = root.join("EasyCLIProxyAPI.app").join("Contents");
    let source = contents_dir.join("MacOS").join("oauth");
    let configured_source = contents_dir
        .join("MacOS")
        .join("unused")
        .join("..")
        .join("oauth");
    let persistent_root = root
        .join("Library")
        .join("Application Support")
        .join("com.cpa.gui");
    let destination = persistent_root.join(OAUTH_DIR_NAME);
    let install_dir = persistent_root.join("cpa-core");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("account.json"), b"oauth-account").unwrap();
    fs::write(source.join("nested").join("token.json"), b"oauth-token").unwrap();
    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&configured_source),
        ..GuiConfigFile::default()
    };

    assert!(auth_dir_is_inside_macos_app_bundle(&configured_source));
    assert!(
        migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination,).unwrap()
    );

    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    assert_eq!(
        fs::read(destination.join("account.json")).unwrap(),
        b"oauth-account"
    );
    assert_eq!(
        fs::read(destination.join("nested").join("token.json")).unwrap(),
        b"oauth-token"
    );
    assert_eq!(
        fs::read(source.join("account.json")).unwrap(),
        b"oauth-account"
    );

    config.auth_dir = path_to_string(&configured_source);
    assert!(
        migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination).unwrap()
    );
    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packaged_macos_auth_migration_recovers_the_replaced_app_backup() {
    let root = agent_test_home("macos-auth-update-backup");
    let app = root.join("Applications/EasyCLIProxyAPI.app");
    let relative = Path::new("Contents/MacOS/oauth");
    let source = app.join(relative);
    let backup = root.join("Applications/.EasyCLIProxyAPI.app.update-backup");
    let old_auth = backup.join(relative);
    let persistent = root.join("Library/Application Support/com.cpa.gui");
    let install_dir = persistent.join("cpa-core");
    let destination = persistent.join(OAUTH_DIR_NAME);
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::create_dir_all(old_auth.join("logs/archive")).unwrap();
    fs::write(old_auth.join("account.json"), b"existing-credential").unwrap();
    fs::write(old_auth.join("logs/archive/core.log"), b"historical-log").unwrap();
    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&source),
        ..GuiConfigFile::default()
    };

    assert!(
        migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination).unwrap()
    );
    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    assert_eq!(fs::read(destination.join("account.json")).unwrap(), b"existing-credential");
    assert_eq!(fs::read(destination.join("logs/archive/core.log")).unwrap(), b"historical-log");
    assert_eq!(fs::read(old_auth.join("account.json")).unwrap(), b"existing-credential");

    config.auth_dir = path_to_string(&source);
    fs::write(destination.join("account.json"), b"different-credential").unwrap();
    let error = migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination)
        .unwrap_err();
    assert!(error.contains("not overwritten"), "{error}");
    assert_eq!(config.auth_dir, path_to_string(&source));
    assert_eq!(fs::read(destination.join("account.json")).unwrap(), b"different-credential");
    assert_eq!(fs::read(old_auth.join("account.json")).unwrap(), b"existing-credential");

    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("account.json"), b"different-credential").unwrap();
    assert!(
        migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination).unwrap()
    );
    assert_eq!(fs::read(destination.join("account.json")).unwrap(), b"different-credential");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packaged_macos_auth_migration_recovers_backup_when_new_directory_is_empty() {
    let root = agent_test_home("macos-auth-empty-update-directory");
    let relative = Path::new("Contents/MacOS/oauth");
    let source = root.join("Applications/EasyCLIProxyAPI.app").join(relative);
    let backup = root.join("Applications/.EasyCLIProxyAPI.app.update-backup").join(relative);
    let persistent = root.join("Library/Application Support/com.cpa.gui");
    let install_dir = persistent.join("cpa-core");
    let destination = persistent.join(OAUTH_DIR_NAME);
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&backup).unwrap();
    fs::write(backup.join("account.json"), b"existing-credential").unwrap();
    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&source),
        ..GuiConfigFile::default()
    };

    assert!(migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination).unwrap());
    assert_eq!(fs::read(destination.join("account.json")).unwrap(), b"existing-credential");
    assert_eq!(fs::read(backup.join("account.json")).unwrap(), b"existing-credential");
    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packaged_macos_auth_migration_does_not_overwrite_conflicting_credentials() {
    let root = agent_test_home("packaged-macos-auth-conflict");
    let source = root
        .join("EasyCLIProxyAPI.app")
        .join("Contents")
        .join("MacOS")
        .join("oauth");
    let persistent_root = root.join("persistent");
    let destination = persistent_root.join(OAUTH_DIR_NAME);
    let install_dir = persistent_root.join("cpa-core");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("account.json"), b"old-app-credential").unwrap();
    fs::write(destination.join("account.json"), b"persistent-credential").unwrap();
    let original_auth_dir = path_to_string(&source);
    let mut config = GuiConfigFile {
        auth_dir: original_auth_dir.clone(),
        ..GuiConfigFile::default()
    };

    let error = migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination)
        .unwrap_err();

    assert!(error.contains("not overwritten"), "{error}");
    assert_eq!(config.auth_dir, original_auth_dir);
    assert_eq!(
        fs::read(destination.join("account.json")).unwrap(),
        b"persistent-credential"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_packaged_macos_auth_directory_is_repointed_to_persistent_storage() {
    let root = agent_test_home("missing-packaged-macos-auth");
    let source = root
        .join("EasyCLIProxyAPI.app")
        .join("Contents")
        .join("MacOS")
        .join("oauth");
    let persistent_root = root.join("persistent");
    let destination = persistent_root.join(OAUTH_DIR_NAME);
    let install_dir = persistent_root.join("cpa-core");
    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&source),
        ..GuiConfigFile::default()
    };

    assert!(
        migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination,).unwrap()
    );

    assert_eq!(config.auth_dir, DEFAULT_AUTH_DIR);
    assert!(destination.is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_custom_auth_directory_is_not_migrated() {
    let root = agent_test_home("external-custom-auth");
    let install_dir = root.join("cpa-core");
    let destination = root.join(OAUTH_DIR_NAME);
    let custom = root.join("custom-auth");
    let mut config = GuiConfigFile {
        auth_dir: path_to_string(&custom),
        ..GuiConfigFile::default()
    };

    assert!(
        !migrate_auth_dir_from_macos_app_bundle(&mut config, &install_dir, &destination,).unwrap()
    );
    assert_eq!(config.auth_dir, path_to_string(&custom));
    assert!(!destination.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_gui_config_can_seed_managed_core_settings() {
    let legacy = "port: 8317\nallow-lan: false\nrun-on-startup: true\n";
    let mut config = serde_yaml::from_str::<GuiConfigFile>(legacy).unwrap();
    let presence = serde_yaml::from_str::<GuiConfigPresence>(legacy).unwrap();
    let core_settings = CoreConfigSettings {
        host: "0.0.0.0".to_string(),
        port: 9000,
        auth_dir: "/tmp/external-auth".to_string(),
        api_keys: vec!["existing-key".to_string()],
        management_secret_configured: true,
        debug: true,
        commercial_mode: true,
        logging_to_file: true,
        logs_max_total_size_mb: 256,
        error_logs_max_files: 24,
        usage_statistics_enabled: false,
        redis_usage_queue_retention_seconds: 120,
        request_log: true,
        plugins_enabled: true,
        routing_strategy: "fill-first".to_string(),
        proxy_url: "http://127.0.0.1:8080".to_string(),
        routing_session_affinity: true,
        routing_session_affinity_ttl: "45m".to_string(),
        disable_cooling: true,
        request_retry: 1,
        max_retry_credentials: 2,
        max_retry_interval: 5,
        streaming_bootstrap_retries: 1,
        management_secret_key: Some("management-secret".to_string()),
    };

    assert!(presence.api_keys.is_none());
    assert!(presence.management_secret_key.is_none());
    assert!(presence.debug.is_none());
    assert!(presence.commercial_mode.is_none());
    assert!(presence.logging_to_file.is_none());
    assert!(presence.logs_max_total_size_mb.is_none());
    assert!(presence.error_logs_max_files.is_none());
    assert!(presence.redis_usage_queue_retention_seconds.is_none());
    assert!(presence.request_log.is_none());
    assert!(presence.plugins_enabled.is_none());
    assert!(presence.routing_strategy.is_none());
    apply_core_settings_to_gui_config(&mut config, &core_settings);

    assert_eq!(gui_api_key_values(&config.api_keys), vec!["existing-key"]);
    assert_eq!(config.management_secret_key, "management-secret");
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 9000);
    assert!(config.disable_cooling);
    assert_eq!(config.auth_dir, "/tmp/external-auth");
    assert!(config.debug);
    assert!(config.commercial_mode);
    assert!(config.logging_to_file);
    assert_eq!(config.logs_max_total_size_mb, 256);
    assert_eq!(config.error_logs_max_files, 24);
    assert!(!config.usage_statistics_enabled);
    assert_eq!(config.redis_usage_queue_retention_seconds, 120);
    assert!(config.request_log);
    assert!(config.plugins_enabled);
    assert_eq!(config.routing_strategy, "fill-first");
    assert_eq!(config.proxy_url, "");
    assert!(config.routing_session_affinity);
    assert_eq!(config.routing_session_affinity_ttl, "45m");
    assert_eq!(config.request_retry, 1);
    assert_eq!(config.max_retry_credentials, 2);
    assert_eq!(config.max_retry_interval, 5);
    assert_eq!(config.streaming_bootstrap_retries, 1);
    assert!(config.run_on_startup);
}

#[test]
fn external_core_proxy_changes_become_manual_overrides() {
    let settings_with_proxy = |proxy_url: &str| {
        let input = format!("proxy-url: \"{proxy_url}\"\n");
        let document = serde_norway::from_str::<serde_norway::Value>(&input).unwrap();
        core_config_settings_from_value(&document).unwrap()
    };

    let detected_url = "http://127.0.0.1:7890";
    let mut unchanged = GuiConfigFile {
        proxy_url: detected_url.to_string(),
        proxy_override: false,
        ..GuiConfigFile::default()
    };
    apply_external_core_proxy_override(&mut unchanged, &settings_with_proxy(detected_url)).unwrap();
    assert!(!unchanged.proxy_override);

    let mut custom = unchanged.clone();
    apply_external_core_proxy_override(
        &mut custom,
        &settings_with_proxy("socks5://127.0.0.1:1080"),
    )
    .unwrap();
    assert!(custom.proxy_override);
    assert_eq!(custom.proxy_url, "socks5://127.0.0.1:1080");

    let mut direct = unchanged;
    apply_external_core_proxy_override(&mut direct, &settings_with_proxy("")).unwrap();
    assert!(direct.proxy_override);
    assert!(direct.proxy_url.is_empty());
    ensure_strong_management_secret(&mut direct).unwrap();
    assert!(validate_gui_config(&direct).is_ok());
}

#[test]
fn example_api_keys_are_not_persisted_as_gui_settings() {
    let input = "api-keys:\n  - your-api-key-1\n  - real-key\nremote-management:\n  secret-key: plain-management-secret\nplugins:\n  enabled: true\nrouting:\n  strategy: fill-first\n";
    let document = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let core_settings = core_config_settings_from_value(&document).unwrap();
    let mut config = GuiConfigFile::default();

    apply_core_settings_to_gui_config(&mut config, &core_settings);

    assert_eq!(core_settings.api_keys, vec!["real-key"]);
    assert_eq!(gui_api_key_values(&config.api_keys), vec!["real-key"]);
    assert_eq!(
        core_settings.management_secret_key.as_deref(),
        Some("plain-management-secret")
    );
    assert_eq!(config.management_secret_key, "plain-management-secret");
    assert!(validate_core_api_key("your-api-key-3").is_err());
}

#[test]
fn hashed_management_secret_is_detected_without_replacing_known_plaintext() {
    let input = "remote-management:\n  secret-key: $2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu\n";
    let document = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let core_settings = core_config_settings_from_value(&document).unwrap();

    assert!(core_settings
        .management_secret_key
        .as_deref()
        .is_some_and(is_hashed_management_secret_key));
    assert!(core_settings.management_secret_configured);
    let mut config = GuiConfigFile {
        management_secret_key: "known-plaintext".to_string(),
        ..GuiConfigFile::default()
    };
    apply_core_settings_to_gui_config(&mut config, &core_settings);
    assert_eq!(config.management_secret_key, "known-plaintext");
}

#[test]
fn management_api_requires_an_available_plaintext_secret() {
    let mut config = GuiConfigFile {
        management_secret_key: String::new(),
        ..GuiConfigFile::default()
    };
    assert!(management_authorization(&config).is_err());

    config.management_secret_key =
        "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu".to_string();
    assert!(management_authorization(&config).is_err());

    config.management_secret_key = "known-plaintext".to_string();
    assert_eq!(
        management_authorization(&config).unwrap(),
        "Bearer known-plaintext"
    );
}

#[test]
fn runtime_network_patch_preserves_comments_and_other_settings() {
    let config = GuiConfigFile {
        locale: "zh-CN".to_string(),
        port: 9527,
        allow_lan: true,
        host: "0.0.0.0".to_string(),
        run_on_startup: false,
        ..GuiConfigFile::default()
    };
    let input = "# Bind address\nhost: 127.0.0.1 # local only\n\n# Service port\nport: 8317 # default\ndebug: true\n";
    let updated = patch_core_network_yaml(input, &config)
        .unwrap()
        .expect("network settings should change");

    assert_eq!(
            updated,
            "# Bind address\nhost: 0.0.0.0 # local only\n\n# Service port\nport: 9527 # default\ndebug: true\n"
        );
    assert!(updated.contains("# Bind address"));
    assert!(updated.contains("# local only"));
    assert!(updated.contains("# Service port"));
    assert!(updated.contains("# default"));
    assert!(updated.contains("debug: true"));

    let document = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(
        document["host"],
        serde_norway::Value::String("0.0.0.0".to_string())
    );
    assert_eq!(document["port"], serde_norway::to_value(9527_u16).unwrap());
}

#[test]
fn runtime_network_patch_skips_unchanged_yaml() {
    let config = GuiConfigFile::default();
    let input = "host: 127.0.0.1\nport: 8317\n";

    assert!(patch_core_network_yaml(input, &config).unwrap().is_none());
}

#[test]
fn confirmed_network_routing_patch_updates_all_fields_together() {
    let config = GuiConfigFile {
        port: 9527,
        allow_lan: true,
        host: "0.0.0.0".to_string(),
        proxy_url: "socks5://127.0.0.1:7890".to_string(),
        routing_session_affinity: true,
        routing_session_affinity_ttl: "2h".to_string(),
        disable_cooling: true,
        request_retry: 1,
        max_retry_credentials: 2,
        max_retry_interval: 5,
        streaming_bootstrap_retries: 1,
        ..GuiConfigFile::default()
    };
    let input = "# network\nhost: 127.0.0.1\nport: 8317\nproxy-url: \"\"\n# routing\nrouting:\n  strategy: round-robin\n# unrelated\ndebug: true\n";
    let updated = patch_core_network_routing_yaml(input, &config)
        .unwrap()
        .expect("confirmed settings should change");
    let document = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();

    assert_eq!(document["host"], "0.0.0.0");
    assert_eq!(document["port"], 9527);
    assert_eq!(document["proxy-url"], "socks5://127.0.0.1:7890");
    assert_eq!(document["routing"]["session-affinity"], true);
    assert_eq!(document["routing"]["session-affinity-ttl"], "2h");
    assert_eq!(document["disable-cooling"], true);
    assert_eq!(document["request-retry"], 1);
    assert_eq!(document["max-retry-credentials"], 2);
    assert_eq!(document["max-retry-interval"], 5);
    assert_eq!(document["streaming"]["bootstrap-retries"], 1);
    assert_eq!(document["routing"]["strategy"], "round-robin");
    assert_eq!(document["debug"], true);
    assert!(updated.contains("# network"));
    assert!(updated.contains("# routing"));
    assert!(updated.contains("# unrelated"));
}

#[test]
fn independent_network_modules_only_patch_their_own_fields() {
    let config = GuiConfigFile {
        host: "192.168.1.20".to_string(),
        port: 9527,
        proxy_url: "socks5://127.0.0.1:7890".to_string(),
        routing_session_affinity: true,
        routing_session_affinity_ttl: "2h".to_string(),
        disable_cooling: true,
        request_retry: 7,
        max_retry_credentials: 8,
        max_retry_interval: 9,
        streaming_bootstrap_retries: 10,
        ..GuiConfigFile::default()
    };
    let input = "host: 127.0.0.1\nport: 8317\nproxy-url: \"\"\nrouting:\n  session-affinity: false\n  session-affinity-ttl: 1h\ndisable-cooling: false\nrequest-retry: 1\nmax-retry-credentials: 2\nmax-retry-interval: 3\nstreaming:\n  bootstrap-retries: 4\n";

    let network = patch_core_network_endpoint_yaml(input, &config)
        .unwrap()
        .expect("network endpoint should change");
    let network = serde_norway::from_str::<serde_norway::Value>(&network).unwrap();
    assert_eq!(network["host"], "192.168.1.20");
    assert_eq!(network["port"], 9527);
    assert_eq!(network["proxy-url"], "socks5://127.0.0.1:7890");
    assert_eq!(network["disable-cooling"], false);
    assert_eq!(network["request-retry"], 1);
    assert_eq!(network["routing"]["session-affinity"], false);

    let retry = patch_core_retry_yaml(input, &config)
        .unwrap()
        .expect("retry settings should change");
    let retry = serde_norway::from_str::<serde_norway::Value>(&retry).unwrap();
    assert_eq!(retry["disable-cooling"], true);
    assert_eq!(retry["request-retry"], 7);
    assert_eq!(retry["max-retry-credentials"], 8);
    assert_eq!(retry["max-retry-interval"], 9);
    assert_eq!(retry["streaming"]["bootstrap-retries"], 10);
    assert_eq!(retry["host"], "127.0.0.1");

    let routing = patch_core_session_routing_yaml(input, &config)
        .unwrap()
        .expect("session routing should change");
    let routing = serde_norway::from_str::<serde_norway::Value>(&routing).unwrap();
    assert_eq!(routing["routing"]["session-affinity"], true);
    assert_eq!(routing["routing"]["session-affinity-ttl"], "2h");
    assert_eq!(routing["request-retry"], 1);
}

#[test]
fn core_config_controls_preserve_comments_and_unrelated_values() {
    let input = "# Client authentication\napi-keys:\n  - old-key\n\n# Plugin runtime\nplugins:\n  enabled: false # global switch\n  dir: plugins\n\n# Credential routing\nrouting:\n  strategy: round-robin # current strategy\n  session-affinity: true\n\ndebug: true # untouched\n";
    let mut document = yaml_serde_edit::YamlValue::parse(input).unwrap();
    let mut updated = document.get().clone();

    set_core_api_keys(
        &mut updated,
        vec!["old-key".to_string(), "new-key".to_string()],
    )
    .unwrap();
    set_nested_yaml_value(&mut updated, &["plugins", "enabled"], true).unwrap();
    set_nested_yaml_value(
        &mut updated,
        &["routing", "strategy"],
        "fill-first".to_string(),
    )
    .unwrap();
    document.set(updated);

    let rendered = document.get_string();
    assert!(rendered.contains("# Client authentication"));
    assert!(rendered.contains("# Plugin runtime"));
    assert!(rendered.contains("# global switch"));
    assert!(rendered.contains("# Credential routing"));
    assert!(rendered.contains("# current strategy"));
    assert!(rendered.contains("debug: true # untouched"));

    let settings = core_config_settings_from_value(document.get()).unwrap();
    assert_eq!(settings.api_keys, vec!["old-key", "new-key"]);
    assert!(settings.plugins_enabled);
    assert_eq!(settings.routing_strategy, "fill-first");
    assert_eq!(document.get()["plugins"]["dir"], "plugins");
    assert_eq!(document.get()["routing"]["session-affinity"], true);
}

#[test]
fn yaml_edit_runtime_patches_supported_fields_without_reflowing_yaml() {
    let input = "# Client authentication\napi-keys:\n  - old-key\n\n# Plugin runtime\nplugins:\n  enabled: false # global switch\n  dir: plugins\n\n# Credential routing\nrouting:\n  strategy: round-robin # current strategy\n  session-affinity: true\n\ndebug: true # untouched\n";
    let file = input.parse::<yaml_edit::YamlFile>().unwrap();
    let document = file.document().unwrap();

    assert!(set_yaml_edit_nested_value(
        &document, "plugins", "enabled", true
    ));
    assert!(set_yaml_edit_nested_value(
        &document,
        "routing",
        "strategy",
        "fill-first".to_string()
    ));

    let rendered = patch_core_api_keys_yaml(
        &file.to_string(),
        &["new-key".to_string(), "backup-key".to_string()],
    )
    .unwrap();
    assert!(rendered.contains("# Client authentication"));
    assert!(rendered.contains("# Plugin runtime"));
    assert!(rendered.contains("# global switch"));
    assert!(rendered.contains("# Credential routing"));
    assert!(rendered.contains("# current strategy"));
    assert!(rendered.contains("debug: true # untouched"));
    assert!(rendered.contains("dir: plugins"));
    assert!(rendered.contains("session-affinity: true"));

    let settings =
        core_config_settings_from_value(&serde_norway::from_str(&rendered).unwrap()).unwrap();
    assert_eq!(settings.api_keys, vec!["new-key", "backup-key"]);
    assert!(settings.plugins_enabled);
    assert_eq!(settings.routing_strategy, "fill-first");
}

#[test]
fn yaml_edit_sequence_shrink_keeps_following_top_level_key_valid() {
    let input = "# API keys for authentication\napi-keys:\n  - first-key\n  - second-key\n  - third-key\n\n# Enable debug logging\ndebug: false\n";
    let original = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let mut updated = original.clone();
    set_core_api_keys(&mut updated, vec!["first-key".to_string()]).unwrap();

    let rendered = render_yaml_value_changes(input, &original, &updated)
        .unwrap_or_else(|error| panic!("sequence shrink failed: {error}"));
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{rendered}"));

    assert_eq!(parsed["api-keys"][0], "first-key");
    assert_eq!(parsed["api-keys"].as_sequence().unwrap().len(), 1);
    assert_eq!(parsed["debug"], false);
    assert!(rendered.find("api-keys:").unwrap() < rendered.find("debug:").unwrap());
}

#[test]
fn runtime_yaml_ast_patch_handles_core_comments_around_nested_mapping() {
    let input = "host: 127.0.0.1\nremote-management:\n# Whether to allow remote access.\n  allow-remote: false\n# Management key.\n# All requests require this key.\n  secret-key: old\n# Disable panel.\n  disable-control-panel: false\nauth-dir: /tmp/old\napi-keys:\n  - old-key\n";
    let rendered = patch_core_yaml_document(input, |document| {
        let auth_changed = set_core_yaml_auth_dir(document, "/tmp/new")?;
        let secret_changed = set_core_yaml_nested_value(
            document,
            "remote-management",
            "secret-key",
            serde_norway::Value::String("123456".to_string()),
        )?;
        Ok(auth_changed || secret_changed)
    })
    .unwrap()
    .unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{rendered}"));

    assert_eq!(parsed["auth-dir"], "/tmp/new");
    assert_eq!(parsed["remote-management"]["secret-key"], "123456");
    assert!(rendered.contains("# All requests require this key."));
    assert!(rendered.contains("disable-control-panel: false"));
}

#[test]
fn yaml_edit_runtime_patch_removes_empty_keys_and_skips_unsupported_sections() {
    let input = "# Client authentication\napi-keys:\n  - old-key\nplugins:\n  enabled: true\nrouting:\n  strategy: fill-first\n";
    let rendered = patch_core_api_keys_yaml(input, &[]).unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered).unwrap();
    let root = parsed.as_mapping().unwrap();
    assert!(yaml_mapping_value(root, "api-keys").is_none(), "{rendered}");
    assert_eq!(
        core_config_settings_from_value(&parsed).unwrap().api_keys,
        Vec::<String>::new()
    );

    let missing_api_keys = "host: 127.0.0.1\nport: 8317\n";
    let rendered = patch_core_api_keys_yaml(
        missing_api_keys,
        &["new-key".to_string(), "backup-key".to_string()],
    )
    .unwrap();
    let settings =
        core_config_settings_from_value(&serde_norway::from_str(&rendered).unwrap()).unwrap();
    assert_eq!(settings.api_keys, vec!["new-key", "backup-key"]);
    assert!(rendered.contains("host: 127.0.0.1"));
    assert!(rendered.contains("port: 8317"));

    let unsupported = "host: 127.0.0.1\nport: 8317\n";
    let file = unsupported.parse::<yaml_edit::YamlFile>().unwrap();
    let document = file.document().unwrap();
    assert!(!set_yaml_edit_nested_value(
        &document, "plugins", "enabled", true
    ));
    assert!(!set_yaml_edit_nested_value(
        &document,
        "routing",
        "strategy",
        "fill-first".to_string()
    ));
    assert_eq!(file.to_string(), unsupported);
}

#[test]
fn yaml_edit_runtime_patch_recreates_api_keys_after_delete_all() {
    let input = "# Client authentication\napi-keys:\n  - old-key\nplugins:\n  enabled: true\n";
    let cleared = patch_core_api_keys_yaml(input, &[]).unwrap();
    let rendered = patch_core_api_keys_yaml(&cleared, &["restored-key".to_string()]).unwrap();
    assert!(rendered.contains("# Client authentication"), "{rendered}");
    let settings =
        core_config_settings_from_value(&serde_norway::from_str(&rendered).unwrap()).unwrap();
    assert_eq!(settings.api_keys, vec!["restored-key"]);
}

#[test]
fn yaml_edit_runtime_patch_adds_api_keys_to_core_style_config() {
    let input = "host: 127.0.0.1\nremote-management:\n# nested setting comment\n  allow-remote: false\nauth-dir: /tmp/oauth\n# API keys for authentication\n# Enable debug logging\ndebug: false\n\n# Optional payload configuration\n# payload:\n#   filter:\n#     - models:\n#         - name: \"gemini-2.5-pro\"\n#       params:\n#         - \"generationConfig.responseJsonSchema\"\n";
    let rendered =
        patch_core_api_keys_yaml(input, &["new-key".to_string(), "backup-key".to_string()])
            .unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{rendered}"));
    let settings = core_config_settings_from_value(&parsed).unwrap();
    assert_eq!(settings.api_keys, vec!["new-key", "backup-key"]);
    assert!(rendered.contains("# Optional payload configuration"));
    assert!(rendered.contains("generationConfig.responseJsonSchema"));
    assert!(rendered.find("auth-dir: /tmp/oauth").unwrap() < rendered.find("api-keys:").unwrap());
    assert!(rendered.find("api-keys:").unwrap() < rendered.find("debug: false").unwrap());
}

#[test]
fn yaml_edit_runtime_patch_updates_existing_real_core_config() {
    let input = "host: 0.0.0.0\nremote-management:\n# nested comment\n  allow-remote: false\nauth-dir: /tmp/oauth\n# API keys for authentication\napi-keys:\n  - '123456'\n# Enable debug logging\ndebug: false\n\n# payload:\n#   filter:\n#     - models:\n#         - name: gemini\n";
    let rendered =
        patch_core_api_keys_yaml(input, &[DEFAULT_API_KEY.to_string(), "new-key".to_string()])
            .unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{rendered}"));
    assert_eq!(
        core_config_settings_from_value(&parsed).unwrap().api_keys,
        vec![DEFAULT_API_KEY, "new-key"]
    );
}

#[test]
fn runtime_api_key_patch_replaces_indentationless_core_sequence() {
    let input =
        "host: 0.0.0.0\nport: 8317\nauth-dir: /tmp/oauth\napi-keys:\n- '123456'\ndebug: false\n";
    let rendered = patch_core_api_keys_yaml(input, &[DEFAULT_API_KEY.to_string()])
        .unwrap_or_else(|error| panic!("patch failed: {error}"));
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{rendered}"));

    assert_eq!(
        core_config_settings_from_value(&parsed).unwrap().api_keys,
        vec![DEFAULT_API_KEY]
    );
    assert_eq!(rendered.matches("- '123456'").count(), 1, "{rendered}");
    assert!(rendered.contains("debug: false"), "{rendered}");
}

#[test]
fn yaml_edit_runtime_patch_migrates_legacy_api_key_entries() {
    let input = "auth:\n  providers:\n    config-api-key:\n      api-key-entries:\n        - api-key: first-key\n        - key: second-key\nplugins:\n  enabled: false\n";
    let rendered = patch_core_api_keys_yaml(input, &["migrated-key".to_string()]).unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&rendered).unwrap();
    let settings = core_config_settings_from_value(&parsed).unwrap();
    assert_eq!(settings.api_keys, vec!["migrated-key"]);
    assert!(
        nested_yaml_value(
            parsed.as_mapping().unwrap(),
            &["auth", "providers", "config-api-key", "api-key-entries"]
        )
        .is_none(),
        "{rendered}"
    );
}

#[test]
fn core_config_reads_legacy_api_key_entries() {
    let input = "auth:\n  providers:\n    config-api-key:\n      api-key-entries:\n        - api-key: first-key\n        - key: second-key\nplugins:\n  enabled: false\nrouting:\n  strategy: round-robin\n";
    let document = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let settings = core_config_settings_from_value(&document).unwrap();

    assert_eq!(settings.api_keys, vec!["first-key", "second-key"]);
    assert!(!settings.plugins_enabled);
    assert_eq!(settings.routing_strategy, "round-robin");
}

#[test]
fn core_config_reads_logging_settings_and_applies_defaults() {
    let configured = serde_norway::from_str::<serde_norway::Value>(
        "debug: true\ncommercial-mode: true\nlogging-to-file: true\nlogs-max-total-size-mb: 512\nerror-logs-max-files: 25\nusage-statistics-enabled: false\nredis-usage-queue-retention-seconds: 7200\nrequest-log: true\n",
    )
    .unwrap();
    let settings = core_config_settings_from_value(&configured).unwrap();

    assert!(settings.debug);
    assert!(settings.commercial_mode);
    assert!(settings.logging_to_file);
    assert_eq!(settings.logs_max_total_size_mb, 512);
    assert_eq!(settings.error_logs_max_files, 25);
    assert!(!settings.usage_statistics_enabled);
    assert_eq!(settings.redis_usage_queue_retention_seconds, 3600);
    assert!(settings.request_log);

    let defaults = core_config_settings_from_value(&serde_norway::from_str("{}").unwrap()).unwrap();
    assert!(!defaults.debug);
    assert!(!defaults.commercial_mode);
    assert!(!defaults.logging_to_file);
    assert_eq!(
        defaults.logs_max_total_size_mb,
        DEFAULT_LOGS_MAX_TOTAL_SIZE_MB
    );
    assert_eq!(defaults.error_logs_max_files, DEFAULT_ERROR_LOGS_MAX_FILES);
    assert!(!defaults.usage_statistics_enabled);
    assert_eq!(
        defaults.redis_usage_queue_retention_seconds,
        DEFAULT_REDIS_USAGE_QUEUE_RETENTION_SECONDS
    );
    assert!(!defaults.request_log);

    let zero_retention =
        serde_norway::from_str::<serde_norway::Value>("redis-usage-queue-retention-seconds: 0\n")
            .unwrap();
    assert_eq!(
        core_config_settings_from_value(&zero_retention)
            .unwrap()
            .redis_usage_queue_retention_seconds,
        DEFAULT_REDIS_USAGE_QUEUE_RETENTION_SECONDS
    );
}

#[test]
fn v8_core_config_reads_and_writes_canonical_nested_fields() {
    let input = "config-version: 8\nserver:\n  host: 0.0.0.0\n  port: 9527\n  commercial-mode: true\n  tls: {enable: true, cert: cert.pem, key: key.pem}\nmanagement: {secret-key: management-key}\naccess: {api-keys: [client-key]}\noauth: {auth-dir: oauth-data}\nrequests:\n  proxy-url: direct\n  streaming: {bootstrap-retries: 4}\nrouting:\n  strategy: fill-first\n  retry: {request-retry: 5, max-retry-credentials: 6, max-retry-interval: 7}\n  cooldown: {disable-cooling: true}\nobservability:\n  logs: {debug: true, logging-to-file: true, logs-max-total-size-mb: 128, error-logs-max-files: 12, request-log: true}\n  usage: {usage-statistics-enabled: false, redis-usage-queue-retention-seconds: 90}\nplugins: {enabled: true}\n";
    let document = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let settings = core_config_settings_from_value(&document).unwrap();
    assert_eq!(settings.host, "0.0.0.0");
    assert_eq!(settings.port, 9527);
    assert_eq!(settings.api_keys, vec!["client-key"]);
    assert_eq!(settings.management_secret_key.as_deref(), Some("management-key"));
    assert_eq!(settings.proxy_url, "direct");
    assert_eq!(settings.request_retry, 5);
    assert_eq!(settings.max_retry_credentials, 6);
    assert_eq!(settings.max_retry_interval, 7);
    assert_eq!(settings.streaming_bootstrap_retries, 4);
    assert!(settings.debug && settings.commercial_mode && settings.request_log);

    let mut config = GuiConfigFile::default();
    config.host = "127.0.0.1".into();
    config.port = 8318;
    config.management_secret_key = "new-management-key".into();
    config.api_keys = vec![GuiApiKeyEntry { key: "new-client-key".into(), remark: String::new() }];
    config.proxy_url = "socks5://127.0.0.1:1080".into();
    config.request_retry = 2;
    let updated = apply_gui_managed_settings(input, &config).unwrap();
    let updated = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(updated["server"]["host"], "127.0.0.1");
    assert_eq!(updated["server"]["port"], 8318);
    assert_eq!(updated["management"]["secret-key"], "new-management-key");
    assert_eq!(updated["access"]["api-keys"][0], "new-client-key");
    assert_eq!(updated["requests"]["proxy-url"], "socks5://127.0.0.1:1080");
    assert_eq!(updated["routing"]["retry"]["request-retry"], 2);
    for legacy in ["host", "port", "remote-management", "proxy-url", "request-retry"] {
        assert!(updated.get(legacy).is_none(), "legacy field leaked into v8 config: {legacy}");
    }
}

#[test]
fn v8_inline_management_hash_is_preserved_without_duplicate_section() {
    let input = "config-version: 8\nserver: {host: 127.0.0.1, port: 8317}\nmanagement: {secret-key: $2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu}\naccess: {api-keys: [client-key]}\nrequests: {proxy-url: direct}\n";
    let mut config = GuiConfigFile::default();
    config.management_secret_key = "plaintext-management-key".into();
    let updated = apply_gui_managed_settings(input, &config)
        .unwrap_or_else(|error| panic!("management key update failed: {error}"));
    let parsed = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(parsed["management"]["secret-key"], "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu");
}

#[test]
fn v8_multiline_management_hash_with_comments_is_preserved() {
    let input = "# generated by CLIProxyAPI\n# keep this comment\nconfig-version: 8\nserver:\n  host: 127.0.0.1\n  port: 8317\nmanagement:\n  secret-key: $2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu\n  allow-remote: false\naccess:\n  api-keys:\n    - client-key\nrequests:\n  proxy-url: direct\n";
    let mut config = GuiConfigFile::default();
    config.management_secret_key = "plaintext-management-key".into();
    let updated = apply_gui_managed_settings(input, &config)
        .unwrap_or_else(|error| panic!("management key update failed: {error}"));
    let parsed = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(parsed["management"]["secret-key"], "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu");
    assert_eq!(parsed["management"]["allow-remote"], false);
}

#[test]
fn gui_start_preserves_hashed_v8_management_key() {
    let hash = "$2a$10$abcdefghijklmnopqrstuuuuuuuuuuuuuuuuuuuuuuuuuuuuu";
    let input = format!(
        "config-version: 8\nserver: {{host: 127.0.0.1, port: 8317}}\nmanagement: {{secret-key: {hash}}}\naccess: {{api-keys: [client-key]}}\nrequests: {{proxy-url: direct}}\n"
    );
    let mut config = GuiConfigFile::default();
    config.management_secret_key = "plaintext-management-key".into();
    let updated = apply_gui_managed_settings(&input, &config).unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(parsed["management"]["secret-key"], hash);
    assert!(!updated.contains("plaintext-management-key"));
}

#[test]
fn v8_omitted_fields_use_runtime_defaults_instead_of_gui_presets() {
    let settings = core_config_settings_from_value(
        &serde_norway::from_str("config-version: 8\nserver: {port: 8317}\n").unwrap(),
    ).unwrap();
    assert_eq!(settings.host, "");
    assert!(!settings.usage_statistics_enabled);
    assert_eq!(settings.request_retry, 0);
    assert_eq!(settings.max_retry_interval, 0);
}

#[test]
fn v8_settings_can_populate_null_sections_without_losing_siblings() {
    let input = "config-version: 8\nserver: null\nmanagement: null\naccess: null\noauth: null\nrequests: null\nrouting: null\nobservability: null\nplugins: null\napi-keys: {codex: [{name: keep, keys: [{api-key: test-key}]}]}\n";
    let updated = apply_gui_managed_settings(input, &GuiConfigFile::default()).unwrap();
    let document: serde_norway::Value = serde_norway::from_str(&updated).unwrap();
    assert_eq!(document["server"]["port"], 8317);
    assert_eq!(document["api-keys"]["codex"][0]["name"], "keep");
    assert!(apply_gui_managed_settings("config-version: 8\nrequests: broken\n", &GuiConfigFile::default()).is_err());
}

#[test]
fn v8_sensitive_words_fall_back_per_field_and_clear_legacy_values() {
    let input = "config-version: 8\nantigravity: {sensitive-words: [old]}\ndevin: {sensitive-words: [old-devin]}\noauth:\n  providers:\n    antigravity: {signature-cache-enabled: true}\n    devin: {sensitive-words: [new-devin]}\n";
    let document: serde_norway::Value = serde_norway::from_str(input).unwrap();
    let settings = core_sensitive_words_settings_from_value(&document).unwrap();
    assert_eq!(settings.antigravity_sensitive_words, vec!["old"]);
    assert_eq!(settings.devin_sensitive_words, vec!["new-devin"]);
    let updated = patch_core_sensitive_words_yaml(input, &CoreSensitiveWordsSettings::default()).unwrap().unwrap();
    let document: serde_norway::Value = serde_norway::from_str(&updated).unwrap();
    for path in [vec!["antigravity"], vec!["devin"], vec!["oauth", "providers", "antigravity"], vec!["oauth", "providers", "devin"]] {
        let section = nested_yaml_value(document.as_mapping().unwrap(), &path).unwrap();
        assert_eq!(section["sensitive-words"].as_sequence().unwrap().len(), 0);
    }
    assert_eq!(document["oauth"]["providers"]["antigravity"]["signature-cache-enabled"], true);
}

#[test]
fn v8_tls_reads_mixed_layout_per_field() {
    let document = serde_norway::from_str("config-version: 8\ntls: {enable: true, cert: old-cert, key: old-key}\nserver: {tls: {cert: new-cert}}\n").unwrap();
    let settings = core_tls_settings_from_value(&document).unwrap();
    assert!(settings.enabled);
    assert_eq!(settings.cert, "new-cert");
    assert_eq!(settings.key, "old-key");
}

#[test]
fn session_ttl_rejects_values_that_the_core_silently_ignores() {
    for value in ["", "30m", "1h30m", "+2h", "1.5s", ".5h", "1.s"] {
        assert_eq!(normalize_session_affinity_ttl(format!(" {value} ")).unwrap(), value);
    }
    for value in ["500ms", "1ns", "2us", "2µs", "2μs"] {
        assert_eq!(normalize_session_affinity_ttl(value.into()).unwrap(), "1s");
    }
    for value in ["0", "0s", "-1h", "1d", "30", "1h 30m", "1s junk", "+", "NaNh", "0.1ns", "999999999999h"] {
        assert!(normalize_session_affinity_ttl(value.into()).is_err(), "accepted {value}");
    }
}

#[test]
fn v8_startup_without_version_marker_preserves_upstream_credentials() {
    let input = "server: {port: 9527}\noauth: {auth-dir: ../oauth}\naccess: {api-keys: [old-client-key]}\napi-keys:\n  codex:\n    - name: custom-provider\n      base-url: https://example.invalid/v1\n      keys: [{api-key: upstream-secret}]\n";
    let original = serde_norway::from_str::<serde_norway::Value>(input).unwrap();
    let config = GuiConfigFile {
        api_keys: vec![GuiApiKeyEntry { key: "new-client-key".into(), remark: String::new() }],
        management_secret_key: "management-secret".into(),
        ..GuiConfigFile::default()
    };
    for keys in [config.api_keys.clone(), Vec::new()] {
        let config = GuiConfigFile { api_keys: keys, ..config.clone() };
        let updated = apply_gui_managed_settings(input, &config).unwrap();
        let updated = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
        assert_eq!(updated["api-keys"], original["api-keys"], "upstream credentials must not be replaced by client keys");
        let effective = core_config_settings_from_value(&updated).unwrap();
        assert_eq!(effective.api_keys, gui_api_key_values(&config.api_keys));
        assert_eq!(effective.auth_dir, "../oauth");
    }
}

#[test]
fn core_config_reads_proxy_and_session_affinity_fields() {
    let canonical = serde_norway::from_str::<serde_norway::Value>(
            "proxy-url: socks5://127.0.0.1:7890\nrouting:\n  session-affinity: true\n  session-affinity-ttl: 2h\n",
        )
        .unwrap();
    let settings = core_config_settings_from_value(&canonical).unwrap();
    assert_eq!(settings.proxy_url, "socks5://127.0.0.1:7890");
    assert!(settings.routing_session_affinity);
    assert_eq!(settings.routing_session_affinity_ttl, "2h");

    let aliases = serde_norway::from_str::<serde_norway::Value>(
        "routing:\n  sessionAffinity: true\n  sessionAffinityTTL: 30m\n",
    )
    .unwrap();
    let settings = core_config_settings_from_value(&aliases).unwrap();
    assert!(settings.routing_session_affinity);
    assert_eq!(settings.routing_session_affinity_ttl, "30m");

    let defaults = core_config_settings_from_value(&serde_norway::from_str("{}").unwrap()).unwrap();
    assert_eq!(defaults.proxy_url, "");
    assert!(!defaults.routing_session_affinity);
    assert_eq!(defaults.routing_session_affinity_ttl, "");
}

#[test]
fn core_config_reads_retry_fields_and_uses_core_defaults() {
    let document = serde_norway::from_str::<serde_norway::Value>(
        "disable-cooling: true\nrequest-retry: 1\nmax-retry-credentials: 2\nmax-retry-interval: 5\nstreaming:\n  bootstrap-retries: 4\n",
    )
    .unwrap();
    let settings = core_config_settings_from_value(&document).unwrap();

    assert!(settings.disable_cooling);
    assert_eq!(settings.request_retry, 1);
    assert_eq!(settings.max_retry_credentials, 2);
    assert_eq!(settings.max_retry_interval, 5);
    assert_eq!(settings.streaming_bootstrap_retries, 4);

    let defaults = core_config_settings_from_value(&serde_norway::from_str("{}").unwrap()).unwrap();
    assert_eq!(defaults.disable_cooling, DEFAULT_DISABLE_COOLING);
    assert_eq!(defaults.request_retry, 0);
    assert_eq!(
        defaults.max_retry_credentials,
        DEFAULT_MAX_RETRY_CREDENTIALS
    );
    assert_eq!(defaults.max_retry_interval, 0);
    assert_eq!(
        defaults.streaming_bootstrap_retries,
        DEFAULT_STREAMING_BOOTSTRAP_RETRIES
    );
}

#[test]
fn managed_session_settings_use_canonical_yaml_and_preserve_unrelated_content() {
    let input = "# global proxy\nproxy-url: old\n# routing options\nrouting:\n  strategy: round-robin\n  sessionAffinity: false\n  sessionAffinityTTL: 10m\n# unrelated option\nunrelated-option: true\ndebug: true\n";
    let config = GuiConfigFile {
        proxy_url: "http://127.0.0.1:8080".to_string(),
        routing_session_affinity: true,
        routing_session_affinity_ttl: "1h".to_string(),
        disable_cooling: true,
        request_retry: 1,
        max_retry_credentials: 2,
        max_retry_interval: 5,
        streaming_bootstrap_retries: 1,
        ..GuiConfigFile::default()
    };
    let rendered = apply_gui_managed_settings(input, &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&rendered).unwrap();

    assert_eq!(document["proxy-url"], "http://127.0.0.1:8080");
    assert_eq!(document["routing"]["session-affinity"], true);
    assert_eq!(document["routing"]["session-affinity-ttl"], "1h");
    assert_eq!(document["disable-cooling"], true);
    assert_eq!(document["request-retry"], 1);
    assert_eq!(document["max-retry-credentials"], 2);
    assert_eq!(document["max-retry-interval"], 5);
    assert_eq!(document["streaming"]["bootstrap-retries"], 1);
    assert!(rendered.contains("# global proxy"));
    assert!(rendered.contains("# unrelated option"));
    assert_eq!(document["unrelated-option"], true);
    assert_eq!(document["debug"], false);
}

#[test]
fn optional_core_strings_are_trimmed_and_reject_control_characters() {
    assert_eq!(
        normalize_optional_config_string("  socks5://proxy:7890  ".to_string(), "Proxy URL")
            .unwrap(),
        "socks5://proxy:7890"
    );
    assert_eq!(
        normalize_optional_config_string(" 1h ".to_string(), "TTL").unwrap(),
        "1h"
    );
    assert!(normalize_optional_config_string("bad\nvalue".to_string(), "Proxy URL").is_err());
}

#[test]
fn core_tls_settings_read_defaults_and_configured_values() {
    let defaults = serde_norway::from_str::<serde_norway::Value>("host: 127.0.0.1\n").unwrap();
    let configured = serde_norway::from_str::<serde_norway::Value>(
        "tls:\n  enable: true\n  cert: C:/certs/server.crt\n  key: C:/certs/server.key\n",
    )
    .unwrap();

    let defaults = core_tls_settings_from_value(&defaults).unwrap();
    assert!(!defaults.enabled);
    assert!(defaults.cert.is_empty());
    assert!(defaults.key.is_empty());

    let configured = core_tls_settings_from_value(&configured).unwrap();
    assert!(configured.enabled);
    assert_eq!(configured.cert, "C:/certs/server.crt");
    assert_eq!(configured.key, "C:/certs/server.key");
}

#[test]
fn tls_patch_preserves_unrelated_yaml_and_paths_when_disabled() {
    let input = "# server\nhost: 127.0.0.1\ntls:\n  enable: true\n  cert: old.crt\n  key: old.key\ncustom:\n  keep: true\n";
    let settings = CoreTlsSettings {
        enabled: false,
        cert: "new.crt".to_string(),
        key: "new.key".to_string(),
    };
    let patched = patch_core_tls_settings_yaml(input, &settings)
        .unwrap()
        .expect("TLS settings should change");
    let document = serde_norway::from_str::<serde_norway::Value>(&patched).unwrap();

    assert!(patched.contains("# server"));
    assert_eq!(document["tls"]["enable"], false);
    assert_eq!(document["tls"]["cert"], "new.crt");
    assert_eq!(document["tls"]["key"], "new.key");
    assert_eq!(document["custom"]["keep"], true);
}

#[test]
fn sensitive_words_patch_preserves_other_config_and_clears_each_provider_independently() {
    let input = "# keep this comment\nantigravity:\n  sensitive-words:\n    - old\n  signature-cache: true\ndevin:\n  sensitive-words:\n    - old-devin\ncustom:\n  keep: true\n";
    let settings = normalize_core_sensitive_words_settings(CoreSensitiveWordsSettings {
        antigravity_sensitive_words: vec!["  Hermes  ".into(), "".into(), "Nous Research".into()],
        devin_sensitive_words: Vec::new(),
    });
    let patched = patch_core_sensitive_words_yaml(input, &settings)
        .unwrap()
        .expect("sensitive words should change");
    let document = serde_norway::from_str::<serde_norway::Value>(&patched).unwrap();
    assert!(patched.contains("# keep this comment"));
    assert_eq!(document["custom"]["keep"], true);
    assert_eq!(document["antigravity"]["signature-cache"], true);
    assert_eq!(
        document["antigravity"]["sensitive-words"],
        serde_norway::to_value(vec!["Hermes", "Nous Research"]).unwrap()
    );
    assert_eq!(
        document["devin"]["sensitive-words"],
        serde_norway::to_value(Vec::<String>::new()).unwrap()
    );
    assert_eq!(
        core_sensitive_words_settings_from_value(&document).unwrap(),
        settings
    );
    assert!(patch_core_sensitive_words_yaml(&patched, &settings)
        .unwrap()
        .is_none());
}

#[test]
fn sensitive_words_patch_adds_missing_sections_without_touching_unrelated_fields() {
    let input = "# config\ndebug: true\n";
    let empty = CoreSensitiveWordsSettings::default();
    assert!(patch_core_sensitive_words_yaml(input, &empty)
        .unwrap()
        .is_none());

    let settings = CoreSensitiveWordsSettings {
        antigravity_sensitive_words: vec!["proxy".into()],
        devin_sensitive_words: vec!["Claude Code".into()],
    };
    let patched = patch_core_sensitive_words_yaml(input, &settings)
        .unwrap()
        .expect("missing sections should be created");
    let document = serde_norway::from_str::<serde_norway::Value>(&patched).unwrap();
    assert!(patched.contains("# config"));
    assert_eq!(document["debug"], true);
    assert_eq!(
        core_sensitive_words_settings_from_value(&document).unwrap(),
        settings
    );

    let invalid =
        serde_norway::from_str::<serde_norway::Value>("devin:\n  sensitive-words: not-a-list\n")
            .unwrap();
    assert!(core_sensitive_words_settings_from_value(&invalid).is_err());
    let empty =
        serde_norway::from_str::<serde_norway::Value>("antigravity:\n  sensitive-words:\ndevin:\n")
            .unwrap();
    assert_eq!(
        core_sensitive_words_settings_from_value(&empty).unwrap(),
        CoreSensitiveWordsSettings::default()
    );
    let updated =
        patch_core_sensitive_words_yaml("antigravity:\n  sensitive-words:\ndevin:\n", &settings)
            .unwrap()
            .unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&updated).unwrap();
    assert_eq!(
        core_sensitive_words_settings_from_value(&parsed).unwrap(),
        settings
    );
    let only_devin = CoreSensitiveWordsSettings {
        antigravity_sensitive_words: Vec::new(),
        devin_sensitive_words: vec!["another phrase".into()],
    };
    let patched =
        patch_core_sensitive_words_yaml("antigravity:\n  sensitive-words:\ndevin:\n", &only_devin)
            .unwrap()
            .unwrap();
    let parsed = serde_norway::from_str::<serde_norway::Value>(&patched).unwrap();
    assert_eq!(
        core_sensitive_words_settings_from_value(&parsed).unwrap(),
        only_devin
    );
    assert!(parsed["antigravity"]["sensitive-words"].is_null());
}

#[test]
fn enabled_tls_requires_both_paths() {
    let missing_key = CoreTlsSettings {
        enabled: true,
        cert: "server.crt".to_string(),
        key: String::new(),
    };
    assert!(normalize_core_tls_settings(missing_key).is_err());

    let disabled = CoreTlsSettings {
        enabled: false,
        cert: "  server.crt  ".to_string(),
        key: String::new(),
    };
    let disabled = normalize_core_tls_settings(disabled).unwrap();
    assert_eq!(disabled.cert, "server.crt");
    assert!(disabled.key.is_empty());
}

#[test]
fn core_loopback_origin_uses_the_selected_transport() {
    assert_eq!(core_loopback_origin(8317, false), "http://127.0.0.1:8317");
    assert_eq!(core_loopback_origin(9527, true), "https://127.0.0.1:9527");
    assert!(is_managed_agent_base_url("https://127.0.0.1:9527/v1"));
    assert!(is_managed_agent_base_url("https://localhost:9527"));
}

#[test]
fn core_origin_uses_connectable_custom_and_ipv6_hosts() {
    assert_eq!(
        core_origin("192.168.1.20", 9527, true),
        "https://192.168.1.20:9527"
    );
    assert_eq!(core_origin("0.0.0.0", 8317, false), "http://127.0.0.1:8317");
    assert_eq!(core_origin("::", 8317, false), "http://[::1]:8317");
    assert_eq!(
        core_origin("2001:db8::1", 8317, true),
        "https://[2001:db8::1]:8317"
    );
}

#[test]
fn configured_proxy_supports_http_and_socks5_urls() {
    for proxy_url in ["http://127.0.0.1:8080", "socks5://127.0.0.1:7890"] {
        apply_configured_proxy(reqwest::Client::builder(), proxy_url)
            .unwrap()
            .build()
            .unwrap();
    }
}

#[test]
fn core_config_validates_keys_and_routing_strategy() {
    assert!(validate_core_api_key("sk-valid_123").is_ok());
    assert!(validate_core_api_key("").is_err());
    assert!(validate_core_api_key("contains space").is_err());
    assert!(validate_routing_strategy("round-robin").is_ok());
    assert!(validate_routing_strategy("weighted-round-robin").is_ok());
    assert!(validate_routing_strategy("fill-first").is_ok());
    assert!(validate_routing_strategy("random").is_err());
}

#[test]
fn unchanged_yaml_is_not_written_again() {
    let path = std::env::temp_dir().join(format!(
        "cpa-gui-unchanged-yaml-{}-{}.yaml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let content = "host: 127.0.0.1\nport: 8317\n";
    fs::write(&path, content).unwrap();

    assert!(!write_yaml_if_changed(&path, content).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), content);

    fs::remove_file(path).unwrap();
}

#[test]
fn core_config_saves_preserve_the_file_watched_by_the_running_core() {
    let root = agent_test_home("core-config-watched-file");
    let path = root.join(CORE_CONFIG_FILE);
    fs::write(&path, "auth-dir: old-directory\nusage-statistics-enabled: false\n").unwrap();
    let mut watched_file = File::open(&path).unwrap();

    for directory in ["first-directory", "x"] {
        patch_core_auth_dir_at(&path, directory).unwrap();
        watched_file.seek(SeekFrom::Start(0)).unwrap();
        let mut watched_content = String::new();
        watched_file.read_to_string(&mut watched_content).unwrap();
        assert_eq!(watched_content, fs::read_to_string(&path).unwrap());
        let document = serde_norway::from_str(&watched_content).unwrap();
        let settings = core_config_settings_from_value(&document).unwrap();
        assert_eq!(settings.auth_dir, directory);
        assert!(!settings.usage_statistics_enabled);
    }
    drop(watched_file);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_preserves_all_user_owned_yaml_and_only_applies_gui_managed_values() {
    let template = "# Current release template\nhost: \"\" # template bind address\nport: 8317\n\n# Client authentication\napi-keys:\n  - template-key\n\n# Plugin runtime\nplugins:\n  enabled: false # plugin switch\n\n# Credential routing\nrouting:\n  strategy: round-robin # routing switch\n\n# New release option\nnew-option: true\nnested:\n  # Nested template comment\n  keep: template\n  added: from-template\nlist:\n  - template-item\n";
    let current = "# User-edited configuration\nhost: 127.0.0.1\nport: 9000\n# User-owned nested values\nnested:\n  keep: current\n  current-only: retained\nlist:\n  - current-a\n  - current-b\nextra: true\ncustom-provider:\n  base-url: https://example.com/v1\n  headers:\n    X-Custom-Header: custom-value\n  models:\n    - name: custom-model\n      aliases: [custom-a, custom-b]\nplugins:\n  custom-runtime-options:\n    sandbox: strict\n    environment:\n      CUSTOM_FLAG: enabled\nrouting:\n  custom-rules:\n    - match: custom-*\n      target: custom-provider\nremote-management:\n  custom-dashboard-option: retained\n";
    let config = GuiConfigFile {
        locale: "zh-CN".to_string(),
        port: 9527,
        allow_lan: true,
        host: "0.0.0.0".to_string(),
        run_on_startup: false,
        start_core_on_launch: true,
        silent_start: false,
        close_behavior: WindowsCloseBehavior::Ask,
        default_terminal: DEFAULT_AGENT_TERMINAL.to_string(),
        window_width: None,
        window_height: None,
        auth_dir: path_to_string(&fixed_oauth_dir().unwrap()),
        auth_dir_user_selected: false,
        api_keys: vec![
            default_api_key_entry(),
            GuiApiKeyEntry {
                key: "gui-key".to_string(),
                remark: "Test key".to_string(),
            },
        ],
        api_access_remarks: Vec::new(),
        management_secret_key: String::new(),
        debug: true,
        commercial_mode: true,
        logging_to_file: true,
        logs_max_total_size_mb: 512,
        error_logs_max_files: 25,
        usage_statistics_enabled: false,
        redis_usage_queue_retention_seconds: 180,
        usage_statistics_disabled: Some(true),
        request_log: true,
        plugins_enabled: true,
        routing_strategy: "fill-first".to_string(),
        proxy_url: "socks5://127.0.0.1:7890".to_string(),
        proxy_override: true,
        download_source: VersionDownloadSource::Github,
        custom_download_mirrors: Vec::new(),
        active_custom_download_mirror: String::new(),
        prefer_gitcode_downloads: false,
        routing_session_affinity: true,
        routing_session_affinity_ttl: "1h".to_string(),
        disable_cooling: true,
        request_retry: 1,
        max_retry_credentials: 2,
        max_retry_interval: 5,
        streaming_bootstrap_retries: 1,
    };
    let merged = merge_core_config_yaml(template, Some(current), &config).unwrap();
    let current_document = serde_norway::from_str::<serde_norway::Value>(current).unwrap();

    assert!(merged.contains("# User-edited configuration"));
    assert!(merged.contains("# User-owned nested values"));
    assert!(!merged.contains("# Current release template"));
    assert!(!merged.contains("# New release option"));

    let document = serde_norway::from_str::<serde_norway::Value>(&merged).unwrap();
    assert_eq!(
        document["host"],
        serde_norway::Value::String("0.0.0.0".to_string())
    );
    assert_eq!(document["port"], serde_norway::to_value(9527_u16).unwrap());
    assert_eq!(document["api-keys"][0], DEFAULT_API_KEY);
    assert_eq!(document["api-keys"][1], "gui-key");
    assert_eq!(document["plugins"]["enabled"], true, "{merged}");
    assert_eq!(document["routing"]["strategy"], "fill-first");
    assert_eq!(document["proxy-url"], "socks5://127.0.0.1:7890");
    assert_eq!(document["routing"]["session-affinity"], true);
    assert_eq!(document["routing"]["session-affinity-ttl"], "1h");
    assert_eq!(document["disable-cooling"], true);
    assert_eq!(document["request-retry"], 1);
    assert_eq!(document["max-retry-credentials"], 2);
    assert_eq!(document["max-retry-interval"], 5);
    assert_eq!(document["streaming"]["bootstrap-retries"], 1);
    assert_eq!(document["debug"], true);
    assert_eq!(document["commercial-mode"], true);
    assert_eq!(document["logging-to-file"], true);
    assert_eq!(document["logs-max-total-size-mb"], 512);
    assert_eq!(document["error-logs-max-files"], 25);
    assert_eq!(document["usage-statistics-enabled"], false);
    assert_eq!(document["redis-usage-queue-retention-seconds"], 180);
    assert_eq!(document["request-log"], true);
    assert!(document.get("new-option").is_none());
    assert_eq!(document["nested"], current_document["nested"]);
    assert!(document["nested"].get("added").is_none());
    for key in ["list", "extra", "custom-provider"] {
        assert_eq!(document[key], current_document[key], "user field {key}");
    }
    assert_eq!(
        document["plugins"]["custom-runtime-options"],
        current_document["plugins"]["custom-runtime-options"]
    );
    assert_eq!(
        document["routing"]["custom-rules"],
        current_document["routing"]["custom-rules"]
    );
    assert_eq!(
        document["remote-management"]["custom-dashboard-option"],
        current_document["remote-management"]["custom-dashboard-option"]
    );
}

#[test]
fn startup_merge_preserves_plugin_store_config_written_by_core() {
    let template = "host: \"\"\nport: 8317\nauth-dir: ~/.cli-proxy-api\napi-keys:\n  - template-key\nremote-management:\n  secret-key: \"\"\nusage-statistics-enabled: true\nplugins:\n  enabled: false\n  dir: plugins\n  configs:\n    example:\n      enabled: true\n      priority: 1\nrouting:\n  strategy: round-robin\n  session-affinity: false\n  session-affinity-ttl: \"\"\nproxy-url: \"\"\ncommercial-mode: false\n";
    let current = "host: 127.0.0.1\nport: 8317\nauth-dir: ~/.cli-proxy-api\napi-keys:\n  - '123456'\nremote-management:\n  secret-key: \"\"\nusage-statistics-enabled: true\nplugins:\n  enabled: true\n  dir: plugins\n  configs:\n    model-fallback-router:\n      enabled: true\n      store:\n        id: model-fallback-router\n        name: Model Fallback Router\n        description: Retries matching model requests through configured fallback model names when the primary model fails with quota, rate-limit, transport, or configured HTTP status errors.\n        author: thebtf\n        version: 0.2.0\n        release-tag: v0.2.0\n        repository: https://github.com/thebtf/cpa-model-fallback-router\n        tags:\n          - Router\n          - Model Router\n          - Fallback\n        install:\n          type: github-release\nrouting:\n  strategy: round-robin\n  session-affinity: false\n  session-affinity-ttl: \"\"\nproxy-url: \"\"\n";
    let config = GuiConfigFile {
        host: "0.0.0.0".to_string(),
        allow_lan: true,
        plugins_enabled: true,
        ..GuiConfigFile::default()
    };

    let merged = merge_core_config_yaml(template, Some(current), &config)
        .unwrap_or_else(|error| panic!("plugin config merge failed: {error}"));
    let document = serde_norway::from_str::<serde_norway::Value>(&merged).unwrap();
    let current_document = serde_norway::from_str::<serde_norway::Value>(current).unwrap();

    assert_eq!(document["host"], "0.0.0.0");
    assert_eq!(
        document["plugins"]["configs"]["model-fallback-router"],
        current_document["plugins"]["configs"]["model-fallback-router"]
    );
    assert_eq!(
        document["plugins"]["configs"]["model-fallback-router"]["store"]["version"],
        "0.2.0"
    );
    assert_eq!(
        document["plugins"]["configs"]["model-fallback-router"]["store"]["install"]["type"],
        "github-release"
    );
    assert_eq!(document["commercial-mode"], false);
    assert!(document["plugins"]["configs"].get("example").is_none());
}

#[test]
fn startup_merge_without_current_config_uses_gui_defaults() {
    let template = "# Template\nhost: \"\"\nport: 9000\napi-keys:\n  - template-key\nplugins:\n  enabled: true\nrouting:\n  strategy: fill-first\ndebug: false\n";
    let mut config = GuiConfigFile::default();
    ensure_strong_management_secret(&mut config).unwrap();
    let merged = merge_core_config_yaml(template, None, &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&merged).unwrap();

    assert!(merged.contains("# Template"));
    assert_eq!(
        document["host"],
        serde_norway::Value::String("127.0.0.1".to_string())
    );
    assert_eq!(document["port"], serde_norway::to_value(8317_u16).unwrap());
    assert_eq!(document["debug"], serde_norway::Value::Bool(false));
    assert_eq!(document["api-keys"][0], DEFAULT_API_KEY, "{merged}");
    assert_eq!(document["plugins"]["enabled"], false);
    assert_eq!(document["routing"]["strategy"], "round-robin");
    assert_eq!(document["commercial-mode"], false);
    assert_eq!(document["logging-to-file"], false);
    assert_eq!(document["logs-max-total-size-mb"], 0);
    assert_eq!(document["error-logs-max-files"], 10);
    assert_eq!(document["usage-statistics-enabled"], true);
    assert_eq!(document["redis-usage-queue-retention-seconds"], 60);
    assert_eq!(document["request-log"], false);
    assert_eq!(
        document["remote-management"]["secret-key"],
        config.management_secret_key
    );
}

#[test]
fn startup_merge_can_shrink_template_api_key_sequence() {
    let template = "host: \"\"\nport: 8317\nremote-management:\n  secret-key: \"\"\nauth-dir: ~/.cli-proxy-api\napi-keys:\n  - template-one\n  - template-two\n  - template-three\ndebug: false\nplugins:\n  enabled: false\nrouting:\n  strategy: round-robin\n";
    let current = "host: 127.0.0.1\nport: 8317\nremote-management:\n  secret-key: hashed\nauth-dir: C:/oauth\napi-keys:\n  - '123456'\ndebug: false\nplugins:\n  enabled: false\nrouting:\n  strategy: round-robin\n";

    let mut config = GuiConfigFile::default();
    ensure_strong_management_secret(&mut config).unwrap();
    let merged = merge_core_config_yaml(template, Some(current), &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&merged)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{merged}"));

    assert_eq!(document["api-keys"][0], DEFAULT_API_KEY);
    assert_eq!(document["api-keys"].as_sequence().unwrap().len(), 1);
    assert_eq!(document["debug"], false);
    assert_eq!(
        document["remote-management"]["secret-key"],
        config.management_secret_key
    );
}



#[test]
fn startup_merge_repairs_disabled_fields_rejected_by_native_provider_keys() {
    let template = "config-version: 8\nhost: \"\"\nport: 8317\napi-keys:\n  codex: []\n  claude: []\n  openai-compatibility: []\n";
    let current = "config-version: 8\nhost: 127.0.0.1\nport: 8317\napi-keys:\n  codex:\n    - name: codex-1\n      excluded-models:\n        - preview-*\n      keys:\n        - api-key: live\n        - api-key: paused\n          disabled: true\n          excluded-models:\n            - preview-*\n  claude:\n    - name: claude-1\n      disabled: true\n      keys:\n        - api-key: claude-key\n  openai-compatibility:\n    - name: relay\n      base-url: https://relay.example/v1\n      disabled: true\n      keys:\n        - api-key: relay-key\n";
    let config = GuiConfigFile::default();
    let merged = merge_core_config_yaml(template, Some(current), &config).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&merged)
        .unwrap_or_else(|error| panic!("invalid YAML: {error}\n{merged}"));

    assert!(serde_norway::to_string(&document["api-keys"]["codex"]).unwrap().contains("disabled") == false);
    assert_eq!(document["api-keys"]["codex"][0]["keys"][1]["excluded-models"][0], "preview-*");
    assert_eq!(document["api-keys"]["codex"][0]["keys"][1]["excluded-models"][1], "*");
    assert!(document["api-keys"]["claude"][0].get("disabled").is_none());
    assert_eq!(document["api-keys"]["claude"][0]["excluded-models"][0], "*");
    assert_eq!(document["api-keys"]["claude"][0]["keys"][0]["api-key"], "claude-key");
    assert_eq!(document["api-keys"]["openai-compatibility"][0]["disabled"], true);
    assert!(!merged.contains("disabled: true\n          excluded-models"));
}

#[test]
fn v8_signed_retry_weighted_routing_and_empty_host_roundtrip() {
    let content = "config-version: 8\nserver: {host: '', port: 8317}\nrouting: {strategy: weighted-round-robin, retry: {max-retry-interval: -1}}\nmanagement: {secret-key: ''}\n";
    let settings = core_config_settings_from_value(&serde_norway::from_str(content).unwrap()).unwrap();
    let mut config = GuiConfigFile::default();
    apply_core_settings_to_gui_config(&mut config, &settings);
    sanitize_gui_config(&mut config).unwrap();
    validate_gui_config(&config).unwrap();
    assert_eq!(config.host, "");
    assert_eq!(config.max_retry_interval, -1);
    assert_eq!(config.routing_strategy, "weighted-round-robin");
    assert!(!ensure_strong_management_secret(&mut config).unwrap());
    assert!(config.management_secret_key.is_empty());
    let serialized = toml::to_string(&config).unwrap();
    let mut restored: GuiConfigFile = toml::from_str(&serialized).unwrap();
    assert!(!ensure_strong_management_secret(&mut restored).unwrap());
    assert_eq!(restored.max_retry_interval, -1);
    let saved = apply_gui_managed_settings(content, &restored).unwrap();
    let document: serde_norway::Value = serde_norway::from_str(&saved).unwrap();
    assert_eq!(document["server"]["host"], "");
    assert_eq!(document["management"]["secret-key"], "");
    assert_eq!(document["routing"]["retry"]["max-retry-interval"], -1);
}


#[test]
fn explicitly_selected_auth_directory_is_not_moved_on_restart() {
    let root = agent_test_home("selected-auth-no-migration");
    let install_dir = root.join("cpa-core");
    let selected = install_dir.join("oauth");
    let persistent = root.join("oauth");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join("account.json"), "credential").unwrap();
    let mut config = GuiConfigFile { auth_dir: "oauth".into(), auth_dir_user_selected: true, ..GuiConfigFile::default() };
    sanitize_gui_config_at(&mut config, &install_dir, &persistent).unwrap();
    assert_eq!(config.auth_dir, "oauth");
    assert!(selected.join("account.json").exists());
    assert!(!persistent.exists());
    let config_path = root.join("selected.toml");
    write_gui_config_to_path(&config, &config_path).unwrap();
    let mut restored: GuiConfigFile = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert!(restored.auth_dir_user_selected);
    sanitize_gui_config_at(&mut restored, &install_dir, &persistent).unwrap();
    assert_eq!(restored.auth_dir, "oauth");
    assert!(selected.join("account.json").exists());
    fs::remove_dir_all(root).unwrap();
}
