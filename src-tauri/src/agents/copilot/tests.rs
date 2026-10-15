use super::*;

#[test]
fn copilot_vscode_template_rejects_missing_or_rotated_credentials_without_writes() {
    let home = Home::new();
    let path = copilot_config_paths(AgentClient::VscodeCopilot, &home.0).remove(0);
    let template = |key: &str| build_copilot_updates(AgentClient::VscodeCopilot, &home.0,
        "http://127.0.0.1:8317/v1", key, "model", &[], true);
    assert_eq!(template("first-key").err().as_deref(), Some("COPILOT_SETUP_REQUIRED"));
    assert!(!path.exists());
    let cache = path.parent().unwrap().join("ezcpa-copilot/credential.json");
    home.save(&cache, &json!({"keyHash": sha256_bytes(b"first-key"),
        "secretReference": "${input:chat.lm.secret.first}"}));
    let updates = template("first-key").unwrap();
    home.save(&path, &serde_json::from_str::<Value>(&updates[0].after).unwrap());
    let before = fs::read(&path).unwrap();
    let cache_before = fs::read(&cache).unwrap();
    assert_eq!(template("rotated-key").err().as_deref(), Some("COPILOT_SETUP_REQUIRED"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&cache).unwrap(), cache_before);
    home.save(&cache, &json!({"keyHash": sha256_bytes(b"rotated-key"),
        "secretReference": "${input:chat.lm.secret.rotated}"}));
    let updated: Value = serde_json::from_str(&template("rotated-key").unwrap()[0].after).unwrap();
    assert_eq!(updated[0]["apiKey"], "${input:chat.lm.secret.rotated}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = crate::unique_test_temp_path("cpa-copilot-test");
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn save(&self, path: &Path, value: &Value) {
        assert!(path.starts_with(&self.0));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
    fn apply(&self, client: AgentClient, model: &str) {
        let paths = config_paths(client.id(), &self.0).unwrap();
        let before = config_images(&paths).unwrap();
        let updates = build_copilot_updates(
            client,
            &self.0,
            "http://127.0.0.1:8317/v1",
            "fixture-key",
            model,
            &[],
            false,
        )
        .unwrap();
        config_updates(
            client.id(),
            &self.0,
            &before,
            &updates,
            "update",
            Some(model.into()),
            None,
        )
        .unwrap();
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn copilot_cli_apply_inspect_update_and_restore() {
    let home = Home::new();
    let client = AgentClient::CopilotCli;
    let paths = copilot_config_paths(client, &home.0);
    home.save(&paths[0], &json!({"custom": true}));
    home.apply(client, "first-model");
    assert_eq!(
        inspect_copilot_config(client, &paths, 8317, "fixture-key").unwrap(),
        (true, Some("first-model".into()))
    );
    assert!(
        !inspect_copilot_config(client, &paths, 8318, "fixture-key")
            .unwrap()
            .0
    );
    home.apply(client, "second-model");
    let current = config_images(&paths).unwrap();
    let restored = prepare_recorded_integration_restore(client, &paths, &current)
        .unwrap()
        .unwrap();
    assert_eq!(
        parse(&paths[0], text(restored[0].1.as_deref()).unwrap()).unwrap(),
        json!({"custom": true})
    );
}

#[test]
fn copilot_vscode_preserves_provider_groups_and_secret_references() {
    let home = Home::new();
    let client = AgentClient::VscodeCopilot;
    let path = copilot_config_paths(client, &home.0).remove(0);
    let other =
        json!({"name": "User models", "vendor": "openai", "apiKey": "${input:user-secret}"});
    home.save(&path, &json!([other]));
    home.apply(client, "first-model");
    let mut root = read_groups(&path, read_optional_text(&path).unwrap().as_deref()).unwrap();
    assert_eq!(root[0], other);
    assert!(root[1].get("apiKey").is_none());
    assert!(
        !inspect_copilot_config(client, &[path.clone()], 8317, "fixture-key")
            .unwrap()
            .0
    );
    root[1]["apiKey"] = json!("${input:chat.lm.secret.fixture}");
    home.save(&path, &root);
    home.save(&path.parent().unwrap().join("ezcpa-copilot/credential.json"),
        &json!({"keyHash": sha256_bytes(b"fixture-key"), "secretReference": "${input:chat.lm.secret.fixture}"}));
    home.apply(client, "second-model");
    let mut root = read_groups(&path, read_optional_text(&path).unwrap().as_deref()).unwrap();
    assert_eq!(root[1]["apiKey"], "${input:chat.lm.secret.fixture}");
    assert_eq!(root[1]["models"][0]["id"], "second-model");
    assert!(!serde_json::to_string(&root)
        .unwrap()
        .contains("fixture-key"));
    assert!(
        inspect_copilot_config(client, &[path.clone()], 8317, "fixture-key")
            .unwrap()
            .0
    );
    root[0]["custom"] = json!("external-edit");
    root.as_array_mut()
        .unwrap()
        .push(json!({"name": "New provider", "vendor": "ollama"}));
    home.save(&path, &root);
    let current = config_images(&[path.clone()]).unwrap();
    let restored = prepare_recorded_integration_restore(client, &[path.clone()], &current)
        .unwrap()
        .unwrap();
    let value = read_groups(&path, text(restored[0].1.as_deref()).unwrap()).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 2);
    assert_eq!(value[0]["custom"], "external-edit");
    assert_eq!(value[1]["vendor"], "ollama");
}

#[test]
fn copilot_vscode_rejects_invalid_roots_without_overwriting() {
    let home = Home::new();
    let client = AgentClient::VscodeCopilot;
    let path = copilot_config_paths(client, &home.0).remove(0);
    home.save(&path, &json!({"keep": true}));
    assert!(build_copilot_updates(
        client,
        &home.0,
        "http://localhost/v1",
        "key",
        "model",
        &[],
        false
    )
    .is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "{\"keep\":true}");
}

#[test]
fn copilot_helper_uses_environment_without_putting_key_in_arguments() {
    let home = Home::new();
    home.apply(AgentClient::CopilotCli, "model with spaces");
    let path = copilot_config_paths(AgentClient::CopilotCli, &home.0).remove(0);
    let executable = env::current_exe().unwrap();
    let args = [
        "ezcpa".into(),
        "--cpa-copilot-cli".into(),
        path.into_os_string(),
        executable.into_os_string(),
        "--help".into(),
    ];
    let command = copilot_helper_command(&args).unwrap();
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("--help")]
    );
    let environment = command
        .get_envs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        environment[std::ffi::OsStr::new("COPILOT_MODEL")],
        Some(std::ffi::OsStr::new("model with spaces"))
    );
    assert_eq!(
        environment[std::ffi::OsStr::new("COPILOT_PROVIDER_API_KEY")],
        Some(std::ffi::OsStr::new("fixture-key"))
    );
    assert_eq!(
        environment[std::ffi::OsStr::new("COPILOT_PROVIDER_BEARER_TOKEN")],
        None
    );
}

#[test]
fn copilot_detection_requires_product_identity_and_actual_chat_extension() {
    assert_eq!(
        copilot_cli_version_output(
            "GitHub Copilot CLI 1.0.96.\nRun 'copilot update' to check for updates."
        ),
        Some("1.0.96".into())
    );
    for output in ["1.141.0", "Cannot find GitHub Copilot CLI (https://docs.github.com/en/copilot/how-tos/set-up/install-copilot-cli)\nInstall GitHub Copilot CLI? (y/N):", "GitHub Copilot CLI is not installed", "GitHub Copilot CLI 1.0.96. Install now?"] {
        assert_eq!(copilot_cli_version_output(output), None);
    }
    for output in [
        "1.141.0",
        "github.copilot@1.400.0",
        "openai.chatgpt@26.1007.21434",
        "github.copilot-chat-other@1.2.3",
    ] {
        assert_eq!(vscode_copilot_version_output(output), None);
    }
    assert_eq!(
        vscode_copilot_version_output("github.copilot@1.400.0\nGitHub.copilot-chat@0.40.2"),
        Some("0.40.2".into())
    );
    for client in [AgentClient::CopilotCli] {
        assert!(!agent_installation_detected(client, None, true, true));
        assert!(!agent_installation_detected(
            client,
            Some("1.0.96"),
            false,
            false
        ));
        assert!(agent_installation_detected(
            client,
            Some("1.0.96"),
            true,
            false
        ));
    }
}

#[cfg(target_os = "windows")]
#[test]
fn copilot_detection_separates_vscode_installation_from_chat_plugin() {
    let home = Home::new();
    let shim = home.0.join("copilot.cmd");
    fs::write(&shim, "@echo off\r\necho Cannot find GitHub Copilot CLI\r\necho Install GitHub Copilot CLI? (y/N):\r\nexit /b 0\r\n").unwrap();
    let status = inspect_agent_config_with_executables(
        AgentClient::CopilotCli,
        &home.0,
        8317,
        "fixture",
        Some(&shim),
        None,
    );
    assert!(!status.installed);
    assert!(status.version.is_none());
    assert!(status.launch_targets.is_empty());
    fs::write(
        &shim,
        "@echo off\r\necho GitHub Copilot CLI 1.0.96.\r\nexit /b 1\r\n",
    )
    .unwrap();
    assert!(detect_copilot_cli(&home.0, Some(&shim)).is_none());
    fs::write(
        &shim,
        "@echo off\r\necho GitHub Copilot CLI 1.0.96.\r\nexit /b 0\r\n",
    )
    .unwrap();
    let status = inspect_agent_config_with_executables(
        AgentClient::CopilotCli,
        &home.0,
        8317,
        "fixture",
        Some(&shim),
        None,
    );
    assert!(status.installed);
    assert_eq!(status.version.as_deref(), Some("1.0.96"));
    assert_eq!(status.launch_targets.len(), 1);
    let code = home.0.join("code.cmd");
    fs::write(&code, "@echo off\r\nif \"%1\"==\"--version\" (echo 1.141.0) else (echo openai.chatgpt@26.1007.21434)\r\nexit /b 0\r\n").unwrap();
    let status = inspect_agent_config_with_executables(
        AgentClient::VscodeCopilot,
        &home.0,
        8317,
        "fixture",
        Some(&code),
        None,
    );
    assert!(status.installed);
    assert!(!status.plugin_installed);
    assert_eq!(status.version.as_deref(), Some("1.141.0"));
    assert_eq!(status.app_version.as_deref(), Some("1.141.0"));
    assert!(status.plugin_version.is_none());
    assert_eq!(status.launch_targets.len(), 1);
    fs::write(
        &code,
        "@echo off\r\nif \"%1\"==\"--version\" (echo 1.141.0) else (echo github.copilot-chat@0.40.2)\r\nexit /b 0\r\n",
    )
    .unwrap();
    let status = inspect_agent_config_with_executables(
        AgentClient::VscodeCopilot,
        &home.0,
        8317,
        "fixture",
        Some(&code),
        None,
    );
    assert!(status.installed);
    assert!(status.plugin_installed);
    assert_eq!(status.version.as_deref(), Some("1.141.0"));
    assert_eq!(status.plugin_version.as_deref(), Some("0.40.2"));
    assert_eq!(status.launch_targets.len(), 1);
}

#[test]
fn copilot_builtin_detection_uses_active_editor_package() {
    let home = Home::new();
    let executable = home.0.join("bin/code");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "fixture").unwrap();
    let commit = "2a59476c9bfcb90b3ddc372c36762471b7dfad1c";
    let app = home.0.join("2a59476c9b/resources/app");
    home.save(&app.join("product.json"), &json!({}));
    let package = app.join("extensions/copilot/package.json");
    home.save(
        &package,
        &json!({"publisher":"GitHub","name":"copilot-chat","version":"0.69.0"}),
    );
    assert_eq!(
        bundled_vscode_copilot_version(&executable, &format!("1.141.0\n{commit}\nx64")),
        Some("0.69.0".into())
    );
    assert_eq!(
        bundled_vscode_copilot_version(
            &executable,
            "1.141.0\n0000000000000000000000000000000000000000\nx64"
        ),
        None
    );
    home.save(
        &package,
        &json!({"publisher":"Other","name":"copilot-chat","version":"0.69.0"}),
    );
    assert_eq!(bundled_vscode_copilot_version(&executable, commit), None);
}

#[test]
fn copilot_native_credential_rotation_and_template_preserve_user_models() {
    let home = Home::new();
    let path = copilot_config_paths(AgentClient::VscodeCopilot, &home.0).remove(0);
    let cache = path.parent().unwrap().join("ezcpa-copilot").join("credential.json");
    let reference = "${input:chat.lm.secret.fixture-native}";
    home.save(&cache, &json!({"keyHash": sha256_bytes(b"fixture-key"), "secretReference": reference}));
    home.save(&path, &json!([{"vendor": "ollama", "name": "Keep me", "models": []}]));
    home.apply(AgentClient::VscodeCopilot, "native-model");
    let content = read_optional_text(&path).unwrap().unwrap();
    assert!(!content.contains("fixture-key"));
    let root = read_groups(&path, Some(&content)).unwrap();
    assert_eq!(root[1]["apiKey"], reference);
    assert!(inspect_copilot_config(AgentClient::VscodeCopilot, &[path.clone()], 8317, "fixture-key").unwrap().0);
    assert!(!inspect_copilot_config(AgentClient::VscodeCopilot, &[path.clone()], 8317, "rotated-key").unwrap().0);
    assert!(setup::cached_vscode_secret(&path, "rotated-key").is_none());
    let updates = build_copilot_updates(AgentClient::VscodeCopilot, &home.0,
        "http://127.0.0.1:8317/v1", "fixture-key", "new-model", &[], true).unwrap();
    let updated = read_groups(&path, Some(&updates[0].after)).unwrap();
    assert_eq!(updated[0], root[0]);
    assert_eq!(updated[1]["apiKey"], reference);
    assert_eq!(updated[1]["models"][0]["id"], "new-model");
}

#[test]
fn copilot_vscode_uses_codex_context_parameters_and_reserves_output_budget() {
    let home = Home::new();
    let path = copilot_config_paths(AgentClient::VscodeCopilot, &home.0).remove(0);
    let mut runtime = codex_catalog::parse_runtime_models(&json!({"models": [
        {"slug": "large-model", "context_window": 1_000_000},
        {"slug": "limited-model", "context_window": 200_000},
        {"slug": "small-model", "max_context_window": 8_192},
        {"slug": "gpt-6-sol", "context_window": 272_000, "max_context_window": 872_000}
    ]})).unwrap();
    codex_catalog::apply_configured_context_limits(&mut runtime,
        "codex-api-key:\n  - models:\n      - name: upstream\n        alias: limited-model\n        max-context-length: 64000\n").unwrap();
    let mut models = ["large-model", "limited-model", "small-model", "gpt-6-sol"].map(|name| AgentModelOption {
        name: name.into(), alias: None, is_alias: false, context_window: None,
        input_modalities: None, harness_metadata: None,
    });
    enrich_vscode_context_windows(&mut models, &runtime).unwrap();
    for (name, expected) in [("large-model", 1_000_000_u64), ("limited-model", 64_000), ("small-model", 8_192), ("gpt-6-sol", 272_000)] {
        let content = build_copilot_config(AgentClient::VscodeCopilot, &path, None,
            "http://localhost/v1", "${input:chat.lm.secret.fixture}", name, &models).unwrap();
        let root: Value = serde_json::from_str(&content).unwrap();
        let model = &root[0]["models"][0];
        assert_eq!(model["contextWindow"], expected);
        // Omitting maxInputTokens lets VS Code reserve output within the full window.
        assert!(model.get("maxInputTokens").is_none());
        let output = model["maxOutputTokens"].as_u64().unwrap();
        assert!(output > 0 && output < expected);
    }
}
