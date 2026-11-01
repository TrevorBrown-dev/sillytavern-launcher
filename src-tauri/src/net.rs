//! LAN addresses, port probes and the bits of process/network inspection the phone
//! diagnostics need (who is listening on the port, which adapter is the Wi-Fi one).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::process;

/// One private IPv4 address this PC has, with the adapter it belongs to.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub ip: String,
    pub adapter: String,
    /// VPN, Hyper-V, WSL, VirtualBox and friends: reachable from nothing a phone is on.
    pub virtual_adapter: bool,
    /// The adapter that owns the default route (what this PC uses to reach the internet).
    pub default_route: bool,
}

/// Adapter-name fragments that mark an address as not-the-Wi-Fi. Lower-case.
const VIRTUAL_HINTS: &[&str] = &[
    "vethernet", "virtualbox", "vmware", "hyper-v", "wsl", "tailscale", "wireguard",
    "nordlynx", "openvpn", "tap-", "docker", "radmin", "hamachi", "zerotier", "npcap",
    "bluetooth", "loopback", "vpn", "proton", "mullvad", "expressvpn", "surfshark", "warp",
    "tun", "pseudo", "virtual",
];

/// The IPv4 address on the interface that owns the default route. No packet is sent
/// (UDP connect only resolves the route).
pub fn default_route_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:53").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if usable(ip) => Some(ip),
        _ => None,
    }
}

fn usable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// Every private IPv4 address, best candidate for a phone first: real adapters before
/// virtual ones, the default-route adapter before the rest.
pub fn addresses() -> Vec<Address> {
    let default = default_route_ip();
    let mut list: Vec<Address> = Vec::new();
    if let Ok(ifaces) = local_ip_address::list_afinet_netifas() {
        for (name, ip) in ifaces {
            let IpAddr::V4(ip) = ip else { continue };
            if !usable(ip) || !ip.is_private() {
                continue;
            }
            if list.iter().any(|a| a.ip == ip.to_string()) {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            list.push(Address {
                ip: ip.to_string(),
                adapter: name,
                virtual_adapter: VIRTUAL_HINTS.iter().any(|h| lower.contains(h)),
                default_route: Some(ip) == default,
            });
        }
    }
    if list.is_empty() {
        if let Some(ip) = default.filter(|ip| ip.is_private()) {
            list.push(Address {
                ip: ip.to_string(),
                adapter: "Network".to_string(),
                virtual_adapter: false,
                default_route: true,
            });
        }
    }
    list.sort_by_key(|a| (a.virtual_adapter, !a.default_route));
    list
}

pub fn lan_ips() -> Vec<Ipv4Addr> {
    addresses().iter().filter_map(|a| a.ip.parse().ok()).collect()
}

pub fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

pub fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(400),
    )
    .is_ok()
}

/// Can this PC reach its own LAN address on the port? Proves the server is bound to
/// all interfaces rather than just localhost.
pub fn reachable(ip: Ipv4Addr, port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((ip, port)), Duration::from_millis(800)).is_ok()
}

/// True when whatever answers on `port` serves SillyTavern's front page.
pub fn looks_like_sillytavern(port: u16) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    else {
        return false;
    };
    match client.get(format!("http://127.0.0.1:{port}/")).send() {
        Ok(resp) => resp.text().map(|t| t.contains("SillyTavern")).unwrap_or(false),
        Err(_) => false,
    }
}

/// A TCP listener on this machine, from `netstat -ano`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    /// "0.0.0.0", "127.0.0.1", "::", "::1", ...
    pub address: String,
    pub pid: u32,
    pub process: Option<String>,
}

impl Listener {
    /// Listening on every interface, i.e. reachable from a phone (firewall permitting).
    pub fn is_wildcard(&self) -> bool {
        matches!(self.address.as_str(), "0.0.0.0" | "::" | "[::]")
    }
}

pub fn listeners_on_port(port: u16) -> Vec<Listener> {
    let Ok(out) = process::quiet_command("netstat").args(["-ano"]).output() else {
        return Vec::new();
    };
    parse_netstat(&String::from_utf8_lossy(&out.stdout), port)
        .into_iter()
        .map(|(address, pid)| Listener {
            process: process_name(pid),
            address,
            pid,
        })
        .collect()
}

fn parse_netstat(text: &str, port: u16) -> Vec<(String, u32)> {
    let suffix = format!(":{port}");
    let mut found = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || cols[0] != "TCP" || cols[3] != "LISTENING" {
            continue;
        }
        let Some(address) = cols[1].strip_suffix(&suffix) else {
            continue;
        };
        let Ok(pid) = cols[4].parse::<u32>() else {
            continue;
        };
        let address = address.trim_matches(|c| c == '[' || c == ']').to_string();
        if !found.iter().any(|(a, p)| *a == address && *p == pid) {
            found.push((address, pid));
        }
    }
    found
}

pub fn process_name(pid: u32) -> Option<String> {
    let out = process::quiet_command("tasklist")
        .args(["/FO", "CSV", "/NH", "/FI", &format!("PID eq {pid}")])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().find(|l| l.starts_with('"'))?;
    first
        .split(',')
        .next()
        .map(|s| s.trim_matches('"').to_string())
}

/// Kills `pid` and its children. Only used for a stray node.exe holding the port.
pub fn kill_pid(pid: u32) -> Result<()> {
    let out = process::quiet_command("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .context("could not run taskkill")?;
    if !out.status.success() {
        bail!(
            "taskkill failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

pub fn qr_svg(data: &str) -> Option<String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(data, EcLevel::M).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .dark_color(svg::Color("#15121c"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_netstat;

    #[test]
    fn parses_listeners_for_the_port_only() {
        let text = "
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1234
  TCP    0.0.0.0:8000           0.0.0.0:0              LISTENING       4321
  TCP    127.0.0.1:8000         127.0.0.1:52000        ESTABLISHED     4321
  TCP    [::]:8000              [::]:0                 LISTENING       4321
  TCP    127.0.0.1:18000        0.0.0.0:0              LISTENING       99
";
        let found = parse_netstat(text, 8000);
        assert_eq!(found, vec![("0.0.0.0".to_string(), 4321), ("::".to_string(), 4321)]);
    }
}
