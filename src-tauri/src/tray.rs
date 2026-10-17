use super::*;
use crate::native_i18n::{native_operation_failed, native_text, NativeText};

#[cfg(target_os = "macos")]
use objc2_app_kit::NSStatusItemBehavior;
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;

pub(crate) fn main_window<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> Option<tauri::Window<R>> {
    manager.get_window("main")
}

#[cfg(target_os = "macos")]
const MACOS_TRAY_ID: &str = "macos-tray";

#[cfg(target_os = "macos")]
#[derive(Default)]
pub(crate) struct MacosTrayClickState {
    last_click: Option<Instant>,
    sequence: u64,
}

#[cfg(target_os = "macos")]
pub(crate) fn set_macos_dock_visible(app_handle: &tauri::AppHandle, visible: bool) {
    if let Err(error) = app_handle.set_dock_visibility(visible) {
        eprintln!("Failed to update Dock icon visibility: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn show_main_window_on_main_thread(app_handle: &tauri::AppHandle) {
    let Some(window) = main_window(app_handle) else {
        return;
    };
    set_macos_dock_visible(app_handle, true);
    if let Err(error) = window.show() {
        eprintln!("Failed to show the main window: {error}");
        set_macos_dock_visible(app_handle, false);
        return;
    }
    if window.is_minimized().unwrap_or(false) {
        if let Err(error) = window.unminimize() {
            eprintln!("Failed to restore the main window: {error}");
        }
    }
    if let Err(error) = window.set_focus() {
        eprintln!("Failed to focus the main window: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn show_main_window(app_handle: &tauri::AppHandle) {
    if MainThreadMarker::new().is_some() {
        show_main_window_on_main_thread(app_handle);
        return;
    }

    let app_handle = app_handle.clone();
    if let Err(error) = app_handle.clone().run_on_main_thread(move || {
        show_main_window_on_main_thread(&app_handle);
    }) {
        eprintln!("Failed to schedule showing the main window: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn show_macos_tray_menu<R: tauri::Runtime>(tray: &TrayIcon<R>) {
    let result = tray.with_inner_tray_icon(|tray_icon| {
        let Some(status_item) = tray_icon.ns_status_item() else {
            return;
        };
        let mtm = MainThreadMarker::new().expect("tray menu must be shown on the main thread");
        if let Some(menu) = status_item.menu(mtm) {
            #[allow(deprecated)]
            status_item.popUpStatusItemMenu(&menu);
        }
    });

    if let Err(error) = result {
        eprintln!("Failed to show the tray menu: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn setup_macos_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<()> {
    let locale = app
        .state::<GuiConfigState>()
        .snapshot()
        .map(|config| config.locale)
        .unwrap_or_else(|_| "en".to_string());
    let open_main_window = MenuItem::with_id(
        app,
        "open-main-window",
        native_text(&locale, NativeText::OpenMainWindow),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        native_text(&locale, NativeText::Quit),
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&open_main_window, &quit])?;
    let click_state = Arc::new(Mutex::new(MacosTrayClickState::default()));
    let double_click_interval = Duration::from_secs_f64(NSEvent::doubleClickInterval());

    let tray = TrayIconBuilder::with_id(MACOS_TRAY_ID)
        .icon(tauri::include_image!("icons/tray-macos.png"))
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app_handle, event| match event.id().as_ref() {
            "open-main-window" => show_main_window(app_handle),
            "quit" => app_handle.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if !matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                return;
            }

            let now = Instant::now();
            let Ok(mut state) = click_state.lock() else {
                eprintln!("Failed to read tray click state");
                return;
            };
            state.sequence += 1;
            let sequence = state.sequence;

            if state
                .last_click
                .is_some_and(|last_click| now.duration_since(last_click) <= double_click_interval)
            {
                state.last_click = None;
                drop(state);
                show_main_window(tray.app_handle());
                return;
            }

            state.last_click = Some(now);
            drop(state);

            let app_handle = tray.app_handle().clone();
            let click_state = Arc::clone(&click_state);
            let tray_id = tray.id().clone();
            thread::spawn(move || {
                thread::sleep(double_click_interval);
                let should_show_menu = match click_state.lock() {
                    Ok(mut state) if state.sequence == sequence => {
                        state.last_click = None;
                        true
                    }
                    Ok(_) => false,
                    Err(_) => {
                        eprintln!("Failed to read tray click state");
                        false
                    }
                };
                if !should_show_menu {
                    return;
                }

                if let Some(tray) = app_handle.tray_by_id(&tray_id) {
                    show_macos_tray_menu(&tray);
                }
            });
        })
        .build(app)?;

    tray.with_inner_tray_icon(|tray_icon| {
        if let Some(status_item) = tray_icon.ns_status_item() {
            status_item.setBehavior(NSStatusItemBehavior::RemovalAllowed);
            status_item.setAutosaveName(Some(&NSString::from_str(MACOS_TRAY_ID)));
        }
    })?;

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_ID: &str = "windows-tray";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_OPEN_MENU_ID: &str = "windows-tray-open-main-window";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_STATUS_MENU_ID: &str = "windows-tray-core-status";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_TOGGLE_CORE_MENU_ID: &str = "windows-tray-toggle-core";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_RESTART_CORE_MENU_ID: &str = "windows-tray-restart-core";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const WINDOWS_TRAY_QUIT_MENU_ID: &str = "windows-tray-quit";

#[cfg(target_os = "linux")]
pub(crate) fn linux_tray_available(app_handle: &tauri::AppHandle) -> bool {
    use dbus::{arg::Variant, blocking::Connection};

    if app_handle.tray_by_id(WINDOWS_TRAY_ID).is_none() {
        return false;
    }

    let Ok(connection) = Connection::new_session() else {
        return false;
    };
    let proxy = connection.with_proxy(
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        Duration::from_millis(500),
    );
    let host_registered: Result<(Variant<bool>,), _> = proxy.method_call(
        "org.freedesktop.DBus.Properties",
        "Get",
        (
            "org.kde.StatusNotifierWatcher",
            "IsStatusNotifierHostRegistered",
        ),
    );
    matches!(host_registered, Ok((Variant(true),)))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Clone, Copy)]
pub(crate) enum WindowsTrayCoreAction {
    Toggle,
    Restart,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) struct WindowsTrayPresentation {
    pub(crate) status_text: String,
    pub(crate) toggle_text: String,
    pub(crate) toggle_enabled: bool,
    pub(crate) restart_enabled: bool,
    pub(crate) tooltip: String,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn windows_tray_presentation(
    status: &CoreStatus,
    busy: bool,
    locale: &str,
) -> WindowsTrayPresentation {
    let status_text = if busy {
        native_text(locale, NativeText::CoreStatusWorking)
    } else if !status.installed {
        native_text(locale, NativeText::CoreStatusNotInstalled)
    } else if status.running {
        native_text(locale, NativeText::CoreStatusRunning)
    } else {
        native_text(locale, NativeText::CoreStatusStopped)
    };
    let toggle_text = if busy {
        native_text(locale, NativeText::Working)
    } else if status.running {
        native_text(locale, NativeText::StopCore)
    } else {
        native_text(locale, NativeText::StartCore)
    };
    let tooltip = if busy {
        native_text(locale, NativeText::TooltipCoreWorking)
    } else if !status.installed {
        native_text(locale, NativeText::TooltipCoreNotInstalled)
    } else if status.running {
        native_text(locale, NativeText::TooltipCoreRunning)
    } else {
        native_text(locale, NativeText::TooltipCoreStopped)
    };

    WindowsTrayPresentation {
        status_text: status_text.to_string(),
        toggle_text: toggle_text.to_string(),
        toggle_enabled: status.installed && !busy,
        restart_enabled: status.installed && status.running && !busy,
        tooltip: tooltip.to_string(),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) struct WindowsTrayState {
    open_main_window: MenuItem<tauri::Wry>,
    status_item: MenuItem<tauri::Wry>,
    toggle_core_item: MenuItem<tauri::Wry>,
    restart_core_item: MenuItem<tauri::Wry>,
    quit_item: MenuItem<tauri::Wry>,
    locale: Mutex<String>,
    busy: AtomicBool,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl WindowsTrayState {
    fn locale(&self) -> String {
        self.locale
            .lock()
            .map(|locale| locale.clone())
            .unwrap_or_else(|_| "en".to_string())
    }

    fn set_locale(&self, locale: &str) {
        let normalized = normalize_app_locale(locale);
        if let Ok(mut current) = self.locale.lock() {
            *current = normalized.to_string();
        }
        let labels = [
            (
                &self.open_main_window,
                native_text(normalized, NativeText::OpenMainWindow),
            ),
            (
                &self.restart_core_item,
                native_text(normalized, NativeText::RestartCore),
            ),
            (&self.quit_item, native_text(normalized, NativeText::Quit)),
        ];
        for (item, label) in labels {
            if let Err(error) = item.set_text(label) {
                eprintln!("Failed to update desktop tray language: {error}");
            }
        }
    }

    fn begin_action(&self) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let locale = self.locale();
        if let Err(error) = self
            .status_item
            .set_text(native_text(&locale, NativeText::CoreStatusWorking)) {
            eprintln!("Failed to update desktop tray core status: {error}");
        }
        if let Err(error) =
            self.toggle_core_item
                .set_text(native_text(&locale, NativeText::Working))
        {
            eprintln!("Failed to update desktop tray action text: {error}");
        }
        if let Err(error) = self.toggle_core_item.set_enabled(false) {
            eprintln!("Failed to update desktop tray action state: {error}");
        }
        if let Err(error) = self.restart_core_item.set_enabled(false) {
            eprintln!("Failed to update desktop tray restart state: {error}");
        }
        true
    }

    fn finish_action(&self) {
        self.busy.store(false, Ordering::Release);
    }

    fn update(&self, status: &CoreStatus) -> String {
        let presentation =
            windows_tray_presentation(status, self.busy.load(Ordering::Acquire), &self.locale());
        if let Err(error) = self.status_item.set_text(presentation.status_text.clone()) {
            eprintln!("Failed to update desktop tray core status: {error}");
        }
        if let Err(error) = self
            .toggle_core_item
            .set_text(presentation.toggle_text.clone())
        {
            eprintln!("Failed to update desktop tray action text: {error}");
        }
        if let Err(error) = self
            .toggle_core_item
            .set_enabled(presentation.toggle_enabled)
        {
            eprintln!("Failed to update desktop tray action state: {error}");
        }
        if let Err(error) = self
            .restart_core_item
            .set_enabled(presentation.restart_enabled)
        {
            eprintln!("Failed to update desktop tray restart state: {error}");
        }
        presentation.tooltip
    }

    fn show_error(&self, error: &str) {
        let mut summary = error.chars().take(48).collect::<String>();
        if error.chars().count() > 48 {
            summary.push('…');
        }
        let text = native_operation_failed(&self.locale(), &summary);
        if let Err(update_error) = self.status_item.set_text(text) {
            eprintln!("Failed to update desktop tray error state: {update_error}");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn show_windows_main_window(app_handle: &tauri::AppHandle) {
    let Some(window) = main_window(app_handle) else {
        return;
    };
    if let Err(error) = window.show() {
        eprintln!("Failed to show the main window: {error}");
        return;
    }
    if window.is_minimized().unwrap_or(false) {
        if let Err(error) = window.unminimize() {
            eprintln!("Failed to restore the main window: {error}");
        }
    }
    if let Err(error) = window.set_focus() {
        eprintln!("Failed to focus the main window: {error}");
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn update_windows_tray_status(app_handle: &tauri::AppHandle, status: &CoreStatus) {
    let tooltip = app_handle
        .try_state::<WindowsTrayState>()
        .map(|tray_state| tray_state.update(status))
        .unwrap_or_else(|| windows_tray_presentation(status, false, "en").tooltip);

    if let Some(tray) = app_handle.tray_by_id(WINDOWS_TRAY_ID) {
        if let Err(error) = tray.set_tooltip(Some(&tooltip)) {
            eprintln!("Failed to update desktop tray tooltip: {error}");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn update_windows_tray_locale(
    app_handle: &tauri::AppHandle,
    locale: &str,
    status: &CoreStatus,
) {
    if let Some(tray_state) = app_handle.try_state::<WindowsTrayState>() {
        tray_state.set_locale(locale);
    }
    update_windows_tray_status(app_handle, status);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn show_windows_tray_action_error(app_handle: &tauri::AppHandle, error: &str) {
    eprintln!("Desktop tray core operation failed: {error}");
    let locale = app_handle
        .try_state::<WindowsTrayState>()
        .map(|state| state.locale())
        .unwrap_or_else(|| "en".to_string());
    if let Some(tray_state) = app_handle.try_state::<WindowsTrayState>() {
        tray_state.show_error(error);
    }
    if let Some(tray) = app_handle.tray_by_id(WINDOWS_TRAY_ID) {
        if let Err(update_error) = tray.set_tooltip(Some(native_text(
            &locale,
            NativeText::TooltipCoreOperationFailed,
        ))) {
            eprintln!("Failed to update desktop tray error tooltip: {update_error}");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn run_windows_tray_core_action(
    app_handle: &tauri::AppHandle,
    action: WindowsTrayCoreAction,
) {
    let Some(tray_state) = app_handle.try_state::<WindowsTrayState>() else {
        return;
    };
    if !tray_state.begin_action() {
        return;
    }
    if let Some(tray) = app_handle.tray_by_id(WINDOWS_TRAY_ID) {
        if let Err(error) = tray.set_tooltip(Some(native_text(
            &tray_state.locale(),
            NativeText::TooltipCoreWorking,
        ))) {
            eprintln!("Failed to update desktop tray busy tooltip: {error}");
        }
    }

    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let process_state = app_handle.state::<CoreProcessState>();
        let gui_config_state = app_handle.state::<GuiConfigState>();
        let result = (|| -> Result<CoreStatus, String> {
            let _guard = lock_core_operation(process_state.inner())?;
            match action {
                WindowsTrayCoreAction::Toggle => {
                    let config = gui_config_state.snapshot()?;
                    let status =
                        current_core_status(Some(process_state.inner()), Some(config.port))?;
                    if status.running {
                        stop_core_process_with_state(
                            process_state.inner(),
                            gui_config_state.inner(),
                        )
                    } else {
                        start_core_process_with_state(
                            process_state.inner(),
                            gui_config_state.inner(),
                        )
                    }
                }
                WindowsTrayCoreAction::Restart => {
                    restart_core_process_with_state(process_state.inner(), gui_config_state.inner())
                }
            }
        })();

        if let Some(tray_state) = app_handle.try_state::<WindowsTrayState>() {
            tray_state.finish_action();
        }

        match result {
            Ok(status) => emit_core_status(&app_handle, &status),
            Err(error) => {
                if let Ok(config) = gui_config_state.snapshot() {
                    if let Ok(status) =
                        current_core_status(Some(process_state.inner()), Some(config.port))
                    {
                        emit_core_status(&app_handle, &status);
                    }
                }
                show_windows_tray_action_error(&app_handle, &error);
            }
        }
    });
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn setup_windows_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<()> {
    let locale = app
        .state::<GuiConfigState>()
        .snapshot()
        .map(|config| config.locale)
        .unwrap_or_else(|_| "en".to_string());
    let open_main_window = MenuItem::with_id(
        app,
        WINDOWS_TRAY_OPEN_MENU_ID,
        native_text(&locale, NativeText::OpenMainWindow),
        true,
        None::<&str>,
    )?;
    let status_item = MenuItem::with_id(
        app,
        WINDOWS_TRAY_STATUS_MENU_ID,
        native_text(&locale, NativeText::CoreStatusChecking),
        false,
        None::<&str>,
    )?;
    let toggle_core_item = MenuItem::with_id(
        app,
        WINDOWS_TRAY_TOGGLE_CORE_MENU_ID,
        native_text(&locale, NativeText::StartCore),
        false,
        None::<&str>,
    )?;
    let restart_core_item = MenuItem::with_id(
        app,
        WINDOWS_TRAY_RESTART_CORE_MENU_ID,
        native_text(&locale, NativeText::RestartCore),
        false,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        WINDOWS_TRAY_QUIT_MENU_ID,
        native_text(&locale, NativeText::Quit),
        true,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_main_window,
            &separator_one,
            &status_item,
            &toggle_core_item,
            &restart_core_item,
            &separator_two,
            &quit,
        ],
    )?;

    // Linux AppIndicator implementations always open the menu when the icon is
    // activated and do not emit tray click events. Keeping "Open Main Window"
    // as the first item provides the reliable restore path on Linux; Windows
    // additionally supports the double-click shortcut below.
    TrayIconBuilder::with_id(WINDOWS_TRAY_ID)
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("application icon is required for the tray"),
        )
        .tooltip("EasyCLIProxyAPI")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app_handle, event| match event.id().as_ref() {
            WINDOWS_TRAY_OPEN_MENU_ID => show_windows_main_window(app_handle),
            WINDOWS_TRAY_TOGGLE_CORE_MENU_ID => {
                run_windows_tray_core_action(app_handle, WindowsTrayCoreAction::Toggle)
            }
            WINDOWS_TRAY_RESTART_CORE_MENU_ID => {
                run_windows_tray_core_action(app_handle, WindowsTrayCoreAction::Restart)
            }
            WINDOWS_TRAY_QUIT_MENU_ID => app_handle.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_windows_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    let _ = app.manage(WindowsTrayState {
        open_main_window,
        status_item,
        toggle_core_item,
        restart_core_item,
        quit_item: quit,
        locale: Mutex::new(locale),
        busy: AtomicBool::new(false),
    });

    Ok(())
}
