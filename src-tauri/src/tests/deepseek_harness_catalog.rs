use super::support::*;
use super::*;
use serde_json::{json, Value};

fn models() -> Vec<AgentModelOption> {
    parse_agent_model_options(&json!({"data":[
        {"id":"vision","input_modalities":["text","image"],"context_window":128000},
        {"id":"text-only","inputModalities":["text"]},
        {"id":"unknown"}
    ]}))
    .unwrap()
}

fn root(home: &Path) -> Value {
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, home);
    serde_json::to_value(
        serde_norway::from_str::<serde_norway::Value>(&fs::read_to_string(&paths[0]).unwrap())
            .unwrap(),
    )
    .unwrap()
}

fn provider(home: &Path) -> Value {
    root(home)["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID].clone()
}

fn apply(home: &Path, models: &[AgentModelOption]) -> AgentConfigActionResult {
    apply_agent_configuration(
        AgentClient::DeepSeekHarness,
        home,
        8317,
        "test-key",
        "vision",
        models,
        None,
    )
    .unwrap()
}

fn request(snapshot: HarnessEditorSnapshot) -> HarnessEditorRequest {
    HarnessEditorRequest {
        revision: snapshot.revision,
        models: snapshot.models,
        provider: snapshot.provider,
    }
}

#[test]
fn harness_api_defaults_support_aliases_and_model_maps_without_guessing() {
    let models = parse_agent_model_options(&json!({"data":[
        {"id":"original","alias":"alias","fork":true,"supported_input_modalities":[" TEXT ","IMAGE","image"],"max_output_tokens":8192,"reasoning_efforts":{"off":null,"high":"high"}},
        {"id":"camel-modalities","supportedInputModalities":["text","image","audio"],"max_completion_tokens":65536},
        {"id":"camel-tokens","maxCompletionTokens":32768},
        {"id":"empty","input_modalities":[]}, {"id":"gpt-known-name"}
    ]})).unwrap();
    assert_eq!(
        models[0].input_modalities,
        Some(vec!["text".into(), "image".into()])
    );
    assert_eq!(models[0].harness_metadata, models[1].harness_metadata);
    assert_eq!(models[1].name, "alias");
    assert_eq!(
        models[1].harness_metadata.as_ref().unwrap()["maxTokens"],
        8192
    );
    assert_eq!(
        models[2].input_modalities,
        Some(vec!["text".into(), "image".into()])
    );
    assert_eq!(
        models[2].harness_metadata.as_ref().unwrap()["maxTokens"],
        65536
    );
    assert_eq!(
        models[3].harness_metadata.as_ref().unwrap()["maxTokens"],
        32768
    );
    assert!(models[3..]
        .iter()
        .all(|model| model.input_modalities.is_none()));
    let mapped =
        parse_agent_model_options(&json!({"models":{"vision":{"input":["image"]},"unknown":{}}}))
            .unwrap();
    assert_eq!(
        mapped
            .iter()
            .find(|m| m.name == "vision")
            .unwrap()
            .input_modalities,
        Some(vec!["image".into()])
    );
}

#[tokio::test]
async fn harness_discovery_uses_authenticated_models_endpoint() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = socket.read(&mut buffer).unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
        }
        let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models http/1.1\r\n"));
        assert!(request.contains("authorization: bearer test-harness-key\r\n"));
        assert!(!request.contains("client_version"));
        let body =
            r#"{"data":[{"id":"vision","input_modalities":["text","image"]},{"id":"unknown"}]}"#;
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let models = fetch_agent_models(port, "test-harness-key").await.unwrap();
    server.join().unwrap();
    assert_eq!(models.len(), 2);
    assert!(models[1].input_modalities.is_none());
}

#[test]
fn harness_context_enrichment_matches_codex_and_preserves_manual_overrides() {
    let home = agent_test_home("harness-context-enrichment");
    let mut models = models();
    let runtime = codex_catalog::parse_runtime_models(&json!({"models":[
        {"slug":"VISION","context_window":272000,"max_context_window":872000,"input_modalities":["text"]},
        {"slug":"text-only","max_context_window":131072,"input_modalities":["text","image"]},
        {"slug":"unknown","input_modalities":["text","image"]}
    ]})).unwrap();
    enrich_harness_context_windows(&mut models, &runtime, None).unwrap();
    assert_eq!(models[0].context_window, Some(272000));
    assert_eq!(models[1].context_window, Some(131072));
    assert_eq!(models[2].context_window, None);
    assert_eq!(models[1].input_modalities, Some(vec!["text".into()]));
    assert_eq!(models[2].input_modalities, None);
    enrich_harness_context_windows(&mut models, &[], Some("codex-api-key:\n  - models:\n      - name: upstream-model\n        alias: vision\n        max-context-length: 64000\n")).unwrap();
    let snapshot = harness_editor_snapshot(&home, &models, 8317).unwrap();
    assert_eq!(snapshot.models[0].defaults["contextWindow"], 64000);
    assert!(snapshot
        .models
        .iter()
        .all(|model| model.configuration.is_empty()));
    apply(&home, &models);
    assert_eq!(provider(&home)["models"][0]["contextWindow"], 64000);
    let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    edit.models[0]
        .configuration
        .insert("contextWindow".into(), json!(96000));
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    enrich_harness_context_windows(&mut models, &runtime, None).unwrap();
    apply(&home, &models);
    assert_eq!(provider(&home)["models"][0]["contextWindow"], 96000);
    let mut reset = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    reset.models[0].configuration.clear();
    save_harness_editor(&home, &models, 8317, reset).unwrap();
    assert_eq!(provider(&home)["models"][0]["contextWindow"], 272000);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_refreshes_api_defaults_and_preserves_catalog_overrides() {
    let home = agent_test_home("harness-catalog-refresh");
    let mut models = models();
    apply(&home, &models);
    assert_eq!(
        provider(&home)["models"][0]["input"],
        json!(["text", "image"])
    );
    assert!(provider(&home)["models"][2].get("input").is_none());
    assert_eq!(apply(&home, &models).outcome, "unchanged");
    let initial = harness_editor_snapshot(&home, &models, 8317).unwrap();
    assert!(initial.models.iter().all(|m| m.configuration.is_empty()));
    let mut edit = request(initial);
    edit.models[1].configuration = json!({"name":"Text model","maxTokens":4096,"reasoningEfforts":false,"compat":{"supportsDeveloperRole":false}}).as_object().unwrap().clone();
    edit.models[2].configuration = json!({"input":["text","image"]})
        .as_object()
        .unwrap()
        .clone();
    edit.provider = json!({"defaultInput":["text","image"],"timeoutMs":60000,"retryPolicy":{"mode":"normal","maxRetries":3}}).as_object().unwrap().clone();
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    models[0].input_modalities = Some(vec!["text".into()]);
    models[0].context_window = Some(256000);
    apply(&home, &models);
    let provider = provider(&home);
    assert_eq!(provider["models"][0]["input"], json!(["text"]));
    assert_eq!(provider["models"][0]["contextWindow"], 256000);
    assert_eq!(provider["models"][1]["name"], "Text model");
    assert_eq!(provider["models"][1]["maxTokens"], 4096);
    assert_eq!(provider["models"][2]["input"], json!(["text", "image"]));
    assert_eq!(provider["defaultInput"], json!(["text", "image"]));
    assert_eq!(root(&home)["agent-default-model"]["model"], "vision");
    let mut reset = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    reset
        .models
        .iter_mut()
        .for_each(|m| m.configuration.clear());
    reset.provider.clear();
    save_harness_editor(&home, &models, 8317, reset).unwrap();
    apply(&home, &models);
    let updated = self::provider(&home);
    assert!(updated["models"][1].get("maxTokens").is_none());
    assert!(updated["models"][1].get("compat").is_none());
    assert!(updated["models"][2].get("input").is_none());
    assert!(updated.get("defaultInput").is_none());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_saves_before_connection_and_applies_documented_protocol() {
    let home = agent_test_home("harness-catalog-draft");
    let mut models = models();
    models[2].harness_metadata = harness_api_metadata(&json!({"compat":{
        "supportsStore":false,"supportsMaxOutputTokens":false,"supportsTemperature":false
    }}));
    let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    edit.provider = json!({"api":"openai-responses","compat":{"supportsMaxOutputTokens":false},"transport":"sse"}).as_object().unwrap().clone();
    edit.models[2]
        .configuration
        .insert("input".into(), json!(["text", "image"]));
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    assert!(!paths[0].exists());
    assert!(!paths[1].exists());
    apply(&home, &models);
    assert_eq!(provider(&home)["api"], "openai-responses");
    assert_eq!(
        provider(&home)["models"][2]["compat"],
        json!({"supportsMaxOutputTokens":false})
    );
    assert_eq!(
        provider(&home)["models"][2]["input"],
        json!(["text", "image"])
    );
    assert!(
        inspect_deepseek_harness_config(&paths, 8317, "test-key")
            .unwrap()
            .0
    );
    let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    edit.provider = json!({"api":"anthropic-messages","compat":{"supportsTemperature":false}})
        .as_object()
        .unwrap()
        .clone();
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    assert_eq!(provider(&home)["baseURL"], "http://127.0.0.1:8317");
    assert_eq!(
        provider(&home)["models"][2]["compat"],
        json!({"supportsTemperature":false})
    );
    assert!(
        inspect_deepseek_harness_config(&paths, 8317, "test-key")
            .unwrap()
            .0
    );
    apply(&home, &models);
    assert_eq!(provider(&home)["baseURL"], "http://127.0.0.1:8317");

    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_rejects_empty_catalog_without_modifying_configured_files() {
    let home = agent_test_home("harness-empty-catalog");
    apply(&home, &models());
    let mut paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    paths.push(deepseek_harness_catalog_state_path(&paths).unwrap());
    for selection in [
        root(&home).get("agent-default-model").cloned(),
        Some(json!({"provider":"deepseek","model":"deepseek-chat"})),
        None,
    ] {
        let mut document = root(&home);
        if let Some(selection) = selection {
            document["agent-default-model"] = selection;
        } else {
            document
                .as_object_mut()
                .unwrap()
                .remove("agent-default-model");
        }
        fs::write(&paths[0], serde_norway::to_string(&document).unwrap()).unwrap();
        let before = config_images(&paths).unwrap();
        let mut edit = request(harness_editor_snapshot(&home, &[], 8317).unwrap());
        edit.provider.insert("timeoutMs".into(), json!(60000));
        assert!(save_harness_editor(&home, &[], 8317, edit).is_err());
        assert_eq!(config_images(&paths).unwrap(), before);
        assert_eq!(provider(&home)["models"].as_array().unwrap().len(), 3);
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_allows_empty_catalog_for_unconnected_provider_drafts() {
    let home = agent_test_home("harness-empty-catalog-draft");
    let mut edit = request(harness_editor_snapshot(&home, &[], 8317).unwrap());
    edit.provider.insert("timeoutMs".into(), json!(60000));
    save_harness_editor(&home, &[], 8317, edit).unwrap();
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(
        harness_editor_snapshot(&home, &[], 8317).unwrap().provider["timeoutMs"],
        60000
    );
    apply(&home, &models());
    assert_eq!(provider(&home)["timeoutMs"], 60000);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_rejects_stale_catalog_and_invalid_fields_without_writing() {
    let home = agent_test_home("harness-catalog-validation");
    let models = models();
    apply(&home, &models);
    let mut changed_models = models.clone();
    changed_models[0].context_window = Some(64);
    let stale = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    assert!(save_harness_editor(&home, &changed_models, 8317, stale)
        .err()
        .unwrap()
        .contains("DSH_MODEL_CATALOG_CHANGED"));
    let mut paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    paths.push(deepseek_harness_catalog_state_path(&paths).unwrap());
    let before = config_images(&paths).unwrap();
    for configuration in [
        json!({"contextWindow":0}),
        json!({"maxTokens":1.5}),
        json!({"input":["audio"]}),
        json!({"reasoningEfforts":{"high":null}}),
        json!({"reasoningEfforts":{"off":null}}),
        json!({"reasoningEfforts":{"off":"none"}}),
        json!({"reasoningEfforts":{}}),
        json!({"compat":{"supportsTemperature":true}}),
        json!({"compat":{"supportsStore":null}}),
        json!({"compat":{"vllmPriority":0.5}}),
        json!({"unknownField":true}),
    ] {
        let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
        edit.models[0].configuration = configuration.as_object().unwrap().clone();
        assert!(save_harness_editor(&home, &models, 8317, edit).is_err());
        assert_eq!(config_images(&paths).unwrap(), before);
    }
    for configuration in [
        json!({"defaultInput":[]}),
        json!({"streamIdleTimeoutMs":2147483648_u64}),
        json!({"headers":{"x-test":"bad\r\nheader"}}),
        json!({"retryPolicy":{"mode":"normal","backoff":{"initialDelayMs":20000}}}),
        json!({"apiKeyEnv":"UNMANAGED"}),
    ] {
        let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
        edit.provider = configuration.as_object().unwrap().clone();
        assert!(save_harness_editor(&home, &models, 8317, edit).is_err());
        assert_eq!(config_images(&paths).unwrap(), before);
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_imports_external_edits_and_keeps_default_selection_and_other_settings() {
    let home = agent_test_home("harness-catalog-external");
    let models = models();
    apply(&home, &models);
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    let stale = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    let mut document = root(&home);
    document["agent-default-model"]["reasoningEffort"] = json!("high");
    document["unrelated"] = json!({"enabled":true});
    document["llm-pi-ai"]["providers"]["another"] = json!({"apiKeyEnv":"OTHER"});
    document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]["name"] =
        json!("Manual name");
    document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]["input"] =
        json!(["text"]);
    document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]
        ["futureExtension"] = json!({"keep":true});
    fs::write(&paths[0], serde_norway::to_string(&document).unwrap()).unwrap();
    assert!(save_harness_editor(&home, &models, 8317, stale).is_err());
    let edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    assert_eq!(edit.models[0].configuration["name"], "Manual name");
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    apply(&home, &models);
    assert_eq!(
        root(&home)["agent-default-model"]["reasoningEffort"],
        "high"
    );
    assert_eq!(root(&home)["unrelated"], json!({"enabled":true}));
    assert_eq!(
        provider(&home)["models"][0]["futureExtension"],
        json!({"keep":true})
    );
    assert_eq!(provider(&home)["models"][0]["input"], json!(["text"]));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_imports_compat_changes_without_freezing_other_api_defaults() {
    let home = agent_test_home("harness-compat-import");
    let mut models = models();
    models[0].harness_metadata = harness_api_metadata(&json!({"compat":{
        "supportsStore":true,"supportsDeveloperRole":true,"supportsStrictMode":true
    }}));
    apply(&home, &models);
    let mut edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    edit.models[0]
        .configuration
        .insert("compat".into(), json!({"supportsStrictMode":false}));
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    let mut document = root(&home);
    document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]["compat"]
        ["supportsStore"] = json!(false);
    fs::write(&paths[0], serde_norway::to_string(&document).unwrap()).unwrap();
    let edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    assert_eq!(
        edit.models[0].configuration["compat"],
        json!({
            "supportsStore":false,"supportsStrictMode":false
        })
    );
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    models[0].harness_metadata = harness_api_metadata(&json!({"compat":{
        "supportsStore":true,"supportsDeveloperRole":false,"supportsStrictMode":true
    }}));
    apply(&home, &models);
    assert_eq!(
        provider(&home)["models"][0]["compat"],
        json!({
            "supportsStore":false,"supportsDeveloperRole":false,"supportsStrictMode":false
        })
    );

    let mut document = root(&home);
    let compat = document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]
        ["compat"]
        .as_object_mut()
        .unwrap();
    compat.remove("supportsStore");
    compat.insert("maxTokensField".into(), json!("max_tokens"));
    fs::write(&paths[0], serde_norway::to_string(&document).unwrap()).unwrap();
    let edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    assert_eq!(
        edit.models[0].configuration["compat"],
        json!({
            "supportsStrictMode":false,"maxTokensField":"max_tokens"
        })
    );
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    assert_eq!(
        provider(&home)["models"][0]["compat"]["supportsStore"],
        true
    );

    let mut document = root(&home);
    document["llm-pi-ai"]["providers"][DEEPSEEK_HARNESS_PROVIDER_ID]["models"][0]
        .as_object_mut()
        .unwrap()
        .remove("compat");
    fs::write(&paths[0], serde_norway::to_string(&document).unwrap()).unwrap();
    let edit = request(harness_editor_snapshot(&home, &models, 8317).unwrap());
    assert!(!edit.models[0].configuration.contains_key("compat"));
    save_harness_editor(&home, &models, 8317, edit).unwrap();
    assert_eq!(
        provider(&home)["models"][0]["compat"],
        json!({
            "supportsStore":true,"supportsDeveloperRole":false,"supportsStrictMode":true
        })
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_state_failure_rolls_back_settings_and_credentials() {
    let home = agent_test_home("harness-catalog-rollback");
    apply(&home, &models());
    let mut changed = models();
    changed[0].input_modalities = Some(vec!["text".into()]);
    let (paths, before, after) =
        prepare_deepseek_harness_configuration(&home, 8317, "new-key", "vision", &changed).unwrap();
    let mut failed = false;
    let result = commit_config_with_writer(
        "deepseek-harness",
        &paths,
        &before,
        &after,
        "update",
        Some("vision".into()),
        None,
        &mut |client, images| {
            for image in images {
                if image.0 == paths[2] && !failed {
                    failed = true;
                    return Err("simulated failure".into());
                }
                write_config_images(client, &vec![image.clone()])?;
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(config_images(&paths).unwrap(), before);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn harness_template_and_backup_restore_reconcile_catalog_state() {
    let home = agent_test_home("harness-catalog-template");
    let paths = agent_config_paths(AgentClient::DeepSeekHarness, &home);
    let before = config_images(&paths).unwrap();
    let settings =
        build_deepseek_harness_settings(None, "http://127.0.0.1:8317/v1", "vision", &models())
            .unwrap();
    let after = vec![
        (paths[0].clone(), Some(settings.into_bytes())),
        (
            paths[1].clone(),
            Some(
                build_deepseek_harness_credentials(None, "test-key")
                    .unwrap()
                    .into_bytes(),
            ),
        ),
    ];
    commit_config(
        "deepseek-harness",
        &paths,
        &before,
        &after,
        "template",
        Some("vision".into()),
    )
    .unwrap();
    assert!(harness_editor_snapshot(&home, &models(), 8317)
        .unwrap()
        .models
        .iter()
        .all(|m| m.configuration.is_empty()));
    let state_path = deepseek_harness_catalog_state_path(&paths).unwrap();
    assert!(state_path.is_file());
    commit_config(
        "deepseek-harness",
        &paths,
        &after,
        &after,
        "restore",
        Some("vision".into()),
    )
    .unwrap();
    assert!(!state_path.exists());
    assert_eq!(
        harness_editor_snapshot(&home, &models(), 8317)
            .unwrap()
            .models[0]
            .configuration["input"],
        json!(["text", "image"])
    );
    fs::remove_dir_all(home).unwrap();
}
