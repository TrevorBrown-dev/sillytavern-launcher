//! Inbound Windows Firewall rule for the bundled node.exe, limited to the local subnet.
//! Adding a rule needs admin rights, so it runs `netsh` through a single UAC prompt.

use std::path::Path;

use anyhow::{Context, Result};
use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::process;

pub const RULE_NAME: &str = "SillyTavern (Node.js)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallOutcome {
    Added,
    /// The user dismissed the UAC prompt.
    Declined,
}

/// `netsh ... show rule` needs no elevation and exits 1 when nothing matches.
pub fn rule_exists() -> bool {
    process::quiet_command("netsh")
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={RULE_NAME}"),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn add_rule_elevated(node_exe: &Path) -> Result<FirewallOutcome> {
    let params = format!(
        "advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow \
         program=\"{}\" protocol=TCP remoteip=LocalSubnet profile=any enable=yes \
         description=\"Lets phones on your Wi-Fi reach SillyTavern\"",
        node_exe.display()
    );
    log::info!("requesting elevation for: netsh {params}");
    let code = run_elevated("netsh.exe", &params)?;
    match code {
        Some(0) => {
            log::info!("firewall rule added");
            Ok(FirewallOutcome::Added)
        }
        Some(c) => anyhow::bail!("netsh exited with code {c}"),
        None => Ok(FirewallOutcome::Declined),
    }
}

/// Runs `file params` with the "runas" verb. `Ok(None)` means the UAC prompt was declined.
fn run_elevated(file: &str, params: &str) -> Result<Option<u32>> {
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(file);
    let params = HSTRING::from(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        // ShellExecuteEx wants COM on the calling thread; balance only a successful init.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> Result<Option<u32>> {
            if let Err(e) = ShellExecuteExW(&mut info) {
                if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    log::info!("elevation prompt declined by the user");
                    return Ok(None);
                }
                return Err(e).context("ShellExecuteEx failed");
            }
            if info.hProcess.is_invalid() {
                anyhow::bail!("elevated process handle was not returned");
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 0u32;
            let exit = GetExitCodeProcess(info.hProcess, &mut code).context("GetExitCodeProcess failed");
            let _ = CloseHandle(info.hProcess);
            exit?;
            Ok(Some(code))
        })();
        if com.is_ok() {
            CoUninitialize();
        }
        result
    }
}
