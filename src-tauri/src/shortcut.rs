//! Desktop and Start Menu `.lnk` files via IShellLink, so Windows resolves them properly.

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

pub fn start_menu_shortcut_path() -> Option<PathBuf> {
    dirs::config_dir().map(|appdata| {
        appdata
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(SHORTCUT_NAME)
    })
}

/// Creates both shortcuts pointing at `target`. Returns the paths written.
pub fn create_all(target: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for lnk in [desktop_shortcut_path(), start_menu_shortcut_path()]
        .into_iter()
        .flatten()
    {
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        create_lnk(&lnk, target, "Launch SillyTavern")?;
        log::info!("created shortcut {}", lnk.display());
        written.push(lnk);
    }
    Ok(written)
}

fn create_lnk(lnk: &Path, target: &Path, description: &str) -> Result<()> {
    let lnk = lnk.to_path_buf();
    let target = target.to_path_buf();
    let description = description.to_string();
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
