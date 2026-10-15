use super::*;

pub(super) fn agent_test_home(name: &str) -> PathBuf {
    let path = crate::unique_test_temp_path(&format!("cpa-gui-agent-{name}"));
    fs::create_dir_all(&path).unwrap();
    path
}

pub(super) fn test_agent_models(names: &[&str]) -> Vec<AgentModelOption> {
    names
        .iter()
        .map(|name| AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: (*name).to_string(),
            alias: None,
            is_alias: false,
            context_window: Some(200_000),
        })
        .collect()
}

pub(super) fn test_codex_models(names: &[&str]) -> String {
    let payload = serde_json::json!({
        "models": names.iter().map(|name| serde_json::json!({ "id": name })).collect::<Vec<_>>()
    });
    let runtime = codex_catalog::parse_runtime_models(&payload).unwrap();
    codex_catalog::prepare_catalog(&runtime).unwrap().json
}

pub(super) fn test_codex_oauth_thinking_source(model: &str) -> ResolvedThinkingAliasSource {
    ResolvedThinkingAliasSource {
        source: ThinkingAliasSource {
            id: format!("codex-oauth:{model}"),
            model: model.to_string(),
            display_name: None,
            provider: "Codex OAuth".to_string(),
            kind: "codex-oauth".to_string(),
            protocol: "codex".to_string(),
            reasoning_levels: vec!["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
        location: ThinkingAliasSourceLocation::Oauth {
            channel: "codex",
            force_mapping: false,
        },
    }
}

#[test]
fn unique_test_temp_paths_do_not_collide_across_threads() {
    let threads = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..500)
                    .map(|_| crate::unique_test_temp_path("cpa-unique-path"))
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    for thread in threads {
        for path in thread.join().unwrap() {
            assert!(
                seen.insert(path.clone()),
                "duplicate test path {}",
                path.display()
            );
        }
    }
}
