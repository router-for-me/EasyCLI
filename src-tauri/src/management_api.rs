#[cfg(target_os = "windows")]
use super::windows_explorer_executable;
use super::{
    auth_dir_path_for_core, configure_background_command, core_install_dir, core_logs_dir_path,
    core_origin, current_core_tls_settings, is_hashed_management_secret_key, open_oauth_url_inner,
    path_to_string, truncate_for_error, GuiConfigFile, GuiConfigState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::LazyLock,
    time::Duration,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthStartResult {
    url: String,
    state: Option<String>,
    user_code: Option<String>,
    flow: Option<String>,
    expires_in: Option<u64>,
    opened: bool,
    open_error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OAuthStatusResult {
    status: String,
    error: Option<String>,
}

#[derive(Deserialize)]
struct OAuthStartApiResponse {
    url: Option<String>,
    state: Option<String>,
    user_code: Option<String>,
    flow: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    #[serde(rename = "error_message")]
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct OAuthStatusApiResponse {
    status: Option<String>,
    error: Option<String>,
    #[serde(rename = "error_message")]
    error_message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagementRequest {
    method: String,
    path: String,
    query: Option<HashMap<String, String>>,
    #[serde(default, deserialize_with = "deserialize_management_body")]
    body: Option<serde_json::Value>,
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}

fn deserialize_management_body<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // An explicit v8 null is a JSON value, not an omitted request body.
    serde_json::Value::deserialize(deserializer).map(Some)
}

#[tauri::command]
pub(crate) async fn management_request(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    request: ManagementRequest,
) -> Result<serde_json::Value, String> {
    let config = gui_config_state.snapshot()?;
    let method = match request.method.trim().to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("Unsupported management API request method".to_string()),
    };
    let path = request.path.trim();
    if path.is_empty() || path.contains("://") || path.contains("..") {
        return Err("Invalid management API path".to_string());
    }

    let client = management_http_client()?;
    let mut builder = client
        .request(method, management_endpoint(&config, path)?)
        .header("Authorization", management_authorization(&config)?);
    if let Some(timeout_ms) = request.timeout_ms {
        builder = builder.timeout(Duration::from_millis(timeout_ms.clamp(1_000, 120_000)));
    }
    if let Some(query) = request.query {
        builder = builder.query(&query);
    }
    if let Some(body) = request.body {
        builder = management_request_body(builder, path, body)?;
    }

    let response = builder
        .send()
        .await
        .map_err(|err| format_management_request_error("Management API request failed", &err))?;
    read_management_value(response).await
}

fn management_request_body(
    builder: reqwest::RequestBuilder,
    path: &str,
    body: serde_json::Value,
) -> Result<reqwest::RequestBuilder, String> {
    if path.trim_start_matches('/') == "config.yaml" {
        let yaml = body.as_str().ok_or("YAML configuration must be text")?;
        Ok(builder
            .header("Content-Type", "application/yaml")
            .body(yaml.to_owned()))
    } else {
        Ok(builder.json(&body))
    }
}

#[tauri::command]
pub(crate) async fn import_vertex_credential(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    name: String,
    data: Vec<u8>,
    location: Option<String>,
) -> Result<serde_json::Value, String> {
    let name = name.trim();
    if name.is_empty() || !name.to_ascii_lowercase().ends_with(".json") {
        return Err("Credential filename must end with .json".to_string());
    }
    let config = gui_config_state.snapshot()?;
    let part = reqwest::multipart::Part::bytes(data)
        .file_name(name.to_owned())
        .mime_str("application/json")
        .map_err(|error| error.to_string())?;
    let mut form = reqwest::multipart::Form::new().part("file", part);
    if let Some(location) = location.filter(|value| !value.trim().is_empty()) {
        form = form.text("location", location.trim().to_owned());
    }
    let response = management_http_client()?
        .post(management_endpoint(&config, "oauth/import")?)
        .header("Authorization", management_authorization(&config)?)
        .query(&[("provider", "vertex")])
        .multipart(form)
        .send()
        .await
        .map_err(|error| format_management_request_error("Failed to import Vertex credential", &error))?;
    read_management_value(response).await
}

#[tauri::command]
pub(crate) async fn upload_auth_file(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    name: String,
    data: Vec<u8>,
) -> Result<serde_json::Value, String> {
    let name = name.trim().to_string();
    if name.is_empty() || !name.to_ascii_lowercase().ends_with(".json") {
        return Err("Credential filename must end with .json".to_string());
    }

    let config = gui_config_state.snapshot()?;
    let client = management_http_client()?;
    let mut query = HashMap::new();
    query.insert("name".to_string(), name);
    let response = client
        .post(management_endpoint(&config, "credentials")?)
        .header("Authorization", management_authorization(&config)?)
        .query(&query)
        .header("Content-Type", "application/json")
        .body(data)
        .send()
        .await
        .map_err(|err| format_management_request_error("Failed to upload credential file", &err))?;
    read_management_value(response).await
}

#[tauri::command]
pub(crate) fn open_auth_files_directory(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<(), String> {
    let config = gui_config_state.snapshot()?;
    let install_dir = core_install_dir()?;
    let auth_dir = auth_dir_path_for_core(&config.auth_dir, &install_dir)?;
    fs::create_dir_all(&auth_dir)
        .map_err(|error| format!("Failed to create credentials directory {}: {error}", path_to_string(&auth_dir)))?;
    open_directory_in_file_manager(&auth_dir)
}

#[tauri::command]
pub(crate) fn open_core_logs_directory(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<(), String> {
    let config = gui_config_state.snapshot()?;
    let install_dir = core_install_dir()?;
    let logs_dir = core_logs_dir_path(&config.auth_dir, &install_dir)?;
    fs::create_dir_all(&logs_dir)
        .map_err(|error| format!("Failed to create log directory {}: {error}", path_to_string(&logs_dir)))?;

    open_directory_in_file_manager(&logs_dir)
}

fn open_directory_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = Command::new(windows_explorer_executable());
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    command.arg(path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to open directory {}: {error}", path_to_string(path)))
}

#[tauri::command]
pub(crate) async fn start_oauth_login(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    provider: String,
    browser: Option<String>,
    plugin_provider: Option<bool>,
) -> Result<OAuthStartResult, String> {
    super::network_proxy::prepare_oauth(&app).await;
    let config = gui_config_state.snapshot()?;
    let provider_key = normalize_oauth_request_provider(&provider, plugin_provider.unwrap_or(false))?;
    let client = management_http_client()?;
    let mut request = client
        .get(management_endpoint(&config, "oauth/auth-url")?)
        .header("Authorization", management_authorization(&config)?);
    request = request.query(&[("provider", provider_key.as_str())]);
    if management_oauth_uses_webui_callback(&provider_key) {
        request = request.query(&[("is_webui", "true")]);
    }
    let response = request
        .send()
        .await
        .map_err(|err| format_management_request_error("Failed to request OAuth login URL", &err))?;
    let payload = read_management_json::<OAuthStartApiResponse>(response).await?;
    let mut result = normalize_oauth_start_response(payload)?;
    match open_oauth_url_inner(&app, &result.url, browser.as_deref()) {
        Ok(()) => result.opened = true,
        Err(error) => result.open_error = Some(error),
    }
    Ok(result)
}

fn normalize_oauth_start_response(payload: OAuthStartApiResponse) -> Result<OAuthStartResult, String> {
    if let Some(error) = payload
        .error
        .or(payload.error_message)
        .filter(|value| !value.trim().is_empty())
    {
        return Err(error);
    }
    let url = payload
        .url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Kernel did not return an OAuth login URL".to_string())?;
    let state = payload
        .state
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(OAuthStartResult {
        url,
        state,
        user_code: payload
            .user_code
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        flow: payload
            .flow
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty()),
        expires_in: payload.expires_in.filter(|value| *value > 0),
        opened: false,
        open_error: None,
    })
}

#[tauri::command]
pub(crate) async fn get_oauth_status(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    state: String,
) -> Result<OAuthStatusResult, String> {
    let state = state.trim().to_string();
    if state.is_empty() {
        return Err("OAuth state cannot be empty".to_string());
    }
    let config = gui_config_state.snapshot()?;
    let client = management_http_client()?;
    let response = client
        .get(management_endpoint(&config, "oauth/status")?)
        .header("Authorization", management_authorization(&config)?)
        .query(&[("state", state)])
        .send()
        .await
        .map_err(|err| format_management_request_error("Failed to query OAuth status", &err))?;
    let payload = read_management_json::<OAuthStatusApiResponse>(response).await?;
    let status = payload
        .status
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "wait".to_string());
    Ok(OAuthStatusResult {
        status,
        error: payload
            .error
            .or(payload.error_message)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

#[tauri::command]
pub(crate) async fn submit_oauth_callback(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    provider: String,
    redirect_url: String,
    plugin_provider: Option<bool>,
) -> Result<(), String> {
    let redirect_url = redirect_url.trim().to_string();
    if redirect_url.is_empty() {
        return Err("Callback URL cannot be empty".to_string());
    }
    let config = gui_config_state.snapshot()?;
    let provider_key = normalize_oauth_request_provider(&provider, plugin_provider.unwrap_or(false))?;
    let client = management_http_client()?;
    let body = serde_json::json!({
        "provider": provider_key,
        "redirect_url": redirect_url,
    });
    let response = client
        .post(management_endpoint(&config, "oauth/callback")?)
        .header("Authorization", management_authorization(&config)?)
        .json(&body)
        .send()
        .await
        .map_err(|err| format_management_request_error("Failed to submit OAuth callback", &err))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format_management_request_error("Failed to read OAuth callback response", &err))?;
    if !status.is_success() {
        return Err(format_management_error(status.as_u16(), &text));
    }
    Ok(())
}

pub(crate) fn management_http_client() -> Result<reqwest::Client, String> {
    static CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
        reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|err| format_management_request_error("Failed to create management API client", &err))
    });
    CLIENT.as_ref().cloned().map_err(Clone::clone)
}

pub(crate) fn format_management_request_error(
    action: &str,
    error: &(dyn Error + 'static),
) -> String {
    let mut messages = Vec::new();
    let mut current = Some(error);

    while let Some(error) = current {
        let message = error.to_string();
        if !message.is_empty()
            && messages
                .last()
                .map(|previous| previous != &message)
                .unwrap_or(true)
        {
            messages.push(message);
        }
        current = error.source();
    }

    if messages.is_empty() {
        action.to_string()
    } else {
        format!("{action}: {}", messages.join(": "))
    }
}

pub(crate) fn management_plaintext_key(config: &GuiConfigFile) -> Result<&str, String> {
    let secret_key = config.management_secret_key.trim();
    if secret_key.is_empty() || is_hashed_management_secret_key(secret_key) {
        return Err("Management interface unavailable: no plaintext management key is available".to_string());
    }
    Ok(secret_key)
}

pub(crate) fn management_authorization(config: &GuiConfigFile) -> Result<String, String> {
    Ok(format!("Bearer {}", management_plaintext_key(config)?))
}

pub(crate) fn management_endpoint(config: &GuiConfigFile, path: &str) -> Result<String, String> {
    if config.port == 0 {
        return Err("Invalid kernel port".to_string());
    }
    let path = path.trim_start_matches('/');
    let origin = core_origin(
        &config.host,
        config.port,
        current_core_tls_settings()?.enabled,
    );
    Ok(format!("{origin}/v8/management/{path}"))
}

fn normalize_oauth_request_provider(provider: &str, plugin_provider: bool) -> Result<String, String> {
    if !plugin_provider {
        return normalize_management_oauth_provider(provider);
    }
    // Plugin provider IDs are literal backend identifiers. A plugin named grok
    // or cognition must not be redirected to the built-in xai or devin flow.
    let key = provider.trim().to_ascii_lowercase();
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Invalid OAuth provider".to_string());
    }
    Ok(key)
}

fn normalize_management_oauth_provider(provider: &str) -> Result<String, String> {
    let key = provider.trim().to_ascii_lowercase().replace('_', "-");
    let key = match key.as_str() {
        "claude" | "anthropic" => "claude".to_string(),
        "anti-gravity" => "antigravity".to_string(),
        "cognition" => "devin".to_string(),
        "muse" => "meta".to_string(),
        "grok" | "x-ai" | "x.ai" => "xai".to_string(),
        other => other.to_string(),
    };
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Invalid OAuth provider".to_string());
    }
    Ok(key)
}

fn management_oauth_uses_webui_callback(provider_key: &str) -> bool {
    matches!(provider_key, "codex" | "claude" | "antigravity" | "xai" | "devin")
}

async fn read_management_json<T>(response: reqwest::Response) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format_management_request_error("Failed to read management API response", &err))?;
    if !status.is_success() {
        return Err(format_management_error(status.as_u16(), &text));
    }
    if text.trim().is_empty() {
        return Err("Management API returned an empty response".to_string());
    }
    serde_json::from_str::<T>(&text).map_err(|err| {
        format!(
            "Failed to parse management API response: {err}; body={}",
            truncate_for_error(&text)
        )
    })
}

pub(crate) async fn read_management_value(
    response: reqwest::Response,
) -> Result<serde_json::Value, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format_management_request_error("Failed to read management API response", &err))?;
    if !status.is_success() {
        return Err(format_management_error(status.as_u16(), &text));
    }
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => Ok(value),
        Err(_) => Ok(serde_json::Value::String(text)),
    }
}

pub(crate) async fn read_management_text(response: reqwest::Response) -> Result<String, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format_management_request_error("Failed to read management API response", &err))?;
    if !status.is_success() {
        return Err(format_management_error(status.as_u16(), &text));
    }
    Ok(text)
}

fn format_management_error(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let field = |key| {
            value.get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        };
        let error = field("error");
        let message = field("message");
        let detail = match (error, message) {
            (Some(error), Some(message)) if error != message => Some(format!("{error}: {message}")),
            (Some(text), _) | (_, Some(text)) => Some(text.to_string()),
            _ => None,
        };
        if let Some(detail) = detail {
            return format!(
                "Management API error ({status}): {}",
                truncate_for_error(&detail)
            );
        }
    }
    let body = body.trim();
    if body.is_empty() {
        format!("Management API error ({status})")
    } else {
        format!("Management API error ({status}): {}", truncate_for_error(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn v8_yaml_body_is_raw_text_and_config_values_remain_json() {
        let client = reqwest::Client::new();
        let yaml = "config-version: 8\naccess: {api-keys: [test-client]}\n";
        let request = management_request_body(
            client.put("http://127.0.0.1/v8/management/config.yaml"),
            "/config.yaml",
            serde_json::json!(yaml),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.headers()["Content-Type"], "application/yaml");
        assert_eq!(request.body().unwrap().as_bytes().unwrap(), yaml.as_bytes());

        for value in [
            serde_json::Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!("direct"),
        ] {
            let request = management_request_body(
                client.put("http://127.0.0.1/v8/management/config/requests/proxy-url"),
                "/config/requests/proxy-url",
                value.clone(),
            )
            .unwrap()
            .build()
            .unwrap();
            assert_eq!(request.headers()["Content-Type"], "application/json");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(request.body().unwrap().as_bytes().unwrap()).unwrap(),
                value,
            );
        }
        assert!(management_request_body(
            client.put("http://127.0.0.1/v8/management/config.yaml"),
            "/config.yaml",
            serde_json::json!({}),
        )
        .is_err());
    }

    #[test]
    fn v8_requests_distinguish_explicit_null_from_an_absent_body() {
        let absent: ManagementRequest = serde_json::from_value(serde_json::json!({
            "method": "GET", "path": "/config",
        }))
        .unwrap();
        let explicit: ManagementRequest = serde_json::from_value(serde_json::json!({
            "method": "PUT", "path": "/config/requests/payload", "body": null,
        }))
        .unwrap();
        assert!(absent.body.is_none());
        assert_eq!(explicit.body, Some(serde_json::Value::Null));
    }

    #[test]
    fn plugin_oauth_preserves_literal_provider_ids_without_builtin_aliases() {
        for provider in [
            "grok",
            "cognition",
            "anti-gravity",
            "anthropic",
            "custom-sso",
        ] {
            assert_eq!(
                normalize_oauth_request_provider(provider, true).unwrap(),
                provider
            );
        }
        assert_eq!(
            normalize_oauth_request_provider(" Custom-SSO ", true).unwrap(),
            "custom-sso"
        );
        for provider in ["", "custom_sso", "custom/sso", "custom.sso", "custom sso"] {
            assert!(normalize_oauth_request_provider(provider, true).is_err());
        }
    }

    #[test]
    fn builtin_oauth_retains_aliases_when_plugin_provider_is_not_requested() {
        for (provider, expected) in [
            ("grok", "xai"),
            ("cognition", "devin"),
            ("anti-gravity", "antigravity"),
            ("anthropic", "claude"),
            ("custom_sso", "custom-sso"),
        ] {
            assert_eq!(
                normalize_oauth_request_provider(provider, false).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn management_errors_include_v8_validation_details() {
        assert_eq!(
            format_management_error(400, r#"{"error":"invalid_config","message":"field name not found in type config.CodexKey"}"#),
            "Management API error (400): invalid_config: field name not found in type config.CodexKey"
        );
        for body in [
            r#"{"error":"not_found"}"#,
            r#"{"error":"not_found","message":"not_found"}"#,
        ] {
            assert_eq!(
                format_management_error(404, body),
                "Management API error (404): not_found"
            );
        }
        assert_eq!(
            format_management_error(400, r#"{"error":" ","message":"validation failed"}"#),
            "Management API error (400): validation failed"
        );
        assert_eq!(
            format_management_error(502, "gateway unavailable"),
            "Management API error (502): gateway unavailable"
        );
        assert_eq!(format_management_error(500, ""), "Management API error (500)");
    }

    #[test]
    fn devin_oauth_uses_the_management_callback_flow() {
        for provider in ["Devin", " cognition "] {
            let key = normalize_management_oauth_provider(provider).unwrap();
            assert_eq!(key, "devin");
            assert!(management_oauth_uses_webui_callback(&key));
        }
    }

    #[test]
    fn meta_oauth_uses_the_device_flow() {
        for provider in ["Meta", " muse "] {
            let key = normalize_management_oauth_provider(provider).unwrap();
            assert_eq!(key, "meta");
            assert!(!management_oauth_uses_webui_callback(&key));
        }
    }

    #[test]
    fn oauth_start_preserves_device_authorization_fields_for_the_frontend() {
        let payload = serde_json::from_value::<OAuthStartApiResponse>(serde_json::json!({
            "status": "ok",
            "url": " https://auth.meta.com/device ",
            "state": " meta-device-state ",
            "user_code": " ABCD-EFGH ",
            "flow": "device",
            "expires_in": 900,
        }))
        .unwrap();
        let result = normalize_oauth_start_response(payload).unwrap();
        let value = serde_json::to_value(result).unwrap();

        assert_eq!(value["url"], "https://auth.meta.com/device");
        assert_eq!(value["state"], "meta-device-state");
        assert_eq!(value["userCode"], "ABCD-EFGH");
        assert_eq!(value["flow"], "device");
        assert_eq!(value["expiresIn"], 900);
        assert_eq!(value["opened"], false);
        assert!(value["openError"].is_null());
    }

    #[test]
    fn oauth_start_keeps_device_metadata_optional_for_existing_providers() {
        for extra in [serde_json::json!({}), serde_json::json!({
            "user_code": " ", "flow": " ", "expires_in": 0,
        })] {
            let mut payload = serde_json::json!({
                "url": "https://example.com/oauth",
                "state": "oauth-state",
            });
            payload
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let result = normalize_oauth_start_response(
                serde_json::from_value::<OAuthStartApiResponse>(payload).unwrap(),
            )
            .unwrap();

            assert!(result.user_code.is_none());
            assert!(result.flow.is_none());
            assert!(result.expires_in.is_none());
        }
    }

    #[test]
    fn core_logs_follow_the_default_auth_directory() {
        let base_dir = PathBuf::from("test-base");
        let install_dir = base_dir.join("cpa-core");

        assert_eq!(
            core_logs_dir_path("../oauth", &install_dir).unwrap(),
            base_dir.join("oauth").join("logs")
        );
    }

    #[test]
    fn core_logs_follow_a_custom_auth_directory() {
        let install_dir = PathBuf::from("test-base").join("cpa-core");
        let auth_dir = PathBuf::from("custom-auth");

        assert_eq!(
            core_logs_dir_path(auth_dir.to_str().unwrap(), &install_dir).unwrap(),
            install_dir.join(auth_dir).join("logs")
        );
    }
}
