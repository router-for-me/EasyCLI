// Tauri v2 backend for EasyCLI
// Ports core Electron main.js logic to Rust with a simpler API surface (KISS)

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{self, Read, Write, BufRead, BufReader};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::Duration;
use tokio::time::sleep;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use std::io::Cursor;
use tauri::tray::TrayIcon;
use thiserror::Error;
use tauri::WindowEvent;
use rfd::FileDialog;

static PROCESS: Lazy<Arc<Mutex<Option<Child>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));
static TRAY_ICON: Lazy<Arc<Mutex<Option<TrayIcon>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));
static CALLBACK_SERVERS: Lazy<Arc<Mutex<HashMap<u16, (Arc<AtomicBool>, thread::JoinHandle<()>)>>>> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

#[derive(Error, Debug)]
enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Other: {0}")]
    Other(String),
}

fn home_dir() -> Result<PathBuf, AppError> {
    home::home_dir().ok_or_else(|| AppError::Other("Failed to resolve home directory".into()))
}

fn app_dir() -> Result<PathBuf, AppError> {
    Ok(home_dir()?.join("cliproxyapi"))
}

fn resolve_path(input: &str, base: Option<&Path>) -> PathBuf {
    if input.is_empty() {
        return PathBuf::new();
    }
    if input.starts_with('~') {
        if let Some(h) = home::home_dir() {
            if input == "~" {
                return h;
            }
            if input.starts_with("~/") {
                return h.join(&input[2..]);
            }
            return h.join(&input[1..]);
        }
    }
    let p = PathBuf::from(input);
    if p.is_absolute() {
        return p;
    }
    if let Some(base) = base {
        return base.join(p);
    }
    p
}

#[derive(Serialize, Deserialize, Debug)]
struct VersionInfo {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Serialize)]
struct OpResult {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")] error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] needsUpdate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] isLatest: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] latestVersion: Option<String>,
}

fn compare_versions(a: &str, b: &str) -> i32 {
    let pa: Vec<i32> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let pb: Vec<i32> = b.split('.').filter_map(|s| s.parse().ok()).collect();
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let va = *pa.get(i).unwrap_or(&0);
        let vb = *pb.get(i).unwrap_or(&0);
        if va > vb { return 1; }
        if va < vb { return -1; }
    }
    0
}

fn current_local_info() -> Result<Option<(String, PathBuf)>, AppError> {
    let dir = app_dir()?;
    let version_file = dir.join("version.txt");
    if !version_file.exists() { return Ok(None); }
    let ver = fs::read_to_string(&version_file)?.trim().to_string();
    let path = dir.join(&ver);
    if !path.exists() { return Ok(None); }
    Ok(Some((ver, path)))
}

fn ensure_config(version_path: &Path) -> Result<(), AppError> {
    let dir = app_dir()?;
    let config = dir.join("config.yaml");
    if config.exists() { return Ok(()); }
    let example = version_path.join("config.example.yaml");
    if example.exists() {
        fs::copy(example, &config)?;
    }
    Ok(())
}

fn parse_proxy(proxy_url: &str, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if proxy_url.is_empty() { return builder; }
    // Accept http/https/socks5
    match reqwest::Proxy::all(proxy_url) {
        Ok(p) => builder.proxy(p),
        Err(_) => builder,
    }
}

async fn fetch_latest_release(proxy_url: String) -> Result<VersionInfo, AppError> {
    let client = parse_proxy(&proxy_url, reqwest::Client::builder())
        .user_agent("EasyCLI")
        .build()?;
    let resp = client
        .get("https://api.github.com/repos/luispater/CLIProxyAPI/releases/latest")
        .header("Accept", "application/vnd.github.v3+json")
        .send().await?
        .error_for_status()?;
    Ok(resp.json::<VersionInfo>().await?)
}

#[tauri::command]
async fn check_version_and_download(window: tauri::Window, proxy_url: Option<String>) -> Result<serde_json::Value, String> {
    let proxy = proxy_url.unwrap_or_default();
    let dir = app_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let local = current_local_info().map_err(|e| e.to_string())?;
    window.emit("download-status", json!({"status": "checking"})).ok();
    let release = fetch_latest_release(proxy.clone()).await.map_err(|e| e.to_string())?;
    let latest = release.tag_name.trim_start_matches('v').to_string();

    if let Some((ver, path)) = local {
        let cmp = compare_versions(&ver, &latest);
        ensure_config(&path).map_err(|e| e.to_string())?;
        if cmp >= 0 {
            window.emit("download-status", json!({"status": "latest", "version": ver})).ok();
            return Ok(json!(OpResult{ success: true, error: None, path: Some(path.to_string_lossy().to_string()), version: Some(ver), needsUpdate: Some(false), isLatest: Some(true), latestVersion: None }));
        } else {
            window.emit("download-status", json!({"status": "update-available", "version": ver, "latest": latest})).ok();
            return Ok(json!(OpResult{ success: true, error: None, path: Some(path.to_string_lossy().to_string()), version: Some(ver), needsUpdate: Some(true), isLatest: Some(false), latestVersion: Some(latest) }));
        }
    }
    // No local found
    Ok(json!(OpResult{ success: true, error: None, path: None, version: None, needsUpdate: Some(true), isLatest: Some(false), latestVersion: Some(latest) }))
}

#[derive(Deserialize)]
struct DownloadArgs { proxy_url: Option<String> }

#[tauri::command]
async fn download_cliproxyapi(window: tauri::Window, proxy_url: Option<String>) -> Result<serde_json::Value, String> {
    let proxy = proxy_url.unwrap_or_default();
    let dir = app_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let release = fetch_latest_release(proxy.clone()).await.map_err(|e| e.to_string())?;
    let latest = release.tag_name.trim_start_matches('v').to_string();

    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let filename = match (platform, arch) {
        ("macos", "aarch64") => format!("CLIProxyAPI_{}_darwin_arm64.tar.gz", latest),
        ("macos", "x86_64") => format!("CLIProxyAPI_{}_darwin_amd64.tar.gz", latest),
        ("linux", "x86_64") => format!("CLIProxyAPI_{}_linux_amd64.tar.gz", latest),
        ("linux", "aarch64") => format!("CLIProxyAPI_{}_linux_arm64.tar.gz", latest),
        ("windows", "x86_64") => format!("CLIProxyAPI_{}_windows_amd64.zip", latest),
        ("windows", "aarch64") => format!("CLIProxyAPI_{}_windows_arm64.zip", latest),
        _ => return Err(format!("Unsupported platform: {} {}", platform, arch)),
    };
    let asset = release.assets.into_iter().find(|a| a.name == filename)
        .ok_or_else(|| format!("No suitable download file found: {}", filename))?;

    let download_path = dir.join(&filename);
    window.emit("download-status", json!({"status": "starting"})).ok();

    // Download with progress
    let client = parse_proxy(&proxy, reqwest::Client::builder()).build().map_err(|e| e.to_string())?;
    let resp = client.get(&asset.browser_download_url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Download failed, status: {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = fs::File::create(&download_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;
        let progress = if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 };
        window.emit("download-progress", json!({"progress": progress, "downloaded": downloaded, "total": total})).ok();
    }

    // Extract
    let extract_path = dir.join(&latest);
    if download_path.extension().and_then(|e| e.to_str()) == Some("zip") {
        extract_zip(&download_path, &extract_path).map_err(|e| e.to_string())?;
    } else {
        extract_targz(&download_path, &extract_path).map_err(|e| e.to_string())?;
    }
    // Save version.txt
    fs::write(dir.join("version.txt"), &latest).map_err(|e| e.to_string())?;
    // Cleanup
    let _ = fs::remove_file(&download_path);

    // Ensure config exists
    ensure_config(&extract_path).map_err(|e| e.to_string())?;

    window.emit("download-status", json!({"status": "completed", "version": latest})).ok();
    Ok(json!(OpResult{ success: true, error: None, path: Some(extract_path.to_string_lossy().to_string()), version: Some(latest), needsUpdate: None, isLatest: None, latestVersion: None }))
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dest)?;
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        let outpath = dest.join(f.mangled_name());
        if f.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() { fs::create_dir_all(p)?; }
            let mut outfile = fs::File::create(&outpath)?;
            io::copy(&mut f, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_targz(tar_gz_path: &Path, dest: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dest)?;
    let tar_gz = fs::File::open(tar_gz_path)?;
    let dec = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(dest)?;
    Ok(())
}

#[tauri::command]
fn check_secret_key() -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let config_path = dir.join("config.yaml");
    if !config_path.exists() {
        return Ok(json!({"needsPassword": true, "reason": "Config file missing"}));
    }
    let content = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let rm = value.get("remote-management").and_then(|v| v.as_mapping()).cloned();
    if let Some(map) = rm {
        if let Some(sk) = map.get(&serde_yaml::Value::from("secret-key")) {
            if sk.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false) {
                return Ok(json!({"needsPassword": false}));
            }
        }
    }
    Ok(json!({"needsPassword": true, "reason": "Missing secret-key"}))
}

#[tauri::command]
fn update_secret_key(secret_key: String) -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Err("Configuration file does not exist".into()); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let mut v: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let rm = v.as_mapping_mut().and_then(|m| m.get_mut(&serde_yaml::Value::from("remote-management")));
    if let Some(rm_val) = rm {
        if rm_val.is_null() { *rm_val = serde_yaml::Value::Mapping(Default::default()); }
    }
    let m = v.as_mapping_mut().unwrap();
    let entry = m.entry(serde_yaml::Value::from("remote-management")).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
    let map = entry.as_mapping_mut().unwrap();
    map.insert(serde_yaml::Value::from("secret-key"), serde_yaml::Value::from(secret_key));
    let out = serde_yaml::to_string(&v).map_err(|e| e.to_string())?;
    fs::write(&p, out).map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

#[tauri::command]
fn read_config_yaml() -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Ok(json!({})); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let v: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let json_v = serde_json::to_value(v).map_err(|e| e.to_string())?;
    Ok(json_v)
}

#[derive(Deserialize)]
struct UpdateConfigArgs { endpoint: String, value: serde_json::Value, isDelete: Option<bool> }

#[tauri::command]
fn update_config_yaml(endpoint: String, value: serde_json::Value, is_delete: Option<bool>) -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Err("Configuration file does not exist".into()); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let mut conf: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let parts: Vec<&str> = endpoint.split('.').collect();
    // Descend mapping
    let mut current = conf.as_mapping_mut().ok_or("Invalid config structure")?;
    for (i, part) in parts.iter().enumerate() {
        let key = serde_yaml::Value::from(*part);
        if i == parts.len() - 1 {
            if is_delete.unwrap_or(false) {
                current.remove(&key);
            } else {
                current.insert(key, serde_yaml::to_value(&value).map_err(|e| e.to_string())?);
            }
        } else {
            let entry = current.entry(key).or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
            if let Some(map) = entry.as_mapping_mut() {
                current = map;
            } else {
                return Err("Invalid nested config path".into());
            }
        }
    }
    let out = serde_yaml::to_string(&conf).map_err(|e| e.to_string())?;
    fs::write(&p, out).map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

#[tauri::command]
fn read_local_auth_files() -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Ok(json!([])); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let conf: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let auth_dir = conf.get("auth-dir").and_then(|v| v.as_str()).unwrap_or("");
    if auth_dir.is_empty() { return Ok(json!([])); }
    let base = p.parent().unwrap();
    let ad = resolve_path(auth_dir, Some(base));
    if !ad.exists() { return Ok(json!([])); }
    let mut result = vec![];
    for entry in fs::read_dir(ad).map_err(|e| e.to_string())? {
        let e = entry.map_err(|e| e.to_string())?;
        let path = e.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.to_lowercase().ends_with(".json") {
                    let meta = e.metadata().map_err(|e| e.to_string())?;
                    let mut file_type = "unknown".to_string();
                    if let Ok(mut f) = fs::File::open(&path) {
                        let mut s = String::new();
                        let _ = f.read_to_string(&mut s);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                            if let Some(t) = v.get("type").and_then(|x| x.as_str()) { file_type = t.to_string(); }
                        }
                    }
                    let mod_ms = meta.modified().ok()
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| (d.as_millis() as u64))
                        .unwrap_or(0);
                    result.push(json!({
                        "name": name,
                        "size": meta.len(),
                        "modtime": mod_ms,
                        "type": file_type
                    }));
                }
            }
        }
    }
    Ok(json!(result))
}

#[derive(Deserialize)]
struct UploadFile { name: String, content: String }

#[tauri::command]
fn upload_local_auth_files(files: Vec<UploadFile>) -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Err("Configuration file does not exist".into()); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let conf: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let auth_dir = conf.get("auth-dir").and_then(|v| v.as_str()).ok_or("auth-dir not configured in config.yaml")?;
    let base = p.parent().unwrap();
    let ad = resolve_path(auth_dir, Some(base));
    fs::create_dir_all(&ad).map_err(|e| e.to_string())?;
    let mut success = 0usize; let mut errors = vec![]; let mut error_count = 0usize;
    for f in files {
        let path = ad.join(&f.name);
        if path.exists() { errors.push(format!("{}: File already exists", f.name)); error_count += 1; continue; }
        if let Err(e) = fs::write(&path, f.content.as_bytes()) { errors.push(format!("{}: {}", f.name, e)); error_count += 1; } else { success += 1; }
    }
    Ok(json!({"success": success>0, "successCount": success, "errorCount": error_count, "errors": if errors.is_empty(){serde_json::Value::Null}else{json!(errors)} }))
}

#[tauri::command]
fn delete_local_auth_files(filenames: Vec<String>) -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Err("Configuration file does not exist".into()); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let conf: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let auth_dir = conf.get("auth-dir").and_then(|v| v.as_str()).ok_or("auth-dir not configured in config.yaml")?;
    let base = p.parent().unwrap();
    let ad = resolve_path(auth_dir, Some(base));
    if !ad.exists() { return Err("Authentication file directory does not exist".into()); }
    let mut success = 0usize; let mut error_count = 0usize;
    for name in filenames {
        let path = ad.join(&name);
        match fs::remove_file(&path) { Ok(_) => success += 1, Err(_) => error_count += 1 }
    }
    Ok(json!({"success": success>0, "successCount": success, "errorCount": error_count}))
}

#[tauri::command]
fn download_local_auth_files(filenames: Vec<String>) -> Result<serde_json::Value, String> {
    let dir = app_dir().map_err(|e| e.to_string())?;
    let p = dir.join("config.yaml");
    if !p.exists() { return Err("Configuration file does not exist".into()); }
    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let conf: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let auth_dir = conf.get("auth-dir").and_then(|v| v.as_str()).ok_or("auth-dir not configured in config.yaml")?;
    let base = p.parent().unwrap();
    let ad = resolve_path(auth_dir, Some(base));
    if !ad.exists() { return Err("Authentication file directory does not exist".into()); }
    let mut files = vec![]; let mut error_count = 0usize;
    for name in filenames {
        let path = ad.join(&name);
        match fs::read_to_string(&path) { Ok(c) => files.push(json!({"name": name, "content": c})), Err(_) => error_count += 1 }
    }
    Ok(json!({"success": !files.is_empty(), "files": files, "errorCount": error_count}))
}

fn find_executable(version_path: &Path) -> Option<PathBuf> {
    let mut exe = PathBuf::from("cli-proxy-api");
    if cfg!(target_os = "windows") { exe.set_extension("exe"); }
    let path = version_path.join(exe);
    if path.exists() { Some(path) } else { None }
}

fn start_monitor(app: tauri::AppHandle) {
    let proc_ref = Arc::clone(&PROCESS);
    thread::spawn(move || {
        loop {
            let mut remove = false;
            let mut exit_code: Option<i32> = None;
            {
                let mut guard = proc_ref.lock();
                if let Some(child) = guard.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            exit_code = status.code();
                            remove = true;
                        }
                        Ok(None) => {
                            // Still running
                        }
                        Err(_) => {
                            // Treat as closed
                            remove = true;
                        }
                    }
                } else {
                    // No process
                    break;
                }
            }
            if remove {
                // Clear stored process
                *proc_ref.lock() = None;
                // Emit event
                if let Some(code) = exit_code {
                    println!("[CLIProxyAPI][EXIT] process exited with code {}", code);
                } else {
                    println!("[CLIProxyAPI][EXIT] process closed (no exit code)");
                }
                if let Some(code) = exit_code { let _ = app.emit("process-exit-error", json!({"code": code})); }
                else { let _ = app.emit("process-closed", json!({"message": "CLIProxyAPI process has closed"})); }
                // Remove tray icon when process exits
                let _ = TRAY_ICON.lock().take();
                break;
            }
            thread::sleep(Duration::from_millis(1000));
        }
    });
}

fn pipe_child_output(child: &mut Child) {
    // Pipe STDOUT
    if let Some(out) = child.stdout.take() {
        thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                match line {
                    Ok(l) => println!("[CLIProxyAPI][STDOUT] {}", l),
                    Err(e) => { eprintln!("[CLIProxyAPI][STDOUT][ERROR] {}", e); break; }
                }
            }
        });
    }
    // Pipe STDERR
    if let Some(err) = child.stderr.take() {
        thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                match line {
                    Ok(l) => eprintln!("[CLIProxyAPI][STDERR] {}", l),
                    Err(e) => { eprintln!("[CLIProxyAPI][STDERR][ERROR] {}", e); break; }
                }
            }
        });
    }
}

#[tauri::command]
fn start_cliproxyapi(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    // If running, return success
    {
        let mut guard = PROCESS.lock();
        if let Some(child) = guard.as_mut() {
            if let Ok(None) = child.try_wait() {
                return Ok(json!({"success": true, "message": "already running"}));
            }
        }
    }

    let info = current_local_info().map_err(|e| e.to_string())?;
    let (_ver, path) = info.ok_or("Version file does not exist")?;
    let exec = find_executable(&path).ok_or("Executable file does not exist")?;
    let config = app_dir().map_err(|e| e.to_string())?.join("config.yaml");
    if !config.exists() { return Err("Configuration file does not exist".into()); }

    println!("[CLIProxyAPI][START] exec: {}", exec.to_string_lossy());
    println!("[CLIProxyAPI][START] args: -config {}", config.to_string_lossy());
    let mut cmd = std::process::Command::new(&exec);
    cmd.args(["-config", config.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        eprintln!("[CLIProxyAPI][ERROR] failed to start process: {}", e);
        e.to_string()
    })?;
    // Attach output piping before storing child
    pipe_child_output(&mut child);
    *PROCESS.lock() = Some(child);
    start_monitor(app.clone());
    // Create tray icon when local process starts
    let _ = create_tray(&app);
    Ok(json!({"success": true}))
}

#[tauri::command]
fn restart_cliproxyapi(app: tauri::AppHandle) -> Result<(), String> {
    // Stop existing
    if let Some(mut child) = PROCESS.lock().take() { let _ = child.kill(); }
    // Start new using current version
    let info = current_local_info().map_err(|e| e.to_string())?;
    let (ver, path) = info.ok_or("Version file does not exist")?;
    let exec = find_executable(&path).ok_or("Executable file does not exist")?;
    let config = app_dir().map_err(|e| e.to_string())?.join("config.yaml");
    if !config.exists() { return Err("Configuration file does not exist".into()); }
    println!("[CLIProxyAPI][RESTART] exec: {}", exec.to_string_lossy());
    println!("[CLIProxyAPI][RESTART] args: -config {}", config.to_string_lossy());
    let mut cmd = std::process::Command::new(&exec);
    cmd.args(["-config", config.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        eprintln!("[CLIProxyAPI][ERROR] failed to restart process: {}", e);
        e.to_string()
    })?;
    pipe_child_output(&mut child);
    *PROCESS.lock() = Some(child);
    start_monitor(app.clone());
    if let Some(w) = app.get_webview_window("main") { let _ = w.emit("cliproxyapi-restarted", json!({"version": ver})); }
    Ok(())
}

fn stop_process_internal() {
    if let Some(mut child) = PROCESS.lock().take() { let _ = child.kill(); }
}

fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::{menu::{MenuBuilder, MenuItemBuilder}, tray::TrayIconBuilder};
    let mut guard = TRAY_ICON.lock();
    if guard.is_some() { return Ok(()); }

    let open_settings = MenuItemBuilder::with_id("open_settings", "Open Settings").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&open_settings, &quit]).build()?;
    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("EasyCLI")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open_settings" => { let _ = open_settings_window(app.clone()); },
            "quit" => {
                // Stop backend process then exit
                stop_process_internal();
                let _ = TRAY_ICON.lock().take();
                let _ = app.exit(0);
            },
            _ => {}
        });
    // Platform-specific tray icon
    #[cfg(target_os = "linux")]
    {
        const ICON_PNG: &[u8] = include_bytes!("../../images/icon.png");
        if let Ok(img) = image::load_from_memory(ICON_PNG) {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
            builder = builder.icon(icon);
        }
    }
    #[cfg(target_os = "windows")]
    {
        const ICON_ICO: &[u8] = include_bytes!("../../images/icon.ico");
        if let Ok(dir) = ico::IconDir::read(Cursor::new(ICON_ICO)) {
            if let Some(entry) = dir.entries().iter().max_by_key(|e| e.width()) {
                if let Ok(img) = entry.decode() {
                    let w = img.width();
                    let h = img.height();
                    let rgba = img.rgba_data().to_vec();
                    let icon = tauri::image::Image::new_owned(rgba, w, h);
                    builder = builder.icon(icon);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // Try decode ICNS and convert to PNG buffer; fallback to PNG if needed.
        const ICON_ICNS: &[u8] = include_bytes!("../../images/icon.icns");
        let mut set = false;
        if let Ok(fam) = icns::IconFamily::read(Cursor::new(ICON_ICNS)) {
            use icns::IconType;
            let prefs = [
                IconType::RGBA32_512x512,
                IconType::RGBA32_256x256,
                IconType::RGBA32_128x128,
                IconType::RGBA32_64x64,
                IconType::RGBA32_32x32,
                IconType::RGBA32_16x16,
            ];
            for ty in prefs.iter() {
                if let Ok(icon_img) = fam.get_icon_with_type(*ty) {
                    let mut png_buf: Vec<u8> = Vec::new();
                    if icon_img.write_png(&mut png_buf).is_ok() {
                        if let Ok(img) = image::load_from_memory(&png_buf) {
                            let rgba = img.into_rgba8();
                            let (w, h) = rgba.dimensions();
                            let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
                            builder = builder.icon(icon);
                            set = true;
                            break;
                        }
                    }
                }
            }
        }
        if !set {
            const ICON_PNG: &[u8] = include_bytes!("../../images/icon.png");
            if let Ok(img) = image::load_from_memory(ICON_PNG) {
                let rgba = img.into_rgba8();
                let (w, h) = rgba.dimensions();
                let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
                builder = builder.icon(icon);
            }
        }
    }
    let tray = builder.build(app)?;
    *guard = Some(tray);
    Ok(())
}

fn callback_path_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "/anthropic/callback",
        "codex" => "/codex/callback",
        "google" => "/google/callback",
        _ => "/callback",
    }
}

fn build_redirect_url(mode: &str, provider: &str, base_url: Option<String>, local_port: Option<u16>, query: &str) -> String {
    let cb = callback_path_for(provider);
    let mut base = String::new();
    if mode == "local" {
        let port = local_port.unwrap_or(8317);
        base = format!("http://127.0.0.1:{}{}", port, cb);
    } else {
        let bu = base_url.unwrap_or_else(|| "http://127.0.0.1:8317".to_string());
        // ensure single slash
        if bu.ends_with('/') { base = format!("{}{}", bu, cb.trim_start_matches('/')); }
        else { base = format!("{}/{}", bu, cb.trim_start_matches('/')); }
    }
    if query.is_empty() { base } else { format!("{}?{}", base, query) }
}

fn run_callback_server(stop: Arc<AtomicBool>, listen_port: u16, mode: String, provider: String, base_url: Option<String>, local_port: Option<u16>) {
    let addr = format!("127.0.0.1:{}", listen_port);
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[CALLBACK] failed to bind {}: {}", addr, e);
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(false) { eprintln!("[CALLBACK] set_nonblocking failed: {}", e); }
    println!("[CALLBACK] listening on {} for provider {}", addr, provider);
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // read request line
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                if reader.read_line(&mut req_line).is_ok() {
                    let pathq = req_line.split_whitespace().nth(1).unwrap_or("/");
                    let query = pathq.splitn(2, '?').nth(1).unwrap_or("");
                    let loc = build_redirect_url(&mode, &provider, base_url.clone(), local_port, query);
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        loc
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            Err(e) => {
                if stop.load(Ordering::SeqCst) { break; }
                eprintln!("[CALLBACK] accept error: {}", e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    println!("[CALLBACK] server on {} stopped", addr);
}

#[tauri::command]
fn start_callback_server(provider: String, listen_port: u16, mode: String, base_url: Option<String>, local_port: Option<u16>) -> Result<serde_json::Value, String> {
    let mut map = CALLBACK_SERVERS.lock();
    if let Some((flag, handle)) = map.remove(&listen_port) {
        flag.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", listen_port));
        let _ = handle.join();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let handle = thread::spawn(move || run_callback_server(stop_clone, listen_port, mode, provider, base_url, local_port));
    map.insert(listen_port, (stop, handle));
    Ok(json!({"success": true}))
}

#[tauri::command]
fn stop_callback_server(listen_port: u16) -> Result<serde_json::Value, String> {
    // Take the server handle out of the map so it won't be stopped twice
    let opt = CALLBACK_SERVERS.lock().remove(&listen_port);
    if let Some((flag, handle)) = opt {
        // Signal stop and nudge the listener, then detach-join in background
        flag.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", listen_port));
        std::thread::spawn(move || {
            let _ = handle.join();
        });
        Ok(json!({"success": true}))
    } else {
        Ok(json!({"success": false, "error": "not running"}))
    }
}

#[tauri::command]
fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    // If settings window already exists (predefined in config), just show and focus it
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        // Ensure Dock icon is visible while settings is open (macOS only)
        #[cfg(target_os = "macos")]
        {
            let _ = app.show();
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
            let _ = app.set_dock_visibility(true);
        }
        // Also close login window shortly after
        let app_cloned = app.clone();
        tauri::async_runtime::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            if let Some(main) = app_cloned.get_webview_window("main") {
                let _ = main.close();
            }
        });
        return Ok(());
    }

    // Otherwise create it and show
    let url = WebviewUrl::App("settings.html".into());
    let win = WebviewWindowBuilder::new(&app, "settings", url)
        .title("EasyCLI Control Panel")
        .inner_size(930.0, 600.0)
        .resizable(false)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = win.show();
    let _ = win.set_focus();
    // Ensure Dock icon is visible while settings is open (macOS only)
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = app.set_dock_visibility(true);
    }
    // Close the main (login) window shortly after to avoid hanging the invoke
    let app_cloned = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        if let Some(main) = app_cloned.get_webview_window("main") {
            let _ = main.close();
        }
    });
    Ok(())
}

#[tauri::command]
fn open_login_window(app: tauri::AppHandle) -> Result<(), String> {
    // If login window already exists (predefined in config), show and focus it
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        // Close settings window shortly after to ensure clean state
        let app_cloned = app.clone();
        tauri::async_runtime::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            if let Some(settings) = app_cloned.get_webview_window("settings") {
                let _ = settings.close();
            }
        });
        return Ok(());
    }

    // Otherwise create the login window and close settings
    let url = WebviewUrl::App("login.html".into());
    let win = WebviewWindowBuilder::new(&app, "main", url)
        .title("EasyCLI")
        .inner_size(530.0, 380.0)
        .resizable(false)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = win.show();
    let _ = win.set_focus();

    let app_cloned = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        if let Some(settings) = app_cloned.get_webview_window("settings") {
            let _ = settings.close();
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Keep running in background (tray) if local process is active
                let running = PROCESS.lock().is_some();
                if running {
                    api.prevent_close();
                    let _ = window.hide();
                    // Hide Dock icon when settings window is closed in Local mode (macOS only)
                    if window.label() == "settings" {
                        #[cfg(target_os = "macos")]
                        {
                            let _ = window
                                .app_handle()
                                .set_activation_policy(tauri::ActivationPolicy::Accessory);
                            let _ = window.app_handle().set_dock_visibility(false);
                        }
                    }
                }
            }
        })
        // Note: Tauri v2 has no Builder::on_exit; we rely on tray Quit and OS termination to close child.
        .invoke_handler(tauri::generate_handler![
            check_version_and_download,
            download_cliproxyapi,
            check_secret_key,
            update_secret_key,
            read_config_yaml,
            update_config_yaml,
            read_local_auth_files,
            upload_local_auth_files,
            delete_local_auth_files,
            download_local_auth_files,
            restart_cliproxyapi,
            start_cliproxyapi,
            open_settings_window,
            open_login_window
            ,start_callback_server
            ,stop_callback_server
            ,save_files_to_directory
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Deserialize)]
struct SaveFile { name: String, content: String }

#[tauri::command]
fn save_files_to_directory(files: Vec<SaveFile>) -> Result<serde_json::Value, String> {
    if files.is_empty() {
        return Ok(json!({"success": false, "error": "No files to save"}));
    }
    // Show a system directory picker to choose the destination folder
    let folder = FileDialog::new()
        .set_title("Choose save directory")
        .pick_folder()
        .ok_or_else(|| "User cancelled directory selection".to_string())?;

    // Write each file into the chosen directory
    let mut success: usize = 0;
    let mut error_count: usize = 0;
    let mut errors: Vec<String> = Vec::new();
    for f in files {
        let path = folder.join(&f.name);
        match fs::write(&path, f.content.as_bytes()) {
            Ok(_) => success += 1,
            Err(e) => { error_count += 1; errors.push(format!("{}: {}", f.name, e)); }
        }
    }

    Ok(json!({
        "success": success > 0,
        "successCount": success,
        "errorCount": error_count,
        "errors": if errors.is_empty() { serde_json::Value::Null } else { json!(errors) }
    }))
}
