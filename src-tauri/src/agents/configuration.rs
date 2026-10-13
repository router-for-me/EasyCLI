use super::*;

#[cfg(test)]
pub(crate) fn build_agent_updates(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    model: &str,
    models: &[AgentModelOption],
    codex_catalog: Option<&str>,
) -> Result<Vec<AgentFileUpdate>, String> {
    build_agent_updates_with_oauth(
        client,
        home,
        port,
        api_key,
        model,
        AgentConfigurationOptions {
            models,
            codex_catalog,
            oauth_configuration: false,
            claude_code_model_mappings: None,
            claude_desktop_model_mappings: None,
        },
    )
}

pub(crate) fn build_agent_updates_with_oauth(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    model: &str,
    options: AgentConfigurationOptions<'_>,
) -> Result<Vec<AgentFileUpdate>, String> {
    let AgentConfigurationOptions {
        models,
        codex_catalog,
        oauth_configuration,
        claude_code_model_mappings,
        claude_desktop_model_mappings,
    } = options;
    let paths = agent_config_paths(client, home);
    let root_base = managed_core_loopback_origin(port);
    let openai_base = format!("{root_base}/v1");
    match client {
        AgentClient::ClaudeCode => {
            let before = read_optional_text(&paths[0])?;
            let after = build_claude_agent_config(
                before.as_deref(),
                &root_base,
                api_key,
                model,
                models,
                claude_code_model_mappings,
            )
            ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::ClaudeDesktop => {
            if paths.len() != 4 {
                return Err("Claude Desktop configuration path is unavailable on the current platform".to_string());
            }
            let normal_before = read_optional_text(&paths[0])?;
            let threep_before = read_optional_text(&paths[1])?;
            let profile_before = read_optional_text(&paths[2])?;
            let meta_before = read_optional_text(&paths[3])?;
            Ok(vec![
                AgentFileUpdate {
                    path: paths[0].clone(),
                    after: build_claude_desktop_deployment_config(normal_before.as_deref())
                        ?,
                },
                AgentFileUpdate {
                    path: paths[1].clone(),
                    after: build_claude_desktop_deployment_config(threep_before.as_deref())
                        ?,
                },
                AgentFileUpdate {
                    path: paths[2].clone(),
                    after: build_claude_desktop_profile(
                        profile_before.as_deref(),
                        &root_base,
                        api_key,
                        model,
                        models,
                        claude_desktop_model_mappings,
                    )
                    ?,
                },
                AgentFileUpdate {
                    path: paths[3].clone(),
                    after: build_claude_desktop_meta(meta_before.as_deref())
                        ?,
                },
            ])
        }
        AgentClient::Codex => {
            let before = read_optional_text(&paths[0])?;
            let after = build_codex_agent_config_with_oauth(
                before.as_deref(),
                &openai_base,
                api_key,
                model,
                oauth_configuration,
            )
            ?;
            let mut updates = vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }];
            let catalog = codex_catalog.ok_or_else(|| "Unable to generate Codex model catalog".to_string())?;
            validate_codex_catalog(catalog, model)?;
            updates.push(AgentFileUpdate {
                path: codex_model_catalog_path(home),
                after: catalog.to_string(),
            });
            updates.push(build_codex_auth_update(home, api_key, oauth_configuration)?);
            Ok(updates)
        }
        AgentClient::OpenCode => {
            let before = read_optional_text(&paths[0])?;
            let after = build_opencode_agent_config(
                before.as_deref(),
                &openai_base,
                api_key,
                model,
                models,
            )
            ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::OpenClaw => {
            let before = read_optional_text(&paths[0])?;
            let after = build_openclaw_agent_config(
                before.as_deref(),
                &openai_base,
                api_key,
                model,
                models,
            )
            ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::Hermes => {
            let before = read_optional_text(&paths[0])?;
            let after =
                build_hermes_agent_config(before.as_deref(), &openai_base, api_key, model, models)
                    ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::DeepSeekHarness => {
            if paths.len() != 2 {
                return Err("Invalid number of DeepSeek Harness configuration paths".to_string());
            }
            let settings_before = read_optional_text(&paths[0])?;
            let credentials_before = read_optional_text(&paths[1])?;
            Ok(vec![
                AgentFileUpdate {
                    path: paths[0].clone(),
                    after: build_deepseek_harness_settings(
                        settings_before.as_deref(),
                        &openai_base,
                        model,
                        models,
                    )?,
                },
                AgentFileUpdate {
                    path: paths[1].clone(),
                    after: build_deepseek_harness_credentials(
                        credentials_before.as_deref(),
                        api_key,
                    )?,
                },
            ])
        }
        AgentClient::AntigravityCli => build_antigravity_updates(client, home, &root_base, api_key, model, false),
        AgentClient::WorkBuddy => {
            let before = read_optional_text(&paths[0])?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after: build_workbuddy_agent_config(before.as_deref(), &openai_base, api_key, model, models)?,
            }])
        }
        AgentClient::ZCode => {
            let before = read_optional_text(&paths[0])?;
            let after = build_zcode_agent_config(
                before.as_deref(), &openai_base, api_key, model, models,
            )?;
            Ok(vec![AgentFileUpdate { path: paths[0].clone(), after }])
        }
        AgentClient::KimiCode => {
            let before = read_optional_text(&paths[0])?;
            let after = build_kimi_code_agent_config(
                before.as_deref(),
                &openai_base,
                api_key,
                model,
                models,
            )
            ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::GrokBuild => {
            let before = read_optional_text(&paths[0])?;
            let after = build_grok_build_agent_config(
                before.as_deref(),
                &openai_base,
                api_key,
                model,
                models,
            )
            ?;
            Ok(vec![AgentFileUpdate {
                path: paths[0].clone(),
                after,
            }])
        }
        AgentClient::Omp => {
            if paths.len() != 2 {
                return Err("Invalid number of Oh My Pi configuration paths".to_string());
            }
            let models_before = read_optional_text(&paths[0])?;
            let settings_before = read_optional_text(&paths[1])?;
            Ok(vec![
                AgentFileUpdate {
                    path: paths[0].clone(),
                    after: build_omp_models_config(models_before.as_deref(), &openai_base, api_key)?,
                },
                AgentFileUpdate {
                    path: paths[1].clone(),
                    after: build_omp_settings_config(settings_before.as_deref(), model)?,
                },
            ])
        }
    }
}

pub(crate) fn read_optional_text(path: &Path) -> Result<Option<String>, String> {
    read_agent_bytes(path)?.map(|bytes| String::from_utf8(bytes)
        .map_err(|_| "Configuration is not UTF-8 text. Restore it from a manual backup or repair it with the base configuration template".to_string())).transpose()
}

pub(crate) fn read_agent_yaml_mapping_or_empty(
    path: &Path,
    label: &str,
) -> Result<serde_norway::Mapping, String> {
    if !path.is_file() {
        return Ok(serde_norway::Mapping::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path_to_string(path)))?;
    parse_agent_yaml_mapping(Some(&content), label)
}

pub(crate) fn parse_agent_yaml_mapping(
    content: Option<&str>,
    label: &str,
) -> Result<serde_norway::Mapping, String> {
    let Some(content) = content.map(str::trim).filter(|content| !content.is_empty()) else {
        return Ok(serde_norway::Mapping::new());
    };
    serde_norway::from_str::<serde_norway::Value>(content)
        .map_err(|error| format!("Invalid {label} YAML format: {error}"))?
        .as_mapping()
        .cloned()
        .ok_or_else(|| format!("{label} root must be a YAML mapping"))
}

pub(crate) fn render_agent_yaml_mapping_update<F>(
    existing: Option<&str>,
    label: &str,
    update: F,
) -> Result<String, String>
where
    F: FnOnce(&mut serde_norway::Mapping) -> Result<(), String>,
{
    let original_mapping = parse_agent_yaml_mapping(existing, label)?;
    let original = serde_norway::Value::Mapping(original_mapping.clone());
    let mut updated_mapping = original_mapping;
    update(&mut updated_mapping)?;
    let updated = serde_norway::Value::Mapping(updated_mapping);
    if updated == original {
        return Ok(existing.unwrap_or("{}\n").to_string());
    }
    let source = existing
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(|_| existing.unwrap_or_default())
        .unwrap_or("{}\n");
    let mut document = yaml_serde_edit::YamlValue::parse(source)
        .map_err(|error| format!("Invalid {label} YAML format: {error}"))?;
    render_updated_core_yaml(&mut document, updated)
        .map_err(|error| format!("Failed to update {label}: {error}"))
}

pub(crate) fn build_deepseek_harness_models(
    models: &[AgentModelOption],
    selected_model: &str,
) -> Result<serde_norway::Value, String> {
    let entries = ordered_agent_models(models, selected_model)
        .into_iter()
        .map(|model| {
            let mut entry = serde_norway::Mapping::new();
            entry.insert(yaml_key("id"), serde_norway::Value::String(model.name));
            if let Some(name) = model.alias.filter(|name| !name.trim().is_empty()) {
                entry.insert(yaml_key("name"), serde_norway::Value::String(name));
            }
            if let Some(context_window) = model.context_window {
                entry.insert(
                    yaml_key("contextWindow"),
                    serde_norway::to_value(context_window)
                        .map_err(|error| format!("Failed to serialize Harness model context: {error}"))?,
                );
            }
            if let Some(input) = model.input_modalities {
                entry.insert(
                    yaml_key("input"),
                    serde_norway::to_value(input)
                        .map_err(|_| "Failed to serialize Harness model input types")?,
                );
            }
            Ok(serde_norway::Value::Mapping(entry))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(serde_norway::Value::Sequence(entries))
}

pub(crate) fn build_deepseek_harness_settings(
    existing: Option<&str>,
    base_url: &str,
    model: &str,
    models: &[AgentModelOption],
) -> Result<String, String> {
    build_deepseek_harness_catalog_settings(
        existing,
        base_url,
        model,
        models,
        &mut DeepSeekHarnessCatalogState::default(),
    )
}

pub(crate) fn render_deepseek_harness_settings(
    existing: Option<&str>,
    base_url: &str,
    model: &str,
    published_models: serde_norway::Value,
) -> Result<String, String> {
    render_agent_yaml_mapping_update(existing, "DeepSeek Harness settings", |root| {
        let llm = root
            .entry(yaml_key("llm-pi-ai"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "DeepSeek Harness llm-pi-ai must be a mapping".to_string())?;
        let providers = llm
            .entry(yaml_key("providers"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "DeepSeek Harness llm-pi-ai.providers must be a mapping".to_string())?;
        let provider = providers
            .entry(yaml_key(DEEPSEEK_HARNESS_PROVIDER_ID))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "DeepSeek Harness EasyCLIProxyAPI provider must be a mapping".to_string())?;
        for (key, value) in [
            ("displayName", "EasyCLIProxyAPI"),
            ("apiKeyEnv", DEEPSEEK_HARNESS_CREDENTIAL),
            ("api", "openai-completions"),
            ("baseURL", base_url),
        ] {
            provider.insert(
                yaml_key(key),
                serde_norway::Value::String(value.to_string()),
            );
        }
        provider.insert(yaml_key("models"), published_models);

        let selection = root
            .entry(yaml_key("agent-default-model"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "DeepSeek Harness agent-default-model must be a mapping".to_string())?;
        let same_model = selection.get(yaml_key("provider")).and_then(serde_norway::Value::as_str) == Some(DEEPSEEK_HARNESS_PROVIDER_ID)
            && selection.get(yaml_key("model")).and_then(serde_norway::Value::as_str) == Some(model);
        selection.insert(
            yaml_key("provider"),
            serde_norway::Value::String(DEEPSEEK_HARNESS_PROVIDER_ID.to_string()),
        );
        selection.insert(
            yaml_key("model"),
            serde_norway::Value::String(model.to_string()),
        );
        if !same_model { selection.remove(yaml_key("reasoningEffort")); }
        Ok(())
    })
}

pub(crate) fn build_deepseek_harness_credentials(
    existing: Option<&str>,
    api_key: &str,
) -> Result<String, String> {
    render_agent_yaml_mapping_update(existing, "DeepSeek Harness credentials", |root| {
        let refs = deepseek_harness_credentials_refs_mut(root, "DeepSeek Harness credentials")?;
        refs.insert(
            yaml_key(DEEPSEEK_HARNESS_CREDENTIAL),
            serde_norway::Value::String(api_key.to_string()),
        );
        Ok(())
    })
}

fn deepseek_harness_credentials_version_is_supported(value: &serde_norway::Value) -> bool {
    value.as_u64() == Some(DEEPSEEK_HARNESS_CREDENTIALS_VERSION)
        || value.as_i64() == Some(DEEPSEEK_HARNESS_CREDENTIALS_VERSION as i64)
}

fn validate_deepseek_harness_versioned_credentials(
    root: &serde_norway::Mapping,
    label: &str,
) -> Result<(), String> {
    let version =
        yaml_mapping_value(root, "version").ok_or_else(|| format!("{label} is missing the version field"))?;
    if !deepseek_harness_credentials_version_is_supported(version) {
        return Err(format!(
            "{label} version must be {DEEPSEEK_HARNESS_CREDENTIALS_VERSION}"
        ));
    }
    for key in root.keys() {
        let Some(key) = key.as_str() else {
            return Err(format!("Top-level field names in {label} must be strings"));
        };
        let _ = key;
    }
    for section in ["refs", "records"] {
        if yaml_mapping_value(root, section).is_some_and(|value| !value.is_mapping()) {
            return Err(format!("{label} {section} must be a mapping"));
        }
    }
    Ok(())
}

fn deepseek_harness_credentials_refs_mut<'a>(
    root: &'a mut serde_norway::Mapping,
    label: &str,
) -> Result<&'a mut serde_norway::Mapping, String> {
    if root.is_empty() {
        root.insert(
            yaml_key("version"),
            serde_norway::Value::Number(DEEPSEEK_HARNESS_CREDENTIALS_VERSION.into()),
        );
    } else {
        validate_deepseek_harness_versioned_credentials(root, label)?;
        root.remove(yaml_key(DEEPSEEK_HARNESS_CREDENTIAL));
    }

    root.entry(yaml_key("refs"))
        .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| format!("{label} refs must be a mapping"))
}

fn remove_deepseek_harness_managed_credential(
    root: &mut serde_norway::Mapping,
    label: &str,
) -> Result<bool, String> {
    if root.is_empty() {
        return Ok(false);
    }

    validate_deepseek_harness_versioned_credentials(root, label)?;
    let mut changed = root.remove(yaml_key(DEEPSEEK_HARNESS_CREDENTIAL)).is_some();
    let mut remove_refs = false;
    if let Some(refs) = root
        .get_mut(yaml_key("refs"))
        .and_then(serde_norway::Value::as_mapping_mut)
    {
        changed |= refs.remove(yaml_key(DEEPSEEK_HARNESS_CREDENTIAL)).is_some();
        remove_refs = refs.is_empty();
    }
    if remove_refs {
        root.remove(yaml_key("refs"));
    }
    Ok(changed)
}

fn deepseek_harness_credentials_document_is_empty(root: &serde_norway::Mapping) -> bool {
    root.iter().all(|(key, value)| match key.as_str() {
        Some("version") => deepseek_harness_credentials_version_is_supported(value),
        Some("refs" | "records") => value
            .as_mapping()
            .is_some_and(serde_norway::Mapping::is_empty),
        _ => false,
    })
}

fn deepseek_harness_original_credential(
    root: &serde_norway::Mapping,
) -> Option<&serde_norway::Value> {
    yaml_mapping_value(root, "refs")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|refs| yaml_mapping_value(refs, DEEPSEEK_HARNESS_CREDENTIAL))
}

pub(crate) fn write_deepseek_harness_file(
    path: &Path,
    content: &[u8],
    owner_only: bool,
) -> Result<(), String> {
    write_bytes_directly(path, content)?;
    #[cfg(unix)]
    if owner_only {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            format!(
                "Failed to set DeepSeek Harness credential permissions {}: {error}",
                path_to_string(path)
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = owner_only;
    Ok(())
}

pub(crate) fn write_agent_configuration_file(
    client: AgentClient,
    path: &Path,
    content: &[u8],
) -> Result<(), String> {
    if client == AgentClient::Codex
        && matches!(path.file_name().and_then(|name| name.to_str()), Some(CODEX_NATIVE_OAUTH_STATE_FILE | "auth.json"))
    {
        return write_codex_private_file(path, content);
    }
    if client == AgentClient::AntigravityCli {
        return write_codex_private_file(path, content);
    }
    if client == AgentClient::DeepSeekHarness {
        let owner_only = path.file_name().and_then(|name| name.to_str())
            == Some(DEEPSEEK_HARNESS_CREDENTIALS_FILE);
        write_deepseek_harness_file(path, content, owner_only)
    } else {
        write_bytes_directly(path, content)
    }
}

pub(crate) fn build_claude_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    models: &[AgentModelOption],
    mappings: Option<&ClaudeDesktopModelMappings>,
) -> Result<String, String> {
    let mut root = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| format!("Invalid Claude Code settings.json format: {error}"))?,
        None => serde_json::json!({}),
    };
    let root = root
        .as_object_mut()
        .ok_or_else(|| "Claude Code settings.json root must be an object".to_string())?;
    // Track only the compaction windows CPA owns so route changes and closing
    // the connection can remove them without deleting unrelated model settings.
    let previous_managed_models = root.get("env").and_then(serde_json::Value::as_object)
        .and_then(|env| env.get(CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default();
    let mappings = mappings
        .cloned()
        .unwrap_or_else(|| ClaudeDesktopModelMappings::all(model));
    let max_context_tokens = mappings.max_context_tokens;
    let model_settings = claude_code_model_settings(&mappings);
    let subagent_model = model_settings.haiku.clone();
    // Explicit/default session selections may switch roles, while Subagents may use
    // another model. A global Sonnet-derived effort override is wrong for both.
    let effort_level = if mappings.startup_model.is_none() && mappings.subagent_model.is_none() {
        claude_code_model_effort_level(models, &mappings.sonnet)?
    } else {
        None
    };
    let env = ensure_json_object_entry(root, "env");
    env.remove("ANTHROPIC_API_KEY");
    env.remove("CLAUDE_CODE_EFFORT_LEVEL");
    env.remove(CLAUDE_CODE_AUTO_MODE_SERVER_ENV);
    for (key, value) in [
        ("ANTHROPIC_BASE_URL", base_url),
        ("ANTHROPIC_AUTH_TOKEN", api_key),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            model_settings.haiku.as_str(),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            model_settings.sonnet.as_str(),
        ),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", model_settings.opus.as_str()),
        (
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            model_settings.fable.as_str(),
        ),
    ] {
        env.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    env.insert(
        "EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL".to_string(),
        serde_json::Value::String(if mappings.startup_model.as_ref().map_or(mappings.manage_default_model, |value| !value.trim().is_empty()) { "1" } else { "0" }.to_string()),
    );
    env.insert(
        "EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
        serde_json::Value::String(if mappings.subagent_model.as_ref().map_or(mappings.manage_subagent_model, |value| !value.trim().is_empty()) { "1" } else { "0" }.to_string()),
    );
    let explicit_startup_model = mappings.startup_model.as_ref().map(|model| model.trim().to_string());
    if mappings.startup_model.is_some() {
        // Claude Code's official persistent startup-model setting is the root
        // `model` key. Keep ANTHROPIC_MODEL out of the generated file so the
        // saved choice can still be changed from Claude Code's /model picker.
        env.remove("ANTHROPIC_MODEL");
    } else if mappings.manage_default_model {
        env.insert(
            "ANTHROPIC_MODEL".to_string(),
            serde_json::Value::String(model_settings.sonnet.clone()),
        );
    } else if env
        .get("ANTHROPIC_MODEL")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == model_settings.opus || value == model_settings.sonnet || value == model_settings.haiku)
    {
        env.remove("ANTHROPIC_MODEL");
    }
    if let Some(model) = &mappings.subagent_model {
        if model.trim().is_empty() {
            env.remove("CLAUDE_CODE_SUBAGENT_MODEL");
        } else {
            env.insert("CLAUDE_CODE_SUBAGENT_MODEL".into(), serde_json::Value::String(model.trim().into()));
        }
    } else if mappings.manage_subagent_model {
        env.entry("CLAUDE_CODE_SUBAGENT_MODEL".to_string())
            .or_insert_with(|| serde_json::Value::String(subagent_model));
    } else if env
        .get("CLAUDE_CODE_SUBAGENT_MODEL")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == model_settings.opus || value == model_settings.sonnet || value == model_settings.haiku)
    {
        env.remove("CLAUDE_CODE_SUBAGENT_MODEL");
    }
    env.insert(
        CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV.to_string(),
        serde_json::Value::String("1".to_string()),
    );
    env.insert(
        CLAUDE_CODE_AUTO_MODE_SERVER_ENV.to_string(),
        serde_json::Value::String("0".to_string()),
    );
    for (key, value) in [
        (CLAUDE_AUTOCOMPACT_PCT_OVERRIDE_ENV, mappings.auto_compact_pct.to_string()),
        (DISABLE_AUTO_COMPACT_ENV, if mappings.disable_auto_compact { "1" } else { "0" }.to_string()),
    ] {
        env.insert(key.to_string(), serde_json::Value::String(value));
    }
    for (key, value) in claude_code_model_presentation_environment(&mappings, models)? {
        env.insert(key, serde_json::Value::String(value));
    }
    if let Some(effort_level) = effort_level {
        env.insert(
            "CLAUDE_CODE_EFFORT_LEVEL".to_string(),
            serde_json::Value::String(effort_level),
        );
    }
    // Claude Code reads one context window from settings.json env and
    // compacts at the smaller of that window and the compact setting.
    let fable_model = if mappings.fable.trim().is_empty() { mappings.sonnet.as_str() } else { mappings.fable.as_str() };
    let mut model_ids = std::collections::BTreeSet::<String>::new();
    for model_id in [fable_model, mappings.opus.as_str(), mappings.sonnet.as_str(), mappings.haiku.as_str()] {
        let model_id = strip_claude_code_context_suffix(model_id).trim();
        if !model_id.is_empty() {
            model_ids.insert(model_id.to_string());
        }
    }
    for model in [&mappings.startup_model, &mappings.subagent_model] {
        let Some(model) = model.as_deref().map(str::trim).filter(|value| !value.is_empty()) else { continue; };
        let role = strip_claude_code_context_suffix(model).trim().to_ascii_lowercase();
        let resolved = match role.as_str() {
            "opus" => mappings.opus.as_str(),
            "sonnet" => mappings.sonnet.as_str(),
            "haiku" => mappings.haiku.as_str(),
            "fable" => fable_model,
            _ => model,
        };
        let model_id = strip_claude_code_context_suffix(resolved).trim();
        if !model_id.is_empty() {
            model_ids.insert(model_id.to_string());
        }
    }
    let env = ensure_json_object_entry(root, "env");
    let window_text = max_context_tokens.to_string();
    env.insert(CLAUDE_CODE_MAX_CONTEXT_TOKENS_ENV.to_string(), serde_json::Value::String(window_text.clone()));
    env.insert(CLAUDE_CODE_AUTO_COMPACT_WINDOW_ENV.to_string(), serde_json::Value::String(window_text));
    root.insert("autoCompactWindow".into(), serde_json::Value::Number(max_context_tokens.into()));
    root.insert("autoCompactEnabled".into(), serde_json::Value::Bool(!mappings.disable_auto_compact));
    let managed_model_ids = model_ids.iter().cloned().collect::<Vec<_>>();
    let model_settings_json = ensure_json_object_entry(root, "modelSettings");
    for model in previous_managed_models {
        if model_ids.contains(&model) { continue; }
        if let Some(entry) = model_settings_json.get_mut(&model).and_then(serde_json::Value::as_object_mut) {
            entry.remove("autoCompactWindow");
            if entry.is_empty() { model_settings_json.remove(&model); }
        }
    }
    for model_id in model_ids {
        let entry = ensure_json_object_entry(model_settings_json, &model_id);
        entry.insert("autoCompactWindow".into(), serde_json::Value::Number(max_context_tokens.into()));
    }
    if let Some(model) = explicit_startup_model {
        if model.is_empty() {
            root.remove("model");
        } else {
            root.insert("model".into(), serde_json::Value::String(model));
        }
    }
    if mappings.startup_model.is_none() && mappings.manage_default_model {
        root.insert(
            "model".to_string(),
            serde_json::Value::String(model_settings.sonnet),
        );
    } else if mappings.startup_model.is_none() && root
        .get("model")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == model_settings.opus || value == model_settings.sonnet || value == model_settings.haiku)
    {
        root.remove("model");
    }
    // The saved selection identifies a CPA default separately from a later
    // user change made through Claude Code's own /model picker.
    let managed_startup = root.get("model").and_then(serde_json::Value::as_str)
        .filter(|_| mappings.startup_model.as_ref().map_or(mappings.manage_default_model, |model| !model.trim().is_empty()))
        .map(str::to_string);
    let env = ensure_json_object_entry(root, "env");
    env.insert(CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV.into(), serde_json::Value::String(
        serde_json::to_string(&managed_model_ids).map_err(|_| "Failed to record managed Claude Code models".to_string())?));
    if let Some(model) = managed_startup {
        env.insert(CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV.into(), serde_json::Value::String(model));
    } else {
        env.remove(CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV);
    }
    let mut rendered = serde_json::to_string_pretty(&serde_json::Value::Object(root.clone()))
        .map_err(|error| format!("Failed to generate Claude Code configuration: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn parse_agent_json_object(
    existing: Option<&str>,
    label: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let value = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| format!("Invalid {label} format: {error}"))?,
        None => serde_json::json!({}),
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} root must be an object"))
}

pub(crate) fn parse_agent_json5_object(
    existing: Option<&str>,
    label: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let value = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => json5::from_str::<serde_json::Value>(value)
            .map_err(|error| format!("Invalid {label} format: {error}"))?,
        None => serde_json::json!({}),
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} root must be an object"))
}

pub(crate) fn ensure_json_object_entry<'a>(
    root: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    if !root.get(key).is_some_and(serde_json::Value::is_object) {
        root.insert(key.to_string(), serde_json::json!({}));
    }
    root.get_mut(key)
        .and_then(serde_json::Value::as_object_mut)
        .expect("object entry was just normalized")
}

pub(crate) fn ensure_json_array_entry<'a>(
    root: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a mut Vec<serde_json::Value> {
    if !root.get(key).is_some_and(serde_json::Value::is_array) {
        root.insert(key.to_string(), serde_json::json!([]));
    }
    root.get_mut(key)
        .and_then(serde_json::Value::as_array_mut)
        .expect("array entry was just normalized")
}

pub(crate) fn render_agent_json(
    root: serde_json::Map<String, serde_json::Value>,
    label: &str,
) -> Result<String, String> {
    let mut rendered = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map_err(|error| format!("Failed to generate {label}: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn ordered_agent_models(
    models: &[AgentModelOption],
    selected_model: &str,
) -> Vec<AgentModelOption> {
    let mut ordered = Vec::with_capacity(models.len().max(1));
    if let Some(selected) = models
        .iter()
        .find(|model| model.name.eq_ignore_ascii_case(selected_model))
    {
        ordered.push(selected.clone());
    } else {
        ordered.push(AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: selected_model.to_string(),
            alias: None,
            is_alias: false,
            context_window: None,
        });
    }
    for model in models {
        if ordered
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&model.name))
        {
            continue;
        }
        ordered.push(model.clone());
    }
    ordered
}

pub(crate) fn build_claude_desktop_deployment_config(
    existing: Option<&str>,
) -> Result<String, String> {
    let mut root = parse_agent_json_object(existing, "Claude Desktop configuration")?;
    root.insert("deploymentMode".to_string(), serde_json::json!("3p"));
    render_agent_json(root, "Claude Desktop configuration")
}

pub(crate) fn build_claude_desktop_profile(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    models: &[AgentModelOption],
    mappings: Option<&ClaudeDesktopModelMappings>,
) -> Result<String, String> {
    let mut root = parse_agent_json_object(existing, "Claude Desktop gateway configuration")?;
    root.insert(
        "disableDeploymentModeChooser".to_string(),
        serde_json::json!(true),
    );
    root.insert(
        "inferenceGatewayApiKey".to_string(),
        serde_json::json!(api_key),
    );
    root.insert(
        "inferenceGatewayAuthScheme".to_string(),
        serde_json::json!("bearer"),
    );
    root.insert(
        "inferenceGatewayBaseUrl".to_string(),
        serde_json::json!(base_url),
    );
    root.insert(
        "inferenceProvider".to_string(),
        serde_json::json!("gateway"),
    );
    let mappings = mappings
        .cloned()
        .unwrap_or_else(|| ClaudeDesktopModelMappings::all(model));
    let inference_models = if let Some(entries) = &mappings.desktop_models {
        validate_claude_desktop_entries(entries)?;
        let mut families = HashSet::new();
        entries
            .iter()
            .map(|entry| {
                let mut value = claude_desktop_inference_model(
                    entry.model_id(),
                    entry.source_or_alias(),
                    entry.context_1m,
                    models,
                );
                value["name"] = serde_json::json!(entry.model_id());
                value["labelOverride"] = serde_json::json!(entry.source_or_alias());
                if let Some(tier) = claude_desktop_family_tier(entry.model_id()) {
                    value["isFamilyDefault"] = serde_json::json!(families.insert(tier));
                }
                value
            })
            .collect()
    } else {
        vec![
            claude_desktop_inference_model(
                CLAUDE_DESKTOP_OPUS_MODEL_ID,
                &mappings.opus,
                mappings.opus_1m,
                models,
            ),
            claude_desktop_inference_model(
                CLAUDE_DESKTOP_SONNET_MODEL_ID,
                &mappings.sonnet,
                mappings.sonnet_1m,
                models,
            ),
            claude_desktop_inference_model(
                CLAUDE_DESKTOP_HAIKU_MODEL_ID,
                &mappings.haiku,
                mappings.haiku_1m,
                models,
            ),
        ]
    };
    let mut deduplicated_models = Vec::<serde_json::Value>::with_capacity(inference_models.len());
    for entry in inference_models {
        let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let existing = deduplicated_models.iter_mut().find(|existing| {
            existing
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|existing_name| existing_name.eq_ignore_ascii_case(name))
        });
        if let Some(existing) = existing {
            if entry.get("supports1m").and_then(serde_json::Value::as_bool) == Some(true) {
                *existing = entry;
            }
        } else {
            deduplicated_models.push(entry);
        }
    }
    root.insert(
        "inferenceModels".to_string(),
        serde_json::Value::Array(deduplicated_models),
    );
    render_agent_json(root, "Claude Desktop gateway configuration")
}

pub(crate) fn claude_desktop_inference_model(
    route_model: &str,
    source_model: &str,
    enable_1m: bool,
    models: &[AgentModelOption],
) -> serde_json::Value {
    let use_source = claude_desktop_uses_source_directly(source_model, models);
    let mut entry = serde_json::Map::new();
    entry.insert(
        "name".to_string(),
        serde_json::json!(if use_source {
            source_model
        } else {
            route_model
        }),
    );
    if !use_source {
        entry.insert("labelOverride".to_string(), serde_json::json!(source_model));
    }
    if let Some(tier) = claude_desktop_family_tier(route_model) {
        entry.insert("anthropicFamilyTier".to_string(), serde_json::json!(tier));
        entry.insert("isFamilyDefault".to_string(), serde_json::json!(true));
    }
    if enable_1m {
        entry.insert("supports1m".to_string(), serde_json::json!(true));
        entry.insert("prefer1m".to_string(), serde_json::json!(true));
    }
    serde_json::Value::Object(entry)
}

pub(crate) fn claude_desktop_family_tier(route_model: &str) -> Option<&'static str> {
    let lowered = route_model.to_ascii_lowercase();
    if lowered.starts_with("claude-opus-") || lowered.contains("-opus-")
        || route_model.eq_ignore_ascii_case(CLAUDE_DESKTOP_OPUS_MODEL_ID)
        || route_model.eq_ignore_ascii_case(LEGACY_CLAUDE_DESKTOP_MODEL_IDS[0])
    {
        Some("opus")
    } else if lowered.starts_with("claude-sonnet-") || lowered.contains("-sonnet-")
        || route_model.eq_ignore_ascii_case(CLAUDE_DESKTOP_SONNET_MODEL_ID)
        || route_model.eq_ignore_ascii_case(LEGACY_CLAUDE_DESKTOP_MODEL_IDS[1])
    {
        Some("sonnet")
    } else if lowered.starts_with("claude-haiku-") || lowered.contains("-haiku-")
        || route_model.eq_ignore_ascii_case(CLAUDE_DESKTOP_HAIKU_MODEL_ID)
        || route_model.eq_ignore_ascii_case(LEGACY_CLAUDE_DESKTOP_MODEL_IDS[2])
    {
        Some("haiku")
    } else {
        None
    }
}

pub(crate) fn build_claude_desktop_meta(existing: Option<&str>) -> Result<String, String> {
    let mut root = parse_agent_json_object(existing, "Claude Desktop configuration index")?;
    repair_claude_desktop_meta_names(&mut root);
    let entries = ensure_json_array_entry(&mut root, "entries");
    let mut managed_entry = None;
    let mut retained_entries = Vec::with_capacity(entries.len());
    for entry in std::mem::take(entries) {
        match entry.get("id").and_then(serde_json::Value::as_str) {
            Some(id) if id == CLAUDE_DESKTOP_PROFILE_ID => {
                if managed_entry.is_none() {
                    managed_entry = entry.as_object().cloned();
                }
            }
            Some(_) => retained_entries.push(entry),
            None => {}
        }
    }
    let mut managed_entry = managed_entry.unwrap_or_default();
    managed_entry.insert(
        "id".to_string(),
        serde_json::json!(CLAUDE_DESKTOP_PROFILE_ID),
    );
    managed_entry.insert("name".to_string(), serde_json::json!("EasyCLIProxyAPI"));
    retained_entries.push(serde_json::Value::Object(managed_entry));
    *entries = retained_entries;
    root.insert(
        "appliedId".to_string(),
        serde_json::json!(CLAUDE_DESKTOP_PROFILE_ID),
    );
    render_agent_json(root, "Claude Desktop configuration index")
}

pub(crate) fn repair_claude_desktop_meta_names(root: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(entries) = root.get_mut("entries").and_then(serde_json::Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if entry.get("name").and_then(serde_json::Value::as_str).is_none() {
            let name = format!("Configuration {id}");
            entry["name"] = serde_json::json!(name);
        }
    }
}

pub(crate) fn clear_agent_managed_configuration(
    client: AgentClient,
    home: &Path,
    port: u16,
) -> Result<AgentConfigActionResult, String> {
    if client == AgentClient::Codex {
        return close_codex_configuration(home);
    }
    let paths = config_paths(client.id(), home)?;
    let before = config_images(&paths)?;
    validate_client_config_images(client.id(), &before)?;
    let after = if let Some(after) = prepare_recorded_integration_restore(client, &paths, &before)? {
        after
    } else {
        let mut after = before.clone();
        for (path, bytes) in prepare_agent_managed_removal(client, &paths, port)? {
            let entry = after
                .iter_mut()
                .find(|(candidate, _)| candidate == &path)
                .ok_or("Configuration cleanup path mismatch")?;
            entry.1 = bytes;
        }
        after
    };
    let mut result = commit_config(
        client.id(),
        &paths,
        &before,
        &after,
        "clear-integration",
        None,
    )?;
    result.enabled = false;
    Ok(result)
}

#[cfg(test)]
pub(crate) fn remove_agent_managed_configuration(
    client: AgentClient,
    paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    let updates = prepare_agent_managed_removal(client, paths, GuiConfigFile::default().port)?;
    write_config_images(client.id(), &updates)?;
    Ok(updates
        .iter()
        .map(|(path, _)| path_to_string(path))
        .collect())
}

pub(crate) fn prepare_agent_json_removal<F>(
    path: &Path,
    label: &str,
    update: F,
) -> Result<Option<FileSnapshot>, String>
where
    F: FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> bool,
{
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path_to_string(path)))?;
    let mut root = parse_agent_json_object(Some(&content), label)?;
    if !update(&mut root) {
        return Ok(None);
    }
    let bytes = if root.is_empty() {
        None
    } else {
        Some(render_agent_json(root, label)?.into_bytes())
    };
    Ok(Some((path.to_path_buf(), bytes)))
}

pub(crate) fn prepare_claude_desktop_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    if paths.len() != 4 {
        return Err("Claude Desktop configuration path is unavailable on the current platform".to_string());
    }
    let meta = read_agent_json_or_empty(&paths[3], "Claude Desktop configuration index")?;
    let applied_id = meta.get("appliedId").and_then(serde_json::Value::as_str);
    let managed_active = applied_id == Some(CLAUDE_DESKTOP_PROFILE_ID)
        || (applied_id.is_none() && agent_has_managed_marker(AgentClient::ClaudeDesktop, paths)?);
    let mut changed = Vec::new();
    for (path, label) in [
        (&paths[0], "Claude Desktop main configuration"),
        (&paths[1], "Claude Desktop 3P configuration"),
    ] {
        if let Some(update) = prepare_agent_json_removal(path, label, |root| {
            if managed_active
                && root
                    .get("deploymentMode")
                    .and_then(serde_json::Value::as_str)
                    == Some("3p")
            {
                root.remove("deploymentMode");
                true
            } else {
                false
            }
        })? {
            changed.push(update);
        }
    }

    if let Some(update) =
        prepare_agent_json_removal(&paths[2], "Claude Desktop gateway configuration", |root| {
            let mut updated = false;
            for key in [
                "disableDeploymentModeChooser",
                "inferenceGatewayApiKey",
                "inferenceGatewayAuthScheme",
                "inferenceGatewayBaseUrl",
                "inferenceProvider",
                "inferenceModels",
            ] {
                updated |= root.remove(key).is_some();
            }
            updated
        })?
    {
        changed.push(update);
    }

    if let Some(update) =
        prepare_agent_json_removal(&paths[3], "Claude Desktop configuration index", |root| {
            let mut updated = false;
            if root.get("appliedId").and_then(serde_json::Value::as_str)
                == Some(CLAUDE_DESKTOP_PROFILE_ID)
            {
                root.remove("appliedId");
                updated = true;
            }
            let entries_empty = if let Some(entries) = root
                .get_mut("entries")
                .and_then(serde_json::Value::as_array_mut)
            {
                let previous_len = entries.len();
                entries.retain(|entry| {
                    entry.get("id").and_then(serde_json::Value::as_str)
                        != Some(CLAUDE_DESKTOP_PROFILE_ID)
                });
                updated |= entries.len() != previous_len;
                entries.is_empty()
            } else {
                false
            };
            if entries_empty {
                root.remove("entries");
            }
            updated
        })?
    {
        changed.push(update);
    }
    Ok(changed)
}

pub(crate) fn prepare_claude_code_managed_removal(
    paths: &[PathBuf],
    expected_base_url: &str,
) -> Result<Images, String> {
    let Some(path) = paths.first() else {
        return Err("Claude Code configuration path is unavailable on the current platform".to_string());
    };
    let expected_base_url = reqwest::Url::parse(expected_base_url)
        .map_err(|_| "Invalid CPA access URL".to_string())?;
    let updated = prepare_agent_json_removal(path, "Claude Code configuration", |root| {
        let managed_model = root
            .get("env")
            .and_then(serde_json::Value::as_object)
            .and_then(|env| env.get(CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV).or_else(|| env.get("ANTHROPIC_MODEL")))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let managed = root
            .get("env")
            .and_then(serde_json::Value::as_object)
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|url| {
                reqwest::Url::parse(url.trim()).is_ok_and(|url| url == expected_base_url)
            });
        if !managed {
            return false;
        }
        let has_managed_model_record = root.get("env").and_then(serde_json::Value::as_object)
            .is_some_and(|env| env.contains_key(CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV));
        let managed_role_selection = !has_managed_model_record && root.get("model").and_then(serde_json::Value::as_str)
            .is_some_and(|model| {
                let base = strip_claude_code_context_suffix(model);
                matches!(base.to_ascii_lowercase().as_str(), "opus" | "sonnet" | "haiku" | "fable")
                    || root.get("env").and_then(serde_json::Value::as_object).is_some_and(|env| {
                        ["ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL",
                            "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_FABLE_MODEL"]
                            .iter().any(|key| env.get(*key).and_then(serde_json::Value::as_str)
                                .is_some_and(|route| strip_claude_code_context_suffix(route) == base))
                    })
            });
        restore_claude_code_compaction_settings(root, None);
        if managed_role_selection || root.get("model").and_then(serde_json::Value::as_str) == managed_model.as_deref() {
            root.remove("model");
        }
        let env_empty = if let Some(env) = root
            .get_mut("env")
            .and_then(serde_json::Value::as_object_mut)
        {
            for key in [
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_MODEL",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                "ANTHROPIC_DEFAULT_FABLE_MODEL",
                "EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL",
                "EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL",
                CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV,
                CLAUDE_CODE_MAX_CONTEXT_TOKENS_ENV,
                CLAUDE_CODE_AUTO_COMPACT_WINDOW_ENV,
                CLAUDE_CODE_AUTO_MODE_SERVER_ENV,
                CLAUDE_AUTOCOMPACT_PCT_OVERRIDE_ENV,
                DISABLE_AUTO_COMPACT_ENV,
                CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV,
                CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV,
                "CLAUDE_CODE_SUBAGENT_MODEL",
                "CLAUDE_CODE_EFFORT_LEVEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
                "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
                "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
                "ANTHROPIC_CUSTOM_MODEL_OPTION",
                "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
                "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
            ] {
                env.remove(key);
            }
            env.is_empty()
        } else {
            false
        };
        if env_empty {
            root.remove("env");
        }
        true
    })?;
    Ok(updated.into_iter().collect())
}

pub(crate) fn prepare_codex_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    use toml_edit::{Document, Item};

    let Some(path) = paths.first() else {
        return Err("Codex configuration path is unavailable on the current platform".to_string());
    };
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read Codex configuration {}: {error}", path_to_string(path)))?;
    let mut document = content
        .parse::<Document>()
        .map_err(|error| format!("Failed to parse Codex configuration: {error}"))?;
    let managed_selected =
        document.get("model_provider").and_then(Item::as_str) == Some(MANAGED_AGENT_PROVIDER_ID);
    let managed_catalog =
        document.get("model_catalog_json").and_then(Item::as_str) == Some(CODEX_MODEL_CATALOG_FILE);
    let managed_provider = document
        .get("model_providers")
        .and_then(Item::as_table)
        .is_some_and(|providers| providers.contains_key(MANAGED_AGENT_PROVIDER_ID));
    if !managed_selected && !managed_catalog && !managed_provider {
        return Ok(Vec::new());
    }
    if managed_selected {
        document.remove("model_provider");
        document.remove("model");
    }
    if managed_catalog {
        document.remove("model_catalog_json");
    }
    let rendered = document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate restored Codex configuration: {error}"))?;
    let mut changed = vec![(
        path.clone(),
        (!rendered.trim().is_empty()).then(|| rendered.into_bytes()),
    )];
    if managed_catalog {
        changed.push((path.with_file_name(CODEX_MODEL_CATALOG_FILE), None));
    }
    Ok(changed)
}

pub(crate) fn prepare_opencode_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    let Some(path) = paths.first() else {
        return Err("OpenCode configuration path is unavailable on the current platform".to_string());
    };
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let updated = prepare_agent_json5_removal(path, "OpenCode configuration", |root| {
        let mut changed = false;
        if root
            .get("model")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|model| model.starts_with(&prefix))
        {
            root.remove("model");
            changed = true;
        }
        let providers_empty = if let Some(providers) = root
            .get_mut("provider")
            .and_then(serde_json::Value::as_object_mut)
        {
            changed |= providers.remove(MANAGED_AGENT_PROVIDER_ID).is_some();
            providers.is_empty()
        } else {
            false
        };
        if providers_empty {
            root.remove("provider");
        }
        changed
    })?;
    Ok(updated.into_iter().collect())
}

pub(crate) fn prepare_zcode_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    let path = paths.first().ok_or("ZCode provider configuration path is unavailable")?;
    let updated = prepare_agent_json_removal(path, "ZCode provider configuration", |root| {
        let Some(config) = root.get_mut("config").and_then(serde_json::Value::as_object_mut) else {
            return false;
        };
        let mut changed = false;
        if config.get("defaultModelSelection")
            .and_then(|selection| selection.get("providerId"))
            .and_then(serde_json::Value::as_str) == Some(MANAGED_AGENT_PROVIDER_ID)
        {
            config.remove("defaultModelSelection");
            changed = true;
        }
        if let Some(order) = config.get_mut("providerOrder").and_then(serde_json::Value::as_array_mut) {
            let old_len = order.len();
            order.retain(|item| item.as_str() != Some(MANAGED_AGENT_PROVIDER_ID));
            changed |= order.len() != old_len;
        }
        if let Some(rules) = config.get_mut("providerConfigRules")
            .and_then(|value| value.get_mut("providerRules"))
            .and_then(serde_json::Value::as_array_mut)
        {
            let old_len = rules.len();
            rules.retain(|rule| rule.get("providerId").and_then(serde_json::Value::as_str)
                != Some(MANAGED_AGENT_PROVIDER_ID));
            changed |= rules.len() != old_len;
        }
        for key in ["providerModelRules", "manualProviderModelRules"] {
            if let Some(rules) = config.get_mut("modelConfigRules")
                .and_then(|value| value.get_mut(key))
                .and_then(serde_json::Value::as_array_mut)
            {
                let old_len = rules.len();
                rules.retain(|rule| rule.get("providerId").and_then(serde_json::Value::as_str)
                    != Some(MANAGED_AGENT_PROVIDER_ID));
                changed |= rules.len() != old_len;
            }
        }
        changed
    })?;
    Ok(updated.into_iter().collect())
}

pub(crate) fn prepare_kimi_code_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    prepare_managed_toml_client_removal(
        paths,
        "Kimi Code",
        Some("providers"),
        "models",
        None,
        "default_model",
    )
}

pub(crate) fn prepare_grok_build_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    prepare_managed_toml_client_removal(
        paths,
        "Grok Build",
        None,
        "model",
        Some("models"),
        "default",
    )
}

pub(crate) fn prepare_managed_toml_client_removal(
    paths: &[PathBuf],
    label: &str,
    provider_section: Option<&str>,
    model_entry_section: &str,
    default_section: Option<&str>,
    default_key: &str,
) -> Result<Images, String> {
    use toml_edit::{Document, Item};

    let Some(path) = paths.first() else {
        return Err(format!("{label} configuration path is unavailable on the current platform"));
    };
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {label} configuration {}: {error}", path_to_string(path)))?;
    let mut document = content
        .parse::<Document>()
        .map_err(|error| format!("Failed to parse {label} configuration: {error}"))?;
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let mut changed = false;

    let default_table: Option<&mut dyn toml_edit::TableLike> =
        if let Some(section) = default_section {
            document
                .as_table_mut()
                .get_mut(section)
                .and_then(Item::as_table_like_mut)
        } else {
            Some(document.as_table_mut())
        };
    if let Some(table) = default_table {
        if table
            .get(default_key)
            .and_then(Item::as_str)
            .is_some_and(|model| model.starts_with(&prefix))
        {
            table.remove(default_key);
            changed = true;
        }
    }

    if let Some(section) = provider_section {
        if let Some(providers) = document
            .as_table_mut()
            .get_mut(section)
            .and_then(Item::as_table_like_mut)
        {
            changed |= providers.remove(MANAGED_AGENT_PROVIDER_ID).is_some();
        }
    }
    if let Some(models) = document
        .as_table_mut()
        .get_mut(model_entry_section)
        .and_then(Item::as_table_like_mut)
    {
        let managed = models
            .iter()
            .map(|(key, _)| key.to_string())
            .filter(|key| key.starts_with(&prefix))
            .collect::<Vec<_>>();
        changed |= !managed.is_empty();
        for key in managed {
            models.remove(&key);
        }
    }
    for section in provider_section.into_iter().chain([model_entry_section]) {
        let empty = document
            .as_table()
            .get(section)
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.is_empty());
        if empty {
            document.as_table_mut().remove(section);
        }
    }
    if let Some(section) = default_section {
        let empty = document
            .as_table()
            .get(section)
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.is_empty());
        if empty {
            document.as_table_mut().remove(section);
        }
    }
    if !changed {
        return Ok(Vec::new());
    }
    let rendered = document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate restored {label} configuration: {error}"))?;
    Ok(vec![(
        path.clone(),
        (!rendered.trim().is_empty()).then(|| rendered.into_bytes()),
    )])
}

pub(crate) fn prepare_agent_json5_removal<F>(
    path: &Path,
    label: &str,
    update: F,
) -> Result<Option<FileSnapshot>, String>
where
    F: FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> bool,
{
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path_to_string(path)))?;
    let value = json5::from_str::<serde_json::Value>(&content)
        .map_err(|error| format!("Failed to parse {label}: {error}"))?;
    let mut root = value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} root must be an object"))?;
    if !update(&mut root) {
        return Ok(None);
    }
    let bytes = if root.is_empty() {
        None
    } else {
        let rendered = render_agent_json(root, label)?;
        let comments = extract_json5_comments(&content);
        Some(
            if comments.is_empty() {
                rendered
            } else {
                format!("{}\n{rendered}", comments.join("\n"))
            }
            .into_bytes(),
        )
    };
    Ok(Some((path.to_path_buf(), bytes)))
}

pub(crate) fn prepare_openclaw_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    let Some(path) = paths.first() else {
        return Err("OpenClaw configuration path is unavailable on the current platform".to_string());
    };
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let updated = prepare_agent_json5_removal(path, "OpenClaw configuration", |root| {
        let mut changed = false;
        if let Some(models) = root
            .get_mut("models")
            .and_then(serde_json::Value::as_object_mut)
        {
            let providers_empty = if let Some(providers) = models
                .get_mut("providers")
                .and_then(serde_json::Value::as_object_mut)
            {
                changed |= providers.remove(MANAGED_AGENT_PROVIDER_ID).is_some();
                providers.is_empty()
            } else {
                false
            };
            if providers_empty {
                models.remove("providers");
            }
        }
        if let Some(defaults) = root
            .get_mut("agents")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|agents| agents.get_mut("defaults"))
            .and_then(serde_json::Value::as_object_mut)
        {
            let model_empty = if let Some(model) = defaults
                .get_mut("model")
                .and_then(serde_json::Value::as_object_mut)
            {
                if model
                    .get("primary")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|model| model.starts_with(&prefix))
                {
                    model.remove("primary");
                    changed = true;
                }
                model.is_empty()
            } else {
                false
            };
            if model_empty {
                defaults.remove("model");
            }
            let catalog_empty = if let Some(catalog) = defaults
                .get_mut("models")
                .and_then(serde_json::Value::as_object_mut)
            {
                let previous_len = catalog.len();
                catalog.retain(|name, _| !name.starts_with(&prefix));
                changed |= catalog.len() != previous_len;
                catalog.is_empty()
            } else {
                false
            };
            if catalog_empty {
                defaults.remove("models");
            }
        }
        changed
    })?;
    Ok(updated.into_iter().collect())
}

pub(crate) fn prepare_hermes_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    let Some(path) = paths.first() else {
        return Err("Hermes configuration path is unavailable on the current platform".to_string());
    };
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read Hermes configuration {}: {error}", path_to_string(path)))?;
    let mut document = yaml_serde_edit::YamlValue::parse(&content)
        .map_err(|error| format!("Failed to parse Hermes configuration: {error}"))?;
    let mut updated = document.get().clone();
    let root = updated
        .as_mapping_mut()
        .ok_or_else(|| "Hermes configuration root must be a mapping".to_string())?;
    let mut changed = false;
    let providers_empty = if let Some(providers) = root
        .get_mut(yaml_key("custom_providers"))
        .and_then(serde_norway::Value::as_sequence_mut)
    {
        let previous_len = providers.len();
        providers.retain(|provider| {
            provider.get("name").and_then(serde_norway::Value::as_str)
                != Some(MANAGED_AGENT_PROVIDER_ID)
        });
        changed |= providers.len() != previous_len;
        providers.is_empty()
    } else {
        false
    };
    if providers_empty {
        root.remove(yaml_key("custom_providers"));
    }
    let model_empty = if let Some(model) = root
        .get_mut(yaml_key("model"))
        .and_then(serde_norway::Value::as_mapping_mut)
    {
        if model
            .get(yaml_key("provider"))
            .and_then(serde_norway::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID)
        {
            model.remove(yaml_key("provider"));
            model.remove(yaml_key("default"));
            changed = true;
        }
        model.is_empty()
    } else {
        false
    };
    if model_empty {
        root.remove(yaml_key("model"));
    }
    if !changed {
        return Ok(Vec::new());
    }
    let bytes = if root.is_empty() {
        None
    } else {
        Some(render_updated_core_yaml(&mut document, updated)?.into_bytes())
    };
    Ok(vec![(path.clone(), bytes)])
}

pub(crate) fn prepare_agent_managed_removal(
    client: AgentClient,
    paths: &[PathBuf],
    port: u16,
) -> Result<Images, String> {
    match client {
        AgentClient::ClaudeCode => {
            prepare_claude_code_managed_removal(paths, &managed_core_loopback_origin(port))
        }
        AgentClient::ClaudeDesktop => prepare_claude_desktop_managed_removal(paths),
        AgentClient::Codex => prepare_codex_managed_removal(paths),
        AgentClient::OpenCode => prepare_opencode_managed_removal(paths),
        AgentClient::OpenClaw => prepare_openclaw_managed_removal(paths),
        AgentClient::Hermes => prepare_hermes_managed_removal(paths),
        AgentClient::DeepSeekHarness => prepare_deepseek_harness_managed_removal(paths),
        AgentClient::ZCode => prepare_zcode_managed_removal(paths),
        AgentClient::WorkBuddy => prepare_workbuddy_managed_removal(paths),
        AgentClient::AntigravityCli => prepare_antigravity_removal(client, paths),
        AgentClient::KimiCode => prepare_kimi_code_managed_removal(paths),
        AgentClient::GrokBuild => prepare_grok_build_managed_removal(paths),
        AgentClient::Omp => prepare_omp_managed_removal(paths),
    }
}

pub(crate) fn remove_deepseek_harness_settings_fields(root: &mut serde_norway::Mapping) -> bool {
    let mut changed = false;
    let mut remove_llm = false;
    if let Some(llm) = root
        .get_mut(yaml_key("llm-pi-ai"))
        .and_then(serde_norway::Value::as_mapping_mut)
    {
        let mut remove_providers = false;
        if let Some(providers) = llm
            .get_mut(yaml_key("providers"))
            .and_then(serde_norway::Value::as_mapping_mut)
        {
            changed |= providers
                .remove(yaml_key(DEEPSEEK_HARNESS_PROVIDER_ID))
                .is_some();
            remove_providers = providers.is_empty();
        }
        if remove_providers {
            llm.remove(yaml_key("providers"));
        }
        remove_llm = llm.is_empty();
    }
    if remove_llm {
        root.remove(yaml_key("llm-pi-ai"));
    }
    let managed_selection = root
        .get(yaml_key("agent-default-model"))
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|selection| yaml_mapping_value(selection, "provider"))
        .and_then(serde_norway::Value::as_str)
        == Some(DEEPSEEK_HARNESS_PROVIDER_ID);
    if managed_selection {
        root.remove(yaml_key("agent-default-model"));
        changed = true;
    }
    changed
}

pub(crate) fn prepare_deepseek_harness_managed_removal(
    paths: &[PathBuf],
) -> Result<Images, String> {
    if paths.len() != 2 {
        return Err("Invalid number of DeepSeek Harness configuration paths".to_string());
    }
    let mut changed_paths = Vec::new();
    if paths[0].is_file() {
        let current = fs::read_to_string(&paths[0])
            .map_err(|error| format!("Failed to read DeepSeek Harness settings: {error}"))?;
        let mut changed = false;
        let rendered = render_agent_yaml_mapping_update(
            Some(&current),
            "DeepSeek Harness settings",
            |root| {
                changed = remove_deepseek_harness_settings_fields(root);
                Ok(())
            },
        )?;
        if changed {
            let root = parse_agent_yaml_mapping(Some(&rendered), "DeepSeek Harness settings")?;
            changed_paths.push((
                paths[0].clone(),
                (!root.is_empty()).then(|| rendered.into_bytes()),
            ));
        }
    }
    if paths[1].is_file() {
        let current = fs::read_to_string(&paths[1])
            .map_err(|error| format!("Failed to read DeepSeek Harness credentials: {error}"))?;
        let mut changed = false;
        let rendered = render_agent_yaml_mapping_update(
            Some(&current),
            "DeepSeek Harness credentials",
            |root| {
                changed = remove_deepseek_harness_managed_credential(
                    root,
                    "DeepSeek Harness credentials",
                )?;
                Ok(())
            },
        )?;
        if changed {
            let root = parse_agent_yaml_mapping(Some(&rendered), "DeepSeek Harness credentials")?;
            changed_paths.push((
                paths[1].clone(),
                (!deepseek_harness_credentials_document_is_empty(&root))
                    .then(|| rendered.into_bytes()),
            ));
        }
    }
    Ok(changed_paths)
}

pub(crate) fn restore_json_key(
    current: &mut serde_json::Map<String, serde_json::Value>,
    original: Option<&serde_json::Map<String, serde_json::Value>>,
    key: &str,
) {
    if let Some(value) = original.and_then(|root| root.get(key)).cloned() {
        current.insert(key.to_string(), value);
    } else {
        current.remove(key);
    }
}

pub(crate) fn parse_restored_json_object(
    content: Option<&str>,
    label: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    content
        .map(|content| parse_agent_json_object(Some(content), label))
        .transpose()
}

pub(crate) fn parse_restored_json5_object(
    content: Option<&str>,
    label: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    content
        .map(|content| parse_agent_json5_object(Some(content), label))
        .transpose()
}

pub(crate) fn render_restored_json(
    root: serde_json::Map<String, serde_json::Value>,
    original_existed: bool,
    label: &str,
) -> Result<Option<String>, String> {
    if root.is_empty() && !original_existed {
        Ok(None)
    } else {
        render_agent_json(root, label).map(Some)
    }
}

fn restore_claude_code_compaction_settings(
    root: &mut serde_json::Map<String, serde_json::Value>,
    original: Option<&serde_json::Map<String, serde_json::Value>>,
) {
    let env = root.get("env").and_then(serde_json::Value::as_object);
    let mut managed_models = [
        "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_MODEL", "CLAUDE_CODE_SUBAGENT_MODEL",
    ].iter().filter_map(|key| env?.get(*key)?.as_str())
        .chain(root.get("model").and_then(serde_json::Value::as_str))
        .map(|model| strip_claude_code_context_suffix(model).to_string())
        .collect::<Vec<_>>();
    if let Some(recorded) = env.and_then(|env| env.get(CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok()) {
        managed_models = recorded;
    }
    managed_models.sort();
    managed_models.dedup();
    for key in ["autoCompactWindow", "autoCompactEnabled"] {
        restore_json_key(root, original, key);
    }
    let original_models = original.and_then(|root| root.get("modelSettings"));
    if original_models.is_some_and(|value| !value.is_object()) {
        restore_json_key(root, original, "modelSettings");
        return;
    }
    let original_models = original_models.and_then(serde_json::Value::as_object);
    if let Some(models) = root.get_mut("modelSettings").and_then(serde_json::Value::as_object_mut) {
        for model in managed_models {
            let original_entry = original_models.and_then(|models| models.get(&model));
            if let Some(entry) = models.get_mut(&model).and_then(serde_json::Value::as_object_mut) {
                restore_json_key(entry, original_entry.and_then(serde_json::Value::as_object), "autoCompactWindow");
                if entry.is_empty() && original_entry.is_none() {
                    models.remove(&model);
                }
            }
        }
        if models.is_empty() && original_models.is_none() {
            root.remove("modelSettings");
        }
    }
}

pub(crate) fn build_restored_claude_code_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let mut root = parse_agent_json_object(Some(current), "Current Claude Code configuration")?;
    let original_root = parse_restored_json_object(original, "Original Claude Code configuration")?;
    restore_claude_code_compaction_settings(&mut root, original_root.as_ref());
    restore_json_key(&mut root, original_root.as_ref(), "model");
    let original_env = original_root
        .as_ref()
        .and_then(|root| root.get("env"))
        .and_then(serde_json::Value::as_object);
    if root.get("env").is_some_and(serde_json::Value::is_object) {
        let env = root
            .get_mut("env")
            .and_then(serde_json::Value::as_object_mut)
            .expect("Claude Code env was checked as an object");
        for key in [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL",
            "EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL",
            CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV,
            CLAUDE_CODE_MAX_CONTEXT_TOKENS_ENV,
            CLAUDE_CODE_AUTO_COMPACT_WINDOW_ENV,
            CLAUDE_CODE_AUTO_MODE_SERVER_ENV,
            CLAUDE_AUTOCOMPACT_PCT_OVERRIDE_ENV,
            DISABLE_AUTO_COMPACT_ENV,
            CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV,
            CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV,
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_EFFORT_LEVEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
        ] {
            restore_json_key(env, original_env, key);
        }
        if env.is_empty() && original_env.is_none() {
            root.remove("env");
        }
    } else {
        restore_json_key(&mut root, original_root.as_ref(), "env");
    }
    render_restored_json(root, original.is_some(), "Restored Claude Code configuration")
}

pub(crate) fn build_restored_claude_desktop_config(
    index: usize,
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let mut root = parse_agent_json_object(Some(current), "Current Claude Desktop configuration")?;
    let original_root = parse_restored_json_object(original, "Original Claude Desktop configuration")?;
    match index {
        0 | 1 => restore_json_key(&mut root, original_root.as_ref(), "deploymentMode"),
        2 => {
            for key in [
                "disableDeploymentModeChooser",
                "inferenceGatewayApiKey",
                "inferenceGatewayAuthScheme",
                "inferenceGatewayBaseUrl",
                "inferenceProvider",
                "inferenceModels",
            ] {
                restore_json_key(&mut root, original_root.as_ref(), key);
            }
        }
        3 => {
            restore_json_key(&mut root, original_root.as_ref(), "appliedId");
            let mut entries = root
                .remove("entries")
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            entries.retain(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str)
                    != Some(CLAUDE_DESKTOP_PROFILE_ID)
            });
            let original_managed_entries = original_root
                .as_ref()
                .and_then(|root| root.get("entries"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    entry.get("id").and_then(serde_json::Value::as_str)
                        == Some(CLAUDE_DESKTOP_PROFILE_ID)
                })
                .cloned();
            entries.extend(original_managed_entries);
            if !entries.is_empty()
                || original_root
                    .as_ref()
                    .and_then(|root| root.get("entries"))
                    .is_some()
            {
                root.insert("entries".to_string(), serde_json::Value::Array(entries));
            }
        }
        _ => return Err("Invalid Claude Desktop configuration file index".to_string()),
    }
    render_restored_json(root, original.is_some(), "Restored Claude Desktop configuration")
}

pub(crate) fn build_restored_opencode_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let mut root = parse_agent_json5_object(Some(current), "Current OpenCode configuration")?;
    let original_root = parse_restored_json5_object(original, "Original OpenCode configuration")?;
    for key in ["$schema", "model"] {
        restore_json_key(&mut root, original_root.as_ref(), key);
    }
    let original_provider = original_root
        .as_ref()
        .and_then(|root| root.get("provider"))
        .and_then(serde_json::Value::as_object);
    let original_managed = original_provider
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
        .and_then(serde_json::Value::as_object);
    if root
        .get("provider")
        .is_some_and(serde_json::Value::is_object)
    {
        let providers = root
            .get_mut("provider")
            .and_then(serde_json::Value::as_object_mut)
            .expect("OpenCode provider was checked as an object");
        if providers
            .get(MANAGED_AGENT_PROVIDER_ID)
            .is_some_and(serde_json::Value::is_object)
        {
            let managed = providers
                .get_mut(MANAGED_AGENT_PROVIDER_ID)
                .and_then(serde_json::Value::as_object_mut)
                .expect("OpenCode managed provider was checked as an object");
            for key in ["npm", "name", "models"] {
                restore_json_key(managed, original_managed, key);
            }
            let original_options = original_managed
                .and_then(|managed| managed.get("options"))
                .and_then(serde_json::Value::as_object);
            if managed
                .get("options")
                .is_some_and(serde_json::Value::is_object)
            {
                let options = managed
                    .get_mut("options")
                    .and_then(serde_json::Value::as_object_mut)
                    .expect("OpenCode options were checked as an object");
                for key in ["baseURL", "apiKey"] {
                    restore_json_key(options, original_options, key);
                }
                if options.is_empty() && original_options.is_none() {
                    managed.remove("options");
                }
            } else {
                restore_json_key(managed, original_managed, "options");
            }
            if managed.is_empty() && original_managed.is_none() {
                providers.remove(MANAGED_AGENT_PROVIDER_ID);
            }
        } else if let Some(original_managed) = original_provider
            .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
            .cloned()
        {
            providers.insert(MANAGED_AGENT_PROVIDER_ID.to_string(), original_managed);
        }
        if providers.is_empty() && original_provider.is_none() {
            root.remove("provider");
        }
    } else {
        restore_json_key(&mut root, original_root.as_ref(), "provider");
    }
    let restored = render_restored_json(root, original.is_some(), "Restored OpenCode configuration")?;
    Ok(restored.map(|rendered| {
        let comments = extract_json5_comments(current);
        if comments.is_empty() {
            rendered
        } else {
            format!("{}\n{rendered}", comments.join("\n"))
        }
    }))
}

pub(crate) fn build_restored_zcode_provider_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let mut root = serde_json::Value::Object(parse_agent_json_object(
        Some(current), "Current ZCode provider configuration",
    )?);
    let original_root = parse_restored_json_object(original, "Original ZCode provider configuration")?
        .map(serde_json::Value::Object);
    if root.pointer("/config").is_none() {
        return render_restored_json(
            root.as_object().cloned().ok_or("Invalid ZCode provider configuration")?,
            original.is_some(),
            "Restored ZCode provider configuration",
        );
    }
    let config = root.pointer_mut("/config").and_then(serde_json::Value::as_object_mut)
        .ok_or("ZCode provider configuration has invalid config")?;
    let original_config = original_root.as_ref().and_then(|value| value.pointer("/config"));
    if original.is_none() || config.get("defaultModelSelection")
        .and_then(|selection| selection.get("providerId"))
        .and_then(serde_json::Value::as_str) == Some(MANAGED_AGENT_PROVIDER_ID)
    {
        if let Some(selection) = original_config.and_then(|value| value.get("defaultModelSelection")) {
            config.insert("defaultModelSelection".to_string(), selection.clone());
        } else {
            config.remove("defaultModelSelection");
        }
    }
    if let Some(order) = config.get_mut("providerOrder").and_then(serde_json::Value::as_array_mut) {
        order.retain(|item| item.as_str() != Some(MANAGED_AGENT_PROVIDER_ID));
        if let Some(original_order) = original_config
            .and_then(|value| value.get("providerOrder"))
            .and_then(serde_json::Value::as_array)
        {
            if let Some(index) = original_order.iter().position(|item| item.as_str() == Some(MANAGED_AGENT_PROVIDER_ID)) {
                order.insert(index.min(order.len()), serde_json::json!(MANAGED_AGENT_PROVIDER_ID));
            }
        }
    }
    for pointer in [
        "/providerConfigRules/providerRules",
        "/modelConfigRules/providerModelRules",
        "/modelConfigRules/manualProviderModelRules",
    ] {
        let Some(rules) = root.pointer(&format!("/config{pointer}"))
            .and_then(serde_json::Value::as_array).cloned() else {
            continue;
        };
        let mut retained = rules.into_iter().filter(|rule| {
            rule.get("providerId").and_then(serde_json::Value::as_str)
                != Some(MANAGED_AGENT_PROVIDER_ID)
        }).collect::<Vec<_>>();
        if let Some(original_rules) = original_config.and_then(|value| value.pointer(pointer))
            .and_then(serde_json::Value::as_array)
        {
            for (index, rule) in original_rules.iter().enumerate() {
                if rule.get("providerId").and_then(serde_json::Value::as_str)
                    == Some(MANAGED_AGENT_PROVIDER_ID)
                {
                    retained.insert(index.min(retained.len()), rule.clone());
                }
            }
        }
        if let Some(target) = root.pointer_mut(&format!("/config{pointer}")) {
            *target = serde_json::Value::Array(retained);
        }
    }
    if original.is_none() {
        let config = root.get("config");
        let only_empty_scaffold = config
            .and_then(serde_json::Value::as_object)
            .is_some_and(|config| {
                config.get("defaultModelSelection").is_none()
                    && config.get("providerOrder").and_then(serde_json::Value::as_array)
                        .is_none_or(Vec::is_empty)
                    && root.pointer("/config/providerConfigRules/providerRules")
                        .and_then(serde_json::Value::as_array).is_some_and(Vec::is_empty)
                    && root.pointer("/config/modelConfigRules/providerModelRules")
                        .and_then(serde_json::Value::as_array).is_some_and(Vec::is_empty)
                    && root.pointer("/config/modelConfigRules/manualProviderModelRules")
                        .and_then(serde_json::Value::as_array).is_some_and(Vec::is_empty)
                    && config.len() <= 3
            });
        if only_empty_scaffold && root.as_object().is_some_and(|root| root.len() <= 2) {
            return Ok(None);
        }
    }
    render_agent_json(root.as_object().cloned().ok_or("Invalid ZCode provider configuration")?,
        "Restored ZCode provider configuration").map(Some)
}

pub(crate) fn build_restored_kimi_code_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    build_restored_managed_toml_client_config(
        current,
        original,
        "Kimi Code",
        Some(("providers", &["type", "base_url", "api_key"])),
        "models",
        None,
        "default_model",
    )
}

pub(crate) fn build_restored_grok_build_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    build_restored_managed_toml_client_config(
        current,
        original,
        "Grok Build",
        None,
        "model",
        Some("models"),
        "default",
    )
}

pub(crate) fn build_restored_managed_toml_client_config(
    current: &str,
    original: Option<&str>,
    label: &str,
    provider: Option<(&str, &[&str])>,
    model_entry_section: &str,
    default_section: Option<&str>,
    default_key: &str,
) -> Result<Option<String>, String> {
    use toml_edit::Item;

    let mut current_document = parse_codex_document(Some(current), &format!("Current {label} configuration"))?;
    let original_document = original
        .map(|content| parse_codex_document(Some(content), &format!("Original {label} configuration")))
        .transpose()?;
    if let Some(original_document) = original_document.as_ref() {
        merge_missing_codex_table_items(
            current_document.as_table_mut(),
            original_document.as_table(),
        );
    }

    if let Some(section) = default_section {
        let original_table = original_document
            .as_ref()
            .and_then(|document| document.as_table().get(section))
            .and_then(Item::as_table);
        if let Some(current_table) = current_document
            .as_table_mut()
            .get_mut(section)
            .and_then(Item::as_table_mut)
        {
            restore_codex_table_item(current_table, original_table, default_key);
        }
    } else {
        restore_codex_table_item(
            current_document.as_table_mut(),
            original_document
                .as_ref()
                .map(|document| document.as_table()),
            default_key,
        );
    }

    if let Some((section, managed_keys)) = provider {
        let original_provider = original_document
            .as_ref()
            .and_then(|document| document.as_table().get(section))
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
            .and_then(Item::as_table)
            .cloned();
        if let Some(providers) = current_document
            .as_table_mut()
            .get_mut(section)
            .and_then(Item::as_table_mut)
        {
            if let Some(current_provider) = providers
                .get_mut(MANAGED_AGENT_PROVIDER_ID)
                .and_then(Item::as_table_mut)
            {
                for key in managed_keys {
                    restore_codex_table_item(current_provider, original_provider.as_ref(), key);
                }
                if current_provider.is_empty() {
                    providers.remove(MANAGED_AGENT_PROVIDER_ID);
                }
            }
        }
    }

    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let original_models = original_document
        .as_ref()
        .and_then(|document| document.as_table().get(model_entry_section))
        .and_then(Item::as_table)
        .map(|models| {
            models
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(key, item)| (key.to_string(), item.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(models) = current_document
        .as_table_mut()
        .get_mut(model_entry_section)
        .and_then(Item::as_table_mut)
    {
        let managed = models
            .iter()
            .map(|(key, _)| key.to_string())
            .filter(|key| key.starts_with(&prefix))
            .collect::<Vec<_>>();
        for key in managed {
            models.remove(&key);
        }
        for (key, item) in &original_models {
            models.insert(key, item.clone());
        }
    } else if !original_models.is_empty() {
        let models = ensure_toml_child_table(current_document.as_table_mut(), model_entry_section);
        for (key, item) in &original_models {
            models.insert(key, item.clone());
        }
    }

    for section in provider
        .map(|(section, _)| section)
        .into_iter()
        .chain([model_entry_section])
        .chain(default_section)
    {
        let empty = current_document
            .as_table()
            .get(section)
            .and_then(Item::as_table)
            .is_some_and(toml_edit::Table::is_empty);
        if empty {
            current_document.as_table_mut().remove(section);
        }
    }
    let rendered = current_document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate restored {label} configuration: {error}"))?;
    if original.is_none() && rendered.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(rendered))
    }
}

pub(crate) fn build_restored_openclaw_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let current_value = json5::from_str::<serde_json::Value>(current)
        .map_err(|error| format!("Invalid current OpenClaw configuration format: {error}"))?;
    let mut root = current_value
        .as_object()
        .cloned()
        .ok_or_else(|| "Current OpenClaw configuration root must be an object".to_string())?;
    let original_root = original
        .map(|content| {
            json5::from_str::<serde_json::Value>(content)
                .map_err(|error| format!("Invalid original OpenClaw configuration format: {error}"))?
                .as_object()
                .cloned()
                .ok_or_else(|| "Original OpenClaw configuration root must be an object".to_string())
        })
        .transpose()?;
    let original_models = original_root
        .as_ref()
        .and_then(|root| root.get("models"))
        .and_then(serde_json::Value::as_object);
    if root.get("models").is_some_and(serde_json::Value::is_object) {
        let models = root
            .get_mut("models")
            .and_then(serde_json::Value::as_object_mut)
            .expect("OpenClaw models were checked as an object");
        restore_json_key(models, original_models, "mode");
        let original_providers = original_models
            .and_then(|models| models.get("providers"))
            .and_then(serde_json::Value::as_object);
        if models
            .get("providers")
            .is_some_and(serde_json::Value::is_object)
        {
            let providers = models
                .get_mut("providers")
                .and_then(serde_json::Value::as_object_mut)
                .expect("OpenClaw providers were checked as an object");
            let original_managed = original_providers
                .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
                .and_then(serde_json::Value::as_object);
            if providers
                .get(MANAGED_AGENT_PROVIDER_ID)
                .is_some_and(serde_json::Value::is_object)
            {
                let managed = providers
                    .get_mut(MANAGED_AGENT_PROVIDER_ID)
                    .and_then(serde_json::Value::as_object_mut)
                    .expect("OpenClaw managed provider was checked as an object");
                for key in ["baseUrl", "apiKey", "api", "models"] {
                    restore_json_key(managed, original_managed, key);
                }
                if managed.is_empty() && original_managed.is_none() {
                    providers.remove(MANAGED_AGENT_PROVIDER_ID);
                }
            } else if let Some(original_managed) = original_providers
                .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
                .cloned()
            {
                providers.insert(MANAGED_AGENT_PROVIDER_ID.to_string(), original_managed);
            }
            if providers.is_empty() && original_providers.is_none() {
                models.remove("providers");
            }
        } else {
            restore_json_key(models, original_models, "providers");
        }
        if models.is_empty() && original_models.is_none() {
            root.remove("models");
        }
    } else {
        restore_json_key(&mut root, original_root.as_ref(), "models");
    }

    let original_defaults = original_root
        .as_ref()
        .and_then(|root| root.get("agents"))
        .and_then(|agents| agents.get("defaults"))
        .and_then(serde_json::Value::as_object);
    if let Some(defaults) = root
        .get_mut("agents")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|agents| agents.get_mut("defaults"))
        .and_then(serde_json::Value::as_object_mut)
    {
        let original_model = original_defaults
            .and_then(|defaults| defaults.get("model"))
            .and_then(serde_json::Value::as_object);
        if defaults
            .get("model")
            .is_some_and(serde_json::Value::is_object)
        {
            let model = defaults
                .get_mut("model")
                .and_then(serde_json::Value::as_object_mut)
                .expect("OpenClaw default model was checked as an object");
            restore_json_key(model, original_model, "primary");
            if model.is_empty() && original_model.is_none() {
                defaults.remove("model");
            }
        } else {
            restore_json_key(defaults, original_defaults, "model");
        }

        let original_catalog = original_defaults
            .and_then(|defaults| defaults.get("models"))
            .and_then(serde_json::Value::as_object);
        if defaults
            .get("models")
            .is_some_and(serde_json::Value::is_object)
        {
            let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
            let catalog = defaults
                .get_mut("models")
                .and_then(serde_json::Value::as_object_mut)
                .expect("OpenClaw model catalog was checked as an object");
            catalog.retain(|name, _| !name.starts_with(&prefix));
            if let Some(original_catalog) = original_catalog {
                catalog.extend(
                    original_catalog
                        .iter()
                        .filter(|(name, _)| name.starts_with(&prefix))
                        .map(|(name, value)| (name.clone(), value.clone())),
                );
            }
            if catalog.is_empty() && original_catalog.is_none() {
                defaults.remove("models");
            }
        } else {
            restore_json_key(defaults, original_defaults, "models");
        }
    }
    let original_agents = original_root
        .as_ref()
        .and_then(|root| root.get("agents"))
        .and_then(serde_json::Value::as_object);
    if let Some(agents) = root
        .get_mut("agents")
        .and_then(serde_json::Value::as_object_mut)
    {
        let remove_defaults = agents
            .get("defaults")
            .and_then(serde_json::Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
            && original_defaults.is_none();
        if remove_defaults {
            agents.remove("defaults");
        }
        if agents.is_empty() && original_agents.is_none() {
            root.remove("agents");
        }
    }
    if root.is_empty() && original.is_none() {
        return Ok(None);
    }
    let rendered = render_agent_json(root, "Restored OpenClaw configuration")?;
    let comments = extract_json5_comments(current);
    Ok(Some(if comments.is_empty() {
        rendered
    } else {
        format!("{}\n{rendered}", comments.join("\n"))
    }))
}

pub(crate) fn restore_yaml_key(
    current: &mut serde_norway::Mapping,
    original: Option<&serde_norway::Mapping>,
    key: &str,
) {
    if let Some(value) = original.and_then(|root| root.get(yaml_key(key))).cloned() {
        current.insert(yaml_key(key), value);
    } else {
        current.remove(yaml_key(key));
    }
}

pub(crate) fn build_restored_hermes_config(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let mut document = yaml_serde_edit::YamlValue::parse(current)
        .map_err(|error| format!("Invalid current Hermes configuration format: {error}"))?;
    let mut updated = document.get().clone();
    let root = updated
        .as_mapping_mut()
        .ok_or_else(|| "Current Hermes configuration root must be a mapping".to_string())?;
    let original_value = original
        .map(|content| {
            serde_norway::from_str::<serde_norway::Value>(content)
                .map_err(|error| format!("Invalid original Hermes configuration format: {error}"))
        })
        .transpose()?;
    let original_root = original_value
        .as_ref()
        .and_then(serde_norway::Value::as_mapping);
    let original_managed = original_root
        .and_then(|root| root.get(yaml_key("custom_providers")))
        .and_then(serde_norway::Value::as_sequence)
        .and_then(|providers| {
            providers.iter().find(|provider| {
                provider.get("name").and_then(serde_norway::Value::as_str)
                    == Some(MANAGED_AGENT_PROVIDER_ID)
            })
        });
    if let Some(providers) = root
        .get_mut(yaml_key("custom_providers"))
        .and_then(serde_norway::Value::as_sequence_mut)
    {
        let current_managed = providers
            .iter()
            .find(|provider| {
                provider.get("name").and_then(serde_norway::Value::as_str)
                    == Some(MANAGED_AGENT_PROVIDER_ID)
            })
            .and_then(serde_norway::Value::as_mapping)
            .cloned();
        providers.retain(|provider| {
            provider.get("name").and_then(serde_norway::Value::as_str)
                != Some(MANAGED_AGENT_PROVIDER_ID)
        });
        let mut managed = current_managed.unwrap_or_default();
        let original_managed = original_managed.and_then(serde_norway::Value::as_mapping);
        for key in ["name", "base_url", "api_key", "api_mode", "model", "models"] {
            restore_yaml_key(&mut managed, original_managed, key);
        }
        if original_managed.is_some() {
            providers.push(serde_norway::Value::Mapping(managed));
        } else if !managed.is_empty() {
            managed.insert(
                yaml_key("name"),
                serde_norway::Value::String(MANAGED_AGENT_PROVIDER_ID.to_string()),
            );
            providers.push(serde_norway::Value::Mapping(managed));
        }
        if providers.is_empty()
            && original_root
                .and_then(|root| root.get(yaml_key("custom_providers")))
                .is_none()
        {
            root.remove(yaml_key("custom_providers"));
        }
    } else {
        restore_yaml_key(root, original_root, "custom_providers");
    }
    let original_model = original_root
        .and_then(|root| root.get(yaml_key("model")))
        .and_then(serde_norway::Value::as_mapping);
    if let Some(model) = root
        .get_mut(yaml_key("model"))
        .and_then(serde_norway::Value::as_mapping_mut)
    {
        for key in ["default", "provider"] {
            restore_yaml_key(model, original_model, key);
        }
        if model.is_empty() && original_model.is_none() {
            root.remove(yaml_key("model"));
        }
    } else {
        restore_yaml_key(root, original_root, "model");
    }
    if root.is_empty() && original.is_none() {
        return Ok(None);
    }
    render_updated_core_yaml(&mut document, updated).map(Some)
}

pub(crate) fn restore_deepseek_harness_provider(
    current: &mut serde_norway::Mapping,
    original: Option<&serde_norway::Mapping>,
) -> Result<(), String> {
    let original_llm = original
        .and_then(|root| yaml_mapping_value(root, "llm-pi-ai"))
        .and_then(serde_norway::Value::as_mapping);
    let original_providers = original_llm
        .and_then(|llm| yaml_mapping_value(llm, "providers"))
        .and_then(serde_norway::Value::as_mapping);
    let original_provider = original_providers
        .and_then(|providers| yaml_mapping_value(providers, DEEPSEEK_HARNESS_PROVIDER_ID))
        .cloned();

    if let Some(original_provider) = original_provider {
        let llm = current
            .entry(yaml_key("llm-pi-ai"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "Current DeepSeek Harness llm-pi-ai must be a mapping".to_string())?;
        let providers = llm
            .entry(yaml_key("providers"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()))
            .as_mapping_mut()
            .ok_or_else(|| "Current DeepSeek Harness llm-pi-ai.providers must be a mapping".to_string())?;
        providers.insert(yaml_key(DEEPSEEK_HARNESS_PROVIDER_ID), original_provider);
    } else {
        let mut remove_llm = false;
        if let Some(llm) = current
            .get_mut(yaml_key("llm-pi-ai"))
            .and_then(serde_norway::Value::as_mapping_mut)
        {
            let mut remove_providers = false;
            if let Some(providers) = llm
                .get_mut(yaml_key("providers"))
                .and_then(serde_norway::Value::as_mapping_mut)
            {
                providers.remove(yaml_key(DEEPSEEK_HARNESS_PROVIDER_ID));
                remove_providers = providers.is_empty() && original_providers.is_none();
            }
            if remove_providers {
                llm.remove(yaml_key("providers"));
            }
            remove_llm = llm.is_empty() && original_llm.is_none();
        }
        if remove_llm {
            current.remove(yaml_key("llm-pi-ai"));
        }
    }
    Ok(())
}

pub(crate) fn build_restored_deepseek_harness_settings(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let original_root = parse_agent_yaml_mapping(original, "Original DeepSeek Harness settings")?;
    let rendered = render_agent_yaml_mapping_update(
        Some(current),
        "Current DeepSeek Harness settings",
        |root| {
            restore_deepseek_harness_provider(root, Some(&original_root))?;
            restore_yaml_key(root, Some(&original_root), "agent-default-model");
            Ok(())
        },
    )?;
    let root = parse_agent_yaml_mapping(Some(&rendered), "Restored DeepSeek Harness settings")?;
    Ok((!root.is_empty() || original.is_some()).then_some(rendered))
}

pub(crate) fn build_restored_deepseek_harness_credentials(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let original_root = parse_agent_yaml_mapping(original, "Original DeepSeek Harness credentials")?;
    let original_credential = deepseek_harness_original_credential(&original_root).cloned();
    let original_had_refs = yaml_mapping_value(&original_root, "refs").is_some();
    let rendered = render_agent_yaml_mapping_update(
        Some(current),
        "Current DeepSeek Harness credentials",
        |root| {
            let refs =
                deepseek_harness_credentials_refs_mut(root, "Current DeepSeek Harness credentials")?;
            if let Some(value) = original_credential {
                refs.insert(yaml_key(DEEPSEEK_HARNESS_CREDENTIAL), value);
            } else {
                refs.remove(yaml_key(DEEPSEEK_HARNESS_CREDENTIAL));
            }
            let remove_refs = refs.is_empty() && !original_had_refs;
            if remove_refs {
                root.remove(yaml_key("refs"));
            }
            Ok(())
        },
    )?;
    let root = parse_agent_yaml_mapping(Some(&rendered), "Restored DeepSeek Harness credentials")?;
    if deepseek_harness_credentials_document_is_empty(&root) {
        if let Some(original) = original.filter(|_| original_root.is_empty()) {
            return Ok(Some(original.to_string()));
        }
        if original.is_none() {
            return Ok(None);
        }
    }
    Ok(Some(rendered))
}

pub(crate) fn agent_config_semantically_equal(
    client: AgentClient,
    actual: &str,
    expected: &str,
) -> bool {
    match client {
        AgentClient::Codex | AgentClient::KimiCode | AgentClient::GrokBuild => {
            toml::from_str::<toml::Value>(actual).ok()
                == toml::from_str::<toml::Value>(expected).ok()
        }
        AgentClient::OpenCode | AgentClient::OpenClaw => {
            json5::from_str::<serde_json::Value>(actual).ok()
                == json5::from_str::<serde_json::Value>(expected).ok()
        }
        AgentClient::Hermes | AgentClient::Omp => {
            serde_norway::from_str::<serde_norway::Value>(actual).ok()
                == serde_norway::from_str::<serde_norway::Value>(expected).ok()
        }
        AgentClient::DeepSeekHarness => {
            serde_norway::from_str::<serde_norway::Value>(actual).ok()
                == serde_norway::from_str::<serde_norway::Value>(expected).ok()
        }
        _ => {
            serde_json::from_str::<serde_json::Value>(actual).ok()
                == serde_json::from_str::<serde_json::Value>(expected).ok()
        }
    }
}

pub(crate) fn build_agent_session_restored_bytes(
    client: AgentClient,
    paths: &[PathBuf],
    path: &Path,
    current: Option<&[u8]>,
    original: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, String> {
    build_agent_session_restored_bytes_with_preference(
        client, paths, path, current, original, true,
    )
}

pub(crate) fn build_agent_session_restored_bytes_with_preference(
    client: AgentClient,
    paths: &[PathBuf],
    path: &Path,
    current: Option<&[u8]>,
    original: Option<&[u8]>,
    prefer_exact_original: bool,
) -> Result<Option<Vec<u8>>, String> {
    let Some(current) = current else {
        return Ok(original.map(ToOwned::to_owned));
    };
    if client == AgentClient::Codex && paths.first().is_none_or(|config| config != path) {
        return Ok(original.map(ToOwned::to_owned));
    }
    let current = std::str::from_utf8(current)
        .map_err(|_| format!("Current agent configuration is not UTF-8 text: {}", path_to_string(path)))?;
    let original_bytes = original;
    let original = original_bytes
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map_err(|_| format!("Original agent configuration is not UTF-8 text: {}", path_to_string(path)))
        })
        .transpose()?;
    let restored = match client {
        AgentClient::ClaudeCode => build_restored_claude_code_config(current, original)?,
        AgentClient::ClaudeDesktop => {
            let index = paths
                .iter()
                .position(|candidate| candidate == path)
                .ok_or_else(|| "Claude Desktop restoration path mismatch".to_string())?;
            build_restored_claude_desktop_config(index, current, original)?
        }
        AgentClient::Codex => build_restored_codex_agent_config(Some(current), original)?,
        AgentClient::OpenCode => build_restored_opencode_config(current, original)?,
        AgentClient::OpenClaw => build_restored_openclaw_config(current, original)?,
        AgentClient::Hermes => build_restored_hermes_config(current, original)?,
        AgentClient::DeepSeekHarness => {
            let index = paths
                .iter()
                .position(|candidate| candidate == path)
                .ok_or_else(|| "DeepSeek Harness restoration path mismatch".to_string())?;
            match index {
                0 => build_restored_deepseek_harness_settings(current, original)?,
                1 => build_restored_deepseek_harness_credentials(current, original)?,
                _ => return Err("Invalid DeepSeek Harness restoration path index".to_string()),
            }
        }
        AgentClient::ZCode => build_restored_zcode_provider_config(current, original)?,
        AgentClient::WorkBuddy => build_restored_workbuddy_config(current, original)?,
        AgentClient::AntigravityCli => restore_antigravity_config(client, path, current, original)?,
        AgentClient::KimiCode => build_restored_kimi_code_config(current, original)?,
        AgentClient::GrokBuild => build_restored_grok_build_config(current, original)?,
        AgentClient::Omp => {
            let index = paths
                .iter()
                .position(|candidate| candidate == path)
                .ok_or_else(|| "Oh My Pi restoration path mismatch".to_string())?;
            match index {
                0 => build_restored_omp_models(current, original)?,
                1 => build_restored_omp_settings(current, original)?,
                _ => return Err("Invalid Oh My Pi restoration path index".to_string()),
            }
        }
    };
    if prefer_exact_original {
        if let (Some(restored), Some(original), Some(original_bytes)) =
            (restored.as_deref(), original, original_bytes)
        {
            if agent_config_semantically_equal(client, restored, original) {
                return Ok(Some(original_bytes.to_vec()));
            }
        }
    }
    Ok(restored.map(String::into_bytes))
}

#[cfg(test)]
pub(crate) fn build_codex_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<String, String> {
    build_codex_agent_config_with_oauth(existing, base_url, api_key, model, false)
}

pub(crate) fn build_codex_agent_config_with_oauth(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    oauth_configuration: bool,
) -> Result<String, String> {
    use toml_edit::{value, Document, Item, Table};

    let mut document = match existing.filter(|value| !value.trim().is_empty()) {
        Some(value) => value
            .parse::<Document>()
            .map_err(|error| format!("Invalid Codex config.toml format: {error}"))?,
        None => Document::new(),
    };
    set_codex_table_item(
        document.as_table_mut(),
        "model_provider",
        value(MANAGED_AGENT_PROVIDER_ID),
    );
    set_codex_table_item(document.as_table_mut(), "model", value(model));
    set_codex_table_item(
        document.as_table_mut(),
        "model_catalog_json",
        value(CODEX_MODEL_CATALOG_FILE),
    );
    if !document
        .get("model_providers")
        .is_some_and(toml_edit::Item::is_table)
    {
        document.remove("model_providers");
        document["model_providers"] = Item::Table(Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| "Codex model_providers must be a TOML table".to_string())?;
    if !providers
        .get(MANAGED_AGENT_PROVIDER_ID)
        .is_some_and(toml_edit::Item::is_table)
    {
        providers.remove(MANAGED_AGENT_PROVIDER_ID);
        providers.insert(MANAGED_AGENT_PROVIDER_ID, Item::Table(Table::new()));
    }
    let provider = providers
        .get_mut(MANAGED_AGENT_PROVIDER_ID)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "Codex cpa-gui provider must be a TOML table".to_string())?;
    set_codex_table_item(provider, "name", value("EasyCLIProxyAPI"));
    set_codex_table_item(provider, "base_url", value(base_url));
    set_codex_table_item(provider, "wire_api", value("responses"));
    set_codex_table_item(provider, "experimental_bearer_token", value(api_key));
    if oauth_configuration {
        set_codex_table_item(provider, "requires_openai_auth", value(true));
    } else {
        provider.remove("requires_openai_auth");
    }
    let rendered = document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate Codex configuration: {error}"))?;
    Ok(rendered)
}

pub(crate) fn build_codex_api_auth(api_key: &str) -> Result<String, String> {
    let mut rendered = serde_json::to_string_pretty(&serde_json::json!({
        "auth_mode": "apikey",
        "OPENAI_API_KEY": api_key,
    }))
    .map_err(|error| format!("Failed to generate Codex auth.json: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn build_codex_auth_update(
    home: &Path,
    api_key: &str,
    oauth_configuration: bool,
) -> Result<AgentFileUpdate, String> {
    let path = codex_configuration_directory(home).join("auth.json");
    let after = if oauth_configuration {
        if validate_codex_oauth_login_at(&path).is_ok() {
            read_optional_text(&path)?
                .ok_or_else(|| "Codex OAuth login credentials were deleted before the application configuration was applied. Sign in again".to_string())?
        } else {
            available_codex_oauth_auth(home)?
        }
    } else {
        let path_text = read_optional_text(&path)?;
        let mut root = parse_agent_json_object(path_text.as_deref(), "Codex auth.json")?;
        for key in ["tokens", "last_refresh"] { root.remove(key); }
        root.insert("auth_mode".into(), serde_json::json!("apikey"));
        root.insert("OPENAI_API_KEY".into(), serde_json::json!(api_key));
        render_agent_json(root, "Codex auth.json")?
    };
    Ok(AgentFileUpdate { path, after })
}

const CODEX_MANAGED_ROOT_KEYS: [&str; 3] = ["model_provider", "model", "model_catalog_json"];
const CODEX_MANAGED_PROVIDER_KEYS: [&str; 5] = [
    "name",
    "base_url",
    "wire_api",
    "experimental_bearer_token",
    "requires_openai_auth",
];

pub(crate) fn set_codex_table_item(
    table: &mut toml_edit::Table,
    key: &str,
    mut item: toml_edit::Item,
) {
    if let (Some(current), Some(next)) = (
        table.get(key).and_then(toml_edit::Item::as_value),
        item.as_value_mut(),
    ) {
        *next.decor_mut() = current.decor().clone();
    }
    *table.entry(key).or_insert(toml_edit::Item::None) = item;
}

pub(crate) fn restore_codex_table_item(
    current: &mut toml_edit::Table,
    original: Option<&toml_edit::Table>,
    key: &str,
) {
    let original_item = original.and_then(|table| table.get(key)).cloned();
    if let Some(item) = original_item {
        let existed = current.contains_key(key);
        let original_key_decor = original.and_then(|table| table.key_decor(key)).cloned();
        set_codex_table_item(current, key, item);
        if !existed {
            if let (Some(current_decor), Some(original_decor)) =
                (current.key_decor_mut(key), original_key_decor)
            {
                *current_decor = original_decor;
            }
        }
    } else {
        current.remove(key);
    }
}

pub(crate) fn parse_codex_document(
    content: Option<&str>,
    label: &str,
) -> Result<toml_edit::Document, String> {
    use toml_edit::Document;

    match content.filter(|value| !value.trim().is_empty()) {
        Some(value) => value
            .parse::<Document>()
            .map_err(|error| format!("Invalid {label} format: {error}")),
        None => Ok(Document::new()),
    }
}

pub(crate) fn codex_provider_table(document: &toml_edit::Document) -> Option<&toml_edit::Table> {
    document
        .as_table()
        .get("model_providers")
        .and_then(toml_edit::Item::as_table)
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
        .and_then(toml_edit::Item::as_table)
}

pub(crate) fn ensure_codex_provider_table(
    document: &mut toml_edit::Document,
) -> Result<&mut toml_edit::Table, String> {
    use toml_edit::{Item, Table};

    let root = document.as_table_mut();
    if !root.contains_key("model_providers") {
        root.insert("model_providers", Item::Table(Table::new()));
    }
    let providers = root
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "Codex model_providers must be a TOML table".to_string())?;
    if !providers.contains_key(MANAGED_AGENT_PROVIDER_ID) {
        providers.insert(MANAGED_AGENT_PROVIDER_ID, Item::Table(Table::new()));
    }
    providers
        .get_mut(MANAGED_AGENT_PROVIDER_ID)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "Codex cpa-gui provider must be a TOML table".to_string())
}

pub(crate) fn merge_missing_codex_table_items(
    current: &mut toml_edit::Table,
    original: &toml_edit::Table,
) {
    for (key, original_item) in original.iter() {
        if !current.contains_key(key) {
            current.insert(key, original_item.clone());
            continue;
        }
        if let (Some(current_table), Some(original_table)) = (
            current.get_mut(key).and_then(toml_edit::Item::as_table_mut),
            original_item.as_table(),
        ) {
            merge_missing_codex_table_items(current_table, original_table);
        }
    }
}

pub(crate) fn build_restored_codex_agent_config(
    current: Option<&str>,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    build_restored_codex_agent_config_with_policy(current, original, true)
}

pub(crate) fn build_restored_codex_agent_config_with_policy(
    current: Option<&str>,
    original: Option<&str>,
    retain_dormant_provider: bool,
) -> Result<Option<String>, String> {
    use toml_edit::Item;

    let mut current_document = parse_codex_document(current, "Current Codex config.toml")?;
    let original_document = original
        .map(|content| parse_codex_document(Some(content), "Original Codex config.toml"))
        .transpose()?;
    if let Some(original_document) = original_document.as_ref() {
        merge_missing_codex_table_items(
            current_document.as_table_mut(),
            original_document.as_table(),
        );
    }

    for key in CODEX_MANAGED_ROOT_KEYS {
        restore_codex_table_item(
            current_document.as_table_mut(),
            original_document
                .as_ref()
                .map(|document| document.as_table()),
            key,
        );
    }

    let original_provider_items = CODEX_MANAGED_PROVIDER_KEYS
        .into_iter()
        .map(|key| {
            (
                key,
                original_document
                    .as_ref()
                    .and_then(codex_provider_table)
                    .and_then(|provider| provider.get(key))
                    .cloned(),
            )
        })
        .collect::<Vec<_>>();
    let needs_provider = original_provider_items
        .iter()
        .any(|(_, item)| item.is_some());

    if needs_provider {
        let provider = ensure_codex_provider_table(&mut current_document)?;
        let original_provider = original_document.as_ref().and_then(codex_provider_table);
        for (key, item) in &original_provider_items {
            if item.is_some() {
                restore_codex_table_item(provider, original_provider, key);
            } else {
                provider.remove(key);
            }
        }
    } else if !retain_dormant_provider {
        if let Some(providers) = current_document
            .as_table_mut()
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
        {
            if let Some(provider) = providers
                .get_mut(MANAGED_AGENT_PROVIDER_ID)
                .and_then(Item::as_table_mut)
            {
                for key in CODEX_MANAGED_PROVIDER_KEYS {
                    provider.remove(key);
                }
                if provider.is_empty() {
                    providers.remove(MANAGED_AGENT_PROVIDER_ID);
                }
            }
        }
    }

    let remove_providers = current_document
        .as_table()
        .get("model_providers")
        .and_then(Item::as_table)
        .is_some_and(toml_edit::Table::is_empty);
    if remove_providers {
        current_document.as_table_mut().remove("model_providers");
    }

    let rendered = current_document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate restored Codex configuration: {error}"))?;
    if original.is_none() && rendered.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(rendered))
    }
}

pub(crate) fn validate_codex_catalog(catalog: &str, model: &str) -> Result<(), String> {
    let root: serde_json::Value = serde_json::from_str(catalog)
        .map_err(|error| format!("Invalid Codex model catalog format: {error}"))?;
    let models = root
        .get("models")
        .and_then(serde_json::Value::as_array)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| "Codex model catalog must contain a non-empty models array".to_string())?;
    if !models.iter().any(|entry| {
        entry
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|slug| slug.eq_ignore_ascii_case(model))
    }) {
        return Err(format!("Default model {model} is not in the generated Codex model catalog"));
    }
    Ok(())
}

pub(crate) fn build_opencode_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    let mut root = parse_agent_json5_object(existing, "OpenCode opencode.json")?;
    root.entry("$schema".to_string())
        .or_insert_with(|| serde_json::json!("https://opencode.ai/config.json"));
    let providers = ensure_json_object_entry(&mut root, "provider");
    let models = ordered_agent_models(available_models, model)
        .into_iter()
        .map(|model| {
            let display_name = model.alias.as_deref().unwrap_or(&model.name).to_string();
            (model.name, serde_json::json!({ "name": display_name }))
        })
        .collect::<serde_json::Map<_, _>>();
    let managed_provider = ensure_json_object_entry(providers, MANAGED_AGENT_PROVIDER_ID);
    managed_provider.insert(
        "npm".to_string(),
        serde_json::json!("@ai-sdk/openai-compatible"),
    );
    managed_provider.insert("name".to_string(), serde_json::json!("EasyCLIProxyAPI"));
    let options = ensure_json_object_entry(managed_provider, "options");
    options.insert("baseURL".to_string(), serde_json::json!(base_url));
    options.insert("apiKey".to_string(), serde_json::json!(api_key));
    managed_provider.insert("models".to_string(), serde_json::Value::Object(models));
    root.insert(
        "model".to_string(),
        serde_json::json!(format!("{MANAGED_AGENT_PROVIDER_ID}/{model}")),
    );
    let rendered = render_agent_json(root, "OpenCode configuration")?;
    let comments = existing.map(extract_json5_comments).unwrap_or_default();
    if comments.is_empty() {
        Ok(rendered)
    } else {
        Ok(format!("{}\n{rendered}", comments.join("\n")))
    }
}

pub(crate) fn build_zcode_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    let mut root = parse_agent_json_object(existing, "ZCode provider_config.json")?;
    if root.get("schemaVersion").is_some_and(|version| version != 1) {
        return Err("Unsupported ZCode provider configuration schemaVersion".to_string());
    }
    root.insert("schemaVersion".to_string(), serde_json::json!(1));
    let config = ensure_json_object_entry(&mut root, "config");
    let provider_rules = ensure_json_object_entry(config, "providerConfigRules");
    let rules = provider_rules
        .entry("providerRules")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("ZCode providerRules must be an array")?;
    let managed_index = rules.iter().position(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID)
    });
    let previous = managed_index.and_then(|index| rules.get(index));
    let mut managed = previous.cloned().unwrap_or_else(|| serde_json::json!({}));
    let managed_obj = managed.as_object_mut().ok_or("ZCode managed provider must be an object")?;
    managed_obj.insert("providerId".to_string(), serde_json::json!(MANAGED_AGENT_PROVIDER_ID));
    managed_obj.insert("providerName".to_string(), serde_json::json!("EasyCLIProxyAPI"));
    managed_obj.insert("enabled".to_string(), serde_json::json!(true));
    let provider_config = ensure_json_object_entry(managed_obj, "config");
    provider_config.insert("group".to_string(), serde_json::json!("standard-personal"));
    let access = ensure_json_object_entry(provider_config, "access");
    access.insert("type".to_string(), serde_json::json!("api-key"));
    access.insert("apiKey".to_string(), serde_json::json!(api_key));
    let api = ensure_json_object_entry(provider_config, "api");
    api.insert("type".to_string(), serde_json::json!("anthropic-messages"));
    api.insert("baseUrl".to_string(), serde_json::json!(base_url));
    let ordered = ordered_agent_models(available_models, model);
    let model_names = ordered.iter().map(|option| option.name.as_str()).collect::<Vec<_>>();
    provider_config.insert("personalModelIds".to_string(), serde_json::json!(model_names));
    provider_config.insert("modelOrder".to_string(), serde_json::json!(model_names));
    rules.retain(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            != Some(MANAGED_AGENT_PROVIDER_ID)
    });
    rules.insert(managed_index.unwrap_or(rules.len()).min(rules.len()), managed);

    let model_rules = ensure_json_object_entry(config, "modelConfigRules");
    let manual_rules = model_rules
        .entry("manualProviderModelRules")
        .or_insert_with(|| serde_json::json!([]))
        .as_array()
        .ok_or("ZCode manualProviderModelRules must be an array")?;
    let existing_manual_rules = manual_rules.iter().filter_map(|rule| {
        if rule.get("providerId").and_then(serde_json::Value::as_str)
            != Some(MANAGED_AGENT_PROVIDER_ID) {
            return None;
        }
        Some((rule.get("modelId")?.as_str()?.to_string(), rule.clone()))
    }).collect::<std::collections::HashMap<_, _>>();
    let first_manual_index = manual_rules.iter().position(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID)
    }).unwrap_or(manual_rules.len());
    let rules = model_rules
        .entry("providerModelRules")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("ZCode providerModelRules must be an array")?;
    let existing_rules = rules.iter().filter_map(|rule| {
        if rule.get("providerId").and_then(serde_json::Value::as_str)
            != Some(MANAGED_AGENT_PROVIDER_ID) {
            return None;
        }
        Some((rule.get("modelId")?.as_str()?.to_string(), rule.clone()))
    }).collect::<std::collections::HashMap<_, _>>();
    let first_managed_index = rules.iter().position(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID)
    }).unwrap_or(rules.len());
    rules.retain(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            != Some(MANAGED_AGENT_PROVIDER_ID)
    });
    let mut manual_updates = Vec::new();
    for option in ordered {
        let manual = existing_manual_rules.get(&option.name);
        if option.context_window.is_none()
            && manual.is_none()
            && !existing_rules.contains_key(&option.name)
        {
            continue;
        }
        let mut rule = manual.or_else(|| existing_rules.get(&option.name))
            .cloned().unwrap_or_else(|| serde_json::json!({}));
        let entry = rule.as_object_mut().ok_or("ZCode model rule must be an object")?;
        entry.insert("providerId".to_string(), serde_json::json!(MANAGED_AGENT_PROVIDER_ID));
        entry.insert("modelId".to_string(), serde_json::json!(option.name));
        let properties = ensure_json_object_entry(ensure_json_object_entry(entry, "config"), "properties");
        if let Some(context_window) = option.context_window {
            properties.insert("contextWindow".to_string(), serde_json::json!(context_window));
        } else if manual.is_none() {
            // Smart rules inherit ZCode defaults; manual rules require a complete
            // config, including contextWindow, even when CPA has no runtime limit.
            properties.remove("contextWindow");
        }
        if properties.is_empty() {
            entry.get_mut("config").and_then(serde_json::Value::as_object_mut)
                .expect("ZCode model config was created").remove("properties");
        }
        if manual.is_some() {
            manual_updates.push(rule);
        } else {
            rules.insert((first_managed_index + rules.iter().filter(|entry| {
                entry.get("providerId").and_then(serde_json::Value::as_str)
                    == Some(MANAGED_AGENT_PROVIDER_ID)
            }).count()).min(rules.len()), rule);
        }
    }
    let manual_rules = model_rules.get_mut("manualProviderModelRules")
        .and_then(serde_json::Value::as_array_mut)
        .expect("ZCode manual model rules were checked as an array");
    manual_rules.retain(|rule| {
        rule.get("providerId").and_then(serde_json::Value::as_str)
            != Some(MANAGED_AGENT_PROVIDER_ID)
    });
    for (offset, rule) in manual_updates.into_iter().enumerate() {
        manual_rules.insert((first_manual_index + offset).min(manual_rules.len()), rule);
    }
    if let Some(provider_order) = config.get_mut("providerOrder") {
        let provider_order = provider_order.as_array_mut()
            .ok_or("ZCode providerOrder must be an array")?;
        if !provider_order.iter().any(|value| value.as_str() == Some(MANAGED_AGENT_PROVIDER_ID)) {
            provider_order.push(serde_json::json!(MANAGED_AGENT_PROVIDER_ID));
        }
    }
    let mut selection = serde_json::json!({
        "providerId": MANAGED_AGENT_PROVIDER_ID,
        "modelId": model,
    });
    if let Some(previous) = config.get("defaultModelSelection") {
        if previous.get("providerId").and_then(serde_json::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID)
            && previous.get("modelId").and_then(serde_json::Value::as_str) == Some(model)
        {
            if let Some(options) = previous.get("options") {
                selection["options"] = options.clone();
            }
        }
    }
    config.insert("defaultModelSelection".to_string(), selection);
    render_agent_json(root, "ZCode provider configuration")
}

pub(crate) fn ensure_toml_child_table<'a>(
    parent: &'a mut toml_edit::Table,
    key: &str,
) -> &'a mut toml_edit::Table {
    use toml_edit::{Item, Table};

    if !parent.get(key).is_some_and(Item::is_table) {
        parent.remove(key);
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .expect("inserted TOML table must remain a table")
}

pub(crate) fn managed_model_alias(model: &str) -> String {
    format!("{MANAGED_AGENT_PROVIDER_ID}/{model}")
}


pub(crate) fn build_omp_models_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
) -> Result<String, String> {
    render_agent_yaml_mapping_update(existing, "Oh My Pi models.yml", |root| {
        let providers = root
            .entry(yaml_key("providers"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()));
        let providers = providers
            .as_mapping_mut()
            .ok_or_else(|| "Oh My Pi providers must be a YAML mapping".to_string())?;
        let mut provider = serde_norway::Mapping::new();
        provider.insert(yaml_key("baseUrl"), serde_norway::Value::String(base_url.to_string()));
        provider.insert(
            yaml_key("apiKey"),
            serde_norway::Value::String(api_key.to_string()),
        );
        provider.insert(
            yaml_key("api"),
            serde_norway::Value::String(OMP_AGENT_API.to_string()),
        );
        let mut discovery = serde_norway::Mapping::new();
        discovery.insert(
            yaml_key("type"),
            serde_norway::Value::String("openai-models-list".to_string()),
        );
        provider.insert(yaml_key("discovery"), serde_norway::Value::Mapping(discovery));
        providers.insert(
            yaml_key(OMP_AGENT_PROVIDER_ID),
            serde_norway::Value::Mapping(provider),
        );
        Ok(())
    })
}

pub(crate) fn build_omp_settings_config(
    existing: Option<&str>,
    model: &str,
) -> Result<String, String> {
    render_agent_yaml_mapping_update(existing, "Oh My Pi config.yml", |root| {
        let roles = root
            .entry(yaml_key("modelRoles"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()));
        let roles = roles
            .as_mapping_mut()
            .ok_or_else(|| "Oh My Pi modelRoles must be a YAML mapping".to_string())?;
        roles.insert(
            yaml_key("default"),
            serde_norway::Value::String(format!("{OMP_AGENT_PROVIDER_ID}/{model}")),
        );
        Ok(())
    })
}

pub(crate) fn prepare_omp_managed_removal(paths: &[PathBuf]) -> Result<Images, String> {
    if paths.len() != 2 {
        return Err("Invalid number of Oh My Pi configuration paths".to_string());
    }
    let mut updates = Vec::new();
    if paths[0].is_file() {
        let content = fs::read_to_string(&paths[0]).map_err(|error| {
            format!("Failed to read Oh My Pi models.yml {}: {error}", path_to_string(&paths[0]))
        })?;
        let rendered = render_agent_yaml_mapping_update(Some(&content), "Oh My Pi models.yml", |root| {
            let mut remove_providers = false;
            if let Some(providers) = root
                .get_mut(yaml_key("providers"))
                .and_then(serde_norway::Value::as_mapping_mut)
            {
                providers.remove(yaml_key(OMP_AGENT_PROVIDER_ID));
                remove_providers = providers.is_empty();
            }
            if remove_providers {
                root.remove(yaml_key("providers"));
            }
            Ok(())
        })?;
        let root = parse_agent_yaml_mapping(Some(&rendered), "Oh My Pi models.yml")?;
        updates.push((
            paths[0].clone(),
            (!root.is_empty()).then(|| rendered.into_bytes()),
        ));
    }
    if paths[1].is_file() {
        let content = fs::read_to_string(&paths[1]).map_err(|error| {
            format!("Failed to read Oh My Pi config.yml {}: {error}", path_to_string(&paths[1]))
        })?;
        let rendered = render_agent_yaml_mapping_update(Some(&content), "Oh My Pi config.yml", |root| {
            let mut remove_roles = false;
            if let Some(roles) = root
                .get_mut(yaml_key("modelRoles"))
                .and_then(serde_norway::Value::as_mapping_mut)
            {
                if roles
                    .get(yaml_key("default"))
                    .and_then(serde_norway::Value::as_str)
                    .is_some_and(|value| value.starts_with(&format!("{OMP_AGENT_PROVIDER_ID}/")))
                {
                    roles.remove(yaml_key("default"));
                }
                remove_roles = roles.is_empty();
            }
            if remove_roles {
                root.remove(yaml_key("modelRoles"));
            }
            Ok(())
        })?;
        let root = parse_agent_yaml_mapping(Some(&rendered), "Oh My Pi config.yml")?;
        updates.push((
            paths[1].clone(),
            (!root.is_empty()).then(|| rendered.into_bytes()),
        ));
    }
    Ok(updates.into_iter().collect())
}

pub(crate) fn build_restored_omp_models(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let original_root = parse_agent_yaml_mapping(original, "Original Oh My Pi models.yml")?;
    let rendered = render_agent_yaml_mapping_update(Some(current), "Current Oh My Pi models.yml", |root| {
        let original_provider = yaml_mapping_value(&original_root, "providers")
            .and_then(serde_norway::Value::as_mapping)
            .and_then(|providers| yaml_mapping_value(providers, OMP_AGENT_PROVIDER_ID))
            .cloned();
        let providers = root
            .entry(yaml_key("providers"))
            .or_insert_with(|| serde_norway::Value::Mapping(serde_norway::Mapping::new()));
        let providers = providers
            .as_mapping_mut()
            .ok_or_else(|| "Oh My Pi providers must be a YAML mapping".to_string())?;
        if let Some(provider) = original_provider {
            providers.insert(yaml_key(OMP_AGENT_PROVIDER_ID), provider);
        } else {
            providers.remove(yaml_key(OMP_AGENT_PROVIDER_ID));
        }
        if providers.is_empty()
            && yaml_mapping_value(&original_root, "providers").is_none()
        {
            root.remove(yaml_key("providers"));
        }
        Ok(())
    })?;
    let root = parse_agent_yaml_mapping(Some(&rendered), "Restored Oh My Pi models.yml")?;
    Ok((!root.is_empty() || original.is_some()).then_some(rendered))
}

pub(crate) fn build_restored_omp_settings(
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    let original_root = parse_agent_yaml_mapping(original, "Original Oh My Pi config.yml")?;
    let rendered = render_agent_yaml_mapping_update(Some(current), "Current Oh My Pi config.yml", |root| {
        restore_yaml_key(root, Some(&original_root), "modelRoles");
        Ok(())
    })?;
    let root = parse_agent_yaml_mapping(Some(&rendered), "Restored Oh My Pi config.yml")?;
    Ok((!root.is_empty() || original.is_some()).then_some(rendered))
}

pub(crate) fn build_kimi_code_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    use toml_edit::{value, Array, Item, Table};

    let mut document = parse_codex_document(existing, "Kimi Code config.toml")?;
    set_codex_table_item(
        document.as_table_mut(),
        "default_model",
        value(managed_model_alias(model)),
    );

    let providers = ensure_toml_child_table(document.as_table_mut(), "providers");
    let provider = ensure_toml_child_table(providers, MANAGED_AGENT_PROVIDER_ID);
    set_codex_table_item(provider, "type", value("openai"));
    set_codex_table_item(provider, "base_url", value(base_url));
    set_codex_table_item(provider, "api_key", value(api_key));

    let models = ensure_toml_child_table(document.as_table_mut(), "models");
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let stale = models
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| key.starts_with(&prefix))
        .collect::<Vec<_>>();
    for key in stale {
        models.remove(&key);
    }
    for option in ordered_agent_models(available_models, model) {
        let mut entry = Table::new();
        entry.insert("provider", value(MANAGED_AGENT_PROVIDER_ID));
        entry.insert("model", value(option.name.clone()));
        entry.insert(
            "display_name",
            value(option.alias.as_deref().unwrap_or(&option.name)),
        );
        let context = option
            .context_window
            .unwrap_or(200_000)
            .min(i64::MAX as u64) as i64;
        entry.insert("max_context_size", value(context));
        let mut capabilities = Array::new();
        capabilities.push("tool_use");
        entry.insert("capabilities", value(capabilities));
        models.insert(&managed_model_alias(&option.name), Item::Table(entry));
    }
    let rendered = document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate Kimi Code configuration: {error}"))?;
    Ok(rendered)
}

pub(crate) fn build_grok_build_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    use toml_edit::{value, Item, Table};

    let mut document = parse_codex_document(existing, "Grok Build config.toml")?;
    let defaults = ensure_toml_child_table(document.as_table_mut(), "models");
    set_codex_table_item(defaults, "default", value(managed_model_alias(model)));

    let models = ensure_toml_child_table(document.as_table_mut(), "model");
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let stale = models
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| key.starts_with(&prefix))
        .collect::<Vec<_>>();
    for key in stale {
        models.remove(&key);
    }
    for option in ordered_agent_models(available_models, model) {
        let mut entry = Table::new();
        entry.insert("model", value(option.name.clone()));
        entry.insert("base_url", value(base_url));
        entry.insert(
            "name",
            value(option.alias.as_deref().unwrap_or(&option.name)),
        );
        entry.insert("api_key", value(api_key));
        entry.insert("api_backend", value("chat_completions"));
        let context = option
            .context_window
            .unwrap_or(200_000)
            .min(i64::MAX as u64) as i64;
        entry.insert("context_window", value(context));
        models.insert(&managed_model_alias(&option.name), Item::Table(entry));
    }
    let rendered = document.to_string();
    toml::from_str::<toml::Value>(&rendered)
        .map_err(|error| format!("Failed to validate Grok Build configuration: {error}"))?;
    Ok(rendered)
}

pub(crate) fn build_openclaw_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    let mut root = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => json5::from_str::<serde_json::Value>(value)
            .map_err(|error| format!("Invalid OpenClaw openclaw.json format: {error}"))?,
        None => serde_json::json!({}),
    };
    let root = root
        .as_object_mut()
        .ok_or_else(|| "OpenClaw openclaw.json root must be an object".to_string())?;
    let ordered_models = ordered_agent_models(available_models, model);
    let models = ensure_json_object_entry(root, "models");
    models
        .entry("mode".to_string())
        .or_insert_with(|| serde_json::json!("merge"));
    let providers = ensure_json_object_entry(models, "providers");
    let managed_provider = ensure_json_object_entry(providers, MANAGED_AGENT_PROVIDER_ID);
    let existing_model_entries = managed_provider
        .get("models")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    managed_provider.insert("baseUrl".to_string(), serde_json::json!(base_url));
    managed_provider.insert("apiKey".to_string(), serde_json::json!(api_key));
    managed_provider.insert("api".to_string(), serde_json::json!("openai-completions"));
    managed_provider.insert(
        "models".to_string(),
        serde_json::Value::Array(
            ordered_models
                .iter()
                .map(|model| {
                    let mut entry = existing_model_entries
                        .iter()
                        .find(|entry| {
                            entry.get("id").and_then(serde_json::Value::as_str)
                                == Some(model.name.as_str())
                        })
                        .and_then(serde_json::Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    entry.insert("id".to_string(), serde_json::json!(model.name.clone()));
                    entry.insert(
                        "name".to_string(),
                        serde_json::json!(model.alias.as_deref().unwrap_or(&model.name)),
                    );
                    if let Some(context_window) = model.context_window {
                        entry.insert(
                            "contextWindow".to_string(),
                            serde_json::json!(context_window),
                        );
                    }
                    serde_json::Value::Object(entry)
                })
                .collect(),
        ),
    );
    let agents = ensure_json_object_entry(root, "agents");
    let defaults = ensure_json_object_entry(agents, "defaults");
    let default_model = ensure_json_object_entry(defaults, "model");
    default_model.insert(
        "primary".to_string(),
        serde_json::json!(format!("{MANAGED_AGENT_PROVIDER_ID}/{model}")),
    );
    let model_catalog = ensure_json_object_entry(defaults, "models");
    let managed_prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let existing_catalog = model_catalog.clone();
    model_catalog.retain(|name, _| !name.starts_with(&managed_prefix));
    for model in &ordered_models {
        let name = format!("{MANAGED_AGENT_PROVIDER_ID}/{}", model.name);
        let mut entry = existing_catalog
            .get(&name)
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(alias) = model.alias.as_deref() {
            entry.insert("alias".to_string(), serde_json::json!(alias));
        } else {
            entry.remove("alias");
        }
        model_catalog.insert(name, serde_json::Value::Object(entry));
    }
    let rendered = render_agent_json(root.clone(), "OpenClaw configuration")?;
    let comments = existing.map(extract_json5_comments).unwrap_or_default();
    if comments.is_empty() {
        Ok(rendered)
    } else {
        Ok(format!("{}\n{rendered}", comments.join("\n")))
    }
}

pub(crate) fn extract_json5_comments(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            let start = index;
            index += 2;
            while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                index += 1;
            }
            comments.push(content[start..index].to_string());
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let start = index;
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            comments.push(content[start..index].to_string());
            continue;
        }
        index += 1;
    }
    comments
}

pub(crate) fn ensure_yaml_mapping_entry<'a>(
    root: &'a mut serde_norway::Mapping,
    key: &str,
) -> &'a mut serde_norway::Mapping {
    let key_value = serde_norway::Value::String(key.to_string());
    if !root
        .get(&key_value)
        .is_some_and(|value| value.as_mapping().is_some())
    {
        root.insert(
            key_value.clone(),
            serde_norway::Value::Mapping(serde_norway::Mapping::new()),
        );
    }
    root.get_mut(&key_value)
        .and_then(serde_norway::Value::as_mapping_mut)
        .expect("mapping entry was just normalized")
}

pub(crate) fn ensure_yaml_sequence_entry<'a>(
    root: &'a mut serde_norway::Mapping,
    key: &str,
) -> &'a mut Vec<serde_norway::Value> {
    let key_value = serde_norway::Value::String(key.to_string());
    if !root
        .get(&key_value)
        .is_some_and(|value| value.as_sequence().is_some())
    {
        root.insert(key_value.clone(), serde_norway::Value::Sequence(Vec::new()));
    }
    root.get_mut(&key_value)
        .and_then(serde_norway::Value::as_sequence_mut)
        .expect("sequence entry was just normalized")
}

pub(crate) fn build_hermes_agent_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
    model: &str,
    available_models: &[AgentModelOption],
) -> Result<String, String> {
    let original = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_norway::from_str::<serde_norway::Value>(value)
            .map_err(|error| format!("Invalid Hermes config.yaml format: {error}"))?,
        None => serde_norway::Value::Mapping(serde_norway::Mapping::new()),
    };
    let mut root = original.clone();
    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| "Hermes config.yaml root must be a mapping".to_string())?;
    let dictionary_provider = mapping
        .get("providers")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|providers| {
            providers
                .iter()
                .find(|(key, provider)| {
                    key.as_str() == Some(MANAGED_AGENT_PROVIDER_ID)
                        || provider.get("name").and_then(serde_norway::Value::as_str)
                            == Some(MANAGED_AGENT_PROVIDER_ID)
                })
                .and_then(|(_, provider)| provider.as_mapping().cloned())
        });
    let providers = ensure_yaml_sequence_entry(mapping, "custom_providers");
    let mut managed_provider = None;
    let mut retained_providers = Vec::with_capacity(providers.len());
    for provider in std::mem::take(providers) {
        match provider.get("name").and_then(serde_norway::Value::as_str) {
            Some(name) if name == MANAGED_AGENT_PROVIDER_ID => {
                if managed_provider.is_none() {
                    managed_provider = provider.as_mapping().cloned();
                }
            }
            Some(_) => retained_providers.push(provider),
            None => {}
        }
    }
    let mut managed_provider = managed_provider.or(dictionary_provider);
    if let Some(provider) = managed_provider.as_mut() {
        for (legacy_key, current_key) in [
            ("baseUrl", "base_url"),
            ("apiKey", "api_key"),
            ("apiMode", "api_mode"),
            ("maxTokens", "max_tokens"),
            ("contextLength", "context_length"),
        ] {
            let legacy_key = serde_norway::Value::String(legacy_key.to_string());
            let current_key = serde_norway::Value::String(current_key.to_string());
            if provider.contains_key(&current_key) {
                provider.remove(&legacy_key);
            } else if let Some(value) = provider.remove(&legacy_key) {
                provider.insert(current_key, value);
            }
        }
    }
    let ordered_models = ordered_agent_models(available_models, model);
    let existing_model_metadata = managed_provider
        .as_ref()
        .and_then(|provider| provider.get("models"))
        .and_then(serde_norway::Value::as_mapping);
    let mut provider_models = serde_norway::Mapping::new();
    for model in &ordered_models {
        let model_key = serde_norway::Value::String(model.name.clone());
        let mut metadata = existing_model_metadata
            .and_then(|models| models.get(&model_key))
            .and_then(serde_norway::Value::as_mapping)
            .cloned()
            .unwrap_or_default();
        if let Some(context_window) = model.context_window {
            let value = serde_norway::to_value(context_window)
                .map_err(|error| format!("Failed to generate Hermes model metadata: {error}"))?;
            metadata.insert(
                serde_norway::Value::String("context_length".to_string()),
                value,
            );
        }
        provider_models.insert(model_key, serde_norway::Value::Mapping(metadata));
    }
    let mut canonical_provider = serde_norway::to_value(serde_json::json!({
        "name": MANAGED_AGENT_PROVIDER_ID,
        "base_url": base_url,
        "api_key": api_key,
        "api_mode": "chat_completions",
        "model": model,
        "models": {}
    }))
    .map_err(|error| format!("Failed to generate Hermes provider: {error}"))?;
    canonical_provider
        .as_mapping_mut()
        .ok_or_else(|| "Failed to generate Hermes provider: root is not a mapping".to_string())?
        .insert(
            serde_norway::Value::String("models".to_string()),
            serde_norway::Value::Mapping(provider_models),
        );
    let canonical_provider = canonical_provider
        .as_mapping()
        .ok_or_else(|| "Failed to generate Hermes provider: root is not a mapping".to_string())?;
    let mut managed_provider = managed_provider.unwrap_or_default();
    for (key, value) in canonical_provider {
        managed_provider.insert(key.clone(), value.clone());
    }
    retained_providers.push(serde_norway::Value::Mapping(managed_provider));
    *providers = retained_providers;
    let model_config = ensure_yaml_mapping_entry(mapping, "model");
    model_config.insert(
        serde_norway::Value::String("default".to_string()),
        serde_norway::Value::String(model.to_string()),
    );
    model_config.insert(
        serde_norway::Value::String("provider".to_string()),
        serde_norway::Value::String(MANAGED_AGENT_PROVIDER_ID.to_string()),
    );
    let rendered = if let Some(existing) = existing.filter(|value| !value.trim().is_empty()) {
        render_yaml_value_changes(existing, &original, &root)
            .or_else(|_| serde_norway::to_string(&root).map_err(|_| "Failed to generate Hermes configuration".to_string()))?
    } else {
        serde_norway::to_string(&root).map_err(|error| format!("Failed to generate Hermes configuration: {error}"))?
    };
    serde_norway::from_str::<serde_norway::Value>(&rendered)
        .map_err(|error| format!("Failed to validate Hermes configuration: {error}"))?;
    Ok(rendered)
}
