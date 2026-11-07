//! A running agent: the runner, the automation engine, and whichever ways in are switched on:
//! HomeKit, Home Assistant (MQTT), the local API, paired iPhones and the link to your server.
//! It needs no account; everything is local unless a server is set up.

use crate::config::{ApiConfig, MqttConfig, ServerConfig, Settings};
use crate::engine::Engine;
use crate::homekit::{BridgeStatus, HomeKitBridge};
use crate::link::{Link, LinkStatus};
use crate::local_api::LocalApi;
use crate::mqtt::{Mqtt, MqttStatus};
use crate::paths::DataDir;
use crate::phone::{PairingInfo, PhoneLink};
use crate::runner::Runner;
use crate::store;
use crate::watch::{FileWatcher, watch_file};
use cmdtrigger_protocol::CatalogEntry;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// Which commands an agent runs. The tray app (or `cmdtrigger` in a terminal) is the
/// foreground agent; the background service (root on Linux, a Windows service) runs the
/// "background" ones, with nobody logged in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ground {
    Foreground,
    Background,
}

impl Ground {
    pub fn as_str(self) -> &'static str {
        match self {
            Ground::Foreground => "foreground",
            Ground::Background => "background",
        }
    }
}

/// Test hooks: ports other than the defaults.
#[derive(Clone, Debug)]
pub struct Options {
    pub homekit_port: u16,
    /// Override the local API port (0 = any free port).
    pub api_port: Option<u16>,
    /// Override the phone link port (0 = any free port).
    pub phone_port: Option<u16>,
}

impl Default for Options {
    fn default() -> Self {
        Self { homekit_port: crate::homekit::PORT, api_port: None, phone_port: None }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub error: Option<String>,
    pub pairing: Option<PairingInfo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub running: bool,
    pub address: Option<String>,
    pub error: Option<String>,
}

struct Inner {
    data: DataDir,
    runner: Arc<Runner>,
    changes: broadcast::Sender<()>,
    _engine: Engine,
    homekit: tokio::sync::Mutex<HomeKitBridge>,
    mqtt: Mutex<Option<Mqtt>>,
    api: tokio::sync::Mutex<(Option<LocalApi>, Option<String>)>,
    phone: tokio::sync::Mutex<(Option<PhoneLink>, Option<String>)>,
    link: Mutex<Option<Link>>,
    _watchers: Vec<FileWatcher>,
    options: Options,
}

#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

impl Agent {
    pub async fn start(data: DataDir, ground: Ground) -> Agent {
        Self::start_with(data, ground, Options::default()).await
    }

    pub async fn start_with(data: DataDir, ground: Ground, options: Options) -> Agent {
        if let Err(e) = store::init_data_dir(&data) {
            tracing::warn!("Can't prepare {}: {e}", data.root().display());
        }
        let _ = store::load_with_backup(&data);
        tracing::info!("Starting the {} agent ({}).", ground.as_str(), data.root().display());
        let runner = Runner::new(data.clone(), ground);
        let (changes, _) = broadcast::channel(16);
        let settle =
            if ground == Ground::Background { Duration::from_millis(1000) } else { Duration::from_millis(250) };
        let mut watchers = Vec::new();
        for file in [data.commands(), data.automations()] {
            match watch_file(&file, settle, changes.clone()) {
                Ok(w) => watchers.push(w),
                Err(e) => tracing::warn!("Can't watch {}: {e}", file.display()),
            }
        }
        let engine = Engine::start(runner.clone(), changes.subscribe());
        let inner = Arc::new(Inner {
            data,
            runner,
            changes,
            _engine: engine,
            homekit: tokio::sync::Mutex::new(HomeKitBridge::disabled()),
            mqtt: Mutex::new(None),
            api: tokio::sync::Mutex::new((None, None)),
            phone: tokio::sync::Mutex::new((None, None)),
            link: Mutex::new(None),
            _watchers: watchers,
            options,
        });
        let agent = Agent { inner };
        agent.restart_api().await;
        agent.restart_phone().await;
        agent.restart_homekit().await;
        agent.restart_mqtt();
        agent.restart_link();
        agent
    }

    pub fn data(&self) -> &DataDir {
        &self.inner.data
    }

    pub fn runner(&self) -> &Arc<Runner> {
        &self.inner.runner
    }

    pub fn ground(&self) -> Ground {
        self.inner.runner.ground()
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.inner.runner.catalog()
    }

    pub fn computer_name(&self) -> String {
        Settings::load(&self.inner.data).computer_name()
    }

    /// Commands or automations changed (the file watcher also notices; this is immediate).
    pub fn changed(&self) {
        let _ = self.inner.changes.send(());
    }

    pub fn subscribe_changes(&self) -> broadcast::Receiver<()> {
        self.inner.changes.subscribe()
    }

    pub async fn restart_homekit(&self) {
        let mut bridge = self.inner.homekit.lock().await;
        bridge.stop();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let runner = self.inner.runner.clone();
        let catalog: crate::homekit::Catalog = {
            let runner = runner.clone();
            Arc::new(move || runner.catalog())
        };
        let run: crate::homekit::server::Runner = Arc::new(move |entry: CatalogEntry, on, level| {
            let runner = runner.clone();
            tokio::spawn(async move {
                runner.run_homekit(entry.kind, &entry.id, on, level).await;
            });
        });
        *bridge = HomeKitBridge::start(
            self.inner.data.clone(),
            catalog,
            self.inner.changes.subscribe(),
            self.inner.options.homekit_port,
            run,
        )
        .await;
    }

    pub async fn homekit_status(&self) -> BridgeStatus {
        self.inner.homekit.lock().await.status()
    }

    pub fn restart_mqtt(&self) {
        let mut slot = self.inner.mqtt.lock().unwrap();
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Mqtt::start(
            MqttConfig::load(&self.inner.data),
            self.computer_name(),
            self.inner.runner.clone(),
            self.inner.changes.subscribe(),
        );
    }

    pub fn mqtt_status(&self) -> Option<MqttStatus> {
        self.inner.mqtt.lock().unwrap().as_ref().map(Mqtt::status)
    }

    pub async fn restart_api(&self) {
        let mut slot = self.inner.api.lock().await;
        if let Some(old) = slot.0.take() {
            old.stop();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let mut config = ApiConfig::load(&self.inner.data);
        if let Some(port) = self.inner.options.api_port {
            config.port = port;
        }
        let pairing: Option<crate::local_api::PairingControl> = (self.ground() == Ground::Foreground).then(|| {
            let weak = Arc::downgrade(&self.inner);
            let control: crate::local_api::PairingControl = Arc::new(move |open| {
                let weak = weak.clone();
                Box::pin(async move {
                    let agent = Agent { inner: weak.upgrade().ok_or("the agent is stopping")? };
                    if open {
                        agent.start_phone_pairing().await.map(Some)
                    } else {
                        Ok(agent.phone_status().await.pairing)
                    }
                })
            });
            control
        });
        *slot = match LocalApi::start(&config, self.inner.runner.clone(), pairing).await {
            Ok(api) => (api, None),
            Err(e) => {
                let port = config.port_for(self.ground());
                tracing::warn!("Local API not started: port {port}: {e}");
                (None, Some(format!("port {port} is in use ({e})")))
            }
        };
    }

    pub async fn api_status(&self) -> ApiStatus {
        let slot = self.inner.api.lock().await;
        ApiStatus {
            running: slot.0.is_some(),
            address: slot.0.as_ref().map(|a| a.addr.to_string()),
            error: slot.1.clone(),
        }
    }

    /// Paired iPhones on the network. Only the foreground agent serves them; it reaches the
    /// background service's commands through the service's local API.
    pub async fn restart_phone(&self) {
        let mut slot = self.inner.phone.lock().await;
        if let Some(old) = slot.0.take() {
            old.stop();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if self.ground() != Ground::Foreground {
            return;
        }
        *slot = match PhoneLink::start(&self.inner.data, self.inner.runner.clone(), self.inner.options.phone_port).await
        {
            Ok(link) => (link, None),
            Err(e) => {
                tracing::warn!("Phone link not started: {e}");
                (None, Some(e))
            }
        };
    }

    pub async fn phone_status(&self) -> PhoneStatus {
        let slot = self.inner.phone.lock().await;
        PhoneStatus {
            running: slot.0.is_some(),
            port: slot.0.as_ref().map(|p| p.addr.port()),
            error: slot.1.clone(),
            pairing: slot.0.as_ref().and_then(PhoneLink::pairing),
        }
    }

    /// Open a pairing window; fails when phones are turned off.
    pub async fn start_phone_pairing(&self) -> Result<PairingInfo, String> {
        let slot = self.inner.phone.lock().await;
        match slot.0.as_ref() {
            Some(link) => Ok(link.start_pairing().await),
            None => Err(slot.1.clone().unwrap_or_else(|| "Turn on iPhone access first.".into())),
        }
    }

    pub async fn cancel_phone_pairing(&self) {
        if let Some(link) = self.inner.phone.lock().await.0.as_ref() {
            link.cancel_pairing();
        }
    }

    pub fn restart_link(&self) {
        let mut slot = self.inner.link.lock().unwrap();
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Link::start(
            ServerConfig::load(&self.inner.data),
            self.computer_name(),
            self.inner.runner.clone(),
            self.inner.changes.subscribe(),
        );
    }

    pub fn link_status(&self) -> LinkStatus {
        self.inner.link.lock().unwrap().as_ref().map(Link::status).unwrap_or(LinkStatus::Off)
    }

    pub fn link_watch(&self) -> Option<tokio::sync::watch::Receiver<LinkStatus>> {
        self.inner.link.lock().unwrap().as_ref().map(Link::watch)
    }

    pub async fn shutdown(&self) {
        let mqtt = self.inner.mqtt.lock().unwrap().take();
        if let Some(m) = mqtt {
            m.stop();
        }
        let link = self.inner.link.lock().unwrap().take();
        if let Some(l) = link {
            l.stop();
        }
        if let Some(a) = self.inner.api.lock().await.0.take() {
            a.stop();
        }
        if let Some(p) = self.inner.phone.lock().await.0.take() {
            p.stop();
        }
        self.inner.homekit.lock().await.stop();
        // Give MQTT a moment to publish "offline".
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
