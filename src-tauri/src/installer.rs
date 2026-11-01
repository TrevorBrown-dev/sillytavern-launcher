//! The end-to-end install and update flows. Every step is idempotent so a failed run
//! can simply be retried and picks up where it left off. Two install modes:
//! managed (SillyTavern lives in the launcher's folder) and adopted (an install the
//! user already had, run in place).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::process::JobObject;
use crate::reporter::{Reporter, StepStatus};
use crate::state::{InstallMode, LauncherState};
use crate::{config, discover, download, firewall, git, node, npm, process, shortcut, sillytavern};

#[derive(Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct InstallOptions {
    pub lan_enabled: bool,
    pub create_shortcut: bool,
    pub add_firewall_rule: bool,
    /// Run this existing SillyTavern folder in place instead of installing a fresh copy.
    pub adopt_path: Option<String>,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            create_shortcut: true,
            add_firewall_rule: true,
            adopt_path: None,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub installed: Option<String>,
    pub latest: String,
    pub available: bool,
    /// "git" for adopted clones (pull on their branch), "release" for archive swaps.
    pub method: String,
    pub branch: Option<String>,
    pub detail: Option<String>,
}

pub fn run_install(
    base: &Paths,
    opts: &InstallOptions,
    job: Option<&JobObject>,
    reporter: &dyn Reporter,
) -> Result<LauncherState> {
    base.ensure_dirs()?;
    let mut state = LauncherState::load(base);
    state.settings.lan_enabled = opts.lan_enabled;
    let adopt: Option<PathBuf> = opts
        .adopt_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let paths = base.with_st_dir(adopt.clone());
    let st_dir = paths.st_dir();
    let client = download::client()?;

    // 1. Node.js (always ours, even for adopted installs: no dependency on whatever they had)
    let node_version = node::install(&client, &paths, reporter)?;
    state.node_version = Some(node_version);

    // 2. SillyTavern itself
    let mut fresh = false;
    if let Some(dir) = &adopt {
        let info = discover::inspect(dir).with_context(|| {
            format!(
                "{} does not look like a SillyTavern folder (it needs server.js and package.json)",
                dir.display()
            )
        })?;
        state.mode = InstallMode::Adopted;
        state.st_dir = Some(PathBuf::from(&info.path));
        state.st_tag = None;
        state.settings.port = info.port;
        let mut detail = format!(
            "Using your SillyTavern {} at {}",
            info.version.clone().unwrap_or_else(|| "(unknown version)".into()),
            info.path
        );
        if let Some(branch) = &info.branch {
            detail.push_str(&format!(" (git, {branch} branch)"));
        }
        reporter.step("sillytavern", StepStatus::Done, &detail);
    } else {
        state.mode = InstallMode::Managed;
        state.st_dir = None;
        if st_dir.join("server.js").is_file() {
            let v = sillytavern::version_in(&st_dir).unwrap_or_default();
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
            sillytavern::download_and_extract(&client, &tag, &st_dir, &paths, reporter)?;
            state.st_tag = Some(tag.clone());
            fresh = true;
            reporter.step(
                "sillytavern",
                StepStatus::Done,
                &format!("SillyTavern {tag} downloaded"),
            );
        }
    }
    state.st_version = sillytavern::version_in(&st_dir);

    // 3. npm packages
    if adopt.is_some() {
        npm::run_npm_install(&paths, &st_dir, job, reporter)?;
        reporter.step("deps", StepStatus::Done, "Packages are in place");
    } else if !fresh && deps_fresh(&st_dir) {
        reporter.step("deps", StepStatus::Done, "Packages already installed");
    } else {
        npm::run_npm_ci(&paths, &st_dir, job, reporter)?;
        reporter.step("deps", StepStatus::Done, "Packages installed");
    }

    // 4. Configuration + a stable copy of this launcher
    reporter.step("config", StepStatus::Running, "Writing configuration...");
    if adopt.is_some() {
        config::apply_adopted(&st_dir)?;
    } else {
        config::apply(&st_dir, opts.lan_enabled)?;
    }
    let exe = install_self_copy(base);
    reporter.step(
        "config",
        StepStatus::Done,
        match (adopt.is_some(), opts.lan_enabled) {
            (true, _) => "Your config.yaml is kept; only the phone whitelist was added",
            (false, true) => "Configured for this PC and your phone",
            (false, false) => "Configured for this PC only",
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
    state.save(base)?;
    Ok(state)
}

/// Is a newer SillyTavern available? Git clones compare against their upstream branch;
/// everything else compares package.json against the latest GitHub release.
pub fn check_update(paths: &Paths) -> Result<UpdateCheck> {
    let st_dir = paths.st_dir();
    if paths.is_adopted() && git::is_repo(&st_dir) {
        if let Some(gitbin) = git::find_git() {
            let branch = git::branch(&st_dir);
            let behind = git::behind_count(&gitbin, &st_dir)?;
            let head = git::head_short(&gitbin, &st_dir);
            let version = sillytavern::version_in(&st_dir);
            let branch_name = branch.clone().unwrap_or_else(|| "upstream".to_string());
            return Ok(UpdateCheck {
                installed: version.map(|v| match &head {
                    Some(h) => format!("{v} ({h})"),
                    None => v,
                }),
                latest: branch_name.clone(),
                available: behind > 0,
                method: "git".to_string(),
                branch,
                detail: Some(if behind == 0 {
                    format!("Your {branch_name} branch is up to date.")
                } else {
                    format!("{behind} new commit(s) on {branch_name}.")
                }),
            });
        }
    }
    let client = download::client()?;
    let latest = sillytavern::latest_tag(&client)?;
    let installed = sillytavern::version_in(&st_dir);
    let available = installed.as_deref() != Some(latest.as_str());
    Ok(UpdateCheck {
        installed,
        latest,
        available,
        method: "release".to_string(),
        branch: None,
        detail: None,
    })
}

/// Updates SillyTavern. Git clones get `git pull --ff-only` on their own branch; everything
/// else gets the newest release installed next to the current one, then a folder swap that
/// carries user data across. The old folder is only deleted once the swap has succeeded.
pub fn run_update(
    base: &Paths,
    job: Option<&JobObject>,
    stop_server: &dyn Fn(),
    reporter: &dyn Reporter,
) -> Result<LauncherState> {
    base.ensure_dirs()?;
    let mut state = LauncherState::load(base);
    let paths = state.paths(base);
    let st_dir = paths.st_dir();

    if paths.is_adopted() && git::is_repo(&st_dir) {
        if let Some(gitbin) = git::find_git() {
            return update_via_git(base, &paths, &gitbin, job, stop_server, reporter, state);
        }
        log::warn!("adopted install is a git clone but git is not available; updating from the release archive");
    }

    let client = download::client()?;
    reporter.step("sillytavern", StepStatus::Running, "Checking the latest release...");
    let tag = sillytavern::latest_tag(&client)?;
    let new_dir = paths.st_new_dir();
    if new_dir.exists() {
        fs::remove_dir_all(&new_dir).context("could not clear the staging folder")?;
    }
    sillytavern::download_and_extract(&client, &tag, &new_dir, &paths, reporter)?;
    reporter.step("sillytavern", StepStatus::Done, &format!("SillyTavern {tag} downloaded"));

    npm::run_npm_ci(&paths, &new_dir, job, reporter)?;
    reporter.step("deps", StepStatus::Done, "Packages installed");

    reporter.step(
        "config",
        StepStatus::Running,
        "Switching to the new version (your chats and characters come along)...",
    );
    stop_server();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let old_dir = paths.st_old_dir();
    if old_dir.exists() {
        fs::remove_dir_all(&old_dir).context("could not clear the previous-version folder")?;
    }
    fs::rename(&st_dir, &old_dir)
        .context("could not move the current SillyTavern folder aside (is a file in it open?)")?;
    if let Err(e) = fs::rename(&new_dir, &st_dir) {
        let _ = fs::rename(&old_dir, &st_dir);
        return Err(e).context("could not move the new version into place");
    }
    for name in ["data", "plugins", "config.yaml", "whitelist.txt", "secrets.json", ".git"] {
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
    if paths.is_adopted() {
        config::apply_adopted(&st_dir)?;
    } else {
        config::apply(&st_dir, state.settings.lan_enabled)?;
    }
    run_server_init(&paths, &st_dir);
    if let Err(e) = fs::remove_dir_all(&old_dir) {
        log::warn!("could not delete {} yet: {e}", old_dir.display());
    }

    state.st_tag = Some(tag);
    state.st_version = sillytavern::version_in(&st_dir);
    state.save(base)?;
    reporter.step("config", StepStatus::Done, "Update complete");
    Ok(state)
}

fn update_via_git(
    base: &Paths,
    paths: &Paths,
    gitbin: &Path,
    job: Option<&JobObject>,
    stop_server: &dyn Fn(),
    reporter: &dyn Reporter,
    mut state: LauncherState,
) -> Result<LauncherState> {
    let st_dir = paths.st_dir();
    reporter.step(
        "sillytavern",
        StepStatus::Running,
        "Pulling the latest SillyTavern with git...",
    );
    stop_server();
    let output = git::pull_ff(gitbin, &st_dir).map_err(|e| {
        anyhow!(
            "{e:#}\nIf you edited files inside the SillyTavern folder, git refuses to fast-forward. Run `git stash` (or `git checkout -- .`) in that folder and try again."
        )
    })?;
    for line in output.lines() {
        log::info!(target: "sillytavern", "[git] {line}");
    }
    let summary = output.lines().last().unwrap_or("Pulled").to_string();
    reporter.step("sillytavern", StepStatus::Done, &summary);

    npm::run_npm_install(paths, &st_dir, job, reporter)?;
    reporter.step("deps", StepStatus::Done, "Packages updated");

    reporter.step("config", StepStatus::Running, "Finishing up...");
    config::apply_adopted(&st_dir)?;
    run_server_init(paths, &st_dir);
    state.st_version = sillytavern::version_in(&st_dir);
    state.save(base)?;
    reporter.step("config", StepStatus::Done, "Update complete");
    Ok(state)
}

/// Lets SillyTavern add any config keys the new version introduced.
fn run_server_init(paths: &Paths, st_dir: &Path) {
    let init = process::quiet_command(paths.node_exe())
        .arg("src/server-init.js")
        .current_dir(st_dir)
        .env("PATH", process::prefixed_path(&paths.node_dir()))
        .output();
    if let Err(e) = init {
        log::warn!("server-init.js did not run: {e}");
    }
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
    match (
        marker.metadata().and_then(|m| m.modified()),
        lock.metadata().and_then(|m| m.modified()),
    ) {
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
