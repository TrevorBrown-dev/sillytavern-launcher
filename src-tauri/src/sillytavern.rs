//! SillyTavern release lookup and download (plain GitHub archives; no Git needed).

use std::path::Path;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::paths::Paths;
use crate::reporter::{Reporter, StepStatus};
use crate::{archive, download};

pub const REPO: &str = "SillyTavern/SillyTavern";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub fn latest_tag(client: &Client) -> Result<String> {
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    match download::get_json::<Release>(client, &api) {
        Ok(r) if !r.tag_name.is_empty() => return Ok(r.tag_name),
        Ok(_) => log::warn!("GitHub API returned an empty tag; falling back to redirect probe"),
        Err(e) => log::warn!("GitHub API lookup failed ({e:#}); falling back to redirect probe"),
    }
    // /releases/latest redirects to /releases/tag/<tag>; no API quota involved.
    let resp = client
        .get(format!("https://github.com/{REPO}/releases/latest"))
        .send()
        .context("could not reach github.com")?;
    let url = resp.url().to_string();
    url.rsplit_once("/tag/")
        .map(|(_, tag)| tag.trim_end_matches('/').to_string())
        .filter(|tag| !tag.is_empty())
        .context("could not determine the latest SillyTavern release")
}

pub fn zip_url(tag: &str) -> String {
    format!("https://github.com/{REPO}/archive/refs/tags/{tag}.zip")
}

/// Version from `<dir>/package.json`.
pub fn version_in(dir: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Pkg {
        version: String,
    }
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str::<Pkg>(&text).ok().map(|p| p.version)
}

/// Downloads the release archive for `tag` and unpacks it into `dest`.
pub fn download_and_extract(
    client: &Client,
    tag: &str,
    dest: &Path,
    paths: &Paths,
    reporter: &dyn Reporter,
) -> Result<()> {
    const STEP: &str = "sillytavern";
    let zip_path = paths.downloads().join(format!("SillyTavern-{tag}.zip"));
    reporter.step(STEP, StepStatus::Running, &format!("Downloading SillyTavern {tag}..."));
    download::download(client, &zip_url(tag), &zip_path, STEP, reporter)?;
    reporter.step(STEP, StepStatus::Running, "Unpacking SillyTavern...");
    archive::extract_zip_strip_top(&zip_path, dest, STEP, reporter)?;
    let _ = std::fs::remove_file(&zip_path);
    if !dest.join("server.js").is_file() {
        bail!("the SillyTavern archive did not contain server.js");
    }
    Ok(())
}
