//! Wake-on-LAN: the magic packet (6 × 0xFF, then the MAC 16 times) sent as a UDP broadcast.

use network_interface::{NetworkInterface, NetworkInterfaceConfig};

pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let separators_ok = text.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.' | ' '));
    if hex.len() != 12 || !separators_ok {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(mac)
}

pub fn magic_packet(mac: [u8; 6]) -> Vec<u8> {
    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac);
    }
    packet
}

/// Send the packet to `broadcast` (default 255.255.255.255) on ports 9 and 7.
pub async fn wake(mac: &str, broadcast: &str) -> anyhow::Result<()> {
    let mac = parse_mac(mac).ok_or_else(|| anyhow::anyhow!("\"{mac}\" is not a MAC address"))?;
    let target = if broadcast.trim().is_empty() { "255.255.255.255" } else { broadcast.trim() };
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let packet = magic_packet(mac);
    for port in [9, 7] {
        socket.send_to(&packet, (target, port)).await?;
    }
    Ok(())
}

/// This computer's adapter MAC addresses (lowercase, no loopback or all-zero), so other
/// computers can wake it.
pub fn mac_addresses() -> Vec<String> {
    let Ok(interfaces) = NetworkInterface::show() else { return Vec::new() };
    let mut macs: Vec<String> = Vec::new();
    for iface in interfaces {
        let loopback = iface.addr.iter().any(|a| a.ip().is_loopback()) || iface.name.starts_with("lo");
        let Some(mac) = iface.mac_addr.map(|m| m.to_lowercase()) else { continue };
        if loopback || mac == "00:00:00:00:00:00" || parse_mac(&mac).is_none() || macs.contains(&mac) {
            continue;
        }
        macs.push(mac);
    }
    macs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        assert_eq!(parse_mac("AA-bb-cc-dd-ee-0F"), Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x0f]));
        assert_eq!(parse_mac("aa:bb:cc:dd:ee"), None);
        assert_eq!(parse_mac("zz:bb:cc:dd:ee:ff"), None);
        let p = magic_packet([1, 2, 3, 4, 5, 6]);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xFF; 6]);
        assert_eq!(&p[96..], &[1, 2, 3, 4, 5, 6]);
    }
}
