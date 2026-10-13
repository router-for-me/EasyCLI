use super::*;
use serde_json::{json, Value};
mod setup;
pub(crate) use setup::prepare_vscode_copilot_connection;

#[cfg(test)]
mod tests;

const CONNECTION_FILE: &str = "ezcpa-connection.json";
const GROUP_NAME: &str = "CPA (EasyCLIProxyAPI)";

fn copilot_version_number(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('.');
    let core = value.split(['-', '+']).next()?;
    let parts = core.split('.').collect::<Vec<_>>();
    (parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && !value.chars().any(char::is_whitespace))
    .then(|| value.to_string())
}

fn copilot_cli_version_output(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| copilot_version_number(line.trim().strip_prefix("GitHub Copilot CLI ")?))
}

fn vscode_copilot_version_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (id, version) = line.trim().split_once('@')?;
        id.eq_ignore_ascii_case("github.copilot-chat")
            .then(|| copilot_version_number(version))
            .flatten()
    })
}

fn copilot_probe_output(executable: &Path, home: &Path, args: &[&str]) -> Option<String> {
    #[cfg(target_os = "windows")]
    let mut command = windows_command_for_executable(executable, false);
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(executable);
    command.args(args);
    configure_background_command(&mut command);
    configure_agent_command_environment(&mut command, home, executable).ok()?;
    // No input is provided to installation shims; only a successful, explicit
    // identity response counts. Do not accept prompts, generic versions or stderr.
    let output = command_output_with_timeout(&mut command, Duration::from_secs(5)).ok()??;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn detect_copilot_cli(
    home: &Path,
    override_path: Option<&Path>,
) -> Option<(PathBuf, String)> {
    let probe = |path: &Path| {
        if !agent_cli_executable(path) {
            return None;
        }
        let version =
            copilot_cli_version_output(&copilot_probe_output(path, home, &["--version"])?)?;
        Some((path.to_path_buf(), version))
    };
    if let Some(path) = override_path {
        return probe(path);
    }
    // A shim earlier on PATH must not hide a real CLI further down PATH.
    for directory in agent_executable_directories(home) {
        #[cfg(target_os = "windows")]
        let names = ["copilot.exe", "copilot.cmd", "copilot.bat", "copilot.com"];
        #[cfg(not(target_os = "windows"))]
        let names = ["copilot"];
        for name in names {
            if let Some(found) = probe(&directory.join(name)) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn read_vscode_copilot_version(executable: &Path, home: &Path) -> Option<String> {
    let external =
        copilot_probe_output(executable, home, &["--list-extensions", "--show-versions"])
            .and_then(|output| vscode_copilot_version_output(&output));
    if external.is_some() {
        return external;
    }
    let version = copilot_probe_output(executable, home, &["--version"])?;
    bundled_vscode_copilot_version(executable, &version)
}

fn bundled_vscode_copilot_version(executable: &Path, version_output: &str) -> Option<String> {
    let resolved = fs::canonicalize(executable).ok()?;
    let commit = version_output
        .lines()
        .map(str::trim)
        .find(|line| line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()));
    // Resolve only this editor's installation, including the active Windows
    // update directory. Never scan arbitrary old installations for a version.
    for parent in resolved.ancestors().skip(1).take(4) {
        let mut apps = vec![parent.join("resources/app"), parent.join("Resources/app")];
        if let Some(commit) = commit {
            apps.insert(0, parent.join(&commit[..10]).join("resources/app"));
        }
        for app in apps {
            if !app.join("product.json").is_file() {
                continue;
            }
            for directory in ["copilot", "github.copilot-chat"] {
                let package = app.join("extensions").join(directory).join("package.json");
                let Some(value) = fs::read_to_string(package)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                else {
                    continue;
                };
                if value
                    .get("publisher")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.eq_ignore_ascii_case("github"))
                    && value.get("name").and_then(Value::as_str) == Some("copilot-chat")
                {
                    if let Some(version) = value
                        .get("version")
                        .and_then(Value::as_str)
                        .and_then(copilot_version_number)
                    {
                        return Some(version);
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn copilot_config_paths(client: AgentClient, home: &Path) -> Vec<PathBuf> {
    if client == AgentClient::CopilotCli {
        let directory = agent_configuration_environment("COPILOT_HOME")
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".copilot"));
        return vec![directory.join(CONNECTION_FILE)];
    }
    #[cfg(target_os = "windows")]
    let directory =
        agent_configuration_environment("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"));
    #[cfg(target_os = "macos")]
    let directory = home.join("Library/Application Support");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let directory = agent_configuration_environment("XDG_CONFIG_HOME")
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    vec![directory.join("Code/User/chatLanguageModels.json")]
}

fn managed_group(value: &Value) -> bool {
    value.get("name").and_then(Value::as_str) == Some(GROUP_NAME)
        && value.get("vendor").and_then(Value::as_str) == Some("customendpoint")
}

fn read_groups(path: &Path, existing: Option<&str>) -> Result<Value, String> {
    let value = existing
        .map(|s| parse(path, Some(s)))
        .transpose()?
        .unwrap_or_else(|| json!([]));
    if !value.is_array() {
        return Err("VS Code model configuration must be an array".into());
    }
    Ok(value)
}

pub(crate) fn validate_copilot_groups_image(
    path: &Path,
    content: Option<&str>,
) -> Result<(), String> {
    read_groups(path, content).map(|_| ())
}

// Use the same API metadata, configured limits, and catalog defaults as Codex,
// while retaining Copilot's public model list and model capabilities.
pub(crate) fn enrich_vscode_context_windows(
    models: &mut [AgentModelOption],
    runtime_models: &[codex_catalog::CodexRuntimeModel],
) -> Result<(), String> {
    let prepared = prepare_codex_agent_models(runtime_models)?;
    for model in models {
        if let Some(source) = prepared.models.iter().find(|source| source.name.eq_ignore_ascii_case(&model.name)) {
            model.context_window = source.context_window.or(model.context_window);
        }
    }
    Ok(())
}

pub(crate) fn build_copilot_config(
    client: AgentClient,
    path: &Path,
    existing: Option<&str>,
    base: &str,
    key: &str,
    model: &str,
    models: &[AgentModelOption],
) -> Result<String, String> {
    if client == AgentClient::CopilotCli {
        let mut root = parse(path, existing)?;
        root["provider"] = json!(MANAGED_AGENT_PROVIDER_ID);
        root["baseUrl"] = json!(base);
        root["apiKey"] = json!(key);
        root["model"] = json!(model);
        return render(path, &root);
    }
    let mut root = read_groups(path, existing)?;
    let entries = root.as_array_mut().unwrap();
    let previous = entries.iter().find(|v| managed_group(v)).cloned();
    let mut group = previous.unwrap_or_else(|| json!({"name": GROUP_NAME, "vendor": "customendpoint"}));
    let metadata = models.iter().find(|m| m.name == model);
    let context = metadata.and_then(|m| m.context_window).filter(|value| *value > 0).unwrap_or(128_000);
    let vision = metadata
        .and_then(|m| m.input_modalities.as_ref())
        .is_some_and(|m| m.iter().any(|s| s == "image"));
    // The one-time helper supplies a native secret reference, never a plaintext key.
    if key.starts_with("${input:chat.lm.secret.") && key.ends_with('}') {
        group["apiKey"] = json!(key);
    }
    group["models"] = json!([{"id": model, "name": model,
        "url": format!("{}/chat/completions", base.trim_end_matches('/')),
        "apiType": "chat-completions",
        "toolCalling": true, "vision": vision,
        "contextWindow": context, "maxOutputTokens": 16_384_u64.min(context / 2)}]);
    entries.retain(|v| !managed_group(v));
    entries.push(group);
    render(path, &root)
}

pub(crate) fn build_copilot_updates(
    client: AgentClient,
    home: &Path,
    base: &str,
    key: &str,
    model: &str,
    models: &[AgentModelOption],
    template: bool,
) -> Result<Vec<AgentFileUpdate>, String> {
    copilot_config_paths(client, home)
        .into_iter()
        .map(|path| {
            let before = if template && client != AgentClient::VscodeCopilot {
                None
            } else {
                read_optional_text(&path)?
            };
            let reference = if client == AgentClient::VscodeCopilot {
                setup::cached_vscode_secret(&path, key)
            } else { None };
            // Preview and apply share this pure builder; reject stale credentials
            // without starting a setup window or changing the previewed files.
            if template && client == AgentClient::VscodeCopilot && reference.is_none() {
                return Err("COPILOT_SETUP_REQUIRED".into());
            }
            let after =
                build_copilot_config(client, &path, before.as_deref(), base,
                    reference.as_deref().unwrap_or(key), model, models)?;
            Ok(AgentFileUpdate { path, after })
        })
        .collect()
}

pub(crate) fn copilot_has_marker(client: AgentClient, paths: &[PathBuf]) -> Result<bool, String> {
    let content = read_optional_text(&paths[0])?;
    if client == AgentClient::CopilotCli {
        Ok(parse(&paths[0], content.as_deref())?
            .get("provider")
            .and_then(Value::as_str)
            == Some(MANAGED_AGENT_PROVIDER_ID))
    } else {
        Ok(read_groups(&paths[0], content.as_deref())?
            .as_array()
            .unwrap()
            .iter()
            .any(managed_group))
    }
}

pub(crate) fn inspect_copilot_config(
    client: AgentClient,
    paths: &[PathBuf],
    port: u16,
    key: &str,
) -> Result<(bool, Option<String>), String> {
    let content = read_optional_text(&paths[0])?;
    let base = format!("{}/v1", managed_core_loopback_origin(port));
    if client == AgentClient::CopilotCli {
        let root = parse(&paths[0], content.as_deref())?;
        let model = root
            .get("model")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        return Ok((
            model.is_some()
                && root["provider"] == MANAGED_AGENT_PROVIDER_ID
                && root["baseUrl"] == base
                && root["apiKey"] == key,
            model,
        ));
    }
    let root = read_groups(&paths[0], content.as_deref())?;
    let Some(group) = root.as_array().unwrap().iter().find(|g| managed_group(g)) else {
        return Ok((false, None));
    };
    let model = group
        .pointer("/models/0/id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let configured = model.is_some()
        && group.pointer("/models/0/url").and_then(Value::as_str)
            == Some(&format!("{base}/chat/completions"))
        && group
            .get("apiKey")
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with("${input:") && s.ends_with('}'))
        && setup::vscode_credential_matches(&paths[0], key, &group["apiKey"]);
    Ok((configured, model))
}

pub(crate) fn restore_copilot_config(
    client: AgentClient,
    path: &Path,
    current: &str,
    original: Option<&str>,
) -> Result<Option<String>, String> {
    if client == AgentClient::CopilotCli {
        let mut root = parse(path, Some(current))?;
        let before = parse(path, original)?;
        for k in ["provider", "baseUrl", "apiKey", "model"] {
            if let Some(v) = before.get(k) {
                root[k] = v.clone();
            } else {
                root.as_object_mut().unwrap().remove(k);
            }
        }
        return if original.is_none() && root.as_object().unwrap().is_empty() {
            Ok(None)
        } else {
            render(path, &root).map(Some)
        };
    }
    let mut root = read_groups(path, Some(current))?;
    let before = read_groups(path, original)?;
    let entries = root.as_array_mut().unwrap();
    entries.retain(|g| !managed_group(g));
    entries.extend(
        before
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| managed_group(g))
            .cloned(),
    );
    if original.is_none() && entries.is_empty() {
        Ok(None)
    } else {
        render(path, &root).map(Some)
    }
}

pub(crate) fn prepare_copilot_removal(
    client: AgentClient,
    paths: &[PathBuf],
) -> Result<Images, String> {
    if !copilot_has_marker(client, paths)? {
        return Ok(Vec::new());
    }
    paths
        .iter()
        .map(|path| {
            let value = read_optional_text(path)?
                .map(|s| restore_copilot_config(client, path, &s, None))
                .transpose()?
                .flatten();
            Ok((path.clone(), value.map(String::into_bytes)))
        })
        .collect()
}

pub(crate) fn validate_copilot_groups_preserved(
    before: &Value,
    after: &Value,
) -> Result<(), String> {
    let unmanaged = |v: &Value| {
        v.as_array()
            .into_iter()
            .flatten()
            .filter(|g| !managed_group(g))
            .cloned()
            .collect::<Vec<_>>()
    };
    if unmanaged(before) != unmanaged(after) {
        return Err("Copilot update changed another VS Code provider group".into());
    }
    Ok(())
}

pub(crate) fn merge_copilot_groups_original(baseline: &Value, current: &Value) -> Value {
    let mut groups = current
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| !managed_group(g))
        .cloned()
        .collect::<Vec<_>>();
    groups.extend(
        baseline
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| managed_group(g))
            .cloned(),
    );
    Value::Array(groups)
}

fn copilot_helper_command(args: &[std::ffi::OsString]) -> Result<Command, String> {
    let path = args
        .get(2)
        .map(PathBuf::from)
        .ok_or("Copilot connection path is missing")?;
    let executable = args
        .get(3)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .ok_or("Copilot executable is missing")?;
    let root = parse(&path, read_optional_text(&path)?.as_deref())?;
    if root["provider"] != MANAGED_AGENT_PROVIDER_ID {
        return Err("Copilot connection is not managed by CPA".into());
    }
    let mut command = Command::new(executable);
    for (field, variable) in [
        ("baseUrl", "COPILOT_PROVIDER_BASE_URL"),
        ("apiKey", "COPILOT_PROVIDER_API_KEY"),
        ("model", "COPILOT_MODEL"),
    ] {
        let value = root
            .get(field)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or("Copilot connection is incomplete")?;
        command.env(variable, value);
    }
    command
        .env("COPILOT_PROVIDER_TYPE", "openai")
        .env("COPILOT_PROVIDER_WIRE_API", "completions");
    for key in [
        "COPILOT_PROVIDER_BEARER_TOKEN",
        "COPILOT_PROVIDER_API_KEY_COMMAND",
        "COPILOT_PROVIDER_HEADERS",
        "COPILOT_PROVIDER_MODEL_ID",
        "COPILOT_PROVIDER_WIRE_MODEL",
        "COPILOT_PROVIDER_TRANSPORT",
    ] {
        command.env_remove(key);
    }
    command.args(args.iter().skip(4));
    Ok(command)
}

pub(crate) fn run_copilot_helper(args: &[std::ffi::OsString]) -> Result<i32, String> {
    let mut command = copilot_helper_command(args)?;
    #[cfg(target_os = "windows")]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(u32::MAX);
    }
    command
        .status()
        .map(|s| s.code().unwrap_or(1))
        .map_err(|_| "Failed to start Copilot CLI".into())
}
