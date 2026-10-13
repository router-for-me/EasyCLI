use super::*;

static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn setup_directory(config_path: &Path) -> PathBuf {
    config_path.parent().unwrap().join("ezcpa-copilot")
}

fn valid_secret_reference(value: &str) -> bool {
    value.starts_with("${input:chat.lm.secret.")
        && value.ends_with('}')
        && !value.chars().any(char::is_control)
}

pub(super) fn cached_vscode_secret(config_path: &Path, key: &str) -> Option<String> {
    let value: Value = serde_json::from_slice(
        &fs::read(setup_directory(config_path).join("credential.json")).ok()?,
    )
    .ok()?;
    if value["keyHash"] != sha256_bytes(key.as_bytes()) {
        return None;
    }
    value["secretReference"]
        .as_str()
        .filter(|s| valid_secret_reference(s))
        .map(str::to_owned)
}

pub(super) fn vscode_credential_matches(config_path: &Path, key: &str, reference: &Value) -> bool {
    cached_vscode_secret(config_path, key)
        .is_some_and(|value| Some(value.as_str()) == reference.as_str())
}

const HELPER_MANIFEST: &str = include_str!("../../../../helpers/vscode-copilot-setup/package.json");
const HELPER_SOURCE: &str = include_str!("../../../../helpers/vscode-copilot-setup/extension.cjs");

fn write_setup_helper(directory: &Path) -> Result<PathBuf, String> {
    let helper = directory.join("helper");
    write_codex_private_file(&helper.join("package.json"), HELPER_MANIFEST.as_bytes())?;
    write_codex_private_file(&helper.join("extension.cjs"), HELPER_SOURCE.as_bytes())?;
    Ok(helper)
}

fn code_command(executable: &Path) -> Command {
    #[cfg(windows)]
    {
        windows_command_for_executable(executable, false)
    }
    #[cfg(not(windows))]
    {
        Command::new(executable)
    }
}

// Only remove our known files, never recursively delete a directory or follow
// a helper-directory symlink supplied by another process.
struct SetupRun(PathBuf);
impl Drop for SetupRun {
    fn drop(&mut self) {
        for name in ["request.json", "response.json", "response.pending"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let helper = self.0.join("helper");
        if fs::symlink_metadata(&helper).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            for name in ["package.json", "extension.cjs"] {
                let _ = fs::remove_file(helper.join(name));
            }
            let _ = fs::remove_dir(helper);
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn setup_command(executable: &Path, helper: &Path) -> Command {
    let mut command = code_command(executable);
    // VS Code loads this source only in a dedicated development-host window.
    // It is not installed, synced, or loaded by ordinary editor windows.
    command.arg("--new-window").arg(format!(
        "--extensionDevelopmentPath={}",
        path_to_string(helper)
    ));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    command
}

fn connect(home: &Path, key: &str, executable_override: Option<&Path>) -> Result<(), String> {
    let config_path = copilot_config_paths(AgentClient::VscodeCopilot, home).remove(0);
    // Validate before asking VS Code to touch its model configuration.
    read_groups(&config_path, read_optional_text(&config_path)?.as_deref())?;
    if cached_vscode_secret(&config_path, key).is_some() {
        return Ok(());
    }
    let executable = executable_override
        .filter(|p| p.is_file())
        .map(Path::to_path_buf)
        .or_else(|| find_agent_executable(AgentClient::VscodeCopilot, home))
        .ok_or("COPILOT_SETUP_NOT_INSTALLED")?;
    let directory = setup_directory(&config_path);
    fs::create_dir_all(&directory).map_err(|_| "Failed to create Copilot setup directory")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Invalid system clock")?;
    let id = sha256_bytes(format!("{}:{}", std::process::id(), now.as_nanos()).as_bytes());
    let run = SetupRun(directory.join(format!("setup-{id}")));
    let helper = write_setup_helper(&run.0)?;
    let request = run.0.join("request.json");
    let response = run.0.join("response.json");
    write_codex_private_file(
        &request,
        &serde_json::to_vec(&json!({
            "id": id, "apiKey": key, "expiresAt": now.as_millis() as u64 + 120_000,
            "configPath": path_to_string(&config_path),
        }))
        .map_err(|_| "Failed to prepare Copilot connection")?,
    )?;
    setup_command(&executable, &helper)
        .spawn()
        .map_err(|_| "COPILOT_SETUP_LAUNCH_FAILED")?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Ok(bytes) = fs::read(&response) {
            if let Ok(result) = serde_json::from_slice::<Value>(&bytes) {
                if result["id"] == id {
                    if let Some(reference) = result["secretReference"]
                        .as_str()
                        .filter(|s| valid_secret_reference(s))
                    {
                        return write_codex_private_file(&directory.join("credential.json"),
                            &serde_json::to_vec(&json!({"keyHash": sha256_bytes(key.as_bytes()), "secretReference": reference})).unwrap());
                    }
                    return Err(match result["error"].as_str() {
                        Some("copilot-disabled" | "ai-disabled") => "COPILOT_SETUP_DISABLED",
                        Some("dirty") => "COPILOT_SETUP_DIRTY",
                        Some("unsupported") => "COPILOT_SETUP_UNSUPPORTED",
                        Some("profile-mismatch") => "COPILOT_SETUP_PROFILE",
                        _ => "COPILOT_SETUP_FAILED",
                    }
                    .into());
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err("COPILOT_SETUP_TIMEOUT".into())
}

pub(crate) async fn prepare_vscode_copilot_connection(
    client: AgentClient,
    home: &Path,
    key: &str,
    overrides: Option<&std::collections::HashMap<String, String>>,
) -> Result<(), String> {
    if client != AgentClient::VscodeCopilot {
        return Ok(());
    }
    let _guard = SETUP_LOCK.lock().await;
    let home = home.to_path_buf();
    let key = key.to_owned();
    let executable = overrides
        .and_then(|v| v.get(client.id()))
        .map(PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || connect(&home, &key, executable.as_deref()))
        .await
        .map_err(|_| "Copilot connection task failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copilot_setup_uses_temporary_development_host_and_cleans_sensitive_files() {
        let directory = env::temp_dir().join(format!(
            "cpa-setup-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let run = SetupRun(directory.clone());
            let helper = write_setup_helper(&run.0).unwrap();
            write_codex_private_file(&run.0.join("request.json"), b"fixture-key").unwrap();
            write_codex_private_file(&run.0.join("response.pending"), b"{}").unwrap();
            let command = setup_command(Path::new("code"), &helper);
            let arguments = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert!(arguments
                .iter()
                .any(|a| a.starts_with("--extensionDevelopmentPath=")));
            assert!(!arguments
                .iter()
                .any(|a| a.contains("install-extension") || a.contains("fixture-key")));
            let manifest: Value = serde_json::from_str(HELPER_MANIFEST).unwrap();
            assert_eq!(manifest["name"], "copilot-setup");
        }
        assert!(!directory.exists());
    }
}
