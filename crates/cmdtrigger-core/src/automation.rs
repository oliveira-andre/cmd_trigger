//! Automations: when something happens (triggers), and the situation allows it (conditions),
//! do these steps in order. Stored in `automations.json`:
//!
//! ```json
//! [{
//!   "id": "k3v9x2", "name": "Morning", "enabled": true, "icon": "☀️", "ground": "foreground",
//!   "triggers": [{"type": "schedule", "at": "07:30", "days": ["mon","tue","wed","thu","fri"]},
//!                {"type": "network", "network": "Home Wi-Fi", "event": "joined"}],
//!   "conditions": [{"type": "time", "after": "06:00", "before": "11:00"}],
//!   "steps": [{"type": "command", "command": "Open a website", "params": "https://news.example"},
//!             {"type": "wait", "seconds": 2},
//!             {"type": "notify", "title": "Good morning"}],
//!   "stopOnError": true
//! }]
//! ```
//!
//! Every automation can also be run by hand (tray, dashboard, API, MCP, HomeKit, Home
//! Assistant), which skips the conditions. A step's `params` may contain `{params}`, replaced
//! by the parameters the automation was run with.

use cmdtrigger_protocol::{CatalogEntry, TargetKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "foreground")]
    pub ground: String,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default = "yes")]
    pub stop_on_error: bool,
    /// Accept parameters when run by hand or by webhook (passed to steps as `{params}`).
    #[serde(default)]
    pub allow_params: bool,
}

fn yes() -> bool {
    true
}
fn foreground() -> String {
    "foreground".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn from_chrono(day: chrono::Weekday) -> Self {
        match day {
            chrono::Weekday::Mon => Weekday::Mon,
            chrono::Weekday::Tue => Weekday::Tue,
            chrono::Weekday::Wed => Weekday::Wed,
            chrono::Weekday::Thu => Weekday::Thu,
            chrono::Weekday::Fri => Weekday::Fri,
            chrono::Weekday::Sat => Weekday::Sat,
            chrono::Weekday::Sun => Weekday::Sun,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkEvent {
    #[default]
    Joined,
    Left,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceEvent {
    #[default]
    Connected,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    /// At a time of day ("HH:MM", local), on some weekdays (none listed = every day).
    Schedule {
        at: String,
        #[serde(default)]
        days: Vec<Weekday>,
    },
    /// Every N minutes while the agent runs.
    Interval { minutes: u32 },
    /// When the agent starts (at login for the tray app, at boot for the service).
    Startup,
    /// When the computer wakes from sleep.
    Wake,
    /// When joining (or leaving) a network. An empty `network` matches any change.
    Network {
        #[serde(default)]
        network: String,
        #[serde(default)]
        event: NetworkEvent,
    },
    /// When a file, or anything in a folder, changes.
    File {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    /// When a USB device whose name contains `name` (empty = any) is plugged in or removed.
    Usb {
        #[serde(default)]
        event: DeviceEvent,
        #[serde(default)]
        name: String,
    },
    /// When a display is connected or disconnected.
    Display {
        #[serde(default)]
        event: DeviceEvent,
    },
    /// `POST http://<this computer>:<port>/hooks/<hook>` (no API key needed; the hook is secret).
    Webhook { hook: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Condition {
    /// Only on (or, with `not`, off) this network.
    Network {
        network: String,
        #[serde(default)]
        not: bool,
    },
    /// Only between two times of day ("22:00"–"06:00" wraps past midnight).
    Time { after: String, before: String },
    /// Only on these weekdays.
    Days { days: Vec<Weekday> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Step {
    /// Run one of this computer's commands.
    Command {
        command: String,
        #[serde(default)]
        params: String,
    },
    /// Run a shell line directly.
    Shell {
        line: String,
    },
    Wait {
        seconds: f64,
    },
    /// Send a Wake-on-LAN magic packet.
    WakeOnLan {
        mac: String,
        #[serde(default)]
        broadcast: String,
    },
    /// Show a system notification.
    Notify {
        title: String,
        #[serde(default)]
        body: String,
    },
}

impl Automation {
    pub fn catalog_entry(&self) -> CatalogEntry {
        CatalogEntry {
            kind: TargetKind::Automation,
            id: self.id.clone(),
            name: self.name.clone(),
            icon: self.icon.clone(),
            description: self.description.clone(),
            allow_params: self.allow_params,
            has_off: false,
            ground: if self.ground == "background" { "background".into() } else { "foreground".into() },
        }
    }

    pub fn webhook(&self) -> Option<&str> {
        self.triggers.iter().find_map(|t| match t {
            Trigger::Webhook { hook } => Some(hook.as_str()),
            _ => None,
        })
    }
}

/// "07:30" → minutes after midnight.
pub fn parse_hhmm(text: &str) -> Option<u32> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Problems that would stop an automation from working, for the editor to show.
pub fn validate(a: &Automation, command_names: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    if a.name.trim().is_empty() {
        problems.push("The automation needs a name.".into());
    }
    if a.steps.is_empty() {
        problems.push("Add at least one step.".into());
    }
    for t in &a.triggers {
        match t {
            Trigger::Schedule { at, .. } if parse_hhmm(at).is_none() => {
                problems.push(format!("\"{at}\" is not a time like 07:30."))
            }
            Trigger::Interval { minutes } if *minutes == 0 => {
                problems.push("An interval must be at least 1 minute.".into())
            }
            Trigger::File { path, .. } if path.trim().is_empty() => {
                problems.push("A file trigger needs a path.".into())
            }
            Trigger::Webhook { hook } if hook.len() < 8 => {
                problems.push("A webhook id must be at least 8 characters.".into())
            }
            _ => {}
        }
    }
    for c in &a.conditions {
        if let Condition::Time { after, before } = c
            && (parse_hhmm(after).is_none() || parse_hhmm(before).is_none())
        {
            problems.push("A time condition needs two times like 08:00.".into());
        }
    }
    for s in &a.steps {
        match s {
            Step::Command { command, .. } if !command_names.iter().any(|n| n.eq_ignore_ascii_case(command)) => {
                problems.push(format!("There is no command named \"{command}\"."))
            }
            Step::WakeOnLan { mac, .. } if crate::wol::parse_mac(mac).is_none() => {
                problems.push(format!("\"{mac}\" is not a MAC address."))
            }
            _ => {}
        }
    }
    problems
}

/// A short random id for automations and webhooks.
pub fn new_id(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let text = r#"[{
          "id": "k3v9x2", "name": "Morning", "icon": "☀️",
          "triggers": [{"type": "schedule", "at": "07:30", "days": ["mon","fri"]},
                       {"type": "network", "network": "Home Wi-Fi"}, {"type": "startup"},
                       {"type": "usb", "event": "disconnected", "name": "Keyboard"}, {"type": "webhook", "hook": "abcdefgh12"}],
          "conditions": [{"type": "time", "after": "06:00", "before": "11:00"}],
          "steps": [{"type": "command", "command": "Calculator"}, {"type": "wait", "seconds": 2},
                    {"type": "wakeOnLan", "mac": "aa:bb:cc:dd:ee:ff"}, {"type": "notify", "title": "Hi"}]
        }]"#;
        let list: Vec<Automation> = serde_json::from_str(text).unwrap();
        let a = &list[0];
        assert!(a.enabled && a.stop_on_error);
        assert_eq!(a.ground, "foreground");
        assert_eq!(a.webhook(), Some("abcdefgh12"));
        assert!(matches!(a.triggers[1], Trigger::Network { event: NetworkEvent::Joined, .. }));
        assert!(validate(a, &["Calculator"]).is_empty());
        assert_eq!(validate(a, &["Other"]).len(), 1);
        let back: Vec<Automation> = serde_json::from_str(&serde_json::to_string(&list).unwrap()).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn times() {
        assert_eq!(parse_hhmm("07:30"), Some(450));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(new_id(10).len(), 10);
    }
}
