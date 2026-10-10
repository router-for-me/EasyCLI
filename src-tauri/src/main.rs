#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod app_settings;
mod app_update;
mod claude_catalog;
mod codex_catalog;
mod codex_sessions;
mod configuration_watcher;
mod core_config;
mod core_runtime;
mod desktop_theme;
mod instance_lock;
mod management_api;
mod network_proxy;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
mod native_i18n;
mod oauth_browser;
mod plugins;
mod progress;
mod provider_health;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
mod tray;
mod usage;

#[cfg(test)]
use configuration_watcher::nearest_existing_watch_directory;
use management_api::{
    format_management_request_error, management_authorization, management_endpoint,
    management_http_client, read_management_text, read_management_value,
};
use oauth_browser::*;
#[cfg(test)]
use provider_health::{
    provider_health_content_type_is_streaming, provider_health_stream_has_terminal_success,
    provider_health_stream_has_text,
};

use agents::*;
use app_settings::*;
use app_update::*;
use core_config::*;
use core_runtime::*;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use instance_lock::*;
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{LazyLock, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tar::Archive;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use tauri::menu::PredefinedMenuItem;
#[cfg(target_os = "macos")]
use tauri::tray::{MouseButtonState, TrayIcon};
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tauri::{Emitter, LogicalSize, Manager};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
use tray::*;
use zip::ZipArchive;

const RELEASE_PAGE_URL: &str = "https://github.com/router-for-me/CLIProxyAPI/releases/latest";
const RELEASE_ATOM_URL: &str = "https://github.com/router-for-me/CLIProxyAPI/releases.atom";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/router-for-me/CLIProxyAPI/releases/download/";
#[cfg(windows)]
const APP_UPDATE_MANIFEST_URL: &str = "https://github.com/router-for-me/EasyCLIProxyAPI/releases/latest/download/portable-update-windows.json";
#[cfg(target_os = "linux")]
const APP_UPDATE_MANIFEST_URL: &str = "https://github.com/router-for-me/EasyCLIProxyAPI/releases/latest/download/portable-update-linux.json";
#[cfg(target_os = "macos")]
const APP_UPDATE_MANIFEST_URL: &str = "https://github.com/router-for-me/EasyCLIProxyAPI/releases/latest/download/portable-update-darwin-v2.json";
const APP_RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/router-for-me/EasyCLIProxyAPI/releases/download/";
#[cfg(windows)]
const APP_UPDATE_MANIFEST_NAME: &str = "portable-update-windows.json";
#[cfg(target_os = "linux")]
const APP_UPDATE_MANIFEST_NAME: &str = "portable-update-linux.json";
#[cfg(target_os = "macos")]
const APP_UPDATE_MANIFEST_NAME: &str = "portable-update-darwin-v2.json";
const CODEX_MODEL_CATALOG_URL: &str = "https://raw.githubusercontent.com/router-for-me/EasyCLIProxyAPI/main/src-tauri/resources/codex_models/model-catalog.json";
const CODEX_MODEL_CATALOG_OVERRIDE_DIR: &str = "codex_models";
const CODEX_MODEL_CATALOG_SOURCE_FILE: &str = "model-catalog.json";
const MAX_CODEX_MODEL_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const APP_UPDATE_PROGRESS_EVENT: &str = "app-update-progress";
const PORTABLE_APP_MANIFEST_FILE: &str = "portable-app.json";
#[cfg(windows)]
const PORTABLE_APP_BINARY: &str = "EasyCLIProxyAPI.exe";
#[cfg(target_os = "linux")]
const PORTABLE_APP_BINARY: &str = "EasyCLIProxyAPI";
const CORE_INSTALL_PROGRESS_EVENT: &str = "core-install-progress";
const CORE_STATUS_EVENT: &str = "core-status-changed";
const CONFIG_FILES_CHANGED_EVENT: &str = "config-files-changed";
const VERSION_DOWNLOAD_SOURCE_CHANGED_EVENT: &str = "version-download-source-changed";
static VERSION_SOURCE_DETECTION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_CLOSE_REQUEST_EVENT: &str = "windows-close-requested";
const CORE_METADATA_FILE: &str = "cpa-gui-meta.json";
const CORE_CONFIG_FILE: &str = "config.yaml";
const CORE_EXAMPLE_CONFIG_FILE: &str = "config.example.yaml";
const CORE_VERSION_FILE: &str = "core-version.txt";
const CORE_CHECKSUMS_FILE: &str = "checksums.txt";
const GUI_CONFIG_FILE: &str = "config.toml";
const LEGACY_GUI_CONFIG_FILE: &str = "cpa-gui.yaml";
const MIN_MAIN_WINDOW_WIDTH: u32 = 640;
const MIN_MAIN_WINDOW_HEIGHT: u32 = 600;
const MAX_SAVED_WINDOW_DIMENSION: u32 = 16_384;
const DEFAULT_MAIN_WINDOW_WIDTH: u32 = 1280;
const DEFAULT_MAIN_WINDOW_HEIGHT: u32 = 800;
const LEGACY_DEFAULT_MAIN_WINDOW_WIDTH: u32 = 1531;
const LEGACY_DEFAULT_MAIN_WINDOW_HEIGHT: u32 = 891;
const OAUTH_DIR_NAME: &str = "oauth";
const DEFAULT_AUTH_DIR: &str = "../oauth";
const DEFAULT_API_KEY: &str = "123456";
const DEFAULT_AGENT_TERMINAL: &str = "auto";
const DEFAULT_API_KEY_INITIAL_REMARK: &str = "Default key";
const DEFAULT_REQUEST_RETRY: u32 = 3;
const DEFAULT_MAX_RETRY_CREDENTIALS: u32 = 0;
const DEFAULT_MAX_RETRY_INTERVAL: i64 = 30;
const DEFAULT_STREAMING_BOOTSTRAP_RETRIES: u32 = 0;
const DEFAULT_DISABLE_COOLING: bool = false;
const DEFAULT_LOGS_MAX_TOTAL_SIZE_MB: u32 = 0;
const DEFAULT_ERROR_LOGS_MAX_FILES: u32 = 10;
const DEFAULT_REDIS_USAGE_QUEUE_RETENTION_SECONDS: u32 = 60;
const LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY: &str = "123456";
const MANAGED_AGENT_PROVIDER_ID: &str = "cpa-gui";
const ZCODE_PROVIDER_CONFIG_FILE: &str = "provider_config.json";
const KIMI_CODE_CONFIG_FILE: &str = "config.toml";
const GROK_BUILD_CONFIG_FILE: &str = "config.toml";
const DEEPSEEK_HARNESS_PROVIDER_ID: &str = "easy-cliproxyapi";
const DEEPSEEK_HARNESS_CREDENTIAL: &str = "EASYCLIPROXYAPI_API_KEY";
const DEEPSEEK_HARNESS_CREDENTIALS_VERSION: u64 = 1;
const DEEPSEEK_HARNESS_DEFAULT_WEB_PORT: u16 = 3080;
const DEEPSEEK_HARNESS_SETTINGS_FILE: &str = "settings.yaml";
const DEEPSEEK_HARNESS_CREDENTIALS_FILE: &str = ".credentials.yaml";
const PI_AGENT_ID: &str = "pi";
const PI_AGENT_NAME: &str = "Pi";
const OMP_AGENT_PROVIDER_ID: &str = "easy-cliproxyapi";
const OMP_AGENT_API: &str = "openai-responses";
const OMP_MODELS_FILE: &str = "models.yml";
const OMP_SETTINGS_FILE: &str = "config.yml";
const PI_CLIPROXYAPI_PACKAGE: &str = "npm:@router-for-me/pi-cliproxyapi-provider";
const PI_CLIPROXYAPI_NPM_LATEST_URL: &str =
    "https://registry.npmjs.org/@router-for-me%2Fpi-cliproxyapi-provider/latest";
const PI_CLIPROXYAPI_PROVIDER_ID: &str = "cliproxyapi";
const PI_AGENT_CONFIG_FILE: &str = "cliproxyapi.json";
const PI_AGENT_SETTINGS_FILE: &str = "settings.json";
const CODEX_MODEL_CATALOG_FILE: &str = "cpa-gui-model-catalog.json";
const CODEX_OAUTH_LOGIN_REQUIRED_ERROR: &str = "CODEX_OAUTH_LOGIN_REQUIRED";
const CLAUDE_DESKTOP_PROFILE_ID: &str = "00000000-0000-4000-8000-000000831700";
const CLAUDE_DESKTOP_OPUS_MODEL_ID: &str = "claude-opus-5-cpa";
const CLAUDE_DESKTOP_SONNET_MODEL_ID: &str = "claude-sonnet-5-cpa";
const CLAUDE_DESKTOP_HAIKU_MODEL_ID: &str = "claude-haiku-4-5-cpa";
const LEGACY_CLAUDE_DESKTOP_MODEL_IDS: [&str; 3] =
    ["claude-opus-5", "claude-sonnet-4-6", "claude-haiku-4-5"];
const MANAGED_CLAUDE_OPUS_ALIAS_DISPLAY_NAME: &str = "EasyCLIProxyAPI managed Claude Opus mapping";
const MANAGED_CLAUDE_SONNET_ALIAS_DISPLAY_NAME: &str =
    "EasyCLIProxyAPI managed Claude Sonnet mapping";
const MANAGED_CLAUDE_HAIKU_ALIAS_DISPLAY_NAME: &str =
    "EasyCLIProxyAPI managed Claude Haiku mapping";
const DEFAULT_CLAUDE_CONTEXT_WINDOW: u64 = 200_000;
const CLAUDE_DESKTOP_EXTENDED_CONTEXT_WINDOW: u64 = 1_000_000;
const CLAUDE_CODE_MAX_CONTEXT_TOKENS_ENV: &str = "CLAUDE_CODE_MAX_CONTEXT_TOKENS";
const CLAUDE_CODE_AUTO_COMPACT_WINDOW_ENV: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";
const CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY_ENV: &str =
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY";
const CLAUDE_CODE_AUTO_MODE_SERVER_ENV: &str = "CLAUDE_CODE_AUTO_MODE_SERVER";
const CLAUDE_AUTOCOMPACT_PCT_OVERRIDE_ENV: &str = "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE";
const DISABLE_AUTO_COMPACT_ENV: &str = "DISABLE_AUTO_COMPACT";
const CLAUDE_CODE_MANAGED_MODEL_SETTINGS_ENV: &str = "EASYCLIPROXY_CLAUDE_CODE_MODEL_SETTINGS";
const CLAUDE_CODE_MANAGED_STARTUP_MODEL_ENV: &str = "EASYCLIPROXY_CLAUDE_CODE_STARTUP_MODEL";
const DEFAULT_CLAUDE_AUTO_COMPACT_PCT: u8 = 90;
const MODEL_ALIAS_CONFIG_SECTIONS: &[&str] = &[
    "codex-api-key",
    "openai-compatibility",
    "claude-api-key",
    "gemini-api-key",
];
#[cfg(test)]
const LEGACY_AGENT_MODIFICATION_STATE_VERSION: u8 = 1;
#[cfg(test)]
const AGENT_MODIFICATION_STATE_VERSION: u8 = 2;
#[cfg(test)]
const AGENT_APPLIED_STATE_VERSION: u8 = 4;
#[cfg(test)]
const AGENT_CONFIGURATION_REVISION: u8 = 1;
#[cfg(test)]
const AGENT_PHASE_APPLYING: &str = "applying";
#[cfg(test)]
const AGENT_PHASE_ACTIVE: &str = "active";
#[cfg(test)]
const AGENT_PHASE_RESTORING: &str = "restoring";
#[cfg(test)]
const AGENT_PHASE_RECOVERY: &str = "recovery";
#[cfg(test)]
const AGENT_MODIFICATION_STATE_CONFLICT: &str = "conflict";
const USER_AGENT: &str = concat!(
    "CPA-GUI/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/router-for-me/CLIProxyAPI)"
);
const APP_USER_AGENT: &str = concat!(
    "EasyCLIProxyAPI/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/router-for-me/EasyCLIProxyAPI)"
);
static CORE_CONFIG_FILE_LOCK: Mutex<()> = Mutex::new(());
static AGENT_CONFIG_FILE_LOCK: Mutex<()> = Mutex::new(());
#[cfg(test)]
static CODEX_APPLIED_STATES: LazyLock<
    Mutex<std::collections::HashMap<PathBuf, AgentAppliedState>>,
> = LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
static CONFIG_WRITE_HASHES: LazyLock<Mutex<std::collections::HashMap<PathBuf, String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn normalize_app_locale(locale: &str) -> &'static str {
    let normalized = locale.trim().to_ascii_lowercase();
    if normalized.starts_with("ja") {
        "ja"
    } else if normalized == "zh-tw"
        || normalized == "zh-hk"
        || normalized == "zh-mo"
        || normalized.starts_with("zh-hant")
    {
        "zh-TW"
    } else if normalized.starts_with("zh") {
        "zh-CN"
    } else {
        "en"
    }
}

#[derive(Default)]
struct CoreDownloadState {
    inner: Mutex<CoreDownloadInner>,
}

#[derive(Default)]
struct CoreDownloadInner {
    running: bool,
    token: Option<CancellationToken>,
    task: CoreInstallTask,
}

#[derive(Default)]
struct AppUpdateState {
    inner: Mutex<AppUpdateInner>,
}

#[derive(Default)]
struct AppUpdateInner {
    task: AppUpdateTask,
    token: Option<CancellationToken>,
}

#[derive(Default)]
struct CoreProcessState {
    child: Mutex<Option<CoreChild>>,
    adopted_processes: Mutex<Vec<AdoptedCoreProcess>>,
    starting: AtomicBool,
    shutting_down: AtomicBool,
    shutdown_complete: AtomicBool,
}

#[derive(Default)]
struct DeepSeekHarnessProcessState {
    process: Mutex<Option<ManagedDeepSeekHarnessProcess>>,
}

struct ManagedDeepSeekHarnessProcess {
    child: Child,
    mode: String,
    launch: DeepSeekHarnessLaunchSnapshot,
}

#[derive(Clone)]
struct DeepSeekHarnessLaunchSnapshot {
    executable: PathBuf,
    working_directory: PathBuf,
    arguments: Vec<String>,
    options: Option<DeepSeekHarnessLaunchOptions>,
}

#[derive(Clone)]
struct AdoptedCoreProcess {
    process_id: u32,
    binary_path: PathBuf,
}

struct GuiConfigState {
    inner: Mutex<GuiConfigFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SavedWindowSize {
    width: u32,
    height: u32,
}

struct MainWindowSizeState {
    inner: Mutex<Option<SavedWindowSize>>,
}

#[derive(Default)]
struct AgentConfigStatusCache {
    refresh_lock: Mutex<()>,
    entry: Mutex<Option<AgentConfigStatusCacheEntry>>,
}

struct AgentConfigStatusCacheEntry {
    port: u16,
    api_key_sha256: String,
    statuses: Vec<AgentConfigStatus>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFilesChangedPayload {
    paths: Vec<String>,
    errors: Vec<String>,
}

impl AgentConfigStatusCache {
    fn get(&self, port: u16, api_key: &str) -> Result<Option<Vec<AgentConfigStatus>>, String> {
        let api_key_sha256 = sha256_bytes(api_key.as_bytes());
        self.entry
            .lock()
            .map(|entry| {
                entry
                    .as_ref()
                    .filter(|entry| entry.port == port && entry.api_key_sha256 == api_key_sha256)
                    .map(|entry| entry.statuses.clone())
            })
            .map_err(|_| "Agent configuration status cache lock is poisoned".to_string())
    }

    fn replace(
        &self,
        port: u16,
        api_key: &str,
        statuses: Vec<AgentConfigStatus>,
    ) -> Result<(), String> {
        let mut current = self
            .entry
            .lock()
            .map_err(|_| "Agent configuration status cache lock is poisoned".to_string())?;
        *current = Some(AgentConfigStatusCacheEntry {
            port,
            api_key_sha256: sha256_bytes(api_key.as_bytes()),
            statuses,
        });
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        let mut current = self
            .entry
            .lock()
            .map_err(|_| "Agent configuration status cache lock is poisoned".to_string())?;
        *current = None;
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CorePlatform {
    os: String,
    arch: String,
    asset_os: String,
    asset_arch: String,
    archive_kind: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreStatus {
    installed: bool,
    running: bool,
    ready: bool,
    starting: bool,
    managed: bool,
    process_id: Option<u32>,
    current_version: Option<String>,
    install_dir: String,
    binary_path: Option<String>,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreLatest {
    version: String,
    asset_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateInfo {
    current_version: String,
    latest_version: String,
    update_available: bool,
    release_url: String,
    release_notes: HashMap<String, String>,
    published_at: String,
    auto_update_supported: bool,
    download_size_bytes: Option<u64>,
    unsupported_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableUpdateManifest {
    schema_version: u32,
    version: String,
    published_at: String,
    release_url: String,
    #[serde(default, deserialize_with = "deserialize_release_notes")]
    release_notes: HashMap<String, String>,
    assets: std::collections::HashMap<String, PortableUpdateAsset>,
    #[serde(default)]
    full_assets: Option<std::collections::HashMap<String, PortableUpdateAsset>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableUpdateAsset {
    url: String,
    #[serde(default)]
    fallback_urls: Vec<String>,
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct GitcodeRelease {
    tag_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableAppManifest {
    schema_version: u32,
    application: String,
    version: String,
    platform: String,
    arch: String,
    #[serde(default)]
    auto_update: bool,
}

#[derive(Clone)]
struct PendingAppUpdate {
    version: String,
    asset: PortableUpdateAsset,
    #[cfg_attr(not(windows), allow(dead_code))]
    arch: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateTask {
    running: bool,
    cancellable: bool,
    phase: String,
    target_version: Option<String>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<f64>,
    message: Option<String>,
}

impl Default for AppUpdateTask {
    fn default() -> Self {
        Self {
            running: false,
            cancellable: false,
            phase: "idle".to_string(),
            target_version: None,
            downloaded_bytes: 0,
            total_bytes: None,
            percent: None,
            message: None,
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
struct PortablePackagePayload {
    manifest: PortableAppManifest,
    core_archive_name: Option<String>,
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableUpdateDescriptor {
    parent_pid: u32,
    current_exe: PathBuf,
    staged_exe: PathBuf,
    current_manifest: PathBuf,
    staged_manifest: PathBuf,
    backup_exe: PathBuf,
    backup_manifest: PathBuf,
    current_core_version: PathBuf,
    staged_core_version: PathBuf,
    backup_core_version: PathBuf,
    staged_core_archive: PathBuf,
    target_core_archive: PathBuf,
    install_core_archive: bool,
    ack_path: PathBuf,
    work_dir: PathBuf,
    target_version: String,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MacosUpdateDescriptor {
    parent_pid: u32,
    current_app: PathBuf,
    staged_app: PathBuf,
    backup_app: PathBuf,
    executable_relative_path: PathBuf,
    ack_path: PathBuf,
    work_dir: PathBuf,
    target_version: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundledCoreInfo {
    version: String,
    asset_name: String,
    size_bytes: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInstallResult {
    version: String,
    asset_name: String,
    install_dir: String,
    binary_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInstallTask {
    running: bool,
    cancellable: bool,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    percent: Option<f64>,
    message: Option<String>,
    result: Option<CoreInstallResult>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct GuiConfigFile {
    locale: String,
    port: u16,
    allow_lan: bool,
    host: String,
    run_on_startup: bool,
    start_core_on_launch: bool,
    silent_start: bool,
    close_behavior: WindowsCloseBehavior,
    default_terminal: String,
    window_width: Option<u32>,
    window_height: Option<u32>,
    auth_dir: String,
    auth_dir_user_selected: bool,
    #[serde(deserialize_with = "deserialize_gui_api_keys")]
    api_keys: Vec<GuiApiKeyEntry>,
    api_access_remarks: Vec<GuiApiAccessRemark>,
    management_secret_key: String,
    debug: bool,
    commercial_mode: bool,
    logging_to_file: bool,
    logs_max_total_size_mb: u32,
    error_logs_max_files: u32,
    usage_statistics_enabled: bool,
    #[serde(rename = "usage_statistics_disabled", skip_serializing_if = "Option::is_none")]
    usage_statistics_disabled: Option<bool>,
    redis_usage_queue_retention_seconds: u32,
    request_log: bool,
    plugins_enabled: bool,
    routing_strategy: String,
    proxy_url: String,
    proxy_override: bool,
    download_source: VersionDownloadSource,
    custom_download_mirrors: Vec<String>,
    active_custom_download_mirror: String,
    prefer_gitcode_downloads: bool,
    routing_session_affinity: bool,
    routing_session_affinity_ttl: String,
    disable_cooling: bool,
    request_retry: u32,
    max_retry_credentials: u32,
    max_retry_interval: i64,
    streaming_bootstrap_retries: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum VersionDownloadSource {
    #[default]
    Github,
    Gitcode,
    GhProxy,
    GhFast,
    Custom,
}

impl VersionDownloadSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitcode => "gitcode",
            Self::GhProxy => "gh-proxy",
            Self::GhFast => "gh-fast",
            Self::Custom => "custom",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "github" => Some(Self::Github),
            "gitcode" => Some(Self::Gitcode),
            "gh-proxy" => Some(Self::GhProxy),
            "gh-fast" => Some(Self::GhFast),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitcode => "GitCode",
            Self::GhProxy => "gh-proxy.com",
            Self::GhFast => "ghfast.top",
            Self::Custom => "Custom mirror",
        }
    }

    fn github_proxy_prefix(self) -> Option<&'static str> {
        match self {
            Self::GhProxy => Some("https://gh-proxy.com/"),
            Self::GhFast => Some("https://ghfast.top/"),
            Self::Custom => None,
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VersionDownloadCandidate {
    source: VersionDownloadSource,
    custom_url: Option<String>,
}

impl VersionDownloadCandidate {
    fn builtin(source: VersionDownloadSource) -> Self {
        Self {
            source,
            custom_url: None,
        }
    }

    fn custom(url: &str) -> Self {
        Self {
            source: VersionDownloadSource::Custom,
            custom_url: Some(url.to_string()),
        }
    }

    fn key(&self) -> String {
        self.custom_url
            .as_ref()
            .map(|url| format!("custom:{url}"))
            .unwrap_or_else(|| self.source.as_str().to_string())
    }

    fn display_name(&self) -> String {
        self.custom_url
            .as_ref()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| self.source.display_name().to_string())
    }

    fn proxy_prefix(&self) -> Option<&str> {
        self.custom_url
            .as_deref()
            .or_else(|| self.source.github_proxy_prefix())
    }
}

fn version_source_url(source: &VersionDownloadCandidate, github_url: &str) -> String {
    source
        .proxy_prefix()
        .map(|prefix| format!("{prefix}{github_url}"))
        .unwrap_or_else(|| github_url.to_string())
}

fn version_download_source_candidates(
    preferred: VersionDownloadCandidate,
    gitcode_available: bool,
    custom_mirrors: &[String],
) -> Vec<VersionDownloadCandidate> {
    let mut candidates = vec![
        preferred,
        VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
        VersionDownloadCandidate::builtin(VersionDownloadSource::Gitcode),
        VersionDownloadCandidate::builtin(VersionDownloadSource::GhProxy),
        VersionDownloadCandidate::builtin(VersionDownloadSource::GhFast),
    ];
    candidates.retain(|candidate| {
        gitcode_available || candidate.source != VersionDownloadSource::Gitcode
    });
    candidates.extend(
        custom_mirrors
            .iter()
            .map(|url| VersionDownloadCandidate::custom(url)),
    );
    candidates
        .into_iter()
        .fold(Vec::new(), |mut sources, source| {
            if !sources.contains(&source) {
                sources.push(source);
            }
            sources
        })
}

fn normalize_custom_download_mirror_url(value: &str) -> Result<String, String> {
    let mut url =
        reqwest::Url::parse(value.trim()).map_err(|error| format!("Invalid mirror URL: {error}"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Mirror URL must be an HTTPS URL without authentication, port, query parameters, or fragment".to_string());
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

impl GuiConfigFile {
    fn selected_download_candidate(&self) -> VersionDownloadCandidate {
        if self.download_source == VersionDownloadSource::Custom
            && self
                .custom_download_mirrors
                .contains(&self.active_custom_download_mirror)
        {
            VersionDownloadCandidate::custom(&self.active_custom_download_mirror)
        } else {
            VersionDownloadCandidate::builtin(self.download_source)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum WindowsCloseAction {
    Exit,
    MinimizeToTray,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum WindowsCloseBehavior {
    #[default]
    Ask,
    Exit,
    MinimizeToTray,
}

impl WindowsCloseBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Exit => "exit",
            Self::MinimizeToTray => "minimize-to-tray",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct GuiApiKeyEntry {
    key: String,
    #[serde(default)]
    remark: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct GuiApiAccessRemark {
    provider_section: String,
    api_key_hash: String,
    #[serde(default)]
    record_hash: String,
    remark: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiAccessRemarkLocator {
    provider_name: String,
    base_url: String,
    api_keys: Vec<String>,
    #[serde(default)]
    config_identity: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiAccessRemarkQuery {
    provider_section: String,
    #[serde(flatten)]
    locator: ApiAccessRemarkLocator,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiAccessRemarkUpdate {
    provider_section: String,
    previous_records: Vec<ApiAccessRemarkLocator>,
    records: Vec<ApiAccessRemarkLocator>,
    all_records: Vec<ApiAccessRemarkLocator>,
    remark: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GuiApiKeyInput {
    Legacy(String),
    Entry(GuiApiKeyEntry),
}

fn deserialize_gui_api_keys<'de, D>(deserializer: D) -> Result<Vec<GuiApiKeyEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries = Vec::<GuiApiKeyInput>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            GuiApiKeyInput::Legacy(key) => GuiApiKeyEntry {
                remark: String::new(),
                key,
            },
            GuiApiKeyInput::Entry(entry) => entry,
        })
        .collect())
}

impl Default for GuiConfigFile {
    fn default() -> Self {
        Self {
            locale: "en".to_string(),
            port: 8317,
            allow_lan: false,
            host: "127.0.0.1".to_string(),
            run_on_startup: false,
            start_core_on_launch: true,
            silent_start: false,
            close_behavior: WindowsCloseBehavior::Ask,
            default_terminal: DEFAULT_AGENT_TERMINAL.to_string(),
            window_width: Some(DEFAULT_MAIN_WINDOW_WIDTH),
            window_height: Some(DEFAULT_MAIN_WINDOW_HEIGHT),
            auth_dir: DEFAULT_AUTH_DIR.to_string(),
            auth_dir_user_selected: false,
            api_keys: vec![default_api_key_entry()],
            api_access_remarks: Vec::new(),
            management_secret_key: LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string(),
            debug: false,
            commercial_mode: false,
            logging_to_file: false,
            logs_max_total_size_mb: DEFAULT_LOGS_MAX_TOTAL_SIZE_MB,
            error_logs_max_files: DEFAULT_ERROR_LOGS_MAX_FILES,
            usage_statistics_enabled: true,
            usage_statistics_disabled: None,
            redis_usage_queue_retention_seconds: DEFAULT_REDIS_USAGE_QUEUE_RETENTION_SECONDS,
            request_log: false,
            plugins_enabled: false,
            routing_strategy: "round-robin".to_string(),
            proxy_url: String::new(),
            proxy_override: false,
            download_source: VersionDownloadSource::Github,
            custom_download_mirrors: Vec::new(),
            active_custom_download_mirror: String::new(),
            prefer_gitcode_downloads: false,
            routing_session_affinity: false,
            routing_session_affinity_ttl: String::new(),
            disable_cooling: DEFAULT_DISABLE_COOLING,
            request_retry: DEFAULT_REQUEST_RETRY,
            max_retry_credentials: DEFAULT_MAX_RETRY_CREDENTIALS,
            max_retry_interval: DEFAULT_MAX_RETRY_INTERVAL,
            streaming_bootstrap_retries: DEFAULT_STREAMING_BOOTSTRAP_RETRIES,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct GuiConfigPresence {
    locale: Option<String>,
    port: Option<u16>,
    allow_lan: Option<bool>,
    host: Option<String>,
    auth_dir: Option<String>,
    api_keys: Option<Vec<GuiApiKeyInput>>,
    api_access_remarks: Option<Vec<GuiApiAccessRemark>>,
    management_secret_key: Option<String>,
    close_behavior: Option<WindowsCloseBehavior>,
    default_terminal: Option<String>,
    start_core_on_launch: Option<bool>,
    silent_start: Option<bool>,
    debug: Option<bool>,
    commercial_mode: Option<bool>,
    logging_to_file: Option<bool>,
    logs_max_total_size_mb: Option<u32>,
    error_logs_max_files: Option<u32>,
    usage_statistics_enabled: Option<bool>,
    redis_usage_queue_retention_seconds: Option<u32>,
    request_log: Option<bool>,
    plugins_enabled: Option<bool>,
    routing_strategy: Option<String>,
    proxy_url: Option<String>,
    proxy_override: Option<bool>,
    download_source: Option<VersionDownloadSource>,
    custom_download_mirrors: Option<Vec<String>>,
    active_custom_download_mirror: Option<String>,
    prefer_gitcode_downloads: Option<bool>,
    routing_session_affinity: Option<bool>,
    routing_session_affinity_ttl: Option<String>,
    disable_cooling: Option<bool>,
    request_retry: Option<u32>,
    max_retry_credentials: Option<u32>,
    max_retry_interval: Option<i64>,
    streaming_bootstrap_retries: Option<u32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiSettings {
    host: String,
    port: u16,
    allow_lan: bool,
    run_on_startup: bool,
    close_behavior: WindowsCloseBehavior,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionSourceSettings {
    source: String,
    gitcode_available: bool,
    custom_mirrors: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SoftwareSettings {
    close_behavior: WindowsCloseBehavior,
    autostart_enabled: bool,
    start_core_on_launch: bool,
    silent_start_enabled: bool,
    default_terminal: String,
    available_terminals: Vec<AgentTerminalOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SoftwareSettingsInput {
    close_behavior: WindowsCloseBehavior,
    autostart_enabled: bool,
    start_core_on_launch: bool,
    silent_start_enabled: bool,
    default_terminal: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentConfigStatus {
    id: String,
    name: String,
    supported_platform: bool,
    installed: bool,
    plugin_installed: bool,
    launch_targets: Vec<AgentLaunchTarget>,
    version: Option<String>,
    cli_version: Option<String>,
    app_version: Option<String>,
    plugin_version: Option<String>,
    config_paths: Vec<String>,
    config_exists: bool,
    config_valid: bool,
    configured: bool,
    configuration_synchronized: bool,
    connection_state: String,
    current_model: Option<String>,
    oauth_configuration: bool,
    codex_native_oauth: bool,
    modification_enabled: bool,
    modification_state: String,
    backup_available: bool,
    applied_model: Option<String>,
    claude_code_model_mappings: Option<ClaudeDesktopModelMappings>,
    claude_desktop_model_mappings: Option<ClaudeDesktopModelMappings>,
    warnings: Vec<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentLaunchTarget {
    id: String,
    label: String,
    detail: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeepSeekHarnessLaunchOptions {
    mode: String,
    web_host: Option<String>,
    web_port: Option<u16>,
    open_browser: Option<bool>,
    #[serde(default)]
    trusted_hosts: Vec<String>,
    task: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    patches: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeepSeekHarnessProcessStatus {
    running: bool,
    pid: Option<u32>,
    mode: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentConfigActionResult {
    outcome: String,
    enabled: bool,
    model: Option<String>,
    changed_files: Vec<String>,
    conflict_files: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PiProviderUpdateStatus {
    installed_version: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexModelCatalogUpdateResult {
    outcome: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelOption {
    name: String,
    alias: Option<String>,
    #[serde(default)]
    is_alias: bool,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_modalities: Option<Vec<String>>,
    #[serde(skip)]
    harness_metadata: Option<serde_json::Value>,
}


#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeDesktopModelMappings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop_models: Option<Vec<ClaudeDesktopModelMapping>>,
    #[serde(default)]
    opus: String,
    #[serde(default)]
    sonnet: String,
    #[serde(default)]
    haiku: String,
    #[serde(default)]
    fable: String,
    #[serde(default)]
    fable_1m: bool,
    #[serde(default)]
    opus_1m: bool,
    #[serde(default)]
    sonnet_1m: bool,
    #[serde(default)]
    haiku_1m: bool,
    #[serde(default = "default_claude_code_max_context_tokens")]
    max_context_tokens: u64,
    #[serde(default = "default_claude_auto_compact_pct")]
    auto_compact_pct: u8,
    #[serde(default)]
    disable_auto_compact: bool,
    /// Keep Claude Code's session/default model selection under CPA control.
    /// Defaults to true for backwards compatibility with existing profiles.
    #[serde(default = "default_true")]
    manage_default_model: bool,
    /// Set CLAUDE_CODE_SUBAGENT_MODEL when enabled; otherwise allow inheritance.
    /// Defaults to true for backwards compatibility with existing profiles.
    #[serde(default = "default_true")]
    manage_subagent_model: bool,
    /// Optional explicit model written to the root Claude Code `model` setting.
    #[serde(default)]
    startup_model: Option<String>,
    /// Optional explicit model written to CLAUDE_CODE_SUBAGENT_MODEL.
    #[serde(default)]
    subagent_model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeDesktopModelMapping {
    #[serde(default)]
    model: String,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    context_1m: bool,
}

impl ClaudeDesktopModelMapping {
    fn model_id(&self) -> &str {
        if self.alias.trim().is_empty() {
            self.model.trim()
        } else {
            self.alias.trim()
        }
    }

    fn source_or_alias(&self) -> &str {
        if self.model.trim().is_empty() {
            self.alias.trim()
        } else {
            self.model.trim()
        }
    }

    fn has_mapping(&self) -> bool {
        !self.model.trim().is_empty()
            && !self.alias.trim().is_empty()
            && !self.model.trim().eq_ignore_ascii_case(self.alias.trim())
    }
}

fn default_claude_code_max_context_tokens() -> u64 {
    DEFAULT_CLAUDE_CONTEXT_WINDOW
}

fn default_claude_auto_compact_pct() -> u8 {
    DEFAULT_CLAUDE_AUTO_COMPACT_PCT
}

fn default_true() -> bool {
    true
}

impl ClaudeDesktopModelMappings {
    fn all(model: &str) -> Self {
        Self {
            desktop_models: None,
            opus: model.to_string(),
            sonnet: model.to_string(),
            haiku: model.to_string(),
            fable: model.to_string(),
            fable_1m: false,
            opus_1m: false,
            sonnet_1m: false,
            haiku_1m: false,
            max_context_tokens: default_claude_code_max_context_tokens(),
            auto_compact_pct: default_claude_auto_compact_pct(),
            disable_auto_compact: false,
            manage_default_model: true,
            manage_subagent_model: true,
            startup_model: None,
            subagent_model: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexModelDefinition {
    id: String,
    display_name: Option<String>,
    description: Option<String>,
    context_window: Option<u64>,
    reasoning_levels: Vec<String>,
    supports_tools: Option<bool>,
}

// Coordinates in the configuration arrays; never persisted into CPA configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelAliasPosition {
    section: String,
    provider_index: Option<usize>,
    model_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingAliasEntry {
    position: ModelAliasPosition,
    source_model: String,
    alias: String,
    effort: Option<String>,
    provider: String,
    kind: String,
    oauth_channel: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeedAliasEntry {
    position: ModelAliasPosition,
    source_model: String,
    alias: String,
    service_tier: String,
    provider: String,
    kind: String,
    oauth_channel: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThinkingAliasSource {
    id: String,
    model: String,
    display_name: Option<String>,
    provider: String,
    kind: String,
    protocol: String,
    reasoning_levels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ThinkingAliasSourceLocation {
    Oauth {
        channel: &'static str,
        force_mapping: bool,
    },
    ConfigModel {
        section: &'static str,
        provider_index: usize,
        model_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedThinkingAliasSource {
    source: ThinkingAliasSource,
    location: ThinkingAliasSourceLocation,
}

#[cfg(test)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModificationRecord {
    version: u8,
    client: String,
    phase: String,
    model: String,
    files: Vec<AgentModificationFile>,
}

#[cfg(test)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModificationFile {
    path: PathBuf,
    backup_path: PathBuf,
    existed_before: bool,
    original_sha256: Option<String>,
    managed_sha256: String,
}

#[cfg(test)]
struct AgentModificationInspection {
    enabled: bool,
    state: String,
    backup_available: bool,
    warnings: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentAppliedState {
    version: u8,
    client: String,
    model: String,
    #[serde(default)]
    configuration_revision: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_desktop_model_mappings: Option<ClaudeDesktopModelMappings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    backup_files: Vec<AgentAppliedBackupFile>,
    updated_at_unix: u64,
}

#[cfg(test)]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentAppliedBackupFile {
    path: PathBuf,
    backup_path: PathBuf,
    existed_before: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentClient {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    OpenCode,
    OpenClaw,
    Hermes,
    DeepSeekHarness,
    ZCode,
    WorkBuddy,
    AntigravityCli,
    KimiCode,
    GrokBuild,
    Omp,
}

#[derive(Clone, Debug)]
enum DesktopAppTarget {
    Application(PathBuf),
    #[cfg(target_os = "windows")]
    WindowsAppId(String),
}

impl AgentClient {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude-code" => Ok(Self::ClaudeCode),
            "claude-desktop" => Ok(Self::ClaudeDesktop),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            "openclaw" => Ok(Self::OpenClaw),
            "hermes" => Ok(Self::Hermes),
            "deepseek-harness" => Ok(Self::DeepSeekHarness),
            "zcode" => Ok(Self::ZCode),
            "workbuddy" => Ok(Self::WorkBuddy),
            "antigravity-cli" => Ok(Self::AntigravityCli),
            "kimi-code" => Ok(Self::KimiCode),
            "grok-build" => Ok(Self::GrokBuild),
            "omp" => Ok(Self::Omp),
            _ => Err(format!("Unsupported agent client: {value}")),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::OpenClaw => "openclaw",
            Self::Hermes => "hermes",
            Self::DeepSeekHarness => "deepseek-harness",
            Self::ZCode => "zcode",
            Self::WorkBuddy => "workbuddy",
            Self::AntigravityCli => "antigravity-cli",
            Self::KimiCode => "kimi-code",
            Self::GrokBuild => "grok-build",
            Self::Omp => "omp",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::OpenClaw => "OpenClaw",
            Self::Hermes => "Hermes Agent",
            Self::DeepSeekHarness => "DeepSeek Harness",
            Self::ZCode => "ZCode",
            Self::WorkBuddy => "WorkBuddy",
            Self::AntigravityCli => "Antigravity CLI",
            Self::KimiCode => "Kimi Code",
            Self::GrokBuild => "Grok Build",
            Self::Omp => "Oh My Pi",
        }
    }

    fn supported_platform(self) -> bool {
        self != Self::ClaudeDesktop
            || cfg!(any(
                target_os = "windows",
                target_os = "macos",
                target_os = "linux"
            ))
    }

    fn executable_names(self) -> &'static [&'static str] {
        match self {
            Self::ClaudeCode => &["claude"],
            Self::ClaudeDesktop => &[],
            Self::Codex => &["codex"],
            Self::OpenCode => &["opencode"],
            Self::OpenClaw => &["openclaw"],
            Self::Hermes => &["hermes"],
            Self::DeepSeekHarness => &["dsh"],
            Self::ZCode => &["zcode"],
            Self::WorkBuddy => &[],
            Self::AntigravityCli => &["agy"],
            Self::KimiCode => &["kimi"],
            Self::GrokBuild => &["grok"],
            Self::Omp => &["omp"],
        }
    }
}

struct AgentFileUpdate {
    path: PathBuf,
    after: String,
}

struct AgentConfigurationOptions<'a> {
    models: &'a [AgentModelOption],
    codex_catalog: Option<&'a str>,
    oauth_configuration: bool,
    claude_code_model_mappings: Option<&'a ClaudeDesktopModelMappings>,
    claude_desktop_model_mappings: Option<&'a ClaudeDesktopModelMappings>,
}

struct PreparedAgentModels {
    models: Vec<AgentModelOption>,
    codex_catalog: Option<String>,
}

type FileSnapshot = (PathBuf, Option<Vec<u8>>);
#[cfg(test)]
type AgentRecordExtension = (AgentModificationRecord, Vec<FileSnapshot>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiNetworkSettings {
    port: u16,
    allow_lan: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiNetworkRoutingSettings {
    port: u16,
    allow_lan: bool,
    routing_session_affinity: bool,
    routing_session_affinity_ttl: String,
    #[serde(default)]
    disable_cooling: bool,
    request_retry: u32,
    max_retry_credentials: u32,
    max_retry_interval: i64,
    streaming_bootstrap_retries: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiNetworkEndpointSettings {
    host: String,
    port: u16,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default)]
    proxy_override: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiRetrySettings {
    #[serde(default)]
    disable_cooling: bool,
    request_retry: u32,
    max_retry_credentials: u32,
    max_retry_interval: i64,
    streaming_bootstrap_retries: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuiSessionRoutingSettings {
    routing_session_affinity: bool,
    routing_session_affinity_ttl: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreTlsSettings {
    enabled: bool,
    cert: String,
    key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreSensitiveWordsSettings {
    antigravity_sensitive_words: Vec<String>,
    devin_sensitive_words: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreLoggingSettingsInput {
    debug: bool,
    commercial_mode: bool,
    logging_to_file: bool,
    logs_max_total_size_mb: u32,
    error_logs_max_files: u32,
    usage_statistics_enabled: bool,
    redis_usage_queue_retention_seconds: u32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreConfigSettings {
    #[serde(skip_serializing)]
    host: String,
    #[serde(skip_serializing)]
    port: u16,
    #[serde(skip_serializing)]
    auth_dir: String,
    api_keys: Vec<String>,
    management_secret_configured: bool,
    debug: bool,
    commercial_mode: bool,
    logging_to_file: bool,
    logs_max_total_size_mb: u32,
    error_logs_max_files: u32,
    usage_statistics_enabled: bool,
    redis_usage_queue_retention_seconds: u32,
    request_log: bool,
    plugins_enabled: bool,
    routing_strategy: String,
    proxy_url: String,
    routing_session_affinity: bool,
    routing_session_affinity_ttl: String,
    disable_cooling: bool,
    request_retry: u32,
    max_retry_credentials: u32,
    max_retry_interval: i64,
    streaming_bootstrap_retries: u32,
    #[allow(dead_code)]
    #[serde(skip_serializing)]
    management_secret_key: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreApiKeyView {
    api_key: String,
    remark: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreConfigView {
    host: String,
    api_keys: Vec<CoreApiKeyView>,
    management_secret_configured: bool,
    port: u16,
    allow_lan: bool,
    debug: bool,
    commercial_mode: bool,
    logging_to_file: bool,
    logs_max_total_size_mb: u32,
    error_logs_max_files: u32,
    usage_statistics_enabled: bool,
    redis_usage_queue_retention_seconds: u32,
    request_log: bool,
    plugins_enabled: bool,
    routing_strategy: String,
    proxy_url: String,
    proxy_override: bool,
    routing_session_affinity: bool,
    routing_session_affinity_ttl: String,
    disable_cooling: bool,
    request_retry: u32,
    max_retry_credentials: u32,
    max_retry_interval: i64,
    streaming_bootstrap_retries: u32,
}

impl Default for CoreInstallTask {
    fn default() -> Self {
        Self {
            running: false,
            cancellable: false,
            phase: "Idle".to_string(),
            downloaded: 0,
            total: None,
            percent: None,
            message: None,
            result: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreMetadata {
    version: String,
    asset_name: String,
    installed_at_unix: u64,
}

struct DownloadedArchive {
    size: u64,
    sha256: String,
}

impl CoreDownloadState {
    fn start(&self, token: CancellationToken, version: Option<String>) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Kernel installation status lock is poisoned".to_string())?;

        if inner.running {
            return Err("A kernel installation task is already running".to_string());
        }

        inner.running = true;
        inner.token = Some(token);
        inner.task = CoreInstallTask {
            running: true,
            cancellable: true,
            phase: version
                .map(|version| format!("Preparing to install {version}"))
                .unwrap_or_else(|| "Preparing to install the latest version".to_string()),
            downloaded: 0,
            total: None,
            percent: None,
            message: None,
            result: None,
        };

        Ok(())
    }

    fn cancel(&self) {
        if let Ok(inner) = self.inner.lock() {
            if let Some(token) = &inner.token {
                token.cancel();
            }
        }
    }

    fn snapshot(&self) -> CoreInstallTask {
        self.inner
            .lock()
            .map(|inner| inner.task.clone())
            .unwrap_or_default()
    }

    fn progress(
        &self,
        window: &tauri::Window,
        phase: &str,
        downloaded: u64,
        total: Option<u64>,
        cancellable: bool,
    ) {
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| downloaded as f64 * 100.0 / total as f64);

        let task = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };

            inner.task.running = inner.running;
            inner.task.cancellable = cancellable;
            inner.task.phase = phase.to_string();
            inner.task.downloaded = downloaded;
            inner.task.total = total;
            inner.task.percent = percent;
            inner.task.clone()
        };

        let _ = window.emit(CORE_INSTALL_PROGRESS_EVENT, task);
    }

    fn finish(&self, window: &tauri::Window, result: Result<CoreInstallResult, String>) {
        let task = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };

            inner.running = false;
            inner.token = None;
            inner.task.running = false;
            inner.task.cancellable = false;

            match result {
                Ok(result) => {
                    inner.task.phase = "Installation complete".to_string();
                    inner.task.downloaded = 1;
                    inner.task.total = Some(1);
                    inner.task.percent = Some(100.0);
                    inner.task.message = Some(format!("{} installation complete", result.version));
                    inner.task.result = Some(result);
                }
                Err(error) => {
                    inner.task.phase = if error.to_ascii_lowercase().contains("cancel") {
                        "Canceled".to_string()
                    } else {
                        "Installation failed".to_string()
                    };
                    inner.task.message = Some(error);
                    inner.task.result = None;
                }
            }

            inner.task.clone()
        };

        let _ = window.emit(CORE_INSTALL_PROGRESS_EVENT, task);
    }
}

impl AppUpdateState {
    fn snapshot(&self) -> AppUpdateTask {
        self.inner
            .lock()
            .map(|inner| inner.task.clone())
            .unwrap_or_default()
    }

    fn set_available(&self, task: AppUpdateTask) {
        if let Ok(mut inner) = self.inner.lock() {
            if !inner.task.running {
                inner.task = task;
            }
        }
    }

    fn start(&self, token: CancellationToken) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Application update status lock is poisoned".to_string())?;
        if inner.task.running {
            return Err("An application update task is already running".to_string());
        }
        inner.token = Some(token);
        inner.task = AppUpdateTask {
            running: true,
            cancellable: true,
            phase: "checking".to_string(),
            ..AppUpdateTask::default()
        };
        Ok(())
    }

    fn start_download(&self, pending: &PendingAppUpdate) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Application update status lock is poisoned".to_string())?;
        if !inner.task.running || inner.task.phase != "checking" {
            return Err("Application update is not in the checking stage".to_string());
        }
        if inner
            .token
            .as_ref()
            .is_none_or(CancellationToken::is_cancelled)
        {
            return Err("Application update download canceled".to_string());
        }
        inner.task = AppUpdateTask {
            running: true,
            cancellable: true,
            phase: "downloading".to_string(),
            target_version: Some(pending.version.clone()),
            downloaded_bytes: 0,
            total_bytes: Some(pending.asset.size_bytes),
            percent: Some(0.0),
            message: None,
        };
        Ok(())
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn update_task<F>(&self, update: F) -> AppUpdateTask
    where
        F: FnOnce(&mut AppUpdateTask),
    {
        let Ok(mut inner) = self.inner.lock() else {
            return AppUpdateTask::default();
        };
        update(&mut inner.task);
        inner.task.clone()
    }

    fn finish(&self, phase: &str, message: Option<String>) -> AppUpdateTask {
        let Ok(mut inner) = self.inner.lock() else {
            return AppUpdateTask::default();
        };
        inner.task.running = false;
        inner.task.cancellable = false;
        inner.task.phase = phase.to_string();
        inner.task.message = message;
        inner.token = None;
        inner.task.clone()
    }

    fn cancel(&self) {
        if let Ok(inner) = self.inner.lock() {
            if let Some(token) = &inner.token {
                token.cancel();
            }
        }
    }
}

impl CoreProcessState {
    fn new(starting: bool) -> Self {
        Self {
            child: Mutex::new(None),
            adopted_processes: Mutex::new(Vec::new()),
            starting: AtomicBool::new(starting),
            shutting_down: AtomicBool::new(false),
            shutdown_complete: AtomicBool::new(false),
        }
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    fn ensure_active(&self) -> Result<(), String> {
        if self.is_shutting_down() {
            Err("Application is exiting; kernel operation canceled".to_string())
        } else {
            Ok(())
        }
    }

    fn is_starting(&self) -> bool {
        self.starting.load(Ordering::Acquire)
    }

    fn set_starting(&self, starting: bool) {
        self.starting.store(starting, Ordering::Release);
    }

    fn managed_pid(&self) -> Option<u32> {
        if let Ok(mut child) = self.child.lock() {
            if let Some(process) = child.as_mut() {
                if let Ok(None) = process.try_wait() {
                    return Some(process.id());
                }

                *child = None;
            }
        }

        self.adopted_processes
            .lock()
            .ok()
            .and_then(|mut processes| {
                processes.retain(|process| {
                    process_executable_path(process.process_id)
                        .as_deref()
                        .is_some_and(|path| executable_paths_match(&process.binary_path, path))
                });
                processes.first().map(|process| process.process_id)
            })
    }

    fn take_child(&self) -> Option<CoreChild> {
        self.child.lock().ok().and_then(|mut child| child.take())
    }

    fn adopt_process_ids(&self, binary_path: &Path, process_ids: Vec<u32>) -> Result<(), String> {
        let mut adopted = self
            .adopted_processes
            .lock()
            .map_err(|_| "Adopted kernel process status lock is poisoned".to_string())?;
        *adopted = process_ids
            .into_iter()
            .filter(|process_id| is_process_alive(*process_id))
            .map(|process_id| AdoptedCoreProcess {
                process_id,
                binary_path: binary_path.to_path_buf(),
            })
            .collect();
        adopted.sort_unstable_by_key(|process| process.process_id);
        adopted.dedup_by_key(|process| process.process_id);
        Ok(())
    }

    fn clear_adopted_processes(&self) -> Result<(), String> {
        self.adopted_processes
            .lock()
            .map(|mut processes| processes.clear())
            .map_err(|_| "Adopted kernel process status lock is poisoned".to_string())
    }

    fn take_adopted_processes(&self) -> Vec<AdoptedCoreProcess> {
        self.adopted_processes
            .lock()
            .map(|mut processes| std::mem::take(&mut *processes))
            .unwrap_or_default()
    }

    fn store_child(&self, child: CoreChild) -> Result<u32, String> {
        self.ensure_active()?;
        let pid = child.id();
        self.clear_adopted_processes()?;
        let mut managed_child = self
            .child
            .lock()
            .map_err(|_| "Kernel process status lock is poisoned".to_string())?;
        *managed_child = Some(child);
        Ok(pid)
    }
}

impl MainWindowSizeState {
    fn new(size: Option<SavedWindowSize>) -> Self {
        Self {
            inner: Mutex::new(size),
        }
    }

    fn snapshot(&self) -> Result<Option<SavedWindowSize>, String> {
        self.inner
            .lock()
            .map(|size| *size)
            .map_err(|_| "Main window size status lock is poisoned".to_string())
    }

    fn replace(&self, size: SavedWindowSize) -> Result<(), String> {
        let mut current = self
            .inner
            .lock()
            .map_err(|_| "Main window size status lock is poisoned".to_string())?;
        *current = Some(size);
        Ok(())
    }
}

impl GuiConfigState {
    fn new(config: GuiConfigFile) -> Self {
        Self {
            inner: Mutex::new(config),
        }
    }

    fn snapshot(&self) -> Result<GuiConfigFile, String> {
        self.inner
            .lock()
            .map(|config| config.clone())
            .map_err(|_| "GUI configuration status lock is poisoned".to_string())
    }

    fn replace_external(&self, config: GuiConfigFile) -> Result<(), String> {
        let mut current = self
            .inner
            .lock()
            .map_err(|_| "GUI configuration status lock is poisoned".to_string())?;
        *current = config;
        Ok(())
    }

    fn replace_core_settings_external(
        &self,
        settings: &CoreConfigSettings,
    ) -> Result<GuiConfigFile, String> {
        let mut current = self
            .inner
            .lock()
            .map_err(|_| "GUI configuration status lock is poisoned".to_string())?;
        let mut config = current.clone();
        apply_core_settings_to_gui_config(&mut config, settings);
        apply_external_core_proxy_override(&mut config, settings)?;
        sanitize_gui_config(&mut config)?;
        validate_gui_config(&config)?;
        *current = config.clone();
        Ok(config)
    }

    fn update_network(&self, port: u16, allow_lan: bool) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.port = port;
            config.allow_lan = allow_lan;
            config.host = if allow_lan { "0.0.0.0" } else { "127.0.0.1" }.to_string();
            Ok(())
        })
    }

    fn update_network_routing(&self, settings: &GuiConfigFile) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.port = settings.port;
            config.allow_lan = settings.allow_lan;
            config.host = settings.host.clone();
            config.proxy_url = settings.proxy_url.clone();
            config.routing_session_affinity = settings.routing_session_affinity;
            config.routing_session_affinity_ttl = settings.routing_session_affinity_ttl.clone();
            config.disable_cooling = settings.disable_cooling;
            config.request_retry = settings.request_retry;
            config.max_retry_credentials = settings.max_retry_credentials;
            config.max_retry_interval = settings.max_retry_interval;
            config.streaming_bootstrap_retries = settings.streaming_bootstrap_retries;
            Ok(())
        })
    }

    fn update_network_endpoint(&self, settings: &GuiConfigFile) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.host = settings.host.clone();
            config.allow_lan = settings.allow_lan;
            config.port = settings.port;
            config.proxy_url = settings.proxy_url.clone();
            config.proxy_override = settings.proxy_override;
            Ok(())
        })
    }

    fn update_retry_settings(&self, settings: &GuiConfigFile) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.disable_cooling = settings.disable_cooling;
            config.request_retry = settings.request_retry;
            config.max_retry_credentials = settings.max_retry_credentials;
            config.max_retry_interval = settings.max_retry_interval;
            config.streaming_bootstrap_retries = settings.streaming_bootstrap_retries;
            Ok(())
        })
    }

    fn update_session_routing(&self, settings: &GuiConfigFile) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.routing_session_affinity = settings.routing_session_affinity;
            config.routing_session_affinity_ttl = settings.routing_session_affinity_ttl.clone();
            Ok(())
        })
    }

    fn set_run_on_startup(&self, run_on_startup: bool) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.run_on_startup = run_on_startup;
            Ok(())
        })
    }

    fn set_close_behavior(
        &self,
        close_behavior: WindowsCloseBehavior,
    ) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.close_behavior = close_behavior;
            Ok(())
        })
    }

    fn set_software_preferences(
        &self,
        close_behavior: WindowsCloseBehavior,
        start_core_on_launch: bool,
        silent_start: bool,
        default_terminal: String,
    ) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.close_behavior = close_behavior;
            config.start_core_on_launch = start_core_on_launch;
            config.silent_start = silent_start;
            config.default_terminal = normalize_agent_terminal(&default_terminal);
            Ok(())
        })
    }

    fn set_locale(&self, locale: String) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.locale = normalize_app_locale(&locale).to_string();
            Ok(())
        })
    }

    fn set_download_source(
        &self,
        download_source: VersionDownloadSource,
    ) -> Result<GuiConfigFile, String> {
        self.set_download_candidate(VersionDownloadCandidate::builtin(download_source))
    }

    fn set_download_candidate(
        &self,
        candidate: VersionDownloadCandidate,
    ) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            if let Some(url) = &candidate.custom_url {
                if !config.custom_download_mirrors.contains(url) {
                    return Err("Custom mirror does not exist".to_string());
                }
                config.active_custom_download_mirror = url.clone();
            }
            config.download_source = candidate.source;
            config.prefer_gitcode_downloads = candidate.source == VersionDownloadSource::Gitcode;
            Ok(())
        })
    }

    fn switch_download_source_after_failure(
        &self,
        expected_source: &VersionDownloadCandidate,
        fallback_source: &VersionDownloadCandidate,
    ) -> Result<Option<GuiConfigFile>, String> {
        let mut current = self
            .inner
            .lock()
            .map_err(|_| "GUI configuration status lock is poisoned".to_string())?;
        if current.selected_download_candidate() != *expected_source
            || expected_source == fallback_source
        {
            return Ok(None);
        }

        let mut config = current.clone();
        config.download_source = fallback_source.source;
        if let Some(url) = &fallback_source.custom_url {
            config.active_custom_download_mirror = url.clone();
        }
        config.prefer_gitcode_downloads = fallback_source.source == VersionDownloadSource::Gitcode;
        sanitize_gui_config(&mut config)?;
        write_gui_config(&config)?;
        *current = config.clone();
        Ok(Some(config))
    }

    fn set_window_size(&self, size: SavedWindowSize) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.window_width = Some(size.width);
            config.window_height = Some(size.height);
            Ok(())
        })
    }

    fn set_management_secret_key(&self, secret_key: String) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.management_secret_key = secret_key;
            Ok(())
        })
    }

    fn sync_core_settings(&self, settings: &CoreConfigSettings) -> Result<GuiConfigFile, String> {
        self.sync_core_settings_internal(settings, None, false, false)
    }

    fn sync_core_logging_settings(&self, settings: &CoreConfigSettings) -> Result<GuiConfigFile, String> {
        self.sync_core_settings_internal(settings, None, false, true)
    }

    fn sync_core_settings_external(
        &self,
        settings: &CoreConfigSettings,
    ) -> Result<GuiConfigFile, String> {
        self.sync_core_settings_internal(settings, None, true, false)
    }

    fn sync_core_settings_with_api_key(
        &self,
        settings: &CoreConfigSettings,
        added_api_key: Option<GuiApiKeyEntry>,
    ) -> Result<GuiConfigFile, String> {
        self.sync_core_settings_internal(settings, added_api_key, false, false)
    }

    fn sync_core_settings_internal(
        &self,
        settings: &CoreConfigSettings,
        added_api_key: Option<GuiApiKeyEntry>,
        apply_external_proxy: bool,
        save_usage_preference: bool,
    ) -> Result<GuiConfigFile, String> {
        self.update(|config| {
            config.api_keys = merge_core_api_keys_with_gui_metadata(
                &config.api_keys,
                &settings.api_keys,
                added_api_key.as_ref(),
            );
            config.host = settings.host.clone();
            config.port = settings.port;
            config.allow_lan = !is_loopback_host(&settings.host);
            config.auth_dir = settings.auth_dir.clone();
            config.debug = settings.debug;
            config.commercial_mode = settings.commercial_mode;
            config.logging_to_file = settings.logging_to_file;
            config.logs_max_total_size_mb = settings.logs_max_total_size_mb;
            config.error_logs_max_files = settings.error_logs_max_files;
            config.usage_statistics_enabled = settings.usage_statistics_enabled;
            if save_usage_preference {
                config.usage_statistics_disabled = (!settings.usage_statistics_enabled).then_some(true);
            }
            config.redis_usage_queue_retention_seconds =
                settings.redis_usage_queue_retention_seconds;
            config.request_log = settings.request_log;
            if let Some(secret_key) = settings
                .management_secret_key
                .as_deref()
                .filter(|secret_key| !is_hashed_management_secret_key(secret_key))
            {
                config.management_secret_key = secret_key.to_string();
            }
            config.plugins_enabled = settings.plugins_enabled;
            config.routing_strategy = settings.routing_strategy.clone();
            config.routing_session_affinity = settings.routing_session_affinity;
            config.routing_session_affinity_ttl = settings.routing_session_affinity_ttl.clone();
            config.disable_cooling = settings.disable_cooling;
            config.request_retry = settings.request_retry;
            config.max_retry_credentials = settings.max_retry_credentials;
            config.max_retry_interval = settings.max_retry_interval;
            config.streaming_bootstrap_retries = settings.streaming_bootstrap_retries;
            if apply_external_proxy {
                apply_external_core_proxy_override(config, settings)?;
            }
            Ok(())
        })
    }

    fn update<F>(&self, update: F) -> Result<GuiConfigFile, String>
    where
        F: FnOnce(&mut GuiConfigFile) -> Result<(), String>,
    {
        let mut current = self
            .inner
            .lock()
            .map_err(|_| "GUI configuration status lock is poisoned".to_string())?;
        let mut config = current.clone();
        update(&mut config)?;
        sanitize_gui_config(&mut config)?;
        write_gui_config(&config)?;
        *current = config.clone();
        Ok(config)
    }
}

impl From<&GuiConfigFile> for GuiSettings {
    fn from(config: &GuiConfigFile) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            allow_lan: config.allow_lan,
            run_on_startup: config.run_on_startup,
            close_behavior: config.close_behavior,
        }
    }
}

impl From<&GuiConfigFile> for CoreConfigSettings {
    fn from(config: &GuiConfigFile) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            auth_dir: config.auth_dir.clone(),
            api_keys: gui_api_key_values(&config.api_keys),
            management_secret_configured: !config.management_secret_key.is_empty(),
            debug: config.debug,
            commercial_mode: config.commercial_mode,
            logging_to_file: config.logging_to_file,
            logs_max_total_size_mb: config.logs_max_total_size_mb,
            error_logs_max_files: config.error_logs_max_files,
            usage_statistics_enabled: config.usage_statistics_enabled,
            redis_usage_queue_retention_seconds: config.redis_usage_queue_retention_seconds,
            request_log: config.request_log,
            plugins_enabled: config.plugins_enabled,
            routing_strategy: config.routing_strategy.clone(),
            proxy_url: config.proxy_url.clone(),
            routing_session_affinity: config.routing_session_affinity,
            routing_session_affinity_ttl: config.routing_session_affinity_ttl.clone(),
            disable_cooling: config.disable_cooling,
            request_retry: config.request_retry,
            max_retry_credentials: config.max_retry_credentials,
            max_retry_interval: config.max_retry_interval,
            streaming_bootstrap_retries: config.streaming_bootstrap_retries,
            management_secret_key: Some(config.management_secret_key.clone()),
        }
    }
}

impl From<&GuiConfigFile> for CoreConfigView {
    fn from(config: &GuiConfigFile) -> Self {
        Self {
            host: config.host.clone(),
            api_keys: config
                .api_keys
                .iter()
                .map(|entry| CoreApiKeyView {
                    api_key: entry.key.clone(),
                    remark: entry.remark.clone(),
                })
                .collect(),
            management_secret_configured: !config.management_secret_key.is_empty(),
            port: config.port,
            allow_lan: config.allow_lan,
            debug: config.debug,
            commercial_mode: config.commercial_mode,
            logging_to_file: config.logging_to_file,
            logs_max_total_size_mb: config.logs_max_total_size_mb,
            error_logs_max_files: config.error_logs_max_files,
            usage_statistics_enabled: config.usage_statistics_enabled,
            redis_usage_queue_retention_seconds: config.redis_usage_queue_retention_seconds,
            request_log: config.request_log,
            plugins_enabled: config.plugins_enabled,
            routing_strategy: config.routing_strategy.clone(),
            proxy_url: config.proxy_url.clone(),
            proxy_override: config.proxy_override,
            routing_session_affinity: config.routing_session_affinity,
            routing_session_affinity_ttl: config.routing_session_affinity_ttl.clone(),
            disable_cooling: config.disable_cooling,
            request_retry: config.request_retry,
            max_retry_credentials: config.max_retry_credentials,
            max_retry_interval: config.max_retry_interval,
            streaming_bootstrap_retries: config.streaming_bootstrap_retries,
        }
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    fallback_download_urls: Vec<String>,
    size: Option<u64>,
    digest: Option<String>,
}

fn main() {
    let helper_args = env::args_os().collect::<Vec<_>>();
    if antigravity_helper_requested(&helper_args) {
        let code = run_antigravity_helper(&helper_args).unwrap_or_else(|error| {
            eprintln!("{error}");
            1
        });
        std::process::exit(code);
    }
    let mut args = env::args_os();
    while let Some(argument) = args.next() {
        if argument == "--portable-update-helper" {
            let result = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "Application update helper is missing its descriptor file".to_string())
                .and_then(|path| run_portable_update_helper(&path));
            if let Err(error) = result {
                eprintln!("{error}");
            }
            return;
        }
    }

    let _instance_guard = match acquire_app_instance_guard() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    };

    let portable_update_ack = portable_update_ack_argument();
    let gui_config = match load_or_create_gui_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            if portable_update_ack.is_some() {
                return;
            }
            let mut config = GuiConfigFile::default();
            // If the GUI configuration cannot be read before the kernel starts,
            // use the shared recovery key so requests and the generated kernel
            // configuration stay synchronized.
            config.management_secret_key = LEGACY_DEFAULT_MANAGEMENT_SECRET_KEY.to_string();
            if let Err(sanitize_error) = sanitize_gui_config(&mut config) {
                eprintln!("Failed to initialize fixed credentials directory: {sanitize_error}");
            }
            config
        }
    };
    let initial_window_size = configured_window_size(&gui_config);
    let start_hidden = should_start_hidden(&gui_config);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(CoreDownloadState::default())
        .manage(AppUpdateState::default())
        .manage(CoreProcessState::new(gui_config.start_core_on_launch))
        .manage(DeepSeekHarnessProcessState::default())
        .manage(usage::UsageCollectorState::default())
        .manage(GuiConfigState::new(gui_config))
        .manage(MainWindowSizeState::new(initial_window_size))
        .manage(AgentConfigStatusCache::default());

    let app = app.on_window_event(|window, event| {
        if window.label() != "main" {
            return;
        }

        let observed_size = match event {
            tauri::WindowEvent::Resized(physical_size) => {
                window.scale_factor().ok().and_then(|scale_factor| {
                    logical_window_size_from_physical(physical_size, scale_factor)
                })
            }
            tauri::WindowEvent::ScaleFactorChanged {
                scale_factor,
                new_inner_size,
                ..
            } => logical_window_size_from_physical(new_inner_size, *scale_factor),
            _ => None,
        };

        if let Some(observed_size) = observed_size {
            let window_size_state = window.state::<MainWindowSizeState>();
            if let Err(error) = window_size_state.replace(observed_size) {
                eprintln!("Failed to record main window size: {error}");
            }
        }
    });

    #[cfg(target_os = "macos")]
    let app = app.on_window_event(|window, event| {
        if window.label() == "main" {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                set_macos_dock_visible(window.app_handle(), false);
                if let Err(error) = window.hide() {
                    eprintln!("Failed to hide main window: {error}");
                    set_macos_dock_visible(window.app_handle(), true);
                }
            }
        }
    });

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let app = app.on_window_event(|window, event| {
        if window.label() == "main" {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.emit(WINDOWS_CLOSE_REQUEST_EVENT, ()) {
                    eprintln!("Failed to show desktop close confirmation: {error}");
                }
            }
        }
    });

    let app = app
        .setup(move |app| {
            if let Err(error) = network_proxy::refresh(app.state::<GuiConfigState>().inner()) {
                eprintln!("Failed to read startup proxy settings: {error}");
            }
            if let Err(error) = codex_catalog::validate_embedded_catalog() {
                eprintln!("Invalid bundled Codex model catalog: {error}");
            }
            if let Err(error) = load_codex_model_catalog_override(app.handle()) {
                eprintln!("Failed to load Codex model catalog update file; using bundled catalog: {error}");
            }
            let catalog_update_app = app.handle().clone();
            if let Err(error) = load_codex_model_customizations(app.handle()) {
                eprintln!("Failed to load Codex custom model configuration: {error}");
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = update_codex_model_catalog_inner(&catalog_update_app).await {
                    eprintln!("Failed to update Codex model catalog in the background; continuing with current catalog: {error}");
                }
            });
            if let Err(error) = restore_main_window_size(app.handle()) {
                eprintln!("{error}");
            }

            #[cfg(target_os = "macos")]
            setup_macos_tray(app)?;
            #[cfg(target_os = "linux")]
            if let Err(error) = setup_windows_tray(app) {
                eprintln!(
                    "Failed to set up Linux tray; keeping the main window available: {error}"
                );
            }
            #[cfg(target_os = "windows")]
            setup_windows_tray(app)?;

            if let Err(error) = configure_initial_main_window(app.handle(), start_hidden) {
                eprintln!("Failed to configure startup window state: {error}");
            }

            if let Err(error) =
                configuration_watcher::start_configuration_file_watcher(app.handle().clone())
            {
                eprintln!("Failed to start configuration file watcher: {error}");
            }

            network_proxy::start_monitor(app.handle().clone());
            start_codex_model_catalog_sync(app.handle().clone());

            let usage_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = usage::initialize_usage_storage() {
                    eprintln!("Failed to initialize usage records directory: {error}");
                }
                usage::start_usage_collector(usage_app);
            });

            let agent_status_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let gui_config_state = agent_status_app.state::<GuiConfigState>();
                let cache = agent_status_app.state::<AgentConfigStatusCache>();
                if let Err(error) = refresh_agent_config_status_cache(
                    &agent_status_app,
                    gui_config_state.inner(),
                    cache.inner(),
                ) {
                    eprintln!("Failed to refresh agent configuration status in the background: {error}");
                }
            });

            let core_app = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let Ok(_guard) = CORE_OPERATION_LOCK.lock() else {
                    return;
                };
                let gui_config_state = core_app.state::<GuiConfigState>();
                let process_state = core_app.state::<CoreProcessState>();
                if process_state.ensure_active().is_err() {
                    process_state.set_starting(false);
                    return;
                }
                let Ok(config) = gui_config_state.snapshot() else {
                    process_state.set_starting(false);
                    return;
                };

                if config.start_core_on_launch {
                    if let Ok(status) =
                        current_core_status(Some(process_state.inner()), Some(config.port))
                    {
                        emit_core_status(&core_app, &status);
                    }
                }

                match auto_install_bundled_core_if_needed(&core_app) {
                    Ok(true) => eprintln!("Automatically installed or upgraded to the CPA kernel version bundled with the application"),
                    Ok(false) => {}
                    Err(error) => eprintln!("Failed to automatically install CPA offline kernel: {error}"),
                }

                let adopted_process_ids = match adopt_existing_core_processes(process_state.inner())
                {
                    Ok(process_ids) => process_ids,
                    Err(error) => {
                        eprintln!("Failed to scan for and adopt the CPA kernel in the current directory: {error}");
                        Vec::new()
                    }
                };
                if !adopted_process_ids.is_empty() {
                    eprintln!(
                        "Adopted running CPA kernel in the current directory: PID {}",
                        adopted_process_ids
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                } else if should_start_core_on_launch(&config) {
                    if let Err(error) = start_core_process_inner(process_state.inner(), &config) {
                        eprintln!("Failed to automatically start CPA kernel: {error}");
                    }
                }

                process_state.set_starting(false);

                if let Ok(status) =
                    current_core_status(Some(process_state.inner()), Some(config.port))
                {
                    emit_core_status(&core_app, &status);
                }
            });

            if let Some(ack_path) = portable_update_ack.as_ref() {
                fs::write(ack_path, env!("CARGO_PKG_VERSION").as_bytes())
                    .map_err(|error| format!("Failed to write application update startup confirmation: {error}"))?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_theme::get_linux_system_theme,
            desktop_theme::get_theme_preference,
            desktop_theme::save_theme_preference,
            health_check,
            detect_core_platform,
            get_core_status,
            get_gui_settings,
            get_usage_view_preferences,
            save_usage_view_preferences,
            resolve_api_access_remarks,
            save_api_access_remark,
            set_app_locale,
            resolve_windows_close_request,
            get_software_settings,
            save_software_settings,
            get_agent_config_statuses,
            create_agent_config_backup,
            list_agent_config_backups,
            preview_agent_config_backup,
            restore_agent_config_backup,
            delete_agent_config_backup,
            preview_agent_config_template,
            apply_agent_config_template,
            refresh_agent_config_statuses,
            get_agent_models,
            check_pi_provider_update,
            install_pi_provider,
            update_pi_provider,
            repair_pi_provider,
            uninstall_pi_provider,
            check_codex_oauth_login,
            update_codex_model_catalog,
            get_deepseek_harness_model_catalog_editor,
            save_deepseek_harness_model_catalog_editor,
            get_codex_model_catalog_editor,
            save_codex_model_catalog_editor,
            get_thinking_aliases,
            get_model_alias_sources,
            get_model_alias_edit_source,
            get_thinking_alias_sources,
            create_thinking_alias,
            delete_thinking_alias,
            get_speed_aliases,
            get_speed_alias_sources,
            create_speed_alias,
            delete_speed_alias,
            apply_agent_config,
            clear_codex_config,
            restore_codex_official_config,
            close_codex_config_modification,
            set_agent_config_enabled,
            update_agent_config,
            launch_agent,
            get_deepseek_harness_process_status,
            stop_deepseek_harness_process,
            restart_deepseek_harness_process,
            restart_agent_app,
            restart_codex_app,
            restart_opencode_app,
            get_lan_ipv4,
            save_gui_settings,
            save_network_routing_settings,
            save_network_endpoint_settings,
            save_retry_settings,
            save_session_routing_settings,
            get_core_tls_settings,
            save_core_tls_settings,
            get_core_sensitive_words_settings,
            save_core_sensitive_words_settings,
            get_core_config_settings,
            get_extended_core_config,
            save_extended_core_config,
            save_core_logging_settings,
            set_core_request_log,
            add_core_api_key,
            update_core_api_key,
            delete_core_api_key,
            set_core_management_secret_key,
            clear_core_management_secret_key,
            management_api::management_request,
            plugins::get_plugin_support,
            plugins::get_plugin_resource_url,
            provider_health::provider_health_probe,
            provider_health::get_core_models,
            provider_health::core_health_probe,
            management_api::upload_auth_file,
            management_api::import_vertex_credential,
            management_api::open_auth_files_directory,
            management_api::open_core_logs_directory,
            set_core_plugins_enabled,
            set_core_routing_strategy,
            set_core_proxy_url,
            set_core_session_affinity,
            set_core_session_affinity_ttl,
            management_api::start_oauth_login,
            management_api::get_oauth_status,
            management_api::submit_oauth_callback,
            list_oauth_browsers,
            open_oauth_url,
            open_external_url,
            get_version_source_settings,
            set_download_source,
            add_custom_download_mirror,
            remove_custom_download_mirror,
            set_prefer_gitcode_downloads,
            check_app_update,
            get_app_update_task,
            start_app_update,
            cancel_app_update,
            check_latest_core,
            detect_bundled_core,
            install_core_version,
            install_bundled_core,
            cancel_core_install,
            get_core_install_task,
            usage::get_usage_collector_status,
            usage::get_usage_overview,
            usage::get_usage_analysis,
            usage::get_usage_events,
            usage::save_usage_events_export,
            usage::get_usage_pricing,
            usage::get_usage_storage_settings,
            usage::repair_usage_cache_records,
            usage::save_usage_storage_settings,
            usage::shrink_usage_database,
            usage::save_usage_model_price,
            usage::delete_usage_model_price,
            usage::preview_usage_model_prices,
            usage::apply_usage_model_prices,
            start_core_process,
            stop_core_process,
            restart_core_process,
            codex_sessions::list_codex_sessions,
            codex_sessions::delete_codex_sessions,
            codex_sessions::repair_codex_session_metadata,
            codex_sessions::preview_codex_session_index_cleanup,
            codex_sessions::apply_codex_session_index_cleanup
        ])
        .build(tauri::generate_context!())
        .expect("failed to build app");

    app.run(|app_handle, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => show_main_window(app_handle),
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            let process_state = app_handle.state::<CoreProcessState>();
            if process_state.shutdown_complete.load(Ordering::Acquire) {
                return;
            }
            api.prevent_exit();
            if process_state.shutting_down.swap(true, Ordering::AcqRel) {
                return;
            }
            if let Err(error) = persist_main_window_size(app_handle) {
                eprintln!("Failed to save main window size: {error}");
            }
            app_handle.state::<CoreDownloadState>().cancel();
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _guard = CORE_OPERATION_LOCK
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let process_state = app_handle.state::<CoreProcessState>();
                let gui_config_state = app_handle.state::<GuiConfigState>();
                shutdown_managed_core(process_state.inner(), gui_config_state.inner());
                process_state
                    .shutdown_complete
                    .store(true, Ordering::Release);
                app_handle.exit(code.unwrap_or(0));
            });
        }
        tauri::RunEvent::Exit => {
            usage::stop_usage_collector(app_handle);
            let deepseek_process_state = app_handle.state::<DeepSeekHarnessProcessState>();
            if let Err(error) = stop_managed_deepseek_harness(deepseek_process_state.inner()) {
                eprintln!("Failed to stop DeepSeek Harness: {error}");
            }
            let gui_config_state = app_handle.state::<GuiConfigState>();
            let process_state = app_handle.state::<CoreProcessState>();
            if !process_state.shutdown_complete.load(Ordering::Acquire) {
                process_state.shutting_down.store(true, Ordering::Release);
                shutdown_managed_core(process_state.inner(), gui_config_state.inner());
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests;
