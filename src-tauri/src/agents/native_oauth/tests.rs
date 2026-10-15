use super::*;

struct TestHome(PathBuf);
impl TestHome {
    fn new() -> Self {
        let path = crate::unique_test_temp_path("cpa-native-oauth");
        fs::create_dir_all(path.join(".codex")).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(".codex").join(name)
    }
    fn write(&self, name: &str, content: &str) {
        fs::write(self.path(name), content).unwrap();
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }
    fn config(&self) -> toml::Value {
        toml::from_str(&self.read("config.toml")).unwrap()
    }
}
impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const CPA_CONFIG: &str = r#"model_provider = "cpa-gui"
model = "cpa-alias"
model_catalog_json = "cpa-gui-model-catalog.json"
openai_base_url = "http://localhost:8317/v1"
chatgpt_base_url = "http://localhost:8317/chatgpt"
forced_login_method = "api"
profile = "legacy-cpa"
# keep this comment
approval_policy = "on-request"
[mcp_servers.example]
command = "example"
[model_providers.cpa-gui]
name = "EasyCLIProxyAPI"
base_url = "http://127.0.0.1:8317/v1"
wire_api = "responses"
experimental_bearer_token = "test-api-key"
[model_providers.openai]
base_url = "http://localhost:8317/v1"
"#;
const OAUTH: &str = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"test-token","refresh_token":"test-refresh"},"last_refresh":"today","custom":"keep"}"#;

#[test]
fn native_oauth_round_trip_preserves_cpa_and_unrelated_user_edits() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    let api = build_codex_api_auth("test-api-key").unwrap();
    home.write("auth.json", &api);
    home.write(CODEX_MODEL_CATALOG_FILE, "existing catalog");
    let original = home.config();
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(codex_native_oauth_enabled(&home.0).unwrap());
    let native = home.config();
    assert_eq!(native["model_provider"].as_str(), Some("openai"));
    assert_eq!(native["forced_login_method"].as_str(), Some("chatgpt"));
    for key in [
        "model",
        "model_catalog_json",
        "openai_base_url",
        "chatgpt_base_url",
        "profile",
    ] {
        assert!(native.get(key).is_none(), "{key}");
    }
    assert!(native["model_providers"].get("openai").is_none());
    assert_eq!(
        native["model_providers"]["cpa-gui"],
        original["model_providers"]["cpa-gui"]
    );
    assert!(!home.path("auth.json").exists());
    assert!(ensure_codex_cpa_mode(&home.0).is_err());
    assert!(
        !sync_codex_model_catalog_if_configured(&home.0, 8317, "test-api-key", &[], "unused")
            .unwrap()
    );
    home.write("auth.json", OAUTH);
    home.write(
        "config.toml",
        &home.read("config.toml").replace("on-request", "never"),
    );
    switch_codex_native_oauth(&home.0, false).unwrap();
    assert!(!codex_native_oauth_enabled(&home.0).unwrap());
    let restored = home.config();
    for key in ROUTING_KEYS {
        assert_eq!(restored.get(*key), original.get(*key), "{key}");
    }
    assert_eq!(restored["model_providers"], original["model_providers"]);
    assert_eq!(restored["approval_policy"].as_str(), Some("never"));
    assert!(home.read("config.toml").contains("# keep this comment"));
    assert_eq!(home.read("auth.json"), api);
    assert_eq!(home.read(CODEX_MODEL_CATALOG_FILE), "existing catalog");
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert_eq!(home.read("auth.json"), OAUTH);
}

#[test]
fn native_oauth_retains_refreshed_tokens_and_native_model_across_switches() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert_eq!(home.read("auth.json"), OAUTH);
    let refreshed = OAUTH.replace("test-refresh", "rotated-refresh");
    home.write("auth.json", &refreshed);
    home.write(
        "config.toml",
        &("model = \"native-model\"\n".to_string() + &home.read("config.toml")),
    );
    switch_codex_native_oauth(&home.0, false).unwrap();
    assert_eq!(home.read("auth.json"), refreshed);
    assert_eq!(home.config()["model"].as_str(), Some("cpa-alias"));
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert_eq!(home.config()["model"].as_str(), Some("native-model"));
    assert_eq!(home.read("auth.json"), refreshed);
    fs::remove_file(home.path("auth.json")).unwrap();
    switch_codex_native_oauth(&home.0, false).unwrap();
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(
        !home.path("auth.json").exists(),
        "Never resurrect credentials after the user signs out"
    );
}

#[test]
fn native_oauth_can_be_enabled_before_first_login_and_is_idempotent() {
    let home = TestHome::new();
    assert_eq!(
        switch_codex_native_oauth(&home.0, false).unwrap().outcome,
        "unchanged"
    );
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(!home.path("auth.json").exists());
    let paths = [
        home.path("config.toml"),
        home.path(CODEX_NATIVE_OAUTH_STATE_FILE),
    ];
    let before = config_images(&paths).unwrap();
    assert_eq!(
        switch_codex_native_oauth(&home.0, true).unwrap().outcome,
        "unchanged"
    );
    assert_eq!(config_images(&paths).unwrap(), before);
    switch_codex_native_oauth(&home.0, false).unwrap();
    assert!(home.config().as_table().unwrap().is_empty());
}

#[test]
fn native_oauth_rejects_malformed_files_without_partial_updates() {
    for name in ["config.toml", "auth.json", CODEX_NATIVE_OAUTH_STATE_FILE] {
        let home = TestHome::new();
        home.write("config.toml", CPA_CONFIG);
        home.write("auth.json", OAUTH);
        home.write(name, "malformed [");
        let paths = [
            home.path("config.toml"),
            home.path("auth.json"),
            home.path(CODEX_NATIVE_OAUTH_STATE_FILE),
        ];
        let before = config_images(&paths).unwrap();
        assert!(switch_codex_native_oauth(&home.0, true).is_err());
        assert_eq!(config_images(&paths).unwrap(), before);
    }
}

#[test]
fn native_oauth_does_not_select_api_key_when_tokens_also_exist() {
    let auth = OAUTH.replace(
        "\"auth_mode\":\"chatgpt\"",
        "\"auth_mode\":\"apikey\",\"OPENAI_API_KEY\":\"old-key\"",
    );
    let normalized: serde_json::Value =
        serde_json::from_str(&oauth_auth(Some(&auth)).unwrap().unwrap()).unwrap();
    assert_eq!(normalized["auth_mode"], "chatgpt");
    assert!(normalized.get("OPENAI_API_KEY").is_none());
    assert_eq!(normalized["tokens"]["refresh_token"], "test-refresh");
}

#[test]
fn clearing_codex_removes_native_oauth_recovery_credentials() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    clear_codex_config_files(&home.0).unwrap();
    assert!(!home.path(CODEX_NATIVE_OAUTH_STATE_FILE).exists());
    assert!(!home.path("auth.json").exists());
    assert!(!codex_native_oauth_enabled(&home.0).unwrap());
}

#[cfg(unix)]
#[test]
fn native_oauth_credential_cache_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert_eq!(
        fs::metadata(home.path(CODEX_NATIVE_OAUTH_STATE_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn native_oauth_handles_inline_provider_tables_without_losing_other_providers() {
    let home = TestHome::new();
    home.write("config.toml", "model_provider = \"cpa-gui\"\nmodel_providers = { openai = { base_url = \"http://localhost:8317/v1\" }, other = { base_url = \"http://localhost:9999\" } }\n");
    let original = home.config();
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(home.config()["model_providers"].get("openai").is_none());
    assert_eq!(
        home.config()["model_providers"]["other"],
        original["model_providers"]["other"]
    );
    switch_codex_native_oauth(&home.0, false).unwrap();
    assert_eq!(home.config(), original);
}

#[test]
fn restore_official_configuration_always_removes_the_cpa_catalog_setting() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(home.config().get("model_catalog_json").is_none());
    home.write(
        "config.toml",
        &format!(
            "model_catalog_json = \"{CODEX_MODEL_CATALOG_FILE}\"\nmodel = \"native-choice\"\n{}",
            home.read("config.toml")
        ),
    );
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(home.config().get("model_catalog_json").is_none());
    assert_eq!(home.config()["model"].as_str(), Some("native-choice"));
    switch_codex_native_oauth(&home.0, false).unwrap();
    assert_eq!(home.config()["model"].as_str(), Some("cpa-alias"));
}

#[test]
fn update_configuration_reconnects_cpa_and_restores_catalog_after_official_restore() {
    for oauth_configuration in [false, true] {
        let home = TestHome::new();
        home.write("config.toml", CPA_CONFIG);
        home.write("auth.json", OAUTH);
        switch_codex_native_oauth(&home.0, true).unwrap();
        let refreshed = OAUTH.replace("test-refresh", "latest-refresh");
        home.write("auth.json", &refreshed);
        let catalog = r#"{"models":[{"slug":"new-cpa-model"}]}"#;
        let result = apply_agent_configuration_with_oauth(
            AgentClient::Codex,
            &home.0,
            9321,
            "new-cpa-key",
            "new-cpa-model",
            AgentConfigurationOptions {
                models: &[],
                codex_catalog: Some(catalog),
                oauth_configuration,
                claude_code_model_mappings: None,
                claude_desktop_model_mappings: None,
            },
        )
        .unwrap();
        assert_eq!(result.outcome, "updated");
        assert!(!codex_native_oauth_enabled(&home.0).unwrap());
        let config = home.config();
        assert_eq!(config["model_provider"].as_str(), Some("cpa-gui"));
        assert_eq!(
            config["model_catalog_json"].as_str(),
            Some(CODEX_MODEL_CATALOG_FILE)
        );
        assert_eq!(config["model"].as_str(), Some("new-cpa-model"));
        assert_eq!(
            config["model_providers"]["cpa-gui"]["base_url"].as_str(),
            Some("http://127.0.0.1:9321/v1")
        );
        assert_eq!(
            config["model_providers"]["cpa-gui"]["experimental_bearer_token"].as_str(),
            Some("new-cpa-key")
        );
        assert_eq!(
            config["forced_login_method"].as_str(),
            Some(if oauth_configuration {
                "chatgpt"
            } else {
                "api"
            })
        );
        validate_codex_catalog_file(&home.path("config.toml"), "new-cpa-model").unwrap();
        if oauth_configuration {
            assert_eq!(home.read("auth.json"), refreshed);
        } else {
            assert!(codex_auth_file_has_api_key(
                &home.path("auth.json"),
                "new-cpa-key"
            ));
        }
        switch_codex_native_oauth(&home.0, true).unwrap();
        assert_eq!(home.read("auth.json"), refreshed);
        assert!(home.config().get("model_catalog_json").is_none());
    }
}

fn apply_cpa(
    home: &TestHome,
    oauth_configuration: bool,
) -> Result<AgentConfigActionResult, String> {
    apply_agent_configuration_with_oauth(
        AgentClient::Codex,
        &home.0,
        8317,
        "test-api-key",
        "cpa-alias",
        AgentConfigurationOptions {
            models: &[],
            codex_catalog: Some(r#"{"models":[{"slug":"cpa-alias"}]}"#),
            oauth_configuration,
            claude_code_model_mappings: None,
            claude_desktop_model_mappings: None,
        },
    )
}

#[test]
fn codex_apply_close_restores_original_login_and_preserves_user_edits() {
    let home = TestHome::new();
    home.write("config.toml", "model = \"official-model\"\nforced_login_method = \"chatgpt\"\napproval_policy = \"on-request\"\n[mcp_servers.example]\ncommand = \"example\"\n");
    home.write("auth.json", OAUTH);
    let original = home.config();
    apply_cpa(&home, false).unwrap();
    assert_eq!(home.config()["model_provider"].as_str(), Some("cpa-gui"));
    assert_eq!(home.config()["forced_login_method"].as_str(), Some("api"));
    assert!(codex_auth_file_has_api_key(
        &home.path("auth.json"),
        "test-api-key"
    ));
    assert!(
        validate_codex_oauth_login(&home.0).is_ok(),
        "Saved login remains selectable after API mode"
    );
    home.write(
        "config.toml",
        &format!(
            "openai_base_url = \"https://example.test/edited\"\n{}",
            home.read("config.toml").replace("on-request", "never")
        ),
    );
    let mut api: serde_json::Value = serde_json::from_str(&home.read("auth.json")).unwrap();
    api["custom"] = serde_json::json!("edited");
    home.write("auth.json", &api.to_string());
    apply_cpa(&home, true).unwrap();
    let refreshed = home
        .read("auth.json")
        .replace("test-refresh", "new-refresh");
    home.write("auth.json", &refreshed);
    apply_cpa(&home, false).unwrap();
    close_codex_configuration(&home.0).unwrap();
    let restored = home.config();
    assert_eq!(restored["model"], original["model"]);
    assert_eq!(
        restored["forced_login_method"],
        original["forced_login_method"]
    );
    assert_eq!(restored["approval_policy"].as_str(), Some("never"));
    assert_eq!(restored["mcp_servers"], original["mcp_servers"]);
    assert_eq!(
        restored["openai_base_url"].as_str(),
        Some("https://example.test/edited")
    );
    assert!(restored.get("model_catalog_json").is_none());
    assert_eq!(
        restored["model_providers"][MANAGED_AGENT_PROVIDER_ID]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    let auth: serde_json::Value = serde_json::from_str(&home.read("auth.json")).unwrap();
    assert_eq!(auth["tokens"]["refresh_token"], "new-refresh");
    assert_eq!(auth["custom"], "edited");
    assert!(auth.get("OPENAI_API_KEY").is_none());
    assert_eq!(
        close_codex_configuration(&home.0).unwrap().outcome,
        "unchanged"
    );
    apply_cpa(&home, false).unwrap();
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(
        home.read("auth.json").contains("new-refresh"),
        "Official restore also uses the saved login"
    );
}

#[test]
fn codex_close_existing_cpa_without_recovery_record_removes_managed_fields() {
    let home = TestHome::new();
    home.write(
        "config.toml",
        &build_codex_agent_config_with_oauth(
            Some("approval_policy = \"never\"\n"),
            "http://127.0.0.1:8317/v1",
            "test-api-key",
            "cpa-alias",
            false,
        )
        .unwrap(),
    );
    home.write("auth.json", &build_codex_api_auth("test-api-key").unwrap());
    close_codex_configuration(&home.0).unwrap();
    assert_eq!(home.config()["approval_policy"].as_str(), Some("never"));
    assert!(home.config().get("model_provider").is_none());
    assert!(home.config().get("model_catalog_json").is_none());
    assert_eq!(
        home.config()["model_providers"][MANAGED_AGENT_PROVIDER_ID]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert!(!home.path("auth.json").exists());
}

#[test]
fn codex_close_restores_preexisting_api_provider_and_key() {
    let home = TestHome::new();
    home.write("config.toml", "model_provider = \"other\"\nmodel = \"other-model\"\n[model_providers.other]\nbase_url = \"https://example.test/v1\"\n");
    home.write("auth.json", &build_codex_api_auth("original-key").unwrap());
    let original = home.config();
    apply_cpa(&home, false).unwrap();
    close_codex_configuration(&home.0).unwrap();
    let restored = home.config();
    assert_eq!(restored["model_provider"], original["model_provider"]);
    assert_eq!(restored["model"], original["model"]);
    assert_eq!(
        restored["model_providers"]["other"],
        original["model_providers"]["other"]
    );
    assert_eq!(
        restored["model_providers"][MANAGED_AGENT_PROVIDER_ID]["base_url"].as_str(),
        Some("http://127.0.0.1:8317/v1")
    );
    assert!(codex_auth_file_has_api_key(
        &home.path("auth.json"),
        "original-key"
    ));
}

#[test]
fn codex_close_does_not_resurrect_an_explicit_logout() {
    let home = TestHome::new();
    home.write("auth.json", OAUTH);
    apply_cpa(&home, true).unwrap();
    fs::remove_file(home.path("auth.json")).unwrap();
    assert!(validate_codex_oauth_login(&home.0).is_err());
    apply_cpa(&home, false).unwrap();
    assert!(validate_codex_oauth_login(&home.0).is_err());
    close_codex_configuration(&home.0).unwrap();
    assert!(!home.path("auth.json").exists());
}

#[test]
fn codex_official_restore_does_not_reuse_credentials_after_logout() {
    let home = TestHome::new();
    home.write("auth.json", OAUTH);
    apply_cpa(&home, true).unwrap();
    fs::remove_file(home.path("auth.json")).unwrap();
    switch_codex_native_oauth(&home.0, true).unwrap();
    assert!(!home.path("auth.json").exists());
}

#[test]
fn codex_close_after_official_reconnect_restores_official_routing() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    apply_cpa(&home, false).unwrap();
    close_codex_configuration(&home.0).unwrap();
    assert_eq!(home.config()["model_provider"].as_str(), Some("openai"));
    assert_eq!(
        home.config()["forced_login_method"].as_str(),
        Some("chatgpt")
    );
    for key in [
        "model_catalog_json",
        "openai_base_url",
        "chatgpt_base_url",
        "profile",
    ] {
        assert!(home.config().get(key).is_none(), "{key}");
    }
    assert_eq!(home.read("auth.json"), OAUTH);
    assert!(!codex_native_oauth_enabled(&home.0).unwrap());
}

#[test]
fn codex_close_rejects_invalid_state_without_changing_any_files() {
    for name in ["config.toml", "auth.json", CODEX_NATIVE_OAUTH_STATE_FILE] {
        let home = TestHome::new();
        home.write("auth.json", OAUTH);
        apply_cpa(&home, false).unwrap();
        home.write(name, "invalid [");
        let paths = vec![
            home.path("config.toml"),
            home.path("auth.json"),
            home.path(CODEX_NATIVE_OAUTH_STATE_FILE),
        ];
        let before = config_images(&paths).unwrap();
        assert!(close_codex_configuration(&home.0).is_err());
        assert_eq!(config_images(&paths).unwrap(), before);
    }
}

#[test]
fn codex_failed_api_apply_preserves_login_and_recovery_record() {
    let home = TestHome::new();
    home.write("config.toml", "model = \"official-model\"\n");
    home.write("auth.json", OAUTH);
    home.write(CODEX_MODEL_CATALOG_FILE, "malformed catalog");
    let paths = vec![
        home.path("config.toml"),
        home.path("auth.json"),
        home.path(CODEX_MODEL_CATALOG_FILE),
        home.path(CODEX_NATIVE_OAUTH_STATE_FILE),
    ];
    let before = config_images(&paths).unwrap();
    assert!(apply_cpa(&home, false).is_err());
    assert_eq!(config_images(&paths).unwrap(), before);
}

#[test]
fn failed_cpa_update_preserves_official_routing_and_login() {
    let home = TestHome::new();
    home.write("config.toml", CPA_CONFIG);
    home.write("auth.json", OAUTH);
    switch_codex_native_oauth(&home.0, true).unwrap();
    let mut paths = config_paths("codex", &home.0).unwrap();
    paths.push(home.path(CODEX_NATIVE_OAUTH_STATE_FILE));
    let before = config_images(&paths).unwrap();
    let result = apply_agent_configuration_with_oauth(
        AgentClient::Codex,
        &home.0,
        8317,
        "test-api-key",
        "unavailable-model",
        AgentConfigurationOptions {
            models: &[],
            codex_catalog: Some(r#"{"models":[{"slug":"other-model"}]}"#),
            oauth_configuration: false,
            claude_code_model_mappings: None,
            claude_desktop_model_mappings: None,
        },
    );
    assert!(result.is_err());
    assert_eq!(config_images(&paths).unwrap(), before);
    assert!(codex_native_oauth_enabled(&home.0).unwrap());
    assert!(home.config().get("model_catalog_json").is_none());
}
