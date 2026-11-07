//! Advertising the bridge as `_hap._tcp` so the Home app finds it.

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub const SERVICE_TYPE: &str = "_hap._tcp.local.";

pub struct Txt {
    pub config_number: u32,
    pub device_id: String,
    pub model: String,
    pub paired: bool,
    pub setup_hash: String,
}

pub struct Advertiser {
    daemon: ServiceDaemon,
    instance: String,
    host: String,
    port: u16,
    fullname: Option<String>,
}

fn host_label(name: &str, device_id: &str) -> String {
    let mut label: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if label.is_empty() {
        label = "CmdTrigger".into();
    }
    label.truncate(40);
    format!("{label}-{}.local.", device_id.replace(':', ""))
}

impl Advertiser {
    pub fn new(instance: &str, device_id: &str, port: u16) -> Result<Self, mdns_sd::Error> {
        let mut instance: String = instance.chars().filter(|c| *c != '.').collect();
        while instance.len() > 63 {
            instance.pop();
        }
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            host: host_label(&instance, device_id),
            instance,
            port,
            fullname: None,
        })
    }

    /// Publish, or re-publish with new TXT records (same name).
    pub fn publish(&mut self, txt: &Txt) -> Result<(), mdns_sd::Error> {
        let c = txt.config_number.to_string();
        let sf = if txt.paired { "0" } else { "1" };
        let properties = [
            ("c#", c.as_str()),
            ("ff", "0"),
            ("id", txt.device_id.as_str()),
            ("md", txt.model.as_str()),
            ("pv", "1.1"),
            ("s#", "1"),
            ("sf", sf),
            ("ci", "2"),
            ("sh", txt.setup_hash.as_str()),
        ];
        let info = ServiceInfo::new(SERVICE_TYPE, &self.instance, &self.host, "", self.port, &properties[..])?
            .enable_addr_auto();
        self.fullname = Some(info.get_fullname().to_string());
        self.daemon.register(info)
    }

    pub fn shutdown(self) {
        if let Some(name) = &self.fullname {
            let _ = self.daemon.unregister(name);
        }
        let _ = self.daemon.shutdown();
    }
}
