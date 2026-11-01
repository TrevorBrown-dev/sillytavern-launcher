//! `stlauncher.exe --cli <install|update|check-update|self-update|discover|start|status>`:
//! the same flows without the window, for testing and for people who like terminals after all.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::installer::{self, InstallOptions};
use crate::paths::Paths;
use crate::process::JobObject;
use crate::reporter::CliReporter;
use crate::server::{ServerManager, ServerState};
use crate::state::{self, LauncherState};
use crate::{commands, discover, download, logging, net, selfupdate};

pub fn run(args: &[String]) -> i32 {
    attach_parent_console();
    let base = match Paths::detect() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            return 1;
        }
    };
    let _ = base.ensure_dirs();
    logging::init(&base.launcher_log(), true);
    let paths = LauncherState::load(&base).paths(&base);
    let job = JobObject::new().ok().map(Arc::new);
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let result: Result<()> = match args.first().map(String::as_str).unwrap_or("help") {
        "install" => {
            let opts = InstallOptions {
                lan_enabled: !has("--no-lan"),
                create_shortcut: !has("--no-shortcut"),
                add_firewall_rule: !has("--no-firewall"),
                adopt_path: value_of("--adopt"),
            };
            let reporter = CliReporter::new("install");
            installer::run_install(&base, &opts, job.as_deref(), &reporter).map(|s| {
                println!(
                    "Set up SillyTavern {} ({:?}) at {} with Node.js {}",
                    s.st_version.clone().unwrap_or_default(),
                    s.mode,
                    s.paths(&base).st_dir().display(),
                    s.node_version.unwrap_or_default()
                );
            })
        }
        "update" => {
            let reporter = CliReporter::new("update");
            installer::run_update(&base, job.as_deref(), &|| {}, &reporter)
                .map(|s| println!("Updated to SillyTavern {}", s.st_version.unwrap_or_default()))
        }
        "check-update" => {
            let report = commands::check_all_updates(&paths);
            if let Some(u) = report.sillytavern {
                println!(
                    "sillytavern: installed={}  latest={}  available={}  method={}  {}",
                    u.installed.unwrap_or_else(|| "(none)".into()),
                    u.latest,
                    u.available,
                    u.method,
                    u.detail.unwrap_or_default()
                );
            }
            if let Some(u) = report.launcher {
                println!(
                    "launcher:    current={}  latest={}  available={}  sha256={}",
                    u.current,
                    u.latest,
                    u.available,
                    u.sha256.as_deref().unwrap_or("(none)")
                );
            }
            for e in report.errors {
                println!("error: {e}");
            }
            Ok(())
        }
        "self-update" => (|| -> Result<()> {
            let client = download::client()?;
            let update = selfupdate::check(&client)?;
            if !update.available && !has("--force") {
                println!("Launcher {} is already the latest.", update.current);
                return Ok(());
            }
            let reporter = CliReporter::new("selfupdate");
            let exe = selfupdate::apply(&paths, &update, &reporter)?;
            println!("Installed launcher {} at {}", update.latest, exe.display());
            Ok(())
        })(),
        "discover" => {
            let managed = base.with_st_dir(None).st_dir();
            let found = discover::discover(Some(&managed));
            if found.is_empty() {
                println!("No existing SillyTavern installs found.");
            }
            for f in found {
                println!(
                    "{}  version={}  git={}  branch={}  port={}  node_modules={}  data={}",
                    f.path,
                    f.version.unwrap_or_else(|| "?".into()),
                    f.is_git,
                    f.branch.unwrap_or_else(|| "-".into()),
                    f.port,
                    f.has_node_modules,
                    f.has_data
                );
            }
            Ok(())
        }
        "start" => start_foreground(&paths, job),
        "takeover" => {
            let settings = LauncherState::load(&base).settings;
            let server = Arc::new(ServerManager::new(job, settings.port));
            server.set_notifier(Box::new(|s| {
                println!("[server] {:?} {} {}", s.state, s.url, s.message.unwrap_or_default());
            }));
            server.take_over(&paths, &settings).and_then(|()| {
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    match server.status().state {
                        ServerState::Running => {
                            println!("Running under the launcher now (stopping again since this is the CLI).");
                            break;
                        }
                        ServerState::Error | ServerState::Stopped => break,
                        ServerState::Starting => {}
                    }
                }
                server.stop()
            })
        }
        "diagnose" => {
            let settings = LauncherState::load(&base).settings;
            let server = crate::server::ServerStatus {
                state: if net::port_open(settings.port) {
                    crate::server::ServerState::Running
                } else {
                    crate::server::ServerState::Stopped
                },
                url: crate::server::local_url(settings.port),
                message: None,
                pid: None,
                external: net::port_open(settings.port),
            };
            let d = crate::diagnose::run(&paths, &settings, &server);
            for c in &d.checks {
                println!("[{:<4}] {}{}", format!("{:?}", c.level).to_ascii_lowercase(), c.title, if c.detail.is_empty() { String::new() } else { format!(" -- {}", c.detail) });
                if let Some(fix) = &c.fix {
                    println!("        fix: {fix}");
                }
            }
            println!("{}", d.summary);
            Ok(())
        }
        "status" => {
            let st = LauncherState::load(&base);
            println!("home:        {}", paths.home.display());
            println!("sillytavern: {} ({:?})", paths.st_dir().display(), st.mode);
            println!("installed:   {}", state::is_installed(&paths));
            println!("version:     {}", st.st_version.unwrap_or_else(|| "(none)".into()));
            println!("node:        {}", st.node_version.unwrap_or_else(|| "(none)".into()));
            println!("launcher:    {}", selfupdate::CURRENT_VERSION);
            println!("lan enabled: {}  port: {}", st.settings.lan_enabled, st.settings.port);
            println!("lan ips:     {:?}", net::lan_ips());
            println!("port open:   {}", net::port_open(st.settings.port));
            Ok(())
        }
        _ => {
            println!("SillyTavern Launcher {}", selfupdate::CURRENT_VERSION);
            println!("usage: --cli install [--adopt <folder>] [--no-lan] [--no-shortcut] [--no-firewall]");
            println!("       --cli update | check-update | self-update [--force] | discover | diagnose | takeover | start | status");
            println!("env:   STLAUNCHER_HOME overrides the launcher folder");
            Ok(())
        }
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            1
        }
    }
}

fn start_foreground(paths: &Paths, job: Option<Arc<JobObject>>) -> Result<()> {
    let settings = LauncherState::load(paths).settings;
    let server = Arc::new(ServerManager::new(job, settings.port));
    server.set_notifier(Box::new(|s| {
        println!(
            "[server] {:?} {} {}",
            s.state,
            s.url,
            s.message.unwrap_or_default()
        );
    }));
    server.start(paths, &settings)?;
    println!("Press Ctrl+C to stop.");
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if matches!(server.status().state, ServerState::Stopped | ServerState::Error) {
            break;
        }
    }
    Ok(())
}

/// Release builds are GUI-subsystem apps; reattach to the caller's console so output shows.
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
