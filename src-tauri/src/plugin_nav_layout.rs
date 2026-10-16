const MAX_LAYOUT_BYTES: usize = 256 * 1024;

fn read_layout(path: &std::path::Path) -> Result<Option<serde_json::Value>, String> {
    match std::fs::read(path) {
        Ok(content) => serde_json::from_slice(&content)
            .map(Some)
            .map_err(|_| "Failed to parse plugin navigation layout".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Failed to read plugin navigation layout".to_string()),
    }
}

fn write_layout(path: &std::path::Path, layout: &serde_json::Value) -> Result<(), String> {
    if !layout.is_object() {
        return Err("Invalid plugin navigation layout".into());
    }
    let content = serde_json::to_vec_pretty(layout).map_err(|error| error.to_string())?;
    if content.len() > MAX_LAYOUT_BYTES {
        return Err("Plugin navigation layout is too large".into());
    }
    super::write_bytes_atomically(path, &content)
}

#[tauri::command]
pub(crate) fn get_plugin_nav_layout() -> Result<Option<serde_json::Value>, String> {
    read_layout(&super::core_base_dir()?.join("plugin-nav-layout.json"))
}

#[tauri::command]
pub(crate) fn save_plugin_nav_layout(layout: serde_json::Value) -> Result<(), String> {
    write_layout(&super::core_base_dir()?.join("plugin-nav-layout.json"), &layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_nav_layout_round_trips_and_keeps_unreadable_files() {
        let directory = std::env::temp_dir().join(format!(
            "ezcpa-plugin-nav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("plugin-nav-layout.json");
        assert_eq!(read_layout(&path).unwrap(), None);
        let layout = serde_json::json!({
            "version": 1,
            "aliases": { "/v0/resource/plugins/a/b": "Renamed" },
            "custom": [{ "kind": "folder", "id": "f1", "name": "Tools", "children": ["/v0/resource/plugins/a/b"] }]
        });
        write_layout(&path, &layout).unwrap();
        assert_eq!(read_layout(&path).unwrap(), Some(layout));
        std::fs::write(&path, b"{broken").unwrap();
        assert!(read_layout(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        assert!(write_layout(&path, &serde_json::json!([1])).is_err());
        let huge = serde_json::json!({ "version": 1, "aliases": { "x": "y".repeat(MAX_LAYOUT_BYTES) }, "custom": [] });
        assert!(write_layout(&path, &huge).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
