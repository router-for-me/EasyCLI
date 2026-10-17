use super::*;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn platform() -> Result<(&'static str, &'static str, &'static str), String> {
    let (os, ext) = match std::env::consts::OS {
        "windows" => ("windows", "dll"),
        "linux" => ("linux", "so"),
        "macos" => ("darwin", "dylib"),
        _ => return Err("Unsupported plugin platform".into()),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        _ => return Err("Unsupported plugin architecture".into()),
    };
    Ok((os, arch, ext))
}

fn plugin_filename(source: &Path, extension: &str) -> Result<String, String> {
    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid plugin filename")?;
    if !source
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(extension))
    {
        return Err(format!(
            "Select a .{extension} plugin binary for this platform (extract archives first)"
        ));
    }
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid plugin filename")?;
    let id = stem
        .rsplit_once("-v")
        .filter(|(_, v)| {
            v.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
        })
        .map(|(id, _)| id)
        .unwrap_or(stem);
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Invalid plugin ID in filename".into());
    }
    Ok(name.into())
}

// Use an exclusive rename, not a hard link: removable FAT/exFAT volumes do
// not support hard links. Never emulate this with exists() + fs::rename(),
// which can overwrite a destination created concurrently on Unix.
fn publish_plugin(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // No REPLACE_EXISTING or COPY_ALLOWED: same-volume rename only.
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let target = std::ffi::CString::new(target.as_os_str().as_bytes())?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (source, target);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unsupported plugin platform",
        ))
    }
}

pub(crate) fn copy_plugin(
    source: &Path,
    directory: &Path,
    extension: &str,
) -> Result<String, String> {
    let name = plugin_filename(source, extension)?;
    let mut input = fs::File::open(source).map_err(|e| format!("Cannot open plugin: {e}"))?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    const LIMIT: u64 = 256 * 1024 * 1024;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > LIMIT {
        return Err("Plugin must be a nonempty regular file no larger than 256 MiB".into());
    }
    fs::create_dir_all(directory).map_err(|e| format!("Cannot create plugin directory: {e}"))?;
    let target = directory.join(name);
    if target.exists() {
        return Err("A plugin file with this name already exists; uninstall it first or select a versioned filename".into());
    }
    // Publish a complete file without replacing an existing file or symlink.
    // The temporary extension is not discoverable by the kernel loader.
    let temporary = directory.join(format!(
        ".local-install-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let copied = std::io::copy(&mut (&mut input).take(LIMIT + 1), &mut output)
            .map_err(|e| e.to_string())?;
        if copied != metadata.len() || copied > LIMIT {
            return Err("Plugin file changed during installation".into());
        }
        output.flush().map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        publish_plugin(&temporary, &target).map_err(|e| {
            format!("Cannot install plugin without overwriting existing files: {e}")
        })?;
        Ok(target.to_string_lossy().into_owned())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[tauri::command]
pub(crate) async fn install_local_plugin(
    source: String,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<String, String> {
    let config = gui_config_state.snapshot()?;
    let value = read_management_value(super::request_plugins(&config).await?).await?;
    let dir = value
        .get("plugins_dir")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("Kernel did not return a plugin directory")?;
    let directory = PathBuf::from(dir);
    let directory = if directory.is_absolute() {
        directory
    } else {
        super::super::core_install_dir()?.join(directory)
    };
    let (os, arch, ext) = platform()?;
    tauri::async_runtime::spawn_blocking(move || {
        copy_plugin(Path::new(&source), &directory.join(os).join(arch), ext)
    })
    .await
    .map_err(|e| format!("Local plugin installation failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_publication_moves_complete_file_and_rejects_existing_target() {
        let root = crate::unique_test_temp_path("plugin-publication");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("staged.tmp");
        let target = root.join("plugin.dll");
        fs::write(&source, b"complete plugin").unwrap();
        publish_plugin(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"complete plugin");
        // Bypass copy_plugin's early exists check to exercise the race guard.
        fs::write(&source, b"replacement").unwrap();
        assert!(publish_plugin(&source, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"complete plugin");
        assert_eq!(fs::read(&source).unwrap(), b"replacement");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn filenames_match_kernel_discovery() {
        assert!(plugin_filename(Path::new("router-v1.2.3.dll"), "dll").is_ok());
        assert!(plugin_filename(Path::new("router.zip"), "dll").is_err());
        assert!(plugin_filename(Path::new("bad name.dll"), "dll").is_err());
        assert!(plugin_filename(Path::new(".dll"), "dll").is_err());
    }
    #[test]
    fn copy_is_complete_and_never_overwrites() {
        let root = crate::unique_test_temp_path("local-plugin");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("sample.dll");
        fs::write(&source, b"plugin fixture").unwrap();
        let directory = root.join("plugins/windows/amd64");
        let result = copy_plugin(&source, &directory, "dll").unwrap();
        assert_eq!(fs::read(result).unwrap(), b"plugin fixture");
        fs::write(&source, b"replacement").unwrap();
        assert!(copy_plugin(&source, &directory, "dll").is_err());
        assert_eq!(
            fs::read(directory.join("sample.dll")).unwrap(),
            b"plugin fixture"
        );
        assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_inputs_do_not_create_installation_files() {
        let root = crate::unique_test_temp_path("invalid-local-plugin");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("empty.dll");
        fs::write(&source, []).unwrap();
        let directory = root.join("plugins");
        assert!(copy_plugin(&source, &directory, "dll").is_err());
        fs::File::create(&source)
            .unwrap()
            .set_len(256 * 1024 * 1024 + 1)
            .unwrap();
        assert!(copy_plugin(&source, &directory, "dll").is_err());
        assert!(!directory.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
