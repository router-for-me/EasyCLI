use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) type ModelCustomizations = BTreeMap<String, Map<String, Value>>;

pub(super) const EDITABLE_FIELDS: [&str; 13] = [
    "display_name",
    "description",
    "base_instructions",
    "context_window",
    "max_context_window",
    "effective_context_window_percent",
    "auto_compact_token_limit",
    "default_reasoning_level",
    "supported_reasoning_levels",
    "input_modalities",
    "visibility",
    "supports_parallel_tool_calls",
    "supports_search_tool",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCustomizations {
    version: u32,
    models: ModelCustomizations,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogEditorSnapshot {
    revision: String,
    models: Vec<CatalogEditorModel>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEditorModel {
    slug: String,
    has_official_template: bool,
    context_source: &'static str,
    customized: bool,
    configuration: Map<String, Value>,
    defaults: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogEditorRequest {
    revision: String,
    models: Vec<CatalogEditorModelRequest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEditorModelRequest {
    slug: String,
    configuration: Map<String, Value>,
}

fn editable_configuration(model: &Map<String, Value>) -> Map<String, Value> {
    EDITABLE_FIELDS
        .iter()
        .map(|field| {
            (
                field.to_string(),
                model.get(*field).cloned().unwrap_or_else(|| {
                    if *field == "supports_search_tool" {
                        Value::Bool(false)
                    } else if *field == "effective_context_window_percent" {
                        Value::from(95)
                    } else {
                        Value::Null
                    }
                }),
            )
        })
        .collect()
}

fn validate_configuration(model: &Map<String, Value>) -> Result<(), String> {
    if let Some(value) = model.get("base_instructions") {
        if !value.as_str().is_some_and(|text| !text.trim().is_empty()) {
            return Err("System prompt must be non-empty text".to_string());
        }
    }
    for field in model.keys() {
        if !EDITABLE_FIELDS.contains(&field.as_str()) {
            return Err(format!("Model field {field} cannot be modified"));
        }
    }
    for field in [
        "context_window",
        "max_context_window",
        "auto_compact_token_limit",
    ] {
        if let Some(value) = model.get(field) {
            if field == "auto_compact_token_limit" && value.is_null() {
                continue;
            }
            if !value
                .as_u64()
                .is_some_and(|value| (1..=9_007_199_254_740_991).contains(&value))
            {
                return Err(format!("{field} must be a valid positive integer"));
            }
        }
    }
    if let (Some(context), Some(maximum)) = (
        model.get("context_window").and_then(Value::as_u64),
        model.get("max_context_window").and_then(Value::as_u64),
    ) {
        if maximum < context {
            return Err("Maximum context window cannot be smaller than the context window".to_string());
        }
        if model
            .get("auto_compact_token_limit")
            .and_then(Value::as_u64)
            .is_some_and(|limit| limit > context)
        {
            return Err("Automatic compaction threshold cannot exceed the context window".to_string());
        }
    }
    if let Some(value) = model.get("effective_context_window_percent") {
        if !value
            .as_u64()
            .is_some_and(|value| (1..=100).contains(&value))
        {
            return Err("Effective context percentage must be an integer from 1 to 100".to_string());
        }
    }
    for field in ["display_name", "description"] {
        if let Some(value) = model.get(field) {
            if field == "description" && value.is_null() {
                continue;
            }
            if !value.as_str().is_some_and(|text| {
                text.len() <= 4_000 && (field == "description" || !text.trim().is_empty())
            }) {
                return Err(format!("{field} must be valid text"));
            }
        }
    }
    if let Some(value) = model.get("visibility") {
        if !matches!(value.as_str(), Some("list" | "hide" | "none")) {
            return Err("Invalid model display status".to_string());
        }
    }
    for field in ["supports_parallel_tool_calls", "supports_search_tool"] {
        if let Some(value) = model.get(field) {
            if !value.is_boolean() {
                return Err(format!("{field} must be a boolean"));
            }
        }
    }
    if let Some(value) = model.get("input_modalities") {
        let modalities = value.as_array().ok_or("Input types must be an array")?;
        let mut seen = HashSet::new();
        if modalities.is_empty()
            || modalities.iter().any(|value| {
                !matches!(value.as_str(), Some("text" | "image")) || !seen.insert(value.as_str())
            })
        {
            return Err("Input types may contain only unique text and image values, with at least one selected".to_string());
        }
    }
    let default = model.get("default_reasoning_level");
    if let Some(value) = default {
        if !value.is_null() && !value.as_str().is_some_and(is_allowed_reasoning_level) {
            return Err("Invalid default reasoning level".to_string());
        }
    }
    if let Some(value) = model.get("supported_reasoning_levels") {
        let levels = value.as_array().ok_or("Reasoning levels must be an array")?;
        let mut seen = HashSet::new();
        for level in levels {
            let effort = level
                .get("effort")
                .and_then(Value::as_str)
                .ok_or("Reasoning level is missing effort")?;
            if !is_allowed_reasoning_level(effort) || !seen.insert(effort) {
                return Err(format!("Invalid or duplicate reasoning level: {effort}"));
            }
            if !level
                .get("description")
                .and_then(Value::as_str)
                .is_some_and(|text| text.len() <= 4_000)
            {
                return Err("Reasoning level description must be valid text".to_string());
            }
        }
        if default
            .and_then(Value::as_str)
            .is_some_and(|effort| !seen.contains(effort))
        {
            return Err("Default reasoning level must be included in the available levels".to_string());
        }
    }
    Ok(())
}

pub(super) fn apply_customizations(
    model: &mut Map<String, Value>,
    customizations: &ModelCustomizations,
) -> Result<(), String> {
    let slug = string_value(model, "slug");
    if let Some(customization) = customizations.get(&normalize_id(&slug)) {
        model.extend(customization.clone());
        // Codex clients can prefer the message template over base_instructions.
        // Keep both paths consistent when the user replaces the system prompt.
        if let Some(prompt) = customization.get("base_instructions") {
            let messages = model.entry("model_messages".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !messages.is_object() {
                *messages = Value::Object(Map::new());
            }
            messages.as_object_mut().unwrap()
                .insert("instructions_template".to_string(), prompt.clone());
        }
        validate_configuration(&editable_configuration(model))
            .map_err(|error| format!("Invalid custom configuration for model {slug}: {error}"))?;
        enable_fast_mode(model);
    }
    Ok(())
}

pub(super) fn snapshot_for_state(
    runtime_models: &[CodexRuntimeModel],
    state: &CatalogState,
) -> Result<CatalogEditorSnapshot, String> {
    let baseline = if runtime_models.is_empty() {
        "{\"models\":[]}".to_string()
    } else {
        prepare_catalog_with_customizations(runtime_models, &state.sources, &Default::default())?
            .json
    };
    let root: Value = serde_json::from_str(&baseline).map_err(|error| error.to_string())?;
    let mut models = Vec::new();
    for value in root["models"]
        .as_array()
        .ok_or("Model catalog is missing a models array")?
    {
        let mut model = value.as_object().cloned().ok_or("Model catalog entry must be an object")?;
        let slug = string_value(&model, "slug");
        let key = normalize_id(&slug);
        let defaults = editable_configuration(&model);
        apply_customizations(&mut model, &state.customizations)?;
        models.push(CatalogEditorModel {
            has_official_template: state.sources.template_for_slug(&slug).is_some(),
            slug,
            context_source: runtime_models
                .iter()
                .find(|runtime| normalize_id(&runtime.slug) == key)
                .map_or("template", |runtime| runtime.context_source),
            customized: state.customizations.contains_key(&key),
            configuration: editable_configuration(&model),
            defaults,
        });
    }
    let mut digest = Sha256::new();
    digest.update(baseline.as_bytes());
    digest.update(serde_json::to_vec(&state.customizations).map_err(|error| error.to_string())?);
    Ok(CatalogEditorSnapshot {
        revision: format!("{:x}", digest.finalize()),
        models,
    })
}

pub(crate) fn editor_snapshot(
    runtime_models: &[CodexRuntimeModel],
) -> Result<CatalogEditorSnapshot, String> {
    let state = catalog_state()?
        .read()
        .map_err(|_| "Codex model catalog memory lock is poisoned")?;
    snapshot_for_state(runtime_models, &state)
}

fn customizations_from_request(
    snapshot: &CatalogEditorSnapshot,
    request: CatalogEditorRequest,
) -> Result<ModelCustomizations, String> {
    if request.revision != snapshot.revision {
        return Err("CODEX_MODEL_CATALOG_CHANGED".to_string());
    }
    if request.models.len() != snapshot.models.len() {
        return Err("Model list has changed. Reload it before editing".to_string());
    }
    let mut seen = HashSet::new();
    let mut customizations = BTreeMap::new();
    for requested in request.models {
        let key = normalize_id(&requested.slug);
        let current = snapshot
            .models
            .iter()
            .find(|model| normalize_id(&model.slug) == key)
            .ok_or_else(|| format!("Model {} is no longer in the current list", requested.slug))?;
        if !seen.insert(key.clone()) {
            return Err(format!("Duplicate model {}", requested.slug));
        }
        if requested.configuration.len() != EDITABLE_FIELDS.len() {
            return Err(format!("Model {} has incomplete configuration fields", requested.slug));
        }
        validate_configuration(&requested.configuration)
            .map_err(|error| format!("Model {}: {error}", requested.slug))?;
        let mut changes: Map<String, Value> = requested
            .configuration
            .iter()
            .filter(|(field, value)| current.defaults.get(*field) != Some(*value))
            .map(|(field, value)| (field.clone(), value.clone()))
            .collect();
        for group in [
            &[
                "context_window",
                "max_context_window",
                "auto_compact_token_limit",
                "effective_context_window_percent",
            ][..],
            &["default_reasoning_level", "supported_reasoning_levels"][..],
        ] {
            if group.iter().any(|field| changes.contains_key(*field)) {
                for field in group {
                    changes.insert(field.to_string(), requested.configuration[*field].clone());
                }
            }
        }
        if !changes.is_empty() {
            customizations.insert(key, changes);
        }
    }
    Ok(customizations)
}

fn decode_customizations(content: &[u8]) -> Result<ModelCustomizations, String> {
    if content.len() > crate::MAX_CODEX_MODEL_CATALOG_BYTES {
        return Err("Codex custom model configuration exceeds the size limit".to_string());
    }
    let saved: SavedCustomizations =
        serde_json::from_slice(content).map_err(|error| error.to_string())?;
    if saved.version != 1 {
        return Err("Unsupported Codex custom model configuration version".to_string());
    }
    let mut normalized = BTreeMap::new();
    for (slug, model) in saved.models {
        let key = normalize_id(&slug);
        if key.is_empty() || normalized.contains_key(&key) {
            return Err("Custom model ID is empty or duplicated".to_string());
        }
        validate_configuration(&model)?;
        normalized.insert(key, model);
    }
    Ok(normalized)
}

pub(crate) fn load_customizations(path: &Path) -> Result<(), String> {
    let customizations = match std::fs::read(path) {
        Ok(content) => decode_customizations(&content)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(error) => return Err(format!("Failed to read Codex custom model configuration: {error}")),
    };
    let mut state = catalog_state()?
        .write()
        .map_err(|_| "Codex model catalog memory lock is poisoned")?;
    state.customizations = customizations;
    Ok(())
}

fn save_for_state(
    path: &Path,
    runtime_models: &[CodexRuntimeModel],
    request: CatalogEditorRequest,
    state: &mut CatalogState,
) -> Result<CatalogEditorSnapshot, String> {
    let snapshot = snapshot_for_state(runtime_models, state)?;
    let mut customizations = customizations_from_request(&snapshot, request)?;
    // The public catalog can temporarily omit disabled or unavailable models.
    // Editing visible models must not erase the saved settings for those IDs.
    let visible: HashSet<String> = snapshot.models.iter()
        .map(|model| normalize_id(&model.slug))
        .collect();
    for (slug, configuration) in &state.customizations {
        if !visible.contains(slug) {
            customizations.insert(slug.clone(), configuration.clone());
        }
    }
    let saved = SavedCustomizations {
        version: 1,
        models: customizations,
    };
    let content = serde_json::to_vec_pretty(&saved).map_err(|error| error.to_string())?;
    if content.len() > crate::MAX_CODEX_MODEL_CATALOG_BYTES {
        return Err("Codex custom model configuration exceeds the size limit".to_string());
    }
    crate::write_bytes_atomically(path, &content)?;
    state.customizations = saved.models;
    snapshot_for_state(runtime_models, state)
}

pub(crate) fn save_customizations(
    path: &Path,
    runtime_models: &[CodexRuntimeModel],
    request: CatalogEditorRequest,
) -> Result<CatalogEditorSnapshot, String> {
    let mut state = catalog_state()?
        .write()
        .map_err(|_| "Codex model catalog memory lock is poisoned")?;
    save_for_state(path, runtime_models, request, &mut state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn runtime_model(slug: &str) -> CodexRuntimeModel {
        CodexRuntimeModel {
            slug: slug.to_string(),
            display_name: None,
            description: None,
            context_window: None,
            max_context_window: None,
            context_source: "template",
            input_modalities: None,
            default_reasoning_level: None,
            hidden: false,
        }
    }

    fn temporary_path() -> std::path::PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "cpa-codex-model-customizations-{}-{}.json",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn tool_search_overrides_survive_regeneration_and_restart_for_prefixed_and_unknown_models() {
        for (slug, enabled) in [("Fy/gpt-6.1-sol", false), ("provider/custom-model", true)] {
            let mut state = CatalogState {
                sources: parse_sources(MODEL_CATALOG_JSON).unwrap(),
                json: MODEL_CATALOG_JSON.to_string(),
                customizations: Default::default(),
            };
            let runtime = parse_public_models(&serde_json::json!({"data":[{"id":slug}]})).unwrap();
            let snapshot = snapshot_for_state(&runtime, &state).unwrap();
            assert_eq!(snapshot.models[0].has_official_template, !enabled);
            assert_eq!(snapshot.models[0].defaults["supports_search_tool"], !enabled);
            let mut configuration = snapshot.models[0].configuration.clone();
            configuration.insert("supports_search_tool".into(), Value::Bool(enabled));
            let path = temporary_path();
            save_for_state(&path, &runtime, CatalogEditorRequest {
                revision: snapshot.revision,
                models: vec![CatalogEditorModelRequest { slug: slug.into(), configuration }],
            }, &mut state).unwrap();
            // A refreshed template must not replace the saved per-routing-ID override.
            let mut cloud: Value = serde_json::from_str(MODEL_CATALOG_JSON).unwrap();
            cloud["catalog_revision"] = Value::from(state.sources.revision + 1);
            for model in cloud["models"].as_array_mut().unwrap() {
                model["description"] = Value::from("Refreshed template");
            }
            let cloud = cloud.to_string();
            assert!(state.activate(&cloud, parse_sources(&cloud).unwrap()));
            let restarted = CatalogState {
                customizations: decode_customizations(&std::fs::read(&path).unwrap()).unwrap(),
                ..state.clone()
            };
            for current in [&state, &restarted] {
                let catalog = prepare_catalog_with_customizations(
                    &runtime, &current.sources, &current.customizations,
                ).unwrap();
                let generated: Value = serde_json::from_str(&catalog.json).unwrap();
                assert_eq!(generated["models"][0]["supports_search_tool"], enabled);
            }
            let snapshot = snapshot_for_state(&runtime, &restarted).unwrap();
            assert!(snapshot.models[0].customized);
            let mut restored = restarted.clone();
            save_for_state(&path, &runtime, CatalogEditorRequest {
                revision: snapshot.revision,
                models: vec![CatalogEditorModelRequest {
                    slug: slug.into(), configuration: snapshot.models[0].defaults.clone(),
                }],
            }, &mut restored).unwrap();
            let snapshot = snapshot_for_state(&runtime, &restored).unwrap();
            assert!(!snapshot.models[0].customized);
            assert_eq!(snapshot.models[0].configuration["supports_search_tool"], !enabled);
            std::fs::remove_file(path).unwrap();
        }
        assert!(validate_configuration(&serde_json::json!({"supports_search_tool":"true"})
            .as_object().unwrap()).is_err());
    }

    #[test]
    fn cloud_catalog_refresh_and_restart_preserve_user_overrides() {
        let mut state = CatalogState {
            sources: parse_sources(MODEL_CATALOG_JSON).unwrap(),
            json: MODEL_CATALOG_JSON.to_string(),
            customizations: Default::default(),
        };
        let runtime = parse_public_models(&serde_json::json!({"data": [
            {"id": "gpt-6.1-sol"}
        ]})).unwrap();
        let snapshot = snapshot_for_state(&runtime, &state).unwrap();
        let mut configuration = snapshot.models[0].configuration.clone();
        configuration.insert("context_window".into(), serde_json::json!(128_000));
        configuration.insert("auto_compact_token_limit".into(), serde_json::json!(100_000));
        configuration.insert("default_reasoning_level".into(), serde_json::json!("high"));
        configuration.insert("base_instructions".into(), serde_json::json!("User instructions"));
        let path = temporary_path();
        save_for_state(&path, &runtime, CatalogEditorRequest {
            revision: snapshot.revision,
            models: vec![CatalogEditorModelRequest {
                slug: "gpt-6.1-sol".into(), configuration,
            }],
        }, &mut state).unwrap();

        let mut cloud: Value = serde_json::from_str(MODEL_CATALOG_JSON).unwrap();
        cloud["catalog_revision"] = serde_json::json!(state.sources.revision + 1);
        let model = cloud["models"].as_array_mut().unwrap().iter_mut()
            .find(|model| model["slug"] == "gpt-6.1-sol").unwrap();
        model["context_window"] = serde_json::json!(600_000);
        model["description"] = serde_json::json!("Updated cloud description");
        let cloud = serde_json::to_string(&cloud).unwrap();
        assert!(state.activate(&cloud, parse_sources(&cloud).unwrap()));
        let restarted = CatalogState {
            sources: parse_sources(&cloud).unwrap(),
            json: cloud,
            customizations: decode_customizations(&std::fs::read(&path).unwrap()).unwrap(),
        };
        for current in [&state, &restarted] {
            let snapshot = snapshot_for_state(&runtime, current).unwrap();
            assert_eq!(snapshot.models[0].defaults["context_window"], 600_000);
            assert_eq!(snapshot.models[0].configuration["context_window"], 128_000);
            let catalog = prepare_catalog_with_customizations(
                &runtime, &current.sources, &current.customizations,
            ).unwrap();
            let generated: Value = serde_json::from_str(&catalog.json).unwrap();
            let model = &generated["models"][0];
            assert_eq!(model["context_window"], 128_000);
            assert_eq!(model["auto_compact_token_limit"], 100_000);
            assert_eq!(model["default_reasoning_level"], "high");
            assert_eq!(model["base_instructions"], "User instructions");
            assert_eq!(model["description"], "Updated cloud description");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn saving_visible_models_preserves_unavailable_models_and_allows_visible_reset() {
        let mut state = CatalogState {
            sources: parse_sources(MODEL_CATALOG_JSON).unwrap(),
            json: MODEL_CATALOG_JSON.to_string(),
            customizations: Default::default(),
        };
        let hidden = serde_json::json!({
            "base_instructions": "Saved prompt for temporarily unavailable A",
            "context_window": 131_072,
            "max_context_window": 262_144,
        }).as_object().unwrap().clone();
        state.customizations.insert("model-a".to_string(), hidden.clone());
        let runtime = vec![runtime_model("model-b")];
        let snapshot = snapshot_for_state(&runtime, &state).unwrap();
        let mut configuration = snapshot.models[0].configuration.clone();
        configuration.insert("display_name".to_string(), Value::String("Edited B".to_string()));
        let path = temporary_path();
        let saved = save_for_state(&path, &runtime, CatalogEditorRequest {
            revision: snapshot.revision,
            models: vec![CatalogEditorModelRequest { slug: "model-b".to_string(), configuration }],
        }, &mut state).unwrap();
        let persisted = decode_customizations(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.get("model-a"), Some(&hidden));
        assert_eq!(persisted["model-b"]["display_name"], "Edited B");

        save_for_state(&path, &runtime, CatalogEditorRequest {
            revision: saved.revision,
            models: vec![CatalogEditorModelRequest {
                slug: "model-b".to_string(), configuration: saved.models[0].defaults.clone(),
            }],
        }, &mut state).unwrap();
        assert!(!state.customizations.contains_key("model-b"));
        assert_eq!(state.customizations.get("model-a"), Some(&hidden));

        let empty = snapshot_for_state(&[], &state).unwrap();
        save_for_state(&path, &[], CatalogEditorRequest {
            revision: empty.revision, models: vec![],
        }, &mut state).unwrap();
        assert_eq!(decode_customizations(&std::fs::read(&path).unwrap()).unwrap().get("model-a"), Some(&hidden));
        let restored = snapshot_for_state(&[runtime_model("model-a")], &state).unwrap();
        assert_eq!(restored.models[0].configuration["base_instructions"], hidden["base_instructions"]);
        assert_eq!(restored.models[0].configuration["context_window"], 131_072);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn customizations_persist_apply_and_restore_to_the_current_template() {
        let sources = parse_sources(MODEL_CATALOG_JSON).unwrap();
        let mut state = CatalogState {
            sources,
            json: MODEL_CATALOG_JSON.to_string(),
            customizations: Default::default(),
        };
        let runtime_models = vec![runtime_model("third-party-model")];
        let snapshot = snapshot_for_state(&runtime_models, &state).unwrap();
        let mut configuration = snapshot.models[0].configuration.clone();
        assert_eq!(configuration["supports_parallel_tool_calls"], true);
        let prompt = "请用中文回答。\nPreserve whitespace and newlines.\n";
        configuration.insert("base_instructions".to_string(), serde_json::json!(prompt));
        configuration.insert("supports_parallel_tool_calls".to_string(), Value::Bool(false));
        configuration.insert("context_window".to_string(), serde_json::json!(131_072));
        configuration.insert("max_context_window".to_string(), serde_json::json!(262_144));
        let path = temporary_path();

        let saved = save_for_state(
            &path,
            &runtime_models,
            CatalogEditorRequest {
                revision: snapshot.revision,
                models: vec![CatalogEditorModelRequest {
                    slug: "third-party-model".to_string(),
                    configuration,
                }],
            },
            &mut state,
        )
        .unwrap();

        assert!(saved.models[0].customized);
        assert_eq!(saved.models[0].configuration["context_window"], 131_072);
        assert_eq!(saved.models[0].configuration["max_context_window"], 262_144);
        let generated = prepare_catalog_with_customizations(
            &runtime_models,
            &state.sources,
            &state.customizations,
        )
        .unwrap();
        let generated: Value = serde_json::from_str(&generated.json).unwrap();
        assert_eq!(generated["models"][0]["context_window"], 131_072);
        assert_eq!(generated["models"][0]["base_instructions"], prompt);
        assert_eq!(generated["models"][0]["model_messages"]["instructions_template"], prompt);
        assert_eq!(generated["models"][0]["supports_parallel_tool_calls"], false);
        assert_eq!(generated["models"][0]["max_context_window"], 262_144);

        let loaded = decode_customizations(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded, state.customizations);

        let restored = save_for_state(
            &path,
            &runtime_models,
            CatalogEditorRequest {
                revision: saved.revision,
                models: vec![CatalogEditorModelRequest {
                    slug: "third-party-model".to_string(),
                    configuration: saved.models[0].defaults.clone(),
                }],
            },
            &mut state,
        )
        .unwrap();
        assert!(!restored.models[0].customized);
        assert_eq!(restored.models[0].configuration["supports_parallel_tool_calls"], true);
        assert_eq!(restored.models[0].configuration["base_instructions"], restored.models[0].defaults["base_instructions"]);
        assert!(state.customizations.is_empty());
        let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["models"], serde_json::json!({}));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn editor_context_defaults_follow_api_updates_and_preserve_explicit_overrides() {
        let mut state = CatalogState {
            sources: parse_sources(MODEL_CATALOG_JSON).unwrap(),
            json: MODEL_CATALOG_JSON.to_string(),
            customizations: Default::default(),
        };
        let mut runtime = runtime_model("gpt-6-astra");
        runtime.context_window = Some(128_000);
        runtime.max_context_window = Some(256_000);
        let snapshot = snapshot_for_state(&[runtime.clone()], &state).unwrap();
        assert_eq!(snapshot.models[0].configuration["context_window"], 128_000);
        assert_eq!(snapshot.models[0].defaults["max_context_window"], 256_000);
        assert_eq!(
            snapshot.models[0].defaults["effective_context_window_percent"],
            95
        );
        let mut configuration = snapshot.models[0].configuration.clone();
        configuration.insert("context_window".to_string(), serde_json::json!(192_000));
        state.customizations = customizations_from_request(
            &snapshot,
            CatalogEditorRequest {
                revision: snapshot.revision.clone(),
                models: vec![CatalogEditorModelRequest {
                    slug: runtime.slug.clone(),
                    configuration,
                }],
            },
        )
        .unwrap();

        runtime.context_window = Some(512_000);
        runtime.max_context_window = Some(1_000_000);
        let updated = snapshot_for_state(&[runtime.clone()], &state).unwrap();
        assert_ne!(updated.revision, snapshot.revision);
        assert_eq!(updated.models[0].defaults["context_window"], 512_000);
        assert_eq!(updated.models[0].defaults["max_context_window"], 1_000_000);
        assert_eq!(updated.models[0].configuration["context_window"], 192_000);
        assert_eq!(
            updated.models[0].configuration["max_context_window"],
            256_000
        );

        state.customizations = customizations_from_request(
            &updated,
            CatalogEditorRequest {
                revision: updated.revision.clone(),
                models: vec![CatalogEditorModelRequest {
                    slug: runtime.slug.clone(),
                    configuration: updated.models[0].defaults.clone(),
                }],
            },
        )
        .unwrap();
        assert!(state.customizations.is_empty());
        let generated =
            prepare_catalog_with_customizations(&[runtime], &state.sources, &state.customizations)
                .unwrap();
        let model: Value = serde_json::from_str(&generated.json).unwrap();
        assert_eq!(model["models"][0]["context_window"], 512_000);
        assert_eq!(model["models"][0]["max_context_window"], 1_000_000);
    }

    #[test]
    fn stale_editor_revision_is_rejected_without_changing_state() {
        let sources = parse_sources(MODEL_CATALOG_JSON).unwrap();
        let mut state = CatalogState {
            sources,
            json: MODEL_CATALOG_JSON.to_string(),
            customizations: Default::default(),
        };
        let runtime_models = vec![runtime_model("third-party-model")];
        let snapshot = snapshot_for_state(&runtime_models, &state).unwrap();
        let path = temporary_path();
        let error = save_for_state(
            &path,
            &runtime_models,
            CatalogEditorRequest {
                revision: "stale".to_string(),
                models: vec![CatalogEditorModelRequest {
                    slug: "third-party-model".to_string(),
                    configuration: snapshot.models[0].configuration.clone(),
                }],
            },
            &mut state,
        )
        .unwrap_err();

        assert_eq!(error, "CODEX_MODEL_CATALOG_CHANGED");
        assert!(state.customizations.is_empty());
        assert!(!path.exists());
    }
}
