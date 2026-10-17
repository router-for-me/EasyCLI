use super::*;

type UsageViewPreferences = std::collections::BTreeMap<String, String>;

#[tauri::command]
pub(crate) fn get_usage_view_preferences() -> Result<UsageViewPreferences, String> {
    let path = core_base_dir()?.join("usage-view-preferences.json");
    match fs::read(&path) {
        Ok(content) => serde_json::from_slice(&content)
            .map_err(|error| format!("Failed to parse usage preferences: {error}")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(format!("Failed to read usage preferences: {error}")),
    }
}

#[tauri::command]
pub(crate) fn save_usage_view_preferences(values: UsageViewPreferences) -> Result<(), String> {
    if values.iter().any(|(key, value)| {
        !(key.starts_with("cpa-gui.usage-") || key == "cpa-gui.pricing-sync-source.v1")
            || key.len() > 128 || value.len() > 16_384
    }) || values.len() > 100 {
        return Err("Invalid usage preferences".into());
    }
    let content = serde_json::to_vec_pretty(&values).map_err(|error| error.to_string())?;
    write_bytes_atomically(&core_base_dir()?.join("usage-view-preferences.json"), &content)
}

#[tauri::command]
pub(crate) fn health_check() -> &'static str {
    "EasyCLIProxyAPI Rust backend is ready"
}

#[tauri::command]
pub(crate) fn detect_core_platform() -> Result<CorePlatform, String> {
    current_core_platform()
}

#[tauri::command]
pub(crate) async fn get_core_status(app: tauri::AppHandle) -> Result<CoreStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = app.state::<GuiConfigState>().snapshot()?;
        current_core_status(
            Some(app.state::<CoreProcessState>().inner()),
            Some(config.port),
        )
    })
    .await
    .map_err(|error| format!("Kernel status background task failed: {error}"))?
}

pub(crate) fn emit_core_status(app: &tauri::AppHandle, status: &CoreStatus) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    update_windows_tray_status(app, status);
    let _ = app.emit(CORE_STATUS_EVENT, status);
}

#[tauri::command]
pub(crate) fn get_gui_settings(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<GuiSettings, String> {
    let config = gui_config_state.snapshot()?;
    Ok(GuiSettings::from(&config))
}

#[tauri::command]
pub(crate) fn resolve_api_access_remarks(
    queries: Vec<ApiAccessRemarkQuery>,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<Vec<String>, String> {
    let config = gui_config_state.snapshot()?;
    queries
        .into_iter()
        .map(|query| resolve_api_access_remark(&config, &query))
        .collect()
}

#[tauri::command]
pub(crate) fn save_api_access_remark(
    update: ApiAccessRemarkUpdate,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<(), String> {
    gui_config_state.update(|config| apply_api_access_remark_update(config, update))?;
    Ok(())
}

fn api_access_locator_identity(
    provider_section: &str,
    locator: &ApiAccessRemarkLocator,
) -> Option<(String, Vec<String>)> {
    let mut api_key_hashes = locator
        .api_keys
        .iter()
        .filter_map(|key| api_access_key_hash(key))
        .collect::<Vec<_>>();
    api_key_hashes.sort();
    api_key_hashes.dedup();
    if api_key_hashes.is_empty() {
        return None;
    }

    let mut identity = Vec::new();
    for component in [
        provider_section.trim(),
        locator.provider_name.trim(),
        locator.base_url.trim(),
    ] {
        identity.extend_from_slice(&(component.len() as u64).to_be_bytes());
        identity.extend_from_slice(component.as_bytes());
    }
    identity.extend_from_slice(&(api_key_hashes.len() as u64).to_be_bytes());
    for hash in &api_key_hashes {
        identity.extend_from_slice(&(hash.len() as u64).to_be_bytes());
        identity.extend_from_slice(hash.as_bytes());
    }
    if !locator.config_identity.is_empty() {
        identity.extend_from_slice(&(locator.config_identity.len() as u64).to_be_bytes());
        identity.extend_from_slice(locator.config_identity.as_bytes());
    }
    Some((sha256_bytes(&identity), api_key_hashes))
}

fn find_api_access_record_remark(
    config: &GuiConfigFile,
    provider_section: &str,
    locator: &ApiAccessRemarkLocator,
) -> Option<String> {
    let (record_hash, api_key_hashes) = api_access_locator_identity(provider_section, locator)?;
    let exact_entries = config.api_access_remarks.iter().filter(|entry| {
        entry.provider_section == provider_section
            && entry.record_hash == record_hash
            && api_key_hashes.contains(&entry.api_key_hash)
    });
    let mut exact_remark = None;
    for entry in exact_entries {
        exact_remark = Some(entry.remark.clone());
        if !entry.remark.is_empty() {
            break;
        }
    }
    exact_remark
}

pub(crate) fn resolve_api_access_remark(
    config: &GuiConfigFile,
    query: &ApiAccessRemarkQuery,
) -> Result<String, String> {
    validate_api_access_provider_section(&query.provider_section)?;
    let mut locators = vec![query.locator.clone()];
    if matches!(
        query.provider_section.as_str(),
        "gemini-api-key" | "codex-api-key" | "claude-api-key"
    ) && !query.locator.provider_name.trim().is_empty()
    {
        let mut unnamed = query.locator.clone();
        unnamed.provider_name.clear();
        locators.push(unnamed);
    }

    for mut locator in locators {
        if let Some(remark) =
            find_api_access_record_remark(config, &query.provider_section, &locator)
        {
            return Ok(remark);
        }
        if !locator.config_identity.is_empty() {
            locator.config_identity.clear();
            if let Some(remark) =
                find_api_access_record_remark(config, &query.provider_section, &locator)
            {
                return Ok(remark);
            }
        }
    }

    let Some((_, api_key_hashes)) =
        api_access_locator_identity(&query.provider_section, &query.locator)
    else {
        return Ok(String::new());
    };
    Ok(api_key_hashes
        .iter()
        .find_map(|hash| {
            config.api_access_remarks.iter().find(|entry| {
                entry.provider_section == query.provider_section
                    && entry.record_hash.is_empty()
                    && entry.api_key_hash == *hash
            })
        })
        .map(|entry| entry.remark.clone())
        .unwrap_or_default())
}

fn apply_api_access_remark_update(
    config: &mut GuiConfigFile,
    update: ApiAccessRemarkUpdate,
) -> Result<(), String> {
    validate_api_access_provider_section(&update.provider_section)?;
    let remark = update.remark.trim().to_string();
    validate_api_key_remark(&remark)?;
    let previous_records = update
        .previous_records
        .iter()
        .filter_map(|locator| api_access_locator_identity(&update.provider_section, locator))
        .collect::<Vec<_>>();
    let next_records = update
        .records
        .iter()
        .filter_map(|locator| api_access_locator_identity(&update.provider_section, locator))
        .collect::<Vec<_>>();
    let all_records = update
        .all_records
        .iter()
        .filter_map(|locator| api_access_locator_identity(&update.provider_section, locator))
        .collect::<Vec<_>>();
    let record_hashes_to_replace = previous_records
        .iter()
        .chain(next_records.iter())
        .map(|(record_hash, _)| record_hash.clone())
        .collect::<HashSet<_>>();
    let provider_section = update.provider_section;
    let legacy_migrations = update
        .all_records
        .iter()
        .filter_map(|locator| {
            let (record_hash, api_key_hashes) =
                api_access_locator_identity(&provider_section, locator)?;
            let remark = resolve_api_access_remark(
                config,
                &ApiAccessRemarkQuery {
                    provider_section: provider_section.clone(),
                    locator: locator.clone(),
                },
            )
            .ok()?;
            Some((record_hash, api_key_hashes, remark))
        })
        .collect::<Vec<_>>();

    config.api_access_remarks.retain(|entry| {
        entry.provider_section != provider_section
            || (!entry.record_hash.is_empty()
                && all_records
                    .iter()
                    .any(|(hash, _)| *hash == entry.record_hash)
                && !record_hashes_to_replace.contains(&entry.record_hash))
    });
    let mut inserted_record_hashes = config
        .api_access_remarks
        .iter()
        .filter(|entry| entry.provider_section == provider_section && !entry.record_hash.is_empty())
        .map(|entry| entry.record_hash.clone())
        .collect::<HashSet<_>>();
    for (record_hash, api_key_hashes) in next_records {
        if !inserted_record_hashes.insert(record_hash.clone()) {
            continue;
        }
        config
            .api_access_remarks
            .extend(
                api_key_hashes
                    .into_iter()
                    .map(|api_key_hash| GuiApiAccessRemark {
                        provider_section: provider_section.clone(),
                        api_key_hash,
                        record_hash: record_hash.clone(),
                        remark: remark.clone(),
                    }),
            );
    }
    for (record_hash, api_key_hashes, legacy_remark) in legacy_migrations {
        if !inserted_record_hashes.insert(record_hash.clone()) {
            continue;
        }
        config
            .api_access_remarks
            .extend(
                api_key_hashes
                    .into_iter()
                    .map(|api_key_hash| GuiApiAccessRemark {
                        provider_section: provider_section.clone(),
                        api_key_hash,
                        record_hash: record_hash.clone(),
                        remark: legacy_remark.clone(),
                    }),
            );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locator(base_url: &str, api_keys: &[&str]) -> ApiAccessRemarkLocator {
        ApiAccessRemarkLocator {
            provider_name: String::new(),
            base_url: base_url.to_string(),
            api_keys: api_keys.iter().map(|key| (*key).to_string()).collect(),
            config_identity: String::new(),
        }
    }

    fn query(locator: ApiAccessRemarkLocator) -> ApiAccessRemarkQuery {
        ApiAccessRemarkQuery {
            provider_section: "codex-api-key".to_string(),
            locator,
        }
    }

    fn update(
        previous_records: Vec<ApiAccessRemarkLocator>,
        records: Vec<ApiAccessRemarkLocator>,
        all_records: Vec<ApiAccessRemarkLocator>,
        remark: &str,
    ) -> ApiAccessRemarkUpdate {
        ApiAccessRemarkUpdate {
            provider_section: "codex-api-key".to_string(),
            previous_records,
            records,
            all_records,
            remark: remark.to_string(),
        }
    }

    fn store_record_remark(
        config: &mut GuiConfigFile,
        provider_section: &str,
        locator: &ApiAccessRemarkLocator,
        remark: &str,
    ) {
        let (record_hash, api_key_hashes) =
            api_access_locator_identity(provider_section, locator).unwrap();
        for api_key_hash in api_key_hashes {
            config.api_access_remarks.push(GuiApiAccessRemark {
                provider_section: provider_section.to_string(),
                api_key_hash,
                record_hash: record_hash.clone(),
                remark: remark.to_string(),
            });
        }
    }

    #[test]
    fn v8_group_names_preserve_and_migrate_unnamed_record_remarks() {
        for provider_section in ["gemini-api-key", "codex-api-key", "claude-api-key"] {
            for config_identity in ["", r#"{"models":[{"name":"a"}]}"#] {
                let mut config = GuiConfigFile::default();
                let mut legacy = locator("https://api.example/v1", &["shared-key"]);
                legacy.config_identity = config_identity.into();
                store_record_remark(&mut config, provider_section, &legacy, "original");
                let legacy_hash = config.api_access_remarks[0].record_hash.clone();

                let mut current = legacy.clone();
                current.provider_name = "production".into();
                current.config_identity = r#"{"models":[{"name":"a"}]}"#.into();
                let current_query = ApiAccessRemarkQuery {
                    provider_section: provider_section.into(),
                    locator: current.clone(),
                };
                assert_eq!(
                    resolve_api_access_remark(&config, &current_query).unwrap(),
                    "original",
                    "{provider_section}, legacy config identity: {config_identity}"
                );

                let mut other = current.clone();
                other.provider_name = "staging".into();
                other.config_identity = r#"{"models":[{"name":"b"}]}"#.into();
                let mut other_update = update(
                    Vec::new(),
                    vec![other.clone()],
                    vec![current.clone(), other.clone()],
                    "other",
                );
                other_update.provider_section = provider_section.into();
                apply_api_access_remark_update(&mut config, other_update).unwrap();

                assert_eq!(config.api_access_remarks.len(), 2);
                let current_hash = api_access_locator_identity(provider_section, &current)
                    .unwrap()
                    .0;
                assert!(config.api_access_remarks.iter().any(|entry| {
                    entry.record_hash == current_hash && entry.remark == "original"
                }));
                assert!(config
                    .api_access_remarks
                    .iter()
                    .all(|entry| entry.record_hash != legacy_hash));
                assert_eq!(
                    resolve_api_access_remark(&config, &current_query).unwrap(),
                    "original"
                );
                assert_eq!(
                    resolve_api_access_remark(
                        &config,
                        &ApiAccessRemarkQuery {
                            provider_section: provider_section.into(),
                            locator: other,
                        }
                    )
                    .unwrap(),
                    "other"
                );
            }
        }
    }

    #[test]
    fn named_record_remarks_override_unnamed_fallbacks_including_empty_notes() {
        for named_config_identity in ["", r#"{"models":[{"name":"a"}]}"#] {
            for remark in ["updated", ""] {
                let mut config = GuiConfigFile::default();
                let mut legacy = locator("https://api.example/v1", &["shared-key"]);
                legacy.config_identity = r#"{"models":[{"name":"a"}]}"#.into();
                store_record_remark(&mut config, "codex-api-key", &legacy, "legacy");
                let mut current = legacy.clone();
                current.provider_name = "production".into();
                let mut saved = current.clone();
                saved.config_identity = named_config_identity.into();
                store_record_remark(&mut config, "codex-api-key", &saved, remark);

                assert_eq!(
                    resolve_api_access_remark(&config, &query(current)).unwrap(),
                    remark
                );
            }
        }
    }

    #[test]
    fn unnamed_fallback_preserves_config_scope_and_explicitly_empty_notes() {
        let mut config = GuiConfigFile::default();
        let legacy = locator("https://api.example/v1", &["shared-key"]);
        store_record_remark(&mut config, "codex-api-key", &legacy, "unscoped");
        for (model, remark) in [("a", "first"), ("b", "")] {
            let mut scoped = legacy.clone();
            scoped.config_identity = format!(r#"{{"models":[{{"name":"{model}"}}]}}"#);
            store_record_remark(&mut config, "codex-api-key", &scoped, remark);
        }

        for (model, expected) in [("a", "first"), ("b", ""), ("c", "unscoped")] {
            let mut current = legacy.clone();
            current.provider_name = "production".into();
            current.config_identity = format!(r#"{{"models":[{{"name":"{model}"}}]}}"#);
            assert_eq!(
                resolve_api_access_remark(&config, &query(current)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn group_name_fallback_does_not_cross_openai_names_or_record_boundaries() {
        let mut config = GuiConfigFile::default();
        let legacy = locator("https://api.example/v1", &["shared-key"]);
        for provider_section in ["codex-api-key", "openai-compatibility"] {
            store_record_remark(&mut config, provider_section, &legacy, "legacy");
        }
        let mut named = legacy.clone();
        named.provider_name = "production".into();
        let mut different_url = named.clone();
        different_url.base_url = "https://other.example/v1".into();
        let mut different_key = named.clone();
        different_key.api_keys = vec!["other-key".into()];
        for (provider_section, locator) in [
            ("openai-compatibility", named.clone()),
            ("claude-api-key", named),
            ("codex-api-key", different_url),
            ("codex-api-key", different_key),
        ] {
            assert_eq!(
                resolve_api_access_remark(
                    &config,
                    &ApiAccessRemarkQuery {
                        provider_section: provider_section.into(),
                        locator,
                    }
                )
                .unwrap(),
                ""
            );
        }
    }

    #[test]
    fn remarks_are_scoped_to_provider_records_that_share_an_api_key() {
        let mut config = GuiConfigFile::default();
        let first = locator("https://first.example/v1", &["shared-key"]);
        let second = locator("https://second.example/v1", &["shared-key"]);

        apply_api_access_remark_update(
            &mut config,
            update(
                Vec::new(),
                vec![first.clone()],
                vec![first.clone()],
                "first",
            ),
        )
        .unwrap();
        apply_api_access_remark_update(
            &mut config,
            update(
                Vec::new(),
                vec![second.clone()],
                vec![first.clone(), second.clone()],
                "second",
            ),
        )
        .unwrap();

        assert_eq!(
            resolve_api_access_remark(&config, &query(first.clone())).unwrap(),
            "first"
        );
        assert_eq!(
            resolve_api_access_remark(&config, &query(second.clone())).unwrap(),
            "second"
        );

        apply_api_access_remark_update(
            &mut config,
            update(vec![first.clone()], Vec::new(), vec![second.clone()], ""),
        )
        .unwrap();
        assert_eq!(
            resolve_api_access_remark(&config, &query(second)).unwrap(),
            "second"
        );
    }

    #[test]
    fn record_scoped_remarks_override_legacy_key_scoped_remarks() {
        let mut config = GuiConfigFile::default();
        let first = locator("https://first.example/v1", &["shared-key"]);
        let second = locator("https://second.example/v1", &["shared-key"]);
        config.api_access_remarks.push(GuiApiAccessRemark {
            provider_section: "codex-api-key".to_string(),
            api_key_hash: api_access_key_hash("shared-key").unwrap(),
            record_hash: String::new(),
            remark: "legacy".to_string(),
        });

        apply_api_access_remark_update(
            &mut config,
            update(
                vec![first.clone()],
                vec![first.clone()],
                vec![first.clone(), second.clone()],
                "updated",
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_api_access_remark(&config, &query(first.clone())).unwrap(),
            "updated"
        );
        assert_eq!(
            resolve_api_access_remark(&config, &query(second.clone())).unwrap(),
            "legacy"
        );

        apply_api_access_remark_update(
            &mut config,
            update(
                vec![first.clone()],
                vec![first.clone()],
                vec![first.clone(), second.clone()],
                "",
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_api_access_remark(&config, &query(first)).unwrap(),
            ""
        );
        assert_eq!(
            resolve_api_access_remark(&config, &query(second)).unwrap(),
            "legacy"
        );
        assert!(config
            .api_access_remarks
            .iter()
            .all(|entry| !entry.record_hash.is_empty()));
    }

    #[test]
    fn updating_a_single_legacy_record_removes_the_stale_key_mapping() {
        let mut config = GuiConfigFile::default();
        let record = locator("https://api.example/v1", &["shared-key"]);
        config.api_access_remarks.push(GuiApiAccessRemark {
            provider_section: "codex-api-key".to_string(),
            api_key_hash: api_access_key_hash("shared-key").unwrap(),
            record_hash: String::new(),
            remark: "legacy".to_string(),
        });

        apply_api_access_remark_update(
            &mut config,
            update(
                vec![record.clone()],
                vec![record.clone()],
                vec![record.clone()],
                "updated",
            ),
        )
        .unwrap();

        assert_eq!(
            resolve_api_access_remark(&config, &query(record)).unwrap(),
            "updated"
        );
        assert_eq!(
            config.api_access_remark_for_source("codex", "shared-key"),
            Some("updated")
        );
        assert!(config
            .api_access_remarks
            .iter()
            .all(|entry| !entry.record_hash.is_empty()));
    }

    #[test]
    fn shared_credentials_keep_separate_config_remarks_and_migrate_old_records() {
        let mut config = GuiConfigFile::default();
        let legacy = locator("https://api.example/v1", &["shared-key"]);
        apply_api_access_remark_update(
            &mut config,
            update(
                Vec::new(),
                vec![legacy.clone()],
                vec![legacy.clone()],
                "original",
            ),
        )
        .unwrap();

        let mut first = legacy.clone();
        first.config_identity = r#"{"models":[{"name":"a"}],"priority":10}"#.into();
        let mut second = legacy.clone();
        second.config_identity = r#"{"models":[{"name":"b"}],"priority":1}"#.into();
        assert_eq!(
            resolve_api_access_remark(&config, &query(first.clone())).unwrap(),
            "original"
        );

        apply_api_access_remark_update(
            &mut config,
            update(
                Vec::new(),
                vec![second.clone()],
                vec![first.clone(), second.clone()],
                "second",
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_api_access_remark(&config, &query(first.clone())).unwrap(),
            "original"
        );
        assert_eq!(
            resolve_api_access_remark(&config, &query(second.clone())).unwrap(),
            "second"
        );
        assert_eq!(config.api_access_remarks.len(), 2);

        let mut edited = second.clone();
        edited.config_identity = r#"{"models":[{"name":"edited"}],"priority":2}"#.into();
        apply_api_access_remark_update(
            &mut config,
            update(
                vec![second],
                vec![edited.clone()],
                vec![first.clone(), edited.clone()],
                "",
            ),
        )
        .unwrap();
        assert_eq!(
            resolve_api_access_remark(&config, &query(edited.clone())).unwrap(),
            ""
        );
        assert_eq!(
            resolve_api_access_remark(&config, &query(first.clone())).unwrap(),
            "original"
        );

        apply_api_access_remark_update(
            &mut config,
            update(vec![edited], Vec::new(), vec![first.clone()], ""),
        )
        .unwrap();
        assert_eq!(config.api_access_remarks.len(), 1);
        assert_eq!(
            resolve_api_access_remark(&config, &query(first)).unwrap(),
            "original"
        );
        assert!(!toml::to_string(&config.api_access_remarks[0])
            .unwrap()
            .contains("shared-key"));
    }

    #[test]
    fn legacy_remark_entries_deserialize_without_a_record_hash() {
        let entry = toml::from_str::<GuiApiAccessRemark>(
            r#"provider-section = "codex-api-key"
api-key-hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
remark = "legacy"
"#,
        )
        .unwrap();

        assert!(entry.record_hash.is_empty());
        assert_eq!(entry.remark, "legacy");
    }

    #[test]
    fn remark_command_payloads_accept_record_locators() {
        let query = serde_json::from_value::<ApiAccessRemarkQuery>(serde_json::json!({
            "providerSection": "openai-compatibility",
            "providerName": "first",
            "baseUrl": "https://api.example/v1",
            "apiKeys": ["shared-key"]
        }))
        .unwrap();
        let update = serde_json::from_value::<ApiAccessRemarkUpdate>(serde_json::json!({
            "providerSection": "openai-compatibility",
            "previousRecords": [{
                "providerName": "first",
                "baseUrl": "https://api.example/v1",
                "apiKeys": ["shared-key"]
            }],
            "records": [{
                "providerName": "second",
                "baseUrl": "https://api.example/v1",
                "apiKeys": ["shared-key"]
            }],
            "allRecords": [{
                "providerName": "second",
                "baseUrl": "https://api.example/v1",
                "apiKeys": ["shared-key"]
            }],
            "remark": "second"
        }))
        .unwrap();

        assert_eq!(query.locator.provider_name, "first");
        assert_eq!(update.previous_records[0].provider_name, "first");
        assert_eq!(update.records[0].provider_name, "second");
        assert_eq!(update.all_records[0].provider_name, "second");
    }
}

#[tauri::command]
pub(crate) fn set_app_locale(
    app: tauri::AppHandle,
    process_state: tauri::State<'_, CoreProcessState>,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    locale: String,
) -> Result<String, String> {
    let config = gui_config_state.set_locale(locale)?;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Ok(status) = current_core_status(Some(process_state.inner()), Some(config.port)) {
        update_windows_tray_locale(&app, &config.locale, &status);
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = (app, process_state);
    Ok(config.locale)
}

#[tauri::command]
pub(crate) fn resolve_windows_close_request(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    action: WindowsCloseAction,
    remember: Option<bool>,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if action == WindowsCloseAction::MinimizeToTray && !linux_tray_available(&app) {
        return Err(
            "System tray is unavailable; keep the main window open or exit the app".to_string(),
        );
    }

    if remember.unwrap_or(false) {
        let close_behavior = match action {
            WindowsCloseAction::Exit => WindowsCloseBehavior::Exit,
            WindowsCloseAction::MinimizeToTray => WindowsCloseBehavior::MinimizeToTray,
        };
        gui_config_state.set_close_behavior(close_behavior)?;
    }

    match action {
        WindowsCloseAction::Exit => app.exit(0),
        WindowsCloseAction::MinimizeToTray => {
            let window =
                main_window(&app).ok_or_else(|| "Main window does not exist".to_string())?;
            window
                .hide()
                .map_err(|error| format!("Failed to hide main window: {error}"))?;
        }
    }

    Ok(())
}

pub(crate) fn app_autostart_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| format!("Failed to read system startup status: {error}"))
}

pub(crate) fn set_app_autostart_enabled(
    app: &tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|error| format!("Failed to enable launch at startup: {error}"))
    } else {
        manager
            .disable()
            .map_err(|error| format!("Failed to disable launch at startup: {error}"))
    }
}

pub(crate) fn software_settings(
    app: &tauri::AppHandle,
    config: &GuiConfigFile,
) -> Result<SoftwareSettings, String> {
    Ok(SoftwareSettings {
        close_behavior: config.close_behavior,
        autostart_enabled: app_autostart_enabled(app)?,
        start_core_on_launch: config.start_core_on_launch,
        silent_start_enabled: config.silent_start,
        default_terminal: normalize_agent_terminal(&config.default_terminal),
        available_terminals: available_agent_terminals(),
    })
}

#[tauri::command]
pub(crate) fn get_software_settings(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<SoftwareSettings, String> {
    let config = gui_config_state.snapshot()?;
    software_settings(&app, &config)
}

#[tauri::command]
pub(crate) fn save_software_settings(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    settings: SoftwareSettingsInput,
) -> Result<SoftwareSettings, String> {
    let previous_config = gui_config_state.snapshot()?;
    let previous_autostart_enabled = app_autostart_enabled(&app)?;
    let autostart_changed = previous_autostart_enabled != settings.autostart_enabled;
    let default_terminal = normalize_agent_terminal(&settings.default_terminal);

    if autostart_changed {
        set_app_autostart_enabled(&app, settings.autostart_enabled)?;
    }

    let config = if previous_config.close_behavior == settings.close_behavior
        && previous_config.start_core_on_launch == settings.start_core_on_launch
        && previous_config.silent_start == settings.silent_start_enabled
        && previous_config.default_terminal == default_terminal
    {
        previous_config
    } else {
        match gui_config_state.set_software_preferences(
            settings.close_behavior,
            settings.start_core_on_launch,
            settings.silent_start_enabled,
            default_terminal.clone(),
        ) {
            Ok(config) => config,
            Err(error) => {
                let rollback_error = autostart_changed
                    .then(|| set_app_autostart_enabled(&app, previous_autostart_enabled).err())
                    .flatten();
                return Err(match rollback_error {
                    Some(rollback_error) => {
                        format!("{error}; failed to roll back startup settings: {rollback_error}")
                    }
                    None => error,
                });
            }
        }
    };

    software_settings(&app, &config)
}
