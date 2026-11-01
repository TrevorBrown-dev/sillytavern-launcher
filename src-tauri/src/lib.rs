//! SillyTavern Launcher: a single exe that installs SillyTavern (portable Node.js,
//! release zip, npm packages) or adopts an install the user already has, configures
//! LAN access for phones, creates shortcuts, keeps itself and SillyTavern updated,
//! and then acts as the tray launcher for everyday use.

mod archive;
mod awake;
mod cli;
mod commands;
mod config;
mod diagnose;
mod discover;
mod download;
mod firewall;
mod git;
mod gui;
mod installer;
mod logging;
mod net;
mod node;
mod npm;
mod paths;
mod process;
mod reporter;
mod selfupdate;
mod server;
mod shortcut;
mod sillytavern;
mod state;

pub fn run() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // A freshly self-updated launcher is started by the old one with its pid; wait for it
    // to be gone so the single-instance guard does not hand control back to the old copy.
    if let Some(i) = args.iter().position(|a| a == "--wait-for-pid") {
        if let Some(pid) = args.get(i + 1).and_then(|v| v.parse::<u32>().ok()) {
            process::wait_for_pid(pid, std::time::Duration::from_secs(30));
        }
        args.drain(i..(i + 2).min(args.len()));
    }

    if args.first().is_some_and(|a| a == "--cli") {
        std::process::exit(cli::run(&args[1..]));
    }
    gui::run();
}
