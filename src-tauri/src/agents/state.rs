use super::*;

#[cfg(test)]
pub(crate) fn agent_backup_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid agent configuration filename: {}", path_to_string(path)))?;
    Ok(path.with_file_name(format!("{file_name}.cpa-gui.backup")))
}

pub(crate) fn agent_state_path(paths: &[PathBuf]) -> Result<PathBuf, String> {
    let primary = paths
        .first()
        .ok_or_else(|| "No agent configuration path is available on the current platform".to_string())?;
    let file_name = primary
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid agent configuration filename: {}", path_to_string(primary)))?;
    Ok(primary.with_file_name(format!("{file_name}.cpa-gui.state.json")))
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn agent_managed_paths(client: AgentClient, home: &Path) -> Vec<PathBuf> {
    let paths = agent_config_paths(client, home);
    expected_agent_record_paths(client, &paths)
}

#[cfg(test)]
pub(crate) fn write_agent_applied_state(
    path: &Path,
    state: &AgentAppliedState,
) -> Result<(), String> {
    if state.client == AgentClient::Codex.id() {
        CODEX_APPLIED_STATES
            .lock()
            .map_err(|_| "Codex application state memory lock is poisoned".to_string())?
            .insert(path.to_path_buf(), state.clone());
        if path.is_file() {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "Failed to clean up legacy Codex application state {}: {error}",
                    path_to_string(path)
                )
            })?;
        }
        return Ok(());
    }

    let mut content = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Failed to generate agent application state: {error}"))?;
    content.push('\n');
    write_bytes_directly(path, content.as_bytes())
}

#[cfg(test)]
pub(crate) fn clear_codex_applied_state(path: &Path) -> Result<(), String> {
    CODEX_APPLIED_STATES
        .lock()
        .map_err(|_| "Codex application state memory lock is poisoned".to_string())?
        .remove(path);
    Ok(())
}

#[cfg(test)]
pub(crate) fn is_dated_agent_backup_name(file_name: &str, original_name: &str) -> bool {
    let Some(date) = file_name
        .strip_prefix(&format!("{original_name}."))
        .and_then(|value| value.strip_suffix(".bak"))
    else {
        return false;
    };
    let bytes = date.as_bytes();
    bytes.len() == 8
        && bytes[0..2].iter().all(u8::is_ascii_digit)
        && bytes[2] == b'-'
        && bytes[3..5].iter().all(u8::is_ascii_digit)
        && bytes[5] == b'-'
        && bytes[6..8].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
pub(crate) fn latest_dated_agent_backup_path(path: &Path) -> Result<Option<PathBuf>, String> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    if !directory.is_dir() {
        return Ok(None);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid agent configuration filename: {}", path_to_string(path)))?;
    let mut candidates = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "Failed to read agent backup directory {}: {error}",
                path_to_string(directory)
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| is_dated_agent_backup_name(value, file_name))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    Ok(candidates.pop())
}

#[cfg(test)]
pub(crate) fn recover_codex_applied_state_from_backups(
    paths: &[PathBuf],
) -> Result<Option<AgentAppliedState>, String> {
    let managed_paths = expected_agent_record_paths(AgentClient::Codex, paths);
    let mut discovered = Vec::with_capacity(managed_paths.len());
    let mut found_backup = false;
    for path in managed_paths {
        let backup = latest_dated_agent_backup_path(&path)?;
        found_backup |= backup.is_some();
        discovered.push((path, backup));
    }
    if !found_backup {
        return Ok(None);
    }

    let model = paths
        .first()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())
        .and_then(|root| {
            root.get("model")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let mut backup_files = Vec::with_capacity(discovered.len());
    for (path, backup) in discovered {
        let existed_before = backup.is_some();
        backup_files.push(AgentAppliedBackupFile {
            backup_path: match backup {
                Some(path) => path,
                None => dated_agent_backup_path(&path)?,
            },
            path,
            existed_before,
        });
    }
    let state = AgentAppliedState {
        version: AGENT_APPLIED_STATE_VERSION,
        client: AgentClient::Codex.id().to_string(),
        model,
        configuration_revision: AGENT_CONFIGURATION_REVISION,
        claude_desktop_model_mappings: None,
        backup_files,
        updated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    Ok(Some(state))
}

#[cfg(test)]
pub(crate) fn validate_agent_applied_state(
    client: AgentClient,
    paths: &[PathBuf],
    state: &AgentAppliedState,
) -> Result<(), String> {
    if state.client != client.id() {
        return Err("Agent application state does not match the client".to_string());
    }
    if state.backup_files.is_empty() {
        return Ok(());
    }
    let expected_paths = expected_agent_record_paths(client, paths);
    let state_paths = state
        .backup_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let codex_paths_without_auth = (client == AgentClient::Codex
        && expected_paths
            .last()
            .and_then(|path| path.file_name())
            .is_some_and(|name| name == "auth.json"))
    .then(|| &expected_paths[..expected_paths.len().saturating_sub(1)]);
    let valid_paths = state_paths == expected_paths
        || codex_paths_without_auth.is_some_and(|expected| state_paths == expected)
        || (client == AgentClient::Codex && state_paths.as_slice() == paths)
        || (client == AgentClient::ZCode
            && paths
                .first()
                .is_some_and(|path| state_paths == [path.clone()]));
    if !valid_paths {
        return Err("Agent application state file count or path mismatch".to_string());
    }
    for file in &state.backup_files {
        let legacy_backup = agent_backup_path(&file.path)?;
        let original_name = file
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("Invalid agent configuration filename: {}", path_to_string(&file.path)))?;
        let dated_backup = file.backup_path.parent() == file.path.parent()
            && file
                .backup_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| is_dated_agent_backup_name(name, original_name));
        if file.backup_path != legacy_backup && !dated_backup {
            return Err("Agent application state contains an unexpected backup path".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn load_agent_applied_state(
    client: AgentClient,
    home: &Path,
) -> Result<Option<AgentAppliedState>, String> {
    let paths = agent_config_paths(client, home);
    let state_path = agent_state_path(&paths)?;
    if client == AgentClient::Codex {
        if let Some(state) = CODEX_APPLIED_STATES
            .lock()
            .map_err(|_| "Codex application state memory lock is poisoned".to_string())?
            .get(&state_path)
            .cloned()
        {
            return Ok(Some(state));
        }
    }
    if !state_path.is_file() {
        let recovered = if client == AgentClient::Codex {
            recover_codex_applied_state_from_backups(&paths)?
        } else {
            None
        };
        if let Some(state) = recovered.as_ref() {
            write_agent_applied_state(&state_path, state)?;
        }
        return Ok(recovered);
    }
    let content = fs::read_to_string(&state_path)
        .map_err(|error| format!("Failed to read agent application state: {error}"))?;
    let value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| format!("Failed to parse agent application state: {error}"))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Agent application state is missing a version number".to_string())?;
    let version = u8::try_from(version).map_err(|_| "Unsupported agent application state version".to_string())?;
    if version == AGENT_APPLIED_STATE_VERSION || version == 3 {
        let state = serde_json::from_value::<AgentAppliedState>(value)
            .map_err(|error| format!("Failed to parse agent application state: {error}"))?;
        validate_agent_applied_state(client, &paths, &state)?;
        if client == AgentClient::Codex {
            write_agent_applied_state(&state_path, &state)?;
        }
        return Ok(Some(state));
    }
    if version != LEGACY_AGENT_MODIFICATION_STATE_VERSION
        && version != AGENT_MODIFICATION_STATE_VERSION
    {
        return Err("Unsupported agent application state version".to_string());
    }

    let record = serde_json::from_value::<AgentModificationRecord>(value)
        .map_err(|error| format!("Failed to parse legacy agent state: {error}"))?;
    validate_agent_record(client, &paths, &record)?;
    let backup_files = record
        .files
        .iter()
        .map(|file| AgentAppliedBackupFile {
            path: file.path.clone(),
            backup_path: file.backup_path.clone(),
            existed_before: file.existed_before,
        })
        .collect();
    let state = AgentAppliedState {
        version: AGENT_APPLIED_STATE_VERSION,
        client: client.id().to_string(),
        model: record.model,
        configuration_revision: 0,
        claude_desktop_model_mappings: None,
        backup_files,
        updated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    write_agent_applied_state(&state_path, &state)?;
    Ok(Some(state))
}

pub(crate) fn read_agent_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    validate_config_path(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(format!("Failed to read agent configuration: {}. Check file permissions or whether the file is in use", path_to_string(path))),
    }
}

#[cfg(test)]
pub(crate) fn read_agent_original_bytes(
    file: &AgentModificationFile,
) -> Result<Option<Vec<u8>>, String> {
    if !file.existed_before {
        return Ok(None);
    }
    let original_sha256 = file
        .original_sha256
        .as_deref()
        .ok_or_else(|| format!("Original configuration backup is missing a checksum: {}", path_to_string(&file.path)))?;
    let bytes = fs::read(&file.backup_path).map_err(|error| {
        format!(
            "Failed to read original configuration backup {}: {error}",
            path_to_string(&file.backup_path)
        )
    })?;
    if sha256_bytes(&bytes) != original_sha256 {
        return Err(format!(
            "Original configuration backup verification failed: {}",
            path_to_string(&file.backup_path)
        ));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
pub(crate) fn read_agent_original_text(
    file: &AgentModificationFile,
) -> Result<Option<String>, String> {
    read_agent_original_bytes(file)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                format!(
                    "Original Codex configuration is not UTF-8 text: {}",
                    path_to_string(&file.path)
                )
            })
        })
        .transpose()
}

#[cfg(test)]
pub(crate) fn write_agent_state(
    path: &Path,
    record: &AgentModificationRecord,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("Failed to create agent state directory {}: {error}", path_to_string(parent))
        })?;
    }
    let mut content = serde_json::to_string_pretty(record)
        .map_err(|error| format!("Failed to generate agent backup state: {error}"))?;
    content.push('\n');
    write_yaml_if_changed(path, &content).map(|_| ())
}

#[cfg(test)]
pub(crate) fn validate_agent_record(
    client: AgentClient,
    paths: &[PathBuf],
    record: &AgentModificationRecord,
) -> Result<(), String> {
    let supported_version = record.version == AGENT_MODIFICATION_STATE_VERSION
        || record.version == LEGACY_AGENT_MODIFICATION_STATE_VERSION;
    if !supported_version || record.client != client.id() {
        return Err("Agent backup state version or client mismatch".to_string());
    }
    if ![
        AGENT_PHASE_APPLYING,
        AGENT_PHASE_ACTIVE,
        AGENT_PHASE_RESTORING,
        AGENT_PHASE_RECOVERY,
    ]
    .contains(&record.phase.as_str())
    {
        return Err("Invalid agent backup state stage".to_string());
    }
    let expected_paths = expected_agent_record_paths(client, paths);
    let record_paths = record
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let valid_paths = record_paths.as_slice() == paths
        || (client == AgentClient::Codex && record_paths == expected_paths)
        || (client == AgentClient::ZCode
            && paths
                .first()
                .is_some_and(|path| record_paths == [path.clone()]));
    if !valid_paths {
        return Err("Agent backup state file count or path mismatch".to_string());
    }
    for file in &record.files {
        if file.backup_path != agent_backup_path(&file.path)? {
            return Err("Agent backup state contains an unexpected path".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn load_agent_record(
    client: AgentClient,
    paths: &[PathBuf],
) -> Result<Option<AgentModificationRecord>, String> {
    let state_path = agent_state_path(paths)?;
    if !state_path.is_file() {
        return Ok(None);
    }
    let mut record: AgentModificationRecord = serde_json::from_str(
        &fs::read_to_string(&state_path)
            .map_err(|error| format!("Failed to read agent backup state: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse agent backup state: {error}"))?;
    validate_agent_record(client, paths, &record)?;
    if record.version == LEGACY_AGENT_MODIFICATION_STATE_VERSION {
        record.version = AGENT_MODIFICATION_STATE_VERSION;
    }
    Ok(Some(record))
}

#[cfg(test)]
pub(crate) fn record_backup_available(record: &AgentModificationRecord) -> bool {
    record
        .files
        .iter()
        .all(|file| !file.existed_before || file.backup_path.is_file())
}

#[cfg(test)]
pub(crate) fn record_conflict_files(
    record: &AgentModificationRecord,
) -> Result<Vec<String>, String> {
    let mut conflicts = Vec::new();
    for file in &record.files {
        let current = read_agent_bytes(&file.path)?;
        let matches =
            current.as_deref().map(sha256_bytes).as_deref() == Some(file.managed_sha256.as_str());
        if !matches {
            conflicts.push(path_to_string(&file.path));
        }
    }
    Ok(conflicts)
}

#[cfg(test)]
pub(crate) fn record_restore_conflict_files(
    record: &AgentModificationRecord,
) -> Result<Vec<String>, String> {
    let mut conflicts = Vec::new();
    for file in &record.files {
        let current = read_agent_bytes(&file.path)?;
        let matches_managed =
            current.as_deref().map(sha256_bytes).as_deref() == Some(file.managed_sha256.as_str());
        let matches_original = if file.existed_before {
            current.as_deref().map(sha256_bytes) == file.original_sha256
        } else {
            current.is_none()
        };
        if !matches_managed && !matches_original {
            conflicts.push(path_to_string(&file.path));
        }
    }
    Ok(conflicts)
}

#[cfg(test)]
pub(crate) fn record_matches_original(record: &AgentModificationRecord) -> Result<bool, String> {
    for file in &record.files {
        let current = read_agent_bytes(&file.path)?;
        let matches = if file.existed_before {
            current.as_deref().map(sha256_bytes) == file.original_sha256
        } else {
            current.is_none()
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
pub(crate) fn codex_record_needs_resync(
    record: &AgentModificationRecord,
    configured: bool,
    current_model: Option<&str>,
) -> Result<bool, String> {
    if !configured || current_model != Some(record.model.as_str()) {
        return Ok(true);
    }
    for file in record.files.iter().skip(1) {
        let current_sha256 = read_agent_bytes(&file.path)?.as_deref().map(sha256_bytes);
        if current_sha256.as_deref() != Some(file.managed_sha256.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
pub(crate) fn inspect_agent_modification(
    client: AgentClient,
    home: &Path,
    port: u16,
    configured: bool,
    current_model: Option<&str>,
) -> AgentModificationInspection {
    let paths = agent_config_paths(client, home);
    let state_path = match agent_state_path(&paths) {
        Ok(path) => path,
        Err(_) => {
            return AgentModificationInspection {
                enabled: false,
                state: "inactive".to_string(),
                backup_available: false,
                warnings: Vec::new(),
            }
        }
    };

    if state_path.is_file() {
        return match load_agent_record(client, &paths) {
            Ok(Some(record)) => {
                let backup_available = record_backup_available(&record);
                let state = if record.phase != AGENT_PHASE_ACTIVE {
                    "recovery"
                } else if client == AgentClient::Codex {
                    "active"
                } else {
                    match record_conflict_files(&record) {
                        Ok(conflicts) if conflicts.is_empty() => "active",
                        Ok(_) => AGENT_MODIFICATION_STATE_CONFLICT,
                        Err(_) => "recovery",
                    }
                };
                let mut warnings = Vec::new();
                if !backup_available {
                    warnings.push("Original configuration backup is incomplete. Do not delete the remaining backup files before restoration".to_string());
                }
                if state == AGENT_MODIFICATION_STATE_CONFLICT {
                    warnings.push("Configuration was modified by another program. Confirmation is required before restoring it when clearing changes".to_string());
                } else if state == "recovery" {
                    warnings.push("The last configuration operation did not complete. Use “Clear Changes” to restore the original configuration".to_string());
                } else if client == AgentClient::Codex
                    && codex_record_needs_resync(&record, configured, current_model).unwrap_or(true)
                {
                    warnings.push(
                        "Managed Codex configuration has changed and will be resynchronized automatically on the next update or launch".to_string(),
                    );
                }
                AgentModificationInspection {
                    enabled: true,
                    state: state.to_string(),
                    backup_available,
                    warnings,
                }
            }
            Ok(None) => AgentModificationInspection {
                enabled: false,
                state: "inactive".to_string(),
                backup_available: false,
                warnings: Vec::new(),
            },
            Err(error) => AgentModificationInspection {
                enabled: true,
                state: "recovery".to_string(),
                backup_available: expected_agent_record_paths(client, &paths)
                    .iter()
                    .filter_map(|path| agent_backup_path(path).ok())
                    .any(|path| path.is_file()),
                warnings: vec![error],
            },
        };
    }

    if configured {
        if let Some(model) = current_model {
            match build_legacy_agent_record(client, home, port, model) {
                Ok(Some(_)) => {
                    return AgentModificationInspection {
                        enabled: true,
                        state: "active".to_string(),
                        backup_available: true,
                        warnings: vec![
                            "Legacy CPA configuration and backup detected. Use “Clear Changes” to restore the original configuration".to_string()
                        ],
                    }
                }
                Ok(None) => {
                    return AgentModificationInspection {
                        enabled: false,
                        state: "inactive".to_string(),
                        backup_available: false,
                        warnings: vec!["CPA configuration detected, but no original backup is available for safe restoration".to_string()],
                    }
                }
                Err(error) => {
                    return AgentModificationInspection {
                        enabled: false,
                        state: "inactive".to_string(),
                        backup_available: false,
                        warnings: vec![error],
                    }
                }
            }
        }
    }

    AgentModificationInspection {
        enabled: false,
        state: "inactive".to_string(),
        backup_available: false,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn fresh_agent_contents(
    client: AgentClient,
    port: u16,
    api_key: &str,
    model: &str,
) -> Result<Vec<String>, String> {
    let models = [AgentModelOption {
        input_modalities: None,
        harness_metadata: None,
        name: model.to_string(),
        alias: None,
        is_alias: false,
        context_window: Some(200_000),
    }];
    fresh_agent_contents_with_oauth(
        client,
        port,
        api_key,
        model,
        AgentConfigurationOptions {
            models: &models,
            codex_catalog: None,
            oauth_configuration: false,
            claude_code_model_mappings: None,
            claude_desktop_model_mappings: None,
        },
    )
}

pub(crate) fn fresh_agent_contents_with_oauth(
    client: AgentClient,
    port: u16,
    api_key: &str,
    model: &str,
    options: AgentConfigurationOptions<'_>,
) -> Result<Vec<String>, String> {
    let AgentConfigurationOptions {
        models,
        oauth_configuration,
        claude_code_model_mappings,
        claude_desktop_model_mappings,
        ..
    } = options;
    let root_base = managed_core_loopback_origin(port);
    let openai_base = format!("{root_base}/v1");
    match client {
        AgentClient::ClaudeCode => Ok(vec![build_claude_agent_config(
            None,
            &root_base,
            api_key,
            model,
            models,
            claude_code_model_mappings,
        )?]),
        AgentClient::ClaudeDesktop => Ok(vec![
            build_claude_desktop_deployment_config(None)?,
            build_claude_desktop_deployment_config(None)?,
            build_claude_desktop_profile(
                None,
                &root_base,
                api_key,
                model,
                models,
                claude_desktop_model_mappings,
            )?,
            build_claude_desktop_meta(None)?,
        ]),
        AgentClient::Codex => Ok(vec![build_codex_agent_config_with_oauth(
            None,
            &openai_base,
            api_key,
            model,
            oauth_configuration,
        )?]),
        AgentClient::OpenCode => Ok(vec![build_opencode_agent_config(
            None,
            &openai_base,
            api_key,
            model,
            models,
        )?]),
        AgentClient::OpenClaw => Ok(vec![build_openclaw_agent_config(
            None,
            &openai_base,
            api_key,
            model,
            models,
        )?]),
        AgentClient::Hermes => Ok(vec![build_hermes_agent_config(
            None,
            &openai_base,
            api_key,
            model,
            models,
        )?]),
        AgentClient::DeepSeekHarness => Ok(vec![
            build_deepseek_harness_settings(None, &openai_base, model, models)?,
            build_deepseek_harness_credentials(None, api_key)?,
        ]),
        AgentClient::AntigravityCli => Err("Antigravity template requires installation and user directory information".into()),
        AgentClient::WorkBuddy => Ok(vec![build_workbuddy_agent_config(
            None, &openai_base, api_key, model, models,
        )?]),
        AgentClient::ZCode => Ok(vec![build_zcode_agent_config(
            None, &openai_base, api_key, model, models,
        )?]),
        AgentClient::KimiCode => Ok(vec![build_kimi_code_agent_config(
            None,
            &openai_base,
            api_key,
            model,
            models,
        )?]),
        AgentClient::GrokBuild => Ok(vec![build_grok_build_agent_config(
            None,
            &openai_base,
            api_key,
            model,
            models,
        )?]),
        AgentClient::Omp => Ok(vec![
            build_omp_models_config(None, &openai_base, api_key)?,
            build_omp_settings_config(None, model)?,
        ]),
    }
}

#[cfg(test)]
pub(crate) fn agent_contents_equal(client: AgentClient, actual: &str, expected: &str) -> bool {
    match client {
        AgentClient::Codex => {
            normalize_codex_config_for_legacy_compare(actual)
                == normalize_codex_config_for_legacy_compare(expected)
        }
        AgentClient::KimiCode | AgentClient::GrokBuild => {
            toml::from_str::<toml::Value>(actual).ok()
                == toml::from_str::<toml::Value>(expected).ok()
        }
        AgentClient::OpenClaw => {
            json5::from_str::<serde_json::Value>(actual).ok()
                == json5::from_str::<serde_json::Value>(expected).ok()
        }
        AgentClient::Hermes => {
            serde_yaml::from_str::<serde_yaml::Value>(actual).ok()
                == serde_yaml::from_str::<serde_yaml::Value>(expected).ok()
        }
        AgentClient::Omp | AgentClient::DeepSeekHarness => {
            serde_norway::from_str::<serde_norway::Value>(actual).ok()
                == serde_norway::from_str::<serde_norway::Value>(expected).ok()
        }
        _ => {
            serde_json::from_str::<serde_json::Value>(actual).ok()
                == serde_json::from_str::<serde_json::Value>(expected).ok()
        }
    }
}

#[cfg(test)]
pub(crate) fn normalize_codex_config_for_legacy_compare(content: &str) -> Option<toml::Value> {
    let mut value = toml::from_str::<toml::Value>(content).ok()?;
    if value
        .get("model_catalog_json")
        .and_then(toml::Value::as_str)
        == Some(CODEX_MODEL_CATALOG_FILE)
    {
        value.as_table_mut()?.remove("model_catalog_json");
    }
    Some(value)
}

#[cfg(test)]
pub(crate) fn build_legacy_agent_record(
    client: AgentClient,
    home: &Path,
    port: u16,
    model: &str,
) -> Result<Option<AgentModificationRecord>, String> {
    let paths = agent_config_paths(client, home);
    let generated = fresh_agent_contents(client, port, DEFAULT_API_KEY, model)?;
    if generated.len() != paths.len() {
        return Ok(None);
    }
    let mut files = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let current = read_agent_bytes(path)?;
        let Some(current) = current else {
            return Ok(None);
        };
        let backup_path = agent_backup_path(path)?;
        let (existed_before, original_sha256) = if backup_path.is_file() {
            let backup = fs::read(&backup_path).map_err(|error| {
                format!(
                    "Failed to read legacy agent backup {}: {error}",
                    path_to_string(&backup_path)
                )
            })?;
            (true, Some(sha256_bytes(&backup)))
        } else {
            let actual = String::from_utf8(current.clone())
                .map_err(|_| format!("Agent configuration is not UTF-8 text: {}", path_to_string(path)))?;
            if !agent_contents_equal(client, &actual, &generated[index]) {
                return Ok(None);
            }
            (false, None)
        };
        files.push(AgentModificationFile {
            path: path.clone(),
            backup_path,
            existed_before,
            original_sha256,
            managed_sha256: sha256_bytes(&current),
        });
    }
    Ok(Some(AgentModificationRecord {
        version: AGENT_MODIFICATION_STATE_VERSION,
        client: client.id().to_string(),
        phase: AGENT_PHASE_ACTIVE.to_string(),
        model: model.to_string(),
        files,
    }))
}

#[cfg(test)]
pub(crate) fn prepare_agent_record(
    client: AgentClient,
    paths: &[PathBuf],
    model: &str,
    updates: &[AgentFileUpdate],
) -> Result<AgentModificationRecord, String> {
    if paths.len() != updates.len() {
        return Err("Agent configuration update file count mismatch".to_string());
    }
    let mut prepared = Vec::new();
    for (path, update) in paths.iter().zip(updates) {
        if path != &update.path {
            return Err("Agent configuration update path mismatch".to_string());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create agent configuration directory {}: {error}", path_to_string(parent))
            })?;
        }
        let backup_path = agent_backup_path(path)?;
        let current = read_agent_bytes(path)?;
        let previous_backup = if backup_path.exists() {
            if !backup_path.is_file() {
                return Err(format!(
                    "Agent backup path is not a file: {}",
                    path_to_string(&backup_path)
                ));
            }
            Some(fs::read(&backup_path).map_err(|error| {
                format!(
                    "Failed to read existing agent backup {}: {error}",
                    path_to_string(&backup_path)
                )
            })?)
        } else {
            None
        };

        prepared.push((
            path.clone(),
            backup_path,
            current,
            previous_backup,
            sha256_bytes(update.after.as_bytes()),
        ));
    }

    let backup_snapshots = prepared
        .iter()
        .map(|(_, backup_path, _, previous_backup, _)| {
            (backup_path.clone(), previous_backup.clone())
        })
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    for (path, backup_path, current, _, managed_sha256) in prepared {
        let backup_result = if let Some(current) = current.as_deref() {
            write_bytes_atomically(&backup_path, current).and_then(|_| {
                let copied = fs::read(&backup_path).map_err(|error| {
                    format!(
                        "Failed to verify agent backup {}: {error}",
                        path_to_string(&backup_path)
                    )
                })?;
                if sha256_bytes(&copied) != sha256_bytes(current) {
                    return Err(format!("Agent backup verification failed: {}", path_to_string(&path)));
                }
                Ok(())
            })
        } else if backup_path.exists() {
            fs::remove_file(&backup_path).map_err(|error| {
                format!(
                    "Failed to clean up old agent backup {}: {error}",
                    path_to_string(&backup_path)
                )
            })
        } else {
            Ok(())
        };
        if let Err(error) = backup_result {
            let rollback = restore_snapshots(&backup_snapshots);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; failed to restore existing backup: {rollback_error}"),
            });
        }

        let existed_before = current.is_some();
        let original_sha256 = current.as_deref().map(sha256_bytes);
        files.push(AgentModificationFile {
            path,
            backup_path,
            existed_before,
            original_sha256,
            managed_sha256,
        });
    }
    Ok(AgentModificationRecord {
        version: AGENT_MODIFICATION_STATE_VERSION,
        client: client.id().to_string(),
        phase: AGENT_PHASE_APPLYING.to_string(),
        model: model.to_string(),
        files,
    })
}

#[cfg(test)]
pub(crate) fn extend_agent_record_for_updates(
    record: &AgentModificationRecord,
    updates: &[AgentFileUpdate],
) -> Result<AgentRecordExtension, String> {
    if record.files.len() > updates.len() {
        return Err("Agent configuration update file count mismatch".to_string());
    }
    for (file, update) in record.files.iter().zip(updates) {
        if file.path != update.path {
            return Err("Agent configuration update path mismatch".to_string());
        }
    }
    if record.files.len() == updates.len() {
        return Ok((record.clone(), Vec::new()));
    }

    let mut prepared = Vec::new();
    for update in updates.iter().skip(record.files.len()) {
        if let Some(parent) = update.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create agent configuration directory {}: {error}", path_to_string(parent))
            })?;
        }
        let backup_path = agent_backup_path(&update.path)?;
        let previous_backup = if backup_path.exists() {
            if !backup_path.is_file() {
                return Err(format!(
                    "Agent backup path is not a file: {}",
                    path_to_string(&backup_path)
                ));
            }
            Some(fs::read(&backup_path).map_err(|error| {
                format!(
                    "Failed to read existing agent backup {}: {error}",
                    path_to_string(&backup_path)
                )
            })?)
        } else {
            None
        };
        let current = read_agent_bytes(&update.path)?;
        prepared.push((update, backup_path, previous_backup, current));
    }

    let backup_snapshots = prepared
        .iter()
        .map(|(_, backup_path, previous_backup, _)| (backup_path.clone(), previous_backup.clone()))
        .collect::<Vec<_>>();
    let mut next = record.clone();
    for (update, backup_path, _, current) in prepared {
        let backup_result = if let Some(current) = current.as_deref() {
            write_bytes_atomically(&backup_path, current).and_then(|_| {
                let copied = fs::read(&backup_path).map_err(|error| {
                    format!(
                        "Failed to verify agent backup {}: {error}",
                        path_to_string(&backup_path)
                    )
                })?;
                if sha256_bytes(&copied) != sha256_bytes(current) {
                    return Err(format!(
                        "Agent backup verification failed: {}",
                        path_to_string(&update.path)
                    ));
                }
                Ok(())
            })
        } else if backup_path.exists() {
            fs::remove_file(&backup_path).map_err(|error| {
                format!(
                    "Failed to clean up old agent backup {}: {error}",
                    path_to_string(&backup_path)
                )
            })
        } else {
            Ok(())
        };
        if let Err(error) = backup_result {
            let rollback = restore_snapshots(&backup_snapshots);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; failed to restore existing backup: {rollback_error}"),
            });
        }
        next.files.push(AgentModificationFile {
            path: update.path.clone(),
            backup_path,
            existed_before: current.is_some(),
            original_sha256: current.as_deref().map(sha256_bytes),
            managed_sha256: sha256_bytes(update.after.as_bytes()),
        });
    }
    Ok((next, backup_snapshots))
}

#[cfg(test)]
pub(crate) fn restore_snapshots(snapshots: &[FileSnapshot]) -> Result<(), String> {
    restore_snapshots_with_direct_path(snapshots, None)
}

#[cfg(test)]
pub(crate) fn write_agent_bytes(
    path: &Path,
    content: &[u8],
    direct_write_path: Option<&Path>,
) -> Result<(), String> {
    if direct_write_path == Some(path) {
        write_bytes_directly(path, content)
    } else {
        write_bytes_atomically(path, content)
    }
}

#[cfg(test)]
pub(crate) fn restore_snapshots_with_direct_path(
    snapshots: &[FileSnapshot],
    direct_write_path: Option<&Path>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for (path, bytes) in snapshots.iter().rev() {
        let result = match bytes {
            Some(bytes) => write_agent_bytes(path, bytes, direct_write_path),
            None if path.exists() => fs::remove_file(path)
                .map_err(|error| format!("Failed to delete configuration {}: {error}", path_to_string(path))),
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
pub(crate) fn apply_agent_file_replacements(
    replacements: &[(PathBuf, Option<Vec<u8>>)],
    direct_write_path: Option<&Path>,
) -> Result<Vec<String>, String> {
    let snapshots = replacements
        .iter()
        .map(|(path, _)| Ok((path.clone(), read_agent_bytes(path)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut changed = Vec::new();
    for (path, replacement) in replacements {
        let current = read_agent_bytes(path)?;
        let result = match replacement {
            Some(bytes) if current.as_deref() == Some(bytes.as_slice()) => Ok(()),
            Some(bytes) => {
                changed.push(path_to_string(path));
                write_agent_bytes(path, bytes, direct_write_path)
            }
            None if current.is_some() => {
                changed.push(path_to_string(path));
                fs::remove_file(path)
                    .map_err(|error| format!("Failed to delete configuration {}: {error}", path_to_string(path)))
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            let rollback = restore_snapshots_with_direct_path(&snapshots, direct_write_path);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
            });
        }
    }
    Ok(changed)
}

#[cfg(test)]
pub(crate) fn restore_codex_agent_record_files(
    record: &AgentModificationRecord,
) -> Result<Vec<String>, String> {
    let config_file = record
        .files
        .first()
        .ok_or_else(|| "Codex configuration state is missing config.toml".to_string())?;
    let current_config = read_agent_bytes(&config_file.path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                format!(
                    "Current Codex configuration is not UTF-8 text: {}",
                    path_to_string(&config_file.path)
                )
            })
        })
        .transpose()?;
    let original_config = read_agent_original_text(config_file)?;
    let restored_config =
        build_restored_codex_agent_config(current_config.as_deref(), original_config.as_deref())?;

    let mut replacements = vec![(
        config_file.path.clone(),
        restored_config.map(String::into_bytes),
    )];
    for file in record.files.iter().skip(1) {
        replacements.push((file.path.clone(), read_agent_original_bytes(file)?));
    }
    apply_agent_file_replacements(&replacements, Some(&config_file.path))
}

#[cfg(test)]
pub(crate) fn apply_agent_updates(
    client: AgentClient,
    updates: &[AgentFileUpdate],
) -> Result<Vec<String>, String> {
    let direct_write_path = if client == AgentClient::Codex {
        updates.first().map(|update| update.path.as_path())
    } else {
        None
    };
    let snapshots = updates
        .iter()
        .map(|update| Ok((update.path.clone(), read_agent_bytes(&update.path)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut changed = Vec::new();
    for update in updates {
        let next = update.after.as_bytes();
        if read_agent_bytes(&update.path)?.as_deref() == Some(next) {
            continue;
        }
        if let Err(error) = write_agent_bytes(&update.path, next, direct_write_path) {
            let rollback = restore_snapshots_with_direct_path(&snapshots, direct_write_path);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
            });
        }
        changed.push(path_to_string(&update.path));
    }
    Ok(changed)
}

#[cfg(test)]
pub(crate) fn restore_agent_snapshots_direct(snapshots: &[FileSnapshot]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (path, content) in snapshots.iter().rev() {
        let result = match content {
            Some(content) => write_bytes_directly(path, content),
            None if path.exists() => fs::remove_file(path)
                .map_err(|error| format!("Failed to delete newly created configuration {}: {error}", path_to_string(path))),
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
pub(crate) fn dated_agent_backup_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid agent configuration filename: {}", path_to_string(path)))?;
    let date = chrono::Local::now().format("%y-%m-%d");
    Ok(path.with_file_name(format!("{file_name}.{date}.bak")))
}

#[cfg(test)]
pub(crate) fn restore_agent_session_configuration(
    client: AgentClient,
    home: &Path,
) -> Result<(), String> {
    let paths = agent_config_paths(client, home);
    let state_path = agent_state_path(&paths)?;
    let Some(state) = load_agent_applied_state(client, home)? else {
        return Ok(());
    };
    restore_agent_applied_state_configuration(client, &paths, &state_path, &state)
}

#[cfg(test)]
pub(crate) fn restore_agent_applied_state_configuration(
    client: AgentClient,
    paths: &[PathBuf],
    state_path: &Path,
    state: &AgentAppliedState,
) -> Result<(), String> {
    validate_agent_applied_state(client, paths, state)?;
    if state.backup_files.is_empty() {
        if agent_has_managed_marker(client, paths)? {
            remove_agent_managed_configuration(client, paths)?;
        }
    } else {
        let originals = state
            .backup_files
            .iter()
            .map(|file| {
                if file.existed_before {
                    fs::read(&file.backup_path).map(Some).map_err(|error| {
                        format!(
                            "Failed to read agent backup {}: {error}",
                            path_to_string(&file.backup_path)
                        )
                    })
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        let snapshots = state
            .backup_files
            .iter()
            .map(|file| Ok((file.path.clone(), read_agent_bytes(&file.path)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let replacements = state
            .backup_files
            .iter()
            .zip(&originals)
            .zip(&snapshots)
            .map(|((file, original), (_, current))| {
                build_agent_session_restored_bytes(
                    client,
                    paths,
                    &file.path,
                    current.as_deref(),
                    original.as_deref(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let restore_result = (|| -> Result<(), String> {
            for (file, replacement) in state.backup_files.iter().zip(&replacements).rev() {
                if let Some(replacement) = replacement {
                    write_agent_configuration_file(client, &file.path, replacement)?;
                } else if file.path.exists() {
                    fs::remove_file(&file.path).map_err(|error| {
                        format!(
                            "Failed to delete temporary agent configuration {}: {error}",
                            path_to_string(&file.path)
                        )
                    })?;
                }
            }
            Ok(())
        })();
        if let Err(error) = restore_result {
            return match restore_agent_snapshots_direct(&snapshots) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!("{error}; failed to roll back restoration: {rollback_error}")),
            };
        }
        for file in &state.backup_files {
            if file.existed_before && file.backup_path.is_file() {
                fs::remove_file(&file.backup_path).map_err(|error| {
                    format!(
                        "Failed to clean up agent backup {}: {error}",
                        path_to_string(&file.backup_path)
                    )
                })?;
            }
        }
    }
    if client == AgentClient::Codex {
        clear_codex_applied_state(state_path)?;
    }
    if state_path.is_file() {
        fs::remove_file(state_path).map_err(|error| {
            format!(
                "Failed to clean up agent application state {}: {error}",
                path_to_string(state_path)
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn commit_agent_configuration(
    client: AgentClient,
    home: &Path,
    model: &str,
    updates: &[AgentFileUpdate],
    outcome: &str,
    _claude_desktop_model_mappings: Option<&ClaudeDesktopModelMappings>,
) -> Result<AgentConfigActionResult, String> {
    let paths = config_paths(client.id(), home)?;
    let before = config_images(&paths)?;
    config_updates(client.id(), home, &before, updates, outcome, Some(model.to_string()), _claude_desktop_model_mappings)
}

pub(crate) fn sync_codex_model_catalog_if_configured(
    home: &Path,
    port: u16,
    api_key: &str,
    models: &[AgentModelOption],
    catalog: &str,
) -> Result<bool, String> {
    let client = AgentClient::Codex;
    let paths = agent_config_paths(client, home);
    let _guard = AGENT_CONFIG_FILE_LOCK
        .lock()
        .map_err(|_| "Agent configuration file lock is poisoned".to_string())?;
    if codex_native_oauth_enabled(home)? { return Ok(false); }
    let before = config_images(&config_paths(client.id(), home)?)?;
    validate_config_images(&before)?;
    let (configured, current_model, _) =
        inspect_agent_managed_config(client, &paths, port, api_key)?;
    if !configured {
        return Ok(false);
    }
    let next_model = models
        .iter()
        .find(|model| {
            current_model
                .as_deref()
                .is_some_and(|current| model.name.eq_ignore_ascii_case(current))
        })
        .or_else(|| models.first())
        .map(|model| model.name.as_str());
    if let Some(model) = next_model {
        validate_codex_catalog(catalog, model)?;
    } else {
        let root: serde_json::Value = serde_json::from_str(catalog)
            .map_err(|error| format!("Invalid Codex model catalog format: {error}"))?;
        if !root
            .get("models")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            return Err("An empty model list must correspond to an empty Codex model catalog".to_string());
        }
    }

    let catalog_path = codex_model_catalog_path(home);
    let mut updates = Vec::new();
    if read_agent_bytes(&catalog_path)?.as_deref() != Some(catalog.as_bytes()) {
        updates.push(AgentFileUpdate {
            path: catalog_path,
            after: catalog.to_string(),
        });
    }
    if current_model.as_deref() != next_model {
        let content = fs::read_to_string(&paths[0])
            .map_err(|error| format!("Failed to read Codex configuration: {error}"))?;
        let mut document = content
            .parse::<toml_edit::Document>()
            .map_err(|error| format!("Invalid Codex config.toml format: {error}"))?;
        if let Some(model) = next_model {
            set_codex_table_item(document.as_table_mut(), "model", toml_edit::value(model));
        } else {
            document.remove("model");
        }
        updates.push(AgentFileUpdate {
            path: paths[0].clone(),
            after: document.to_string(),
        });
    }
    if updates.is_empty() {
        return Ok(false);
    }

    let result = config_updates(client.id(), home, &before, &updates, "sync", next_model.map(str::to_string), None)?;
    Ok(!result.changed_files.is_empty())
}

#[cfg(test)]
pub(crate) fn apply_agent_configuration(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    model: &str,
    models: &[AgentModelOption],
    codex_catalog: Option<&str>,
) -> Result<AgentConfigActionResult, String> {
    apply_agent_configuration_with_oauth(
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

pub(crate) fn apply_agent_configuration_with_oauth(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    model: &str,
    options: AgentConfigurationOptions<'_>,
) -> Result<AgentConfigActionResult, String> {
    if client == AgentClient::Codex {
        return apply_codex_cpa_configuration(home, port, api_key, model, options);
    }
    if client == AgentClient::DeepSeekHarness {
        return apply_deepseek_harness_configuration(home, port, api_key, model, options.models);
    }
    let before = config_images(&config_paths(client.id(), home)?)?;
    validate_config_images(&before)?;
    let mappings = options.claude_desktop_model_mappings;
    let updates = build_agent_updates_with_oauth(client, home, port, api_key, model, options)
        .map_err(|error| {
            if client == AgentClient::WorkBuddy {
                error
            } else {
                "Failed to build configuration. Check its structure, restore from a manual backup, or repair it with the base configuration template".to_string()
            }
        })?;
    config_updates(client.id(), home, &before, &updates, "update", Some(model.to_string()), mappings)
}

#[cfg(test)]
pub(crate) fn reset_agent_configuration_to_default(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    model: &str,
    codex_catalog: Option<&str>,
) -> Result<AgentConfigActionResult, String> {
    reset_agent_configuration_to_default_with_oauth(AgentDefaultConfiguration {
        client,
        home,
        port,
        api_key,
        model,
        models: &[AgentModelOption {
            input_modalities: None,
            harness_metadata: None,
            name: model.to_string(),
            alias: None,
            is_alias: false,
            context_window: Some(200_000),
        }],
        codex_catalog,
        oauth_configuration: false,
        claude_code_model_mappings: None,
        claude_desktop_model_mappings: None,
    })
}

pub(crate) struct AgentDefaultConfiguration<'a> {
    pub(crate) client: AgentClient,
    pub(crate) home: &'a Path,
    pub(crate) port: u16,
    pub(crate) api_key: &'a str,
    pub(crate) model: &'a str,
    pub(crate) models: &'a [AgentModelOption],
    pub(crate) codex_catalog: Option<&'a str>,
    pub(crate) oauth_configuration: bool,
    pub(crate) claude_code_model_mappings: Option<&'a ClaudeDesktopModelMappings>,
    pub(crate) claude_desktop_model_mappings: Option<&'a ClaudeDesktopModelMappings>,
}

#[cfg(test)]
pub(crate) fn reset_agent_configuration_to_default_with_oauth(request: AgentDefaultConfiguration<'_>) -> Result<AgentConfigActionResult, String> {
    let client = request.client;
    let home = request.home;
    let model = request.model.to_string();
    let mappings = request.claude_desktop_model_mappings.cloned();
    let before = config_images(&config_paths(client.id(), home)?)?;
    let updates = build_agent_template_updates(request)?;
    config_updates(client.id(), home, &before, &updates, "template", Some(model), mappings.as_ref())
}

pub(crate) fn build_agent_template_updates(
    request: AgentDefaultConfiguration<'_>,
) -> Result<Vec<AgentFileUpdate>, String> {
    let AgentDefaultConfiguration {
        client,
        home,
        port,
        api_key,
        model,
        models,
        codex_catalog,
        oauth_configuration,
        claude_code_model_mappings,
        claude_desktop_model_mappings,
    } = request;
    if client == AgentClient::AntigravityCli {
        return build_antigravity_updates(client, home, &managed_core_loopback_origin(port), api_key, model, true);
    }
    let paths = agent_config_paths(client, home);
    let contents = fresh_agent_contents_with_oauth(
        client,
        port,
        api_key,
        model,
        AgentConfigurationOptions {
            models,
            codex_catalog,
            oauth_configuration,
            claude_code_model_mappings,
            claude_desktop_model_mappings,
        },
    )?;
    if paths.len() != contents.len() {
        return Err("Base configuration template file count mismatch".to_string());
    }
    let mut updates = paths
        .into_iter()
        .zip(contents)
        .map(|(path, after)| AgentFileUpdate { path, after })
        .collect::<Vec<_>>();
    if client == AgentClient::Codex {
        let catalog = codex_catalog.ok_or_else(|| "Unable to generate Codex model catalog".to_string())?;
        validate_codex_catalog(catalog, model)?;
        updates.push(AgentFileUpdate {
            path: codex_model_catalog_path(home),
            after: catalog.to_string(),
        });
        updates.push(if oauth_configuration { build_codex_template_auth(home)? } else {
            AgentFileUpdate { path: codex_configuration_directory(home).join("auth.json"), after: build_codex_api_auth(api_key)? }
        });
    }
    Ok(updates)
}

#[cfg(test)]
pub(crate) fn restore_agent_record_files(
    client: AgentClient,
    record: &AgentModificationRecord,
) -> Result<Vec<String>, String> {
    let direct_write_path = if client == AgentClient::Codex {
        record.files.first().map(|file| file.path.as_path())
    } else {
        None
    };
    let snapshots = record
        .files
        .iter()
        .map(|file| Ok((file.path.clone(), read_agent_bytes(&file.path)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let mut changed = Vec::new();
    for file in &record.files {
        let result = if file.existed_before {
            let backup = fs::read(&file.backup_path).map_err(|error| {
                format!(
                    "Failed to read original configuration backup {}: {error}",
                    path_to_string(&file.backup_path)
                )
            })?;
            if Some(sha256_bytes(&backup)) != file.original_sha256 {
                return Err(format!(
                    "Original configuration backup verification failed: {}",
                    path_to_string(&file.backup_path)
                ));
            }
            if read_agent_bytes(&file.path)?.as_deref() == Some(backup.as_slice()) {
                Ok(())
            } else {
                changed.push(path_to_string(&file.path));
                write_agent_bytes(&file.path, &backup, direct_write_path)
            }
        } else if file.path.exists() {
            changed.push(path_to_string(&file.path));
            fs::remove_file(&file.path).map_err(|error| {
                format!("Failed to delete agent configuration {}: {error}", path_to_string(&file.path))
            })
        } else {
            Ok(())
        };
        if let Err(error) = result {
            let rollback = restore_snapshots_with_direct_path(&snapshots, direct_write_path);
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}"),
            });
        }
    }
    Ok(changed)
}

#[cfg(test)]
pub(crate) fn discard_prepared_agent_backups(
    record: &AgentModificationRecord,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for file in &record.files {
        if file.backup_path.exists() {
            if let Err(error) = fs::remove_file(&file.backup_path) {
                errors.push(format!(
                    "Failed to delete disabled agent backup {}: {error}",
                    path_to_string(&file.backup_path)
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
pub(crate) fn cleanup_agent_record(
    state_path: &Path,
    record: &AgentModificationRecord,
) -> Result<(), String> {
    if state_path.exists() {
        fs::remove_file(state_path).map_err(|error| {
            format!(
                "Failed to delete agent backup state {}: {error}",
                path_to_string(state_path)
            )
        })?;
    }
    for file in &record.files {
        if file.backup_path.exists() {
            if let Err(error) = fs::remove_file(&file.backup_path) {
                eprintln!(
                    "Failed to clean up agent backup {}: {error}",
                    path_to_string(&file.backup_path)
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn action_result(
    outcome: &str,
    enabled: bool,
    model: Option<String>,
    changed_files: Vec<String>,
    conflict_files: Vec<String>,
) -> AgentConfigActionResult {
    AgentConfigActionResult {
        outcome: outcome.to_string(),
        enabled,
        model,
        changed_files,
        conflict_files,
        }
}

#[cfg(test)]
pub(crate) fn enable_agent_modification(
    client: AgentClient,
    home: &Path,
    port: u16,
    model: &str,
    models: &[AgentModelOption],
    codex_catalog: Option<&str>,
) -> Result<AgentConfigActionResult, String> {
    let paths = agent_config_paths(client, home);
    let state_path = agent_state_path(&paths)?;
    if let Some(record) = load_agent_record(client, &paths)? {
        write_agent_state(&state_path, &record)?;
        return Ok(action_result(
            "enabled",
            true,
            Some(record.model),
            Vec::new(),
            Vec::new(),
        ));
    }
    let (_, current_model, _) =
        inspect_agent_managed_config(client, &paths, port, DEFAULT_API_KEY)?;
    if agent_has_managed_marker(client, &paths)? {
        if let Some(current_model) = current_model.as_deref() {
            if let Some(record) = build_legacy_agent_record(client, home, port, current_model)? {
                write_agent_state(&state_path, &record)?;
                return Ok(action_result(
                    "enabled",
                    true,
                    Some(record.model),
                    Vec::new(),
                    Vec::new(),
                ));
            }
        }
        return Err(
            "CPA configuration detected, but no original backup is available for safe restoration. Restore the client configuration manually first".to_string(),
        );
    }

    let updates = build_agent_updates(
        client,
        home,
        port,
        DEFAULT_API_KEY,
        model,
        models,
        codex_catalog,
    )?;
    let update_paths = updates
        .iter()
        .map(|update| update.path.clone())
        .collect::<Vec<_>>();
    let mut record = prepare_agent_record(client, &update_paths, model, &updates)?;
    if let Err(error) = write_agent_state(&state_path, &record) {
        let cleanup = discard_prepared_agent_backups(&record);
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => format!("{error}；{cleanup_error}"),
        });
    }
    match apply_agent_updates(client, &updates) {
        Ok(changed) => {
            record.phase = AGENT_PHASE_ACTIVE.to_string();
            write_agent_state(&state_path, &record)?;
            Ok(action_result(
                "enabled",
                true,
                Some(model.to_string()),
                changed,
                Vec::new(),
            ))
        }
        Err(error) => match restore_agent_record_files(client, &record) {
            Ok(_) => {
                let _ = cleanup_agent_record(&state_path, &record);
                Err(error)
            }
            Err(restore_error) => {
                record.phase = AGENT_PHASE_RECOVERY.to_string();
                let _ = write_agent_state(&state_path, &record);
                Err(format!("{error}; failed to restore original configuration: {restore_error}"))
            }
        },
    }
}

#[cfg(test)]
pub(crate) fn disable_agent_modification(
    client: AgentClient,
    home: &Path,
    port: u16,
    force_restore: bool,
) -> Result<AgentConfigActionResult, String> {
    let paths = agent_config_paths(client, home);
    let state_path = agent_state_path(&paths)?;
    let mut record = match load_agent_record(client, &paths)? {
        Some(record) => record,
        None => {
            let (_, model, _) =
                inspect_agent_managed_config(client, &paths, port, DEFAULT_API_KEY)?;
            if !agent_has_managed_marker(client, &paths)? {
                return Ok(action_result(
                    "disabled",
                    false,
                    None,
                    Vec::new(),
                    Vec::new(),
                ));
            }
            let model = model.ok_or_else(|| "Unable to identify the current CPA model".to_string())?;
            let record = build_legacy_agent_record(client, home, port, &model)?
                .ok_or_else(|| "CPA configuration detected, but no original backup is available for safe restoration".to_string())?;
            write_agent_state(&state_path, &record)?;
            record
        }
    };

    if client == AgentClient::Codex {
        record.phase = AGENT_PHASE_RESTORING.to_string();
        write_agent_state(&state_path, &record)?;
        return match restore_codex_agent_record_files(&record) {
            Ok(changed) => {
                cleanup_agent_record(&state_path, &record)?;
                Ok(action_result("disabled", false, None, changed, Vec::new()))
            }
            Err(error) => {
                record.phase = AGENT_PHASE_RECOVERY.to_string();
                let _ = write_agent_state(&state_path, &record);
                Err(format!("Failed to restore original configuration: {error}"))
            }
        };
    }

    if record_matches_original(&record)? {
        cleanup_agent_record(&state_path, &record)?;
        return Ok(action_result(
            "disabled",
            false,
            None,
            Vec::new(),
            Vec::new(),
        ));
    }

    let conflicts = record_restore_conflict_files(&record)?;
    if !conflicts.is_empty() && !force_restore {
        return Ok(action_result(
            "restore-conflict",
            true,
            Some(record.model),
            Vec::new(),
            conflicts,
        ));
    }

    record.phase = AGENT_PHASE_RESTORING.to_string();
    write_agent_state(&state_path, &record)?;
    match restore_agent_record_files(client, &record) {
        Ok(changed) => {
            cleanup_agent_record(&state_path, &record)?;
            Ok(action_result("disabled", false, None, changed, Vec::new()))
        }
        Err(error) => {
            record.phase = AGENT_PHASE_RECOVERY.to_string();
            let _ = write_agent_state(&state_path, &record);
            Err(format!("Failed to restore original configuration: {error}"))
        }
    }
}

#[cfg(test)]
pub(crate) fn update_agent_modification(
    client: AgentClient,
    home: &Path,
    port: u16,
    model: &str,
    models: &[AgentModelOption],
    codex_catalog: Option<&str>,
) -> Result<AgentConfigActionResult, String> {
    let paths = agent_config_paths(client, home);
    let state_path = agent_state_path(&paths)?;
    let record = match load_agent_record(client, &paths)? {
        Some(record) => record,
        None => {
            let (_, current_model, _) =
                inspect_agent_managed_config(client, &paths, port, DEFAULT_API_KEY)?;
            if !agent_has_managed_marker(client, &paths)? {
                return Err("Apply configuration changes first".to_string());
            }
            let current_model = current_model.ok_or_else(|| "Unable to identify the current CPA model".to_string())?;
            let record = build_legacy_agent_record(client, home, port, &current_model)?
                .ok_or_else(|| "Original configuration backup is missing; cannot update safely".to_string())?;
            write_agent_state(&state_path, &record)?;
            record
        }
    };
    if record.phase != AGENT_PHASE_ACTIVE {
        return Err("The last configuration operation has not completed. Use “Clear Changes” to restore the original configuration first".to_string());
    }
    if client != AgentClient::Codex {
        let conflicts = record_conflict_files(&record)?;
        if !conflicts.is_empty() {
            return Err(format!(
                "Configuration was modified by another program and cannot be updated: {}",
                conflicts.join("、")
            ));
        }
    }

    let updates = build_agent_updates(
        client,
        home,
        port,
        DEFAULT_API_KEY,
        model,
        models,
        codex_catalog,
    )?;
    let (mut next, backup_snapshots) = extend_agent_record_for_updates(&record, &updates)?;
    next.phase = AGENT_PHASE_APPLYING.to_string();
    next.model = model.to_string();
    for (file, update) in next.files.iter_mut().zip(&updates) {
        if file.path != update.path {
            return Err("Agent configuration update path mismatch".to_string());
        }
        file.managed_sha256 = sha256_bytes(update.after.as_bytes());
    }
    if let Err(error) = write_agent_state(&state_path, &next) {
        let rollback = restore_snapshots(&backup_snapshots);
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback_error) => format!("{error}; failed to restore model catalog backup: {rollback_error}"),
        });
    }
    match apply_agent_updates(client, &updates) {
        Ok(changed) => {
            next.phase = AGENT_PHASE_ACTIVE.to_string();
            write_agent_state(&state_path, &next)?;
            Ok(action_result(
                "updated",
                true,
                Some(model.to_string()),
                changed,
                Vec::new(),
            ))
        }
        Err(error) => {
            let state_rollback = write_agent_state(&state_path, &record).err();
            let backup_rollback = restore_snapshots(&backup_snapshots).err();
            let mut errors = vec![error];
            if let Some(rollback_error) = state_rollback {
                errors.push(format!("Failed to restore original state: {rollback_error}"));
            }
            if let Some(rollback_error) = backup_rollback {
                errors.push(format!("Failed to restore model catalog backup: {rollback_error}"));
            }
            Err(errors.join("；"))
        }
    }
}
