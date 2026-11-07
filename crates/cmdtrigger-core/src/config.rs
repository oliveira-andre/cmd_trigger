//! The settings files in the data folder:
//! - `settings.json`: the computer's display name and the language;
//! - `homekit.json`: the HomeKit bridge;
//! - `mqtt.json`: the Home Assistant connection (owner-only, holds a password);
//! - `api.json`: the local HTTP API and its key (owner-only);
//! - `server.json`: the link to your CmdTrigger server and this computer's agent token
//!   (owner-only).

use crate::paths::{DataDir, hostname};
use crate::store::{read_json, to_json_2, write_atomic, write_private};
use serde::{Deserialize, Serialize};

// ---------- general ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Shown in HomeKit, Home Assistant and on the server. Blank = the hostname.
    pub computer_name: String,
    /// "en", "es", "pt", "de", "fr", "jp", "it". Blank = the system language.
    pub language: String,
}

impl Settings {
    pub fn load(data: &DataDir) -> Self {
        read_json(&data.settings())
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        write_atomic(&data.settings(), to_json_2(self).as_bytes())
    }

    pub fn computer_name(&self) -> String {
        let name = self.computer_name.trim();
        if name.is_empty() { hostname() } else { name.to_string() }
    }
}

pub use crate::i18n::Lang;

/// The saved language, or the system's (saved on first run).
pub fn load_language(data: &DataDir) -> Lang {
    let mut settings = Settings::load(data);
    if !settings.language.is_empty() {
        return Lang::from_code(&settings.language);
    }
    let detected = sys_locale::get_locale().map(|l| Lang::from_code(&l[..l.len().min(2)])).unwrap_or(Lang::En);
    settings.language = detected.code().into();
    let _ = settings.save(data);
    detected
}

pub fn save_language(data: &DataDir, lang: Lang) -> std::io::Result<()> {
    let mut settings = Settings::load(data);
    settings.language = lang.code().into();
    settings.save(data)
}

// ---------- HomeKit ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeKitConfig {
    pub enabled: bool,
    pub bridge_name: String,
    pub pin: String,
    pub username: String,
    pub debug_logging: bool,
}

/// Codes HomeKit refuses (HAP spec 5.6.1).
const TRIVIAL_PINS: [&str; 12] = [
    "00000000", "11111111", "22222222", "33333333", "44444444", "55555555", "66666666", "77777777", "88888888",
    "99999999", "12345678", "87654321",
];

pub fn random_pin() -> String {
    loop {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).expect("the OS has randomness");
        let digits = format!("{:08}", u32::from_le_bytes(b) % 100_000_000);
        if !TRIVIAL_PINS.contains(&digits.as_str()) {
            return format!("{}-{}-{}", &digits[..3], &digits[3..5], &digits[5..]);
        }
    }
}

pub fn random_username() -> String {
    let mut b = [0u8; 6];
    getrandom::fill(&mut b).expect("the OS has randomness");
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":")
}

/// A PIN HomeKit accepts: `XXX-XX-XXX`, not a trivial sequence.
pub fn valid_pin(pin: &str) -> bool {
    let b = pin.as_bytes();
    let shaped = b.len() == 10
        && b[3] == b'-'
        && b[6] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 3 || i == 6 || c.is_ascii_digit());
    shaped && !TRIVIAL_PINS.contains(&pin.replace('-', "").as_str())
}

/// `XX:XX:XX:XX:XX:XX` in hex.
pub fn valid_username(username: &str) -> bool {
    let parts: Vec<&str> = username.split(':').collect();
    parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

impl HomeKitConfig {
    pub fn defaults(data: &DataDir) -> Self {
        Self {
            enabled: false,
            bridge_name: format!("CmdTrigger {}", Settings::load(data).computer_name()),
            pin: random_pin(),
            username: random_username(),
            debug_logging: false,
        }
    }

    /// Read, fixing anything missing or invalid, and write back (a new bridge gets its PIN
    /// and identity on first load and keeps them).
    pub fn load(data: &DataDir) -> Self {
        let defaults = Self::defaults(data);
        let stored: Option<serde_json::Value> =
            std::fs::read_to_string(data.homekit()).ok().and_then(|t| serde_json::from_str(&t).ok());
        let get = |k: &str| stored.as_ref().and_then(|v| v.get(k)).cloned();
        let text = |k: &str| get(k).and_then(|v| v.as_str().map(str::to_string));
        let config = Self {
            enabled: get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
            bridge_name: text("bridgeName").filter(|n| !n.trim().is_empty()).unwrap_or(defaults.bridge_name),
            pin: text("pin").filter(|p| valid_pin(p)).unwrap_or(defaults.pin),
            username: text("username")
                .filter(|u| valid_username(u))
                .map(|u| u.to_uppercase())
                .unwrap_or(defaults.username),
            debug_logging: get("debugLogging").and_then(|v| v.as_bool()).unwrap_or(false),
        };
        if stored.as_ref() != Some(&serde_json::to_value(&config).unwrap()) {
            let _ = config.save(data);
        }
        config
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        write_atomic(&data.homekit(), to_json_2(self).as_bytes())
    }
}

// ---------- Home Assistant (MQTT) ----------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Home Assistant's discovery prefix.
    pub discovery_prefix: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "homeassistant.local".into(),
            port: 1883,
            username: String::new(),
            password: String::new(),
            discovery_prefix: "homeassistant".into(),
        }
    }
}

impl MqttConfig {
    pub fn load(data: &DataDir) -> Self {
        read_json(&data.mqtt())
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        write_private(&data.mqtt(), to_json_2(self).as_bytes())
    }
}

// ---------- local HTTP API ----------

pub const DEFAULT_API_PORT: u16 = 7171;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiConfig {
    pub enabled: bool,
    /// The tray app's port; the background service uses the next one.
    pub port: u16,
    /// Listen on the network, not just this computer.
    pub allow_lan: bool,
    pub key: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self { enabled: true, port: DEFAULT_API_PORT, allow_lan: false, key: String::new() }
    }
}

pub fn new_key() -> String {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).expect("the OS has randomness");
    format!("ct_{}", hex::encode(b))
}

impl ApiConfig {
    /// Read, creating the file (with a fresh key) on first use.
    pub fn load(data: &DataDir) -> Self {
        let exists = data.local_api().exists();
        let mut config: Self = read_json(&data.local_api());
        if config.key.len() < 16 || config.port == 0 || !exists {
            if config.key.len() < 16 {
                config.key = new_key();
            }
            if config.port == 0 {
                config.port = DEFAULT_API_PORT;
            }
            let _ = config.save(data);
        }
        config
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        write_private(&data.local_api(), to_json_2(self).as_bytes())
    }

    pub fn port_for(&self, ground: crate::Ground) -> u16 {
        match ground {
            crate::Ground::Foreground => self.port,
            crate::Ground::Background => self.port.saturating_add(1),
        }
    }
}

// ---------- the CmdTrigger server ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    pub url: String,
    pub email: String,
    pub computer_id: String,
    pub agent_token: String,
}

impl ServerConfig {
    pub fn load(data: &DataDir) -> Self {
        read_json(&data.server())
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        write_private(&data.server(), to_json_2(self).as_bytes())
    }

    pub fn connected(&self) -> bool {
        !self.url.is_empty() && !self.agent_token.is_empty()
    }

    pub fn clear(data: &DataDir) -> std::io::Result<()> {
        match std::fs::remove_file(data.server()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_and_usernames() {
        for _ in 0..50 {
            assert!(valid_pin(&random_pin()));
            assert!(valid_username(&random_username()));
        }
        assert!(!valid_pin("123-45-678"));
        assert!(valid_pin("031-45-154"));
    }

    #[test]
    fn homekit_keeps_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        Settings { computer_name: "Den PC".into(), language: String::new() }.save(&data).unwrap();
        let first = HomeKitConfig::load(&data);
        assert_eq!(first.bridge_name, "CmdTrigger Den PC");
        assert_eq!(HomeKitConfig::load(&data), first);
        std::fs::write(data.homekit(), r#"{"enabled": true, "pin": "111-11-111", "username": "aa:bb:cc:dd:ee:ff"}"#)
            .unwrap();
        let fixed = HomeKitConfig::load(&data);
        assert!(fixed.enabled && valid_pin(&fixed.pin));
        assert_eq!(fixed.username, "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn api_key_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        let first = ApiConfig::load(&data);
        assert!(first.key.starts_with("ct_") && first.enabled);
        assert_eq!(ApiConfig::load(&data).key, first.key);
        assert_eq!(first.port_for(crate::Ground::Background), DEFAULT_API_PORT + 1);
    }
}
