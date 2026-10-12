// Privacy mode only changes how the GUI renders account identities. It is
// stored outside the webview so losing browser storage cannot silently turn it
// off while the user is sharing their screen.

fn read_preference(path: &std::path::Path) -> Result<Option<bool>, String> {
    match std::fs::read(path) {
        Ok(content) => serde_json::from_slice(&content)
            .map(Some)
            .map_err(|_| "Failed to parse privacy mode preference".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Failed to read privacy mode preference".to_string()),
    }
}

fn write_preference(path: &std::path::Path, enabled: bool) -> Result<(), String> {
    let content = serde_json::to_vec(&enabled).map_err(|error| error.to_string())?;
    super::write_bytes_atomically(path, &content)
}

#[tauri::command]
pub(crate) fn get_privacy_mode() -> Result<Option<bool>, String> {
    read_preference(&super::core_base_dir()?.join("privacy-mode.json"))
}

#[tauri::command]
pub(crate) fn save_privacy_mode(enabled: bool) -> Result<(), String> {
    write_preference(&super::core_base_dir()?.join("privacy-mode.json"), enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_mode_survives_restart_and_rejects_invalid_values() {
        let directory = std::env::temp_dir().join(format!(
            "ezcpa-privacy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("privacy-mode.json");
        assert_eq!(read_preference(&path).unwrap(), None);
        for enabled in [true, false] {
            write_preference(&path, enabled).unwrap();
            assert_eq!(read_preference(&path).unwrap(), Some(enabled));
        }
        std::fs::write(&path, b"\"on\"").unwrap();
        assert!(read_preference(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
