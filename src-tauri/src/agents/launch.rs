use super::*;
use std::net::TcpListener;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTerminalOption {
    pub(crate) id: String,
    pub(crate) label: String,
}

fn terminal_option(id: &str, label: &str) -> AgentTerminalOption {
    AgentTerminalOption {
        id: id.to_string(),
        label: label.to_string(),
    }
}

fn program_on_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| {
        let candidate = directory.join(program);
        candidate.is_file().then_some(candidate)
    })
}

#[cfg(target_os = "macos")]
fn macos_iterm2_installed() -> bool {
    Path::new("/Applications/iTerm.app").is_dir()
        || env::var_os("HOME")
            .map(PathBuf::from)
            .is_some_and(|home| home.join("Applications/iTerm.app").is_dir())
}

#[cfg(target_os = "macos")]
fn macos_ghostty_installed() -> bool {
    Path::new("/Applications/Ghostty.app").is_dir()
        || env::var_os("HOME")
            .map(PathBuf::from)
            .is_some_and(|home| home.join("Applications/Ghostty.app").is_dir())
}

#[cfg(target_os = "linux")]
fn linux_terminal_definitions() -> &'static [(
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
)] {
    &[
        (
            "x-terminal-emulator",
            "x-terminal-emulator",
            &["-e"],
            "System terminal",
        ),
        (
            "gnome-terminal",
            "gnome-terminal",
            &["--"],
            "GNOME Terminal",
        ),
        ("konsole", "konsole", &["-e"], "Konsole"),
        ("xfce4-terminal", "xfce4-terminal", &["-e"], "Xfce Terminal"),
        ("mate-terminal", "mate-terminal", &["--"], "MATE Terminal"),
        ("kitty", "kitty", &["-e"], "Kitty"),
        ("alacritty", "alacritty", &["-e"], "Alacritty"),
        ("ghostty", "ghostty", &["-e"], "Ghostty"),
        ("xterm", "xterm", &["-e"], "XTerm"),
    ]
}

pub(crate) fn available_agent_terminals() -> Vec<AgentTerminalOption> {
    let mut options = vec![terminal_option("auto", "Automatic")];
    #[cfg(target_os = "macos")]
    {
        options.push(terminal_option("terminal", "Terminal"));
        if macos_iterm2_installed() {
            options.push(terminal_option("iterm2", "iTerm2"));
        }
        if macos_ghostty_installed() {
            options.push(terminal_option("ghostty", "Ghostty"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if program_on_path("wt.exe").is_some() {
            options.push(terminal_option("windows-terminal", "Windows Terminal"));
        }
        let powershell = windows_powershell_executable();
        if powershell.is_file() {
            options.push(terminal_option("powershell", "PowerShell"));
        }
        let command_prompt = windows_command_processor();
        if command_prompt.is_file() {
            options.push(terminal_option("cmd", "Command Prompt"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        for (id, program, _, label) in linux_terminal_definitions() {
            if program_on_path(program).is_some() {
                options.push(terminal_option(id, label));
            }
        }
    }
    options
}

pub(crate) fn normalize_agent_terminal(value: &str) -> String {
    let value = value.trim();
    if available_agent_terminals()
        .iter()
        .any(|option| option.id == value)
    {
        value.to_string()
    } else {
        DEFAULT_AGENT_TERMINAL.to_string()
    }
}

#[tauri::command]
pub(crate) fn launch_agent(
    app: tauri::AppHandle,
    gui_config_state: tauri::State<'_, GuiConfigState>,
    deepseek_process_state: tauri::State<'_, DeepSeekHarnessProcessState>,
    client: String,
    target: Option<String>,
    executable_path: Option<String>,
    desktop_executable_path: Option<String>,
    working_directory: Option<String>,
    deepseek_harness_options: Option<DeepSeekHarnessLaunchOptions>,
) -> Result<(), String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    let config = gui_config_state.snapshot()?;
    let terminal = normalize_agent_terminal(&config.default_terminal);
    let requested_target = target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if deepseek_harness_options.is_some() && !client.trim().eq_ignore_ascii_case("deepseek-harness")
    {
        return Err("DeepSeek Harness launch options cannot be used with other clients".to_string());
    }

    if client.trim().eq_ignore_ascii_case(PI_AGENT_ID) {
        if requested_target.is_some_and(|value| value != "cli") {
            return Err("Pi supports only CLI launch mode".to_string());
        }
        let executable_override = executable_path.as_deref().map(Path::new);
        let status = inspect_pi_provider_status_with_executable(&home, config.port, effective_agent_api_key(&config), executable_override);
        if !status.installed {
            return Err("Pi CLI was not detected. Install Pi and detect it again".to_string());
        }
        let executable = executable_override.filter(|path| path.is_file()).map(Path::to_path_buf)
            .or_else(|| find_pi_executable(&home))
            .ok_or_else(|| "Pi CLI executable not found".to_string())?;
        let launch_directory = resolve_launch_directory(working_directory.as_deref(), &home)?;
        return launch_cli_agent(
            &executable,
            PI_AGENT_NAME,
            &launch_directory,
            &[],
            &[],
            &[],
            &terminal,
        );
    }

    let client = AgentClient::parse(&client)?;
    if !client.supported_platform() {
        return Err(format!("The current platform does not support launching {}", client.name()));
    }
    let executable_override = executable_path.as_deref().map(Path::new);
    let desktop_override = desktop_executable_path.as_deref().map(Path::new);
    let status = inspect_agent_config_with_executables(client, &home, config.port, effective_agent_api_key(&config), executable_override, desktop_override);
    if !status.installed {
        return Err(format!("{} was not detected. Install it and detect it again", client.name()));
    }
    let default_target = status
        .launch_targets
        .first()
        .map(|target| target.id.as_str())
        .unwrap_or("cli");
    let requested_target = requested_target.unwrap_or(default_target);
    match (client, requested_target) {
        (AgentClient::ClaudeDesktop, "app") => launch_claude_desktop(&home, executable_override),
        (AgentClient::AntigravityCli, "cli")
            if antigravity_has_marker(client, &agent_config_paths(client, &home))? => {
            if !status.configured {
                return Err("Antigravity CLI configuration does not match CPA. Apply the configuration again".into());
            }
            let executable = env::current_exe().map_err(|_| "Unable to locate CPA launch adapter")?;
            let directory = resolve_launch_directory(working_directory.as_deref(), &home)?;
            launch_cli_agent(&executable, client.name(), &directory, &antigravity_cli_helper_arguments(&home), &[], &[], &terminal)
        }
        (AgentClient::WorkBuddy, "app") => {
            let executable = manual_desktop_path(executable_override).map(Path::to_path_buf)
                .or_else(|| find_workbuddy_desktop_executable(&home))
                .ok_or_else(|| "WorkBuddy application not found".to_string())?;
            launch_desktop_agent(&executable, client.name())
        }
        (AgentClient::ZCode, "app") => {
            let executable = manual_desktop_path(executable_override).map(Path::to_path_buf)
                .or_else(|| find_zcode_desktop_executable(&home))
                .ok_or_else(|| "ZCode application not found".to_string())?;
            launch_desktop_agent(&executable, client.name())
        }
        (AgentClient::Codex, "app") => {
            launch_codex_desktop(&home, desktop_override)
        }
        (AgentClient::OpenCode, "app") => launch_opencode_desktop(&home, desktop_override),
        (AgentClient::DeepSeekHarness, "app") => {
            let executable = manual_desktop_path(desktop_override).map(Path::to_path_buf)
                .or_else(|| find_deepseek_harness_desktop_application(&home))
                .ok_or_else(|| "DeepSeek Harness Desktop application not found".to_string())?;
            launch_desktop_agent(&executable, "DeepSeek Harness Desktop")
        }
        (AgentClient::ClaudeDesktop | AgentClient::ZCode | AgentClient::WorkBuddy, "cli") => {
            Err(format!("{} does not support CLI launch mode", client.name()))
        }
        (_, "cli") => {
            let executable = executable_override.filter(|path| path.is_file()).map(Path::to_path_buf)
                .or_else(|| find_agent_executable(client, &home))
                .ok_or_else(|| format!("Executable for {} not found", client.name()))?;
            let launch_directory = resolve_launch_directory(working_directory.as_deref(), &home)?;
            if client == AgentClient::DeepSeekHarness {
                let mode = deepseek_harness_launch_mode(deepseek_harness_options.as_ref())?;
                let arguments =
                    agent_cli_launch_arguments(client, deepseek_harness_options.as_ref())?;
                return launch_managed_deepseek_harness(
                    deepseek_process_state.inner(),
                    &executable,
                    &launch_directory,
                    &arguments,
                    &mode,
                    deepseek_harness_options.as_ref(),
                );
            }
            let environment_to_remove = if client == AgentClient::ClaudeCode {
                &["ANTHROPIC_API_KEY"][..]
            } else {
                &[]
            };
            let arguments = agent_cli_launch_arguments(client, deepseek_harness_options.as_ref())?;
            launch_cli_agent(
                &executable,
                client.name(),
                &launch_directory,
                &arguments,
                environment_to_remove,
                &[],
                &terminal,
            )
        }
        (_, "app") => Err(format!("{} does not support desktop App launch mode", client.name())),
        _ => Err("Unsupported agent launch mode".to_string()),
    }
}


fn deepseek_harness_launch_mode(
    options: Option<&DeepSeekHarnessLaunchOptions>,
) -> Result<String, String> {
    let mode = options
        .map(|options| options.mode.trim().to_ascii_lowercase())
        .filter(|mode| !mode.is_empty())
        .unwrap_or_else(|| "web".to_string());
    if matches!(
        mode.as_str(),
        "web" | "headless" | "acp" | "sdk" | "sdk-minimal" | "custom"
    ) {
        Ok(mode)
    } else {
        Err(format!("Unsupported DeepSeek Harness launch mode: {mode}"))
    }
}

fn agent_cli_launch_arguments(
    client: AgentClient,
    deepseek_harness_options: Option<&DeepSeekHarnessLaunchOptions>,
) -> Result<Vec<String>, String> {
    if client != AgentClient::DeepSeekHarness {
        if deepseek_harness_options.is_some() {
            return Err("DeepSeek Harness launch options cannot be used with other clients".to_string());
        }
        return Ok(Vec::new());
    }

    build_deepseek_harness_launch_arguments(deepseek_harness_options)
}

fn build_deepseek_harness_launch_arguments(
    options: Option<&DeepSeekHarnessLaunchOptions>,
) -> Result<Vec<String>, String> {
    let mode = deepseek_harness_launch_mode(options)?;
    let profile = match mode.as_str() {
        "web" | "headless" | "acp" | "sdk" | "sdk-minimal" => mode.as_str(),
        "custom" => validate_deepseek_harness_profile(
            options
                .and_then(|options| options.profile.as_deref())
                .unwrap_or_default(),
        )?,
        _ => return Err(format!("Unsupported DeepSeek Harness launch mode: {mode}")),
    };
    let mut arguments = if profile == "web" {
        vec!["web".to_string()]
    } else {
        vec!["--profile".to_string(), profile.to_string()]
    };

    if let Some(options) = options {
        for patch in &options.patches {
            arguments.push("--patch".to_string());
            arguments.push(validate_deepseek_harness_argument(patch, "Patch path")?);
        }
    }

    match mode.as_str() {
        "web" => {
            if let Some(host) = options
                .and_then(|options| options.web_host.as_deref())
                .map(str::trim)
                .filter(|host| !host.is_empty())
            {
                arguments.push("--host".to_string());
                arguments.push(validate_deepseek_harness_argument(host, "Web host")?);
            }
            if let Some(port) = options.and_then(|options| options.web_port) {
                arguments.push("--port".to_string());
                arguments.push(port.to_string());
            }
            if options.and_then(|options| options.open_browser) == Some(false) {
                arguments.push("--no-open".to_string());
            }
            if let Some(options) = options {
                for authority in &options.trusted_hosts {
                    arguments.push("--trusted-host".to_string());
                    arguments.push(validate_deepseek_harness_argument(
                        authority,
                        "trusted host",
                    )?);
                }
            }
        }
        "headless" => {
            let task = options
                .and_then(|options| options.task.as_deref())
                .unwrap_or_default();
            arguments.push(validate_deepseek_harness_argument(task, "Headless task")?);
        }
        "acp" | "sdk" | "sdk-minimal" | "custom" => {}
        _ => unreachable!(),
    }
    Ok(arguments)
}

impl DeepSeekHarnessProcessState {
    fn status(&self) -> Result<DeepSeekHarnessProcessStatus, String> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| "DeepSeek Harness process status lock is poisoned".to_string())?;
        let Some(managed) = process.as_mut() else {
            return Ok(DeepSeekHarnessProcessStatus::default());
        };
        match managed.child.try_wait() {
            Ok(None) => Ok(DeepSeekHarnessProcessStatus {
                running: true,
                pid: Some(managed.child.id()),
                mode: Some(managed.mode.clone()),
            }),
            Ok(Some(_)) => {
                *process = None;
                Ok(DeepSeekHarnessProcessStatus::default())
            }
            Err(error) => Err(format!("Failed to check DeepSeek Harness process status: {error}")),
        }
    }
}

#[tauri::command]
pub(crate) fn get_deepseek_harness_process_status(
    process_state: tauri::State<'_, DeepSeekHarnessProcessState>,
) -> Result<DeepSeekHarnessProcessStatus, String> {
    process_state.status()
}

#[tauri::command]
pub(crate) fn stop_deepseek_harness_process(
    process_state: tauri::State<'_, DeepSeekHarnessProcessState>,
) -> Result<DeepSeekHarnessProcessStatus, String> {
    stop_managed_deepseek_harness(process_state.inner())?;
    process_state.status()
}

fn launch_managed_deepseek_harness(
    process_state: &DeepSeekHarnessProcessState,
    executable: &Path,
    working_directory: &Path,
    arguments: &[String],
    mode: &str,
    options: Option<&DeepSeekHarnessLaunchOptions>,
) -> Result<(), String> {
    let mut process = process_state
        .process
        .lock()
        .map_err(|_| "DeepSeek Harness process status lock is poisoned".to_string())?;
    if let Some(managed) = process.as_mut() {
        match managed.child.try_wait() {
            Ok(None) => {
                return Err(format!(
                    "DeepSeek Harness is already running (PID {}). Stop the current process first",
                    managed.child.id()
                ));
            }
            Ok(Some(_)) => *process = None,
            Err(error) => return Err(format!("Failed to check DeepSeek Harness process status: {error}")),
        }
    }

    if mode == "web" {
        ensure_deepseek_harness_web_endpoint_available(options)?;
    }

    let child = spawn_managed_deepseek_harness(executable, working_directory, arguments)?;
    *process = Some(ManagedDeepSeekHarnessProcess {
        child,
        mode: mode.to_string(),
        launch: DeepSeekHarnessLaunchSnapshot {
            executable: executable.to_path_buf(),
            working_directory: working_directory.to_path_buf(),
            arguments: arguments.to_vec(),
            options: options.cloned(),
        },
    });
    Ok(())
}

#[tauri::command]
pub(crate) async fn restart_deepseek_harness_process(
    app: tauri::AppHandle,
) -> Result<DeepSeekHarnessProcessStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        restart_managed_deepseek_harness(app.state::<DeepSeekHarnessProcessState>().inner())
    })
    .await
    .map_err(|error| format!("DeepSeek Harness Web restart task failed: {error}"))?
}

fn restart_managed_deepseek_harness(
    process_state: &DeepSeekHarnessProcessState,
) -> Result<DeepSeekHarnessProcessStatus, String> {
    restart_managed_deepseek_harness_with(
        process_state,
        terminate_deepseek_harness_process_tree,
        spawn_managed_deepseek_harness,
    )
}

fn restart_managed_deepseek_harness_with(
    process_state: &DeepSeekHarnessProcessState,
    stop: impl FnOnce(&mut Child) -> Result<(), String>,
    spawn: impl FnOnce(&Path, &Path, &[String]) -> Result<Child, String>,
) -> Result<DeepSeekHarnessProcessStatus, String> {
    let mut process = process_state
        .process
        .lock()
        .map_err(|_| "DeepSeek Harness process status lock is poisoned".to_string())?;
    let managed = process
        .as_mut()
        .ok_or_else(|| "No DeepSeek Harness Web service is running".to_string())?;
    if managed
        .child
        .try_wait()
        .map_err(|error| format!("Failed to check DeepSeek Harness process status: {error}"))?
        .is_some()
    {
        *process = None;
        return Err("DeepSeek Harness Web has exited. Start it again".to_string());
    }
    if managed.mode != "web" {
        return Err("Only Web mode supports restart".to_string());
    }
    let launch = managed.launch.clone();
    if !launch.executable.is_file() || !launch.working_directory.is_dir() {
        return Err("DeepSeek Harness launcher or working directory no longer exists".to_string());
    }
    stop(&mut managed.child)?;
    *process = None;
    ensure_deepseek_harness_web_endpoint_available(launch.options.as_ref())?;
    let child = spawn(
        &launch.executable,
        &launch.working_directory,
        &launch.arguments,
    )?;
    let status = DeepSeekHarnessProcessStatus {
        running: true,
        pid: Some(child.id()),
        mode: Some("web".to_string()),
    };
    *process = Some(ManagedDeepSeekHarnessProcess {
        child,
        mode: "web".to_string(),
        launch,
    });
    Ok(status)
}

fn ensure_deepseek_harness_web_endpoint_available(
    options: Option<&DeepSeekHarnessLaunchOptions>,
) -> Result<(), String> {
    let port = options
        .and_then(|options| options.web_port)
        .unwrap_or(DEEPSEEK_HARNESS_DEFAULT_WEB_PORT);
    if port == 0 {
        return Ok(());
    }
    let host = options
        .and_then(|options| options.web_host.as_deref())
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .unwrap_or("127.0.0.1");
    let listener = TcpListener::bind((host, port)).map_err(|error| {
        if error.kind() == io::ErrorKind::AddrInUse {
            format!(
                "DeepSeek Harness Web address {host}:{port} is in use. Stop the existing service or choose another port"
            )
        } else {
            format!("Cannot use DeepSeek Harness Web address {host}:{port}: {error}")
        }
    })?;
    drop(listener);
    Ok(())
}

fn spawn_managed_deepseek_harness(
    executable: &Path,
    working_directory: &Path,
    arguments: &[String],
) -> Result<Child, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let mut command = windows_command_for_executable(executable, false);
        command
            .args(arguments)
            .current_dir(working_directory)
            .creation_flags(
                (if cfg!(test) {
                    0x0800_0000
                } else {
                    CREATE_NEW_CONSOLE
                }) | CREATE_NEW_PROCESS_GROUP,
            );
        return command
            .spawn()
            .map_err(|error| format!("Failed to start DeepSeek Harness: {error}"));
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir(working_directory)
            .process_group(0);
        return command
            .spawn()
            .map_err(|error| format!("Failed to start DeepSeek Harness: {error}"));
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = (executable, working_directory, arguments);
        Err("The current platform does not support managed DeepSeek Harness startup".to_string())
    }
}

pub(crate) fn stop_managed_deepseek_harness(
    process_state: &DeepSeekHarnessProcessState,
) -> Result<(), String> {
    let mut process = process_state
        .process
        .lock()
        .map_err(|_| "DeepSeek Harness process status lock is poisoned".to_string())?;
    let Some(managed) = process.as_mut() else {
        return Ok(());
    };
    if managed
        .child
        .try_wait()
        .map_err(|error| format!("Failed to check DeepSeek Harness process status: {error}"))?
        .is_some()
    {
        *process = None;
        return Ok(());
    }
    terminate_deepseek_harness_process_tree(&mut managed.child)?;
    *process = None;
    Ok(())
}

fn terminate_deepseek_harness_process_tree(child: &mut Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        let process_id = child.id().to_string();
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &process_id, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        let status = command
            .status()
            .map_err(|error| format!("Failed to stop DeepSeek Harness process tree: {error}"))?;
        if !status.success() {
            return Err(format!(
                "Failed to stop DeepSeek Harness process tree: PID {process_id}"
            ));
        }
        let _ = child.wait();
        return Ok(());
    }

    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let term_status = Command::new("kill")
            .args(["-TERM", &process_group])
            .status()
            .map_err(|error| format!("Failed to stop DeepSeek Harness process group: {error}"))?;
        if !term_status.success() {
            return Err(format!(
                "Failed to stop DeepSeek Harness process group: PID {}",
                child.id()
            ));
        }
        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    return Err(format!("Failed to check DeepSeek Harness process status: {error}"));
                }
            }
        }
        let kill_status = Command::new("kill")
            .args(["-KILL", &process_group])
            .status()
            .map_err(|error| format!("Failed to force-stop DeepSeek Harness process group: {error}"))?;
        if !kill_status.success() {
            return Err(format!(
                "Failed to force-stop DeepSeek Harness process group: PID {}",
                child.id()
            ));
        }
        child
            .wait()
            .map_err(|error| format!("Failed while waiting for DeepSeek Harness process to exit: {error}"))?;
        return Ok(());
    }

    #[cfg(not(any(windows, unix)))]
    {
        child
            .kill()
            .map_err(|error| format!("Failed to stop DeepSeek Harness process: {error}"))?;
        child
            .wait()
            .map_err(|error| format!("Failed while waiting for DeepSeek Harness process to exit: {error}"))?;
        Ok(())
    }
}

fn validate_deepseek_harness_profile(profile: &str) -> Result<&str, String> {
    let profile = profile.trim();
    if profile.is_empty() {
        return Err("Enter a DeepSeek Harness profile name".to_string());
    }
    if matches!(profile, "." | "..")
        || !profile.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(
            "DeepSeek Harness profile name may contain only letters, numbers, periods, hyphens, and underscores".to_string(),
        );
    }
    Ok(profile)
}

fn validate_deepseek_harness_argument(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} cannot contain control characters"));
    }
    Ok(value.to_string())
}

#[tauri::command]
pub(crate) async fn restart_codex_app(app: tauri::AppHandle) -> Result<(), String> {
    restart_agent_app(app, "codex".to_string(), None).await
}

#[tauri::command]
pub(crate) async fn restart_opencode_app(app: tauri::AppHandle) -> Result<(), String> {
    restart_agent_app(app, "opencode".to_string(), None).await
}

#[tauri::command]
pub(crate) async fn restart_agent_app(
    app: tauri::AppHandle,
    client: String,
    executable_path: Option<String>,
) -> Result<(), String> {
    let client = AgentClient::parse(&client)?;
    if !matches!(
        client,
        AgentClient::Codex
            | AgentClient::OpenCode
            | AgentClient::ClaudeDesktop
            | AgentClient::ZCode
            | AgentClient::WorkBuddy
    ) {
        return Err(format!("{} does not support restarting the desktop application", client.name()));
    }
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("Failed to get user directory: {error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let executable_override = executable_path.as_deref().map(Path::new);
        let target = find_desktop_restart_target(client, &home, executable_override)?;
        match client {
            AgentClient::Codex => stop_codex_desktop(&target)?,
            AgentClient::OpenCode => {
                let DesktopAppTarget::Application(path) = &target else {
                    return Err("Invalid OpenCode desktop installation type".to_string());
                };
                stop_opencode_desktop(path)?;
            }
            _ => stop_other_desktop(&target, client.name())?,
        }
        match &target {
            DesktopAppTarget::Application(path) => launch_desktop_agent(path, client.name()),
            #[cfg(target_os = "windows")]
            DesktopAppTarget::WindowsAppId(app_id) => {
                launch_windows_store_app(app_id, client.name())
            }
        }
    })
    .await
    .map_err(|error| format!("Desktop application restart task failed: {error}"))?
}

fn find_desktop_restart_target(
    client: AgentClient,
    home: &Path,
    executable_override: Option<&Path>,
) -> Result<DesktopAppTarget, String> {
    if let Some(path) = manual_desktop_path(executable_override) {
        return Ok(DesktopAppTarget::Application(path.to_path_buf()));
    }
    let target = match client {
        AgentClient::Codex => find_codex_app_installation(home),
        AgentClient::OpenCode => find_opencode_desktop_application(home).map(DesktopAppTarget::Application),
        AgentClient::ZCode => {
            find_zcode_desktop_executable(home).map(DesktopAppTarget::Application)
        }
        AgentClient::WorkBuddy => {
            find_workbuddy_desktop_executable(home).map(DesktopAppTarget::Application)
        }
        AgentClient::ClaudeDesktop => {
            let executable = find_claude_desktop_executable(home).map(DesktopAppTarget::Application);
            #[cfg(target_os = "windows")]
            {
                executable
                    .or_else(|| find_windows_claude_app_id().map(DesktopAppTarget::WindowsAppId))
            }
            #[cfg(not(target_os = "windows"))]
            {
                executable
            }
        }
        _ => None,
    };
    target.ok_or_else(|| format!("{} desktop application was not detected. Detect it again", client.name()))
}

#[cfg(target_os = "windows")]
fn stop_other_desktop(target: &DesktopAppTarget, label: &str) -> Result<(), String> {
    let (executable, install_root) = windows_desktop_stop_target(target)?;
    stop_windows_matching_processes(executable.as_deref(), install_root.as_deref(), &[], label)
}

#[cfg(target_os = "macos")]
fn stop_other_desktop(target: &DesktopAppTarget, label: &str) -> Result<(), String> {
    let DesktopAppTarget::Application(path) = target;
    stop_macos_desktop_application(path, label)
}

#[cfg(target_os = "linux")]
fn stop_other_desktop(target: &DesktopAppTarget, label: &str) -> Result<(), String> {
    let DesktopAppTarget::Application(path) = target;
    stop_linux_desktop_application(path, label)
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn stop_other_desktop(_target: &DesktopAppTarget, label: &str) -> Result<(), String> {
    Err(format!("The current platform does not support restarting {label}"))
}

fn resolve_launch_directory(value: Option<&str>, fallback: &Path) -> Result<PathBuf, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(fallback.to_path_buf());
    };
    if value.chars().any(char::is_control) {
        return Err("Working directory contains invalid characters".to_string());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("Working directory must be an absolute path".to_string());
    }
    if !path.is_dir() {
        return Err(format!("Working directory does not exist: {}", path_to_string(&path)));
    }
    Ok(path)
}

fn launch_codex_desktop(home: &Path, executable_override: Option<&Path>) -> Result<(), String> {
    let target = find_desktop_restart_target(AgentClient::Codex, home, executable_override)?;
    launch_codex_target(&target)
}

fn launch_codex_target(target: &DesktopAppTarget) -> Result<(), String> {
    match target {
        #[cfg(target_os = "windows")]
        DesktopAppTarget::WindowsAppId(app_id) => launch_windows_store_app(app_id, "Codex App"),
        DesktopAppTarget::Application(path) => launch_desktop_agent(path, "Codex App"),
    }
}

#[cfg(target_os = "windows")]
fn stop_codex_desktop(target: &DesktopAppTarget) -> Result<(), String> {
    let (executable, install_root) = windows_desktop_stop_target(target)?;
    stop_windows_matching_processes(
        executable.as_deref(),
        install_root.as_deref(),
        &["ChatGPT.exe", "Codex.exe"],
        "Codex App",
    )
}

#[cfg(target_os = "windows")]
fn stop_opencode_desktop(application: &Path) -> Result<(), String> {
    stop_windows_matching_processes(
        Some(application),
        None,
        &["OpenCode.exe"],
        "OpenCode Desktop",
    )
}

#[cfg(target_os = "macos")]
fn stop_codex_desktop(target: &DesktopAppTarget) -> Result<(), String> {
    let DesktopAppTarget::Application(application) = target;
    stop_macos_desktop_application(application, "Codex App")
}

#[cfg(target_os = "macos")]
fn stop_opencode_desktop(application: &Path) -> Result<(), String> {
    stop_macos_desktop_application(application, "OpenCode Desktop")
}

#[cfg(any(target_os = "macos", test))]
fn desktop_process_ids_from_ps(output: &str, application: &Path) -> Vec<i32> {
    let bundle = application
        .ancestors()
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"));
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let split = line.find(char::is_whitespace)?;
            let pid = line[..split].parse::<i32>().ok()?;
            if pid <= 0 || pid == std::process::id() as i32 {
                return None;
            }
            let executable = Path::new(line[split..].trim());
            (executable == application || bundle.is_some_and(|root| executable.starts_with(root)))
                .then_some(pid)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_processes_for_application(application: &Path) -> Result<Vec<i32>, String> {
    let output = Command::new("ps")
        .args(["-axww", "-o", "pid=", "-o", "comm="])
        .output()
        .map_err(|error| format!("Failed to read desktop application processes: {error}"))?;
    if !output.status.success() {
        return Err("Failed to read desktop application processes".to_string());
    }
    Ok(desktop_process_ids_from_ps(
        &String::from_utf8_lossy(&output.stdout),
        application,
    ))
}

#[cfg(target_os = "macos")]
fn stop_macos_desktop_application(application: &Path, label: &str) -> Result<(), String> {
    if macos_processes_for_application(application)?.is_empty() {
        return Ok(());
    }
    let bundle = application
        .ancestors()
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        .unwrap_or(application);
    let escaped = path_to_string(bundle)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let mut quit = Command::new("osascript");
    quit.args(["-e", &format!("tell application \"{escaped}\" to quit")]);
    let _ = command_output_with_timeout(&mut quit, Duration::from_secs(5));
    for _ in 0..50 {
        if macos_processes_for_application(application)?.is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    for pid in macos_processes_for_application(application)? {
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("Failed to stop {label} process {pid}: {error}"));
            }
        }
    }
    for _ in 0..10 {
        if macos_processes_for_application(application)?.is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{label} did not close completely"))
}

#[cfg(target_os = "linux")]
fn stop_opencode_desktop(application: &Path) -> Result<(), String> {
    stop_linux_desktop_application(application, "OpenCode Desktop")
}

#[cfg(target_os = "linux")]
fn stop_linux_desktop_application(application: &Path, label: &str) -> Result<(), String> {
    let application = fs::canonicalize(application).unwrap_or_else(|_| application.to_path_buf());
    let processes = linux_processes_for_application(&application);
    let mut signal_error = None;
    for process_id in processes {
        if let Err(error) = signal_linux_process(process_id, libc::SIGTERM) {
            signal_error.get_or_insert(error);
        }
    }

    for _ in 0..50 {
        if linux_processes_for_application(&application).is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    let remaining = linux_processes_for_application(&application);
    for process_id in &remaining {
        if let Err(error) = signal_linux_process(*process_id, libc::SIGKILL) {
            signal_error.get_or_insert(error);
        }
    }
    for _ in 0..10 {
        if linux_processes_for_application(&application).is_empty() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    let remaining = linux_processes_for_application(&application);
    if remaining.is_empty() {
        Ok(())
    } else if let Some(error) = signal_error {
        Err(error)
    } else {
        Err(format!(
            "{label} did not close completely; remaining process IDs: {}",
            remaining
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

#[cfg(target_os = "linux")]
fn linux_processes_for_application(application: &Path) -> Vec<i32> {
    let current_process = std::process::id() as i32;
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut process_ids = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|process_id| *process_id != current_process)
        .filter(|process_id| linux_process_matches_application(*process_id, application))
        .collect::<Vec<_>>();
    process_ids.sort_unstable();
    process_ids
}

#[cfg(target_os = "linux")]
fn linux_process_matches_application(process_id: i32, application: &Path) -> bool {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let process_directory = PathBuf::from(format!("/proc/{process_id}"));
    if fs::read_link(process_directory.join("exe"))
        .ok()
        .is_some_and(|path| linux_process_path_matches(&path, application))
    {
        return true;
    }

    if let Ok(command_line) = fs::read(process_directory.join("cmdline")) {
        let installation_root = linux_opencode_nix_installation_root(application);
        if command_line
            .split(|byte| *byte == 0)
            .filter(|argument| !argument.is_empty())
            .map(|argument| Path::new(OsStr::from_bytes(argument)))
            .enumerate()
            .any(|(index, argument)| {
                (index == 0 && linux_process_path_matches(argument, application))
                    || installation_root.is_some_and(|root| {
                        fs::canonicalize(argument)
                            .ok()
                            .is_some_and(|argument| argument.starts_with(root))
                    })
            })
        {
            return true;
        }
    }

    fs::read(process_directory.join("environ"))
        .ok()
        .is_some_and(|environment| {
            environment
                .split(|byte| *byte == 0)
                .filter_map(|entry| entry.strip_prefix(b"APPIMAGE="))
                .any(|path| {
                    !path.is_empty()
                        && linux_process_path_matches(
                            Path::new(OsStr::from_bytes(path)),
                            application,
                        )
                })
        })
}

#[cfg(target_os = "linux")]
fn linux_process_path_matches(candidate: &Path, application: &Path) -> bool {
    candidate == application
        || fs::canonicalize(candidate)
            .ok()
            .is_some_and(|candidate| candidate == application)
}

#[cfg(target_os = "linux")]
fn linux_opencode_nix_installation_root(application: &Path) -> Option<&Path> {
    application.ancestors().find(|ancestor| {
        ancestor.parent() == Some(Path::new("/nix/store"))
            && ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().contains("opencode"))
    })
}

#[cfg(target_os = "linux")]
fn signal_linux_process(process_id: i32, signal: i32) -> Result<(), String> {
    if unsafe { libc::kill(process_id, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!("Failed to stop desktop application process {process_id}: {error}"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn stop_codex_desktop(_target: &DesktopAppTarget) -> Result<(), String> {
    Err("The current platform does not support restarting Codex App".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn stop_opencode_desktop(_application: &Path) -> Result<(), String> {
    Err("The current platform does not support restarting OpenCode Desktop".to_string())
}

fn launch_claude_desktop(home: &Path, executable_override: Option<&Path>) -> Result<(), String> {
    let executable = manual_desktop_path(executable_override).map(Path::to_path_buf)
        .or_else(|| find_claude_desktop_executable(home));
    if let Some(executable) = executable {
        return launch_desktop_agent(&executable, "Claude Desktop");
    }
    #[cfg(target_os = "windows")]
    {
        launch_windows_claude_store_app()
    }
    #[cfg(not(target_os = "windows"))]
    Err("Claude Desktop application was not detected. Install or detect it again".to_string())
}

fn launch_opencode_desktop(home: &Path, executable_override: Option<&Path>) -> Result<(), String> {
    let application = manual_desktop_path(executable_override).map(Path::to_path_buf)
        .or_else(|| find_opencode_desktop_application(home))
        .ok_or_else(|| "OpenCode Desktop application was not detected. Install or detect it again".to_string())?;
    launch_desktop_agent(&application, "OpenCode Desktop")
}

fn launch_desktop_agent(executable: &Path, label: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let application = executable
            .ancestors()
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
            .unwrap_or(executable);
        let mut command = Command::new("open");
        command.arg(application);
        command
    };

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let mut command = Command::new(executable);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (executable, label);
        return Err("The current platform does not support desktop agents".to_string());
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Failed to start {label}: {error}"))
    }
}

#[cfg(target_os = "windows")]
fn launch_windows_store_app(app_id: &str, label: &str) -> Result<(), String> {
    let mut command = Command::new(windows_explorer_executable());
    command
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to start {label}: {error}"))
}

#[cfg(target_os = "windows")]
fn launch_windows_claude_store_app() -> Result<(), String> {
    let app_id = find_windows_claude_app_id()
        .ok_or_else(|| "No launchable Claude Desktop application found".to_string())?;
    launch_windows_store_app(&app_id, "Claude Desktop")
}

#[cfg(target_os = "macos")]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "macos")]
fn launch_cli_agent(
    executable: &Path,
    label: &str,
    working_directory: &Path,
    arguments: &[String],
    environment_to_remove: &[&str],
    environment_to_set: &[(&str, &str)],
    terminal: &str,
) -> Result<(), String> {
    let removals = environment_to_remove
        .iter()
        .map(|key| format!("-u {}", shell_single_quote(key)))
        .collect::<Vec<_>>()
        .join(" ");
    let assignments = environment_to_set
        .iter()
        .map(|(key, value)| format!("{}={}", shell_single_quote(key), shell_single_quote(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let invocation = std::iter::once(shell_single_quote(&path_to_string(executable)))
        .chain(
            arguments
                .iter()
                .map(|argument| shell_single_quote(argument)),
        )
        .collect::<Vec<_>>()
        .join(" ");
    let command_line = format!(
        "cd {} && exec env {} {} {}",
        shell_single_quote(&path_to_string(working_directory)),
        removals,
        assignments,
        invocation,
    );
    let script = if terminal == "iterm2" {
        format!(
            "tell application \"iTerm2\"\nactivate\nset newWindow to (create window with default profile)\ntell current session of newWindow\nwrite text \"{}\"\nend tell\nend tell",
            command_line.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else if terminal == "ghostty" {
        format!(
            "tell application \"Ghostty\"\nactivate\nset surfaceConfig to new surface configuration\nset initial input of surfaceConfig to \"{}\\n\"\nnew window with configuration surfaceConfig\nend tell",
            command_line.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else if matches!(terminal, "auto" | "terminal") {
        format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            command_line.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        return Err(format!("Failed to start {label}: selected terminal is unsupported"));
    };
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("Failed to start {label} terminal: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("Failed to start {label} terminal: {detail}"))
    }
}

#[cfg(target_os = "linux")]
fn launch_cli_agent(
    executable: &Path,
    label: &str,
    working_directory: &Path,
    command_arguments: &[String],
    environment_to_remove: &[&str],
    environment_to_set: &[(&str, &str)],
    terminal: &str,
) -> Result<(), String> {
    let definitions = linux_terminal_definitions();
    let mut last_error = None;
    for definition in definitions {
        let (id, program, arguments, _) = *definition;
        if terminal != "auto" && id != terminal {
            continue;
        }
        if program_on_path(program).is_none() {
            continue;
        }
        let mut command = Command::new(program);
        command
            .args(arguments)
            .arg(executable)
            .args(command_arguments)
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for key in environment_to_remove {
            command.env_remove(key);
        }
        for (key, value) in environment_to_set {
            command.env(key, value);
        }
        match command.spawn() {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if terminal != "auto" && last_error.is_none() {
        return Err(format!("Failed to start {label}: selected terminal not found"));
    }
    Err(match last_error {
        Some(error) => format!("Failed to start {label}: {error}"),
        None => format!("Failed to start {label}: no available terminal program found"),
    })
}

#[cfg(target_os = "windows")]
fn windows_powershell_cli_script(
    executable: &Path,
    working_directory: &Path,
    arguments: &[String],
) -> String {
    let directory = windows_powershell_single_quoted_literal(&path_to_string(working_directory));
    let executable = windows_powershell_single_quoted_literal(&path_to_string(executable));
    if arguments
        .first()
        .is_some_and(|arg| arg == "--cpa-antigravity-cli")
    {
        let arguments = arguments
            .iter()
            .map(|argument| {
                let mut quoted = String::from("\"");
                let mut backslashes = 0;
                for ch in argument.chars() {
                    if ch == '\\' {
                        backslashes += 1;
                        continue;
                    }
                    quoted.push_str(&"\\".repeat(if ch == '"' {
                        backslashes * 2 + 1
                    } else {
                        backslashes
                    }));
                    backslashes = 0;
                    quoted.push(ch);
                }
                quoted.push_str(&"\\".repeat(backslashes * 2));
                quoted.push('"');
                quoted
            })
            .collect::<Vec<_>>()
            .join(" ");
        return format!(
            "Set-Location -LiteralPath {directory}; \
             $cpaLaunchInfo = New-Object System.Diagnostics.ProcessStartInfo; \
             $cpaLaunchInfo.FileName = {executable}; \
             $cpaLaunchInfo.WorkingDirectory = {directory}; \
             $cpaLaunchInfo.UseShellExecute = $false; \
             $cpaLaunchInfo.Arguments = {}; \
             $cpaLaunchProcess = [System.Diagnostics.Process]::Start($cpaLaunchInfo); \
             try {{ $cpaLaunchProcess.WaitForExit(); $global:LASTEXITCODE = $cpaLaunchProcess.ExitCode }} \
             finally {{ $cpaLaunchProcess.Dispose() }}",
            windows_powershell_single_quoted_literal(&arguments),
        );
    }
    let arguments = arguments
        .iter()
        .map(|argument| windows_powershell_single_quoted_literal(argument))
        .collect::<Vec<_>>()
        .join(" ");
    format!("Set-Location -LiteralPath {directory}; & {executable} {arguments}")
}

#[cfg(target_os = "windows")]
fn launch_cli_agent(
    executable: &Path,
    label: &str,
    working_directory: &Path,
    arguments: &[String],
    environment_to_remove: &[&str],
    environment_to_set: &[(&str, &str)],
    terminal: &str,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let is_batch_script = executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });

    let mut command = match terminal {
        "auto" => {
            let mut command = windows_command_for_executable(executable, true);
            command
                .args(arguments)
                .current_dir(working_directory)
                .creation_flags(CREATE_NEW_CONSOLE);
            command
        }
        "windows-terminal" => {
            let terminal_executable = program_on_path("wt.exe")
                .ok_or_else(|| format!("Failed to start {label}: Windows Terminal not found"))?;
            let directory = path_to_string(working_directory);
            let mut command = Command::new(terminal_executable);
            command.args(["-d", &directory, "--"]);
            if is_batch_script {
                command
                    .arg(windows_command_processor())
                    .args(["/D", "/K", "call"])
                    .arg(windows_batch_executable_argument(executable))
                    .args(arguments);
            } else {
                command.arg(executable).args(arguments);
            }
            command
        }
        "powershell" => {
            let mut command = Command::new(windows_powershell_executable());
            let script = windows_powershell_cli_script(executable, working_directory, arguments);
            command.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &script]);
            command.creation_flags(CREATE_NEW_CONSOLE);
            command
        }
        "cmd" => {
            let mut command = Command::new(windows_command_processor());
            let arguments = arguments
                .iter()
                .map(|argument| format!(" \"{}\"", argument.replace('"', "\"\"")))
                .collect::<String>();
            let command_line = format!(
                "cd /d \"{}\" && call \"{}\"{}",
                path_to_string(working_directory).replace('"', "\"\""),
                path_to_string(executable).replace('"', "\"\""),
                arguments,
            );
            command.args(["/D", "/K", &command_line]);
            command.creation_flags(CREATE_NEW_CONSOLE);
            command
        }
        _ => return Err(format!("Failed to start {label}: selected terminal is unsupported")),
    };
    for key in environment_to_remove {
        command.env_remove(key);
    }
    for (key, value) in environment_to_set {
        command.env(key, value);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Failed to start {label}: {error}"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn launch_cli_agent(
    _executable: &Path,
    label: &str,
    _working_directory: &Path,
    _arguments: &[String],
    _environment_to_remove: &[&str],
    _environment_to_set: &[(&str, &str)],
    _terminal: &str,
) -> Result<(), String> {
    Err(format!("The current platform does not support launching {label}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_desktop_paths_drive_detection_and_restart_without_replacing_cli_paths() {
        let root = env::temp_dir().join(format!("cpa-manual-desktop-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        let desktop = root.join("Custom Desktop.exe");
        let cli = root.join("custom-cli.exe");
        fs::write(&desktop, b"desktop fixture").unwrap();
        fs::write(&cli, b"cli fixture").unwrap();
        for client in [AgentClient::Codex, AgentClient::OpenCode, AgentClient::DeepSeekHarness] {
            let status = inspect_agent_config_with_executables(client, &root, 8317, "test-key", Some(&cli), Some(&desktop));
            assert!(status.installed, "{}", client.name());
            assert!(status.launch_targets.iter().any(|target| target.id == "app"), "{}", client.name());
            let cli_target = status.launch_targets.iter().find(|target| target.id == "cli").unwrap();
            assert!(cli_target.detail.contains("custom-cli.exe"));
            assert!(!cli_target.detail.contains("Custom Desktop.exe"));
        }
        for client in [AgentClient::ClaudeDesktop, AgentClient::ZCode, AgentClient::WorkBuddy] {
            let status = inspect_agent_config_with_executable(client, &root, 8317, "test-key", Some(&desktop));
            assert!(status.installed, "{}", client.name());
            assert_eq!(status.launch_targets.len(), 1);
            assert_eq!(status.launch_targets[0].id, "app");
            assert_eq!(status.launch_targets[0].detail, path_to_string(&desktop));
        }
        for client in [AgentClient::Codex, AgentClient::OpenCode, AgentClient::ClaudeDesktop,
            AgentClient::ZCode, AgentClient::WorkBuddy] {
            match find_desktop_restart_target(client, &root, Some(&desktop)).unwrap() {
                DesktopAppTarget::Application(path) => assert_eq!(path, desktop),
                #[cfg(target_os = "windows")]
                DesktopAppTarget::WindowsAppId(_) => panic!("manual path must take precedence"),
            }
        }
        assert!(manual_desktop_path(Some(&root.join("missing.exe"))).is_none());
        assert!(manual_desktop_path(Some(&root)).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    fn deepseek_harness_options(mode: &str) -> DeepSeekHarnessLaunchOptions {
        DeepSeekHarnessLaunchOptions {
            mode: mode.to_string(),
            web_host: None,
            web_port: None,
            open_browser: None,
            trusted_hosts: Vec::new(),
            task: None,
            profile: None,
            patches: Vec::new(),
        }
    }

    #[test]
    fn unknown_terminal_defaults_to_automatic() {
        assert_eq!(
            normalize_agent_terminal("missing-terminal"),
            DEFAULT_AGENT_TERMINAL
        );
        assert_eq!(normalize_agent_terminal(" auto "), "auto");
    }

    #[cfg(windows)]
    #[test]
    fn antigravity_powershell_waits_for_gui_helper_and_preserves_arguments() {
        let directory = env::temp_dir()
            .join(format!(
                "cpa-powershell-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ))
            .join("User's [workspace] $value");
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("GUI helper.exe");
        let compiled = directory.parent().unwrap().join("helper.exe");
        let source = r#"
using System;
using System.IO;
using System.Threading;
class GuiHelper {
    static void Main(string[] args) {
        Thread.Sleep(350);
        File.WriteAllLines(Path.Combine(Environment.CurrentDirectory, "arguments.txt"), args);
        File.WriteAllText(Path.Combine(Environment.CurrentDirectory, "finished.txt"), "done");
    }
}
"#;
        let script = format!(
            "Add-Type -TypeDefinition {} -OutputAssembly {} -OutputType WindowsApplication -ErrorAction Stop",
            windows_powershell_single_quoted_literal(source),
            windows_powershell_single_quoted_literal(&path_to_string(&compiled)),
        );
        let run = |script: &str| {
            let mut command = Command::new(windows_powershell_executable());
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ]);
            configure_background_command(&mut command);
            let result = command_output_with_timeout(&mut command, Duration::from_secs(20))
                .unwrap()
                .expect("PowerShell timed out");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        run(&script);
        fs::rename(compiled, &executable).unwrap();
        let arguments = vec![
            "--cpa-antigravity-cli".into(),
            path_to_string(&directory),
            "trailing slash \\".into(),
            "quote \" and \\\"".into(),
            "literal $value & [x] 'quoted'".into(),
            String::new(),
        ];
        let script = format!(
            "{}; if (![IO.File]::Exists({})) {{ throw 'GUI helper is still running' }}",
            windows_powershell_cli_script(&executable, &directory, &arguments),
            windows_powershell_single_quoted_literal(&path_to_string(
                &directory.join("finished.txt")
            )),
        );
        run(&script);
        let received = fs::read_to_string(directory.join("arguments.txt")).unwrap();
        assert_eq!(
            received.lines().collect::<Vec<_>>(),
            arguments.iter().map(String::as_str).collect::<Vec<_>>()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn desktop_process_matching_is_scoped_to_the_exact_bundle() {
        let application = Path::new("/Applications/Claude Desktop.app/Contents/MacOS/Claude");
        let processes = "10 /Applications/Claude Desktop.app/Contents/MacOS/Claude\n11 /Applications/Claude Desktop.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper\n12 /Applications/Claude Desktop.app.old/Contents/MacOS/Claude\n13 /Users/test/Claude Desktop.app/Contents/MacOS/Claude\n14 /usr/local/bin/claude\n";
        assert_eq!(
            desktop_process_ids_from_ps(processes, application),
            vec![10, 11]
        );
        assert!(desktop_process_ids_from_ps(
            "bad input\n-1 /opt/desktop",
            Path::new("/opt/desktop")
        )
        .is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn desktop_restart_matching_is_scoped_to_the_exact_installation() {
        let claude = PathBuf::from(r"C:\Apps\Claude's Desktop\Claude.exe");
        assert!(windows_process_matches_stop_target(
            &claude,
            Some(&claude),
            None,
            &[],
        ));
        assert!(!windows_process_matches_stop_target(
            Path::new(r"C:\Apps\Claude's Desktop.old\Claude.exe"),
            Some(&claude),
            None,
            &[],
        ));
        let zcode = PathBuf::from(r"C:\Apps\ZCode\ZCode.exe");
        assert!(windows_process_matches_stop_target(
            &zcode,
            Some(&zcode),
            None,
            &[]
        ));
        let store_root = PathBuf::from(r"C:\Program Files\WindowsApps\Anthropic.Claude_family");
        assert!(windows_process_matches_stop_target(
            &store_root.join("Claude.exe"),
            None,
            Some(&store_root),
            &[],
        ));
        assert!(!windows_process_matches_stop_target(
            Path::new(r"C:\Program Files\WindowsApps\Other\Claude.exe"),
            None,
            Some(&store_root),
            &[],
        ));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "helper launched only by the isolated desktop restart test"]
    fn desktop_restart_test_process() {
        if let Some(marker) = env::var_os("CPA_DESKTOP_RESTART_TEST_MARKER") {
            fs::write(marker, "ready").unwrap();
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[cfg(windows)]
    #[test]
    fn desktop_stop_waits_for_only_the_selected_installation() {
        struct Processes {
            children: Vec<Child>,
            directory: PathBuf,
        }
        impl Drop for Processes {
            fn drop(&mut self) {
                for child in &mut self.children {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                let _ = fs::remove_dir_all(&self.directory);
            }
        }
        let directory = env::temp_dir().join(format!(
            "cpa-desktop-restart-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let executable = env::current_exe().unwrap();
        let target = directory.join("DesktopTest.exe");
        fs::copy(&executable, &target).unwrap();
        let mut processes = Processes {
            children: Vec::new(),
            directory,
        };
        for (index, application) in [&target, &executable].into_iter().enumerate() {
            let marker = processes.directory.join(format!("ready-{index}"));
            let mut command = Command::new(application);
            command
                .args([
                    "--exact",
                    "agents::launch::tests::desktop_restart_test_process",
                    "--ignored",
                ])
                .env("CPA_DESKTOP_RESTART_TEST_MARKER", &marker)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            configure_background_command(&mut command);
            processes.children.push(command.spawn().unwrap());
            for _ in 0..100 {
                if marker.exists() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            assert!(marker.exists(), "desktop helper did not start");
        }
        let installation = DesktopAppTarget::Application(target);
        stop_other_desktop(&installation, "Test Desktop").unwrap();
        assert!(processes.children[0].try_wait().unwrap().is_some());
        assert!(
            processes.children[1].try_wait().unwrap().is_none(),
            "unrelated installation must keep running"
        );
        stop_other_desktop(&installation, "Test Desktop").unwrap();
    }

    struct HarnessRestartFixture {
        state: DeepSeekHarnessProcessState,
        directory: PathBuf,
    }

    impl Drop for HarnessRestartFixture {
        fn drop(&mut self) {
            let _ = stop_managed_deepseek_harness(&self.state);
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn harness_restart_fixture() -> HarnessRestartFixture {
        let directory = env::temp_dir().join(format!(
            "cpa-harness-restart-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        #[cfg(windows)]
        let (executable, arguments) = (windows_powershell_executable(), vec![
            "-NoLogo".to_string(), "-NoProfile".to_string(), "-NonInteractive".to_string(), "-Command".to_string(),
            "[IO.File]::AppendAllText((Join-Path (Get-Location).Path 'launches.txt'), ((Get-Location).Path + [Environment]::NewLine)); Start-Sleep -Seconds 60".to_string(),
        ]);
        #[cfg(unix)]
        let (executable, arguments) = (
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_string(),
                "pwd >> launches.txt; sleep 60".to_string(),
            ],
        );
        let mut options = deepseek_harness_options("web");
        options.web_port = Some(0);
        options.trusted_hosts = vec!["localhost".to_string()];
        options.patches = vec!["preserve a value with spaces".to_string()];
        let fixture = HarnessRestartFixture {
            state: DeepSeekHarnessProcessState::default(),
            directory,
        };
        launch_managed_deepseek_harness(
            &fixture.state,
            &executable,
            &fixture.directory,
            &arguments,
            "web",
            Some(&options),
        )
        .unwrap();
        wait_for_harness_launches(&fixture, 1);
        fixture
    }

    fn wait_for_harness_launches(fixture: &HarnessRestartFixture, expected: usize) {
        for _ in 0..100 {
            if fs::read_to_string(fixture.directory.join("launches.txt"))
                .unwrap_or_default()
                .lines()
                .count()
                >= expected
            {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("test process did not record launch {expected}");
    }

    #[test]
    fn harness_web_restart_preserves_snapshot_and_serializes_stop_and_spawn() {
        let fixture = harness_restart_fixture();
        let old_pid = fixture.state.status().unwrap().pid;
        let snapshot = fixture
            .state
            .process
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .launch
            .clone();
        let status = restart_managed_deepseek_harness_with(
            &fixture.state,
            |child| {
                assert!(fixture.state.process.try_lock().is_err());
                terminate_deepseek_harness_process_tree(child)
            },
            |executable, directory, arguments| {
                assert!(fixture.state.process.try_lock().is_err());
                assert_eq!(executable, snapshot.executable);
                assert_eq!(directory, snapshot.working_directory);
                assert_eq!(arguments, snapshot.arguments);
                spawn_managed_deepseek_harness(executable, directory, arguments)
            },
        )
        .unwrap();
        assert!(status.running);
        assert_ne!(status.pid, old_pid);
        wait_for_harness_launches(&fixture, 2);
        let launches = fs::read_to_string(fixture.directory.join("launches.txt")).unwrap();
        let lines = launches.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], lines[1]);
        let process = fixture.state.process.lock().unwrap();
        let options = process.as_ref().unwrap().launch.options.as_ref().unwrap();
        assert_eq!(options.web_port, Some(0));
        assert_eq!(options.trusted_hosts, vec!["localhost"]);
        assert_eq!(options.patches, vec!["preserve a value with spaces"]);
    }

    #[test]
    fn harness_restart_stop_failure_retains_process_and_does_not_spawn() {
        let fixture = harness_restart_fixture();
        let pid = fixture.state.status().unwrap().pid;
        let result = restart_managed_deepseek_harness_with(
            &fixture.state,
            |_| Err("stop failed".to_string()),
            |_, _, _| panic!("must not spawn after stop failure"),
        );
        assert_eq!(result.unwrap_err(), "stop failed");
        assert_eq!(fixture.state.status().unwrap().pid, pid);
    }

    #[test]
    fn harness_restart_spawn_failure_reports_stopped_state() {
        let fixture = harness_restart_fixture();
        let result = restart_managed_deepseek_harness_with(
            &fixture.state,
            terminate_deepseek_harness_process_tree,
            |_, _, _| Err("spawn failed".to_string()),
        );
        assert_eq!(result.unwrap_err(), "spawn failed");
        assert!(!fixture.state.status().unwrap().running);
    }

    #[test]
    fn harness_restart_rejects_missing_or_non_web_process_without_stopping_it() {
        assert!(restart_managed_deepseek_harness(&DeepSeekHarnessProcessState::default()).is_err());
        let fixture = harness_restart_fixture();
        let pid = fixture.state.status().unwrap().pid;
        fixture.state.process.lock().unwrap().as_mut().unwrap().mode = "headless".to_string();
        assert!(restart_managed_deepseek_harness(&fixture.state)
            .unwrap_err()
            .contains("Web"));
        assert_eq!(fixture.state.status().unwrap().pid, pid);
    }

    #[test]
    fn relative_launch_directory_is_rejected() {
        let error = resolve_launch_directory(Some("relative/project"), Path::new("/fallback"))
            .expect_err("relative path should be rejected");
        assert!(error.contains("absolute path"));
    }

    #[test]
    fn omitted_launch_directory_uses_fallback() {
        let fallback = if cfg!(target_os = "windows") {
            Path::new(r"C:\Users\tester")
        } else {
            Path::new("/home/tester")
        };
        assert_eq!(resolve_launch_directory(None, fallback).unwrap(), fallback);
    }

    #[test]
    fn deepseek_harness_launch_selects_the_web_profile() {
        assert_eq!(
            agent_cli_launch_arguments(AgentClient::DeepSeekHarness, None).unwrap(),
            ["web"]
        );
        assert!(agent_cli_launch_arguments(AgentClient::Codex, None)
            .unwrap()
            .is_empty());

        let targets = agent_launch_targets(
            AgentClient::DeepSeekHarness,
            Some(Path::new("dsh")),
            Some("0.1.2-rc.1"),
            false,
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "cli");
        assert_eq!(targets[0].label, "DeepSeek Harness Web");
        assert_eq!(targets[0].detail, "dsh web");
    }

    #[test]
    fn deepseek_harness_web_launch_arguments_include_supported_options() {
        let mut options = deepseek_harness_options("web");
        options.web_host = Some("127.0.0.1".to_string());
        options.web_port = Some(0);
        options.open_browser = Some(false);
        options.trusted_hosts = vec!["localhost:3000".to_string(), "example.test".to_string()];
        options.patches = vec!["./extra.yml".to_string()];

        assert_eq!(
            build_deepseek_harness_launch_arguments(Some(&options)).unwrap(),
            [
                "web",
                "--patch",
                "./extra.yml",
                "--host",
                "127.0.0.1",
                "--port",
                "0",
                "--no-open",
                "--trusted-host",
                "localhost:3000",
                "--trusted-host",
                "example.test",
            ]
        );
    }

    #[test]
    fn deepseek_harness_non_web_profiles_build_expected_arguments() {
        for mode in ["acp", "sdk", "sdk-minimal"] {
            let options = deepseek_harness_options(mode);
            assert_eq!(
                build_deepseek_harness_launch_arguments(Some(&options)).unwrap(),
                ["--profile", mode]
            );
        }

        let mut headless = deepseek_harness_options("headless");
        headless.task = Some("review this repository".to_string());
        assert_eq!(
            build_deepseek_harness_launch_arguments(Some(&headless)).unwrap(),
            ["--profile", "headless", "review this repository"]
        );

        let mut custom = deepseek_harness_options("custom");
        custom.profile = Some("tui-dev".to_string());
        assert_eq!(
            build_deepseek_harness_launch_arguments(Some(&custom)).unwrap(),
            ["--profile", "tui-dev"]
        );
    }

    #[test]
    fn deepseek_harness_launch_options_reject_missing_required_values() {
        let headless = deepseek_harness_options("headless");
        assert!(build_deepseek_harness_launch_arguments(Some(&headless)).is_err());

        let custom = deepseek_harness_options("custom");
        assert!(build_deepseek_harness_launch_arguments(Some(&custom)).is_err());

        let options = deepseek_harness_options("web");
        assert!(agent_cli_launch_arguments(AgentClient::Codex, Some(&options)).is_err());
    }

    #[test]
    fn managed_deepseek_harness_state_stops_only_its_tracked_process() {
        #[cfg(windows)]
        let child = {
            let mut command = Command::new(windows_powershell_executable());
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]);
            configure_background_command(&mut command);
            command.spawn().unwrap()
        };
        #[cfg(unix)]
        let child = {
            use std::os::unix::process::CommandExt;

            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30"]).process_group(0);
            command.spawn().unwrap()
        };

        let state = DeepSeekHarnessProcessState::default();
        let tracked_pid = child.id();
        *state.process.lock().unwrap() = Some(ManagedDeepSeekHarnessProcess {
            child,
            mode: "test".to_string(),
            launch: DeepSeekHarnessLaunchSnapshot {
                executable: PathBuf::new(),
                working_directory: PathBuf::new(),
                arguments: Vec::new(),
                options: None,
            },
        });

        let running = state.status().unwrap();
        assert!(running.running);
        assert_eq!(running.pid, Some(tracked_pid));
        assert_eq!(running.mode.as_deref(), Some("test"));

        stop_managed_deepseek_harness(&state).unwrap();
        assert!(!state.status().unwrap().running);
    }

    #[test]
    fn deepseek_harness_web_launch_rejects_an_occupied_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut options = deepseek_harness_options("web");
        options.web_port = Some(port);

        let error = ensure_deepseek_harness_web_endpoint_available(Some(&options)).unwrap_err();

        assert!(error.contains("is in use"), "{error}");
    }

    #[test]
    fn deepseek_harness_web_launch_allows_random_ports() {
        let mut options = deepseek_harness_options("web");
        options.web_port = Some(0);

        ensure_deepseek_harness_web_endpoint_available(Some(&options)).unwrap();
    }

    #[test]
    fn codex_exposes_independent_app_and_cli_targets() {
        let executable = Path::new("codex");
        let targets =
            agent_launch_targets(AgentClient::Codex, Some(executable), Some("1.0.0"), true);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            ["app", "cli"]
        );
    }

    #[test]
    fn opencode_desktop_is_available_without_a_cli() {
        let targets = agent_launch_targets(AgentClient::OpenCode, None, None, true);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            ["app"]
        );
        assert_eq!(targets[0].label, "OpenCode Desktop");
    }

    #[test]
    fn opencode_keeps_the_cli_as_the_default_when_both_are_installed() {
        let executable = Path::new("opencode");
        let targets = agent_launch_targets(AgentClient::OpenCode, Some(executable), None, true);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            ["cli", "app"]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opencode_linux_restart_matches_the_exact_appimage_environment() {
        let directory = env::temp_dir().join(format!(
            "cpa-opencode-linux-restart-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let application = directory.join("opencode-desktop-linux-amd64.AppImage");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&application, []).unwrap();
        let mut child = Command::new("sleep")
            .arg("30")
            .env("APPIMAGE", &application)
            .spawn()
            .unwrap();
        thread::sleep(Duration::from_millis(100));

        stop_opencode_desktop(&application).unwrap();
        child.wait().unwrap();
        assert!(child.try_wait().unwrap().is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn desktop_and_cli_clients_get_the_correct_target_kind() {
        let executable = Path::new("agent");
        let zcode =
            agent_launch_targets(AgentClient::ZCode, Some(executable), Some("1.0.0"), false);
        let kimi = agent_launch_targets(AgentClient::KimiCode, Some(executable), None, false);
        assert_eq!(zcode[0].id, "app");
        assert_eq!(kimi[0].id, "cli");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn codex_restart_matching_is_scoped_to_the_detected_installation() {
        let executable = PathBuf::from(r"C:\Apps\Codex\Codex.exe");
        assert!(windows_process_matches_stop_target(
            &executable,
            Some(&executable),
            None,
            &["ChatGPT.exe", "Codex.exe"],
        ));
        assert!(!windows_process_matches_stop_target(
            Path::new(r"C:\Apps\Codex\helper.exe"),
            Some(&executable),
            None,
            &["ChatGPT.exe", "Codex.exe"],
        ));
        let store_root = PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_123");
        assert!(windows_process_matches_stop_target(
            &store_root.join("ChatGPT.exe"),
            None,
            Some(&store_root),
            &["ChatGPT.exe", "Codex.exe"],
        ));
        assert!(!windows_process_matches_stop_target(
            Path::new(r"C:\Program Files\WindowsApps\Other\ChatGPT.exe"),
            None,
            Some(&store_root),
            &["ChatGPT.exe", "Codex.exe"],
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn opencode_restart_matching_is_scoped_to_the_detected_installation() {
        let application = PathBuf::from(
            r"C:\Users\tester\AppData\Local\Programs\@opencode-aidesktop\OpenCode.exe",
        );
        assert!(windows_process_matches_stop_target(
            &application,
            Some(&application),
            None,
            &["OpenCode.exe"],
        ));
        assert!(!windows_process_matches_stop_target(
            Path::new(r"C:\Users\tester\AppData\Local\Programs\other\OpenCode.exe"),
            Some(&application),
            None,
            &["OpenCode.exe"],
        ));
    }
}
