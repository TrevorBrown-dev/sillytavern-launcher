//! "Diagnose phone access": the checks a helper would run over someone's shoulder,
//! each with a plain-language result and, where possible, a one-click fix id.

use serde::Serialize;

use crate::paths::Paths;
use crate::server::{ServerState, ServerStatus};
use crate::state::Settings;
use crate::{config, firewall, net};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub level: Level,
    pub title: String,
    pub detail: String,
    /// One of: "lan", "start", "takeover", "firewall", "whitelist".
    pub fix: Option<String>,
    pub fix_label: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    pub checks: Vec<Check>,
    pub ok: bool,
    pub summary: String,
}

fn check(id: &str, level: Level, title: &str, detail: impl Into<String>) -> Check {
    Check {
        id: id.to_string(),
        level,
        title: title.to_string(),
        detail: detail.into(),
        fix: None,
        fix_label: None,
    }
}

fn with_fix(mut c: Check, fix: &str, label: &str) -> Check {
    c.fix = Some(fix.to_string());
    c.fix_label = Some(label.to_string());
    c
}

pub fn run(paths: &Paths, settings: &Settings, server: &ServerStatus) -> Diagnosis {
    let port = settings.port;
    let mut checks: Vec<Check> = Vec::new();

    // 1. Phone access switch
    if settings.lan_enabled {
        checks.push(check("lan", Level::Ok, "Phone access is on", ""));
    } else {
        checks.push(with_fix(
            check("lan", Level::Fail, "Phone access is turned off", "SillyTavern only answers on this PC until it is on."),
            "lan",
            "Turn it on",
        ));
    }

    // 2. Server running?
    let running = server.state == ServerState::Running;
    if running {
        checks.push(check("server", Level::Ok, "SillyTavern is running", ""));
    } else {
        checks.push(with_fix(
            check("server", Level::Warn, "SillyTavern is not running", "Phones can only connect while it runs."),
            "start",
            "Start it",
        ));
    }

    // 3. Listening on all interfaces?
    let listeners = net::listeners_on_port(port);
    let wildcard = listeners.iter().any(|l| l.is_wildcard());
    if wildcard {
        checks.push(check("listening", Level::Ok, &format!("Listening for phones on port {port}"), ""));
    } else if !listeners.is_empty() {
        let c = if server.external {
            check(
                "listening",
                Level::Fail,
                "The running copy only accepts this PC",
                "It was started outside the launcher (Start.bat or an old shortcut), so its own settings apply and phones are refused.",
            )
        } else {
            check(
                "listening",
                Level::Fail,
                "Only accepting connections from this PC",
                "The server is bound to localhost. Restarting it from the launcher fixes that.",
            )
        };
        checks.push(with_fix(c, "takeover", "Restart under the launcher"));
    } else if running {
        checks.push(check(
            "listening",
            Level::Warn,
            &format!("Nothing is listening on port {port}"),
            "The server may still be starting; try again in a few seconds.",
        ));
    }

    // 4. Firewall
    let blocks = firewall::block_rules_for(&paths.node_exe());
    if !blocks.is_empty() {
        checks.push(with_fix(
            check(
                "firewall-block",
                Level::Fail,
                "Windows Firewall is blocking Node.js",
                format!(
                    "A block rule exists ({}), usually from clicking Cancel on the Windows prompt. Phones cannot get through until it is removed.",
                    blocks.join(", ")
                ),
            ),
            "firewall",
            "Fix the firewall",
        ));
    } else if firewall::rule_exists() {
        checks.push(check("firewall", Level::Ok, "Windows Firewall allows phones", ""));
    } else {
        checks.push(with_fix(
            check(
                "firewall",
                Level::Warn,
                "No firewall rule yet",
                "Windows may show an 'allow Node.js' prompt when a phone connects. Adding the rule now avoids that (one admin prompt).",
            ),
            "firewall",
            "Allow through the firewall",
        ));
    }

    // 5. Address
    let addresses = net::addresses();
    match addresses.first() {
        None => checks.push(check(
            "address",
            Level::Fail,
            "No Wi-Fi or LAN address found",
            "This PC does not seem to be connected to a network a phone could share.",
        )),
        Some(primary) if primary.virtual_adapter => checks.push(check(
            "address",
            Level::Warn,
            "Only a VPN or virtual adapter has an address",
            format!(
                "{} ({}) is not reachable from a phone. Connect the PC to Wi-Fi or Ethernet.",
                primary.ip, primary.adapter
            ),
        )),
        Some(primary) => {
            let vpn_on_route = addresses
                .iter()
                .any(|a| a.virtual_adapter && a.default_route);
            if vpn_on_route {
                checks.push(check(
                    "address",
                    Level::Warn,
                    "A VPN or virtual adapter is active",
                    format!(
                        "Use the Wi-Fi address http://{}:{port} ({}) on the phone; the VPN address will not work.",
                        primary.ip, primary.adapter
                    ),
                ));
            } else {
                checks.push(check(
                    "address",
                    Level::Ok,
                    &format!("Phone address: http://{}:{port}", primary.ip),
                    format!("via {}", primary.adapter),
                ));
            }
            // 6. Can this PC reach itself on that address? (proves the binding)
            if running && wildcard {
                if let Ok(ip) = primary.ip.parse() {
                    if net::reachable(ip, port) {
                        checks.push(check("reach", Level::Ok, "The server answers on the Wi-Fi address", ""));
                    } else {
                        checks.push(with_fix(
                            check(
                                "reach",
                                Level::Fail,
                                "The server does not answer on the Wi-Fi address",
                                format!("http://{}:{port} failed from this PC itself.", primary.ip),
                            ),
                            "takeover",
                            "Restart under the launcher",
                        ));
                    }
                }
            }
        }
    }

    // 7. config.yaml
    let flags = config::read_flags(&paths.st_dir());
    if flags.exists {
        if flags.whitelist_mode.unwrap_or(true) && !flags.whitelist_covers_lan() {
            checks.push(with_fix(
                check(
                    "whitelist",
                    Level::Fail,
                    "config.yaml does not allow phones",
                    "The whitelist is missing the home-network ranges.",
                ),
                "whitelist",
                "Fix config.yaml",
            ));
        } else {
            checks.push(check("whitelist", Level::Ok, "config.yaml allows home-network devices", ""));
        }
        if flags.basic_auth == Some(true) {
            checks.push(check(
                "basic-auth",
                Level::Info,
                "The phone will ask for a username and password",
                "Basic auth is on in config.yaml (basicAuthUser). That is expected, not a failure.",
            ));
        }
        if flags.user_accounts == Some(true) {
            checks.push(check(
                "accounts",
                Level::Info,
                "The phone will show a login screen",
                "User accounts are on in config.yaml. Log in with the same user as on this PC.",
            ));
        }
    }

    let ok = !checks.iter().any(|c| c.level == Level::Fail);
    let summary = if ok {
        "Everything on this PC looks right. On the phone: same Wi-Fi as this PC (not mobile data, not a guest network), and type http:// rather than https://. The PC must stay awake.".to_string()
    } else {
        "Something on this PC needs fixing first; use the buttons below.".to_string()
    };
    Diagnosis { checks, ok, summary }
}
