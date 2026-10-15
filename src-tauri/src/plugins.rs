use super::{
    core_origin, current_core_tls_settings, format_management_request_error,
    management_authorization, management_endpoint, management_http_client, read_management_value,
    GuiConfigFile, GuiConfigState,
};

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
