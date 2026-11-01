//! Tauri commands the UI invokes, plus the background helpers (server start, update checks,
//! self-update restart) they share with the tray and startup code.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::diagnose::Diagnosis;
use crate::discover::ExistingInstall;
use crate::gui::{show_main_window, AppState};
use crate::installer::{self, InstallOptions, UpdateCheck};
use crate::reporter::{Reporter, StepStatus, TaskSnapshot, TauriReporter};
use crate::selfupdate::{self, LauncherUpdate};
use crate::server::{Connections, ServerState, ServerStatus, READY_TIMEOUT};
use crate::state::{self, InstallMode, LauncherState, Settings};
use crate::{awake, config, diagnose, discover, download, firewall, git, logging, net, shortcut, sillytavern};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AltAddress {
    pub url: String,
    pub adapter: String,
    pub virtual_adapter: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PhoneInfo {
    pub enabled: bool,
    pub url: Option<String>,
    pub adapter: Option<String>,
    pub qr_svg: Option<String>,
    pub alternatives: Vec<AltAddress>,
    pub hostname_url: Option<String>,
    /// A VPN/virtual adapter owns the default route; the shown address is the Wi-Fi one.
    pub vpn_active: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub installed: bool,
    pub busy: bool,
    pub mode: InstallMode,
    pub st_dir: String,
    pub is_git: bool,
    pub branch: Option<String>,
    pub st_version: Option<String>,
    pub node_version: Option<String>,
    pub settings: Settings,
    pub server: ServerStatus,
    pub phone: PhoneInfo,
    pub connections: Connections,
    pub task: TaskSnapshot,
    pub start_with_windows: bool,
    pub home: String,
    pub launcher_version: String,
    pub firewall_rule: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReport {
    pub sillytavern: Option<UpdateCheck>,
    pub launcher: Option<LauncherUpdate>,
    pub errors: Vec<String>,
}

fn phone_info(settings: &Settings) -> PhoneInfo {
    let addresses = net::addresses();
    let primary = addresses.first();
    let url = primary.map(|a| format!("http://{}:{}", a.ip, settings.port));
    PhoneInfo {
        enabled: settings.lan_enabled,
        qr_svg: url.as_deref().and_then(net::qr_svg),
        adapter: primary.map(|a| a.adapter.clone()),
        alternatives: addresses
            .iter()
            .skip(1)
            .map(|a| AltAddress {
                url: format!("http://{}:{}", a.ip, settings.port),
                adapter: a.adapter.clone(),
                virtual_adapter: a.virtual_adapter,
            })
            .collect(),
        hostname_url: net::hostname().map(|h| format!("http://{h}.local:{}", settings.port)),
        vpn_active: addresses.iter().any(|a| a.virtual_adapter && a.default_route),
        url,
    }
}

pub fn build_status(st: &AppState) -> StatusInfo {
    let paths = st.paths();
    let state = LauncherState::load(&paths);
    let st_dir = paths.st_dir();
    let is_git = git::is_repo(&st_dir);
    StatusInfo {
        installed: state::is_installed(&paths),
        busy: st.busy.load(Ordering::SeqCst),
        mode: state.mode,
        st_dir: discover::display_path(&st_dir),
        is_git,
        branch: if is_git { git::branch(&st_dir) } else { None },
        st_version: sillytavern::version_in(&st_dir).or(state.st_version),
        node_version: state.node_version,
        phone: phone_info(&state.settings),
        connections: st.server.connections(),
        task: st.task.lock().unwrap().clone(),
        start_with_windows: shortcut::start_with_windows_enabled(),
        server: st.server.status(),
        home: paths.home.display().to_string(),
        launcher_version: selfupdate::CURRENT_VERSION.to_string(),
        firewall_rule: state.firewall_rule_added || firewall::rule_exists(),
        settings: state.settings,
    }
}

/// Starts SillyTavern on a worker thread and, once it answers, opens the browser
/// (when the setting allows and `open_browser` is set).
pub fn start_server_bg(app: AppHandle, open_browser: bool) {
    std::thread::spawn(move || {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let settings = LauncherState::load(&paths).settings;
        let server = Arc::clone(&st.server);
        if let Err(e) = server.start(&paths, &settings) {
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

/// Both update checks, run together.
pub fn check_all_updates(paths: &crate::paths::Paths) -> UpdateReport {
    let mut report = UpdateReport {
        sillytavern: None,
        launcher: None,
        errors: Vec::new(),
    };
    match installer::check_update(paths) {
        Ok(u) => report.sillytavern = Some(u),
        Err(e) => report.errors.push(format!("SillyTavern: {e:#}")),
    }
    match download::client().and_then(|c| selfupdate::check(&c)) {
        Ok(u) => report.launcher = Some(u),
        Err(e) => report.errors.push(format!("Launcher: {e:#}")),
    }
    report
}

/// A few seconds after start, look for updates and tell the UI (`updates-available`).
pub fn updates_on_launch(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        let paths = app.state::<AppState>().paths();
        let report = check_all_updates(&paths);
        log::info!(
            "update check: sillytavern={:?} launcher={:?} errors={:?}",
            report.sillytavern.as_ref().map(|u| u.available),
            report.launcher.as_ref().map(|u| u.available),
            report.errors
        );
        let _ = app.emit("updates-available", &report);
    });
}

/// Keep-awake follows the server state and the setting.
pub fn apply_keep_awake(app: &AppHandle) {
    let st = app.state::<AppState>();
    let settings = LauncherState::load(&st.paths()).settings;
    let running = st.server.status().state == ServerState::Running;
    awake::set(running && settings.keep_awake);
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
        awake::set(false);
        log::info!("launcher exiting");
        app.exit(0);
    });
}

fn claim_busy(app: &AppHandle) -> Result<(), String> {
    if app.state::<AppState>().busy.swap(true, Ordering::SeqCst) {
        return Err("Another task is already running.".to_string());
    }
    Ok(())
}

fn release_busy(app: &AppHandle) {
    app.state::<AppState>().busy.store(false, Ordering::SeqCst);
}

fn run_install_task(app: AppHandle, options: InstallOptions) {
    std::thread::spawn(move || {
        let reporter = TauriReporter {
            app: app.clone(),
            task: "install".to_string(),
        };
        let result = {
            let st = app.state::<AppState>();
            let base = st.paths().with_st_dir(None);
            if st.server.is_up() && !st.server.status().external {
                let _ = st.server.stop();
                log::info!("stopped the running server before re-running setup");
            }
            installer::run_install(&base, &options, st.job.as_deref(), &reporter).map(|state| {
                st.set_paths(state.paths(&base));
                state
            })
        };
        release_busy(&app);
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
}

#[tauri::command]
pub async fn get_status(app: AppHandle) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || build_status(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn discover_installs(app: AppHandle) -> Result<Vec<ExistingInstall>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let managed = app.state::<AppState>().paths().with_st_dir(None).st_dir();
        discover::discover(Some(&managed))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Native folder picker; validates the choice before handing it back.
#[tauri::command]
pub async fn pick_install_folder() -> Result<Option<ExistingInstall>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Choose your SillyTavern folder (the one that contains server.js)")
            .pick_folder();
        match picked {
            None => Ok(None),
            Some(dir) => discover::inspect(&dir).map(Some).ok_or_else(|| {
                format!(
                    "{} does not look like a SillyTavern folder. Pick the folder that contains server.js and Start.bat.",
                    dir.display()
                )
            }),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_install(app: AppHandle, options: InstallOptions) -> Result<(), String> {
    claim_busy(&app)?;
    run_install_task(app, options);
    Ok(())
}

/// Re-runs setup with the current settings (idempotent: fixes a half-finished install
/// without downloading what is already there).
#[tauri::command]
pub async fn repair_install(app: AppHandle) -> Result<(), String> {
    claim_busy(&app)?;
    let state = LauncherState::load(&app.state::<AppState>().paths());
    let options = InstallOptions {
        lan_enabled: state.settings.lan_enabled,
        create_shortcut: true,
        add_firewall_rule: state.settings.lan_enabled,
        adopt_path: if state.mode == InstallMode::Adopted {
            state.st_dir.map(|p| p.display().to_string())
        } else {
            None
        },
    };
    run_install_task(app, options);
    Ok(())
}

#[tauri::command]
pub async fn start_server(app: AppHandle) -> Result<(), String> {
    if !state::is_installed(&app.state::<AppState>().paths()) {
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

/// Closes a SillyTavern that runs outside the launcher and starts ours.
#[tauri::command]
pub async fn take_over_server(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let settings = LauncherState::load(&paths).settings;
        st.server
            .take_over(&paths, &settings)
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

fn restart_if_running(st: &AppState, settings: &Settings) -> anyhow::Result<()> {
    let status = st.server.status();
    if matches!(status.state, ServerState::Running | ServerState::Starting) && !status.external {
        st.server.stop()?;
        st.server.start(&st.paths(), settings)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_lan_enabled(app: AppHandle, enabled: bool) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<StatusInfo> {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let mut state = LauncherState::load(&paths);
        state.settings.lan_enabled = enabled;
        state.save(&paths)?;
        let st_dir = paths.st_dir();
        if st_dir.join("server.js").is_file() {
            if paths.is_adopted() {
                config::apply_adopted(&st_dir)?;
            } else {
                config::apply(&st_dir, enabled)?;
            }
        }
        if enabled && !state.firewall_rule_added && !firewall::rule_exists() {
            match firewall::add_rule_elevated(&paths.node_exe()) {
                Ok(firewall::FirewallOutcome::Added) => {
                    state.firewall_rule_added = true;
                    state.save(&paths)?;
                }
                Ok(firewall::FirewallOutcome::Declined) => log::info!("firewall prompt declined"),
                Err(e) => log::warn!("firewall rule failed: {e:#}"),
            }
        }
        restart_if_running(&st, &state.settings)?;
        Ok(build_status(&st))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

/// Adds a specific address (a blocked phone) to config.yaml's whitelist and restarts.
#[tauri::command]
pub async fn allow_ip(app: AppHandle, ip: String) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<StatusInfo> {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let ip = ip.trim().to_string();
        if ip.is_empty() || !ip.chars().all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':') {
            anyhow::bail!("{ip} is not an IP address");
        }
        config::add_whitelist_entries(&paths.st_dir(), &[ip.clone()])?;
        log::info!("whitelisted {ip}");
        let settings = LauncherState::load(&paths).settings;
        restart_if_running(&st, &settings)?;
        Ok(build_status(&st))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn diagnose_phone(app: AppHandle) -> Result<Diagnosis, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let settings = LauncherState::load(&paths).settings;
        let server = st.server.status();
        let d = diagnose::run(&paths, &settings, &server);
        log::info!(
            "phone diagnosis: ok={} {:?}",
            d.ok,
            d.checks.iter().map(|c| format!("{}={:?}", c.id, c.level)).collect::<Vec<_>>()
        );
        d
    })
    .await
    .map_err(|e| e.to_string())
}

/// Removes block rules for node.exe and (re)adds the allow rule; one UAC prompt.
#[tauri::command]
pub async fn fix_firewall(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let paths = st.paths();
        match firewall::repair_elevated(&paths.node_exe()) {
            Ok(firewall::FirewallOutcome::Added) => {
                let mut s = LauncherState::load(&paths);
                s.firewall_rule_added = true;
                let _ = s.save(&paths);
                Ok("Windows Firewall now allows phones to reach SillyTavern.".to_string())
            }
            Ok(firewall::FirewallOutcome::Declined) => {
                Err("The Windows permission prompt was declined; nothing was changed.".to_string())
            }
            Err(e) => Err(format!("{e:#}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Writes the LAN whitelist into config.yaml (whatever the install mode) and restarts.
#[tauri::command]
pub async fn fix_whitelist(app: AppHandle) -> Result<StatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<StatusInfo> {
        let st = app.state::<AppState>();
        let paths = st.paths();
        let settings = LauncherState::load(&paths).settings;
        if paths.is_adopted() {
            config::apply_adopted(&paths.st_dir())?;
        } else {
            config::apply(&paths.st_dir(), settings.lan_enabled)?;
        }
        restart_if_running(&st, &settings)?;
        Ok(build_status(&st))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn set_open_browser(app: AppHandle, enabled: bool) -> Result<(), String> {
    let paths = app.state::<AppState>().paths();
    let mut state = LauncherState::load(&paths);
    state.settings.open_browser_on_start = enabled;
    state.save(&paths).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn set_check_updates_on_start(app: AppHandle, enabled: bool) -> Result<(), String> {
    let paths = app.state::<AppState>().paths();
    let mut state = LauncherState::load(&paths);
    state.settings.check_updates_on_start = enabled;
    state.save(&paths).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn set_keep_awake(app: AppHandle, enabled: bool) -> Result<(), String> {
    let paths = app.state::<AppState>().paths();
    let mut state = LauncherState::load(&paths);
    state.settings.keep_awake = enabled;
    state.save(&paths).map_err(|e| format!("{e:#}"))?;
    apply_keep_awake(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_start_with_windows(app: AppHandle, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = app.state::<AppState>().paths();
        let exe = if paths.exe_copy().is_file() {
            paths.exe_copy()
        } else {
            std::env::current_exe().map_err(|e| e.to_string())?
        };
        shortcut::set_start_with_windows(enabled, &exe).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<UpdateReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        check_all_updates(&app.state::<AppState>().paths())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_update(app: AppHandle) -> Result<(), String> {
    claim_busy(&app)?;
    std::thread::spawn(move || {
        let reporter = TauriReporter {
            app: app.clone(),
            task: "update".to_string(),
        };
        let result = {
            let st = app.state::<AppState>();
            let base = st.paths().with_st_dir(None);
            let server = Arc::clone(&st.server);
            let stop = move || {
                let _ = server.stop();
            };
            installer::run_update(&base, st.job.as_deref(), &stop, &reporter)
        };
        release_busy(&app);
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

/// Downloads the newest launcher, swaps it in, relaunches it and exits this one.
#[tauri::command]
pub async fn start_self_update(app: AppHandle) -> Result<(), String> {
    claim_busy(&app)?;
    std::thread::spawn(move || {
        let reporter = TauriReporter {
            app: app.clone(),
            task: "selfupdate".to_string(),
        };
        let result = (|| -> anyhow::Result<std::path::PathBuf> {
            let paths = app.state::<AppState>().paths();
            let client = download::client()?;
            let update = selfupdate::check(&client)?;
            if !update.available {
                anyhow::bail!(
                    "You already have the latest launcher ({}).",
                    update.current
                );
            }
            selfupdate::apply(&paths, &update, &reporter)
        })();
        release_busy(&app);
        match result {
            Ok(new_exe) => {
                reporter.step("restart", StepStatus::Running, "Restarting the launcher...");
                let st = app.state::<AppState>();
                let _ = st.server.stop();
                awake::set(false);
                let spawned = std::process::Command::new(&new_exe)
                    .arg("--wait-for-pid")
                    .arg(std::process::id().to_string())
                    .current_dir(st.paths().home)
                    .spawn();
                match spawned {
                    Ok(_) => {
                        reporter.step("restart", StepStatus::Done, "New launcher starting");
                        reporter.finished(true, None);
                        std::thread::sleep(Duration::from_millis(600));
                        app.exit(0);
                    }
                    Err(e) => {
                        log::error!("could not start the new launcher: {e}");
                        reporter.finished(
                            false,
                            Some(format!(
                                "The new launcher is installed at {} but could not be started ({e}). Start it from the shortcut.",
                                new_exe.display()
                            )),
                        );
                    }
                }
            }
            Err(e) => {
                log::error!("self-update failed: {e:#}");
                reporter.finished(false, Some(format!("{e:#}")));
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_install_folder(app: AppHandle) -> Result<(), String> {
    let dir = app.state::<AppState>().paths().st_dir();
    open::that(&dir).map_err(|e| e.to_string())
}

/// Opens config.yaml in Notepad (always present, never asks "how do you want to open this").
#[tauri::command]
pub async fn open_config_file(app: AppHandle) -> Result<(), String> {
    let cfg = app.state::<AppState>().paths().st_dir().join("config.yaml");
    if !cfg.is_file() {
        return Err("config.yaml does not exist yet; start SillyTavern once first.".to_string());
    }
    std::process::Command::new("notepad.exe")
        .arg(&cfg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_logs(limit: Option<usize>) -> Result<Vec<String>, String> {
    Ok(logging::recent(limit.unwrap_or(400)))
}

#[tauri::command]
pub async fn create_shortcuts(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = app.state::<AppState>().paths();
        let exe = if paths.exe_copy().is_file() {
            paths.exe_copy()
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
    fix_firewall(app).await
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
