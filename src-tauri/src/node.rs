//! Portable Node.js: resolve a release, download, verify, unpack. Nothing touches the system PATH.

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::paths::Paths;
use crate::reporter::{Reporter, StepStatus};
use crate::{archive, download, process};

/// Node major the launcher prefers. Bump when SillyTavern moves on; the resolver
/// always takes the newest patch release of this line.
pub const PREFERRED_MAJOR: u32 = 24;
/// SillyTavern's `engines.node` floor.
pub const MIN_MAJOR: u32 = 20;
const DIST_INDEX: &str = "https://nodejs.org/dist/index.json";

#[derive(Deserialize)]
struct DistEntry {
    version: String,
    #[serde(default)]
    lts: serde_json::Value,
    #[serde(default)]
    files: Vec<String>,
}

pub struct NodeRelease {
    pub version: String,
    pub zip_name: String,
    pub zip_url: String,
    pub shasums_url: String,
}

pub fn major_of(version: &str) -> Option<u32> {
    version.trim_start_matches('v').split('.').next()?.parse().ok()
}

pub fn resolve(client: &Client) -> Result<NodeRelease> {
    let entries: Vec<DistEntry> = download::get_json(client, DIST_INDEX)
        .context("could not read the Node.js release list")?;
    let has_zip = |e: &&DistEntry| e.files.iter().any(|f| f == "win-x64-zip");
    // index.json is newest-first, so the first hit is the latest patch of that line.
    let pick = entries
        .iter()
        .filter(has_zip)
        .find(|e| major_of(&e.version) == Some(PREFERRED_MAJOR))
        .or_else(|| {
            entries.iter().filter(has_zip).find(|e| {
                let is_lts = !e.lts.is_null() && e.lts.as_bool() != Some(false);
                is_lts && major_of(&e.version).is_some_and(|m| m >= MIN_MAJOR)
            })
        })
        .context("no suitable Node.js release found")?;
    let version = pick.version.clone();
    let zip_name = format!("node-{version}-win-x64.zip");
    Ok(NodeRelease {
        zip_url: format!("https://nodejs.org/dist/{version}/{zip_name}"),
        shasums_url: format!("https://nodejs.org/dist/{version}/SHASUMS256.txt"),
        zip_name,
        version,
    })
}

/// Runs `node --version` on the bundled copy.
pub fn installed_version(paths: &Paths) -> Option<String> {
    let out = process::quiet_command(paths.node_exe())
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Ensures a usable Node.js lives in `paths.node_dir()`. Returns its version string.
pub fn install(client: &Client, paths: &Paths, reporter: &dyn Reporter) -> Result<String> {
    const STEP: &str = "node";
    if let Some(v) = installed_version(paths) {
        if major_of(&v).is_some_and(|m| m >= MIN_MAJOR) {
            reporter.step(STEP, StepStatus::Done, &format!("Node.js {v} is already installed"));
            return Ok(v);
        }
        log::info!("bundled Node.js {v} is too old; replacing it");
    }

    reporter.step(STEP, StepStatus::Running, "Finding the latest Node.js release...");
    let rel = resolve(client)?;
    let zip_path = paths.downloads().join(&rel.zip_name);

    reporter.step(STEP, StepStatus::Running, &format!("Downloading Node.js {}...", rel.version));
    download::download(client, &rel.zip_url, &zip_path, STEP, reporter)?;

    reporter.step(STEP, StepStatus::Running, "Verifying the download...");
    let sums = download::get_text(client, &rel.shasums_url)?;
    let expected = sums
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let hash = it.next()?;
            let name = it.next()?;
            (name == rel.zip_name).then(|| hash.to_ascii_lowercase())
        })
        .next()
        .context("checksum for the Node.js zip was not found")?;
    let actual = download::sha256_file(&zip_path)?;
    if actual != expected {
        let _ = std::fs::remove_file(&zip_path);
        bail!("the Node.js download was corrupted (checksum mismatch)");
    }

    reporter.step(STEP, StepStatus::Running, "Unpacking Node.js...");
    let node_dir = paths.node_dir();
    if node_dir.exists() {
        std::fs::remove_dir_all(&node_dir).context("could not remove the old Node.js folder")?;
    }
    archive::extract_zip_strip_top(&zip_path, &node_dir, STEP, reporter)?;
    let _ = std::fs::remove_file(&zip_path);

    let v = installed_version(paths).context("Node.js was unpacked but does not run")?;
    reporter.step(STEP, StepStatus::Done, &format!("Node.js {v} ready"));
    Ok(v)
}
