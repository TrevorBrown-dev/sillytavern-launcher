//! Just enough git for adopted installs that are clones: find the binary, read the branch,
//! count commits behind, fast-forward pull. Never touches the user's working tree otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::process;

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Current branch from `.git/HEAD` (no git binary needed). `None` for a detached HEAD.
pub fn branch(dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(dir.join(".git").join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string())
}

/// `git` from PATH, else the usual Windows install locations.
pub fn find_git() -> Option<PathBuf> {
    if works(Path::new("git")) {
        return Some(PathBuf::from("git"));
    }
    let mut candidates = vec![PathBuf::from(r"C:\Program Files\Git\cmd\git.exe")];
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("Programs").join("Git").join("cmd").join("git.exe"));
    }
    candidates.into_iter().find(|p| p.is_file() && works(p))
}

fn works(git: &Path) -> bool {
    process::quiet_command(git)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn command(git: &Path, dir: &Path) -> Command {
    let mut cmd = process::quiet_command(git);
    cmd.arg("-C")
        .arg(dir)
        // Never hang on a credential prompt; these are public clones.
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn run(git: &Path, dir: &Path, args: &[&str]) -> Result<String> {
    let out = command(git, dir)
        .args(args)
        .output()
        .with_context(|| format!("could not run git {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        bail!(
            "git {} failed ({}): {}",
            args.join(" "),
            out.status,
            if stderr.is_empty() { stdout } else { stderr }
        );
    }
    Ok(if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}").trim().to_string()
    })
}

pub fn head_short(git: &Path, dir: &Path) -> Option<String> {
    run(git, dir, &["rev-parse", "--short", "HEAD"]).ok()
}

/// Fetches and returns how many commits the checkout is behind its upstream.
pub fn behind_count(git: &Path, dir: &Path) -> Result<u32> {
    run(git, dir, &["fetch", "--quiet"])?;
    let count = run(git, dir, &["rev-list", "--count", "HEAD..@{u}"])?;
    count
        .trim()
        .parse()
        .with_context(|| format!("unexpected rev-list output: {count}"))
}

/// `git pull --ff-only`; fails cleanly on local edits or a diverged branch.
pub fn pull_ff(git: &Path, dir: &Path) -> Result<String> {
    run(git, dir, &["pull", "--ff-only"])
}
