use super::*;

struct CoreRestore {
    before: String,
    after: String,
}

pub(crate) struct RestorePlan {
    paths: Vec<PathBuf>,
    pub(crate) preview: BackupPreview,
    local_revision: String,
    before: Images,
    after: Images,
    version: BackupVersion,
    core: Option<CoreRestore>,
}

fn local_restore_plan(client: &str, home: &Path, id: &str) -> Result<RestorePlan, String> {
    let _guard = AGENT_CONFIG_FILE_LOCK
        .lock()
        .map_err(|_| "Configuration file lock is poisoned")?;
    let paths = config_paths(client, home)?;
    let (preview, before, after) = preview(client, &paths, id)?;
    let version = read_version(client, &paths, id)?;
    Ok(RestorePlan {
        paths,
        local_revision: preview.revision.clone(),
        preview,
        before,
        after,
        version,
        core: None,
    })
}

fn desktop_restore_configuration(
    plan: &RestorePlan,
) -> Result<Option<(ClaudeDesktopModelMappings, Vec<AgentModelOption>)>, String> {
    if plan.version.client != "claude-desktop" {
        return Ok(None);
    }
    if !desktop_profile_needs_mapping(&plan.after)?
        && plan.version.mappings.as_ref()
            .and_then(|m| m.desktop_models.as_ref()).is_none()
    {
        return Ok(None);
    }
    let (path, bytes) = &plan.after[2];
    let profile = parse(path, text(bytes.as_deref())?)?;
    let names = profile
        .get("inferenceModels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let mut mappings = plan.version.mappings.as_ref().ok_or(
            "This backup version lacks Claude Desktop model mappings and cannot safely restore kernel routing. Reconfigure the models",
        )?.clone();
    if mappings.desktop_models.is_none() {
        let roles = [
            (CLAUDE_DESKTOP_OPUS_MODEL_ID, LEGACY_CLAUDE_DESKTOP_MODEL_IDS[0], &mappings.opus, mappings.opus_1m),
            (CLAUDE_DESKTOP_SONNET_MODEL_ID, LEGACY_CLAUDE_DESKTOP_MODEL_IDS[1], &mappings.sonnet, mappings.sonnet_1m),
            (CLAUDE_DESKTOP_HAIKU_MODEL_ID, LEGACY_CLAUDE_DESKTOP_MODEL_IDS[2], &mappings.haiku, mappings.haiku_1m),
        ];
        let mut entries = Vec::<ClaudeDesktopModelMapping>::new();
        for id in names {
            let Some((_, _, source, context_1m)) = roles.iter()
                .find(|(_, _, source, _)| source.eq_ignore_ascii_case(id))
                .or_else(|| roles.iter().find(|(current, legacy, _, _)| {
                    current.eq_ignore_ascii_case(id) || legacy.eq_ignore_ascii_case(id)
                }))
            else {
                return Err("A model ID in this backup has no corresponding source, so kernel routing cannot be safely restored".into());
            };
            if let Some(existing) = entries.iter_mut()
                .find(|entry| entry.model_id().eq_ignore_ascii_case(id))
            {
                existing.context_1m |= *context_1m;
            } else {
                entries.push(ClaudeDesktopModelMapping {
                    model: source.to_string(),
                    alias: if source.eq_ignore_ascii_case(id) {
                        String::new()
                    } else {
                        id.into()
                    },
                    context_1m: *context_1m,
                });
            }
        }
        mappings.desktop_models = Some(entries);
    }
    let entries = mappings.desktop_models.as_ref().ok_or("Backup model mapping is missing")?;
    validate_claude_desktop_entries(entries)?;
    let models = entries.iter()
        .map(|entry| AgentModelOption {
            name: entry.source_or_alias().to_string(),
            alias: None,
            is_alias: false,
            context_window: None,
            input_modalities: None,
            harness_metadata: None,
            catalog_metadata: None,
        })
        .collect();
    Ok(Some((mappings, models)))
}

fn attach_core_restore(
    plan: &mut RestorePlan,
    before: String,
    after: String,
) -> Result<(), String> {
    let original =
        serde_norway::from_str::<serde_norway::Value>(&before).map_err(|e| e.to_string())?;
    let target =
        serde_norway::from_str::<serde_norway::Value>(&after).map_err(|e| e.to_string())?;
    let mut routes = vec![
        CLAUDE_DESKTOP_OPUS_MODEL_ID,
        CLAUDE_DESKTOP_SONNET_MODEL_ID,
        CLAUDE_DESKTOP_HAIKU_MODEL_ID,
        LEGACY_CLAUDE_DESKTOP_MODEL_IDS[0],
        LEGACY_CLAUDE_DESKTOP_MODEL_IDS[1],
        LEGACY_CLAUDE_DESKTOP_MODEL_IDS[2],
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    for root in [&original, &target]
        .into_iter()
        .filter_map(serde_norway::Value::as_mapping)
    {
        for alias in managed_claude_desktop_aliases(root) {
            if !routes.contains(&alias) {
                routes.push(alias);
            }
        }
    }
    for route in routes {
        let source = |root: &serde_norway::Value| {
            root.as_mapping()
                .and_then(|root| configured_model_client_identity(root, &route))
                .map(|(source, _)| source)
        };
        let old = source(&original);
        let new = source(&target);
        if old != new {
            plan.preview.differences.push(BackupDifference {
                file: "CPA/config.yaml".into(),
                field: format!("modelMappings.{route}"),
                before: old.unwrap_or_else(|| "—".into()),
                after: new.unwrap_or_else(|| "—".into()),
            });
        }
    }
    plan.preview.revision = sha256_bytes(
        format!(
            "{}:{}:{}",
            plan.local_revision,
            model_alias_config_revision(&before)?,
            model_alias_config_revision(&after)?
        )
        .as_bytes(),
    );
    plan.core = Some(CoreRestore { before, after });
    Ok(())
}

pub(crate) async fn prepare_restore_plan(
    config: &GuiConfigFile,
    client: &str,
    home: &Path,
    id: &str,
) -> Result<RestorePlan, String> {
    let mut plan = local_restore_plan(client, home, id)?;
    if let Some((mappings, models)) = desktop_restore_configuration(&plan)? {
        let before = fetch_management_config_yaml(config)
            .await
            .map_err(agent_core_error)?;
        let after = match ensure_claude_desktop_model_aliases_with_oauth_definitions_in_yaml(
            &before,
            &mappings,
            &models,
            &[],
        ) {
            Ok(after) => after,
            Err(_) => {
                let definitions = fetch_oauth_model_definitions(config).await;
                ensure_claude_desktop_model_aliases_with_oauth_definitions_in_yaml(
                    &before,
                    &mappings,
                    &models,
                    &definitions,
                )
                .map_err(agent_core_error)?
            }
        };
        attach_core_restore(&mut plan, before, after).map_err(agent_core_error)?;
    }
    Ok(plan)
}

pub(crate) async fn execute_restore_plan(
    config: &GuiConfigFile,
    plan: RestorePlan,
    revision: &str,
) -> Result<AgentConfigActionResult, String> {
    if plan.preview.revision != revision {
        return Err("Configuration changed after preview. Select the backup version again".into());
    }
    let commit = || {
        let _guard = AGENT_CONFIG_FILE_LOCK
            .lock()
            .map_err(|_| "Configuration file lock is poisoned")?;
        let latest = preview(&plan.version.client, &plan.paths, &plan.version.id)?.0;
        let version = read_version(&plan.version.client, &plan.paths, &plan.version.id)?;
        if latest.revision != plan.local_revision
            || serde_json::to_vec(&version).map_err(|e| e.to_string())?
                != serde_json::to_vec(&plan.version).map_err(|e| e.to_string())?
        {
            return Err("Configuration or backup changed during restoration. Select the backup version again".into());
        }
        commit_config_with_mappings(
            &plan.version.client,
            &plan.paths,
            &plan.before,
            &plan.after,
            "restore",
            plan.version.mappings.as_ref().map(|m| m.sonnet.clone()),
            plan.version.mappings.clone(),
        )
    };
    match &plan.core {
        Some(core) => {
            commit_management_alias_config_changes(config, &core.before, &core.after, commit)
                .await
                .map_err(agent_core_error)
        }
        None => commit(),
    }
}

#[tauri::command]
pub(crate) async fn preview_agent_config_backup(
    app: tauri::AppHandle,
    client: String,
    id: String,
) -> Result<BackupPreview, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let config = app.state::<GuiConfigState>().snapshot()?;
    Ok(prepare_restore_plan(&config, &client, &home, &id)
        .await?
        .preview)
}

#[tauri::command]
pub(crate) async fn restore_agent_config_backup(
    app: tauri::AppHandle,
    client: String,
    id: String,
    revision: String,
) -> Result<AgentConfigActionResult, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let config = app.state::<GuiConfigState>().snapshot()?;
    let plan = prepare_restore_plan(&config, &client, &home, &id).await?;
    let result = execute_restore_plan(&config, plan, &revision).await?;
    app.state::<AgentConfigStatusCache>().clear()?;
    Ok(result)
}

pub(super) fn desktop_profile_needs_mapping(images: &Images) -> Result<bool, String> {
    let (path, bytes) = &images[2];
    let profile = parse(path, text(bytes.as_deref())?)?;
    let names = profile
        .get("inferenceModels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let labeled_mapping = profile.get("inferenceModels").and_then(Value::as_array)
        .into_iter().flatten().any(|entry| {
            let name = entry.get("name").and_then(Value::as_str);
            let label = entry.get("labelOverride").and_then(Value::as_str);
            matches!((name, label), (Some(name), Some(label)) if name != label)
        });
    if !labeled_mapping && !names.iter().any(|name| is_claude_desktop_route_id(name)) {
        return Ok(false);
    }
    Ok(true)
}
