use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const MODEL_CATALOG_JSON: &str = include_str!("../resources/claude_models/model-catalog.json");

static CATALOG_STATE: OnceLock<Result<ClaudeCatalog, String>> = OnceLock::new();

#[derive(Clone, Debug)]
struct ClaudeCatalog {
    fallback_context_window: u64,
    context_windows: HashMap<String, u64>,
    display_names: HashMap<String, String>,
    claude_code_effort_levels: HashMap<String, String>,
}

pub(crate) fn context_window_for(
    model_name: &str,
    runtime_context_window: Option<u64>,
) -> Result<u64, String> {
    let catalog = catalog_state()?;
    Ok(runtime_context_window
        .or_else(|| catalog.context_windows.get(&normalize_id(model_name)).copied())
        .unwrap_or(catalog.fallback_context_window))
}

pub(crate) fn display_name_for(model_name: &str) -> Result<Option<String>, String> {
    let catalog = catalog_state()?;
    Ok(catalog
        .display_names
        .get(&normalize_id(model_name))
        .cloned())
}

pub(crate) fn claude_code_effort_level_for(model_name: &str) -> Result<Option<String>, String> {
    let catalog = catalog_state()?;
    Ok(catalog
        .claude_code_effort_levels
        .get(&normalize_id(model_name))
        .cloned())
}

fn catalog_state() -> Result<&'static ClaudeCatalog, String> {
    CATALOG_STATE
        .get_or_init(|| parse_catalog(MODEL_CATALOG_JSON))
        .as_ref()
        .map_err(Clone::clone)
}

fn parse_catalog(content: &str) -> Result<ClaudeCatalog, String> {
    let root: Value = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse bundled Claude model-catalog.json: {error}"))?;
    let root = root
        .as_object()
        .ok_or_else(|| "Bundled Claude model-catalog.json root must be an object".to_string())?;
    let fallback_context_window = root
        .get("fallback_model")
        .and_then(Value::as_object)
        .and_then(|model| model.get("context_window"))
        .and_then(positive_u64)
        .ok_or_else(|| {
            "Bundled Claude model-catalog.json is missing a valid fallback_model.context_window".to_string()
        })?;
    let models = root
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "Bundled Claude model-catalog.json must contain a models array".to_string())?;
    let mut context_windows = HashMap::with_capacity(models.len());
    let mut display_names = HashMap::with_capacity(models.len());
    let mut claude_code_effort_levels = HashMap::with_capacity(models.len());
    let mut model_ids = HashSet::with_capacity(models.len());
    for (index, model) in models.iter().enumerate() {
        let model = model.as_object().ok_or_else(|| {
            format!(
                "Model {} in bundled Claude model-catalog.json must be an object",
                index + 1
            )
        })?;
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "The slug of model {} in bundled Claude model-catalog.json cannot be empty",
                    index + 1
                )
            })?;
        let key = normalize_id(slug);
        if !model_ids.insert(key.clone()) {
            return Err(format!(
                "Bundled Claude model-catalog.json contains duplicate model slugs differing only by case: {slug}"
            ));
        }
        if let Some(context_window) = model.get("context_window").and_then(positive_u64) {
            context_windows.insert(key.clone(), context_window);
        }
        if let Some(display_name) = model
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(slug))
        {
            display_names.insert(key.clone(), display_name.to_string());
        }
        if let Some(effort_level) = model
            .get("claude_code_effort_level")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            claude_code_effort_levels.insert(key, effort_level.to_string());
        }
    }
    Ok(ClaudeCatalog {
        fallback_context_window,
        context_windows,
        display_names,
        claude_code_effort_levels,
    })
}

fn positive_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| {
            value
                .as_i64()
                .filter(|value| *value > 0)
                .map(|value| value as u64)
        })
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn normalize_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{parse_catalog, MODEL_CATALOG_JSON};

    #[test]
    fn bundled_catalog_describes_deepseek_v4() {
        let catalog = parse_catalog(MODEL_CATALOG_JSON).unwrap();
        assert_eq!(catalog.context_windows["deepseek-v4-flash"], 1_000_000);
        assert_eq!(catalog.context_windows["deepseek-v4-pro"], 1_000_000);
        assert!(!catalog
            .claude_code_effort_levels
            .contains_key("deepseek-v4-flash"));
        assert!(!catalog
            .claude_code_effort_levels
            .contains_key("deepseek-v4-pro"));
        assert_eq!(
            catalog.display_names["deepseek-v4-flash"],
            "DeepSeek V4 Flash"
        );
        assert_eq!(catalog.display_names["deepseek-v4-pro"], "DeepSeek V4 Pro");
    }

    #[test]
    fn catalog_prefers_file_override_over_runtime_metadata() {
        let catalog = parse_catalog(
            r#"{
                "fallback_model": {"context_window": 200000},
                "models": [{
                    "slug": "gpt-test",
                    "display_name": "GPT Test",
                    "context_window": 272000,
                    "claude_code_effort_level": "max"
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(catalog.context_windows["gpt-test"], 272000);
        assert_eq!(catalog.display_names["gpt-test"], "GPT Test");
        assert_eq!(catalog.claude_code_effort_levels["gpt-test"], "max");
    }

    #[test]
    fn catalog_rejects_duplicate_model_ids() {
        let error = parse_catalog(
            r#"{
                "fallback_model": {"context_window": 200000},
                "models": [
                    {"slug": "gpt-test", "context_window": 272000},
                    {"slug": "GPT-TEST", "context_window": 128000}
                ]
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("duplicate model slugs differing only by case"));
    }

    #[test]
    fn catalog_allows_model_entries_without_context_override() {
        let catalog = parse_catalog(
            r#"{
                "fallback_model": {"context_window": 200000},
                "models": [{"slug": "runtime-model"}]
            }"#,
        )
        .unwrap();
        assert!(catalog.context_windows.is_empty());
        assert!(catalog.display_names.is_empty());
        assert!(catalog.claude_code_effort_levels.is_empty());
    }
}
