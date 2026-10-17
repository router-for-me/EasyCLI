use super::support::agent_test_home;
use super::*;
use std::process::{Child, Stdio};

struct TestCore {
    child: Option<Child>,
    directory: PathBuf,
    config: PathBuf,
    executable: PathBuf,
    origin: String,
    client: reqwest::Client,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        self.stop();
        if std::thread::panicking() {
            eprintln!(
                "{}",
                fs::read_to_string(self.directory.join("core.log")).unwrap_or_default()
            );
        }
        if self.directory.parent() == Some(std::env::temp_dir().as_path())
            && self
                .directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cpa-gui-agent-v8-contract-")
        {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

impl TestCore {
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    async fn start(&mut self) {
        let mut command = Command::new(&self.executable);
        command
            .args(["-config"])
            .arg(&self.config)
            .arg("-local-model")
            .current_dir(&self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                File::create(self.directory.join("core.log")).unwrap(),
            ))
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        self.child = Some(command.spawn().unwrap());
        for _ in 0..100 {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "test kernel exited during startup"
            );
            if self
                .client
                .get(format!("{}/v8/management/config", self.origin))
                .bearer_auth("isolated-test-secret")
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("test kernel startup timed out");
    }

    async fn config_view(&self) -> serde_json::Value {
        self.client
            .get(format!("{}/v8/management/config", self.origin))
            .bearer_auth("isolated-test-secret")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn validate_save(&self, yaml: &str) -> serde_json::Value {
        let response = self
            .client
            .put(format!("{}/v8/management/config.yaml", self.origin))
            .bearer_auth("isolated-test-secret")
            .header("Content-Type", "application/yaml")
            .body(yaml.to_string())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(
            status.is_success(),
            "v8 rejected generated settings: {status} {body}"
        );
        self.config_view().await
    }

    async fn wait_for_client_key(&self, key: &str, expected: u16) {
        for _ in 0..100 {
            let status = self
                .client
                .get(format!("{}/v1/models", self.origin))
                .bearer_auth(key)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16();
            if status == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("client key did not reach HTTP {expected}");
    }
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v8_claude_desktop_and_code_share_persisted_model_routes() {
    check_claude_model_routes(None).await;
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v7_claude_routes_survive_v8_migration_and_reapply() {
    for marker in ["", "config-version: 7\n"] {
        check_claude_model_routes(Some(marker)).await;
    }
}

async fn check_claude_model_routes(legacy_marker: Option<&str>) {
    let executable = fs::canonicalize(std::env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE")).unwrap();
    let directory = agent_test_home("v8-contract-claude-routes");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut core = TestCore {
        child: None, config: directory.join("config.yaml"), directory, executable,
        origin: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap(),
    };
    let gui = GuiConfigFile {
        port, management_secret_key: "isolated-test-secret".into(),
        auth_dir: path_to_string(&core.directory.join("auth")),
        api_keys: vec![GuiApiKeyEntry { key: "isolated-client-key".into(), remark: String::new() }],
        ..GuiConfigFile::default()
    };
    let initial = "config-version: 8\nmanagement: {disable-control-panel: true, disable-auto-update-panel: true}\napi-keys:\n  codex:\n    - name: test-upstream\n      base-url: http://127.0.0.1:1/v1\n      models: [{name: model-one}]\n      keys: [{api-key: upstream-test, weight: 2}]\n";
    let migrated;
    let initial = if let Some(marker) = legacy_marker {
        let legacy = format!("{marker}codex-api-key:\n  - api-key: upstream-test\n    base-url: http://127.0.0.1:1/v1\n    models: [{{name: model-one, alias: claude-sonnet-custom}}]\noauth-model-alias:\n  codex: [{{name: oauth-original, alias: claude-opus-legacy, fork: true}}]\npayload:\n  override:\n    - models: [{{name: claude-sonnet-custom, protocol: codex}}]\n      params: {{reasoning.effort: high, service_tier: priority}}\n");
        migrated = migrate_legacy_core_config_to_v8("config-version: 8\nserver: {port: 8317}\nmanagement: {disable-control-panel: true, disable-auto-update-panel: true}\n", &legacy).unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
        assert_eq!(document["config-version"].as_u64(), Some(8));
        assert_eq!(document["oauth"]["model-alias"]["codex"][0]["alias"], "claude-opus-legacy");
        assert_eq!(document["requests"]["payload"]["override"][0]["params"]["service_tier"], "priority");
        for key in ["codex-api-key", "oauth-model-alias", "payload"] {
            assert!(document.get(key).is_none(), "legacy field {key} must be migrated");
        }
        migrated.as_str()
    } else { initial };
    fs::write(&core.config, apply_gui_managed_settings(initial, &gui).unwrap()).unwrap();
    core.start().await;
    let before = core.config_view().await;
    let desktop = fetch_prepared_agent_models(AgentClient::ClaudeDesktop, &gui).await.unwrap();
    let mappings = ClaudeDesktopModelMappings {
        desktop_models: Some(vec![ClaudeDesktopModelMapping {
            model: if legacy_marker.is_some() { "claude-sonnet-custom" } else { "model-one" }.into(),
            alias: if legacy_marker.is_some() { "" } else { "claude-sonnet-custom" }.into(), context_1m: false,
        }]),
        ..ClaudeDesktopModelMappings::all("model-one")
    };
    let profile = commit_agent_with_core(&gui, Some(mappings), &desktop.models, |mappings| {
        build_claude_desktop_profile(None, &core.origin, "isolated-client-key", "model-one", &desktop.models, mappings)
    }).await.unwrap();
    let profile: serde_json::Value = serde_json::from_str(&profile).unwrap();
    assert_eq!(profile["inferenceModels"][0]["name"], "claude-sonnet-custom");
    assert_eq!(profile["inferenceGatewayBaseUrl"], core.origin);
    let saved = core.config_view().await;
    if legacy_marker.is_some() {
        // Reapplying an existing public route must preserve its upstream,
        // credentials, OAuth mappings, and reasoning/Fast overrides.
        assert_eq!(saved["api-keys"], before["api-keys"]);
        assert_eq!(saved["oauth"]["model-alias"], before["oauth"]["model-alias"]);
        assert_eq!(saved["requests"]["payload"], before["requests"]["payload"]);
    }
    assert_eq!(saved["api-keys"]["codex"][0]["keys"], before["api-keys"]["codex"][0]["keys"]);
    assert!(saved["api-keys"]["codex"][0]["models"].as_array().unwrap().iter()
        .any(|entry| entry["name"] == "model-one" && entry["alias"] == "claude-sonnet-custom"));

    for client in [AgentClient::ClaudeDesktop, AgentClient::ClaudeCode] {
        let mut available = None;
        for _ in 0..50 {
            let prepared = fetch_prepared_agent_models(client, &gui).await.unwrap();
            if prepared.models.iter().any(|model| model.name == "claude-sonnet-custom") {
                available = Some(prepared.models);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let models = available.expect("Claude client must see the route published by the core");
        if client == AgentClient::ClaudeCode {
            let settings = build_claude_agent_config(None, &core.origin, "isolated-client-key", "claude-sonnet-custom", &models, None).unwrap();
            let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
            assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "claude-sonnet-custom");
            assert_eq!(settings["env"]["ANTHROPIC_BASE_URL"], core.origin);
        }
    }
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v8_model_write_migrates_legacy_provider_with_version_marker() {
    let executable = fs::canonicalize(std::env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE")).unwrap();
    let directory = agent_test_home("v8-contract-mixed-models");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut core = TestCore {
        child: None, config: directory.join("config.yaml"), directory, executable,
        origin: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap(),
    };
    let gui = GuiConfigFile {
        port, management_secret_key: "isolated-test-secret".into(),
        ..GuiConfigFile::default()
    };
    let initial = format!("config-version: 8\nserver: {{host: 127.0.0.1, port: {port}}}\nmanagement: {{secret-key: isolated-test-secret, disable-control-panel: true, disable-auto-update-panel: true}}\noauth: {{auth-dir: '{}' }}\napi-keys: [isolated-client-key]\ncodex-api-key:\n  - api-key: upstream-test\n    base-url: http://127.0.0.1:1/v1\n    models: [{{name: model-one}}]\n", path_to_string(&core.directory.join("auth")));
    fs::write(&core.config, initial).unwrap();
    core.start().await;
    let current = fetch_management_config_yaml(&gui).await.unwrap();
    let mut updated: serde_norway::Value = serde_norway::from_str(&current).unwrap();
    updated["codex-api-key"][0]["models"].as_sequence_mut().unwrap()
        .push(serde_norway::from_str("{name: model-two}").unwrap());
    put_management_alias_config_changes(&gui, &current, &serde_norway::to_string(&updated).unwrap()).await.unwrap();
    let saved = core.config_view().await;
    assert_eq!(saved["access"]["api-keys"], serde_json::json!(["isolated-client-key"]));
    assert_eq!(saved["api-keys"]["codex"][0]["models"], serde_json::json!([{"name":"model-one"},{"name":"model-two"}]));
    assert_eq!(saved["api-keys"]["codex"][0]["keys"][0]["api-key"], "upstream-test");
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v8_alias_save_preserves_multi_key_groups_and_restores_inheritance_after_failure() {
    let executable = fs::canonicalize(std::env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE")).unwrap();
    let directory = agent_test_home("v8-contract-groups");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut core = TestCore {
        child: None, config: directory.join("config.yaml"), directory, executable,
        origin: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap(),
    };
    let gui = GuiConfigFile {
        host: "127.0.0.1".into(), port, auth_dir: path_to_string(&core.directory.join("auth")),
        management_secret_key: "isolated-test-secret".into(),
        api_keys: vec![GuiApiKeyEntry { key: "isolated-client-key".into(), remark: String::new() }],
        ..GuiConfigFile::default()
    };
    let initial = r#"
config-version: 8
management: {disable-control-panel: true, disable-auto-update-panel: true}
api-keys:
  codex:
    - name: shared-upstream
      base-url: http://127.0.0.1:1/v1
      models: [{name: gpt-test}]
      keys:
        - {api-key: test-first, weight: 2, priority: null}
        - {api-key: test-second, weight: 5, models: null, headers: {}, excluded-models: [], disable-cooling: false}
        - api-key: test-third
          models: [{name: private-model}]
"#;
    fs::write(&core.config, apply_gui_managed_settings(initial, &gui).unwrap()).unwrap();
    core.start().await;
    core.validate_save(&fs::read_to_string(&core.config).unwrap()).await;
    let original = core.config_view().await;
    let current = fetch_management_config_yaml(&gui).await.unwrap();
    let sources = resolved_thinking_alias_sources(&current, &[], &super::support::test_agent_models(&["gpt-test"])).unwrap();
    let updated = add_model_alias_to_yaml(&current, &sources[0], "gpt-test-fast", "", false).unwrap();
    let error = commit_management_alias_config_changes(&gui, &current, &updated, || Err::<(), _>("local save failed".into())).await.unwrap_err();
    assert!(error.contains("Original configuration was restored"), "{error}");
    assert_eq!(core.config_view().await["api-keys"], original["api-keys"]);

    let current = fetch_management_config_yaml(&gui).await.unwrap();
    let sources = resolved_thinking_alias_sources(&current, &[], &super::support::test_agent_models(&["gpt-test"])).unwrap();
    let updated = add_model_alias_to_yaml(&current, &sources[0], "gpt-test-fast", "", false).unwrap();
    put_management_alias_config_changes(&gui, &current, &updated).await.unwrap();
    let saved = core.config_view().await;
    let group = &saved["api-keys"]["codex"][0];
    assert_eq!(saved["api-keys"]["codex"].as_array().unwrap().len(), 1);
    assert_eq!(group["name"], "shared-upstream");
    assert_eq!(group["keys"].as_array().unwrap().len(), 3);
    assert_eq!(group["models"], original["api-keys"]["codex"][0]["models"]);
    assert_eq!(group["keys"][0]["models"][1]["alias"], "gpt-test-fast");
    assert_eq!(group["keys"][1], original["api-keys"]["codex"][0]["keys"][1]);
    assert_eq!(group["keys"][2], original["api-keys"]["codex"][0]["keys"][2]);
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v8_accepts_gui_settings_and_reloads_client_keys() {
    let executable =
        fs::canonicalize(std::env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE"))
            .unwrap();
    let directory = agent_test_home("v8-contract");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut core = TestCore {
        child: None,
        config: directory.join("config.yaml"),
        directory,
        executable,
        origin: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
    };
    let mut gui = GuiConfigFile {
        host: "127.0.0.1".into(),
        port,
        auth_dir: path_to_string(&core.directory.join("auth")),
        management_secret_key: "isolated-test-secret".into(),
        api_keys: vec![GuiApiKeyEntry {
            key: "client-one".into(),
            remark: String::new(),
        }],
        proxy_url: "direct".into(),
        routing_strategy: "round-robin".into(),
        routing_session_affinity: true,
        routing_session_affinity_ttl: "2h30m".into(),
        request_retry: 2,
        max_retry_credentials: 3,
        max_retry_interval: 5,
        streaming_bootstrap_retries: 2,
        disable_cooling: true,
        debug: true,
        commercial_mode: true,
        logging_to_file: true,
        logs_max_total_size_mb: 5,
        error_logs_max_files: 4,
        usage_statistics_enabled: true,
        redis_usage_queue_retention_seconds: 90,
        request_log: true,
        ..GuiConfigFile::default()
    };
    let initial = "config-version: 8\nmanagement: {disable-control-panel: true, disable-auto-update-panel: true}\nrequests: null\nobservability: null\noauth: {providers: {codex: {header-defaults: {user-agent: preserve-test}}}}\napi-keys: {openai-compatibility: [{name: preserved, base-url: 'http://127.0.0.1:1/v1', models: [{name: test-model}], keys: [{api-key: upstream-test}]}]}\n";
    let content = apply_gui_managed_settings(initial, &gui).unwrap();
    let content = patch_core_sensitive_words_yaml(
        &content,
        &CoreSensitiveWordsSettings {
            antigravity_sensitive_words: vec!["sample".into()],
            devin_sensitive_words: vec!["sample-devin".into()],
        },
    )
    .unwrap()
    .unwrap();
    fs::write(&core.config, &content).unwrap();
    core.start().await;
    let view = core.validate_save(&content).await;
    assert_eq!(view["routing"]["strategy"], "round-robin");
    assert_eq!(view["routing"]["retry"]["request-retry"], 2);
    assert_eq!(view["routing"]["retry"]["max-retry-credentials"], 3);
    assert_eq!(view["routing"]["retry"]["max-retry-interval"], 5);
    assert_eq!(view["requests"]["streaming"]["bootstrap-retries"], 2);
    assert_eq!(view["routing"]["cooldown"]["disable-cooling"], true);
    assert_eq!(
        view["observability"]["usage"]["redis-usage-queue-retention-seconds"],
        90
    );
    assert_eq!(
        view["oauth"]["providers"]["devin"]["sensitive-words"][0],
        "sample-devin"
    );

    gui.request_retry = 0;
    gui.max_retry_credentials = 0;
    gui.max_retry_interval = 0;
    gui.streaming_bootstrap_retries = 0;
    gui.disable_cooling = false;
    gui.routing_session_affinity = false;
    gui.routing_session_affinity_ttl.clear();
    gui.proxy_url.clear();
    let current = fs::read_to_string(&core.config).unwrap();
    let content = patch_core_retry_yaml(&current, &gui).unwrap().unwrap();
    let content = patch_core_session_routing_yaml(&content, &gui)
        .unwrap()
        .unwrap();
    let content = patch_core_network_endpoint_yaml(&content, &gui)
        .unwrap()
        .unwrap();
    let mut logging =
        core_config_settings_from_value(&serde_norway::from_str(&content).unwrap()).unwrap();
    logging.debug = false;
    logging.commercial_mode = false;
    logging.logging_to_file = false;
    logging.logs_max_total_size_mb = 0;
    logging.error_logs_max_files = 0;
    logging.usage_statistics_enabled = false;
    logging.redis_usage_queue_retention_seconds = 1;
    let content =
        patch_core_yaml_document(&content, |doc| apply_core_logging_settings(doc, &logging))
            .unwrap()
            .unwrap();
    let content = patch_core_sensitive_words_yaml(&content, &CoreSensitiveWordsSettings::default())
        .unwrap()
        .unwrap();
    let content = patch_core_tls_settings_yaml(
        &content,
        &CoreTlsSettings {
            enabled: false,
            cert: "test-cert.pem".into(),
            key: "test-key.pem".into(),
        },
    )
    .unwrap()
    .unwrap();
    let view = core.validate_save(&content).await;
    assert_eq!(view["routing"]["retry"]["request-retry"], 0);
    assert_eq!(view["routing"]["session-affinity"], false);
    assert_eq!(view["routing"]["session-affinity-ttl"], "");
    assert_eq!(view["routing"]["cooldown"]["disable-cooling"], false);
    assert_eq!(
        view["observability"]["usage"]["usage-statistics-enabled"],
        false
    );
    assert_eq!(view["observability"]["logs"]["logging-to-file"], false);
    assert_eq!(view["observability"]["logs"]["request-log"], true);
    assert_eq!(view["server"]["tls"]["cert"], "test-cert.pem");
    assert_eq!(view["server"]["tls"]["enable"], false);
    assert_eq!(
        view["api-keys"]["openai-compatibility"][0]["name"],
        "preserved"
    );
    assert_eq!(
        view["oauth"]["providers"]["codex"]["header-defaults"]["user-agent"],
        "preserve-test"
    );
    assert_eq!(
        view["oauth"]["providers"]["antigravity"]["sensitive-words"],
        serde_json::json!([])
    );
    let current = fs::read_to_string(&core.config).unwrap();
    let content = patch_core_api_keys_yaml(&current, &["client-two".into()]).unwrap();
    write_core_config_if_changed(&core.config, &content).unwrap();
    core.wait_for_client_key("client-two", 200).await;
    core.wait_for_client_key("client-one", 401).await;
    core.stop();
    core.start().await;
    core.wait_for_client_key("client-two", 200).await;
    core.wait_for_client_key("client-one", 401).await;
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE pointing to a v8 executable"]
async fn v8_reloads_usage_after_repeated_gui_saves_in_both_config_layouts() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    for initial in [
        "remote-management: {disable-control-panel: true, disable-auto-update-panel: true}\nusage-statistics-enabled: false\n",
        "config-version: 8\nmanagement: {disable-control-panel: true, disable-auto-update-panel: true}\nobservability: {usage: {usage-statistics-enabled: false}}\nusage-statistics-enabled: true\n",
    ] {
        let executable = fs::canonicalize(std::env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE")).unwrap();
        let directory = agent_test_home("v8-contract-usage");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut core = TestCore {
            child: None,
            config: directory.join("config.yaml"),
            directory,
            executable,
            origin: format!("http://127.0.0.1:{port}"),
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap(),
        };
        let gui = GuiConfigFile {
            host: "127.0.0.1".into(),
            port,
            auth_dir: path_to_string(&core.directory.join("existing-credentials")),
            management_secret_key: "isolated-test-secret".into(),
            usage_statistics_enabled: false,
            ..GuiConfigFile::default()
        };
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let upstream_task = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut stream, _) = upstream.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                        }).unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = r#"{"id":"usage-test","object":"chat.completion","model":"usage-probe","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#;
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        fs::create_dir_all(&gui.auth_dir).unwrap();
        let initial = format!("{initial}openai-compatibility:\n  - name: isolated-usage-test\n    base-url: http://127.0.0.1:{upstream_port}/v1\n    api-key-entries: [{{api-key: isolated-upstream-key}}]\n    models: [{{name: usage-probe, alias: usage-probe}}]\n");
        let content = apply_gui_managed_settings(&initial, &gui).unwrap();
        fs::write(&core.config, &content).unwrap();
        core.start().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(core.config_view().await["observability"]["usage"]["usage-statistics-enabled"], false);

        for enabled in [true, false, true] {
            let content = fs::read_to_string(&core.config).unwrap();
            let mut settings = core_config_settings_from_value(&serde_norway::from_str(&content).unwrap()).unwrap();
            assert_eq!(settings.auth_dir, gui.auth_dir);
            settings.usage_statistics_enabled = enabled;
            let updated = patch_core_yaml_document(&content, |document| apply_core_logging_settings(document, &settings)).unwrap().unwrap();
            write_core_config_if_changed(&core.config, &updated).unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
            let saved = fs::read_to_string(&core.config).unwrap();
            let settings = core_config_settings_from_value(&serde_norway::from_str(&saved).unwrap()).unwrap();
            assert_eq!(settings.auth_dir, gui.auth_dir);
            assert_eq!(settings.usage_statistics_enabled, enabled);
            core.client.post(format!("{}/v1/chat/completions", core.origin))
                .bearer_auth(&gui.api_keys[0].key)
                .json(&serde_json::json!({"model": "usage-probe", "messages": [{"role": "user", "content": "test"}]}))
                .send().await.unwrap().error_for_status().unwrap()
                .bytes().await.unwrap();
            let mut records = Vec::new();
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let batch: Vec<serde_json::Value> = core.client
                    .get(format!("{}/v8/management/observability/usage/queue?count=100", core.origin))
                    .bearer_auth("isolated-test-secret")
                    .send().await.unwrap().error_for_status().unwrap()
                    .json().await.unwrap();
                records.extend(batch);
                if !records.is_empty() {
                    break;
                }
            }
            assert_eq!(records.len(), usize::from(enabled), "usage output did not follow the saved setting");
        }
        upstream_task.await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires CPA_V8_TEST_CORE and CPA_LOCAL_TEST_PLUGIN"]
async fn local_plugin_copy_is_discovered_and_loads_in_real_kernel() {
    let executable = fs::canonicalize(env::var_os("CPA_V8_TEST_CORE").expect("set CPA_V8_TEST_CORE")).unwrap();
    let source = fs::canonicalize(env::var_os("CPA_LOCAL_TEST_PLUGIN").expect("set CPA_LOCAL_TEST_PLUGIN")).unwrap();
    let directory = agent_test_home("v8-contract-local-plugin");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut core = TestCore {
        child: None, config: directory.join("config.yaml"), directory, executable,
        origin: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap(),
    };
    let gui = GuiConfigFile {
        port, plugins_enabled: true, management_secret_key: "isolated-test-secret".into(),
        auth_dir: path_to_string(&core.directory.join("auth")),
        api_keys: vec![GuiApiKeyEntry { key: "isolated-client-key".into(), remark: String::new() }],
        ..GuiConfigFile::default()
    };
    let initial = "config-version: 8\nmanagement: {disable-control-panel: true, disable-auto-update-panel: true}\nplugins: {enabled: true, dir: plugins}\n";
    fs::write(&core.config, merge_core_config_yaml(initial, None, &gui).unwrap()).unwrap();
    core.start().await;
    let list = |client: reqwest::Client, origin: String| async move {
        client.get(format!("{origin}/v8/management/plugins")).bearer_auth("isolated-test-secret")
            .send().await.unwrap().error_for_status().unwrap().json::<serde_json::Value>().await.unwrap()
    };
    let before = list(core.client.clone(), core.origin.clone()).await;
    let returned_dir = PathBuf::from(before["plugins_dir"].as_str().unwrap());
    let root = if returned_dir.is_absolute() { returned_dir } else { core.directory.join(returned_dir) };
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os="macos") { "darwin" } else { "linux" };
    let arch = if cfg!(target_arch="aarch64") { "arm64" } else { "amd64" };
    let extension = source.extension().unwrap().to_str().unwrap();
    let installed = crate::plugins::local_install::copy_plugin(&source, &root.join(os).join(arch), extension).unwrap();
    assert_eq!(fs::read(&installed).unwrap(), fs::read(&source).unwrap());
    let after = list(core.client.clone(), core.origin.clone()).await;
    let entry = after["plugins"].as_array().unwrap().iter().find(|entry| {
        Path::new(entry["path"].as_str().unwrap_or("")).file_name() == source.file_name()
    }).expect("copied plugin must be discovered");
    let id = entry["id"].as_str().unwrap().to_string();
    core.client.put(format!("{}/v8/management/config/plugins/configs/{id}/enabled", core.origin))
        .bearer_auth("isolated-test-secret").json(&true)
        .send().await.unwrap().error_for_status().unwrap();
    core.stop();
    core.start().await;
    let loaded = list(core.client.clone(), core.origin.clone()).await;
    let entry = loaded["plugins"].as_array().unwrap().iter().find(|entry| entry["id"] == id).unwrap();
    assert_eq!(entry["registered"], true, "plugin must load: {entry}");
    assert_eq!(entry["effective_enabled"], true, "plugin must be enabled: {entry}");
    println!("Verified actual kernel discovery and loading: {id}");
}
