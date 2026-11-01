use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// File name of the copy of the launcher that lives inside the install root.
/// Shortcuts point at this copy so they keep working if the original download is deleted,
/// and self-update replaces this copy.
pub const EXE_NAME: &str = "SillyTavern Launcher.exe";

/// Every location the launcher reads or writes. `home` is the launcher's own folder; the
/// SillyTavern folder is normally inside it but can be an adopted install anywhere else.
#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    st_dir_override: Option<PathBuf>,
}

impl Paths {
    /// `%LOCALAPPDATA%\SillyTavern`, or `STLAUNCHER_HOME` when that variable is set (handy for testing).
    /// The adopted-install override is applied afterwards from `launcher-state.json` (see `with_st_dir`).
    pub fn detect() -> Result<Self> {
        let home = match std::env::var_os("STLAUNCHER_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => dirs::data_local_dir()
                .context("could not resolve the local app data folder")?
                .join("SillyTavern"),
        };
        Ok(Self {
            home,
            st_dir_override: None,
        })
    }

    /// Same home, with the SillyTavern folder pointed somewhere else (or back to the managed default).
    pub fn with_st_dir(&self, dir: Option<PathBuf>) -> Self {
        Self {
            home: self.home.clone(),
            st_dir_override: dir,
        }
    }

    /// True when SillyTavern lives outside the launcher's own folder (an adopted install).
    pub fn is_adopted(&self) -> bool {
        self.st_dir_override.is_some()
    }

    pub fn node_dir(&self) -> PathBuf {
        self.home.join("node")
    }

    pub fn node_exe(&self) -> PathBuf {
        self.node_dir().join("node.exe")
    }

    /// npm's entry script inside the portable Node folder; run with node.exe so no PATH setup is needed.
    pub fn npm_cli(&self) -> PathBuf {
        self.node_dir()
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js")
    }

    pub fn st_dir(&self) -> PathBuf {
        self.st_dir_override
            .clone()
            .unwrap_or_else(|| self.home.join("SillyTavern"))
    }

    /// Where the updater unpacks the next version before swapping (a sibling of the live folder).
    pub fn st_new_dir(&self) -> PathBuf {
        sibling(&self.st_dir(), ".new")
    }

    pub fn st_old_dir(&self) -> PathBuf {
        sibling(&self.st_dir(), ".old")
    }

    pub fn downloads(&self) -> PathBuf {
        self.home.join("downloads")
    }

    pub fn npm_cache(&self) -> PathBuf {
        self.home.join("npm-cache")
    }

    pub fn logs(&self) -> PathBuf {
        self.home.join("logs")
    }

    pub fn launcher_log(&self) -> PathBuf {
        self.logs().join("launcher.log")
    }

    pub fn state_file(&self) -> PathBuf {
        self.home.join("launcher-state.json")
    }

    pub fn exe_copy(&self) -> PathBuf {
        self.home.join(EXE_NAME)
    }

    /// Staging name for a freshly downloaded launcher during self-update.
    pub fn exe_new(&self) -> PathBuf {
        self.home.join("SillyTavern Launcher.new.exe")
    }

    /// Where the previous launcher is parked during self-update (deleted on the next start).
    pub fn exe_old(&self) -> PathBuf {
        self.home.join("SillyTavern Launcher.old.exe")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.home.clone(), self.downloads(), self.npm_cache(), self.logs()] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        Ok(())
    }
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "SillyTavern".to_string());
    dir.with_file_name(format!("{name}{suffix}"))
}
