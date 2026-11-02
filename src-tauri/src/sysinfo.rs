//! Small facts about this PC for log headers, crash reports and the support bundle.

use std::time::{Duration, SystemTime};

/// "Windows 11 25H2 (build 26200.7019)", read from the registry (GetVersionEx lies to
/// unmanifested callers and RtlGetVersion has no display version).
pub fn windows_version() -> String {
    let Some(build) = reg_string("CurrentBuild") else {
        return "Windows (version unknown)".to_string();
    };
    let major = if build.parse::<u32>().unwrap_or(0) >= 22000 { "11" } else { "10" };
    let display = reg_string("DisplayVersion")
        .map(|d| format!(" {d}"))
        .unwrap_or_default();
    let ubr = reg_dword("UBR").map(|u| format!(".{u}")).unwrap_or_default();
    format!("Windows {major}{display} (build {build}{ubr})")
}

/// The WebView2 runtime the window renders with; a blank or broken window is usually this.
pub fn webview_version() -> String {
    tauri::webview_version().unwrap_or_else(|e| format!("unavailable ({e})"))
}

/// When Windows last booted. Fast Startup "shutdowns" do not reset this (the kernel
/// hibernates), so it only moves on a real restart or after power loss.
pub fn boot_time() -> Option<SystemTime> {
    let uptime = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    SystemTime::now().checked_sub(Duration::from_millis(uptime))
}

const CURRENT_VERSION_KEY: windows::core::PCWSTR =
    windows::core::w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_string(name: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let name = wide(name);
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CURRENT_VERSION_KEY,
            windows::core::PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let chars = (size as usize / 2).min(buf.len());
    let value = String::from_utf16_lossy(&buf[..chars]);
    let value = value.trim_end_matches('\0').trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn reg_dword(name: &str) -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let name = wide(name);
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            CURRENT_VERSION_KEY,
            windows::core::PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (rc == ERROR_SUCCESS).then_some(value)
}
