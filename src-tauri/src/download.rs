use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::reporter::Reporter;

pub fn client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("STLauncher/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        // Whole-request cap (the blocking client defaults to 30 s, far too short for a zip).
        .timeout(Duration::from_secs(30 * 60))
        .build()
        .context("could not create the HTTP client")
}

pub fn get_text(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let resp = resp
        .error_for_status()
        .with_context(|| format!("{url} returned an error"))?;
    resp.text().context("could not read the response body")
}

pub fn get_json<T: DeserializeOwned>(client: &Client, url: &str) -> Result<T> {
    let text = get_text(client, url)?;
    serde_json::from_str(&text).with_context(|| format!("unexpected JSON from {url}"))
}

/// Downloads `url` to `dest` with progress on `step`, retrying transient failures.
pub fn download(
    client: &Client,
    url: &str,
    dest: &Path,
    step: &str,
    reporter: &dyn Reporter,
) -> Result<()> {
    const ATTEMPTS: u32 = 3;
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        match download_once(client, url, dest, step, reporter) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::warn!("download attempt {attempt}/{ATTEMPTS} failed for {url}: {e:#}");
                let _ = std::fs::remove_file(part_path(dest));
                last_err = Some(e);
                if attempt < ATTEMPTS {
                    std::thread::sleep(Duration::from_secs(2 * u64::from(attempt)));
                }
            }
        }
    }
    Err(last_err.expect("at least one attempt ran")).with_context(|| format!("could not download {url}"))
}

fn download_once(
    client: &Client,
    url: &str,
    dest: &Path,
    step: &str,
    reporter: &dyn Reporter,
) -> Result<()> {
    let part = part_path(dest);
    let mut resp = client.get(url).send()?.error_for_status()?;
    let total = resp.content_length();
    let mut file = BufWriter::with_capacity(1 << 20, File::create(&part)?);
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now();
    reporter.progress(step, 0, total, "bytes");
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if last.elapsed() >= Duration::from_millis(150) {
            reporter.progress(step, done, total, "bytes");
            last = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    if let Some(t) = total {
        if done != t {
            bail!("incomplete download: got {done} of {t} bytes");
        }
    }
    reporter.progress(step, done, Some(done), "bytes");
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn human_bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n >= (10.0 * MB) as u64 {
        format!("{:.0} MB", n as f64 / MB)
    } else if n >= MB as u64 {
        format!("{:.1} MB", n as f64 / MB)
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}
