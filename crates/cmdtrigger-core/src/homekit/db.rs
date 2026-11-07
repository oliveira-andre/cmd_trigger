//! The accessory database: the bridge, then one light per command and automation.
//!
//! Each light is named "<computer> <name>" and has an On switch and a Brightness slider.
//! Accessory IDs are kept per light UUID in the [`IdentifierCache`], so rooms and scenes in
//! the Home app survive edits to the command list.

use super::storage::{IdentifierCache, generate_uuid};
use cmdtrigger_protocol::{CatalogEntry, TargetKind};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

const BASE: &str = "-0000-1000-8000-0026BB765291";

fn long_uuid(short: &str) -> String {
    format!("{:0>8}{BASE}", short)
}

pub const ACCESSORY_INFORMATION: &str = "3E";
pub const PROTOCOL_INFORMATION: &str = "A2";
pub const LIGHTBULB: &str = "43";

pub const IDENTIFY: &str = "14";
pub const MANUFACTURER: &str = "20";
pub const MODEL: &str = "21";
pub const NAME: &str = "23";
pub const SERIAL_NUMBER: &str = "30";
pub const FIRMWARE_REVISION: &str = "52";
pub const VERSION: &str = "37";
pub const ON: &str = "25";
pub const BRIGHTNESS: &str = "8";

/// The bridge may expose at most 149 accessories besides itself (HAP spec).
pub const MAX_BRIDGED: usize = 149;

/// What writing to a characteristic does.
#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    Static,
    Identify,
    /// Index into [`Database::lights`].
    On(usize),
    Brightness(usize),
}

#[derive(Clone, Debug)]
pub struct Characteristic {
    pub iid: u64,
    pub kind: &'static str,
    pub format: &'static str,
    pub perms: &'static [&'static str],
    pub value: Value,
    pub role: Role,
}

impl Characteristic {
    pub fn readable(&self) -> bool {
        self.perms.contains(&"pr")
    }
    pub fn writable(&self) -> bool {
        self.perms.contains(&"pw")
    }
    pub fn notifies(&self) -> bool {
        self.perms.contains(&"ev")
    }

    fn to_json(&self) -> Value {
        let mut c = json!({ "iid": self.iid, "type": self.kind, "perms": self.perms, "format": self.format });
        if self.readable() {
            c["value"] = self.value.clone();
        }
        if self.kind == BRIGHTNESS {
            c["unit"] = json!("percentage");
            c["minValue"] = json!(0);
            c["maxValue"] = json!(100);
            c["minStep"] = json!(1);
        }
        c
    }
}

#[derive(Clone, Debug)]
pub struct Service {
    pub iid: u64,
    pub kind: &'static str,
    pub primary: bool,
    pub characteristics: Vec<Characteristic>,
}

#[derive(Clone, Debug)]
pub struct Accessory {
    pub aid: u64,
    pub name: String,
    pub services: Vec<Service>,
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    pub accessories: Vec<Accessory>,
    /// The command or automation behind each light, in light order.
    pub lights: Vec<CatalogEntry>,
}

/// The light's resting state: off at full brightness. Every write snaps back to it, so a light
/// works like a button.
pub const RESTING_ON: bool = false;
pub const RESTING_BRIGHTNESS: u8 = 100;

struct Info<'a> {
    manufacturer: &'a str,
    model: &'a str,
    name: &'a str,
    serial: &'a str,
}

fn information(ids: &mut IdentifierCache, uuid: &str, info: Info) -> Service {
    let svc = long_uuid(ACCESSORY_INFORMATION);
    let mut iid = |kind: &str| ids.iid(uuid, &svc, None, Some(&long_uuid(kind)));
    let text = |kind: &'static str, iid: u64, value: &str| Characteristic {
        iid,
        kind,
        format: "string",
        perms: &["pr"],
        value: json!(value),
        role: Role::Static,
    };
    let characteristics = vec![
        Characteristic {
            iid: iid(IDENTIFY),
            kind: IDENTIFY,
            format: "bool",
            perms: &["pw"],
            value: Value::Null,
            role: Role::Identify,
        },
        text(MANUFACTURER, iid(MANUFACTURER), info.manufacturer),
        text(MODEL, iid(MODEL), info.model),
        text(NAME, iid(NAME), info.name),
        text(SERIAL_NUMBER, iid(SERIAL_NUMBER), info.serial),
        text(FIRMWARE_REVISION, iid(FIRMWARE_REVISION), crate::VERSION),
    ];
    Service { iid: 1, kind: ACCESSORY_INFORMATION, primary: false, characteristics }
}

/// "Den PC Calculator".
pub fn light_name(computer_name: &str, name: &str) -> String {
    let name = if name.trim().is_empty() { "Command" } else { name.trim() };
    format!("{} {name}", computer_name.trim())
}

pub fn build(
    ids: &mut IdentifierCache,
    username: &str,
    bridge_name: &str,
    computer_name: &str,
    entries: &[CatalogEntry],
) -> Database {
    ids.start_tracking();
    let bridge_uuid = generate_uuid(&format!("cmdtrigger-bridge:{username}"));
    let mut accessories = Vec::new();

    // Information first: hap-nodejs hands out IDs in service order.
    let bridge_info = information(
        ids,
        &bridge_uuid,
        Info { manufacturer: "CmdTrigger", model: "CmdTrigger Bridge", name: bridge_name, serial: username },
    );
    let protocol_svc = long_uuid(PROTOCOL_INFORMATION);
    let protocol = Service {
        iid: 2_000_000_000 + ids.iid(&bridge_uuid, &protocol_svc, None, None),
        kind: PROTOCOL_INFORMATION,
        primary: false,
        characteristics: vec![Characteristic {
            iid: ids.iid(&bridge_uuid, &protocol_svc, None, Some(&long_uuid(VERSION))),
            kind: VERSION,
            format: "string",
            perms: &["pr", "ev"],
            value: json!("1.1.0"),
            role: Role::Static,
        }],
    };
    accessories.push(Accessory { aid: 1, name: bridge_name.to_string(), services: vec![bridge_info, protocol] });

    let mut lights: Vec<CatalogEntry> = Vec::new();
    for entry in entries {
        if entry.name.trim().is_empty() || lights.iter().any(|l| l.kind == entry.kind && l.id == entry.id) {
            continue;
        }
        if lights.len() == MAX_BRIDGED {
            tracing::warn!("[HomeKit] A bridge holds at most {MAX_BRIDGED} accessories; the rest are left out.");
            break;
        }
        let index = lights.len();
        let name = light_name(computer_name, &entry.name);
        let seed = match entry.kind {
            TargetKind::Command => format!("cmdtrigger:command:{}", entry.id),
            TargetKind::Automation => format!("cmdtrigger:automation:{}", entry.id),
        };
        let uuid = generate_uuid(&seed);
        let aid = ids.aid(&uuid);
        let info = information(
            ids,
            &uuid,
            Info {
                manufacturer: "CmdTrigger",
                model: if entry.kind == TargetKind::Command { "Command" } else { "Automation" },
                name: &name,
                serial: &uuid[..12],
            },
        );
        let svc = long_uuid(LIGHTBULB);
        let sub = Some("lightbulb");
        let light = Service {
            iid: ids.iid(&uuid, &svc, sub, None),
            kind: LIGHTBULB,
            primary: true,
            characteristics: vec![
                Characteristic {
                    iid: ids.iid(&uuid, &svc, sub, Some(&long_uuid(NAME))),
                    kind: NAME,
                    format: "string",
                    perms: &["pr"],
                    value: json!(name),
                    role: Role::Static,
                },
                Characteristic {
                    iid: ids.iid(&uuid, &svc, sub, Some(&long_uuid(ON))),
                    kind: ON,
                    format: "bool",
                    perms: &["pr", "pw", "ev"],
                    value: json!(RESTING_ON),
                    role: Role::On(index),
                },
                Characteristic {
                    iid: ids.iid(&uuid, &svc, sub, Some(&long_uuid(BRIGHTNESS))),
                    kind: BRIGHTNESS,
                    format: "int",
                    perms: &["pr", "pw", "ev"],
                    value: json!(RESTING_BRIGHTNESS),
                    role: Role::Brightness(index),
                },
            ],
        };
        accessories.push(Accessory { aid, name, services: vec![info, light] });
        lights.push(entry.clone());
    }
    ids.finish_tracking();
    if let Err(e) = ids.save() {
        tracing::warn!("[HomeKit] Could not save the identifier cache: {e}");
    }
    Database { accessories, lights }
}

impl Database {
    pub fn to_json(&self) -> Value {
        let accessories: Vec<Value> = self
            .accessories
            .iter()
            .map(|a| {
                let services: Vec<Value> = a
                    .services
                    .iter()
                    .map(|s| {
                        let mut v = json!({
                            "iid": s.iid,
                            "type": s.kind,
                            "characteristics": s.characteristics.iter().map(Characteristic::to_json).collect::<Vec<_>>(),
                        });
                        if s.primary {
                            v["primary"] = json!(true);
                        }
                        v
                    })
                    .collect();
                json!({ "aid": a.aid, "services": services })
            })
            .collect();
        json!({ "accessories": accessories })
    }

    pub fn find(&self, aid: u64, iid: u64) -> Option<&Characteristic> {
        self.accessories
            .iter()
            .find(|a| a.aid == aid)?
            .services
            .iter()
            .flat_map(|s| s.characteristics.iter())
            .find(|c| c.iid == iid)
    }

    /// The (aid, iid) of a light's On and Brightness characteristics.
    pub fn light_ids(&self, index: usize) -> Option<(u64, u64, u64)> {
        let accessory = self.accessories.get(index + 1)?;
        let find = |role: Role| {
            accessory.services.iter().flat_map(|s| &s.characteristics).find(|c| c.role == role).map(|c| c.iid)
        };
        Some((accessory.aid, find(Role::On(index))?, find(Role::Brightness(index))?))
    }

    /// A digest of the layout (not the values). It changes when lights come or go, which is
    /// when HomeKit needs a new configuration number.
    pub fn layout_hash(&self) -> String {
        let mut hasher = Sha1::new();
        for a in &self.accessories {
            hasher.update(format!("{}:{};", a.aid, a.name));
            for s in &a.services {
                hasher.update(format!("{}:{};", s.iid, s.kind));
                for c in &s.characteristics {
                    hasher.update(format!(
                        "{}:{}:{};",
                        c.iid,
                        c.kind,
                        if c.role == Role::Static { c.value.to_string() } else { String::new() }
                    ));
                }
            }
        }
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: TargetKind, id: &str) -> CatalogEntry {
        CatalogEntry {
            kind,
            id: id.into(),
            name: id.into(),
            icon: String::new(),
            description: String::new(),
            allow_params: false,
            has_off: false,
            ground: "foreground".into(),
        }
    }

    #[test]
    fn ids_are_assigned_in_order_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut ids = IdentifierCache::load(dir.path(), "AA:BB:CC:DD:EE:FF");
        let all = vec![
            entry(TargetKind::Command, "Calculator"),
            entry(TargetKind::Command, "Calculator"),
            entry(TargetKind::Command, "Lamp"),
            entry(TargetKind::Automation, "morning"),
        ];
        let db = build(&mut ids, "AA:BB:CC:DD:EE:FF", "CmdTrigger Den", "Den", &all);
        assert_eq!(db.accessories.len(), 4);
        assert_eq!(db.lights.len(), 3);
        let calc = &db.accessories[1];
        assert_eq!((calc.aid, calc.name.as_str()), (2, "Den Calculator"));
        // AccessoryInformation: 1, then Identify..FirmwareRevision 2..7; Lightbulb 8, Name 9, On 10, Brightness 11.
        assert_eq!(calc.services[0].characteristics.iter().map(|c| c.iid).collect::<Vec<_>>(), vec![2, 3, 4, 5, 6, 7]);
        assert_eq!(calc.services[1].characteristics.iter().map(|c| c.iid).collect::<Vec<_>>(), vec![9, 10, 11]);
        assert_eq!(db.light_ids(1), Some((3, 10, 11)));
        assert_eq!(db.to_json()["accessories"][1]["services"][1]["characteristics"][1]["type"], "25");

        // Removing Calculator keeps the others' accessory IDs.
        let before = db.layout_hash();
        let db2 = build(&mut ids, "AA:BB:CC:DD:EE:FF", "CmdTrigger Den", "Den", &all[2..]);
        assert_eq!(db2.accessories[1].aid, 3);
        assert_eq!(db2.accessories[2].aid, 4);
        assert_ne!(before, db2.layout_hash());
    }
}
