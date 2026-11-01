use std::path::PathBuf;

use anyhow::{Context, Result};

/// File name of the copy of the launcher that lives inside the install root.
/// Shortcuts point at this copy so they keep working if the original download is deleted.
pub const EXE_NAME: &str = "SillyTavern Launcher.exe";

/// Every location the launcher reads or writes, derived from one root folder.
#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    /// `%LOCALAPPDATA%\SillyTavern`, or `STLAUNCHER_HOME` when that variable is set (handy for testing).
    pub fn detect() -> Result<Self> {
        let home = match std::env::var_os("STLAUNCHER_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => dirs::data_local_dir()
                .context("could not resolve the local app data folder")?
                .join("SillyTavern"),
        };
        Ok(Self { home })
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
        self.home.join("SillyTavern")
    }

    pub fn st_new_dir(&self) -> PathBuf {
        self.home.join("SillyTavern.new")
    }

    pub fn st_old_dir(&self) -> PathBuf {
        self.home.join("SillyTavern.old")
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

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.home.clone(), self.downloads(), self.npm_cache(), self.logs()] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        Ok(())
    }
}
