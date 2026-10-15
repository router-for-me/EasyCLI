use super::*;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = crate::unique_test_temp_path("cpa-workbuddy-test");
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join(".workbuddy-ai/models.json")
    }
    fn save(&self, value: &Value) {
        fs::create_dir_all(self.path().parent().unwrap()).unwrap();
        fs::write(self.path(), serde_json::to_string_pretty(value).unwrap()).unwrap();
    }
    fn read(&self) -> Value {
        parse_workbuddy_config(read_optional_text(&self.path()).unwrap().as_deref()).unwrap()
    }
    fn apply(&self, model: &str) -> Result<AgentConfigActionResult, String> {
        apply_agent_configuration_with_oauth(
            AgentClient::WorkBuddy,
            &self.0,
            8317,
            "test-key",
            model,
            AgentConfigurationOptions {
                models: &[],
                codex_catalog: None,
                oauth_configuration: false,
                claude_code_model_mappings: None,
                claude_desktop_model_mappings: None,
            },
        )
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn workbuddy_formats_round_trip_through_update_backup_and_disable() {
    for original in [
        json!([]),
        json!({}),
        json!({"models": []}),
        json!([{"id":"my-model","url":"https://example.test/v1/chat/completions","apiKey":"private"}]),
        json!({"models":[{"id":"my-model","extensions":{"keep":true}}],"availableModels":["my-model"],"custom":true}),
        json!({"availableModels":["gpt-one","my-model"]}),
    ] {
        let home = Home::new();
        home.save(&original);
        home.apply("gpt-one").unwrap();
        assert_eq!(
            inspect_workbuddy_agent_config(&home.path(), 8317, "test-key").unwrap(),
            (true, Some("gpt-one".into()))
        );
        let bytes = fs::read(home.path()).unwrap();
        assert_eq!(home.apply("gpt-one").unwrap().outcome, "unchanged");
        assert_eq!(fs::read(home.path()).unwrap(), bytes);
        assert_eq!(
            serde_json::to_value(create_backup("workbuddy", &home.0).unwrap()).unwrap()
                ["restorable"],
            true
        );
        clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
        assert_eq!(home.read(), original);
    }
}

#[test]
fn workbuddy_switching_and_disable_keep_external_models_and_settings() {
    for array in [false, true] {
        let home = Home::new();
        home.save(&if array {
            json!([])
        } else {
            json!({"availableModels":["built-in"]})
        });
        home.apply("gpt-one").unwrap();
        let mut edited = home.read();
        let mut entries = workbuddy_models(&edited).to_vec();
        entries[0]["temperature"] = json!(0.2);
        let first_model = entries[0].clone();
        entries.push(json!({"id":"user-added","apiKey":"user-key","extra":{"a":[1,2]}}));
        set_workbuddy_models(&mut edited, entries);
        home.save(&edited);
        home.apply("gpt-two").unwrap();
        let updated = home.read();
        assert_eq!(workbuddy_models(&updated).len(), 3);
        assert!(workbuddy_models(&updated).contains(&first_model));
        assert_eq!(
            inspect_workbuddy_agent_config(&home.path(), 8317, "test-key").unwrap(),
            (true, Some("gpt-two".into()))
        );
        if !array {
            assert_eq!(
                updated["availableModels"],
                json!(["built-in", "gpt-one", "gpt-two"])
            );
        }
        let second_model = workbuddy_models(&updated)
            .iter()
            .find(|m| m["id"] == "gpt-two")
            .unwrap()
            .clone();
        home.apply("gpt-one").unwrap();
        let switched_back = home.read();
        assert_eq!(workbuddy_models(&switched_back).len(), 3);
        assert!(workbuddy_models(&switched_back).contains(&first_model));
        assert!(workbuddy_models(&switched_back).contains(&second_model));
        assert_eq!(
            inspect_workbuddy_agent_config(&home.path(), 8317, "test-key").unwrap(),
            (true, Some("gpt-one".into()))
        );
        let bytes = fs::read(home.path()).unwrap();
        assert_eq!(home.apply("gpt-one").unwrap().outcome, "unchanged");
        assert_eq!(fs::read(home.path()).unwrap(), bytes);
        clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
        assert_eq!(
            workbuddy_models(&home.read()),
            &[json!({"id":"user-added","apiKey":"user-key","extra":{"a":[1,2]}})]
        );
        if !array {
            assert_eq!(home.read()["availableModels"], json!(["built-in"]));
        }
    }
}

#[test]
fn workbuddy_status_checks_endpoint_key_visibility_and_tool_support() {
    let home = Home::new();
    home.apply("gpt-one").unwrap();
    assert!(
        !inspect_workbuddy_agent_config(&home.path(), 8318, "test-key")
            .unwrap()
            .0
    );
    assert!(
        !inspect_workbuddy_agent_config(&home.path(), 8317, "wrong-key")
            .unwrap()
            .0
    );
    let original = home.read();
    for (pointer, value) in [
        ("/availableModels", json!(["hidden"])),
        ("/models/0/disabled", json!(true)),
        ("/models/0/supportsToolCall", json!(false)),
        ("/models/0/url", json!("http://127.0.0.1:8317/v1")),
    ] {
        let mut changed = original.clone();
        if pointer == "/availableModels" {
            changed["availableModels"] = value;
        } else {
            *changed.pointer_mut(pointer).unwrap() = value;
        }
        home.save(&changed);
        assert!(
            !inspect_workbuddy_agent_config(&home.path(), 8317, "test-key")
                .unwrap()
                .0
        );
        assert!(workbuddy_has_managed_marker(&home.path()).unwrap());
    }
    let mut prefixed = original;
    prefixed["availableModels"] = json!(["custom-local:gpt-one"]);
    home.save(&prefixed);
    assert!(
        inspect_workbuddy_agent_config(&home.path(), 8317, "test-key")
            .unwrap()
            .0
    );
}

#[test]
fn workbuddy_config_file_allows_cpa_connection_without_detected_application() {
    let home = Home::new();
    home.save(&json!({}));

    let status = inspect_agent_config_with_executables(AgentClient::WorkBuddy, &home.0, 8317, "test-key", None, None);
    assert!(!status.installed);
    assert!(status.config_exists);
    assert!(status.config_valid);
    assert!(status.launch_targets.is_empty());
    assert!(validate_agent_can_enable(
        AgentClient::WorkBuddy,
        &home.0,
        8317,
        "test-key",
        None
    )
    .is_ok());

    home.apply("gpt-one").unwrap();
    let connected = inspect_agent_config_with_executables(AgentClient::WorkBuddy, &home.0, 8317, "test-key", None, None);
    assert!(!connected.installed);
    assert!(connected.configured);
    assert_eq!(connected.current_model.as_deref(), Some("gpt-one"));
}

#[test]
fn workbuddy_visibility_restores_original_references_and_keeps_user_edits() {
    for external_edit in [false, true] {
        let home = Home::new();
        home.save(&json!({"availableModels":["gpt-one","built-in"]}));
        home.apply("gpt-one").unwrap();
        home.apply("gpt-two").unwrap();
        if external_edit {
            let mut current = home.read();
            current["availableModels"] = json!(["gpt-two", "user-added"]);
            home.save(&current);
        }
        clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
        assert_eq!(
            home.read()["availableModels"],
            if external_edit {
                json!(["user-added"])
            } else {
                json!(["gpt-one", "built-in"])
            }
        );
    }
}

#[test]
fn workbuddy_new_visibility_filters_remove_managed_references_on_disable() {
    for original in [json!({}), json!({"availableModels": []}), json!([])] {
        for reapply in [false, true] {
            for (visible, expected) in [
                (json!(["gpt-one"]), json!([])),
                (
                    json!(["custom-local:gpt-one", "custom-local:gpt-two", "built-in"]),
                    json!(["built-in"]),
                ),
            ] {
                let home = Home::new();
                home.save(&original);
                home.apply("gpt-one").unwrap();
                home.apply("gpt-two").unwrap();
                let current = home.read();
                let mut current = if current.is_array() {
                    json!({"models": current})
                } else {
                    current
                };
                current["availableModels"] = visible;
                home.save(&current);
                if reapply {
                    home.apply("gpt-two").unwrap();
                }
                clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
                let restored = home.read();
                assert!(workbuddy_models(&restored).is_empty());
                assert_eq!(restored["availableModels"], expected);
            }
        }
    }
}

#[test]
fn workbuddy_cleared_visibility_does_not_restore_old_references() {
    for reapply in [false, true] {
        for remove_field in [false, true] {
            let home = Home::new();
            home.save(&json!({"availableModels": ["gpt-one", "built-in"]}));
            home.apply("gpt-one").unwrap();
            home.apply("gpt-two").unwrap();
            let mut current = home.read();
            if remove_field {
                current.as_object_mut().unwrap().remove("availableModels");
            } else {
                current["availableModels"] = json!([]);
            }
            home.save(&current);
            if reapply {
                home.apply("gpt-two").unwrap();
            }
            clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
            let restored = home.read();
            assert!(workbuddy_models(&restored).is_empty());
            if remove_field {
                assert!(restored.get("availableModels").is_none());
            } else {
                assert_eq!(restored["availableModels"], json!([]));
            }
        }
    }
}

#[test]
fn workbuddy_visibility_keeps_models_detached_from_cpa() {
    for reapply in [false, true] {
        let home = Home::new();
        home.apply("gpt-one").unwrap();
        let mut current = home.read();
        current["models"][0]["vendor"] = json!("user-provider");
        current["availableModels"] = json!(["custom-local:gpt-one", "built-in"]);
        home.save(&current);
        if reapply {
            home.apply("gpt-two").unwrap();
        }
        clear_agent_managed_configuration(AgentClient::WorkBuddy, &home.0, 8317).unwrap();
        assert_eq!(home.read(), current);
    }
}

#[test]
fn workbuddy_write_guard_rejects_removing_unrelated_visible_models() {
    let before = json!({"availableModels":["user-model"]});
    let rendered = build_workbuddy_agent_config(
        Some(&before.to_string()),
        "http://127.0.0.1:8317/v1",
        "key",
        "gpt-one",
        &[],
    )
    .unwrap();
    let mut after = parse_workbuddy_config(Some(&rendered)).unwrap();
    validate_workbuddy_unmanaged_preserved(&before, &after).unwrap();
    after["availableModels"] = json!(["gpt-one"]);
    assert!(validate_workbuddy_unmanaged_preserved(&before, &after).is_err());
}

#[test]
fn workbuddy_rejects_collisions_and_malformed_configs_without_writing() {
    for value in [
        json!([{"id":"gpt-one","apiKey":"user-key"}]),
        json!({"models":[{"id":"custom-local:gpt-one","apiKey":"user-key"}]}),
        json!({"models":{}}),
        json!({"availableModels":false}),
        json!({"models":[{"id":"same"},{"id":"custom-local:same"}]}),
    ] {
        let home = Home::new();
        home.save(&value);
        let before = fs::read(home.path()).unwrap();
        assert!(home.apply("gpt-one").is_err());
        assert_eq!(fs::read(home.path()).unwrap(), before);
    }
}

#[test]
fn workbuddy_switch_refreshes_retained_credentials_and_metadata() {
    let home = Home::new();
    home.apply("vision").unwrap();
    let mut current = home.read();
    current["models"][0]["id"] = json!("custom-local:vision");
    current["models"][0]["temperature"] = json!(0.2);
    current["availableModels"] = json!(["custom-local:vision"]);
    let next = build_workbuddy_agent_config(
        Some(&current.to_string()),
        "http://127.0.0.1:8318/v1",
        "new-key",
        "gpt-two",
        &[AgentModelOption {
            name: "vision".into(),
            alias: None,
            is_alias: false,
            context_window: Some(128_000),
            input_modalities: Some(vec!["text".into(), "image".into()]),
            harness_metadata: None,
        }],
    )
    .unwrap();
    let updated = parse_workbuddy_config(Some(&next)).unwrap();
    assert_eq!(
        updated["availableModels"],
        json!(["custom-local:vision", "gpt-two"])
    );
    assert_eq!(workbuddy_models(&updated).len(), 2);
    let retained = &updated["models"][1];
    assert_eq!(retained["id"], "custom-local:vision");
    assert_eq!(retained["maxInputTokens"], 128_000);
    assert_eq!(retained["supportsImages"], true);
    assert_eq!(retained["temperature"], 0.2);
    for entry in workbuddy_models(&updated) {
        assert_eq!(entry["url"], "http://127.0.0.1:8318/v1/chat/completions");
        assert_eq!(entry["apiKey"], "new-key");
    }
    home.save(&updated);
    assert_eq!(
        inspect_workbuddy_agent_config(&home.path(), 8318, "new-key").unwrap(),
        (true, Some("gpt-two".into()))
    );
    for (field, value) in [
        ("apiKey", json!("old-key")),
        ("url", json!("http://127.0.0.1:8317/v1/chat/completions")),
        ("disabled", json!(true)),
        ("supportsToolCall", json!(false)),
    ] {
        let mut stale = updated.clone();
        stale["models"][1][field] = value;
        home.save(&stale);
        assert_eq!(
            inspect_workbuddy_agent_config(&home.path(), 8318, "new-key").unwrap(),
            (false, Some("gpt-two".into()))
        );
    }
    let switched_back = build_workbuddy_agent_config(
        Some(&next),
        "http://127.0.0.1:8318/v1",
        "new-key",
        "vision",
        &[],
    )
    .unwrap();
    let switched_back = parse_workbuddy_config(Some(&switched_back)).unwrap();
    assert_eq!(workbuddy_models(&switched_back).len(), 2);
    assert_eq!(switched_back["models"][0]["id"], "vision");
    assert_eq!(switched_back["models"][0]["temperature"], 0.2);
    assert_eq!(switched_back["availableModels"], updated["availableModels"]);
    home.save(&switched_back);
    assert_eq!(
        inspect_workbuddy_agent_config(&home.path(), 8318, "new-key").unwrap(),
        (true, Some("vision".into()))
    );
}

#[test]
fn workbuddy_metadata_and_model_extensions_survive_reapply() {
    let config = build_workbuddy_agent_config(
        None,
        "http://127.0.0.1:8317/v1",
        "key",
        "vision",
        &[AgentModelOption {
            name: "vision".into(),
            alias: None,
            is_alias: false,
            context_window: Some(128_000),
            input_modalities: Some(vec!["text".into(), "image".into()]),
            harness_metadata: None,
        }],
    )
    .unwrap();
    let mut value = parse_workbuddy_config(Some(&config)).unwrap();
    assert_eq!(value["models"][0]["maxInputTokens"], 128_000);
    assert_eq!(value["models"][0]["supportsImages"], true);
    assert_eq!(
        value["models"][0]["url"],
        "http://127.0.0.1:8317/v1/chat/completions"
    );
    value["models"][0]["temperature"] = json!(0.2);
    let next = build_workbuddy_agent_config(
        Some(&value.to_string()),
        "http://127.0.0.1:8318/v1",
        "new",
        "vision",
        &[],
    )
    .unwrap();
    assert_eq!(
        parse_workbuddy_config(Some(&next)).unwrap()["models"][0]["temperature"],
        0.2
    );
}

#[test]
fn workbuddy_paths_use_environment_then_installed_product_data_folder() {
    let home = Home::new();
    let executable = home.0.join("install/WorkBuddyAI.exe");
    let product = workbuddy_resources(&executable)
        .unwrap()
        .join("app.asar.unpacked/cli/product.json");
    fs::create_dir_all(product.parent().unwrap()).unwrap();
    fs::write(product, r#"{"dataFolderName":".workbuddy-enterprise"}"#).unwrap();
    let resolve = |wb, cb| workbuddy_home_from_environment(&home.0, wb, cb, Some(&executable));
    assert_eq!(resolve(None, None), home.0.join(".workbuddy-enterprise"));
    assert_eq!(resolve(Some(&home.0), None), home.0);
    assert_eq!(resolve(None, Some(&home.0)), home.0);
    assert_eq!(resolve(Some(&home.0), Some(Path::new("unused"))), home.0);
}

#[test]
fn workbuddy_macos_finds_plist_executable_before_legacy_name() {
    let home = Home::new();
    for (index, app) in ["WorkBuddy", "WorkBuddy AI"].into_iter().enumerate() {
        let root = home.0.join(format!("install-{index}"));
        let application = root.join(format!("{app}.app"));
        let macos = application.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(application.join("Contents/Info.plist"), "Electron").unwrap();
        fs::write(macos.join(app), "legacy").unwrap();
        let electron = macos.join("Electron");
        fs::write(&electron, "electron").unwrap();

        assert_eq!(
            find_workbuddy_macos_executable_in_roots(&[root], |info| {
                fs::read_to_string(info).ok()
            }),
            Some(electron)
        );
    }
}

#[test]
fn workbuddy_macos_falls_back_for_missing_or_invalid_plist_executable() {
    let home = Home::new();
    let outside = home.0.join("outside");
    fs::write(&outside, "outside").unwrap();
    for (index, app, declared) in [
        (0, "WorkBuddy", None),
        (1, "WorkBuddy AI", Some("Electron".to_string())),
        (2, "WorkBuddy", Some("../outside".to_string())),
        (3, "WorkBuddy AI", Some(".".to_string())),
        (4, "WorkBuddy", Some(outside.to_string_lossy().into_owned())),
    ] {
        let root = home.0.join(format!("install-{index}"));
        let application = root.join(format!("{app}.app"));
        let macos = application.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        if let Some(declared) = declared {
            fs::write(application.join("Contents/Info.plist"), declared).unwrap();
        }
        fs::write(application.join("Contents/outside"), "outside").unwrap();
        let fallback = macos.join(app);
        fs::write(&fallback, "legacy").unwrap();

        assert_eq!(
            find_workbuddy_macos_executable_in_roots(&[root], |info| {
                fs::read_to_string(info).ok()
            }),
            Some(fallback)
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn workbuddy_reads_xml_and_binary_bundle_executable() {
    let home = Home::new();
    let xml = home.0.join("Info.xml.plist");
    fs::write(
        &xml,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleExecutable</key><string>Electron</string></dict></plist>"#,
    )
    .unwrap();
    assert_eq!(
        read_workbuddy_bundle_executable(&xml).as_deref(),
        Some("Electron")
    );

    let binary = home.0.join("Info.binary.plist");
    let status = Command::new("/usr/bin/plutil")
        .args(["-convert", "binary1", "-o"])
        .arg(&binary)
        .arg(&xml)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        read_workbuddy_bundle_executable(&binary).as_deref(),
        Some("Electron")
    );
}

#[cfg(target_os = "windows")]
#[test]
fn workbuddy_detects_local_install_without_launching_it() {
    let home = Home::new();
    let executable = home
        .0
        .join("AppData/Local/Programs/WorkBuddyAI/WorkBuddyAI.exe");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "not an executable").unwrap();
    assert_eq!(
        find_workbuddy_desktop_executable(&home.0),
        Some(executable.clone())
    );
    assert!(agent_installation_detected(
        AgentClient::WorkBuddy,
        None,
        true,
        false
    ));
    assert!(!should_probe_primary_agent_executable_version(
        AgentClient::WorkBuddy
    ));
    assert_eq!(read_workbuddy_app_version(&executable), None);
    let targets = agent_launch_targets(AgentClient::WorkBuddy, Some(&executable), None, false);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "app");
    assert_eq!(
        parse_windows_desktop_registration(
            "icon",
            &format!("\"{}\",0", executable.display()),
            "WorkBuddyAI.exe"
        ),
        Some(executable)
    );
}
