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

/// Creates config.yaml from `default/config.yaml` on first run, then patches the
/// keys the launcher owns: `listen`, `whitelistMode`, the `whitelist` list and
/// `browserLaunch.enabled` (the launcher opens the browser itself).
pub fn apply(st_dir: &Path, lan_enabled: bool) -> Result<()> {
    let cfg = st_dir.join("config.yaml");
    let source = if cfg.is_file() {
        cfg.clone()
    } else {
        st_dir.join("default").join("config.yaml")
    };
    let text = fs::read_to_string(&source)
        .with_context(|| format!("could not read {}", source.display()))?;
    let patched = patch(&text, lan_enabled);
    if cfg.is_file() && patched == text {
        return Ok(());
    }
    fs::write(&cfg, patched).with_context(|| format!("could not write {}", cfg.display()))?;
    log::info!("wrote {} (lan_enabled={lan_enabled})", cfg.display());
    Ok(())
}

pub fn patch(text: &str, lan_enabled: bool) -> String {
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
            out.push(format!("listen: {lan_enabled}"));
            continue;
        }
        if is_top_scalar(line, "whitelistMode") {
            out.push("whitelistMode: true".to_string());
            continue;
        }
        if top_key.as_deref() == Some("browserLaunch") && is_nested_scalar(line, "enabled") {
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
        out.push(format!("listen: {lan_enabled}"));
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
}
