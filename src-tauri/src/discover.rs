//! Finds SillyTavern installs the user already has, so the launcher can run them in place.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{config, git};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExistingInstall {
    pub path: String,
    pub version: Option<String>,
    pub is_git: bool,
    pub branch: Option<String>,
    pub port: u16,
    pub has_node_modules: bool,
    pub has_data: bool,
}

/// A folder is SillyTavern when it has server.js and a package.json named "sillytavern".
pub fn looks_like_sillytavern(dir: &Path) -> bool {
    dir.join("server.js").is_file() && package_name(dir).as_deref() == Some("sillytavern")
}

pub fn inspect(dir: &Path) -> Option<ExistingInstall> {
    if !looks_like_sillytavern(dir) {
        return None;
    }
    let is_git = git::is_repo(dir);
    Some(ExistingInstall {
        path: display_path(dir),
        version: crate::sillytavern::version_in(dir),
        is_git,
        branch: if is_git { git::branch(dir) } else { None },
        port: config::read_port(dir).unwrap_or(8000),
        has_node_modules: dir.join("node_modules").is_dir(),
        has_data: dir.join("data").is_dir(),
    })
}

/// Looks in the usual places. `exclude` is the launcher's own managed folder.
pub fn discover(exclude: Option<&Path>) -> Vec<ExistingInstall> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        bases.push(home.clone());
        for sub in ["Desktop", "Documents", "Downloads", "OneDrive\\Desktop", "OneDrive\\Documents"] {
            bases.push(home.join(sub));
        }
    }
    if let Some(desktop) = dirs::desktop_dir() {
        bases.push(desktop);
    }
    if let Some(docs) = dirs::document_dir() {
        bases.push(docs);
    }
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if std::fs::metadata(&root).is_ok() {
            bases.push(root);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for base in &bases {
        if !base.is_dir() {
            continue;
        }
        for name in ["SillyTavern", "sillytavern", "SillyTavern-Launcher\\SillyTavern", "ST"] {
            candidates.push(base.join(name));
        }
        // Shallow scan: any folder whose name mentions SillyTavern (people rename clones).
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten().take(400) {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if path.is_dir() && name.contains("sillytavern") {
                    candidates.push(path.clone());
                    candidates.push(path.join("SillyTavern"));
                }
            }
        }
    }

    let excluded = exclude.and_then(|p| std::fs::canonicalize(p).ok());
    let mut seen: HashSet<String> = HashSet::new();
    let mut found = Vec::new();
    for candidate in candidates {
        let Ok(canonical) = std::fs::canonicalize(&candidate) else {
            continue;
        };
        if excluded.as_ref() == Some(&canonical) {
            continue;
        }
        let key = canonical.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        if let Some(install) = inspect(&candidate) {
            found.push(install);
        }
    }
    found
}

fn package_name(dir: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Pkg {
        name: String,
    }
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str::<Pkg>(&text).ok().map(|p| p.name)
}

/// Absolute path without the `\\?\` prefix canonicalize adds on Windows.
pub fn display_path(dir: &Path) -> String {
    let p = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}
