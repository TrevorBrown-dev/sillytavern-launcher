//! Keeps SillyTavern's config.yaml in line with the launcher's settings without
//! clobbering anything else the user may have edited by hand. Line-based on
//! purpose: a YAML round-trip would strip the comments users rely on.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Private IPv4 ranges plus loopback. SillyTavern matches these with `ip-matching`
/// (CIDR supported) after normalising IPv4-mapped IPv6 addresses.
pub const LAN_WHITELIST: &[&str] = &[
    "::1",
    "127.0.0.1",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchMode {
    /// Launcher-owned install: `listen`, `whitelistMode`, the whitelist and
    /// `browserLaunch.enabled` are all ours to set.
    Managed { lan_enabled: bool },
    /// Someone else's install: only merge the whitelist, and turn whitelist mode on
    /// unless basic auth already protects the server. `listen` and the browser launch
    /// are driven by command-line flags for our launches, so their file keeps working
    /// exactly as before with Start.bat.
    Adopted,
}

/// Creates config.yaml from `default/config.yaml` on first run, then patches the
/// keys the launcher owns for a managed install.
pub fn apply(st_dir: &Path, lan_enabled: bool) -> Result<()> {
    apply_with(st_dir, PatchMode::Managed { lan_enabled })
}

/// Adopted install: minimal, reversible touch (see `PatchMode::Adopted`).
pub fn apply_adopted(st_dir: &Path) -> Result<()> {
    apply_with(st_dir, PatchMode::Adopted)
}

fn apply_with(st_dir: &Path, mode: PatchMode) -> Result<()> {
    let cfg = st_dir.join("config.yaml");
    let (source, mode) = if cfg.is_file() {
        (cfg.clone(), mode)
    } else {
        // No config yet (never started): seed it like a managed install would.
        let seeded = match mode {
            PatchMode::Adopted => PatchMode::Managed { lan_enabled: true },
            m => m,
        };
        (st_dir.join("default").join("config.yaml"), seeded)
    };
    let text = fs::read_to_string(&source)
        .with_context(|| format!("could not read {}", source.display()))?;
    let patched = patch_with(&text, mode);
    if cfg.is_file() && patched == text {
        return Ok(());
    }
    fs::write(&cfg, patched).with_context(|| format!("could not write {}", cfg.display()))?;
    log::info!("wrote {} ({mode:?})", cfg.display());
    Ok(())
}

#[cfg(test)]
pub fn patch(text: &str, lan_enabled: bool) -> String {
    patch_with(text, PatchMode::Managed { lan_enabled })
}

pub fn patch_with(text: &str, mode: PatchMode) -> String {
    let basic_auth = text
        .lines()
        .any(|l| is_top_scalar(l, "basicAuthMode") && scalar_value(l) == "true");
    let force_whitelist_mode = match mode {
        PatchMode::Managed { .. } => true,
        PatchMode::Adopted => !basic_auth,
    };
    let managed_lan = match mode {
        PatchMode::Managed { lan_enabled } => Some(lan_enabled),
        PatchMode::Adopted => None,
    };

    let mut out: Vec<String> = Vec::with_capacity(text.lines().count() + 8);
    let mut top_key: Option<String> = None;
    let mut saw_listen = false;
    let mut saw_whitelist = false;
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        if let Some(key) = top_level_key(line) {
            top_key = Some(key.to_string());
        }

        if is_top_scalar(line, "listen") {
            saw_listen = true;
            if let Some(lan_enabled) = managed_lan {
                out.push(format!("listen: {lan_enabled}"));
                continue;
            }
        }
        if is_top_scalar(line, "whitelistMode") && force_whitelist_mode {
            out.push("whitelistMode: true".to_string());
            continue;
        }
        if managed_lan.is_some()
            && top_key.as_deref() == Some("browserLaunch")
            && is_nested_scalar(line, "enabled")
        {
            out.push(format!("{}enabled: false", leading_ws(line)));
            continue;
        }
        if top_level_key(line) == Some("whitelist") {
            saw_whitelist = true;
            let mut existing: Vec<String> = Vec::new();
            let after_colon = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
            if after_colon.starts_with('[') {
                existing.extend(
                    after_colon
                        .trim_matches(|c| c == '[' || c == ']')
                        .split(',')
                        .map(unquote)
                        .filter(|s| !s.is_empty()),
                );
            } else {
                while let Some(next) = lines.peek() {
                    let trimmed = next.trim_start();
                    if next.starts_with(' ') && trimmed.starts_with("- ") {
                        existing.push(unquote(&trimmed[2..]));
                        lines.next();
                    } else {
                        break;
                    }
                }
            }
            out.push("whitelist:".to_string());
            for entry in merged_whitelist(existing) {
                out.push(format!("  - {entry}"));
            }
            continue;
        }
        out.push(line.to_string());
    }

    if !saw_listen {
        if let Some(lan_enabled) = managed_lan {
            out.push(format!("listen: {lan_enabled}"));
        }
    }
    if !saw_whitelist {
        out.push("whitelist:".to_string());
        for entry in merged_whitelist(Vec::new()) {
            out.push(format!("  - {entry}"));
        }
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// The `port:` value from an install's config.yaml, if it has one.
pub fn read_port(st_dir: &Path) -> Option<u16> {
    let text = fs::read_to_string(st_dir.join("config.yaml")).ok()?;
    text.lines()
        .find(|l| is_top_scalar(l, "port"))
        .and_then(|l| scalar_value(l).parse().ok())
}

fn merged_whitelist(existing: Vec<String>) -> Vec<String> {
    let mut merged = existing;
    for entry in LAN_WHITELIST {
        if !merged.iter().any(|m| m == entry) {
            merged.push((*entry).to_string());
        }
    }
    merged
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.split('#').next().unwrap_or("").trim();
    s.trim_matches(|c| c == '"' || c == '\'').to_string()
}

fn scalar_value(line: &str) -> &str {
    let value = line.split_once(':').map(|(_, v)| v).unwrap_or("");
    value.split('#').next().unwrap_or("").trim()
}

fn top_level_key(line: &str) -> Option<&str> {
    let first = line.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let (key, _) = line.split_once(':')?;
    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(key)
    } else {
        None
    }
}

fn is_top_scalar(line: &str, key: &str) -> bool {
    line.starts_with(key) && line[key.len()..].starts_with(':')
}

fn is_nested_scalar(line: &str, key: &str) -> bool {
    line.starts_with(' ') && {
        let t = line.trim_start();
        t.starts_with(key) && t[key.len()..].starts_with(':')
    }
}

fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "# -- DATA CONFIGURATION --
dataRoot: ./data
# Listen for incoming connections
listen: false
listenAddress:
  ipv4: 0.0.0.0
  ipv6: '[::]'
browserLaunch:
  # Open the browser automatically on server startup.
  enabled: true
  browser: 'default'
port: 8000
# Toggle whitelist mode
whitelistMode: true
enableForwardedWhitelist: true
# Whitelist of allowed IP addresses
whitelist:
  - ::1
  - 127.0.0.1
whitelistDockerHosts: true
basicAuthMode: false
sso:
  trustedProxies:
    - ::1
    - 127.0.0.1
";

    #[test]
    fn patches_only_owned_keys() {
        let out = patch(DEFAULT, true);
        assert!(out.contains("\nlisten: true\n"));
        assert!(out.contains("listenAddress:\n  ipv4: 0.0.0.0\n"));
        assert!(out.contains("browserLaunch:\n  # Open the browser automatically on server startup.\n  enabled: false\n  browser: 'default'\n"));
        assert!(out.contains("whitelistMode: true\n"));
        assert!(out.contains("whitelist:\n  - ::1\n  - 127.0.0.1\n  - 10.0.0.0/8\n  - 172.16.0.0/12\n  - 192.168.0.0/16\nwhitelistDockerHosts: true\n"));
        assert!(out.contains("sso:\n  trustedProxies:\n    - ::1\n    - 127.0.0.1\n"));
    }

    #[test]
    fn is_idempotent_and_keeps_user_entries() {
        let once = patch(DEFAULT, true);
        let with_user = once.replace("  - 192.168.0.0/16\n", "  - 192.168.0.0/16\n  - 100.64.0.5\n");
        let twice = patch(&with_user, true);
        assert_eq!(patch(&twice, true), twice);
        assert!(twice.contains("  - 100.64.0.5\n"));
        assert_eq!(twice.matches("10.0.0.0/8").count(), 1);
    }

    #[test]
    fn can_turn_listen_off() {
        let out = patch(&patch(DEFAULT, true), false);
        assert!(out.contains("\nlisten: false\n"));
    }

    #[test]
    fn adopted_only_merges_the_whitelist() {
        let out = patch_with(DEFAULT, PatchMode::Adopted);
        assert!(out.contains("\nlisten: false\n"), "listen must be left alone");
        assert!(out.contains("  enabled: true\n  browser: 'default'\n"), "browserLaunch must be left alone");
        assert!(out.contains("whitelistMode: true\n"));
        assert!(out.contains("  - 192.168.0.0/16\n"));
        assert_eq!(patch_with(&out, PatchMode::Adopted), out);
    }

    #[test]
    fn adopted_respects_basic_auth() {
        let text = DEFAULT
            .replace("whitelistMode: true", "whitelistMode: false")
            .replace("basicAuthMode: false", "basicAuthMode: true");
        let out = patch_with(&text, PatchMode::Adopted);
        assert!(out.contains("whitelistMode: false\n"), "basic auth already protects the server");
        assert!(out.contains("  - 10.0.0.0/8\n"));
    }

    #[test]
    fn reads_scalar_values_without_comments() {
        assert_eq!(scalar_value("port: 8123 # custom"), "8123");
        assert_eq!(scalar_value("basicAuthMode: true"), "true");
    }
}
