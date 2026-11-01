use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use zip::ZipArchive;

use crate::reporter::Reporter;

/// Extracts `zip_path` into `dest`, dropping the single top-level folder that
/// GitHub source archives and nodejs.org zips wrap everything in.
pub fn extract_zip_strip_top(
    zip_path: &Path,
    dest: &Path,
    step: &str,
    reporter: &dyn Reporter,
) -> Result<()> {
    let file = File::open(zip_path)
        .with_context(|| format!("could not open {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .context("the downloaded file is not a valid zip archive")?;
    let total = archive.len() as u64;
    fs::create_dir_all(dest)?;
    reporter.progress(step, 0, Some(total), "files");

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // skips entries with unsafe paths (../ etc.)
        };
        let stripped: PathBuf = {
            let mut components = rel.components();
            components.next();
            components.as_path().to_path_buf()
        };
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&stripped);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = File::create(&out)
                .with_context(|| format!("could not create {}", out.display()))?;
            io::copy(&mut entry, &mut f)?;
        }
        if i % 250 == 0 {
            reporter.progress(step, i as u64, Some(total), "files");
        }
    }
    reporter.progress(step, total, Some(total), "files");
    Ok(())
}
