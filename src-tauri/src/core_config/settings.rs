use super::*;

pub(crate) fn validate_core_api_key(api_key: &str) -> Result<(), String> {
    if api_key.is_empty() {
        return Err("Authentication key cannot be empty".to_string());
    }
    if !api_key.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err("Authentication key may contain only visible ASCII characters and cannot contain spaces".to_string());
    }
    if is_example_core_api_key(api_key) {
        return Err("The example authentication key from the kernel template cannot be used".to_string());
    }
    Ok(())
}

pub(crate) fn validate_api_key_remark(remark: &str) -> Result<(), String> {
    if remark.chars().count() > 80 {
        return Err("Key note cannot exceed 80 characters".to_string());
    }
    if remark.chars().any(char::is_control) {
        return Err("Key note cannot contain line breaks or control characters".to_string());
    }
    Ok(())
}

pub(crate) fn validate_api_access_provider_section(section: &str) -> Result<(), String> {
    if matches!(
        section,
        "gemini-api-key" | "interactions-api-key" | "vertex-api-key" | "codex-api-key"
            | "claude-api-key" | "xai-api-key" | "meta-api-key" | "openai-compatibility"
    ) {
        Ok(())
    } else {
        Err("Invalid API access type".to_string())
    }
}

pub(crate) fn api_access_key_hash(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| sha256_bytes(value.as_bytes()))
}

pub(crate) fn usage_provider_section(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => Some("codex-api-key"),
        "claude" => Some("claude-api-key"),
        "gemini" | "aistudio" => Some("gemini-api-key"),
        "interactions" => Some("interactions-api-key"),
        "vertex" | "vertex-ai" => Some("vertex-api-key"),
        "xai" | "grok" => Some("xai-api-key"),
        "meta" => Some("meta-api-key"),
        "openai" | "openai-compatibility" => Some("openai-compatibility"),
        _ => None,
    }
}

impl GuiConfigFile {
    pub(crate) fn api_access_remark_for_source(
        &self,
        provider: &str,
        source: &str,
    ) -> Option<&str> {
        let hash = api_access_key_hash(source)?;
        let preferred_section = usage_provider_section(provider);
        let resolve = |section: Option<&str>| {
            let mut resolved: Option<Option<&str>> = None;
            for entry in self.api_access_remarks.iter().filter(|entry| {
                section.is_none_or(|section| entry.provider_section == section)
                    && entry.api_key_hash == hash
            }) {
                let candidate = (!entry.remark.is_empty()).then_some(entry.remark.as_str());
                match resolved {
                    None => resolved = Some(candidate),
                    Some(current) if current == candidate => {}
                    Some(_) => return Some(None),
                }
            }
            resolved
        };

        if let Some(section) = preferred_section {
            if let Some(remark) = resolve(Some(section)) {
                return remark;
            }
        }
        resolve(None).flatten()
    }
}

pub(crate) fn validate_management_secret_key(secret_key: &str) -> Result<(), String> {
    if secret_key.chars().count() > 512 {
        return Err("Management key cannot exceed 512 characters".to_string());
    }
    if secret_key.chars().any(char::is_control) {
        return Err("Management key cannot contain control characters".to_string());
    }
    Ok(())
}

pub(crate) fn validate_strong_management_secret_key(secret_key: &str) -> Result<(), String> {
    validate_management_secret_key(secret_key)?;
    if secret_key.trim().is_empty() {
        return Err("WebUI key cannot be empty".to_string());
    }
    if is_hashed_management_secret_key(secret_key) {
        return Err("GUI configuration must save a plaintext WebUI key that can authenticate with the management interface".to_string());
    }
    Ok(())
}

pub(crate) fn management_secret_requires_rotation(secret_key: &str) -> bool {
    let secret_key = secret_key.trim();
    // Empty is an intentional disabled Management API, including after restart.
    !secret_key.is_empty() && validate_strong_management_secret_key(secret_key).is_err()
}

pub(crate) fn ensure_strong_management_secret(config: &mut GuiConfigFile) -> Result<bool, String> {
    if !management_secret_requires_rotation(&config.management_secret_key) {
        return Ok(false);
    }
    config.management_secret_key = LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string();
    Ok(true)
}

// A hashed kernel value cannot supply a request credential. If the GUI saved
// an empty key while the kernel still has an enabled key, recover both sides
// to the same usable credential. A disabled kernel stays disabled.
pub(crate) fn recover_management_secret(
    imported: &str,
    saved: Option<&str>,
    kernel_key: Option<&str>,
) -> String {
    if validate_strong_management_secret_key(imported).is_ok() {
        return imported.trim().to_string();
    }
    if let Some(saved) = saved {
        if validate_strong_management_secret_key(saved).is_ok() {
            return saved.trim().to_string();
        }
        if saved.is_empty() {
            if let Some(kernel_key) = kernel_key {
                if validate_strong_management_secret_key(kernel_key).is_ok() {
                    return kernel_key.trim().to_string();
                }
                if is_hashed_management_secret_key(kernel_key) {
                    return LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string();
                }
            }
            return String::new();
        }
    }
    LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string()
}

pub(crate) fn normalize_management_secret_key(secret_key: String) -> Result<String, String> {
    let secret_key = secret_key.trim().to_string();
    validate_strong_management_secret_key(&secret_key)?;
    Ok(secret_key)
}

pub(crate) fn is_example_core_api_key(api_key: &str) -> bool {
    let value = api_key.trim();
    value == "your-api-key" || value.starts_with("your-api-key-")
}

pub(crate) fn is_hashed_management_secret_key(secret_key: &str) -> bool {
    let value = secret_key.trim();
    value.starts_with("$2a$")
        || value.starts_with("$2b$")
        || value.starts_with("$2y$")
        || value.starts_with("$argon2")
        || value.starts_with("$scrypt$")
        || value.starts_with("bcrypt:")
        || value.starts_with("argon2:")
        || value.starts_with("argon2id:")
        || value.starts_with("sha256:")
        || value.starts_with("sha512:")
}

pub(crate) fn validate_routing_strategy(strategy: &str) -> Result<(), String> {
    if matches!(strategy, "round-robin" | "weighted-round-robin" | "fill-first") {
        return Ok(());
    }
    Err("Routing strategy supports round-robin, weighted-round-robin, or fill-first".to_string())
}

pub(crate) fn normalize_optional_config_string(
    value: String,
    field_name: &str,
) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.chars().any(char::is_control) {
        return Err(format!("{field_name} cannot contain control characters"));
    }
    Ok(value)
}

pub(crate) fn normalize_session_affinity_ttl(value: String) -> Result<String, String> {
    let value = normalize_optional_config_string(value, "Session affinity TTL")?;
    if value.is_empty() {
        return Ok(value);
    }
    static PART: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"([0-9]+(?:\.[0-9]*)?|\.[0-9]+)(ns|us|µs|μs|ms|s|m|h)").unwrap()
    });
    let text = value.strip_prefix('+').unwrap_or(&value);
    let mut end = 0;
    let mut nanos = 0.0_f64;
    for part in PART.captures_iter(text) {
        let matched = part.get(0).unwrap();
        if matched.start() != end { break; }
        let multiplier = match &part[2] {
            "ns" => 1.0, "us" | "µs" | "μs" => 1_000.0, "ms" => 1_000_000.0,
            "s" => 1_000_000_000.0, "m" => 60_000_000_000.0, "h" => 3_600_000_000_000.0,
            _ => unreachable!(),
        };
        nanos += (part[1].parse::<f64>().unwrap_or(f64::INFINITY) * multiplier).trunc();
        end = matched.end();
    }
    if end != text.len() || nanos <= 0.0 || nanos >= i64::MAX as f64 {
        return Err("Session affinity TTL must be a positive duration such as 30m or 1h30m; leave it blank to use the core default".to_string());
    }
    Ok(if nanos < 1_000_000_000.0 { "1s".to_string() } else { value })
}

pub(crate) fn normalize_core_tls_settings(
    settings: CoreTlsSettings,
) -> Result<CoreTlsSettings, String> {
    let cert = normalize_optional_config_string(settings.cert, "TLS certificate path")?;
    let key = normalize_optional_config_string(settings.key, "TLS private key path")?;
    if settings.enabled && (cert.is_empty() || key.is_empty()) {
        return Err(
            "TLS is enabled, so both the certificate and private key paths are required"
                .to_string(),
        );
    }
    Ok(CoreTlsSettings {
        enabled: settings.enabled,
        cert,
        key,
    })
}

#[cfg(test)]
pub(crate) fn core_loopback_origin(port: u16, tls_enabled: bool) -> String {
    core_origin("127.0.0.1", port, tls_enabled)
}

pub(crate) fn core_connect_host(listen_host: &str) -> String {
    let host = listen_host
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_else(|| listen_host.trim());
    match host {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" => "::1".to_string(),
        _ => host.to_string(),
    }
}

pub(crate) fn core_origin(listen_host: &str, port: u16, tls_enabled: bool) -> String {
    let scheme = if tls_enabled { "https" } else { "http" };
    let host = core_connect_host(listen_host);
    let url_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    format!("{scheme}://{url_host}:{port}")
}

pub(crate) fn managed_core_tls_enabled() -> bool {
    #[cfg(test)]
    {
        false
    }
    #[cfg(not(test))]
    {
        current_core_tls_settings()
            .map(|settings| settings.enabled)
            .unwrap_or(false)
    }
}

pub(crate) fn managed_core_loopback_origin(port: u16) -> String {
    #[cfg(test)]
    {
        core_loopback_origin(port, false)
    }
    #[cfg(not(test))]
    {
        let host = read_installed_core_config_settings()
            .map(|settings| settings.host)
            .unwrap_or_else(|_| "127.0.0.1".to_string());
        core_origin(&host, port, managed_core_tls_enabled())
    }
}

pub(crate) fn config_update_error_with_rollback(
    error: String,
    rollback_error: Option<String>,
) -> String {
    match rollback_error {
        Some(rollback_error) => format!("{error}; failed to roll back kernel configuration: {rollback_error}"),
        None => error,
    }
}

pub(crate) fn merge_core_config_yaml(
    template: &str,
    current: Option<&str>,
    config: &GuiConfigFile,
) -> Result<String, String> {
    let base = match current {
        Some(current) => {
            let template_document: serde_norway::Value = serde_norway::from_str(template)
                .map_err(|error| format!("Failed to parse kernel configuration template: {error}"))?;
            if core_config_uses_v8(&template_document) {
                migrate_legacy_core_config_to_v8(template, current)?
            } else {
                current.to_string()
            }
        }
        None => merge_core_config_fields(template, None)?,
    };
    // Old mirrored values may be stale after a kernel upgrade. Only an explicit
    // software-side opt-out should disable statistics when starting the kernel.
    let mut startup_config = config.clone();
    startup_config.usage_statistics_enabled = config.usage_statistics_disabled != Some(true);
    let managed = apply_gui_managed_settings(&base, &startup_config)?;
    sanitize_unsupported_provider_disabled_yaml(&managed)
}

pub(crate) fn merge_core_config_fields(
    template: &str,
    current: Option<&str>,
) -> Result<String, String> {
    let current_value = current
        .map(|current| {
            let current = serde_norway::from_str::<serde_norway::Value>(current)
                .map_err(|err| format!("Failed to parse existing kernel configuration; writing stopped to prevent configuration loss: {err}"))?;
            if !current.is_mapping() {
                return Err("Existing kernel configuration root must be a YAML mapping; writing stopped".to_string());
            }
            Ok(current)
        })
        .transpose()?;
    merge_core_config_value(template, current_value)
}

pub(crate) fn merge_core_config_value(
    template: &str,
    current: Option<serde_norway::Value>,
) -> Result<String, String> {
    let template_value = serde_norway::from_str::<serde_norway::Value>(template)
        .map_err(|err| format!("Failed to parse kernel configuration template: {err}"))?;
    if !template_value.is_mapping() {
        return Err("Kernel configuration template root must be a YAML mapping".to_string());
    }
    let mut merged = template_value.clone();

    if let Some(current) = current {
        merge_yaml_values(&mut merged, current);
    }

    let rendered = render_yaml_value_changes(template, &template_value, &merged)?;
    let rendered_value = serde_norway::from_str::<serde_norway::Value>(&rendered)
        .map_err(|err| format!("Failed to validate migrated kernel configuration: {err}"))?;
    if !rendered_value.is_mapping() {
        return Err("Migrated kernel configuration root must be a YAML mapping".to_string());
    }
    Ok(rendered)
}

pub(crate) fn patch_core_network_yaml(
    content: &str,
    config: &GuiConfigFile,
) -> Result<Option<String>, String> {
    patch_core_yaml_document(content, |document| {
        let original = document.clone();
        apply_network_settings(document, config)?;
        Ok(*document != original)
    })
}

pub(crate) fn patch_core_network_routing_yaml(
    content: &str,
    config: &GuiConfigFile,
) -> Result<Option<String>, String> {
    patch_core_yaml_document(content, |document| {
        let original = document.clone();
        apply_network_settings(document, config)?;
        set_core_yaml_schema_value(
            document,
            &["proxy-url"],
            &["requests", "proxy-url"],
            serde_norway::Value::String(config.proxy_url.clone()),
        )?;
        set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity",
            serde_norway::Value::Bool(config.routing_session_affinity),
        )?;
        set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity-ttl",
            serde_norway::Value::String(config.routing_session_affinity_ttl.clone()),
        )?;
        set_core_yaml_schema_value(
            document,
            &["disable-cooling"],
            &["routing", "cooldown", "disable-cooling"],
            serde_norway::Value::Bool(config.disable_cooling),
        )?;
        set_core_yaml_schema_value(
            document,
            &["request-retry"],
            &["routing", "retry", "request-retry"],
            serde_norway::to_value(config.request_retry)
                .map_err(|err| format!("Failed to serialize request retry count: {err}"))?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["max-retry-credentials"],
            &["routing", "retry", "max-retry-credentials"],
            serde_norway::to_value(config.max_retry_credentials)
                .map_err(|err| format!("Failed to serialize maximum retry credentials: {err}"))?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["max-retry-interval"],
            &["routing", "retry", "max-retry-interval"],
            serde_norway::to_value(config.max_retry_interval)
                .map_err(|err| format!("Failed to serialize maximum retry interval: {err}"))?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["streaming", "bootstrap-retries"],
            &["requests", "streaming", "bootstrap-retries"],
            serde_norway::to_value(config.streaming_bootstrap_retries)
                .map_err(|err| format!("Failed to serialize streaming bootstrap retry count: {err}"))?,
        )?;
        Ok(*document != original)
    })
}

pub(crate) fn patch_core_network_endpoint_yaml(
    content: &str,
    config: &GuiConfigFile,
) -> Result<Option<String>, String> {
    patch_core_yaml_document(content, |document| {
        let original = document.clone();
        apply_network_settings(document, config)?;
        set_core_yaml_schema_value(
            document,
            &["proxy-url"],
            &["requests", "proxy-url"],
            serde_norway::Value::String(config.proxy_url.clone()),
        )?;
        Ok(*document != original)
    })
}

pub(crate) fn patch_core_retry_yaml(
    content: &str,
    config: &GuiConfigFile,
) -> Result<Option<String>, String> {
    patch_core_yaml_document(content, |document| {
        let original = document.clone();
        set_core_yaml_schema_value(
            document,
            &["disable-cooling"],
            &["routing", "cooldown", "disable-cooling"],
            serde_norway::Value::Bool(config.disable_cooling),
        )?;
        set_core_yaml_schema_value(
            document,
            &["request-retry"],
            &["routing", "retry", "request-retry"],
            serde_norway::to_value(config.request_retry).map_err(|err| err.to_string())?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["max-retry-credentials"],
            &["routing", "retry", "max-retry-credentials"],
            serde_norway::to_value(config.max_retry_credentials).map_err(|err| err.to_string())?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["max-retry-interval"],
            &["routing", "retry", "max-retry-interval"],
            serde_norway::to_value(config.max_retry_interval).map_err(|err| err.to_string())?,
        )?;
        set_core_yaml_schema_value(
            document,
            &["streaming", "bootstrap-retries"],
            &["requests", "streaming", "bootstrap-retries"],
            serde_norway::to_value(config.streaming_bootstrap_retries)
                .map_err(|err| err.to_string())?,
        )?;
        Ok(*document != original)
    })
}

pub(crate) fn patch_core_session_routing_yaml(
    content: &str,
    config: &GuiConfigFile,
) -> Result<Option<String>, String> {
    patch_core_yaml_document(content, |document| {
        let original = document.clone();
        set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity",
            serde_norway::Value::Bool(config.routing_session_affinity),
        )?;
        set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity-ttl",
            serde_norway::Value::String(config.routing_session_affinity_ttl.clone()),
        )?;
        Ok(*document != original)
    })
}

pub(crate) fn merge_yaml_values(base: &mut serde_norway::Value, current: serde_norway::Value) {
    match (base, current) {
        (
            serde_norway::Value::Mapping(base_mapping),
            serde_norway::Value::Mapping(current_mapping),
        ) => {
            for (key, current_value) in current_mapping {
                if let Some(base_value) = base_mapping.get_mut(&key) {
                    merge_yaml_values(base_value, current_value);
                } else {
                    base_mapping.insert(key, current_value);
                }
            }
        }
        (base, current) => *base = current,
    }
}

pub(crate) fn apply_network_settings(
    document: &mut serde_norway::Value,
    config: &GuiConfigFile,
) -> Result<(), String> {
    let host = config.host.trim();
    set_core_yaml_schema_value(
        document,
        &["host"],
        &["server", "host"],
        serde_norway::Value::String(host.to_string()),
    )?;
    set_core_yaml_schema_value(
        document,
        &["port"],
        &["server", "port"],
        serde_norway::to_value(config.port).map_err(|err| format!("Failed to serialize kernel port: {err}"))?,
    )?;
    Ok(())
}

pub(crate) fn apply_gui_managed_settings(
    content: &str,
    config: &GuiConfigFile,
) -> Result<String, String> {
    let host = config.host.trim();
    let updated = patch_core_yaml_document(content, |document| {
        let mut changed = false;
        changed |= set_core_yaml_schema_value(
            document,
            &["host"],
            &["server", "host"],
            serde_norway::Value::String(host.to_string()),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["port"],
            &["server", "port"],
            serde_norway::to_value(config.port)
                .map_err(|err| format!("Failed to serialize kernel port: {err}"))?,
        )?;
        changed |= set_core_yaml_auth_dir(document, &config.auth_dir)?;
        changed |= set_core_yaml_schema_value(
            document,
            &["debug"],
            &["observability", "logs", "debug"],
            serde_norway::Value::Bool(config.debug),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["commercial-mode"],
            &["server", "commercial-mode"],
            serde_norway::Value::Bool(config.commercial_mode),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["logging-to-file"],
            &["observability", "logs", "logging-to-file"],
            serde_norway::Value::Bool(config.logging_to_file),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["logs-max-total-size-mb"],
            &["observability", "logs", "logs-max-total-size-mb"],
            serde_norway::to_value(config.logs_max_total_size_mb)
                .map_err(|err| format!("Failed to serialize log size limit: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["error-logs-max-files"],
            &["observability", "logs", "error-logs-max-files"],
            serde_norway::to_value(config.error_logs_max_files)
                .map_err(|err| format!("Failed to serialize error log retention count: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["usage-statistics-enabled"],
            &["observability", "usage", "usage-statistics-enabled"],
            serde_norway::Value::Bool(config.usage_statistics_enabled),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["redis-usage-queue-retention-seconds"],
            &["observability", "usage", "redis-usage-queue-retention-seconds"],
            serde_norway::to_value(config.redis_usage_queue_retention_seconds)
                .map_err(|err| format!("Failed to serialize Redis usage queue retention: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["request-log"],
            &["observability", "logs", "request-log"],
            serde_norway::Value::Bool(config.request_log),
        )?;
        // v8 kernels persist this credential as a bcrypt/argon2 hash after a
        // successful start. It is already the canonical value the kernel
        // expects; replacing it with the GUI plaintext needlessly enters the
        // format-preserving YAML editor and can produce a duplicate top-level
        // `management` mapping for some valid config layouts.
        let stored_management_key = document
            .as_mapping()
            .and_then(|root| nested_yaml_value(root, &["management", "secret-key"]))
            .or_else(|| {
                document.as_mapping().and_then(|root| {
                    nested_yaml_value(root, &["remote-management", "secret-key"])
                })
            });
        if !stored_management_key
            .and_then(serde_norway::Value::as_str)
            .is_some_and(is_hashed_management_secret_key)
        {
            changed |= set_core_yaml_schema_value(
                document,
                &["remote-management", "secret-key"],
                &["management", "secret-key"],
                serde_norway::Value::String(config.management_secret_key.clone()),
            )?;
        }
        changed |= set_core_yaml_nested_value(
            document,
            "plugins",
            "enabled",
            serde_norway::Value::Bool(config.plugins_enabled),
        )?;
        changed |= set_core_yaml_nested_value(
            document,
            "routing",
            "strategy",
            serde_norway::Value::String(config.routing_strategy.clone()),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["proxy-url"],
            &["requests", "proxy-url"],
            serde_norway::Value::String(config.proxy_url.clone()),
        )?;
        changed |= set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity",
            serde_norway::Value::Bool(config.routing_session_affinity),
        )?;
        changed |= set_core_yaml_nested_value(
            document,
            "routing",
            "session-affinity-ttl",
            serde_norway::Value::String(config.routing_session_affinity_ttl.clone()),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["disable-cooling"],
            &["routing", "cooldown", "disable-cooling"],
            serde_norway::Value::Bool(config.disable_cooling),
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["request-retry"],
            &["routing", "retry", "request-retry"],
            serde_norway::to_value(config.request_retry)
                .map_err(|err| format!("Failed to serialize request retry count: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["max-retry-credentials"],
            &["routing", "retry", "max-retry-credentials"],
            serde_norway::to_value(config.max_retry_credentials)
                .map_err(|err| format!("Failed to serialize maximum retry credentials: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["max-retry-interval"],
            &["routing", "retry", "max-retry-interval"],
            serde_norway::to_value(config.max_retry_interval)
                .map_err(|err| format!("Failed to serialize maximum retry interval: {err}"))?,
        )?;
        changed |= set_core_yaml_schema_value(
            document,
            &["streaming", "bootstrap-retries"],
            &["requests", "streaming", "bootstrap-retries"],
            serde_norway::to_value(config.streaming_bootstrap_retries)
                .map_err(|err| format!("Failed to serialize streaming bootstrap retry count: {err}"))?,
        )?;
        Ok(changed)
    })?
    .unwrap_or_else(|| content.to_string());

    let updated = patch_core_api_keys_yaml(&updated, &gui_api_key_values(&config.api_keys))?;
    serde_norway::from_str::<serde_norway::Value>(&updated)
        .map_err(|err| format!("Failed to validate kernel startup configuration: {err}"))?;
    Ok(updated)
}

pub(crate) fn write_bytes_directly(path: &Path, content: &[u8]) -> Result<(), String> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create configuration directory {}: {error}", path_to_string(directory)))?;

    let write_result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(content)?;
        file.set_len(content.len() as u64)?;
        file.sync_all()
    })();

    write_result.map_err(|error| format!("Failed to write configuration directly {}: {error}", path_to_string(path)))?;
    remember_software_write(path, content);
    Ok(())
}

pub(crate) fn write_bytes_atomically(path: &Path, content: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)
        .map_err(|error| format!("Failed to create configuration directory {}: {error}", path_to_string(directory)))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.yaml");
    let temporary_path = directory.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let write_result = (|| -> io::Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        replace_file_atomically(&temporary_path, path)
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "Failed to write configuration atomically {}: {error}",
            path_to_string(path)
        ));
    }

    remember_software_write(path, content);

    Ok(())
}

pub(crate) fn normalized_config_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn remember_software_write(path: &Path, content: &[u8]) {
    if let Ok(mut hashes) = CONFIG_WRITE_HASHES.lock() {
        hashes.insert(normalized_config_path(path), sha256_bytes(content));
    }
}

pub(crate) fn consume_software_write(path: &Path) -> bool {
    let Ok(content) = fs::read(path) else {
        return false;
    };
    let key = normalized_config_path(path);
    let hash = sha256_bytes(&content);
    let Ok(mut hashes) = CONFIG_WRITE_HASHES.lock() else {
        return false;
    };
    hashes
        .remove(&key)
        .is_some_and(|expected_hash| expected_hash == hash)
}

pub(crate) fn write_yaml_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }

    write_bytes_atomically(path, content.as_bytes())?;

    Ok(true)
}

pub(crate) fn write_core_config_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }
    write_bytes_directly(path, content.as_bytes())?;
    Ok(true)
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(
    temporary_path: &Path,
    destination_path: &Path,
) -> io::Result<()> {
    fs::rename(temporary_path, destination_path)
}

#[cfg(windows)]
pub(crate) fn retry_windows_file_replace(
    mut replace: impl FnMut() -> io::Result<()>,
    mut wait: impl FnMut(Duration),
) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
        ERROR_UNABLE_TO_REMOVE_REPLACED,
    };

    let mut delays = [50, 100, 200, 400, 800].into_iter();
    loop {
        match replace() {
            Ok(()) => return Ok(()),
            Err(error) => {
                let retryable = matches!(
                    error.raw_os_error().map(|code| code as u32),
                    Some(
                        ERROR_ACCESS_DENIED
                            | ERROR_LOCK_VIOLATION
                            | ERROR_SHARING_VIOLATION
                            | ERROR_UNABLE_TO_REMOVE_REPLACED
                    )
                );
                if retryable {
                    if let Some(delay) = delays.next() {
                        wait(Duration::from_millis(delay));
                        continue;
                    }
                }
                return Err(error);
            }
        }
    }
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(
    temporary_path: &Path,
    destination_path: &Path,
) -> io::Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let destination = destination_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let replacement = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    retry_windows_file_replace(
        || {
            if !destination_path.exists() {
                return fs::rename(temporary_path, destination_path);
            }
            let replaced = unsafe {
                ReplaceFileW(
                    destination.as_ptr(),
                    replacement.as_ptr(),
                    ptr::null(),
                    0,
                    ptr::null(),
                    ptr::null(),
                )
            };
            if replaced == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        },
        thread::sleep,
    )
}

pub(crate) fn fixed_oauth_dir() -> Result<PathBuf, String> {
    Ok(core_base_dir()?.join(OAUTH_DIR_NAME))
}

pub(crate) fn auth_dir_path_for_core(auth_dir: &str, install_dir: &Path) -> Result<PathBuf, String> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let user_home = env::var_os(variable).filter(|value| !value.is_empty()).map(PathBuf::from);
    auth_dir_path_for_core_with_home(auth_dir, install_dir, user_home.as_deref())
}

pub(crate) fn auth_dir_path_for_core_with_home(
    auth_dir: &str,
    install_dir: &Path,
    user_home: Option<&Path>,
) -> Result<PathBuf, String> {
    let auth_dir = if auth_dir.is_empty() { "~/.cli-proxy-api" } else { auth_dir };
    let path = if let Some(remainder) = auth_dir.strip_prefix('~') {
        let home = user_home.ok_or_else(|| "Unable to determine user home for credentials directory".to_string())?;
        home.join(remainder.trim_start_matches(['/', '\\']).replace('\\', "/"))
    } else {
        PathBuf::from(auth_dir)
    };
    Ok(resolve_core_relative_path(&path, install_dir))
}

fn resolve_core_relative_path(path: &Path, install_dir: &Path) -> PathBuf {
    normalize_path_lexically(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        install_dir.join(path)
    })
}

pub(crate) fn core_logs_dir_path(auth_dir: &str, install_dir: &Path) -> Result<PathBuf, String> {
    let writable_base = ["WRITABLE_PATH", "writable_path"].into_iter().find_map(|key| {
        env::var(key).ok().filter(|value| !value.trim().is_empty())
    });
    core_logs_dir_path_with_base(auth_dir, install_dir, writable_base.as_deref())
}

pub(crate) fn core_logs_dir_path_with_base(
    auth_dir: &str,
    install_dir: &Path,
    writable_base: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(base) = writable_base.map(str::trim).filter(|base| !base.is_empty()) {
        return Ok(resolve_core_relative_path(&Path::new(base).join("logs"), install_dir));
    }
    let local_logs = install_dir.join("logs");
    if directory_is_writable(&local_logs) {
        return Ok(normalize_path_lexically(&local_logs));
    }
    Ok(auth_dir_path_for_core(auth_dir, install_dir)?.join("logs"))
}

fn directory_is_writable(path: &Path) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    if !path.is_dir() {
        return false;
    }
    let probe = path.join(format!(
        ".cpa-gui-log-probe-{}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let Ok(file) = File::options().write(true).create_new(true).open(&probe) else {
        return false;
    };
    drop(file);
    let _ = fs::remove_file(probe);
    true
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn auth_dir_is_inside_macos_app_bundle(path: &Path) -> bool {
    let normalized = normalize_path_lexically(path);
    let mut previous_component_was_app = false;
    for component in normalized.components() {
        let Component::Normal(name) = component else {
            previous_component_was_app = false;
            continue;
        };
        if previous_component_was_app && name == std::ffi::OsStr::new("Contents") {
            return true;
        }
        previous_component_was_app = Path::new(name)
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("app"));
    }
    false
}

#[cfg(any(target_os = "macos", test))]
fn copy_auth_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "OAuth migration target is not a regular file: {}",
                    path_to_string(destination)
                ));
            }
            let source_bytes = fs::read(source).map_err(|error| {
                format!("Failed to read old OAuth file {}: {error}", path_to_string(source))
            })?;
            let destination_bytes = fs::read(destination).map_err(|error| {
                format!(
                    "Failed to read existing OAuth file {}: {error}",
                    path_to_string(destination)
                )
            })?;
            if source_bytes == destination_bytes {
                return Ok(());
            }
            return Err(format!(
                "OAuth migration target already contains different content and was not overwritten: {}",
                path_to_string(destination)
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to check OAuth migration target {}: {error}",
                path_to_string(destination)
            ));
        }
    }

    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let directory = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Failed to create OAuth migration directory {}: {error}",
            path_to_string(directory)
        )
    })?;
    let file_name = destination
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "oauth".into());
    let temporary_path = directory.join(format!(
        ".{file_name}.migration.{}.{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let copy_result = (|| -> io::Result<()> {
        fs::copy(source, &temporary_path)?;
        let temporary_file = fs::OpenOptions::new().write(true).open(&temporary_path)?;
        temporary_file.sync_all()?;
        drop(temporary_file);
        fs::rename(&temporary_path, destination)
    })();
    if let Err(error) = copy_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "Failed to copy OAuth file {} -> {}: {error}",
            path_to_string(source),
            path_to_string(destination)
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn copy_auth_directory(source: &Path, destination: &Path) -> Result<(), String> {
    let source_metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return fs::create_dir_all(destination).map_err(|create_error| {
                format!(
                    "Failed to create OAuth directory {}: {create_error}",
                    path_to_string(destination)
                )
            });
        }
        Err(error) => {
            return Err(format!(
                "Failed to check old OAuth directory {}: {error}",
                path_to_string(source)
            ));
        }
    };
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        return Err(format!(
            "Old OAuth path is not a regular directory: {}",
            path_to_string(source)
        ));
    }

    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "OAuth migration target is not a regular directory: {}",
                path_to_string(destination)
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(destination).map_err(|create_error| {
                format!(
                    "Failed to create OAuth migration directory {}: {create_error}",
                    path_to_string(destination)
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Failed to check OAuth migration directory {}: {error}",
                path_to_string(destination)
            ));
        }
    }

    for entry in fs::read_dir(source)
        .map_err(|error| format!("Failed to read old OAuth directory {}: {error}", path_to_string(source)))?
    {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read old OAuth directory entry {}: {error}",
                path_to_string(source)
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "Failed to check old OAuth directory entry {}: {error}",
                path_to_string(&entry.path())
            )
        })?;
        if file_type.is_symlink() {
            return Err(format!(
                "OAuth migration does not accept symbolic links: {}",
                path_to_string(&entry.path())
            ));
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_auth_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            copy_auth_file_atomically(&entry.path(), &target)?;
        } else {
            return Err(format!(
                "OAuth migration does not support this file type: {}",
                path_to_string(&entry.path())
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn migrate_auth_dir_from_macos_app_bundle(
    config: &mut GuiConfigFile,
    install_dir: &Path,
    persistent_auth_dir: &Path,
) -> Result<bool, String> {
    let source = auth_dir_path_for_core(&config.auth_dir, install_dir)?;
    if !auth_dir_is_inside_macos_app_bundle(&source) {
        return Ok(false);
    }
    let destination = normalize_path_lexically(persistent_auth_dir);
    if auth_dir_is_inside_macos_app_bundle(&destination) {
        return Err(format!(
            "OAuth persistence directory cannot be inside a macOS application bundle: {}",
            path_to_string(&destination)
        ));
    }

    let source_is_empty = match fs::symlink_metadata(&source) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => fs::read_dir(&source)
            .and_then(|mut entries| entries.next().transpose())
            .map_err(|error| format!("Failed to inspect OAuth directory {}: {error}", path_to_string(&source)))?
            .is_none(),
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(error) => {
            return Err(format!(
                "Failed to check old OAuth directory {}: {error}",
                path_to_string(&source)
            ));
        }
    };
    let migration_source = if source_is_empty {
        source
            .ancestors()
            .find_map(|directory| {
                if directory.file_name()? != std::ffi::OsStr::new("Contents") {
                    return None;
                }
                let app = directory.parent()?;
                if !app.extension()?.to_string_lossy().eq_ignore_ascii_case("app") {
                    return None;
                }
                Some(
                    app.parent()?
                        .join(".EasyCLIProxyAPI.app.update-backup")
                        .join(source.strip_prefix(app).ok()?),
                )
            })
            .unwrap_or_else(|| source.clone())
    } else {
        source.clone()
    };
    copy_auth_directory(&migration_source, &destination)?;
    config.auth_dir = DEFAULT_AUTH_DIR.to_string();
    Ok(true)
}

fn auth_directory_has_json_files(path: &Path) -> Result<bool, String> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Failed to inspect OAuth directory {}: {error}",
                path_to_string(path)
            ));
        }
    };
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Failed to inspect OAuth directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect OAuth file type: {error}"))?;
        if (file_type.is_file() || file_type.is_symlink())
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn recover_relative_oauth_dir(
    config: &mut GuiConfigFile,
    install_dir: &Path,
    persistent_auth_dir: &Path,
) -> Result<bool, String> {
    let current_auth_dir = auth_dir_path_for_core(&config.auth_dir, install_dir)?;
    if current_auth_dir != normalize_path_lexically(&install_dir.join(OAUTH_DIR_NAME)) {
        return Ok(false);
    }
    if auth_directory_has_json_files(&current_auth_dir)?
        || !auth_directory_has_json_files(persistent_auth_dir)?
    {
        return Ok(false);
    }
    config.auth_dir = DEFAULT_AUTH_DIR.to_string();
    Ok(true)
}

pub(crate) fn migrate_auth_dir_at(
    config: &mut GuiConfigFile,
    install_dir: &Path,
    persistent_auth_dir: &Path,
) -> Result<bool, String> {
    let mut candidate = config.clone();
    #[cfg(target_os = "macos")]
    let migrated =
        migrate_auth_dir_from_macos_app_bundle(&mut candidate, install_dir, persistent_auth_dir)?;
    #[cfg(not(target_os = "macos"))]
    let migrated = false;
    let recovered = recover_relative_oauth_dir(&mut candidate, install_dir, persistent_auth_dir)?;
    if migrated || recovered {
        patch_core_auth_dir_at(&install_dir.join(CORE_CONFIG_FILE), &candidate.auth_dir)
            .map_err(|error| format!("Failed to update kernel OAuth directory: {error}"))?;
        *config = candidate;
    }
    Ok(migrated || recovered)
}

pub(crate) fn should_import_core_api_keys(
    had_existing_gui_config: bool,
    core_api_keys: &[String],
) -> bool {
    had_existing_gui_config || !core_api_keys.is_empty()
}

pub(crate) fn file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

pub(crate) fn load_or_create_gui_config() -> Result<GuiConfigFile, String> {
    let config_path = gui_config_path()?;
    let legacy_config_path = legacy_gui_config_path()?;
    let gui_config_exists = config_path.is_file();
    let legacy_config_exists = legacy_config_path.is_file();
    let had_existing_gui_config = gui_config_exists || legacy_config_exists;

    let (mut config, presence, mut changed, gui_parse_failed) = if gui_config_exists {
        let content = fs::read_to_string(&config_path)
            .map_err(|err| format!("Failed to read GUI configuration {}: {err}", path_to_string(&config_path)))?;
        match (
            toml::from_str::<GuiConfigFile>(&content),
            toml::from_str::<GuiConfigPresence>(&content),
        ) {
            (Ok(config), Ok(presence)) => (config, presence, false, false),
            _ => (
                GuiConfigFile::default(),
                GuiConfigPresence::default(),
                true,
                true,
            ),
        }
    } else if legacy_config_exists {
        let content = fs::read_to_string(&legacy_config_path).map_err(|err| {
            format!(
                "Failed to read old GUI configuration {}: {err}",
                path_to_string(&legacy_config_path)
            )
        })?;
        let config = serde_yaml::from_str::<GuiConfigFile>(&content)
            .map_err(|err| format!("Failed to parse old GUI configuration: {err}"))?;
        let presence = serde_yaml::from_str::<GuiConfigPresence>(&content)
            .map_err(|err| format!("Failed to parse old GUI configuration field: {err}"))?;
        (config, presence, true, false)
    } else {
        (
            GuiConfigFile::default(),
            GuiConfigPresence::default(),
            true,
            false,
        )
    };

    let saved_management_secret = presence.management_secret_key.as_deref();

    if gui_config_exists {
        if let Ok(content) = fs::read_to_string(&config_path) {
            changed |= [
                "codex-session-repair-on-launch",
                "claude-code-working-directory",
                "claude-code-working-directory-prompt-disabled",
            ]
            .iter()
            .any(|key| {
                content
                    .lines()
                    .any(|line| line.trim_start().starts_with(key))
            });
        }
    }

    let core_config_path = core_install_dir()?.join(CORE_CONFIG_FILE);
    let core_is_newer = file_modified_time(&core_config_path)
        .zip(file_modified_time(&config_path))
        .is_some_and(|(core, gui)| core > gui);
    if core_is_newer && !gui_parse_failed {
        if let Ok(core_settings) = read_installed_core_config_settings() {
            apply_core_settings_to_gui_config(&mut config, &core_settings);
            apply_external_core_proxy_override(&mut config, &core_settings)?;
            changed = true;
        }
    }

    let missing_core_settings = presence.host.is_none()
        || presence.port.is_none()
        || presence.auth_dir.is_none()
        || presence.api_keys.is_none()
        || presence.management_secret_key.is_none()
        || presence.debug.is_none()
        || presence.commercial_mode.is_none()
        || presence.logging_to_file.is_none()
        || presence.logs_max_total_size_mb.is_none()
        || presence.error_logs_max_files.is_none()
        || presence.usage_statistics_enabled.is_none()
        || presence.redis_usage_queue_retention_seconds.is_none()
        || presence.request_log.is_none()
        || presence.plugins_enabled.is_none()
        || presence.routing_strategy.is_none()
        || presence.proxy_url.is_none()
        || presence.routing_session_affinity.is_none()
        || presence.routing_session_affinity_ttl.is_none()
        || presence.disable_cooling.is_none()
        || presence.request_retry.is_none()
        || presence.max_retry_credentials.is_none()
        || presence.max_retry_interval.is_none()
        || presence.streaming_bootstrap_retries.is_none();
    if missing_core_settings && !gui_parse_failed {
        if let Ok(core_settings) = read_installed_core_config_settings() {
            if presence.host.is_none() {
                config.host = core_settings.host.clone();
                config.allow_lan = !is_loopback_host(&config.host);
            }
            if presence.port.is_none() {
                config.port = core_settings.port;
            }
            if presence.auth_dir.is_none() {
                config.auth_dir = core_settings.auth_dir.clone();
            }
            if presence.api_keys.is_none()
                && should_import_core_api_keys(had_existing_gui_config, &core_settings.api_keys)
            {
                config.api_keys = merge_core_api_keys_with_gui_metadata(
                    &config.api_keys,
                    &core_settings.api_keys,
                    None,
                );
            }
            if presence.plugins_enabled.is_none() {
                config.plugins_enabled = core_settings.plugins_enabled;
            }
            if presence.usage_statistics_enabled.is_none() {
                config.usage_statistics_enabled = core_settings.usage_statistics_enabled;
            }
            if presence.debug.is_none() {
                config.debug = core_settings.debug;
            }
            if presence.commercial_mode.is_none() {
                config.commercial_mode = core_settings.commercial_mode;
            }
            if presence.logging_to_file.is_none() {
                config.logging_to_file = core_settings.logging_to_file;
            }
            if presence.logs_max_total_size_mb.is_none() {
                config.logs_max_total_size_mb = core_settings.logs_max_total_size_mb;
            }
            if presence.error_logs_max_files.is_none() {
                config.error_logs_max_files = core_settings.error_logs_max_files;
            }
            if presence.redis_usage_queue_retention_seconds.is_none() {
                config.redis_usage_queue_retention_seconds =
                    core_settings.redis_usage_queue_retention_seconds;
            }
            if presence.request_log.is_none() {
                config.request_log = core_settings.request_log;
            }
            if presence.management_secret_key.is_none() {
                config.management_secret_key = core_settings
                    .management_secret_key
                    .as_deref()
                    .unwrap_or(&config.management_secret_key)
                    .to_string();
            }
            if presence.routing_strategy.is_none() {
                config.routing_strategy = core_settings.routing_strategy;
            }
            if presence.proxy_url.is_none() {
                config.proxy_url = core_settings.proxy_url;
                config.proxy_override = !config.proxy_url.trim().is_empty();
            }
            if presence.routing_session_affinity.is_none() {
                config.routing_session_affinity = core_settings.routing_session_affinity;
            }
            if presence.routing_session_affinity_ttl.is_none() {
                config.routing_session_affinity_ttl = core_settings.routing_session_affinity_ttl;
            }
            if presence.disable_cooling.is_none() {
                config.disable_cooling = core_settings.disable_cooling;
            }
            if presence.request_retry.is_none() {
                config.request_retry = core_settings.request_retry;
            }
            if presence.max_retry_credentials.is_none() {
                config.max_retry_credentials = core_settings.max_retry_credentials;
            }
            if presence.max_retry_interval.is_none() {
                config.max_retry_interval = core_settings.max_retry_interval;
            }
            if presence.streaming_bootstrap_retries.is_none() {
                config.streaming_bootstrap_retries = core_settings.streaming_bootstrap_retries;
            }
        }
        changed = true;
    }
    if presence.management_secret_key.is_none()
        && (had_existing_gui_config || core_config_path.is_file())
    {
        config.management_secret_key = read_installed_core_config_settings()
            .ok()
            .and_then(|settings| settings.management_secret_key)
            .unwrap_or_else(|| config.management_secret_key.clone());
        changed = true;
    }
    if presence.host.is_none() || presence.auth_dir.is_none() {
        changed = true;
    }
    if presence.locale.is_none() {
        changed = true;
    }
    if presence.close_behavior.is_none() {
        changed = true;
    }
    if presence.start_core_on_launch.is_none() {
        changed = true;
    }
    if presence.silent_start.is_none() {
        changed = true;
    }
    if presence.default_terminal.is_none() {
        changed = true;
    }
    if presence.download_source.is_none() {
        config.download_source = if config.prefer_gitcode_downloads {
            VersionDownloadSource::Gitcode
        } else {
            VersionDownloadSource::Github
        };
        changed = true;
    }
    if presence.custom_download_mirrors.is_none()
        || presence.active_custom_download_mirror.is_none()
    {
        changed = true;
    }
    if presence.prefer_gitcode_downloads.is_none() {
        changed = true;
    }
    changed |= network_proxy::initialize_override(
        &mut config,
        presence.proxy_override.is_some(),
    );
    config.prefer_gitcode_downloads = config.download_source == VersionDownloadSource::Gitcode;
    let installed_management_secret = if saved_management_secret == Some("") {
        read_installed_core_config_settings()
            .ok()
            .and_then(|settings| settings.management_secret_key)
    } else {
        None
    };
    let recovered_secret = recover_management_secret(
        &config.management_secret_key,
        saved_management_secret,
        installed_management_secret.as_deref(),
    );
    let management_secret_rotated = recovered_secret != config.management_secret_key;
    config.management_secret_key = recovered_secret;
    changed |= management_secret_rotated;
    changed |= sanitize_gui_config(&mut config)?;
    validate_gui_config(&config)?;
    if changed {
        write_gui_config(&config)?;
    }
    if management_secret_rotated {
        if let Err(error) = patch_core_management_secret_key(&config.management_secret_key) {
            eprintln!("Failed to update legacy CPA WebUI key; will retry the next time the kernel starts: {error}");
        }
    }
    if !config.auth_dir.trim().is_empty() {
        let install_dir = core_install_dir()?;
        let auth_dir = auth_dir_path_for_core(&config.auth_dir, &install_dir)?;
        fs::create_dir_all(&auth_dir)
            .map_err(|error| format!("Failed to create credentials directory {}: {error}", path_to_string(&auth_dir)))?;
    }
    Ok(config)
}

pub(crate) fn apply_core_settings_to_gui_config(
    config: &mut GuiConfigFile,
    core_settings: &CoreConfigSettings,
) {
    config.host = core_settings.host.clone();
    config.port = core_settings.port;
    config.allow_lan = !is_loopback_host(&core_settings.host);
    config.auth_dir = core_settings.auth_dir.clone();
    config.api_keys =
        merge_core_api_keys_with_gui_metadata(&config.api_keys, &core_settings.api_keys, None);
    if let Some(secret_key) = core_settings
        .management_secret_key
        .as_deref()
        .filter(|secret_key| !is_hashed_management_secret_key(secret_key))
    {
        config.management_secret_key = secret_key.to_string();
    }
    config.debug = core_settings.debug;
    config.commercial_mode = core_settings.commercial_mode;
    config.logging_to_file = core_settings.logging_to_file;
    config.logs_max_total_size_mb = core_settings.logs_max_total_size_mb;
    config.error_logs_max_files = core_settings.error_logs_max_files;
    config.usage_statistics_enabled = core_settings.usage_statistics_enabled;
    config.redis_usage_queue_retention_seconds = core_settings.redis_usage_queue_retention_seconds;
    config.request_log = core_settings.request_log;
    config.plugins_enabled = core_settings.plugins_enabled;
    config.routing_strategy = core_settings.routing_strategy.clone();
    config.routing_session_affinity = core_settings.routing_session_affinity;
    config.routing_session_affinity_ttl = core_settings.routing_session_affinity_ttl.clone();
    config.disable_cooling = core_settings.disable_cooling;
    config.request_retry = core_settings.request_retry;
    config.max_retry_credentials = core_settings.max_retry_credentials;
    config.max_retry_interval = core_settings.max_retry_interval;
    config.streaming_bootstrap_retries = core_settings.streaming_bootstrap_retries;
}

pub(crate) fn apply_external_core_proxy_override(
    config: &mut GuiConfigFile,
    core_settings: &CoreConfigSettings,
) -> Result<(), String> {
    if core_settings.proxy_url == config.proxy_url {
        return Ok(());
    }
    config.proxy_url = network_proxy::normalize_optional_proxy_url(&core_settings.proxy_url)?;
    // Changes made directly in the core YAML are user choices. Persist them as
    // manual overrides so the system-proxy monitor does not revert them.
    config.proxy_override = true;
    Ok(())
}

pub(crate) fn default_api_key_entry() -> GuiApiKeyEntry {
    GuiApiKeyEntry {
        key: DEFAULT_API_KEY.to_string(),
        remark: DEFAULT_API_KEY_INITIAL_REMARK.to_string(),
    }
}

pub(crate) fn gui_api_key_values(entries: &[GuiApiKeyEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.key.clone()).collect()
}

pub(crate) fn effective_agent_api_key(config: &GuiConfigFile) -> &str {
    config
        .api_keys
        .iter()
        .map(|entry| entry.key.trim())
        .find(|key| !key.is_empty())
        .unwrap_or(DEFAULT_API_KEY)
}

pub(crate) fn merge_core_api_keys_with_gui_metadata(
    existing: &[GuiApiKeyEntry],
    core_api_keys: &[String],
    added_api_key: Option<&GuiApiKeyEntry>,
) -> Vec<GuiApiKeyEntry> {
    let mut merged: Vec<GuiApiKeyEntry> = Vec::new();

    for api_key in core_api_keys {
        let api_key = api_key.trim();
        if api_key.is_empty() || is_example_core_api_key(api_key) {
            continue;
        }
        if merged.iter().any(|entry| entry.key == api_key) {
            continue;
        }

        let remark = added_api_key
            .filter(|entry| entry.key == api_key)
            .map(|entry| entry.remark.clone())
            .or_else(|| {
                existing
                    .iter()
                    .find(|entry| entry.key == api_key)
                    .map(|entry| entry.remark.clone())
            })
            .unwrap_or_default();
        merged.push(GuiApiKeyEntry {
            key: api_key.to_string(),
            remark,
        });
    }

    merged
}

pub(crate) fn normalized_saved_window_size(width: u32, height: u32) -> SavedWindowSize {
    SavedWindowSize {
        width: width.clamp(MIN_MAIN_WINDOW_WIDTH, MAX_SAVED_WINDOW_DIMENSION),
        height: height.clamp(MIN_MAIN_WINDOW_HEIGHT, MAX_SAVED_WINDOW_DIMENSION),
    }
}

pub(crate) fn configured_window_size(config: &GuiConfigFile) -> Option<SavedWindowSize> {
    match (config.window_width, config.window_height) {
        (Some(width), Some(height)) => Some(normalized_saved_window_size(width, height)),
        _ => None,
    }
}

pub(crate) fn logical_window_size_from_physical(
    physical_size: &tauri::PhysicalSize<u32>,
    scale_factor: f64,
) -> Option<SavedWindowSize> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }

    let width = (f64::from(physical_size.width) / scale_factor).round() as u32;
    let height = (f64::from(physical_size.height) / scale_factor).round() as u32;
    if width < MIN_MAIN_WINDOW_WIDTH || height < MIN_MAIN_WINDOW_HEIGHT {
        return None;
    }

    Some(normalized_saved_window_size(width, height))
}

pub(crate) fn fit_window_size_to_current_monitor<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    saved_size: SavedWindowSize,
) -> SavedWindowSize {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return saved_size;
    };

    let scale_factor = monitor.scale_factor();
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return saved_size;
    }

    let (frame_width, frame_height) = window
        .outer_size()
        .ok()
        .zip(window.inner_size().ok())
        .map(|(outer, inner)| {
            (
                outer.width.saturating_sub(inner.width),
                outer.height.saturating_sub(inner.height),
            )
        })
        .unwrap_or_default();
    let work_area = monitor.work_area();
    let max_width =
        (f64::from(work_area.size.width.saturating_sub(frame_width)) / scale_factor).floor() as u32;
    let max_height = (f64::from(work_area.size.height.saturating_sub(frame_height)) / scale_factor)
        .floor() as u32;

    SavedWindowSize {
        width: saved_size.width.min(max_width.max(MIN_MAIN_WINDOW_WIDTH)),
        height: saved_size
            .height
            .min(max_height.max(MIN_MAIN_WINDOW_HEIGHT)),
    }
}

pub(crate) fn restore_main_window_size(app: &tauri::AppHandle) -> Result<(), String> {
    let window_size_state = app.state::<MainWindowSizeState>();
    let Some(saved_size) = window_size_state.snapshot()? else {
        return Ok(());
    };
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window does not exist; cannot restore window size".to_string())?;
    let restored_size = fit_window_size_to_current_monitor(&window, saved_size);

    window
        .set_size(LogicalSize::new(
            f64::from(restored_size.width),
            f64::from(restored_size.height),
        ))
        .map_err(|error| format!("Failed to restore main window size: {error}"))?;
    window_size_state.replace(restored_size)
}

pub(crate) fn persist_main_window_size(app: &tauri::AppHandle) -> Result<(), String> {
    let window_size_state = app.state::<MainWindowSizeState>();
    let saved_size = match window_size_state.snapshot()? {
        Some(size) => size,
        None => {
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "Main window does not exist; cannot save window size".to_string())?;
            let physical_size = window
                .inner_size()
                .map_err(|error| format!("Failed to read main window size: {error}"))?;
            let scale_factor = window
                .scale_factor()
                .map_err(|error| format!("Failed to read main window scale factor: {error}"))?;
            logical_window_size_from_physical(&physical_size, scale_factor)
                .ok_or_else(|| "Invalid main window size; save skipped".to_string())?
        }
    };

    app.state::<GuiConfigState>()
        .set_window_size(saved_size)
        .map(|_| ())
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(crate) fn sanitize_gui_config(config: &mut GuiConfigFile) -> Result<bool, String> {
    let install_dir = core_install_dir()?;
    let persistent_auth_dir = fixed_oauth_dir()?;
    sanitize_gui_config_at(config, &install_dir, &persistent_auth_dir)
}

pub(crate) fn sanitize_gui_config_at(
    config: &mut GuiConfigFile,
    install_dir: &Path,
    persistent_auth_dir: &Path,
) -> Result<bool, String> {
    let mut changed = false;
    let normalized_locale = normalize_app_locale(&config.locale);
    if config.locale != normalized_locale {
        config.locale = normalized_locale.to_string();
        changed = true;
    }
    let default_terminal = normalize_agent_terminal(&config.default_terminal);
    if config.default_terminal != default_terminal {
        config.default_terminal = default_terminal;
        changed = true;
    }
    let host = config.host.trim();
    if config.host != host {
        config.host = host.to_string();
        changed = true;
    }
    let allow_lan = !is_loopback_host(&config.host);
    if config.allow_lan != allow_lan {
        config.allow_lan = allow_lan;
        changed = true;
    }
    if !config.auth_dir_user_selected {
        match migrate_auth_dir_at(config, install_dir, persistent_auth_dir) {
            Ok(migrated) => changed |= migrated,
            Err(error) => {
                eprintln!("Failed to recover OAuth directory; keeping existing configuration: {error}");
            }
        }
    }
    if config.auth_dir.trim().is_empty()
        || (!config.auth_dir_user_selected && Path::new(config.auth_dir.trim()) == persistent_auth_dir)
    {
        config.auth_dir = DEFAULT_AUTH_DIR.to_string();
        changed = true;
    }
    let original_api_keys = config.api_keys.clone();
    let configured_keys = config
        .api_keys
        .iter()
        .map(|entry| entry.key.trim().to_string())
        .collect::<Vec<_>>();
    config.api_keys =
        merge_core_api_keys_with_gui_metadata(&config.api_keys, &configured_keys, None);
    for entry in &mut config.api_keys {
        entry.key = entry.key.trim().to_string();
        entry.remark = entry.remark.trim().to_string();
    }
    if config.api_keys != original_api_keys {
        changed = true;
    }
    let proxy_url = if config.proxy_override {
        network_proxy::normalize_optional_proxy_url(&config.proxy_url)?
    } else {
        let proxy_url = config.proxy_url.trim().to_string();
        if proxy_url.is_empty() || network_proxy::normalize_proxy_url(&proxy_url).is_ok() {
            proxy_url
        } else {
            String::new()
        }
    };
    if config.proxy_url != proxy_url {
        config.proxy_url = proxy_url;
        changed = true;
    }
    let original_custom_mirrors = config.custom_download_mirrors.clone();
    config.custom_download_mirrors = config
        .custom_download_mirrors
        .iter()
        .filter_map(|url| normalize_custom_download_mirror_url(url).ok())
        .fold(Vec::new(), |mut mirrors, url| {
            if !mirrors.contains(&url) {
                mirrors.push(url);
            }
            mirrors
        });
    if config.custom_download_mirrors != original_custom_mirrors {
        changed = true;
    }
    if !config.active_custom_download_mirror.is_empty() {
        match normalize_custom_download_mirror_url(&config.active_custom_download_mirror) {
            Ok(normalized) if normalized != config.active_custom_download_mirror => {
                config.active_custom_download_mirror = normalized;
                changed = true;
            }
            Err(_) => {
                config.active_custom_download_mirror.clear();
                changed = true;
            }
            _ => {}
        }
    }
    if config.download_source == VersionDownloadSource::Custom
        && !config
            .custom_download_mirrors
            .contains(&config.active_custom_download_mirror)
    {
        config.download_source = VersionDownloadSource::Github;
        config.active_custom_download_mirror.clear();
        changed = true;
    }
    let routing_session_affinity_ttl = config.routing_session_affinity_ttl.trim().to_string();
    if config.routing_session_affinity_ttl != routing_session_affinity_ttl {
        config.routing_session_affinity_ttl = routing_session_affinity_ttl;
        changed = true;
    }
    let window_size = configured_window_size(config).map(|size| {
        if size.width == LEGACY_DEFAULT_MAIN_WINDOW_WIDTH
            && size.height == LEGACY_DEFAULT_MAIN_WINDOW_HEIGHT
        {
            SavedWindowSize {
                width: DEFAULT_MAIN_WINDOW_WIDTH,
                height: DEFAULT_MAIN_WINDOW_HEIGHT,
            }
        } else {
            size
        }
    });
    let normalized_width = window_size.map(|size| size.width);
    let normalized_height = window_size.map(|size| size.height);
    if config.window_width != normalized_width || config.window_height != normalized_height {
        config.window_width = normalized_width;
        config.window_height = normalized_height;
        changed = true;
    }
    Ok(changed)
}

pub(crate) fn write_gui_config(config: &GuiConfigFile) -> Result<(), String> {
    write_gui_config_to_path(config, &gui_config_path()?)
}

pub(crate) fn write_gui_config_to_path(
    config: &GuiConfigFile,
    config_path: &Path,
) -> Result<(), String> {
    use toml_edit::{value, Array, Document, InlineTable, Item, Value};

    validate_gui_config(config)?;
    let existing = fs::read_to_string(config_path).ok();
    let mut document = existing
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .and_then(|content| content.parse::<Document>().ok())
        .unwrap_or_default();
    let root = document.as_table_mut();
    if config.usage_statistics_disabled == Some(true) {
        set_codex_table_item(root, "usage_statistics_disabled", value(true));
    } else {
        root.remove("usage_statistics_disabled");
    }
    for (key, item) in [
        ("locale", value(config.locale.as_str())),
        ("port", value(i64::from(config.port))),
        ("allow-lan", value(config.allow_lan)),
        ("host", value(config.host.as_str())),
        ("run-on-startup", value(config.run_on_startup)),
        ("start-core-on-launch", value(config.start_core_on_launch)),
        ("silent-start", value(config.silent_start)),
        ("close-behavior", value(config.close_behavior.as_str())),
        ("default-terminal", value(config.default_terminal.as_str())),
        ("auth-dir", value(config.auth_dir.as_str())),
        ("auth-dir-user-selected", value(config.auth_dir_user_selected)),
        (
            "management-secret-key",
            value(config.management_secret_key.as_str()),
        ),
        ("debug", value(config.debug)),
        ("commercial-mode", value(config.commercial_mode)),
        ("logging-to-file", value(config.logging_to_file)),
        (
            "logs-max-total-size-mb",
            value(i64::from(config.logs_max_total_size_mb)),
        ),
        (
            "error-logs-max-files",
            value(i64::from(config.error_logs_max_files)),
        ),
        (
            "usage-statistics-enabled",
            value(config.usage_statistics_enabled),
        ),
        (
            "redis-usage-queue-retention-seconds",
            value(i64::from(config.redis_usage_queue_retention_seconds)),
        ),
        ("request-log", value(config.request_log)),
        ("plugins-enabled", value(config.plugins_enabled)),
        ("routing-strategy", value(config.routing_strategy.as_str())),
        ("proxy-url", value(config.proxy_url.as_str())),
        ("proxy-override", value(config.proxy_override)),
        ("download-source", value(config.download_source.as_str())),
        (
            "prefer-gitcode-downloads",
            value(config.prefer_gitcode_downloads),
        ),
        (
            "routing-session-affinity",
            value(config.routing_session_affinity),
        ),
        (
            "routing-session-affinity-ttl",
            value(config.routing_session_affinity_ttl.as_str()),
        ),
        ("disable-cooling", value(config.disable_cooling)),
        ("request-retry", value(i64::from(config.request_retry))),
        (
            "max-retry-credentials",
            value(i64::from(config.max_retry_credentials)),
        ),
        (
            "max-retry-interval",
            value(i64::from(config.max_retry_interval)),
        ),
        (
            "streaming-bootstrap-retries",
            value(i64::from(config.streaming_bootstrap_retries)),
        ),
    ] {
        set_codex_table_item(root, key, item);
    }
    for key in [
        "proxy-mode",
        "proxy-manual-url",
        "codex-session-repair-on-launch",
        "claude-code-working-directory",
        "claude-code-working-directory-prompt-disabled",
    ] {
        root.remove(key);
    }
    for (key, dimension) in [
        ("window-width", config.window_width),
        ("window-height", config.window_height),
    ] {
        if let Some(dimension) = dimension {
            set_codex_table_item(root, key, value(i64::from(dimension)));
        } else {
            root.remove(key);
        }
    }
    let mut api_keys = Array::new();
    for entry in &config.api_keys {
        let mut table = InlineTable::new();
        table.insert("key", Value::from(entry.key.as_str()));
        table.insert("remark", Value::from(entry.remark.as_str()));
        api_keys.push(Value::InlineTable(table));
    }
    set_codex_table_item(root, "api-keys", Item::Value(Value::Array(api_keys)));
    let mut custom_download_mirrors = Array::new();
    for url in &config.custom_download_mirrors {
        custom_download_mirrors.push(url.as_str());
    }
    set_codex_table_item(
        root,
        "custom-download-mirrors",
        Item::Value(Value::Array(custom_download_mirrors)),
    );
    set_codex_table_item(
        root,
        "active-custom-download-mirror",
        value(config.active_custom_download_mirror.as_str()),
    );
    let mut api_access_remarks = Array::new();
    for entry in &config.api_access_remarks {
        let mut table = InlineTable::new();
        table.insert(
            "provider-section",
            Value::from(entry.provider_section.as_str()),
        );
        table.insert("api-key-hash", Value::from(entry.api_key_hash.as_str()));
        if !entry.record_hash.is_empty() {
            table.insert("record-hash", Value::from(entry.record_hash.as_str()));
        }
        table.insert("remark", Value::from(entry.remark.as_str()));
        api_access_remarks.push(Value::InlineTable(table));
    }
    set_codex_table_item(
        root,
        "api-access-remarks",
        Item::Value(Value::Array(api_access_remarks)),
    );

    let content = document.to_string();
    toml::from_str::<GuiConfigFile>(&content)
        .map_err(|error| format!("Failed to validate GUI configuration: {error}"))?;
    if existing.as_deref() == Some(content.as_str()) {
        return Ok(());
    }
    write_bytes_directly(config_path, content.as_bytes())
}

pub(crate) fn validate_gui_config(config: &GuiConfigFile) -> Result<(), String> {
    if config.port == 0 {
        return Err("GUI configuration port must be between 1 and 65535".to_string());
    }
    if config.host.chars().any(char::is_control) {
        return Err("Invalid GUI configuration host".to_string());
    }
    if config.auth_dir.trim().is_empty() || config.auth_dir.chars().any(char::is_control) {
        return Err("Credentials directory cannot be empty or contain control characters".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let auth_dir = auth_dir_path_for_core(&config.auth_dir, &core_install_dir()?)?;
        if auth_dir_is_inside_macos_app_bundle(&auth_dir) {
            return Err("OAuth credentials directory cannot be inside a macOS application bundle".to_string());
        }
    }
    for entry in &config.api_keys {
        validate_core_api_key(&entry.key)?;
        validate_api_key_remark(&entry.remark)?;
    }
    for entry in &config.api_access_remarks {
        validate_api_access_provider_section(&entry.provider_section)?;
        if entry.api_key_hash.len() != 64
            || !entry
                .api_key_hash
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err("Invalid key fingerprint in API access note".to_string());
        }
        if !entry.record_hash.is_empty()
            && (entry.record_hash.len() != 64
                || !entry
                    .record_hash
                    .chars()
                    .all(|character| character.is_ascii_hexdigit()))
        {
            return Err("Invalid entry fingerprint in API access note".to_string());
        }
        validate_api_key_remark(&entry.remark)?;
    }
    if config.management_secret_key.is_empty() {
        validate_management_secret_key(&config.management_secret_key)?;
    } else {
        validate_strong_management_secret_key(&config.management_secret_key)?;
    }
    validate_routing_strategy(config.routing_strategy.trim())?;
    if config.proxy_override {
        network_proxy::normalize_optional_proxy_url(&config.proxy_url)?;
    } else if config.proxy_url.chars().any(char::is_control) {
        return Err("Proxy URL cannot contain control characters".to_string());
    }
    for url in &config.custom_download_mirrors {
        if normalize_custom_download_mirror_url(url).as_deref() != Ok(url.as_str()) {
            return Err(format!("Invalid custom download mirror URL: {url}"));
        }
    }
    if config.download_source == VersionDownloadSource::Custom
        && !config
            .custom_download_mirrors
            .contains(&config.active_custom_download_mirror)
    {
        return Err("The selected custom download mirror does not exist".to_string());
    }
    if config
        .routing_session_affinity_ttl
        .chars()
        .any(char::is_control)
    {
        return Err("Session affinity TTL cannot contain control characters".to_string());
    }
    if !(1..=3600).contains(&config.redis_usage_queue_retention_seconds) {
        return Err("Redis usage queue retention must be between 1 and 3600 seconds".to_string());
    }
    Ok(())
}

pub(crate) fn gui_config_path() -> Result<PathBuf, String> {
    Ok(core_base_dir()?.join(GUI_CONFIG_FILE))
}

pub(crate) fn legacy_gui_config_path() -> Result<PathBuf, String> {
    Ok(core_base_dir()?.join(LEGACY_GUI_CONFIG_FILE))
}
