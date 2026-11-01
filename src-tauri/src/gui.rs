//! Tauri app shell: window, tray icon, single-instance handling, server auto-start.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::paths::Paths;
use crate::process::JobObject;
use crate::server::ServerManager;
use crate::state::{self, LauncherState};
use crate::{commands, logging};

pub struct AppState {
    pub paths: Paths,
    pub server: Arc<ServerManager>,
    pub job: Option<Arc<JobObject>>,
    /// An install or update is in progress.
    pub busy: AtomicBool,
}

pub fn run() {
    let paths = Paths::detect().expect("could not determine the install folder");
    let _ = paths.ensure_dirs();
    logging::init(&paths.launcher_log(), cfg!(debug_assertions));
    log::info!(
        "SillyTavern Launcher {} starting (home: {})",
        env!("CARGO_PKG_VERSION"),
        paths.home.display()
    );
    let job = match JobObject::new() {
        Ok(j) => Some(Arc::new(j)),
        Err(e) => {
            log::warn!("job object unavailable: {e:#}");
            None
        }
    };
    let settings = LauncherState::load(&paths).settings;
    let server = Arc::new(ServerManager::new(job.clone(), settings.port));
    let app_state = AppState {
        paths,
        server,
        job,
        busy: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("second launcher instance started; showing the existing window");
            show_main_window(app);
            let st = app.state::<AppState>();
            if st.server.is_up() {
                commands::open_browser_now(app);
            } else if state::is_installed(&st.paths) {
                commands::start_server_bg(app.clone(), true);
            }
        }))
        .manage(app_state)
        .setup(|app| {
            logging::attach_app(app.handle().clone());
            build_tray(app.handle())?;
            let handle = app.handle().clone();
            let st = app.state::<AppState>();
            st.server.set_notifier(Box::new(move |status| {
                let _ = handle.emit("server-event", &status);
            }));
            if state::is_installed(&st.paths) {
                commands::start_server_bg(app.handle().clone(), true);
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
            commands::start_install,
            commands::start_server,
            commands::stop_server,
            commands::open_browser,
            commands::set_lan_enabled,
            commands::set_open_browser,
            commands::check_update,
            commands::start_update,
            commands::open_install_folder,
            commands::get_logs,
            commands::create_shortcuts,
            commands::add_firewall_rule,
            commands::quit_app,
            commands::show_window,
            commands::ui_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SillyTavern Launcher");
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
    let quit_item = MenuItem::with_id(app, "quit", "Quit (stops SillyTavern)", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &show_item,
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
