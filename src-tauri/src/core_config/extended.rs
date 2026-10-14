use super::*;
use serde_json::Value as Json;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtendedConfigChange {
    path: Vec<String>,
    #[serde(default)]
    value: Json,
    #[serde(default)]
    remove: bool,
    #[serde(default)]
    expected: Json,
    expected_exists: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtendedConfigResult {
    config: Json,
    restart_required: bool,
}

// Longest prefixes come first. Keep legacy files in their existing layout;
// a native v8 file always reads the new spelling before a legacy fallback.
const PATHS: &[(&str, &str)] = &[
    (
        "client/codex/optimize-multi-agent-v2",
        "codex/optimize-multi-agent-v2",
    ),
    (
        "client/codex/enable-apply-patch",
        "codex/enable-apply-patch",
    ),
    (
        "oauth/providers/claude/disable-claude-cloak-mode",
        "disable-claude-cloak-mode",
    ),
    ("oauth/providers/claude/claude-code", "claude-code"),
    (
        "oauth/providers/claude/header-defaults",
        "claude-header-defaults",
    ),
    (
        "oauth/providers/codex/header-defaults",
        "codex-header-defaults",
    ),
    (
        "oauth/providers/antigravity/antigravity-credits",
        "quota-exceeded/antigravity-credits",
    ),
    (
        "oauth/providers/antigravity/signature-cache-enabled",
        "antigravity-signature-cache-enabled",
    ),
    (
        "oauth/providers/antigravity/signature-bypass-strict",
        "antigravity-signature-bypass-strict",
    ),
    ("oauth/providers/aistudio/ws-auth", "ws-auth"),
    ("oauth/providers/codex", "codex"),
    ("oauth/providers/claude", "claude"),
    ("oauth/providers/antigravity", "antigravity"),
    ("oauth/providers/devin", "devin"),
    ("oauth/providers/xai", "xai"),
    ("server/host", "host"),
    ("server/port", "port"),
    ("server/trusted-proxies", "trusted-proxies"),
    ("server/discovery", "discovery"),
    ("server/tls", "tls"),
    ("server/commercial-mode", "commercial-mode"),
    ("server/github-token", "github-token"),
    ("management", "remote-management"),
    ("credentials/concurrency", "credential-concurrency"),
    ("credentials/in-flight", "credential-in-flight"),
    ("routing/force-model-prefix", "force-model-prefix"),
    ("routing/retry/request-retry", "request-retry"),
    (
        "routing/retry/max-retry-credentials",
        "max-retry-credentials",
    ),
    ("routing/retry/max-retry-interval", "max-retry-interval"),
    ("routing/cooldown/disable-cooling", "disable-cooling"),
    (
        "routing/cooldown/save-cooldown-status",
        "save-cooldown-status",
    ),
    (
        "routing/cooldown/transient-error-cooldown-seconds",
        "transient-error-cooldown-seconds",
    ),
    ("requests/proxy-url", "proxy-url"),
    ("requests/passthrough-headers", "passthrough-headers"),
    (
        "requests/nonstream-keepalive-interval",
        "nonstream-keepalive-interval",
    ),
    ("requests/streaming", "streaming"),
    ("requests/payload", "payload"),
    ("oauth/auth-dir", "auth-dir"),
    (
        "oauth/auth-auto-refresh-workers",
        "auth-auto-refresh-workers",
    ),
    ("oauth/model-alias", "oauth-model-alias"),
    ("oauth/settings", "oauth-settings"),
    ("oauth/excluded-models", "oauth-excluded-models"),
    ("oauth/request-scoped-errors", "oauth-request-scoped-errors"),
    (
        "multimedia/disable-image-generation",
        "disable-image-generation",
    ),
    (
        "multimedia/gpt-image-2-base-model",
        "gpt-image-2-base-model",
    ),
    (
        "multimedia/video-result-auth-cache-ttl",
        "video-result-auth-cache-ttl",
    ),
    ("observability/logs/debug", "debug"),
    ("observability/logs/logging-to-file", "logging-to-file"),
    (
        "observability/logs/logs-max-total-size-mb",
        "logs-max-total-size-mb",
    ),
    (
        "observability/logs/error-logs-max-files",
        "error-logs-max-files",
    ),
    ("observability/logs/request-log", "request-log"),
    (
        "observability/usage/usage-statistics-enabled",
        "usage-statistics-enabled",
    ),
    (
        "observability/usage/redis-usage-queue-retention-seconds",
        "redis-usage-queue-retention-seconds",
    ),
    ("observability/pprof", "pprof"),
];

pub(crate) fn legacy_extended_config_paths() -> Vec<(Vec<&'static str>, Vec<&'static str>)> {
    PATHS
        .iter()
        .map(|(native, legacy)| {
            (
                legacy.split('/').collect::<Vec<_>>(),
                native.split('/').collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn parts(path: &str) -> Vec<String> {
    path.split('/').map(str::to_owned).collect()
}
fn at<'a>(value: &'a Json, path: &[String]) -> Option<&'a Json> {
    path.iter()
        .try_fold(value, |value, key| value.as_object()?.get(key))
}
fn set(value: &mut Json, path: &[String], next: Option<Json>) -> Result<(), String> {
    if next.is_none() {
        fn remove(value: &mut Json, path: &[String]) {
            if let Some((key, rest)) = path.split_first() {
                if let Some(object) = value.as_object_mut() {
                    if rest.is_empty() {
                        object.remove(key);
                    } else if let Some(child) = object.get_mut(key) {
                        remove(child, rest);
                        if child.as_object().is_some_and(|map| map.is_empty()) {
                            object.remove(key);
                        }
                    }
                }
            }
        }
        remove(value, path);
        return Ok(());
    }
    let (key, parents) = path
        .split_last()
        .ok_or("Configuration path cannot be empty")?;
    let mut parent = value;
    for key in parents {
        if next.is_none() && parent.get(key).is_none() {
            return Ok(());
        }
        let object = parent
            .as_object_mut()
            .ok_or_else(|| format!("Configuration section {key} must be an object"))?;
        parent = object
            .entry(key.clone())
            .or_insert_with(|| serde_json::json!({}));
        if parent.is_null() {
            *parent = serde_json::json!({});
        }
    }
    let object = parent
        .as_object_mut()
        .ok_or("Configuration parent must be an object")?;
    if let Some(next) = next {
        object.insert(key.clone(), next);
    } else {
        object.remove(key);
    }
    Ok(())
}
fn with_fallback(current: &Json, fallback: &Json) -> Json {
    match (current.as_object(), fallback.as_object()) {
        (Some(current), Some(fallback)) => {
            if current.is_empty() {
                return Json::Object(current.clone());
            }
            let mut merged = fallback.clone();
            for (key, value) in current {
                merged.insert(
                    key.clone(),
                    fallback
                        .get(key)
                        .map_or_else(|| value.clone(), |old| with_fallback(value, old)),
                );
            }
            Json::Object(merged)
        }
        _ => current.clone(),
    }
}

pub(crate) fn extended_config_view(document: &serde_norway::Value) -> Result<Json, String> {
    let original = serde_json::to_value(document).map_err(|error| error.to_string())?;
    if !original.is_object() {
        return Err("Kernel configuration root must be a mapping".into());
    }
    let mut config = original.clone();
    for field in ["enable-apply-patch", "optimize-multi-agent-v2"] {
        let legacy = parts(&format!("oauth/providers/codex/{field}"));
        let native = parts(&format!("client/codex/{field}"));
        if let Some(old) = at(&config, &legacy).cloned() {
            if at(&config, &native).is_none() {
                set(&mut config, &native, Some(old))?;
            }
            set(&mut config, &legacy, None)?;
        }
    }
    // Move narrow legacy fields first so broader provider blocks do not retain
    // fields which now belong to client.* or a separate header-defaults block.
    for (native, legacy) in PATHS {
        let native = parts(native);
        let legacy = parts(legacy);
        if let Some(old) = at(&config, &legacy).cloned() {
            let value =
                at(&config, &native).map_or_else(|| old.clone(), |new| with_fallback(new, &old));
            set(&mut config, &legacy, None)?;
            set(&mut config, &native, Some(value))?;
        }
    }
    if original.get("api-keys").is_some_and(Json::is_array) {
        if at(&config, &parts("access/api-keys")).is_none() {
            set(
                &mut config,
                &parts("access/api-keys"),
                original.get("api-keys").cloned(),
            )?;
        }
        config.as_object_mut().unwrap().remove("api-keys");
    }
    for (legacy, provider) in V8_PROVIDER_FAMILIES {
        if let Some(records) = original.get(legacy) {
            let path = vec!["api-keys".into(), provider.into()];
            if at(&config, &path).is_none() {
                let records = serde_norway::to_value(records).map_err(|error| error.to_string())?;
                let groups = group_legacy_provider_records(provider, &records)?;
                set(
                    &mut config,
                    &path,
                    Some(serde_json::to_value(groups).map_err(|error| error.to_string())?),
                )?;
            }
            config.as_object_mut().unwrap().remove(legacy);
        }
    }
    Ok(config)
}

fn legacy_path(path: &[String]) -> Vec<String> {
    for (native, legacy) in PATHS {
        let prefix = parts(native);
        if path.starts_with(&prefix) {
            let mut result = parts(legacy);
            result.extend_from_slice(&path[prefix.len()..]);
            return result;
        }
    }
    path.to_vec()
}

// Only template-backed settings are writable. Existing GUI-owned settings and
// system metadata cannot be changed by this command, even via a parent object.
const OBJECT_PATHS: &[&str] = &[
    "credentials/concurrency",
    "credentials/in-flight",
    "client/codex",
    "observability/pprof",
    "multimedia",
    "oauth/providers",
    "oauth/providers/aistudio",
    "oauth/providers/codex",
    "oauth/providers/codex/live-media-relay",
    "oauth/providers/codex/header-defaults",
    "oauth/providers/claude",
    "oauth/providers/claude/claude-code",
    "oauth/providers/claude/header-defaults",
    "oauth/providers/antigravity",
    "oauth/providers/antigravity/connection-pool",
    "oauth/providers/devin",
    "oauth/providers/xai",
];
const FIELD_GROUPS: &[(&str, &str, &str)] = &[
    ("server", "string", "github-token"),
    ("management", "bool", "allow-remote disable-control-panel disable-auto-update-panel"),
    ("management", "string", "base-url panel-github-repository"),
    ("credentials/concurrency", "string", "cpa-heartbeat-timeout cpa-cancel-bound reclaim-grace cleanup-interval release-flush-interval release-max-backoff busy-retry-min busy-retry-max"),
    ("credentials/concurrency", "uint", "max-limit"),
    ("credentials/in-flight", "string", "snapshot-interval stale-after staging-retention"),
    ("credentials/in-flight", "uint", "max-part-bytes max-part-count max-revision-bytes max-aggregate-groups max-details max-string-bytes"),
    ("routing", "bool", "session-affinity-subagents force-model-prefix"),
    ("routing/cooldown", "bool", "save-cooldown-status"),
    ("routing/cooldown", "int", "transient-error-cooldown-seconds"),
    ("requests", "bool", "passthrough-headers"),
    ("requests", "int", "nonstream-keepalive-interval"),
    ("requests/streaming", "int", "keepalive-seconds"),
    ("client/codex", "bool", "enable-apply-patch optimize-multi-agent-v2"),
    ("oauth", "int", "auth-auto-refresh-workers"),
    ("oauth", "auth-dir", "auth-dir"),
    ("oauth/providers/aistudio", "bool", "ws-auth"),
    ("oauth/providers/codex", "bool", "response-steering disable-codex-cloaking stream-bootstrap-buffering orphan-delegation-compatibility model-level-cooling"),
    ("oauth/providers/codex", "string", "stream-bootstrap-timeout"),
    ("oauth/providers/codex/live-media-relay", "bool", "enabled disable-private-remote-ips"),
    ("oauth/providers/codex/live-media-relay", "string", "public-ip"),
    ("oauth/providers/codex/live-media-relay", "uint", "max-sessions"),
    ("oauth/providers/codex/live-media-relay", "port", "udp-port-min udp-port-max"),
    ("oauth/providers/codex/live-media-relay", "ice", "ice-servers"),
    ("oauth/providers/codex/header-defaults", "string", "user-agent beta-features"),
    ("oauth/providers/claude", "bool", "model-level-cooling disable-claude-cloak-mode"),
    ("oauth/providers/claude/claude-code", "bool", "disable-cloaking-model-list"),
    ("oauth/providers/claude/header-defaults", "string", "user-agent package-version runtime-version os arch timeout timezone"),
    ("oauth/providers/claude/header-defaults", "bool", "stabilize-device-profile"),
    ("oauth/providers/antigravity", "bool", "antigravity-credits signature-cache-enabled signature-bypass-strict"),
    ("oauth/providers/antigravity", "strings", "sensitive-words"),
    ("oauth/providers/antigravity/connection-pool", "bool", "enabled"),
    ("oauth/providers/antigravity/connection-pool", "string", "idle-conn-timeout"),
    ("oauth/providers/antigravity/connection-pool", "uint", "max-idle-conns-per-host"),
    ("oauth/providers/devin", "strings", "sensitive-words"),
    ("oauth/providers/xai", "bool", "inject-x-search"),
    ("multimedia", "image-mode", "disable-image-generation"),
    ("multimedia", "string", "gpt-image-2-base-model video-result-auth-cache-ttl"),
    ("observability/logs", "bool", "request-log"),
    ("observability/pprof", "bool", "enable"),
    ("observability/pprof", "string", "addr"),
    ("plugins", "bool", "enabled"),
    ("plugins", "string", "dir"),
    ("plugins", "strings", "store-sources"),
    ("plugins", "store-auth", "store-auth"),
];
fn field_kind(path: &[String]) -> Option<&'static str> {
    let key = path.join("/");
    if OBJECT_PATHS.contains(&key.as_str()) {
        return Some("object");
    }
    let (field, parent) = path.split_last()?;
    let parent = parent.join("/");
    FIELD_GROUPS.iter().find_map(|(prefix, kind, fields)| {
        (*prefix == parent && fields.split_whitespace().any(|name| name == field)).then_some(*kind)
    })
}
fn dynamic_root(path: &[String]) -> Option<&'static str> {
    [
        "plugins/configs",
        "requests/payload",
        "oauth/settings",
        "oauth/model-alias",
        "oauth/excluded-models",
        "oauth/request-scoped-errors",
    ]
    .into_iter()
    .find(|root| path.starts_with(&parts(root)))
}
fn editable(path: &[String]) -> bool {
    if path
        .iter()
        .any(|key| key.is_empty() || key.chars().any(char::is_control))
    {
        return false;
    }
    field_kind(path).is_some() || dynamic_root(path).is_some()
}
fn duration_nanos(value: &str) -> Option<i64> {
    if matches!(value, "0" | "+0" | "-0") {
        return Some(0);
    }
    static PART: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"([0-9]+(?:\.[0-9]*)?|\.[0-9]+)(ns|us|µs|μs|ms|s|m|h)").unwrap()
    });
    let text = value.strip_prefix(['+', '-']).unwrap_or(value);
    if text.is_empty() {
        return None;
    }
    let mut end = 0;
    let mut nanos = 0.0_f64;
    for part in PART.captures_iter(text) {
        let matched = part.get(0).unwrap();
        if matched.start() != end {
            return None;
        }
        let scale = match &part[2] {
            "ns" => 1.0,
            "us" | "µs" | "μs" => 1e3,
            "ms" => 1e6,
            "s" => 1e9,
            "m" => 60e9,
            "h" => 3600e9,
            _ => unreachable!(),
        };
        nanos += part[1].parse::<f64>().unwrap_or(f64::INFINITY) * scale;
        end = matched.end();
    }
    (end == text.len() && nanos < i64::MAX as f64)
        .then(|| (nanos as i64) * if value.starts_with('-') { -1 } else { 1 })
}
fn valid_ice_url(value: &str) -> bool {
    let Some((scheme, address)) = value.split_once(':') else {
        return false;
    };
    if !matches!(scheme, "stun" | "stuns" | "turn" | "turns")
        || address.is_empty()
        || value.chars().any(char::is_whitespace)
    {
        return false;
    }
    reqwest::Url::parse(&format!("{scheme}://{address}")).is_ok_and(|url| {
        url.host_str().is_some() && url.path().is_empty() && url.fragment().is_none()
    })
}
fn validate_payload_rule(kind: &str, rule: &Json) -> bool {
    let Some(models) = rule.get("models").and_then(Json::as_array) else {
        return false;
    };
    if models.is_empty() {
        return false;
    }
    for model in models {
        if !model
            .get("name")
            .and_then(Json::as_str)
            .is_some_and(|name| !name.trim().is_empty())
        {
            return false;
        }
        for field in ["protocol", "from-protocol"] {
            if model.get(field).is_some_and(|value| !value.is_string()) {
                return false;
            }
        }
        if model.get("headers").is_some_and(|value| {
            !value
                .as_object()
                .is_some_and(|headers| headers.values().all(Json::is_string))
        }) {
            return false;
        }
        for field in ["match", "not-match"] {
            if model.get(field).is_some_and(|value| {
                !value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Json::is_object))
            }) {
                return false;
            }
        }
        for field in ["exist", "not-exist"] {
            if model.get(field).is_some_and(|value| {
                !value
                    .as_array()
                    .is_some_and(|values| values.iter().all(Json::is_string))
            }) {
                return false;
            }
        }
    }
    if kind == "filter" {
        return rule
            .get("params")
            .and_then(Json::as_array)
            .is_some_and(|params| params.iter().all(Json::is_string));
    }
    let Some(params) = rule.get("params").and_then(Json::as_object) else {
        return false;
    };
    if kind.ends_with("-raw") {
        return params.values().all(|value| {
            value
                .as_str()
                .is_none_or(|text| serde_json::from_str::<Json>(text).is_ok())
        });
    }
    true
}
fn validate_string(path: &[String], value: &str) -> bool {
    if value.chars().any(char::is_control) {
        return false;
    }
    let key = path.join("/");
    let leaf = path.last().map(String::as_str).unwrap_or_default();
    if leaf == "stream-bootstrap-timeout" {
        let normalized = value.trim().to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "" | "0" | "none" | "unlimited" | "disabled" | "off" | "never"
        ) {
            return true;
        }
        if let Ok(seconds) = normalized.parse::<u64>() {
            return seconds <= (i64::MAX / 1_000_000_000) as u64;
        }
        return duration_nanos(&normalized).is_some();
    }
    if matches!(
        leaf,
        "cpa-heartbeat-timeout"
            | "cpa-cancel-bound"
            | "reclaim-grace"
            | "cleanup-interval"
            | "release-flush-interval"
            | "release-max-backoff"
            | "busy-retry-min"
            | "busy-retry-max"
            | "snapshot-interval"
            | "stale-after"
            | "staging-retention"
            | "idle-conn-timeout"
            | "stream-bootstrap-timeout"
            | "video-result-auth-cache-ttl"
    ) {
        return duration_nanos(value)
            .is_some_and(|nanos| !path.starts_with(&parts("credentials")) || nanos > 0);
    }
    if key == "observability/pprof/addr" {
        return value
            .rsplit_once(':')
            .is_some_and(|(_, port)| port.parse::<u16>().is_ok_and(|port| port > 0))
            && reqwest::Url::parse(&format!("http://{value}")).is_ok_and(|url| {
                url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none()
            });
    }
    if leaf == "public-ip" {
        return value.is_empty() || value.parse::<IpAddr>().is_ok();
    }
    if key == "multimedia/gpt-image-2-base-model" {
        return value.is_empty() || value.to_ascii_lowercase().starts_with("gpt-");
    }
    true
}

fn validate_value(path: &[String], value: &Json, before: &Json) -> Result<(), String> {
    // An unchanged unknown field or system revision inside a subtree is preserved.
    if at(before, path) == Some(value) {
        return Ok(());
    }
    let key = path.join("/");
    let error = || format!("Invalid value for {key}");
    if value.is_null() {
        return Err(format!("{key}: use remove to restore the default"));
    }
    if let Some(root) = dynamic_root(path) {
        if root == "plugins/configs" {
            if path.len() <= 3 && !value.is_object() {
                return Err(error());
            }
            return Ok(());
        }
        if path.len() == 2 {
            let object = value.as_object().ok_or_else(error)?;
            for (channel, value) in object {
                let mut child = path.to_vec();
                child.push(channel.clone());
                validate_value(&child, value, before)?;
            }
            return Ok(());
        }
        if root == "requests/payload"
            && !matches!(
                path[2].as_str(),
                "default" | "default-raw" | "override" | "override-raw" | "filter"
            )
        {
            return Err(error());
        }
        if path.len() != 3 {
            return Err(error());
        }
        let items = value.as_array().ok_or_else(error)?;
        if root == "oauth/excluded-models" {
            return if items.iter().all(Json::is_string) {
                Ok(())
            } else {
                Err(error())
            };
        }
        if !items.iter().all(Json::is_object) {
            return Err(error());
        }
        for item in items {
            if root == "requests/payload" && !validate_payload_rule(&path[2], item) {
                return Err(error());
            }
            if matches!(root, "oauth/model-alias" | "oauth/settings") {
                if !item
                    .get("name")
                    .and_then(Json::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return Err(error());
                }
                if root == "oauth/model-alias"
                    && !item
                        .get("alias")
                        .and_then(Json::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                {
                    return Err(error());
                }
                for field in ["fork", "force-mapping"] {
                    if item.get(field).is_some_and(|value| !value.is_boolean()) {
                        return Err(error());
                    }
                }
                if item
                    .get("max-context-length")
                    .is_some_and(|value| value.as_u64().is_none())
                {
                    return Err(error());
                }
            }
            if root == "oauth/request-scoped-errors" {
                if !item.get("status").is_some_and(|value| {
                    value
                        .as_u64()
                        .is_some_and(|status| (100..=599).contains(&status))
                }) {
                    return Err(error());
                }
                if !matches!(
                    item.get("action").and_then(Json::as_str),
                    Some("stop" | "stop-and-cooldown" | "continue" | "continue-and-cooldown")
                ) {
                    return Err(error());
                }
                for field in ["match", "match-regexr"] {
                    if item.get(field).is_some_and(|value| {
                        !value
                            .as_array()
                            .is_some_and(|items| items.iter().all(Json::is_string))
                    }) {
                        return Err(error());
                    }
                }
            }
        }
        return Ok(());
    }
    match field_kind(path) {
        Some("object") => {
            let object = value.as_object().ok_or_else(error)?;
            for (child, value) in object {
                let mut path = path.to_vec();
                path.push(child.clone());
                validate_value(&path, value, before)?;
            }
        }
        Some("bool") if value.is_boolean() => {}
        Some("auth-dir") => {
            let value = value.as_str().ok_or_else(error)?;
            if value.trim().is_empty()
                || value.trim() != value
                || value.chars().any(char::is_control)
            {
                return Err(error());
            }
        }
        Some("string")
            if value
                .as_str()
                .is_some_and(|value| validate_string(path, value)) => {}
        Some("int") if value.as_i64().is_some() => {}
        Some("uint") if value.as_u64().is_some_and(|value| value <= i64::MAX as u64) => {
            let number = value.as_u64().unwrap();
            let valid = match key.as_str() {
                "credentials/concurrency/max-limit" => (1..=1_000_000).contains(&number),
                "credentials/in-flight/max-part-bytes" => number >= 1024,
                "credentials/in-flight/max-part-count" => (1..=64).contains(&number),
                "credentials/in-flight/max-revision-bytes" => (1..=16_777_216).contains(&number),
                "credentials/in-flight/max-aggregate-groups" => (1..=100_000).contains(&number),
                "credentials/in-flight/max-details" => number <= 10_000,
                "credentials/in-flight/max-string-bytes" => (1..=256).contains(&number),
                _ => true,
            };
            if !valid {
                return Err(error());
            }
        }
        Some("port") if value.as_u64().is_some_and(|port| port <= 65535) => {}
        Some("strings")
            if value.as_array().is_some_and(|items| {
                items.iter().all(|value| {
                    value
                        .as_str()
                        .is_some_and(|text| !text.chars().any(char::is_control))
                })
            }) => {}
        Some("image-mode")
            if value.is_boolean() || matches!(value.as_str(), Some("chat" | "passthrough")) => {}
        Some("ice") => {
            for server in value.as_array().ok_or_else(error)? {
                let object = server.as_object().ok_or_else(error)?;
                if !object.get("urls").is_some_and(|urls| {
                    urls.as_array().is_some_and(|urls| {
                        !urls.is_empty()
                            && urls
                                .iter()
                                .all(|url| url.as_str().is_some_and(valid_ice_url))
                    })
                }) {
                    return Err(error());
                }
                for field in ["username", "credential"] {
                    if object.get(field).is_some_and(|value| !value.is_string()) {
                        return Err(error());
                    }
                }
            }
        }
        Some("store-auth") => {
            for rule in value.as_array().ok_or_else(error)? {
                let object = rule.as_object().ok_or_else(error)?;
                if !object.get("match").is_some_and(Json::is_string) {
                    return Err(error());
                }
                if object.get("type").is_some_and(|value| {
                    !matches!(
                        value.as_str(),
                        Some("" | "none" | "bearer" | "basic" | "header" | "github-token")
                    )
                }) {
                    return Err(error());
                }
                for field in [
                    "token-env",
                    "username-env",
                    "password-env",
                    "header-name",
                    "header-value-env",
                ] {
                    if object.get(field).is_some_and(|value| !value.is_string()) {
                        return Err(error());
                    }
                }
                if object
                    .get("allow-insecure")
                    .is_some_and(|value| !value.is_boolean())
                {
                    return Err(error());
                }
                if object.get("apply-to").is_some_and(|value| {
                    !value.as_array().is_some_and(|items| {
                        items.iter().all(|item| {
                            matches!(item.as_str(), Some("registry" | "metadata" | "artifact"))
                        })
                    })
                }) {
                    return Err(error());
                }
            }
        }
        _ => return Err(error()),
    }
    Ok(())
}

fn remove_yaml_path(mapping: &yaml_edit::Mapping, path: &[String]) {
    if let Some((key, rest)) = path.split_first() {
        if rest.is_empty() {
            mapping.remove(key.as_str());
        } else if let Some(node) = mapping.get(key.as_str()) {
            if let Some(child) = node.as_mapping() {
                remove_yaml_path(&child, rest);
            }
        }
    }
}

fn render_patch(
    content: &str,
    original: &serde_norway::Value,
    updated: &serde_norway::Value,
) -> Result<String, String> {
    fn deletions(
        before: &serde_norway::Value,
        after: &serde_norway::Value,
        path: &mut Vec<String>,
        result: &mut Vec<Vec<String>>,
    ) {
        if let (Some(before), Some(after)) = (before.as_mapping(), after.as_mapping()) {
            for (key, value) in before {
                if let Some(key) = key.as_str() {
                    path.push(key.into());
                    if let Some(next) = after.get(yaml_key(key)) {
                        deletions(value, next, path, result);
                    } else {
                        result.push(path.clone());
                    }
                    path.pop();
                }
            }
        }
    }
    let mut removed = Vec::new();
    deletions(original, updated, &mut Vec::new(), &mut removed);
    if removed.is_empty() {
        return render_yaml_value_changes(content, original, updated);
    }
    let file = content
        .parse::<yaml_edit::YamlFile>()
        .map_err(|error| error.to_string())?;
    let root = file
        .document()
        .and_then(|document| document.as_mapping())
        .ok_or("Configuration root must be a mapping")?;
    for path in removed {
        remove_yaml_path(&root, &path);
    }
    let prepared = file.to_string();
    let before = serde_norway::from_str(&prepared).map_err(|error| error.to_string())?;
    render_yaml_value_changes(&prepared, &before, updated)
}

fn prepare_patch(
    content: &str,
    changes: &[ExtendedConfigChange],
) -> Result<(String, ExtendedConfigResult), String> {
    let original: serde_norway::Value =
        serde_norway::from_str(content).map_err(|error| format!("Invalid kernel YAML: {error}"))?;
    let before = extended_config_view(&original)?;
    let mut native = before.clone();
    for (index, change) in changes.iter().enumerate() {
        if !editable(&change.path) {
            return Err(format!("Setting {} is read-only", change.path.join("/")));
        }
        if changes[..index].iter().any(|other| {
            change.path.starts_with(&other.path) || other.path.starts_with(&change.path)
        }) {
            return Err("Configuration changes must not overlap".into());
        }
        let current = at(&before, &change.path);
        if current.is_some() != change.expected_exists
            || current.is_some_and(|value| *value != change.expected)
        {
            return Err(format!(
                "Setting {} changed externally. Reload before saving",
                change.path.join("/")
            ));
        }
        if !change.remove {
            validate_value(&change.path, &change.value, &before)?;
        }
        set(
            &mut native,
            &change.path,
            (!change.remove).then(|| change.value.clone()),
        )?;
    }
    // Even replacing a whole subtree cannot alter system-owned revisions.
    for path in [
        "credentials/concurrency/lifecycle-config-revision",
        "credentials/concurrency/observation-barrier-revision",
        "plugins/auth-revision",
    ] {
        if at(&before, &parts(path)) != at(&native, &parts(path)) {
            return Err(format!("Setting {path} is read-only"));
        }
    }
    let changed = |path: &str| at(&before, &parts(path)) != at(&native, &parts(path));
    let duration = |path: &str, fallback: i64| {
        at(&native, &parts(path))
            .and_then(Json::as_str)
            .and_then(duration_nanos)
            .unwrap_or(fallback)
    };
    for (minimum, maximum, default_min, default_max) in [
        (
            "credentials/concurrency/release-flush-interval",
            "credentials/concurrency/release-max-backoff",
            250_000_000,
            2_000_000_000,
        ),
        (
            "credentials/concurrency/busy-retry-min",
            "credentials/concurrency/busy-retry-max",
            250_000_000,
            1_000_000_000,
        ),
    ] {
        if changed(minimum) || changed(maximum) {
            let min = duration(minimum, default_min);
            let max = duration(maximum, default_max);
            if min > max
                || (minimum.ends_with("busy-retry-min")
                    && (min % 1_000_000 != 0 || max % 1_000_000 != 0))
            {
                return Err(format!(
                    "Invalid credential duration range: {minimum} / {maximum}"
                ));
            }
        }
    }
    if changed("credentials/in-flight/snapshot-interval")
        || changed("credentials/in-flight/stale-after")
    {
        if duration("credentials/in-flight/snapshot-interval", 2_000_000_000)
            > duration("credentials/in-flight/stale-after", 10_000_000_000) / 3
        {
            return Err("Credential stale-after must be at least three snapshot intervals".into());
        }
    }
    if ["max-part-bytes", "max-part-count", "max-revision-bytes"]
        .iter()
        .any(|field| changed(&format!("credentials/in-flight/{field}")))
    {
        let number = |field: &str, default| {
            at(&native, &parts(&format!("credentials/in-flight/{field}")))
                .and_then(Json::as_u64)
                .unwrap_or(default)
        };
        let part = number("max-part-bytes", 262144);
        let count = number("max-part-count", 64);
        let revision = number("max-revision-bytes", 16777216);
        if part == 0 || revision < part || revision.saturating_add(part - 1) / part > count {
            return Err("Credential revision size exceeds the configured part capacity".into());
        }
    }
    let relay = at(&native, &parts("oauth/providers/codex/live-media-relay"));
    if let Some(relay) = relay.filter(|_| {
        changes
            .iter()
            .any(|change| change.path.starts_with(&parts("oauth/providers")))
    }) {
        let min = relay
            .get("udp-port-min")
            .and_then(Json::as_u64)
            .unwrap_or(0);
        let max = relay
            .get("udp-port-max")
            .and_then(Json::as_u64)
            .unwrap_or(0);
        let sessions = relay
            .get("max-sessions")
            .and_then(Json::as_u64)
            .filter(|value| *value > 0)
            .unwrap_or(32);
        if (min == 0) != (max == 0)
            || (min > 0 && (max < min || max - min + 1 < sessions.saturating_mul(2)))
        {
            return Err(
                "The UDP port range must provide at least two ports per media session".into(),
            );
        }
    }
    let v8 = core_config_uses_v8(&original);
    let mut updated = serde_json::to_value(&original).map_err(|error| error.to_string())?;
    fn write_legacy(updated: &mut Json, path: &[String], value: &Json) -> Result<(), String> {
        if let Some(object) = value.as_object().filter(|object| !object.is_empty()) {
            for (key, value) in object {
                let mut child = path.to_vec();
                child.push(key.clone());
                write_legacy(updated, &child, value)?;
            }
            return Ok(());
        }
        let legacy = legacy_path(path);
        // Empty maps may share a legacy block with settings now owned by client.*.
        let target = if value.as_object().is_some() && at(updated, &legacy).is_some() {
            path
        } else {
            &legacy
        };
        set(updated, target, Some(value.clone()))
    }
    for change in changes {
        if at(&before, &change.path) == at(&native, &change.path) {
            continue;
        }
        let mut mappings: Vec<(Vec<String>, Vec<String>)> = PATHS
            .iter()
            .map(|(native, legacy)| (parts(native), parts(legacy)))
            .collect();
        for field in ["enable-apply-patch", "optimize-multi-agent-v2"] {
            mappings.push((
                parts(&format!("client/codex/{field}")),
                parts(&format!("oauth/providers/codex/{field}")),
            ));
        }
        let mut removals = vec![change.path.clone()];
        for (canonical, legacy) in &mappings {
            if change.path.starts_with(canonical) {
                let mut target = legacy.clone();
                target.extend_from_slice(&change.path[canonical.len()..]);
                removals.push(target);
            } else if canonical.starts_with(&change.path) {
                removals.push(legacy.clone());
            }
        }
        // A provider block can contain old client flags. Preserve those values
        // while clearing every spelling of the edited settings, including hidden
        // legacy fallbacks underneath an explicit empty native object.
        let preserved: Vec<_> = mappings
            .iter()
            .filter(|(canonical, legacy)| {
                !canonical.starts_with(&change.path)
                    && removals.iter().any(|path| legacy.starts_with(path))
            })
            .filter_map(|(_, legacy)| {
                at(&updated, legacy)
                    .cloned()
                    .map(|value| (legacy.clone(), value))
            })
            .collect();
        for path in removals {
            set(&mut updated, &path, None)?;
        }
        for (path, value) in preserved {
            set(&mut updated, &path, Some(value))?;
        }
        if let Some(value) = at(&native, &change.path) {
            if v8 {
                set(&mut updated, &change.path, Some(value.clone()))?;
            } else {
                write_legacy(&mut updated, &change.path, value)?;
            }
        }
    }
    let updated = serde_norway::to_value(updated).map_err(|error| error.to_string())?;
    let rendered = render_patch(content, &original, &updated)?;
    let config = extended_config_view(&updated)?;
    for change in changes {
        if at(&config, &change.path) != at(&native, &change.path) {
            return Err(format!(
                "Cannot preserve mixed-layout configuration for {}",
                change.path.join("/")
            ));
        }
    }
    let restart_required = changes.iter().any(|change| {
        at(&before, &change.path) != at(&config, &change.path)
            && [
                "server/host",
                "server/port",
                "server/tls",
                "management",
                "oauth/auth-dir",
            ]
            .iter()
            .any(|prefix| change.path.starts_with(&parts(prefix)))
    });
    Ok((
        rendered,
        ExtendedConfigResult {
            config,
            restart_required,
        },
    ))
}

#[tauri::command]
pub(crate) fn get_extended_core_config() -> Result<Json, String> {
    let _guard = lock_core_config_file()?;
    let path = core_install_dir()?.join(CORE_CONFIG_FILE);
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Cannot read kernel configuration: {error}"))?;
    let document = serde_norway::from_str(&content)
        .map_err(|error| format!("Invalid kernel YAML: {error}"))?;
    extended_config_view(&document)
}

#[tauri::command]
pub(crate) fn save_extended_core_config(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    changes: Vec<ExtendedConfigChange>,
) -> Result<ExtendedConfigResult, String> {
    let _guard = lock_core_config_file()?;
    let path = core_install_dir()?.join(CORE_CONFIG_FILE);
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Cannot read kernel configuration: {error}"))?;
    let (updated, result) = prepare_patch(&content, &changes)?;
    let mut config_guard = gui_config_state
        .inner
        .lock()
        .map_err(|_| "GUI configuration lock is poisoned")?;
    let previous = config_guard.clone();
    let mut next = previous.clone();
    let image_preference_changed = record_image_generation_preference(&mut next, &changes);
    if updated == content && !image_preference_changed {
        return Ok(result);
    }
    if changes
        .iter()
        .any(|change| change.path == parts("observability/logs/request-log"))
    {
        next.request_log = at(&result.config, &parts("observability/logs/request-log"))
            .and_then(Json::as_bool)
            .unwrap_or(false);
    }
    if changes
        .iter()
        .any(|change| change.path == parts("plugins/enabled"))
    {
        next.plugins_enabled = at(&result.config, &parts("plugins/enabled"))
            .and_then(Json::as_bool)
            .unwrap_or(false);
    }
    if changes
        .iter()
        .any(|change| change.path == parts("oauth/auth-dir"))
    {
        next.auth_dir = at(&result.config, &parts("oauth/auth-dir"))
            .and_then(Json::as_str)
            .unwrap_or("~/.cli-proxy-api")
            .to_string();
        auth_dir_path_for_core(&next.auth_dir, &core_install_dir()?)?;
        next.auth_dir_user_selected = true;
    }
    validate_gui_config(&next)?;
    if fs::read_to_string(&path).map_err(|error| error.to_string())? != content {
        return Err("Configuration changed externally. Reload before saving".into());
    }
    // Preserve the inode watched by the running core. Every change is validated
    // before this single write, and restore both stores if persistence fails.
    let gui_changed = next.request_log != previous.request_log
        || image_preference_changed
        || next.plugins_enabled != previous.plugins_enabled
        || next.auth_dir != previous.auth_dir
        || next.auth_dir_user_selected != previous.auth_dir_user_selected;
    if let Err(error) = write_core_config_if_changed(&path, &updated).and_then(|_| {
        if gui_changed {
            write_gui_config(&next)
        } else {
            Ok(())
        }
    }) {
        let rollback = write_core_config_if_changed(&path, &content).err();
        return Err(config_update_error_with_rollback(error, rollback));
    }
    *config_guard = next;
    Ok(result)
}

fn record_image_generation_preference(config: &mut GuiConfigFile, changes: &[ExtendedConfigChange]) -> bool {
    if config.image_generation_user_selected
        || !changes.iter().any(|change| change.path == parts("multimedia/disable-image-generation"))
    {
        return false;
    }
    config.image_generation_user_selected = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(content: &str, path: &str, value: Json, remove: bool) -> ExtendedConfigChange {
        let document = serde_norway::from_str(content).unwrap();
        let view = extended_config_view(&document).unwrap();
        let path = parts(path);
        let current = at(&view, &path);
        ExtendedConfigChange {
            path: path.clone(),
            value,
            remove,
            expected: current.cloned().unwrap_or(Json::Null),
            expected_exists: current.is_some(),
        }
    }

    #[test]
    fn image_generation_user_choice_survives_startup_including_enable_and_reset() {
        for initial in [
            "config-version: 8\nmultimedia: {disable-image-generation: false}\napi-keys: {codex: [{name: relay, keys: [{api-key: test-key}]}]}\n",
            "disable-image-generation: false\ncodex-api-key: [{api-key: test-key}]\n",
        ] {
            for (value, remove) in [(json!(false), false), (json!(true), false), (json!("chat"), false), (json!("passthrough"), false), (Json::Null, true)] {
                let edits = vec![change(initial, "multimedia/disable-image-generation", value.clone(), remove)];
                let (saved, _) = prepare_patch(initial, &edits).unwrap();
                let mut config = GuiConfigFile::default();
                assert!(record_image_generation_preference(&mut config, &edits));
                assert!(!record_image_generation_preference(&mut config, &edits));
                let restarted = merge_core_config_yaml(initial, Some(&saved), &config).unwrap();
                let document = serde_norway::from_str(&restarted).unwrap();
                let view = extended_config_view(&document).unwrap();
                let actual = at(&view, &parts("multimedia/disable-image-generation"));
                assert_eq!(actual, if remove { None } else { Some(&value) });
            }
        }
        let initial = "config-version: 8\nplugins: {enabled: false}\n";
        let edits = vec![change(initial, "plugins/enabled", json!(true), false)];
        let mut config = GuiConfigFile::default();
        assert!(!record_image_generation_preference(&mut config, &edits));
        assert!(!config.image_generation_user_selected);
    }

    #[test]
    fn extended_view_preserves_native_false_zero_empty_and_turn_secrets() {
        let document = serde_norway::from_str(
            r#"
config-version: 8
codex:
  response-steering: true
  optimize-multi-agent-v2: true
  live-media-relay:
    max-sessions: 7
    ice-servers: [{urls: ['turn:example'], username: user, credential: 'secret-value'}]
oauth:
  providers:
    codex:
      response-steering: false
      live-media-relay: {max-sessions: 0, public-ip: ''}
client: {codex: {optimize-multi-agent-v2: false}}
"#,
        )
        .unwrap();
        let view = extended_config_view(&document).unwrap();
        assert_eq!(
            view["oauth"]["providers"]["codex"]["response-steering"],
            false
        );
        assert_eq!(
            view["oauth"]["providers"]["codex"]["live-media-relay"]["max-sessions"],
            0
        );
        assert_eq!(
            view["oauth"]["providers"]["codex"]["live-media-relay"]["public-ip"],
            ""
        );
        assert_eq!(
            view["oauth"]["providers"]["codex"]["live-media-relay"]["ice-servers"][0]["credential"],
            "secret-value"
        );
        assert_eq!(view["client"]["codex"]["optimize-multi-agent-v2"], false);
        assert!(view.get("codex").is_none());
    }

    #[test]
    fn extended_patch_preserves_comments_provider_groups_and_credentials() {
        let content = "# keep heading\nconfig-version: 8\nserver: {host: '', port: 8317}\napi-keys: {codex: [{name: group, keys: [{api-key: upstream-secret}]}]}\noauth:\n  providers:\n    codex:\n      live-media-relay:\n        enabled: false # preserve hint\n        ice-servers: [{urls: ['turn:example'], username: user, credential: 'turn-secret'}]\n";
        let changes = [
            change(
                content,
                "oauth/providers/codex/response-steering",
                json!(true),
                false,
            ),
            change(content, "management/allow-remote", json!(true), false),
        ];
        let (rendered, result) = prepare_patch(content, &changes).unwrap();
        assert!(rendered.contains("# keep heading"));
        assert!(rendered.contains("# preserve hint"));
        assert_eq!(
            result.config["api-keys"]["codex"][0]["keys"][0]["api-key"],
            "upstream-secret"
        );
        assert_eq!(
            result.config["oauth"]["providers"]["codex"]["live-media-relay"]["ice-servers"][0]
                ["credential"],
            "turn-secret"
        );
        assert!(result.restart_required);
    }

    #[test]
    fn extended_conflicting_or_invalid_batches_are_rejected() {
        let content = "config-version: 8\nplugins: {enabled: false}\n";
        let valid = change(content, "plugins/enabled", json!(true), false);
        let invalid = change(content, "server/port", json!(2), false);
        assert!(prepare_patch(content, &[valid.clone(), invalid]).is_err());
        let mut stale = valid.clone();
        stale.expected = json!(true);
        assert!(prepare_patch(content, &[stale])
            .unwrap_err()
            .contains("changed externally"));
        for (path, value) in [
            ("plugins/enabled", json!({})),
            ("oauth/providers/codex/unknown", json!("x")),
            ("multimedia/disable-image-generation", json!("bad")),
            ("credentials/concurrency/max-limit", json!(-1)),
            ("oauth/model-alias/plugin-x", json!("bad")),
        ] {
            assert!(
                prepare_patch(content, &[change(content, path, value, false)]).is_err(),
                "{path}"
            );
        }
        assert!(prepare_patch("bad: [yaml", &[valid]).is_err());
    }

    #[test]
    fn extended_subtrees_preserve_revisions_and_validate_new_values() {
        let content = "config-version: 8\ncredentials:\n  concurrency: {lifecycle-config-revision: 7, max-limit: 12}\n";
        let good = change(
            content,
            "credentials/concurrency",
            json!({"lifecycle-config-revision":7,"max-limit":20}),
            false,
        );
        assert_eq!(
            prepare_patch(content, &[good]).unwrap().1.config["credentials"]["concurrency"]
                ["lifecycle-config-revision"],
            7
        );
        let remove = change(content, "credentials/concurrency", Json::Null, true);
        assert!(prepare_patch(content, &[remove]).is_err());
        let bad = change(
            content,
            "credentials/concurrency",
            json!({"lifecycle-config-revision":8,"max-limit":20}),
            false,
        );
        assert!(prepare_patch(content, &[bad]).is_err());
    }

    #[test]
    fn extended_legacy_provider_subtree_changes_preserve_client_flags() {
        let content = "codex:\n  optimize-multi-agent-v2: true # client flag\n  response-steering: true\n  live-media-relay: {enabled: false}\nclaude-header-defaults: {user-agent: old}\n";
        let change = change(
            content,
            "oauth/providers",
            json!({"claude":{"header-defaults":{"user-agent":"new"}}}),
            false,
        );
        let (rendered, result) = prepare_patch(content, &[change]).unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert_eq!(document["codex"]["optimize-multi-agent-v2"], true);
        assert!(document["codex"].get("response-steering").is_none());
        assert_eq!(document["claude-header-defaults"]["user-agent"], "new");
        assert_eq!(
            result.config["client"]["codex"]["optimize-multi-agent-v2"],
            true
        );
    }

    #[test]
    fn extended_mixed_remove_does_not_resurrect_legacy_fallback() {
        let content = "config-version: 8\ncodex: {response-steering: true, live-media-relay: {enabled: true}}\noauth: {providers: {codex: {response-steering: false, live-media-relay: {enabled: false}}}}\n";
        let (rendered, result) = prepare_patch(
            content,
            &[change(content, "oauth/providers/codex", Json::Null, true)],
        )
        .unwrap();
        assert!(at(&result.config, &parts("oauth/providers/codex")).is_none());
        assert!(!rendered.contains("response-steering"));
    }

    #[test]
    fn extended_empty_maps_and_disjoint_external_changes_survive() {
        let original = "config-version: 8\nplugins: {configs: {example: {enabled: true}}}\n";
        let empty = change(original, "plugins/configs", json!({}), false);
        let disk = format!("{original}custom-extension: {{untouched: true}}\n");
        let (_, result) = prepare_patch(&disk, &[empty]).unwrap();
        assert_eq!(result.config["plugins"]["configs"], json!({}));
        assert_eq!(result.config["custom-extension"]["untouched"], true);
    }

    #[test]
    fn extended_dynamic_plugin_channels_and_ice_are_writable() {
        let content = "config-version: 8\n";
        let changes = [
            change(
                content,
                "oauth/model-alias/plugin-example",
                json!([{"name":"upstream","alias":"client","fork":false}]),
                false,
            ),
            change(
                content,
                "oauth/settings/meta",
                json!([{"name":"muse","max-context-length":524288}]),
                false,
            ),
            change(
                content,
                "oauth/providers/codex/live-media-relay",
                json!({"max-sessions":2,"udp-port-min":10000,"udp-port-max":10003,"ice-servers":[{"urls":["turn:example"],"username":"","credential":"secret"}]}),
                false,
            ),
        ];
        let (_, result) = prepare_patch(content, &changes).unwrap();
        assert_eq!(
            result.config["oauth"]["model-alias"]["plugin-example"][0]["fork"],
            false
        );
        let bad = change(
            content,
            "oauth/providers/codex/live-media-relay",
            json!({"max-sessions":2,"udp-port-min":10000,"udp-port-max":10002}),
            false,
        );
        assert!(prepare_patch(content, &[bad]).is_err());
    }
    #[test]
    fn extended_validates_network_duration_payload_and_alias_values() {
        let content = "config-version: 8\n";
        assert!(validate_string(
            &parts("oauth/providers/codex/stream-bootstrap-timeout"),
            "15"
        ));
        assert!(validate_string(
            &parts("oauth/providers/codex/stream-bootstrap-timeout"),
            ""
        ));
        assert!(!validate_string(
            &parts("oauth/providers/codex/stream-bootstrap-timeout"),
            "999999999999999999"
        ));
        let rejected = [
            ("observability/pprof/addr", json!("bad-address")),
            (
                "oauth/providers/codex/live-media-relay/public-ip",
                json!("example.com"),
            ),
            (
                "oauth/providers/codex/live-media-relay/ice-servers",
                json!([{"urls":["https://example.com"]}]),
            ),
            ("multimedia/video-result-auth-cache-ttl", json!("tomorrow")),
            ("multimedia/gpt-image-2-base-model", json!("claude-model")),
            ("credentials/concurrency/cpa-heartbeat-timeout", json!("0s")),
            ("credentials/concurrency/busy-retry-min", json!("1.1ms")),
            (
                "credentials/concurrency/release-flush-interval",
                json!("3s"),
            ),
            ("credentials/in-flight/snapshot-interval", json!("4s")),
            ("credentials/in-flight/max-part-count", json!(1)),
            (
                "requests/payload/override-raw",
                json!([{"models":[{"name":"gpt-*"}],"params":{"x":"not-json"}}]),
            ),
            (
                "requests/payload/filter",
                json!([{"models":[{"name":"gpt-*"}],"params":{"x":true}}]),
            ),
            (
                "requests/payload/default",
                json!([{"models":[{"name":"gpt-*","headers":["bad"]}],"params":{"x":1}}]),
            ),
            (
                "oauth/model-alias/plugin-x",
                json!([{"name":"model","alias":" "}]),
            ),
            ("oauth/auth-dir", json!("")),
        ];
        for (path, value) in rejected {
            assert!(
                prepare_patch(content, &[change(content, path, value, false)]).is_err(),
                "{path}"
            );
        }
        let accepted = [
            ("observability/pprof/addr", json!("[::1]:8316")),
            (
                "oauth/providers/codex/live-media-relay/public-ip",
                json!(""),
            ),
            ("multimedia/video-result-auth-cache-ttl", json!("1h30m")),
            (
                "oauth/providers/codex/stream-bootstrap-timeout",
                json!("unlimited"),
            ),
            (
                "oauth/providers/codex/stream-bootstrap-timeout",
                json!("20s"),
            ),
            (
                "requests/payload/override-raw",
                json!([{"models":[{"name":"gpt-*"}],"params":{"object":{"x":true},"number":123,"bool":false,"encoded":"{\"valid\":true}"}}]),
            ),
            (
                "requests/payload/filter",
                json!([{"models":[{"name":"gpt-*"}],"params":["tools.0"]}]),
            ),
            (
                "oauth/settings/meta",
                json!([{"name":"muse","max-context-length":0}]),
            ),
            ("oauth/auth-dir", json!("~/custom-oauth")),
            (
                "plugins/store-auth",
                json!([{"match":"https://example.com/", "type":"none", "apply-to":["metadata"]}, {"match":"https://example.org/", "apply-to":[]}]),
            ),
        ];
        for (path, value) in accepted {
            prepare_patch(content, &[change(content, path, value, false)])
                .unwrap_or_else(|error| panic!("{path}: {error}"));
        }
    }

    #[test]
    fn extended_deleting_empty_native_object_removes_hidden_legacy_fallback() {
        let content = "config-version: 8\ncodex: {response-steering: true, optimize-multi-agent-v2: true}\noauth: {providers: {codex: {}}}\n";
        let (_, result) = prepare_patch(
            content,
            &[change(content, "oauth/providers/codex", Json::Null, true)],
        )
        .unwrap();
        assert!(at(&result.config, &parts("oauth/providers/codex")).is_none());
        assert_eq!(
            result.config["client"]["codex"]["optimize-multi-agent-v2"],
            true
        );
    }

    #[test]
    fn extended_auth_directory_edit_marks_restart_and_removes_legacy_value() {
        let content = "config-version: 8\nauth-dir: old\noauth: {auth-dir: current}\n";
        let (rendered, result) = prepare_patch(
            content,
            &[change(
                content,
                "oauth/auth-dir",
                json!("~/selected"),
                false,
            )],
        )
        .unwrap();
        assert!(result.restart_required);
        assert_eq!(result.config["oauth"]["auth-dir"], "~/selected");
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert!(document.get("auth-dir").is_none());
    }

    #[test]
    fn extended_github_token_is_writable_in_native_and_legacy_layouts() {
        let native = "config-version: 8\nserver: {port: 8317}\n";
        let (rendered, result) = prepare_patch(
            native,
            &[change(
                native,
                "server/github-token",
                json!("github-secret"),
                false,
            )],
        )
        .unwrap();
        assert_eq!(result.config["server"]["github-token"], "github-secret");
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert_eq!(document["server"]["github-token"], "github-secret");
        assert_eq!(document["server"]["port"], 8317);

        let configured =
            "config-version: 8\nserver: {github-token: kept-secret}\nplugins: {enabled: false}\n";
        let (rendered, _) = prepare_patch(
            configured,
            &[change(configured, "plugins/enabled", json!(true), false)],
        )
        .unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert_eq!(document["server"]["github-token"], "kept-secret");

        let legacy = "port: 8317\ngithub-token: old-secret\n";
        let view = extended_config_view(&serde_norway::from_str(legacy).unwrap()).unwrap();
        assert_eq!(view["server"]["github-token"], "old-secret");
        let (rendered, _) = prepare_patch(
            legacy,
            &[change(
                legacy,
                "server/github-token",
                json!("new-secret"),
                false,
            )],
        )
        .unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert_eq!(document["github-token"], "new-secret");
        assert!(document.get("server").is_none());

        let (rendered, _) = prepare_patch(
            legacy,
            &[change(legacy, "server/github-token", Json::Null, true)],
        )
        .unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert!(document.get("github-token").is_none());
    }

    #[test]
    fn extended_restart_hint_distinguishes_live_updates_from_startup_settings() {
        let content = "config-version: 8\n";
        for (path, value) in [
            (
                "observability/pprof",
                json!({"enable": true, "addr": "127.0.0.1:8316"}),
            ),
            ("observability/pprof/addr", json!("127.0.0.1:9316")),
            ("plugins/dir", json!("custom-plugins")),
            ("plugins/enabled", json!(true)),
            ("server/github-token", json!("github-secret")),
        ] {
            let (_, result) =
                prepare_patch(content, &[change(content, path, value, false)]).unwrap();
            assert!(!result.restart_required, "{path} supports live updates");
        }
        for (path, value) in [
            ("management/allow-remote", json!(true)),
            ("oauth/auth-dir", json!("~/selected-oauth")),
        ] {
            let (_, result) =
                prepare_patch(content, &[change(content, path, value, false)]).unwrap();
            assert!(result.restart_required, "{path} requires a restart");
        }
        let unchanged = "config-version: 8\nmanagement: {allow-remote: false}\n";
        let (_, result) = prepare_patch(
            unchanged,
            &[change(
                unchanged,
                "management/allow-remote",
                json!(false),
                false,
            )],
        )
        .unwrap();
        assert!(!result.restart_required);
    }
}
