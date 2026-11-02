//! Tauri app shell: window, tray icon, single-instance handling, server auto-start,
//! update check on launch, keep-awake, connection feedback.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::logging::SessionKind;
use crate::paths::Paths;
use crate::process::JobObject;
use crate::reporter::TaskSnapshot;
use crate::server::ServerManager;
use crate::state::{self, LauncherState};
use crate::{commands, crash, logging, selfupdate, sysinfo};

pub struct AppState {
    paths: RwLock<Paths>,
    pub server: Arc<ServerManager>,
    pub job: Option<Arc<JobObject>>,
    /// An install, update or self-update is in progress.
    pub busy: AtomicBool,
    /// Steps of the current/last task, for windows reopened mid-task.
    pub task: Mutex<TaskSnapshot>,
}

impl AppState {
    pub fn paths(&self) -> Paths {
        self.paths.read().unwrap().clone()
    }

    /// Called after setup changes the SillyTavern folder (adopt / un-adopt).
    pub fn set_paths(&self, paths: Paths) {
        *self.paths.write().unwrap() = paths;
    }
}

pub fn run() {
    let minimized = std::env::args().any(|a| a == "--minimized");
    let base = Paths::detect().expect("could not determine the install folder");
    let _ = base.ensure_dirs();
    // Lines are buffered until setup opens the file: a second launch that only hands off
    // to the running launcher exits before that and leaves no log behind.
    logging::init(&base.logs(), SessionKind::Gui, cfg!(debug_assertions));
    crash::install_panic_hook(&base.logs());
    let state = LauncherState::load(&base);
    let paths = state.paths(&base);
    log::info!(
        "SillyTavern Launcher {} starting (home: {}, sillytavern: {}, mode: {:?}{})",
        selfupdate::CURRENT_VERSION,
        paths.home.display(),
        paths.st_dir().display(),
        state.mode,
        if minimized { ", minimized" } else { "" }
    );
    selfupdate::cleanup_old(&paths);
    let job = match JobObject::new() {
        Ok(j) => Some(Arc::new(j)),
        Err(e) => {
            log::warn!("job object unavailable: {e:#}");
            None
        }
    };
    let server = Arc::new(ServerManager::new(job.clone(), state.settings.port));
    let app_state = AppState {
        paths: RwLock::new(paths),
        server,
        job,
        busy: AtomicBool::new(false),
        task: Mutex::new(TaskSnapshot::default()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("second launcher instance started; showing the existing window");
            show_main_window(app);
            let st = app.state::<AppState>();
            if st.server.is_up() {
                commands::open_browser_now(app);
            } else if state::is_installed(&st.paths()) {
                commands::start_server_bg(app.clone(), true);
            }
        }))
        .manage(app_state)
        .setup(move |app| {
            let st = app.state::<AppState>();
            logging::open_file();
            crash::begin_session(&st.paths().logs());
            log_environment(&st.paths());
            logging::attach_app(app.handle().clone());
            build_tray(app.handle())?;
            watch_session_end(app.handle());

            let handle = app.handle().clone();
            st.server.set_notifier(Box::new(move |status| {
                let _ = handle.emit("server-event", &status);
                commands::apply_keep_awake(&handle);
            }));
            let handle = app.handle().clone();
            st.server.set_connection_notifier(Box::new(move |event| {
                let _ = handle.emit("connection-event", &event);
            }));

            if !minimized {
                show_main_window(app.handle());
            }
            let paths = st.paths();
            if state::is_installed(&paths) {
                commands::start_server_bg(app.handle().clone(), !minimized);
                if LauncherState::load(&paths).settings.check_updates_on_start {
                    commands::updates_on_launch(app.handle().clone());
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the window keeps SillyTavern running; Quit lives in the tray menu.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::discover_installs,
            commands::pick_install_folder,
            commands::start_install,
            commands::repair_install,
            commands::start_server,
            commands::stop_server,
            commands::take_over_server,
            commands::open_browser,
            commands::set_lan_enabled,
            commands::allow_ip,
            commands::diagnose_phone,
            commands::fix_firewall,
            commands::fix_whitelist,
            commands::set_open_browser,
            commands::set_check_updates_on_start,
            commands::set_keep_awake,
            commands::set_start_with_windows,
            commands::check_updates,
            commands::start_update,
            commands::start_self_update,
            commands::open_install_folder,
            commands::open_config_file,
            commands::get_logs,
            commands::create_shortcuts,
            commands::add_firewall_rule,
            commands::quit_app,
            commands::show_window,
            commands::ui_log,
            commands::save_support_bundle,
            commands::open_logs_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting SillyTavern Launcher")
        .run(|_app, event| {
            // Every orderly exit (tray Quit, self-update restart) passes through here.
            if let tauri::RunEvent::Exit = event {
                crash::end_session("launcher exited");
            }
        });
}

/// The facts every bug report needs, at the top of each session log.
fn log_environment(paths: &Paths) {
    let state = LauncherState::load(paths);
    log::info!(
        "{}, WebView2 {}, exe {}",
        sysinfo::windows_version(),
        sysinfo::webview_version(),
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".to_string())
    );
    log::info!(
        "installed={} sillytavern={} node={} settings: lan={} port={} open_browser={} check_updates={} keep_awake={}",
        state::is_installed(paths),
        state.st_version.as_deref().unwrap_or("-"),
        state.node_version.as_deref().unwrap_or("-"),
        state.settings.lan_enabled,
        state.settings.port,
        state.settings.open_browser_on_start,
        state.settings.check_updates_on_start,
        state.settings.keep_awake
    );
}

/// Windows ends the process right after `WM_ENDSESSION` (sign-out, shutdown, restart, and
/// Fast Startup's hibernate-style shutdown) without our quit path running. Hooking the main
/// window (hidden windows get the broadcast too) closes the session marker so the next start
/// does not report a crash that never happened.
fn watch_session_end(app: &AppHandle) {
    use windows::Win32::UI::Shell::SetWindowSubclass;
    let Some(window) = app.get_webview_window("main") else { return };
    match window.hwnd() {
        Ok(hwnd) => {
            if !unsafe { SetWindowSubclass(hwnd, Some(session_end_proc), 1, 0) }.as_bool() {
                log::warn!("could not watch for Windows shutdown; a shutdown may be reported as a crash");
            }
        }
        Err(e) => log::warn!("no window handle to watch for Windows shutdown: {e}"),
    }
}

unsafe extern "system" fn session_end_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{ENDSESSION_LOGOFF, WM_ENDSESSION};
    if msg == WM_ENDSESSION && wparam.0 != 0 {
        let logoff = (lparam.0 as u32) & ENDSESSION_LOGOFF != 0;
        crash::end_session(if logoff {
            "signing out of Windows"
        } else {
            "Windows is shutting down or restarting"
        });
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open", "Open SillyTavern in browser", true, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", "Show launcher", true, None::<&str>)?;
    // Also here, not only in the window: it has to work when the window itself is the problem.
    let logs_item = MenuItem::with_id(app, "logs", "Save logs to Desktop (for troubleshooting)", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit (stops SillyTavern)", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &show_item,
            &PredefinedMenuItem::separator(app)?,
            &logs_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("SillyTavern Launcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().0.as_str() {
            "open" => commands::open_browser_now(app),
            "show" => show_main_window(app),
            "logs" => commands::save_bundle_bg(app.clone()),
            "quit" => commands::quit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}
