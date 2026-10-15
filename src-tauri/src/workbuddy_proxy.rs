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

/// Port the companion proxy actually bound. The preferred port may be taken by
/// another process, in which case the proxy falls back to a free one and the
/// kernel config must be pointed at the real port rather than the default.
static ACTIVE_WORKBUDDY_PORT: std::sync::atomic::AtomicU16 =
    std::sync::atomic::AtomicU16::new(DEFAULT_WORKBUDDY_PORT);

pub(crate) fn active_workbuddy_port() -> u16 {
    ACTIVE_WORKBUDDY_PORT.load(std::sync::atomic::Ordering::Relaxed)
}

fn set_active_workbuddy_port(port: u16) {
    ACTIVE_WORKBUDDY_PORT.store(port, std::sync::atomic::Ordering::Relaxed);
}

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
    let payload = if let Some(path) = target_path {
        json!({ "path": path })
    } else {
        json!({ "all": true })
    };
    workbuddy_gateway_post(proxy_state.inner(), "/accounts/import/desktop", payload).await
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
    proxy_state: State<'_, WorkBuddyProxyState>,
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
    let client = Client::builder().timeout(Duration::from_secs(12)).build().unwrap_or_default();

    let mut req = client.get(&url);
    if selected_realm == "cn" {
        req = req
            .header("User-Agent", "WorkBuddy/5.5.6")
            .header("Origin", "https://www.codebuddy.cn")
            .header("Referer", "https://www.codebuddy.cn/")
            .header("X-Requested-With", "XMLHttpRequest");
    } else {
        req = req
            .header("User-Agent", "WorkBuddy/5.5.2")
            .header("Origin", "https://www.workbuddy.ai")
            .header("Referer", "https://www.workbuddy.ai/")
            .header("X-Requested-With", "XMLHttpRequest");
    }

    let response = req
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
        return Ok(WorkBuddyOAuthPollResult {
            status: "waiting".to_string(),
            message: Some("Waiting for authorization...".to_string()),
            uid: None,
            access_token: None,
            refresh_token: None,
            expires_in: None,
        });
    }

    // Extract claims from JWT
    let claims = extract_jwt_claims(&access_token);
    let mut uid = claims
        .uid
        .unwrap_or_else(|| format!("wb_{}", &state[..8.min(state.len())]));
    let mut email = claims.email;
    let mut nickname = claims.name;

    // If nickname or email is missing, query the account profile endpoint
    if nickname.is_none() || email.is_none() {
        let acct_url = format!("{upstream}/v2/plugin/login/account?state={state}");
        let mut acct_req = client
            .get(&acct_url)
            .header("Authorization", format!("Bearer {access_token}"));
        if selected_realm == "cn" {
            acct_req = acct_req
                .header("User-Agent", "WorkBuddy/5.5.6")
                .header("Origin", "https://www.codebuddy.cn")
                .header("Referer", "https://www.codebuddy.cn/")
                .header("X-Requested-With", "XMLHttpRequest");
        } else {
            acct_req = acct_req
                .header("User-Agent", "WorkBuddy/5.5.2")
                .header("Origin", "https://www.workbuddy.ai")
                .header("Referer", "https://www.workbuddy.ai/")
                .header("X-Requested-With", "XMLHttpRequest");
        }

        if let Ok(resp) = acct_req.send().await {
            if let Ok(profile) = resp.json::<Value>().await {
                let pdata = profile.get("data");
                if nickname.is_none() {
                    nickname = pdata
                        .and_then(|d| d.get("nickname"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                let phone = pdata
                    .and_then(|d| d.get("phoneNumber"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let uin = pdata.and_then(|d| d.get("uin")).and_then(|v| {
                    v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string()))
                });
                if email.is_none() {
                    email = pdata
                        .and_then(|d| d.get("email"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or(phone)
                        .or(uin);
                }
                if let Some(prof_uid) = pdata.and_then(|d| d.get("uid")).and_then(Value::as_str) {
                    if !prof_uid.trim().is_empty() {
                        uid = prof_uid.to_string();
                    }
                }
            }
        }
    }

    // Determine clear, human-readable display label
    let display_label = match (&nickname, &email) {
        (Some(nick), Some(id)) if !nick.is_empty() && !id.is_empty() && nick != id => {
            format!("{nick} ({id})")
        }
        (_, Some(id)) if !id.is_empty() => id.clone(),
        (Some(nick), _) if !nick.is_empty() => format!("{nick} ({})", &uid[..8.min(uid.len())]),
        _ => format!("WorkBuddy ({})", &uid[..8.min(uid.len())]),
    };

    let now_sec = chrono::Utc::now().timestamp();
    let expires_at = expires_in.map(|exp| now_sec + exp).unwrap_or(now_sec + 2592000);

    let account_record = json!({
        "uid": uid,
        "nickname": nickname.clone().unwrap_or_default(),
        "email": email.clone().unwrap_or_default(),
        "account": display_label,
        "label": display_label,
        "realm": selected_realm,
        "platform": "CLI",
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "expiresAt": expires_at,
        "source": "oauth",
        "enabled": true
    });

    // 1. Save into workbuddy-proxy accounts directory
    if let Ok(proxy_dir) = ensure_workbuddy_proxy_dir() {
        let accounts_dir = proxy_dir.join("accounts");
        let account_file = accounts_dir.join(format!("{uid}.json"));
        let _ = fs::write(account_file, serde_json::to_string_pretty(&account_record).unwrap_or_default());
    }

    // Import into running companion gateway in-memory pool immediately
    let _ = workbuddy_gateway_post(
        proxy_state.inner(),
        "/accounts/import",
        json!({
            "format": "workbuddy-accounts",
            "accounts": [account_record.clone()],
            "overwrite": true
        }),
    )
    .await;

    // Trigger credit refresh immediately so credits are available
    let _ = workbuddy_gateway_post(
        proxy_state.inner(),
        "/accounts/credits",
        json!({ "uid": uid, "refresh": true }),
    )
    .await;

    // 2. Also persist into the kernel's credentials directory so the account
    //    survives a restart. auth_dir is stored relative to the install dir;
    //    resolving it against the GUI process CWD would write to the wrong place.
    if let Ok(config) = gui_config_state.snapshot() {
        let install_dir = core_install_dir()?;
        if let Ok(auth_dir) = auth_dir_path_for_core(&config.auth_dir, &install_dir) {
            if fs::create_dir_all(&auth_dir).is_ok() {
                let auth_file_path = auth_dir.join(format!("workbuddy-{uid}.json"));
                let cpa_auth_record = json!({
                    "name": format!("workbuddy-{uid}.json"),
                    "type": "workbuddy",
                    "account_type": "oauth",
                    "auth_kind": "oauth",
                    "uid": uid,
                    "email": email.clone().unwrap_or_default(),
                    "nickname": nickname.clone().unwrap_or_default(),
                    "account": display_label,
                    "label": display_label,
                    "access_token": access_token,
                    "refresh_token": refresh_token,
                    "token_type": "Bearer",
                    "expires_in": expires_in,
                    "expires_at": expires_at,
                    "realm": selected_realm,
                    "platform": "CLI",
                    "source": "oauth",
                    "created_at": chrono::Utc::now().to_rfc3339()
                });
                fs::write(&auth_file_path, serde_json::to_string_pretty(&cpa_auth_record).unwrap_or_default())
                    .map_err(|e| format!("Failed to persist WorkBuddy credential to {}: {e}", path_to_string(&auth_file_path)))?;
            }
        }
    }

    Ok(WorkBuddyOAuthPollResult {
        status: "success".to_string(),
        message: Some("Authorization successful".to_string()),
        uid: Some(display_label),
        access_token: Some(access_token),
        refresh_token,
        expires_in,
    })
}

struct WorkBuddyJwtClaims {
    uid: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

fn extract_jwt_claims(jwt: &str) -> WorkBuddyJwtClaims {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return WorkBuddyJwtClaims {
            uid: None,
            email: None,
            name: None,
        };
    }
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let decoded = engine
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(parts[1]))
        .ok();
    let payload: Option<Value> = decoded.and_then(|d| serde_json::from_slice(&d).ok());

    let uid = payload.as_ref().and_then(|p| {
        p.get("uid")
            .or_else(|| p.get("sub"))
            .or_else(|| p.get("userId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });

    let email = payload.as_ref().and_then(|p| {
        p.get("email")
            .or_else(|| p.get("preferred_username"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });

    let name = payload.as_ref().and_then(|p| {
        p.get("name")
            .or_else(|| p.get("nickname"))
            .or_else(|| p.get("given_name"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });

    WorkBuddyJwtClaims { uid, email, name }
}

pub(crate) const WORKBUDDY_MODEL_NAMES: &[&str] = &[
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

/// Injects or refreshes the workbuddy openai-compatibility group in a v8 config document.
///
/// Kernel v8 validates every `openai-compatibility` entry with `expandV8Groups`, which
/// rejects any group whose `keys` is missing or is not a sequence. Older builds of this
/// GUI wrote a bare `api-key` field instead, so this also repairs such legacy entries.
pub(crate) fn apply_workbuddy_provider(
    doc: &mut serde_norway::Value,
    port: u16,
    api_key: Option<&str>,
) -> Result<(), String> {
    let base_url = format!("http://127.0.0.1:{port}/v1");
    let key_value = api_key.unwrap_or_default();

    let models_json: Vec<Value> = WORKBUDDY_MODEL_NAMES
        .iter()
        .map(|m| json!({ "name": m }))
        .collect();

    // Kernel v8 requires every openai-compatibility group to carry a `keys` sequence;
    // a bare `api-key` field makes the whole config unloadable.
    let new_provider_group = json!({
        "name": "workbuddy",
        "base-url": base_url,
        "keys": [ { "api-key": key_value } ],
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

    // Repair any pre-existing provider group that was written without `keys`.
    for item in groups_seq.iter_mut() {
        let Some(map) = item.as_mapping_mut() else { continue };
        if map.contains_key(&yaml_key("keys")) {
            continue;
        }
        let legacy_key = map
            .remove(&yaml_key("api-key"))
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        map.insert(
            yaml_key("keys"),
            serde_norway::Value::Sequence(vec![serde_norway::Value::Mapping({
                let mut entry = serde_norway::Mapping::new();
                entry.insert(yaml_key("api-key"), serde_norway::Value::String(legacy_key));
                entry
            })]),
        );
    }

    Ok(())
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

    apply_workbuddy_provider(&mut doc, port, api_key)?;

    let new_content = serde_norway::to_string(&doc)
        .map_err(|e| format!("Failed to render updated YAML: {e}"))?;

    write_core_config_if_changed(&config_path, &new_content)?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    Ok(format!("WorkBuddy successfully synchronized as OpenAI-compatible provider at {base_url}"))
}

#[tauri::command]
pub(crate) async fn sync_workbuddy_provider_to_cpa(
    port: Option<u16>,
    api_key: Option<String>,
) -> Result<String, String> {
    sync_workbuddy_provider_to_cpa_sync(port.unwrap_or(DEFAULT_WORKBUDDY_PORT), api_key.as_deref())
}

/// True when nothing is listening on the host/port pair.
///
/// A foreign process holding the port would otherwise swallow our requests and
/// answer with its own credentials, which surfaces as an unexplained rejection.
fn workbuddy_port_is_available(host: &str, port: u16) -> bool {
    use std::net::TcpListener;
    let bind_host = if host == "0.0.0.0" { "0.0.0.0" } else { host };
    TcpListener::bind((bind_host, port)).is_ok()
}

/// Picks the preferred port when free, otherwise the first free port at or above it.
fn select_workbuddy_port(host: &str, preferred: u16) -> Option<u16> {
    if workbuddy_port_is_available(host, preferred) {
        return Some(preferred);
    }
    (preferred + 1..=preferred.saturating_add(64)).find(|port| workbuddy_port_is_available(host, *port))
}

/// Resolves and records the port the companion proxy will use.
///
/// Runs before the kernel config is written so the provider entry and the proxy
/// agree on the same port even when the preferred one is occupied.
pub(crate) fn reserve_workbuddy_port(preferred: u16, host: &str) -> u16 {
    let port = select_workbuddy_port(host, preferred).unwrap_or(preferred);
    set_active_workbuddy_port(port);
    port
}

pub(crate) fn start_workbuddy_proxy_sync(
    proxy_state: &WorkBuddyProxyState,
) -> Result<u32, String> {
    let proxy_dir = ensure_workbuddy_proxy_dir()?;
    let script_path = proxy_dir.join("wb_proxy.py");
    if !script_path.is_file() {
        return Err(format!("wb_proxy.py not found at {}", path_to_string(&script_path)));
    }

    // Prefer an explicit port, then any port already reserved for this start,
    // and only then the built-in default.
    let preferred_port = proxy_state
        .port
        .lock()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(active_workbuddy_port);
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

    // A foreign WorkBuddy gateway may already hold the preferred port. Binding
    // would fail silently and the kernel would then talk to that other process,
    // so pick a free port and record it for routing and provider sync.
    let selected_port = select_workbuddy_port(&selected_host, preferred_port).ok_or_else(|| {
        format!(
            "No free port available for the WorkBuddy companion proxy near {preferred_port}. Stop the process using that port and retry"
        )
    })?;
    if selected_port != preferred_port {
        eprintln!(
            "WorkBuddy companion proxy: port {preferred_port} is in use by another process; using {selected_port} instead"
        );
    }
    *proxy_state.port.lock().map_err(|e| e.to_string())? = Some(selected_port);
    set_active_workbuddy_port(selected_port);

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

    // Keep the companion's output on disk: a bind failure (port taken) would
    // otherwise be invisible and surface later as an unexplained rejection.
    let log_file = fs::File::create(proxy_dir.join("companion.log"));
    command.stdin(Stdio::null());
    match log_file {
        Ok(stdout_file) => {
            let stderr_file = stdout_file.try_clone().ok();
            command.stdout(Stdio::from(stdout_file));
            command.stderr(stderr_file.map(Stdio::from).unwrap_or_else(Stdio::null));
        }
        Err(_) => {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
        }
    }

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

// ---------------------------------------------------------------------------
// Account tasks (daily check-in, activity tasks, cat travel) served by the
// companion gateway. Every call goes through the same management endpoints the
// bundled dashboard uses, so results match what the web panel reports.
// ---------------------------------------------------------------------------

fn workbuddy_gateway_origin(proxy_state: &WorkBuddyProxyState) -> Result<String, String> {
    let port = proxy_state
        .port
        .lock()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(active_workbuddy_port);
    let host = proxy_state
        .host
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or_else(|| DEFAULT_WORKBUDDY_HOST.to_string());
    Ok(format!("http://{host}:{port}"))
}

fn workbuddy_gateway_key(proxy_state: &WorkBuddyProxyState) -> Option<String> {
    proxy_state
        .api_key
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

async fn workbuddy_obtain_panel_token(client: &Client, origin: &str) -> Option<String> {
    let login_url = format!("{origin}/panel/login");
    let resp = client
        .post(&login_url)
        .json(&json!({ "password": "admin" }))
        .send()
        .await
        .ok()?;
    let payload: Value = resp.json().await.ok()?;
    payload.get("token").and_then(Value::as_str).map(str::to_string)
}

async fn workbuddy_gateway_post(
    proxy_state: &WorkBuddyProxyState,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let origin = workbuddy_gateway_origin(proxy_state)?;
    let url = format!("{origin}{path}");
    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client.post(&url).json(&body);
    if let Some(key) = workbuddy_gateway_key(proxy_state) {
        request = request.header("Authorization", format!("Bearer {key}"));
    }

    let mut response = request.send().await.map_err(|e| {
        format!("Failed to reach the WorkBuddy gateway at {url}. Make sure the kernel is running: {e}")
    })?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        if let Some(panel_token) = workbuddy_obtain_panel_token(&client, &origin).await {
            let mut retry_request = client
                .post(&url)
                .json(&body)
                .header("X-Panel-Token", &panel_token);
            if let Some(key) = workbuddy_gateway_key(proxy_state) {
                retry_request = retry_request.header("Authorization", format!("Bearer {key}"));
            }
            if let Ok(retry_resp) = retry_request.send().await {
                response = retry_resp;
            }
        }
    }

    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|e| format!("The WorkBuddy gateway returned an unreadable response: {e}"))?;

    if !status.is_success() {
        let message = payload
            .get("msg")
            .or_else(|| payload.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return Err(format!("WorkBuddy gateway error ({status}): {message}"));
    }

    Ok(payload)
}

async fn workbuddy_gateway_get(
    proxy_state: &WorkBuddyProxyState,
    path: &str,
) -> Result<Value, String> {
    let origin = workbuddy_gateway_origin(proxy_state)?;
    let url = format!("{origin}{path}");
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client.get(&url);
    if let Some(key) = workbuddy_gateway_key(proxy_state) {
        request = request.header("Authorization", format!("Bearer {key}"));
    }

    let mut response = request.send().await.map_err(|e| {
        format!("Failed to reach the WorkBuddy gateway at {url}. Make sure the kernel is running: {e}")
    })?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        if let Some(panel_token) = workbuddy_obtain_panel_token(&client, &origin).await {
            let mut retry_request = client
                .get(&url)
                .header("X-Panel-Token", &panel_token);
            if let Some(key) = workbuddy_gateway_key(proxy_state) {
                retry_request = retry_request.header("Authorization", format!("Bearer {key}"));
            }
            if let Ok(retry_resp) = retry_request.send().await {
                response = retry_resp;
            }
        }
    }

    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|e| format!("The WorkBuddy gateway returned an unreadable response: {e}"))?;

    if !status.is_success() {
        let message = payload
            .get("msg")
            .or_else(|| payload.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return Err(format!("WorkBuddy gateway error ({status}): {message}"));
    }

    Ok(payload)
}

async fn sync_auth_dir_accounts_to_companion(
    proxy_state: &WorkBuddyProxyState,
    auth_dir: &Path,
) {
    if !auth_dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(auth_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut to_import = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("workbuddy-") || !name.ends_with(".json") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                let token = val
                    .get("access_token")
                    .or_else(|| val.get("accessToken"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if token.is_empty() {
                    continue;
                }
                let claims = extract_jwt_claims(token);
                let uid = val
                    .get("uid")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(claims.uid)
                    .unwrap_or_default();
                let email = val
                    .get("email")
                    .or_else(|| val.get("account"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(claims.email)
                    .unwrap_or_default();
                let nickname = val
                    .get("nickname")
                    .or_else(|| val.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(claims.name)
                    .unwrap_or_default();
                let realm = val
                    .get("realm")
                    .and_then(Value::as_str)
                    .unwrap_or("intl");
                let refresh_token = val
                    .get("refresh_token")
                    .or_else(|| val.get("refreshToken"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                to_import.push(json!({
                    "uid": uid,
                    "nickname": if !nickname.is_empty() { nickname.clone() } else { email.clone() },
                    "email": email,
                    "realm": realm,
                    "accessToken": token,
                    "refreshToken": refresh_token,
                    "platform": "CLI",
                    "source": "oauth",
                    "enabled": val.get("disabled").and_then(Value::as_bool).map(|d| !d).unwrap_or(true),
                }));
            }
        }
    }
    if !to_import.is_empty() {
        let _ = workbuddy_gateway_post(
            proxy_state,
            "/accounts/import",
            json!({
                "format": "workbuddy-accounts",
                "accounts": to_import,
                "overwrite": false
            }),
        )
        .await;
    }
}

fn enrich_accounts_with_auth_metadata(
    gui_config_state: &GuiConfigState,
    val: &mut Value,
) {
    let accounts = match val.get_mut("accounts").and_then(Value::as_array_mut) {
        Some(accs) => accs,
        None => match val.as_array_mut() {
            Some(accs) => accs,
            None => return,
        },
    };
    let config = match gui_config_state.snapshot() {
        Ok(c) => c,
        Err(_) => return,
    };
    let install_dir = match core_install_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let auth_dir = match auth_dir_path_for_core(&config.auth_dir, &install_dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    if !auth_dir.is_dir() {
        return;
    }
    let mut meta_map: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    if let Ok(entries) = fs::read_dir(&auth_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("workbuddy-") || !name.ends_with(".json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<Value>(&content) {
                    let uid = v.get("uid").and_then(Value::as_str).unwrap_or("");
                    let email = v.get("email").or_else(|| v.get("account")).and_then(Value::as_str).unwrap_or("");
                    let nickname = v.get("nickname").or_else(|| v.get("name")).and_then(Value::as_str).unwrap_or("");
                    if !uid.is_empty() {
                        meta_map.insert(uid.to_string(), (email.to_string(), nickname.to_string()));
                    }
                }
            }
        }
    }
    for acc in accounts.iter_mut() {
        let uid = acc.get("uid").and_then(Value::as_str).unwrap_or("").to_string();
        if let Some((email, nickname)) = meta_map.get(&uid) {
            if let Some(map) = acc.as_object_mut() {
                if !email.is_empty() {
                    map.insert("email".to_string(), json!(email));
                }
                let current_nick = map.get("nickname").and_then(Value::as_str).unwrap_or("");
                if (current_nick.is_empty() || current_nick == &uid[..8.min(uid.len())]) && !nickname.is_empty() {
                    map.insert("nickname".to_string(), json!(nickname));
                } else if current_nick.is_empty() && !email.is_empty() {
                    map.insert("nickname".to_string(), json!(email));
                }
            }
        }
    }
}

fn read_fallback_workbuddy_accounts(gui_config_state: &GuiConfigState) -> Vec<Value> {
    let mut result = Vec::new();
    let config = match gui_config_state.snapshot() {
        Ok(c) => c,
        Err(_) => return result,
    };
    let install_dir = match core_install_dir() {
        Ok(d) => d,
        Err(_) => return result,
    };
    let auth_dir = match auth_dir_path_for_core(&config.auth_dir, &install_dir) {
        Ok(d) => d,
        Err(_) => return result,
    };
    if !auth_dir.is_dir() {
        return result;
    }
    if let Ok(entries) = fs::read_dir(&auth_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("workbuddy-") || !name.ends_with(".json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<Value>(&content) {
                    let uid = v.get("uid").and_then(Value::as_str).unwrap_or("");
                    let email = v.get("email").or_else(|| v.get("account")).and_then(Value::as_str).unwrap_or("");
                    let nickname = v.get("nickname").or_else(|| v.get("name")).and_then(Value::as_str).unwrap_or("");
                    let realm = v.get("realm").and_then(Value::as_str).unwrap_or("intl");
                    result.push(json!({
                        "uid": uid,
                        "nickname": if !nickname.is_empty() { nickname } else if !email.is_empty() { email } else { &uid[..8.min(uid.len())] },
                        "email": email,
                        "realm": realm,
                        "enabled": v.get("disabled").and_then(Value::as_bool).map(|d| !d).unwrap_or(true),
                        "credits": null
                    }));
                }
            }
        }
    }
    result
}

/// Lists accounts known to the companion gateway, with realm and readiness.
#[tauri::command]
pub(crate) async fn get_workbuddy_accounts(
    gui_config_state: State<'_, GuiConfigState>,
    proxy_state: State<'_, WorkBuddyProxyState>,
) -> Result<Value, String> {
    // 1. Sync accounts from auth_dir to companion gateway
    if let Ok(config) = gui_config_state.snapshot() {
        if let Ok(install_dir) = core_install_dir() {
            if let Ok(auth_dir) = auth_dir_path_for_core(&config.auth_dir, &install_dir) {
                sync_auth_dir_accounts_to_companion(proxy_state.inner(), &auth_dir).await;
            }
        }
    }

    // 2. Query companion gateway
    let payload = workbuddy_gateway_get(proxy_state.inner(), "/accounts?realm=all").await;
    match payload {
        Ok(mut val) => {
            enrich_accounts_with_auth_metadata(gui_config_state.inner(), &mut val);
            Ok(val)
        }
        Err(err) => {
            let fallback = read_fallback_workbuddy_accounts(gui_config_state.inner());
            if !fallback.is_empty() {
                Ok(json!({ "accounts": fallback, "offline": true, "gateway_error": err }))
            } else {
                Err(err)
            }
        }
    }
}

/// Refreshes upstream credits/quotas for WorkBuddy accounts.
#[tauri::command]
pub(crate) async fn refresh_workbuddy_credits(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    let mut body = json!({ "refresh": true });
    if let Some(uid) = uid.filter(|value| !value.trim().is_empty()) {
        body["uid"] = json!(uid);
    }
    workbuddy_gateway_post(proxy_state.inner(), "/accounts/credits", body).await
}

/// Runs one task endpoint for an optional single account.
async fn run_workbuddy_task_endpoint(
    proxy_state: &WorkBuddyProxyState,
    path: &str,
    uid: Option<&str>,
) -> Result<Value, String> {
    let mut body = json!({});
    if let Some(uid) = uid.filter(|value| !value.trim().is_empty()) {
        body["uid"] = json!(uid);
    }
    workbuddy_gateway_post(proxy_state, path, body).await
}

/// Runs the daily check-in for one account, or every eligible account when uid is empty.
#[tauri::command]
pub(crate) async fn run_workbuddy_checkin(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    run_workbuddy_task_endpoint(proxy_state.inner(), "/accounts/checkin", uid.as_deref()).await
}

/// Runs the international-realm daily activity chat.
#[tauri::command]
pub(crate) async fn run_workbuddy_daily_activity(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    run_workbuddy_task_endpoint(proxy_state.inner(), "/accounts/daily-chat", uid.as_deref()).await
}

/// Runs the one-click growth task bundle (credits, growth, sign-in streaks).
#[tauri::command]
pub(crate) async fn run_workbuddy_growth_tasks(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    run_workbuddy_task_endpoint(proxy_state.inner(), "/tasks/run", uid.as_deref()).await
}

/// Runs the streak keeper and lottery bonus task.
#[tauri::command]
pub(crate) async fn run_workbuddy_streak_bonus(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    run_workbuddy_task_endpoint(proxy_state.inner(), "/tasks/streak-bonus", uid.as_deref()).await
}

/// Runs the cat travel task.
#[tauri::command]
pub(crate) async fn run_workbuddy_travel(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    run_workbuddy_task_endpoint(proxy_state.inner(), "/tasks/travel", uid.as_deref()).await
}

/// Task definitions shared by the one-click runner and the UI.
const WORKBUDDY_TASK_STEPS: &[(&str, &str, &str)] = &[
    ("checkin", "每日签到", "/accounts/checkin"),
    ("daily-activity", "每日活跃打卡", "/accounts/daily-chat"),
    ("growth", "一键成长任务", "/tasks/run"),
    ("streak-bonus", "连登管家与抽奖", "/tasks/streak-bonus"),
    ("travel", "猫猫旅行", "/tasks/travel"),
];

/// Runs every supported task in sequence for the given account (or all accounts).
/// Returns one aggregated result so the UI can render a single completion report.
#[tauri::command]
pub(crate) async fn run_workbuddy_all_tasks(
    proxy_state: State<'_, WorkBuddyProxyState>,
    uid: Option<String>,
) -> Result<Value, String> {
    let target = uid.as_deref().filter(|value| !value.trim().is_empty());
    let mut steps: Vec<Value> = Vec::with_capacity(WORKBUDDY_TASK_STEPS.len());

    for (task, label, path) in WORKBUDDY_TASK_STEPS {
        let result = run_workbuddy_task_endpoint(proxy_state.inner(), path, target)
            .await
            .unwrap_or_else(|error| json!({ "ok": false, "msg": error }));
        steps.push(json!({ "task": task, "label": label, "result": result }));
    }

    Ok(json!({ "steps": steps }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V8_TEMPLATE: &str = "config-version: 8\nserver:\n  host: 127.0.0.1\n  port: 8319\n";

    fn provider_groups(doc: &serde_norway::Value) -> &Vec<serde_norway::Value> {
        doc["api-keys"]["openai-compatibility"]
            .as_sequence()
            .expect("openai-compatibility must be a sequence")
    }

    // The credential was previously written to a path resolved against the GUI process
    // CWD, so it never reached the kernel's auth directory and the login was lost on
    // restart. The relative auth-dir must resolve under the install directory.
    #[test]
    fn resolves_relative_auth_dir_under_install_dir() {
        let install_dir = Path::new("/opt/cpa-core");
        let resolved = auth_dir_path_for_core("../oauth", install_dir).unwrap();
        assert_eq!(resolved, Path::new("/opt/oauth"));
    }

    #[test]
    fn resolves_absolute_auth_dir_unchanged() {
        let install_dir = Path::new("/opt/cpa-core");
        let resolved = auth_dir_path_for_core("/var/lib/cpa/oauth", install_dir).unwrap();
        assert_eq!(resolved, Path::new("/var/lib/cpa/oauth"));
    }

    // A foreign process on the preferred port used to swallow every request and
    // answer with its own key, surfacing as an unexplained rejection. The proxy
    // must move to a free port instead of failing silently.
    #[test]
    fn selects_preferred_port_when_free() {
        use std::net::TcpListener;
        // Bind an ephemeral port, then release it so it is almost certainly free.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(select_workbuddy_port("127.0.0.1", port), Some(port));
    }

    #[test]
    fn falls_back_to_a_free_port_when_preferred_is_taken() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let selected = select_workbuddy_port("127.0.0.1", port);
        assert!(selected.is_some(), "a free port must be found");
        assert_ne!(selected, Some(port), "the occupied port must not be reused");
    }

    // Kernel v8 `expandV8Groups` rejects a group whose `keys` is absent, which makes the
    // entire config unloadable and the kernel exit immediately. Any injected entry must
    // therefore always carry a `keys` sequence.
    #[test]
    fn injects_workbuddy_group_with_keys_sequence() {
        let mut doc: serde_norway::Value = serde_norway::from_str(V8_TEMPLATE).unwrap();
        apply_workbuddy_provider(&mut doc, 8788, None).unwrap();

        let groups = provider_groups(&doc);
        assert_eq!(groups.len(), 1, "exactly one injected provider group");
        let group = groups[0].as_mapping().unwrap();
        assert_eq!(group[&yaml_key("name")].as_str(), Some("workbuddy"));
        assert_eq!(
            group[&yaml_key("base-url")].as_str(),
            Some("http://127.0.0.1:8788/v1")
        );
        assert!(
            group[&yaml_key("keys")].as_sequence().is_some(),
            "keys must be a sequence"
        );
        assert!(
            !group.contains_key(&yaml_key("api-key")),
            "v8 groups must not use the legacy bare api-key field"
        );
    }

    #[test]
    fn populates_all_supported_models() {
        let mut doc: serde_norway::Value = serde_norway::from_str(V8_TEMPLATE).unwrap();
        apply_workbuddy_provider(&mut doc, 8788, None).unwrap();

        let group = provider_groups(&doc)[0].as_mapping().unwrap();
        let names: Vec<&str> = group[&yaml_key("models")]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|m| m[&yaml_key("name")].as_str().unwrap())
            .collect();
        assert_eq!(names, WORKBUDDY_MODEL_NAMES.to_vec());
    }

    #[test]
    fn is_idempotent_and_refreshes_in_place() {
        let mut doc: serde_norway::Value = serde_norway::from_str(V8_TEMPLATE).unwrap();
        apply_workbuddy_provider(&mut doc, 8788, None).unwrap();
        apply_workbuddy_provider(&mut doc, 9999, Some("secret")).unwrap();

        let groups = provider_groups(&doc);
        assert_eq!(groups.len(), 1, "refresh must not duplicate the group");
        let group = groups[0].as_mapping().unwrap();
        assert_eq!(
            group[&yaml_key("base-url")].as_str(),
            Some("http://127.0.0.1:9999/v1")
        );
        let key = &group[&yaml_key("keys")].as_sequence().unwrap()[0];
        assert_eq!(key[&yaml_key("api-key")].as_str(), Some("secret"));
    }

    // Configs written by earlier GUI builds used a bare `api-key`, which the kernel
    // refuses to load. The repair path must convert them so a start succeeds again.
    #[test]
    fn repairs_legacy_group_written_with_bare_api_key() {
        let legacy = "config-version: 8\napi-keys:\n  openai-compatibility:\n  - name: workbuddy\n    base-url: http://127.0.0.1:8788/v1\n    api-key: ''\n    models:\n    - name: default-model\n";
        let mut doc: serde_norway::Value = serde_norway::from_str(legacy).unwrap();
        apply_workbuddy_provider(&mut doc, 8788, None).unwrap();

        let groups = provider_groups(&doc);
        assert_eq!(groups.len(), 1);
        let group = groups[0].as_mapping().unwrap();
        assert!(group[&yaml_key("keys")].as_sequence().is_some());
        assert!(!group.contains_key(&yaml_key("api-key")));
    }

    #[test]
    fn preserves_unrelated_openai_compat_providers() {
        let existing = "config-version: 8\napi-keys:\n  openai-compatibility:\n  - name: openrouter\n    base-url: https://openrouter.ai/api/v1\n    keys:\n    - api-key: sk-or\n";
        let mut doc: serde_norway::Value = serde_norway::from_str(existing).unwrap();
        apply_workbuddy_provider(&mut doc, 8788, None).unwrap();

        let names: Vec<&str> = provider_groups(&doc)
            .iter()
            .map(|g| g[&yaml_key("name")].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["openrouter", "workbuddy"]);
    }
}
