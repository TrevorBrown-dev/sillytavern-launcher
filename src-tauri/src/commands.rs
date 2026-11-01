//! Tauri commands the UI invokes, plus the shared "start the server in the background" helper.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::gui::{show_main_window, AppState};
use crate::installer::{self, InstallOptions, UpdateCheck};
use crate::reporter::{Reporter, TauriReporter};
use crate::server::{ServerState, ServerStatus, READY_TIMEOUT};
use crate::state::{self, LauncherState, Settings};
use crate::{config, firewall, logging, net, shortcut, sillytavern};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PhoneInfo {
    pub enabled: bool,
    pub url: Option<String>,
    pub qr_svg: Option<String>,
    pub alternatives: Vec<String>,
    pub hostname_url: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub installed: bool,
    pub busy: bool,
    pub st_version: Option<String>,
    pub node_version: Option<String>,
    pub settings: Settings,
    pub server: ServerStatus,
    pub phone: PhoneInfo,
    pub home: String,
    pub launcher_version: String,
    pub firewall_rule: bool,
}

fn phone_info(settings: &Settings) -> PhoneInfo {
    let ips = net::lan_ips();
    let url = ips
        .first()
        .map(|ip| format!("http://{ip}:{}", settings.port));
    PhoneInfo {
        enabled: settings.lan_enabled,
        qr_svg: url.as_deref().and_then(net::qr_svg),
        alternatives: ips
            .iter()
            .skip(1)
            .map(|ip| format!("http://{ip}:{}", settings.port))
            .collect(),
        hostname_url: net::hostname().map(|h| format!("http://{h}.local:{}", settings.port)),
        url,
    }
}

pub fn build_status(st: &AppState) -> StatusInfo {
    let state = LauncherState::load(&st.paths);
    StatusInfo {
        installed: state::is_installed(&st.paths),
        busy: st.busy.load(Ordering::SeqCst),
        st_version: sillytavern::installed_version(&st.paths).or(state.st_version),
        node_version: state.node_version,
        phone: phone_info(&state.settings),
        server: st.server.status(),
        home: st.paths.home.display().to_string(),
        launcher_version: env!("CARGO_PKG_VERSION").to_string(),
        firewall_rule: state.firewall_rule_added,
        settings: state.settings,
    }
}

/// Starts SillyTavern on a worker thread and, once it answers, opens the browser
/// (when the setting allows and `open_browser` is set).
pub fn start_server_bg(app: AppHandle, open_browser: bool) {
    std::thread::spawn(move || {
        let st = app.state::<AppState>();
        let settings = LauncherState::load(&st.paths).settings;
        let server = Arc::clone(&st.server);
        if let Err(e) = server.start(&st.paths, &settings) {
            log::error!("could not start SillyTavern: {e:#}");
            server.set_error(format!("{e:#}"));
            return;
        }
        if !(open_browser && settings.open_browser_on_start) {
            return;
        }
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let status = server.status();
            match status.state {
                ServerState::Running => {
                    if let Err(e) = open::that(&status.url) {
                        log::warn!("could not open the browser: {e}");
                    }
                    return;
                }
                ServerState::Starting if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(400));
                }
                _ => return,
            }
        }
    });
}

pub fn open_browser_now(app: &AppHandle) {
    let url = app.state::<AppState>().server.status().url;
    if let Err(e) = open::that(&url) {
        log::warn!("could not open the browser: {e}");
    }
}

pub fn quit(app: AppHandle) {
    std::thread::spawn(move || {
        let _ = app.state::<AppState>().server.stop();
        log::info!("launcher exiting");
        app.exit(0);
    });
}

#[tauri::command]
pub async fn get_status(app: AppHandle) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || build_status(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_install(app: AppHandle, options: InstallOptions) -> Result<(), String> {
    if app.state::<AppState>().busy.swap(true, Ordering::SeqCst) {
        return Err("Another task is already running.".to_string());
    }
    std::thread::spawn(move || {
        let reporter = TauriReporter {
            app: app.clone(),
            task: "install".to_string(),
        };
        let result = {
            let st = app.state::<AppState>();
            installer::run_install(&st.paths, &options, st.job.as_deref(), &reporter)
        };
        app.state::<AppState>().busy.store(false, Ordering::SeqCst);
        match result {
            Ok(_) => {
                reporter.finished(true, None);
                start_server_bg(app.clone(), true);
            }
            Err(e) => {
                log::error!("install failed: {e:#}");
                reporter.finished(false, Some(format!("{e:#}")));
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn start_server(app: AppHandle) -> Result<(), String> {
    if !state::is_installed(&app.state::<AppState>().paths) {
        return Err("SillyTavern is not installed yet.".to_string());
    }
    start_server_bg(app, true);
    Ok(())
}

#[tauri::command]
pub async fn stop_server(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .server
            .stop()
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn open_browser(app: AppHandle) -> Result<(), String> {
    open_browser_now(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_lan_enabled(app: AppHandle, enabled: bool) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<StatusInfo> {
        let st = app.state::<AppState>();
        let mut state = LauncherState::load(&st.paths);
        state.settings.lan_enabled = enabled;
        state.save(&st.paths)?;
        if st.paths.st_dir().join("server.js").is_file() {
            config::apply(&st.paths.st_dir(), enabled)?;
        }
        if enabled && !state.firewall_rule_added && !firewall::rule_exists() {
            match firewall::add_rule_elevated(&st.paths.node_exe()) {
                Ok(firewall::FirewallOutcome::Added) => {
                    state.firewall_rule_added = true;
                    state.save(&st.paths)?;
                }
                Ok(firewall::FirewallOutcome::Declined) => log::info!("firewall prompt declined"),
                Err(e) => log::warn!("firewall rule failed: {e:#}"),
            }
        }
        let status = st.server.status();
        if matches!(status.state, ServerState::Running | ServerState::Starting) && !status.external {
            st.server.stop()?;
            st.server.start(&st.paths, &state.settings)?;
        }
        Ok(build_status(&st))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn set_open_browser(app: AppHandle, enabled: bool) -> Result<(), String> {
    let st = app.state::<AppState>();
    let mut state = LauncherState::load(&st.paths);
    state.settings.open_browser_on_start = enabled;
    state.save(&st.paths).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateCheck, String> {
    tauri::async_runtime::spawn_blocking(move || {
        installer::check_update(&app.state::<AppState>().paths).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_update(app: AppHandle) -> Result<(), String> {
    if app.state::<AppState>().busy.swap(true, Ordering::SeqCst) {
        return Err("Another task is already running.".to_string());
    }
    std::thread::spawn(move || {
        let reporter = TauriReporter {
            app: app.clone(),
            task: "update".to_string(),
        };
        let result = {
            let st = app.state::<AppState>();
            let server = Arc::clone(&st.server);
            let stop = move || {
                let _ = server.stop();
            };
            installer::run_update(&st.paths, st.job.as_deref(), &stop, &reporter)
        };
        app.state::<AppState>().busy.store(false, Ordering::SeqCst);
        match result {
            Ok(_) => {
                reporter.finished(true, None);
                start_server_bg(app.clone(), true);
            }
            Err(e) => {
                log::error!("update failed: {e:#}");
                reporter.finished(false, Some(format!("{e:#}")));
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_install_folder(app: AppHandle) -> Result<(), String> {
    let home = app.state::<AppState>().paths.home.clone();
    open::that(&home).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_logs(limit: Option<usize>) -> Result<Vec<String>, String> {
    Ok(logging::recent(limit.unwrap_or(400)))
}

#[tauri::command]
pub async fn create_shortcuts(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let exe = if st.paths.exe_copy().is_file() {
            st.paths.exe_copy()
        } else {
            std::env::current_exe().map_err(|e| e.to_string())?
        };
        shortcut::create_all(&exe)
            .map(|v| format!("Created {} shortcut(s).", v.len()))
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_firewall_rule(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        if firewall::rule_exists() {
            return Ok("The firewall rule is already in place.".to_string());
        }
        match firewall::add_rule_elevated(&st.paths.node_exe()) {
            Ok(firewall::FirewallOutcome::Added) => {
                let mut s = LauncherState::load(&st.paths);
                s.firewall_rule_added = true;
                let _ = s.save(&st.paths);
                Ok("Firewall rule added.".to_string())
            }
            Ok(firewall::FirewallOutcome::Declined) => {
                Err("The Windows permission prompt was declined.".to_string())
            }
            Err(e) => Err(format!("{e:#}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn quit_app(app: AppHandle) -> Result<(), String> {
    quit(app);
    Ok(())
}

#[tauri::command]
pub async fn show_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// Lets the webview leave breadcrumbs in launcher.log (what it rendered, which events it saw).
#[tauri::command]
pub async fn ui_log(line: String) -> Result<(), String> {
    log::info!("UI: {line}");
    Ok(())
}
