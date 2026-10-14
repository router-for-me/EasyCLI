use super::support::*;
use super::*;

#[test]
fn retry_must_preserve_backup_from_failed_rollback() {
    let root = agent_test_home("review-preserve-failed-rollback");
    let install = root.join("cpa-core");
    let staging = root.join("cpa-core.staging");
    let rollback = root.join("cpa-core.rollback");
    fs::create_dir_all(&install).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(install.join(core_binary_name()), b"working-original").unwrap();
    fs::write(staging.join(core_binary_name()), b"broken-update").unwrap();
    backup_core_for_rollback(&install, &staging, &rollback).unwrap();
    // A failed restore leaves the new binary installed and the original in rollback.
    fs::write(install.join(core_binary_name()), b"broken-update").unwrap();
    let error = apply_staged_core_with_rollback(&install, &staging, &rollback, || {
        panic!("a blocked update must not restart the kernel")
    }).unwrap_err();
    assert!(error.contains("will not be overwritten"));
    assert_eq!(fs::read(install.join(core_binary_name())).unwrap(), b"broken-update");
    assert!(staging.is_dir());

    // A missing installed binary must not allow a subsequent successful update
    // to delete the only remaining backup either.
    fs::remove_file(install.join(core_binary_name())).unwrap();
    assert!(apply_staged_core_with_rollback(&install, &staging, &rollback, || {
        panic!("a missing installed binary must not bypass backup protection")
    }).is_err());
    let retained = fs::read(rollback.join("files").join(core_binary_name())).unwrap();
    fs::remove_dir_all(root).unwrap();
    assert_eq!(retained, b"working-original", "retry destroyed the recoverable original binary");
}

#[test]
fn restored_rollback_allows_a_subsequent_successful_update() {
    let root = agent_test_home("rollback-restore-then-retry");
    let install = root.join("cpa-core");
    let staging = root.join("cpa-core.staging");
    let rollback = root.join("cpa-core.rollback");
    fs::create_dir_all(&install).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(install.join(core_binary_name()), b"working-original").unwrap();
    fs::write(staging.join(core_binary_name()), b"working-update").unwrap();
    backup_core_for_rollback(&install, &staging, &rollback).unwrap();
    assert!(backup_core_for_rollback(&install, &staging, &rollback).is_err());
    fs::write(install.join(core_binary_name()), b"broken-update").unwrap();
    restore_core_rollback(&install, &rollback).unwrap();
    assert_eq!(fs::read(install.join(core_binary_name())).unwrap(), b"working-original");
    assert!(!rollback.exists());
    apply_staged_core_with_rollback(&install, &staging, &rollback, || Ok(())).unwrap();
    assert_eq!(fs::read(install.join(core_binary_name())).unwrap(), b"working-update");
    assert!(!rollback.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn core_process_starting_state_tracks_automatic_launch() {
    let state = CoreProcessState::new(true);
    assert!(state.is_starting());
    state.set_starting(false);
    assert!(!state.is_starting());
}

#[test]
fn executable_path_matching_keeps_core_instances_directory_scoped() {
    let root = agent_test_home("core-process-path-scope");
    let first_dir = root.join("first").join("cpa-core");
    let second_dir = root.join("second").join("cpa-core");
    fs::create_dir_all(&first_dir).unwrap();
    fs::create_dir_all(&second_dir).unwrap();
    let first_binary = first_dir.join(core_binary_name());
    let second_binary = second_dir.join(core_binary_name());
    fs::write(&first_binary, b"first").unwrap();
    fs::write(&second_binary, b"second").unwrap();

    assert!(executable_paths_match(&first_binary, &first_binary));
    assert!(!executable_paths_match(&first_binary, &second_binary));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn core_process_discovery_sleep_helper() {
    if env::var_os("EASYCLIPROXYAPI_PROCESS_DISCOVERY_TEST_HELPER").is_some() {
        thread::sleep(Duration::from_secs(10));
    }
}

fn core_child_sleep_command() -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "tests::core_runtime::core_process_discovery_sleep_helper",
            "--nocapture",
        ])
        .env("EASYCLIPROXYAPI_PROCESS_DISCOVERY_TEST_HELPER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    command
}

fn assert_core_child_survives(child: &mut Child) {
    thread::sleep(Duration::from_millis(200));
    let status = child.try_wait();
    let _ = child.kill();
    let _ = child.wait();
    assert!(status.unwrap().is_none(), "core exited with its launcher");
}

#[test]
fn core_child_survives_launcher_thread_exit() {
    let mut child = thread::spawn(|| spawn_core_child(core_child_sleep_command()).unwrap())
        .join()
        .unwrap();
    assert_core_child_survives(&mut child);
}

#[test]
fn core_child_survives_blocking_runtime_shutdown() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut child = runtime
        .block_on(runtime.spawn_blocking(|| spawn_core_child(core_child_sleep_command()).unwrap()))
        .unwrap();
    drop(runtime);
    assert_core_child_survives(&mut child);
}

#[test]
fn core_child_spawner_remains_available_after_spawn_failure() {
    let missing_binary = agent_test_home("missing-core-spawner-binary").join(core_binary_name());
    assert!(spawn_core_child_on_lifetime_thread(Command::new(missing_binary)).is_err());
    let mut child =
        thread::spawn(|| spawn_core_child_on_lifetime_thread(core_child_sleep_command()).unwrap())
            .join()
            .unwrap();
    assert_core_child_survives(&mut child);
}

#[test]
fn exiting_during_core_startup_cancels_the_port_wait() {
    let state = std::sync::Arc::new(CoreProcessState::new(true));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut child = spawn_core_child(core_child_sleep_command()).unwrap();
    let child_id = child.id();
    let exiting_state = state.clone();
    let exit = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        exiting_state.shutting_down.store(true, Ordering::Release);
    });

    let started = Instant::now();
    let result = wait_for_core_management_port(&mut child, address, &state);
    drop(child);
    exit.join().unwrap();
    assert!(matches!(result, Err(CoreStartupFailure::ShuttingDown)));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!is_process_alive(child_id));
}

#[test]
fn exiting_rejects_new_core_operations_and_cleans_up_an_in_flight_child() {
    let state = CoreProcessState::new(false);
    let child = spawn_core_child(core_child_sleep_command()).unwrap();
    let child_id = child.id();
    state.shutting_down.store(true, Ordering::Release);

    assert!(lock_core_operation(&state)
        .unwrap_err()
        .contains("Application is exiting"));
    assert!(state
        .store_child(child)
        .unwrap_err()
        .contains("Application is exiting"));
    assert!(!is_process_alive(child_id));
    assert_eq!(state.managed_pid(), None);
}

#[cfg(windows)]
#[test]
fn core_config_file_lock_helper() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let Some(path) = env::var_os("EASYCLIPROXYAPI_CORE_FILE_LOCK_TEST_HELPER") else {
        return;
    };
    let _file = File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)
        .unwrap();
    println!("CORE_FILE_LOCKED");
    io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(10));
}

#[cfg(windows)]
#[test]
fn updating_stops_an_adopted_core_without_a_port_before_replacing_its_config() {
    use std::io::BufRead;

    let root = agent_test_home("core-stop-before-replace");
    let config_path = root.join(CORE_CONFIG_FILE);
    let replacement = root.join("new-config.yaml");
    fs::write(&config_path, b"old config").unwrap();
    fs::write(&replacement, b"new config").unwrap();
    let binary_path = env::current_exe().unwrap();
    let mut command = Command::new(&binary_path);
    command
        .args([
            "--exact",
            "tests::core_runtime::core_config_file_lock_helper",
            "--nocapture",
        ])
        .env("EASYCLIPROXYAPI_CORE_FILE_LOCK_TEST_HELPER", &config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let mut child = command.spawn().unwrap();
    let output = io::BufReader::new(child.stdout.take().unwrap());
    let ready = output
        .lines()
        .any(|line| line.unwrap() == "CORE_FILE_LOCKED");
    let state = CoreProcessState::new(false);
    state
        .adopt_process_ids(&binary_path, vec![child.id()])
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let status = current_core_status(Some(&state), Some(port)).unwrap();
    let locked_result = copy_core_file_replace(&replacement, &config_path);
    let stopped = pause_core_process_for_install(&state);
    let exited = child.try_wait().unwrap().is_some();
    let replace_result = copy_core_file_replace(&replacement, &config_path);
    let _ = child.kill();
    let _ = child.wait();

    assert!(ready);
    assert!(
        status.running,
        "the tracked orphan remains a running process without a listening management port"
    );
    assert!(!status.ready, "the orphan is not management-ready");
    assert!(
        locked_result.is_err(),
        "the orphan must hold a real Windows file lock"
    );
    assert!(
        stopped.unwrap(),
        "the update must stop the orphan despite the closed port"
    );
    assert!(
        exited,
        "stopping must wait until the process has actually exited"
    );
    replace_result.unwrap();
    assert_eq!(fs::read(&config_path).unwrap(), b"new config");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn core_child_owner_process_helper() {
    if env::var_os("EASYCLIPROXYAPI_CORE_OWNER_TEST_HELPER").is_none() {
        return;
    }
    let mut command = core_child_sleep_command();
    command.stdout(Stdio::inherit());
    let _child = spawn_core_child(command).unwrap();
    println!("CORE_CHILD_READY");
    io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(10));
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn core_child_stops_when_owner_process_is_killed() {
    use std::io::BufRead;

    let mut owner = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::core_runtime::core_child_owner_process_helper",
            "--nocapture",
        ])
        .env("EASYCLIPROXYAPI_CORE_OWNER_TEST_HELPER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut output = io::BufReader::new(owner.stdout.take().unwrap());
    let mut ready = false;
    loop {
        let mut line = String::new();
        match output.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line.trim() == "CORE_CHILD_READY" => {
                ready = true;
                break;
            }
            Ok(_) => {}
        }
    }

    let killed = owner.kill();
    let waited = owner.wait();
    let shutdown_started = Instant::now();
    let mut remaining_output = String::new();
    let drained = output.read_to_string(&mut remaining_output);
    assert!(ready, "owner did not finish spawning its core child");
    killed.unwrap();
    waited.unwrap();
    drained.unwrap();
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(5),
        "core kept its output pipe open after the owner was killed"
    );
}

#[test]
fn running_core_process_discovery_ignores_the_same_binary_name_in_another_directory() {
    let root = agent_test_home("running-core-process-scope");
    let first_dir = root.join("first").join("cpa-core");
    let second_dir = root.join("second").join("cpa-core");
    fs::create_dir_all(&first_dir).unwrap();
    fs::create_dir_all(&second_dir).unwrap();
    let first_binary = first_dir.join(core_binary_name());
    let second_binary = second_dir.join(core_binary_name());

    let source_binary = env::current_exe().unwrap();
    fs::copy(&source_binary, &first_binary).unwrap();
    fs::copy(&source_binary, &second_binary).unwrap();
    let arguments = [
        "--exact",
        "tests::core_runtime::core_process_discovery_sleep_helper",
        "--nocapture",
    ];
    let mut first = Command::new(&first_binary)
        .args(&arguments)
        .env("EASYCLIPROXYAPI_PROCESS_DISCOVERY_TEST_HELPER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut second = Command::new(&second_binary)
        .args(&arguments)
        .env("EASYCLIPROXYAPI_PROCESS_DISCOVERY_TEST_HELPER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(200));

    let first_running = first.try_wait().unwrap().is_none();
    let second_running = second.try_wait().unwrap().is_none();
    let candidate_process_ids = find_candidate_core_process_ids();
    let first_actual_path = process_executable_path(first.id());
    let second_actual_path = process_executable_path(second.id());
    let first_matches = find_core_process_ids(&first_binary);
    let second_matches = find_core_process_ids(&second_binary);

    let _ = first.kill();
    let _ = second.kill();
    let _ = first.wait();
    let _ = second.wait();
    assert_eq!(
        first_matches,
        vec![first.id()],
        "running={first_running}, candidate={}, actual={first_actual_path:?}, expected={first_binary:?}",
        candidate_process_ids.contains(&first.id())
    );
    assert_eq!(
        second_matches,
        vec![second.id()],
        "running={second_running}, candidate={}, actual={second_actual_path:?}, expected={second_binary:?}",
        candidate_process_ids.contains(&second.id())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_process_executable_path_can_be_resolved() {
    let expected = env::current_exe().unwrap();
    let actual = process_executable_path(std::process::id()).unwrap();
    assert!(executable_paths_match(&expected, &actual));
}

#[test]
fn core_process_state_tracks_and_releases_adopted_processes() {
    let state = CoreProcessState::new(false);
    let binary_path = env::current_exe().unwrap();
    state
        .adopt_process_ids(&binary_path, vec![std::process::id(), std::process::id()])
        .unwrap();

    assert_eq!(state.managed_pid(), Some(std::process::id()));
    assert_eq!(
        state
            .take_adopted_processes()
            .into_iter()
            .map(|process| process.process_id)
            .collect::<Vec<_>>(),
        vec![std::process::id()]
    );
    assert_eq!(state.managed_pid(), None);
}

#[test]
fn tracked_core_stays_running_when_a_management_port_probe_misses() {
    let state = CoreProcessState::new(false);
    let binary_path = env::current_exe().unwrap();
    let process_id = std::process::id();
    state
        .adopt_process_ids(&binary_path, vec![process_id])
        .unwrap();

    // Port zero cannot be the configured management endpoint. This models a
    // transient failed health probe while the tracked process is still alive.
    let status = current_core_status(Some(&state), Some(0)).unwrap();
    state.clear_adopted_processes().unwrap();

    assert!(status.running);
    assert!(!status.ready);
    assert_eq!(status.process_id, Some(process_id));
}

#[test]
fn tracked_core_is_ready_when_its_management_port_accepts_connections() {
    let state = CoreProcessState::new(false);
    let binary_path = env::current_exe().unwrap();
    let process_id = std::process::id();
    state
        .adopt_process_ids(&binary_path, vec![process_id])
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let status = current_core_status(Some(&state), Some(port)).unwrap();
    state.clear_adopted_processes().unwrap();

    assert!(status.running);
    assert!(status.ready);
    assert_eq!(status.process_id, Some(process_id));
}

#[test]
fn successful_core_install_remains_successful_when_restart_succeeds() {
    let result = combine_install_and_restart_results(Ok("installed"), Ok(()));
    assert_eq!(result.unwrap(), "installed");
}

#[test]
fn successful_core_install_reports_automatic_restart_failure() {
    let result = combine_install_and_restart_results(Ok("installed"), Err("port busy".into()));
    assert_eq!(
        result.unwrap_err(),
        "Kernel installed, but failed to automatically resume operation: port busy"
    );
}

#[test]
fn failed_core_install_keeps_the_install_error_after_runtime_is_restored() {
    let result =
        combine_install_and_restart_results::<()>(Err("download cancelled".into()), Ok(()));
    assert_eq!(result.unwrap_err(), "download cancelled");
}

#[test]
fn failed_core_install_reports_restart_failure_too() {
    let result = combine_install_and_restart_results::<()>(
        Err("checksum mismatch".into()),
        Err("port busy".into()),
    );
    assert_eq!(
        result.unwrap_err(),
        "checksum mismatch; failed to automatically restore the previous kernel running state: port busy"
    );
}

#[test]
fn replacing_a_core_preserves_only_regular_bundled_assets() {
    let root = agent_test_home("bundled-assets");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(
        source.join("CLIProxyAPI_7.2.83_linux_amd64.tar.gz"),
        b"archive",
    )
    .unwrap();
    fs::write(
        source.join("CLIProxyAPI_7.2.83_linux_amd64_no-plugin.tar.gz"),
        b"portable",
    )
    .unwrap();
    fs::write(source.join(CORE_CHECKSUMS_FILE), b"checksums").unwrap();

    preserve_bundled_core_assets(&source, &target).unwrap();

    assert!(target
        .join("CLIProxyAPI_7.2.83_linux_amd64.tar.gz")
        .is_file());
    assert!(!target
        .join("CLIProxyAPI_7.2.83_linux_amd64_no-plugin.tar.gz")
        .exists());
    assert!(target.join(CORE_CHECKSUMS_FILE).is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlaying_a_core_updates_packaged_files_and_preserves_plugins() {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let root = agent_test_home("core-overlay-preserves-plugins");
    let install_dir = root.join("cpa-core");
    let staging_dir = root.join("cpa-core.staging");
    fs::create_dir_all(install_dir.join("plugins/custom-router")).unwrap();
    fs::create_dir_all(staging_dir.join("plugins/bundled-router")).unwrap();
    fs::create_dir_all(staging_dir.join("runtime")).unwrap();
    fs::write(install_dir.join(core_binary_name()), b"old core").unwrap();
    fs::write(install_dir.join("README.md"), b"old readme").unwrap();
    fs::write(install_dir.join("user-data.json"), b"user data").unwrap();
    fs::write(
        install_dir.join("plugins/custom-router/plugin.js"),
        b"custom plugin",
    )
    .unwrap();
    fs::write(staging_dir.join(core_binary_name()), b"new core").unwrap();
    fs::write(staging_dir.join("README.md"), b"new readme").unwrap();
    fs::write(
        staging_dir.join("plugins/bundled-router/plugin.js"),
        b"bundled plugin",
    )
    .unwrap();
    fs::write(staging_dir.join("runtime/default.json"), b"new runtime").unwrap();
    #[cfg(unix)]
    let original_binary_inode = fs::metadata(install_dir.join(core_binary_name()))
        .unwrap()
        .ino();

    overlay_install_dir(&install_dir, &staging_dir).unwrap();

    assert_eq!(
        fs::read(install_dir.join(core_binary_name())).unwrap(),
        b"new core"
    );
    assert_eq!(
        fs::read(install_dir.join("README.md")).unwrap(),
        b"new readme"
    );
    assert_eq!(
        fs::read(install_dir.join("runtime/default.json")).unwrap(),
        b"new runtime"
    );
    assert_eq!(
        fs::read(install_dir.join("plugins/custom-router/plugin.js")).unwrap(),
        b"custom plugin"
    );
    assert_eq!(
        fs::read(install_dir.join("plugins/bundled-router/plugin.js")).unwrap(),
        b"bundled plugin"
    );
    assert_eq!(
        fs::read(install_dir.join("user-data.json")).unwrap(),
        b"user data"
    );
    #[cfg(unix)]
    assert_ne!(
        fs::metadata(install_dir.join(core_binary_name()))
            .unwrap()
            .ino(),
        original_binary_inode,
        "The updated kernel must use a new inode"
    );
    assert!(!staging_dir.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn installing_a_core_into_a_missing_directory_moves_the_complete_staging_tree() {
    let root = agent_test_home("core-overlay-first-install");
    let install_dir = root.join("cpa-core");
    let staging_dir = root.join("cpa-core.staging");
    fs::create_dir_all(&staging_dir).unwrap();
    fs::write(staging_dir.join(core_binary_name()), b"new core").unwrap();
    fs::write(staging_dir.join(CORE_EXAMPLE_CONFIG_FILE), b"port: 8317\n").unwrap();

    overlay_install_dir(&install_dir, &staging_dir).unwrap();

    assert_eq!(
        fs::read(install_dir.join(core_binary_name())).unwrap(),
        b"new core"
    );
    assert!(install_dir.join(CORE_EXAMPLE_CONFIG_FILE).is_file());
    assert!(!staging_dir.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacing_a_core_migrates_old_fields_into_the_new_template() {
    let root = agent_test_home("core-config-migrate");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let old_config = "# Old comment\nhost: 127.0.0.1\nport: 9527\nnested:\n  keep: old\n  old-only: retained\nlist:\n  - old-a\n  - old-b\nextra: true\n";
    let new_template = "# New template\nhost: \"\"\nport: 8317\nnested:\n  keep: new-default\n  added: new-field\nlist:\n  - new-default\nnew-option: true\n";
    fs::write(source.join(CORE_CONFIG_FILE), old_config).unwrap();
    fs::write(target.join(CORE_EXAMPLE_CONFIG_FILE), new_template).unwrap();

    migrate_core_config_for_update(&source, &target).unwrap();

    let migrated = fs::read_to_string(target.join(CORE_CONFIG_FILE)).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&migrated).unwrap();
    assert!(migrated.contains("# New template"));
    assert_eq!(document["host"], "127.0.0.1");
    assert_eq!(document["port"], 9527);
    assert_eq!(document["nested"]["keep"], "old");
    assert_eq!(document["nested"]["added"], "new-field");
    assert_eq!(document["nested"]["old-only"], "retained");
    assert_eq!(document["list"][0], "old-a");
    assert_eq!(document["list"][1], "old-b");
    assert_eq!(document["new-option"], true);
    assert_eq!(document["extra"], true);
    assert_eq!(
        fs::read_to_string(target.join(CORE_EXAMPLE_CONFIG_FILE)).unwrap(),
        new_template
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacing_v7_with_v8_lifts_legacy_values_into_the_new_layout() {
    let root = agent_test_home("core-config-v8-preserve");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let old_config = "# User v7 config\nhost: 127.0.0.1\nport: 9527\nremote-management:\n  secret-key: user-secret\napi-keys: [client-key]\nproxy-url: direct\nrouting:\n  strategy: fill-first\n  session-affinity: true\n  session-affinity-ttl: 2h\nplugins:\n  configs:\n    local:\n      enabled: true\n  store-auth:\n    registry: stored-token\n";
    let v8_template = "config-version: 8\nserver: {host: '', port: 8317}\nmanagement: {secret-key: ''}\naccess: {api-keys: [template-key]}\napi-keys: {codex: [{name: template-upstream, keys: []}]}\nrequests: {proxy-url: ''}\nrouting: {strategy: round-robin, session-affinity: false, session-affinity-ttl: 30m}\nplugins: {configs: {}, store-auth: {}}\n";
    fs::write(source.join(CORE_CONFIG_FILE), old_config).unwrap();
    fs::write(target.join(CORE_EXAMPLE_CONFIG_FILE), v8_template).unwrap();

    migrate_core_config_for_update(&source, &target).unwrap();

    let migrated = fs::read_to_string(target.join(CORE_CONFIG_FILE)).unwrap();
    let document = serde_norway::from_str::<serde_norway::Value>(&migrated).unwrap();
    let settings = core_config_settings_from_value(&document).unwrap();
    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 9527);
    assert_eq!(settings.api_keys, vec!["client-key"]);
    assert_eq!(settings.management_secret_key.as_deref(), Some("user-secret"));
    assert_eq!(settings.proxy_url, "direct");
    assert_eq!(document["routing"]["strategy"], "fill-first");
    assert_eq!(document["routing"]["session-affinity"], true);
    assert_eq!(document["routing"]["session-affinity-ttl"], "2h");
    assert_eq!(document["plugins"]["configs"]["local"]["enabled"], true);
    assert_eq!(document["plugins"]["store-auth"]["registry"], "stored-token");
    assert_eq!(document["api-keys"]["codex"][0]["name"], "template-upstream");
    assert!(migrated.starts_with("config-version:"));
    assert!(document.get("host").is_none());
    assert!(document.get("port").is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v8_core_updates_preserve_partially_migrated_config_and_credentials() {
    let root = agent_test_home("core-v8-partial-migration");
    let install_dir = root.join("cpa-core");
    let staging_dir = root.join("cpa-core.staging");
    fs::create_dir_all(&install_dir).unwrap();
    fs::create_dir_all(root.join("oauth")).unwrap();
    let credential_path = root.join("oauth/account.json");
    fs::write(&credential_path, b"existing-credential").unwrap();
    let legacy = "# User configuration\nusage-statistics-enabled: true\nhost: 127.0.0.1\nport: 9527\nauth-dir: ../oauth\napi-keys: [client-key]\nremote-management: {secret-key: user-secret}\nproxy-url: direct\nrequest-retry: 9\ncodex-api-key: [{api-key: upstream-key, base-url: 'https://example.invalid/v1'}]\noauth-model-alias: {codex: [{name: model, alias: custom-model}]}\n";
    let template = "observability: {usage: {usage-statistics-enabled: false}}\nserver: {host: '', port: 8317}\noauth: {auth-dir: '~/.cli-proxy-api'}\naccess: {api-keys: [your-api-key-1]}\nmanagement: {secret-key: ''}\nrequests: {proxy-url: ''}\nrouting: {retry: {request-retry: 3}}\n";

    for prefix in ["", "config-version: 7\n", "server: {host: 127.0.0.1}\n"] {
        let original = format!("{prefix}{legacy}");
        fs::write(install_dir.join(CORE_CONFIG_FILE), &original).unwrap();
        for template_prefix in ["config-version: 8\n", ""] {
            fs::create_dir_all(&staging_dir).unwrap();
            fs::write(staging_dir.join(CORE_EXAMPLE_CONFIG_FILE), format!("{template_prefix}{template}")).unwrap();
            migrate_core_config_for_update(&install_dir, &staging_dir).unwrap();
            let migrated = fs::read_to_string(staging_dir.join(CORE_CONFIG_FILE)).unwrap();
            overlay_install_dir(&install_dir, &staging_dir).unwrap();
            fs::write(install_dir.join(CORE_CONFIG_FILE), &original).unwrap();

            let document = serde_norway::from_str::<serde_norway::Value>(&migrated).unwrap();
            assert_eq!(document["config-version"].as_u64(), Some(8));
            let settings = core_config_settings_from_value(&document).unwrap();
            assert_eq!(settings.auth_dir, "../oauth", "credential directory changed during update");
            assert_eq!(settings.port, 9527);
            assert!(settings.usage_statistics_enabled);
            assert_eq!(settings.api_keys, vec!["client-key"]);
            assert_eq!(settings.management_secret_key.as_deref(), Some("user-secret"));
            assert_eq!(settings.proxy_url, "direct");
            assert_eq!(settings.request_retry, 9);
            assert!(migrated.contains("server:"), "legacy configuration must be lifted into the v8 layout: {migrated}");
            assert!(document.get("host").is_none(), "{migrated}");
            assert!(document.get("auth-dir").is_none());
            assert!(document.get("codex-api-key").is_none());
            assert_eq!(document["oauth"]["model-alias"]["codex"][0]["alias"], "custom-model");
            assert_eq!(document["api-keys"]["codex"][0]["keys"][0]["api-key"], "upstream-key");

            let mut gui = GuiConfigFile::default();
            apply_core_settings_to_gui_config(&mut gui, &settings);
            gui.proxy_url = settings.proxy_url.clone();
            let startup = merge_core_config_yaml(template, Some(&migrated), &gui).unwrap();
            let startup = serde_norway::from_str::<serde_norway::Value>(&startup).unwrap();
            assert_eq!(startup["config-version"].as_u64(), Some(8));
            let effective = core_config_settings_from_value(&startup).unwrap();
            assert_eq!(auth_dir_path_for_core(&effective.auth_dir, &install_dir).unwrap(), root.join("oauth"));
            assert!(effective.usage_statistics_enabled);
            assert_eq!(startup["api-keys"]["codex"], document["api-keys"]["codex"]);
            assert_eq!(startup["oauth"]["model-alias"], document["oauth"]["model-alias"]);
            assert_eq!(fs::read(&credential_path).unwrap(), b"existing-credential");
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn v7_config_startup_with_v8_template_writes_v8_version() {
    for marker in ["", "config-version: 8\n"] {
        let template = format!("{marker}server: {{port: 8317}}\naccess: {{api-keys: []}}\n");
        let current = "config-version: 7\ncodex-api-key: [{api-key: upstream-test, models: [{name: example}]}]\n";
        let rendered = merge_core_config_yaml(&template, Some(current), &GuiConfigFile::default()).unwrap();
        let document: serde_norway::Value = serde_norway::from_str(&rendered).unwrap();
        assert_eq!(document["config-version"].as_u64(), Some(8));
        assert_eq!(document["api-keys"]["codex"][0]["keys"][0]["api-key"], "upstream-test");
        assert!(document.get("codex-api-key").is_none());
    }
}

#[test]
fn v8_migration_preserves_each_legacy_provider_family_independently() {
    let template = "config-version: 8\napi-keys: {codex: [], claude: []}\n";
    for marker in ["", "config-version: 7\n"] {
        let old = format!("{marker}api-keys: {{codex: []}}\ncodex-api-key: [{{api-key: ignored}}]\nclaude-api-key: [{{api-key: fixture, models: [{{name: upstream, alias: claude-alias}}]}}]\n");
        let migrated = migrate_legacy_core_config_to_v8(template, &old).unwrap();
        let value: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
        assert_eq!(value["api-keys"]["codex"].as_sequence().unwrap().len(), 0);
        assert_eq!(value["api-keys"]["claude"][0]["keys"][0]["api-key"], "fixture");
        assert_eq!(value["api-keys"]["claude"][0]["models"][0]["alias"], "claude-alias");
        assert!(value.get("claude-api-key").is_none());
        assert!(value.get("codex-api-key").is_none());
    }
}

#[test]
fn v8_migration_preserves_narrow_fields_and_canonical_struct_leaves() {
    let template = "config-version: 8\nmanagement: {allow-remote: false}\noauth: {providers: {claude: {model-level-cooling: false}, codex: {response-steering: false}}}\n";
    let legacy = r#"
remote-management: {allow-remote: true, secret-key: old-secret}
management: {secret-key: canonical-secret}
claude-header-defaults: {user-agent: custom-agent, timezone: custom-zone}
claude-code: {disable-cloaking-model-list: true}
disable-claude-cloak-mode: true
claude: {model-level-cooling: true}
codex-header-defaults: {user-agent: codex-agent}
codex: {response-steering: true, optimize-multi-agent-v2: true}
oauth:
  providers:
    claude:
      model-level-cooling: false
      header-defaults: {timezone: canonical-zone}
    codex: {response-steering: false}
"#;
    let migrated = migrate_legacy_core_config_to_v8(template, legacy).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
    let claude = &value["oauth"]["providers"]["claude"];
    assert_eq!(claude["header-defaults"]["user-agent"], "custom-agent");
    assert_eq!(claude["header-defaults"]["timezone"], "canonical-zone");
    assert_eq!(claude["model-level-cooling"].as_bool(), Some(false));
    assert_eq!(claude["claude-code"]["disable-cloaking-model-list"].as_bool(), Some(true));
    assert_eq!(claude["disable-claude-cloak-mode"].as_bool(), Some(true));
    assert_eq!(value["management"]["allow-remote"].as_bool(), Some(true));
    assert_eq!(value["management"]["secret-key"], "canonical-secret");
    assert_eq!(value["oauth"]["providers"]["codex"]["header-defaults"]["user-agent"], "codex-agent");
    assert_eq!(value["oauth"]["providers"]["codex"]["response-steering"].as_bool(), Some(false));
    assert!(value["oauth"]["providers"]["codex"].get("optimize-multi-agent-v2").is_none());
    assert_eq!(value["client"]["codex"]["optimize-multi-agent-v2"].as_bool(), Some(true));
}

#[test]
fn v8_migration_preserves_network_settings_over_template_defaults() {
    let template = "config-version: 8\nserver: {trusted-proxies: [], discovery: {enabled: false}, tls: {enable: false, cert: '', key: ''}}\n";
    let legacy = "trusted-proxies: [127.0.0.1]\ndiscovery: {enabled: true, service-name: office}\ntls: {enable: true, cert: old-cert, key: old-key}\nserver: {tls: {cert: canonical-cert}}\n";
    let migrated = migrate_legacy_core_config_to_v8(template, legacy).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
    assert_eq!(value["server"]["trusted-proxies"][0], "127.0.0.1");
    assert_eq!(value["server"]["discovery"]["enabled"].as_bool(), Some(true));
    assert_eq!(value["server"]["discovery"]["service-name"], "office");
    assert_eq!(value["server"]["tls"]["enable"].as_bool(), Some(true));
    assert_eq!(value["server"]["tls"]["cert"], "canonical-cert");
    assert_eq!(value["server"]["tls"]["key"], "old-key");
    assert!(value.get("trusted-proxies").is_none());
    assert!(value.get("discovery").is_none());
}

#[test]
fn v8_migration_preserves_github_token() {
    let template = "config-version: 8\nserver: {github-token: ''}\n";
    let migrated = migrate_legacy_core_config_to_v8(template, "github-token: legacy-token\n").unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
    assert_eq!(value["server"]["github-token"], "legacy-token");
    assert!(value.get("github-token").is_none());
    let current = "config-version: 8\nserver: {github-token: current-token}\n";
    assert_eq!(migrate_legacy_core_config_to_v8(template, current).unwrap(), current);
}

#[test]
fn v8_migration_preserves_explicit_canonical_clears_and_existing_v8_files() {
    let template = "config-version: 8\nserver: {trusted-proxies: [192.0.2.1]}\n";
    let old = "trusted-proxies: [127.0.0.1]\nserver: {trusted-proxies: []}\nclaude-header-defaults: {user-agent: legacy}\nclaude: {model-level-cooling: true}\noauth: {providers: {claude: null}, model-alias: {}}\noauth-model-alias: {claude: [{name: original, alias: old-alias}]}\n";
    let migrated = migrate_legacy_core_config_to_v8(template, old).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(&migrated).unwrap();
    assert!(value["server"]["trusted-proxies"].as_sequence().unwrap().is_empty());
    assert!(value["oauth"]["providers"]["claude"].is_null());
    assert!(value["oauth"]["model-alias"].as_mapping().unwrap().is_empty());
    let already_v8 = format!("config-version: 8\n{old}");
    assert_eq!(migrate_legacy_core_config_to_v8(template, &already_v8).unwrap(), already_v8);
}

#[test]
fn replacing_a_core_rejects_invalid_config_without_overwriting_files() {
    let root = agent_test_home("core-config-migrate-invalid");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(
        source.join(CORE_CONFIG_FILE),
        "host: 127.0.0.1\nbroken: [invalid\nport: 9527\napi-keys:\n- old-a\n- old-b\n",
    )
    .unwrap();
    fs::write(
            target.join(CORE_EXAMPLE_CONFIG_FILE),
            "host: \"\"\nbroken: new-default\nport: 8317\napi-keys:\n  - new-default\nnew-option: true\n",
        )
        .unwrap();
    fs::write(target.join(CORE_CONFIG_FILE), "staged: untouched\n").unwrap();

    let original = fs::read(source.join(CORE_CONFIG_FILE)).unwrap();
    assert!(migrate_core_config_for_update(&source, &target).is_err());
    assert_eq!(fs::read(source.join(CORE_CONFIG_FILE)).unwrap(), original);
    assert_eq!(
        fs::read_to_string(target.join(CORE_CONFIG_FILE)).unwrap(),
        "staged: untouched\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn replacing_a_core_rejects_locked_config_without_using_defaults() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = agent_test_home("core-config-locked");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let original = "codex-api-key:\n  - api-key: test-key\n    base-url: https://example.test\n";
    let source_path = source.join(CORE_CONFIG_FILE);
    fs::write(&source_path, original).unwrap();
    fs::write(target.join(CORE_EXAMPLE_CONFIG_FILE), "port: 8317\n").unwrap();
    fs::write(target.join(CORE_CONFIG_FILE), "staged: untouched\n").unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&source_path)
        .unwrap();
    let result = migrate_core_config_for_update(&source, &target);
    drop(locked);
    assert!(
        result.is_err(),
        "unreadable configuration must not be replaced by defaults"
    );
    assert_eq!(fs::read_to_string(source_path).unwrap(), original);
    assert_eq!(
        fs::read_to_string(target.join(CORE_CONFIG_FILE)).unwrap(),
        "staged: untouched\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundled_core_install_handles_missing_and_unversioned_binaries() {
    let root = agent_test_home("bundled-bootstrap-detection");
    let install_dir = root.join("cpa-core");

    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));

    let existing_version = install_dir.join("existing-version");
    fs::create_dir_all(&existing_version).unwrap();
    fs::write(existing_version.join(core_binary_name()), b"existing core").unwrap();

    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));
    assert!(!core_needs_bundled_install(&install_dir, "invalid"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundled_core_install_replaces_older_installs_without_downgrading() {
    let root = agent_test_home("bundled-upgrade-detection");
    let install_dir = root.join("cpa-core");
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join(core_binary_name()), b"installed core").unwrap();

    for (installed, bundled, should_install) in [
        ("v7.3.9", "v7.3.17", true),
        ("7.3.15", "v7.3.17", true),
        ("v7.3.17-beta.1", "v7.3.17", true),
        ("v7.3.17", "7.3.17", false),
        ("v7.3.18", "v7.3.17", false),
        ("unknown", "v7.3.17", true),
        ("v7.3.15", "invalid", false),
    ] {
        write_core_metadata(
            &install_dir,
            &CoreMetadata {
                version: installed.to_string(),
                asset_name: "installed-core.zip".to_string(),
                installed_at_unix: 0,
            },
        )
        .unwrap();
        assert_eq!(
            core_needs_bundled_install(&install_dir, bundled),
            should_install,
            "installed {installed}, bundled {bundled}"
        );
    }

    fs::remove_file(install_dir.join(core_binary_name())).unwrap();
    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn handled_bundled_core_does_not_undo_a_manual_rollback_on_restart() {
    let root = agent_test_home("bundled-manual-rollback");
    let install_dir = root.join("cpa-core");
    fs::create_dir_all(&install_dir).unwrap();
    let binary = install_dir.join(core_binary_name());
    fs::write(&binary, b"installed core").unwrap();
    write_core_metadata(
        &install_dir,
        &CoreMetadata {
            version: "v7.3.15".to_string(),
            asset_name: "installed-core.zip".to_string(),
            installed_at_unix: 0,
        },
    )
    .unwrap();

    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));
    remember_bundled_core_when_up_to_date(&install_dir, "v7.3.17").unwrap();
    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));

    mark_bundled_core_version_handled(&install_dir, "v7.3.17").unwrap();
    assert!(!core_needs_bundled_install(&install_dir, "v7.3.17"));
    assert!(core_needs_bundled_install(&install_dir, "v7.3.18"));

    fs::remove_file(binary).unwrap();
    assert!(core_needs_bundled_install(&install_dir, "v7.3.17"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_already_newer_core_satisfies_the_bundle_after_manual_rollback() {
    let root = agent_test_home("bundled-newer-manual-rollback");
    let install_dir = root.join("cpa-core");
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join(core_binary_name()), b"installed core").unwrap();
    write_core_metadata(
        &install_dir,
        &CoreMetadata {
            version: "v7.3.18".to_string(),
            asset_name: "installed-core.zip".to_string(),
            installed_at_unix: 0,
        },
    )
    .unwrap();

    assert!(!core_needs_bundled_install(&install_dir, "v7.3.17"));
    remember_bundled_core_when_up_to_date(&install_dir, "v7.3.17").unwrap();
    write_core_metadata(
        &install_dir,
        &CoreMetadata {
            version: "v7.3.15".to_string(),
            asset_name: "manual-rollback.zip".to_string(),
            installed_at_unix: 1,
        },
    )
    .unwrap();
    assert!(!core_needs_bundled_install(&install_dir, "v7.3.17"));
    assert!(core_needs_bundled_install(&install_dir, "v7.3.19"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_core_checksums_do_not_block_a_new_bundled_archive() {
    let root = agent_test_home("bundled-stale-checksums");
    let archive = root.join("CLIProxyAPI_7.3.17_windows_amd64.zip");
    fs::write(&archive, b"new bundled core").unwrap();
    fs::write(
        root.join(CORE_CHECKSUMS_FILE),
        format!("{}  CLIProxyAPI_7.3.15_windows_amd64.zip\n", "a".repeat(64)),
    )
    .unwrap();
    validate_bundled_core_checksum(&archive).unwrap();

    fs::write(
        root.join(CORE_CHECKSUMS_FILE),
        format!(
            "{}  {}\n",
            "a".repeat(64),
            archive.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    assert!(validate_bundled_core_checksum(&archive).is_err());

    fs::write(
        root.join(CORE_CHECKSUMS_FILE),
        format!(
            "{}  {}\n",
            sha256_file(&archive).unwrap(),
            archive.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    validate_bundled_core_checksum(&archive).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundled_core_locations_include_macos_app_resources() {
    let contents_dir = agent_test_home("bundled-macos-resources")
        .join("EasyCLIProxyAPI.app")
        .join("Contents");
    let executable_dir = contents_dir.join("MacOS");
    let base_dir = agent_test_home("bundled-macos-data");
    let resource_location = (
        contents_dir.join("Resources").join(CORE_VERSION_FILE),
        contents_dir.join("Resources").join("cpa-core"),
    );

    assert_eq!(
        macos_app_resources_dir(&executable_dir),
        Some(contents_dir.join("Resources"))
    );
    let locations = bundled_core_locations(&base_dir, &executable_dir);
    assert_eq!(locations.first(), Some(&resource_location));
}

#[test]
fn source_project_root_is_detected_from_the_portable_development_directory() {
    let root = agent_test_home("bundled-source-root");
    fs::create_dir_all(root.join("src-tauri")).unwrap();
    fs::create_dir_all(root.join("bin-work")).unwrap();
    fs::write(root.join("package.json"), b"{}").unwrap();

    assert_eq!(
        source_project_root(&root.join("bin-work")),
        Some(root.clone())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_source_archive_and_checksums_are_copied_into_the_installation() {
    let root = agent_test_home("selected-bundled-asset");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let archive = source.join("CLIProxyAPI_7.2.83_linux_amd64.tar.gz");
    fs::write(&archive, b"archive").unwrap();
    fs::write(source.join(CORE_CHECKSUMS_FILE), b"checksums").unwrap();

    preserve_selected_bundled_core_asset(&archive, &target).unwrap();

    assert_eq!(
        fs::read(target.join("CLIProxyAPI_7.2.83_linux_amd64.tar.gz")).unwrap(),
        b"archive"
    );
    assert_eq!(
        fs::read(target.join(CORE_CHECKSUMS_FILE)).unwrap(),
        b"checksums"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn release_page_assets_parse_download_links_and_sha256() {
    let html = r#"
          <li><a href="/router-for-me/CLIProxyAPI/releases/download/v1.2.3/checksums.txt">checksums.txt</a></li>
          <li><a href="/router-for-me/CLIProxyAPI/releases/download/v1.2.3/CLIProxyAPI_1.2.3_linux_amd64.tar.gz">asset</a>
            <span>sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef</span>
          </li>
        "#;

    let assets = parse_release_assets(html);
    assert_eq!(assets.len(), 2);
    assert_eq!(assets[1].name, "CLIProxyAPI_1.2.3_linux_amd64.tar.gz");
    assert_eq!(
        assets[1].digest.as_deref(),
        Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert!(assets[1]
        .browser_download_url
        .ends_with("/releases/download/v1.2.3/CLIProxyAPI_1.2.3_linux_amd64.tar.gz"));
}

#[test]
fn rematerializing_core_binary_preserves_bytes_and_cleans_up_temporary_file() {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let root = agent_test_home("core-rematerialize");
    fs::create_dir_all(&root).unwrap();
    let binary_path = root.join(core_binary_name());
    fs::write(&binary_path, b"signed core bytes").unwrap();
    #[cfg(unix)]
    let original_inode = fs::metadata(&binary_path).unwrap().ino();

    rematerialize_core_binary(&binary_path).unwrap();

    assert_eq!(fs::read(&binary_path).unwrap(), b"signed core bytes");
    #[cfg(unix)]
    assert_ne!(fs::metadata(&binary_path).unwrap().ino(), original_inode);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn core_start_log_path_follows_the_managed_logs_directory() {
    let base_dir = PathBuf::from("test-base");
    let install_dir = base_dir.join("cpa-core");

    assert_eq!(
        core_start_log_path(&install_dir, DEFAULT_AUTH_DIR).unwrap(),
        base_dir
            .join("oauth")
            .join("logs")
            .join("core-start-output.log")
    );
    assert_eq!(
        core_start_log_path(&install_dir, "custom-auth").unwrap(),
        install_dir
            .join("custom-auth")
            .join("logs")
            .join("core-start-output.log")
    );
}

#[test]
fn core_start_output_helper() {
    if env::var_os("EASYCLIPROXYAPI_CORE_OUTPUT_TEST_HELPER").is_none() {
        return;
    }
    println!("core stdout marker");
    eprintln!("core stderr marker");
}

#[test]
fn core_start_log_captures_stdout_and_stderr() {
    let root = agent_test_home("core-start-output");
    let log_path = root.join("logs").join("core-start-output.log");
    fs::create_dir_all(log_path.parent().unwrap()).unwrap();
    fs::write(&log_path, "stale startup output").unwrap();
    let (stdout, stderr) = core_start_stdio(&log_path).unwrap();
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "tests::core_runtime::core_start_output_helper",
            "--nocapture",
        ])
        .env("EASYCLIPROXYAPI_CORE_OUTPUT_TEST_HELPER", "1")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    configure_background_command(&mut command);

    assert!(command.status().unwrap().success());

    let output = fs::read_to_string(&log_path).unwrap();
    assert!(output.contains("===== CPA kernel startup"));
    assert!(output.contains("core stdout marker"));
    assert!(output.contains("core stderr marker"));
    assert!(output.starts_with("stale startup output\n"));
    drop(core_start_stdio(&log_path).unwrap());
    let restarted_output = fs::read_to_string(&log_path).unwrap();
    assert!(restarted_output.starts_with(&output));
    assert_eq!(restarted_output.matches("===== CPA kernel startup").count(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn apply_staged_core_with_rollback_restores_previous_binary_on_restart_failure() {
    let root = agent_test_home("core-staged-rollback");
    let install_dir = root.join("cpa-core");
    let staging_dir = root.join("cpa-core.staging");
    let rollback_dir = root.join("cpa-core.rollback");
    fs::create_dir_all(&install_dir).unwrap();
    fs::create_dir_all(&staging_dir).unwrap();

    fs::write(install_dir.join(core_binary_name()), b"working-v1-binary").unwrap();
    write_core_metadata(
        &install_dir,
        &CoreMetadata {
            version: "v8.0.26".to_string(),
            asset_name: "v8.0.26.tar.gz".to_string(),
            installed_at_unix: 10,
        },
    )
    .unwrap();

    // 1. Empty staged binary is rejected before touching install_dir
    fs::write(staging_dir.join(core_binary_name()), b"").unwrap();
    assert!(apply_staged_core_with_rollback(&install_dir, &staging_dir, &rollback_dir, || Ok(())).is_err());
    assert_eq!(
        fs::read(install_dir.join(core_binary_name())).unwrap(),
        b"working-v1-binary"
    );

    // 2. Staged binary that fails restart rolls back to working-v1-binary and metadata
    fs::write(staging_dir.join(core_binary_name()), b"broken-v2-binary").unwrap();
    write_core_metadata(
        &staging_dir,
        &CoreMetadata {
            version: "v8.0.27".to_string(),
            asset_name: "v8.0.27.tar.gz".to_string(),
            installed_at_unix: 20,
        },
    )
    .unwrap();

    let err = apply_staged_core_with_rollback(&install_dir, &staging_dir, &rollback_dir, || {
        Err("new binary exited immediately".to_string())
    })
    .unwrap_err();
    assert!(err.contains("was rolled back to the previous version"));
    assert_eq!(
        fs::read(install_dir.join(core_binary_name())).unwrap(),
        b"working-v1-binary"
    );
    let restored_meta = read_core_metadata(&install_dir).unwrap();
    assert_eq!(restored_meta.version, "v8.0.26");
    assert!(!rollback_dir.exists());

    // 3. Staged binary that succeeds restart keeps new binary and cleans up rollback_dir
    fs::create_dir_all(&staging_dir).unwrap();
    fs::write(staging_dir.join(core_binary_name()), b"healthy-v2-binary").unwrap();
    write_core_metadata(
        &staging_dir,
        &CoreMetadata {
            version: "v8.0.27".to_string(),
            asset_name: "v8.0.27.tar.gz".to_string(),
            installed_at_unix: 30,
        },
    )
    .unwrap();
    apply_staged_core_with_rollback(&install_dir, &staging_dir, &rollback_dir, || Ok(())).unwrap();
    assert_eq!(
        fs::read(install_dir.join(core_binary_name())).unwrap(),
        b"healthy-v2-binary"
    );
    assert_eq!(read_core_metadata(&install_dir).unwrap().version, "v8.0.27");
    assert!(!rollback_dir.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rollback_removes_files_added_by_the_failed_update_and_keeps_user_files() {
    let root = agent_test_home("core-staged-rollback-added-files");
    let install_dir = root.join("cpa-core");
    let staging_dir = root.join("cpa-core.staging");
    let rollback_dir = root.join("cpa-core.rollback");
    fs::create_dir_all(install_dir.join("static")).unwrap();
    fs::create_dir_all(staging_dir.join("static")).unwrap();
    fs::create_dir_all(staging_dir.join("plugins")).unwrap();

    fs::write(install_dir.join(core_binary_name()), b"working-v1-binary").unwrap();
    fs::write(install_dir.join("README.md"), b"v1 readme").unwrap();
    fs::write(install_dir.join("static").join("app.js"), b"v1 js").unwrap();
    fs::write(install_dir.join("user-notes.txt"), b"not part of any package").unwrap();

    fs::write(staging_dir.join(core_binary_name()), b"broken-v2-binary").unwrap();
    fs::write(staging_dir.join("README.md"), b"v2 readme").unwrap();
    fs::write(staging_dir.join("static").join("app.js"), b"v2 js").unwrap();
    fs::write(staging_dir.join("static").join("new.css"), b"v2 css").unwrap();
    fs::write(staging_dir.join("plugins").join("extra.so"), b"v2 plugin").unwrap();

    let err = apply_staged_core_with_rollback(&install_dir, &staging_dir, &rollback_dir, || {
        Err("new binary exited immediately".to_string())
    })
    .unwrap_err();

    assert!(err.contains("was rolled back to the previous version"));
    assert_eq!(fs::read(install_dir.join(core_binary_name())).unwrap(), b"working-v1-binary");
    assert_eq!(fs::read(install_dir.join("README.md")).unwrap(), b"v1 readme");
    assert_eq!(fs::read(install_dir.join("static").join("app.js")).unwrap(), b"v1 js");
    assert_eq!(fs::read(install_dir.join("user-notes.txt")).unwrap(), b"not part of any package");
    assert!(!install_dir.join("static").join("new.css").exists());
    assert!(!install_dir.join("plugins").exists());
    assert!(!rollback_dir.exists());
    fs::remove_dir_all(root).unwrap();
}

