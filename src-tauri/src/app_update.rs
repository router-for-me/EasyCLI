use super::*;

pub(crate) const PORTABLE_UPDATE_HELPER_ACK_FILE: &str = "update-helper-started.ack";
const PORTABLE_UPDATE_HELPER_START_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn deserialize_release_notes<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    // Untagged legacy text and invalid entries cannot identify a translation.
    // Treat them as unavailable instead of failing the entire version check.
    let mut notes = HashMap::new();
    if let Some(translations) = value.as_object() {
        for locale in ["zh-CN", "zh-TW", "en", "ja"] {
            if let Some(text) = translations.get(locale).and_then(|value| value.as_str()) {
                if !text.trim().is_empty() {
                    notes.insert(locale.to_string(), text.to_string());
                }
            }
        }
    }
    Ok(notes)
}

#[tauri::command]
pub(crate) fn get_version_source_settings(
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<VersionSourceSettings, String> {
    let config = gui_config_state.snapshot()?;
    Ok(version_source_settings(&config))
}

fn version_source_settings(config: &GuiConfigFile) -> VersionSourceSettings {
    VersionSourceSettings {
        source: config.selected_download_candidate().key(),
        gitcode_available: gitcode_version_source_available(),
        custom_mirrors: config.custom_download_mirrors.clone(),
    }
}

#[tauri::command]
pub(crate) fn set_download_source(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    source: String,
) -> Result<VersionSourceSettings, String> {
    let candidate = if let Some(url) = source.strip_prefix("custom:") {
        VersionDownloadCandidate::custom(&normalize_custom_download_mirror_url(url)?)
    } else {
        VersionDownloadCandidate::builtin(
            VersionDownloadSource::from_str(&source)
                .filter(|source| *source != VersionDownloadSource::Custom)
                .ok_or_else(|| "Unsupported download source".to_string())?,
        )
    };
    if candidate.source == VersionDownloadSource::Gitcode && !gitcode_version_source_available() {
        return Err("This build does not include complete GitCode application and kernel mirror sources".to_string());
    }
    let config = gui_config_state.set_download_candidate(candidate)?;
    Ok(version_source_settings(&config))
}

#[tauri::command]
pub(crate) fn add_custom_download_mirror(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    url: String,
) -> Result<VersionSourceSettings, String> {
    let url = normalize_custom_download_mirror_url(&url)?;
    let config = gui_config_state.update(|config| {
        if !config.custom_download_mirrors.contains(&url) {
            if config.custom_download_mirrors.len() >= 12 {
                return Err("You can add up to 12 custom mirrors".to_string());
            }
            config.custom_download_mirrors.push(url.clone());
        }
        config.download_source = VersionDownloadSource::Custom;
        config.active_custom_download_mirror = url.clone();
        config.prefer_gitcode_downloads = false;
        Ok(())
    })?;
    Ok(version_source_settings(&config))
}

#[tauri::command]
pub(crate) fn remove_custom_download_mirror(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    url: String,
) -> Result<VersionSourceSettings, String> {
    let url = normalize_custom_download_mirror_url(&url)?;
    let config = gui_config_state.update(|config| {
        config.custom_download_mirrors.retain(|item| item != &url);
        if config.download_source == VersionDownloadSource::Custom
            && config.active_custom_download_mirror == url
        {
            config.download_source = VersionDownloadSource::Github;
            config.active_custom_download_mirror.clear();
        }
        Ok(())
    })?;
    Ok(version_source_settings(&config))
}

#[tauri::command]
pub(crate) fn set_prefer_gitcode_downloads(
    gui_config_state: tauri::State<'_, GuiConfigState>,
    enabled: bool,
) -> Result<VersionSourceSettings, String> {
    let source = if enabled {
        VersionDownloadSource::Gitcode
    } else {
        VersionDownloadSource::Github
    };
    if enabled && !gitcode_version_source_available() {
        return Err("This build does not include complete GitCode application and kernel mirror sources".to_string());
    }
    let config = gui_config_state.set_download_source(source)?;
    Ok(version_source_settings(&config))
}

pub(crate) fn gitcode_version_source_available() -> bool {
    configured_gitcode_gui_repository().is_some() && configured_gitcode_core_repository().is_some()
}

pub(crate) fn persist_automatic_download_source_switch(
    app: &tauri::AppHandle,
    gui_config_state: &GuiConfigState,
    requested_source: &VersionDownloadCandidate,
    resolved_source: &VersionDownloadCandidate,
) -> Result<(), String> {
    let Some(config) =
        gui_config_state.switch_download_source_after_failure(requested_source, resolved_source)?
    else {
        return Ok(());
    };
    app.emit(
        VERSION_DOWNLOAD_SOURCE_CHANGED_EVENT,
        version_source_settings(&config),
    )
    .map_err(|error| format!("Failed to notify automatic download source switch: {error}"))
}

#[tauri::command]
pub(crate) fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    open_external_url_inner(&app, &url)
}

#[tauri::command]
pub(crate) async fn check_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdateState>,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<AppUpdateInfo, String> {
    // Startup and manual version checks also refresh model defaults. Keep this
    // independent so a catalog download failure does not fail the version check.
    let catalog_app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = update_codex_model_catalog_inner(&catalog_app).await {
            eprintln!("Failed to update Codex model catalog during version check; continuing with current catalog: {error}");
        }
    });
    let (info, _) = resolve_app_update(&app, gui_config_state.inner()).await?;
    state.set_available(AppUpdateTask {
        phase: if info.update_available {
            "available".to_string()
        } else {
            "idle".to_string()
        },
        target_version: info.update_available.then(|| info.latest_version.clone()),
        total_bytes: info.download_size_bytes,
        ..AppUpdateTask::default()
    });
    Ok(info)
}

async fn resolve_app_update(
    app: &tauri::AppHandle,
    gui_config_state: &GuiConfigState,
) -> Result<(AppUpdateInfo, Option<PendingAppUpdate>), String> {
    let _detection_guard = VERSION_SOURCE_DETECTION_LOCK.lock().await;
    let config = gui_config_state.snapshot()?;
    let proxy_url = config.proxy_url.clone();
    let client = build_http_client_with_proxy(
        reqwest::Client::builder()
            .redirect(release_https_redirect_policy_with_mirrors(
                &config.custom_download_mirrors,
            ))
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(20)),
        &proxy_url,
        "Failed to create version check client",
    )?;
    let requested_source = config.selected_download_candidate();
    let (manifest, resolved_source) = fetch_portable_update_manifest(
        &client,
        requested_source.clone(),
        &config.custom_download_mirrors,
    )
    .await?;
    persist_automatic_download_source_switch(
        app,
        gui_config_state,
        &requested_source,
        &resolved_source,
    )?;
    let latest_version = normalize_version(&manifest.version);
    let current_version = normalize_version(env!("CARGO_PKG_VERSION"));
    let update_available = is_app_update_available(&current_version, &latest_version)?;
    let target = portable_update_target();
    let asset_catalog = manifest.full_assets.as_ref().unwrap_or(&manifest.assets);
    let asset = target.and_then(|(key, _)| asset_catalog.get(key)).cloned();
    let portable_support = target
        .map(|(_, arch)| validate_local_portable_app_manifest(arch))
        .transpose()?;
    let auto_update_supported = portable_support == Some(true) && asset.is_some();
    let unsupported_reason = if auto_update_supported {
        None
    } else if portable_support != Some(true) {
        Some("This application is not a portable version that supports automatic upgrades. Manually download the first supported version".to_string())
    } else {
        Some("The update manifest does not include the current platform or architecture".to_string())
    };

    let pending = if update_available && auto_update_supported {
        let (_, arch) = target.expect("portable target checked above");
        Some(PendingAppUpdate {
            version: latest_version.clone(),
            asset: asset.clone().expect("portable asset checked above"),
            arch: arch.to_string(),
        })
    } else {
        None
    };
    let info = AppUpdateInfo {
        current_version,
        latest_version,
        update_available,
        release_url: manifest.release_url,
        release_notes: manifest.release_notes,
        published_at: manifest.published_at,
        auto_update_supported,
        download_size_bytes: asset.map(|value| value.size_bytes),
        unsupported_reason,
    };
    Ok((info, pending))
}

pub(crate) async fn fetch_portable_update_manifest(
    client: &reqwest::Client,
    source: VersionDownloadCandidate,
    custom_mirrors: &[String],
) -> Result<(PortableUpdateManifest, VersionDownloadCandidate), String> {
    let gitcode_repository = configured_gitcode_gui_repository();
    let mut failures = Vec::new();
    for candidate in
        version_download_source_candidates(source, gitcode_repository.is_some(), custom_mirrors)
    {
        let result = match candidate.source {
            VersionDownloadSource::Gitcode => {
                fetch_portable_update_manifest_from_gitcode(
                    client,
                    gitcode_repository.expect("GitCode candidate requires a configured repository"),
                )
                .await
            }
            VersionDownloadSource::Github
            | VersionDownloadSource::GhProxy
            | VersionDownloadSource::GhFast => {
                let url = version_source_url(&candidate, APP_UPDATE_MANIFEST_URL);
                fetch_portable_update_manifest_url(client, &url).await
            }
            VersionDownloadSource::Custom => {
                let url = version_source_url(&candidate, APP_UPDATE_MANIFEST_URL);
                fetch_portable_update_manifest_url(client, &url).await
            }
        };
        match result {
            Ok(manifest) => return Ok((manifest, candidate)),
            Err(error) => failures.push(format!("{}: {error}", candidate.display_name())),
        }
    }
    Err(format!("All application version check sources failed: {}", failures.join("; ")))
}

pub(crate) async fn fetch_portable_update_manifest_from_gitcode(
    client: &reqwest::Client,
    repository: &str,
) -> Result<PortableUpdateManifest, String> {
    let release_url = format!("https://api.gitcode.com/api/v5/repos/{repository}/releases/latest");
    let release = app_update_metadata_request(client, &release_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("query GitCode latest release: {error}"))?
        .error_for_status()
        .map_err(|error| format!("read GitCode latest release: {error}"))?
        .json::<GitcodeRelease>()
        .await
        .map_err(|error| format!("parse GitCode latest release: {error}"))?;
    validate_release_tag(&release.tag_name)?;
    let manifest_url =
        gitcode_release_attachment_url(repository, &release.tag_name, APP_UPDATE_MANIFEST_NAME);
    fetch_portable_update_manifest_url(client, &manifest_url).await
}

pub(crate) async fn fetch_portable_update_manifest_url(
    client: &reqwest::Client,
    url: &str,
) -> Result<PortableUpdateManifest, String> {
    let manifest = app_update_metadata_request(client, url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("request update manifest: {error}"))?
        .error_for_status()
        .map_err(|error| format!("read update manifest: {error}"))?
        .json::<PortableUpdateManifest>()
        .await
        .map_err(|error| format!("parse update manifest: {error}"))?;
    validate_portable_update_manifest(&manifest)?;
    Ok(manifest)
}

fn app_update_metadata_request(client: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
    client
        .get(url)
        .query(&[("_", nonce)])
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .header(reqwest::header::PRAGMA, "no-cache")
}

pub(crate) fn configured_gitcode_gui_repository() -> Option<&'static str> {
    configured_gitcode_repository(option_env!("GITCODE_GUI_REPOSITORY"))
}

pub(crate) fn configured_gitcode_core_repository() -> Option<&'static str> {
    configured_gitcode_repository(option_env!("GITCODE_CORE_REPOSITORY"))
}

pub(crate) fn configured_gitcode_repository(
    repository: Option<&'static str>,
) -> Option<&'static str> {
    let repository = repository?.trim();
    let mut parts = repository.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repo), None) if valid_part(owner) && valid_part(repo) => {
            Some(repository)
        }
        _ => None,
    }
}

pub(crate) fn validate_release_tag(tag: &str) -> Result<(), String> {
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map(|_| ())
        .map_err(|error| format!("invalid release tag {tag}: {error}"))
}

pub(crate) fn gitcode_release_attachment_url(
    repository: &str,
    tag: &str,
    filename: &str,
) -> String {
    format!(
        "https://api.gitcode.com/api/v5/repos/{repository}/releases/{tag}/attach_files/{filename}/download"
    )
}

pub(crate) fn release_https_redirect_policy_with_mirrors(
    custom_mirrors: &[String],
) -> reqwest::redirect::Policy {
    let custom_hosts = custom_mirrors
        .iter()
        .filter_map(|value| reqwest::Url::parse(value).ok())
        .filter_map(|url| url.host_str().map(str::to_string))
        .collect::<HashSet<_>>();
    reqwest::redirect::Policy::custom(move |attempt| {
        let url = attempt.url();
        let trusted_host = matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "api.gitcode.com"
                    | "gitcode.com"
                    | "file-cdn.gitcode.com"
                    | "gh-proxy.com"
                    | "ghfast.top"
            )
        ) || url
            .host_str()
            .is_some_and(|host| custom_hosts.contains(host));
        if url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && trusted_host
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

pub(crate) fn github_https_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let url = attempt.url();
        let trusted_host = matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "raw.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
            )
        );
        if url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && trusted_host
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

pub(crate) fn validate_portable_update_manifest(
    manifest: &PortableUpdateManifest,
) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err(format!(
            "Unsupported application update manifest version: {}",
            manifest.schema_version
        ));
    }
    semver::Version::parse(manifest.version.trim().trim_start_matches('v'))
        .map_err(|error| format!("Invalid application update version: {error}"))?;
    chrono::DateTime::parse_from_rfc3339(manifest.published_at.trim())
        .map_err(|error| format!("Invalid application update release date: {error}"))?;
    let release_url = reqwest::Url::parse(&manifest.release_url)
        .map_err(|_| "Invalid application update release URL".to_string())?;
    if release_url.scheme() != "https"
        || release_url.host_str() != Some("github.com")
        || release_url.port().is_some()
        || !release_url.username().is_empty()
        || release_url.password().is_some()
        || release_url.query().is_some()
        || release_url.fragment().is_some()
        || !release_url
            .path()
            .starts_with("/router-for-me/EasyCLIProxyAPI/releases/tag/v")
    {
        return Err("Untrusted application update release URL".to_string());
    }
    let (platform, display_platform, suffix) = portable_update_asset_platform()?;
    validate_portable_update_asset_catalog(
        &manifest.assets,
        manifest,
        platform,
        display_platform,
        suffix,
        platform == "windows",
    )?;
    if let Some(full_assets) = &manifest.full_assets {
        validate_portable_update_asset_catalog(
            full_assets,
            manifest,
            platform,
            display_platform,
            suffix,
            false,
        )?;
    }
    Ok(())
}

pub(crate) fn portable_update_asset_platform(
) -> Result<(&'static str, &'static str, &'static str), String> {
    match portable_update_platform_key() {
        Some("windows") => Ok(("windows", "Windows", "zip")),
        Some("linux") => Ok(("linux", "Linux", "tar.gz")),
        Some("darwin") => Ok(("darwin", "Darwin", "dmg")),
        _ => Err("The current platform does not support in-app automatic upgrades".to_string()),
    }
}

fn validate_portable_update_asset_catalog(
    assets: &std::collections::HashMap<String, PortableUpdateAsset>,
    manifest: &PortableUpdateManifest,
    platform: &str,
    display_platform: &str,
    suffix: &str,
    allow_windows_legacy: bool,
) -> Result<(), String> {
    if assets.len() != 2 {
        return Err(format!("The application update manifest must include both {display_platform} architectures"));
    }
    let version = manifest.version.trim().trim_start_matches('v');
    let tag = format!("v{version}");
    for arch in ["amd64", "aarch64"] {
        let key = format!("{platform}-{arch}");
        let asset = assets
            .get(&key)
            .ok_or_else(|| format!("Application update manifest is missing {key}"))?;
        validate_portable_update_asset(asset)?;
        let full_package_name =
            format!("EasyCLIProxyAPI-v{version}-{display_platform}-{arch}.{suffix}");
        let full_package_url = format!("{APP_RELEASE_DOWNLOAD_PREFIX}{tag}/{full_package_name}");
        let legacy_package_name = format!("EasyCLIProxyAPI-update-v{version}-Windows-{arch}.zip");
        let legacy_package_url =
            format!("{APP_RELEASE_DOWNLOAD_PREFIX}{tag}/{legacy_package_name}");
        let is_expected = asset.url == full_package_url
            || (allow_windows_legacy && asset.url == legacy_package_url);
        if !is_expected {
            return Err(format!("Application update asset name does not match {key}"));
        }
        let mut expected_filenames = vec![full_package_name.as_str()];
        if allow_windows_legacy {
            expected_filenames.push(legacy_package_name.as_str());
        }
        validate_portable_update_asset_fallbacks(asset, &tag, &expected_filenames)?;
    }
    Ok(())
}

pub(crate) fn validate_portable_update_asset(asset: &PortableUpdateAsset) -> Result<(), String> {
    let url = reqwest::Url::parse(&asset.url).map_err(|_| "Invalid application update download URL".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !asset.url.starts_with(APP_RELEASE_DOWNLOAD_PREFIX)
    {
        return Err("Untrusted application update download URL".to_string());
    }
    if asset.size_bytes == 0 || asset.size_bytes > 512 * 1024 * 1024 {
        return Err("Invalid application update package size".to_string());
    }
    let digest = asset.sha256.trim().to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid application update SHA-256".to_string());
    }
    Ok(())
}

pub(crate) fn validate_portable_update_asset_fallbacks(
    asset: &PortableUpdateAsset,
    tag: &str,
    expected_filenames: &[&str],
) -> Result<(), String> {
    validate_portable_update_asset_fallbacks_for_repository(
        asset,
        tag,
        expected_filenames,
        configured_gitcode_gui_repository(),
    )
}

pub(crate) fn validate_portable_update_asset_fallbacks_for_repository(
    asset: &PortableUpdateAsset,
    tag: &str,
    expected_filenames: &[&str],
    repository: Option<&str>,
) -> Result<(), String> {
    if asset.fallback_urls.is_empty() {
        return Ok(());
    }
    let repository = repository
        .ok_or_else(|| "GitCode fallback is not configured for this build".to_string())?;
    if asset.fallback_urls.len() != 1 {
        return Err("Software update assets may define only one fallback URL".to_string());
    }
    let fallback = &asset.fallback_urls[0];
    let trusted = expected_filenames
        .iter()
        .any(|filename| fallback == &gitcode_release_attachment_url(repository, tag, filename));
    if !trusted {
        return Err("Software update fallback URL is not trusted".to_string());
    }
    Ok(())
}

pub(crate) fn portable_update_target() -> Option<(&'static str, &'static str)> {
    let platform = portable_update_platform_key()?;
    let arch = match env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    match (platform, arch) {
        ("windows", "amd64") => Some(("windows-amd64", "amd64")),
        ("windows", "aarch64") => Some(("windows-aarch64", "aarch64")),
        ("linux", "amd64") => Some(("linux-amd64", "amd64")),
        ("linux", "aarch64") => Some(("linux-aarch64", "aarch64")),
        ("darwin", "amd64") => Some(("darwin-amd64", "amd64")),
        ("darwin", "aarch64") => Some(("darwin-aarch64", "aarch64")),
        _ => None,
    }
}

pub(crate) fn portable_update_platform_key() -> Option<&'static str> {
    match env::consts::OS {
        "windows" => Some("windows"),
        "linux" => Some("linux"),
        "macos" => Some("darwin"),
        _ => None,
    }
}

pub(crate) fn local_portable_app_manifest_path() -> Result<PathBuf, String> {
    let executable_dir = executable_dir()?;
    #[cfg(target_os = "macos")]
    if let Some(resources_dir) = macos_app_resources_dir(&executable_dir) {
        return Ok(resources_dir.join(PORTABLE_APP_MANIFEST_FILE));
    }
    Ok(executable_dir.join(PORTABLE_APP_MANIFEST_FILE))
}

pub(crate) fn validate_local_portable_app_manifest(expected_arch: &str) -> Result<bool, String> {
    let path = local_portable_app_manifest_path()?;
    if !path.is_file() {
        return Ok(false);
    }
    let contents =
        fs::read_to_string(&path).map_err(|error| format!("Failed to read portable version marker: {error}"))?;
    let manifest = serde_json::from_str::<PortableAppManifest>(&contents)
        .map_err(|error| format!("Failed to parse portable version marker: {error}"))?;
    Ok(manifest.schema_version == 1
        && manifest.application == "EasyCLIProxyAPI"
        && Some(manifest.platform.as_str()) == portable_update_platform_key()
        && manifest.arch == expected_arch
        && manifest.auto_update
        && normalize_version(&manifest.version) == normalize_version(env!("CARGO_PKG_VERSION")))
}

#[tauri::command]
pub(crate) fn get_app_update_task(state: tauri::State<'_, AppUpdateState>) -> AppUpdateTask {
    state.snapshot()
}

#[tauri::command]
pub(crate) fn cancel_app_update(state: tauri::State<'_, AppUpdateState>) -> Result<(), String> {
    let task = state.snapshot();
    if !task.running || !task.cancellable {
        return Err("The current application update stage cannot be canceled".to_string());
    }
    state.cancel();
    Ok(())
}

#[tauri::command]
pub(crate) async fn start_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdateState>,
) -> Result<(), String> {
    if portable_update_platform_key().is_none() {
        return Err("The current platform does not support in-app automatic upgrades".to_string());
    }
    let token = CancellationToken::new();
    state.start(token.clone())?;
    let task = state.snapshot();
    let _ = app.emit(APP_UPDATE_PROGRESS_EVENT, task);
    let update_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = async {
            let state = update_app.state::<AppUpdateState>();
            let gui_config_state = update_app.state::<GuiConfigState>();
            let pending = refresh_app_update_for_install(state.inner(), &token, async {
                let (info, pending) =
                    resolve_app_update(&update_app, gui_config_state.inner()).await?;
                pending.ok_or_else(|| {
                    if info.update_available {
                        info.unsupported_reason
                            .unwrap_or_else(|| "The current version does not support automatic updates".to_string())
                    } else {
                        "No application update is currently available to install. Check for updates again".to_string()
                    }
                })
            })
            .await?;
            let _ = update_app.emit(APP_UPDATE_PROGRESS_EVENT, state.snapshot());
            let config = gui_config_state.snapshot()?;
            download_and_stage_portable_app_update(
                &update_app,
                &pending,
                &token,
                &config.proxy_url,
                config.selected_download_candidate(),
            )
            .await
        }
        .await;
        if let Err(error) = outcome {
            let state = update_app.state::<AppUpdateState>();
            let cancelled = token.is_cancelled();
            let task = state.finish(
                if cancelled { "cancelled" } else { "failed" },
                Some(if cancelled {
                    "Application update download canceled".to_string()
                } else {
                    error
                }),
            );
            let _ = update_app.emit(APP_UPDATE_PROGRESS_EVENT, task);
        }
    });
    Ok(())
}

pub(crate) async fn refresh_app_update_for_install<F>(
    state: &AppUpdateState,
    token: &CancellationToken,
    refresh: F,
) -> Result<PendingAppUpdate, String>
where
    F: std::future::Future<Output = Result<PendingAppUpdate, String>>,
{
    let pending = tokio::select! {
        biased;
        _ = token.cancelled() => return Err("Application update download canceled".to_string()),
        result = refresh => result?,
    };
    state.start_download(&pending)?;
    Ok(pending)
}

pub(crate) async fn download_and_stage_portable_app_update(
    app: &tauri::AppHandle,
    pending: &PendingAppUpdate,
    token: &CancellationToken,
    proxy_url: &str,
    download_source: VersionDownloadCandidate,
) -> Result<(), String> {
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (app, pending, token, proxy_url, download_source);
        Err("The current platform does not support in-app automatic upgrades".to_string())
    }

    #[cfg(windows)]
    {
        validate_portable_update_asset(&pending.asset)?;
        let work_dir = env::temp_dir().join(format!(
            "EasyCLIProxyAPI-update-{}-{}-{}",
            pending.version,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&work_dir)
            .map_err(|error| format!("Failed to create temporary application update directory: {error}"))?;
        let archive_path = work_dir.join("update.zip");
        let result = async {
            download_portable_update_archive(
                app,
                pending,
                token,
                &archive_path,
                proxy_url,
                &download_source,
            )
            .await?;
            if token.is_cancelled() {
                return Err("Application update download canceled".to_string());
            }

            update_app_task(app, |task| {
                task.cancellable = false;
                task.phase = "verifying".to_string();
                task.message = Some("Verifying application update package".to_string());
            });
            let actual_sha256 = sha256_file(&archive_path)?;
            if actual_sha256 != pending.asset.sha256.trim().to_ascii_lowercase() {
                return Err("Application update package SHA-256 verification failed".to_string());
            }

            update_app_task(app, |task| {
                task.phase = "staging".to_string();
                task.message = Some("Preparing application update".to_string());
            });
            let staging_dir = work_dir.join("staging");
            let package = extract_portable_update_archive(&archive_path, &staging_dir)?;
            if normalize_version(&package.manifest.version) != normalize_version(&pending.version)
                || package.manifest.platform != "windows"
                || package.manifest.arch != pending.arch
            {
                return Err("Application update package version or architecture mismatch".to_string());
            }

            let current_exe =
                env::current_exe().map_err(|error| format!("Failed to read current executable path: {error}"))?;
            let app_dir = current_exe
                .parent()
                .ok_or_else(|| "Current executable path has no parent directory".to_string())?;
            preflight_portable_update_directory(app_dir)?;
            let core_archive_name = match package.core_archive_name {
                Some(name) => name,
                None => stage_current_portable_core_payload(app_dir, &staging_dir, &pending.arch)?,
            };
            let helper_path = work_dir.join("EasyCLIProxyAPI-updater.exe");
            fs::copy(&current_exe, &helper_path)
                .map_err(|error| format!("Failed to prepare application update helper: {error}"))?;

            let descriptor = PortableUpdateDescriptor {
                parent_pid: std::process::id(),
                current_exe: current_exe.clone(),
                staged_exe: staging_dir.join(PORTABLE_APP_BINARY),
                current_manifest: app_dir.join(PORTABLE_APP_MANIFEST_FILE),
                staged_manifest: staging_dir.join(PORTABLE_APP_MANIFEST_FILE),
                backup_exe: app_dir.join(".EasyCLIProxyAPI.exe.update-backup"),
                backup_manifest: app_dir.join(".portable-app.json.update-backup"),
                current_core_version: app_dir.join(CORE_VERSION_FILE),
                staged_core_version: staging_dir.join(CORE_VERSION_FILE),
                backup_core_version: app_dir.join(".core-version.txt.update-backup"),
                staged_core_archive: staging_dir.join("cpa-core").join(&core_archive_name),
                target_core_archive: app_dir.join("cpa-core").join(&core_archive_name),
                install_core_archive: !app_dir.join("cpa-core").join(&core_archive_name).is_file(),
                ack_path: work_dir.join("update-started.ack"),
                work_dir: work_dir.clone(),
                target_version: pending.version.clone(),
            };
            launch_portable_update_helper(app, &helper_path, &work_dir, &descriptor).await
        }
        .await;

        if result.is_err() {
            let _ = fs::remove_dir_all(&work_dir);
        }
        result
    }

    #[cfg(target_os = "linux")]
    {
        validate_portable_update_asset(&pending.asset)?;
        let work_dir = portable_update_work_dir(&pending.version);
        fs::create_dir_all(&work_dir)
            .map_err(|error| format!("Failed to create temporary application update directory: {error}"))?;
        let archive_path = work_dir.join("update.tar.gz");
        let result = async {
            download_portable_update_archive(
                app,
                pending,
                token,
                &archive_path,
                proxy_url,
                &download_source,
            )
            .await?;
            ensure_portable_update_download(token, &archive_path, pending)?;
            update_app_task(app, |task| {
                task.cancellable = false;
                task.phase = "staging".to_string();
                task.message = Some("Preparing application update".to_string());
            });

            let staging_dir = work_dir.join("staging");
            let package = extract_portable_update_tar_gz(&archive_path, &staging_dir)?;
            if normalize_version(&package.manifest.version) != normalize_version(&pending.version)
                || package.manifest.platform != "linux"
                || package.manifest.arch != pending.arch
            {
                return Err("Application update package version or architecture mismatch".to_string());
            }
            let core_archive_name = package
                .core_archive_name
                .ok_or_else(|| "Linux application update package is missing the bundled kernel".to_string())?;
            let current_exe =
                env::current_exe().map_err(|error| format!("Failed to read current executable path: {error}"))?;
            let app_dir = current_exe
                .parent()
                .ok_or_else(|| "Current executable path has no parent directory".to_string())?;
            preflight_portable_update_directory(app_dir)?;
            let helper_path = work_dir.join("EasyCLIProxyAPI-updater");
            fs::copy(&current_exe, &helper_path)
                .map_err(|error| format!("Failed to prepare application update helper: {error}"))?;

            let descriptor = PortableUpdateDescriptor {
                parent_pid: std::process::id(),
                current_exe: current_exe.clone(),
                staged_exe: staging_dir.join(PORTABLE_APP_BINARY),
                current_manifest: app_dir.join(PORTABLE_APP_MANIFEST_FILE),
                staged_manifest: staging_dir.join(PORTABLE_APP_MANIFEST_FILE),
                backup_exe: app_dir.join(".EasyCLIProxyAPI.update-backup"),
                backup_manifest: app_dir.join(".portable-app.json.update-backup"),
                current_core_version: app_dir.join(CORE_VERSION_FILE),
                staged_core_version: staging_dir.join(CORE_VERSION_FILE),
                backup_core_version: app_dir.join(".core-version.txt.update-backup"),
                staged_core_archive: staging_dir.join("cpa-core").join(&core_archive_name),
                target_core_archive: app_dir.join("cpa-core").join(&core_archive_name),
                install_core_archive: !app_dir.join("cpa-core").join(&core_archive_name).is_file(),
                ack_path: work_dir.join("update-started.ack"),
                work_dir: work_dir.clone(),
                target_version: pending.version.clone(),
            };
            launch_portable_update_helper(app, &helper_path, &work_dir, &descriptor).await
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_dir_all(&work_dir);
        }
        result
    }

    #[cfg(target_os = "macos")]
    {
        validate_portable_update_asset(&pending.asset)?;
        let work_dir = portable_update_work_dir(&pending.version);
        fs::create_dir_all(&work_dir)
            .map_err(|error| format!("Failed to create temporary application update directory: {error}"))?;
        let archive_path = work_dir.join("update.dmg");
        let result = async {
            download_portable_update_archive(
                app,
                pending,
                token,
                &archive_path,
                proxy_url,
                &download_source,
            )
            .await?;
            ensure_portable_update_download(token, &archive_path, pending)?;
            update_app_task(app, |task| {
                task.cancellable = false;
                task.phase = "staging".to_string();
                task.message = Some("Preparing application update".to_string());
            });

            let staged_app = work_dir.join("staging").join("EasyCLIProxyAPI.app");
            stage_macos_application_from_dmg(&archive_path, &work_dir, &staged_app)?;
            validate_macos_staged_application(&staged_app, pending)?;
            let current_exe =
                env::current_exe().map_err(|error| format!("Failed to read current executable path: {error}"))?;
            let current_app = macos_application_bundle_from_executable(&current_exe)?;
            preflight_macos_update_directory(&current_app)?;
            let executable_relative_path = current_exe
                .strip_prefix(&current_app)
                .map_err(|_| "Invalid macOS application path".to_string())?
                .to_path_buf();
            let backup_app = current_app
                .parent()
                .ok_or_else(|| "macOS application path has no parent directory".to_string())?
                .join(".EasyCLIProxyAPI.app.update-backup");
            let descriptor = MacosUpdateDescriptor {
                parent_pid: std::process::id(),
                current_app,
                staged_app,
                backup_app,
                executable_relative_path,
                ack_path: work_dir.join("update-started.ack"),
                work_dir: work_dir.clone(),
                target_version: pending.version.clone(),
            };
            let helper_path = macos_update_helper_path(&current_exe);
            launch_portable_update_helper(app, &helper_path, &work_dir, &descriptor).await
        }
        .await;
        // A busy DMG may still be mounted here. Never recursively clean its mount point.
        if result.is_err() {
            cleanup_macos_update_dmg(&work_dir);
            if !work_dir.join("mount").exists() {
                let _ = fs::remove_dir_all(&work_dir);
            }
        }
        result
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn portable_update_work_dir(version: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "EasyCLIProxyAPI-update-{}-{}-{}",
        version,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_portable_update_download(
    token: &CancellationToken,
    archive_path: &Path,
    pending: &PendingAppUpdate,
) -> Result<(), String> {
    if token.is_cancelled() {
        return Err("Application update download canceled".to_string());
    }
    let actual_sha256 = sha256_file(archive_path)?;
    if actual_sha256 != pending.asset.sha256.trim().to_ascii_lowercase() {
        return Err("Application update package SHA-256 verification failed".to_string());
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_update_helper_path(current_exe: &Path) -> PathBuf {
    current_exe.to_path_buf()
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
async fn launch_portable_update_helper<T: Serialize>(
    app: &tauri::AppHandle,
    helper_path: &Path,
    work_dir: &Path,
    descriptor: &T,
) -> Result<(), String> {
    let descriptor_path = work_dir.join("update-descriptor.json");
    fs::write(
        &descriptor_path,
        serde_json::to_vec_pretty(descriptor)
            .map_err(|error| format!("Failed to serialize application update descriptor: {error}"))?,
    )
    .map_err(|error| format!("Failed to write application update descriptor: {error}"))?;
    let helper_ack_path = portable_update_helper_ack_path(work_dir);
    let _ = fs::remove_file(&helper_ack_path);
    let mut command = Command::new(helper_path);
    command
        .arg("--portable-update-helper")
        .arg(&descriptor_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start application update helper: {error}"))?;
    wait_for_portable_update_helper_start(
        &mut child,
        &helper_ack_path,
        PORTABLE_UPDATE_HELPER_START_TIMEOUT,
    )
    .await?;
    update_app_task(app, |task| {
        task.cancellable = false;
        task.phase = "restarting".to_string();
        task.message = Some("Update is ready. The application will restart shortly".to_string());
    });
    app.exit(0);
    Ok(())
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn portable_update_helper_ack_path(work_dir: &Path) -> PathBuf {
    work_dir.join(PORTABLE_UPDATE_HELPER_ACK_FILE)
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn acknowledge_portable_update_helper_start(work_dir: &Path) -> Result<(), String> {
    fs::write(
        portable_update_helper_ack_path(work_dir),
        std::process::id().to_string(),
    )
    .map_err(|error| format!("Failed to write application update helper startup confirmation: {error}"))
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
async fn wait_for_portable_update_helper_start(
    child: &mut Child,
    ack_path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if ack_path.is_file() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Failed to check application update helper status: {error}"))?
        {
            return Err(format!("Application update helper failed to start; exit status: {status}"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Application update helper did not confirm startup within {} seconds",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) async fn download_portable_update_archive(
    app: &tauri::AppHandle,
    pending: &PendingAppUpdate,
    token: &CancellationToken,
    destination: &Path,
    proxy_url: &str,
    download_source: &VersionDownloadCandidate,
) -> Result<(), String> {
    let client = build_http_client_with_proxy(
        reqwest::Client::builder()
            .redirect(release_https_redirect_policy_with_mirrors(
                &download_source
                    .custom_url
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>(),
            ))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(15 * 60)),
        proxy_url,
        "Failed to create application update download client",
    )?;
    let urls = portable_update_download_urls(&pending.asset, download_source);
    let mut failures = Vec::new();
    for (index, url) in urls.iter().enumerate() {
        update_app_task(app, |task| {
            task.downloaded_bytes = 0;
            task.percent = Some(0.0);
            if index > 0 {
                task.message = Some(format!(
                    "Download failed; switching to {}",
                    update_download_source_name(url)
                ));
            }
        });
        match download_portable_update_archive_url(app, pending, token, destination, &client, url)
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) if token.is_cancelled() => return Err(error),
            Err(error) => failures.push(error),
        }
    }
    Err(format!("All application update download sources failed: {}", failures.join("; ")))
}

pub(crate) fn portable_update_download_urls(
    asset: &PortableUpdateAsset,
    source: &VersionDownloadCandidate,
) -> Vec<String> {
    let mut urls = std::iter::once(asset.url.as_str())
        .chain(asset.fallback_urls.iter().map(String::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if source.source == VersionDownloadSource::Gitcode {
        urls.sort_by_key(|url| update_download_source_name(url) != "GitCode");
    } else if source.proxy_prefix().is_some() {
        if let Some(github_url) = urls
            .iter()
            .find(|url| update_download_source_name(url) == "GitHub")
            .cloned()
        {
            urls.insert(0, version_source_url(source, &github_url));
        }
    }
    urls.dedup();
    urls
}

pub(crate) fn update_download_source_name(url: &str) -> String {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));
    match host.as_deref() {
        Some("gh-proxy.com") => "gh-proxy.com".to_string(),
        Some("ghfast.top") => "ghfast.top".to_string(),
        Some(host) if host == "api.gitcode.com" || host.ends_with(".gitcode.com") => {
            "GitCode".to_string()
        }
        Some(
            "github.com" | "objects.githubusercontent.com" | "release-assets.githubusercontent.com",
        ) => "GitHub".to_string(),
        Some(host) => host.to_string(),
        None => "GitHub".to_string(),
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) async fn download_portable_update_archive_url(
    app: &tauri::AppHandle,
    pending: &PendingAppUpdate,
    token: &CancellationToken,
    destination: &Path,
    client: &reqwest::Client,
    url: &str,
) -> Result<(), String> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("Failed to download application update: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Failed to download application update: {error}"))?;
    let mut stream = response.bytes_stream();
    let mut file =
        File::create(destination).map_err(|error| format!("Failed to create temporary application update file: {error}"))?;
    let mut downloaded = 0_u64;
    let mut progress = crate::progress::ProgressThrottle::default();
    while let Some(chunk) = stream.next().await {
        if token.is_cancelled() {
            return Err("Application update download canceled".to_string());
        }
        let chunk = chunk.map_err(|error| format!("Failed to read application update download data: {error}"))?;
        file.write_all(&chunk)
            .map_err(|error| format!("Failed to write temporary application update file: {error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > pending.asset.size_bytes {
            return Err("Application update package exceeds the size declared in the manifest".to_string());
        }
        if progress.ready(Instant::now(), downloaded == pending.asset.size_bytes) {
            update_app_task(app, |task| {
                task.downloaded_bytes = downloaded;
                task.total_bytes = Some(pending.asset.size_bytes);
                task.percent = Some((downloaded as f64 / pending.asset.size_bytes as f64) * 100.0);
                task.message = Some(format!(
                    "{} / {}",
                    format_byte_count(downloaded),
                    format_byte_count(pending.asset.size_bytes)
                ));
            });
        }
    }
    file.flush()
        .map_err(|error| format!("Failed to save temporary application update file: {error}"))?;
    if downloaded != pending.asset.size_bytes {
        return Err(format!(
            "Application update package size mismatch: expected {}, got {}",
            pending.asset.size_bytes, downloaded
        ));
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn update_app_task<F>(app: &tauri::AppHandle, update: F)
where
    F: FnOnce(&mut AppUpdateTask),
{
    let state = app.state::<AppUpdateState>();
    let task = state.update_task(update);
    let _ = app.emit(APP_UPDATE_PROGRESS_EVENT, task);
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn format_byte_count(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(windows)]
pub(crate) fn extract_portable_update_archive(
    archive_path: &Path,
    staging_dir: &Path,
) -> Result<PortablePackagePayload, String> {
    fs::create_dir_all(staging_dir)
        .map_err(|error| format!("Failed to create application update staging directory: {error}"))?;
    let file = File::open(archive_path).map_err(|error| format!("Failed to open application update package: {error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("Failed to read application update ZIP: {error}"))?;
    let archive_names = archive
        .file_names()
        .map(|name| name.replace('\\', "/"))
        .collect::<Vec<_>>();
    let legacy_layout = archive_names.len() == 2
        && archive_names.iter().any(|name| name == PORTABLE_APP_BINARY)
        && archive_names
            .iter()
            .any(|name| name == PORTABLE_APP_MANIFEST_FILE);
    let mut package_root = None::<String>;
    let mut seen_binary = false;
    let mut seen_manifest = false;
    let mut seen_core_version = false;
    let mut core_archive_name = None::<String>;
    let mut regular_file_count = 0_u32;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read application update ZIP entry: {error}"))?;
        let name = entry.name().replace('\\', "/");
        if entry.enclosed_name().is_none() {
            return Err("Application update package contains an unsafe path".to_string());
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Application update package cannot contain symbolic links".to_string());
        }
        if entry.is_dir() {
            continue;
        }
        regular_file_count = regular_file_count.saturating_add(1);
        let relative;
        let relative_name;
        if legacy_layout {
            relative = Vec::new();
            relative_name = name.clone();
        } else {
            let mut components = name.split('/');
            let root = components
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Application update package is missing a top-level directory".to_string())?;
            relative = components.collect::<Vec<_>>();
            if relative.is_empty() || relative.iter().any(|value| value.is_empty()) {
                return Err("Invalid application update package directory structure".to_string());
            }
            if package_root.as_deref().is_some_and(|value| value != root) {
                return Err("Application update package must contain exactly one top-level directory".to_string());
            }
            package_root.get_or_insert_with(|| root.to_string());
            relative_name = relative.join("/");
        }
        let destination = match relative_name.as_str() {
            PORTABLE_APP_BINARY if !seen_binary => {
                if entry.size() == 0 || entry.size() > 256 * 1024 * 1024 {
                    return Err("Invalid application updater size".to_string());
                }
                seen_binary = true;
                staging_dir.join(PORTABLE_APP_BINARY)
            }
            PORTABLE_APP_MANIFEST_FILE if !seen_manifest => {
                if entry.size() == 0 || entry.size() > 64 * 1024 {
                    return Err("Invalid application update marker size".to_string());
                }
                seen_manifest = true;
                staging_dir.join(PORTABLE_APP_MANIFEST_FILE)
            }
            CORE_VERSION_FILE if !seen_core_version => {
                if entry.size() == 0 || entry.size() > 1024 {
                    return Err("Invalid bundled kernel version file size in application update package".to_string());
                }
                seen_core_version = true;
                staging_dir.join(CORE_VERSION_FILE)
            }
            _ if relative.len() == 2
                && relative[0] == "cpa-core"
                && core_archive_name.is_none() =>
            {
                let filename = relative[1];
                if !filename.starts_with("CLIProxyAPI_")
                    || !filename.ends_with(".zip")
                    || filename.contains("_no-plugin")
                    || entry.size() == 0
                    || entry.size() > 512 * 1024 * 1024
                {
                    return Err("Invalid bundled kernel archive in application update package".to_string());
                }
                core_archive_name = Some(filename.to_string());
                let core_staging_dir = staging_dir.join("cpa-core");
                fs::create_dir_all(&core_staging_dir)
                    .map_err(|error| format!("Failed to create bundled kernel staging directory: {error}"))?;
                core_staging_dir.join(filename)
            }
            _ => return Err(format!("Application update package contains an unknown file: {name}")),
        };
        let mut output = File::create(&destination)
            .map_err(|error| format!("Failed to create application update staging file: {error}"))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Failed to extract application update file: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("Failed to save application update staging file: {error}"))?;
    }
    let required_files_present = if legacy_layout {
        regular_file_count == 2 && seen_binary && seen_manifest
    } else {
        regular_file_count == 4 && seen_binary && seen_manifest && seen_core_version
    };
    if !required_files_present {
        return Err("Application update package is missing required files".to_string());
    }
    let manifest = fs::read_to_string(staging_dir.join(PORTABLE_APP_MANIFEST_FILE))
        .map_err(|error| format!("Failed to read application update marker: {error}"))?;
    let manifest = serde_json::from_str::<PortableAppManifest>(&manifest)
        .map_err(|error| format!("Failed to parse application update marker: {error}"))?;
    if manifest.schema_version != 1
        || manifest.application != "EasyCLIProxyAPI"
        || !manifest.auto_update
    {
        return Err("Invalid application update marker".to_string());
    }
    if !legacy_layout {
        let expected_root = format!(
            "EasyCLIProxyAPI-v{}-Windows-{}",
            manifest.version.trim().trim_start_matches('v'),
            manifest.arch
        );
        if package_root.as_deref() != Some(expected_root.as_str()) {
            return Err("Application update package top-level directory does not match the version or architecture".to_string());
        }
        let core_version = fs::read_to_string(staging_dir.join(CORE_VERSION_FILE))
            .map_err(|error| format!("Failed to read bundled kernel version: {error}"))?;
        let core_version = core_version.trim().trim_start_matches('v');
        if core_version.is_empty()
            || !core_version.split('.').all(|segment| {
                !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err("Invalid bundled kernel version in application update package".to_string());
        }
        let expected_core_archive =
            format!("CLIProxyAPI_{core_version}_windows_{}.zip", manifest.arch);
        if core_archive_name.as_deref() != Some(expected_core_archive.as_str()) {
            return Err("Bundled kernel version or architecture mismatch in application update package".to_string());
        }
    }
    Ok(PortablePackagePayload {
        manifest,
        core_archive_name,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn extract_portable_update_tar_gz(
    archive_path: &Path,
    staging_dir: &Path,
) -> Result<PortablePackagePayload, String> {
    fs::create_dir_all(staging_dir)
        .map_err(|error| format!("Failed to create application update staging directory: {error}"))?;
    let file = File::open(archive_path).map_err(|error| format!("Failed to open application update package: {error}"))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("Failed to read application update TAR.GZ: {error}"))?;
    let mut package_root = None::<String>;
    let mut seen_binary = false;
    let mut seen_manifest = false;
    let mut seen_core_version = false;
    let mut core_archive_name = None::<String>;
    let mut regular_file_count = 0_u32;

    for entry in entries {
        let mut entry = entry.map_err(|error| format!("Failed to read application update entry: {error}"))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err("Application update package cannot contain links or special files".to_string());
        }
        let entry_path = entry
            .path()
            .map_err(|error| format!("Failed to read application update entry path: {error}"))?;
        let mut components = Vec::new();
        for component in entry_path.components() {
            match component {
                Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err("Application update package contains an unsafe path".to_string());
                }
            }
        }
        if components.len() < 2 || components.iter().any(String::is_empty) {
            return Err("Invalid application update package directory structure".to_string());
        }
        let root = &components[0];
        if package_root.as_deref().is_some_and(|value| value != root) {
            return Err("Application update package must contain exactly one top-level directory".to_string());
        }
        package_root.get_or_insert_with(|| root.clone());
        let relative = &components[1..];
        let relative_name = relative.join("/");
        regular_file_count = regular_file_count.saturating_add(1);
        let size = entry.size();
        let destination = match relative_name.as_str() {
            PORTABLE_APP_BINARY if !seen_binary => {
                if size == 0 || size > 256 * 1024 * 1024 {
                    return Err("Invalid application updater size".to_string());
                }
                seen_binary = true;
                staging_dir.join(PORTABLE_APP_BINARY)
            }
            PORTABLE_APP_MANIFEST_FILE if !seen_manifest => {
                if size == 0 || size > 64 * 1024 {
                    return Err("Invalid application update marker size".to_string());
                }
                seen_manifest = true;
                staging_dir.join(PORTABLE_APP_MANIFEST_FILE)
            }
            CORE_VERSION_FILE if !seen_core_version => {
                if size == 0 || size > 1024 {
                    return Err("Invalid bundled kernel version file size in application update package".to_string());
                }
                seen_core_version = true;
                staging_dir.join(CORE_VERSION_FILE)
            }
            _ if relative.len() == 2
                && relative[0] == "cpa-core"
                && core_archive_name.is_none() =>
            {
                let filename = &relative[1];
                if !filename.starts_with("CLIProxyAPI_")
                    || !filename.ends_with(".tar.gz")
                    || filename.contains("_no-plugin")
                    || size == 0
                    || size > 512 * 1024 * 1024
                {
                    return Err("Invalid bundled kernel archive in application update package".to_string());
                }
                core_archive_name = Some(filename.clone());
                let core_staging_dir = staging_dir.join("cpa-core");
                fs::create_dir_all(&core_staging_dir)
                    .map_err(|error| format!("Failed to create bundled kernel staging directory: {error}"))?;
                core_staging_dir.join(filename)
            }
            _ => return Err(format!("Application update package contains an unknown file: {relative_name}")),
        };
        let mut output = File::create(&destination)
            .map_err(|error| format!("Failed to create application update staging file: {error}"))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Failed to extract application update file: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("Failed to save application update staging file: {error}"))?;
    }
    if regular_file_count != 4
        || !seen_binary
        || !seen_manifest
        || !seen_core_version
        || core_archive_name.is_none()
    {
        return Err("Application update package is missing required files".to_string());
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        staging_dir.join(PORTABLE_APP_BINARY),
        fs::Permissions::from_mode(0o755),
    )
    .map_err(|error| format!("Failed to set application updater permissions: {error}"))?;
    let manifest = fs::read_to_string(staging_dir.join(PORTABLE_APP_MANIFEST_FILE))
        .map_err(|error| format!("Failed to read application update marker: {error}"))?;
    let manifest = serde_json::from_str::<PortableAppManifest>(&manifest)
        .map_err(|error| format!("Failed to parse application update marker: {error}"))?;
    if manifest.schema_version != 1
        || manifest.application != "EasyCLIProxyAPI"
        || manifest.platform != "linux"
        || !manifest.auto_update
    {
        return Err("Invalid application update marker".to_string());
    }
    let expected_root = format!(
        "EasyCLIProxyAPI-v{}-Linux-{}",
        manifest.version.trim().trim_start_matches('v'),
        manifest.arch
    );
    if package_root.as_deref() != Some(expected_root.as_str()) {
        return Err("Application update package top-level directory does not match the version or architecture".to_string());
    }
    let core_version = fs::read_to_string(staging_dir.join(CORE_VERSION_FILE))
        .map_err(|error| format!("Failed to read bundled kernel version: {error}"))?;
    let core_version = core_version.trim().trim_start_matches('v');
    if core_version.is_empty()
        || !core_version
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Invalid bundled kernel version in application update package".to_string());
    }
    let expected_core_archive =
        format!("CLIProxyAPI_{core_version}_linux_{}.tar.gz", manifest.arch);
    if core_archive_name.as_deref() != Some(expected_core_archive.as_str()) {
        return Err("Bundled kernel version or architecture mismatch in application update package".to_string());
    }
    Ok(PortablePackagePayload {
        manifest,
        core_archive_name,
    })
}

#[cfg(windows)]
pub(crate) fn stage_current_portable_core_payload(
    app_dir: &Path,
    staging_dir: &Path,
    arch: &str,
) -> Result<String, String> {
    let current_core_version = app_dir.join(CORE_VERSION_FILE);
    let core_version = fs::read_to_string(&current_core_version)
        .map_err(|error| format!("Failed to read current bundled kernel version: {error}"))?;
    let core_version = core_version.trim().trim_start_matches('v');
    if core_version.is_empty()
        || !core_version
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Current bundled kernel version is invalid and cannot support legacy update packages".to_string());
    }
    let core_archive_name = format!("CLIProxyAPI_{core_version}_windows_{arch}.zip");
    let current_core_archive = app_dir.join("cpa-core").join(&core_archive_name);
    if !current_core_archive.is_file() {
        return Err(format!(
            "Current portable version is missing bundled kernel archive {core_archive_name} and cannot support legacy update packages"
        ));
    }
    let staged_core_dir = staging_dir.join("cpa-core");
    fs::create_dir_all(&staged_core_dir)
        .map_err(|error| format!("Failed to create legacy update compatibility staging directory: {error}"))?;
    fs::copy(&current_core_version, staging_dir.join(CORE_VERSION_FILE))
        .map_err(|error| format!("Failed to stage current kernel version file: {error}"))?;
    fs::copy(
        current_core_archive,
        staged_core_dir.join(&core_archive_name),
    )
    .map_err(|error| format!("Failed to stage current bundled kernel archive: {error}"))?;
    Ok(core_archive_name)
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn preflight_portable_update_directory(app_dir: &Path) -> Result<(), String> {
    if !app_dir.join(CORE_VERSION_FILE).is_file() || !app_dir.join("cpa-core").is_dir() {
        return Err("The current portable directory is incomplete. Download the latest full package to upgrade".to_string());
    }
    let probe = app_dir.join(format!(
        ".easycliproxy-update-write-test-{}",
        std::process::id()
    ));
    fs::write(&probe, b"update-write-test")
        .map_err(|error| format!("Application directory is not writable; automatic upgrade is unavailable: {error}"))?;
    fs::remove_file(&probe).map_err(|error| format!("Failed to clean up application update write test: {error}"))?;
    let core_probe = app_dir.join("cpa-core").join(format!(
        ".easycliproxy-update-write-test-{}",
        std::process::id()
    ));
    fs::write(&core_probe, b"update-write-test")
        .map_err(|error| format!("Bundled kernel directory is not writable; automatic upgrade is unavailable: {error}"))?;
    fs::remove_file(&core_probe)
        .map_err(|error| format!("Failed to clean up bundled kernel update write test: {error}"))?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn wait_for_windows_process_exit(
    process_id: u32,
    timeout: Duration,
) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_FAILED, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };

    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    let handle = unsafe { OpenProcess(SYNCHRONIZE_ACCESS, 0, process_id) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(87) {
            return Ok(());
        }
        return Err(format!("Failed to open legacy application process: {error}"));
    }
    let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let result = unsafe { WaitForSingleObject(handle, timeout_ms) };
    unsafe { CloseHandle(handle) };
    if result == WAIT_TIMEOUT {
        return Err("Timed out waiting for legacy application to exit".to_string());
    }
    if result == WAIT_FAILED {
        return Err(format!(
            "Failed while waiting for legacy application to exit: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn wait_for_portable_parent_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    wait_for_windows_process_exit(pid, timeout)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn wait_for_portable_parent_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err("Invalid application update parent process ID".to_string());
    }
    let deadline = Instant::now() + timeout;
    loop {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            if error.raw_os_error() != Some(libc::EPERM) {
                return Err(format!("Failed to check legacy application process: {error}"));
            }
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for legacy application to exit".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn validate_portable_update_descriptor(
    descriptor_path: &Path,
    descriptor: &PortableUpdateDescriptor,
) -> Result<(), String> {
    if descriptor.parent_pid == 0
        || semver::Version::parse(descriptor.target_version.trim().trim_start_matches('v')).is_err()
    {
        return Err("Invalid application update descriptor".to_string());
    }
    let app_dir = descriptor
        .current_exe
        .parent()
        .ok_or_else(|| "Invalid application update target path".to_string())?;
    if !descriptor.current_exe.is_absolute()
        || descriptor.current_exe != app_dir.join(PORTABLE_APP_BINARY)
        || descriptor.current_manifest != app_dir.join(PORTABLE_APP_MANIFEST_FILE)
        || descriptor.backup_exe != app_dir.join(portable_update_backup_executable_name())
        || descriptor.backup_manifest != app_dir.join(".portable-app.json.update-backup")
        || descriptor.current_core_version != app_dir.join(CORE_VERSION_FILE)
        || descriptor.backup_core_version != app_dir.join(".core-version.txt.update-backup")
    {
        return Err("Invalid application update target path".to_string());
    }
    let core_archive_name = descriptor
        .staged_core_archive
        .file_name()
        .ok_or_else(|| "Invalid application update bundled kernel filename".to_string())?;
    if descriptor.target_core_archive != app_dir.join("cpa-core").join(core_archive_name)
        || !core_archive_name
            .to_string_lossy()
            .starts_with("CLIProxyAPI_")
        || !core_archive_name
            .to_string_lossy()
            .ends_with(portable_update_core_archive_suffix())
        || descriptor.install_core_archive == descriptor.target_core_archive.is_file()
    {
        return Err("Invalid application update bundled kernel target path".to_string());
    }
    let canonical_work_dir = fs::canonicalize(&descriptor.work_dir)
        .map_err(|error| format!("Failed to read application update temporary directory: {error}"))?;
    let canonical_temp_dir = fs::canonicalize(env::temp_dir())
        .map_err(|error| format!("Failed to read system temporary directory: {error}"))?;
    if !canonical_work_dir.starts_with(&canonical_temp_dir)
        || !canonical_work_dir
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("EasyCLIProxyAPI-update-"))
    {
        return Err("Invalid application update working directory".to_string());
    }
    let canonical_descriptor = fs::canonicalize(descriptor_path)
        .map_err(|error| format!("Failed to read application update descriptor path: {error}"))?;
    if canonical_descriptor.parent() != Some(canonical_work_dir.as_path())
        || canonical_descriptor
            .file_name()
            .and_then(|value| value.to_str())
            != Some("update-descriptor.json")
        || descriptor.ack_path != descriptor.work_dir.join("update-started.ack")
    {
        return Err("Application update descriptor path is out of bounds".to_string());
    }
    for staged in [
        &descriptor.staged_exe,
        &descriptor.staged_manifest,
        &descriptor.staged_core_version,
        &descriptor.staged_core_archive,
    ] {
        let canonical = fs::canonicalize(staged)
            .map_err(|error| format!("Failed to read application update staging file: {error}"))?;
        if !canonical.starts_with(&canonical_work_dir) {
            return Err("Application update staging path is out of bounds".to_string());
        }
    }
    if descriptor.staged_exe
        != descriptor
            .work_dir
            .join("staging")
            .join(PORTABLE_APP_BINARY)
        || descriptor.staged_manifest
            != descriptor
                .work_dir
                .join("staging")
                .join(PORTABLE_APP_MANIFEST_FILE)
        || descriptor.staged_core_version
            != descriptor.work_dir.join("staging").join(CORE_VERSION_FILE)
        || descriptor.staged_core_archive
            != descriptor
                .work_dir
                .join("staging")
                .join("cpa-core")
                .join(core_archive_name)
    {
        return Err("Invalid application update staging path".to_string());
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn portable_update_backup_executable_name() -> &'static str {
    if cfg!(windows) {
        ".EasyCLIProxyAPI.exe.update-backup"
    } else {
        ".EasyCLIProxyAPI.update-backup"
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn portable_update_replacement_executable_name() -> &'static str {
    if cfg!(windows) {
        ".EasyCLIProxyAPI.exe.update-new"
    } else {
        ".EasyCLIProxyAPI.update-new"
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn portable_update_core_archive_suffix() -> &'static str {
    if cfg!(windows) {
        ".zip"
    } else {
        ".tar.gz"
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn restore_portable_update_backup(
    descriptor: &PortableUpdateDescriptor,
) -> Result<(), String> {
    if !descriptor.backup_exe.is_file()
        || !descriptor.backup_manifest.is_file()
        || !descriptor.backup_core_version.is_file()
    {
        return Err("Application update backup is incomplete; cannot roll back".to_string());
    }
    if descriptor.current_exe.exists() {
        fs::remove_file(&descriptor.current_exe)
            .map_err(|error| format!("Failed to remove new application version: {error}"))?;
    }
    fs::rename(&descriptor.backup_exe, &descriptor.current_exe)
        .map_err(|error| format!("Failed to restore old application version: {error}"))?;
    if descriptor.current_manifest.exists() {
        fs::remove_file(&descriptor.current_manifest)
            .map_err(|error| format!("Failed to remove new portable version marker: {error}"))?;
    }
    fs::rename(&descriptor.backup_manifest, &descriptor.current_manifest)
        .map_err(|error| format!("Failed to restore old portable version marker: {error}"))?;
    if descriptor.current_core_version.exists() {
        fs::remove_file(&descriptor.current_core_version)
            .map_err(|error| format!("Failed to remove new kernel version file: {error}"))?;
    }
    fs::rename(
        &descriptor.backup_core_version,
        &descriptor.current_core_version,
    )
    .map_err(|error| format!("Failed to restore old kernel version file: {error}"))?;
    if descriptor.install_core_archive && descriptor.target_core_archive.exists() {
        fs::remove_file(&descriptor.target_core_archive)
            .map_err(|error| format!("Failed to remove new bundled kernel archive: {error}"))?;
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn replace_portable_update_files(
    descriptor: &PortableUpdateDescriptor,
) -> Result<(), String> {
    let app_dir = descriptor
        .current_exe
        .parent()
        .ok_or_else(|| "Invalid application update target path".to_string())?;
    let replacement_exe = app_dir.join(portable_update_replacement_executable_name());
    let replacement_manifest = app_dir.join(".portable-app.json.update-new");
    let replacement_core_version = app_dir.join(".core-version.txt.update-new");
    let replacement_core_archive = app_dir.join(".cpa-core-archive.update-new");
    let _ = fs::remove_file(&replacement_exe);
    let _ = fs::remove_file(&replacement_manifest);
    let _ = fs::remove_file(&replacement_core_version);
    let _ = fs::remove_file(&replacement_core_archive);
    fs::copy(&descriptor.staged_exe, &replacement_exe)
        .map_err(|error| format!("Failed to prepare new application executable: {error}"))?;
    if let Err(error) = fs::copy(&descriptor.staged_manifest, &replacement_manifest) {
        let _ = fs::remove_file(&replacement_exe);
        return Err(format!("Failed to prepare new portable version marker: {error}"));
    }
    if let Err(error) = fs::copy(&descriptor.staged_core_version, &replacement_core_version) {
        let _ = fs::remove_file(&replacement_exe);
        let _ = fs::remove_file(&replacement_manifest);
        return Err(format!("Failed to prepare new kernel version file: {error}"));
    }
    if descriptor.install_core_archive {
        if let Err(error) = fs::copy(&descriptor.staged_core_archive, &replacement_core_archive) {
            let _ = fs::remove_file(&replacement_exe);
            let _ = fs::remove_file(&replacement_manifest);
            let _ = fs::remove_file(&replacement_core_version);
            return Err(format!("Failed to prepare new bundled kernel archive: {error}"));
        }
    }

    let _ = fs::remove_file(&descriptor.backup_exe);
    let _ = fs::remove_file(&descriptor.backup_manifest);
    let _ = fs::remove_file(&descriptor.backup_core_version);
    if let Err(error) = fs::rename(&descriptor.current_exe, &descriptor.backup_exe) {
        let _ = fs::remove_file(&replacement_exe);
        let _ = fs::remove_file(&replacement_manifest);
        let _ = fs::remove_file(&replacement_core_version);
        let _ = fs::remove_file(&replacement_core_archive);
        return Err(format!("Failed to back up old application version: {error}"));
    }
    if let Err(error) = fs::rename(&descriptor.current_manifest, &descriptor.backup_manifest) {
        let _ = fs::rename(&descriptor.backup_exe, &descriptor.current_exe);
        let _ = fs::remove_file(&replacement_exe);
        let _ = fs::remove_file(&replacement_manifest);
        let _ = fs::remove_file(&replacement_core_version);
        let _ = fs::remove_file(&replacement_core_archive);
        return Err(format!("Failed to back up portable version marker: {error}"));
    }
    if let Err(error) = fs::rename(
        &descriptor.current_core_version,
        &descriptor.backup_core_version,
    ) {
        let _ = fs::rename(&descriptor.backup_manifest, &descriptor.current_manifest);
        let _ = fs::rename(&descriptor.backup_exe, &descriptor.current_exe);
        let _ = fs::remove_file(&replacement_exe);
        let _ = fs::remove_file(&replacement_manifest);
        let _ = fs::remove_file(&replacement_core_version);
        let _ = fs::remove_file(&replacement_core_archive);
        return Err(format!("Failed to back up old kernel version file: {error}"));
    }

    let replace_result = (|| -> Result<(), String> {
        fs::rename(&replacement_exe, &descriptor.current_exe)
            .map_err(|error| format!("Failed to replace application executable: {error}"))?;
        fs::rename(&replacement_manifest, &descriptor.current_manifest)
            .map_err(|error| format!("Failed to replace portable version marker: {error}"))?;
        fs::rename(&replacement_core_version, &descriptor.current_core_version)
            .map_err(|error| format!("Failed to replace kernel version file: {error}"))?;
        if descriptor.install_core_archive {
            fs::rename(&replacement_core_archive, &descriptor.target_core_archive)
                .map_err(|error| format!("Failed to install new bundled kernel archive: {error}"))?;
        }
        Ok(())
    })();
    if let Err(error) = replace_result {
        let _ = fs::remove_file(&replacement_exe);
        let _ = fs::remove_file(&replacement_manifest);
        let _ = fs::remove_file(&replacement_core_version);
        let _ = fs::remove_file(&replacement_core_archive);
        restore_portable_update_backup(descriptor)
            .map_err(|rollback| format!("{error}；{rollback}"))?;
        return Err(error);
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn cleanup_superseded_bundled_core_archives(descriptor: &PortableUpdateDescriptor) {
    let Some(core_dir) = descriptor.target_core_archive.parent() else {
        return;
    };
    let Some(current_name) = descriptor
        .target_core_archive
        .file_name()
        .and_then(|value| value.to_str())
    else {
        return;
    };
    let Ok(entries) = fs::read_dir(core_dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_file()
            && name != current_name
            && name.starts_with("CLIProxyAPI_")
            && name.ends_with(portable_update_core_archive_suffix())
            && !name.contains("_no-plugin")
        {
            if let Err(error) = fs::remove_file(&path) {
                eprintln!(
                    "Failed to clean up old bundled kernel archive {}: {error}",
                    path_to_string(&path)
                );
            }
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn cleanup_portable_update_payload(
    descriptor_path: &Path,
    descriptor: &PortableUpdateDescriptor,
) {
    let _ = fs::remove_file(descriptor.work_dir.join(if cfg!(windows) {
        "update.zip"
    } else {
        "update.tar.gz"
    }));
    let _ = fs::remove_dir_all(descriptor.work_dir.join("staging"));
    let _ = fs::remove_file(&descriptor.ack_path);
    let _ = fs::remove_file(portable_update_helper_ack_path(&descriptor.work_dir));
    let _ = fs::remove_file(descriptor_path);
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn run_portable_update_helper(descriptor_path: &Path) -> Result<(), String> {
    let descriptor = serde_json::from_slice::<PortableUpdateDescriptor>(
        &fs::read(descriptor_path).map_err(|error| format!("Failed to read application update descriptor: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse application update descriptor: {error}"))?;
    validate_portable_update_descriptor(descriptor_path, &descriptor)?;
    acknowledge_portable_update_helper_start(&descriptor.work_dir)?;
    if let Err(error) =
        wait_for_portable_parent_exit(descriptor.parent_pid, Duration::from_secs(120))
    {
        cleanup_portable_update_payload(descriptor_path, &descriptor);
        return Err(error);
    }

    if let Err(error) = replace_portable_update_files(&descriptor) {
        let mut rollback = Command::new(&descriptor.current_exe);
        configure_background_command(&mut rollback);
        let _ = rollback.spawn();
        cleanup_portable_update_payload(descriptor_path, &descriptor);
        return Err(error);
    }

    let _ = fs::remove_file(&descriptor.ack_path);
    let mut command = Command::new(&descriptor.current_exe);
    command
        .arg("--portable-update-ack")
        .arg(&descriptor.ack_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            restore_portable_update_backup(&descriptor)?;
            let mut rollback = Command::new(&descriptor.current_exe);
            configure_background_command(&mut rollback);
            let _ = rollback.spawn();
            cleanup_portable_update_payload(descriptor_path, &descriptor);
            return Err(format!("Failed to start new application version: {error}"));
        }
    };

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut confirmed = false;
    while Instant::now() < deadline {
        if descriptor.ack_path.is_file() {
            confirmed = true;
            break;
        }
        if child
            .try_wait()
            .map_err(|error| format!("Failed to check new application version status: {error}"))?
            .is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    if confirmed {
        let _ = fs::remove_file(&descriptor.backup_exe);
        let _ = fs::remove_file(&descriptor.backup_manifest);
        let _ = fs::remove_file(&descriptor.backup_core_version);
        cleanup_superseded_bundled_core_archives(&descriptor);
        cleanup_portable_update_payload(descriptor_path, &descriptor);
        return Ok(());
    }

    let _ = child.kill();
    let _ = child.wait();
    restore_portable_update_backup(&descriptor)?;
    let mut rollback = Command::new(&descriptor.current_exe);
    configure_background_command(&mut rollback);
    let _ = rollback.spawn();
    cleanup_portable_update_payload(descriptor_path, &descriptor);
    Err(format!(
        "New application version {} did not confirm startup within 60 seconds; rolled back",
        descriptor.target_version
    ))
}

#[cfg(target_os = "macos")]
fn run_macos_update_command(command: &mut Command, action: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("Failed to {action}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!("Failed to {action} ({}): {details}", output.status))
}

#[cfg(target_os = "macos")]
pub(crate) fn stage_macos_application_from_dmg(
    dmg_path: &Path,
    work_dir: &Path,
    staged_app: &Path,
) -> Result<(), String> {
    stage_macos_application_from_dmg_with_commands(
        dmg_path,
        work_dir,
        staged_app,
        run_macos_update_command,
        thread::sleep,
    )
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn stage_macos_application_from_dmg_with_commands(
    dmg_path: &Path,
    work_dir: &Path,
    staged_app: &Path,
    mut run: impl FnMut(&mut Command, &str) -> Result<(), String>,
    mut wait: impl FnMut(Duration),
) -> Result<(), String> {
    let mount_dir = work_dir.join("mount");
    fs::create_dir_all(&mount_dir).map_err(|error| format!("Failed to create DMG mount directory: {error}"))?;
    let mut attach = Command::new("hdiutil");
    attach
        .arg("attach")
        .arg(dmg_path)
        .args(["-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount_dir);
    run(&mut attach, "Mount application update DMG")?;

    let source_app = mount_dir.join("EasyCLIProxyAPI.app");
    let stage_result = (|| -> Result<(), String> {
        if !source_app.is_dir() {
            return Err("Application update DMG is missing EasyCLIProxyAPI.app".to_string());
        }
        let staging_parent = staged_app
            .parent()
            .ok_or_else(|| "Invalid application update staging path".to_string())?;
        fs::create_dir_all(staging_parent)
            .map_err(|error| format!("Failed to create application update staging directory: {error}"))?;
        let mut ditto = Command::new("ditto");
        ditto.arg(&source_app).arg(staged_app);
        run(&mut ditto, "Stage new macOS application")?;
        let mut codesign = Command::new("codesign");
        codesign
            .args(["--verify", "--deep", "--strict"])
            .arg(staged_app);
        run(&mut codesign, "Verify new macOS application signature")?;
        let mut gatekeeper = Command::new("spctl");
        gatekeeper
            .args(["--assess", "--type", "execute", "--verbose=2"])
            .arg(staged_app);
        run(&mut gatekeeper, "Verify new macOS application system trust status")
    })();

    match eject_macos_update_dmg_with_commands(&mount_dir, &mut run, &mut wait) {
        Ok(()) => {
            let _ = fs::remove_dir(&mount_dir);
        }
        Err(error) => {
            // A successfully staged app no longer depends on the read-only DMG.
            // Preserve staging errors, but never fail an update just for cleanup.
            eprintln!("Application update DMG cleanup deferred: {error}");
        }
    }
    stage_result
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn eject_macos_update_dmg_with_commands(
    mount_dir: &Path,
    mut run: impl FnMut(&mut Command, &str) -> Result<(), String>,
    mut wait: impl FnMut(Duration),
) -> Result<(), String> {
    let mut last_error = String::new();
    // Spotlight or security scanning can briefly hold the newly mounted image busy.
    for attempt in 0..3 {
        if attempt > 0 {
            wait(Duration::from_millis(500));
        }
        let mut eject = Command::new("diskutil");
        eject.arg("eject").arg(mount_dir);
        match run(&mut eject, "Eject application update DMG") {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }

    // Compatibility fallback, limited to this updater's private read-only mount.
    let mut detach = Command::new("hdiutil");
    detach.args(["detach", "-force"]).arg(mount_dir);
    run(&mut detach, "Force unmount application update DMG")
        .map_err(|error| format!("{last_error}; {error}"))
}

#[cfg(target_os = "macos")]
fn cleanup_macos_update_dmg(work_dir: &Path) {
    cleanup_macos_update_dmg_with_commands(work_dir, run_macos_update_command, thread::sleep);
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn cleanup_macos_update_dmg_with_commands(
    work_dir: &Path,
    run: impl FnMut(&mut Command, &str) -> Result<(), String>,
    wait: impl FnMut(Duration),
) {
    let mount_dir = work_dir.join("mount");
    if mount_dir.exists() {
        if let Err(error) = eject_macos_update_dmg_with_commands(&mount_dir, run, wait) {
            eprintln!("Application update DMG cleanup deferred: {error}");
            return;
        }
        let _ = fs::remove_dir(&mount_dir);
    }
    let _ = fs::remove_file(work_dir.join("update.dmg"));
}

#[cfg(target_os = "macos")]
pub(crate) fn validate_macos_staged_application(
    staged_app: &Path,
    pending: &PendingAppUpdate,
) -> Result<(), String> {
    let manifest_path = staged_app
        .join("Contents")
        .join("Resources")
        .join(PORTABLE_APP_MANIFEST_FILE);
    let contents = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("Failed to read new macOS automatic update marker: {error}"))?;
    let manifest = serde_json::from_str::<PortableAppManifest>(&contents)
        .map_err(|error| format!("Failed to parse new macOS automatic update marker: {error}"))?;
    if manifest.schema_version != 1
        || manifest.application != "EasyCLIProxyAPI"
        || manifest.platform != "darwin"
        || manifest.arch != pending.arch
        || !manifest.auto_update
        || normalize_version(&manifest.version) != normalize_version(&pending.version)
    {
        return Err("New macOS application marker does not match the update target".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_application_bundle_from_executable(
    executable: &Path,
) -> Result<PathBuf, String> {
    let macos_dir = executable
        .parent()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("MacOS"))
        .ok_or_else(|| "Current executable is not inside a standard macOS application bundle".to_string())?;
    let contents_dir = macos_dir
        .parent()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("Contents"))
        .ok_or_else(|| "Current executable is not inside a standard macOS application bundle".to_string())?;
    let app = contents_dir
        .parent()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        .ok_or_else(|| "Current executable is not inside a standard macOS application bundle".to_string())?;
    Ok(app.to_path_buf())
}

#[cfg(target_os = "macos")]
pub(crate) fn preflight_macos_update_directory(current_app: &Path) -> Result<(), String> {
    if !current_app.is_dir() {
        return Err("Current macOS application bundle does not exist".to_string());
    }
    let app_parent = current_app
        .parent()
        .ok_or_else(|| "macOS application bundle has no parent directory".to_string())?;
    let probe = app_parent.join(format!(
        ".easycliproxy-update-write-test-{}",
        std::process::id()
    ));
    fs::write(&probe, b"update-write-test")
        .map_err(|error| format!("macOS application directory is not writable; automatic upgrade is unavailable: {error}"))?;
    fs::remove_file(&probe).map_err(|error| format!("Failed to clean up application update write test: {error}"))
}

#[cfg(target_os = "macos")]
pub(crate) fn validate_macos_update_descriptor(
    descriptor_path: &Path,
    descriptor: &MacosUpdateDescriptor,
) -> Result<(), String> {
    if descriptor.parent_pid == 0
        || semver::Version::parse(descriptor.target_version.trim().trim_start_matches('v')).is_err()
        || !descriptor.current_app.is_absolute()
        || descriptor
            .current_app
            .extension()
            .and_then(|value| value.to_str())
            != Some("app")
    {
        return Err("Invalid macOS application update descriptor".to_string());
    }
    let app_parent = descriptor
        .current_app
        .parent()
        .ok_or_else(|| "Invalid macOS application update target path".to_string())?;
    if descriptor.backup_app != app_parent.join(".EasyCLIProxyAPI.app.update-backup")
        || descriptor.executable_relative_path.is_absolute()
        || descriptor
            .executable_relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !descriptor
            .executable_relative_path
            .starts_with(Path::new("Contents").join("MacOS"))
    {
        return Err("Invalid macOS application update target path".to_string());
    }
    let canonical_work_dir = fs::canonicalize(&descriptor.work_dir)
        .map_err(|error| format!("Failed to read application update temporary directory: {error}"))?;
    let canonical_temp_dir = fs::canonicalize(env::temp_dir())
        .map_err(|error| format!("Failed to read system temporary directory: {error}"))?;
    if !canonical_work_dir.starts_with(&canonical_temp_dir)
        || !canonical_work_dir
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("EasyCLIProxyAPI-update-"))
    {
        return Err("Invalid application update working directory".to_string());
    }
    let canonical_descriptor = fs::canonicalize(descriptor_path)
        .map_err(|error| format!("Failed to read application update descriptor path: {error}"))?;
    let canonical_staged_app = fs::canonicalize(&descriptor.staged_app)
        .map_err(|error| format!("Failed to read new macOS application path: {error}"))?;
    if canonical_descriptor.parent() != Some(canonical_work_dir.as_path())
        || canonical_descriptor
            .file_name()
            .and_then(|value| value.to_str())
            != Some("update-descriptor.json")
        || descriptor.staged_app
            != descriptor
                .work_dir
                .join("staging")
                .join("EasyCLIProxyAPI.app")
        || !canonical_staged_app.starts_with(&canonical_work_dir)
        || descriptor.ack_path != descriptor.work_dir.join("update-started.ack")
    {
        return Err("macOS application update staging path is out of bounds".to_string());
    }
    if !descriptor
        .current_app
        .join(&descriptor.executable_relative_path)
        .is_file()
        || !descriptor
            .staged_app
            .join(&descriptor.executable_relative_path)
            .is_file()
    {
        return Err("macOS application update executable is missing".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn restore_macos_update_backup(
    descriptor: &MacosUpdateDescriptor,
) -> Result<(), String> {
    if !descriptor.backup_app.is_dir() {
        return Err("macOS application update backup is incomplete; cannot roll back".to_string());
    }
    if descriptor.current_app.exists() {
        fs::remove_dir_all(&descriptor.current_app)
            .map_err(|error| format!("Failed to remove new macOS application: {error}"))?;
    }
    fs::rename(&descriptor.backup_app, &descriptor.current_app)
        .map_err(|error| format!("Failed to restore old macOS application: {error}"))
}

#[cfg(target_os = "macos")]
pub(crate) fn replace_macos_application(descriptor: &MacosUpdateDescriptor) -> Result<(), String> {
    let app_parent = descriptor
        .current_app
        .parent()
        .ok_or_else(|| "Invalid macOS application update target path".to_string())?;
    let replacement_app = app_parent.join(".EasyCLIProxyAPI-update-new.app");
    let legacy_replacement_app = app_parent.join(".EasyCLIProxyAPI.app.update-new");
    if legacy_replacement_app.exists() {
        fs::remove_dir_all(&legacy_replacement_app)
            .map_err(|error| format!("Failed to clean up legacy macOS update staging data: {error}"))?;
    }
    if replacement_app.exists() {
        fs::remove_dir_all(&replacement_app)
            .map_err(|error| format!("Failed to clean up old macOS update staging data: {error}"))?;
    }
    let mut ditto = Command::new("ditto");
    ditto.arg(&descriptor.staged_app).arg(&replacement_app);
    run_macos_update_command(&mut ditto, "Prepare new macOS application")?;
    let mut codesign = Command::new("codesign");
    codesign
        .args(["--verify", "--deep", "--strict"])
        .arg(&replacement_app);
    if let Err(error) = run_macos_update_command(&mut codesign, "Verify replacement macOS application signature")
    {
        let _ = fs::remove_dir_all(&replacement_app);
        return Err(error);
    }
    let mut gatekeeper = Command::new("spctl");
    gatekeeper
        .args(["--assess", "--type", "execute", "--verbose=2"])
        .arg(&replacement_app);
    if let Err(error) =
        run_macos_update_command(&mut gatekeeper, "Verify replacement macOS application system trust status")
    {
        let _ = fs::remove_dir_all(&replacement_app);
        return Err(error);
    }
    if descriptor.backup_app.exists() {
        fs::remove_dir_all(&descriptor.backup_app)
            .map_err(|error| format!("Failed to clean up old macOS application backup: {error}"))?;
    }
    fs::rename(&descriptor.current_app, &descriptor.backup_app)
        .map_err(|error| format!("Failed to back up old macOS application: {error}"))?;
    if let Err(error) = fs::rename(&replacement_app, &descriptor.current_app) {
        let _ = fs::rename(&descriptor.backup_app, &descriptor.current_app);
        let _ = fs::remove_dir_all(&replacement_app);
        return Err(format!("Failed to replace macOS application: {error}"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn cleanup_macos_update_payload(descriptor_path: &Path, descriptor: &MacosUpdateDescriptor) {
    cleanup_macos_update_dmg(&descriptor.work_dir);
    let _ = fs::remove_dir_all(descriptor.work_dir.join("staging"));
    let _ = fs::remove_file(&descriptor.ack_path);
    let _ = fs::remove_file(portable_update_helper_ack_path(&descriptor.work_dir));
    let _ = fs::remove_file(descriptor_path);
}

#[cfg(target_os = "macos")]
pub(crate) fn run_portable_update_helper(descriptor_path: &Path) -> Result<(), String> {
    let descriptor = serde_json::from_slice::<MacosUpdateDescriptor>(
        &fs::read(descriptor_path).map_err(|error| format!("Failed to read application update descriptor: {error}"))?,
    )
    .map_err(|error| format!("Failed to parse application update descriptor: {error}"))?;
    validate_macos_update_descriptor(descriptor_path, &descriptor)?;
    acknowledge_portable_update_helper_start(&descriptor.work_dir)?;
    if let Err(error) =
        wait_for_portable_parent_exit(descriptor.parent_pid, Duration::from_secs(120))
    {
        cleanup_macos_update_payload(descriptor_path, &descriptor);
        return Err(error);
    }
    if let Err(error) = replace_macos_application(&descriptor) {
        let rollback_exe = descriptor
            .current_app
            .join(&descriptor.executable_relative_path);
        let _ = Command::new(rollback_exe).spawn();
        cleanup_macos_update_payload(descriptor_path, &descriptor);
        return Err(error);
    }

    let current_exe = descriptor
        .current_app
        .join(&descriptor.executable_relative_path);
    let _ = fs::remove_file(&descriptor.ack_path);
    let mut command = Command::new(&current_exe);
    command
        .arg("--portable-update-ack")
        .arg(&descriptor.ack_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            restore_macos_update_backup(&descriptor)?;
            let rollback_exe = descriptor
                .current_app
                .join(&descriptor.executable_relative_path);
            let _ = Command::new(rollback_exe).spawn();
            cleanup_macos_update_payload(descriptor_path, &descriptor);
            return Err(format!("Failed to start new macOS application: {error}"));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if descriptor.ack_path.is_file() {
            let _ = fs::remove_dir_all(&descriptor.backup_app);
            cleanup_macos_update_payload(descriptor_path, &descriptor);
            return Ok(());
        }
        if child
            .try_wait()
            .map_err(|error| format!("Failed to check new macOS application status: {error}"))?
            .is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    restore_macos_update_backup(&descriptor)?;
    let rollback_exe = descriptor
        .current_app
        .join(&descriptor.executable_relative_path);
    let _ = Command::new(rollback_exe).spawn();
    cleanup_macos_update_payload(descriptor_path, &descriptor);
    Err(format!(
        "New application version {} did not confirm startup within 60 seconds; rolled back",
        descriptor.target_version
    ))
}

pub(crate) fn portable_update_ack_argument() -> Option<PathBuf> {
    let mut args = env::args_os();
    while let Some(argument) = args.next() {
        if argument == "--portable-update-ack" {
            let path = PathBuf::from(args.next()?);
            let parent = path.parent()?;
            let valid_name =
                path.file_name().and_then(|value| value.to_str()) == Some("update-started.ack");
            let valid_parent = parent.starts_with(env::temp_dir())
                && parent
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.starts_with("EasyCLIProxyAPI-update-"));
            return (valid_name && valid_parent).then_some(path);
        }
    }
    None
}
