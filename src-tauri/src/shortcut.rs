//! Desktop, Start Menu and Startup `.lnk` files via IShellLink, so Windows resolves them properly.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

pub const SHORTCUT_NAME: &str = "SillyTavern.lnk";

pub fn desktop_shortcut_path() -> Option<PathBuf> {
    dirs::desktop_dir().map(|d| d.join(SHORTCUT_NAME))
}

fn programs_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|appdata| {
        appdata
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    })
}

pub fn start_menu_shortcut_path() -> Option<PathBuf> {
    programs_dir().map(|p| p.join(SHORTCUT_NAME))
}

/// The per-user Startup folder: anything here runs at sign-in.
pub fn startup_shortcut_path() -> Option<PathBuf> {
    programs_dir().map(|p| p.join("Startup").join(SHORTCUT_NAME))
}

/// Creates the Desktop and Start Menu shortcuts pointing at `target`. Returns the paths written.
pub fn create_all(target: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for lnk in [desktop_shortcut_path(), start_menu_shortcut_path()]
        .into_iter()
        .flatten()
    {
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        create_lnk(&lnk, target, "Launch SillyTavern", None)?;
        log::info!("created shortcut {}", lnk.display());
        written.push(lnk);
    }
    Ok(written)
}

pub fn start_with_windows_enabled() -> bool {
    startup_shortcut_path().map(|p| p.is_file()).unwrap_or(false)
}

/// Adds or removes the Startup-folder shortcut (started minimized to the tray).
pub fn set_start_with_windows(enabled: bool, target: &Path) -> Result<()> {
    let lnk = startup_shortcut_path().context("could not resolve the Startup folder")?;
    if enabled {
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        create_lnk(&lnk, target, "SillyTavern (starts with Windows)", Some("--minimized"))?;
        log::info!("created startup shortcut {}", lnk.display());
    } else if lnk.exists() {
        std::fs::remove_file(&lnk).with_context(|| format!("could not remove {}", lnk.display()))?;
        log::info!("removed startup shortcut {}", lnk.display());
    }
    Ok(())
}

fn create_lnk(lnk: &Path, target: &Path, description: &str, arguments: Option<&str>) -> Result<()> {
    let lnk = lnk.to_path_buf();
    let target = target.to_path_buf();
    let description = description.to_string();
    let arguments = arguments.map(str::to_string);
    // Fresh thread so COM apartment state never collides with the UI thread's.
    std::thread::spawn(move || -> Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let result = (|| -> Result<()> {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .context("could not create a shell link object")?;
                let target_h = HSTRING::from(target.as_os_str());
                link.SetPath(&target_h)?;
                if let Some(dir) = target.parent() {
                    link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
                }
                link.SetDescription(&HSTRING::from(description.as_str()))?;
                link.SetIconLocation(&target_h, 0)?;
                if let Some(args) = &arguments {
                    link.SetArguments(&HSTRING::from(args.as_str()))?;
                }
                let file: IPersistFile = link.cast()?;
                file.Save(&HSTRING::from(lnk.as_os_str()), true)?;
                Ok(())
            })();
            CoUninitialize();
            result
        }
    })
    .join()
    .map_err(|_| anyhow::anyhow!("shortcut thread panicked"))?
}
