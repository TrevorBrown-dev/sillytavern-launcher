use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// User-facing settings the launcher owns.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Listen on the LAN so phones on the same Wi-Fi can connect.
    pub lan_enabled: bool,
    pub port: u16,
    /// Open the browser once SillyTavern reports ready.
    pub open_browser_on_start: bool,
    /// Look for SillyTavern and launcher updates a few seconds after each start.
    pub check_updates_on_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lan_enabled: true,
            port: 8000,
            open_browser_on_start: true,
            check_updates_on_start: true,
        }
    }
}

/// Whether SillyTavern lives in the launcher's own folder or somewhere the user already had it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    #[default]
    Managed,
    Adopted,
}

/// Persisted record of what the launcher installed, stored as `launcher-state.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct LauncherState {
    pub mode: InstallMode,
    /// Only set for adopted installs: the folder that holds server.js.
    pub st_dir: Option<PathBuf>,
    pub st_version: Option<String>,
    pub st_tag: Option<String>,
    pub node_version: Option<String>,
    pub installed_at: Option<String>,
    pub shortcut_created: bool,
    pub firewall_rule_added: bool,
    pub settings: Settings,
}

impl LauncherState {
    pub fn load(paths: &Paths) -> Self {
        match std::fs::read_to_string(paths.state_file()) {
            // Tolerate a UTF-8 BOM (Notepad and PowerShell like to add one); serde_json does not.
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
                log::warn!("launcher-state.json is unreadable ({e}); starting from defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::create_dir_all(&paths.home)?;
        std::fs::write(paths.state_file(), text)
            .with_context(|| format!("could not write {}", paths.state_file().display()))
    }

    /// Paths with this state's adopted folder applied (if any).
    pub fn paths(&self, base: &Paths) -> Paths {
        base.with_st_dir(if self.mode == InstallMode::Adopted {
            self.st_dir.clone()
        } else {
            None
        })
    }
}

/// True when every piece needed to run SillyTavern is present on disk.
pub fn is_installed(paths: &Paths) -> bool {
    paths.node_exe().is_file()
        && paths.st_dir().join("server.js").is_file()
        && paths.st_dir().join("node_modules").is_dir()
}
