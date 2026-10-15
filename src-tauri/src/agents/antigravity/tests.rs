use super::*;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = env::temp_dir().join(format!(
            "cpa-antigravity-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        let updates = paths
            .iter()
            .map(|path| AgentFileUpdate {
                path: path.clone(),
                after: build_antigravity_config(
                    client,
                    path,
                    read_optional_text(path).unwrap().as_deref(),
                    "http://127.0.0.1:8317",
                    "test-key",
                    model,
                )
                .unwrap(),
            })
            .collect::<Vec<_>>();
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
        assert_eq!(
            inspect_antigravity_config(client, &paths, 8317, "test-key").unwrap(),
            (true, Some(model.into()))
        );
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn antigravity_cli_paths_and_launch_targets() {
    let home = Home::new();
    let cli = antigravity_config_paths(AgentClient::AntigravityCli, &home.0);
    assert_eq!(cli.len(), 2);
    assert_eq!(cli[0], home.0.join(".gemini/antigravity-cli/settings.json"));
    assert_eq!(
        cli[1],
        home.0.join(".gemini/antigravity-cli/cpa-connection.json")
    );
    let targets =
        agent_launch_targets(AgentClient::AntigravityCli, Some(Path::new("fixture")), None, false);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "cli");
    assert!(should_probe_primary_agent_executable_version(
        AgentClient::AntigravityCli
    ));
}

#[test]
fn antigravity_updates_and_disable_preserve_external_settings() {
    let client = AgentClient::AntigravityCli;
    let home = Home::new();
    let paths = config_paths(client.id(), &home.0).unwrap();
    let original = json!({
        "modelProvider": "original",
        "colorScheme": "dark",
        "customModelsConfig": {
            "customModels": {
                "User Model": {"modelName": "user"},
                CLI_MODEL_LABEL: {"modelName": "original", "maxTokens": 123}
            }
        }
    });
    home.save(&paths[0], &original);
    home.apply(client, "model-one");
    let applied = config_images(&paths).unwrap();
    home.apply(client, "model-one");
    assert_eq!(config_images(&paths).unwrap(), applied);
    let mut current =
        parse(&paths[0], read_optional_text(&paths[0]).unwrap().as_deref()).unwrap();
    current["user-added"] = json!({"keep": [1, 2]});
    home.save(&paths[0], &current);
    home.apply(client, "model-two");
    assert!(
        !inspect_antigravity_config(client, &paths, 8318, "test-key")
            .unwrap()
            .0
    );
    assert!(
        !inspect_antigravity_config(client, &paths, 8317, "other-key")
            .unwrap()
            .0
    );
    clear_agent_managed_configuration(client, &home.0, 8317).unwrap();
    let restored = parse(&paths[0], read_optional_text(&paths[0]).unwrap().as_deref()).unwrap();
    let mut expected = original;
    expected["user-added"] = json!({"keep": [1, 2]});
    assert_eq!(restored, expected);
    assert!(!paths[1].exists());
    assert!(!antigravity_has_marker(client, &paths).unwrap());
}

#[test]
fn antigravity_manual_backup_restores_only_its_client() {
    let home = Home::new();
    let client = AgentClient::AntigravityCli;
    home.apply(client, "model-one");
    let cli_paths = config_paths("antigravity-cli", &home.0).unwrap();
    let cli = config_images(&cli_paths).unwrap();
    let backup = create_backup("antigravity-cli", &home.0).unwrap();
    home.apply(client, "model-two");
    test_restore_backup(client, &home.0, &backup.id);
    assert_eq!(config_images(&cli_paths).unwrap(), cli);
    clear_agent_managed_configuration(client, &home.0, 8317).unwrap();
}

#[test]
fn antigravity_rejects_malformed_owned_containers() {
    let path = Path::new("settings.json");
    for input in [
        json!({"customModelsConfig": []}),
        json!({"customModelsConfig": {"customModels": false}}),
    ] {
        assert!(build_antigravity_config(
            AgentClient::AntigravityCli,
            path,
            Some(&serde_json::to_string(&input).unwrap()),
            "http://127.0.0.1:8317",
            "test-key",
            "model-one",
        )
        .is_err());
    }
}

#[test]
fn antigravity_cli_template_covers_settings_and_credentials() {
    let home = Home::new();
    let updates = build_agent_template_updates(AgentDefaultConfiguration {
        client: AgentClient::AntigravityCli,
        home: &home.0,
        port: 8317,
        api_key: "key",
        model: "model",
        models: &[],
        codex_catalog: None,
        oauth_configuration: false,
        claude_code_model_mappings: None,
        claude_desktop_model_mappings: None,
    })
    .unwrap();
    assert_eq!(updates.len(), 2);
    let paths = config_paths("antigravity-cli", &home.0).unwrap();
    prepare_config_updates(
        "antigravity-cli",
        &paths,
        &config_images(&paths).unwrap(),
        &updates,
        true,
    )
    .unwrap();
}

#[test]
fn antigravity_cli_helper_uses_managed_credentials_and_model() {
    let home = Home::new();
    home.apply(AgentClient::AntigravityCli, "model-one");
    let binary = if cfg!(windows) {
        home.0.join("AppData/Local/agy/bin/agy.exe")
    } else {
        home.0.join(".local/bin/agy")
    };
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, []).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let args = [
        "cpa".into(),
        "--cpa-antigravity-cli".into(),
        home.0.clone().into_os_string(),
        "--print=hello".into(),
    ];
    let command = antigravity_helper_command(&args).unwrap();
    assert_eq!(Path::new(command.get_program()), binary.as_path());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new(&format!(
                "--gemini_dir={}",
                home.0.join(".gemini").display()
            )),
            std::ffi::OsStr::new(&format!("--model={CLI_MODEL_LABEL}")),
            std::ffi::OsStr::new("--print=hello"),
        ]
    );
    let variables = command
        .get_envs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        variables[std::ffi::OsStr::new("GEMINI_API_KEY")],
        Some(std::ffi::OsStr::new("test-key"))
    );
    assert_eq!(
        variables[std::ffi::OsStr::new("GOOGLE_GEMINI_BASE_URL")],
        Some(std::ffi::OsStr::new("http://127.0.0.1:8317"))
    );
    assert_eq!(variables[std::ffi::OsStr::new("GOOGLE_API_KEY")], None);
    let paths = antigravity_config_paths(AgentClient::AntigravityCli, &home.0);
    let valid = fs::read_to_string(&paths[1]).unwrap();
    for key in ["apiKey", "baseUrl", "model", "provider"] {
        let mut connection: Value = serde_json::from_str(&valid).unwrap();
        connection[key] = json!("");
        home.save(&paths[1], &connection);
        assert!(antigravity_helper_command(&args).is_err(), "{key}");
    }
    fs::write(&paths[1], valid).unwrap();
    home.save(&paths[0], &json!({"modelProvider": "other"}));
    assert!(antigravity_helper_command(&args).is_err());
}

#[test]
fn antigravity_removal_without_history_keeps_unmanaged_settings() {
    let client = AgentClient::AntigravityCli;
    let home = Home::new();
    home.apply(client, "model-one");
    let paths = antigravity_config_paths(client, &home.0);
    let mut settings =
        parse(&paths[0], read_optional_text(&paths[0]).unwrap().as_deref()).unwrap();
    settings["externalSetting"] = json!(42);
    home.save(&paths[0], &settings);
    let removals = prepare_antigravity_removal(client, &paths).unwrap();
    let restored = parse(
        &paths[0],
        Some(std::str::from_utf8(removals[0].1.as_ref().unwrap()).unwrap()),
    )
    .unwrap();
    assert_eq!(restored, json!({"externalSetting": 42}));
    assert!(removals[1].1.is_none());
}
