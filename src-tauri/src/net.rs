use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

/// The IPv4 address a phone on the same network would use to reach this PC:
/// whichever interface owns the default route. No packet is sent (UDP connect only).
pub fn primary_lan_ip() -> Option<Ipv4Addr> {
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

/// Primary address first, then any other private IPv4 (VPN/virtual adapters included,
/// which is why the UI shows them as "if that doesn't work, try").
pub fn lan_ips() -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    if let Some(p) = primary_lan_ip() {
        ips.push(p);
    }
    if let Ok(ifaces) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in ifaces {
            if let IpAddr::V4(ip) = ip {
                if usable(ip) && ip.is_private() && !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
        }
    }
    ips
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
