use super::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub(crate) const DEFAULT_WORKBUDDY_PORT: u16 = 8788;
pub(crate) const DEFAULT_WORKBUDDY_HOST: &str = "127.0.0.1";

#[derive(Default)]
pub(crate) struct WorkBuddyProxyState {
    process: Mutex<Option<ManagedWorkBuddyProxyProcess>>,
    port: Mutex<Option<u16>>,
    host: Mutex<Option<String>>,
    api_key: Mutex<Option<String>>,
    python_path: Mutex<Option<String>>,
}

struct ManagedWorkBuddyProxyProcess {
    child: Child,
    #[cfg(windows)]
    job: isize,
    pid: u32,
    host: String,
    port: u16,
}

impl Drop for ManagedWorkBuddyProxyProcess {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            close_windows_handle(self.job);
            let _ = self.child.wait();
        }
        #[cfg(not(windows))]
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = terminate_child(&mut self.child);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyProxyStatus {
    pub running: bool,
    pub ready: bool,
    pub host: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub python_path: Option<String>,
    pub accounts_count: usize,
    pub base_url: String,
    pub accounts_dir: String,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyDesktopAccountInfo {
    pub file_name: String,
    pub file_path: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub realm: String,
    pub domain: Option<String>,
    pub encrypted: bool,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyOAuthStartResult {
    pub state: String,
    pub auth_url: String,
    pub realm: String,
    pub platform: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyOAuthPollResult {
    pub status: String, // "waiting" | "success" | "expired" | "error"
    pub message: Option<String>,
    pub uid: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
}

/// Detects available python executable on the host.
pub(crate) fn find_python_executable(custom: Option<&str>) -> Option<PathBuf> {
    if let Some(custom_path) = custom {
        let path = PathBuf::from(custom_path.trim());
        if path.is_file() {
            return Some(path);
        }
    }

    #[cfg(windows)]
    {
        // 1. Try standard command in PATH
        for cmd in &["python", "python3", "py"] {
            let mut command = Command::new(cmd);
            command.arg("--version");
            command.creation_flags(CREATE_NO_WINDOW);
            if let Ok(output) = command.output() {
                if output.status.success() {
                    // Try to resolve full path via where
                    let mut where_cmd = Command::new("where");
                    where_cmd.arg(cmd);
                    where_cmd.creation_flags(CREATE_NO_WINDOW);
                    if let Ok(where_out) = where_cmd.output() {
                        if let Ok(text) = String::from_utf8(where_out.stdout) {
                            if let Some(first_line) = text.lines().next() {
                                let trimmed = first_line.trim();
                                if !trimmed.is_empty() && Path::new(trimmed).is_file() {
                                    return Some(PathBuf::from(trimmed));
                                }
                            }
                        }
                    }
                    return Some(PathBuf::from(cmd));
                }
            }
        }

        // 2. Scan standard installation locations
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let programs_python = Path::new(&local_app_data).join("Programs").join("Python");
            if programs_python.is_dir() {
                if let Ok(entries) = fs::read_dir(&programs_python) {
                    let mut pythons: Vec<PathBuf> = entries
                        .filter_map(Result::ok)
                        .map(|e| e.path().join("python.exe"))
                        .filter(|p| p.is_file())
                        .collect();
                    pythons.sort_by(|a, b| b.cmp(a)); // Newest first
                    if let Some(found) = pythons.into_iter().next() {
                        return Some(found);
                    }
                }
            }
        }

        for root in &["C:\\Python313", "C:\\Python312", "C:\\Python311", "C:\\Python310"] {
            let p = Path::new(root).join("python.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    #[cfg(not(windows))]
    {
        for cmd in &["python3", "python"] {
            let mut command = Command::new(cmd);
            command.arg("--version");
            if let Ok(output) = command.output() {
                if output.status.success() {
                    return Some(PathBuf::from(cmd));
                }
            }
        }
    }

    None
}

/// Resolves or initializes the working directory for workbuddy-proxy.
pub(crate) fn ensure_workbuddy_proxy_dir() -> Result<PathBuf, String> {
    let base = core_base_dir()?;
    let proxy_dir = base.join("workbuddy-proxy");

    // Check if target already has wb_proxy.py
    if !proxy_dir.join("wb_proxy.py").is_file() {
        fs::create_dir_all(&proxy_dir)
            .map_err(|e| format!("Failed to create workbuddy-proxy directory: {e}"))?;

        // Locate bundled resources
        let candidate_resource_dirs = [
            base.join("resources").join("workbuddy-proxy"),
            PathBuf::from("resources").join("workbuddy-proxy"),
            PathBuf::from("src-tauri").join("resources").join("workbuddy-proxy"),
        ];

        let mut copied = false;
        for res_dir in &candidate_resource_dirs {
            if res_dir.join("wb_proxy.py").is_file() {
                copy_dir_all(res_dir, &proxy_dir)?;
                copied = true;
                break;
            }
        }

        if !copied && !proxy_dir.join("wb_proxy.py").is_file() {
            return Err("WorkBuddy proxy scripts not found in resources".to_string());
        }
    }

    let accounts_dir = proxy_dir.join("accounts");
    if !accounts_dir.is_dir() {
        let _ = fs::create_dir_all(&accounts_dir);
    }

    Ok(proxy_dir)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    if !dst.exists() {
        fs::create_dir_all(dst).map_err(|e| format!("Failed to create directory {dst:?}: {e}"))?;
    }
    for entry in fs::read_dir(src).map_err(|e| format!("Failed to read {src:?}: {e}"))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
        let ty = entry.file_type().map_err(|e| format!("Failed to read type: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            let _ = fs::copy(&src_path, &dst_path);
        }
    }
    Ok(())
}

fn count_accounts(accounts_dir: &Path) -> usize {
    if !accounts_dir.is_dir() {
        return 0;
    }
    fs::read_dir(accounts_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|s| s.eq_ignore_ascii_case("json"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

#[tauri::command]
pub(crate) async fn get_workbuddy_proxy_status(
    proxy_state: State<'_, WorkBuddyProxyState>,
) -> Result<WorkBuddyProxyStatus, String> {
    let proxy_dir = ensure_workbuddy_proxy_dir()?;
    let accounts_dir = proxy_dir.join("accounts");
    let accounts_count = count_accounts(&accounts_dir);

    let (mut is_running, pid, host, port) = {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        if let Some(managed) = guard.as_mut() {
            match managed.child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;
                    (false, None, DEFAULT_WORKBUDDY_HOST.to_string(), DEFAULT_WORKBUDDY_PORT)
                }
                Ok(None) => (true, Some(managed.pid), managed.host.clone(), managed.port),
                Err(_) => {
                    *guard = None;
                    (false, None, DEFAULT_WORKBUDDY_HOST.to_string(), DEFAULT_WORKBUDDY_PORT)
                }
            }
        } else {
            let configured_port = *proxy_state.port.lock().map_err(|e| e.to_string())?;
            let configured_host = proxy_state.host.lock().map_err(|e| e.to_string())?.clone();
            (
                false,
                None,
                configured_host.unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string()),
                configured_port.unwrap_or(DEFAULT_WORKBUDDY_PORT),
            )
        }
    };

    // If child process wasn't tracked by us, also check if the port is responding to health
    let client = Client::builder().timeout(Duration::from_millis(500)).build().unwrap_or_default();
    let health_url = format!("http://{host}:{port}/health");
    let ready = match client.get(&health_url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    };

    if !is_running && ready {
        is_running = true;
    }

    let python_path = proxy_state
        .python_path
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .or_else(|| find_python_executable(None).map(|p| path_to_string(&p)));

    let message = if is_running && ready {
        format!("WorkBuddy proxy is running at http://{host}:{port}/v1")
    } else if is_running && !ready {
        format!("WorkBuddy proxy is starting at http://{host}:{port}")
    } else {
        "WorkBuddy proxy is stopped".to_string()
    };

    Ok(WorkBuddyProxyStatus {
        running: is_running,
        ready,
        host: host.clone(),
        port,
        pid,
        python_path,
        accounts_count,
        base_url: format!("http://{host}:{port}/v1"),
        accounts_dir: path_to_string(&accounts_dir),
        message,
        error: None,
    })
}

#[tauri::command]
pub(crate) async fn start_workbuddy_proxy(
    proxy_state: State<'_, WorkBuddyProxyState>,
    port: Option<u16>,
    host: Option<String>,
    api_key: Option<String>,
    python_path: Option<String>,
) -> Result<WorkBuddyProxyStatus, String> {
    let proxy_dir = ensure_workbuddy_proxy_dir()?;
    let script_path = proxy_dir.join("wb_proxy.py");
    if !script_path.is_file() {
        return Err(format!("wb_proxy.py not found at {}", path_to_string(&script_path)));
    }

    let selected_port = port.unwrap_or(DEFAULT_WORKBUDDY_PORT);
    let selected_host = host.unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string());

    // Stop existing process if any
    {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        *guard = None;
    }

    let resolved_python = find_python_executable(python_path.as_deref())
        .ok_or_else(|| "Python executable (>= 3.9) not found. Please install Python or set the path in settings".to_string())?;

    let accounts_dir = proxy_dir.join("accounts");
    let _ = fs::create_dir_all(&accounts_dir);

    let mut command = Command::new(&resolved_python);
    command.current_dir(&proxy_dir);
    command.arg(&script_path);
    command.arg("--host").arg(&selected_host);
    command.arg("--port").arg(selected_port.to_string());
    command.arg("--accounts-dir").arg(&accounts_dir);

    if let Some(ref key) = api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            command.arg("--api-key").arg(trimmed);
        }
    }

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to launch WorkBuddy proxy: {e}"))?;

    let pid = child.id();

    #[cfg(windows)]
    let job = attach_child_to_windows_job(&child)?;

    {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        *guard = Some(ManagedWorkBuddyProxyProcess {
            child,
            #[cfg(windows)]
            job,
            pid,
            host: selected_host.clone(),
            port: selected_port,
        });
    }

    // Update state cache
    *proxy_state.port.lock().map_err(|e| e.to_string())? = Some(selected_port);
    *proxy_state.host.lock().map_err(|e| e.to_string())? = Some(selected_host.clone());
    *proxy_state.api_key.lock().map_err(|e| e.to_string())? = api_key;
    *proxy_state.python_path.lock().map_err(|e| e.to_string())? = Some(path_to_string(&resolved_python));

    // Wait briefly for health
    let client = Client::builder().timeout(Duration::from_millis(500)).build().unwrap_or_default();
    let health_url = format!("http://{selected_host}:{selected_port}/health");
    let mut ready = false;
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(resp) = client.get(&health_url).send().await {
            if resp.status().is_success() {
                ready = true;
                break;
            }
        }
    }

    let accounts_count = count_accounts(&accounts_dir);
    Ok(WorkBuddyProxyStatus {
        running: true,
        ready,
        host: selected_host.clone(),
        port: selected_port,
        pid: Some(pid),
        python_path: Some(path_to_string(&resolved_python)),
        accounts_count,
        base_url: format!("http://{selected_host}:{selected_port}/v1"),
        accounts_dir: path_to_string(&accounts_dir),
        message: format!("WorkBuddy proxy started at http://{selected_host}:{selected_port}/v1"),
        error: None,
    })
}

#[tauri::command]
pub(crate) async fn stop_workbuddy_proxy(
    proxy_state: State<'_, WorkBuddyProxyState>,
) -> Result<WorkBuddyProxyStatus, String> {
    {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        *guard = None;
    }

    let proxy_dir = ensure_workbuddy_proxy_dir()?;
    let accounts_dir = proxy_dir.join("accounts");
    let accounts_count = count_accounts(&accounts_dir);

    let host = proxy_state
        .host
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string());
    let port = proxy_state
        .port
        .lock()
        .map_err(|e| e.to_string())?
        .unwrap_or(DEFAULT_WORKBUDDY_PORT);
    let python_path = proxy_state
        .python_path
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .or_else(|| find_python_executable(None).map(|p| path_to_string(&p)));

    Ok(WorkBuddyProxyStatus {
        running: false,
        ready: false,
        host: host.clone(),
        port,
        pid: None,
        python_path,
        accounts_count,
        base_url: format!("http://{host}:{port}/v1"),
        accounts_dir: path_to_string(&accounts_dir),
        message: "WorkBuddy proxy stopped".to_string(),
        error: None,
    })
}

#[tauri::command]
pub(crate) async fn restart_workbuddy_proxy(
    proxy_state: State<'_, WorkBuddyProxyState>,
    port: Option<u16>,
    host: Option<String>,
    api_key: Option<String>,
    python_path: Option<String>,
) -> Result<WorkBuddyProxyStatus, String> {
    let _ = stop_workbuddy_proxy(proxy_state.clone()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    start_workbuddy_proxy(proxy_state, port, host, api_key, python_path).await
}

/// Scans local WorkBuddy desktop credential files (*.info).
#[tauri::command]
pub(crate) fn detect_workbuddy_desktop_accounts() -> Result<Vec<WorkBuddyDesktopAccountInfo>, String> {
    let auth_dir = {
        #[cfg(target_os = "windows")]
        {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| "LOCALAPPDATA environment variable not found".to_string())?;
            Path::new(&local_app_data)
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
        }
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| "HOME environment variable not found".to_string())?;
            Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| "HOME environment variable not found".to_string())?;
            Path::new(&home)
                .join(".config")
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
        }
    };

    if !auth_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let entries = fs::read_dir(&auth_dir)
        .map_err(|e| format!("Failed to read WorkBuddy desktop auth directory: {e}"))?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if !file_name.ends_with(".info") {
            continue;
        }

        let metadata = fs::metadata(&path).ok();
        let last_modified = metadata
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let datetime: chrono::DateTime<chrono::Local> = t.into();
                datetime.format("%Y-%m-%d %H:%M:%S").to_string()
            });

        // Read and analyze json content safely
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(value) = serde_json::from_str::<Value>(&content) {
                let uid = value
                    .get("account")
                    .and_then(|a| a.get("uid"))
                    .or_else(|| value.get("uid"))
                    .and_then(Value::as_str)
                    .map(str::to_string);

                let domain = value
                    .get("auth")
                    .and_then(|a| a.get("domain"))
                    .or_else(|| value.get("domain"))
                    .and_then(Value::as_str)
                    .map(str::to_string);

                let realm = if domain
                    .as_deref()
                    .map(|d| d.contains("tencent.com") || d.contains("codebuddy.cn"))
                    .unwrap_or(false)
                {
                    "cn".to_string()
                } else {
                    "intl".to_string()
                };

                let nickname = value
                    .get("account")
                    .and_then(|a| a.get("nickname"))
                    .and_then(Value::as_str)
                    .map(str::to_string);

                let encrypted = value
                    .get("auth")
                    .and_then(|a| a.get("accessToken"))
                    .map(|token| token.is_object() && token.get("$wbEncrypted").is_some())
                    .unwrap_or(false);

                results.push(WorkBuddyDesktopAccountInfo {
                    file_name,
                    file_path: path_to_string(&path),
                    uid,
                    nickname,
                    realm,
                    domain,
                    encrypted,
                    last_modified,
                });
            }
        }
    }

    Ok(results)
}

/// Imports desktop account credentials into the WorkBuddy proxy.
#[tauri::command]
pub(crate) async fn import_workbuddy_desktop_account(
    proxy_state: State<'_, WorkBuddyProxyState>,
    target_path: Option<String>,
) -> Result<Value, String> {
    let port = proxy_state
        .port
        .lock()
        .map_err(|e| e.to_string())?
        .unwrap_or(DEFAULT_WORKBUDDY_PORT);
    let host = proxy_state
        .host
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string());

    let client = Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
    let import_url = format!("http://{host}:{port}/accounts/import/desktop");

    let payload = if let Some(path) = target_path {
        json!({ "path": path })
    } else {
        json!({ "all": true })
    };

    let response = client
        .post(&import_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to contact WorkBuddy proxy service at {import_url}. Ensure the service is running: {e}"))?;

    let result: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    Ok(result)
}

/// Starts an OAuth login flow for WorkBuddy.
#[tauri::command]
pub(crate) async fn start_workbuddy_oauth(
    _app: AppHandle,
    realm: Option<String>,
    platform: Option<String>,
) -> Result<WorkBuddyOAuthStartResult, String> {
    let selected_realm = realm.unwrap_or_else(|| "intl".to_string());
    let selected_platform = platform.unwrap_or_else(|| "CLI".to_string());

    let upstream = if selected_realm == "cn" {
        "https://copilot.tencent.com"
    } else {
        "https://www.workbuddy.ai"
    };

    let url = format!("{upstream}/v2/plugin/auth/state?platform={selected_platform}");
    let client = Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default();

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| format!("Failed to request WorkBuddy OAuth state from {url}: {e}"))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OAuth state response: {e}"))?;

    let data = body.get("data").ok_or_else(|| format!("Invalid response: {body}"))?;
    let state = data.get("state").and_then(Value::as_str).ok_or("Missing state")?.to_string();
    let auth_url = data.get("authUrl").and_then(Value::as_str).ok_or("Missing authUrl")?.to_string();

    // Open in default browser
    let _ = tauri_plugin_opener::open_url(&auth_url, None::<&str>);

    Ok(WorkBuddyOAuthStartResult {
        state,
        auth_url,
        realm: selected_realm,
        platform: selected_platform,
    })
}

/// Polls token endpoint for WorkBuddy OAuth completion and saves the resulting credential.
#[tauri::command]
pub(crate) async fn poll_workbuddy_oauth(
    gui_config_state: State<'_, GuiConfigState>,
    state: String,
    realm: Option<String>,
) -> Result<WorkBuddyOAuthPollResult, String> {
    let selected_realm = realm.unwrap_or_else(|| "intl".to_string());
    let upstream = if selected_realm == "cn" {
        "https://copilot.tencent.com"
    } else {
        "https://www.workbuddy.ai"
    };

    let url = format!("{upstream}/v2/plugin/auth/token?state={state}");
    let client = Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to poll WorkBuddy OAuth token from {url}: {e}"))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse poll response: {e}"))?;

    let code = body.get("code").and_then(Value::as_i64).unwrap_or(-1);

    if code == 11217 {
        return Ok(WorkBuddyOAuthPollResult {
            status: "waiting".to_string(),
            message: Some("Waiting for authorization in browser...".to_string()),
            uid: None,
            access_token: None,
            refresh_token: None,
            expires_in: None,
        });
    }

    if code != 0 {
        let msg = body.get("msg").and_then(Value::as_str).unwrap_or("Authorization failed");
        return Ok(WorkBuddyOAuthPollResult {
            status: "error".to_string(),
            message: Some(msg.to_string()),
            uid: None,
            access_token: None,
            refresh_token: None,
            expires_in: None,
        });
    }

    let data = body.get("data").ok_or("Missing token data in response")?;
    let access_token = data.get("accessToken").and_then(Value::as_str).unwrap_or("").to_string();
    let refresh_token = data.get("refreshToken").and_then(Value::as_str).map(str::to_string);
    let expires_in = data.get("expiresIn").and_then(Value::as_i64);

    if access_token.is_empty() {
        return Err("Received empty access token".to_string());
    }

    // Extract UID from JWT payload
    let uid = extract_uid_from_jwt(&access_token).unwrap_or_else(|| format!("wb_{}", &state[..8.min(state.len())]));

    // 1. Save into workbuddy-proxy accounts directory
    if let Ok(proxy_dir) = ensure_workbuddy_proxy_dir() {
        let accounts_dir = proxy_dir.join("accounts");
        let account_file = accounts_dir.join(format!("{uid}.json"));
        let now_sec = chrono::Utc::now().timestamp();
        let expires_at = expires_in.map(|exp| now_sec + exp).unwrap_or(now_sec + 2592000);
        let account_record = json!({
            "uid": uid,
            "realm": selected_realm,
            "platform": "CLI",
            "accessToken": access_token,
            "refreshToken": refresh_token,
            "expiresAt": expires_at,
            "source": "oauth",
            "enabled": true
        });
        let _ = fs::write(account_file, serde_json::to_string_pretty(&account_record).unwrap_or_default());
    }

    // 2. Also save into CPA's auth_dir as an OAuth credential JSON
    if let Ok(config) = gui_config_state.snapshot() {
        let auth_dir = PathBuf::from(&config.auth_dir);
        if auth_dir.is_dir() {
            let auth_file_path = auth_dir.join(format!("workbuddy-{uid}.json"));
            let cpa_auth_record = json!({
                "type": "workbuddy",
                "name": format!("workbuddy-{uid}.json"),
                "account_type": "oauth",
                "uid": uid,
                "access_token": access_token,
                "refresh_token": refresh_token,
                "realm": selected_realm,
                "created_at": chrono::Utc::now().to_rfc3339()
            });
            let _ = fs::write(auth_file_path, serde_json::to_string_pretty(&cpa_auth_record).unwrap_or_default());
        }
    }

    Ok(WorkBuddyOAuthPollResult {
        status: "success".to_string(),
        message: Some("Authorization successful".to_string()),
        uid: Some(uid),
        access_token: Some(access_token),
        refresh_token,
        expires_in,
    })
}

fn extract_uid_from_jwt(jwt: &str) -> Option<String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let decoded = engine.decode(parts[1]).or_else(|_| {
        base64::engine::general_purpose::STANDARD.decode(parts[1])
    }).ok()?;
    let payload: Value = serde_json::from_slice(&decoded).ok()?;
    payload
        .get("uid")
        .or_else(|| payload.get("sub"))
        .or_else(|| payload.get("userId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Synchronizes WorkBuddy Reverse Proxy as an OpenAI-compatible provider in EasyCLIProxyAPI's config.yaml.
pub(crate) fn sync_workbuddy_provider_to_cpa_sync(
    port: u16,
    api_key: Option<&str>,
) -> Result<String, String> {
    let install_dir = core_install_dir()?;
    let config_path = install_dir.join(CORE_CONFIG_FILE);

    if !config_path.is_file() {
        return Ok("Kernel configuration file not found yet; skipping sync".to_string());
    }

    let _lock = lock_core_config_file()?;
    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read kernel configuration: {e}"))?;

    let mut doc: serde_norway::Value = serde_norway::from_str(&content)
        .map_err(|e| format!("Failed to parse YAML configuration: {e}"))?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    let key_value = api_key.unwrap_or_default();

    let models_list = vec![
        "default-model",
        "fast-model",
        "balanced-model",
        "primary-model",
        "deep-model",
        "deepseek-v4.1-flash",
        "claude-3-5-sonnet-20241022",
        "claude-3-7-sonnet",
        "gpt-4o",
        "gpt-4o-mini",
        "deepseek-chat",
        "deepseek-reasoner",
    ];

    let models_json: Vec<Value> = models_list
        .iter()
        .map(|m| json!({ "name": m }))
        .collect();

    let new_provider_group = json!({
        "name": "workbuddy",
        "base-url": base_url,
        "api-key": key_value,
        "models": models_json
    });

    let new_provider_yaml: serde_norway::Value = serde_norway::to_value(&new_provider_group)
        .map_err(|e| format!("Failed to serialize provider: {e}"))?;

    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| "Configuration root must be a mapping".to_string())?;

    let api_keys_key = yaml_key("api-keys");
    let openai_compat_key = yaml_key("openai-compatibility");

    if !root.contains_key(&api_keys_key) {
        root.insert(api_keys_key.clone(), serde_norway::Value::Mapping(serde_norway::Mapping::new()));
    }

    let api_keys_map = root
        .get_mut(&api_keys_key)
        .and_then(serde_norway::Value::as_mapping_mut)
        .ok_or_else(|| "api-keys must be a mapping".to_string())?;

    if !api_keys_map.contains_key(&openai_compat_key) {
        api_keys_map.insert(openai_compat_key.clone(), serde_norway::Value::Sequence(Vec::new()));
    }

    let groups_seq = api_keys_map
        .get_mut(&openai_compat_key)
        .and_then(serde_norway::Value::as_sequence_mut)
        .ok_or_else(|| "openai-compatibility must be a sequence".to_string())?;

    // Check if workbuddy entry already exists
    let mut updated = false;
    for item in groups_seq.iter_mut() {
        if let Some(map) = item.as_mapping_mut() {
            if map.get(&yaml_key("name")).and_then(serde_norway::Value::as_str) == Some("workbuddy") {
                *item = new_provider_yaml.clone();
                updated = true;
                break;
            }
        }
    }

    if !updated {
        groups_seq.push(new_provider_yaml);
    }

    let new_content = serde_norway::to_string(&doc)
        .map_err(|e| format!("Failed to render updated YAML: {e}"))?;

    write_core_config_if_changed(&config_path, &new_content)?;

    Ok(format!("WorkBuddy successfully synchronized as OpenAI-compatible provider at {base_url}"))
}

#[tauri::command]
pub(crate) async fn sync_workbuddy_provider_to_cpa(
    port: Option<u16>,
    api_key: Option<String>,
) -> Result<String, String> {
    sync_workbuddy_provider_to_cpa_sync(port.unwrap_or(DEFAULT_WORKBUDDY_PORT), api_key.as_deref())
}

pub(crate) fn start_workbuddy_proxy_sync(
    proxy_state: &WorkBuddyProxyState,
) -> Result<u32, String> {
    let proxy_dir = ensure_workbuddy_proxy_dir()?;
    let script_path = proxy_dir.join("wb_proxy.py");
    if !script_path.is_file() {
        return Err(format!("wb_proxy.py not found at {}", path_to_string(&script_path)));
    }

    let selected_port = proxy_state
        .port
        .lock()
        .map_err(|e| e.to_string())?
        .unwrap_or(DEFAULT_WORKBUDDY_PORT);
    let selected_host = proxy_state
        .host
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string());
    let api_key = proxy_state.api_key.lock().map_err(|e| e.to_string())?.clone();
    let python_path = proxy_state.python_path.lock().map_err(|e| e.to_string())?.clone();

    // Check if already running
    {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        if let Some(managed) = guard.as_mut() {
            if managed.child.try_wait().ok().flatten().is_none() {
                return Ok(managed.pid);
            }
            *guard = None;
        }
    }

    let resolved_python = find_python_executable(python_path.as_deref())
        .ok_or_else(|| "Python >= 3.9 not found".to_string())?;

    let accounts_dir = proxy_dir.join("accounts");
    let _ = fs::create_dir_all(&accounts_dir);

    let mut command = Command::new(&resolved_python);
    command.current_dir(&proxy_dir);
    command.arg(&script_path);
    command.arg("--host").arg(&selected_host);
    command.arg("--port").arg(selected_port.to_string());
    command.arg("--accounts-dir").arg(&accounts_dir);

    if let Some(ref key) = api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            command.arg("--api-key").arg(trimmed);
        }
    }

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to launch WorkBuddy companion proxy: {e}"))?;

    let pid = child.id();

    #[cfg(windows)]
    let job = attach_child_to_windows_job(&child)?;

    {
        let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
        *guard = Some(ManagedWorkBuddyProxyProcess {
            child,
            #[cfg(windows)]
            job,
            pid,
            host: selected_host,
            port: selected_port,
        });
    }

    Ok(pid)
}

pub(crate) fn stop_workbuddy_proxy_sync(
    proxy_state: &WorkBuddyProxyState,
) -> Result<(), String> {
    let mut guard = proxy_state.process.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}
