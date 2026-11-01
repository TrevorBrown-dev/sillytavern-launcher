//! Self-update for the launcher: newest release on GitHub, download the exe, verify the
//! SHA-256 digest GitHub publishes for release assets, swap it in next to the running copy.
//! Windows refuses to overwrite a running exe but allows renaming it, which is all we need.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::reporter::{Reporter, StepStatus};
use crate::download;

pub const LAUNCHER_REPO: &str = "TrevorBrown-dev/sillytavern-launcher";
pub const ASSET_NAME: &str = "SillyTavern-Launcher.exe";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LauncherUpdate {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub url: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub notes_url: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it
        .next()
        .map(|p| p.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("0"))
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

pub fn check(client: &Client) -> Result<LauncherUpdate> {
    let api = format!("https://api.github.com/repos/{LAUNCHER_REPO}/releases/latest");
    let rel: Release = download::get_json(client, &api).context("could not read the launcher's releases")?;
    let asset = rel.assets.iter().find(|a| a.name == ASSET_NAME);
    let latest = rel.tag_name.trim_start_matches('v').to_string();
    let available = match (parse_version(&latest), parse_version(CURRENT_VERSION)) {
        (Some(l), Some(c)) => l > c && asset.is_some(),
        _ => false,
    };
    Ok(LauncherUpdate {
        current: CURRENT_VERSION.to_string(),
        latest,
        available,
        url: asset.map(|a| a.browser_download_url.clone()),
        size: asset.map(|a| a.size),
        sha256: asset.and_then(|a| {
            a.digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(|h| h.to_ascii_lowercase())
        }),
        notes_url: Some(rel.html_url),
    })
}

/// Downloads and installs `update` into the launcher's home. Returns the exe to relaunch.
/// The caller stops the server, spawns that exe with `--wait-for-pid <own pid>` and exits.
pub fn apply(paths: &Paths, update: &LauncherUpdate, reporter: &dyn Reporter) -> Result<PathBuf> {
    let url = update
        .url
        .clone()
        .context("the latest release has no launcher exe attached")?;
    paths.ensure_dirs()?;
    let client = download::client()?;
    let staged = paths.exe_new();

    reporter.step("download", StepStatus::Running, &format!("Downloading launcher {}...", update.latest));
    download::download(&client, &url, &staged, "download", reporter)?;
    reporter.step("download", StepStatus::Done, "Downloaded");

    reporter.step("verify", StepStatus::Running, "Verifying the download...");
    let meta = fs::metadata(&staged)?;
    if let Some(size) = update.size {
        if meta.len() != size {
            let _ = fs::remove_file(&staged);
            bail!("downloaded file size does not match the release ({} vs {size} bytes)", meta.len());
        }
    }
    let mut head = [0u8; 2];
    {
        use std::io::Read;
        fs::File::open(&staged)?.read_exact(&mut head)?;
    }
    if &head != b"MZ" {
        let _ = fs::remove_file(&staged);
        bail!("downloaded file is not a Windows executable");
    }
    match &update.sha256 {
        Some(expected) => {
            let actual = download::sha256_file(&staged)?;
            if &actual != expected {
                let _ = fs::remove_file(&staged);
                bail!("checksum mismatch: the download does not match the published release");
            }
            reporter.step("verify", StepStatus::Done, "Checksum matches the release");
        }
        None => reporter.step("verify", StepStatus::Done, "Size and format look right (no checksum published)"),
    }

    reporter.step("swap", StepStatus::Running, "Installing the new launcher...");
    let target = paths.exe_copy();
    let parked = paths.exe_old();
    let _ = fs::remove_file(&parked);
    if target.exists() {
        fs::rename(&target, &parked).with_context(|| {
            format!("could not move the current launcher aside ({})", target.display())
        })?;
    }
    if let Err(e) = fs::rename(&staged, &target) {
        let _ = fs::rename(&parked, &target);
        return Err(e).context("could not move the new launcher into place");
    }
    reporter.step("swap", StepStatus::Done, &format!("Launcher {} installed", update.latest));
    log::info!("self-update: {} -> {} at {}", update.current, update.latest, target.display());
    Ok(target)
}

/// Deletes the previous launcher left behind by `apply`, once it is no longer running.
pub fn cleanup_old(paths: &Paths) {
    let parked = paths.exe_old();
    if parked.exists() {
        match fs::remove_file(&parked) {
            Ok(()) => log::info!("removed previous launcher {}", parked.display()),
            Err(e) => log::debug!("previous launcher still locked ({e}); will retry next start"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn parses_tags_and_crate_versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.19.0"), Some((1, 19, 0)));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
        assert!(parse_version("v0.2.0") > parse_version("0.1.0"));
    }
}
