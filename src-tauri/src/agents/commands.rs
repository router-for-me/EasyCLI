use super::*;
use std::collections::HashMap;

const AGENT_STATUS_DETECTION_CONCURRENCY: usize = 4;

pub(crate) fn agent_executable_overrides(
    executable_overrides: &HashMap<String, String>,
    client: AgentClient,
) -> (Option<&Path>, Option<&Path>) {
    (
        executable_overrides.get(client.id()).map(Path::new),
        executable_overrides
            .get(&format!("{}:app", client.id()))
            .map(Path::new),
    )
}

pub(crate) fn pi_executable_override(
    executable_overrides: Option<&HashMap<String, String>>,
) -> Option<&Path> {
    executable_overrides
        .and_then(|overrides| overrides.get(PI_AGENT_ID))
        .map(Path::new)
}

pub(crate) fn resolve_pi_executable(
    home: &Path,
    executable_overrides: Option<&HashMap<String, String>>,
) -> Option<PathBuf> {
    pi_executable_override(executable_overrides)
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| find_pi_executable(home))
}

static CODEX_CATALOG_SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CODEX_CATALOG_UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CODEX_CATALOG_REFRESH: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[derive(Clone, Copy)]
enum AgentStatusDetectionTarget {
    Client(AgentClient),
    PiProvider,
}

pub(crate) fn inspect_agent_config_statuses(
    app: &tauri::AppHandle,
    config: &GuiConfigFile,
) -> Result<Vec<AgentConfigStatus>, String> {
    inspect_agent_config_statuses_with_overrides(app, config, &HashMap::new())
}

pub(crate) fn inspect_agent_config_statuses_with_overrides(
    app: &tauri::AppHandle,
    config: &GuiConfigFile,
    executable_overrides: &HashMap<String, String>,
) -> Result<Vec<AgentConfigStatus>, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let api_key = effective_agent_api_key(config);
    let targets = [
        AgentStatusDetectionTarget::Client(AgentClient::ClaudeCode),
        AgentStatusDetectionTarget::Client(AgentClient::ClaudeDesktop),
        AgentStatusDetectionTarget::Client(AgentClient::Codex),
        AgentStatusDetectionTarget::Client(AgentClient::OpenCode),
        AgentStatusDetectionTarget::Client(AgentClient::OpenClaw),
        AgentStatusDetectionTarget::Client(AgentClient::Hermes),
        AgentStatusDetectionTarget::Client(AgentClient::DeepSeekHarness),
        AgentStatusDetectionTarget::Client(AgentClient::ZCode),
        AgentStatusDetectionTarget::Client(AgentClient::WorkBuddy),
        AgentStatusDetectionTarget::Client(AgentClient::AntigravityCli),
        AgentStatusDetectionTarget::Client(AgentClient::KimiCode),
        AgentStatusDetectionTarget::Client(AgentClient::GrokBuild),
        AgentStatusDetectionTarget::Client(AgentClient::Omp),
        AgentStatusDetectionTarget::PiProvider,
    ];
    let queue = Mutex::new(targets.into_iter().enumerate());
    let results = Mutex::new(vec![None; targets.len()]);
    thread::scope(|scope| {
        let mut workers = Vec::with_capacity(AGENT_STATUS_DETECTION_CONCURRENCY);
        for _ in 0..AGENT_STATUS_DETECTION_CONCURRENCY {
            let queue = &queue;
            let results = &results;
            let home = &home;
            workers.push(scope.spawn(move || -> Result<(), String> {
                loop {
                    let next = queue
                        .lock()
                        .map_err(|_| "Agent detection task queue lock is poisoned".to_string())?
                        .next();
                    let Some((index, target)) = next else {
                        return Ok(());
                    };
                    let status = match target {
                        AgentStatusDetectionTarget::Client(client) => {
                            let (executable_override, desktop_override) =
                                agent_executable_overrides(executable_overrides, client);
                            inspect_agent_config_with_executables(client, home, config.port, api_key,
                                executable_override, desktop_override)
                        }
                        AgentStatusDetectionTarget::PiProvider => {
                            inspect_pi_provider_status_with_executable(home, config.port, api_key,
                                executable_overrides.get(PI_AGENT_ID).map(Path::new))
                        }
                    };
                    results
                        .lock()
                        .map_err(|_| "Agent detection result lock is poisoned".to_string())?[index] = Some(status);
                }
            }));
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "Agent detection worker thread exited unexpectedly".to_string())??;
        }
        Ok::<(), String>(())
    })?;
    results
        .into_inner()
        .map_err(|_| "Agent detection result lock is poisoned".to_string())?
        .into_iter()
        .map(|status| status.ok_or_else(|| "Agent detection result is incomplete".to_string()))
        .collect()
}

pub(crate) fn refresh_agent_config_status_cache(
    app: &tauri::AppHandle,
    gui_config_state: &GuiConfigState,
    cache: &AgentConfigStatusCache,
) -> Result<Vec<AgentConfigStatus>, String> {
    let _refresh_guard = cache
        .refresh_lock
        .lock()
        .map_err(|_| "Agent configuration status refresh lock is poisoned".to_string())?;
    let config = gui_config_state.snapshot()?;
    let port = config.port;
    let api_key = effective_agent_api_key(&config);
    let statuses = inspect_agent_config_statuses(app, &config)?;
    cache.replace(port, api_key, statuses.clone())?;
    Ok(statuses)
}

#[tauri::command]
pub(crate) async fn get_agent_config_statuses(
    app: tauri::AppHandle,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<Vec<AgentConfigStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gui_config_state = app.state::<GuiConfigState>();
        let cache = app.state::<AgentConfigStatusCache>();
        {
            let _refresh_guard = cache
                .refresh_lock
                .lock()
                .map_err(|_| "Agent configuration status refresh lock is poisoned".to_string())?;
            let config = gui_config_state.snapshot()?;
            let port = config.port;
            if executable_overrides.as_ref().is_none_or(HashMap::is_empty) {
              if let Some(statuses) = cache.get(port, effective_agent_api_key(&config))? {
                return Ok(statuses);
              }
            }
        }
        let config = gui_config_state.snapshot()?;
        let statuses = inspect_agent_config_statuses_with_overrides(&app, &config, &executable_overrides.unwrap_or_default())?;
        cache.replace(config.port, effective_agent_api_key(&config), statuses.clone())?;
        Ok(statuses)
    })
    .await
    .map_err(|error| format!("Agent detection background task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn refresh_agent_config_statuses(
    app: tauri::AppHandle,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<Vec<AgentConfigStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gui_config_state = app.state::<GuiConfigState>();
        let cache = app.state::<AgentConfigStatusCache>();
        let config = gui_config_state.snapshot()?;
        let statuses = inspect_agent_config_statuses_with_overrides(&app, &config, &executable_overrides.unwrap_or_default())?;
        cache.replace(config.port, effective_agent_api_key(&config), statuses.clone())?;
        Ok(statuses)
    })
    .await
    .map_err(|error| format!("Agent detection background task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn get_agent_models(
    _app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    client: String,
) -> Result<Vec<AgentModelOption>, String> {
    if client.trim().eq_ignore_ascii_case(PI_AGENT_ID) {
        let config = gui_config_state.snapshot()?;
        return fetch_agent_models(config.port, effective_agent_api_key(&config)).await;
    }
    let client = AgentClient::parse(&client)?;
    let _sync_guard = if client == AgentClient::Codex {
        Some(CODEX_CATALOG_SYNC_LOCK.lock().await)
    } else {
        None
    };
    let config = gui_config_state.snapshot()?;
    let prepared = fetch_prepared_agent_models(client, &config).await?;
    Ok(prepared.models)
}

pub(crate) async fn resolve_pi_default_model(
    config: &GuiConfigFile,
    model: &str,
) -> Result<String, String> {
    let models = fetch_agent_models(config.port, effective_agent_api_key(config)).await?;
    resolve_available_agent_model(&models, &validate_agent_model(model)?)
}

#[tauri::command]
pub(crate) async fn check_pi_provider_update(
    app: tauri::AppHandle,
) -> Result<PiProviderUpdateStatus, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let installed_version = read_pi_provider_version(&home)?;
    let Some(installed_version_value) = installed_version.as_deref() else {
        return Ok(PiProviderUpdateStatus {
            installed_version: None,
            latest_version: None,
            update_available: false,
        });
    };
    let proxy_url = app.state::<GuiConfigState>().snapshot()?.proxy_url;
    let latest_version = fetch_pi_provider_latest_version(&proxy_url).await?;
    let update_available = pi_provider_update_available(installed_version_value, &latest_version)?;
    Ok(PiProviderUpdateStatus {
        installed_version,
        latest_version: Some(latest_version),
        update_available,
    })
}

#[tauri::command]
pub(crate) async fn install_pi_provider(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    cache: tauri::State<'_, AgentConfigStatusCache>,
    model: String,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let config = gui_config_state.snapshot()?;
    let executable = resolve_pi_executable(&home, executable_overrides.as_ref());
    if executable.is_none()
        && !inspect_pi_provider_status_with_executable(
            &home,
            config.port,
            effective_agent_api_key(&config),
            pi_executable_override(executable_overrides.as_ref()),
        )
        .config_exists
    {
        return Err("Pi CLI and Pi configuration file were not detected".to_string());
    }
    let model = resolve_pi_default_model(&config, &model).await?;
    let port = config.port;
    let api_key = effective_agent_api_key(&config).to_string();
    let proxy_url = config.proxy_url.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if let Some(executable) = executable {
            install_pi_provider_inner(&home, &executable, port, &api_key, &model, &proxy_url)
        } else {
            configure_pi_provider_without_cli_inner(&home, port, &api_key, &model)
        }
    })
    .await
    .map_err(|error| format!("Pi CLIProxyAPI provider installation task failed: {error}"))??;
    cache.clear()?;
    Ok(result)
}

#[tauri::command]
pub(crate) async fn update_pi_provider(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    cache: tauri::State<'_, AgentConfigStatusCache>,
    model: String,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let config = gui_config_state.snapshot()?;
    let executable = resolve_pi_executable(&home, executable_overrides.as_ref())
        .ok_or_else(|| "Pi CLI was not detected. Install Pi and ensure the pi command is in PATH".to_string())?;
    let model = resolve_pi_default_model(&config, &model).await?;
    let port = config.port;
    let api_key = effective_agent_api_key(&config).to_string();
    let proxy_url = config.proxy_url.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        update_pi_provider_inner(&home, &executable, port, &api_key, &model, &proxy_url)
    })
    .await
    .map_err(|error| format!("Pi plugin update task failed: {error}"))??;
    cache.clear()?;
    Ok(result)
}

#[tauri::command]
pub(crate) async fn repair_pi_provider(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    cache: tauri::State<'_, AgentConfigStatusCache>,
    model: String,
) -> Result<AgentConfigActionResult, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let config = gui_config_state.snapshot()?;
    let model = resolve_pi_default_model(&config, &model).await?;
    let port = config.port;
    let api_key = effective_agent_api_key(&config).to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        repair_pi_provider_inner(&home, port, &api_key, &model)
    })
    .await
    .map_err(|error| format!("Pi configuration repair task failed: {error}"))??;
    cache.clear()?;
    Ok(result)
}

#[tauri::command]
pub(crate) async fn uninstall_pi_provider(
    app: tauri::AppHandle,
    cache: tauri::State<'_, AgentConfigStatusCache>,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let executable = resolve_pi_executable(&home, executable_overrides.as_ref())
        .ok_or_else(|| "Pi CLI was not detected. Install Pi first".to_string())?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        uninstall_pi_provider_inner(&home, &executable)
    })
    .await
    .map_err(|error| format!("Pi plugin uninstall task failed: {error}"))??;
    cache.clear()?;
    Ok(result)
}

#[tauri::command]
pub(crate) fn check_codex_oauth_login(app: tauri::AppHandle) -> Result<(), String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    validate_codex_oauth_login(&home)
}

#[tauri::command]
pub(crate) async fn update_codex_model_catalog(
    app: tauri::AppHandle,
) -> Result<CodexModelCatalogUpdateResult, String> {
    update_codex_model_catalog_inner(&app).await
}

pub(crate) async fn update_codex_model_catalog_inner(
    app: &tauri::AppHandle,
) -> Result<CodexModelCatalogUpdateResult, String> {
    let _update_guard = CODEX_CATALOG_UPDATE_LOCK.lock().await;
    let proxy_url = app.state::<GuiConfigState>().snapshot()?.proxy_url;
    let client = build_http_client_with_proxy(
        reqwest::Client::builder()
            .redirect(github_https_redirect_policy())
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(25)),
        &proxy_url,
        "Failed to create Codex model catalog update client",
    )?;
    let response = client
        .get(CODEX_MODEL_CATALOG_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("Failed to read Codex model catalog from GitHub: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Failed to read Codex model catalog from GitHub: HTTP {}",
            status.as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_CODEX_MODEL_CATALOG_BYTES as u64)
    {
        return Err(format!(
            "GitHub Codex model catalog exceeds the {} MiB limit",
            MAX_CODEX_MODEL_CATALOG_BYTES / 1024 / 1024
        ));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Failed to read GitHub Codex model catalog: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_CODEX_MODEL_CATALOG_BYTES {
            return Err(format!(
                "GitHub Codex model catalog exceeds the {} MiB limit",
                MAX_CODEX_MODEL_CATALOG_BYTES / 1024 / 1024
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let catalog_json = String::from_utf8(bytes)
        .map_err(|_| "GitHub Codex model catalog is not a valid UTF-8 file".to_string())?;
    let revision = codex_catalog::validate_catalog_json(&catalog_json)
        .map_err(|error| format!("GitHub Codex model catalog verification failed: {error}"))?;

    if revision < codex_catalog::current_catalog_revision()?
        || codex_catalog::current_catalog_json()? == catalog_json
    {
        return Ok(CodexModelCatalogUpdateResult {
            outcome: "unchanged".to_string(),
        });
    }

    let path = codex_model_catalog_override_path(app)?;
    write_bytes_atomically(&path, catalog_json.as_bytes())?;
    let changed = codex_catalog::activate_catalog_json(&catalog_json)?;
    Ok(CodexModelCatalogUpdateResult {
        outcome: if changed { "updated" } else { "unchanged" }.to_string(),
    })
}

pub(crate) fn codex_model_catalog_override_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Failed to get application data directory: {error}"))?;
    Ok(app_data
        .join(CODEX_MODEL_CATALOG_OVERRIDE_DIR)
        .join(CODEX_MODEL_CATALOG_SOURCE_FILE))
}

pub(crate) fn load_codex_model_catalog_override(app: &tauri::AppHandle) -> Result<(), String> {
    let path = codex_model_catalog_override_path(app)?;
    if !path.is_file() {
        return Ok(());
    }
    let catalog_json = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read local Codex model catalog update file: {error}"))?;
    codex_catalog::activate_catalog_json(&catalog_json)
        .map_err(|error| format!("Invalid local Codex model catalog update file: {error}"))?;
    Ok(())
}

fn codex_model_customizations_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(codex_model_catalog_override_path(app)?.with_file_name("model-customizations.json"))
}

pub(crate) fn load_codex_model_customizations(app: &tauri::AppHandle) -> Result<(), String> {
    codex_catalog::load_customizations(&codex_model_customizations_path(app)?)
}

#[tauri::command]
pub(crate) async fn get_codex_model_catalog_editor(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<codex_catalog::CatalogEditorSnapshot, String> {
    let _sync_guard = CODEX_CATALOG_SYNC_LOCK.lock().await;
    let config = gui_config_state.snapshot()?;
    let runtime_models = fetch_codex_catalog_runtime_models(&config).await?;
    codex_catalog::editor_snapshot(&runtime_models)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexCatalogEditorSaveResult {
    snapshot: codex_catalog::CatalogEditorSnapshot,
    synchronization_error: Option<String>,
}

#[tauri::command]
pub(crate) async fn save_codex_model_catalog_editor(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    request: codex_catalog::CatalogEditorRequest,
) -> Result<CodexCatalogEditorSaveResult, String> {
    let _sync_guard = CODEX_CATALOG_SYNC_LOCK.lock().await;
    let config = gui_config_state.snapshot()?;
    let runtime_models = fetch_codex_catalog_runtime_models(&config).await?;
    let snapshot = codex_catalog::save_customizations(
        &codex_model_customizations_path(&app)?,
        &runtime_models,
        request,
    )?;
    let synchronization_error = prepare_codex_agent_models(&runtime_models)
        .and_then(|prepared| sync_prepared_codex_model_catalog(&app, &config, &prepared))
        .err();
    Ok(CodexCatalogEditorSaveResult {
        snapshot,
        synchronization_error,
    })
}

#[tauri::command]
pub(crate) async fn get_thinking_aliases(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<ThinkingAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let content = fetch_management_config_yaml(&config).await?;
    thinking_aliases_from_yaml(&content)
}

#[tauri::command]
pub(crate) async fn get_model_alias_sources(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<ThinkingAliasSource>, String> {
    let config = gui_config_state.snapshot()?;
    let content = fetch_management_config_yaml(&config).await?;
    let available_models =
        fetch_agent_models(config.port, effective_agent_api_key(&config)).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    Ok(resolved_oauth_alias_sources(
        &content,
        &definitions,
        &available_models,
        AliasSourceCapability::Base,
    )?
    .into_iter()
    .map(|resolved| resolved.source)
    .collect())
}

#[tauri::command]
pub(crate) async fn get_thinking_alias_sources(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<ThinkingAliasSource>, String> {
    let config = gui_config_state.snapshot()?;
    let content = fetch_management_config_yaml(&config).await?;
    let available_models =
        fetch_agent_models(config.port, effective_agent_api_key(&config)).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    Ok(resolved_oauth_alias_sources(
        &content,
        &definitions,
        &available_models,
        AliasSourceCapability::Reasoning,
    )?
    .into_iter()
    .map(|resolved| resolved.source)
    .collect())
}

#[tauri::command]
pub(crate) async fn get_model_alias_edit_source(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    alias: String,
    entry: Option<ThinkingAliasEntry>,
) -> Result<ModelAliasEditContext, String> {
    let config = gui_config_state.snapshot()?;
    let alias = existing_thinking_alias_model_id(&alias, "Alias model")?;
    let content = fetch_management_config_yaml(&config).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    model_alias_edit_context_for_mapping(&content, &alias, &definitions, entry.as_ref())
}

#[tauri::command]
pub(crate) async fn create_thinking_alias(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    source_id: String,
    alias: String,
    effort: String,
    fast: Option<bool>,
    original_alias: Option<String>,
    original_entry: Option<ThinkingAliasEntry>,
) -> Result<Vec<ThinkingAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let source_id = source_id.trim().to_string();
    if source_id.is_empty() {
        return Err("Select the source model first".to_string());
    }
    let original_alias = original_alias
        .as_deref()
        .map(|original| existing_thinking_alias_model_id(original, "Alias model"))
        .transpose()?;
    let alias = match original_alias.as_deref() {
        Some(original) if original.eq_ignore_ascii_case(alias.trim()) => {
            existing_thinking_alias_model_id(&alias, "Alias model")?
        }
        _ => validate_thinking_alias_model_id(&alias, "Alias model")?,
    };
    let effort = if effort.trim().is_empty() {
        String::new()
    } else {
        validate_thinking_alias_effort(&effort)?
    };
    let fast = fast.unwrap_or(false);
    let content = fetch_management_config_yaml(&config).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    let capability = if !effort.is_empty() {
        AliasSourceCapability::Reasoning
    } else if fast {
        AliasSourceCapability::Fast
    } else {
        AliasSourceCapability::Base
    };
    let source = if let Some(original) = original_alias.as_deref()
        .filter(|original| source_id == model_alias_edit_source_id(original))
    {
        resolve_model_alias_edit_source_for_mapping(&content, original, &definitions, original_entry.as_ref())?
    } else {
        let available_models =
            fetch_agent_models(config.port, effective_agent_api_key(&config)).await?;
        resolved_oauth_alias_sources(&content, &definitions, &available_models, capability)?
            .into_iter()
            .find(|source| source.source.id == source_id)
            .ok_or_else(|| {
                "The source model is no longer available in the kernel, or its configuration source has changed. Refresh and select it again".to_string()
            })?
    };
    if fast && !alias_source_supports_fast(&source) {
        return Err("Fast supports only OpenAI-compatible API, Codex API, or Codex OAuth model sources".to_string());
    }
    if !effort.is_empty()
        && !source
            .source
            .reasoning_levels
            .iter()
            .any(|level| level.eq_ignore_ascii_case(&effort))
    {
        return Err(format!(
            "Reasoning effort {effort} is not among the levels currently supported by model {}",
            source.source.model
        ));
    }
    if source.source.model.eq_ignore_ascii_case(&alias) {
        return Err("Alias model cannot be the same as the source model".to_string());
    }

    let updated = match original_alias.as_deref() {
        Some(original) => edit_model_alias_in_yaml_for_mapping(&content, original, &source, &alias, &effort, fast, original_entry.as_ref())?,
        None => add_model_alias_to_yaml(&content, &source, &alias, &effort, fast)?,
    };
    put_management_alias_config_changes(&config, &content, &updated).await?;
    thinking_aliases_from_yaml(&updated)
}

#[tauri::command]
pub(crate) async fn delete_thinking_alias(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    alias: String,
    oauth_channel: Option<String>,
    entry: Option<ThinkingAliasEntry>,
) -> Result<Vec<ThinkingAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let alias = existing_thinking_alias_model_id(&alias, "Alias model")?;
    let content = fetch_management_config_yaml(&config).await?;
    let updated = match entry.as_ref() {
        Some(entry) => remove_model_alias_mapping_from_yaml(&content, &alias, entry)?,
        None => remove_thinking_alias_from_yaml_for_channel(&content, &alias, oauth_channel.as_deref())?,
    };
    put_management_alias_config_changes(&config, &content, &updated).await?;
    thinking_aliases_from_yaml(&updated)
}

#[tauri::command]
pub(crate) async fn get_speed_aliases(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<SpeedAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let content = fetch_management_config_yaml(&config).await?;
    speed_aliases_from_yaml(&content)
}

#[tauri::command]
pub(crate) async fn get_speed_alias_sources(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<ThinkingAliasSource>, String> {
    let config = gui_config_state.snapshot()?;
    let content = fetch_management_config_yaml(&config).await?;
    let available_models =
        fetch_agent_models(config.port, effective_agent_api_key(&config)).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    Ok(resolved_oauth_alias_sources(
        &content,
        &definitions,
        &available_models,
        AliasSourceCapability::Fast,
    )?
    .into_iter()
    .map(|resolved| resolved.source)
    .collect())
}

#[tauri::command]
pub(crate) async fn create_speed_alias(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    source_id: String,
    alias: String,
) -> Result<Vec<SpeedAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let source_id = source_id.trim().to_string();
    if source_id.is_empty() {
        return Err("Select the source model first".to_string());
    }
    let alias = validate_thinking_alias_model_id(&alias, "Alias model")?;
    let content = fetch_management_config_yaml(&config).await?;
    let available_models =
        fetch_agent_models(config.port, effective_agent_api_key(&config)).await?;
    let definitions = fetch_oauth_model_definitions(&config).await;
    let sources = resolved_oauth_alias_sources(
        &content,
        &definitions,
        &available_models,
        AliasSourceCapability::Fast,
    )?;
    let source = sources
        .iter()
        .find(|source| source.source.id == source_id)
        .cloned()
        .ok_or_else(|| {
            "The source model is no longer available in the kernel, or its configuration source has changed. Refresh and try again".to_string()
        })?;
    if source.source.model.eq_ignore_ascii_case(&alias) {
        return Err("Alias model cannot be the same as the source model".to_string());
    }
    let updated = add_speed_alias_to_yaml(&content, &source, &alias)?;
    put_management_alias_config_changes(&config, &content, &updated).await?;
    speed_aliases_from_yaml(&updated)
}

#[tauri::command]
pub(crate) async fn delete_speed_alias(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    alias: String,
    oauth_channel: Option<String>,
    entry: Option<ThinkingAliasEntry>,
) -> Result<Vec<SpeedAliasEntry>, String> {
    let config = gui_config_state.snapshot()?;
    let alias = existing_thinking_alias_model_id(&alias, "Alias model")?;
    let content = fetch_management_config_yaml(&config).await?;
    let updated = match entry.as_ref() {
        Some(entry) => remove_model_alias_mapping_from_yaml(&content, &alias, entry)?,
        None => remove_speed_alias_from_yaml_for_channel(&content, &alias, oauth_channel.as_deref())?,
    };
    put_management_alias_config_changes(&config, &content, &updated).await?;
    speed_aliases_from_yaml(&updated)
}

pub(crate) async fn fetch_agent_models(
    port: u16,
    api_key: &str,
) -> Result<Vec<AgentModelOption>, String> {
    let payload = fetch_agent_model_payload(port, api_key).await?;
    parse_agent_model_options(&payload)
}

pub(crate) async fn fetch_agent_model_payload(
    port: u16,
    api_key: &str,
) -> Result<serde_json::Value, String> {
    if port == 0 {
        return Err("Invalid kernel port".to_string());
    }
    let tls_enabled = managed_core_tls_enabled();
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .danger_accept_invalid_certs(tls_enabled)
        .build()
        .map_err(|error| format!("Failed to create model list client: {error}"))?;
    let base_url = managed_core_loopback_origin(port);
    let endpoints = [
        format!("{base_url}/v1/models"),
        format!("{base_url}/models"),
    ];

    for (index, endpoint) in endpoints.iter().enumerate() {
        let response = client
            .get(endpoint)
            .bearer_auth(api_key)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|error| format!("Failed to request local model list: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read local model list: {error}"))?;
        if status.is_success() {
            let payload = serde_json::from_str::<serde_json::Value>(&body).map_err(|error| {
                format!(
                    "Failed to parse local model list: {error}; body={}",
                    truncate_for_error(&body)
                )
            })?;
            return Ok(payload);
        }

        let can_try_legacy_path = index == 0 && matches!(status.as_u16(), 404 | 405);
        if !can_try_legacy_path {
            return Err(format_agent_models_error(status.as_u16(), &body));
        }
    }

    Err("Local kernel does not support the model list endpoint".to_string())
}

pub(crate) async fn fetch_codex_runtime_models(
    port: u16,
    api_key: &str,
) -> Result<Vec<codex_catalog::CodexRuntimeModel>, String> {
    if port == 0 {
        return Err("Invalid kernel port".to_string());
    }
    let tls_enabled = managed_core_tls_enabled();
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(15))
        .danger_accept_invalid_certs(tls_enabled)
        .build()
        .map_err(|error| format!("Failed to create Codex model list client: {error}"))?;
    let base_url = managed_core_loopback_origin(port);
    let endpoints = [
        format!("{base_url}/v1/models"),
        format!("{base_url}/models"),
    ];

    for (index, endpoint) in endpoints.iter().enumerate() {
        let response = client
            .get(endpoint)
            .query(&[("client_version", env!("CARGO_PKG_VERSION"))])
            .bearer_auth(api_key)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|error| format!("Failed to request local Codex model list: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Failed to read local Codex model list: {error}"))?;
        if status.is_success() {
            let payload = serde_json::from_str::<serde_json::Value>(&body).map_err(|error| {
                format!(
                    "Failed to parse local Codex model list: {error}; body={}",
                    truncate_for_error(&body)
                )
            })?;
            return codex_catalog::parse_runtime_models(&payload);
        }

        let can_try_legacy_path = index == 0 && matches!(status.as_u16(), 404 | 405);
        if !can_try_legacy_path {
            return Err(format_agent_models_error(status.as_u16(), &body));
        }
    }

    Err("Local kernel does not support the Codex model list endpoint".to_string())
}

pub(crate) async fn fetch_codex_catalog_runtime_models(
    config: &GuiConfigFile,
) -> Result<Vec<codex_catalog::CodexRuntimeModel>, String> {
    let (runtime, content) = tokio::join!(
        fetch_agent_model_payload(config.port, effective_agent_api_key(config)),
        fetch_management_config_yaml(config),
    );
    // Codex selection and its written catalog must use the public /v1/models
    // IDs, not the alternate catalog selected by client_version.
    let mut runtime = codex_catalog::parse_public_models(&runtime?)?;
    codex_catalog::apply_configured_context_limits(&mut runtime, &content?)?;
    Ok(runtime)
}

pub(crate) async fn fetch_prepared_agent_models(
    client: AgentClient,
    config: &GuiConfigFile,
) -> Result<PreparedAgentModels, String> {
    let api_key = effective_agent_api_key(config);
    if client == AgentClient::Codex {
        let runtime_models = fetch_codex_catalog_runtime_models(config).await?;
        prepare_codex_agent_models(&runtime_models)
    } else {
        let mut models = if client == AgentClient::DeepSeekHarness {
            fetch_deepseek_harness_models(config).await?
        } else {
            fetch_agent_models(config.port, api_key).await?
        };
        if agent_uses_cpa_runtime_context_windows(client) {
            let runtime_models = fetch_codex_runtime_models(config.port, api_key).await?;
            codex_catalog::merge_runtime_context_windows(&mut models, &runtime_models);
        }
        if matches!(client, AgentClient::ClaudeCode | AgentClient::ClaudeDesktop) {
            let content = fetch_management_config_yaml(config).await?;
            mark_configured_agent_model_aliases(&mut models, &content)?;
            for model in &mut models {
                model.context_window = Some(claude_catalog::context_window_for(
                    &model.name,
                    model.context_window,
                )?);
                if !model.is_alias {
                    if let Some(display_name) = claude_catalog::display_name_for(&model.name)? {
                        model.alias = Some(display_name);
                    }
                }
            }
        }
        Ok(PreparedAgentModels {
            models,
            codex_catalog: None,
        })
    }
}

fn sync_prepared_codex_model_catalog(
    app: &tauri::AppHandle,
    config: &GuiConfigFile,
    prepared: &PreparedAgentModels,
) -> Result<bool, String> {
    let Some(catalog) = prepared.codex_catalog.as_deref() else {
        return Ok(false);
    };
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let changed = sync_codex_model_catalog_if_configured(
        &home,
        config.port,
        effective_agent_api_key(config),
        &prepared.models,
        catalog,
    )?;
    if changed {
        app.state::<AgentConfigStatusCache>().clear()?;
        let _ = app.emit(
            CONFIG_FILES_CHANGED_EVENT,
            ConfigFilesChangedPayload {
                paths: agent_managed_paths(AgentClient::Codex, &home)
                    .iter()
                    .map(|path| path_to_string(path))
                    .collect(),
                errors: Vec::new(),
            },
        );
    }
    Ok(changed)
}

pub(crate) async fn refresh_applied_codex_model_catalog(
    app: &tauri::AppHandle,
    config: &GuiConfigFile,
) -> Result<bool, String> {
    let _sync_guard = CODEX_CATALOG_SYNC_LOCK.lock().await;
    let runtime_models = fetch_codex_catalog_runtime_models(config).await?;
    let prepared = prepare_codex_agent_models(&runtime_models)?;
    sync_prepared_codex_model_catalog(app, config, &prepared)
}

pub(crate) fn request_codex_model_catalog_refresh() {
    CODEX_CATALOG_REFRESH.notify_one();
}

pub(crate) fn start_codex_model_catalog_sync(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(home) = app.path().home_dir() else {
            return;
        };
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {},
                _ = CODEX_CATALOG_REFRESH.notified() => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                },
            }
            let Ok(config) = app.state::<GuiConfigState>().snapshot() else {
                continue;
            };
            let paths = agent_config_paths(AgentClient::Codex, &home);
            if !matches!(
                inspect_agent_managed_config(
                    AgentClient::Codex,
                    &paths,
                    config.port,
                    effective_agent_api_key(&config),
                ),
                Ok((true, _, _))
            ) {
                continue;
            }
            if let Err(error) = refresh_applied_codex_model_catalog(&app, &config).await {
                eprintln!("Failed to synchronize Codex model catalog in the background; keeping the existing configuration: {error}");
            }
        }
    });
}

pub(crate) fn agent_uses_cpa_runtime_context_windows(client: AgentClient) -> bool {
    matches!(
        client,
        AgentClient::ZCode | AgentClient::WorkBuddy | AgentClient::KimiCode | AgentClient::GrokBuild
    )
}

fn resolve_claude_desktop_source_model(models: &[AgentModelOption], value: &str) -> Result<String, String> {
    let model = validate_agent_model(value)?;
    Ok(models.iter().find(|entry| entry.name.eq_ignore_ascii_case(&model))
        .map(|entry| entry.name.clone()).unwrap_or(model))
}

pub(crate) fn resolve_agent_configuration_model(
    client: AgentClient,
    models: &[AgentModelOption],
    model: &str,
    desktop_mappings: Option<&ClaudeDesktopModelMappings>,
) -> Result<String, String> {
    if client == AgentClient::ClaudeDesktop {
        if let Some(entries) = desktop_mappings.and_then(|mappings| mappings.desktop_models.as_ref()) {
            validate_claude_desktop_entries(entries)?;
            return resolve_claude_desktop_source_model(models, entries[0].source_or_alias());
        }
    }
    resolve_available_agent_model(models, &validate_agent_model(model)?)
}

pub(crate) fn resolve_claude_desktop_model_mappings(
    client: AgentClient,
    models: &[AgentModelOption],
    _selected_model: &str,
    requested: Option<ClaudeDesktopModelMappings>,
) -> Result<Option<ClaudeDesktopModelMappings>, String> {
    if client != AgentClient::ClaudeDesktop {
        return Ok(None);
    }
    let mut requested = requested.ok_or("Reconfigure the Claude Desktop model and alias")?;
    let resolve =
        |model: &str| resolve_available_agent_model(models, &validate_agent_model(model)?);
    if let Some(entries) = requested.desktop_models.as_mut() {
        validate_claude_desktop_entries(entries)?;
        for entry in entries.iter_mut() {
            entry.model = resolve_claude_desktop_source_model(models, entry.source_or_alias())?;
            entry.alias = if entry.alias.trim().eq_ignore_ascii_case(&entry.model) {
                String::new()
            } else {
                entry.alias.trim().to_string()
            };
        }
        requested.sonnet = entries[0].source_or_alias().to_string();
        requested.opus.clear();
        requested.haiku.clear();
        return Ok(Some(requested));
    }
    Ok(Some(ClaudeDesktopModelMappings {
        desktop_models: None,
        opus: resolve(&requested.opus)?,
        sonnet: resolve(&requested.sonnet)?,
        haiku: resolve(&requested.haiku)?,
        opus_1m: requested.opus_1m,
        sonnet_1m: requested.sonnet_1m,
        haiku_1m: requested.haiku_1m,
        fable: resolve(if requested.fable.trim().is_empty() { &requested.sonnet } else { &requested.fable })?,
        fable_1m: requested.fable_1m,
        max_context_tokens: requested.max_context_tokens,
        auto_compact_pct: requested.auto_compact_pct,
        disable_auto_compact: requested.disable_auto_compact,
        manage_default_model: requested.manage_default_model,
        manage_subagent_model: requested.manage_subagent_model,
        startup_model: requested.startup_model.map(|value| value.trim().to_string()),
        subagent_model: requested.subagent_model.map(|value| value.trim().to_string()),
    }))
}

pub(crate) fn resolve_claude_code_model_mappings(
    client: AgentClient,
    models: &[AgentModelOption],
    selected_model: &str,
    requested: Option<ClaudeDesktopModelMappings>,
) -> Result<Option<ClaudeDesktopModelMappings>, String> {
    if client != AgentClient::ClaudeCode {
        return Ok(None);
    }
    let requested = requested.unwrap_or_else(|| ClaudeDesktopModelMappings::all(selected_model));
    if !(100_000..=1_000_000).contains(&requested.max_context_tokens) {
        return Err("Claude Code maximum context window must be between 100000 and 1000000".to_string());
    }
    if !(1..=100).contains(&requested.auto_compact_pct) {
        return Err("Claude Code compaction trigger percentage must be between 1 and 100".to_string());
    }
    let resolve =
        |model: &str| resolve_available_agent_model(models, &validate_agent_model(model)?);
    let normalize_override = |value: Option<String>| -> Result<Option<String>, String> {
        value.map(|value| {
            if value.trim().is_empty() { Ok(String::new()) } else { validate_agent_model(&value) }
        }).transpose()
    };
    Ok(Some(ClaudeDesktopModelMappings {
        desktop_models: None,
        opus: resolve(&requested.opus)?,
        sonnet: resolve(&requested.sonnet)?,
        haiku: resolve(&requested.haiku)?,
        opus_1m: requested.opus_1m,
        sonnet_1m: requested.sonnet_1m,
        haiku_1m: requested.haiku_1m,
        fable: resolve(if requested.fable.trim().is_empty() { &requested.sonnet } else { &requested.fable })?,
        fable_1m: if requested.fable.trim().is_empty() { requested.sonnet_1m } else { requested.fable_1m },
        // 1M preferences select the upstream extended-context route. They do
        // not override the user's independent auto-compaction window.
        max_context_tokens: requested.max_context_tokens,
        auto_compact_pct: requested.auto_compact_pct,
        disable_auto_compact: requested.disable_auto_compact,
        manage_default_model: requested.manage_default_model,
        manage_subagent_model: requested.manage_subagent_model,
        startup_model: normalize_override(requested.startup_model)?,
        subagent_model: normalize_override(requested.subagent_model)?,
    }))
}

pub(crate) fn prepare_codex_agent_models(
    runtime_models: &[codex_catalog::CodexRuntimeModel],
) -> Result<PreparedAgentModels, String> {
    match codex_catalog::prepare_catalog(runtime_models) {
        Ok(catalog) => Ok(PreparedAgentModels {
            models: catalog.models,
            codex_catalog: Some(catalog.json),
        }),
        Err(error) if error.contains("CPA currently has no models that can be written to Codex") => {
            Ok(PreparedAgentModels {
                models: Vec::new(),
                codex_catalog: Some("{\n  \"models\": []\n}\n".to_string()),
            })
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_agent_config(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    client: String,
    model: String,
    oauth_configuration: bool,
    claude_code_model_mappings: Option<ClaudeDesktopModelMappings>,
    claude_desktop_model_mappings: Option<ClaudeDesktopModelMappings>,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    let client = AgentClient::parse(&client)?;
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let config = gui_config_state.snapshot()?;
    let api_key = effective_agent_api_key(&config);
    if client == AgentClient::Codex && oauth_configuration {
        validate_codex_oauth_login(&home)?;
    }
    validate_agent_can_enable(client, &home, config.port, api_key, executable_overrides.as_ref())?;
    let prepared = fetch_prepared_agent_models(client, &config).await?;
    let model = resolve_agent_configuration_model(
        client, &prepared.models, &model, claude_desktop_model_mappings.as_ref(),
    )?;
    let claude_code_model_mappings = resolve_claude_code_model_mappings(
        client,
        &prepared.models,
        &model,
        claude_code_model_mappings,
    )?;
    let claude_desktop_model_mappings = resolve_claude_desktop_model_mappings(
        client,
        &prepared.models,
        &model,
        claude_desktop_model_mappings,
    )?;
    commit_agent_with_core(&config, claude_desktop_model_mappings, &prepared.models, |claude_desktop_model_mappings| {
        let _guard = AGENT_CONFIG_FILE_LOCK
            .lock()
            .map_err(|_| "Agent configuration file lock is poisoned".to_string())?;
        apply_agent_configuration_with_oauth(
            client,
            &home,
            config.port,
            api_key,
            &model,
            AgentConfigurationOptions {
                models: &prepared.models,
                codex_catalog: prepared.codex_catalog.as_deref(),
                oauth_configuration,
                claude_code_model_mappings: claude_code_model_mappings.as_ref(),
                claude_desktop_model_mappings,
            },
        )
    }).await
}

#[tauri::command]
pub(crate) fn clear_codex_config(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let _guard = AGENT_CONFIG_FILE_LOCK
        .lock()
        .map_err(|_| "Agent configuration file lock is poisoned".to_string())?;
    clear_codex_config_files(&home)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn set_agent_config_enabled(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    client: String,
    model: String,
    enabled: bool,
    force_restore: bool,
    claude_code_model_mappings: Option<ClaudeDesktopModelMappings>,
    claude_desktop_model_mappings: Option<ClaudeDesktopModelMappings>,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    let client = AgentClient::parse(&client)?;
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    if enabled {
        let config = gui_config_state.snapshot()?;
        let port = config.port;
        let api_key = effective_agent_api_key(&config);
        validate_agent_can_enable(client, &home, port, api_key, executable_overrides.as_ref())?;
        let prepared = fetch_prepared_agent_models(client, &config).await?;
        let model = resolve_agent_configuration_model(
            client, &prepared.models, &model, claude_desktop_model_mappings.as_ref(),
        )?;
        let claude_code_model_mappings = resolve_claude_code_model_mappings(
            client,
            &prepared.models,
            &model,
            claude_code_model_mappings,
        )?;
        let claude_desktop_model_mappings = resolve_claude_desktop_model_mappings(
            client,
            &prepared.models,
            &model,
            claude_desktop_model_mappings,
        )?;
        commit_agent_with_core(&config, claude_desktop_model_mappings, &prepared.models, |claude_desktop_model_mappings| {
            let _guard = AGENT_CONFIG_FILE_LOCK
                .lock()
                .map_err(|_| "Agent configuration file lock is poisoned".to_string())?;
            let oauth_configuration = if client == AgentClient::Codex {
                current_codex_oauth_configuration(&home)?
            } else {
                false
            };
            apply_agent_configuration_with_oauth(
                client,
                &home,
                port,
                api_key,
                &model,
                AgentConfigurationOptions {
                    models: &prepared.models,
                    codex_catalog: prepared.codex_catalog.as_deref(),
                    oauth_configuration,
                    claude_code_model_mappings: claude_code_model_mappings.as_ref(),
                    claude_desktop_model_mappings,
                },
            )
        }).await
    } else {
        let config = gui_config_state.snapshot()?;
        let _guard = AGENT_CONFIG_FILE_LOCK
            .lock()
            .map_err(|_| "Agent configuration file lock is poisoned".to_string())?;
        let _ = force_restore;
        let result = clear_agent_managed_configuration(client, &home, config.port)?;
        app.state::<AgentConfigStatusCache>().clear()?;
        Ok(result)
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_agent_config(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    client: String,
    model: String,
    oauth_configuration: bool,
    claude_code_model_mappings: Option<ClaudeDesktopModelMappings>,
    claude_desktop_model_mappings: Option<ClaudeDesktopModelMappings>,
    executable_overrides: Option<HashMap<String, String>>,
) -> Result<AgentConfigActionResult, String> {
    apply_agent_config(
        app,
        gui_config_state,
        client,
        model,
        oauth_configuration,
        claude_code_model_mappings,
        claude_desktop_model_mappings,
        executable_overrides,
    ).await
}

pub(crate) fn validate_agent_can_enable(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    executable_overrides: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    if !client.supported_platform() {
        return Err(format!(
            "{} is not supported on the current platform",
            client.name()
        ));
    }
    let (executable_override, desktop_override) = executable_overrides
        .map(|overrides| agent_executable_overrides(overrides, client))
        .unwrap_or((None, None));
    let detection = inspect_agent_config_with_executables(
        client,
        home,
        port,
        api_key,
        executable_override,
        desktop_override,
    );
    if !detection.installed && !detection.config_exists {
        return Err(format!("{} is not installed", client.name()));
    }
    Ok(())
}

pub(crate) fn validate_agent_model(value: &str) -> Result<String, String> {
    let model = value.trim();
    if model.is_empty() {
        return Err("Select a model first".to_string());
    }
    if model.len() > 240 || model.chars().any(char::is_control) {
        return Err("Invalid model name format".to_string());
    }
    Ok(model.to_string())
}

pub(crate) fn resolve_available_agent_model(
    models: &[AgentModelOption],
    model: &str,
) -> Result<String, String> {
    if models.is_empty() {
        return Err("The current kernel has no selectable models, so configuration changes cannot be applied".to_string());
    }
    models
        .iter()
        .find(|available| available.name.eq_ignore_ascii_case(model))
        .map(|available| available.name.clone())
        .ok_or_else(|| format!("Model {model} is not in the current list of available models. Refresh and select it again"))
}

pub(crate) fn parse_agent_model_options(
    payload: &serde_json::Value,
) -> Result<Vec<AgentModelOption>, String> {
    let object_models = payload.get("models").and_then(serde_json::Value::as_object).map(|models| {
        models.iter().map(|(id, value)| {
            let mut entry = value.as_object().cloned().unwrap_or_default();
            entry.entry("id").or_insert(serde_json::json!(id));
            serde_json::Value::Object(entry)
        }).collect::<Vec<_>>()
    });
    let source = payload.as_array()
        .or_else(|| payload.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| payload.get("models").and_then(serde_json::Value::as_array))
        .or(object_models.as_ref())
        .ok_or_else(|| "Local model list response is missing a data array or models catalog".to_string())?;
    let mut models = Vec::new();
    for item in source {
        let name = if let Some(name) = item.as_str() {
            name.trim().to_string()
        } else {
            ["id", "name", "model", "value"]
                .into_iter()
                .find_map(|key| item.get(key).and_then(serde_json::Value::as_str))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let display_name = ["display_name", "displayName", "name"]
            .into_iter()
            .find_map(|key| item.get(key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(&name))
            .map(str::to_string);
        let model_alias = item
            .get("alias")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(&name))
            .map(str::to_string);
        let keep_original = item
            .get("fork")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let context_window = [
            "context_length",
            "contextLength",
            "ContextLength",
            "context_window",
            "contextWindow",
            "max_input_tokens",
            "maxInputTokens",
            "input_token_limit",
            "inputTokenLimit",
        ]
        .into_iter()
        .find_map(|key| item.get(key).and_then(json_positive_u64));
        let input_modalities = codex_catalog::parse_modalities(item).or_else(|| {
            codex_catalog::parse_modalities(&serde_json::json!({"input_modalities": item.get("input")}))
        });
        let harness_metadata = harness_api_metadata(item);

        if let Some(model_alias) = model_alias {
            if keep_original {
                append_agent_model_option(&mut models, &name, display_name, false, context_window, input_modalities.clone(), harness_metadata.clone());
            }
            append_agent_model_option(&mut models, &model_alias, Some(name), true, context_window, input_modalities, harness_metadata);
        } else {
            append_agent_model_option(&mut models, &name, display_name, false, context_window, input_modalities, harness_metadata);
        }
    }
    Ok(models)
}

pub(crate) fn append_agent_model_option(
    models: &mut Vec<AgentModelOption>,
    name: &str,
    alias: Option<String>,
    is_alias: bool,
    context_window: Option<u64>,
    input_modalities: Option<Vec<String>>,
    harness_metadata: Option<serde_json::Value>,
) {
    let name = name.trim();
    if name.is_empty()
        || models
            .iter()
            .any(|model| model.name.eq_ignore_ascii_case(name))
    {
        return;
    }
    let alias = alias
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(name))
        .map(str::to_string);
    models.push(AgentModelOption {
        input_modalities,
        harness_metadata,
        name: name.to_string(),
        alias,
        is_alias,
        context_window,
    });
}

pub(crate) fn mark_configured_agent_model_aliases(
    models: &mut [AgentModelOption],
    content: &str,
) -> Result<(), String> {
    let document = serde_norway::from_str::<serde_norway::Value>(content)
        .map_err(|error| format!("Failed to parse kernel YAML configuration: {error}"))?;
    let root = document
        .as_mapping()
        .ok_or_else(|| "Kernel configuration root must be a YAML mapping".to_string())?;

    for section in MODEL_ALIAS_CONFIG_SECTIONS {
        let Some(providers) = yaml_mapping_value(root, section) else {
            continue;
        };
        let providers = providers
            .as_sequence()
            .ok_or_else(|| format!("{section} must be an array"))?;
        for provider in providers {
            let Some(provider) = provider.as_mapping() else {
                continue;
            };
            if yaml_mapping_value(provider, "disabled") == Some(&serde_norway::Value::Bool(true)) {
                continue;
            }
            let Some(configured_models) = yaml_mapping_value(provider, "models") else {
                continue;
            };
            let configured_models = configured_models
                .as_sequence()
                .ok_or_else(|| format!("{section}.models must be an array"))?;
            for configured in configured_models {
                if configured_model_identity(configured).is_some_and(|(_, client_model, _)| {
                    configured_provider_model_is_enabled(provider, &client_model)
                }) {
                    mark_agent_model_aliases_from_sequence(models, std::slice::from_ref(configured));
                }
            }
        }
    }

    if let Some(oauth_aliases) = yaml_mapping_value(root, "oauth-model-alias") {
        let oauth_aliases = oauth_aliases
            .as_mapping()
            .ok_or_else(|| "oauth-model-alias must be a YAML mapping".to_string())?;
        for entries in oauth_aliases.values() {
            let Some(entries) = entries.as_sequence() else {
                continue;
            };
            mark_agent_model_aliases_from_sequence(models, entries);
        }
    }
    Ok(())
}

pub(crate) fn mark_agent_model_aliases_from_sequence(
    models: &mut [AgentModelOption],
    configured_models: &[serde_norway::Value],
) {
    for configured in configured_models {
        let Some((source_model, client_model, _)) = configured_model_identity(configured) else {
            continue;
        };
        if source_model.eq_ignore_ascii_case(&client_model) {
            continue;
        }
        let Some(model) = models
            .iter_mut()
            .find(|model| model.name.eq_ignore_ascii_case(&client_model))
        else {
            continue;
        };
        model.is_alias = true;
        model.alias = Some(source_model);
    }
}

pub(crate) fn parse_codex_model_definitions(
    payload: &serde_json::Value,
) -> Result<Vec<CodexModelDefinition>, String> {
    let source = payload
        .as_array()
        .or_else(|| payload.get("models").and_then(serde_json::Value::as_array))
        .or_else(|| payload.get("data").and_then(serde_json::Value::as_array))
        .ok_or_else(|| "Codex model definition response is missing a models or data array".to_string())?;
    let mut definitions = Vec::new();
    for item in source {
        let id = ["id", "ID", "name"]
            .into_iter()
            .find_map(|key| item.get(key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(id) = id else {
            continue;
        };
        if definitions
            .iter()
            .any(|definition: &CodexModelDefinition| definition.id.eq_ignore_ascii_case(id))
        {
            continue;
        }
        let display_name = ["display_name", "displayName", "DisplayName"]
            .into_iter()
            .find_map(|key| item.get(key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let description = item
            .get("description")
            .or_else(|| item.get("Description"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let context_window = ["context_length", "contextLength", "ContextLength"]
            .into_iter()
            .find_map(|key| item.get(key).and_then(json_positive_u64));
        let reasoning_levels = item
            .get("thinking")
            .or_else(|| item.get("Thinking"))
            .and_then(|thinking| thinking.get("levels").or_else(|| thinking.get("Levels")))
            .and_then(serde_json::Value::as_array)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .map(str::to_ascii_lowercase)
                    .filter(|level| is_codex_reasoning_level(level))
                    .fold(Vec::new(), |mut result, level| {
                        if !result.contains(&level) {
                            result.push(level);
                        }
                        result
                    })
            })
            .unwrap_or_default();
        let supports_tools = item
            .get("supported_parameters")
            .or_else(|| item.get("supportedParameters"))
            .or_else(|| item.get("SupportedParameters"))
            .and_then(serde_json::Value::as_array)
            .map(|parameters| {
                parameters.iter().any(|parameter| {
                    parameter
                        .as_str()
                        .is_some_and(|value| value.eq_ignore_ascii_case("tools"))
                })
            });
        definitions.push(CodexModelDefinition {
            id: id.to_string(),
            display_name,
            description,
            context_window,
            reasoning_levels,
            supports_tools,
        });
    }
    Ok(definitions)
}

pub(crate) fn json_positive_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

pub(crate) fn is_codex_reasoning_level(value: &str) -> bool {
    matches!(
        value,
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    )
}

pub(crate) fn format_agent_models_error(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let message = value
            .get("error")
            .and_then(|error| {
                error
                    .as_str()
                    .or_else(|| error.get("message").and_then(serde_json::Value::as_str))
            })
            .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|message| !message.is_empty());
        if let Some(message) = message {
            return format!("Failed to get local model list ({status}): {message}");
        }
    }
    let body = body.trim();
    if body.is_empty() {
        format!("Failed to get local model list ({status})")
    } else {
        format!(
            "Failed to get local model list ({status}): {}",
            truncate_for_error(body)
        )
    }
}
