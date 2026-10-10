use super::*;

const AGENT_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const AGENT_VERSION_PROBE_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[cfg(not(test))]
pub(crate) fn agent_configuration_environment(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
pub(crate) fn agent_configuration_environment(_: &str) -> Option<PathBuf> {
    None
}

pub(crate) fn agent_config_paths(client: AgentClient, home: &Path) -> Vec<PathBuf> {
    match client {
        AgentClient::ClaudeCode => {
            let directory = home.join(".claude");
            let settings = directory.join("settings.json");
            let legacy = directory.join("claude.json");
            let settings_state_exists = agent_state_path(std::slice::from_ref(&settings))
                .map(|path| path.exists())
                .unwrap_or(false);
            let legacy_state_exists = agent_state_path(std::slice::from_ref(&legacy))
                .map(|path| path.exists())
                .unwrap_or(false);
            vec![if settings_state_exists {
                settings
            } else if legacy_state_exists || (!settings.exists() && legacy.exists()) {
                legacy
            } else {
                settings
            }]
        }
        AgentClient::ClaudeDesktop => claude_desktop_config_paths(home),
        AgentClient::Codex => vec![codex_configuration_directory(home).join("config.toml")],
        AgentClient::OpenCode => vec![opencode_config_path(home)],
        AgentClient::OpenClaw => vec![openclaw_agent_config_path(home)],
        AgentClient::Hermes => vec![hermes_agent_config_path(home)],
        AgentClient::DeepSeekHarness => {
            let directory = deepseek_harness_home(home);
            vec![
                directory.join(DEEPSEEK_HARNESS_SETTINGS_FILE),
                directory.join(DEEPSEEK_HARNESS_CREDENTIALS_FILE),
            ]
        }
        AgentClient::WorkBuddy => vec![workbuddy_home(home).join("models.json")],
        AgentClient::AntigravityCli => antigravity_config_paths(client, home),
        AgentClient::ZCode => vec![home.join(".zcode/v2").join(ZCODE_PROVIDER_CONFIG_FILE)],
        AgentClient::KimiCode => vec![kimi_code_home(home).join(KIMI_CODE_CONFIG_FILE)],
        AgentClient::GrokBuild => vec![grok_build_home(home).join(GROK_BUILD_CONFIG_FILE)],
        AgentClient::Omp => {
            let directory = omp_agent_directory(home);
            vec![directory.join(OMP_MODELS_FILE), directory.join(OMP_SETTINGS_FILE)]
        }
    }
}

pub(crate) fn opencode_config_path(home: &Path) -> PathBuf {
    #[cfg(test)]
    let (custom_config, xdg_config_home): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    #[cfg(not(test))]
    let custom_config = env::var_os("OPENCODE_CONFIG").map(PathBuf::from);
    #[cfg(not(test))]
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    opencode_config_path_from_environment(
        home,
        custom_config.as_deref(),
        xdg_config_home.as_deref(),
    )
}

pub(crate) fn opencode_config_path_from_environment(
    home: &Path,
    custom_config: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> PathBuf {
    if let Some(path) = custom_config.filter(|path| !path.as_os_str().is_empty()) {
        return path.to_path_buf();
    }

    let config_directory = xdg_config_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode");
    let json = config_directory.join("opencode.json");
    let jsonc = config_directory.join("opencode.jsonc");
    let candidates = [json.clone(), jsonc];

    candidates
        .iter()
        .find(|path| {
            agent_state_path(std::slice::from_ref(*path))
                .map(|state| state.exists())
                .unwrap_or(false)
        })
        .or_else(|| candidates.iter().find(|path| path.is_file()))
        .cloned()
        .unwrap_or(json)
}

pub(crate) fn openclaw_agent_config_path(home: &Path) -> PathBuf {
    #[cfg(test)]
    let (custom_config, state_directory, openclaw_home): (
        Option<PathBuf>,
        Option<PathBuf>,
        Option<PathBuf>,
    ) = (None, None, None);
    #[cfg(not(test))]
    let custom_config = agent_configuration_environment("OPENCLAW_CONFIG_PATH");
    #[cfg(not(test))]
    let state_directory = agent_configuration_environment("OPENCLAW_STATE_DIR");
    #[cfg(not(test))]
    let openclaw_home = agent_configuration_environment("OPENCLAW_HOME");
    openclaw_agent_config_path_from_environment(
        home,
        custom_config.as_deref(),
        state_directory.as_deref(),
        openclaw_home.as_deref(),
    )
}

pub(crate) fn openclaw_agent_config_path_from_environment(
    home: &Path,
    custom_config: Option<&Path>,
    state_directory: Option<&Path>,
    openclaw_home: Option<&Path>,
) -> PathBuf {
    if let Some(path) = custom_config.filter(|path| !path.as_os_str().is_empty()) {
        return expand_openclaw_path(home, path);
    }

    if let Some(path) = state_directory.filter(|path| !path.as_os_str().is_empty()) {
        return expand_openclaw_path(home, path).join("openclaw.json");
    }

    openclaw_home
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| expand_openclaw_path(home, path))
        .unwrap_or_else(|| home.to_path_buf())
        .join(".openclaw/openclaw.json")
}

fn expand_openclaw_path(home: &Path, path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if value == "~" {
        return home.to_path_buf();
    }
    if let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return home.join(suffix);
    }
    path.to_path_buf()
}

pub(crate) fn kimi_code_home(home: &Path) -> PathBuf {
    agent_configuration_environment("KIMI_CODE_HOME").unwrap_or_else(|| home.join(".kimi-code"))
}

pub(crate) fn grok_build_home(home: &Path) -> PathBuf {
    agent_configuration_environment("GROK_HOME").unwrap_or_else(|| home.join(".grok"))
}

pub(crate) fn omp_agent_directory(home: &Path) -> PathBuf {
    agent_configuration_environment("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".omp/agent"))
}

pub(crate) fn deepseek_harness_home(home: &Path) -> PathBuf {
    agent_configuration_environment("DSH_HOME").unwrap_or_else(|| home.join(".dsh"))
}

pub(crate) fn read_deepseek_harness_profile_version(home: &Path) -> Option<String> {
    let directory = deepseek_harness_home(home);
    [
        directory.join("profiles/node_modules/@deepseek-ai/dsh/package.json"),
        directory.join("node_modules/@deepseek-ai/dsh/package.json"),
    ]
    .into_iter()
    .find_map(|path| read_package_json_version(&path))
}

pub(crate) fn read_package_json_version(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let package = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string)
}

pub(crate) fn pi_agent_directory(home: &Path) -> PathBuf {
    agent_configuration_environment("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi/agent"))
}

pub(crate) fn pi_provider_config_path(home: &Path) -> PathBuf {
    pi_agent_directory(home).join(PI_AGENT_CONFIG_FILE)
}

pub(crate) fn pi_provider_settings_path(home: &Path) -> PathBuf {
    pi_agent_directory(home).join(PI_AGENT_SETTINGS_FILE)
}

pub(crate) fn pi_provider_package_json_path(home: &Path) -> PathBuf {
    pi_agent_directory(home)
        .join("npm/node_modules/@router-for-me/pi-cliproxyapi-provider/package.json")
}

pub(crate) fn read_pi_provider_version(home: &Path) -> Result<Option<String>, String> {
    let path = pi_provider_package_json_path(home);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Failed to read Pi provider package.json {}: {error}",
            path_to_string(&path)
        )
    })?;
    let package = serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
        format!(
            "Failed to parse Pi provider package.json {}: {error}",
            path_to_string(&path)
        )
    })?;
    Ok(package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

pub(crate) fn parse_pi_provider_latest_version(
    payload: &serde_json::Value,
) -> Result<String, String> {
    payload
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Invalid Pi provider version returned by npm registry".to_string())
}

pub(crate) async fn fetch_pi_provider_latest_version(proxy_url: &str) -> Result<String, String> {
    let client = build_http_client_with_proxy(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12)),
        proxy_url,
        "Failed to create Pi provider update check client",
    )?;
    let response = client
        .get(PI_CLIPROXYAPI_NPM_LATEST_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("Failed to query latest Pi provider version: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Failed to query latest Pi provider version: HTTP {}",
            status.as_u16()
        ));
    }
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("Failed to parse latest Pi provider version: {error}"))?;
    parse_pi_provider_latest_version(&payload)
}

pub(crate) fn pi_provider_update_available(installed: &str, latest: &str) -> Result<bool, String> {
    let parse = |value: &str| {
        semver::Version::parse(value.trim().trim_start_matches('v'))
            .map_err(|error| format!("Failed to parse Pi provider version {value}: {error}"))
    };
    Ok(parse(latest)? > parse(installed)?)
}

pub(crate) fn pi_package_source_matches(value: &str) -> bool {
    let value = value.trim();
    value == PI_CLIPROXYAPI_PACKAGE || value.starts_with(&format!("{PI_CLIPROXYAPI_PACKAGE}@"))
}

pub(crate) fn read_pi_settings(home: &Path) -> Result<Option<serde_json::Value>, String> {
    let path = pi_provider_settings_path(home);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Failed to read Pi settings.json {}: {error}",
            path_to_string(&path)
        )
    })?;
    serde_json::from_str::<serde_json::Value>(&content)
        .map(Some)
        .map_err(|error| {
            format!(
                "Failed to parse Pi settings.json {}: {error}",
                path_to_string(&path)
            )
        })
}

pub(crate) fn pi_settings_contains_provider(settings: &serde_json::Value) -> bool {
    settings
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|packages| {
            packages.iter().any(|package| {
                package.as_str().is_some_and(pi_package_source_matches)
                    || package
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(pi_package_source_matches)
            })
        })
}

pub(crate) fn pi_provider_package_installed(home: &Path) -> Result<bool, String> {
    Ok(read_pi_settings(home)?
        .as_ref()
        .is_some_and(pi_settings_contains_provider))
}

pub(crate) fn build_pi_provider_config(
    existing: Option<&str>,
    base_url: &str,
    api_key: &str,
) -> Result<String, String> {
    let mut root = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| format!("Failed to parse Pi CLIProxyAPI configuration: {error}"))?,
        None => serde_json::json!({}),
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| "Pi CLIProxyAPI configuration root must be a JSON object".to_string())?;
    object.insert("baseUrl".to_string(), serde_json::json!(base_url));
    object.insert("apiKey".to_string(), serde_json::json!(api_key));
    let mut rendered = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("Failed to generate Pi CLIProxyAPI configuration: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn build_pi_provider_settings(
    existing: &str,
    default_model: &str,
) -> Result<String, String> {
    let default_model = default_model.trim();
    if default_model.is_empty() {
        return Err("Pi default model cannot be empty".to_string());
    }
    let mut root = serde_json::from_str::<serde_json::Value>(existing)
        .map_err(|error| format!("Failed to parse Pi settings.json: {error}"))?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| "Pi settings.json root must be a JSON object".to_string())?;
    let packages = object
        .entry("packages")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| "Pi settings.json packages must be an array".to_string())?;
    if !packages.iter().any(|package| {
        package.as_str().is_some_and(pi_package_source_matches)
            || package
                .get("source")
                .and_then(serde_json::Value::as_str)
                .is_some_and(pi_package_source_matches)
    }) {
        packages.push(serde_json::json!(PI_CLIPROXYAPI_PACKAGE));
    }
    object.insert(
        "defaultProvider".to_string(),
        serde_json::json!(PI_CLIPROXYAPI_PROVIDER_ID),
    );
    object.insert("defaultModel".to_string(), serde_json::json!(default_model));
    let mut rendered = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("Failed to generate Pi settings.json: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn inspect_pi_provider_status(
    home: &Path,
    port: u16,
    api_key: &str,
) -> AgentConfigStatus {
    inspect_pi_provider_status_with_executable(home, port, api_key, None)
}

pub(crate) fn inspect_pi_provider_status_with_executable(
    home: &Path,
    port: u16,
    api_key: &str,
    executable_override: Option<&Path>,
) -> AgentConfigStatus {
    let config_path = pi_provider_config_path(home);
    let settings_path = pi_provider_settings_path(home);
    let executable = executable_override.filter(|path| path.is_file()).map(Path::to_path_buf)
        .or_else(|| find_pi_executable(home));
    let mut errors = Vec::new();
    let (plugin_installed, default_provider_matches, current_model) = match read_pi_settings(home) {
        Ok(Some(settings)) => {
            let plugin_installed = pi_settings_contains_provider(&settings);
            let default_provider_matches = settings
                .get("defaultProvider")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value.trim() == PI_CLIPROXYAPI_PROVIDER_ID);
            let current_model = settings
                .get("defaultModel")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            (plugin_installed, default_provider_matches, current_model)
        }
        Ok(None) => (false, false, None),
        Err(error) => {
            errors.push(error);
            (false, false, None)
        }
    };
    let mut credentials_match = false;
    if config_path.is_file() {
        match fs::read_to_string(&config_path)
            .map_err(|error| format!("Failed to read Pi CLIProxyAPI configuration: {error}"))
            .and_then(|content| {
                let root = serde_json::from_str::<serde_json::Value>(&content)
                    .map_err(|error| format!("Failed to parse Pi CLIProxyAPI configuration: {error}"))?;
                let object = root
                    .as_object()
                    .ok_or_else(|| "Pi CLIProxyAPI configuration root must be a JSON object".to_string())?;
                let expected_base_url = managed_core_loopback_origin(port);
                credentials_match = object
                    .get("baseUrl")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value.trim() == expected_base_url)
                    && object.get("apiKey").and_then(serde_json::Value::as_str) == Some(api_key);
                Ok(())
            }) {
            Ok(()) => {}
            Err(error) => errors.push(error),
        }
    }
    let config_exists = config_path.is_file() || settings_path.is_file();
    let config_valid = errors.is_empty();
    let configured = plugin_installed
        && credentials_match
        && default_provider_matches
        && current_model.is_some()
        && config_valid;
    let mut warnings = Vec::new();
    if executable.is_some() && !plugin_installed && config_valid {
        warnings.push("Pi CLIProxyAPI provider plugin is not installed".to_string());
    } else if plugin_installed
        && (!credentials_match || !default_provider_matches || current_model.is_none())
        && config_valid
        && config_exists
    {
        warnings.push(
            "Pi plugin configuration is incomplete. Use “Apply Configuration” to rewrite the credentials, default provider, and default model".to_string(),
        );
    }
    if executable.is_none() && config_exists {
        warnings.push(
            "Only a Pi configuration file was detected. You can still edit the configuration and connect it to CPA, but launch functionality is unavailable".to_string(),
        );
    }
    let modification_state = if configured {
        "applied"
    } else {
        "unconfigured"
    };
    let error = errors.into_iter().next();
    let cli_version = executable
        .as_deref()
        .and_then(|path| read_agent_version(path, home));
    let plugin_version = read_pi_provider_version(home).ok().flatten();
    AgentConfigStatus {
        id: PI_AGENT_ID.to_string(),
        name: PI_AGENT_NAME.to_string(),
        supported_platform: true,
        installed: executable.is_some(),
        plugin_installed,
        launch_targets: executable
            .as_ref()
            .map(|path| {
                vec![AgentLaunchTarget {
                    id: "cli".to_string(),
                    label: PI_AGENT_NAME.to_string(),
                    detail: path_to_string(path),
                }]
            })
            .unwrap_or_default(),
        version: cli_version.clone(),
        cli_version,
        app_version: None,
        plugin_version,
        config_paths: vec![path_to_string(&config_path), path_to_string(&settings_path)],
        config_exists,
        config_valid,
        configured,
        configuration_synchronized: configured,
        connection_state: agent_connection_state(
            configured,
            config_valid,
            Ok(config_path.is_file() || default_provider_matches),
        )
        .into(),
        current_model: current_model.clone(),
        oauth_configuration: false,
        codex_native_oauth: false,
        modification_enabled: configured,
        modification_state: modification_state.to_string(),
        backup_available: false,
        applied_model: current_model,
        claude_code_model_mappings: None,
        claude_desktop_model_mappings: None,
        warnings,
        error,
    }
}

pub(crate) fn install_pi_provider_inner(
    home: &Path,
    executable: &Path,
    port: u16,
    api_key: &str,
    default_model: &str,
    proxy_url: &str,
) -> Result<AgentConfigActionResult, String> {
    if port == 0 {
        return Err("Invalid kernel port".to_string());
    }
    if api_key.trim().is_empty() {
        return Err("EasyCLIProxyAPI has no available API key".to_string());
    }
    let settings_path = pi_provider_settings_path(home);
    let mut changed_files = Vec::new();
    if !pi_provider_package_installed(home)? {
        config_package_operation(home, "plugin-install", || {
            install_pi_package(executable, home, proxy_url)
        })?;
        changed_files.push(path_to_string(&settings_path));
    }

    let mut result = repair_pi_provider_inner(home, port, api_key, default_model)?;
    changed_files.extend(result.changed_files);
    result.changed_files = changed_files;
    Ok(result)
}

pub(crate) fn configure_pi_provider_without_cli_inner(
    home: &Path,
    port: u16,
    api_key: &str,
    default_model: &str,
) -> Result<AgentConfigActionResult, String> {
    if port == 0 {
        return Err("Invalid kernel port".to_string());
    }
    if api_key.trim().is_empty() {
        return Err("EasyCLIProxyAPI has no available API key".to_string());
    }
    let settings_path = pi_provider_settings_path(home);
    config_package_operation(home, "plugin-config", || {
        let settings = if settings_path.is_file() {
            fs::read_to_string(&settings_path).map_err(|error| {
                format!(
                    "Failed to read Pi settings.json {}: {error}",
                    path_to_string(&settings_path)
                )
            })?
        } else {
            "{}".to_string()
        };
        let rendered = build_pi_provider_settings(&settings, default_model)?;
        write_bytes_directly(&settings_path, rendered.as_bytes())
    })?;
    repair_pi_provider_inner(home, port, api_key, default_model)
}

pub(crate) fn repair_pi_provider_inner(
    home: &Path,
    port: u16,
    api_key: &str,
    default_model: &str,
) -> Result<AgentConfigActionResult, String> {
    if port == 0 {
        return Err("Core port is invalid".to_string());
    }
    if api_key.trim().is_empty() {
        return Err("EasyCLIProxyAPI has no usable API key".to_string());
    }
    let config_path = pi_provider_config_path(home);
    let settings_path = pi_provider_settings_path(home);
    let _guard = AGENT_CONFIG_FILE_LOCK
        .lock()
        .map_err(|_| "Configuration file lock is poisoned")?;
    let before = config_images(&config_paths(PI_AGENT_ID, home)?)?;
    validate_config_images(&before)?;
    if !pi_provider_package_installed(home)? {
        return Err("Pi CLIProxyAPI provider is not installed".to_string());
    }
    let existing = if config_path.is_file() {
        Some(fs::read_to_string(&config_path).map_err(|error| {
            format!(
                "Failed to read Pi CLIProxyAPI configuration {}: {error}",
                path_to_string(&config_path)
            )
        })?)
    } else {
        None
    };
    let base_url = managed_core_loopback_origin(port);
    let rendered = build_pi_provider_config(existing.as_deref(), &base_url, api_key)?;
    let settings = fs::read_to_string(&settings_path).map_err(|error| {
        format!(
            "Failed to read Pi settings.json {}: {error}",
            path_to_string(&settings_path)
        )
    })?;
    let rendered_settings = build_pi_provider_settings(&settings, default_model)?;
    config_updates(
        PI_AGENT_ID,
        home,
        &before,
        &[
            AgentFileUpdate {
                path: config_path,
                after: rendered,
            },
            AgentFileUpdate {
                path: settings_path,
                after: rendered_settings,
            },
        ],
        "update",
        Some(default_model.to_string()),
        None,
    )
}

pub(crate) fn update_pi_provider_inner(
    home: &Path,
    executable: &Path,
    port: u16,
    api_key: &str,
    default_model: &str,
    proxy_url: &str,
) -> Result<AgentConfigActionResult, String> {
    if !pi_provider_package_installed(home)? {
        return Err("Pi CLIProxyAPI provider plugin is not installed".to_string());
    }
    config_package_operation(home, "plugin-update", || {
        update_pi_package(executable, home, proxy_url)
    })?;
    let mut result = repair_pi_provider_inner(home, port, api_key, default_model)?;
    result.outcome = "updated".to_string();
    Ok(result)
}

pub(crate) fn uninstall_pi_provider_inner(
    home: &Path,
    executable: &Path,
) -> Result<AgentConfigActionResult, String> {
    if !pi_provider_package_installed(home)? {
        return Ok(action_result(
            "not-installed",
            false,
            None,
            Vec::new(),
            Vec::new(),
        ));
    }
    config_package_operation(home, "plugin-remove", || {
        remove_pi_package(executable, home)
    })?;
    Ok(action_result(
        "removed",
        false,
        None,
        vec![path_to_string(&pi_provider_settings_path(home))],
        Vec::new(),
    ))
}

pub(crate) fn agent_command_path(
    home: &Path,
    executable: &Path,
) -> Result<std::ffi::OsString, String> {
    let mut directories = agent_executable_directories(home);
    if let Some(executable_directory) = executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        directories.retain(|directory| directory != executable_directory);
        directories.insert(0, executable_directory.to_path_buf());
    }

    env::join_paths(directories).map_err(|error| format!("Failed to construct agent command PATH: {error}"))
}

pub(crate) fn configure_agent_command_environment(
    command: &mut Command,
    home: &Path,
    executable: &Path,
) -> Result<(), String> {
    command.env("PATH", agent_command_path(home, executable)?);
    Ok(())
}

pub(crate) fn install_pi_package(
    executable: &Path,
    home: &Path,
    proxy_url: &str,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = windows_command_for_executable(executable, false);
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(executable);

    command
        .arg("install")
        .arg(PI_CLIPROXYAPI_PACKAGE)
        .current_dir(home)
        .stdin(Stdio::null());
    configure_background_command(&mut command);
    configure_networked_command(&mut command, proxy_url);
    configure_agent_command_environment(&mut command, home, executable)?;
    let output = command
        .output()
        .map_err(|error| format!("Failed to run Pi plugin installation: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "Pi CLIProxyAPI provider plugin installation failed{}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

pub(crate) fn update_pi_package(
    executable: &Path,
    home: &Path,
    proxy_url: &str,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = windows_command_for_executable(executable, false);
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(executable);

    command
        .arg("update")
        .arg("--extension")
        .arg(PI_CLIPROXYAPI_PACKAGE)
        .current_dir(home)
        .stdin(Stdio::null());
    configure_background_command(&mut command);
    configure_networked_command(&mut command, proxy_url);
    configure_agent_command_environment(&mut command, home, executable)?;
    let output = command
        .output()
        .map_err(|error| format!("Failed to run Pi plugin update: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "Pi CLIProxyAPI provider plugin update failed{}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

pub(crate) fn remove_pi_package(executable: &Path, home: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = windows_command_for_executable(executable, false);
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(executable);

    command
        .arg("remove")
        .arg(PI_CLIPROXYAPI_PACKAGE)
        .current_dir(home)
        .stdin(Stdio::null());
    configure_background_command(&mut command);
    configure_agent_command_environment(&mut command, home, executable)?;
    let output = command
        .output()
        .map_err(|error| format!("Failed to run Pi plugin uninstall: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "Pi CLIProxyAPI provider plugin uninstall failed{}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

pub(crate) fn codex_configuration_directory(home: &Path) -> PathBuf {
    agent_configuration_environment("CODEX_HOME").unwrap_or_else(|| home.join(".codex"))
}

pub(crate) fn current_codex_oauth_configuration(home: &Path) -> Result<bool, String> {
    let path = codex_configuration_directory(home).join("config.toml");
    if !path.is_file() {
        return Ok(false);
    }
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(&path).map_err(|error| format!("Failed to read Codex configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Codex configuration: {error}"))?;
    Ok(root
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
        .and_then(toml::Value::as_table)
        .and_then(|provider| provider.get("requires_openai_auth"))
        .and_then(toml::Value::as_bool)
        == Some(true))
}

pub(crate) fn codex_auth_file_has_oauth_tokens(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    root.get("tokens")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|tokens| {
            tokens
                .values()
                .any(|value| value.as_str().is_some_and(|token| !token.trim().is_empty()))
        })
}

pub(crate) fn codex_auth_file_has_api_key(path: &Path, api_key: &str) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    let api_key_matches = root
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == api_key);
    let mode_matches = root
        .get("auth_mode")
        .is_none_or(|value| value.as_str() == Some("apikey"));
    api_key_matches && mode_matches
}

pub(crate) fn validate_codex_oauth_login(home: &Path) -> Result<(), String> {
    if validate_codex_oauth_login_at(&codex_configuration_directory(home).join("auth.json")).is_ok()
    {
        return Ok(());
    }
    available_codex_oauth_auth(home).map(|_| ())
}

pub(crate) fn validate_codex_oauth_login_at(auth_path: &Path) -> Result<(), String> {
    if codex_auth_file_has_oauth_tokens(auth_path) {
        Ok(())
    } else {
        Err(CODEX_OAUTH_LOGIN_REQUIRED_ERROR.to_string())
    }
}

pub(crate) fn clear_codex_config_files(home: &Path) -> Result<Vec<String>, String> {
    let mut paths = config_paths("codex", home)?;
    paths.push(codex_configuration_directory(home).join(CODEX_NATIVE_OAUTH_STATE_FILE));
    let before = config_images(&paths)?;
    let mut after = before.clone();
    for (path, bytes) in &mut after {
        if path.file_name().and_then(|v| v.to_str()).is_some_and(|v| {
            v == "auth.json" || v == "config.toml" || v == CODEX_NATIVE_OAUTH_STATE_FILE
        }) {
            *bytes = None;
        }
    }
    Ok(commit_config("codex", &paths, &before, &after, "clear", None)?.changed_files)
}

pub(crate) fn codex_model_catalog_path(home: &Path) -> PathBuf {
    codex_configuration_directory(home).join(CODEX_MODEL_CATALOG_FILE)
}

pub(crate) fn expected_agent_record_paths(client: AgentClient, paths: &[PathBuf]) -> Vec<PathBuf> {
    if client == AgentClient::Codex && paths.len() == 1 {
        let mut expected = paths.to_vec();
        expected.push(paths[0].with_file_name(CODEX_MODEL_CATALOG_FILE));
        expected.push(paths[0].with_file_name("auth.json"));
        expected
    } else {
        paths.to_vec()
    }
}

pub(crate) fn claude_desktop_config_paths(_home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let (normal, threep) = {
        let support = _home.join("Library/Application Support");
        (support.join("Claude"), support.join("Claude-3p"))
    };
    #[cfg(target_os = "windows")]
    {
        let local = agent_configuration_environment("LOCALAPPDATA")
            .unwrap_or_else(|| _home.join("AppData/Local"));
        claude_desktop_config_paths_from_local_app_data(&local)
    }
    #[cfg(target_os = "linux")]
    let (normal, threep) = {
        let config_home = agent_configuration_environment("XDG_CONFIG_HOME")
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| _home.join(".config"));
        (config_home.join("Claude"), config_home.join("Claude-3p"))
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Vec::new();

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        claude_desktop_config_paths_from_directories(normal, threep)
    }
}

pub(crate) fn claude_desktop_config_paths_from_directories(
    normal: PathBuf,
    threep: PathBuf,
) -> Vec<PathBuf> {
    let library = threep.join("configLibrary");
    vec![
        normal.join("claude_desktop_config.json"),
        threep.join("claude_desktop_config.json"),
        library.join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")),
        library.join("_meta.json"),
    ]
}

#[cfg(target_os = "windows")]
pub(crate) fn claude_desktop_config_paths_from_local_app_data(
    local_app_data: &Path,
) -> Vec<PathBuf> {
    let normal = find_windows_claude_data_directory(local_app_data, false)
        .unwrap_or_else(|| local_app_data.join("Claude"));
    let threep = find_windows_claude_data_directory(local_app_data, true)
        .unwrap_or_else(|| local_app_data.join("Claude-3p"));
    claude_desktop_config_paths_from_directories(normal, threep)
}

#[cfg(target_os = "windows")]
pub(crate) fn find_windows_claude_data_directory(
    local_app_data: &Path,
    threep: bool,
) -> Option<PathBuf> {
    let exact_name = if threep { "Claude-3p" } else { "Claude" };
    let exact = local_app_data.join(exact_name);
    if exact.is_dir() {
        return Some(exact);
    }

    let mut candidates = fs::read_dir(local_app_data)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                return false;
            };
            let normalized = name.to_ascii_lowercase();
            normalized.starts_with("claude") && normalized.contains("-3p") == threep
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

pub(crate) fn hermes_agent_config_path(home: &Path) -> PathBuf {
    if let Some(directory) = agent_configuration_environment("HERMES_HOME") {
        return directory.join("config.yaml");
    }
    #[cfg(target_os = "windows")]
    {
        agent_configuration_environment("LOCALAPPDATA")
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("hermes/config.yaml")
    }
    #[cfg(not(target_os = "windows"))]
    home.join(".hermes/config.yaml")
}

fn hermes_managed_provider(root: &serde_yaml::Value) -> Option<&serde_yaml::Value> {
    let custom_provider = root
        .get("custom_providers")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|providers| {
            providers.iter().find(|provider| {
                provider.get("name").and_then(serde_yaml::Value::as_str)
                    == Some(MANAGED_AGENT_PROVIDER_ID)
            })
        });
    custom_provider.or_else(|| {
        root.get("providers")
            .and_then(serde_yaml::Value::as_mapping)
            .and_then(|providers| {
                providers
                    .iter()
                    .find(|(key, provider)| {
                        key.as_str() == Some(MANAGED_AGENT_PROVIDER_ID)
                            || provider.get("name").and_then(serde_yaml::Value::as_str)
                                == Some(MANAGED_AGENT_PROVIDER_ID)
                    })
                    .map(|(_, provider)| provider)
            })
    })
}

pub(crate) fn inspect_agent_config(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
) -> AgentConfigStatus {
    inspect_agent_config_with_executable(client, home, port, api_key, None)
}

pub(crate) fn manual_desktop_path(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| {
        path.is_file()
            || (cfg!(target_os = "macos") && path.is_dir()
                && path.extension().is_some_and(|extension| extension == "app"))
    })
}

pub(crate) fn inspect_agent_config_with_executable(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    executable_override: Option<&Path>,
) -> AgentConfigStatus {
    inspect_agent_config_with_executables(client, home, port, api_key, executable_override, None)
}

pub(crate) fn inspect_agent_config_with_executables(
    client: AgentClient,
    home: &Path,
    port: u16,
    api_key: &str,
    executable_override: Option<&Path>,
    desktop_override: Option<&Path>,
) -> AgentConfigStatus {
    let paths = agent_config_paths(client, home);
    let config_exists = paths.iter().any(|path| path.is_file());
    let result = config_paths(client.id(), home)
        .and_then(|mut paths| {
            if client == AgentClient::Codex && codex_native_oauth_enabled(home)? {
                paths.retain(|path| {
                    path.file_name().and_then(|name| name.to_str())
                        != Some(CODEX_MODEL_CATALOG_FILE)
                });
            }
            config_images(&paths)
        })
        .and_then(|images| validate_config_images(&images))
        .and_then(|_| inspect_agent_managed_config(client, &paths, port, api_key))
        .and_then(|(configured, model, oauth_configuration)| {
            if client == AgentClient::Codex && configured {
                let model = model
                    .as_deref()
                    .ok_or_else(|| "Codex configuration is missing a default model".to_string())?;
                validate_codex_catalog_file(&paths[0], model)?;
            }
            Ok((configured, model, oauth_configuration))
        });
    let (configured, current_model, oauth_configuration, config_valid, error) = match result {
        Ok((configured, model, oauth_configuration)) => {
            (configured, model, oauth_configuration, true, None)
        }
        Err(_) => (
            false,
            None,
            false,
            false,
            Some(
                "Failed to read or parse configuration. Check file permissions, restore from a manual backup, or repair it with the base configuration template"
                    .to_string(),
            ),
        ),
    };
    let executable = executable_override.filter(|path| path.is_file()
        || (matches!(client, AgentClient::ClaudeDesktop | AgentClient::ZCode | AgentClient::WorkBuddy)
            && manual_desktop_path(Some(path)).is_some())).map(Path::to_path_buf)
        .or_else(|| find_agent_executable(client, home));
    let cli_version = if client == AgentClient::ZCode {
        find_named_agent_executable(home, &["zcode"])
            .filter(|path| executable.as_ref() != Some(path))
            .as_deref()
            .and_then(|path| read_agent_version(path, home))
    } else if should_probe_primary_agent_executable_version(client) {
        executable
            .as_deref()
            .and_then(|path| read_agent_version(path, home))
    } else {
        None
    };
    let codex_app_installation = (client == AgentClient::Codex)
        .then(|| manual_desktop_path(desktop_override)
            .map(|path| DesktopAppTarget::Application(path.to_path_buf()))
            .or_else(|| find_codex_app_installation(home)))
        .flatten();
    let opencode_desktop_application = (client == AgentClient::OpenCode)
        .then(|| manual_desktop_path(desktop_override).map(Path::to_path_buf)
            .or_else(|| find_opencode_desktop_application(home)))
        .flatten();
    let harness_desktop_application = (client == AgentClient::DeepSeekHarness)
        .then(|| manual_desktop_path(desktop_override).map(Path::to_path_buf)
            .or_else(|| find_deepseek_harness_desktop_application(home)))
        .flatten();
    let app_version = match client {
        AgentClient::ClaudeDesktop => read_claude_desktop_version(home),
        AgentClient::Codex => codex_app_installation
            .as_ref()
            .and_then(|installation| read_codex_app_installation_version(installation, home)),
        AgentClient::OpenCode => opencode_desktop_application
            .as_deref()
            .and_then(read_opencode_desktop_version),
        AgentClient::DeepSeekHarness => harness_desktop_application
            .as_deref()
            .and_then(read_deepseek_harness_desktop_version),
        AgentClient::ZCode => executable.as_deref().and_then(read_zcode_app_version),
        AgentClient::WorkBuddy => executable.as_deref().and_then(read_workbuddy_app_version),
        _ => None,
    };
    let version = cli_version.clone().or_else(|| app_version.clone()).or_else(|| {
        (client == AgentClient::DeepSeekHarness)
            .then(|| read_deepseek_harness_profile_version(home))
            .flatten()
    });
    let app_installed = codex_app_installation.is_some()
        || opencode_desktop_application.is_some()
        || harness_desktop_application.is_some();
    let installed = agent_installation_detected(
        client,
        version.as_deref(),
        executable.is_some(),
        app_installed,
    );
    let launch_targets = agent_launch_targets(
        client,
        executable.as_deref(),
        app_version.as_deref(),
        app_installed,
    );
    let mut warnings = Vec::new();
    if !client.supported_platform() {
        warnings.push("The current platform does not support Claude Desktop 3P configuration".to_string());
    } else if !installed && config_exists {
        warnings.push(
            "Only a configuration file was detected. You can still edit the configuration and connect it to CPA, but launch functionality is unavailable".to_string(),
        );
    }
    if let Some(message) = error.as_ref() {
        warnings.push(message.clone());
    }

    let configuration_synchronized = agent_configuration_is_synchronized(client, home, configured);

    AgentConfigStatus {
        id: client.id().to_string(),
        name: client.name().to_string(),
        supported_platform: client.supported_platform(),
        installed,
        plugin_installed: true,
        launch_targets,
        version,
        cli_version,
        app_version,
        plugin_version: None,
        config_paths: paths.iter().map(|path| path_to_string(path)).collect(),
        config_exists,
        config_valid,
        configured,
        configuration_synchronized,
        connection_state: agent_connection_state(
            configured,
            config_valid,
            agent_has_connection_evidence(client, &paths),
        )
        .into(),
        current_model: current_model.clone(),
        oauth_configuration,
        codex_native_oauth: client == AgentClient::Codex
            && codex_native_oauth_enabled(home).unwrap_or(false),
        modification_enabled: configured,
        modification_state: if !config_valid {
            "invalid"
        } else if configured {
            "applied"
        } else {
            "unconfigured"
        }
        .into(),
        backup_available: false,
        applied_model: current_model,
        claude_code_model_mappings: (client == AgentClient::ClaudeCode)
            .then(|| inspect_claude_code_model_mappings(&paths[0]).ok().flatten())
            .flatten(),
        claude_desktop_model_mappings: (client == AgentClient::ClaudeDesktop)
            .then(|| current_desktop_mappings(home))
            .flatten(),
        warnings,
        error,
    }
}

pub(crate) fn agent_installation_detected(
    client: AgentClient,
    version: Option<&str>,
    executable_found: bool,
    app_installed: bool,
) -> bool {
    executable_found
        || app_installed
        || (client == AgentClient::ClaudeDesktop && version.is_some())
}

pub(crate) fn should_probe_primary_agent_executable_version(client: AgentClient) -> bool {
    !matches!(client, AgentClient::ClaudeDesktop | AgentClient::ZCode | AgentClient::WorkBuddy)
}

pub(crate) fn agent_launch_targets(
    client: AgentClient,
    executable: Option<&Path>,
    app_version: Option<&str>,
    app_installed: bool,
) -> Vec<AgentLaunchTarget> {
    let mut targets = Vec::new();
    match client {
        AgentClient::ClaudeDesktop => {
            if executable.is_some() || app_version.is_some() {
                targets.push(AgentLaunchTarget {
                    id: "app".to_string(),
                    label: "Claude Desktop".to_string(),
                    detail: executable
                        .map(path_to_string)
                        .unwrap_or_else(|| "Claude Desktop application".to_string()),
                });
            }
        }
        AgentClient::Codex => {
            if app_installed {
                targets.push(AgentLaunchTarget {
                    id: "app".to_string(),
                    label: "Codex App".to_string(),
                    detail: "Codex desktop application".to_string(),
                });
            }
            if let Some(executable) = executable {
                targets.push(AgentLaunchTarget {
                    id: "cli".to_string(),
                    label: "Codex CLI".to_string(),
                    detail: path_to_string(executable),
                });
            }
        }
        AgentClient::OpenCode => {
            if let Some(executable) = executable {
                targets.push(AgentLaunchTarget {
                    id: "cli".to_string(),
                    label: "OpenCode CLI".to_string(),
                    detail: path_to_string(executable),
                });
            }
            if app_installed {
                targets.push(AgentLaunchTarget {
                    id: "app".to_string(),
                    label: "OpenCode Desktop".to_string(),
                    detail: "OpenCode Desktop".to_string(),
                });
            }
        }
        AgentClient::ZCode | AgentClient::WorkBuddy => {
            if let Some(executable) = executable {
                targets.push(AgentLaunchTarget {
                    id: "app".to_string(),
                    label: client.name().to_string(),
                    detail: path_to_string(executable),
                });
            }
        }
        AgentClient::DeepSeekHarness => {
            if let Some(executable) = executable {
                targets.push(AgentLaunchTarget {
                    id: "cli".to_string(),
                    label: "DeepSeek Harness Web".to_string(),
                    detail: format!("{} web", path_to_string(executable)),
                });
            }
            if app_installed {
                targets.push(AgentLaunchTarget {
                    id: "app".to_string(),
                    label: "DeepSeek Harness Desktop".to_string(),
                    detail: "DeepSeek Harness Desktop".to_string(),
                });
            }
        }
        _ => {
            if let Some(executable) = executable {
                targets.push(AgentLaunchTarget {
                    id: "cli".to_string(),
                    label: client.name().to_string(),
                    detail: path_to_string(executable),
                });
            }
        }
    }
    targets
}

pub(crate) fn agent_configuration_is_synchronized(
    _client: AgentClient,
    _home: &Path,
    configured: bool,
) -> bool {
    configured
}

#[cfg(test)]
pub(crate) fn inspect_agent_application(
    client: AgentClient,
    home: &Path,
) -> AgentModificationInspection {
    match load_agent_applied_state(client, home) {
        Ok(Some(state)) => {
            let paths = agent_config_paths(client, home);
            let backup_available = !state.backup_files.is_empty()
                && state
                    .backup_files
                    .iter()
                    .all(|file| !file.existed_before || file.backup_path.is_file());
            if state.backup_files.is_empty() {
                match agent_has_managed_marker(client, &paths) {
                    Ok(false) => {
                        return AgentModificationInspection {
                            enabled: false,
                            state: "unconfigured".to_string(),
                            backup_available: false,
                            warnings: Vec::new(),
                        };
                    }
                    Err(error) => {
                        return AgentModificationInspection {
                            enabled: false,
                            state: "invalid".to_string(),
                            backup_available: false,
                            warnings: vec![error],
                        };
                    }
                    Ok(true) => {}
                }
            }
            let mut warnings = Vec::new();
            if state.backup_files.is_empty() {
                warnings.push("Legacy application state detected; only CPA-managed configuration fields will be removed when disabling".to_string());
            } else if !backup_available {
                warnings.push("The original configuration session backup is incomplete and cannot be safely restored yet".to_string());
            }
            AgentModificationInspection {
                enabled: true,
                state: "applied".to_string(),
                backup_available,
                warnings,
            }
        }
        Ok(None) => AgentModificationInspection {
            enabled: false,
            state: "unconfigured".to_string(),
            backup_available: false,
            warnings: Vec::new(),
        },
        Err(error) => AgentModificationInspection {
            enabled: false,
            state: "invalid".to_string(),
            backup_available: false,
            warnings: vec![error],
        },
    }
}

pub(crate) fn inspect_agent_managed_config(
    client: AgentClient,
    paths: &[PathBuf],
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>, bool), String> {
    match client {
        AgentClient::ClaudeCode => inspect_claude_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::ClaudeDesktop if client.supported_platform() => {
            inspect_claude_desktop_agent_config(paths, port, api_key)
                .map(|(configured, model)| (configured, model, false))
        }
        AgentClient::ClaudeDesktop => Ok((false, None, false)),
        AgentClient::Codex => inspect_codex_agent_config(&paths[0], port, api_key),
        AgentClient::OpenCode => inspect_opencode_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::OpenClaw => inspect_openclaw_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::Hermes => inspect_hermes_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::DeepSeekHarness => inspect_deepseek_harness_config(paths, port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::ZCode => {
            if paths.len() != 1 {
                return Err("Invalid number of ZCode configuration paths".to_string());
            }
            inspect_zcode_agent_config(&paths[0], port, api_key)
                .map(|(configured, model)| (configured, model, false))
        }
        AgentClient::AntigravityCli => inspect_antigravity_config(client, paths, port, api_key).map(|(configured, model)| (configured, model, false)),
        AgentClient::WorkBuddy => inspect_workbuddy_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::KimiCode => inspect_kimi_code_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::GrokBuild => inspect_grok_build_agent_config(&paths[0], port, api_key)
            .map(|(configured, model)| (configured, model, false)),
        AgentClient::Omp => inspect_omp_agent_config(paths, port, api_key)
            .map(|(configured, model)| (configured, model, false)),
    }
}

pub(crate) fn agent_connection_state(
    configured: bool,
    valid: bool,
    managed: Result<bool, String>,
) -> &'static str {
    if !valid || managed.is_err() {
        "invalid"
    } else if configured {
        "configured"
    } else if managed == Ok(true) {
        "needs-update"
    } else {
        "not-configured"
    }
}

pub(crate) fn agent_has_connection_evidence(
    client: AgentClient,
    paths: &[PathBuf],
) -> Result<bool, String> {
    let active_marker = agent_has_managed_marker(client, paths)?;
    if active_marker {
        return Ok(true);
    }
    for path in paths {
        let Some(bytes) = read_agent_bytes(path)? else {
            continue;
        };
        let content = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let value: serde_json::Value = match path.extension().and_then(|v| v.to_str()) {
            Some("toml") => serde_json::to_value(
                toml::from_str::<toml::Value>(content).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
            Some("yaml" | "yml") => serde_yaml::from_str(content).map_err(|e| e.to_string())?,
            _ => json5::from_str(content).map_err(|e| e.to_string())?,
        };
        let provider_present = client != AgentClient::Codex
            && [
                "/provider/cpa-gui",
                "/model_providers/cpa-gui",
                "/providers/cpa-gui",
                "/models/providers/cpa-gui",
            ]
            .iter()
            .any(|pointer| value.pointer(pointer).is_some());
        let selected = [
            "/model_provider",
            "/model",
            "/model/main",
            "/model/provider",
            "/default_model",
            "/models/default",
            "/agents/defaults/model/primary",
        ]
        .iter()
        .filter_map(|pointer| value.pointer(pointer).and_then(serde_json::Value::as_str))
        .any(|model| {
            model == MANAGED_AGENT_PROVIDER_ID
                || model.starts_with(&format!("{MANAGED_AGENT_PROVIDER_ID}/"))
        });
        let catalog = client == AgentClient::Codex
            && value
                .get("model_catalog_json")
                .and_then(serde_json::Value::as_str)
                == Some(CODEX_MODEL_CATALOG_FILE);
        let hermes_provider = client == AgentClient::Hermes
            && value
                .get("custom_providers")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("name").and_then(serde_json::Value::as_str)
                            == Some(MANAGED_AGENT_PROVIDER_ID)
                    })
                });
        if provider_present || selected || catalog || hermes_provider {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn agent_has_managed_marker(
    client: AgentClient,
    paths: &[PathBuf],
) -> Result<bool, String> {
    match client {
        AgentClient::ClaudeCode => {
            if !paths[0].is_file() {
                return Ok(false);
            }
            let root = read_agent_json_or_empty(&paths[0], "Claude Code configuration")?;
            let env = root.get("env");
            Ok(env
                .and_then(|value| value.get("ANTHROPIC_BASE_URL"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_managed_agent_base_url))
        }
        AgentClient::ClaudeDesktop => {
            if paths.len() != 4 {
                return Ok(false);
            }
            let meta = read_agent_json_or_empty(&paths[3], "Claude Desktop configuration index")?;
            let profile = read_agent_json_or_empty(&paths[2], "Claude Desktop gateway configuration")?;
            Ok(meta.get("appliedId").and_then(serde_json::Value::as_str)
                == Some(CLAUDE_DESKTOP_PROFILE_ID)
                || meta
                    .get("entries")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.get("id").and_then(serde_json::Value::as_str)
                                == Some(CLAUDE_DESKTOP_PROFILE_ID)
                        })
                    })
                || profile
                    .get("inferenceGatewayBaseUrl")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_managed_agent_base_url))
        }
        AgentClient::Codex => {
            if !paths[0].is_file() {
                return Ok(false);
            }
            let root: toml::Value = toml::from_str(
                &fs::read_to_string(&paths[0])
                    .map_err(|error| format!("Failed to read Codex configuration: {error}"))?,
            )
            .map_err(|error| format!("Failed to parse Codex configuration: {error}"))?;
            Ok(root.get("model_provider").and_then(toml::Value::as_str)
                == Some(MANAGED_AGENT_PROVIDER_ID))
        }
        AgentClient::OpenCode => {
            if !paths[0].is_file() {
                return Ok(false);
            }
            let root = read_agent_json5_or_empty(&paths[0], "OpenCode configuration")?;
            let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
            let provider_exists = root
                .get("provider")
                .and_then(|value| value.get(MANAGED_AGENT_PROVIDER_ID))
                .is_some();
            let model_selected = root
                .get("model")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|model| model.starts_with(&prefix));
            Ok(provider_exists && model_selected)
        }
        AgentClient::OpenClaw => {
            if !paths[0].is_file() {
                return Ok(false);
            }
            let root: serde_json::Value = json5::from_str(
                &fs::read_to_string(&paths[0])
                    .map_err(|error| format!("Failed to read OpenClaw configuration: {error}"))?,
            )
            .map_err(|error| format!("Failed to parse OpenClaw configuration: {error}"))?;
            let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
            let provider_exists = root
                .get("models")
                .and_then(|value| value.get("providers"))
                .and_then(|value| value.get(MANAGED_AGENT_PROVIDER_ID))
                .is_some();
            let model_selected = root
                .get("agents")
                .and_then(|value| value.get("defaults"))
                .and_then(|value| value.get("model"))
                .and_then(|value| value.get("primary"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|model| model.starts_with(&prefix));
            Ok(provider_exists && model_selected)
        }
        AgentClient::Hermes => {
            if !paths[0].is_file() {
                return Ok(false);
            }
            let root: serde_yaml::Value = serde_yaml::from_str(
                &fs::read_to_string(&paths[0])
                    .map_err(|error| format!("Failed to read Hermes configuration: {error}"))?,
            )
            .map_err(|error| format!("Failed to parse Hermes configuration: {error}"))?;
            let provider_exists = hermes_managed_provider(&root).is_some();
            let model_selected = root
                .get("model")
                .and_then(|value| value.get("provider"))
                .and_then(serde_yaml::Value::as_str)
                == Some(MANAGED_AGENT_PROVIDER_ID);
            Ok(provider_exists && model_selected)
        }
        AgentClient::DeepSeekHarness => deepseek_harness_has_managed_marker(paths),
        AgentClient::WorkBuddy => workbuddy_has_managed_marker(&paths[0]),
        AgentClient::AntigravityCli => antigravity_has_marker(client, paths),
        AgentClient::ZCode => {
            let Some(path) = paths.first() else { return Ok(false); };
            if !path.is_file() { return Ok(false); }
            let root = read_agent_json_or_empty(path, "ZCode provider configuration")?;
            let provider_exists = root.pointer("/config/providerConfigRules/providerRules")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|rules| rules.iter().any(|rule| {
                    rule.get("providerId").and_then(serde_json::Value::as_str)
                        == Some(MANAGED_AGENT_PROVIDER_ID)
                }));
            let model_selected = root.pointer("/config/defaultModelSelection/providerId")
                .and_then(serde_json::Value::as_str)
                == Some(MANAGED_AGENT_PROVIDER_ID);
            Ok(provider_exists && model_selected)
        }
        AgentClient::KimiCode => inspect_managed_toml_model_marker(
            &paths[0],
            "Kimi Code configuration",
            Some("providers"),
            "models",
            None,
            "default_model",
        ),
        AgentClient::GrokBuild => inspect_managed_toml_model_marker(
            &paths[0],
            "Grok Build configuration",
            None,
            "model",
            Some("models"),
            "default",
        ),
        AgentClient::Omp => omp_has_managed_marker(paths),
    }
}

pub(crate) fn is_managed_agent_base_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    if value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("https://127.0.0.1:")
        || value.starts_with("https://localhost:")
    {
        return true;
    }
    #[cfg(not(test))]
    {
        let Ok(settings) = read_installed_core_config_settings() else {
            return false;
        };
        let host = core_connect_host(&settings.host);
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        value.starts_with(&format!("http://{host}:"))
            || value.starts_with(&format!("https://{host}:"))
    }
    #[cfg(test)]
    {
        false
    }
}

pub(crate) fn inspect_managed_toml_model_marker(
    path: &Path,
    label: &str,
    provider_section: Option<&str>,
    model_entry_section: &str,
    default_section: Option<&str>,
    default_key: &str,
) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read {label}: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse {label}: {error}"))?;
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let selected = (if let Some(section) = default_section {
        root.get(section)
            .and_then(toml::Value::as_table)
            .and_then(|section| section.get(default_key))
    } else {
        root.get(default_key)
    })
    .and_then(toml::Value::as_str)
    .filter(|model| model.starts_with(&prefix));
    let catalog_has_selected = selected.is_some_and(|model| {
        root.get(model_entry_section)
            .and_then(toml::Value::as_table)
            .is_some_and(|catalog| catalog.contains_key(model))
    });
    let provider_exists = provider_section.is_none_or(|section| {
        root.get(section)
            .and_then(toml::Value::as_table)
            .is_some_and(|providers| providers.contains_key(MANAGED_AGENT_PROVIDER_ID))
    });
    Ok(provider_exists && catalog_has_selected)
}

pub(crate) fn find_codex_app_installation(home: &Path) -> Option<DesktopAppTarget> {
    #[cfg(target_os = "macos")]
    {
        [PathBuf::from("/Applications"), home.join("Applications")]
            .into_iter()
            .flat_map(|directory| {
                [
                    "ChatGPT.app",
                    "Codex.app",
                    "OpenAI Codex.app",
                    "OpenAI.Codex.app",
                ]
                .into_iter()
                .map(move |name| directory.join(name))
            })
            .find(|path| path.is_dir())
            .map(DesktopAppTarget::Application)
    }

    #[cfg(target_os = "windows")]
    {
        find_windows_codex_app_installation(home)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = home;
        None
    }
}

pub(crate) fn find_opencode_desktop_application(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            PathBuf::from("/Applications/OpenCode.app"),
            home.join("Applications/OpenCode.app"),
        ]
        .into_iter()
        .find(|path| path.is_dir())
    }

    #[cfg(target_os = "windows")]
    {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| home.join("AppData/Local"));
        let program_files_roots = ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<Vec<_>>();
        find_windows_opencode_desktop_application_in_roots(&local, &program_files_roots)
    }

    #[cfg(target_os = "linux")]
    {
        let mut executable_directories = agent_executable_directories(home);
        for directory in ["/opt/OpenCode", "/opt/opencode-desktop"] {
            let directory = PathBuf::from(directory);
            if !executable_directories.contains(&directory) {
                executable_directories.push(directory);
            }
        }

        let data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        let mut data_directories = vec![data_home];
        let system_data_directories = env::var_os("XDG_DATA_DIRS")
            .map(|value| env::split_paths(&value).collect::<Vec<_>>())
            .filter(|directories| !directories.is_empty())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            });
        for directory in system_data_directories {
            if directory.is_absolute() && !data_directories.contains(&directory) {
                data_directories.push(directory);
            }
        }

        let appimage_directories = [
            home.join("Applications"),
            home.join(".local/Applications"),
            home.join("Downloads"),
            home.join("Desktop"),
        ];
        find_linux_opencode_desktop_application_in_roots(
            home,
            &executable_directories,
            &data_directories,
            &appimage_directories,
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = home;
        None
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn find_windows_opencode_desktop_application_in_roots(
    local_app_data: &Path,
    program_files_roots: &[PathBuf],
) -> Option<PathBuf> {
    let mut candidates = vec![
        local_app_data.join("Programs/@opencode-aidesktop/OpenCode.exe"),
        local_app_data.join("Programs/OpenCode/OpenCode.exe"),
        local_app_data.join("Programs/opencode/OpenCode.exe"),
        local_app_data.join("OpenCode/OpenCode.exe"),
        local_app_data.join("opencode/OpenCode.exe"),
    ];
    for root in program_files_roots {
        candidates.extend([
            root.join("@opencode-aidesktop/OpenCode.exe"),
            root.join("OpenCode/OpenCode.exe"),
            root.join("opencode/OpenCode.exe"),
        ]);
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn find_linux_opencode_desktop_application_in_roots(
    home: &Path,
    executable_directories: &[PathBuf],
    data_directories: &[PathBuf],
    appimage_directories: &[PathBuf],
) -> Option<PathBuf> {
    const EXECUTABLE_NAMES: &[&str] = &[
        "ai.opencode.desktop",
        "ai.opencode.desktop.beta",
        "ai.opencode.desktop.dev",
        "opencode-desktop",
        "@opencode-aidesktop",
        "OpenCode",
        "OpenCode Desktop",
    ];

    for directory in executable_directories {
        for name in EXECUTABLE_NAMES {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    find_linux_opencode_application_from_desktop_entries(
        home,
        executable_directories,
        data_directories,
    )
    .or_else(|| {
        appimage_directories.iter().find_map(|directory| {
            let mut candidates = fs::read_dir(directory)
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file() && is_opencode_desktop_appimage(path))
                .collect::<Vec<_>>();
            candidates.sort();
            candidates.into_iter().next()
        })
    })
}

#[cfg(any(target_os = "linux", test))]
fn find_linux_opencode_application_from_desktop_entries(
    home: &Path,
    executable_directories: &[PathBuf],
    data_directories: &[PathBuf],
) -> Option<PathBuf> {
    for data_directory in data_directories {
        let applications = data_directory.join("applications");
        let mut entries = fs::read_dir(applications)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
            })
            .collect::<Vec<_>>();
        entries.sort();

        for entry in entries {
            let Ok(content) = fs::read_to_string(&entry) else {
                continue;
            };
            if !is_opencode_desktop_entry(&entry, &content) {
                continue;
            }
            let Some(command) = linux_desktop_entry_command(&content) else {
                continue;
            };
            if let Some(application) =
                resolve_linux_desktop_command(home, executable_directories, command.as_str())
            {
                return Some(application);
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn is_opencode_desktop_entry(path: &Path, content: &str) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.contains("opencode") {
        return true;
    }

    let mut in_desktop_entry = false;
    content.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            return false;
        }
        if !in_desktop_entry {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let value = value.trim().to_ascii_lowercase();
        match key.trim() {
            "Name" => {
                value == "opencode"
                    || value.starts_with("opencode beta")
                    || value.starts_with("opencode dev")
            }
            "Icon" | "StartupWMClass" => {
                value.starts_with("ai.opencode.desktop")
                    || value == "@opencode-aidesktop"
                    || value == "opencode"
            }
            _ => false,
        }
    })
}

#[cfg(any(target_os = "linux", test))]
fn linux_desktop_entry_command(content: &str) -> Option<String> {
    let mut in_desktop_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("Exec") {
            continue;
        }
        let tokens = tokenize_linux_desktop_exec(value.trim());
        let mut index = 0;
        if tokens.first().is_some_and(|token| {
            Path::new(token)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "env")
        }) {
            index += 1;
            while tokens.get(index).is_some_and(|token| token.contains('=')) {
                index += 1;
            }
        }
        let command = tokens.get(index)?.trim();
        if command.is_empty()
            || command.starts_with('%')
            || ["flatpak", "snap", "gtk-launch", "sh", "bash"]
                .iter()
                .any(|wrapper| {
                    Path::new(command)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name == *wrapper)
                })
        {
            return None;
        }
        return Some(command.to_string());
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn tokenize_linux_desktop_exec(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '"' || character == '\'' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(any(target_os = "linux", test))]
fn resolve_linux_desktop_command(
    home: &Path,
    executable_directories: &[PathBuf],
    command: &str,
) -> Option<PathBuf> {
    let candidate = if let Some(relative) = command.strip_prefix("~/") {
        home.join(relative)
    } else {
        PathBuf::from(command)
    };
    if candidate.is_absolute() || command.contains('/') {
        return candidate.is_file().then_some(candidate);
    }
    executable_directories
        .iter()
        .map(|directory| directory.join(&candidate))
        .find(|path| path.is_file())
}

#[cfg(any(target_os = "linux", test))]
fn is_opencode_desktop_appimage(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    file_name.ends_with(".appimage")
        && (file_name.starts_with("opencode-desktop") || file_name.starts_with("opencode_"))
}

pub(crate) fn read_opencode_desktop_version(application: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        read_windows_executable_version(application)
    }

    #[cfg(target_os = "macos")]
    {
        read_macos_app_version(application)
    }

    #[cfg(target_os = "linux")]
    {
        read_linux_opencode_desktop_version(application)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = application;
        None
    }
}

#[cfg(target_os = "linux")]
fn read_linux_opencode_desktop_version(application: &Path) -> Option<String> {
    let path_version = linux_opencode_desktop_version_from_path(application);
    if application
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("appimage"))
    {
        path_version
    } else {
        path_version.or_else(read_linux_opencode_package_version)
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_opencode_desktop_version_from_path(application: &Path) -> Option<String> {
    application
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(|component| {
            let lower = component.to_ascii_lowercase();
            if !lower.contains("opencode") {
                return None;
            }
            let component = lower
                .strip_suffix(".appimage")
                .map(|_| &component[..component.len() - ".appimage".len()])
                .unwrap_or(component);
            component.split(['-', '_']).find_map(|token| {
                let version = token.strip_prefix('v').unwrap_or(token);
                (version.starts_with(|character: char| character.is_ascii_digit())
                    && version.contains('.')
                    && version.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '.' | '+' | '~')
                    }))
                .then(|| version.to_string())
            })
        })
}

#[cfg(target_os = "linux")]
fn read_linux_opencode_package_version() -> Option<String> {
    const PACKAGE_NAMES: &[&str] = &[
        "opencode",
        "opencode-desktop",
        "opencode-ai-desktop",
        "opencode-aidesktop",
    ];

    let mut dpkg = Command::new("dpkg-query");
    dpkg.args(["--show", "--showformat=${Package}\\t${Version}\\n"])
        .args(PACKAGE_NAMES);
    if let Some(version) = linux_package_version_from_output(&mut dpkg, '\t') {
        return Some(version);
    }

    let mut rpm = Command::new("rpm");
    rpm.args(["--query", "--queryformat", "%{NAME}\\t%{VERSION}\\n"])
        .args(PACKAGE_NAMES);
    if let Some(version) = linux_package_version_from_output(&mut rpm, '\t') {
        return Some(version);
    }

    let mut pacman = Command::new("pacman");
    pacman.arg("--query").args(PACKAGE_NAMES);
    linux_package_version_from_output(&mut pacman, ' ')
}

#[cfg(target_os = "linux")]
fn linux_package_version_from_output(command: &mut Command, separator: char) -> Option<String> {
    configure_background_command(command);
    let output = command_output_with_timeout(command, AGENT_VERSION_PROBE_TIMEOUT).ok()??;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let (package, version) = line.trim().split_once(separator)?;
            let package = package.trim();
            let version = version.trim();
            ([
                "opencode",
                "opencode-desktop",
                "opencode-ai-desktop",
                "opencode-aidesktop",
            ]
            .contains(&package))
            .then(|| normalize_detected_agent_version(version))
            .flatten()
        })
}

pub(crate) fn read_claude_desktop_version(home: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        read_windows_claude_desktop_store_version().or_else(|| {
            find_claude_desktop_executable(home)
                .and_then(|path| read_windows_executable_version(&path))
        })
    }
    #[cfg(target_os = "macos")]
    {
        find_claude_desktop_executable(home).and_then(|path| {
            let application = path.ancestors().find(|candidate| {
                candidate.extension().and_then(|value| value.to_str()) == Some("app")
            })?;
            read_macos_app_version(application)
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = home;
        None
    }
}

pub(crate) fn read_codex_app_installation_version(
    installation: &DesktopAppTarget,
    _home: &Path,
) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        match installation {
            DesktopAppTarget::WindowsAppId(app_id) => read_windows_codex_store_version(app_id),
            DesktopAppTarget::Application(path) => read_windows_codex_desktop_version(path),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let DesktopAppTarget::Application(path) = installation;
        read_macos_app_version(path)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = installation;
        None
    }
}

pub(crate) fn read_zcode_app_version(executable: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        read_windows_executable_version(executable)
    }
    #[cfg(target_os = "macos")]
    {
        let application = executable
            .ancestors()
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))?;
        read_macos_app_version(application)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = executable;
        None
    }
}

pub(crate) fn find_zcode_desktop_executable(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| home.join("AppData/Local"));
        let mut candidates = vec![
            local.join("Programs/ZCode/ZCode.exe"),
            local.join("ZCode/ZCode.exe"),
        ];
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty())
            {
                candidates.push(root.join("ZCode/ZCode.exe"));
            }
        }
        candidates
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| find_named_agent_executable(home, &["zcode"]))
            .or_else(find_windows_registered_zcode_executable)
    }
    #[cfg(target_os = "macos")]
    {
        [
            PathBuf::from("/Applications/ZCode.app/Contents/MacOS/ZCode"),
            home.join("Applications/ZCode.app/Contents/MacOS/ZCode"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| find_named_agent_executable(home, &["zcode"]))
    }
    #[cfg(target_os = "linux")]
    {
        let mut candidates = agent_executable_directories(home)
            .into_iter()
            .flat_map(|directory| [directory.join("zcode"), directory.join("ZCode")])
            .collect::<Vec<_>>();
        candidates.extend([
            PathBuf::from("/opt/ZCode/zcode"),
            PathBuf::from("/opt/ZCode/ZCode"),
            PathBuf::from("/opt/zcode/zcode"),
        ]);
        candidates.into_iter().find(|path| path.is_file())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        find_named_agent_executable(home, &["zcode"])
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn read_macos_app_version(application: &Path) -> Option<String> {
    let info_plist = application.join("Contents/Info.plist");
    for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
        let mut command = Command::new("/usr/bin/plutil");
        command
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(&info_plist);
        let output =
            command_output_with_timeout(&mut command, AGENT_VERSION_PROBE_TIMEOUT).ok()??;
        if !output.status.success() {
            continue;
        }
        if let Some(version) =
            normalize_detected_agent_version(&String::from_utf8_lossy(&output.stdout))
        {
            return Some(version);
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_system_root() -> PathBuf {
    env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_powershell_executable() -> PathBuf {
    let executable = windows_system_root().join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if executable.is_file() {
        executable
    } else {
        PathBuf::from("powershell.exe")
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_explorer_executable() -> PathBuf {
    let executable = windows_system_root().join("explorer.exe");
    if executable.is_file() {
        executable
    } else {
        PathBuf::from("explorer.exe")
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_command_processor() -> PathBuf {
    env::var_os("ComSpec")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| windows_system_root().join("System32/cmd.exe"))
}

#[cfg(target_os = "windows")]
pub(crate) fn find_windows_codex_app_installation(home: &Path) -> Option<DesktopAppTarget> {
    find_windows_codex_app_executable(home)
        .map(DesktopAppTarget::Application)
        .or_else(find_windows_registered_codex_app_installation)
        .or_else(|| find_windows_codex_app_id_via_registry().map(DesktopAppTarget::WindowsAppId))
}

#[cfg(target_os = "windows")]
pub(crate) fn read_windows_codex_store_version(app_id: &str) -> Option<String> {
    find_windows_codex_store_executable(app_id)
        .and_then(|path| read_windows_codex_desktop_version(&path))
}

#[cfg(target_os = "windows")]
pub(crate) fn read_windows_codex_desktop_version(executable: &Path) -> Option<String> {
    let resources = executable.parent()?.join("resources");
    let owl_ini = resources.join("owl-app.ini");
    let asar = resources.join("app.asar");
    fs::read_to_string(&owl_ini)
        .ok()
        .and_then(|content| parse_codex_owl_app_version(&content))
        .or_else(|| read_codex_asar_version(&asar))
        .or_else(|| {
            if owl_ini.exists() || asar.exists() || resources.join("owl-electron-app.json").exists()
            {
                None
            } else {
                read_windows_executable_version(executable)
            }
        })
}

#[cfg(target_os = "windows")]
pub(crate) fn parse_codex_owl_app_version(content: &str) -> Option<String> {
    let mut in_owl_section = false;
    for line in content
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
    {
        if line.starts_with('[') && line.ends_with(']') {
            in_owl_section = line.eq_ignore_ascii_case("[Owl]");
        } else if in_owl_section {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("AppVersion") {
                    return normalize_detected_agent_version(value);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub(crate) fn read_codex_asar_version(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = fs::File::open(path).ok()?;
    let mut prefix = [0_u8; 16];
    file.read_exact(&mut prefix).ok()?;
    let word = |offset| u32::from_le_bytes(prefix[offset..offset + 4].try_into().unwrap()) as u64;
    let header_size = word(4);
    let json_size = word(12);
    if word(0) != 4
        || !(8..=16 * 1024 * 1024).contains(&header_size)
        || word(8) != header_size - 4
        || json_size > header_size - 8
    {
        return None;
    }
    let mut header = vec![0; json_size as usize];
    file.read_exact(&mut header).ok()?;
    let header: serde_json::Value = serde_json::from_slice(&header).ok()?;
    let entry = header.get("files")?.get("package.json")?;
    if entry.get("unpacked").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }
    let size = entry.get("size")?.as_u64()?;
    if size > 1024 * 1024 {
        return None;
    }
    let offset = entry.get("offset")?.as_str()?.parse::<u64>().ok()?;
    let position = (8 + header_size).checked_add(offset)?;
    if position.checked_add(size)? > file.metadata().ok()?.len() {
        return None;
    }
    file.seek(SeekFrom::Start(position)).ok()?;
    let mut package = vec![0; size as usize];
    file.read_exact(&mut package).ok()?;
    let package: serde_json::Value = serde_json::from_slice(&package).ok()?;
    normalize_detected_agent_version(package.get("version")?.as_str()?)
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_powershell_single_quoted_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(target_os = "windows")]
pub(crate) fn find_windows_codex_app_executable(home: &Path) -> Option<PathBuf> {
    let local = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    let mut candidates = vec![
        local.join("Programs/OpenAI/ChatGPT/ChatGPT.exe"),
        local.join("Programs/ChatGPT/ChatGPT.exe"),
        local.join("OpenAI/ChatGPT/ChatGPT.exe"),
        local.join("OpenAI/Codex/Codex.exe"),
        local.join("Programs/OpenAI/Codex/Codex.exe"),
        local.join("Programs/Codex/Codex.exe"),
        local.join("Microsoft/WindowsApps/ChatGPT.exe"),
    ];
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        let Some(root) = env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
        else {
            continue;
        };
        candidates.extend([
            root.join("OpenAI/ChatGPT/ChatGPT.exe"),
            root.join("ChatGPT/ChatGPT.exe"),
            root.join("OpenAI/Codex/Codex.exe"),
            root.join("Codex/Codex.exe"),
        ]);
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub(crate) fn find_agent_executable(client: AgentClient, home: &Path) -> Option<PathBuf> {
    if client == AgentClient::AntigravityCli { return find_antigravity_cli(home); }
    if client == AgentClient::ClaudeDesktop {
        return find_claude_desktop_executable(home);
    }
    if client == AgentClient::ZCode {
        return find_zcode_desktop_executable(home);
    }
    if client == AgentClient::WorkBuddy {
        return find_workbuddy_desktop_executable(home);
    }
    if client == AgentClient::KimiCode {
        return find_kimi_code_executable(home);
    }
    if client == AgentClient::GrokBuild {
        return find_grok_build_executable(home);
    }
    if client == AgentClient::Omp {
        return find_omp_executable(home);
    }
    if client == AgentClient::OpenCode {
        return find_opencode_executable(home);
    }
    find_named_agent_executable(home, client.executable_names())
}

pub(crate) fn find_opencode_executable(home: &Path) -> Option<PathBuf> {
    find_named_agent_executable(home, &["opencode"])
        .or_else(|| find_opencode_managed_executable(home))
        .or_else(|| find_named_agent_executable(home, &["opencode-cli"]))
}

pub(crate) fn find_opencode_managed_executable(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            home.join(".opencode/bin/opencode"),
            PathBuf::from("/Applications/OpenCode.app/Contents/MacOS/opencode-cli"),
            home.join("Applications/OpenCode.app/Contents/MacOS/opencode-cli"),
        ]
        .into_iter()
        .find(|path| agent_cli_executable(path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        find_named_agent_executable_in_directories(&[home.join(".opencode/bin")], &["opencode"])
    }
}

pub(crate) fn find_kimi_code_executable(home: &Path) -> Option<PathBuf> {
    let directory = kimi_code_home(home).join("bin");
    #[cfg(target_os = "windows")]
    let managed = [directory.join("kimi.exe"), directory.join("kimi.cmd")];
    #[cfg(not(target_os = "windows"))]
    let managed = [directory.join("kimi")];
    managed
        .into_iter()
        .find(|path| agent_cli_executable(path))
        .or_else(|| find_named_agent_executable(home, &["kimi"]))
}

pub(crate) fn find_grok_build_executable(home: &Path) -> Option<PathBuf> {
    let directory = grok_build_home(home).join("bin");
    #[cfg(target_os = "windows")]
    let managed = [directory.join("grok.exe"), directory.join("grok.cmd")];
    #[cfg(not(target_os = "windows"))]
    let managed = [directory.join("grok")];
    managed
        .into_iter()
        .find(|path| agent_cli_executable(path))
        .or_else(|| find_named_agent_executable(home, &["grok"]))
}

pub(crate) fn find_pi_executable(home: &Path) -> Option<PathBuf> {
    find_named_agent_executable(home, &["pi"])
}

pub(crate) fn find_named_agent_executable(home: &Path, names: &[&str]) -> Option<PathBuf> {
    find_named_agent_executable_in_directories(&agent_executable_directories(home), names)
}

pub(crate) fn agent_cli_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(target_os = "windows")]
    {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["exe", "cmd", "bat", "com"]
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            })
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    true
}

pub(crate) fn find_named_agent_executable_in_directories(
    directories: &[PathBuf],
    names: &[&str],
) -> Option<PathBuf> {
    for directory in directories {
        for name in names {
            #[cfg(target_os = "windows")]
            let candidates = [
                directory.join(format!("{name}.exe")),
                directory.join(format!("{name}.cmd")),
                directory.join(format!("{name}.bat")),
                directory.join(format!("{name}.com")),
            ];
            #[cfg(not(target_os = "windows"))]
            let candidates = [directory.join(name)];
            if let Some(candidate) = candidates
                .into_iter()
                .find(|path| agent_cli_executable(path))
            {
                return Some(candidate);
            }
        }
    }
    None
}

pub(crate) fn agent_executable_directories(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    let registered = windows_registered_path_directories();
    #[cfg(not(target_os = "windows"))]
    let registered = Vec::new();
    agent_executable_directories_from_environment(home, |name| env::var_os(name), &registered)
}

pub(crate) fn agent_executable_directories_from_environment(
    home: &Path,
    environment: impl Fn(&str) -> Option<std::ffi::OsString>,
    registered: &[PathBuf],
) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut push = |path: PathBuf| {
        if !path.as_os_str().is_empty() && !directories.iter().any(|item| item == &path) {
            directories.push(path);
        }
    };

    if let Some(path) = environment("PATH") {
        env::split_paths(&path).for_each(&mut push);
    }
    registered.iter().cloned().for_each(&mut push);
    [
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".cargo/bin"),
        home.join(".volta/bin"),
        home.join(".local/share/pnpm"),
        home.join(".pyenv/shims"),
        home.join("bin"),
    ]
    .into_iter()
    .for_each(&mut push);
    for variable in [
        "PNPM_HOME",
        "BUN_INSTALL",
        "NPM_CONFIG_PREFIX",
        "npm_config_prefix",
        "VOLTA_HOME",
        "UV_TOOL_BIN_DIR",
        "NVM_SYMLINK",
    ] {
        if let Some(path) = environment(variable).filter(|path| !path.is_empty()) {
            let path = PathBuf::from(path);
            if matches!(variable, "NPM_CONFIG_PREFIX" | "npm_config_prefix") {
                push(if cfg!(target_os = "windows") {
                    path
                } else {
                    path.join("bin")
                });
            } else {
                push(if matches!(variable, "BUN_INSTALL" | "VOLTA_HOME") {
                    path.join("bin")
                } else {
                    path
                });
            }
        }
    }
    for root in [
        home.join(".nvm/versions/node"),
        home.join(".local/state/fnm_multishells"),
    ] {
        if let Ok(entries) = fs::read_dir(root) {
            let mut entries = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            entries.sort();
            for entry in entries.into_iter().rev() {
                push(entry.join("bin"));
            }
        }
    }

    #[cfg(unix)]
    for path in ["/usr/local/bin", "/usr/bin", "/bin"] {
        push(PathBuf::from(path));
    }

    #[cfg(target_os = "macos")]
    push(PathBuf::from("/opt/homebrew/bin"));

    #[cfg(target_os = "windows")]
    {
        let app_data = environment("APPDATA")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = environment("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        push(app_data.join("npm"));
        push(home.join(".npm-global"));
        push(local.join("Microsoft/WindowsApps"));
        push(local.join("pnpm"));
        push(local.join("Volta/bin"));
        for root in [local.join("Programs/Python"), local.join("Python")] {
            if let Ok(entries) = fs::read_dir(root) {
                let mut entries = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect::<Vec<_>>();
                entries.sort();
                for entry in entries.into_iter().rev() {
                    push(entry.join("Scripts"));
                }
            }
        }
    }

    directories
}

pub(crate) fn find_claude_desktop_executable(home: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            PathBuf::from("/Applications/Claude.app/Contents/MacOS/Claude"),
            home.join("Applications/Claude.app/Contents/MacOS/Claude"),
        ]
        .into_iter()
        .find(|path| path.is_file())
    }
    #[cfg(target_os = "windows")]
    {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        [
            local.join("Programs/Claude/Claude.exe"),
            local.join("Claude/Claude.exe"),
        ]
        .into_iter()
        .find(|path| path.is_file())
    }
    #[cfg(target_os = "linux")]
    {
        let mut candidates = agent_executable_directories(home)
            .into_iter()
            .map(|directory| directory.join("claude-desktop"))
            .collect::<Vec<_>>();
        candidates.extend([
            PathBuf::from("/opt/Claude/claude-desktop"),
            PathBuf::from("/opt/Claude/claude"),
            PathBuf::from("/opt/claude-desktop/claude-desktop"),
        ]);
        candidates.into_iter().find(|path| path.is_file())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = home;
        None
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_command_for_executable(executable: &Path, keep_shell_open: bool) -> Command {
    use std::os::windows::process::CommandExt;

    let is_batch_script = executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    if !is_batch_script {
        return Command::new(executable);
    }

    let mut command = Command::new(windows_command_processor());
    command.args(["/D", if keep_shell_open { "/K" } else { "/C" }, "call"]);
    command.raw_arg(windows_batch_executable_argument(executable));
    command
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_batch_executable_argument(executable: &Path) -> String {
    format!("\"{}\"", path_to_string(executable))
}

pub(crate) fn command_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<Option<std::process::Output>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Unable to read agent detection standard output"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("Unable to read agent detection error output"))?;
    thread::scope(|scope| {
        let stdout_reader = scope.spawn(move || {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).map(|_| output)
        });
        let stderr_reader = scope.spawn(move || {
            let mut output = Vec::new();
            stderr.read_to_end(&mut output).map(|_| output)
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                terminate_agent_probe_process(&mut child);
                break None;
            }
            thread::sleep(AGENT_VERSION_PROBE_POLL_INTERVAL);
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| io::Error::other("Agent detection output thread exited unexpectedly"))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| io::Error::other("Agent detection error thread exited unexpectedly"))??;
        Ok(status.map(|status| std::process::Output {
            status,
            stdout,
            stderr,
        }))
    })
}

fn terminate_agent_probe_process(child: &mut Child) {
    #[cfg(target_os = "windows")]
    {
        let process_id = child.id().to_string();
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &process_id, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        let _ = command.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn read_agent_version(path: &Path, home: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    let mut command = windows_command_for_executable(path, false);

    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(path);

    command.arg("--version");
    configure_background_command(&mut command);
    configure_agent_command_environment(&mut command, home, path).ok()?;
    let output = command_output_with_timeout(&mut command, AGENT_VERSION_PROBE_TIMEOUT).ok()??;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .find_map(normalize_detected_agent_version)
}

pub(crate) fn normalize_detected_agent_version(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 256
        && value.chars().any(|character| character.is_ascii_digit())
        && !value.chars().any(char::is_control))
    .then(|| value.to_string())
}

pub(crate) fn inspect_claude_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read Claude Code configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Claude Code configuration: {error}"))?;
    let env = root.get("env").and_then(serde_json::Value::as_object);
    let expected_base = managed_core_loopback_origin(port);
    let configured = env
        .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        == Some(expected_base.as_str())
        && env
            .and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN"))
            .and_then(serde_json::Value::as_str)
            == Some(api_key)
        && env
            .and_then(|env| env.get(CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV))
            .and_then(serde_json::Value::as_str)
            == Some("1")
        && env
            .and_then(|env| env.get(CLAUDE_CODE_AUTO_MODE_SERVER_ENV))
            .and_then(serde_json::Value::as_str)
            == Some("0");
    let model = env
        .and_then(|env| env.get("ANTHROPIC_MODEL"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| root.get("model").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(strip_claude_code_context_suffix)
        .map(str::to_string);
    Ok((configured, model))
}

pub(crate) fn inspect_claude_code_model_mappings(
    path: &Path,
) -> Result<Option<ClaudeDesktopModelMappings>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let root: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path)
            .map_err(|error| format!("Failed to read Claude Code configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Claude Code configuration: {error}"))?;
    let Some(env) = root.get("env").and_then(serde_json::Value::as_object) else {
        return Ok(None);
    };
    let fallback = env
        .get("ANTHROPIC_MODEL")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| root.get("model").and_then(serde_json::Value::as_str).map(str::trim).filter(|value| !value.is_empty()));
    let read_model = |key: &str| {
        let value = env
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(fallback)?;
        let normalized = strip_claude_code_context_suffix(value);
        Some((normalized.to_string(), normalized.len() != value.len()))
    };
    let Some((opus, opus_had_1m)) = read_model("ANTHROPIC_DEFAULT_OPUS_MODEL") else {
        return Ok(None);
    };
    let Some((sonnet, sonnet_had_1m)) = read_model("ANTHROPIC_DEFAULT_SONNET_MODEL") else {
        return Ok(None);
    };
    let Some((haiku, haiku_had_1m)) = read_model("ANTHROPIC_DEFAULT_HAIKU_MODEL") else {
        return Ok(None);
    };
    let legacy_1m = opus_had_1m || sonnet_had_1m || haiku_had_1m;
    let read_window = |value: &serde_json::Value| value.as_u64().or_else(|| value.as_str().and_then(|text| text.trim().parse::<u64>().ok()));
    let max_context_tokens = env
        .get(CLAUDE_CODE_MAX_CONTEXT_TOKENS_ENV)
        .and_then(read_window)
        .or_else(|| root.get("autoCompactWindow").and_then(read_window))
        .unwrap_or_else(|| {
            if legacy_1m {
                CLAUDE_DESKTOP_EXTENDED_CONTEXT_WINDOW
            } else {
                default_claude_code_max_context_tokens()
            }
        });
    let auto_compact_pct = env
        .get(CLAUDE_AUTOCOMPACT_PCT_OVERRIDE_ENV)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or_else(default_claude_auto_compact_pct);
    let disable_auto_compact = root
        .get("autoCompactEnabled")
        .and_then(serde_json::Value::as_bool)
        .map(|enabled| !enabled)
        .unwrap_or_else(|| env
            .get(DISABLE_AUTO_COMPACT_ENV)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true")));
    Ok(Some(ClaudeDesktopModelMappings {
        desktop_models: None,
        opus,
        sonnet,
        haiku,
        opus_1m: opus_had_1m,
        sonnet_1m: sonnet_had_1m,
        haiku_1m: haiku_had_1m,
        fable: read_model("ANTHROPIC_DEFAULT_FABLE_MODEL").map(|entry| entry.0).unwrap_or_default(),
        fable_1m: read_model("ANTHROPIC_DEFAULT_FABLE_MODEL").is_some_and(|entry| entry.1),
        max_context_tokens,
        auto_compact_pct,
        disable_auto_compact,
        manage_default_model: env
            .get("EASYCLIPROXY_MANAGE_CLAUDE_CODE_DEFAULT_MODEL")
            .and_then(serde_json::Value::as_str)
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true),
        manage_subagent_model: env
            .get("EASYCLIPROXY_MANAGE_CLAUDE_CODE_SUBAGENT_MODEL")
            .and_then(serde_json::Value::as_str)
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true),
        startup_model: Some(env
            .get("ANTHROPIC_MODEL")
            .and_then(serde_json::Value::as_str)
            .or_else(|| root.get("model").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string()),
        subagent_model: Some(env
            .get("CLAUDE_CODE_SUBAGENT_MODEL")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()),
    }))
}

pub(crate) fn strip_claude_code_context_suffix(value: &str) -> &str {
    let mut value = value.trim();
    loop {
        let Some(suffix_start) = value.len().checked_sub(4) else {
            return value;
        };
        if !value[suffix_start..].eq_ignore_ascii_case("[1m]") {
            return value;
        }
        value = value[..suffix_start].trim_end();
    }
}

pub(crate) fn agent_model_source_name<'a>(
    models: &'a [AgentModelOption],
    model_name: &'a str,
) -> &'a str {
    let model_name = strip_claude_code_context_suffix(model_name);
    let Some(model) = models
        .iter()
        .find(|model| model.name.eq_ignore_ascii_case(model_name))
    else {
        return model_name;
    };
    if model.is_alias {
        model.alias.as_deref().unwrap_or(model.name.as_str())
    } else {
        model.name.as_str()
    }
}

pub(crate) fn agent_model_context_window(
    models: &[AgentModelOption],
    model_name: &str,
) -> Option<u64> {
    let model_name = strip_claude_code_context_suffix(model_name);
    let selected = models
        .iter()
        .find(|model| model.name.eq_ignore_ascii_case(model_name))?;
    if !selected.is_alias {
        return Some(
            selected
                .context_window
                .unwrap_or(DEFAULT_CLAUDE_CONTEXT_WINDOW),
        );
    }
    selected
        .alias
        .as_deref()
        .and_then(|source| {
            models
                .iter()
                .find(|model| model.name.eq_ignore_ascii_case(source))
        })
        .and_then(|model| model.context_window)
        .or(selected.context_window)
        .or(Some(DEFAULT_CLAUDE_CONTEXT_WINDOW))
}

pub(crate) fn claude_effective_context_window(
    models: &[AgentModelOption],
    model_name: &str,
    enable_1m: bool,
) -> Option<u64> {
    let context_window = agent_model_context_window(models, model_name);
    if enable_1m {
        return Some(
            context_window
                .unwrap_or(DEFAULT_CLAUDE_CONTEXT_WINDOW)
                .max(CLAUDE_DESKTOP_EXTENDED_CONTEXT_WINDOW),
        );
    }
    context_window
}

pub(crate) fn agent_model_display_name<'a>(
    models: &'a [AgentModelOption],
    model_name: &'a str,
) -> &'a str {
    let model_name = strip_claude_code_context_suffix(model_name);
    let Some(model) = models
        .iter()
        .find(|model| model.name.eq_ignore_ascii_case(model_name))
    else {
        return model_name;
    };
    if model.is_alias {
        model.name.as_str()
    } else {
        model.alias.as_deref().unwrap_or(model.name.as_str())
    }
}

pub(crate) fn claude_code_model_effort_level(
    models: &[AgentModelOption],
    model_name: &str,
) -> Result<Option<String>, String> {
    claude_catalog::claude_code_effort_level_for(agent_model_source_name(models, model_name))
}

pub(crate) fn claude_code_model_setting(model_name: &str, enable_1m_variant: bool) -> String {
    let model_name = strip_claude_code_context_suffix(model_name);
    if enable_1m_variant {
        format!("{model_name}[1m]")
    } else {
        model_name.to_string()
    }
}

pub(crate) fn claude_code_model_settings(
    mappings: &ClaudeDesktopModelMappings,
) -> ClaudeDesktopModelMappings {
    ClaudeDesktopModelMappings {
        desktop_models: None,
        opus: claude_code_model_setting(&mappings.opus, mappings.opus_1m),
        sonnet: claude_code_model_setting(&mappings.sonnet, mappings.sonnet_1m),
        haiku: claude_code_model_setting(&mappings.haiku, mappings.haiku_1m),
        opus_1m: mappings.opus_1m,
        sonnet_1m: mappings.sonnet_1m,
        haiku_1m: mappings.haiku_1m,
        fable: claude_code_model_setting(if mappings.fable.is_empty() { &mappings.sonnet } else { &mappings.fable }, if mappings.fable.is_empty() { mappings.sonnet_1m } else { mappings.fable_1m }),
        fable_1m: mappings.fable_1m,
        max_context_tokens: mappings.max_context_tokens,
        auto_compact_pct: mappings.auto_compact_pct,
        disable_auto_compact: mappings.disable_auto_compact,
        manage_default_model: mappings.manage_default_model,
        manage_subagent_model: mappings.manage_subagent_model,
        startup_model: mappings.startup_model.clone(),
        subagent_model: mappings.subagent_model.clone(),
    }
}


pub(crate) fn format_context_window(context_window: u64) -> String {
    if context_window.is_multiple_of(1_000_000) {
        format!("{}M", context_window / 1_000_000)
    } else if context_window.is_multiple_of(1_000) {
        format!("{}K", context_window / 1_000)
    } else {
        context_window.to_string()
    }
}

pub(crate) fn claude_code_model_presentation(
    models: &[AgentModelOption],
    model_name: &str,
    enable_1m: bool,
    context_window_override: Option<u64>,
    role: Option<&str>,
) -> (String, String) {
    let display_name = agent_model_display_name(models, model_name);
    let extended = enable_1m || model_name.trim().to_ascii_lowercase().ends_with("[1m]");
    let display_name = if extended {
        format!("{display_name}[1m]")
    } else {
        display_name.to_string()
    };
    let context_window = context_window_override.unwrap_or_else(|| {
        claude_effective_context_window(models, model_name, enable_1m)
            .unwrap_or(DEFAULT_CLAUDE_CONTEXT_WINDOW)
    });
    let context_label = format_context_window(context_window);
    let name = match role {
        Some(role) => format!("{role} · {display_name}"),
        None => display_name.to_string(),
    };
    let description = if extended {
        format!("1M enabled · Auto-compact window: {context_label}")
    } else {
        format!("Auto-compact window: {context_label}")
    };
    (name, description)
}

pub(crate) fn claude_code_model_presentation_environment(
    mappings: &ClaudeDesktopModelMappings,
    models: &[AgentModelOption],
) -> Result<Vec<(String, String)>, String> {
    let opus = claude_code_model_presentation(
        models,
        &mappings.opus,
        mappings.opus_1m,
        Some(mappings.max_context_tokens),
        Some("Opus"),
    );
    let sonnet = claude_code_model_presentation(
        models,
        &mappings.sonnet,
        mappings.sonnet_1m,
        Some(mappings.max_context_tokens),
        Some("Sonnet"),
    );
    let haiku = claude_code_model_presentation(
        models,
        &mappings.haiku,
        mappings.haiku_1m,
        Some(mappings.max_context_tokens),
        Some("Haiku"),
    );
    let fable = claude_code_model_presentation(
        models,
        if mappings.fable.is_empty() { &mappings.sonnet } else { &mappings.fable },
        if mappings.fable.is_empty() { mappings.sonnet_1m } else { mappings.fable_1m },
        Some(mappings.max_context_tokens),
        Some("Fable"),
    );
    let custom = claude_code_model_presentation(
        models,
        &mappings.sonnet,
        mappings.sonnet_1m,
        Some(mappings.max_context_tokens),
        None,
    );
    let custom_model = claude_code_model_setting(&mappings.sonnet, mappings.sonnet_1m);
    Ok([
        ("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME", opus.0),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION", opus.1),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME", sonnet.0),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION", sonnet.1),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME", haiku.0),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION", haiku.1),
        ("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME", fable.0),
        ("ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION", fable.1),
        ("ANTHROPIC_CUSTOM_MODEL_OPTION", custom_model),
        ("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME", custom.0),
        ("ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION", custom.1),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect())
}

pub(crate) fn inspect_claude_desktop_agent_config(
    paths: &[PathBuf],
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if paths.len() != 4 || !paths.iter().any(|path| path.is_file()) {
        return Ok((false, None));
    }
    let normal = read_agent_json_or_empty(&paths[0], "Claude Desktop main configuration")?;
    let threep = read_agent_json_or_empty(&paths[1], "Claude Desktop 3P configuration")?;
    let profile = read_agent_json_or_empty(&paths[2], "Claude Desktop gateway configuration")?;
    let meta = read_agent_json_or_empty(&paths[3], "Claude Desktop configuration index")?;
    let expected_base = managed_core_loopback_origin(port);
    let configured = normal
        .get("deploymentMode")
        .and_then(serde_json::Value::as_str)
        == Some("3p")
        && threep
            .get("deploymentMode")
            .and_then(serde_json::Value::as_str)
            == Some("3p")
        && profile
            .get("inferenceGatewayBaseUrl")
            .and_then(serde_json::Value::as_str)
            == Some(expected_base.as_str())
        && profile
            .get("inferenceGatewayApiKey")
            .and_then(serde_json::Value::as_str)
            == Some(api_key)
        && meta.get("appliedId").and_then(serde_json::Value::as_str)
            == Some(CLAUDE_DESKTOP_PROFILE_ID);
    let model = profile
        .get("inferenceModels")
        .and_then(serde_json::Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| {
            model
                .as_str()
                .or_else(|| {
                    model
                        .get("labelOverride")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| model.get("name").and_then(serde_json::Value::as_str))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok((configured, model))
}

pub(crate) fn read_agent_json_or_empty(
    path: &Path,
    label: &str,
) -> Result<serde_json::Value, String> {
    if !path.is_file() {
        return Ok(serde_json::json!({}));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("Failed to read {label}: {error}"))?;
    if content.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|error| format!("Failed to parse {label}: {error}"))?;
    if !value.is_object() {
        return Err(format!("{label} root must be an object"));
    }
    Ok(value)
}

pub(crate) fn read_agent_json5_or_empty(
    path: &Path,
    label: &str,
) -> Result<serde_json::Value, String> {
    if !path.is_file() {
        return Ok(serde_json::json!({}));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("Failed to read {label}: {error}"))?;
    if content.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value =
        json5::from_str(&content).map_err(|error| format!("Failed to parse {label}: {error}"))?;
    if !value.is_object() {
        return Err(format!("{label} root must be an object"));
    }
    Ok(value)
}

pub(crate) fn inspect_codex_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>, bool), String> {
    if !path.is_file() {
        return Ok((false, None, false));
    }
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read Codex configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Codex configuration: {error}"))?;
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let provider = root
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
        .and_then(toml::Value::as_table);
    let uses_api_key = provider
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(toml::Value::as_str)
        == Some(api_key);
    let oauth_configuration = provider
        .and_then(|provider| provider.get("requires_openai_auth"))
        .and_then(toml::Value::as_bool)
        == Some(true);
    let auth_path = path.with_file_name("auth.json");
    let auth_configured = if oauth_configuration {
        codex_auth_file_has_oauth_tokens(&auth_path)
    } else {
        codex_auth_file_has_api_key(&auth_path, api_key)
    };
    let configured_without_auth = root.get("model_provider").and_then(toml::Value::as_str)
        == Some(MANAGED_AGENT_PROVIDER_ID)
        && root.get("model_catalog_json").and_then(toml::Value::as_str)
            == Some(CODEX_MODEL_CATALOG_FILE)
        && provider
            .and_then(|provider| provider.get("name"))
            .and_then(toml::Value::as_str)
            == Some("EasyCLIProxyAPI")
        && provider
            .and_then(|provider| provider.get("base_url"))
            .and_then(toml::Value::as_str)
            == Some(expected_base.as_str())
        && uses_api_key
        && provider
            .and_then(|provider| provider.get("wire_api"))
            .and_then(toml::Value::as_str)
            == Some("responses");
    let configured = configured_without_auth && auth_configured;
    let model = root
        .get("model")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok((configured, model, oauth_configuration))
}

pub(crate) fn validate_codex_catalog_file(config_path: &Path, model: &str) -> Result<(), String> {
    let catalog_path = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(CODEX_MODEL_CATALOG_FILE);
    let catalog = fs::read_to_string(&catalog_path).map_err(|error| {
        format!(
            "Failed to read Codex model catalog {}: {error}",
            path_to_string(&catalog_path)
        )
    })?;
    validate_codex_catalog(&catalog, model)
}

pub(crate) fn inspect_opencode_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root = read_agent_json5_or_empty(path, "OpenCode configuration")?;
    let provider = root
        .get("provider")
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID));
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let configured = provider
        .and_then(|provider| provider.get("options"))
        .and_then(|options| options.get("baseURL"))
        .and_then(serde_json::Value::as_str)
        == Some(expected_base.as_str())
        && provider
            .and_then(|provider| provider.get("options"))
            .and_then(|options| options.get("apiKey"))
            .and_then(serde_json::Value::as_str)
            == Some(api_key);
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let model = root
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.strip_prefix(&prefix).unwrap_or(value))
        .map(str::to_string);
    Ok((configured, model))
}

pub(crate) fn inspect_zcode_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read ZCode provider configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse ZCode provider configuration: {error}"))?;
    if root.get("schemaVersion").and_then(serde_json::Value::as_u64) != Some(1) {
        return Ok((false, None));
    }
    let config = root.get("config");
    let provider = config
        .and_then(|config| config.get("providerConfigRules"))
        .and_then(|rules| rules.get("providerRules"))
        .and_then(serde_json::Value::as_array)
        .and_then(|rules| rules.iter().find(|rule| {
            rule.get("providerId").and_then(serde_json::Value::as_str)
                == Some(MANAGED_AGENT_PROVIDER_ID)
        }));
    let selected = config
        .and_then(|config| config.get("defaultModelSelection"));
    let model = selected
        .filter(|selection| {
            selection.get("providerId").and_then(serde_json::Value::as_str)
                == Some(MANAGED_AGENT_PROVIDER_ID)
        })
        .and_then(|selection| selection.get("modelId"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let expected_origin = managed_core_loopback_origin(port);
    let expected_base = format!("{expected_origin}/v1");
    let provider_base = provider
        .and_then(|provider| provider.pointer("/config/api/baseUrl"))
        .and_then(serde_json::Value::as_str);
    let configured = model.as_deref().is_some_and(|model| {
        provider
            .and_then(|provider| provider.get("config"))
            .and_then(|config| config.get("personalModelIds"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|models| models.iter().any(|value| value.as_str() == Some(model)))
    }) && provider
        .and_then(|provider| provider.get("enabled"))
        .and_then(serde_json::Value::as_bool) == Some(true)
        && provider
            .and_then(|provider| provider.get("config"))
            .and_then(|config| config.get("group"))
            .and_then(serde_json::Value::as_str) == Some("standard-personal")
        && provider
            .and_then(|provider| provider.pointer("/config/access/type"))
            .and_then(serde_json::Value::as_str) == Some("api-key")
        && provider
            .and_then(|provider| provider.pointer("/config/access/apiKey"))
            .and_then(serde_json::Value::as_str) == Some(api_key)
        && provider
            .and_then(|provider| provider.pointer("/config/api/type"))
            .and_then(serde_json::Value::as_str) == Some("anthropic-messages")
        && provider_base.is_some_and(|base| {
            // Older CPA configurations used the origin. ZCode also normalizes
            // that form to /v1 before making Anthropic Messages requests.
            let base = base.trim_end_matches('/');
            base == expected_base || base == expected_origin
        });
    Ok((configured, model))
}


pub(crate) fn omp_provider_mapping(root: &serde_norway::Mapping) -> Option<&serde_norway::Mapping> {
    yaml_mapping_value(root, "providers")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|providers| yaml_mapping_value(providers, OMP_AGENT_PROVIDER_ID))
        .and_then(serde_norway::Value::as_mapping)
}

pub(crate) fn omp_selected_model(root: &serde_norway::Mapping) -> Option<String> {
    let selector = yaml_mapping_value(root, "modelRoles")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|roles| yaml_mapping_value(roles, "default"))
        .and_then(serde_norway::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let prefix = format!("{OMP_AGENT_PROVIDER_ID}/");
    selector.strip_prefix(&prefix).map(str::to_string)
}

pub(crate) fn inspect_omp_agent_config(
    paths: &[PathBuf],
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if paths.len() != 2 || !paths[0].is_file() || !paths[1].is_file() {
        return Ok((false, None));
    }
    let models = read_agent_yaml_mapping_or_empty(&paths[0], "Oh My Pi models.yml")?;
    let settings = read_agent_yaml_mapping_or_empty(&paths[1], "Oh My Pi config.yml")?;
    let provider = omp_provider_mapping(&models);
    let model = omp_selected_model(&settings);
    let expected = format!("{}/v1", managed_core_loopback_origin(port));
    let configured = provider
        .and_then(|provider| yaml_mapping_value(provider, "baseUrl"))
        .and_then(serde_norway::Value::as_str)
        == Some(expected.as_str())
        && provider
            .and_then(|provider| yaml_mapping_value(provider, "apiKey"))
            .and_then(serde_norway::Value::as_str)
            == Some(api_key)
        && provider
            .and_then(|provider| yaml_mapping_value(provider, "api"))
            .and_then(serde_norway::Value::as_str)
            .is_some_and(|api| api == OMP_AGENT_API || api == "openai-completions")
        && provider
            .and_then(|provider| yaml_mapping_value(provider, "discovery"))
            .and_then(serde_norway::Value::as_mapping)
            .and_then(|discovery| yaml_mapping_value(discovery, "type"))
            .and_then(serde_norway::Value::as_str)
            == Some("openai-models-list")
        && model.is_some();
    Ok((configured, model))
}

pub(crate) fn omp_has_managed_marker(paths: &[PathBuf]) -> Result<bool, String> {
    if paths.first().is_none_or(|path| !path.is_file()) {
        return Ok(false);
    }
    let models = read_agent_yaml_mapping_or_empty(&paths[0], "Oh My Pi models.yml")?;
    Ok(omp_provider_mapping(&models).is_some())
}

pub(crate) fn find_omp_executable(home: &Path) -> Option<PathBuf> {
    let bun = home.join(".bun/bin");
    #[cfg(target_os = "windows")]
    let managed = [bun.join("omp.exe"), bun.join("omp.cmd")];
    #[cfg(not(target_os = "windows"))]
    let managed = [bun.join("omp")];
    managed
        .into_iter()
        .find(|path| agent_cli_executable(path))
        .or_else(|| find_named_agent_executable(home, &["omp"]))
}

pub(crate) fn inspect_kimi_code_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read Kimi Code configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Kimi Code configuration: {error}"))?;
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let provider = root
        .get("providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID))
        .and_then(toml::Value::as_table);
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let selected = root
        .get("default_model")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| value.starts_with(&prefix));
    let model_entry = selected.and_then(|alias| {
        root.get("models")
            .and_then(toml::Value::as_table)
            .and_then(|models| models.get(alias))
            .and_then(toml::Value::as_table)
    });
    let model = model_entry
        .and_then(|entry| entry.get("model"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let configured = selected.is_some()
        && model.is_some()
        && provider
            .and_then(|provider| provider.get("type"))
            .and_then(toml::Value::as_str)
            == Some("openai")
        && provider
            .and_then(|provider| provider.get("base_url"))
            .and_then(toml::Value::as_str)
            == Some(expected_base.as_str())
        && provider
            .and_then(|provider| provider.get("api_key"))
            .and_then(toml::Value::as_str)
            == Some(api_key)
        && model_entry
            .and_then(|entry| entry.get("provider"))
            .and_then(toml::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID);
    Ok((configured, model))
}

pub(crate) fn inspect_grok_build_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root: toml::Value = toml::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read Grok Build configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Grok Build configuration: {error}"))?;
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let selected = root
        .get("models")
        .and_then(toml::Value::as_table)
        .and_then(|models| models.get("default"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| value.starts_with(&prefix));
    let model_entry = selected.and_then(|alias| {
        root.get("model")
            .and_then(toml::Value::as_table)
            .and_then(|models| models.get(alias))
            .and_then(toml::Value::as_table)
    });
    let model = model_entry
        .and_then(|entry| entry.get("model"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let configured = selected.is_some()
        && model.is_some()
        && model_entry
            .and_then(|entry| entry.get("base_url"))
            .and_then(toml::Value::as_str)
            == Some(expected_base.as_str())
        && model_entry
            .and_then(|entry| entry.get("api_key"))
            .and_then(toml::Value::as_str)
            == Some(api_key)
        && model_entry
            .and_then(|entry| entry.get("api_backend"))
            .and_then(toml::Value::as_str)
            == Some("chat_completions");
    Ok((configured, model))
}

pub(crate) fn inspect_openclaw_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("Failed to read OpenClaw configuration: {error}"))?;
    let root: serde_json::Value = json5::from_str(&content)
        .map_err(|error| format!("Failed to parse OpenClaw JSON5 configuration: {error}"))?;
    let provider = root
        .get("models")
        .and_then(|models| models.get("providers"))
        .and_then(|providers| providers.get(MANAGED_AGENT_PROVIDER_ID));
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let configured = provider
        .and_then(|provider| provider.get("baseUrl"))
        .and_then(serde_json::Value::as_str)
        == Some(expected_base.as_str())
        && provider
            .and_then(|provider| provider.get("apiKey"))
            .and_then(serde_json::Value::as_str)
            == Some(api_key);
    let prefix = format!("{MANAGED_AGENT_PROVIDER_ID}/");
    let model = root
        .get("agents")
        .and_then(|agents| agents.get("defaults"))
        .and_then(|defaults| defaults.get("model"))
        .and_then(|model| model.get("primary"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.strip_prefix(&prefix).unwrap_or(value))
        .map(str::to_string);
    Ok((configured, model))
}

pub(crate) fn inspect_hermes_agent_config(
    path: &Path,
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if !path.is_file() {
        return Ok((false, None));
    }
    let root: serde_yaml::Value = serde_yaml::from_str(
        &fs::read_to_string(path).map_err(|error| format!("Failed to read Hermes configuration: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse Hermes YAML configuration: {error}"))?;
    let provider = hermes_managed_provider(&root);
    let expected_base = format!("{}/v1", managed_core_loopback_origin(port));
    let configured = provider
        .and_then(|provider| provider.get("base_url").or_else(|| provider.get("baseUrl")))
        .and_then(serde_yaml::Value::as_str)
        == Some(expected_base.as_str())
        && provider
            .and_then(|provider| provider.get("api_key").or_else(|| provider.get("apiKey")))
            .and_then(serde_yaml::Value::as_str)
            == Some(api_key)
        && root
            .get("model")
            .and_then(|model| model.get("provider"))
            .and_then(serde_yaml::Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID);
    let model = root
        .get("model")
        .and_then(|model| model.get("default"))
        .and_then(serde_yaml::Value::as_str)
        .or_else(|| provider.and_then(|provider| provider.get("model")?.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok((configured, model))
}

pub(crate) fn inspect_deepseek_harness_config(
    paths: &[PathBuf],
    port: u16,
    api_key: &str,
) -> Result<(bool, Option<String>), String> {
    if paths.len() != 2 {
        return Err("Invalid number of DeepSeek Harness configuration paths".to_string());
    }
    let settings = read_agent_yaml_mapping_or_empty(&paths[0], "DeepSeek Harness settings")?;
    let credentials = read_agent_yaml_mapping_or_empty(&paths[1], "DeepSeek Harness credentials")?;
    let provider = yaml_mapping_value(&settings, "llm-pi-ai")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|section| yaml_mapping_value(section, "providers"))
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|providers| yaml_mapping_value(providers, DEEPSEEK_HARNESS_PROVIDER_ID))
        .and_then(serde_norway::Value::as_mapping);
    let selection = yaml_mapping_value(&settings, "agent-default-model")
        .and_then(serde_norway::Value::as_mapping);
    let selected_provider = selection
        .and_then(|selection| yaml_mapping_value(selection, "provider"))
        .and_then(serde_norway::Value::as_str);
    let model = (selected_provider == Some(DEEPSEEK_HARNESS_PROVIDER_ID))
        .then(|| {
            selection
                .and_then(|selection| yaml_mapping_value(selection, "model"))
                .and_then(serde_norway::Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
        })
        .flatten();
    let expected_base = if provider
        .and_then(|p| yaml_mapping_value(p, "api"))
        .and_then(serde_norway::Value::as_str)
        == Some("anthropic-messages")
    {
        managed_core_loopback_origin(port)
    } else {
        format!("{}/v1", managed_core_loopback_origin(port))
    };
    let credentials_layout_supported = deepseek_harness_credentials_layout_supported(&credentials);
    let credential = yaml_mapping_value(&credentials, "refs")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|refs| yaml_mapping_value(refs, DEEPSEEK_HARNESS_CREDENTIAL))
        .and_then(serde_norway::Value::as_str);
    let model_is_published = model.as_deref().is_some_and(|selected| {
        provider
            .and_then(|provider| yaml_mapping_value(provider, "models"))
            .and_then(serde_norway::Value::as_sequence)
            .is_some_and(|models| {
                models.iter().any(|model| {
                    model
                        .as_mapping()
                        .and_then(|model| yaml_mapping_value(model, "id"))
                        .and_then(serde_norway::Value::as_str)
                        .is_some_and(|id| id.eq_ignore_ascii_case(selected))
                })
            })
    });
    let configured = provider
        .and_then(|provider| yaml_mapping_value(provider, "apiKeyEnv"))
        .and_then(serde_norway::Value::as_str)
        == Some(DEEPSEEK_HARNESS_CREDENTIAL)
        && provider
            .and_then(|provider| yaml_mapping_value(provider, "api"))
            .and_then(serde_norway::Value::as_str)
            .is_some_and(|api| {
                matches!(
                    api,
                    "openai-completions" | "openai-responses" | "anthropic-messages"
                )
            })
        && provider
            .and_then(|provider| yaml_mapping_value(provider, "baseURL"))
            .and_then(serde_norway::Value::as_str)
            == Some(expected_base.as_str())
        && selected_provider == Some(DEEPSEEK_HARNESS_PROVIDER_ID)
        && model_is_published
        && credentials_layout_supported
        && credential == Some(api_key);
    Ok((configured, model))
}

fn deepseek_harness_credentials_layout_supported(credentials: &serde_norway::Mapping) -> bool {
    let Some(version) = yaml_mapping_value(credentials, "version") else {
        return false;
    };
    let version_supported = version.as_u64() == Some(DEEPSEEK_HARNESS_CREDENTIALS_VERSION)
        || version.as_i64() == Some(DEEPSEEK_HARNESS_CREDENTIALS_VERSION as i64);
    version_supported
        && credentials
            .keys()
            .all(|key| matches!(key.as_str(), Some("version" | "refs" | "records")))
        && ["refs", "records"].into_iter().all(|section| {
            yaml_mapping_value(credentials, section).is_none_or(serde_norway::Value::is_mapping)
        })
}

pub(crate) fn deepseek_harness_has_managed_marker(paths: &[PathBuf]) -> Result<bool, String> {
    if paths.len() != 2 {
        return Err("Invalid number of DeepSeek Harness configuration paths".to_string());
    }
    let settings = read_agent_yaml_mapping_or_empty(&paths[0], "DeepSeek Harness settings")?;
    let credentials = read_agent_yaml_mapping_or_empty(&paths[1], "DeepSeek Harness credentials")?;
    let provider_exists = yaml_mapping_value(&settings, "llm-pi-ai")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|section| yaml_mapping_value(section, "providers"))
        .and_then(serde_norway::Value::as_mapping)
        .is_some_and(|providers| {
            yaml_mapping_value(providers, DEEPSEEK_HARNESS_PROVIDER_ID).is_some()
        });
    let default_selected = yaml_mapping_value(&settings, "agent-default-model")
        .and_then(serde_norway::Value::as_mapping)
        .and_then(|selection| yaml_mapping_value(selection, "provider"))
        .and_then(serde_norway::Value::as_str)
        == Some(DEEPSEEK_HARNESS_PROVIDER_ID);
    let credential_exists = yaml_mapping_value(&credentials, "refs")
        .and_then(serde_norway::Value::as_mapping)
        .is_some_and(|refs| yaml_mapping_value(refs, DEEPSEEK_HARNESS_CREDENTIAL).is_some())
        || yaml_mapping_value(&credentials, DEEPSEEK_HARNESS_CREDENTIAL).is_some();
    Ok(provider_exists || default_selected || credential_exists)
}
