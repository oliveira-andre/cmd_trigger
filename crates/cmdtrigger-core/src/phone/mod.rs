//! The phone link: the CmdTrigger iPhone app talks to this computer directly on the home Wi-Fi.
//!
//! ```text
//! GET  /phone/v1/hello                      → {app, v, id, name, pairing}       (public)
//! POST /phone/v1/pair  {phoneId, phoneName, phonePublic, proof}
//!                                           → {macId, macName, macPublic, proof}
//! POST /phone/v1/call  {phone, sealed}      → {sealed}
//! ```
//!
//! Pairing needs the one-time code the computer shows (QR code or text), see [`crypto`]. Every
//! later call is sealed with the per-phone key, carries a timestamp and a fresh nonce (replays
//! are refused), and is one of:
//!
//! ```text
//! {op: "status"}                            → {name, os, catalog, interfaces, wakeForNetwork, server}
//! {op: "run", kind, target, params?}        → {run: RunRecord}
//! {op: "runs", limit?}                      → {runs: [RunRecord]}
//! {op: "wakeScreen"}                        → {}
//! {op: "unpair"}                            → {}
//! ```
//!
//! Only the foreground agent (the app, or `cmdtrigger console`) serves phones. It reaches the
//! background service's commands through the service's local API.

pub mod bonjour;
pub mod crypto;
pub mod host;

use crate::agent::Ground;
use crate::config::{ApiConfig, ServerConfig, Settings};
use crate::history::now_ms;
use crate::paths::DataDir;
use crate::runner::Runner;
use crate::store;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use cmdtrigger_protocol::{CatalogEntry, TargetKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PORT: u16 = 7180;
const PAIRING_WINDOW: Duration = Duration::from_secs(300);
const MAX_ATTEMPTS: u32 = 5;
/// How far a call's timestamp may be from this computer's clock.
const CLOCK_SLACK_SECS: i64 = 300;
const RUN_WAIT: Duration = Duration::from_secs(55);

fn yes() -> bool {
    true
}
fn default_port() -> u16 {
    DEFAULT_PORT
}

/// `phones.json` (owner-only): whether phones may connect, the port, this computer's id, and
/// the paired phones with their keys.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub mac_id: String,
    #[serde(default)]
    pub phones: Vec<PairedPhone>,
}

impl Default for PhoneConfig {
    fn default() -> Self {
        Self { enabled: true, port: DEFAULT_PORT, mac_id: String::new(), phones: Vec::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedPhone {
    pub id: String,
    pub name: String,
    /// Base64 of the 32-byte key.
    pub key: String,
    pub paired_at: u64,
    #[serde(default)]
    pub last_seen: Option<u64>,
}

impl PairedPhone {
    fn key(&self) -> Option<[u8; 32]> {
        B64.decode(&self.key).ok()?.try_into().ok()
    }
}

/// A paired phone as the window shows it (no key).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneView {
    pub id: String,
    pub name: String,
    pub paired_at: u64,
    pub last_seen: Option<u64>,
}

impl PhoneConfig {
    /// The saved config; gives this computer an id the first time.
    pub fn load(data: &DataDir) -> Self {
        let mut config: PhoneConfig = store::read_json(&data.phones());
        if config.mac_id.is_empty() {
            config.mac_id = hex::encode(crypto::random::<8>());
            let _ = config.save(data);
        }
        config
    }

    pub fn save(&self, data: &DataDir) -> std::io::Result<()> {
        store::write_private(&data.phones(), store::to_json_2(self).as_bytes())
    }

    pub fn views(&self) -> Vec<PhoneView> {
        self.phones
            .iter()
            .map(|p| PhoneView {
                id: p.id.clone(),
                name: p.name.clone(),
                paired_at: p.paired_at,
                last_seen: p.last_seen,
            })
            .collect()
    }

    pub fn remove(data: &DataDir, id: &str) -> std::io::Result<bool> {
        let mut config = Self::load(data);
        let before = config.phones.len();
        config.phones.retain(|p| p.id != id);
        config.save(data)?;
        Ok(config.phones.len() != before)
    }
}

/// An open pairing window, as the window shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    /// "ABCD-EFGH-JKMN-PQRS".
    pub code: String,
    /// `cmdtrigger://pair?…`: the QR code's contents; opens the iPhone app from the Camera.
    pub url: String,
    pub qr_svg: String,
    pub expires_in: u64,
    pub hosts: Vec<String>,
    pub port: u16,
    /// Set once a phone has paired with this code.
    pub paired_with: Option<String>,
}

struct Pairing {
    code: String,
    info: PairingInfo,
    expires: Instant,
    attempts: u32,
    paired_with: Option<String>,
}

/// Nonces seen per phone, with when.
type SeenNonces = HashMap<String, VecDeque<([u8; 12], Instant)>>;

struct Shared {
    data: DataDir,
    runner: Arc<Runner>,
    port: u16,
    pairing: Mutex<Option<Pairing>>,
    /// Recent nonces per phone, to refuse replays.
    seen: Mutex<SeenNonces>,
    /// Serializes `phones.json` writes.
    writing: Mutex<()>,
    http: reqwest::Client,
}

pub struct PhoneLink {
    cancel: CancellationToken,
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    _advertiser: Option<bonjour::Advertiser>,
}

impl PhoneLink {
    /// Listen on every interface when phones are allowed. Fails when the port is taken.
    pub async fn start(
        data: &DataDir,
        runner: Arc<Runner>,
        port_override: Option<u16>,
    ) -> Result<Option<Self>, String> {
        let config = PhoneConfig::load(data);
        if !config.enabled {
            return Ok(None);
        }
        let port = port_override.unwrap_or(config.port);
        let listener =
            tokio::net::TcpListener::bind(("0.0.0.0", port)).await.map_err(|e| format!("port {port}: {e}"))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared {
            data: data.clone(),
            runner,
            port: addr.port(),
            pairing: Mutex::new(None),
            seen: Mutex::new(HashMap::new()),
            writing: Mutex::new(()),
            http: reqwest::Client::builder().timeout(Duration::from_secs(70)).build().map_err(|e| e.to_string())?,
        });
        let cancel = CancellationToken::new();
        let app = router(shared.clone());
        let stop = cancel.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async move { stop.cancelled().await }).await;
        });
        let ips: Vec<String> = host::interfaces().await.into_iter().map(|i| i.ip).collect();
        let name = Settings::load(data).computer_name();
        let advertiser = match bonjour::Advertiser::publish(
            &name,
            &bonjour::Txt { id: &config.mac_id, ips: &ips.join(","), port: addr.port() },
        ) {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!(
                    "Phones can't find this computer by name (Bonjour: {e}); pairing by QR code still works."
                );
                None
            }
        };
        tracing::info!("Phone link on {addr}");
        Ok(Some(Self { cancel, addr, shared, _advertiser: advertiser }))
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// Open a five-minute pairing window with a fresh code (replacing any open one).
    pub async fn start_pairing(&self) -> PairingInfo {
        let config = PhoneConfig::load(&self.shared.data);
        let code = crypto::new_code();
        let hosts: Vec<String> = host::interfaces().await.into_iter().map(|i| i.ip).collect();
        let name = Settings::load(&self.shared.data).computer_name();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("v", "1")
            .append_pair("id", &config.mac_id)
            .append_pair("n", &name)
            .append_pair("h", &hosts.join(","))
            .append_pair("p", &self.shared.port.to_string())
            .append_pair("c", &code)
            .finish();
        let url = format!("cmdtrigger://pair?{query}");
        let info = PairingInfo {
            code: crypto::grouped(&code),
            qr_svg: qr_svg(&url),
            url,
            expires_in: PAIRING_WINDOW.as_secs(),
            hosts,
            port: self.shared.port,
            paired_with: None,
        };
        *self.shared.pairing.lock().unwrap() = Some(Pairing {
            code,
            info: info.clone(),
            expires: Instant::now() + PAIRING_WINDOW,
            attempts: 0,
            paired_with: None,
        });
        info
    }

    /// The open window (with how long it has left), or `None`.
    pub fn pairing(&self) -> Option<PairingInfo> {
        let slot = self.shared.pairing.lock().unwrap();
        let p = slot.as_ref()?;
        let left = p.expires.saturating_duration_since(Instant::now());
        if left.is_zero() && p.paired_with.is_none() {
            return None;
        }
        Some(PairingInfo { expires_in: left.as_secs(), paired_with: p.paired_with.clone(), ..p.info.clone() })
    }

    pub fn cancel_pairing(&self) {
        *self.shared.pairing.lock().unwrap() = None;
    }
}

impl Drop for PhoneLink {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(232, 232)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#101010"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}

fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/phone/v1/hello", get(hello))
        .route("/phone/v1/pair", post(pair))
        .route("/phone/v1/call", post(call))
        .with_state(shared)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn hello(State(s): State<Arc<Shared>>) -> Response {
    let config = PhoneConfig::load(&s.data);
    let pairing =
        s.pairing.lock().unwrap().as_ref().is_some_and(|p| p.paired_with.is_none() && p.expires > Instant::now());
    Json(json!({
        "app": "cmdtrigger",
        "v": 1,
        "id": config.mac_id,
        "name": Settings::load(&s.data).computer_name(),
        "pairing": pairing,
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairBody {
    phone_id: String,
    phone_name: String,
    phone_public: String,
    proof: String,
}

async fn pair(State(s): State<Arc<Shared>>, Json(body): Json<PairBody>) -> Response {
    let (Some(phone_public), Some(proof)) =
        (B64.decode(&body.phone_public).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()), B64.decode(&body.proof).ok())
    else {
        return error(StatusCode::BAD_REQUEST, "malformed pairing request");
    };
    let phone_id: String = body.phone_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
    if phone_id.is_empty() {
        return error(StatusCode::BAD_REQUEST, "malformed pairing request");
    }
    let code_key = {
        let mut slot = s.pairing.lock().unwrap();
        let Some(p) = slot.as_mut().filter(|p| p.paired_with.is_none() && p.expires > Instant::now()) else {
            return error(StatusCode::CONFLICT, "pairing isn't open on the computer");
        };
        p.attempts += 1;
        let key = crypto::code_key(&p.code);
        if !crypto::constant_time_eq(&crypto::phone_proof(&key, &phone_public, &phone_id), &proof) {
            if p.attempts >= MAX_ATTEMPTS {
                *slot = None;
                tracing::warn!("Pairing closed after {MAX_ATTEMPTS} wrong codes");
                return error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many wrong codes; start pairing again on the computer",
                );
            }
            return error(StatusCode::FORBIDDEN, "wrong code");
        }
        key
    };
    let (mac_public, key, proof) = crypto::mac_side(&code_key, &phone_public);
    let name: String = body.phone_name.trim().chars().take(60).collect();
    let name = if name.is_empty() { "iPhone".to_string() } else { name };
    let config = {
        let _w = s.writing.lock().unwrap();
        let mut config = PhoneConfig::load(&s.data);
        config.phones.retain(|p| p.id != phone_id);
        config.phones.push(PairedPhone {
            id: phone_id.clone(),
            name: name.clone(),
            key: B64.encode(key),
            paired_at: now_ms(),
            last_seen: Some(now_ms()),
        });
        if let Err(e) = config.save(&s.data) {
            return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("couldn't save the pairing: {e}"));
        }
        config
    };
    if let Some(p) = s.pairing.lock().unwrap().as_mut() {
        p.paired_with = Some(name.clone());
    }
    tracing::info!("Paired with the phone \"{name}\"");
    Json(json!({
        "macId": config.mac_id,
        "macName": Settings::load(&s.data).computer_name(),
        "macPublic": B64.encode(mac_public),
        "proof": B64.encode(proof),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CallBody {
    phone: String,
    sealed: String,
}

#[derive(Deserialize)]
struct Call {
    at: i64,
    op: String,
    #[serde(default)]
    kind: Option<TargetKind>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    params: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn call(State(s): State<Arc<Shared>>, Json(body): Json<CallBody>) -> Response {
    let config = PhoneConfig::load(&s.data);
    let Some(phone) = config.phones.iter().find(|p| p.id == body.phone) else {
        return error(StatusCode::UNAUTHORIZED, "not paired");
    };
    let Some(key) = phone.key() else { return error(StatusCode::UNAUTHORIZED, "not paired") };
    let Some((plain, nonce)) =
        B64.decode(&body.sealed).ok().and_then(|sealed| crypto::open(&key, "call", &phone.id, &sealed))
    else {
        return error(StatusCode::UNAUTHORIZED, "not paired");
    };
    let Ok(request) = serde_json::from_slice::<Call>(&plain) else {
        return error(StatusCode::BAD_REQUEST, "unreadable request");
    };
    let now = (now_ms() / 1000) as i64;
    if (now - request.at).abs() > CLOCK_SLACK_SECS {
        return error(StatusCode::BAD_REQUEST, "the phone's clock and this computer's disagree by more than 5 minutes");
    }
    if !fresh(&s, &phone.id, nonce) {
        return error(StatusCode::CONFLICT, "replayed request");
    }
    touch(&s, &phone.id);
    let reply = handle(&s, &phone.id, request).await;
    let sealed = crypto::seal(&key, "reply", &phone.id, reply.to_string().as_bytes());
    Json(json!({ "sealed": B64.encode(sealed) })).into_response()
}

/// Records the nonce; false if it was used in the last ten minutes.
fn fresh(s: &Shared, phone: &str, nonce: [u8; 12]) -> bool {
    let mut seen = s.seen.lock().unwrap();
    let list = seen.entry(phone.to_string()).or_default();
    let cutoff = Duration::from_secs(2 * CLOCK_SLACK_SECS as u64);
    while list.front().is_some_and(|(_, at)| at.elapsed() > cutoff) {
        list.pop_front();
    }
    if list.iter().any(|(n, _)| *n == nonce) {
        return false;
    }
    list.push_back((nonce, Instant::now()));
    if list.len() > 5000 {
        list.pop_front();
    }
    true
}

/// Remembers when the phone was last here (at most once a minute, to spare the disk).
fn touch(s: &Shared, phone: &str) {
    let _w = s.writing.lock().unwrap();
    let mut config = PhoneConfig::load(&s.data);
    if let Some(p) = config.phones.iter_mut().find(|p| p.id == phone)
        && p.last_seen.is_none_or(|t| now_ms().saturating_sub(t) > 60_000)
    {
        p.last_seen = Some(now_ms());
        let _ = config.save(&s.data);
    }
}

async fn handle(s: &Shared, phone: &str, call: Call) -> Value {
    match call.op.as_str() {
        "status" => {
            let server = ServerConfig::load(&s.data);
            let server = server.connected().then(|| json!({ "url": server.url, "computerId": server.computer_id }));
            json!({
                "ok": true,
                "id": PhoneConfig::load(&s.data).mac_id,
                "name": Settings::load(&s.data).computer_name(),
                "os": std::env::consts::OS,
                "version": crate::VERSION,
                "catalog": catalog(s).await,
                "interfaces": host::interfaces().await,
                "wakeForNetwork": host::wake_for_network().await,
                "server": server,
            })
        }
        "run" => {
            let (Some(kind), Some(target)) = (call.kind, call.target) else {
                return json!({ "error": "run needs kind and target" });
            };
            let params = call.params.filter(|p| !p.trim().is_empty());
            let own = s.runner.catalog().into_iter().find(|e| e.kind == kind && e.id == target);
            if own.is_some() {
                let runner = s.runner.clone();
                let task = tokio::spawn(async move { runner.run(kind, &target, params, "phone").await });
                return match tokio::time::timeout(RUN_WAIT, task).await {
                    Ok(Ok(record)) => json!({ "ok": true, "run": record }),
                    _ => json!({ "ok": true, "running": true }),
                };
            }
            match peer(s) {
                Some(peer) => match peer.run(kind, &target, params).await {
                    Ok(record) => json!({ "ok": true, "run": record }),
                    Err(e) => json!({ "error": e }),
                },
                None => json!({ "error": format!("nothing called \"{target}\" on this computer") }),
            }
        }
        "runs" => {
            let limit = call.limit.unwrap_or(50).min(200);
            let mut runs: Vec<Value> =
                s.runner.recent(limit).into_iter().filter_map(|r| serde_json::to_value(r).ok()).collect();
            if let Some(peer) = peer(s) {
                runs.extend(peer.runs(limit).await);
                runs.sort_by_key(|r| std::cmp::Reverse(r.get("at").and_then(Value::as_u64).unwrap_or(0)));
                runs.truncate(limit);
            }
            json!({ "ok": true, "runs": runs })
        }
        "wakeScreen" => match host::wake_screen().await {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "error": e }),
        },
        "unpair" => {
            let _w = s.writing.lock().unwrap();
            match PhoneConfig::remove(&s.data, phone) {
                Ok(_) => json!({ "ok": true }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        other => json!({ "error": format!("unknown request \"{other}\"") }),
    }
}

/// This agent's catalog plus the background service's, when it runs.
async fn catalog(s: &Shared) -> Vec<CatalogEntry> {
    let mut list = s.runner.catalog();
    if let Some(peer) = peer(s) {
        for e in peer.catalog().await {
            if !list.iter().any(|o| o.kind == e.kind && o.id == e.id) {
                list.push(e);
            }
        }
    }
    list
}

/// The other agent on this computer (the background service, from the app), reached through
/// its local API.
struct Peer<'a> {
    base: String,
    key: String,
    http: &'a reqwest::Client,
}

fn peer(s: &Shared) -> Option<Peer<'_>> {
    let config = ApiConfig::load(&s.data);
    let other = match s.runner.ground() {
        Ground::Foreground => Ground::Background,
        Ground::Background => Ground::Foreground,
    };
    (config.enabled && !config.key.is_empty()).then(|| Peer {
        base: format!("http://127.0.0.1:{}", config.port_for(other)),
        key: config.key,
        http: &s.http,
    })
}

impl Peer<'_> {
    async fn catalog(&self) -> Vec<CatalogEntry> {
        let reply = self
            .http
            .get(format!("{}/api/v1/catalog", self.base))
            .bearer_auth(&self.key)
            .timeout(Duration::from_millis(1500))
            .send()
            .await;
        match reply {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    async fn run(&self, kind: TargetKind, target: &str, params: Option<String>) -> Result<Value, String> {
        let reply = self
            .http
            .post(format!("{}/api/v1/run", self.base))
            .bearer_auth(&self.key)
            .json(&json!({ "target": target, "kind": kind, "params": params, "wait": true, "source": "phone" }))
            .send()
            .await
            .map_err(|_| "the background service isn't answering".to_string())?;
        let ok = reply.status().is_success();
        let body: Value = reply.json().await.map_err(|e| e.to_string())?;
        if ok {
            Ok(body)
        } else {
            Err(body.get("error").and_then(Value::as_str).unwrap_or("the background service refused").to_string())
        }
    }

    async fn runs(&self, limit: usize) -> Vec<Value> {
        let reply = self
            .http
            .get(format!("{}/api/v1/runs?limit={limit}", self.base))
            .bearer_auth(&self.key)
            .timeout(Duration::from_millis(1500))
            .send()
            .await;
        match reply {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}

// End to end with shell commands and a real listener + Bonjour, like the other runner tests:
// Unix only (Wine can't run the Windows advertiser; real Windows is a README follow-up).
#[cfg(all(test, unix))]
mod tests;
