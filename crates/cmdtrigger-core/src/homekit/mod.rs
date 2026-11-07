//! The HomeKit bridge: every command and automation becomes a light in the Home app (and so
//! works with Siri: "Hey Siri, turn on Calculator"). Turning a light on runs it, turning a
//! command's light off runs its off command, and with parameters allowed the brightness is
//! passed along. The light then snaps back to off, like a button.
//!
//! A native HAP implementation: SRP pair setup, X25519/Ed25519 pair verify, ChaCha20-Poly1305
//! sessions, mDNS advertisement, on port 47129.

pub mod crypto;
pub mod db;
pub mod mdns;
pub mod server;
pub mod srp;
pub mod storage;
pub mod tlv;

use crate::config::{HomeKitConfig, Settings};
use crate::paths::DataDir;
use cmdtrigger_protocol::CatalogEntry;
use server::{Runner, Shared};
use std::sync::Arc;
use tokio::sync::broadcast;

pub const PORT: u16 = 47129;

/// The lights to publish, read again after each change.
pub type Catalog = Arc<dyn Fn() -> Vec<CatalogEntry> + Send + Sync>;

/// What the HomeKit settings window shows.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BridgeStatus {
    pub running: bool,
    pub paired: bool,
    pub controllers: usize,
    pub lights: usize,
    pub port: u16,
    pub error: Option<String>,
}

pub struct HomeKitBridge {
    shared: Option<Arc<Shared>>,
    error: Option<String>,
    port: u16,
}

fn computer_name(data: &DataDir) -> String {
    Settings::load(data).computer_name()
}

fn txt_for(shared: &Shared, model: &str) -> mdns::Txt {
    let info = shared.info.lock().unwrap();
    mdns::Txt {
        config_number: info.config_version,
        device_id: info.username.clone(),
        model: model.to_string(),
        paired: info.is_paired(),
        setup_hash: info.setup_hash(),
    }
}

impl HomeKitBridge {
    pub fn disabled() -> Self {
        Self { shared: None, error: None, port: PORT }
    }

    /// Start the bridge when the settings enable it. `changes` fires when the commands or
    /// automations change, to rebuild the lights from `catalog`.
    pub async fn start(
        data: DataDir,
        catalog: Catalog,
        mut changes: broadcast::Receiver<()>,
        port: u16,
        runner: Runner,
    ) -> Self {
        let config = HomeKitConfig::load(&data);
        if !config.enabled {
            tracing::info!("HomeKit bridge is disabled.");
            return Self { shared: None, error: None, port };
        }
        let cache = data.homekit_cache();
        if let Err(e) = std::fs::create_dir_all(&cache) {
            return Self { shared: None, error: Some(format!("cannot create {}: {e}", cache.display())), port };
        }
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(l) => l,
            Err(e) => {
                let message = format!("port {port} is unavailable ({e}); is another HomeKit bridge running?");
                tracing::warn!("HomeKit bridge not started: {message}");
                return Self { shared: None, error: Some(message), port };
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);

        let mut info =
            storage::AccessoryInfo::load_or_create(&cache, &config.username, &config.bridge_name, &config.pin);
        let mut ids = storage::IdentifierCache::load(&cache, &config.username);
        let built = db::build(&mut ids, &info.username, &config.bridge_name, &computer_name(&data), &catalog());
        info.note_configuration(&built.layout_hash());
        let lights = built.lights.len();
        let shared = Shared::new(info, built, config.pin.clone(), config.debug_logging, runner);

        let mut advertiser = match mdns::Advertiser::new(&config.bridge_name, &config.username, port) {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!("[HomeKit] Could not start mDNS ({e}); the Home app won't find the bridge.");
                None
            }
        };
        let model = config.bridge_name.clone();
        if let Some(a) = advertiser.as_mut()
            && let Err(e) = a.publish(&txt_for(&shared, &model))
        {
            tracing::warn!("[HomeKit] Could not advertise the bridge: {e}");
        }

        tokio::spawn(server::serve(shared.clone(), listener));

        // Rebuild on command/automation changes; re-advertise on pairing or layout changes.
        let task_shared = shared.clone();
        tokio::spawn(async move {
            let shared = task_shared;
            loop {
                tokio::select! {
                    _ = shared.cancel.cancelled() => break,
                    _ = shared.pairing_changed.notified() => {}
                    changed = changes.recv() => {
                        if matches!(changed, Err(broadcast::error::RecvError::Closed)) { break }
                        let entries = catalog();
                        let config = HomeKitConfig::load(&data);
                        let rebuilt = {
                            let info = shared.info.lock().unwrap();
                            db::build(&mut ids, &info.username, &config.bridge_name, &computer_name(&data), &entries)
                        };
                        let count = rebuilt.lights.len();
                        let changed = shared.info.lock().unwrap().note_configuration(&rebuilt.layout_hash());
                        shared.replace_database(rebuilt);
                        tracing::info!("[HomeKit] Published {count} light(s)");
                        if !changed { continue }
                    }
                }
                if let Some(a) = advertiser.as_mut()
                    && let Err(e) = a.publish(&txt_for(&shared, &model))
                {
                    tracing::warn!("[HomeKit] Could not update the advertisement: {e}");
                }
            }
            if let Some(a) = advertiser {
                a.shutdown();
            }
        });

        tracing::info!("[HomeKit] Published {lights} light(s)");
        tracing::info!("[HomeKit] HomeKit bridge started on port {port}. Pair with PIN: {}", config.pin);
        Self { shared: Some(shared), error: None, port }
    }

    pub fn status(&self) -> BridgeStatus {
        match &self.shared {
            None => BridgeStatus { error: self.error.clone(), port: self.port, ..BridgeStatus::default() },
            Some(shared) => {
                let info = shared.info.lock().unwrap();
                BridgeStatus {
                    running: true,
                    paired: info.is_paired(),
                    controllers: info.paired.len(),
                    lights: shared.db.read().unwrap().lights.len(),
                    port: self.port,
                    error: None,
                }
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.cancel.cancel();
            tracing::info!("HomeKit bridge stopped.");
        }
    }
}

impl Drop for HomeKitBridge {
    fn drop(&mut self) {
        self.stop();
    }
}
