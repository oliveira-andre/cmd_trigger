//! What a phone needs to wake this computer, and waking its screen.

use serde::Serialize;
use std::time::Duration;

/// A network interface the phone can send a wake-up packet to.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Interface {
    pub name: String,
    pub mac: String,
    pub ip: String,
    /// Carries the default route (the Wi-Fi or Ethernet the computer is really using).
    pub primary: bool,
}

/// IPv4 interfaces with a hardware address, the default-route one first.
pub async fn interfaces() -> Vec<Interface> {
    use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
    let primary = default_interface().await;
    let mut list: Vec<Interface> = NetworkInterface::show()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|i| {
            let mac = i.mac_addr.clone().unwrap_or_default().to_lowercase();
            let name = i.name.clone();
            let primary = primary.as_deref() == Some(name.as_str());
            i.addr.into_iter().filter_map(move |a| match a {
                Addr::V4(v4) => {
                    Some(Interface { name: name.clone(), mac: mac.clone(), ip: v4.ip.to_string(), primary })
                }
                Addr::V6(_) => None,
            })
        })
        .filter(|i| {
            let ip: std::net::Ipv4Addr = i.ip.parse().unwrap_or(std::net::Ipv4Addr::LOCALHOST);
            !ip.is_loopback()
                && !ip.is_link_local()
                && i.mac.len() == 17
                && i.mac != "00:00:00:00:00:00"
                // VPNs, bridges and virtual adapters can't be woken.
                && !["utun", "bridge", "awdl", "llw", "vmnet", "docker", "veth", "br-", "virbr", "tailscale", "zt"]
                    .iter()
                    .any(|p| i.name.starts_with(p))
        })
        .collect();
    list.sort_by_key(|i| !i.primary);
    list.dedup_by(|a, b| a.ip == b.ip);
    list
}

async fn output(program: &str, args: &[&str]) -> Option<String> {
    let run = tokio::process::Command::new(program).args(args).output();
    let out = tokio::time::timeout(Duration::from_secs(5), run).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn default_interface() -> Option<String> {
    if cfg!(target_os = "macos") {
        let text = output("/sbin/route", &["-n", "get", "default"]).await?;
        text.lines().find_map(|l| l.trim().strip_prefix("interface:").map(|v| v.trim().to_string()))
    } else if cfg!(target_os = "linux") {
        let text = output("ip", &["route", "show", "default"]).await?;
        let mut words = text.split_whitespace();
        words.find(|w| *w == "dev").and_then(|_| words.next()).map(str::to_string)
    } else {
        None
    }
}

/// macOS "Wake for network access" (`pmset` womp) for the current power source. `None` where
/// it can't be read.
pub async fn wake_for_network() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let text = output("/usr/bin/pmset", &["-g"]).await?;
    parse_womp(&text)
}

fn parse_womp(pmset: &str) -> Option<bool> {
    pmset.lines().find_map(|l| {
        let mut words = l.split_whitespace();
        (words.next() == Some("womp")).then(|| words.next() == Some("1"))
    })
}

/// Turns on "Wake for network access" for every power source. macOS asks for an
/// administrator's password.
pub async fn enable_wake_for_network() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("only on macOS".into());
    }
    let script = r#"do shell script "/usr/bin/pmset -a womp 1" with prompt "CmdTrigger wants to let your iPhone wake this Mac over the network." with administrator privileges"#;
    let out = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if message.contains("-128") { "cancelled".into() } else { message })
    }
}

/// Turns the display on, as if someone touched the keyboard.
pub async fn wake_screen() -> Result<(), String> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("/usr/bin/caffeinate", &["-u", "-t", "3"])
    } else if cfg!(target_os = "linux") {
        ("xset", &["dpms", "force", "on"])
    } else {
        return Err("waking the screen isn't supported on this system".into());
    };
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    // caffeinate stays for its 3 seconds; don't wait for it.
    drop(status);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_womp() {
        let pmset = "System-wide power settings:\nCurrently in use:\n standby              1\n womp                 1\n networkoversleep     0\n";
        assert_eq!(parse_womp(pmset), Some(true));
        assert_eq!(parse_womp(" womp                 0\n"), Some(false));
        assert_eq!(parse_womp(" sleep 1\n"), None);
    }

    #[tokio::test]
    async fn lists_real_interfaces() {
        for i in interfaces().await {
            assert!(i.ip.parse::<std::net::Ipv4Addr>().is_ok(), "{i:?}");
            assert_eq!(i.mac.len(), 17, "{i:?}");
        }
    }
}
