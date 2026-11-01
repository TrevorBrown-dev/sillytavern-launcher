//! `stlauncher.exe --cli <install|update|start|status>`: the same flows without the
//! window, for testing and for people who like terminals after all.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::installer::{self, InstallOptions};
use crate::paths::Paths;
use crate::process::JobObject;
use crate::reporter::CliReporter;
use crate::server::{ServerManager, ServerState};
use crate::state::{self, LauncherState};
use crate::{logging, net};

pub fn run(args: &[String]) -> i32 {
    attach_parent_console();
    let paths = match Paths::detect() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            return 1;
        }
    };
    let _ = paths.ensure_dirs();
    logging::init(&paths.launcher_log(), true);
    let job = JobObject::new().ok().map(Arc::new);
    let has = |flag: &str| args.iter().any(|a| a == flag);

    let result: Result<()> = match args.first().map(String::as_str).unwrap_or("help") {
        "install" => {
            let opts = InstallOptions {
                lan_enabled: !has("--no-lan"),
                create_shortcut: !has("--no-shortcut"),
                add_firewall_rule: !has("--no-firewall"),
            };
            let reporter = CliReporter::new("install");
            installer::run_install(&paths, &opts, job.as_deref(), &reporter).map(|s| {
                println!(
                    "Installed SillyTavern {} with Node.js {}",
                    s.st_version.unwrap_or_default(),
                    s.node_version.unwrap_or_default()
                );
            })
        }
        "update" => {
            let reporter = CliReporter::new("update");
            installer::run_update(&paths, job.as_deref(), &|| {}, &reporter)
                .map(|s| println!("Updated to SillyTavern {}", s.st_version.unwrap_or_default()))
        }
        "check-update" => installer::check_update(&paths).map(|u| {
            println!(
                "installed: {}  latest: {}  update available: {}",
                u.installed.unwrap_or_else(|| "(none)".into()),
                u.latest,
                u.available
            );
        }),
        "start" => start_foreground(&paths, job),
        "status" => {
            let st = LauncherState::load(&paths);
            println!("home:        {}", paths.home.display());
            println!("installed:   {}", state::is_installed(&paths));
            println!("sillytavern: {}", st.st_version.unwrap_or_else(|| "(none)".into()));
            println!("node:        {}", st.node_version.unwrap_or_else(|| "(none)".into()));
            println!("lan enabled: {}  port: {}", st.settings.lan_enabled, st.settings.port);
            println!("lan ips:     {:?}", net::lan_ips());
            println!("port open:   {}", net::port_open(st.settings.port));
            Ok(())
        }
        _ => {
            println!("SillyTavern Launcher {}", env!("CARGO_PKG_VERSION"));
            println!("usage: --cli install [--no-lan] [--no-shortcut] [--no-firewall]");
            println!("       --cli update | check-update | start | status");
            println!("env:   STLAUNCHER_HOME overrides the install folder");
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
