//! What the system-event triggers watch: the current network, USB devices, displays, and
//! sleep/wake.
//!
//! Networks are identified by Wi-Fi name where the OS shows it, otherwise by the router's MAC
//! address. macOS hides Wi-Fi names from apps without Location access, but the router's
//! address is just as stable and needs no permission.

use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct NetworkInfo {
    pub ssid: Option<String>,
    pub gateway: Option<String>,
    pub gateway_mac: Option<String>,
}

impl NetworkInfo {
    /// A stable id: the Wi-Fi name, else `router:<mac>`, else `gateway:<ip>`.
    pub fn id(&self) -> Option<String> {
        if let Some(ssid) = &self.ssid {
            return Some(ssid.clone());
        }
        if let Some(mac) = &self.gateway_mac {
            return Some(format!("router:{mac}"));
        }
        self.gateway.as_ref().map(|ip| format!("gateway:{ip}"))
    }

    /// Does a configured network name match this one? Any of the forms counts.
    pub fn matches(&self, wanted: &str) -> bool {
        let wanted = wanted.trim().to_lowercase();
        if wanted.is_empty() {
            return false;
        }
        let candidates = [
            self.id(),
            self.ssid.clone(),
            self.gateway_mac.clone(),
            self.gateway_mac.as_ref().map(|m| format!("router:{m}")),
            self.gateway.clone(),
        ];
        candidates.into_iter().flatten().any(|c| c.to_lowercase() == wanted)
    }
}

pub(crate) async fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(Duration::from_secs(10), tokio::process::Command::new(program).args(args).output())
        .await
        .ok()?
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(any(target_os = "macos", windows))]
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim_start_matches([':', ' ', '\t']).trim()))
}

fn usable_ssid(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && s != "<redacted>" && !s.starts_with("You are not")).then(|| s.to_string())
}

/// The network this computer is on now; `None` when offline.
pub async fn network() -> Option<NetworkInfo> {
    let info = platform_network().await?;
    (info.gateway.is_some() || info.ssid.is_some()).then_some(info)
}

#[cfg(target_os = "macos")]
async fn platform_network() -> Option<NetworkInfo> {
    let route = output("/sbin/route", &["-n", "get", "default"]).await?;
    let gateway = field(&route, "gateway").map(str::to_string);
    let interface = field(&route, "interface").unwrap_or("en0").to_string();
    let ssid = match output("/usr/sbin/ipconfig", &["getsummary", &interface]).await {
        Some(summary) => summary.lines().find_map(|l| l.trim().strip_prefix("SSID :").and_then(usable_ssid)),
        None => None,
    };
    let gateway_mac = match &gateway {
        Some(ip) => output("/usr/sbin/arp", &["-n", ip]).await.and_then(|t| mac_in(&t)),
        None => None,
    };
    Some(NetworkInfo { ssid, gateway, gateway_mac })
}

#[cfg(target_os = "linux")]
async fn platform_network() -> Option<NetworkInfo> {
    let route = output("ip", &["route", "show", "default"]).await?;
    let gateway = route.split_whitespace().skip_while(|w| *w != "via").nth(1).map(str::to_string);
    let ssid = match output("iwgetid", &["-r"]).await {
        Some(s) => usable_ssid(&s),
        None => output("nmcli", &["-t", "-f", "active,ssid", "dev", "wifi"])
            .await
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("yes:").and_then(usable_ssid))),
    };
    let gateway_mac = match &gateway {
        Some(ip) => output("ip", &["neigh", "show", ip]).await.and_then(|t| mac_in(&t)),
        None => None,
    };
    Some(NetworkInfo { ssid, gateway, gateway_mac })
}

#[cfg(windows)]
async fn platform_network() -> Option<NetworkInfo> {
    let script = "$r = Get-NetRoute -DestinationPrefix 0.0.0.0/0 | Sort-Object RouteMetric | Select-Object -First 1; \
                  $n = Get-NetNeighbor -IPAddress $r.NextHop -ErrorAction SilentlyContinue; \
                  \"gateway: $($r.NextHop)\"; \"mac: $($n.LinkLayerAddress)\"";
    let text = output("powershell", &["-NoProfile", "-Command", script]).await?;
    let gateway = field(&text, "gateway").filter(|g| !g.is_empty()).map(str::to_string);
    let gateway_mac = field(&text, "mac").and_then(|m| mac_in(&m.replace('-', ":")));
    let ssid = output("netsh", &["wlan", "show", "interfaces"]).await.and_then(|t| {
        t.lines()
            .find(|l| l.trim().starts_with("SSID"))
            .and_then(|l| l.split_once(':').and_then(|(_, v)| usable_ssid(v)))
    });
    Some(NetworkInfo { ssid, gateway, gateway_mac })
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
async fn platform_network() -> Option<NetworkInfo> {
    None
}

/// The first MAC address in some command output, lowercased, zero-padded.
fn mac_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')').find_map(|word| {
        let parts: Vec<&str> = word.split(':').collect();
        (parts.len() == 6
            && parts.iter().all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_hexdigit())))
        .then(|| parts.iter().map(|p| format!("{:0>2}", p.to_lowercase())).collect::<Vec<_>>().join(":"))
    })
}

/// Names of the USB devices plugged in now.
pub async fn usb_devices() -> Vec<String> {
    let mut names = platform_usb().await;
    names.sort();
    names
}

#[cfg(target_os = "macos")]
async fn platform_usb() -> Vec<String> {
    let text = output("/usr/sbin/ioreg", &["-p", "IOUSB", "-w0", "-l"]).await.unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            l.split_once("\"USB Product Name\" = \"").map(|(_, rest)| rest.trim_end_matches('"').to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
async fn platform_usb() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/sys/bus/usb/devices") else { return Vec::new() };
    dir.filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("product")).ok())
        .map(|s| s.trim().to_string())
        .collect()
}

#[cfg(windows)]
async fn platform_usb() -> Vec<String> {
    let script =
        "Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'USB*' } | ForEach-Object { $_.FriendlyName }";
    output("powershell", &["-NoProfile", "-Command", script])
        .await
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
async fn platform_usb() -> Vec<String> {
    Vec::new()
}

/// How many displays are active; `None` when it can't be told.
pub async fn display_count() -> Option<u32> {
    platform_displays().await
}

#[cfg(target_os = "macos")]
async fn platform_displays() -> Option<u32> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    }
    let mut count = 0u32;
    // SAFETY: a null display buffer with max 0 asks only for the count, written to `count`.
    let err = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    (err == 0).then_some(count)
}

#[cfg(target_os = "linux")]
async fn platform_displays() -> Option<u32> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    let connected = dir
        .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("status")).ok())
        .filter(|s| s.trim() == "connected")
        .count();
    Some(connected as u32)
}

#[cfg(windows)]
async fn platform_displays() -> Option<u32> {
    let script = "@(Get-CimInstance -Namespace root\\wmi -ClassName WmiMonitorBasicDisplayParams -ErrorAction SilentlyContinue).Count";
    output("powershell", &["-NoProfile", "-Command", script]).await?.trim().parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
async fn platform_displays() -> Option<u32> {
    None
}

/// Notices sleep: the monotonic clock stops while asleep, the wall clock doesn't.
pub struct WakeDetector {
    mono: Instant,
    wall: SystemTime,
}

impl Default for WakeDetector {
    fn default() -> Self {
        Self { mono: Instant::now(), wall: SystemTime::now() }
    }
}

impl WakeDetector {
    /// Call regularly; true when the computer slept for more than `threshold` since last time.
    pub fn check(&mut self, threshold: Duration) -> bool {
        let (mono, wall) = (Instant::now(), SystemTime::now());
        let mono_elapsed = mono.duration_since(self.mono);
        let wall_elapsed = wall.duration_since(self.wall).unwrap_or_default();
        self.mono = mono;
        self.wall = wall;
        wall_elapsed > mono_elapsed + threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_macs() {
        assert_eq!(mac_in("? (192.168.3.1) at b0:fe:e5:25:cb:c3 on en0"), Some("b0:fe:e5:25:cb:c3".into()));
        assert_eq!(mac_in("192.168.1.1 dev wlan0 lladdr 0:11:2:aa:bb:c REACHABLE"), Some("00:11:02:aa:bb:0c".into()));
        assert_eq!(mac_in("no mac here 12:30"), None);
    }

    #[test]
    fn network_matching() {
        let home = NetworkInfo {
            ssid: None,
            gateway: Some("192.168.3.1".into()),
            gateway_mac: Some("b0:fe:e5:25:cb:c3".into()),
        };
        assert_eq!(home.id().as_deref(), Some("router:b0:fe:e5:25:cb:c3"));
        assert!(home.matches("router:B0:FE:E5:25:CB:C3"));
        assert!(home.matches("192.168.3.1"));
        assert!(!home.matches("Office"));
        let wifi = NetworkInfo { ssid: Some("Home".into()), ..home };
        assert!(wifi.matches("home"));
    }

    #[tokio::test]
    async fn this_computer_reports_something() {
        // Smoke test: these must not panic or hang on any platform.
        let _ = network().await;
        let _ = usb_devices().await;
        let _ = display_count().await;
        assert!(!WakeDetector::default().check(Duration::from_secs(30)));
    }
}
