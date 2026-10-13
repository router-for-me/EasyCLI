use super::*;
use serde_json::Value;

#[cfg(test)]
mod tests;

pub(crate) type Images = Vec<(PathBuf, Option<Vec<u8>>)>;

const AGENT_INTEGRATION_STATE_VERSION: u8 = 1;
const AGENT_INTEGRATION_STATE_FILE: &str = "integration-state.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentIntegrationState {
    version: u8,
    client: String,
    paths: Vec<PathBuf>,
    // `original` is promoted with user edits; `applied` remains the last CPA-written snapshot.
    original: Images,
    applied: Images,
}

fn integration_managed_paths<'a>(client: &str, paths: &'a [PathBuf]) -> &'a [PathBuf] {
    if client == "deepseek-harness" && paths.len() > 2 {
        &paths[..2]
    } else {
        paths
    }
}

fn integration_state_path(client: &str, paths: &[PathBuf]) -> Result<PathBuf, String> {
    let paths = integration_managed_paths(client, paths);
    let identity = sha256_bytes(
        &serde_json::to_vec(paths).map_err(|_| "Failed to generate access configuration marker".to_string())?,
    );
    Ok(agent_data_directory(paths)?
        .join("agents")
        .join(client)
        .join(identity)
        .join(AGENT_INTEGRATION_STATE_FILE))
}

fn parse_integration_state(
    client: &str,
    paths: &[PathBuf],
    bytes: Option<&[u8]>,
) -> Result<Option<AgentIntegrationState>, String> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let state: AgentIntegrationState = serde_json::from_slice(bytes)
        .map_err(|_| "Access configuration merge state is corrupted. Back up the configuration manually first".to_string())?;
    let paths = integration_managed_paths(client, paths);
    let valid_images = |images: &Images| {
        images.len() == paths.len()
            && images
                .iter()
                .zip(paths)
                .all(|((path, _), expected)| path == expected)
    };
    if state.version != AGENT_INTEGRATION_STATE_VERSION
        || state.client != client
        || state.paths != paths
        || !valid_images(&state.original)
        || !valid_images(&state.applied)
    {
        return Err("Access configuration merge state does not match the current client. Back up the configuration manually first".into());
    }
    Ok(Some(state))
}

fn integration_state_bytes(state: &AgentIntegrationState) -> Result<Vec<u8>, String> {
    serde_json::to_vec(state).map_err(|_| "Failed to generate access configuration merge state".to_string())
}

fn merge_user_value(
    baseline: Option<&Value>,
    applied: Option<&Value>,
    current: Option<&Value>,
) -> Option<Value> {
    // Three-way merge: unchanged CPA values return to the baseline, while external edits win.
    if current == applied {
        return baseline.cloned();
    }
    match (
        applied.and_then(Value::as_object),
        current.and_then(Value::as_object),
    ) {
        (Some(applied), Some(current)) => {
            let baseline = baseline.and_then(Value::as_object);
            let mut keys = applied
                .keys()
                .chain(current.keys())
                .chain(baseline.into_iter().flat_map(|value| value.keys()))
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            let mut merged = serde_json::Map::new();
            for key in keys {
                if let Some(value) = merge_user_value(
                    baseline.and_then(|value| value.get(&key)),
                    applied.get(&key),
                    current.get(&key),
                ) {
                    merged.insert(key, value);
                }
            }
            Some(Value::Object(merged))
        }
        _ => current.cloned(),
    }
}

fn merge_user_image(
    client: &str,
    path: &Path,
    baseline: Option<&[u8]>,
    applied: Option<&[u8]>,
    current: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, String> {
    if current == applied {
        return Ok(baseline.map(ToOwned::to_owned));
    }
    let Some(current) = current else {
        return Ok(None);
    };
    let current_text = std::str::from_utf8(current)
        .map_err(|_| format!("Configuration is not UTF-8 text: {}", path_to_string(path)))?;
    let applied_value = parse(
        path,
        applied
            .map(|bytes| std::str::from_utf8(bytes).map_err(|_| "Access state is not UTF-8 text"))
            .transpose()?,
    )?;
    let current_value = parse(path, Some(current_text))?;
    let baseline_value = parse(
        path,
        baseline
            .map(|bytes| std::str::from_utf8(bytes).map_err(|_| "Original state is not UTF-8 text"))
            .transpose()?,
    )?;
    let mut merged = merge_user_value(
        baseline.map(|_| &baseline_value),
        applied.map(|_| &applied_value),
        Some(&current_value),
    );
    if client == "workbuddy" {
        if let Some(merged) = &mut merged {
            merge_workbuddy_visibility(&baseline_value, &applied_value, &current_value, merged);
        }
    }
    merged
        .map(|value| render(path, &value).map(|value| value.into_bytes()))
        .transpose()
}

fn promote_user_changes(
    client: &str,
    paths: &[PathBuf],
    baseline: &Images,
    applied: &Images,
    current: &Images,
) -> Result<Images, String> {
    paths
        .iter()
        .zip(baseline)
        .zip(applied)
        .zip(current)
        .map(|(((path, (_, baseline)), (_, applied)), (_, current))| {
            Ok((
                path.clone(),
                merge_user_image(
                    client,
                    path,
                    baseline.as_deref(),
                    applied.as_deref(),
                    current.as_deref(),
                )?,
            ))
        })
        .collect()
}

fn prepare_integration_restore(
    client: AgentClient,
    paths: &[PathBuf],
    current: &Images,
    state: &AgentIntegrationState,
) -> Result<Images, String> {
    let effective_original = promote_user_changes(
        client.id(),
        paths,
        &state.original,
        &state.applied,
        current,
    )?;
    current
        .iter()
        .zip(&effective_original)
        .map(|((path, current), (_, original))| {
            Ok((
                path.clone(),
                build_agent_session_restored_bytes_with_preference(
                    client,
                    paths,
                    path,
                    current.as_deref(),
                    original.as_deref(),
                    false,
                )?,
            ))
        })
        .collect()
}

pub(crate) fn prepare_recorded_integration_restore(
    client: AgentClient,
    paths: &[PathBuf],
    current: &Images,
) -> Result<Option<Images>, String> {
    let state_path = integration_state_path(client.id(), paths)?;
    let state = parse_integration_state(
        client.id(),
        paths,
        read_agent_bytes(&state_path)?.as_deref(),
    )?;
    state
        .as_ref()
        .map(|state| prepare_integration_restore(client, paths, current, state))
        .transpose()
}

pub(crate) fn config_paths(client: &str, home: &Path) -> Result<Vec<PathBuf>, String> {
    if client == PI_AGENT_ID {
        return Ok(vec![
            pi_provider_config_path(home),
            pi_provider_settings_path(home),
        ]);
    }
    let client = AgentClient::parse(client)?;
    if !client.supported_platform() {
        return Err("The current platform does not support this agent configuration".into());
    }
    Ok(expected_agent_record_paths(
        client,
        &agent_config_paths(client, home),
    ))
}

pub(crate) fn validate_config_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Configuration path must be absolute and remain within bounds".into());
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                if path_has_reparse_point(ancestor, &meta)
                    && !relocated_configuration_root_allowed(ancestor)
                {
                    return Err(format!(
                        "Configuration path cannot contain symbolic links: {}",
                        path_to_string(ancestor)
                    ));
                }
                if ancestor == path && !meta.is_file() {
                    return Err("Configuration path is not a regular file".into());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("Unable to check configuration path".into()),
        }
    }
    Ok(())
}

fn path_has_reparse_point(path: &Path, meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let _ = path;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// A configuration root may itself be relocated with a symlink or junction.
/// Deeper links are still rejected so a file inside that root cannot escape it.
fn relocated_configuration_root_allowed(path: &Path) -> bool {
    let Ok(canonical_path) = normalize_existing_path(path) else {
        return false;
    };
    let Some(home) = user_home_directory() else {
        return false;
    };
    let Ok(canonical_home) = normalize_existing_path(&home) else {
        return false;
    };
    if canonical_path == canonical_home {
        return true;
    }

    configuration_root_candidates(&home)
        .into_iter()
        .any(|root| paths_equivalent(&root, path) || paths_equivalent(&root, &canonical_path))
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (normalize_existing_path(left), normalize_existing_path(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn normalize_existing_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|_| "Unable to resolve configuration path".into())
}

fn user_home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let name = "USERPROFILE";
    #[cfg(not(windows))]
    let name = "HOME";
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn configuration_root_candidates(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".claude"),
        home.join(".codex"),
        home.join(".config").join("opencode"),
        home.join(".openclaw"),
        home.join(".hermes"),
        home.join(".dsh"),
        home.join(".workbuddy"),
        home.join(".workbuddy-ai"),
        home.join(".gemini"),
        home.join(".gemini").join("antigravity-cli"),
        home.join(".zcode"),
        home.join(".zcode").join("v2"),
        home.join(".kimi-code"),
        home.join(".grok"),
        home.join(".omp"),
        home.join(".omp").join("agent"),
        home.join(".pi"),
        home.join(".pi").join("agent"),
        home.join(".minimax"),
    ];
    #[cfg(target_os = "macos")]
    {
        let support = home.join("Library/Application Support");
        roots.push(support.join("Claude"));
        roots.push(support.join("Claude-3p"));
    }
    #[cfg(target_os = "windows")]
    {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join("AppData/Local"));
        roots.push(local.join("Claude"));
        roots.push(local.join("Claude-3p"));
        roots.push(local.join("hermes"));
    }
    #[cfg(target_os = "linux")]
    {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        roots.push(config_home.join("Claude"));
        roots.push(config_home.join("Claude-3p"));
    }
    for name in [
        "CODEX_HOME",
        "OPENCODE_CONFIG",
        "XDG_CONFIG_HOME",
        "OPENCLAW_CONFIG_PATH",
        "OPENCLAW_STATE_DIR",
        "OPENCLAW_HOME",
        "KIMI_CODE_HOME",
        "GROK_HOME",
        "PI_CODING_AGENT_DIR",
        "DSH_HOME",
        "HERMES_HOME",
        "WORKBUDDY_CONFIG_DIR",
        "CODEBUDDY_CONFIG_DIR",
        "MINIMAX_DATA_DIR",
    ] {
        if let Some(path) = env::var_os(name).map(PathBuf::from) {
            if path.is_absolute() {
                roots.push(path);
            }
        }
    }
    roots
}

pub(crate) fn config_images(paths: &[PathBuf]) -> Result<Images, String> {
    paths
        .iter()
        .map(|path| Ok((path.clone(), read_agent_bytes(path)?)))
        .collect()
}

pub(crate) fn image_hash(bytes: Option<&[u8]>) -> String {
    bytes.map(sha256_bytes).unwrap_or_else(|| "missing".into())
}

pub(crate) fn image_revision(images: &Images) -> String {
    sha256_bytes(
        &serde_json::to_vec(
            &images
                .iter()
                .map(|(path, bytes)| (path, image_hash(bytes.as_deref())))
                .collect::<Vec<_>>(),
        )
        .expect("image hashes serialize"),
    )
}

pub(crate) fn write_config_images(client: &str, images: &Images) -> Result<(), String> {
    for (path, bytes) in images {
        if read_agent_bytes(path)? == *bytes {
            continue;
        }
        if let Some(bytes) = bytes {
            if path.file_name().and_then(|name| name.to_str())
                == Some(AGENT_INTEGRATION_STATE_FILE)
            {
                write_codex_private_file(path, bytes)?;
            } else if client == PI_AGENT_ID {
                write_bytes_directly(path, bytes)?;
            } else {
                write_agent_configuration_file(AgentClient::parse(client)?, path, bytes)?;
            }
        } else {
            fs::remove_file(path).map_err(|_| "Failed to delete configuration file".to_string())?;
        }
        if read_agent_bytes(path)? != *bytes {
            return Err("Configuration post-write verification failed".into());
        }
    }
    Ok(())
}

pub(crate) fn commit_config(
    client: &str,
    paths: &[PathBuf],
    before: &Images,
    after: &Images,
    _source: &str,
    model: Option<String>,
) -> Result<AgentConfigActionResult, String> {
    commit_config_with_mappings(client, paths, before, after, _source, model, None)
}

pub(crate) fn commit_config_with_mappings(
    client: &str,
    paths: &[PathBuf],
    before: &Images,
    after: &Images,
    source: &str,
    model: Option<String>,
    mappings: Option<ClaudeDesktopModelMappings>,
) -> Result<AgentConfigActionResult, String> {
    commit_config_transaction(
        client,
        paths,
        before,
        after,
        source,
        model,
        mappings,
        &mut write_config_images,
        true,
    )
}

#[cfg(test)]
pub(crate) fn commit_config_with_writer(
    client: &str,
    paths: &[PathBuf],
    before: &Images,
    after: &Images,
    source: &str,
    model: Option<String>,
    mappings: Option<ClaudeDesktopModelMappings>,
    writer: &mut impl FnMut(&str, &Images) -> Result<(), String>,
) -> Result<AgentConfigActionResult, String> {
    commit_config_transaction(
        client, paths, before, after, source, model, mappings, writer, false,
    )
}

fn commit_config_transaction(
    client: &str,
    paths: &[PathBuf],
    before: &Images,
    after: &Images,
    source: &str,
    model: Option<String>,
    mappings: Option<ClaudeDesktopModelMappings>,
    writer: &mut impl FnMut(&str, &Images) -> Result<(), String>,
    checked: bool,
) -> Result<AgentConfigActionResult, String> {
    if before.len() != paths.len()
        || after.len() != paths.len()
        || before
            .iter()
            .zip(paths)
            .any(|((p, _), expected)| p != expected)
        || after
            .iter()
            .zip(paths)
            .any(|((p, _), expected)| p != expected)
    {
        return Err("Configuration update path mismatch".into());
    }
    if config_images(paths)? != *before {
        return Err("Configuration was modified by another program. Refresh and try again".into());
    }
    let mut previous = before.clone();
    let mut target = after.clone();
    if client != PI_AGENT_ID && client != AgentClient::Codex.id() {
        let managed_paths = integration_managed_paths(client, paths);
        let managed_len = managed_paths.len();
        let state_path = integration_state_path(client, paths)?;
        let state_before = read_agent_bytes(&state_path)?;
        let current_state = parse_integration_state(client, paths, state_before.as_deref())?;
        let state_after = match source {
            "update" => {
                let parsed_client = AgentClient::parse(client)?;
                let next = if let Some(state) = current_state {
                    let original = promote_user_changes(
                        client,
                        managed_paths,
                        &state.original,
                        &state.applied,
                        &before[..managed_len].to_vec(),
                    )?;
                    Some(AgentIntegrationState {
                        version: AGENT_INTEGRATION_STATE_VERSION,
                        client: client.to_string(),
                        paths: managed_paths.to_vec(),
                        original,
                        applied: after[..managed_len].to_vec(),
                    })
                } else if before[..managed_len] != after[..managed_len]
                    && !agent_has_managed_marker(parsed_client, managed_paths)?
                {
                    Some(AgentIntegrationState {
                        version: AGENT_INTEGRATION_STATE_VERSION,
                        client: client.to_string(),
                        paths: managed_paths.to_vec(),
                        original: before[..managed_len].to_vec(),
                        applied: after[..managed_len].to_vec(),
                    })
                } else {
                    None
                };
                next.as_ref().map(integration_state_bytes).transpose()?
            }
            "clear-integration" | "restore" | "template" => None,
            _ => state_before.clone(),
        };
        previous.push((state_path.clone(), state_before));
        target.push((state_path, state_after));
    }
    if source == "clear-integration" {
        let state_path = agent_state_path(paths)?;
        previous.push((state_path.clone(), read_agent_bytes(&state_path)?));
        target.push((state_path, None));
    }
    if client == "deepseek-harness" && matches!(source, "template" | "restore" | "clear-integration") {
        let state_path = deepseek_harness_catalog_state_path(paths)?;
        let state_before = read_agent_bytes(&state_path)?;
        let state_after = if source == "template" {
            Some(deepseek_harness_template_catalog_state(after)?)
        } else {
            None
        };
        previous.push((state_path.clone(), state_before));
        target.push((state_path, state_after));
    }
    if client == "claude-desktop" {
        let state_path = desktop_mapping_path(paths)?;
        let state_before = read_agent_bytes(&state_path)?;
        let next = if source == "clear-integration" {
            None
        } else if source == "restore" {
            mappings
        } else {
            mappings.or_else(|| matching_desktop_mappings(paths, after))
        };
        let state_after = desktop_mapping_bytes(after, next.as_ref())?;
        previous.push((state_path.clone(), state_before));
        target.push((state_path, state_after));
    }
    let changed = previous
        .iter()
        .zip(&target)
        .filter(|((_, a), (_, b))| a != b)
        .map(|((p, _), _)| path_to_string(p))
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Ok(action_result(
            "unchanged",
            true,
            model,
            Vec::new(),
            Vec::new(),
        ));
    }
    let all_paths = previous.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
    if config_images(&all_paths)? != previous {
        return Err("Configuration was modified by another program. Refresh and try again".into());
    }
    let mut attempted = Vec::new();
    let result = (if checked {
        (|| {
            for (index, ((path, old), (_, next))) in previous.iter().zip(&target).enumerate() {
                if read_agent_bytes(path)? != *old {
                    return Err("Configuration was modified by another program".into());
                }
                if old == next {
                    continue;
                }
                let written = writer(client, &vec![target[index].clone()]);
                attempted.push((index, read_agent_bytes(path)));
                written?;
            }
            Ok(())
        })()
    } else {
        writer(client, &target)
    })
    .and_then(|_| {
        if config_images(&all_paths)? == target {
            Ok(())
        } else {
            Err("Configuration post-write verification failed".into())
        }
    });
    if result.is_err() {
        let rollback = (if checked {
            let mut failed = false;
            for (index, observed) in attempted.into_iter().rev() {
                let (path, _) = &previous[index];
                if observed.is_err() || read_agent_bytes(path) != observed {
                    failed = true;
                    continue;
                }
                failed |= writer(client, &vec![previous[index].clone()]).is_err();
            }
            if failed {
                Err("Rollback failed".into())
            } else {
                Ok(())
            }
        } else {
            writer(client, &previous)
        })
        .and_then(|_| {
            if config_images(&all_paths)? == previous {
                Ok(())
            } else {
                Err("Rollback verification failed".into())
            }
        });
        return Err(if rollback.is_ok() {
            "Configuration write failed; this change was rolled back"
        } else {
            "Configuration write and rollback both failed. Check the configuration and restore it from a manual backup or repair it with the base configuration template"
        }
        .into());
    }
    Ok(action_result("updated", true, model, changed, Vec::new()))
}

pub(crate) fn validate_config_images(images: &Images) -> Result<(), String> {
    for (path, bytes) in images {
        parse(path, text(bytes.as_deref())?)?;
    }
    Ok(())
}

pub(crate) fn validate_client_config_images(client: &str, images: &Images) -> Result<(), String> {
    validate_config_images(images)?;
    if client == "workbuddy" {
        for (_, bytes) in images {
            parse_workbuddy_config(text(bytes.as_deref())?)?;
        }
        return Ok(());
    }
    for (path, bytes) in images {
        if !parse(path, text(bytes.as_deref())?)?.is_object() {
            return Err("Configuration root must be an object".into());
        }
    }
    if matches!(client, "opencode" | "openclaw") {
        return Ok(());
    }
    for (path, bytes) in images {
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("toml" | "yaml" | "yml")
        ) {
            continue;
        }
        if let Some(content) = text(bytes.as_deref())? {
            serde_json::from_str::<Value>(content.strip_prefix('\u{feff}').unwrap_or(content))
                .map_err(|_| {
                    "Invalid configuration JSON. Restore it from a manual backup or repair it with the base configuration template".to_string()
                })?;
        }
    }
    Ok(())
}

pub(crate) fn prepare_config_updates(
    client: &str,
    paths: &[PathBuf],
    before: &Images,
    updates: &[AgentFileUpdate],
    template: bool,
) -> Result<Images, String> {
    if !template {
        validate_client_config_images(client, before)?;
    }
    let mut after = before.clone();
    for update in updates {
        let entry = after
            .iter_mut()
            .find(|(path, _)| path == &update.path)
            .ok_or("Configuration update path mismatch")?;
        let mut rendered = update.after.clone();
        let mut generated = parse(&update.path, Some(&rendered))?;
        if !template {
            let original = parse(&update.path, text(entry.1.as_deref())?)?;
            if client == "codex"
                && update.path.file_name().and_then(|v| v.to_str())
                    == Some(CODEX_MODEL_CATALOG_FILE)
            {
                let mut merged = original.clone();
                set(&mut merged, &["models".into()], generated.get("models"));
                if merged != generated {
                    generated = merged;
                    rendered = render(&update.path, &generated)?;
                }
            }
            let built = generated.clone();
            preserve_model_extensions(client, &update.path, &original, &mut generated);
            if generated != built {
                rendered = render(&update.path, &generated)?;
            }
            validate_unmanaged_preserved(client, paths, &update.path, &original, &generated)?;
            if entry.1.is_some() && original == generated {
                continue;
            }
        }
        entry.1 = Some(rendered.into_bytes());
    }
    if template
        && (updates.len() != paths.len()
            || paths
                .iter()
                .any(|p| updates.iter().filter(|u| &u.path == p).count() != 1))
    {
        return Err("Base configuration template must cover the entire file group".into());
    }
    validate_client_config_images(client, &after)?;
    Ok(after)
}

fn preserve_model_extensions(client: &str, path: &Path, before: &Value, after: &mut Value) {
    fn fill_missing(before: &Value, after: &mut Value) {
        if let (Some(old), Some(new)) = (before.as_object(), after.as_object_mut()) {
            for (key, value) in old {
                if let Some(next) = new.get_mut(key) {
                    fill_missing(value, next);
                } else {
                    new.insert(key.clone(), value.clone());
                }
            }
        }
    }
    fn merge_entry(client: &str, before: &Value, after: &mut Value) {
        let mut extensions = before.clone();
        let owned: &[&str] = match client {
            "claude-desktop" => &["name", "labelOverride", "anthropicFamilyTier", "isFamilyDefault", "contextWindow", "supports1m", "prefer1m"],
            "opencode" => &["name"],
            "openclaw" => &["id", "name", "alias"],
            "deepseek-harness" => &["id", "name", "contextWindow", "input", "maxTokens", "reasoningEfforts", "compat"],
            "kimi-code" => &[
                "provider",
                "model",
                "display_name",
                "max_context_size",
                "capabilities",
            ],
            "grok-build" => &[
                "model",
                "base_url",
                "name",
                "api_key",
                "api_backend",
                "context_window",
            ],
            _ => &[],
        };
        if let Some(object) = extensions.as_object_mut() {
            if client == "codex" {
                object.retain(|key, _| !codex_catalog::is_managed_model_field(key));
            }
            for key in owned {
                object.remove(*key);
            }
        }
        fill_missing(&extensions, after);
    }
    fn inventory(client: &str, before: &Value, after: &mut Value) {
        if let (Some(old), Some(new)) = (before.as_array(), after.as_array_mut()) {
            let identity = |v: &Value| {
                ["id", "slug", "name"]
                    .into_iter()
                    .find_map(|key| v.get(key).and_then(Value::as_str).map(str::to_string))
            };
            for entry in new {
                if let Some(id) = identity(entry) {
                    if let Some(old) = old.iter().find(|v| identity(v).as_deref() == Some(&id)) {
                        merge_entry(client, old, entry);
                    }
                }
            }
        } else if let (Some(old), Some(new)) = (before.as_object(), after.as_object_mut()) {
            for (key, entry) in new {
                if let Some(old) = old.get(key) {
                    merge_entry(client, old, entry);
                }
            }
        }
    }
    let pointers: &[&str] = match client {
        "codex" if path.file_name().and_then(|v| v.to_str()) == Some(CODEX_MODEL_CATALOG_FILE) => {
            &["/models"]
        }
        "claude-desktop" => &["/inferenceModels"],
        "opencode" => &["/provider/cpa-gui/models"],
        "openclaw" => &[
            "/models/providers/cpa-gui/models",
            "/agents/defaults/models",
        ],
        "kimi-code" => &["/models"],
        "grok-build" => &["/model"],
        _ => &[],
    };
    for pointer in pointers {
        if let (Some(old), Some(new)) = (before.pointer(pointer), after.pointer_mut(pointer)) {
            inventory(client, old, new);
        }
    }
    if client == "deepseek-harness" {
        let pointer = format!("/llm-pi-ai/providers/{DEEPSEEK_HARNESS_PROVIDER_ID}/models");
        if let (Some(old), Some(new)) = (before.pointer(&pointer), after.pointer_mut(&pointer)) {
            inventory(client, old, new);
        }
    }
    if client == "hermes" {
        let old = before
            .get("custom_providers")
            .and_then(Value::as_array)
            .and_then(|providers| {
                providers.iter().find(|p| {
                    p.get("name").and_then(Value::as_str) == Some(MANAGED_AGENT_PROVIDER_ID)
                })
            })
            .and_then(|p| p.get("models"));
        let new = after
            .get_mut("custom_providers")
            .and_then(Value::as_array_mut)
            .and_then(|providers| {
                providers.iter_mut().find(|p| {
                    p.get("name").and_then(Value::as_str) == Some(MANAGED_AGENT_PROVIDER_ID)
                })
            })
            .and_then(|p| p.get_mut("models"));
        if let (Some(old), Some(new)) = (old, new) {
            inventory(client, old, new);
        }
    }
}

pub(crate) fn config_updates(
    client: &str,
    home: &Path,
    before: &Images,
    updates: &[AgentFileUpdate],
    source: &str,
    model: Option<String>,
    mappings: Option<&ClaudeDesktopModelMappings>,
) -> Result<AgentConfigActionResult, String> {
    let paths = config_paths(client, home)?;
    let after = prepare_config_updates(client, &paths, before, updates, source == "template")?;
    commit_config_with_mappings(
        client,
        &paths,
        before,
        &after,
        source,
        model,
        mappings.cloned(),
    )
}

pub(crate) fn config_package_operation(
    home: &Path,
    _source: &str,
    operation: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _guard = AGENT_CONFIG_FILE_LOCK
        .lock()
        .map_err(|_| "Configuration file lock is poisoned")?;
    let paths = config_paths(PI_AGENT_ID, home)?;
    let before = config_images(&paths)?;
    validate_config_images(&before)?;
    if operation()
        .and_then(|_| validate_config_images(&config_images(&paths)?))
        .is_err()
    {
        return match write_config_images(PI_AGENT_ID, &before) {
            Ok(()) => Err("Plugin operation failed; this configuration change was rolled back".into()),
            Err(_) => Err("Plugin operation and configuration rollback both failed. Check the configuration".into()),
        };
    }
    Ok(())
}

fn validate_unmanaged_preserved(
    client: &str,
    paths: &[PathBuf],
    path: &Path,
    before: &Value,
    after: &Value,
) -> Result<(), String> {
    if client == "workbuddy" {
        return validate_workbuddy_unmanaged_preserved(before, after);
    }
    let project = |value: &Value| -> Result<Value, String> {
        let mut value = value.clone();
        if client == "claude-desktop" && paths.get(3).is_some_and(|p| p == path) {
            if let Some(root) = value.as_object_mut() {
                repair_claude_desktop_meta_names(root);
            }
        }
        if client == "claude-code" {
            if let Some(root) = value.as_object_mut() {
                // These are CPA-managed Claude Code settings. Exclude them
                // when comparing user-owned configuration so a model update
                // does not get rejected as an unrelated custom edit.
                root.remove("model");
                root.remove("modelSettings");
                root.remove("autoCompactWindow");
                root.remove("autoCompactEnabled");
            }
            if let Some(env) = value.get_mut("env").and_then(Value::as_object_mut) {
                env.retain(|key, _| {
                    !matches!(
                        key.as_str(),
                        "CLAUDE_CODE_SUBAGENT_MODEL"
                            | CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV
                            | CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV
                            | "CLAUDE_CODE_EFFORT_LEVEL"
                            | "EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL"
                            | "EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL"
                    ) && !key.starts_with("ANTHROPIC_DEFAULT_")
                        && !key.starts_with("ANTHROPIC_MODEL_")
                        && !key.starts_with("ANTHROPIC_CUSTOM_MODEL_OPTION")
                });
            }
        }
        if client == "deepseek-harness" {
            if path == &paths[0] {
                for key in ["apiKeyEnv", "baseURL", "models"].into_iter().chain(harness_fields("provider")) {
                    set(
                        &mut value,
                        &[
                            "llm-pi-ai".into(),
                            "providers".into(),
                            DEEPSEEK_HARNESS_PROVIDER_ID.into(),
                            key.into(),
                        ],
                        None,
                    );
                }
                for key in ["provider", "model", "reasoningEffort"] {
                    set(
                        &mut value,
                        &["agent-default-model".into(), key.into()],
                        None,
                    );
                }
            } else {
                set(
                    &mut value,
                    &["refs".into(), DEEPSEEK_HARNESS_CREDENTIAL.into()],
                    None,
                );
                set(&mut value, &[DEEPSEEK_HARNESS_CREDENTIAL.into()], None);
                if value.get("version") == Some(&serde_json::json!(1)) {
                    set(&mut value, &["version".into()], None);
                }
            }
            return Ok(value);
        }
        let rendered = render(path, &value)?;
        let stripped = restore_text(client, paths, path, &rendered, None)?;
        parse(path, stripped.as_deref())
    };
    if project(before)? != project(after)? {
        return Err(format!(
            "The update unexpectedly changed custom configuration and was not written: {}",
            path_to_string(path)
        ));
    }
    Ok(())
}

pub(crate) fn text(bytes: Option<&[u8]>) -> Result<Option<&str>, String> {
    bytes
        .map(|bytes| std::str::from_utf8(bytes).map_err(|_| "Configuration is not UTF-8 text".to_string()))
        .transpose()
}

pub(crate) fn parse(path: &Path, content: Option<&str>) -> Result<Value, String> {
    let Some(content) = content else {
        return Ok(serde_json::json!({}));
    };
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let result = match path.extension().and_then(|v| v.to_str()) {
        Some("toml") => toml::from_str::<toml::Value>(content)
            .map_err(|e| e.to_string())
            .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        Some("yaml" | "yml") => serde_norway::from_str::<Value>(content).map_err(|e| e.to_string()),
        _ => json5::from_str::<Value>(content).map_err(|e| e.to_string()),
    };
    let value = result.map_err(|_| {
        format!(
            "Invalid {} configuration format. Restore it from a manual backup or repair it with the base configuration template",
            path_to_string(path)
        )
    })?;
    // WorkBuddy also saves models.json as a top-level array.
    if !value.is_object() && !(path.file_name().is_some_and(|n| n == "models.json") && value.is_array()) {
        return Err(format!("{} configuration root must be an object", path_to_string(path)));
    }
    Ok(value)
}

pub(crate) fn render(path: &Path, value: &Value) -> Result<String, String> {
    match path.extension().and_then(|v| v.to_str()) {
        Some("toml") => toml::to_string_pretty(value).map_err(|e| e.to_string()),
        Some("yaml" | "yml") => serde_norway::to_string(value).map_err(|e| e.to_string()),
        _ => serde_json::to_string_pretty(value)
            .map(|v| format!("{v}\n"))
            .map_err(|e| e.to_string()),
    }
}

fn set(value: &mut Value, path: &[String], item: Option<&Value>) {
    if path.is_empty() {
        return;
    }
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let object = value.as_object_mut().unwrap();
    if path.len() == 1 {
        if let Some(item) = item {
            object.insert(path[0].clone(), item.clone());
        } else {
            object.remove(&path[0]);
        }
    } else if item.is_some() || object.contains_key(&path[0]) {
        let child = object
            .entry(path[0].clone())
            .or_insert_with(|| serde_json::json!({}));
        set(child, &path[1..], item);
        if child.as_object().is_some_and(|v| v.is_empty()) {
            object.remove(&path[0]);
        }
    }
}

fn special_keys(client: &str, path: &Path) -> Option<&'static [&'static str]> {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    match (client, name) {
        ("codex", "auth.json") => Some(&["auth_mode", "OPENAI_API_KEY", "tokens", "last_refresh"]),
        ("codex", CODEX_MODEL_CATALOG_FILE) => Some(&["models"]),
        ("pi", PI_AGENT_CONFIG_FILE) => Some(&["baseUrl", "apiKey"]),
        ("pi", PI_AGENT_SETTINGS_FILE) => Some(&["defaultProvider", "defaultModel"]),
        _ => None,
    }
}

fn restore_text(
    client: &str,
    paths: &[PathBuf],
    path: &Path,
    current: &str,
    target: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(keys) = special_keys(client, path) {
        let mut root = parse(path, Some(current))?;
        let target_value = parse(path, target)?;
        for key in keys {
            set(&mut root, &[key.to_string()], target_value.get(key));
        }
        return if root.as_object().unwrap().is_empty() && target.is_none() {
            Ok(None)
        } else {
            render(path, &root).map(Some)
        };
    }
    if client == "claude-desktop" && paths.get(3).is_some_and(|p| p == path) {
        let mut root = parse(path, Some(current))?;
        let target = parse(path, target)?;
        set(&mut root, &["appliedId".into()], target.get("appliedId"));
        let mut entries = root
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let original = target
            .get("entries")
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries.iter().find(|e| {
                    e.get("id").and_then(Value::as_str) == Some(CLAUDE_DESKTOP_PROFILE_ID)
                })
            });
        let mut managed = entries
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(CLAUDE_DESKTOP_PROFILE_ID))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        entries.retain(|e| e.get("id").and_then(Value::as_str) != Some(CLAUDE_DESKTOP_PROFILE_ID));
        for key in ["id", "name"] {
            set(
                &mut managed,
                &[key.into()],
                original.and_then(|v| v.get(key)),
            );
        }
        if managed.as_object().is_some_and(|o| !o.is_empty()) {
            managed["id"] = serde_json::json!(CLAUDE_DESKTOP_PROFILE_ID);
            entries.push(managed);
        }
        if entries.is_empty() {
            root.as_object_mut().unwrap().remove("entries");
        } else {
            root["entries"] = Value::Array(entries);
        }
        return render(path, &root).map(Some);
    }
    let parsed = AgentClient::parse(client)?;
    let base_paths = if parsed == AgentClient::Codex {
        &paths[..1]
    } else {
        paths
    };
    if parsed == AgentClient::Codex && base_paths.first().is_some_and(|config| config == path) {
        return build_restored_codex_agent_config_with_policy(Some(current), target, false);
    }
    build_agent_session_restored_bytes(
        parsed,
        base_paths,
        path,
        Some(current.as_bytes()),
        target.map(str::as_bytes),
    )?
    .map(|v| String::from_utf8(v).map_err(|e| e.to_string()))
    .transpose()
}
