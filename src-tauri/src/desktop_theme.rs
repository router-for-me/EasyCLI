#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThemeSelection {
    version: u8,
    selected_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_light_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_dark_id: Option<String>,
}

impl ThemeSelection {
    fn validate(&self) -> Result<(), String> {
        let valid_id = |id: &str| {
            let bytes = id.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 64
                && bytes[0].is_ascii_lowercase()
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        };
        if self.version != 1
            || !valid_id(&self.selected_id)
            || [self.last_light_id.as_deref(), self.last_dark_id.as_deref()]
                .into_iter()
                .flatten()
                .any(|id| id == "system" || !valid_id(id))
        {
            return Err("Invalid theme selection".to_string());
        }
        Ok(())
    }
}

fn read_selection(path: &std::path::Path) -> Result<Option<ThemeSelection>, String> {
    match std::fs::read(path) {
        Ok(content) => {
            let selection: ThemeSelection = serde_json::from_slice(&content)
                .map_err(|_| "Failed to parse theme selection".to_string())?;
            selection.validate()?;
            Ok(Some(selection))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Failed to read theme selection".to_string()),
    }
}

fn write_selection(path: &std::path::Path, selection: &ThemeSelection) -> Result<(), String> {
    selection.validate()?;
    let content = serde_json::to_vec(selection).map_err(|error| error.to_string())?;
    super::write_bytes_atomically(path, &content)
}

// This configuration intentionally does not migrate theme-preference.json.
#[tauri::command]
pub(crate) fn get_theme_selection() -> Result<Option<ThemeSelection>, String> {
    read_selection(&super::core_base_dir()?.join("theme-selection.json"))
}

#[tauri::command]
pub(crate) fn save_theme_selection(selection: ThemeSelection) -> Result<(), String> {
    write_selection(
        &super::core_base_dir()?.join("theme-selection.json"),
        &selection,
    )
}

#[tauri::command]
pub(crate) async fn get_linux_system_theme() -> Option<tauri::Theme> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(read_portal_theme)
            .await
            .ok()
            .flatten()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn read_portal_theme() -> Option<tauri::Theme> {
    use dbus::{arg::Variant, blocking::Connection};
    use std::time::Duration;

    let connection = Connection::new_session().ok()?;
    let proxy = connection.with_proxy(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        Duration::from_secs(1),
    );
    let (value,): (Variant<Variant<u32>>,) = proxy
        .method_call(
            "org.freedesktop.portal.Settings",
            "Read",
            ("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    portal_color_scheme(value.0 .0)
}

#[cfg(any(target_os = "linux", test))]
fn portal_color_scheme(value: u32) -> Option<tauri::Theme> {
    match value {
        1 => Some(tauri::Theme::Dark),
        2 => Some(tauri::Theme::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_selection_preserves_ids_and_rejects_invalid_records() {
        let directory = std::env::temp_dir().join(format!(
            "ezcpa-theme-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("theme-selection.json");
        // An old configuration must not influence the new default.
        std::fs::write(directory.join("theme-preference.json"), b"\"dark\"").unwrap();
        assert_eq!(read_selection(&path).unwrap(), None);
        for id in ["light", "dark", "system", "theme1-dark", "removed-theme"] {
            let selection = ThemeSelection {
                version: 1,
                selected_id: id.to_string(),
                last_light_id: None,
                last_dark_id: None,
            };
            write_selection(&path, &selection).unwrap();
            assert_eq!(read_selection(&path).unwrap(), Some(selection));
        }
        for data in [
            r#""dark""#,
            r#"{"version":2,"selectedId":"light"}"#,
            r#"{"version":1,"selectedId":"../bad"}"#,
            r#"{"version":1,"selectedId":"system","lastLightId":"../bad"}"#,
            r#"{"version":1,"selectedId":"system","lastDarkId":"system"}"#,
            "{",
        ] {
            std::fs::write(&path, data).unwrap();
            assert!(read_selection(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
        }
        let invalid = ThemeSelection {
            version: 1,
            selected_id: String::new(),
            last_light_id: None,
            last_dark_id: None,
        };
        assert!(write_selection(&path, &invalid).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn theme_selection_round_trips_history_and_accepts_missing_history() {
        for data in [
            r#"{"version":1,"selectedId":"rose-paper-light"}"#,
            r#"{"version":1,"selectedId":"system","lastLightId":"rose-paper-light"}"#,
            r#"{"version":1,"selectedId":"system","lastLightId":"rose-paper-light","lastDarkId":"pine-dark"}"#,
            r#"{"version":1,"selectedId":"system","lastDarkId":"removed-theme"}"#,
        ] {
            let selection: ThemeSelection = serde_json::from_str(data).unwrap();
            selection.validate().unwrap();
            let restored: ThemeSelection =
                serde_json::from_slice(&serde_json::to_vec(&selection).unwrap()).unwrap();
            assert_eq!(restored, selection);
        }
        for (light, dark) in [("system", "dark"), ("light", "../bad"), ("", "dark")] {
            let invalid = ThemeSelection {
                version: 1,
                selected_id: "system".to_string(),
                last_light_id: Some(light.to_string()),
                last_dark_id: Some(dark.to_string()),
            };
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn follows_the_freedesktop_color_scheme_values() {
        assert_eq!(portal_color_scheme(1), Some(tauri::Theme::Dark));
        assert_eq!(portal_color_scheme(2), Some(tauri::Theme::Light));
        assert_eq!(portal_color_scheme(0), None);
        assert_eq!(portal_color_scheme(3), None);
    }
}
