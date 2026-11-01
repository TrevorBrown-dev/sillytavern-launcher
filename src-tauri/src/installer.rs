//! The end-to-end install and update flows. Every step is idempotent so a failed run
//! can simply be retried and picks up where it left off.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::process::JobObject;
use crate::reporter::{Reporter, StepStatus};
use crate::state::LauncherState;
use crate::{config, download, firewall, node, npm, process, shortcut, sillytavern};

#[derive(Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct InstallOptions {
    pub lan_enabled: bool,
    pub create_shortcut: bool,
    pub add_firewall_rule: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            create_shortcut: true,
            add_firewall_rule: true,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub installed: Option<String>,
    pub latest: String,
    pub available: bool,
}

pub fn run_install(
    paths: &Paths,
    opts: &InstallOptions,
    job: Option<&JobObject>,
    reporter: &dyn Reporter,
) -> Result<LauncherState> {
    paths.ensure_dirs()?;
    let mut state = LauncherState::load(paths);
    state.settings.lan_enabled = opts.lan_enabled;
    let client = download::client()?;

    // 1. Node.js
    let node_version = node::install(&client, paths, reporter)?;
    state.node_version = Some(node_version);

    // 2. SillyTavern itself
    let st_dir = paths.st_dir();
    let mut fresh = false;
    if st_dir.join("server.js").is_file() {
        let v = sillytavern::installed_version(paths).unwrap_or_default();
        reporter.step(
            "sillytavern",
            StepStatus::Done,
            &format!("SillyTavern {v} is already installed"),
        );
    } else {
        reporter.step(
            "sillytavern",
            StepStatus::Running,
            "Finding the latest SillyTavern release...",
        );
        let tag = sillytavern::latest_tag(&client)?;
        if st_dir.exists() {
            move_aside(&st_dir)?;
        }
        sillytavern::download_and_extract(&client, &tag, &st_dir, paths, reporter)?;
        state.st_tag = Some(tag.clone());
        fresh = true;
        reporter.step(
            "sillytavern",
            StepStatus::Done,
            &format!("SillyTavern {tag} downloaded"),
        );
    }
    state.st_version = sillytavern::installed_version(paths);

    // 3. npm packages
    if !fresh && deps_fresh(&st_dir) {
        reporter.step("deps", StepStatus::Done, "Packages already installed");
    } else {
        npm::run_npm_ci(paths, &st_dir, job, reporter)?;
        reporter.step("deps", StepStatus::Done, "Packages installed");
    }

    // 4. Configuration + a stable copy of this launcher
    reporter.step("config", StepStatus::Running, "Writing configuration...");
    config::apply(&st_dir, opts.lan_enabled)?;
    let exe = install_self_copy(paths);
    reporter.step(
        "config",
        StepStatus::Done,
        if opts.lan_enabled {
            "Configured for this PC and your phone"
        } else {
            "Configured for this PC only"
        },
    );

    // 5. Shortcuts
    if opts.create_shortcut {
        reporter.step("shortcut", StepStatus::Running, "Creating shortcuts...");
        match shortcut::create_all(&exe) {
            Ok(_) => {
                state.shortcut_created = true;
                reporter.step(
                    "shortcut",
                    StepStatus::Done,
                    "Desktop and Start Menu shortcuts created",
                );
            }
            Err(e) => {
                log::warn!("shortcut creation failed: {e:#}");
                reporter.step(
                    "shortcut",
                    StepStatus::Skipped,
                    &format!("Could not create shortcuts: {e:#}"),
                );
            }
        }
    } else {
        reporter.step("shortcut", StepStatus::Skipped, "Not requested");
    }

    // 6. Firewall
    if opts.lan_enabled && opts.add_firewall_rule {
        reporter.step(
            "firewall",
            StepStatus::Running,
            "Allowing phone connections through Windows Firewall (Windows will ask for permission)...",
        );
        if firewall::rule_exists() {
            state.firewall_rule_added = true;
            reporter.step("firewall", StepStatus::Done, "Firewall rule already present");
        } else {
            match firewall::add_rule_elevated(&paths.node_exe()) {
                Ok(firewall::FirewallOutcome::Added) => {
                    state.firewall_rule_added = true;
                    reporter.step("firewall", StepStatus::Done, "Phone connections allowed");
                }
                Ok(firewall::FirewallOutcome::Declined) => reporter.step(
                    "firewall",
                    StepStatus::Skipped,
                    "You said no to the Windows prompt. Windows will ask again the first time SillyTavern starts; choose Allow then.",
                ),
                Err(e) => {
                    log::warn!("firewall rule failed: {e:#}");
                    reporter.step(
                        "firewall",
                        StepStatus::Skipped,
                        &format!("Could not add the rule ({e:#}). Click Allow if Windows asks when SillyTavern starts."),
                    );
                }
            }
        }
    } else {
        reporter.step("firewall", StepStatus::Skipped, "Not needed");
    }

    state.installed_at = Some(chrono::Local::now().to_rfc3339());
    state.save(paths)?;
    Ok(state)
}

pub fn check_update(paths: &Paths) -> Result<UpdateCheck> {
    let client = download::client()?;
    let latest = sillytavern::latest_tag(&client)?;
    let installed = sillytavern::installed_version(paths);
    let available = installed.as_deref() != Some(latest.as_str());
    Ok(UpdateCheck {
        installed,
        latest,
        available,
    })
}

/// Installs the newest release next to the current one, then swaps folders and carries
/// user data across. The old folder is only deleted once the swap has succeeded.
pub fn run_update(
    paths: &Paths,
    job: Option<&JobObject>,
    stop_server: &dyn Fn(),
    reporter: &dyn Reporter,
) -> Result<LauncherState> {
    paths.ensure_dirs()?;
    let mut state = LauncherState::load(paths);
    let client = download::client()?;

    reporter.step("sillytavern", StepStatus::Running, "Checking the latest release...");
    let tag = sillytavern::latest_tag(&client)?;
    let new_dir = paths.st_new_dir();
    if new_dir.exists() {
        fs::remove_dir_all(&new_dir).context("could not clear SillyTavern.new")?;
    }
    sillytavern::download_and_extract(&client, &tag, &new_dir, paths, reporter)?;
    reporter.step("sillytavern", StepStatus::Done, &format!("SillyTavern {tag} downloaded"));

    npm::run_npm_ci(paths, &new_dir, job, reporter)?;
    reporter.step("deps", StepStatus::Done, "Packages installed");

    reporter.step(
        "config",
        StepStatus::Running,
        "Switching to the new version (your chats and characters come along)...",
    );
    stop_server();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let st_dir = paths.st_dir();
    let old_dir = paths.st_old_dir();
    if old_dir.exists() {
        fs::remove_dir_all(&old_dir).context("could not clear SillyTavern.old")?;
    }
    fs::rename(&st_dir, &old_dir)
        .context("could not move the current SillyTavern folder aside (is a file in it open?)")?;
    if let Err(e) = fs::rename(&new_dir, &st_dir) {
        let _ = fs::rename(&old_dir, &st_dir);
        return Err(e).context("could not move the new version into place");
    }
    for name in ["data", "plugins", "config.yaml", "whitelist.txt", "secrets.json"] {
        let from = old_dir.join(name);
        if !from.exists() {
            continue;
        }
        let to = st_dir.join(name);
        if to.is_dir() {
            fs::remove_dir_all(&to).ok();
        } else if to.is_file() {
            fs::remove_file(&to).ok();
        }
        fs::rename(&from, &to).with_context(|| format!("could not carry over {name}"))?;
    }
    config::apply(&st_dir, state.settings.lan_enabled)?;
    // Let SillyTavern add any config keys the new version introduced.
    let init = process::quiet_command(paths.node_exe())
        .arg("src/server-init.js")
        .current_dir(&st_dir)
        .env("PATH", process::prefixed_path(&paths.node_dir()))
        .output();
    if let Err(e) = init {
        log::warn!("server-init.js did not run: {e}");
    }
    if let Err(e) = fs::remove_dir_all(&old_dir) {
        log::warn!("could not delete {} yet: {e}", old_dir.display());
    }

    state.st_tag = Some(tag);
    state.st_version = sillytavern::installed_version(paths);
    state.save(paths)?;
    reporter.step("config", StepStatus::Done, "Update complete");
    Ok(state)
}

/// Copies the running exe into the install root so shortcuts have a permanent target.
/// Falls back to the current path if the copy fails (e.g. running from the install root already).
fn install_self_copy(paths: &Paths) -> PathBuf {
    let Ok(current) = std::env::current_exe() else {
        return paths.exe_copy();
    };
    let target = paths.exe_copy();
    let same = match (fs::canonicalize(&current), fs::canonicalize(&target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return target;
    }
    match fs::copy(&current, &target) {
        Ok(_) => {
            log::info!("copied launcher to {}", target.display());
            target
        }
        Err(e) => {
            log::warn!("could not copy the launcher into the install folder: {e}");
            current
        }
    }
}

/// True when node_modules looks like it was produced from the current package-lock.
fn deps_fresh(st_dir: &Path) -> bool {
    let marker = st_dir.join("node_modules").join(".package-lock.json");
    let lock = st_dir.join("package-lock.json");
    match (marker.metadata().and_then(|m| m.modified()), lock.metadata().and_then(|m| m.modified())) {
        (Ok(marker_time), Ok(lock_time)) => marker_time >= lock_time,
        _ => false,
    }
}

/// A SillyTavern folder without server.js is a broken partial install. Never delete it
/// in case it holds chats; park it under a timestamped name instead.
fn move_aside(dir: &Path) -> Result<()> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let parked = dir.with_file_name(format!("SillyTavern.broken-{stamp}"));
    fs::rename(dir, &parked)
        .with_context(|| format!("could not move {} aside", dir.display()))?;
    log::warn!("moved incomplete install to {}", parked.display());
    Ok(())
}
