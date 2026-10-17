use super::{
    core_origin, current_core_tls_settings, format_management_request_error,
    management_authorization, management_endpoint, management_http_client, read_management_value,
    GuiConfigFile, GuiConfigState,
};
use futures_util::StreamExt;

pub(crate) mod local_install;

#[derive(serde::Serialize)]
pub(crate) struct PluginLatestRelease {
    repository: String,
    version: Option<String>,
    error: Option<String>,
}

#[tauri::command]
pub(crate) async fn get_plugin_latest_releases(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    repositories: Vec<String>,
) -> Result<Vec<PluginLatestRelease>, String> {
    let config = gui_config_state.snapshot()?;
    let response = management_http_client()?
        .get(management_endpoint(&config, "config/server")?)
        .header("Authorization", management_authorization(&config)?)
        .send()
        .await
        .map_err(|_| "Failed to read GitHub settings".to_string())?;
    let settings = read_plugin_github_settings(response).await?;
    let token = plugin_github_token(&settings, std::env::var("GITHUB_TOKEN").ok());
    let client = super::build_http_client_with_proxy(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(8))
            .timeout(std::time::Duration::from_secs(20)),
        &config.proxy_url,
        "Failed to create GitHub client",
    )?;
    let client = &client;
    let token = token.as_deref();
    Ok(futures_util::stream::iter(repositories)
        .map(|repository| async move {
            let result = fetch_plugin_release(client, &repository, token).await;
            PluginLatestRelease {
                repository,
                version: result.as_ref().ok().cloned(),
                error: result.err(),
            }
        })
        .buffered(2)
        .collect()
        .await)
}

async fn read_plugin_github_settings(
    response: reqwest::Response,
) -> Result<serde_json::Value, String> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(serde_json::Value::Null);
    }
    read_management_value(response).await
}

async fn fetch_plugin_release(
    client: &reqwest::Client,
    repository: &str,
    token: Option<&str>,
) -> Result<String, String> {
    let response = plugin_release_request(client, repository, token)?
        .send()
        .await
        .map_err(|_| "Failed to fetch latest GitHub release".to_string())?;
    read_plugin_release(response).await
}

fn plugin_github_token(
    settings: &serde_json::Value,
    environment: Option<String>,
) -> Option<String> {
    settings
        .get("github-token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
        .or(environment)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn plugin_release_request(
    client: &reqwest::Client,
    repository: &str,
    token: Option<&str>,
) -> Result<reqwest::RequestBuilder, String> {
    let mut request = client
        .get(plugin_release_url(repository)?)
        .header("User-Agent", "EasyCLIProxyAPI")
        .header("Accept", "application/vnd.github+json")
        .header("Cache-Control", "no-cache");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request)
}

fn plugin_release_url(repository: &str) -> Result<String, String> {
    let parts: Vec<_> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err("Invalid GitHub repository".to_string());
    }
    Ok(format!(
        "https://api.github.com/repos/{repository}/releases/latest"
    ))
}

async fn read_plugin_release(response: reqwest::Response) -> Result<String, String> {
    let status = response.status();
    if status.as_u16() == 429
        || (status.as_u16() == 403
            && (response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|value| value == "0")
                || response.headers().contains_key("retry-after")))
    {
        return Err(format!("GitHub API rate limited ({status})"));
    }
    if !status.is_success() {
        return Err(format!("Failed to fetch latest GitHub release ({status})"));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid GitHub release response".to_string())?;
    value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(|tag| tag.strip_prefix(['v', 'V']).unwrap_or(tag))
        .filter(|tag| {
            tag.bytes().next().is_some_and(|byte| byte.is_ascii_digit())
                && tag.len() <= 128
                && tag
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        })
        .map(str::to_owned)
        .ok_or_else(|| "Invalid GitHub release tag".to_string())
}

#[tauri::command]
pub(crate) async fn get_plugin_support(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<bool, String> {
    let config = gui_config_state.snapshot()?;
    let response = request_plugins(&config).await?;
    read_plugin_support(response).await
}

pub(crate) async fn resolve_plugin_entry_url(
    config: &GuiConfigFile,
    plugin_id: &str,
    menu_index: usize,
) -> Result<String, String> {
    let response = request_plugins(config).await?;
    let plugins = read_management_value(response).await?;
    let path = active_plugin_menu_path(&plugins, plugin_id, menu_index)?;
    let origin = core_origin(
        &config.host,
        config.port,
        current_core_tls_settings()?.enabled,
    );
    resolve_plugin_resource_url(&origin, path)
}

async fn request_plugins(config: &GuiConfigFile) -> Result<reqwest::Response, String> {
    management_http_client()?
        .get(management_endpoint(config, "plugins")?)
        .header("Authorization", management_authorization(config)?)
        .send()
        .await
        .map_err(|error| format_management_request_error("Failed to query plugins", &error))
}

async fn read_plugin_support(response: reqwest::Response) -> Result<bool, String> {
    if let Some(supported) = response
        .headers()
        .get("X-CPA-SUPPORT-PLUGIN")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_plugin_support_header)
    {
        return Ok(supported);
    }
    if matches!(response.status().as_u16(), 404 | 501) {
        return Ok(false);
    }
    let value = read_management_value(response).await?;
    if value
        .get("plugins")
        .is_some_and(serde_json::Value::is_array)
    {
        return Ok(true);
    }
    Err("Kernel returned an invalid plugin capability response".to_string())
}

fn parse_plugin_support_header(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn active_plugin_menu_path<'a>(
    payload: &'a serde_json::Value,
    plugin_id: &str,
    menu_index: usize,
) -> Result<&'a str, String> {
    let plugin = payload
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .and_then(|plugins| {
            plugins.iter().find(|plugin| {
                plugin.get("id").and_then(serde_json::Value::as_str) == Some(plugin_id)
                    && plugin
                        .get("effective_enabled")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            })
        })
        .ok_or_else(|| "Plugin is unavailable or disabled".to_string())?;
    plugin
        .get("menus")
        .and_then(serde_json::Value::as_array)
        .and_then(|menus| menus.get(menu_index))
        .and_then(|menu| menu.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "Plugin page is unavailable".to_string())
}

fn resolve_plugin_resource_url(origin: &str, path: &str) -> Result<String, String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err("Invalid plugin resource path".to_string());
    }
    let base = reqwest::Url::parse(origin).map_err(|_| "Invalid kernel origin".to_string())?;
    let url = base
        .join(path)
        .map_err(|_| "Invalid plugin resource path".to_string())?;
    if url.origin() != base.origin() {
        return Err("Plugin resources must use the kernel origin".to_string());
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_release_auth_uses_configured_token_before_environment() {
        let configured = serde_json::json!({"github-token": " configured-fixture "});
        let token = plugin_github_token(&configured, Some("environment-fixture".to_string()));
        assert_eq!(token.as_deref(), Some("configured-fixture"));
        let empty = serde_json::json!({"github-token": " "});
        assert_eq!(
            plugin_github_token(&empty, Some(" environment-fixture ".to_string())).as_deref(),
            Some("environment-fixture")
        );
        assert_eq!(plugin_github_token(&empty, None), None);
        let client = reqwest::Client::new();
        let request = plugin_release_request(&client, "example/plugin", token.as_deref())
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().host_str(), Some("api.github.com"));
        assert_eq!(
            request.headers()["authorization"],
            "Bearer configured-fixture"
        );
        assert_eq!(request.headers()["cache-control"], "no-cache");
        let anonymous = plugin_release_request(&client, "example/plugin", None)
            .unwrap()
            .build()
            .unwrap();
        assert!(!anonymous.headers().contains_key("authorization"));
    }

    #[test]
    fn plugin_release_requests_are_restricted_to_github_repositories() {
        assert_eq!(
            plugin_release_url("example/plugin").unwrap(),
            "https://api.github.com/repos/example/plugin/releases/latest"
        );
        for repository in [
            "",
            "../plugin",
            "example/..",
            "example/plugin/extra",
            "https://example.test/plugin",
            "example/plugin?token=secret",
            "example/plugin#fragment",
            "example/%2e%2e",
        ] {
            assert!(plugin_release_url(repository).is_err());
        }
    }

    #[tokio::test]
    async fn plugin_release_settings_fall_back_to_environment_without_server_section() {
        let missing = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(404)
                .body(r#"{"error":"not_found"}"#)
                .unwrap(),
        );
        let settings = read_plugin_github_settings(missing).await.unwrap();
        assert_eq!(
            plugin_github_token(&settings, Some("environment-fixture".to_string())).as_deref(),
            Some("environment-fixture")
        );
        let unauthorized = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(401)
                .body(r#"{"error":"unauthorized"}"#)
                .unwrap(),
        );
        assert!(read_plugin_github_settings(unauthorized).await.is_err());
    }

    #[tokio::test]
    async fn plugin_release_refresh_reads_fresh_versions_and_surfaces_failures() {
        for (body, expected) in [
            (r#"{"tag_name":"v1.2.3"}"#, "1.2.3"),
            (r#"{"tag_name":"V2.0.0-rc.1"}"#, "2.0.0-rc.1"),
        ] {
            let response = reqwest::Response::from(
                tauri::http::Response::builder()
                    .status(200)
                    .body(body)
                    .unwrap(),
            );
            assert_eq!(read_plugin_release(response).await.unwrap(), expected);
        }
        for (status, body) in [
            (429, "sensitive body"),
            (401, "sensitive body"),
            (200, r#"{"tag_name":""}"#),
            (200, r#"{"tag_name":"latest"}"#),
            (200, r#"{"tag_name":"vv1.0.0"}"#),
            (200, r#"{"tag_name":"1_0"}"#),
            (200, "invalid"),
        ] {
            let response = reqwest::Response::from(
                tauri::http::Response::builder()
                    .status(status)
                    .body(body)
                    .unwrap(),
            );
            let error = read_plugin_release(response).await.unwrap_err();
            assert!(!error.contains("sensitive body"));
            if status == 429 {
                assert!(error.contains("GitHub API rate limited"));
            }
        }
    }

    #[test]
    fn plugin_support_respects_build_capability_headers() {
        for value in ["1", "TRUE", " yes ", "on"] {
            assert_eq!(parse_plugin_support_header(value), Some(true));
        }
        for value in ["0", "FALSE", " no ", "off"] {
            assert_eq!(parse_plugin_support_header(value), Some(false));
        }
        assert_eq!(parse_plugin_support_header("unknown"), None);
        assert_eq!(parse_plugin_support_header(""), None);
    }

    #[tokio::test]
    async fn plugin_support_uses_headers_before_list_fallback() {
        let response = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(200)
                .header("X-CPA-SUPPORT-PLUGIN", "false")
                .body(r#"{"plugins":[]}"#)
                .unwrap(),
        );
        assert!(!read_plugin_support(response).await.unwrap());
        let response = reqwest::Response::from(
            tauri::http::Response::builder()
                .status(200)
                .body(r#"{"plugins":[]}"#)
                .unwrap(),
        );
        assert!(read_plugin_support(response).await.unwrap());
        for status in [404, 501] {
            let response = reqwest::Response::from(
                tauri::http::Response::builder()
                    .status(status)
                    .body("")
                    .unwrap(),
            );
            assert!(!read_plugin_support(response).await.unwrap());
        }
        for (status, body) in [
            (401, r#"{"error":"unauthorized"}"#),
            (200, "<html>error</html>"),
        ] {
            let response = reqwest::Response::from(
                tauri::http::Response::builder()
                    .status(status)
                    .body(body)
                    .unwrap(),
            );
            assert!(read_plugin_support(response).await.is_err());
        }
    }

    #[test]
    fn plugin_pages_require_a_current_enabled_menu_declaration() {
        let payload = serde_json::json!({"plugins": [
            {"id": "enabled", "effective_enabled": true, "menus": [
                {"path": "/v0/resource/plugins/enabled/status"},
                {"path": "/v9/resource/plugins/enabled/settings"}
            ]},
            {"id": "disabled", "effective_enabled": false, "menus": [
                {"path": "/v0/resource/plugins/disabled/status"}
            ]}
        ]});
        assert_eq!(
            active_plugin_menu_path(&payload, "enabled", 1).unwrap(),
            "/v9/resource/plugins/enabled/settings"
        );
        assert!(active_plugin_menu_path(&payload, "enabled", 2).is_err());
        assert!(active_plugin_menu_path(&payload, "disabled", 0).is_err());
        assert!(active_plugin_menu_path(&payload, "deleted", 0).is_err());
    }

    #[test]
    fn plugin_resource_urls_preserve_declared_paths_and_local_tls_origin() {
        for (origin, path, expected) in [
            (
                "http://127.0.0.1:8317",
                "/v0/resource/plugins/demo/page",
                "http://127.0.0.1:8317/v0/resource/plugins/demo/page",
            ),
            (
                "https://[::1]:8443",
                "/v9/resource/plugins/demo/page?mode=full#section",
                "https://[::1]:8443/v9/resource/plugins/demo/page?mode=full#section",
            ),
        ] {
            assert_eq!(resolve_plugin_resource_url(origin, path).unwrap(), expected);
        }
        for path in [
            "https://example.com/page",
            "//example.com/page",
            "/\\example.com/page",
            "javascript:alert(1)",
            "/page\n",
        ] {
            assert!(resolve_plugin_resource_url("http://127.0.0.1:8317", path).is_err());
        }
    }
}
