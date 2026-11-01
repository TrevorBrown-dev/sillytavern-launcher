//! Inbound Windows Firewall rule for the bundled node.exe, limited to private address
//! ranges. Adding or removing rules needs admin rights, so those run through a single
//! UAC prompt; reading rules does not.

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
/// Every private IPv4 range, so homes with more than one subnet still work while the
/// internet side stays closed. Matches the whitelist the launcher writes.
pub const PRIVATE_RANGES: &str = "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub direction: String,
    pub action: String,
    pub program: Option<String>,
}

/// Names of inbound *Block* rules that target `program`. Windows creates these when
/// someone clicks Cancel on the "allow Node.js" popup, and they beat our allow rule.
pub fn block_rules_for(program: &Path) -> Vec<String> {
    let Ok(out) = process::quiet_command("netsh")
        .args(["advfirewall", "firewall", "show", "rule", "name=all", "dir=in", "verbose"])
        .output()
    else {
        return Vec::new();
    };
    let wanted = program.to_string_lossy().to_ascii_lowercase();
    parse_rules(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .filter(|r| {
            r.action.eq_ignore_ascii_case("block")
                && r.program
                    .as_deref()
                    .map(|p| p.to_ascii_lowercase() == wanted)
                    .unwrap_or(false)
        })
        .map(|r| r.name)
        .collect()
}

/// Parses `netsh advfirewall firewall show rule ... verbose` output (English labels).
pub fn parse_rules(text: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut current: Option<Rule> = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "Rule Name" => {
                if let Some(r) = current.take() {
                    rules.push(r);
                }
                current = Some(Rule {
                    name: value.to_string(),
                    direction: String::new(),
                    action: String::new(),
                    program: None,
                });
            }
            "Direction" => {
                if let Some(r) = current.as_mut() {
                    r.direction = value.to_string();
                }
            }
            "Action" => {
                if let Some(r) = current.as_mut() {
                    r.action = value.to_string();
                }
            }
            "Program" => {
                if let Some(r) = current.as_mut() {
                    if !value.is_empty() && value != "Any" {
                        r.program = Some(value.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(r) = current {
        rules.push(r);
    }
    rules
}

fn add_rule_args(node_exe: &Path) -> String {
    format!(
        "advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow \
         program=\"{}\" protocol=TCP remoteip={PRIVATE_RANGES} profile=any enable=yes \
         description=\"Lets phones on your Wi-Fi reach SillyTavern\"",
        node_exe.display()
    )
}

pub fn add_rule_elevated(node_exe: &Path) -> Result<FirewallOutcome> {
    let params = add_rule_args(node_exe);
    log::info!("requesting elevation for: netsh {params}");
    match run_elevated("netsh.exe", &params)? {
        Some(0) => {
            log::info!("firewall rule added");
            Ok(FirewallOutcome::Added)
        }
        Some(c) => anyhow::bail!("netsh exited with code {c}"),
        None => Ok(FirewallOutcome::Declined),
    }
}

/// One UAC prompt that removes any Block rules for node.exe, replaces our allow rule
/// (so an older local-subnet-only rule gets the private ranges) and adds it back.
pub fn repair_elevated(node_exe: &Path) -> Result<FirewallOutcome> {
    let blocks = block_rules_for(node_exe);
    let mut commands: Vec<String> = Vec::new();
    for name in &blocks {
        commands.push(format!(
            "netsh advfirewall firewall delete rule name=\"{name}\" program=\"{}\"",
            node_exe.display()
        ));
    }
    commands.push(format!(
        "netsh advfirewall firewall delete rule name=\"{RULE_NAME}\" >nul 2>&1"
    ));
    commands.push(format!("netsh {}", add_rule_args(node_exe)));
    let params = format!("/c {}", commands.join(" & "));
    log::info!(
        "requesting elevation to repair the firewall ({} block rule(s) to remove)",
        blocks.len()
    );
    match run_elevated("cmd.exe", &params)? {
        Some(0) => Ok(FirewallOutcome::Added),
        Some(c) => anyhow::bail!("firewall repair exited with code {c}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verbose_rule_dump() {
        let text = "
Rule Name:                            Node.js JavaScript Runtime
----------------------------------------------------------------------
Enabled:                              Yes
Direction:                            In
Profiles:                             Public
Grouping:
LocalIP:                              Any
RemoteIP:                             Any
Protocol:                             TCP
LocalPort:                            Any
RemotePort:                           Any
Edge traversal:                       No
Program:                              C:\\Users\\x\\AppData\\Local\\SillyTavern\\node\\node.exe
InterfaceTypes:                       Any
Security:                             NotRequired
Rule source:                          Local Setting
Action:                               Block

Rule Name:                            SillyTavern (Node.js)
----------------------------------------------------------------------
Enabled:                              Yes
Direction:                            In
Program:                              C:\\Users\\x\\AppData\\Local\\SillyTavern\\node\\node.exe
Action:                               Allow
";
        let rules = parse_rules(text);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "Node.js JavaScript Runtime");
        assert_eq!(rules[0].action, "Block");
        assert_eq!(
            rules[0].program.as_deref(),
            Some(r"C:\Users\x\AppData\Local\SillyTavern\node\node.exe")
        );
        assert_eq!(rules[1].action, "Allow");
    }
}
