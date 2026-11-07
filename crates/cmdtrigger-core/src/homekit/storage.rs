//! HomeKit state on disk, in the layout hap-nodejs (the original agent's bridge) keeps under
//! `~/.TRIGGERcmdData/cache`: node-persist files named after their key, holding JSON.
//!
//! - `AccessoryInfo.<USERNAME>.json`: the bridge's long-term Ed25519 keys, paired controllers
//!   and configuration number;
//! - `IdentifierCache.<USERNAME>.json`: which accessory and characteristic IDs were handed out.
//!
//! Reading and writing the same files means a bridge paired with the original keeps its
//! pairing, room assignments and scenes here, and the other way round.

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// hap-nodejs's `uuid.generate`: a SHA-1 of the text laid out as a version-4-looking UUID.
pub fn generate_uuid(data: &str) -> String {
    let hex = hex::encode(Sha1::digest(data.as_bytes()));
    let digits: Vec<char> = hex.chars().collect();
    let mut i = 0;
    "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"
        .chars()
        .map(|c| match c {
            'x' => {
                let d = digits[i];
                i += 1;
                d
            }
            'y' => {
                let d = digits[i].to_digit(16).unwrap();
                i += 1;
                char::from_digit((d & 0x3) | 0x8, 16).unwrap()
            }
            other => other,
        })
        .collect()
}

fn key_name(kind: &str, username: &str) -> String {
    format!("{kind}.{}.json", username.replace(':', "").to_uppercase())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::store::write_atomic(path, serde_json::to_string(value).expect("serializable").as_bytes())
}

pub const PERMISSION_ADMIN: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Pairing {
    pub public_key: [u8; 32],
    pub permission: u8,
}

/// The bridge's identity and pairings.
pub struct AccessoryInfo {
    path: PathBuf,
    pub username: String,
    pub display_name: String,
    pub pincode: String,
    pub signing: SigningKey,
    pub paired: BTreeMap<String, Pairing>,
    pub config_version: u32,
    pub config_hash: String,
    pub setup_id: String,
    /// Fields other versions wrote that this one doesn't use, kept on save.
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct SavedInfo {
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    category: Value,
    #[serde(default)]
    pincode: String,
    #[serde(rename = "signSk", default)]
    sign_sk: String,
    #[serde(rename = "signPk", default)]
    sign_pk: String,
    #[serde(rename = "pairedClients", default)]
    paired_clients: BTreeMap<String, String>,
    #[serde(rename = "pairedClientsPermission", default)]
    paired_clients_permission: BTreeMap<String, u8>,
    #[serde(rename = "configVersion", default)]
    config_version: u32,
    #[serde(rename = "configHash", default)]
    config_hash: String,
    #[serde(rename = "setupID", default)]
    setup_id: String,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS has randomness");
    b
}

fn random_setup_id() -> String {
    const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    random_bytes::<4>().iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

impl AccessoryInfo {
    /// Load the bridge's identity for `username`, or create a new one.
    pub fn load_or_create(cache: &Path, username: &str, display_name: &str, pincode: &str) -> Self {
        let path = cache.join(key_name("AccessoryInfo", username));
        let saved = read_json(&path).and_then(|v| serde_json::from_value::<SavedInfo>(v).ok());
        let mut info = match saved.and_then(|s| Self::from_saved(path.clone(), username, s)) {
            Some(info) => info,
            None => Self {
                path,
                username: username.to_uppercase(),
                display_name: String::new(),
                pincode: String::new(),
                signing: SigningKey::from_bytes(&random_bytes::<32>()),
                paired: BTreeMap::new(),
                config_version: 1,
                config_hash: String::new(),
                setup_id: random_setup_id(),
                extra: Map::new(),
            },
        };
        info.display_name = display_name.to_string();
        info.pincode = pincode.to_string();
        if info.setup_id.is_empty() {
            info.setup_id = random_setup_id();
        }
        let _ = info.save();
        info
    }

    fn from_saved(path: PathBuf, username: &str, s: SavedInfo) -> Option<Self> {
        // tweetnacl's 64-byte secret key is seed || public key.
        let sk = hex::decode(&s.sign_sk).ok()?;
        let seed: [u8; 32] = sk.get(..32)?.try_into().ok()?;
        let mut paired = BTreeMap::new();
        for (id, pk_hex) in &s.paired_clients {
            let Ok(pk) = hex::decode(pk_hex) else { continue };
            let Ok(public_key) = <[u8; 32]>::try_from(pk.as_slice()) else { continue };
            let permission = s.paired_clients_permission.get(id).copied().unwrap_or(PERMISSION_ADMIN);
            paired.insert(id.clone(), Pairing { public_key, permission });
        }
        let mut extra = s.extra;
        extra.insert("category".into(), s.category);
        Some(Self {
            path,
            username: username.to_uppercase(),
            display_name: s.display_name,
            pincode: s.pincode,
            signing: SigningKey::from_bytes(&seed),
            paired,
            config_version: s.config_version.clamp(1, 0xFFFF),
            config_hash: s.config_hash,
            setup_id: s.setup_id,
            extra,
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        let mut saved = SavedInfo {
            display_name: self.display_name.clone(),
            category: Value::from(2), // Categories.BRIDGE
            pincode: self.pincode.clone(),
            sign_sk: hex::encode(self.signing.to_keypair_bytes()),
            sign_pk: hex::encode(self.signing.verifying_key().to_bytes()),
            paired_clients: BTreeMap::new(),
            paired_clients_permission: BTreeMap::new(),
            config_version: self.config_version,
            config_hash: self.config_hash.clone(),
            setup_id: self.setup_id.clone(),
            extra: self.extra.clone(),
        };
        saved.extra.remove("category");
        saved.extra.entry("lastFirmwareVersion").or_insert_with(|| Value::from(crate::VERSION));
        for (id, p) in &self.paired {
            saved.paired_clients.insert(id.clone(), hex::encode(p.public_key));
            saved.paired_clients_permission.insert(id.clone(), p.permission);
        }
        write_json(&self.path, &saved)
    }

    pub fn is_paired(&self) -> bool {
        !self.paired.is_empty()
    }

    pub fn has_admin(&self) -> bool {
        self.paired.values().any(|p| p.permission == PERMISSION_ADMIN)
    }

    /// Bump the configuration number (`c#`) when the accessory layout changed; wraps 65535 → 1.
    pub fn note_configuration(&mut self, hash: &str) -> bool {
        if self.config_hash == hash {
            return false;
        }
        self.config_hash = hash.to_string();
        self.config_version = if self.config_version >= 0xFFFF { 1 } else { self.config_version + 1 };
        let _ = self.save();
        true
    }

    /// The `sh` TXT record: the first 4 bytes of SHA-512(setup ID + username), base64.
    pub fn setup_hash(&self) -> String {
        use base64::Engine;
        use sha2::Sha512;
        let digest = <Sha512 as sha2::Digest>::digest(format!("{}{}", self.setup_id, self.username).as_bytes());
        base64::engine::general_purpose::STANDARD.encode(&digest[..4])
    }
}

/// hap-nodejs's IdentifierCache: stable accessory IDs (`aid`) per accessory UUID and instance
/// IDs (`iid`) per accessory/service/characteristic, with unused entries dropped on rebuild.
pub struct IdentifierCache {
    path: PathBuf,
    cache: BTreeMap<String, u64>,
    used: Option<BTreeMap<String, u64>>,
}

impl IdentifierCache {
    pub fn load(cache_dir: &Path, username: &str) -> Self {
        let path = cache_dir.join(key_name("IdentifierCache", username));
        let cache = read_json(&path)
            .and_then(|v| v.get("cache").cloned())
            .and_then(|v| serde_json::from_value::<BTreeMap<String, u64>>(v).ok())
            .unwrap_or_default();
        Self { path, cache, used: None }
    }

    pub fn start_tracking(&mut self) {
        self.used = Some(BTreeMap::new());
    }

    pub fn finish_tracking(&mut self) {
        if let Some(used) = self.used.take() {
            self.cache = used;
        }
    }

    fn get(&mut self, key: &str) -> Option<u64> {
        let value = self.cache.get(key).copied();
        if let (Some(used), Some(v)) = (self.used.as_mut(), value) {
            used.insert(key.to_string(), v);
        }
        value
    }

    fn set(&mut self, key: &str, value: u64) -> u64 {
        self.cache.insert(key.to_string(), value);
        if let Some(used) = self.used.as_mut() {
            used.insert(key.to_string(), value);
        }
        value
    }

    pub fn aid(&mut self, accessory_uuid: &str) -> u64 {
        self.get("|nextAID");
        if let Some(aid) = self.get(accessory_uuid) {
            return aid;
        }
        let next = self.get("|nextAID").unwrap_or(2);
        self.set("|nextAID", next + 1);
        self.set(accessory_uuid, next)
    }

    pub fn iid(
        &mut self,
        accessory_uuid: &str,
        service_uuid: &str,
        subtype: Option<&str>,
        characteristic_uuid: Option<&str>,
    ) -> u64 {
        let mut key = format!("{accessory_uuid}|{service_uuid}");
        if let Some(s) = subtype {
            key.push('|');
            key.push_str(s);
        }
        if let Some(c) = characteristic_uuid {
            key.push('|');
            key.push_str(c);
        }
        let next_key = format!("{accessory_uuid}|nextIID");
        self.get(&next_key);
        if let Some(iid) = self.get(&key) {
            return iid;
        }
        let next = self.get(&next_key).unwrap_or(2);
        self.set(&next_key, next + 1);
        self.set(&key, next)
    }

    pub fn save(&self) -> std::io::Result<()> {
        write_json(&self.path, &serde_json::json!({ "cache": self.cache }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_matches_hap_nodejs() {
        // node -e "console.log(require('hap-nodejs').uuid.generate('hello'))"
        assert_eq!(generate_uuid("hello"), "aaf4c61d-dcc5-4e8a-adab-ede0f3b482cd");
        assert_eq!(generate_uuid("triggercmd-homekit:Den PC Calculator"), "3123701e-c55d-4c72-b8fb-6445d781f5da");
    }

    #[test]
    fn identifiers_are_stable_and_expire() {
        let dir = tempfile::tempdir().unwrap();
        let mut ids = IdentifierCache::load(dir.path(), "AA:BB:CC:DD:EE:FF");
        ids.start_tracking();
        assert_eq!(ids.aid("a"), 2);
        assert_eq!(ids.aid("b"), 3);
        assert_eq!(ids.iid("a", "S", Some("x"), None), 2);
        assert_eq!(ids.iid("a", "S", Some("x"), Some("C")), 3);
        ids.finish_tracking();
        ids.save().unwrap();

        let mut again = IdentifierCache::load(dir.path(), "aa:bb:cc:dd:ee:ff");
        again.start_tracking();
        assert_eq!(again.aid("b"), 3);
        assert_eq!(again.aid("c"), 4);
        again.finish_tracking();
        assert!(!again.cache.contains_key("a"));
    }

    #[test]
    fn accessory_info_round_trips_in_hap_nodejs_format() {
        let dir = tempfile::tempdir().unwrap();
        let mut info = AccessoryInfo::load_or_create(dir.path(), "AA:BB:CC:DD:EE:FF", "Bridge", "111-22-333");
        info.paired.insert("ctrl".into(), Pairing { public_key: [5u8; 32], permission: 1 });
        info.save().unwrap();
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("AccessoryInfo.AABBCCDDEEFF.json")).unwrap())
                .unwrap();
        assert_eq!(raw["signSk"].as_str().unwrap().len(), 128);
        assert_eq!(raw["pairedClientsPermission"]["ctrl"], 1);
        let back = AccessoryInfo::load_or_create(dir.path(), "AA:BB:CC:DD:EE:FF", "Bridge", "111-22-333");
        assert_eq!(back.signing.verifying_key(), info.signing.verifying_key());
        assert_eq!(back.paired, info.paired);
        assert_eq!(back.setup_id, info.setup_id);
    }
}
