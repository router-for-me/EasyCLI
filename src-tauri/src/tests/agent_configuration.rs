use super::support::*;
use super::*;

#[test]
fn claude_code_explicit_model_inputs_replace_clear_and_round_trip() {
    let directory = agent_test_home("claude-code-explicit-models");
    let path = directory.join("settings.json");
    let models = test_agent_models(&["route-model"]);
    let mut mappings = ClaudeDesktopModelMappings::all("route-model");
    mappings.startup_model = Some(" custom-startup[1m] ".into());
    mappings.subagent_model = Some("custom-worker".into());
    let resolved = resolve_claude_code_model_mappings(
        AgentClient::ClaudeCode, &models, "route-model", Some(mappings),
    ).unwrap().unwrap();
    let rendered = build_claude_agent_config(
        Some(r#"{"model":"user-choice","env":{"KEEP":"yes","ANTHROPIC_MODEL":"old-startup","CLAUDE_CODE_SUBAGENT_MODEL":"old-worker"}}"#),
        "http://127.0.0.1:8317", "test-key", "route-model", &models, Some(&resolved),
    ).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["model"], "custom-startup[1m]");
    assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "custom-worker");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "route-model");
    assert_eq!(value["modelSettings"]["route-model"]["autoCompactWindow"], 200_000);
    assert_eq!(value["modelSettings"]["custom-startup"]["autoCompactWindow"], 200_000);
    assert_eq!(value["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "200000");
    assert_eq!(value["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "200000");
    assert_eq!(value["autoCompactWindow"], 200_000);
    assert_eq!(value["model"], "custom-startup[1m]");
    fs::write(&path, &rendered).unwrap();
    let mut inspected = inspect_claude_code_model_mappings(&path).unwrap().unwrap();
    assert_eq!(inspected.startup_model.as_deref(), Some("custom-startup[1m]"));
    assert_eq!(inspected.subagent_model.as_deref(), Some("custom-worker"));
    inspected.startup_model = Some(String::new());
    inspected.subagent_model = Some(String::new());
    let cleared = build_claude_agent_config(
        Some(&rendered), "http://127.0.0.1:8317", "test-key", "route-model", &models, Some(&inspected),
    ).unwrap();
    let value: serde_json::Value = serde_json::from_str(&cleared).unwrap();
    assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
    assert!(value["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none());
    assert!(value.get("model").is_none());
    assert_eq!(value["env"]["KEEP"], "yes");
}

#[test]
fn claude_code_role_selection_survives_mapping_changes_without_global_effort_override() {
    let models = test_agent_models(&["route-a", "route-b"]);
    let mut mappings = ClaudeDesktopModelMappings::all("route-a");
    mappings.startup_model = Some("sonnet".into());
    mappings.subagent_model = Some("haiku".into());
    for route in ["route-a", "route-b"] {
        mappings.sonnet = route.into();
        mappings.sonnet_1m = true;
        let rendered = build_claude_agent_config(
            Some(r#"{"model":"user-choice","env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#),
            "http://127.0.0.1:8317", "test-key", route, &models, Some(&mappings),
        ).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["model"], "sonnet");
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "haiku");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], format!("{route}[1m]"));
        assert_eq!(value["modelSettings"][route]["autoCompactWindow"], 200_000);
        assert_eq!(value["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "200000");
        assert_eq!(value["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "200000");
        assert!(value["env"].get("CLAUDE_CODE_EFFORT_LEVEL").is_none());
        assert_eq!(value["model"], "sonnet");
    }
    mappings.sonnet_1m = false;
    mappings.startup_model = Some("route-a[1m]".into());
    let resolved = resolve_claude_code_model_mappings(
        AgentClient::ClaudeCode, &models, "route-a", Some(mappings),
    ).unwrap().unwrap();
    assert_eq!(resolved.max_context_tokens, 200_000);
    assert_eq!(resolved.startup_model.as_deref(), Some("route-a[1m]"));
    assert_eq!(resolved.fable_1m, resolved.sonnet_1m);
}

#[test]
fn claude_agent_config_preserves_existing_fields() {
    let rendered = build_claude_agent_config(
            Some(
                r#"{"theme":"dark","env":{"KEEP":"yes","ANTHROPIC_API_KEY":"legacy-key","CLAUDE_CODE_EFFORT_LEVEL":"max","CLAUDE_CODE_AUTO_MODE_SERVER":"1"}}"#,
            ),
            "http://127.0.0.1:8317",
            DEFAULT_API_KEY,
            "claude-test",
            &test_agent_models(&["claude-test"]),
            None,
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(value["theme"], "dark");
    assert_eq!(value["env"]["KEEP"], "yes");
    assert!(value["env"].get("ANTHROPIC_API_KEY").is_none());
    assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
    assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], DEFAULT_API_KEY);
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-test");
    assert_eq!(
        value["env"][CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV],
        "1"
    );
    assert_eq!(value["env"][CLAUDE_CODE_AUTO_MODE_SERVER_ENV], "0");
    assert_eq!(value["autoCompactWindow"], 200_000);
    assert_eq!(value["autoCompactEnabled"], true);
    assert!(value["env"].get("CLAUDE_CODE_EFFORT_LEVEL").is_none());
    assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-test");
    assert_eq!(
        value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"],
        "claude-test"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION"],
        "Auto-compact window: 200K"
    );
    assert_eq!(value["model"], "claude-test");
}

#[test]
fn claude_code_inspection_normalizes_legacy_1m_suffix() {
    let directory = agent_test_home("claude-code-1m-inspection");
    let path = directory.join("settings.json");
    fs::write(
        &path,
        r#"{
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8317",
                    "ANTHROPIC_AUTH_TOKEN": "test-key",
                    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "1",
                    "CLAUDE_CODE_AUTO_MODE_SERVER": "0",
                    "ANTHROPIC_MODEL": "deepseek-v4-pro[1m]",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "deepseek-v4-pro[1m]",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "deepseek-v4-pro[1m]",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "deepseek-v4-flash"
                },
                "model": "deepseek-v4-pro[1m]"
            }"#,
    )
    .unwrap();

    let (configured, model) = inspect_claude_agent_config(&path, 8317, "test-key").unwrap();
    let mappings = inspect_claude_code_model_mappings(&path).unwrap().unwrap();

    assert!(configured);
    assert_eq!(model.as_deref(), Some("deepseek-v4-pro"));
    assert_eq!(mappings.opus, "deepseek-v4-pro");
    assert_eq!(mappings.sonnet, "deepseek-v4-pro");
    assert_eq!(mappings.haiku, "deepseek-v4-flash");
    assert!(mappings.opus_1m);
    assert!(mappings.sonnet_1m);
    assert!(!mappings.haiku_1m);
    assert_eq!(mappings.max_context_tokens, 1_000_000);
    assert_eq!(mappings.auto_compact_pct, 90);
    assert!(!mappings.disable_auto_compact);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn claude_code_requires_gateway_model_discovery_to_be_enabled() {
    let directory = agent_test_home("claude-code-gateway-model-discovery");
    let path = directory.join("settings.json");
    fs::write(
        &path,
        r#"{
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8317",
                    "ANTHROPIC_AUTH_TOKEN": "test-key",
                    "ANTHROPIC_MODEL": "gpt-test",
                    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "0",
                    "CLAUDE_CODE_AUTO_MODE_SERVER": "0"
                }
            }"#,
    )
    .unwrap();

    let (configured, model) = inspect_claude_agent_config(&path, 8317, "test-key").unwrap();

    assert!(!configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn claude_code_requires_server_classifier_to_be_disabled_for_gateway() {
    let directory = agent_test_home("claude-code-auto-mode-server");
    let path = directory.join("settings.json");
    fs::write(
        &path,
        r#"{
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8317",
                    "ANTHROPIC_AUTH_TOKEN": "test-key",
                    "ANTHROPIC_MODEL": "gpt-test",
                    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "1",
                    "CLAUDE_CODE_AUTO_MODE_SERVER": "1"
                }
            }"#,
    )
    .unwrap();

    let (configured, model) = inspect_claude_agent_config(&path, 8317, "test-key").unwrap();

    assert!(!configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn claude_mapping_legacy_json_defaults_1m_preferences_off() {
    let mappings: ClaudeDesktopModelMappings =
        serde_json::from_str(r#"{"opus":"model-a","sonnet":"model-b","haiku":"model-c"}"#).unwrap();

    assert!(!mappings.opus_1m);
    assert!(!mappings.sonnet_1m);
    assert!(!mappings.haiku_1m);
    assert_eq!(mappings.max_context_tokens, 200_000);
    assert_eq!(mappings.auto_compact_pct, 90);
    assert!(!mappings.disable_auto_compact);
}

#[test]
fn claude_code_role_mappings_drive_settings() {
    let mappings = ClaudeDesktopModelMappings {
        desktop_models: None,
        opus: "gpt-opus".to_string(),
        sonnet: "gpt-sonnet".to_string(),
        haiku: "gpt-haiku".to_string(),
        opus_1m: false,
        sonnet_1m: false,
        haiku_1m: false,
        max_context_tokens: 272_000,
        auto_compact_pct: 80,
        disable_auto_compact: false,
        manage_default_model: true,
        manage_subagent_model: true,
        fable: String::new(),
        fable_1m: false,
        startup_model: None,
        subagent_model: None,
    };
    let models = vec![
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "gpt-opus-base".to_string(),
            alias: None,
            is_alias: false,
            context_window: Some(1_000_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.opus.clone(),
            alias: Some("gpt-opus-base".to_string()),
            is_alias: true,
            context_window: Some(128_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.sonnet.clone(),
            alias: None,
            is_alias: false,
            context_window: Some(272_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.haiku.clone(),
            alias: None,
            is_alias: false,
            context_window: Some(128_000),
        },
    ];
    let rendered = build_claude_agent_config(
        None,
        "http://127.0.0.1:8317",
        "test-key",
        "gpt-sonnet",
        &models,
        Some(&mappings),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "gpt-sonnet");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "gpt-opus");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "gpt-sonnet");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "gpt-haiku");
    assert_eq!(value["autoCompactWindow"], 272_000);
    assert_eq!(
        value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"],
        "gpt-sonnet"
    );
}

#[test]
fn claude_code_can_leave_session_model_and_subagents_to_claude_code() {
    let mappings = ClaudeDesktopModelMappings {
        manage_default_model: false,
        manage_subagent_model: false,
        ..ClaudeDesktopModelMappings::all("gpt-sonnet")
    };
    let rendered = build_claude_agent_config(
        Some(r#"{"model":"user-selected","env":{"ANTHROPIC_MODEL":"user-selected","CLAUDE_CODE_SUBAGENT_MODEL":"user-subagent"}}"#),
        "http://127.0.0.1:8317",
        "test-key",
        "gpt-sonnet",
        &test_agent_models(&["gpt-sonnet"]),
        Some(&mappings),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["model"], "user-selected");
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "user-selected");
    assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "user-subagent");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "gpt-sonnet");
    assert_eq!(value["env"]["EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL"], "0");
    assert_eq!(value["env"]["EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL"], "0");
}

#[test]
fn claude_code_unmanaged_mode_removes_previous_cpa_overrides() {
    let mappings = ClaudeDesktopModelMappings {
        manage_default_model: false,
        manage_subagent_model: false,
        ..ClaudeDesktopModelMappings::all("gpt-sonnet")
    };
    let rendered = build_claude_agent_config(
        Some(r#"{"model":"gpt-sonnet","env":{"ANTHROPIC_MODEL":"gpt-sonnet","CLAUDE_CODE_SUBAGENT_MODEL":"gpt-sonnet"}}"#),
        "http://127.0.0.1:8317",
        "test-key",
        "gpt-sonnet",
        &test_agent_models(&["gpt-sonnet"]),
        Some(&mappings),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert!(value.get("model").is_none());
    assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
    assert!(value["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none());
}

#[test]
fn claude_code_runtime_settings_keep_per_role_1m_suffixes() {
    let mappings = ClaudeDesktopModelMappings {
        desktop_models: None,
        opus: "custom-pro".to_string(),
        sonnet: "custom-pro".to_string(),
        haiku: "custom-flash".to_string(),
        opus_1m: true,
        sonnet_1m: true,
        haiku_1m: false,
        max_context_tokens: 1_000_000,
        auto_compact_pct: 75,
        disable_auto_compact: true,
        manage_default_model: true,
        manage_subagent_model: true,
        fable: String::new(),
        fable_1m: false,
        startup_model: None,
        subagent_model: None,
    };
    let models = vec![
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "custom-pro".to_string(),
            alias: Some("Custom Pro".to_string()),
            is_alias: false,
            context_window: Some(200_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "custom-flash".to_string(),
            alias: Some("Custom Flash".to_string()),
            is_alias: false,
            context_window: Some(200_000),
        },
    ];
    let rendered = build_claude_agent_config(
        None,
        "http://127.0.0.1:8317",
        "test-key",
        "custom-pro",
        &models,
        Some(&mappings),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(value["autoCompactWindow"], 1_000_000);
    assert_eq!(value["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "1000000");
    assert_eq!(value["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "1000000");
    assert_eq!(value["autoCompactEnabled"], false);
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "custom-pro[1m]");
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "custom-pro[1m]"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
        "custom-pro[1m]"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "custom-flash"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
        "custom-pro[1m]"
    );
    assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "custom-flash");
    assert!(value["env"].get("CLAUDE_CODE_EFFORT_LEVEL").is_none());
    assert_eq!(
        value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION"],
        "custom-pro[1m]"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"],
        "Custom Pro[1m]"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"],
        "Fable · Custom Pro[1m]"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION"],
        "1M enabled · Auto-compact window: 1M"
    );
    assert_eq!(value["env"]["ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION"], "1M enabled · Auto-compact window: 1M");
    assert_eq!(value["modelSettings"]["custom-flash"]["autoCompactWindow"], 1_000_000);
    assert_eq!(value["modelSettings"]["custom-pro"]["autoCompactWindow"], 1_000_000);
    assert_eq!(value["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "1000000");
    assert_eq!(value["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "1000000");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION"], "1M enabled · Auto-compact window: 1M");
    assert_eq!(value["model"], "custom-pro[1m]");
}

#[test]
fn claude_desktop_omits_unsupported_context_window_when_1m_is_off() {
    let models = vec![AgentModelOption {
        input_modalities: None,
        harness_metadata: None,
        name: "runtime-model".to_string(),
        alias: None,
        is_alias: false,
        context_window: Some(1_000_000),
    }];
    let entry = claude_desktop_inference_model(
        CLAUDE_DESKTOP_SONNET_MODEL_ID,
        "runtime-model",
        false,
        &models,
    );

    assert!(entry.get("contextWindow").is_none());
    assert!(entry.get("supports1m").is_none());
    assert!(entry.get("prefer1m").is_none());
}

#[test]
fn codex_agent_config_uses_managed_provider_without_losing_comments() {
    let rendered = build_codex_agent_config(
        Some("# keep this comment\napproval_policy = \"on-request\"\n"),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
    )
    .unwrap();
    let value: toml::Value = toml::from_str(&rendered).unwrap();

    assert!(rendered.contains("# keep this comment"));
    assert_eq!(value["approval_policy"].as_str(), Some("on-request"));
    assert_eq!(
        value["model_provider"].as_str(),
        Some(MANAGED_AGENT_PROVIDER_ID)
    );
    assert_eq!(value["model"].as_str(), Some("gpt-test"));
    assert_eq!(
        value["model_catalog_json"].as_str(),
        Some(CODEX_MODEL_CATALOG_FILE)
    );
    assert_eq!(
        value["model_providers"][MANAGED_AGENT_PROVIDER_ID]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert_eq!(
        value["model_providers"][MANAGED_AGENT_PROVIDER_ID]["experimental_bearer_token"].as_str(),
        Some(DEFAULT_API_KEY)
    );
}

#[test]
fn codex_oauth_configuration_uses_openai_auth_with_bearer_token() {
    let home = agent_test_home("codex-oauth-configuration");
    let path = home.join(".codex/config.toml");
    let rendered = build_codex_agent_config_with_oauth(
        Some("[model_providers.cpa-gui]\nexperimental_bearer_token = \"old-key\"\n"),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        true,
    )
    .unwrap();
    let value = toml::from_str::<toml::Value>(&rendered).unwrap();
    let provider = value["model_providers"][MANAGED_AGENT_PROVIDER_ID]
        .as_table()
        .unwrap();

    assert_eq!(
        provider
            .get("requires_openai_auth")
            .and_then(toml::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        provider
            .get("experimental_bearer_token")
            .and_then(toml::Value::as_str),
        Some(DEFAULT_API_KEY)
    );

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, rendered).unwrap();
    fs::write(
        path.with_file_name("auth.json"),
        r#"{"tokens":{"access_token":"oauth-access-token"}}"#,
    )
    .unwrap();
    let (configured, model, oauth_configuration) =
        inspect_codex_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap();
    assert!(configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    assert!(oauth_configuration);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn codex_oauth_configuration_is_preserved_for_legacy_updates() {
    let home = agent_test_home("codex-oauth-preserved-update");
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::write(
        codex_dir.join("config.toml"),
        "[model_providers.cpa-gui]\nrequires_openai_auth = true\n",
    )
    .unwrap();
    fs::write(
        codex_dir.join("auth.json"),
        r#"{"tokens":{"access_token":"oauth-access-token"}}"#,
    )
    .unwrap();

    let oauth_configuration = current_codex_oauth_configuration(&home).unwrap();
    assert!(oauth_configuration);

    let models = test_agent_models(&["gpt-test"]);
    let catalog = test_codex_models(&["gpt-test"]);
    let updates = build_agent_updates_with_oauth(
        AgentClient::Codex,
        &home,
        8317,
        DEFAULT_API_KEY,
        "gpt-test",
        AgentConfigurationOptions {
            models: &models,
            codex_catalog: Some(&catalog),
            oauth_configuration,
            claude_code_model_mappings: None,
            claude_desktop_model_mappings: None,
        },
    )
    .unwrap();

    assert!(updates[0].after.contains("requires_openai_auth = true"));
    assert!(
        serde_json::from_str::<serde_json::Value>(&updates[2].after).unwrap()["tokens"]
            ["access_token"]
            .is_string()
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn codex_api_and_oauth_modes_write_the_same_catalog() {
    let home = agent_test_home("codex-auth-mode-catalog");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"access_token":"oauth-access-token"}}"#,
    )
    .unwrap();
    let models = test_agent_models(&["gpt-5.5", "third-party-model"]);
    let catalog = test_codex_models(&["gpt-5.5", "third-party-model"]);
    let build = |oauth_configuration| {
        build_agent_updates_with_oauth(
            AgentClient::Codex,
            &home,
            8317,
            DEFAULT_API_KEY,
            "gpt-5.5",
            AgentConfigurationOptions {
                models: &models,
                codex_catalog: Some(&catalog),
                oauth_configuration,
                claude_code_model_mappings: None,
                claude_desktop_model_mappings: None,
            },
        )
        .unwrap()
    };
    let api_updates = build(false);
    let oauth_updates = build(true);

    assert_eq!(api_updates[1].after, oauth_updates[1].after);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&api_updates[2].after).unwrap()["OPENAI_API_KEY"],
        DEFAULT_API_KEY
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&api_updates[2].after).unwrap()["auth_mode"],
        "apikey"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&oauth_updates[2].after).unwrap()["tokens"]
            ["access_token"]
            .is_string()
    );
    assert!(api_updates[0].after.contains("model_catalog_json"));
    assert!(oauth_updates[0].after.contains("model_catalog_json"));
    assert!(!api_updates[0].after.contains("service_tier"));
    assert!(!oauth_updates[0].after.contains("service_tier"));
    assert!(!api_updates[0].after.contains("requires_openai_auth"));
    assert!(oauth_updates[0]
        .after
        .contains("requires_openai_auth = true"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn codex_status_requires_a_valid_catalog_containing_the_default_model() {
    let home = agent_test_home("codex-catalog-status");
    let config_path = home.join(".codex/config.toml");
    let catalog_path = codex_model_catalog_path(&home);
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        build_codex_agent_config(
            None,
            "http://127.0.0.1:8317/v1",
            DEFAULT_API_KEY,
            "gpt-test",
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(
        config_path.with_file_name("auth.json"),
        build_codex_api_auth(DEFAULT_API_KEY).unwrap(),
    )
    .unwrap();

    let missing = inspect_agent_config(AgentClient::Codex, &home, 8317, DEFAULT_API_KEY);
    assert!(!missing.configured);
    assert!(!missing.config_valid);

    fs::write(&catalog_path, "not json").unwrap();
    let damaged = inspect_agent_config(AgentClient::Codex, &home, 8317, DEFAULT_API_KEY);
    assert!(!damaged.configured);
    assert!(!damaged.config_valid);

    fs::write(&catalog_path, test_codex_models(&["other-model"])).unwrap();
    let wrong_model = inspect_agent_config(AgentClient::Codex, &home, 8317, DEFAULT_API_KEY);
    assert!(!wrong_model.configured);
    assert!(!wrong_model.config_valid);

    fs::write(&catalog_path, test_codex_models(&["gpt-test"])).unwrap();
    let valid = inspect_agent_config(AgentClient::Codex, &home, 8317, DEFAULT_API_KEY);
    assert!(valid.configured);
    assert!(valid.config_valid);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn invalid_codex_catalog_does_not_partially_update_existing_files() {
    let home = agent_test_home("invalid-codex-catalog-transaction");
    let config_path = home.join(".codex/config.toml");
    let catalog_path = codex_model_catalog_path(&home);
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let original_config = "model = \"user-model\"\n";
    let original_catalog = "{\"models\":[{\"slug\":\"user-model\"}]}\n";
    fs::write(&config_path, original_config).unwrap();
    fs::write(&catalog_path, original_catalog).unwrap();

    let result = apply_agent_configuration(
        AgentClient::Codex,
        &home,
        8317,
        DEFAULT_API_KEY,
        "gpt-test",
        &test_agent_models(&["gpt-test"]),
        Some("{invalid"),
    );
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&config_path).unwrap(), original_config);
    assert_eq!(fs::read_to_string(&catalog_path).unwrap(), original_catalog);
    assert!(!agent_state_path(&[config_path]).unwrap().exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn claude_desktop_config_builds_gateway_profile_and_index() {
    let profile = build_claude_desktop_profile(
        Some(r#"{"keep":true,"coworkEgressAllowedHosts":["*"]}"#),
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "claude-sonnet-test",
        &[],
        None,
    )
    .unwrap();
    let meta = build_claude_desktop_meta(Some(
            &format!(
                r#"{{"entries":[{{"id":"other","name":"Other"}},{{"id":"{CLAUDE_DESKTOP_PROFILE_ID}","name":"Old","custom":true}},{{"id":"{CLAUDE_DESKTOP_PROFILE_ID}","name":"Duplicate"}},{{"name":"broken"}}]}}"#,
            ),
        ))
        .unwrap();
    let profile: serde_json::Value = serde_json::from_str(&profile).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();

    assert_eq!(profile["keep"], true);
    assert_eq!(
        profile["coworkEgressAllowedHosts"],
        serde_json::json!(["*"])
    );
    assert_eq!(profile["inferenceGatewayApiKey"], DEFAULT_API_KEY);
    assert_eq!(profile["inferenceGatewayBaseUrl"], "http://127.0.0.1:8317");
    assert_eq!(
        profile["inferenceModels"],
        serde_json::json!([
            {
                "name": "claude-sonnet-test",
                "anthropicFamilyTier": "opus",
                "isFamilyDefault": true
            }
        ])
    );
    assert_eq!(meta["appliedId"], CLAUDE_DESKTOP_PROFILE_ID);
    assert_eq!(meta["entries"].as_array().unwrap().len(), 2);
    let managed_entry = meta["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == CLAUDE_DESKTOP_PROFILE_ID)
        .unwrap();
    assert_eq!(managed_entry["custom"], true);
}

#[test]
fn claude_desktop_profile_keeps_non_claude_models_internal() {
    let mappings = ClaudeDesktopModelMappings {
        opus: "gpt-5.6-sol".to_string(),
        sonnet: "gpt-5.6".to_string(),
        haiku: "gpt-5.6-mini".to_string(),
        opus_1m: true,
        sonnet_1m: false,
        haiku_1m: true,
        ..ClaudeDesktopModelMappings::all("")
    };
    let models = vec![
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.opus.clone(),
            alias: None,
            is_alias: false,
            context_window: Some(1_000_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.sonnet.clone(),
            alias: None,
            is_alias: false,
            context_window: Some(272_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: mappings.haiku.clone(),
            alias: None,
            is_alias: false,
            context_window: Some(1_000_000),
        },
    ];
    let profile = build_claude_desktop_profile(
        None,
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "gpt-5.6-sol",
        &models,
        Some(&mappings),
    )
    .unwrap();
    let profile: serde_json::Value = serde_json::from_str(&profile).unwrap();

    assert_eq!(
        profile["inferenceModels"],
        serde_json::json!([
            {
                "name": CLAUDE_DESKTOP_OPUS_MODEL_ID,
                "labelOverride": "gpt-5.6-sol",
                "anthropicFamilyTier": "opus",
                "isFamilyDefault": true,
                "supports1m": true,
                "prefer1m": true
            },
            {
                "name": CLAUDE_DESKTOP_SONNET_MODEL_ID,
                "labelOverride": "gpt-5.6",
                "anthropicFamilyTier": "sonnet",
                "isFamilyDefault": true
            },
            {
                "name": CLAUDE_DESKTOP_HAIKU_MODEL_ID,
                "labelOverride": "gpt-5.6-mini",
                "anthropicFamilyTier": "haiku",
                "isFamilyDefault": true,
                "supports1m": true,
                "prefer1m": true
            }
        ])
    );
    assert_eq!(profile["inferenceModels"][0]["labelOverride"], "gpt-5.6-sol");
}

#[cfg(target_os = "windows")]
#[test]
fn claude_desktop_detects_windows_variant_config_directories() {
    let home = agent_test_home("claude-desktop-windows-variant-paths");
    let local_app_data = home.join("AppData/Local");
    let normal = local_app_data.join("Claude-Canary");
    let threep = local_app_data.join("Claude-3p-Canary");
    fs::create_dir_all(&normal).unwrap();
    fs::create_dir_all(&threep).unwrap();

    let paths = claude_desktop_config_paths_from_local_app_data(&local_app_data);

    assert_eq!(paths.len(), 4);
    assert_eq!(paths[0], normal.join("claude_desktop_config.json"));
    assert_eq!(paths[1], threep.join("claude_desktop_config.json"));
    assert_eq!(
        paths[2],
        threep
            .join("configLibrary")
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json"))
    );
    assert_eq!(paths[3], threep.join("configLibrary/_meta.json"));
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn claude_desktop_uses_linux_beta_config_paths() {
    let home = agent_test_home("claude-desktop-linux-paths");
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let paths = claude_desktop_config_paths(&home);

    assert!(AgentClient::ClaudeDesktop.supported_platform());
    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths[0],
        config_home.join("Claude/claude_desktop_config.json")
    );
    assert_eq!(
        paths[1],
        config_home.join("Claude-3p/claude_desktop_config.json")
    );
    assert_eq!(
        paths[2],
        config_home
            .join("Claude-3p/configLibrary")
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json"))
    );
    assert_eq!(
        paths[3],
        config_home.join("Claude-3p/configLibrary/_meta.json")
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_agent_config_preserves_other_providers() {
    let models = test_agent_models(&["gpt-test", "deepseek-test"]);
    let rendered = build_opencode_agent_config(
            Some(
                r#"{"theme":"dark","provider":{"other":{"npm":"other"},"cpa-gui":{"custom":"keep","options":{"timeout":30}}}}"#,
            ),
            "http://127.0.0.1:8317/v1",
            DEFAULT_API_KEY,
            "gpt-test",
            &models,
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(value["theme"], "dark");
    assert_eq!(value["provider"]["other"]["npm"], "other");
    assert_eq!(
        value["provider"][MANAGED_AGENT_PROVIDER_ID]["custom"],
        "keep"
    );
    assert_eq!(
        value["provider"][MANAGED_AGENT_PROVIDER_ID]["options"]["timeout"],
        30
    );
    assert_eq!(
        value["provider"][MANAGED_AGENT_PROVIDER_ID]["options"]["baseURL"],
        "http://127.0.0.1:8317/v1"
    );
    assert_eq!(value["model"], "cpa-gui/gpt-test");
    assert!(value["provider"][MANAGED_AGENT_PROVIDER_ID]["models"]["gpt-test"].is_object());
    assert!(value["provider"][MANAGED_AGENT_PROVIDER_ID]["models"]["deepseek-test"].is_object());
}

#[test]
fn opencode_config_path_supports_jsonc_and_custom_config() {
    let home = agent_test_home("opencode-config-paths");
    let default_path = home.join(".config/opencode/opencode.json");
    let jsonc_path = home.join(".config/opencode/opencode.jsonc");

    assert_eq!(
        opencode_config_path_from_environment(&home, None, None),
        default_path
    );

    fs::create_dir_all(jsonc_path.parent().unwrap()).unwrap();
    fs::write(&jsonc_path, "{}\n").unwrap();
    assert_eq!(
        opencode_config_path_from_environment(&home, None, None),
        jsonc_path
    );

    let custom_path = home.join("custom/opencode.jsonc");
    assert_eq!(
        opencode_config_path_from_environment(&home, Some(&custom_path), None),
        custom_path
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_cli_or_desktop_is_detected_as_installed() {
    assert!(agent_installation_detected(
        AgentClient::OpenCode,
        None,
        true,
        false,
    ));
    assert!(agent_installation_detected(
        AgentClient::OpenCode,
        None,
        false,
        true,
    ));
    assert!(!agent_installation_detected(
        AgentClient::OpenCode,
        None,
        false,
        false,
    ));
}

#[cfg(target_os = "windows")]
#[test]
fn opencode_windows_desktop_is_found_in_the_electron_installer_directory() {
    let home = agent_test_home("opencode-windows-desktop");
    let local_app_data = home.join("AppData/Local");
    let application = local_app_data.join("Programs/@opencode-aidesktop/OpenCode.exe");
    fs::create_dir_all(application.parent().unwrap()).unwrap();
    fs::write(&application, []).unwrap();

    assert_eq!(
        find_windows_opencode_desktop_application_in_roots(&local_app_data, &[]),
        Some(application)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_linux_desktop_finds_the_current_package_binary() {
    let home = agent_test_home("opencode-linux-package");
    let executable_directory = home.join("opt/OpenCode");
    let application = executable_directory.join("ai.opencode.desktop");
    fs::create_dir_all(&executable_directory).unwrap();
    fs::write(&application, []).unwrap();

    assert_eq!(
        find_linux_opencode_desktop_application_in_roots(&home, &[executable_directory], &[], &[],),
        Some(application)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_linux_desktop_resolves_an_appimage_desktop_entry() {
    let home = agent_test_home("opencode-linux-appimage-entry");
    let data_directory = home.join("share");
    let applications = data_directory.join("applications");
    let application = home.join("Applications/opencode-desktop-linux-amd64.AppImage");
    fs::create_dir_all(&applications).unwrap();
    fs::create_dir_all(application.parent().unwrap()).unwrap();
    fs::write(&application, []).unwrap();
    let application = PathBuf::from(path_to_string(&application).replace('\\', "/"));
    fs::write(
        applications.join("ai.opencode.desktop.desktop"),
        format!(
            "[Desktop Entry]\nName=OpenCode\nExec=\"{}\" %U\nType=Application\n",
            path_to_string(&application)
        ),
    )
    .unwrap();

    assert_eq!(
        find_linux_opencode_desktop_application_in_roots(&home, &[], &[data_directory], &[],),
        Some(application)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_linux_desktop_finds_an_unregistered_appimage() {
    let home = agent_test_home("opencode-linux-appimage-directory");
    let applications = home.join("Applications");
    let application = applications.join("opencode-desktop-linux-aarch64.AppImage");
    fs::create_dir_all(&applications).unwrap();
    fs::write(&application, []).unwrap();

    assert_eq!(
        find_linux_opencode_desktop_application_in_roots(&home, &[], &[], &[applications],),
        Some(application)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn opencode_linux_desktop_reads_versions_from_appimage_and_nix_paths() {
    assert_eq!(
        linux_opencode_desktop_version_from_path(Path::new(
            "/home/tester/Applications/opencode-desktop-1.18.16-linux-amd64.AppImage"
        ))
        .as_deref(),
        Some("1.18.16")
    );
    assert_eq!(
        linux_opencode_desktop_version_from_path(Path::new(
            "/nix/store/hash-opencode-desktop-1.18.16/bin/opencode-desktop"
        ))
        .as_deref(),
        Some("1.18.16")
    );
    assert_eq!(
        linux_opencode_desktop_version_from_path(Path::new(
            "/home/tester.2026/Downloads/opencode-desktop-linux-amd64.AppImage"
        )),
        None
    );
}

#[test]
fn opencode_agent_config_accepts_jsonc_and_preserves_comments() {
    let home = agent_test_home("opencode-jsonc");
    let path = home.join(".config/opencode/opencode.jsonc");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let existing = r#"{
        // Keep this comment when CPA updates the file.
        "provider": {
            "cpa-gui": {
                "options": {
                    "timeout": 30,
                },
            },
        },
    }"#;
    fs::write(&path, existing).unwrap();

    let rendered = build_opencode_agent_config(
        Some(existing),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        &test_agent_models(&["gpt-test"]),
    )
    .unwrap();
    assert!(rendered.contains("// Keep this comment"));
    let value: serde_json::Value = json5::from_str(&rendered).unwrap();
    assert_eq!(
        value["provider"][MANAGED_AGENT_PROVIDER_ID]["options"]["timeout"],
        30
    );

    fs::write(&path, &rendered).unwrap();
    assert_eq!(
        inspect_opencode_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("gpt-test".to_string()))
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn zcode_agent_config_preserves_other_providers_and_uses_anthropic_messages() {
    let home = agent_test_home("zcode-agent-config");
    let path = home.join(".zcode/v2/provider_config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut models = test_agent_models(&["gpt-test", "deepseek-test"]);
    models[0].context_window = Some(372_000);
    models[1].context_window = Some(272_000);
    let rendered = build_zcode_agent_config(
        Some(
            r#"{"schemaVersion":1,"config":{"providerConfigRules":{"providerRules":[{"providerId":"other","providerName":"Other","config":{"group":"standard-personal","personalModelIds":["other-model"]}},{"providerId":"cpa-gui","config":{"access":{"type":"api-key","apiKeyManagementUrl":"https://example.com/keys"}}}]},"modelConfigRules":{"providerModelRules":[{"providerId":"other","modelId":"other-model","config":{"enabled":true}},{"providerId":"cpa-gui","modelId":"gpt-test","config":{"properties":{"supportsToolCall":true}}}],"manualProviderModelRules":[]},"providerOrder":["other","cpa-gui"]}}"#,
        ),
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    fs::write(&path, &rendered).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(value["schemaVersion"], 1);
    let rules = value["config"]["providerConfigRules"]["providerRules"].as_array().unwrap();
    assert_eq!(rules[0]["providerId"], "other");
    let managed = rules.iter().find(|rule| rule["providerId"] == MANAGED_AGENT_PROVIDER_ID).unwrap();
    assert_eq!(managed["config"]["access"]["apiKeyManagementUrl"], "https://example.com/keys");
    assert_eq!(managed["config"]["access"]["apiKey"], DEFAULT_API_KEY);
    assert_eq!(managed["config"]["api"]["type"], "anthropic-messages");
    assert_eq!(managed["config"]["api"]["baseUrl"], "http://127.0.0.1:8317");
    assert_eq!(managed["config"]["personalModelIds"], serde_json::json!(["gpt-test", "deepseek-test"]));
    assert_eq!(value["config"]["defaultModelSelection"], serde_json::json!({
        "providerId": MANAGED_AGENT_PROVIDER_ID, "modelId": "gpt-test"
    }));
    let model_rules = value["config"]["modelConfigRules"]["providerModelRules"].as_array().unwrap();
    assert_eq!(model_rules[0]["providerId"], "other");
    assert_eq!(model_rules[1]["config"]["properties"]["supportsToolCall"], true);
    assert_eq!(model_rules[1]["config"]["properties"]["contextWindow"], 372_000);
    assert_eq!(model_rules[2]["config"]["properties"]["contextWindow"], 272_000);
    assert_eq!(
        inspect_zcode_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("gpt-test".to_string()))
    );

    let mut normalized = value;
    normalized["config"]["defaultModelSelection"]["providerId"] = serde_json::json!("other");
    fs::write(&path, serde_json::to_string_pretty(&normalized).unwrap()).unwrap();
    assert_eq!(
        inspect_zcode_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (false, None)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn zcode_desktop_and_cli_share_the_provider_configuration() {
    let home = agent_test_home("zcode-shared-provider-config");
    let paths = agent_config_paths(AgentClient::ZCode, &home);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], home.join(".zcode/v2/provider_config.json"));
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    let models = test_agent_models(&["gpt-test", "deepseek-test"]);
    let config = build_zcode_agent_config(
        None,
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    fs::write(&paths[0], &config).unwrap();
    assert_eq!(
        inspect_agent_managed_config(AgentClient::ZCode, &paths, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("gpt-test".to_string()), false)
    );

    let mut changed: serde_json::Value = serde_json::from_str(&config).unwrap();
    changed["config"]["defaultModelSelection"]["modelId"] = serde_json::json!("deepseek-test");
    fs::write(&paths[0], serde_json::to_string_pretty(&changed).unwrap()).unwrap();
    assert_eq!(
        inspect_agent_managed_config(AgentClient::ZCode, &paths, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("deepseek-test".to_string()), false)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn zcode_preserves_manual_model_rules_and_rejects_unknown_schema_versions() {
    let fixture: serde_json::Value = serde_json::from_str(
        include_str!("fixtures/zcode-provider-config-3.14.4.json"),
    ).unwrap();
    let existing = serde_json::json!({
        "schemaVersion": 1,
        "config": {
            "providerConfigRules": {"providerRules": []},
            "modelConfigRules": {
                "providerModelRules": [],
                "manualProviderModelRules": fixture["config"]["modelConfigRules"]["manualProviderModelRules"],
            },
        },
    });
    let mut models = test_agent_models(&["gpt-manual", "without-context"]);
    models[1].context_window = None;
    let rendered = build_zcode_agent_config(
        Some(&existing.to_string()),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-manual",
        &models,
    ).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert!(value["config"]["modelConfigRules"]["providerModelRules"]
        .as_array().unwrap().is_empty());
    let manual = &value["config"]["modelConfigRules"]["manualProviderModelRules"][0];
    assert_eq!(manual["config"]["properties"]["inputFormat"]["supportsImage"], true);
    assert_eq!(manual["config"]["properties"]["contextWindow"], 200_000);
    assert!(build_zcode_agent_config(
        Some(r#"{"schemaVersion":2,"config":{}}"#),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-manual",
        &test_agent_models(&["gpt-manual"]),
    ).is_err());
}

#[test]
fn zcode_latest_manual_rules_preserve_required_fields_without_runtime_context() {
    let existing = include_str!("fixtures/zcode-provider-config-3.14.4.json");
    let original: serde_json::Value = serde_json::from_str(existing).unwrap();
    let mut models = test_agent_models(&["gpt-manual", "gpt-test"]);
    for option in &mut models {
        option.context_window = None;
    }
    let rendered = build_zcode_agent_config(
        Some(existing),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-manual",
        &models,
    ).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(
        value["config"]["modelConfigRules"]["manualProviderModelRules"],
        original["config"]["modelConfigRules"]["manualProviderModelRules"],
    );
    assert_eq!(value["config"]["defaultModelSelection"], original["config"]["defaultModelSelection"]);
    let smart = &value["config"]["modelConfigRules"]["providerModelRules"][1];
    assert!(smart["config"]["properties"].get("contextWindow").is_none());
    assert_eq!(smart["config"]["properties"]["supportsToolCall"], true);
    assert_eq!(
        build_zcode_agent_config(
            Some(&rendered), "http://127.0.0.1:8317/v1", DEFAULT_API_KEY, "gpt-manual", &models,
        ).unwrap(),
        rendered,
    );
}

#[test]
fn zcode_latest_manual_rules_update_context_and_restore_without_losing_native_options() {
    let existing = include_str!("fixtures/zcode-provider-config-3.14.4.json");
    let original: serde_json::Value = serde_json::from_str(existing).unwrap();
    let mut models = test_agent_models(&["gpt-manual", "gpt-test"]);
    models[0].context_window = Some(372_000);
    let rendered = build_zcode_agent_config(
        Some(existing),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-manual",
        &models,
    ).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    let mut expected_manual = original["config"]["modelConfigRules"]["manualProviderModelRules"].clone();
    expected_manual[0]["config"]["properties"]["contextWindow"] = serde_json::json!(372_000);
    assert_eq!(value["config"]["modelConfigRules"]["manualProviderModelRules"], expected_manual);
    assert_eq!(value["config"]["defaultModelSelection"], original["config"]["defaultModelSelection"]);
    assert_eq!(
        value["config"]["providerConfigRules"]["providerRules"][0],
        original["config"]["providerConfigRules"]["providerRules"][0],
    );
    assert_eq!(
        value["config"]["providerConfigRules"]["providerRules"][1]["config"]["api"]["headers"],
        original["config"]["providerConfigRules"]["providerRules"][1]["config"]["api"]["headers"],
    );
    // Restoring CPA must not undo later edits made in ZCode to another provider.
    value["config"]["providerConfigRules"]["providerRules"][0]["providerName"] = serde_json::json!("Renamed in ZCode");
    let restored = build_restored_zcode_provider_config(
        &value.to_string(), Some(existing),
    ).unwrap().unwrap();
    let restored: serde_json::Value = serde_json::from_str(&restored).unwrap();
    let mut expected = original;
    expected["config"]["providerConfigRules"]["providerRules"][0]["providerName"] = serde_json::json!("Renamed in ZCode");
    assert_eq!(restored, expected);
}

#[test]
fn zcode_versioned_api_configuration_and_legacy_addresses_are_detected() {
    let home = agent_test_home("zcode-versioned-api");
    let paths = agent_config_paths(AgentClient::ZCode, &home);
    let updates = build_agent_updates(
        AgentClient::ZCode, &home, 8317, DEFAULT_API_KEY, "gpt-test",
        &test_agent_models(&["gpt-test"]), None,
    ).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&updates[0].after).unwrap();
    assert_eq!(
        value["config"]["providerConfigRules"]["providerRules"][0]["config"]["api"]["baseUrl"],
        "http://127.0.0.1:8317/v1",
    );
    let fresh = fresh_agent_contents(AgentClient::ZCode, 8317, DEFAULT_API_KEY, "gpt-test").unwrap();
    let fresh: serde_json::Value = serde_json::from_str(&fresh[0]).unwrap();
    assert_eq!(
        fresh["config"]["providerConfigRules"]["providerRules"][0]["config"]["api"]["baseUrl"],
        "http://127.0.0.1:8317/v1",
    );
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    for base_url in [
        "http://127.0.0.1:8317/v1",
        "http://127.0.0.1:8317/v1/",
        "http://127.0.0.1:8317",
        "http://127.0.0.1:8317/",
        "http://127.0.0.1:8318/v1",
        "http://127.0.0.1:8317/other/v1",
    ] {
        value["config"]["providerConfigRules"]["providerRules"][0]["config"]["api"]["baseUrl"] = serde_json::json!(base_url);
        fs::write(&paths[0], value.to_string()).unwrap();
        assert_eq!(
            inspect_zcode_agent_config(&paths[0], 8317, DEFAULT_API_KEY).unwrap(),
            (matches!(base_url.trim_end_matches('/'), "http://127.0.0.1:8317" | "http://127.0.0.1:8317/v1"), Some("gpt-test".to_string())),
            "{base_url}",
        );
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn zcode_inspection_requires_the_default_model_to_belong_to_the_managed_provider() {
    let home = agent_test_home("zcode-default-model-inspection");
    let path = home.join(".zcode/v2/provider_config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let rendered = build_zcode_agent_config(
        None,
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "gpt-test",
        &test_agent_models(&["gpt-test"]),
    )
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    value["config"]["defaultModelSelection"]["providerId"] = serde_json::json!("other");
    fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    assert_eq!(
        inspect_zcode_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (false, None)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn kimi_code_config_preserves_existing_tables_and_uses_openai_provider() {
    let home = agent_test_home("kimi-code-agent-config");
    let path = home.join(".kimi-code/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut models = test_agent_models(&["gpt-test", "deepseek-test"]);
    models[0].context_window = Some(1_048_576);
    models[1].context_window = Some(131_072);
    let rendered = build_kimi_code_agent_config(
        Some(
            r#"theme = "dark"

[providers.other]
type = "openai"
base_url = "https://example.com/v1"

[models."other/model"]
provider = "other"
model = "model"
"#,
        ),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    fs::write(&path, &rendered).unwrap();
    let value: toml::Value = toml::from_str(&rendered).unwrap();

    assert_eq!(value["theme"].as_str(), Some("dark"));
    assert_eq!(
        value["providers"]["other"]["base_url"].as_str(),
        Some("https://example.com/v1")
    );
    assert_eq!(value["default_model"].as_str(), Some("cpa-gui/gpt-test"));
    assert_eq!(
        value["providers"][MANAGED_AGENT_PROVIDER_ID]["type"].as_str(),
        Some("openai")
    );
    assert_eq!(
        value["providers"][MANAGED_AGENT_PROVIDER_ID]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert_eq!(
        value["models"]["cpa-gui/gpt-test"]["capabilities"]
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("tool_use")
    );
    assert_eq!(
        value["models"]["cpa-gui/gpt-test"]["max_context_size"].as_integer(),
        Some(1_048_576)
    );
    assert_eq!(
        value["models"]["cpa-gui/deepseek-test"]["max_context_size"].as_integer(),
        Some(131_072)
    );
    assert_eq!(
        inspect_kimi_code_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("gpt-test".to_string()))
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn grok_build_config_preserves_existing_models_and_uses_chat_completions() {
    let home = agent_test_home("grok-build-agent-config");
    let path = home.join(".grok/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut models = test_agent_models(&["gpt-test", "deepseek-test"]);
    models[0].context_window = Some(1_048_576);
    models[1].context_window = Some(131_072);
    let rendered = build_grok_build_agent_config(
        Some(
            r#"theme = "dark"

[models]
default = "other"

[model.other]
model = "other-model"
base_url = "https://example.com/v1"
api_key = "other-key"
"#,
        ),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    fs::write(&path, &rendered).unwrap();
    let value: toml::Value = toml::from_str(&rendered).unwrap();

    assert_eq!(value["theme"].as_str(), Some("dark"));
    assert_eq!(
        value["model"]["other"]["model"].as_str(),
        Some("other-model")
    );
    assert_eq!(
        value["models"]["default"].as_str(),
        Some("cpa-gui/gpt-test")
    );
    assert_eq!(
        value["model"]["cpa-gui/gpt-test"]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert_eq!(
        value["model"]["cpa-gui/gpt-test"]["api_backend"].as_str(),
        Some("chat_completions")
    );
    assert_eq!(
        value["model"]["cpa-gui/gpt-test"]["context_window"].as_integer(),
        Some(1_048_576)
    );
    assert_eq!(
        value["model"]["cpa-gui/deepseek-test"]["context_window"].as_integer(),
        Some(131_072)
    );
    assert_eq!(
        inspect_grok_build_agent_config(&path, 8317, DEFAULT_API_KEY).unwrap(),
        (true, Some("gpt-test".to_string()))
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn kimi_and_grok_toml_restore_preserves_runtime_fields_and_original_selection() {
    use toml_edit::{value, Document};

    let models = test_agent_models(&["gpt-test", "deepseek-test"]);
    let kimi_original = r#"default_model = "cpa-gui/original"

[providers.cpa-gui]
type = "openai_responses"
base_url = "https://original.example/v1"
api_key = "original-key"

[models."cpa-gui/original"]
provider = "cpa-gui"
model = "original"
"#;
    let kimi_managed = build_kimi_code_agent_config(
        Some(kimi_original),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    let mut kimi_runtime = kimi_managed.parse::<Document>().unwrap();
    kimi_runtime
        .as_table_mut()
        .insert("runtime_added", value(true));
    kimi_runtime["providers"][MANAGED_AGENT_PROVIDER_ID]
        .as_table_mut()
        .unwrap()
        .insert("runtime_custom", value("keep"));
    let kimi_restored =
        build_restored_kimi_code_config(&kimi_runtime.to_string(), Some(kimi_original))
            .unwrap()
            .unwrap();
    let kimi: toml::Value = toml::from_str(&kimi_restored).unwrap();
    assert_eq!(kimi["default_model"].as_str(), Some("cpa-gui/original"));
    assert_eq!(kimi["runtime_added"].as_bool(), Some(true));
    assert_eq!(
        kimi["providers"][MANAGED_AGENT_PROVIDER_ID]["type"].as_str(),
        Some("openai_responses")
    );
    assert_eq!(
        kimi["providers"][MANAGED_AGENT_PROVIDER_ID]["runtime_custom"].as_str(),
        Some("keep")
    );
    assert!(kimi["models"].get("cpa-gui/gpt-test").is_none());
    assert_eq!(
        kimi["models"]["cpa-gui/original"]["model"].as_str(),
        Some("original")
    );

    let grok_original = r#"[models]
default = "cpa-gui/original"

[model."cpa-gui/original"]
model = "original"
base_url = "https://original.example/v1"
api_key = "original-key"
"#;
    let grok_managed = build_grok_build_agent_config(
        Some(grok_original),
        "http://127.0.0.1:8317/v1",
        DEFAULT_API_KEY,
        "gpt-test",
        &models,
    )
    .unwrap();
    let mut grok_runtime = grok_managed.parse::<Document>().unwrap();
    grok_runtime
        .as_table_mut()
        .insert("runtime_added", value(true));
    let grok_restored =
        build_restored_grok_build_config(&grok_runtime.to_string(), Some(grok_original))
            .unwrap()
            .unwrap();
    let grok: toml::Value = toml::from_str(&grok_restored).unwrap();
    assert_eq!(grok["models"]["default"].as_str(), Some("cpa-gui/original"));
    assert_eq!(grok["runtime_added"].as_bool(), Some(true));
    assert!(grok["model"].get("cpa-gui/gpt-test").is_none());
    assert_eq!(
        grok["model"]["cpa-gui/original"]["base_url"].as_str(),
        Some("https://original.example/v1")
    );
}

#[test]
fn openclaw_agent_config_accepts_json5_and_preserves_unknown_fields() {
    let models = test_agent_models(&["gpt-test", "deepseek-test"]);
    let rendered = build_openclaw_agent_config(
            Some(
                "// keep this comment\n{ theme: 'dark', models: { mode: 'merge', providers: { 'cpa-gui': { custom: 'keep' } } } }",
            ),
            "http://127.0.0.1:8317/v1",
            DEFAULT_API_KEY,
            "gpt-test",
            &models,
        )
        .unwrap();
    let value: serde_json::Value = json5::from_str(&rendered).unwrap();

    assert!(rendered.contains("// keep this comment"));
    assert_eq!(value["theme"], "dark");
    assert_eq!(
        value["models"]["providers"][MANAGED_AGENT_PROVIDER_ID]["custom"],
        "keep"
    );
    assert_eq!(
        value["models"]["providers"][MANAGED_AGENT_PROVIDER_ID]["api"],
        "openai-completions"
    );
    assert_eq!(
        value["agents"]["defaults"]["model"]["primary"],
        "cpa-gui/gpt-test"
    );
    assert_eq!(
        value["models"]["providers"][MANAGED_AGENT_PROVIDER_ID]["models"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(value["agents"]["defaults"]["models"]["cpa-gui/gpt-test"].is_object());
    assert!(value["agents"]["defaults"]["models"]["cpa-gui/deepseek-test"].is_object());
}

#[test]
fn hermes_agent_config_preserves_unknown_fields_and_uses_current_schema() {
    let models = test_agent_models(&["gpt-test", "deepseek-test"]);
    let rendered = build_hermes_agent_config(
            Some("# keep this comment\ntheme: dark\ncustom_providers:\n  - name: other\n    base_url: https://example.com\n  - name: cpa-gui\n    custom: keep\n  - name: cpa-gui\n    duplicate: true\n  - broken: true\n"),
            "http://127.0.0.1:8317/v1",
            DEFAULT_API_KEY,
            "gpt-test",
            &models,
        )
        .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
    let providers = value["custom_providers"].as_sequence().unwrap();
    let managed = providers
        .iter()
        .find(|provider| provider["name"].as_str() == Some(MANAGED_AGENT_PROVIDER_ID))
        .unwrap();

    assert_eq!(value["theme"].as_str(), Some("dark"));
    assert!(rendered.contains("# keep this comment"));
    assert_eq!(providers.len(), 2);
    assert_eq!(managed["custom"].as_str(), Some("keep"));
    assert!(managed.get("duplicate").is_none());
    assert_eq!(managed["api_mode"].as_str(), Some("chat_completions"));
    assert_eq!(managed["model"].as_str(), Some("gpt-test"));
    assert!(managed["models"]["gpt-test"].is_mapping());
    assert!(managed["models"]["deepseek-test"].is_mapping());
    assert_eq!(
        value["model"]["provider"].as_str(),
        Some(MANAGED_AGENT_PROVIDER_ID)
    );
}

#[test]
fn agent_model_list_parser_exposes_aliases_as_selectable_model_ids() {
    let models = parse_agent_model_options(&serde_json::json!({
            "object": "list",
            "data": [
                {"id": "gpt-5", "display_name": "GPT 5", "context_length": 272000},
                {"name": "claude-sonnet", "alias": "claude-sonnet-xhigh", "fork": true, "contextLength": "1000000"},
                {"name": "hidden-original", "alias": "visible-alias", "ContextLength": 128000},
                "deepseek-chat",
                {"id": "GPT-5"},
                {"id": ""}
            ]
        }))
        .unwrap();

    assert_eq!(
        models,
        vec![
            AgentModelOption {
                input_modalities: None,
                harness_metadata: None,
                name: "gpt-5".to_string(),
                alias: Some("GPT 5".to_string()),
                is_alias: false,
                context_window: Some(272_000),
            },
            AgentModelOption {
                input_modalities: None,
                harness_metadata: None,
                name: "claude-sonnet".to_string(),
                alias: None,
                is_alias: false,
                context_window: Some(1_000_000),
            },
            AgentModelOption {
                input_modalities: None,
                harness_metadata: None,
                name: "claude-sonnet-xhigh".to_string(),
                alias: Some("claude-sonnet".to_string()),
                is_alias: true,
                context_window: Some(1_000_000),
            },
            AgentModelOption {
                input_modalities: None,
                harness_metadata: None,
                name: "visible-alias".to_string(),
                alias: Some("hidden-original".to_string()),
                is_alias: true,
                context_window: Some(128_000),
            },
            AgentModelOption {
                input_modalities: None,
                harness_metadata: None,
                name: "deepseek-chat".to_string(),
                alias: None,
                is_alias: false,
                context_window: None,
            },
        ]
    );
}

#[tokio::test]
async fn codex_configuration_uses_public_v1_model_ids_for_selection_and_files() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_string();
            let body = if path == "/v1/models" {
                assert!(request.to_lowercase().contains("authorization: bearer public-test-key\r\n"));
                r#"{"data":[{"id":"public-model","alias":"not-an-id","slug":"not-public","context_length":64000},{"id":"another-model"}]}"#
            } else {
                assert_eq!(path, "/v8/management/config.yaml");
                "config-version: 8\n"
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            paths.push(path);
        }
        assert!(paths.iter().any(|path| path == "/v1/models"));
    });
    let config = GuiConfigFile {
        port,
        management_secret_key: "management-test-key".into(),
        api_keys: vec![GuiApiKeyEntry { key: "public-test-key".into(), remark: String::new() }],
        ..GuiConfigFile::default()
    };
    let prepared = fetch_prepared_agent_models(AgentClient::Codex, &config).await.unwrap();
    server.join().unwrap();
    let mut ids: Vec<_> = prepared.models.iter().map(|model| model.name.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["another-model", "public-model"]);
    let catalog: serde_json::Value = serde_json::from_str(prepared.codex_catalog.as_deref().unwrap()).unwrap();
    let mut written: Vec<_> = catalog["models"].as_array().unwrap().iter().map(|model| model["slug"].as_str().unwrap()).collect();
    written.sort();
    assert_eq!(ids, written);
}

#[test]
fn omp_configuration_stores_credentials_and_preserves_unmanaged_settings() {
    let home = agent_test_home("omp-provider");
    let existing_models = "providers:\n  other:\n    baseUrl: https://example.test/v1\n    apiKey: KEEP_OTHER\n";
    let existing_settings = "shellPath: E:/software/toolchain/w64devkit/bin/bash.exe\nmodelRoles:\n  tiny: local/lfm2.5-230m\n";
    let models = build_omp_models_config(Some(existing_models), "http://127.0.0.1:8317/v1", "secret-value").unwrap();
    let settings = build_omp_settings_config(Some(existing_settings), "gpt-test").unwrap();
    let models_value: serde_norway::Value = serde_norway::from_str(&models).unwrap();
    let settings_value: serde_norway::Value = serde_norway::from_str(&settings).unwrap();
    let provider = &models_value["providers"]["easy-cliproxyapi"];
    assert_eq!(provider["baseUrl"].as_str(), Some("http://127.0.0.1:8317/v1"));
    assert_eq!(provider["apiKey"].as_str(), Some("secret-value"));
    assert_eq!(provider["api"].as_str(), Some("openai-responses"));
    assert_eq!(provider["discovery"]["type"].as_str(), Some("openai-models-list"));
    assert!(provider.get("models").is_none());
    assert_eq!(models_value["providers"]["other"]["apiKey"].as_str(), Some("KEEP_OTHER"));
    assert_eq!(settings_value["shellPath"].as_str(), Some("E:/software/toolchain/w64devkit/bin/bash.exe"));
    assert_eq!(settings_value["modelRoles"]["tiny"].as_str(), Some("local/lfm2.5-230m"));
    assert_eq!(settings_value["modelRoles"]["default"].as_str(), Some("easy-cliproxyapi/gpt-test"));

    let paths = agent_config_paths(AgentClient::Omp, &home);
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    fs::write(&paths[0], models).unwrap();
    fs::write(&paths[1], settings).unwrap();
    let (configured, model) = inspect_omp_agent_config(&paths, 8317, "secret-value").unwrap();
    assert!(configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    assert!(omp_has_managed_marker(&paths).unwrap());

    let removed = prepare_omp_managed_removal(&paths).unwrap();
    for (path, bytes) in removed {
        match bytes {
            Some(bytes) => fs::write(&path, bytes).unwrap(),
            None => fs::write(&path, "").unwrap(),
        }
    }
    let restored_models = fs::read_to_string(&paths[0]).unwrap();
    let restored_settings = fs::read_to_string(&paths[1]).unwrap();
    assert!(!restored_models.contains("easy-cliproxyapi"));
    assert!(restored_models.contains("KEEP_OTHER"));
    assert!(!restored_settings.contains("easy-cliproxyapi"));
    assert!(restored_settings.contains("shellPath"));
    assert!(restored_settings.contains("local/lfm2.5-230m"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn omp_legacy_environment_configuration_requires_update_and_migrates() {
    let home = agent_test_home("omp-legacy-credentials");
    let paths = agent_config_paths(AgentClient::Omp, &home);
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    let original = "providers:\n  other:\n    apiKey: KEEP_OTHER\n  easy-cliproxyapi:\n    baseUrl: http://127.0.0.1:8317/v1\n    apiKey: EASYCLIPROXYAPI_API_KEY\n    api: openai-responses\n    discovery:\n      type: openai-models-list\n";
    fs::write(&paths[0], original).unwrap();
    fs::write(&paths[1], build_omp_settings_config(None, "gpt-test").unwrap()).unwrap();

    let (configured, model) = inspect_omp_agent_config(&paths, 8317, "test-cpa-key").unwrap();
    assert!(!configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    assert_eq!(agent_connection_state(configured, true, omp_has_managed_marker(&paths)), "needs-update");

    let updated = build_omp_models_config(Some(original), "http://127.0.0.1:8317/v1", "test-cpa-key").unwrap();
    fs::write(&paths[0], &updated).unwrap();
    assert!(inspect_omp_agent_config(&paths, 8317, "test-cpa-key").unwrap().0);
    assert!(!updated.contains("EASYCLIPROXYAPI_API_KEY"));
    assert!(updated.contains("KEEP_OTHER"));
    assert_eq!(build_omp_models_config(Some(&updated), "http://127.0.0.1:8317/v1", "test-cpa-key").unwrap(), updated);

    let restored = build_restored_omp_models(&updated, Some(original)).unwrap().unwrap();
    let restored: serde_norway::Value = serde_norway::from_str(&restored).unwrap();
    let original: serde_norway::Value = serde_norway::from_str(original).unwrap();
    assert_eq!(restored, original);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn omp_detects_rotated_credentials_and_round_trips_yaml_sensitive_keys() {
    let home = agent_test_home("omp-rotated-credentials");
    let paths = agent_config_paths(AgentClient::Omp, &home);
    fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
    fs::write(&paths[1], build_omp_settings_config(None, "gpt-test").unwrap()).unwrap();
    let mut models = build_omp_models_config(None, "http://127.0.0.1:8317/v1", "old-test-key").unwrap();
    fs::write(&paths[0], &models).unwrap();
    assert!(!inspect_omp_agent_config(&paths, 8317, "new-test-key").unwrap().0);

    for key in ["new-test-key", "test: key # with 'quotes' and \"double quotes\"", "true", "0123"] {
        models = build_omp_models_config(Some(&models), "http://127.0.0.1:8317/v1", key).unwrap();
        fs::write(&paths[0], &models).unwrap();
        assert!(inspect_omp_agent_config(&paths, 8317, key).unwrap().0);
        assert!(!inspect_omp_agent_config(&paths, 8318, key).unwrap().0);
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
#[ignore = "requires an isolated live-test directory containing cpa-core/config.yaml"]
fn export_omp_live_configuration() {
    let root = PathBuf::from(env::var("EZCPA_OMP_LIVE_ROOT").expect("isolated test directory"));
    let config: serde_norway::Value = serde_norway::from_str(
        &fs::read_to_string(root.join("cpa-core/config.yaml")).unwrap(),
    ).expect("valid copied core configuration");
    let port = config["server"]["port"].as_u64().expect("test port") as u16;
    assert_eq!(port, 8320);
    let key = config["access"]["api-keys"][0].as_str().expect("configured test key");
    let directory = root.join("omp-agent");
    fs::create_dir_all(&directory).unwrap();
    let rendered = build_omp_models_config(None, &format!("http://127.0.0.1:{port}/v1"), key).unwrap();
    fs::write(directory.join("models.yml"), rendered).unwrap();
}

#[test]
fn kimi_grok_and_zcode_use_cpa_runtime_context_windows() {
    for client in [
        AgentClient::KimiCode,
        AgentClient::GrokBuild,
        AgentClient::ZCode,
    ] {
        assert!(agent_uses_cpa_runtime_context_windows(client));
    }
    for client in [
        AgentClient::ClaudeCode,
        AgentClient::ClaudeDesktop,
        AgentClient::Codex,
        AgentClient::OpenCode,
        AgentClient::OpenClaw,
        AgentClient::Hermes,
        AgentClient::DeepSeekHarness,
        AgentClient::Omp,
    ] {
        assert!(!agent_uses_cpa_runtime_context_windows(client));
    }
}

#[test]
fn desktop_application_binaries_are_not_used_for_cli_version_probes() {
    assert!(!should_probe_primary_agent_executable_version(
        AgentClient::ClaudeDesktop
    ));
    assert!(!should_probe_primary_agent_executable_version(
        AgentClient::ZCode
    ));
    assert!(should_probe_primary_agent_executable_version(
        AgentClient::ClaudeCode
    ));
    assert!(should_probe_primary_agent_executable_version(
        AgentClient::Codex
    ));
}

#[test]
fn claude_model_lists_mark_yaml_aliases_when_api_returns_only_model_ids() {
    let mut models = test_agent_models(&[
        "gpt-original",
        "gpt-high",
        "claude-opus-5",
        "oauth-original",
        "oauth-fast",
    ]);
    let yaml = r#"
codex-api-key:
  - api-key: test
    models:
      - name: gpt-original
      - name: gpt-original
        alias: gpt-high
      - name: gpt-original
        alias: claude-opus-5
oauth-model-alias:
  codex:
    - name: oauth-original
      alias: oauth-fast
      fork: true
"#;

    mark_configured_agent_model_aliases(&mut models, yaml).unwrap();

    let find = |name: &str| models.iter().find(|model| model.name == name).unwrap();
    assert!(!find("gpt-original").is_alias);
    assert_eq!(find("gpt-high").alias.as_deref(), Some("gpt-original"));
    assert!(find("gpt-high").is_alias);
    assert_eq!(find("claude-opus-5").alias.as_deref(), Some("gpt-original"));
    assert!(find("claude-opus-5").is_alias);
    assert_eq!(find("oauth-fast").alias.as_deref(), Some("oauth-original"));
    assert!(find("oauth-fast").is_alias);
}

#[test]
fn deepseek_harness_settings_preserve_unrelated_sections_and_publish_models() {
    let existing = r#"# keep this comment
ui-onboarding:
  welcomeNoticeVersion: 3
llm-pi-ai:
  providers:
    existing:
      api: openai-completions
      baseURL: https://example.invalid/v1
agent-default-model:
  provider: existing
  model: old-model
  reasoningEffort: high
"#;
    let models = vec![
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "gpt-selected".to_string(),
            alias: Some("Selected Model".to_string()),
            is_alias: false,
            context_window: Some(272_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "gpt-other".to_string(),
            alias: None,
            is_alias: false,
            context_window: None,
        },
    ];
    let rendered = build_deepseek_harness_settings(
        Some(existing),
        "http://127.0.0.1:8317/v1",
        "gpt-selected",
        &models,
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();

    assert!(rendered.contains("# keep this comment"));
    assert_eq!(
        value["ui-onboarding"]["welcomeNoticeVersion"].as_u64(),
        Some(3)
    );
    assert_eq!(
        value["llm-pi-ai"]["providers"]["existing"]["baseURL"].as_str(),
        Some("https://example.invalid/v1")
    );
    let provider = &value["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID];
    assert_eq!(
        provider["apiKeyEnv"].as_str(),
        Some(DEEPSEEK_HARNESS_CREDENTIAL)
    );
    assert_eq!(provider["api"].as_str(), Some("openai-completions"));
    assert_eq!(
        provider["baseURL"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert_eq!(provider["models"][0]["id"].as_str(), Some("gpt-selected"));
    assert_eq!(
        provider["models"][0]["name"].as_str(),
        Some("Selected Model")
    );
    assert_eq!(
        provider["models"][0]["contextWindow"].as_u64(),
        Some(272_000)
    );
    assert_eq!(provider["models"][1]["id"].as_str(), Some("gpt-other"));
    assert_eq!(
        value["agent-default-model"]["provider"].as_str(),
        Some(DEEPSEEK_HARNESS_PROVIDER_ID)
    );
    assert_eq!(
        value["agent-default-model"]["model"].as_str(),
        Some("gpt-selected")
    );
    assert!(value["agent-default-model"]
        .get("reasoningEffort")
        .is_none());
}

#[test]
fn deepseek_harness_credentials_preserve_other_entries() {
    let rendered = build_deepseek_harness_credentials(
        Some(
            "# existing key\nversion: 1\nrefs:\n  OTHER_API_KEY: other-secret\nrecords:\n  plugin/account:\n    kind: api-key\n    key: record-secret\n",
        ),
        "cpa-secret",
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();

    assert!(rendered.contains("# existing key"));
    assert_eq!(value["version"].as_u64(), Some(1));
    assert_eq!(
        value["refs"]["OTHER_API_KEY"].as_str(),
        Some("other-secret")
    );
    assert_eq!(
        value["refs"][DEEPSEEK_HARNESS_CREDENTIAL].as_str(),
        Some("cpa-secret")
    );
    assert_eq!(
        value["records"]["plugin/account"]["key"].as_str(),
        Some("record-secret")
    );
    assert!(value.get(DEEPSEEK_HARNESS_CREDENTIAL).is_none());
}

#[test]
fn deepseek_harness_credentials_reject_legacy_flat_layout() {
    let error = build_deepseek_harness_credentials(
        Some("# legacy flat layout\nOTHER_API_KEY: other-secret\n"),
        "cpa-secret",
    )
    .unwrap_err();

    assert!(error.contains("missing the version field"), "{error}");
}

#[test]
fn deepseek_harness_credentials_repair_misplaced_managed_key() {
    let rendered = build_deepseek_harness_credentials(
        Some(
            "version: 1\nrefs:\n  OTHER_API_KEY: other-secret\nrecords: {}\nEASYCLIPROXYAPI_API_KEY: misplaced-secret\n",
        ),
        "cpa-secret",
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();

    assert_eq!(
        value["refs"][DEEPSEEK_HARNESS_CREDENTIAL].as_str(),
        Some("cpa-secret")
    );
    assert!(value.get(DEEPSEEK_HARNESS_CREDENTIAL).is_none());
}

#[test]
fn deepseek_harness_removal_preserves_other_versioned_credentials() {
    let home = agent_test_home("deepseek-harness-remove-versioned-credentials");
    let paths = vec![home.join("settings.yaml"), home.join(".credentials.yaml")];
    fs::create_dir_all(&home).unwrap();
    fs::write(
        &paths[1],
        "version: 1\nrefs:\n  OTHER_API_KEY: other-secret\n  EASYCLIPROXYAPI_API_KEY: cpa-secret\nrecords:\n  plugin/account:\n    kind: api-key\n    key: record-secret\n",
    )
    .unwrap();

    let changed = remove_agent_managed_configuration(AgentClient::DeepSeekHarness, &paths).unwrap();
    let value: serde_norway::Value =
        serde_norway::from_str(&fs::read_to_string(&paths[1]).unwrap()).unwrap();

    assert_eq!(changed, vec![path_to_string(&paths[1])]);
    assert_eq!(value["version"].as_u64(), Some(1));
    assert_eq!(
        value["refs"]["OTHER_API_KEY"].as_str(),
        Some("other-secret")
    );
    assert!(value["refs"].get(DEEPSEEK_HARNESS_CREDENTIAL).is_none());
    assert_eq!(
        value["records"]["plugin/account"]["key"].as_str(),
        Some("record-secret")
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deepseek_harness_removal_deletes_semantically_empty_credentials_file() {
    let home = agent_test_home("deepseek-harness-remove-empty-credentials");
    let paths = vec![home.join("settings.yaml"), home.join(".credentials.yaml")];
    fs::create_dir_all(&home).unwrap();
    fs::write(
        &paths[1],
        build_deepseek_harness_credentials(None, "cpa-secret").unwrap(),
    )
    .unwrap();

    let changed = remove_agent_managed_configuration(AgentClient::DeepSeekHarness, &paths).unwrap();

    assert_eq!(changed, vec![path_to_string(&paths[1])]);
    assert!(!paths[1].exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deepseek_harness_inspection_requires_managed_route_selection_and_credential() {
    let home = agent_test_home("deepseek-harness-inspection");
    let paths = vec![home.join("settings.yaml"), home.join(".credentials.yaml")];
    let models = test_agent_models(&["gpt-test"]);
    fs::write(
        &paths[0],
        build_deepseek_harness_settings(None, "http://127.0.0.1:8317/v1", "gpt-test", &models)
            .unwrap(),
    )
    .unwrap();
    fs::write(
        &paths[1],
        build_deepseek_harness_credentials(None, "agent-key").unwrap(),
    )
    .unwrap();

    let (configured, model) = inspect_deepseek_harness_config(&paths, 8317, "agent-key").unwrap();
    assert!(configured);
    assert_eq!(model.as_deref(), Some("gpt-test"));
    assert!(deepseek_harness_has_managed_marker(&paths).unwrap());

    let (wrong_key, _) = inspect_deepseek_harness_config(&paths, 8317, "different-key").unwrap();
    assert!(!wrong_key);

    let credentials = fs::read_to_string(&paths[1]).unwrap();
    let credentials = render_agent_yaml_mapping_update(
        Some(&credentials),
        "test invalid Harness credentials",
        |root| {
            root.insert(
                yaml_key(DEEPSEEK_HARNESS_CREDENTIAL),
                serde_norway::Value::String("misplaced-key".to_string()),
            );
            Ok(())
        },
    )
    .unwrap();
    fs::write(&paths[1], credentials).unwrap();
    let (invalid_layout, _) = inspect_deepseek_harness_config(&paths, 8317, "agent-key").unwrap();
    assert!(!invalid_layout);
    assert!(deepseek_harness_has_managed_marker(&paths).unwrap());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deepseek_harness_profile_version_comes_from_installed_profile() {
    let home = agent_test_home("deepseek-harness-version");
    let package = home.join(".dsh/profiles/node_modules/@deepseek-ai/dsh/package.json");
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    fs::write(&package, r#"{"version":"0.1.0-rc.5"}"#).unwrap();

    assert_eq!(
        read_deepseek_harness_profile_version(&home).as_deref(),
        Some("0.1.0-rc.5")
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn claude_desktop_aliases_expose_role_routes_only() {
    let input = "openai-compatibility:\n  - name: CPA\n    base-url: https://example.com/v1\n    models:\n      - name: gpt-5.6-sol\n";
    let mappings = ClaudeDesktopModelMappings {
        opus: "gpt-5.6-sol".to_string(),
        sonnet: "gpt-5.6-sol".to_string(),
        haiku: "gpt-5.6-sol".to_string(),
        opus_1m: false,
        sonnet_1m: false,
        haiku_1m: false,
        ..ClaudeDesktopModelMappings::all("")
    };
    let available_models = test_agent_models(&["gpt-5.6-sol"]);
    let rendered =
        ensure_claude_desktop_model_aliases_in_yaml(input, &mappings, &available_models).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    let providers = yaml_mapping_value(root, "openai-compatibility")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();
    let models = yaml_mapping_value(providers[0].as_mapping().unwrap(), "models")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();

    assert_eq!(models.len(), 4);
    assert_eq!(
        configured_model_identity(&models[1]).unwrap().1,
        CLAUDE_DESKTOP_OPUS_MODEL_ID
    );
    assert_eq!(
        configured_model_identity(&models[2]).unwrap().1,
        CLAUDE_DESKTOP_SONNET_MODEL_ID
    );
    assert_eq!(
        configured_model_identity(&models[3]).unwrap().1,
        CLAUDE_DESKTOP_HAIKU_MODEL_ID
    );
    assert_eq!(
        configured_model_identity(&models[1]).unwrap().2.as_deref(),
        Some(MANAGED_CLAUDE_OPUS_ALIAS_DISPLAY_NAME)
    );
    assert_eq!(
        configured_model_identity(&models[2]).unwrap().2.as_deref(),
        Some(MANAGED_CLAUDE_SONNET_ALIAS_DISPLAY_NAME)
    );
    assert_eq!(
        configured_model_identity(&models[3]).unwrap().2.as_deref(),
        Some(MANAGED_CLAUDE_HAIKU_ALIAS_DISPLAY_NAME)
    );
    assert!(!rendered.contains("models: [{"));
    assert!(rendered.contains("\n      - name: gpt-5.6-sol\n"));
    assert!(rendered.contains(&format!("\n        alias: {CLAUDE_DESKTOP_OPUS_MODEL_ID}\n")));
    assert!(
        rendered.contains("\n        display-name: EasyCLIProxyAPI managed Claude Opus mapping\n")
    );
    assert_eq!(
        ensure_claude_desktop_model_aliases_in_yaml(&rendered, &mappings, &available_models,)
            .unwrap(),
        rendered
    );
}

#[test]
fn claude_desktop_aliases_support_codex_oauth_models() {
    let input = "port: 8317\n";
    let mappings = ClaudeDesktopModelMappings::all("gpt-5.6-sol");
    let available_models = test_agent_models(&["gpt-5.6-sol"]);
    let definitions = vec![CodexModelDefinition {
        id: "gpt-5.6-sol".to_string(),
        display_name: None,
        description: None,
        context_window: Some(372_000),
        reasoning_levels: vec!["high".to_string()],
        supports_tools: Some(true),
    }];

    let rendered = ensure_claude_desktop_model_aliases_with_codex_oauth_in_yaml(
        input,
        &mappings,
        &available_models,
        &definitions,
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    let aliases = yaml_mapping_value(root, "oauth-model-alias")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|channels| yaml_mapping_value(channels, "codex"))
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();

    assert_eq!(aliases.len(), 3);
    assert!(aliases.iter().all(|entry| {
        configured_model_identity(entry).is_some_and(|(source, _, _)| source == "gpt-5.6-sol")
    }));
    assert!(aliases.iter().all(|entry| {
        entry
            .as_mapping()
            .and_then(|entry| yaml_mapping_value(entry, "fork"))
            == Some(&serde_norway::Value::Bool(true))
    }));
    assert_eq!(
        ensure_claude_desktop_model_aliases_with_codex_oauth_in_yaml(
            &rendered,
            &mappings,
            &available_models,
            &definitions,
        )
        .unwrap(),
        rendered
    );

    let cleaned = remove_managed_claude_model_aliases_in_yaml(&rendered).unwrap();
    let cleaned: serde_norway::Value = serde_norway::from_str(&cleaned).unwrap();
    let cleaned = cleaned.as_mapping().unwrap();
    assert!(yaml_mapping_value(cleaned, "oauth-model-alias").is_none());
    assert_eq!(
        yaml_mapping_value(cleaned, "port").and_then(serde_norway::Value::as_i64),
        Some(8317)
    );
}

#[test]
fn claude_desktop_aliases_support_claude_oauth_models() {
    let input = "port: 8317\n";
    let mappings = ClaudeDesktopModelMappings::all("claude-sonnet-4-6");
    let available_models = test_agent_models(&["claude-sonnet-4-6"]);
    let definitions = vec![OAuthModelDefinitions {
        channel: oauth_alias_channel("claude").unwrap(),
        models: vec![CodexModelDefinition {
            id: "claude-sonnet-4-6".to_string(),
            display_name: None,
            description: None,
            context_window: Some(200_000),
            reasoning_levels: vec!["high".to_string()],
            supports_tools: Some(true),
        }],
    }];

    let rendered = ensure_claude_desktop_model_aliases_with_oauth_definitions_in_yaml(
        input,
        &mappings,
        &available_models,
        &definitions,
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    assert!(yaml_mapping_value(root, "oauth-model-alias").is_none());
    assert_eq!(
        ensure_claude_desktop_model_aliases_with_oauth_definitions_in_yaml(
            &rendered,
            &mappings,
            &available_models,
            &definitions,
        )
        .unwrap(),
        rendered
    );
}

#[test]
fn claude_desktop_aliases_support_xai_oauth_models() {
    let input = "port: 8317\n";
    let mappings = ClaudeDesktopModelMappings::all("grok-4");
    let available_models = test_agent_models(&["grok-4"]);
    let definitions = vec![OAuthModelDefinitions {
        channel: oauth_alias_channel("xai").unwrap(),
        models: vec![CodexModelDefinition {
            id: "grok-4".to_string(),
            display_name: None,
            description: None,
            context_window: Some(131_072),
            reasoning_levels: vec!["high".to_string()],
            supports_tools: Some(true),
        }],
    }];

    let rendered = ensure_claude_desktop_model_aliases_with_oauth_definitions_in_yaml(
        input,
        &mappings,
        &available_models,
        &definitions,
    )
    .unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    let aliases = yaml_mapping_value(root, "oauth-model-alias")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|channels| yaml_mapping_value(channels, "xai"))
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();

    assert_eq!(aliases.len(), 3);
    assert!(aliases.iter().all(|entry| {
        configured_model_identity(entry).is_some_and(|(source, _, _)| source == "grok-4")
    }));
    assert!(aliases.iter().all(|entry| {
        entry
            .as_mapping()
            .and_then(|entry| yaml_mapping_value(entry, "fork"))
            == Some(&serde_norway::Value::Bool(true))
    }));
}

#[test]
fn claude_oauth_alias_changes_use_the_runtime_refresh_endpoint() {
    let current = "port: 8317\nopenai-compatibility: []\n";
    let oauth_only = "port: 8317\nopenai-compatibility: []\noauth-model-alias:\n  codex:\n    - name: gpt-5.6-sol\n      alias: claude-opus-5\n      fork: true\n";
    let changes = management_alias_config_changes(current, oauth_only).unwrap();

    assert!(!changes.update_config_yaml);
    assert_eq!(
        changes.oauth_model_aliases,
        Some(serde_json::json!({
            "codex": [{
                "name": "gpt-5.6-sol",
                "alias": "claude-opus-5",
                "fork": true,
            }]
        }))
    );

    let mixed = oauth_only.replace(
        "openai-compatibility: []",
        "openai-compatibility:\n  - name: CPA\n    base-url: https://example.com/v1",
    );
    let changes = management_alias_config_changes(current, &mixed).unwrap();
    assert!(changes.update_config_yaml);
    assert!(changes.oauth_model_aliases.is_some());

    let changes = management_alias_config_changes(oauth_only, current).unwrap();
    assert!(!changes.update_config_yaml);
    assert_eq!(changes.oauth_model_aliases, Some(serde_json::json!({})));
}

#[test]
fn claude_managed_aliases_expand_flow_yaml_and_are_removed_selectively() {
    let compact = "# keep this comment\ncodex-api-key: [{api-key: test, models: [{name: gpt-5.6-sol, alias: ''}]}]\ncredential-concurrency: {max-limit: 1000000, cleanup-interval: 5s}\n";
    let mappings = ClaudeDesktopModelMappings::all("gpt-5.6-sol");
    let available_models = test_agent_models(&["gpt-5.6-sol"]);
    let rendered =
        ensure_claude_desktop_model_aliases_in_yaml(compact, &mappings, &available_models).unwrap();

    assert!(rendered.starts_with("# keep this comment\n"));
    assert!(!rendered.contains("codex-api-key: ["));
    assert!(!rendered.contains("credential-concurrency: {"));
    assert!(rendered.contains("codex-api-key:\n  - api-key: test\n"));
    assert!(rendered.contains("    models:\n      - name: gpt-5.6-sol\n"));

    let cleaned = remove_managed_claude_model_aliases_in_yaml(&rendered).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&cleaned).unwrap();
    let root = value.as_mapping().unwrap();
    let providers = yaml_mapping_value(root, "codex-api-key")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();
    let models = yaml_mapping_value(providers[0].as_mapping().unwrap(), "models")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(
        configured_model_identity(&models[0]).unwrap().0,
        "gpt-5.6-sol"
    );
}

#[test]
fn claude_alias_cleanup_does_not_delete_user_owned_routes() {
    let input = format!(
            "codex-api-key:\n  - api-key: test\n    models:\n      - name: user-opus\n        alias: {opus}\n        display-name: User managed route\n      - name: app-sonnet\n        alias: {sonnet}\n        display-name: {managed_sonnet}\n",
            opus = CLAUDE_DESKTOP_OPUS_MODEL_ID,
            sonnet = CLAUDE_DESKTOP_SONNET_MODEL_ID,
            managed_sonnet = MANAGED_CLAUDE_SONNET_ALIAS_DISPLAY_NAME,
        );

    let cleaned = remove_managed_claude_model_aliases_in_yaml(&input).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&cleaned).unwrap();
    let root = value.as_mapping().unwrap();
    assert_eq!(
        configured_model_client_identity(root, CLAUDE_DESKTOP_OPUS_MODEL_ID)
            .unwrap()
            .0,
        "user-opus"
    );
    assert!(configured_model_client_identity(root, CLAUDE_DESKTOP_SONNET_MODEL_ID).is_none());
}

#[test]
fn claude_alias_cleanup_preserves_user_owned_oauth_routes() {
    let input = format!(
        "oauth-model-alias:\n  codex:\n    - name: user-opus\n      alias: {opus}\n      fork: true\n    - name: app-sonnet\n      alias: {sonnet}\n      fork: true\n      display-name: {managed_sonnet}\n",
        opus = CLAUDE_DESKTOP_OPUS_MODEL_ID,
        sonnet = CLAUDE_DESKTOP_SONNET_MODEL_ID,
        managed_sonnet = MANAGED_CLAUDE_SONNET_ALIAS_DISPLAY_NAME,
    );

    let cleaned = remove_managed_claude_model_aliases_in_yaml(&input).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&cleaned).unwrap();
    let root = value.as_mapping().unwrap();
    assert_eq!(
        configured_model_client_identity(root, CLAUDE_DESKTOP_OPUS_MODEL_ID)
            .unwrap()
            .0,
        "user-opus"
    );
    assert!(configured_model_client_identity(root, CLAUDE_DESKTOP_SONNET_MODEL_ID).is_none());
}

#[test]
fn claude_legacy_aliases_are_adopted_and_moved_to_the_selected_model() {
    let input = format!(
            "codex-api-key:\n  - api-key: test\n    models:\n      - name: old-model\n      - name: new-model\n      - name: old-model\n        alias: {opus}\n",
            opus = CLAUDE_DESKTOP_OPUS_MODEL_ID,
        );
    let mappings = ClaudeDesktopModelMappings::all("new-model");
    let models = test_agent_models(&["old-model", "new-model"]);

    let rendered = ensure_claude_desktop_model_aliases_in_yaml(&input, &mappings, &models).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    assert_eq!(
        configured_model_client_identity(root, CLAUDE_DESKTOP_OPUS_MODEL_ID)
            .unwrap()
            .0,
        "new-model"
    );
    assert!(rendered.contains(MANAGED_CLAUDE_OPUS_ALIAS_DISPLAY_NAME));
}

#[test]
fn claude_desktop_uses_selected_alias_directly_with_original_context() {
    let input = "openai-compatibility:\n  - name: CPA\n    base-url: https://example.com/v1\n    models:\n      - name: gpt-original\n      - name: gpt-original\n        alias: gpt-high\n      - name: gpt-original\n        alias: claude-opus-5\n        display-name: EasyCLIProxyAPI managed Claude Opus mapping\n";
    let mappings = ClaudeDesktopModelMappings {
        opus: "gpt-high".to_string(),
        sonnet: "gpt-high".to_string(),
        haiku: "gpt-high".to_string(),
        opus_1m: false,
        sonnet_1m: true,
        haiku_1m: false,
        ..ClaudeDesktopModelMappings::all("")
    };
    let models = vec![
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "gpt-original".to_string(),
            alias: None,
            is_alias: false,
            context_window: Some(1_000_000),
        },
        AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: "gpt-high".to_string(),
            alias: Some("gpt-original".to_string()),
            is_alias: true,
            context_window: Some(128_000),
        },
    ];

    let rendered = ensure_claude_desktop_model_aliases_in_yaml(input, &mappings, &models).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
    let root = value.as_mapping().unwrap();
    let providers = yaml_mapping_value(root, "openai-compatibility")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();
    let configured_models = yaml_mapping_value(providers[0].as_mapping().unwrap(), "models")
        .and_then(serde_norway::Value::as_sequence)
        .unwrap();
    let client_models = configured_models
        .iter()
        .filter_map(configured_model_identity)
        .map(|(_, client_model, _)| client_model)
        .collect::<Vec<_>>();
    assert_eq!(client_models, vec!["gpt-original", "gpt-high"]);

    let profile = build_claude_desktop_profile(
        None,
        "http://127.0.0.1:8317",
        DEFAULT_API_KEY,
        "gpt-high",
        &models,
        Some(&mappings),
    )
    .unwrap();
    let profile: serde_json::Value = serde_json::from_str(&profile).unwrap();
    assert_eq!(
        profile["inferenceModels"],
        serde_json::json!([{
            "name": "gpt-high",
            "anthropicFamilyTier": "sonnet",
            "isFamilyDefault": true,
            "supports1m": true,
            "prefer1m": true
        }])
    );
}

#[test]
fn claude_desktop_role_mappings_can_use_different_available_models() {
    let models = test_agent_models(&["gpt-5.6-sol", "deepseek-chat", "gemini-3-pro"]);
    let mappings = resolve_claude_desktop_model_mappings(
        AgentClient::ClaudeDesktop,
        &models,
        "gpt-5.6-sol",
        Some(ClaudeDesktopModelMappings {
            opus: "gpt-5.6-sol".to_string(),
            sonnet: "deepseek-chat".to_string(),
            haiku: "gemini-3-pro".to_string(),
            opus_1m: true,
            sonnet_1m: false,
            haiku_1m: true,
            ..ClaudeDesktopModelMappings::all("")
        }),
    )
    .unwrap()
    .unwrap();

    assert_eq!(mappings.opus, "gpt-5.6-sol");
    assert_eq!(mappings.sonnet, "deepseek-chat");
    assert_eq!(mappings.haiku, "gemini-3-pro");
    assert!(mappings.opus_1m);
    assert!(!mappings.sonnet_1m);
    assert!(mappings.haiku_1m);
    assert!(resolve_claude_desktop_model_mappings(
        AgentClient::ClaudeCode,
        &models,
        "gpt-5.6-sol",
        None,
    )
    .unwrap()
    .is_none());
}

#[test]
fn agent_model_list_parser_rejects_unexpected_response_shape() {
    assert!(parse_agent_model_options(&serde_json::json!({"data": null})).is_err());
}

#[test]
fn detected_agent_version_requires_a_real_version_value() {
    assert_eq!(
        normalize_detected_agent_version("  opencode 1.2.3  ").as_deref(),
        Some("opencode 1.2.3")
    );
    assert_eq!(
        normalize_detected_agent_version("Claude Code v4").as_deref(),
        Some("Claude Code v4")
    );
    assert!(normalize_detected_agent_version("").is_none());
    assert!(normalize_detected_agent_version("version unknown").is_none());
    assert!(normalize_detected_agent_version("1.2.3\0invalid").is_none());
    assert!(normalize_detected_agent_version(&"1".repeat(257)).is_none());
}

#[test]
fn agent_command_path_prioritizes_the_executable_and_preserves_discovered_paths() {
    let home = agent_test_home("command-path");
    let executable_directory = home.join("node-runtime/bin");
    let executable = executable_directory.join("pi");
    fs::create_dir_all(&executable_directory).unwrap();

    let path = agent_command_path(&home, &executable).unwrap();
    let directories = env::split_paths(&path).collect::<Vec<_>>();

    assert_eq!(directories.first(), Some(&executable_directory));
    assert!(directories.contains(&home.join(".local/bin")));
    if let Some(inherited_path) = env::var_os("PATH") {
        for inherited_directory in env::split_paths(&inherited_path) {
            assert!(directories.contains(&inherited_directory));
        }
    }

    fs::remove_dir_all(home).unwrap();
}

#[test]
fn agent_version_probe_exposes_the_executable_directory_to_child_commands() {
    let home = agent_test_home("version-path");
    let executable_directory = home.join("node-runtime/bin");
    fs::create_dir_all(&executable_directory).unwrap();

    #[cfg(target_os = "windows")]
    let (executable, helper) = (
        executable_directory.join("pi.cmd"),
        executable_directory.join("agent-version-helper.cmd"),
    );
    #[cfg(not(target_os = "windows"))]
    let (executable, helper) = (
        executable_directory.join("pi"),
        executable_directory.join("agent-version-helper"),
    );

    #[cfg(target_os = "windows")]
    {
        fs::write(&executable, "@echo off\r\nagent-version-helper.cmd\r\n").unwrap();
        fs::write(&helper, "@echo off\r\necho pi 1.2.3\r\n").unwrap();
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::write(&executable, "#!/usr/bin/env sh\nagent-version-helper\n").unwrap();
        fs::write(&helper, "#!/usr/bin/env sh\nprintf 'pi 1.2.3\\n'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert_eq!(
        read_agent_version(&executable, &home).as_deref(),
        Some("pi 1.2.3")
    );

    fs::remove_dir_all(home).unwrap();
}

#[test]
fn agent_probe_command_captures_output_before_timeout() {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new(windows_command_processor());
        command.args(["/D", "/C", "echo agent 1.2.3"]);
        configure_background_command(&mut command);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'agent 1.2.3\\n'"]);
        command
    };

    let output = command_output_with_timeout(&mut command, Duration::from_secs(2))
        .unwrap()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "agent 1.2.3"
    );
}

#[test]
fn agent_probe_command_stops_at_timeout() {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new(windows_powershell_executable());
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 5",
        ]);
        configure_background_command(&mut command);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        command
    };

    let started = Instant::now();
    let output = command_output_with_timeout(&mut command, Duration::from_millis(100)).unwrap();

    assert!(output.is_none());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn codex_model_list_is_empty_when_cpa_has_no_writable_models() {
    let prepared = prepare_codex_agent_models(&[]).unwrap();

    assert!(prepared.models.is_empty());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(prepared.codex_catalog.as_deref().unwrap())
            .unwrap(),
        serde_json::json!({"models": []})
    );
}

#[test]
fn agent_model_validation_only_accepts_models_in_current_list() {
    let models = vec![AgentModelOption {
        input_modalities: None,
        harness_metadata: None,
        name: "gpt-5.4".to_string(),
        alias: Some("GPT 5.4".to_string()),
        is_alias: false,
        context_window: None,
    }];

    assert_eq!(
        resolve_available_agent_model(&models, "GPT-5.4").unwrap(),
        "gpt-5.4"
    );
    assert!(resolve_available_agent_model(&models, "removed-model").is_err());
    assert!(resolve_available_agent_model(&[], "gpt-5.4").is_err());
}

#[test]
fn thinking_alias_sources_only_include_current_core_models() {
    let input = "codex-api-key:\n  - name: Codex API\n    api-key: test\n    models:\n      - name: config-only\nopenai-compatibility:\n  - name: DeepSeek\n    base-url: https://api.deepseek.com\n    api-key-entries:\n      - api-key: test\n    models:\n      - name: DeepSeek-Chat\n";
    let definitions = parse_codex_model_definitions(&serde_json::json!({
        "models": [
            {
                "id": "gpt-runtime",
                "thinking": { "levels": ["low", "high"] }
            },
            {
                "id": "gpt-built-in-only",
                "thinking": { "levels": ["low", "high"] }
            }
        ]
    }))
    .unwrap();
    let available_models = test_agent_models(&["GPT-RUNTIME", "deepseek-chat"]);

    let sources = resolved_thinking_alias_sources(input, &definitions, &available_models).unwrap();
    let source_models = sources
        .iter()
        .map(|source| source.source.model.as_str())
        .collect::<Vec<_>>();

    assert_eq!(source_models, vec!["DeepSeek-Chat", "gpt-runtime"]);
    assert!(!source_models.contains(&"gpt-built-in-only"));
    assert!(!source_models.contains(&"config-only"));
}

#[test]
fn thinking_alias_prefers_codex_api_key_model_over_same_named_oauth_definition() {
    let input = "codex-api-key:\n  - name: CPA\n    api-key: test\n    models:\n      - name: gpt-5.6-luna\n";
    let definitions = parse_codex_model_definitions(&serde_json::json!({
        "models": [{
            "id": "gpt-5.6-luna",
            "thinking": { "levels": ["low", "high", "xhigh"] }
        }]
    }))
    .unwrap();
    let available_models = test_agent_models(&["gpt-5.6-luna"]);

    let sources = resolved_thinking_alias_sources(input, &definitions, &available_models).unwrap();

    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source.model, "gpt-5.6-luna");
    assert_eq!(sources[0].source.kind, "codex-api");
    assert!(matches!(
        sources[0].location,
        ThinkingAliasSourceLocation::ConfigModel {
            section: "codex-api-key",
            ..
        }
    ));

    let rendered =
        add_model_alias_to_yaml(input, &sources[0], "gpt-5.6-luna-xhigh", "xhigh", false).unwrap();
    assert!(rendered.contains("alias: gpt-5.6-luna-xhigh"), "{rendered}");
    assert!(!rendered.contains("oauth-model-alias"), "{rendered}");
}

#[cfg(target_os = "windows")]
#[test]
fn windows_batch_agent_commands_use_call_without_embedded_quotes() {
    let executable = Path::new(r"C:\工具 目录\opencode.cmd");
    let command = windows_command_for_executable(executable, true);
    let args = command
        .get_args()
        .take(3)
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        Path::new(command.get_program())
            .file_name()
            .and_then(|name| name.to_str()),
        Some("cmd.exe")
    );
    assert_eq!(
        args,
        vec!["/D".to_string(), "/K".to_string(), "call".to_string(),]
    );
    assert_eq!(
        windows_batch_executable_argument(executable),
        r#""C:\工具 目录\opencode.cmd""#
    );

    let batch = windows_command_for_executable(Path::new(r"C:\tools\agent.bat"), false);
    let batch_args = batch
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(batch_args[1], "/C");
    assert_eq!(batch_args[2], "call");

    let native = windows_command_for_executable(Path::new(r"C:\tools\agent.exe"), true);
    assert_eq!(
        native.get_program().to_string_lossy(),
        r"C:\tools\agent.exe"
    );
    assert_eq!(native.get_args().count(), 0);
}

#[test]
fn claude_code_fable_mapping_is_independent_and_round_trips() {
    let directory = agent_test_home("fable-mapping");
    let path = directory.join("settings.json");
    let models = test_agent_models(&["route-sonnet", "route-fable"]);
    let mut mappings = ClaudeDesktopModelMappings::all("route-sonnet");
    mappings.fable = "route-fable".into();
    mappings.fable_1m = true;
    mappings.startup_model = Some("fable".into());
    let resolved = resolve_claude_code_model_mappings(AgentClient::ClaudeCode, &models, "route-sonnet", Some(mappings)).unwrap().unwrap();
    let rendered = build_claude_agent_config(None, "http://127.0.0.1:8317", "test-key", "route-sonnet", &models, Some(&resolved)).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "route-fable[1m]");
    assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "route-sonnet");
    assert_eq!(value["model"], "fable");
    fs::write(&path, rendered).unwrap();
    let inspected = inspect_claude_code_model_mappings(&path).unwrap().unwrap();
    assert_eq!(inspected.fable, "route-fable");
    assert!(inspected.fable_1m);
}
