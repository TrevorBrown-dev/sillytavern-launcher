use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::thread::JoinHandle;

use anyhow::{bail, Context, Result};

use crate::paths::Paths;
use crate::process::{self, JobObject};
use crate::reporter::{Reporter, StepStatus};

/// Same flags SillyTavern's own Start.bat uses, minus the per-launch reinstall.
const COMMON_FLAGS: &[&str] = &[
    "--no-audit",
    "--no-fund",
    "--loglevel=error",
    "--no-progress",
    "--omit=dev",
    "--ignore-scripts",
];

/// Installs SillyTavern's dependencies into `st_dir` with the bundled npm.
pub fn run_npm_ci(
    paths: &Paths,
    st_dir: &Path,
    job: Option<&JobObject>,
    reporter: &dyn Reporter,
) -> Result<()> {
    const STEP: &str = "deps";
    reporter.step(
        STEP,
        StepStatus::Running,
        "Installing SillyTavern's packages (this is the slow part, 1-5 minutes)...",
    );
    let status = run_npm(paths, st_dir, &["ci"], job)?;
    if status.success() {
        return Ok(());
    }
    log::warn!("npm ci exited with {status}; falling back to npm install");
    reporter.step(STEP, StepStatus::Running, "First attempt failed, retrying with npm install...");
    let status = run_npm(paths, st_dir, &["install", "--no-save"], job)?;
    if !status.success() {
        bail!("npm failed ({status}). Open the log for details; a flaky connection or antivirus scan is the usual cause.");
    }
    Ok(())
}

fn run_npm(
    paths: &Paths,
    st_dir: &Path,
    sub: &[&str],
    job: Option<&JobObject>,
) -> Result<ExitStatus> {
    let mut cmd = process::quiet_command(paths.node_exe());
    cmd.arg(paths.npm_cli())
        .args(sub)
        .args(COMMON_FLAGS)
        .current_dir(st_dir)
        .env("PATH", process::prefixed_path(&paths.node_dir()))
        .env("NODE_ENV", "production")
        .env("npm_config_cache", paths.npm_cache())
        .env("npm_config_update_notifier", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    log::info!("running npm {} in {}", sub.join(" "), st_dir.display());
    let mut child = cmd.spawn().context("could not start npm")?;
    process::attach_to_job(job, &child, "npm");
    let out = child.stdout.take().map(|o| spawn_line_logger(o, "npm"));
    let err = child.stderr.take().map(|e| spawn_line_logger(e, "npm"));
    let status = child.wait()?;
    if let Some(t) = out {
        let _ = t.join();
    }
    if let Some(t) = err {
        let _ = t.join();
    }
    Ok(status)
}

/// Streams a child's output into the log, one line at a time.
pub fn spawn_line_logger<R: Read + Send + 'static>(reader: R, label: &'static str) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(l) if !l.trim().is_empty() => log::info!(target: "sillytavern", "[{label}] {l}"),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
}
