//! SillyTavern Launcher: a single exe that installs SillyTavern (portable Node.js,
//! release zip, npm packages), configures LAN access for phones, creates shortcuts,
//! and then acts as the tray launcher for everyday use.

mod archive;
mod cli;
mod commands;
mod config;
mod download;
mod firewall;
mod gui;
mod installer;
mod logging;
mod net;
mod node;
mod npm;
mod paths;
mod process;
mod reporter;
mod server;
mod shortcut;
mod sillytavern;
mod state;

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--cli") {
        std::process::exit(cli::run(&args[1..]));
    }
    gui::run();
}
