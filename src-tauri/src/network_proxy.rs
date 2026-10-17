use super::*;

pub(crate) fn normalize_proxy_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    let invalid = || {
        "Invalid proxy address. Use http://host:port, https://host:port, or socks5://host:port (not a PAC address)".to_string()
    };
    if value.is_empty() || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid());
    }
    let value = if let Some(rest) = value.strip_prefix("socks://") {
        format!("socks5://{rest}")
    } else {
        value.to_string()
    };
    let url = reqwest::Url::parse(&value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none()
        || url.port() == Some(0)
        || (url.port_or_known_default().is_none() && url.port().is_none())
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn fixed_proxy(value: &str, scheme: &str) -> Result<String, String> {
    parse_detected_proxy(&if value.contains("://") {
        value.to_string()
    } else {
        format!("{scheme}://{value}")
    })
}

fn parse_detected_proxy(value: &str) -> Result<String, String> {
    let value = value.trim().trim_matches('\'').trim_matches('"');
    if value.is_empty() {
        return Err("System proxy address is empty".to_string());
    }
    if value.contains("://") {
        normalize_proxy_url(value)
    } else {
        normalize_proxy_url(&format!("http://{value}"))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn parse_env_proxy(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim().trim_matches('\'').trim_matches('"');
    if value.is_empty() {
        return Err("System proxy address is empty".to_string());
    }
    if !value.contains("://") && name.eq_ignore_ascii_case("socks_proxy") {
        return normalize_proxy_url(&format!("socks5://{value}"));
    }
    parse_detected_proxy(value)
}

#[cfg(any(target_os = "linux", test))]
fn desktop_scalar(value: &str) -> String {
    let value = value.trim().trim_matches('\'').trim_matches('"');
    value
        .strip_prefix("uint32 ")
        .unwrap_or(value)
        .trim()
        .to_string()
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_proxy(value: &str) -> Result<String, String> {
    if !value.contains('=') {
        return fixed_proxy(value.trim(), "http");
    }
    for protocol in ["https", "http", "socks"] {
        if let Some((_, address)) = value
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find(|(key, _)| key.trim().eq_ignore_ascii_case(protocol))
        {
            return fixed_proxy(
                address.trim(),
                if protocol == "socks" {
                    "socks5"
                } else {
                    "http"
                },
            );
        }
    }
    Err("System proxy has no available HTTP / HTTPS / SOCKS address".to_string())
}

#[cfg(any(target_os = "windows", test))]
fn pac_error() -> String {
    "PAC / auto-discovery proxy detected; conversion to a fixed kernel proxy is not supported yet".to_string()
}

#[cfg(target_os = "windows")]
fn detect_system_proxy() -> Result<String, String> {
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        Networking::WinHttp::{
            WinHttpGetIEProxyConfigForCurrentUser, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG,
        },
    };
    unsafe fn read_wide(ptr: *mut u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
    unsafe {
        let mut config: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = std::mem::zeroed();
        if WinHttpGetIEProxyConfigForCurrentUser(&mut config) == 0 {
            return Err("Unable to read Windows system proxy for the current user".to_string());
        }
        let proxy = read_wide(config.lpszProxy);
        let pac = read_wide(config.lpszAutoConfigUrl);
        let auto_detect = config.fAutoDetect != 0;
        for ptr in [
            config.lpszProxy,
            config.lpszProxyBypass,
            config.lpszAutoConfigUrl,
        ] {
            if !ptr.is_null() {
                GlobalFree(ptr.cast());
            }
        }
        select_windows_proxy(&proxy, &pac, auto_detect)
    }
}

#[cfg(any(target_os = "windows", test))]
fn select_windows_proxy(proxy: &str, pac: &str, auto_detect: bool) -> Result<String, String> {
    if !proxy.trim().is_empty() {
        return parse_windows_proxy(proxy);
    }
    if !pac.trim().is_empty() || auto_detect {
        return Err(pac_error());
    }
    Ok(String::new())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn env_proxy() -> Option<String> {
    for name in [
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "http_proxy",
        "HTTP_PROXY",
        "socks_proxy",
        "SOCKS_PROXY",
    ] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                if let Ok(url) = parse_env_proxy(name, &value) {
                    return Some(url);
                }
            }
        }
    }
    None
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn desktop_setting(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| format!("Unable to run {program} to read system proxy"))?;
    let deadline = std::time::Instant::now() + Duration::from_millis(400);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("{program} failed to read system proxy"));
                }
                let output = child
                    .wait_with_output()
                    .map_err(|_| "Failed to read system proxy".to_string())?;
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Timed out while reading system proxy".to_string());
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_system_proxy() -> Result<String, String> {
    if let Ok(output) = desktop_setting("/usr/sbin/scutil", &["--proxy"]) {
        if let Ok(url) = parse_macos_proxy(&output) {
            if !url.is_empty() {
                return Ok(url);
            }
        }
    }
    if let Ok(url) = macos_networksetup_proxy() {
        if !url.is_empty() {
            return Ok(url);
        }
    }
    Ok(env_proxy().unwrap_or_default())
}

#[cfg(target_os = "macos")]
fn macos_networksetup_proxy() -> Result<String, String> {
    let list = desktop_setting("/usr/sbin/networksetup", &["-listallnetworkservices"])?;
    for service in list.lines().skip(1).filter_map(macos_network_service_name) {
        for (flag, scheme) in [
            ("-getsecurewebproxy", "http"),
            ("-getwebproxy", "http"),
            ("-getsocksfirewallproxy", "socks5"),
        ] {
            if let Ok(output) = desktop_setting("/usr/sbin/networksetup", &[flag, service]) {
                if let Ok(url) = parse_networksetup_proxy(&output, scheme) {
                    if !url.is_empty() {
                        return Ok(url);
                    }
                }
            }
        }
    }
    Ok(String::new())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_proxy(output: &str) -> Result<String, String> {
    let fields: std::collections::HashMap<_, _> = output
        .lines()
        .filter_map(|line| {
            line.split_once(':')
                .map(|(key, value)| (key.trim(), value.trim()))
        })
        .collect();
    for (prefix, scheme) in [("HTTPS", "http"), ("HTTP", "http"), ("SOCKS", "socks5")] {
        if fields.get(format!("{prefix}Enable").as_str()) != Some(&"1") {
            continue;
        }
        let Some(host) = fields.get(format!("{prefix}Proxy").as_str()).copied() else {
            continue;
        };
        let Some(port) = fields.get(format!("{prefix}Port").as_str()).copied() else {
            continue;
        };
        if host.is_empty() || port.is_empty() || port == "0" {
            continue;
        }
        if let Ok(url) = fixed_proxy(&format!("{}:{port}", bracket_host(host)), scheme) {
            return Ok(url);
        }
    }
    if fields.get("ProxyAutoConfigEnable") == Some(&"1")
        || fields.get("ProxyAutoDiscoveryEnable") == Some(&"1")
    {
        return Ok(String::new());
    }
    Ok(String::new())
}

#[cfg(any(target_os = "macos", test))]
fn macos_network_service_name(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('*') {
        None
    } else {
        Some(line)
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_networksetup_proxy(output: &str, scheme: &str) -> Result<String, String> {
    let mut enabled = false;
    let mut host = String::new();
    let mut port = String::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.eq_ignore_ascii_case("Enabled") {
            enabled = value.eq_ignore_ascii_case("Yes") || value == "Yes";
        } else if key.eq_ignore_ascii_case("Server") || key == "Server" {
            host = value.to_string();
        } else if key.eq_ignore_ascii_case("Port") || key == "Port" {
            port = value.to_string();
        }
    }
    if !enabled || host.is_empty() || port.is_empty() || port == "0" {
        return Ok(String::new());
    }
    fixed_proxy(&format!("{}:{port}", bracket_host(&host)), scheme)
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn bracket_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

#[cfg(any(target_os = "linux", test))]
fn gnome_manual_proxy(
    mode: &str,
    https_host: &str,
    https_port: &str,
    http_host: &str,
    http_port: &str,
    socks_host: &str,
    socks_port: &str,
    http_auth: &str,
) -> Result<String, String> {
    match desktop_scalar(mode).as_str() {
        "none" | "auto" => Ok(String::new()),
        "manual" => {
            let http_auth = desktop_scalar(http_auth).eq_ignore_ascii_case("true");
            for (host, port, scheme, auth) in [
                (https_host, https_port, "http", false),
                (http_host, http_port, "http", http_auth),
                (socks_host, socks_port, "socks5", false),
            ] {
                let host = desktop_scalar(host);
                let port = desktop_scalar(port);
                if host.is_empty() || port.is_empty() || port == "0" || auth {
                    continue;
                }
                if let Ok(url) = fixed_proxy(&format!("{}:{port}", bracket_host(&host)), scheme) {
                    return Ok(url);
                }
            }
            Ok(String::new())
        }
        _ => Ok(String::new()),
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_kde_proxy(proxy_type: &str, https: &str, http: &str, socks: &str) -> Result<Option<String>, String> {
    match desktop_scalar(proxy_type).as_str() {
        "0" | "" | "2" | "3" => Ok(Some(String::new())),
        "4" => Ok(None),
        _ => {
            for value in [https, http, socks] {
                let value = desktop_scalar(value);
                if value.is_empty() {
                    continue;
                }
                if let Ok(url) = parse_detected_proxy(&value) {
                    return Ok(Some(url));
                }
            }
            Ok(Some(String::new()))
        }
    }
}

#[cfg(target_os = "linux")]
fn gnome_system_proxy() -> Result<Option<String>, String> {
    let get = |schema: &str, key: &str| desktop_setting("gsettings", &["get", schema, key]);
    let Ok(mode) = get("org.gnome.system.proxy", "mode") else {
        return Ok(None);
    };
    let https_host = get("org.gnome.system.proxy.https", "host").unwrap_or_default();
    let https_port = get("org.gnome.system.proxy.https", "port").unwrap_or_default();
    let http_host = get("org.gnome.system.proxy.http", "host").unwrap_or_default();
    let http_port = get("org.gnome.system.proxy.http", "port").unwrap_or_default();
    let socks_host = get("org.gnome.system.proxy.socks", "host").unwrap_or_default();
    let socks_port = get("org.gnome.system.proxy.socks", "port").unwrap_or_default();
    let http_auth = get("org.gnome.system.proxy.http", "use-authentication").unwrap_or_default();
    gnome_manual_proxy(
        &mode,
        &https_host,
        &https_port,
        &http_host,
        &http_port,
        &socks_host,
        &socks_port,
        &http_auth,
    )
    .map(Some)
}

#[cfg(target_os = "linux")]
fn kde_config(key: &str) -> Result<String, String> {
    for program in ["kreadconfig6", "kreadconfig5"] {
        if let Ok(value) = desktop_setting(
            program,
            &[
                "--file",
                "kioslaverc",
                "--group",
                "Proxy Settings",
                "--key",
                key,
            ],
        ) {
            return Ok(value);
        }
    }
    Err("Unable to read KDE system proxy".to_string())
}

#[cfg(target_os = "linux")]
fn kde_system_proxy() -> Result<Option<String>, String> {
    parse_kde_proxy(
        &kde_config("ProxyType")?,
        &kde_config("httpsProxy").unwrap_or_default(),
        &kde_config("httpProxy").unwrap_or_default(),
        &kde_config("socksProxy").unwrap_or_default(),
    )
}

#[cfg(target_os = "linux")]
fn linux_desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(target_os = "linux")]
fn detect_system_proxy() -> Result<String, String> {
    if let Some(url) = env_proxy() {
        return Ok(url);
    }
    let desktop = linux_desktop();
    let gnome_desktop = ["gnome", "unity", "cinnamon", "budgie", "pantheon"]
        .iter()
        .any(|name| desktop.contains(name));
    let kde_desktop = desktop.contains("kde") || desktop.contains("plasma");
    if gnome_desktop {
        match gnome_system_proxy() {
            Ok(Some(url)) => return Ok(url),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }
    if kde_desktop || !gnome_desktop {
        match kde_system_proxy() {
            Ok(Some(url)) => return Ok(url),
            Ok(None) | Err(_) => {}
        }
    }
    if !gnome_desktop {
        match gnome_system_proxy() {
            Ok(Some(url)) => return Ok(url),
            Ok(None) | Err(_) => {}
        }
    }
    Ok(String::new())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn detect_system_proxy() -> Result<String, String> {
    Ok(String::new())
}

pub(crate) fn detect() -> String {
    detect_system_proxy().unwrap_or_default()
}

pub(crate) fn resolve(config: &GuiConfigFile) -> String {
    if config.proxy_override {
        config.proxy_url.clone()
    } else {
        detect()
    }
}

pub(crate) fn initialize_override(config: &mut GuiConfigFile, had_override: bool) -> bool {
    if had_override {
        return false;
    }
    // Before proxy-override existed, every non-empty proxy-url was explicitly
    // entered by the user. Preserve that choice when migrating the config.
    config.proxy_override = !config.proxy_url.trim().is_empty();
    true
}

pub(crate) fn normalize_optional_proxy_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    normalize_proxy_url(value)
}

fn apply(state: &GuiConfigState) -> Result<GuiConfigFile, String> {
    let snapshot = state
        .inner
        .lock()
        .map_err(|_| "Proxy configuration lock is poisoned".to_string())?
        .clone();
    let detected = if snapshot.proxy_override {
        None
    } else {
        Some(detect())
    };
    let mut current = state
        .inner
        .lock()
        .map_err(|_| "Proxy configuration lock is poisoned".to_string())?;
    let mut next = current.clone();
    next.proxy_url = if next.proxy_override {
        next.proxy_url.clone()
    } else {
        detected.unwrap_or_else(detect)
    };
    commit(&mut current, next, patch_core_proxy_url, write_gui_config)?;
    Ok(current.clone())
}

fn commit(
    current: &mut GuiConfigFile,
    next: GuiConfigFile,
    mut patch: impl FnMut(&str) -> Result<(), String>,
    persist: impl FnOnce(&GuiConfigFile) -> Result<(), String>,
) -> Result<(), String> {
    let changed =
        next.proxy_url != current.proxy_url || next.proxy_override != current.proxy_override;
    patch(&next.proxy_url)?;
    if changed {
        if let Err(error) = persist(&next) {
            return Err(config_update_error_with_rollback(
                error,
                patch(&current.proxy_url).err(),
            ));
        }
        *current = next;
    }
    Ok(())
}

pub(crate) fn refresh(state: &GuiConfigState) -> Result<(), String> {
    apply(state).map(|_| ())
}

pub(crate) fn set_manual(state: &GuiConfigState, proxy_url: String) -> Result<(), String> {
    let proxy_url = normalize_optional_proxy_url(&proxy_url)?;
    let detected = if proxy_url.is_empty() {
        Some(detect())
    } else {
        None
    };
    let mut current = state
        .inner
        .lock()
        .map_err(|_| "Proxy configuration lock is poisoned".to_string())?;
    let mut next = current.clone();
    next.proxy_override = !proxy_url.is_empty();
    next.proxy_url = if next.proxy_override {
        proxy_url
    } else {
        detected.unwrap_or_else(detect)
    };
    commit(&mut current, next, patch_core_proxy_url, write_gui_config)
}

pub(crate) async fn verify_core_proxy(config: &GuiConfigFile) -> Result<(), String> {
    let client = management_api::management_http_client()?;
    let mut updated = false;
    for attempt in 0..10 {
        let response = client
            .get(management_api::management_endpoint(config, "config/requests/proxy-url")?)
            .header(
                "Authorization",
                management_api::management_authorization(config)?,
            )
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        if let Ok(response) = response {
            if response.status().is_success() {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    if let Some(actual) = value.as_str() {
                        if actual == config.proxy_url {
                            return Ok(());
                        }
                        if attempt >= 2 && !updated {
                            let response = client
                                .put(management_api::management_endpoint(config, "config/requests/proxy-url")?)
                                .header(
                                    "Authorization",
                                    management_api::management_authorization(config)?,
                                )
                                .json(&config.proxy_url)
                                .timeout(Duration::from_secs(3))
                                .send()
                                .await
                                .map_err(|_| "Unable to update kernel proxy. Restart the kernel and try again".to_string())?;
                            if !response.status().is_success() {
                                return Err(
                                    "Kernel rejected the proxy update. Restart or upgrade the kernel and try again".to_string()
                                );
                            }
                            updated = true;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(
        "Proxy configuration was saved, but the kernel has not confirmed applying it. Make sure the kernel is running and restart it if necessary."
            .to_string(),
    )
}

static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn synchronize(app: &tauri::AppHandle, require_core: bool) -> Result<(), String> {
    let _guard = SYNC_LOCK.lock().await;
    // Keep the existing process check active even if proxy synchronization fails.
    let core_running = require_core || app.state::<CoreProcessState>().managed_pid().is_some();
    let work_app = app.clone();
    let config = tauri::async_runtime::spawn_blocking(move || {
        apply(work_app.state::<GuiConfigState>().inner())
    })
    .await
    .map_err(|e| e.to_string())??;
    if core_running {
        verify_core_proxy(&config).await?;
    }
    Ok(())
}

pub(crate) async fn prepare_oauth(app: &tauri::AppHandle) {
    let _ = synchronize(app, true).await;
}

pub(crate) fn start_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            if app
                .state::<CoreProcessState>()
                .shutting_down
                .load(Ordering::Acquire)
            {
                return;
            }
            let _ = synchronize(&app, false).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_gui_write_rolls_back_core() {
        let mut current = GuiConfigFile {
            proxy_url: "http://localhost:1000".into(),
            ..GuiConfigFile::default()
        };
        let mut next = current.clone();
        next.proxy_url = "http://localhost:2000".into();
        let mut writes = Vec::new();
        let result = commit(
            &mut current,
            next,
            |url| {
                writes.push(url.to_string());
                Ok(())
            },
            |_| Err("disk full".into()),
        );
        assert!(result.is_err());
        assert_eq!(writes, ["http://localhost:2000", "http://localhost:1000"]);
        assert_eq!(current.proxy_url, "http://localhost:1000");
    }

    #[test]
    fn failed_core_write_never_persists() {
        let mut current = GuiConfigFile::default();
        let mut next = current.clone();
        next.proxy_url = "http://localhost:7890".into();
        assert!(commit(
            &mut current,
            next,
            |_| Err("read only".into()),
            |_| panic!("must not persist")
        )
        .is_err());
        assert!(current.proxy_url.is_empty());
    }

    #[test]
    fn unchanged_refresh_does_not_rewrite_gui_config() {
        let mut current = GuiConfigFile::default();
        let next = current.clone();
        let mut reconciled = false;
        commit(
            &mut current,
            next,
            |_| {
                reconciled = true;
                Ok(())
            },
            |_| panic!("no changes"),
        )
        .unwrap();
        assert!(reconciled);
    }

    #[tokio::test]
    async fn configured_proxy_routes_upstream_but_bypasses_local_management() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let proxy_request = tokio::spawn(async move {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nvia")
                .await
                .unwrap();
            String::from_utf8_lossy(&bytes[..count]).to_string()
        });
        let client = build_http_client_with_proxy(
            reqwest::Client::builder().timeout(Duration::from_secs(2)),
            &proxy_url,
            "test",
        )
        .unwrap();
        assert_eq!(
            client
                .get("http://proxy-target.invalid/test")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "via"
        );
        assert!(proxy_request
            .await
            .unwrap()
            .starts_with("GET http://proxy-target.invalid/test HTTP/1.1"));
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_url = format!("http://{}", local.local_addr().unwrap());
        let response = tokio::spawn(async move {
            let (mut socket, _) = local.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nlocal",
                )
                .await
                .unwrap();
        });
        assert_eq!(
            client
                .get(local_url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "local"
        );
        response.await.unwrap();
        assert!(client
            .get("http://proxy-target.invalid/test")
            .send()
            .await
            .is_err());
    }

    #[test]
    fn windows_fixed_and_protocol_specific_proxies() {
        assert_eq!(
            parse_windows_proxy("127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            parse_windows_proxy("http=localhost:1;https=localhost:2;socks=localhost:3").unwrap(),
            "http://localhost:2"
        );
        assert_eq!(
            parse_windows_proxy("socks=[::1]:1080").unwrap(),
            "socks5://[::1]:1080"
        );
        assert!(parse_windows_proxy("ftp=localhost:21").is_err());
        assert_eq!(
            select_windows_proxy("127.0.0.1:7890", "http://wpad/proxy.pac", true).unwrap(),
            "http://127.0.0.1:7890"
        );
        assert!(select_windows_proxy("", "http://wpad/proxy.pac", false).is_err());
        assert_eq!(select_windows_proxy("", "", false).unwrap(), "");
    }

    #[test]
    fn rejects_pac_paths_invalid_ports_and_schemes() {
        for url in [
            "",
            "localhost:7890",
            "file:///proxy.pac",
            "http://localhost/proxy.pac",
            "http://localhost:0",
            "http://localhost:99999",
            "http://localhost:8?x=1",
            "socks5://localhost",
            "http://local host:8",
        ] {
            assert!(normalize_proxy_url(url).is_err(), "{url}");
        }
        assert_eq!(
            normalize_proxy_url(" http://localhost:7890/ ").unwrap(),
            "http://localhost:7890"
        );
        for url in [
            "http://localhost:7890",
            "https://proxy.example",
            "socks5://[::1]:1080",
            "socks5h://user:pass@localhost:1080",
            "socks://127.0.0.1:1080",
        ] {
            assert!(normalize_proxy_url(url).is_ok(), "{url}");
        }
        assert_eq!(
            normalize_proxy_url("socks://127.0.0.1:1080").unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert!(normalize_optional_proxy_url("not a proxy").is_err());
        assert!(normalize_optional_proxy_url("http://127.0.0.1:7890/path").is_err());
        assert_eq!(
            parse_detected_proxy("  '127.0.0.1:7890'  ").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert!(parse_detected_proxy("").is_err());
    }

    #[test]
    fn user_override_keeps_manual_proxy() {
        let mut config = GuiConfigFile {
            proxy_url: "socks5://127.0.0.1:1080".into(),
            proxy_override: true,
            ..GuiConfigFile::default()
        };
        assert_eq!(resolve(&config), "socks5://127.0.0.1:1080");
        config.proxy_override = false;
        config.proxy_url = "http://localhost:1".into();
        assert_eq!(resolve(&config), detect());
        assert!(!initialize_override(&mut config, true));
        let mut leftover = GuiConfigFile {
            proxy_url: "http://127.0.0.1:7890".into(),
            proxy_override: true,
            ..GuiConfigFile::default()
        };
        assert!(!initialize_override(&mut leftover, true));
        assert!(leftover.proxy_override);
        let mut legacy = GuiConfigFile {
            proxy_url: "http://127.0.0.1:7890".into(),
            ..GuiConfigFile::default()
        };
        assert!(initialize_override(&mut legacy, false));
        assert!(legacy.proxy_override);
        assert_eq!(resolve(&legacy), "http://127.0.0.1:7890");
        let mut auto = GuiConfigFile {
            proxy_url: "http://127.0.0.1:7890".into(),
            ..GuiConfigFile::default()
        };
        assert!(!initialize_override(&mut auto, true));
        assert!(!auto.proxy_override);
        assert_eq!(resolve(&auto), detect());
        assert_eq!(normalize_optional_proxy_url("").unwrap(), "");
        assert_eq!(
            normalize_optional_proxy_url(" socks5://127.0.0.1:7890/ ").unwrap(),
            "socks5://127.0.0.1:7890"
        );
    }

    #[test]
    fn macos_fixed_and_pac() {
        assert_eq!(
            parse_macos_proxy("  HTTPEnable : 1\n  HTTPProxy : 127.0.0.1\n  HTTPPort : 7890")
                .unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(parse_macos_proxy("HTTPEnable : 0").unwrap(), "");
        assert_eq!(
            parse_macos_proxy("ProxyAutoConfigEnable : 1\nHTTPEnable : 1\nHTTPProxy : 127.0.0.1\nHTTPPort : 7890")
                .unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(parse_macos_proxy("ProxyAutoConfigEnable : 1").unwrap(), "");
        assert_eq!(
            parse_macos_proxy("HTTPSEnable : 1\nHTTPSProxy : ::1\nHTTPSPort : 7890").unwrap(),
            "http://[::1]:7890"
        );
        assert_eq!(macos_network_service_name("*Wi-Fi"), None);
        assert_eq!(macos_network_service_name("Wi-Fi"), Some("Wi-Fi"));
    }

    #[test]
    fn macos_networksetup_and_unix_env_style_proxies() {
        assert_eq!(
            parse_networksetup_proxy(
                "Enabled: Yes\nServer: 127.0.0.1\nPort: 7890",
                "http",
            )
            .unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            parse_networksetup_proxy("Enabled: No\nServer: 127.0.0.1\nPort: 7890", "http")
                .unwrap(),
            ""
        );
        assert_eq!(
            parse_detected_proxy("127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            parse_detected_proxy("socks://127.0.0.1:1080").unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            parse_env_proxy("socks_proxy", "127.0.0.1:1080").unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            parse_env_proxy("https_proxy", "127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            gnome_manual_proxy(
                "'manual'",
                "'127.0.0.1'",
                "uint32 7890",
                "",
                "uint32 0",
                "",
                "uint32 0",
                "false",
            )
            .unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            gnome_manual_proxy("none", "", "", "", "", "", "", "false").unwrap(),
            ""
        );
        assert_eq!(
            gnome_manual_proxy(
                "manual",
                "",
                "0",
                "127.0.0.1",
                "7890",
                "127.0.0.1",
                "1080",
                "true",
            )
            .unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            gnome_manual_proxy("auto", "127.0.0.1", "7890", "", "", "", "", "false").unwrap(),
            ""
        );
        assert_eq!(
            parse_kde_proxy("1", "http://127.0.0.1:7890", "", "").unwrap(),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            parse_kde_proxy("0", "http://127.0.0.1:7890", "", "").unwrap(),
            Some(String::new())
        );
        assert_eq!(parse_kde_proxy("2", "", "", "").unwrap(), Some(String::new()));
        assert_eq!(parse_kde_proxy("4", "http://127.0.0.1:7890", "", "").unwrap(), None);
        assert_eq!(
            parse_kde_proxy("1", "http://localhost:0", "socks://127.0.0.1:1080", "").unwrap(),
            Some("socks5://127.0.0.1:1080".to_string())
        );
        let mut current = GuiConfigFile {
            proxy_url: "http://localhost:1000".into(),
            ..GuiConfigFile::default()
        };
        let mut next = current.clone();
        next.proxy_override = true;
        let mut persisted = false;
        commit(
            &mut current,
            next,
            |_| Ok(()),
            |_| {
                persisted = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(persisted);
        assert!(current.proxy_override);
    }
}
