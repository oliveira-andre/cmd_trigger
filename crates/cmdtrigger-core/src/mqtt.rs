//! Home Assistant, through its MQTT integration: every command and automation appears in Home
//! Assistant on its own, as entities of a "<computer>" device. They show up on dashboards and
//! can be used in its automations and voice assistant.
//!
//! - every item: a **button** (press = run);
//! - commands with an off command: a **switch** (on/off run the command / off command);
//! - items that take parameters: a **text** entity (setting it runs with that text).
//!
//! Discovery configs are retained, so Home Assistant keeps the entities across restarts; when
//! an item is removed, its config is cleared. The agent's availability topic goes "offline"
//! through the MQTT last will when it stops.

use crate::agent::Ground;
use crate::config::MqttConfig;
use crate::runner::Runner;
use cmdtrigger_protocol::{CatalogEntry, TargetKind, slug};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MqttStatus {
    pub connected: bool,
    pub error: Option<String>,
    /// Entities published.
    pub entities: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Press,
    Set,
    Text,
}

/// What gets published for one computer's agent.
pub struct Layout {
    prefix: String,
    base: String,
    device_id: String,
    computer: String,
    availability: String,
}

impl Layout {
    pub fn new(discovery_prefix: &str, computer: &str, ground: Ground) -> Self {
        let pc = slug(computer);
        let base = format!("cmdtrigger/{pc}");
        Self {
            prefix: if discovery_prefix.trim().is_empty() {
                "homeassistant".into()
            } else {
                discovery_prefix.trim().into()
            },
            availability: format!("{base}/{}/status", ground.as_str()),
            base,
            device_id: format!("cmdtrigger_{pc}"),
            computer: computer.to_string(),
        }
    }

    fn object(entry: &CatalogEntry) -> String {
        match entry.kind {
            TargetKind::Command => slug(&entry.name),
            TargetKind::Automation => format!("automation_{}", slug(&entry.id)),
        }
    }

    fn device(&self) -> Value {
        json!({
            "identifiers": [self.device_id],
            "name": self.computer,
            "manufacturer": "CmdTrigger",
            "model": "CmdTrigger agent",
            "sw_version": crate::VERSION,
        })
    }

    /// (config topic, config payload, command topic, action) for every entity of `entries`.
    pub fn entities(&self, entries: &[CatalogEntry]) -> Vec<(String, Value, String, CatalogEntry, &'static str)> {
        let mut out = Vec::new();
        for e in entries {
            let obj = Self::object(e);
            let icon = if e.kind == TargetKind::Automation { "mdi:robot" } else { "mdi:console" };
            let common = |name: String, unique: String, command_topic: &str| {
                json!({
                    "name": name,
                    "unique_id": unique,
                    "object_id": unique,
                    "command_topic": command_topic,
                    "availability_topic": self.availability,
                    "device": self.device(),
                    "icon": icon,
                })
            };
            let press = format!("{}/{obj}/press", self.base);
            let mut button = common(e.name.clone(), format!("{}_{obj}", self.device_id), &press);
            button["payload_press"] = json!("PRESS");
            out.push((
                format!("{}/button/{}/{obj}/config", self.prefix, self.device_id),
                button,
                press,
                e.clone(),
                "press",
            ));
            if e.kind == TargetKind::Command && e.has_off {
                let set = format!("{}/{obj}/set", self.base);
                let mut switch = common(format!("{} switch", e.name), format!("{}_{obj}_switch", self.device_id), &set);
                switch["payload_on"] = json!("ON");
                switch["payload_off"] = json!("OFF");
                switch["optimistic"] = json!(true);
                out.push((
                    format!("{}/switch/{}/{obj}/config", self.prefix, self.device_id),
                    switch,
                    set,
                    e.clone(),
                    "set",
                ));
            }
            if e.allow_params {
                let run = format!("{}/{obj}/run", self.base);
                let mut text = common(format!("{} with text", e.name), format!("{}_{obj}_text", self.device_id), &run);
                text["mode"] = json!("text");
                text["max"] = json!(255);
                out.push((
                    format!("{}/text/{}/{obj}_text/config", self.prefix, self.device_id),
                    text,
                    run,
                    e.clone(),
                    "text",
                ));
            }
        }
        out
    }
}

pub struct Mqtt {
    cancel: CancellationToken,
    status: watch::Receiver<MqttStatus>,
}

impl Mqtt {
    /// Connect when enabled. `changes` fires when the commands or automations change.
    pub fn start(
        config: MqttConfig,
        computer: String,
        runner: Arc<Runner>,
        changes: broadcast::Receiver<()>,
    ) -> Option<Self> {
        if !config.enabled || config.host.trim().is_empty() {
            return None;
        }
        let cancel = CancellationToken::new();
        let (tx, status) = watch::channel(MqttStatus::default());
        tokio::spawn(run(config, computer, runner, changes, tx, cancel.clone()));
        Some(Self { cancel, status })
    }

    pub fn status(&self) -> MqttStatus {
        self.status.borrow().clone()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Mqtt {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn run(
    config: MqttConfig,
    computer: String,
    runner: Arc<Runner>,
    mut changes: broadcast::Receiver<()>,
    status: watch::Sender<MqttStatus>,
    cancel: CancellationToken,
) {
    let ground = runner.ground();
    let layout = Layout::new(&config.discovery_prefix, &computer, ground);
    let client_id = format!("cmdtrigger-{}-{}-{}", slug(&computer), ground.as_str(), crate::automation::new_id(6));
    let mut options = MqttOptions::new(client_id, config.host.trim(), config.port);
    options.set_keep_alive(Duration::from_secs(30));
    options.set_last_will(LastWill::new(&layout.availability, "offline", QoS::AtLeastOnce, true));
    if !config.username.is_empty() {
        options.set_credentials(config.username.clone(), config.password.clone());
    }
    options.set_max_packet_size(256 * 1024, 256 * 1024);
    let (client, mut events) = AsyncClient::new(options, 64);
    let mut routes: HashMap<String, (CatalogEntry, Action)> = HashMap::new();
    let mut published: Vec<String> = Vec::new();

    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = client.publish(&layout.availability, QoS::AtLeastOnce, true, "offline").await;
                let _ = client.disconnect().await;
                // Let the last messages go out.
                let _ = tokio::time::timeout(Duration::from_millis(500), events.poll()).await;
                return;
            }
            changed = changes.recv() => {
                if matches!(changed, Err(broadcast::error::RecvError::Closed)) { return }
                if status.borrow().connected {
                    publish_all(&client, &layout, &runner, &mut routes, &mut published, &status).await;
                }
                continue;
            }
            event = events.poll() => event,
        };
        match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("Connected to MQTT at {}:{}", config.host, config.port);
                status.send_modify(|s| {
                    s.connected = true;
                    s.error = None;
                });
                let _ = client.publish(&layout.availability, QoS::AtLeastOnce, true, "online").await;
                for suffix in ["press", "set", "run"] {
                    let _ = client.subscribe(format!("{}/+/{suffix}", layout.base), QoS::AtLeastOnce).await;
                }
                // Retained configs from earlier runs: anything we no longer publish gets cleared.
                let _ = client
                    .subscribe(format!("{}/+/{}/+/config", layout.prefix, layout.device_id), QoS::AtLeastOnce)
                    .await;
                publish_all(&client, &layout, &runner, &mut routes, &mut published, &status).await;
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let payload = String::from_utf8_lossy(&p.payload).to_string();
                if p.topic.ends_with("/config") {
                    let ours = serde_json::from_str::<Value>(&payload).ok().is_some_and(|v| {
                        v.get("availability_topic").and_then(Value::as_str) == Some(layout.availability.as_str())
                    });
                    if ours && !published.contains(&p.topic) {
                        tracing::info!("Removing stale Home Assistant entity {}", p.topic);
                        let _ = client.publish(p.topic.clone(), QoS::AtLeastOnce, true, Vec::new()).await;
                    }
                    continue;
                }
                let Some((entry, action)) = routes.get(&p.topic).cloned() else { continue };
                let params = match action {
                    Action::Press => None,
                    Action::Set => Some(if payload.eq_ignore_ascii_case("OFF") { "off" } else { "on" }.to_string()),
                    Action::Text => Some(payload.clone()).filter(|t| !t.trim().is_empty()),
                };
                let runner = runner.clone();
                tokio::spawn(async move {
                    runner.run(entry.kind, &entry.id, params, "mqtt").await;
                });
            }
            Ok(_) => {}
            Err(e) => {
                let message = e.to_string();
                if status.borrow().error.as_deref() != Some(message.as_str()) {
                    tracing::warn!("MQTT: {message}");
                }
                status.send_modify(|s| {
                    s.connected = false;
                    s.error = Some(message);
                });
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
            }
        }
    }
}

async fn publish_all(
    client: &AsyncClient,
    layout: &Layout,
    runner: &Runner,
    routes: &mut HashMap<String, (CatalogEntry, Action)>,
    published: &mut Vec<String>,
    status: &watch::Sender<MqttStatus>,
) {
    let entities = layout.entities(&runner.catalog());
    let now: Vec<String> = entities.iter().map(|(topic, ..)| topic.clone()).collect();
    for old in published.iter().filter(|t| !now.contains(t)) {
        let _ = client.publish(old.clone(), QoS::AtLeastOnce, true, Vec::new()).await;
    }
    routes.clear();
    for (topic, config, command_topic, entry, kind) in &entities {
        let _ = client.publish(topic.clone(), QoS::AtLeastOnce, true, config.to_string()).await;
        let action = match *kind {
            "set" => Action::Set,
            "text" => Action::Text,
            _ => Action::Press,
        };
        routes.insert(command_topic.clone(), (entry.clone(), action));
    }
    *published = now;
    status.send_modify(|s| s.entities = entities.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: TargetKind, name: &str, has_off: bool, allow_params: bool) -> CatalogEntry {
        CatalogEntry {
            kind,
            id: if kind == TargetKind::Command { name.into() } else { "a1b2".into() },
            name: name.into(),
            icon: String::new(),
            description: String::new(),
            allow_params,
            has_off,
            ground: "foreground".into(),
        }
    }

    #[test]
    fn discovery_layout() {
        let layout = Layout::new("homeassistant", "Den PC", Ground::Foreground);
        let entities = layout.entities(&[
            entry(TargetKind::Command, "Living room TV", true, true),
            entry(TargetKind::Automation, "Morning", false, false),
        ]);
        let topics: Vec<&str> = entities.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(
            topics,
            [
                "homeassistant/button/cmdtrigger_den_pc/living_room_tv/config",
                "homeassistant/switch/cmdtrigger_den_pc/living_room_tv/config",
                "homeassistant/text/cmdtrigger_den_pc/living_room_tv_text/config",
                "homeassistant/button/cmdtrigger_den_pc/automation_a1b2/config",
            ]
        );
        let (_, button, press, ..) = &entities[0];
        assert_eq!(press, "cmdtrigger/den_pc/living_room_tv/press");
        assert_eq!(button["availability_topic"], "cmdtrigger/den_pc/foreground/status");
        assert_eq!(button["device"]["identifiers"][0], "cmdtrigger_den_pc");
        assert_eq!(entities[1].1["payload_off"], "OFF");
    }
}
