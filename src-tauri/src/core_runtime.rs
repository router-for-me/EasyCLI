use super::*;

pub(crate) static CORE_OPERATION_LOCK: Mutex<()> = Mutex::new(());
const BUNDLED_CORE_HANDLED_VERSION_FILE: &str = "cpa-gui-bundled-core-handled.txt";

pub(crate) fn lock_core_operation(
    process_state: &CoreProcessState,
) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    let guard = CORE_OPERATION_LOCK
        .try_lock()
        .map_err(|_| "The kernel is performing another operation. Try again later".to_string())?;
    process_state.ensure_active()?;
    Ok(guard)
}

pub(crate) struct CoreChild {
    child: Child,
    #[cfg(windows)]
    job: isize,
}

impl std::ops::Deref for CoreChild {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for CoreChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for CoreChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            close_windows_handle(self.job);
            let _ = self.child.wait();
        }
        #[cfg(not(windows))]
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = terminate_child(&mut self.child);
        }
    }
}

#[cfg(any(target_os = "linux", test))]
struct CoreSpawnRequest {
    command: Command,
    reply: std::sync::mpsc::SyncSender<io::Result<Child>>,
}

#[cfg(any(target_os = "linux", test))]
static CORE_PROCESS_SPAWNER: LazyLock<Result<std::sync::mpsc::Sender<CoreSpawnRequest>, String>> =
    LazyLock::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<CoreSpawnRequest>();
        thread::Builder::new()
            .name("cpa-core-spawner".to_string())
            .spawn(move || {
                for mut request in receiver {
                    configure_child_lifetime(&mut request.command);
                    let result = request.command.spawn();
                    if let Err(std::sync::mpsc::SendError(Ok(mut child))) =
                        request.reply.send(result)
                    {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            })
            .map_err(|error| format!("Failed to create CPA kernel startup thread: {error}"))?;
        Ok(sender)
    });

pub(crate) fn spawn_core_child(command: Command) -> Result<CoreChild, String> {
    #[cfg(target_os = "linux")]
    let child = spawn_core_child_on_lifetime_thread(command)?;

    #[cfg(not(target_os = "linux"))]
    let child = {
        let mut command = command;
        command
            .spawn()
            .map_err(|error| format!("Failed to start CPA kernel: {error}"))?
    };

    #[cfg(windows)]
    let job = match attach_child_to_windows_job(&child) {
        Ok(job) => job,
        Err(error) => {
            let mut child = child;
            return match terminate_child(&mut child) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}; failed to clean up unmanaged kernel process: {cleanup_error}"
                )),
            };
        }
    };
    Ok(CoreChild {
        child,
        #[cfg(windows)]
        job,
    })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn spawn_core_child_on_lifetime_thread(command: Command) -> Result<Child, String> {
    let sender = CORE_PROCESS_SPAWNER.as_ref().map_err(Clone::clone)?;
    let (reply, result) = std::sync::mpsc::sync_channel(1);
    sender
        .send(CoreSpawnRequest { command, reply })
        .map_err(|_| "CPA kernel startup thread exited".to_string())?;
    result
        .recv()
        .map_err(|_| "CPA kernel startup thread did not return a startup result".to_string())?
        .map_err(|error| format!("Failed to start CPA kernel: {error}"))
}

async fn run_core_command(
    app: tauri::AppHandle,
    operation: fn(&CoreProcessState, &GuiConfigState) -> Result<CoreStatus, String>,
) -> Result<CoreStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_core_operation(app.state::<CoreProcessState>().inner())?;
        let status = operation(
            app.state::<CoreProcessState>().inner(),
            app.state::<GuiConfigState>().inner(),
        )?;
        if status.running {
            let wb_state = app.state::<workbuddy_proxy::WorkBuddyProxyState>();
            let _ = workbuddy_proxy::start_workbuddy_proxy_sync(wb_state.inner());
        } else {
            let wb_state = app.state::<workbuddy_proxy::WorkBuddyProxyState>();
            let _ = workbuddy_proxy::stop_workbuddy_proxy_sync(wb_state.inner());
        }
        emit_core_status(&app, &status);
        Ok(status)
    })
    .await
    .map_err(|error| format!("Kernel background task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn check_latest_core(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
) -> Result<CoreLatest, String> {
    let _detection_guard = VERSION_SOURCE_DETECTION_LOCK.lock().await;
    let platform = current_core_platform()?;
    let config = gui_config_state.snapshot()?;
    let proxy_url = config.proxy_url.clone();
    let client = http_client(&proxy_url, &config.custom_download_mirrors)?;
    let requested_source = config.selected_download_candidate();
    let (release, resolved_source) = fetch_release(
        &client,
        None,
        requested_source.clone(),
        &config.custom_download_mirrors,
    )
    .await?;
    persist_automatic_download_source_switch(
        &app,
        gui_config_state.inner(),
        &requested_source,
        &resolved_source,
    )?;
    let asset = select_release_asset(&release, &platform)?;

    Ok(CoreLatest {
        version: normalize_version(&release.tag_name),
        asset_name: asset.name.clone(),
    })
}

#[tauri::command]
pub(crate) fn detect_bundled_core() -> Result<Option<BundledCoreInfo>, String> {
    bundled_core_archive().map(|value| value.map(|(info, _)| info))
}

#[tauri::command]
pub(crate) async fn install_bundled_core(
    app: tauri::AppHandle,
    window: tauri::Window,
) -> Result<CoreInstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_core_operation(app.state::<CoreProcessState>().inner())?;
        let state = app.state::<CoreDownloadState>();
        let process_state = app.state::<CoreProcessState>();
        let gui_config_state = app.state::<GuiConfigState>();
        let (info, archive_path) = bundled_core_archive()?
            .ok_or_else(|| "This distribution has no bundled kernel matching the system architecture".to_string())?;
        let token = CancellationToken::new();
        state.start(token, Some(info.version.clone()))?;
        let result = install_core_with_runtime_restore(
            &app,
            process_state.inner(),
            gui_config_state.inner(),
            || install_bundled_core_inner(&window, state.inner(), &info, &archive_path),
        );
        if result.is_err() {
            let _ = cleanup_core_work_dirs();
        }
        state.finish(&window, result.clone());
        result
    })
    .await
    .map_err(|error| format!("Offline kernel installation background task failed: {error}"))?
}

pub(crate) fn core_needs_bundled_install(install_dir: &Path, bundled_version: &str) -> bool {
    let Ok(bundled_version) =
        semver::Version::parse(bundled_version.trim().trim_start_matches('v'))
    else {
        return false;
    };
    if find_core_binary(install_dir).is_none() {
        return true;
    }
    if bundled_core_version_handled(install_dir, &bundled_version) {
        return false;
    }
    read_core_metadata(install_dir)
        .and_then(|metadata| {
            semver::Version::parse(metadata.version.trim().trim_start_matches('v')).ok()
        })
        .is_none_or(|installed| bundled_version > installed)
}

fn bundled_core_version_handled(install_dir: &Path, bundled_version: &semver::Version) -> bool {
    fs::read_to_string(install_dir.join(BUNDLED_CORE_HANDLED_VERSION_FILE))
        .ok()
        .and_then(|version| semver::Version::parse(version.trim().trim_start_matches('v')).ok())
        .is_some_and(|version| &version == bundled_version)
}

pub(crate) fn mark_bundled_core_version_handled(
    install_dir: &Path,
    bundled_version: &str,
) -> Result<(), String> {
    write_bytes_atomically(
        &install_dir.join(BUNDLED_CORE_HANDLED_VERSION_FILE),
        bundled_version.as_bytes(),
    )
    .map_err(|error| format!("Failed to record processed bundled kernel version: {error}"))
}

pub(crate) fn remember_bundled_core_when_up_to_date(
    install_dir: &Path,
    bundled_version: &str,
) -> Result<(), String> {
    let Ok(bundled) = semver::Version::parse(bundled_version.trim().trim_start_matches('v')) else {
        return Ok(());
    };
    if bundled_core_version_handled(install_dir, &bundled) {
        return Ok(());
    }
    let installed = read_core_metadata(install_dir).and_then(|metadata| {
        semver::Version::parse(metadata.version.trim().trim_start_matches('v')).ok()
    });
    if find_core_binary(install_dir).is_some()
        && installed.is_some_and(|version| version >= bundled)
    {
        mark_bundled_core_version_handled(install_dir, bundled_version)?;
    }
    Ok(())
}

pub(crate) fn auto_install_bundled_core_if_needed(app: &tauri::AppHandle) -> Result<bool, String> {
    let install_dir = core_install_dir()?;
    let (info, archive_path) = match bundled_core_archive()? {
        Some(bundled) => bundled,
        None if find_core_binary(&install_dir).is_some() => return Ok(false),
        None => return Err("CPA kernel was not detected, and this distribution has no matching offline kernel".to_string()),
    };
    if !core_needs_bundled_install(&install_dir, &info.version) {
        remember_bundled_core_when_up_to_date(&install_dir, &info.version)?;
        return Ok(false);
    }
    let window = main_window(app).ok_or_else(|| {
        "Cannot access the main window to automatically install the offline kernel".to_string()
    })?;
    let state = app.state::<CoreDownloadState>();
    state.start(CancellationToken::new(), Some(info.version.clone()))?;
    let result = install_core_with_runtime_restore(
        app,
        app.state::<CoreProcessState>().inner(),
        app.state::<GuiConfigState>().inner(),
        || install_bundled_core_inner(&window, state.inner(), &info, &archive_path),
    );
    if result.is_err() {
        let _ = cleanup_core_work_dirs();
    }
    state.finish(&window, result.clone());
    result?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn cancel_core_install(state: tauri::State<'_, CoreDownloadState>) {
    state.cancel();
}

#[tauri::command]
pub(crate) fn get_core_install_task(state: tauri::State<'_, CoreDownloadState>) -> CoreInstallTask {
    state.snapshot()
}

pub(crate) struct StagedCoreInstall {
    pub(crate) version: String,
    pub(crate) asset_name: String,
    pub(crate) install_dir: PathBuf,
    pub(crate) staging_dir: PathBuf,
    pub(crate) binary_relative_path: PathBuf,
}

#[tauri::command]
pub(crate) async fn install_core_version(
    app: tauri::AppHandle,
    window: tauri::Window,
    version: Option<String>,
) -> Result<CoreInstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_core_operation(app.state::<CoreProcessState>().inner())?;
        let state = app.state::<CoreDownloadState>();
        let process_state = app.state::<CoreProcessState>();
        let gui_config_state = app.state::<GuiConfigState>();
        let config = gui_config_state.snapshot()?;
        let proxy_url = config.proxy_url.clone();
        let rollback_dir = core_base_dir()?.join("cpa-core.rollback");
        archive_previous_core_rollback(&rollback_dir)?;
        let token = CancellationToken::new();
        state.start(token.clone(), version.clone())?;

        // Stage download, SHA-256 verification, and archive extraction BEFORE stopping the running core.
        let staged_result = tauri::async_runtime::block_on(stage_core_version_inner(
            &app,
            &window,
            state.inner(),
            gui_config_state.inner(),
            token,
            version,
            &proxy_url,
            config.selected_download_candidate(),
            config.custom_download_mirrors.clone(),
        ));

        let result = staged_result.and_then(|staged| {
            let was_running = pause_core_for_install(&app, process_state.inner(), &config)?;
            let swap_and_restore = apply_staged_core_with_rollback(
                &staged.install_dir,
                &staged.staging_dir,
                &rollback_dir,
                || {
                    if was_running && !process_state.is_shutting_down() {
                        start_core_process_inner(process_state.inner(), &config)
                    } else {
                        Ok(())
                    }
                },
            );
            let swap_and_restore = match swap_and_restore {
                Err(error) if was_running && !process_state.is_shutting_down() => {
                    match start_core_process_inner(process_state.inner(), &config) {
                        Ok(()) => Err(error),
                        Err(resume_error) => Err(format!(
                            "{error}; failed to start the previous kernel again: {resume_error}"
                        )),
                    }
                }
                other => other,
            };
            emit_current_core_status(&app, process_state.inner(), config.port);
            swap_and_restore.map(|()| CoreInstallResult {
                version: staged.version,
                asset_name: staged.asset_name,
                install_dir: path_to_string(&staged.install_dir),
                binary_path: Some(path_to_string(
                    &staged.install_dir.join(&staged.binary_relative_path),
                )),
            })
        });

        if result.is_err() {
            let _ = cleanup_core_work_dirs();
        }
        state.finish(&window, result.clone());

        result
    })
    .await
    .map_err(|error| format!("Kernel installation background task failed: {error}"))?
}

fn install_core_with_runtime_restore<F>(
    app: &tauri::AppHandle,
    process_state: &CoreProcessState,
    gui_config_state: &GuiConfigState,
    install: F,
) -> Result<CoreInstallResult, String>
where
    F: FnOnce() -> Result<CoreInstallResult, String>,
{
    let config = gui_config_state.snapshot()?;
    let was_running = pause_core_for_install(app, process_state, &config)?;
    let result = install();
    restore_core_after_install(app, process_state, &config, was_running, result)
}

fn pause_core_for_install(
    app: &tauri::AppHandle,
    process_state: &CoreProcessState,
    config: &GuiConfigFile,
) -> Result<bool, String> {
    let was_running = pause_core_process_for_install(process_state)?;
    if was_running {
        emit_current_core_status(app, process_state, config.port);
    }
    Ok(was_running)
}

pub(crate) fn pause_core_process_for_install(
    process_state: &CoreProcessState,
) -> Result<bool, String> {
    process_state.ensure_active()?;
    let was_running = current_core_status(Some(process_state), None)?.running
        || process_state.recovery.wants_running();
    // Also cancel a pending recovery when an update finds an already-dead core.
    process_state.recovery.cancel();
    if was_running {
        stop_core_process_inner(process_state)?;
    }
    Ok(was_running)
}

fn restore_core_after_install<T>(
    app: &tauri::AppHandle,
    process_state: &CoreProcessState,
    config: &GuiConfigFile,
    was_running: bool,
    install_result: Result<T, String>,
) -> Result<T, String> {
    let restart_result = if was_running && !process_state.is_shutting_down() {
        start_core_process_inner(process_state, config)
    } else {
        Ok(())
    };
    emit_current_core_status(app, process_state, config.port);
    combine_install_and_restart_results(install_result, restart_result)
}

pub(crate) fn combine_install_and_restart_results<T>(
    install_result: Result<T, String>,
    restart_result: Result<(), String>,
) -> Result<T, String> {
    match (install_result, restart_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Ok(_), Err(restart_error)) => {
            Err(format!("Kernel installed, but failed to automatically resume operation: {restart_error}"))
        }
        (Err(install_error), Ok(())) => Err(install_error),
        (Err(install_error), Err(restart_error)) => Err(format!(
            "{install_error}; failed to automatically restore the previous kernel running state: {restart_error}"
        )),
    }
}

fn emit_current_core_status(app: &tauri::AppHandle, process_state: &CoreProcessState, port: u16) {
    if let Ok(status) = current_core_status(Some(process_state), Some(port)) {
        emit_core_status(app, &status);
    }
}

#[tauri::command]
pub(crate) async fn start_core_process(app: tauri::AppHandle) -> Result<CoreStatus, String> {
    run_core_command(app, start_core_process_with_state).await
}

pub(crate) fn start_core_process_with_state(
    process_state: &CoreProcessState,
    gui_config_state: &GuiConfigState,
) -> Result<CoreStatus, String> {
    network_proxy::refresh(gui_config_state)?;
    let config = gui_config_state.snapshot()?;
    process_state.recovery.cancel();
    start_core_process_inner(process_state, &config)?;
    if let Err(error) = gui_config_state.set_run_on_startup(true) {
        let _ = stop_core_process_inner(process_state);
        return Err(error);
    }
    current_core_status(Some(process_state), Some(config.port))
}

#[tauri::command]
pub(crate) async fn stop_core_process(app: tauri::AppHandle) -> Result<CoreStatus, String> {
    run_core_command(app, stop_core_process_with_state).await
}

pub(crate) fn stop_core_process_with_state(
    process_state: &CoreProcessState,
    gui_config_state: &GuiConfigState,
) -> Result<CoreStatus, String> {
    stop_core_process_inner(process_state)?;
    let config = gui_config_state.set_run_on_startup(false)?;
    current_core_status(Some(process_state), Some(config.port))
}

#[tauri::command]
pub(crate) async fn restart_core_process(app: tauri::AppHandle) -> Result<CoreStatus, String> {
    run_core_command(app, restart_core_process_with_state).await
}

pub(crate) fn restart_core_process_with_state(
    process_state: &CoreProcessState,
    gui_config_state: &GuiConfigState,
) -> Result<CoreStatus, String> {
    network_proxy::refresh(gui_config_state)?;
    let config = gui_config_state.snapshot()?;
    process_state.recovery.cancel();
    if current_core_status(Some(process_state), None)?.running {
        stop_core_process_inner(process_state)?;
    }
    start_core_process_inner(process_state, &config)?;
    if let Err(error) = gui_config_state.set_run_on_startup(true) {
        let _ = stop_core_process_inner(process_state);
        return Err(error);
    }
    current_core_status(Some(process_state), Some(config.port))
}

pub(crate) async fn stage_core_version_inner(
    app: &tauri::AppHandle,
    window: &tauri::Window,
    state: &CoreDownloadState,
    gui_config_state: &GuiConfigState,
    token: CancellationToken,
    version: Option<String>,
    proxy_url: &str,
    download_source: VersionDownloadCandidate,
    custom_mirrors: Vec<String>,
) -> Result<StagedCoreInstall, String> {
    let platform = current_core_platform()?;
    let client = http_client(proxy_url, &custom_mirrors)?;
    state.progress(window, "Check version", 0, None, true);
    let requested_source = download_source.clone();
    let (release, resolved_source) = fetch_release_cancelable(
        &client,
        version.as_deref(),
        &token,
        download_source,
        &custom_mirrors,
    )
    .await?;
    let asset = select_release_asset(&release, &platform)?;
    let download_candidates = core_download_candidates(
        &release.tag_name,
        asset,
        resolved_source,
        configured_gitcode_core_repository(),
        &custom_mirrors,
    );

    let install_dir = core_install_dir()?;
    let base_dir = core_base_dir()?;
    let staging_dir = base_dir.join("cpa-core.staging");
    let download_dir = base_dir.join("cpa-core.download");

    reset_dir(&staging_dir)?;
    reset_dir(&download_dir)?;

    let archive_file_name = Path::new(&asset.name)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .ok_or_else(|| format!("Invalid asset filename: {}", asset.name))?;
    let archive_path = download_dir.join(archive_file_name);

    let (downloaded, successful_source) = download_asset(
        &client,
        asset,
        &download_candidates,
        &archive_path,
        window,
        state,
        &token,
    )
    .await?;
    validate_downloaded_asset(asset, &downloaded)?;
    persist_automatic_download_source_switch(
        app,
        gui_config_state,
        &requested_source,
        &successful_source,
    )?;

    ensure_not_cancelled(&token, Some(&archive_path))?;
    state.progress(
        window,
        "Extracting",
        downloaded.size,
        Some(downloaded.size),
        false,
    );
    match platform.archive_kind.as_str() {
        "tar.gz" => extract_tar_gz(&archive_path, &staging_dir)?,
        "zip" => extract_zip(&archive_path, &staging_dir)?,
        other => return Err(format!("Unsupported archive type: {other}")),
    }
    ensure_not_cancelled(&token, Some(&archive_path))?;

    let binary_path = find_core_binary(&staging_dir)
        .ok_or_else(|| "CPA kernel binary was not found after extraction".to_string())?;
    let binary_size = fs::metadata(&binary_path)
        .map_err(|err| format!("Failed to inspect extracted kernel binary: {err}"))?
        .len();
    if binary_size == 0 {
        return Err("Extracted CPA kernel binary is empty".to_string());
    }
    let binary_relative_path = binary_path
        .strip_prefix(&staging_dir)
        .map_err(|err| format!("Failed to calculate relative path of kernel binary: {err}"))?
        .to_path_buf();
    migrate_core_config_for_update(&install_dir, &staging_dir)?;
    preserve_bundled_core_assets(&install_dir, &staging_dir)?;
    write_core_metadata(
        &staging_dir,
        &CoreMetadata {
            version: normalize_version(&release.tag_name),
            asset_name: asset.name.clone(),
            installed_at_unix: unix_now(),
        },
    )?;

    let _ = fs::remove_dir_all(&download_dir);

    Ok(StagedCoreInstall {
        version: normalize_version(&release.tag_name),
        asset_name: asset.name.clone(),
        install_dir,
        staging_dir,
        binary_relative_path,
    })
}

pub(crate) fn install_bundled_core_inner(
    window: &tauri::Window,
    state: &CoreDownloadState,
    info: &BundledCoreInfo,
    archive_path: &Path,
) -> Result<CoreInstallResult, String> {
    let platform = current_core_platform()?;
    let install_dir = core_install_dir()?;
    let base_dir = core_base_dir()?;
    let staging_dir = base_dir.join("cpa-core.staging");

    if current_core_status(None, None)?.running {
        return Err("CPA kernel is running. Stop it before using the bundled kernel".to_string());
    }

    let archive_size = fs::metadata(archive_path)
        .map_err(|error| format!("Failed to read bundled kernel archive: {error}"))?
        .len();
    state.progress(window, "Verify bundled kernel", 0, Some(archive_size), false);
    validate_bundled_core_checksum(archive_path)?;
    reset_dir(&staging_dir)?;
    state.progress(
        window,
        "Extract bundled kernel",
        archive_size,
        Some(archive_size),
        false,
    );
    match platform.archive_kind.as_str() {
        "tar.gz" => extract_tar_gz(archive_path, &staging_dir)?,
        "zip" => extract_zip(archive_path, &staging_dir)?,
        other => return Err(format!("Unsupported bundled archive type: {other}")),
    }

    let binary_path = find_core_binary(&staging_dir)
        .ok_or_else(|| "Bundled archive does not contain a CPA kernel binary".to_string())?;
    let binary_relative_path = binary_path
        .strip_prefix(&staging_dir)
        .map_err(|error| format!("Failed to calculate bundled kernel binary path: {error}"))?
        .to_path_buf();
    migrate_core_config_for_update(&install_dir, &staging_dir)?;
    preserve_bundled_core_assets(&install_dir, &staging_dir)?;
    preserve_selected_bundled_core_asset(archive_path, &staging_dir)?;
    write_core_metadata(
        &staging_dir,
        &CoreMetadata {
            version: info.version.clone(),
            asset_name: info.asset_name.clone(),
            installed_at_unix: unix_now(),
        },
    )?;
    overlay_install_dir(&install_dir, &staging_dir)?;
    mark_bundled_core_version_handled(&install_dir, &info.version)?;

    Ok(CoreInstallResult {
        version: info.version.clone(),
        asset_name: info.asset_name.clone(),
        install_dir: path_to_string(&install_dir),
        binary_path: Some(path_to_string(&install_dir.join(binary_relative_path))),
    })
}

pub(crate) async fn fetch_release(
    client: &reqwest::Client,
    version: Option<&str>,
    source: VersionDownloadCandidate,
    custom_mirrors: &[String],
) -> Result<(GithubRelease, VersionDownloadCandidate), String> {
    if let Some(version) = version {
        return Ok((
            release_from_tag_for_repositories(
                version,
                configured_gitcode_core_repository(),
                &source,
            ),
            source,
        ));
    }
    let gitcode_repository = configured_gitcode_core_repository();
    let mut failures = Vec::new();
    for candidate in
        version_download_source_candidates(source, gitcode_repository.is_some(), custom_mirrors)
    {
        let result = match candidate.source {
            VersionDownloadSource::Gitcode => {
                fetch_release_from_gitcode(
                    client,
                    gitcode_repository.expect("GitCode candidate requires a configured repository"),
                )
                .await
            }
            VersionDownloadSource::Github
            | VersionDownloadSource::GhProxy
            | VersionDownloadSource::GhFast
            | VersionDownloadSource::Custom => fetch_release_from_github(client, &candidate).await,
        };
        match result {
            Ok(release) => return Ok((release, candidate)),
            Err(error) => failures.push(format!("{}: {error}", candidate.display_name())),
        }
    }
    Err(format!("All kernel version check sources failed: {}", failures.join("；")))
}

pub(crate) async fn fetch_release_from_github(
    client: &reqwest::Client,
    source: &VersionDownloadCandidate,
) -> Result<GithubRelease, String> {
    let atom_result = fetch_release_from_atom(client, source).await;
    match atom_result {
        Ok(release) => Ok(release),
        Err(atom_error) => fetch_release_from_page(client, source)
            .await
            .map_err(|page_error| {
                format!("GitHub release source request failed: {atom_error}; release page request failed: {page_error}")
            }),
    }
}

pub(crate) async fn fetch_release_from_gitcode(
    client: &reqwest::Client,
    repository: &str,
) -> Result<GithubRelease, String> {
    let release_url = format!("https://api.gitcode.com/api/v5/repos/{repository}/releases/latest");
    let release = client
        .get(release_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("Failed to query latest GitCode kernel release: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Failed to read latest GitCode kernel release: {error}"))?
        .json::<GitcodeRelease>()
        .await
        .map_err(|error| format!("Failed to parse latest GitCode kernel release: {error}"))?;
    validate_release_tag(&release.tag_name)?;
    Ok(release_from_gitcode_tag(&release.tag_name, repository))
}

pub(crate) async fn fetch_release_from_page(
    client: &reqwest::Client,
    source: &VersionDownloadCandidate,
) -> Result<GithubRelease, String> {
    let release_page_url = version_source_url(source, RELEASE_PAGE_URL);
    let response = client
        .get(release_page_url)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("GitHub release page request failed: {err}"))?;
    let status = response.status();
    let final_url = response.url().clone();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .map_err(|err| format!("Failed to read GitHub release page: {err}"))?;
        return Err(format_github_error(status.as_u16(), &body));
    }

    let tag = release_tag_from_url(&final_url)
        .ok_or_else(|| "GitHub release page did not return a version tag".to_string())?;
    Ok(release_from_tag_for_repositories(
        &tag,
        configured_gitcode_core_repository(),
        source,
    ))
}

pub(crate) async fn fetch_release_from_atom(
    client: &reqwest::Client,
    source: &VersionDownloadCandidate,
) -> Result<GithubRelease, String> {
    let release_atom_url = version_source_url(source, RELEASE_ATOM_URL);
    let response = client
        .get(release_atom_url)
        .header(
            reqwest::header::ACCEPT,
            "application/atom+xml,application/xml,text/xml",
        )
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("GitHub Atom feed request failed: {err}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("Failed to read GitHub Atom feed: {err}"))?;
    if !status.is_success() {
        return Err(format_github_error(status.as_u16(), &body));
    }
    let tag = release_tag_from_atom(&body)
        .ok_or_else(|| "GitHub Atom feed did not return a version tag".to_string())?;
    Ok(release_from_tag_for_repositories(
        &tag,
        configured_gitcode_core_repository(),
        source,
    ))
}

pub(crate) fn release_tag_from_atom(xml: &str) -> Option<String> {
    let entry = xml.split_once("<entry>")?.1;
    if let Some(tag_path) = entry.split_once("/releases/tag/").map(|(_, value)| value) {
        let tag = tag_path
            .split(['\"', '<', '?', '#'])
            .next()
            .unwrap_or_default()
            .trim_matches('/');
        if !tag.is_empty() {
            return Some(normalize_version(tag));
        }
    }
    let title = entry
        .split_once("<title>")?
        .1
        .split_once("</title>")?
        .0
        .trim();
    (!title.is_empty()).then(|| normalize_version(title))
}

#[cfg(test)]
pub(crate) fn release_from_tag(tag: &str) -> GithubRelease {
    release_from_tag_for_repositories(
        tag,
        configured_gitcode_core_repository(),
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Github),
    )
}

pub(crate) fn release_from_gitcode_tag(tag: &str, repository: &str) -> GithubRelease {
    release_from_tag_for_repositories(
        tag,
        Some(repository),
        &VersionDownloadCandidate::builtin(VersionDownloadSource::Gitcode),
    )
}

pub(crate) fn release_from_tag_for_repositories(
    tag: &str,
    gitcode_repository: Option<&str>,
    source: &VersionDownloadCandidate,
) -> GithubRelease {
    let tag = normalize_version(tag);
    let version = tag.trim_start_matches('v');
    let assets = [
        ("linux", "amd64", "tar.gz"),
        ("linux", "aarch64", "tar.gz"),
        ("darwin", "amd64", "tar.gz"),
        ("darwin", "aarch64", "tar.gz"),
        ("windows", "amd64", "zip"),
        ("windows", "aarch64", "zip"),
    ]
    .into_iter()
    .map(|(os, arch, extension)| {
        let name = format!("CLIProxyAPI_{version}_{os}_{arch}.{extension}");
        let github_url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{name}");
        let gitcode_url = gitcode_repository
            .map(|repository| gitcode_release_attachment_url(repository, &tag, &name));
        let (browser_download_url, fallback_download_urls) = match source.source {
            VersionDownloadSource::Gitcode => match gitcode_url {
                Some(gitcode_url) => (gitcode_url, vec![github_url]),
                None => (github_url, Vec::new()),
            },
            VersionDownloadSource::GhProxy | VersionDownloadSource::GhFast => {
                let mirror_url = version_source_url(source, &github_url);
                let mut fallbacks = vec![github_url];
                fallbacks.extend(gitcode_url);
                (mirror_url, fallbacks)
            }
            VersionDownloadSource::Github => (github_url, gitcode_url.into_iter().collect()),
            VersionDownloadSource::Custom => {
                let mirror_url = version_source_url(source, &github_url);
                let mut fallbacks = vec![github_url];
                fallbacks.extend(gitcode_url);
                (mirror_url, fallbacks)
            }
        };
        GithubAsset {
            browser_download_url,
            fallback_download_urls,
            name,
            size: None,
            digest: None,
        }
    })
    .collect();
    GithubRelease {
        tag_name: tag,
        assets,
    }
}

pub(crate) fn core_download_candidates(
    tag: &str,
    asset: &GithubAsset,
    preferred: VersionDownloadCandidate,
    gitcode_repository: Option<&str>,
    custom_mirrors: &[String],
) -> Vec<(VersionDownloadCandidate, String)> {
    let tag = normalize_version(tag);
    let generated_github_url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", asset.name);
    let provided_urls = std::iter::once(&asset.browser_download_url)
        .chain(asset.fallback_download_urls.iter())
        .collect::<Vec<_>>();
    let github_url = provided_urls
        .iter()
        .find(|url| core_download_source_name(url) == "GitHub")
        .map(|url| (*url).clone())
        .unwrap_or(generated_github_url);
    let gitcode_url = provided_urls
        .iter()
        .find(|url| core_download_source_name(url) == "GitCode")
        .map(|url| (*url).clone())
        .or_else(|| {
            gitcode_repository
                .map(|repository| gitcode_release_attachment_url(repository, &tag, &asset.name))
        });
    version_download_source_candidates(preferred, gitcode_url.is_some(), custom_mirrors)
        .into_iter()
        .filter_map(|candidate| {
            let url = match candidate.source {
                VersionDownloadSource::Github => github_url.clone(),
                VersionDownloadSource::Gitcode => gitcode_url.clone()?,
                VersionDownloadSource::GhProxy
                | VersionDownloadSource::GhFast
                | VersionDownloadSource::Custom => version_source_url(&candidate, &github_url),
            };
            Some((candidate, url))
        })
        .collect()
}

pub(crate) fn release_tag_from_url(url: &reqwest::Url) -> Option<String> {
    let mut segments = url.path_segments()?;
    let tag = segments.next_back()?.trim();
    if tag.is_empty() || tag == "latest" {
        None
    } else {
        Some(tag.to_string())
    }
}

pub(crate) fn is_app_update_available(current: &str, latest: &str) -> Result<bool, String> {
    let parse = |value: &str| {
        semver::Version::parse(value.trim().trim_start_matches('v'))
            .map_err(|error| format!("Failed to parse version {value}: {error}"))
    };
    Ok(parse(latest)? > parse(current)?)
}

#[cfg(test)]
pub(crate) fn parse_release_assets(html: &str) -> Vec<GithubAsset> {
    let mut assets = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = html[cursor..].find("releases/download/") {
        let download_start = cursor + relative_start;
        let Some(href_start) = html[..download_start].rfind("href=\"") else {
            cursor = download_start + "releases/download/".len();
            continue;
        };
        let href_start = href_start + "href=\"".len();
        let Some(relative_end) = html[download_start..].find('"') else {
            break;
        };
        let href_end = download_start + relative_end;
        let href = &html[href_start..href_end];
        let Some(name) = href.rsplit('/').next().filter(|name| !name.is_empty()) else {
            cursor = href_end + 1;
            continue;
        };
        let item_end = html[href_end..]
            .find("</li>")
            .map(|offset| href_end + offset)
            .unwrap_or(html.len());
        let item = &html[href_start..item_end];
        let digest = item.find("sha256:").and_then(|offset| {
            let value = &item[offset + "sha256:".len()..];
            let hash: String = value
                .chars()
                .take_while(|character| character.is_ascii_hexdigit())
                .collect();
            (hash.len() == 64).then(|| format!("sha256:{hash}"))
        });
        let browser_download_url = if href.starts_with("http://") || href.starts_with("https://") {
            href.to_string()
        } else {
            format!("https://github.com{href}")
        };

        if !assets.iter().any(|asset: &GithubAsset| asset.name == name) {
            assets.push(GithubAsset {
                name: name.to_string(),
                browser_download_url,
                fallback_download_urls: Vec::new(),
                size: None,
                digest,
            });
        }
        cursor = href_end + 1;
    }

    assets
}

pub(crate) fn format_github_error(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(message) = value.get("message").and_then(|item| item.as_str()) {
            return format!("GitHub returned an error ({status}): {}", message.trim());
        }
    }
    let body = body.trim();
    if body.is_empty() {
        format!("GitHub returned an error ({status})")
    } else {
        format!("GitHub returned an error ({status}): {}", truncate_for_error(body))
    }
}

pub(crate) async fn fetch_release_cancelable(
    client: &reqwest::Client,
    version: Option<&str>,
    token: &CancellationToken,
    source: VersionDownloadCandidate,
    custom_mirrors: &[String],
) -> Result<(GithubRelease, VersionDownloadCandidate), String> {
    tokio::select! {
        result = fetch_release(client, version, source, custom_mirrors) => result,
        _ = token.cancelled() => Err("Download canceled".to_string()),
    }
}

pub(crate) fn apply_configured_proxy(
    builder: reqwest::ClientBuilder,
    proxy_url: &str,
) -> Result<reqwest::ClientBuilder, String> {
    let proxy_url = proxy_url.trim();
    let builder = builder.no_proxy();
    if proxy_url.is_empty() {
        return Ok(builder);
    }
    let proxy = reqwest::Proxy::all(proxy_url)
        .map_err(|_| "Invalid proxy URL".to_string())?
        .no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
    Ok(builder.proxy(proxy))
}

pub(crate) fn build_http_client_with_proxy(
    builder: reqwest::ClientBuilder,
    proxy_url: &str,
    error_prefix: &str,
) -> Result<reqwest::Client, String> {
    apply_configured_proxy(builder, proxy_url)
        .map_err(|error| format!("{error_prefix}: {error}"))?
        .build()
        .map_err(|error| format!("{error_prefix}: {error}"))
}

pub(crate) fn http_client(
    proxy_url: &str,
    custom_mirrors: &[String],
) -> Result<reqwest::Client, String> {
    build_http_client_with_proxy(
        reqwest::Client::builder()
            .redirect(release_https_redirect_policy_with_mirrors(custom_mirrors))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600)),
        proxy_url,
        "Failed to create HTTP client",
    )
}

pub(crate) fn select_release_asset<'a>(
    release: &'a GithubRelease,
    platform: &CorePlatform,
) -> Result<&'a GithubAsset, String> {
    let expected_name = core_release_asset_name(&release.tag_name, platform);
    let mut matches = release
        .assets
        .iter()
        .filter(|asset| asset.name == expected_name && !asset.name.contains("_no-plugin"));
    let asset = matches
        .next()
        .ok_or_else(|| format!("No release asset matches the current platform: {expected_name}"))?;

    if matches.next().is_some() {
        return Err(format!("Multiple release assets match: {expected_name}"));
    }

    Ok(asset)
}

pub(crate) fn core_release_asset_name(version: &str, platform: &CorePlatform) -> String {
    let version = normalize_version(version);
    let version = version.trim_start_matches('v');
    format!(
        "CLIProxyAPI_{}_{}_{}.{}",
        version, platform.asset_os, platform.asset_arch, platform.archive_kind
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_asset(
    client: &reqwest::Client,
    asset: &GithubAsset,
    download_candidates: &[(VersionDownloadCandidate, String)],
    archive_path: &Path,
    window: &tauri::Window,
    state: &CoreDownloadState,
    token: &CancellationToken,
) -> Result<(DownloadedArchive, VersionDownloadCandidate), String> {
    let mut failures = Vec::new();
    for (index, (candidate, url)) in download_candidates.iter().enumerate() {
        if index > 0 {
            state.progress(
                window,
                &format!("Download failed; switching to {}", candidate.display_name()),
                0,
                asset.size,
                true,
            );
        }
        let result = download_asset_inner(
            client,
            url,
            archive_path,
            asset.size,
            asset.digest.as_deref(),
            window,
            state,
            token,
        )
        .await;
        match result {
            Ok(downloaded) => return Ok((downloaded, candidate.clone())),
            Err(error) if token.is_cancelled() => {
                let _ = fs::remove_file(archive_path);
                return Err(error);
            }
            Err(error) => {
                let _ = fs::remove_file(archive_path);
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        let _ = fs::remove_file(archive_path);
        return Err("Kernel release has no available download URL".to_string());
    }
    Err(format!("All kernel download sources failed: {}", failures.join("；")))
}

pub(crate) fn core_download_source_name(url: &str) -> String {
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_asset_inner(
    client: &reqwest::Client,
    url: &str,
    archive_path: &Path,
    expected_total: Option<u64>,
    expected_digest: Option<&str>,
    window: &tauri::Window,
    state: &CoreDownloadState,
    token: &CancellationToken,
) -> Result<DownloadedArchive, String> {
    state.progress(window, "Preparing download", 0, expected_total, true);
    ensure_not_cancelled(token, Some(archive_path))?;

    let request = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send();
    let response = tokio::select! {
        response = request => response.map_err(|err| format!("Failed to download kernel archive: {err}"))?,
        _ = token.cancelled() => return Err("Download canceled".to_string()),
    }
    .error_for_status()
    .map_err(|err| format!("Download URL returned an error status: {err}"))?;
    let total = expected_total.or_else(|| response.content_length());
    let mut stream = response.bytes_stream();
    let mut file =
        File::create(archive_path).map_err(|err| format!("Failed to create kernel archive: {err}"))?;
    let mut downloaded = 0_u64;
    let mut hasher = Sha256::new();
    let mut progress = crate::progress::ProgressThrottle::default();

    while let Some(chunk) = tokio::select! {
        chunk = stream.next() => chunk,
        _ = token.cancelled() => return Err("Download canceled".to_string()),
    } {
        ensure_not_cancelled(token, Some(archive_path))?;

        let chunk = chunk.map_err(|err| format!("Failed to read download data: {err}"))?;
        file.write_all(&chunk)
            .map_err(|err| format!("Failed to save download data: {err}"))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if progress.ready(Instant::now(), total == Some(downloaded)) {
            state.progress(window, "Downloading", downloaded, total, true);
        }
    }

    state.progress(window, "Downloading", downloaded, total, true);
    file.flush()
        .map_err(|err| format!("Failed to flush kernel archive: {err}"))?;
    ensure_not_cancelled(token, Some(archive_path))?;

    let sha256 = format!("{:x}", hasher.finalize());
    validate_download_metadata(downloaded, expected_total, &sha256, expected_digest)?;

    Ok(DownloadedArchive {
        size: downloaded,
        sha256,
    })
}

pub(crate) fn ensure_not_cancelled(
    token: &CancellationToken,
    archive_path: Option<&Path>,
) -> Result<(), String> {
    if token.is_cancelled() {
        if let Some(archive_path) = archive_path {
            let _ = fs::remove_file(archive_path);
        }

        return Err("Download canceled".to_string());
    }

    Ok(())
}

pub(crate) fn current_core_platform() -> Result<CorePlatform, String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let (asset_os, archive_kind) = match os {
        "linux" => ("linux", "tar.gz"),
        "macos" => ("darwin", "tar.gz"),
        "windows" => ("windows", "zip"),
        other => return Err(format!("Unsupported operating system: {other}")),
    };

    let asset_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "aarch64",
        other => return Err(format!("Unsupported CPU architecture: {other}")),
    };

    Ok(CorePlatform {
        os: os.to_string(),
        arch: arch.to_string(),
        asset_os: asset_os.to_string(),
        asset_arch: asset_arch.to_string(),
        archive_kind: archive_kind.to_string(),
    })
}

pub(crate) fn current_core_status(
    process_state: Option<&CoreProcessState>,
    management_port: Option<u16>,
) -> Result<CoreStatus, String> {
    let install_dir = core_install_dir()?;
    let binary_path = find_core_binary(&install_dir);
    let installed = binary_path.is_some();
    let starting = process_state.is_some_and(CoreProcessState::is_starting);
    let managed_pid = process_state.and_then(|state| state.managed_pid());
    let management_port_open = management_port.map(is_management_port_open);
    let process_id = match managed_pid {
        Some(process_id) => Some(process_id),
        None if management_port_open.unwrap_or(true) => binary_path
            .as_ref()
            .and_then(|path| find_core_process_ids(path).first().copied()),
        None => None,
    };
    // A tracked process remains running even if a single management-port probe
    // times out. The port probe is still required before discovering an
    // untracked process, but using it to override a known live PID makes the UI
    // oscillate between running and stopped while continuing to show that PID.
    let running = process_id.is_some();
    let ready = running && management_port_open.unwrap_or(true);
    let current_version = read_core_metadata(&install_dir).map(|metadata| metadata.version);

    let message = if starting {
        "CPA kernel is starting".to_string()
    } else if !installed {
        "CPA kernel is not installed. Install the latest version first".to_string()
    } else if running {
        "CPA kernel is running".to_string()
    } else {
        "CPA kernel is installed but not running".to_string()
    };

    Ok(CoreStatus {
        installed,
        running,
        ready,
        starting,
        recovery: process_state.and_then(|state| state.recovery.status()),
        managed: managed_pid.is_some(),
        process_id,
        current_version,
        install_dir: path_to_string(&install_dir),
        binary_path: binary_path.map(|path| path_to_string(&path)),
        message,
    })
}

pub(crate) fn is_management_port_open(port: u16) -> bool {
    let listen_host = read_installed_core_config_settings()
        .map(|settings| settings.host)
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let Ok(address) = core_management_address(&listen_host, port) else {
        return false;
    };
    for attempt in 0..3 {
        if TcpStream::connect_timeout(&address, Duration::from_millis(150)).is_ok() {
            return true;
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(50));
        }
    }
    false
}

fn core_management_address(listen_host: &str, port: u16) -> Result<SocketAddr, String> {
    let host = core_connect_host(listen_host);
    let ip = host
        .parse::<IpAddr>()
        .map_err(|_| format!("Invalid core listen IP: {listen_host}"))?;
    Ok(SocketAddr::new(ip, port))
}

pub(crate) enum CoreStartupFailure {
    ShuttingDown,
    Exited(std::process::ExitStatus),
    Spawn(String),
    StatusCheck(io::Error),
    TimedOut(u16),
}

impl CoreStartupFailure {
    fn child_has_exited(&self) -> bool {
        matches!(self, Self::Exited(_))
    }

    #[cfg(target_os = "macos")]
    fn was_killed_by_sigkill(&self) -> bool {
        use std::os::unix::process::ExitStatusExt;

        matches!(
            self,
            Self::Exited(status)
                if status.code().is_none() && status.signal() == Some(libc::SIGKILL)
        )
    }
}

impl std::fmt::Display for CoreStartupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShuttingDown => formatter.write_str("Application is exiting; kernel startup canceled"),
            Self::Exited(status) => write!(formatter, "CPA kernel exited immediately after startup: {status}"),
            Self::Spawn(error) => formatter.write_str(error),
            Self::StatusCheck(error) => write!(formatter, "Failed to check CPA kernel startup status: {error}"),
            Self::TimedOut(port) => {
                write!(formatter, "CPA kernel startup timed out: management port {port} was not listening within 10 seconds")
            }
        }
    }
}

pub(crate) fn core_start_log_path(install_dir: &Path, auth_dir: &str) -> Result<PathBuf, String> {
    Ok(core_logs_dir_path(auth_dir, install_dir)?.join("core-start-output.log"))
}

pub(crate) fn core_start_stdio(log_path: &Path) -> io::Result<(Stdio, Stdio)> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut header_file = File::options()
        .append(true)
        .create(true)
        .open(log_path)?;
    writeln!(header_file, "\n===== CPA kernel startup {} =====", unix_now())?;
    drop(header_file);

    let stdout_file = File::options().append(true).open(log_path)?;
    let stderr_file = File::options().append(true).create(true).open(log_path)?;

    Ok((Stdio::from(stdout_file), Stdio::from(stderr_file)))
}

fn core_start_error_with_log(error: &str, log_path: &Path, log_error: Option<&str>) -> String {
    match log_error {
        Some(log_error) => format!(
            "{error}; failed to write startup log {}: {log_error}",
            path_to_string(log_path)
        ),
        None => format!("{error}; startup log: {}", path_to_string(log_path)),
    }
}

struct CoreStartAttemptFailure {
    failure: CoreStartupFailure,
    log_error: Option<String>,
}

impl CoreStartAttemptFailure {
    #[cfg(target_os = "macos")]
    fn was_killed_by_sigkill(&self) -> bool {
        self.failure.was_killed_by_sigkill()
    }

    fn message(&self, log_path: &Path) -> String {
        self.message_with_detail(log_path, None)
    }

    fn message_with_detail(&self, log_path: &Path, detail: Option<&str>) -> String {
        let error = match detail {
            Some(detail) => format!("{}；{detail}", self.failure),
            None => self.failure.to_string(),
        };
        core_start_error_with_log(&error, log_path, self.log_error.as_deref())
    }
}

fn start_core_process_once(
    process_state: &CoreProcessState,
    binary_path: &Path,
    config_path: &str,
    install_dir: &Path,
    log_path: &Path,
    management_address: SocketAddr,
) -> Result<CoreChild, CoreStartAttemptFailure> {
    let mut command = Command::new(binary_path);
    command
        .args(["-config", config_path])
        .current_dir(install_dir)
        .stdin(Stdio::null());
    let log_error = match core_start_stdio(log_path) {
        Ok((stdout, stderr)) => {
            command.stdout(stdout).stderr(stderr);
            None
        }
        Err(error) => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
            Some(error.to_string())
        }
    };
    configure_background_command(&mut command);
    for variable in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(variable);
    }

    let mut child = match spawn_core_child(command) {
        Ok(child) => child,
        Err(error) => {
            return Err(CoreStartAttemptFailure {
                failure: CoreStartupFailure::Spawn(error),
                log_error,
            });
        }
    };
    match wait_for_core_management_port(&mut child, management_address, process_state) {
        Ok(()) => Ok(child),
        Err(failure) => {
            if !failure.child_has_exited() {
                let _ = terminate_child(&mut child);
            }
            Err(CoreStartAttemptFailure { failure, log_error })
        }
    }
}

pub(crate) fn start_core_process_inner(
    process_state: &CoreProcessState,
    gui_config: &GuiConfigFile,
) -> Result<(), String> {
    process_state.ensure_active()?;
    let mut resolved_config = gui_config.clone();
    resolved_config.proxy_url = network_proxy::resolve(gui_config);
    let gui_config = &resolved_config;
    let install_dir = core_install_dir()?;
    if !gui_config.auth_dir.trim().is_empty() {
        let auth_dir = auth_dir_path_for_core(&gui_config.auth_dir, &install_dir)?;
        fs::create_dir_all(&auth_dir)
            .map_err(|error| format!("Failed to create credentials directory {}: {error}", path_to_string(&auth_dir)))?;
    }
    let binary_path = find_core_binary(&install_dir)
        .ok_or_else(|| "CPA kernel is not installed. Install the latest version first".to_string())?;

    let existing_process_ids = find_core_process_ids(&binary_path);
    if process_state.managed_pid().is_some() || !existing_process_ids.is_empty() {
        if !existing_process_ids.is_empty() {
            process_state.adopt_process_ids(&binary_path, existing_process_ids)?;
        }
        return Err("CPA kernel is already running".to_string());
    }
    let management_address = core_management_address(&gui_config.host, gui_config.port)?;
    if TcpStream::connect_timeout(&management_address, Duration::from_millis(250)).is_ok() {
        return Err(format!(
            "Port {} is in use by another program. Change the port and try again",
            gui_config.port
        ));
    }

    // Pick the companion port before writing the config so both sides agree even
    // when the preferred port is held by another process.
    let companion_port = workbuddy_proxy::reserve_workbuddy_port(
        workbuddy_proxy::DEFAULT_WORKBUDDY_PORT,
        workbuddy_proxy::DEFAULT_WORKBUDDY_HOST,
    );
    let config_path = merge_core_config_for_start(&install_dir, gui_config)?;
    // Runs after the merge so the merged template cannot drop the provider entry.
    let _ = workbuddy_proxy::sync_workbuddy_provider_to_cpa_sync(companion_port, None);
    let config_path = path_to_string(&config_path);
    let log_path = core_start_log_path(&install_dir, &gui_config.auth_dir)?;
    let start_once = || {
        start_core_process_once(
            process_state,
            &binary_path,
            &config_path,
            &install_dir,
            &log_path,
            management_address,
        )
    };

    #[cfg(target_os = "macos")]
    let child = match start_once() {
        Ok(child) => child,
        Err(failure) if failure.was_killed_by_sigkill() => {
            if let Err(heal_error) = rematerialize_core_binary(&binary_path) {
                return Err(failure.message_with_detail(
                    &log_path,
                    Some(&format!("Failed to automatically repair CPA kernel files: {heal_error}")),
                ));
            }
            match start_once() {
                Ok(child) => child,
                Err(failure) if failure.was_killed_by_sigkill() => {
                    return Err(failure.message_with_detail(
                        &log_path,
                        Some("The system terminated the CPA kernel again. Reinstall the kernel and try again"),
                    ));
                }
                Err(failure) => return Err(failure.message(&log_path)),
            }
        }
        Err(failure) => return Err(failure.message(&log_path)),
    };
    #[cfg(not(target_os = "macos"))]
    let child = start_once().map_err(|failure| failure.message(&log_path))?;

    process_state.store_child(child)?;
    Ok(())
}

pub(crate) fn wait_for_core_management_port(
    child: &mut Child,
    address: SocketAddr,
    process_state: &CoreProcessState,
) -> Result<(), CoreStartupFailure> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if process_state.is_shutting_down() {
            return Err(CoreStartupFailure::ShuttingDown);
        }
        if let Some(status) = child.try_wait().map_err(CoreStartupFailure::StatusCheck)? {
            return Err(CoreStartupFailure::Exited(status));
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(CoreStartupFailure::TimedOut(address.port()));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(any(target_os = "linux", test))]
fn configure_child_lifetime(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;

        let parent_process_id = unsafe { libc::getpid() };
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(io::Error::last_os_error());
                }

                if libc::getppid() != parent_process_id {
                    return Err(io::Error::other(
                        "EasyCLIProxyAPI exited before the core process started",
                    ));
                }

                Ok(())
            });
        }
    }

    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

pub(crate) fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    let _ = command;
}

pub(crate) fn configure_networked_command(command: &mut Command, proxy_url: &str) {
    let proxy_url = proxy_url.trim();
    for variable in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        if proxy_url.is_empty() {
            command.env_remove(variable);
        } else {
            command.env(variable, proxy_url);
        }
    }
    command
        .env("NO_PROXY", "localhost,127.0.0.1,::1")
        .env("no_proxy", "localhost,127.0.0.1,::1");
}

pub(crate) fn stop_core_process_inner(process_state: &CoreProcessState) -> Result<(), String> {
    process_state.recovery.cancel();
    let mut stopped_any = false;
    let mut errors = Vec::new();

    if let Some(mut child) = process_state.take_child() {
        stopped_any = true;
        if let Err(error) = terminate_child(&mut child) {
            errors.push(error);
        }
    }

    let mut process_ids = process_state
        .take_adopted_processes()
        .into_iter()
        .filter(|process| {
            process_executable_path(process.process_id)
                .as_deref()
                .is_some_and(|path| executable_paths_match(&process.binary_path, path))
        })
        .map(|process| process.process_id)
        .collect::<Vec<_>>();
    if let Ok(install_dir) = core_install_dir() {
        if let Some(binary_path) = find_core_binary(&install_dir) {
            process_ids.extend(find_core_process_ids(&binary_path));
        }
    }
    process_ids.sort_unstable();
    process_ids.dedup();

    for process_id in process_ids {
        if !is_process_alive(process_id) {
            continue;
        }
        stopped_any = true;
        if let Err(error) = terminate_process_id(process_id) {
            errors.push(error);
        }
    }

    if !errors.is_empty() {
        Err(errors.join("；"))
    } else if stopped_any {
        Ok(())
    } else {
        Err("CPA kernel is not currently running".to_string())
    }
}

pub(crate) fn core_install_dir() -> Result<PathBuf, String> {
    Ok(core_base_dir()?.join("cpa-core"))
}

pub(crate) fn executable_dir() -> Result<PathBuf, String> {
    let exe_path = env::current_exe().map_err(|err| format!("Failed to read current executable path: {err}"))?;
    exe_path
        .parent()
        .map(|path| path.to_path_buf())
        .ok_or_else(|| format!("Current executable path has no parent directory: {}", path_to_string(&exe_path)))
}

pub(crate) fn macos_app_resources_dir(executable_dir: &Path) -> Option<PathBuf> {
    if executable_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        return None;
    }
    let contents_dir = executable_dir.parent()?;
    if contents_dir.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        return None;
    }
    let app_dir = contents_dir.parent()?;
    if app_dir.extension().and_then(|extension| extension.to_str()) != Some("app") {
        return None;
    }
    Some(contents_dir.join("Resources"))
}

pub(crate) fn core_base_dir() -> Result<PathBuf, String> {
    let executable_dir = executable_dir()?;
    #[cfg(target_os = "macos")]
    if macos_app_resources_dir(&executable_dir).is_some() {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "Unable to determine macOS user directory".to_string())?;
        return Ok(home_dir
            .join("Library")
            .join("Application Support")
            .join("com.cpa.gui"));
    }
    Ok(executable_dir)
}

pub(crate) fn bundled_core_locations(
    base_dir: &Path,
    executable_dir: &Path,
) -> Vec<(PathBuf, PathBuf)> {
    let mut locations = Vec::new();
    if let Some(resources_dir) = macos_app_resources_dir(executable_dir) {
        locations.push((
            resources_dir.join(CORE_VERSION_FILE),
            resources_dir.join("cpa-core"),
        ));
    }
    locations.push((base_dir.join(CORE_VERSION_FILE), base_dir.join("cpa-core")));
    if let Some(project_root) = source_project_root(executable_dir) {
        if project_root != base_dir {
            locations.push((
                project_root.join(CORE_VERSION_FILE),
                project_root.join("cpa-core"),
            ));
        }
    }
    locations
}

pub(crate) fn bundled_core_archive() -> Result<Option<(BundledCoreInfo, PathBuf)>, String> {
    let platform = current_core_platform()?;
    let base_dir = core_base_dir()?;
    let executable_dir = executable_dir()?;
    let locations = bundled_core_locations(&base_dir, &executable_dir);

    let configured_version = locations.iter().find_map(|(version_path, _)| {
        fs::read_to_string(version_path)
            .ok()
            .map(|value| normalize_version(value.trim()))
            .filter(|value| value != "v")
    });
    if let Some(version) = configured_version {
        let asset_name = core_release_asset_name(&version, &platform);
        for (_, archive_dir) in &locations {
            let archive_path = archive_dir.join(&asset_name);
            if !archive_path.is_file() {
                continue;
            }
            let size_bytes = fs::metadata(&archive_path)
                .map_err(|error| format!("Failed to read bundled kernel information: {error}"))?
                .len();
            return Ok(Some((
                BundledCoreInfo {
                    version,
                    asset_name,
                    size_bytes,
                },
                archive_path,
            )));
        }
        return Ok(None);
    }

    let suffix = format!(
        "_{}_{}.{}",
        platform.asset_os, platform.asset_arch, platform.archive_kind
    );
    let mut matches = Vec::new();
    for (_, archive_dir) in &locations {
        if !archive_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(archive_dir)
            .map_err(|error| format!("Failed to read bundled kernel directory: {error}"))?
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.path().is_file()
                || !name.starts_with("CLIProxyAPI_")
                || !name.ends_with(&suffix)
                || name.contains("_no-plugin")
            {
                continue;
            }
            let Some(version) = name
                .strip_prefix("CLIProxyAPI_")
                .and_then(|value| value.strip_suffix(&suffix))
            else {
                continue;
            };
            let version = normalize_version(version);
            if matches.iter().any(|(existing, _, _)| existing == &version) {
                continue;
            }
            matches.push((version, name, entry.path()));
        }
    }
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    if matches.len() > 1 {
        return Err(format!(
            "Multiple bundled kernels match the current platform. Specify a release version in {}",
            CORE_VERSION_FILE
        ));
    }
    let Some((version, asset_name, archive_path)) = matches.pop() else {
        return Ok(None);
    };
    let size_bytes = fs::metadata(&archive_path)
        .map_err(|error| format!("Failed to read bundled kernel information: {error}"))?
        .len();
    Ok(Some((
        BundledCoreInfo {
            version,
            asset_name,
            size_bytes,
        },
        archive_path,
    )))
}

pub(crate) fn source_project_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|directory| {
        (directory.join("package.json").is_file() && directory.join("src-tauri").is_dir())
            .then(|| directory.to_path_buf())
    })
}

pub(crate) fn preserve_bundled_core_assets(
    source_dir: &Path,
    target_dir: &Path,
) -> Result<(), String> {
    if !source_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(source_dir)
        .map_err(|error| format!("Failed to read bundled kernel file: {error}"))?
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_archive = name.starts_with("CLIProxyAPI_")
            && (name.ends_with(".tar.gz") || name.ends_with(".zip"))
            && !name.contains("_no-plugin");
        if !is_archive && name != CORE_CHECKSUMS_FILE {
            continue;
        }
        fs::copy(&path, target_dir.join(&name))
            .map_err(|error| format!("Failed to preserve bundled kernel file {name}: {error}"))?;
    }
    Ok(())
}

pub(crate) fn preserve_selected_bundled_core_asset(
    archive_path: &Path,
    target_dir: &Path,
) -> Result<(), String> {
    let archive_name = archive_path
        .file_name()
        .ok_or_else(|| "Invalid bundled kernel archive filename".to_string())?;
    fs::copy(archive_path, target_dir.join(archive_name))
        .map_err(|error| format!("Failed to preserve selected bundled kernel archive: {error}"))?;
    if let Some(source_dir) = archive_path.parent() {
        let checksums = source_dir.join(CORE_CHECKSUMS_FILE);
        if checksums.is_file() {
            fs::copy(&checksums, target_dir.join(CORE_CHECKSUMS_FILE))
                .map_err(|error| format!("Failed to preserve bundled kernel checksum file: {error}"))?;
        }
    }
    Ok(())
}

pub(crate) fn migrate_core_config_for_update(
    source_dir: &Path,
    target_dir: &Path,
) -> Result<(), String> {
    if !source_dir.is_dir() {
        return Ok(());
    }
    let old_config_path = source_dir.join(CORE_CONFIG_FILE);
    let old_config = match fs::read_to_string(&old_config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Failed to read legacy kernel configuration; update canceled to prevent configuration loss {}: {error}",
                path_to_string(&old_config_path)
            ));
        }
    };

    let template_path = target_dir.join(CORE_EXAMPLE_CONFIG_FILE);
    if !template_path.is_file() {
        return Err(format!(
            "New kernel is missing a configuration template; update canceled: {}",
            path_to_string(&template_path)
        ));
    }
    let template = fs::read_to_string(&template_path).map_err(|error| {
        format!(
            "Failed to read new kernel configuration template {}: {error}",
            path_to_string(&template_path)
        )
    })?;
    let old_document = serde_norway::from_str::<serde_norway::Value>(&old_config)
        .map_err(|error| format!("Failed to parse existing kernel configuration; update canceled to prevent configuration loss: {error}"))?;
    if !old_document.is_mapping() {
        return Err("Existing kernel configuration root must be a YAML mapping; update canceled".to_string());
    }
    let template_document = serde_norway::from_str::<serde_norway::Value>(&template)
        .map_err(|error| format!("Failed to parse new kernel configuration template: {error}"))?;
    let migrated = if (core_config_declares_v8(&template_document) || core_config_uses_v8(&template_document))
        && !core_config_declares_v8(&old_document)
    {
        migrate_legacy_core_config_to_v8(&template, &old_config)?
    } else if core_config_uses_v8(&template_document) {
        old_config
    } else {
        merge_core_config_fields(&template, Some(&old_config))?
    };
    let config_path = target_dir.join(CORE_CONFIG_FILE);
    fs::write(&config_path, migrated).map_err(|error| {
        format!(
            "Failed to write migrated new kernel configuration {}: {error}",
            path_to_string(&config_path)
        )
    })?;
    Ok(())
}

pub(crate) fn validate_bundled_core_checksum(archive_path: &Path) -> Result<(), String> {
    let Some(directory) = archive_path.parent() else {
        return Err("Bundled kernel archive has no parent directory".to_string());
    };
    let checksums_path = directory.join(CORE_CHECKSUMS_FILE);
    if !checksums_path.is_file() {
        return Ok(());
    }
    let archive_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Invalid bundled kernel archive filename".to_string())?;
    let checksums = fs::read_to_string(&checksums_path)
        .map_err(|error| format!("Failed to read bundled kernel checksum file: {error}"))?;
    let expected = checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == archive_name && digest.len() == 64).then(|| digest.to_ascii_lowercase())
    });
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = sha256_file(archive_path)?;
    if actual != expected {
        return Err("Bundled kernel archive SHA-256 verification failed".to_string());
    }
    Ok(())
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Failed to open checksum file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read checksum file: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn read_core_metadata(install_dir: &Path) -> Option<CoreMetadata> {
    let metadata_path = install_dir.join(CORE_METADATA_FILE);
    let content = fs::read_to_string(metadata_path).ok()?;
    serde_json::from_str(&content).ok()
}

pub(crate) fn write_core_metadata(
    install_dir: &Path,
    metadata: &CoreMetadata,
) -> Result<(), String> {
    let metadata_path = install_dir.join(CORE_METADATA_FILE);
    let content = serde_json::to_string_pretty(metadata)
        .map_err(|err| format!("Failed to generate kernel metadata: {err}"))?;
    fs::write(metadata_path, content).map_err(|err| format!("Failed to write kernel metadata: {err}"))
}

pub(crate) fn validate_downloaded_asset(
    asset: &GithubAsset,
    downloaded: &DownloadedArchive,
) -> Result<(), String> {
    validate_download_metadata(
        downloaded.size,
        asset.size,
        &downloaded.sha256,
        asset.digest.as_deref(),
    )
}

pub(crate) fn validate_download_metadata(
    downloaded: u64,
    expected_total: Option<u64>,
    sha256: &str,
    expected_digest: Option<&str>,
) -> Result<(), String> {
    if let Some(expected_total) = expected_total {
        if downloaded != expected_total {
            return Err(format!(
                "Download size verification failed: got {downloaded} bytes, expected {expected_total} bytes"
            ));
        }
    }

    if let Some(expected_digest) = expected_digest {
        let expected = expected_digest
            .strip_prefix("sha256:")
            .unwrap_or(expected_digest)
            .to_ascii_lowercase();

        if !expected.is_empty() && sha256 != expected {
            return Err("Downloaded file SHA-256 verification failed".to_string());
        }
    }

    Ok(())
}

pub(crate) fn cleanup_core_work_dirs() -> Result<(), String> {
    let base_dir = core_base_dir()?;
    let mut last_error = None;

    for name in ["cpa-core.staging", "cpa-core.download"] {
        let path = base_dir.join(name);
        if path.exists() {
            if let Err(err) = fs::remove_dir_all(&path) {
                last_error = Some(format!("Failed to clean up temporary directory {}: {err}", path_to_string(&path)));
            }
        }
    }

    if let Some(error) = last_error {
        Err(error)
    } else {
        Ok(())
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn normalize_version(version: &str) -> String {
    let version = version.trim();

    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

pub(crate) fn is_core_running(binary_path: &Path) -> bool {
    !find_core_process_ids(binary_path).is_empty()
}

pub(crate) fn find_core_process_ids(binary_path: &Path) -> Vec<u32> {
    find_candidate_core_process_ids()
        .into_iter()
        .filter(|process_id| {
            process_executable_path(*process_id)
                .as_deref()
                .is_some_and(|path| executable_paths_match(binary_path, path))
        })
        .collect()
}

pub(crate) fn executable_paths_match(expected: &Path, actual: &Path) -> bool {
    let expected = fs::canonicalize(expected).unwrap_or_else(|_| expected.to_path_buf());
    let actual = fs::canonicalize(actual).unwrap_or_else(|_| actual.to_path_buf());

    #[cfg(windows)]
    {
        path_to_string(&expected).eq_ignore_ascii_case(&path_to_string(&actual))
    }

    #[cfg(not(windows))]
    {
        expected == actual
    }
}

#[cfg(all(not(target_os = "linux"), target_os = "windows"))]
pub(crate) fn find_candidate_core_process_ids() -> Vec<u32> {
    use std::{ffi::OsString, mem, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }

    let mut entry = unsafe { mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut process_ids = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            let name_length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            let image_name = OsString::from_wide(&entry.szExeFile[..name_length]);
            if entry.th32ProcessID != 0
                && image_name
                    .to_string_lossy()
                    .eq_ignore_ascii_case(core_binary_name())
            {
                process_ids.push(entry.th32ProcessID);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    process_ids
}

#[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
pub(crate) fn find_candidate_core_process_ids() -> Vec<u32> {
    Command::new("pgrep")
        .args(["-x", core_binary_name()])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
pub(crate) fn find_candidate_core_process_ids() -> Vec<u32> {
    Command::new("pgrep")
        .args(["-x", core_binary_name()])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
pub(crate) fn process_executable_path(process_id: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{process_id}/exe")).ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn process_executable_path(process_id: u32) -> Option<PathBuf> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffer_size: u32) -> i32;
    }

    let mut buffer = vec![0_u8; 4096];
    let length = unsafe {
        proc_pidpath(
            process_id.try_into().ok()?,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsString::from_vec(buffer)))
}

#[cfg(windows)]
pub(crate) fn process_executable_path(process_id: u32) -> Option<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    let success =
        unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) };
    unsafe { CloseHandle(handle as HANDLE) };
    if success == 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

pub(crate) fn adopt_existing_core_processes(
    process_state: &CoreProcessState,
) -> Result<Vec<u32>, String> {
    let install_dir = core_install_dir()?;
    let Some(binary_path) = find_core_binary(&install_dir) else {
        process_state.clear_adopted_processes()?;
        return Ok(Vec::new());
    };
    let process_ids = find_core_process_ids(&binary_path);
    process_state.adopt_process_ids(&binary_path, process_ids.clone())?;
    Ok(process_ids)
}

pub(crate) fn shutdown_managed_core(
    process_state: &CoreProcessState,
    gui_config_state: &GuiConfigState,
) {
    let was_running = process_state.managed_pid().is_some()
        || core_install_dir()
            .ok()
            .and_then(|install_dir| find_core_binary(&install_dir))
            .is_some_and(|binary_path| is_core_running(&binary_path));
    if was_running {
        if let Err(error) = stop_core_process_inner(process_state) {
            eprintln!("Failed to stop CPA kernel on exit: {error}");
        }
    }
    if let Err(error) = gui_config_state.set_run_on_startup(was_running) {
        eprintln!("Failed to save kernel state before exit: {error}");
    }
}

#[cfg(windows)]
pub(crate) fn attach_child_to_windows_job(child: &Child) -> Result<isize, String> {
    use std::{mem, os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return Err(format!(
                "Failed to create CPA kernel process job: {}",
                io::Error::last_os_error()
            ));
        }

        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if configured == 0 {
            let error = io::Error::last_os_error();
            CloseHandle(job);
            return Err(format!("Failed to configure CPA kernel process job: {error}"));
        }

        let process_handle = child.as_raw_handle() as HANDLE;
        if AssignProcessToJobObject(job, process_handle) == 0 {
            let error = io::Error::last_os_error();
            CloseHandle(job);
            return Err(format!("Failed to manage CPA kernel child process: {error}"));
        }

        Ok(job as isize)
    }
}

#[cfg(windows)]
pub(crate) fn close_windows_handle(handle: isize) {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

    unsafe {
        CloseHandle(handle as HANDLE);
    }
}

pub(crate) fn terminate_child(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("Failed to check CPA kernel process status: {error}"))?
        .is_some()
    {
        return Ok(());
    }
    #[cfg(windows)]
    {
        child
            .kill()
            .map_err(|err| format!("Failed to stop CPA kernel process: {err}"))?;
        child
            .wait()
            .map_err(|err| format!("Failed while waiting for CPA kernel process to exit: {err}"))?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let process_id = child.id();
        send_process_signal(process_id, "TERM")?;

        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(err) => return Err(format!("Failed to check CPA kernel process status: {err}")),
            }
        }

        child
            .kill()
            .map_err(|err| format!("Failed to force-stop CPA kernel process: {err}"))?;
        child
            .wait()
            .map_err(|err| format!("Failed while waiting for CPA kernel process to exit: {err}"))?;

        Ok(())
    }
}

pub(crate) fn terminate_process_id(process_id: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };

        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
        if handle.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                return Ok(());
            }
            return Err(format!("Failed to open CPA kernel process: PID {process_id}: {error}"));
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let raw_handle = handle.as_raw_handle();
        if unsafe { WaitForSingleObject(raw_handle, 0) } == WAIT_OBJECT_0 {
            return Ok(());
        }
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        let status = command
            .status()
            .map_err(|err| format!("Failed to stop CPA kernel process: {err}"))?;
        let wait =
            unsafe { WaitForSingleObject(raw_handle, if status.success() { 10_000 } else { 0 }) };
        match wait {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(format!("CPA kernel process did not exit: PID {process_id}")),
            _ => Err(format!(
                "Failed while waiting for CPA kernel process to exit: PID {process_id}: {}",
                io::Error::last_os_error()
            )),
        }
    }

    #[cfg(not(windows))]
    {
        send_process_signal(process_id, "TERM")?;

        for _ in 0..20 {
            if !is_process_alive(process_id) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }

        send_process_signal(process_id, "KILL")
    }
}

#[cfg(not(windows))]
pub(crate) fn send_process_signal(process_id: u32, signal: &str) -> Result<(), String> {
    let status = Command::new("kill")
        .args([format!("-{signal}"), process_id.to_string()])
        .status()
        .map_err(|err| format!("Failed to send process signal: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to send process signal: PID {process_id}"))
    }
}

#[cfg(windows)]
pub(crate) fn is_process_alive(process_id: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };

    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    let handle = unsafe { OpenProcess(SYNCHRONIZE_ACCESS, 0, process_id) };
    if handle.is_null() {
        return false;
    }
    let result = unsafe { WaitForSingleObject(handle, 0) };
    unsafe { CloseHandle(handle) };
    result == WAIT_TIMEOUT
}

#[cfg(not(windows))]
pub(crate) fn is_process_alive(process_id: u32) -> bool {
    let process_id = process_id.to_string();
    Command::new("kill")
        .args(["-0", &process_id])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn reset_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("Failed to clean up directory {}: {err}", path_to_string(path)))?;
    }

    fs::create_dir_all(path).map_err(|err| format!("Failed to create directory {}: {err}", path_to_string(path)))
}

const ROLLBACK_FILES_DIR: &str = "files";
const ROLLBACK_ADDED_FILES: &str = "added-files.txt";

/// Retain an interrupted update's backup before starting another transaction.
/// Never infer that a backup is disposable from the installed binary's presence:
/// it may be the broken replacement. History is deliberately not auto-deleted.
pub(crate) fn archive_previous_core_rollback(rollback_dir: &Path) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(rollback_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!(
            "Failed to inspect kernel rollback backup {}: {error}",
            path_to_string(rollback_dir)
        )),
        Ok(_) => {}
    }
    let history = rollback_dir.with_file_name("cpa-core.rollback-history");
    let archive_error = |error: io::Error| format!(
        "Failed to archive previous kernel rollback backup {}: {error}. The backup has been preserved; move it to a safe location before retrying the update",
        path_to_string(rollback_dir)
    );
    fs::create_dir_all(&history).map_err(&archive_error)?;
    // Reserve a unique directory exclusively, so even retries in the same second
    // cannot overwrite any earlier backup. Rename stays on the same filesystem.
    let timestamp = unix_now();
    for sequence in 0..10000 {
        let slot = history.join(format!("{timestamp}-{sequence}"));
        match fs::create_dir(&slot) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(archive_error(error)),
            Ok(()) => {}
        }
        let archived = slot.join("backup");
        if let Err(error) = fs::rename(rollback_dir, &archived) {
            let _ = fs::remove_dir(&slot);
            return Err(archive_error(error));
        }
        eprintln!("Previous kernel rollback backup preserved at {}", path_to_string(&archived));
        return Ok(Some(archived));
    }
    Err(format!("Could not reserve a unique kernel backup directory in {}", path_to_string(&history)))
}

fn ensure_core_rollback_available(rollback_dir: &Path) -> Result<(), String> {
    match fs::symlink_metadata(rollback_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect kernel rollback backup {}: {error}",
            path_to_string(rollback_dir)
        )),
        Ok(_) => Err(format!(
            "A previous kernel rollback backup is still present at {}. Restore it or move it to a safe location before retrying the update; it will not be overwritten",
            path_to_string(rollback_dir)
        )),
    }
}

fn cleanup_completed_core_rollback(rollback_dir: &Path) {
    if let Err(error) = fs::remove_dir_all(rollback_dir) {
        if error.kind() != io::ErrorKind::NotFound {
            eprintln!(
                "Kernel transaction completed, but failed to clean up backup {}: {error}. A subsequent update will archive it before proceeding",
                path_to_string(rollback_dir)
            );
        }
    }
}

fn collect_relative_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir)
        .map_err(|err| format!("Failed to read directory {}: {err}", path_to_string(dir)))?
    {
        let entry = entry.map_err(|err| format!("Failed to read directory entry: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("Failed to read directory entry type: {err}"))?;
        if file_type.is_dir() {
            collect_relative_files(root, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|err| format!("Failed to compute relative path: {err}"))?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Backs up every installed file that the staged package is about to overwrite and records
/// the files it will add, so a rollback restores the previous installation exactly.
/// Returns `false` when there is no installed kernel to roll back to.
pub(crate) fn backup_core_for_rollback(
    install_dir: &Path,
    staging_dir: &Path,
    rollback_dir: &Path,
) -> Result<bool, String> {
    // Check even if the installed binary is missing: the retained backup may be
    // the only remaining copy of the previous working installation.
    ensure_core_rollback_available(rollback_dir)?;
    let Some(binary_path) = find_core_binary(install_dir) else {
        return Ok(false);
    };
    // Exclusive creation also protects a backup that appears after the check.
    fs::create_dir(rollback_dir).map_err(|error| format!(
        "Failed to create kernel rollback backup {} without overwriting existing files: {error}",
        path_to_string(rollback_dir)
    ))?;
    let files_dir = rollback_dir.join(ROLLBACK_FILES_DIR);

    let mut staged_files = Vec::new();
    collect_relative_files(staging_dir, staging_dir, &mut staged_files)?;
    let installed_binary = binary_path
        .strip_prefix(install_dir)
        .map_err(|err| format!("Failed to compute rollback binary relative path: {err}"))?
        .to_path_buf();
    if !staged_files.contains(&installed_binary) {
        staged_files.push(installed_binary);
    }

    let mut added_files = Vec::new();
    for relative in staged_files {
        let source = install_dir.join(&relative);
        if !source.is_file() {
            added_files.push(path_to_string(&relative));
            continue;
        }
        let target = files_dir.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create rollback directory: {err}"))?;
        }
        copy_core_file_replace(&source, &target)?;
    }
    fs::create_dir_all(&files_dir)
        .map_err(|err| format!("Failed to create rollback directory: {err}"))?;
    fs::write(rollback_dir.join(ROLLBACK_ADDED_FILES), added_files.join("\n"))
        .map_err(|err| format!("Failed to write rollback file list: {err}"))?;
    Ok(true)
}

pub(crate) fn restore_core_rollback(
    install_dir: &Path,
    rollback_dir: &Path,
) -> Result<(), String> {
    let files_dir = rollback_dir.join(ROLLBACK_FILES_DIR);
    if !files_dir.is_dir() {
        return Err(format!(
            "Kernel rollback directory does not exist: {}",
            path_to_string(rollback_dir)
        ));
    }
    overlay_directory(&files_dir, install_dir)?;
    let added_files = fs::read_to_string(rollback_dir.join(ROLLBACK_ADDED_FILES)).unwrap_or_default();
    for relative in added_files.lines().filter(|line| !line.is_empty()) {
        let added = install_dir.join(relative);
        match fs::remove_file(&added) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "Failed to remove file added by the failed update {}: {err}",
                    path_to_string(&added)
                ));
            }
        }
        // Drop directories the update created; `remove_dir` refuses non-empty ones.
        let mut parent = added.parent();
        while let Some(dir) = parent.filter(|dir| *dir != install_dir) {
            if fs::remove_dir(dir).is_err() {
                break;
            }
            parent = dir.parent();
        }
    }
    cleanup_completed_core_rollback(rollback_dir);
    Ok(())
}

pub(crate) fn apply_staged_core_with_rollback<F>(
    install_dir: &Path,
    staging_dir: &Path,
    rollback_dir: &Path,
    verify_or_restart: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    let staged_binary = find_core_binary(staging_dir)
        .ok_or_else(|| "Staged kernel directory is missing a CPA kernel binary".to_string())?;
    let staged_size = fs::metadata(&staged_binary)
        .map_err(|err| format!("Failed to read staged kernel binary metadata: {err}"))?
        .len();
    if staged_size == 0 {
        return Err("Staged CPA kernel binary is empty".to_string());
    }

    let had_backup = backup_core_for_rollback(install_dir, staging_dir, rollback_dir)?;
    if let Err(overlay_error) = overlay_install_dir(install_dir, staging_dir) {
        if had_backup {
            if let Err(rollback_error) = restore_core_rollback(install_dir, rollback_dir) {
                return Err(format!(
                    "{overlay_error}; restoring the previous version also failed: {rollback_error}. The previous kernel files are kept in {}",
                    path_to_string(rollback_dir)
                ));
            }
        }
        return Err(overlay_error);
    }

    match verify_or_restart() {
        Ok(()) => {
            cleanup_completed_core_rollback(rollback_dir);
            Ok(())
        }
        Err(restart_error) if had_backup => match restore_core_rollback(install_dir, rollback_dir) {
            Ok(()) => Err(format!(
                "Kernel update failed to start and was rolled back to the previous version: {restart_error}"
            )),
            Err(rollback_error) => Err(format!(
                "Kernel update failed to start: {restart_error}; restoring the previous version also failed: {rollback_error}. The previous kernel files are kept in {}",
                path_to_string(rollback_dir)
            )),
        },
        Err(restart_error) => Err(format!(
            "Kernel installed, but failed to automatically resume operation: {restart_error}"
        )),
    }
}

pub(crate) fn overlay_install_dir(install_dir: &Path, staging_dir: &Path) -> Result<(), String> {
    if !staging_dir.is_dir() {
        return Err(format!(
            "Kernel staging directory does not exist: {}",
            path_to_string(staging_dir)
        ));
    }

    if !install_dir.exists() {
        return fs::rename(staging_dir, install_dir)
            .map_err(|err| format!("Failed to install new kernel directory: {err}"));
    }
    if !install_dir.is_dir() {
        return Err(format!(
            "Kernel installation path is not a directory: {}",
            path_to_string(install_dir)
        ));
    }

    overlay_directory(staging_dir, install_dir)?;
    fs::remove_dir_all(staging_dir).map_err(|err| format!("Failed to clean up kernel staging directory: {err}"))
}

fn overlay_directory(source_dir: &Path, target_dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source_dir)
        .map_err(|err| format!("Failed to read kernel staging directory {}: {err}", path_to_string(source_dir)))?
    {
        let entry = entry.map_err(|err| format!("Failed to read kernel staging entry: {err}"))?;
        let source_path = entry.path();
        let target_path = target_dir.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("Failed to read kernel staging entry type: {err}"))?;

        if file_type.is_dir() {
            if target_path.exists() && !target_path.is_dir() {
                return Err(format!(
                    "Cannot overwrite a same-named file with a kernel directory: {}",
                    path_to_string(&target_path)
                ));
            }
            fs::create_dir_all(&target_path).map_err(|err| {
                format!(
                    "Failed to create kernel installation subdirectory {}: {err}",
                    path_to_string(&target_path)
                )
            })?;
            overlay_directory(&source_path, &target_path)?;
        } else if file_type.is_file() {
            if target_path.exists() && !target_path.is_file() {
                return Err(format!(
                    "Cannot overwrite a same-named directory with a kernel file: {}",
                    path_to_string(&target_path)
                ));
            }
            copy_core_file_replace(&source_path, &target_path)?;
        } else {
            return Err(format!(
                "Kernel staging directory contains an unsupported entry: {}",
                path_to_string(&source_path)
            ));
        }
    }

    Ok(())
}

pub(crate) fn copy_core_file_replace(source_path: &Path, target_path: &Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = target_path.parent().ok_or_else(|| {
        format!(
            "Failed to overwrite kernel file {}: cannot determine parent directory",
            path_to_string(target_path)
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create kernel file directory {}: {error}", path_to_string(parent)))?;
    let file_name = target_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "cpa-core".into());
    let temporary_path = parent.join(format!(
        ".{file_name}.replace.{}.{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let replace_result = (|| -> io::Result<()> {
        fs::copy(source_path, &temporary_path)?;
        let temporary_file = File::options().write(true).open(&temporary_path)?;
        temporary_file.sync_all()?;
        drop(temporary_file);
        replace_file_atomically(&temporary_path, target_path)
    })();

    if let Err(error) = replace_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!(
            "Failed to overwrite kernel file {}: {error}",
            path_to_string(target_path)
        ));
    }

    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn rematerialize_core_binary(binary_path: &Path) -> Result<(), String> {
    copy_core_file_replace(binary_path, binary_path)
}

pub(crate) fn extract_tar_gz(archive_path: &Path, install_dir: &Path) -> Result<(), String> {
    let archive_file =
        File::open(archive_path).map_err(|err| format!("Failed to open tar.gz: {err}"))?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| format!("Failed to read tar.gz entry: {err}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|err| format!("Failed to read tar.gz entry: {err}"))?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("Failed to read tar.gz entry path: {err}"))?;
        let out_path = checked_archive_path(install_dir, entry_path.as_ref())?;
        let entry_type = entry.header().entry_type();

        if entry_type.is_dir() {
            fs::create_dir_all(&out_path).map_err(|err| format!("Failed to create directory: {err}"))?;
        } else if entry_type.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|err| format!("Failed to create directory: {err}"))?;
            }
            entry
                .unpack(&out_path)
                .map_err(|err| format!("Failed to extract tar.gz file: {err}"))?;
        } else {
            return Err(format!(
                "tar.gz contains an unsupported entry type: {}",
                path_to_string(&out_path)
            ));
        }
    }

    Ok(())
}

pub(crate) fn extract_zip(archive_path: &Path, install_dir: &Path) -> Result<(), String> {
    let archive_file = File::open(archive_path).map_err(|err| format!("Failed to open zip: {err}"))?;
    let mut archive =
        ZipArchive::new(archive_file).map_err(|err| format!("Failed to read zip: {err}"))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| format!("Failed to read zip entry: {err}"))?;
        let enclosed_name = file
            .enclosed_name()
            .ok_or_else(|| format!("Unsafe zip entry path: {}", file.name()))?;
        let out_path = checked_archive_path(install_dir, &enclosed_name)?;

        if is_zip_symlink(&file) {
            return Err(format!("zip contains an unsupported symbolic link entry: {}", file.name()));
        }

        if file.is_dir() {
            fs::create_dir_all(&out_path).map_err(|err| format!("Failed to create directory: {err}"))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("Failed to create directory: {err}"))?;
        }

        let mut out_file = File::create(&out_path).map_err(|err| format!("Failed to create file: {err}"))?;
        io::copy(&mut file, &mut out_file).map_err(|err| format!("Failed to write file: {err}"))?;

        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))
                .map_err(|err| format!("Failed to set file permissions: {err}"))?;
        }
    }

    Ok(())
}

pub(crate) fn checked_archive_path(base_dir: &Path, entry_path: &Path) -> Result<PathBuf, String> {
    if entry_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "Unsafe archive entry path: {}",
            path_to_string(entry_path)
        ));
    }

    Ok(base_dir.join(entry_path))
}

pub(crate) fn is_zip_symlink(file: &zip::read::ZipFile<'_>) -> bool {
    file.unix_mode()
        .map(|mode| mode & 0o170000 == 0o120000)
        .unwrap_or(false)
}

pub(crate) fn find_core_binary(install_dir: &Path) -> Option<PathBuf> {
    let binary_path = install_dir.join(core_binary_name());
    if binary_path.is_file() {
        return Some(binary_path);
    }

    let mut dirs = vec![install_dir.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        let entries = fs::read_dir(dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .map(|file_name| file_name == core_binary_name())
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }

    None
}

pub(crate) fn core_binary_name() -> &'static str {
    if env::consts::OS == "windows" {
        "cli-proxy-api.exe"
    } else {
        "cli-proxy-api"
    }
}

pub(crate) fn should_start_hidden(config: &GuiConfigFile) -> bool {
    config.silent_start
        && cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        ))
}

pub(crate) fn should_start_core_on_launch(config: &GuiConfigFile) -> bool {
    config.start_core_on_launch
}

pub(crate) fn configure_initial_main_window(
    app_handle: &tauri::AppHandle,
    start_hidden: bool,
) -> Result<(), String> {
    let window = main_window(app_handle).ok_or_else(|| "Main window does not exist".to_string())?;

    #[cfg(target_os = "macos")]
    set_macos_dock_visible(app_handle, !start_hidden);

    #[cfg(target_os = "linux")]
    let start_hidden = if start_hidden && !linux_tray_available(app_handle) {
        eprintln!(
            "System tray is unavailable; showing the main window instead of starting hidden"
        );
        false
    } else {
        start_hidden
    };

    if start_hidden {
        return window
            .hide()
            .map_err(|error| format!("Failed to hide main window during silent startup: {error}"));
    }

    window
        .show()
        .map_err(|error| format!("Failed to show main window: {error}"))?;
    if window.is_minimized().unwrap_or(false) {
        window
            .unminimize()
            .map_err(|error| format!("Failed to restore main window: {error}"))?;
    }
    window
        .set_focus()
        .map_err(|error| format!("Failed to focus main window: {error}"))
}

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
