//! The HomeKit Accessory Protocol server: HTTP/1.1 over TCP, pair-setup (SRP), pair-verify
//! (X25519 + Ed25519), the encrypted session, `/accessories`, `/characteristics` (read, write,
//! event subscriptions) and `/pairings`.

use super::crypto::{FrameCipher, hkdf_sha512, label_nonce, open, seal};
use super::db::{Database, RESTING_BRIGHTNESS, RESTING_ON, Role};
use super::storage::{AccessoryInfo, PERMISSION_ADMIN, Pairing};
use super::{srp, tlv, tlv::Tlv};
use cmdtrigger_protocol::CatalogEntry;
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Runs a light's command or automation: (what, turned on?, brightness).
pub type Runner = Arc<dyn Fn(CatalogEntry, bool, Option<u8>) + Send + Sync>;

/// With parameters allowed, "on" waits this long in case a brightness change follows.
pub const ON_DELAY: Duration = Duration::from_millis(800);
/// After a brightness change, an "on" within this window is part of the same gesture.
pub const SUPPRESS_ON: Duration = Duration::from_millis(1500);
/// How long a light shows its new state before snapping back to off.
pub const RESET_AFTER: Duration = Duration::from_millis(400);
const MAX_PAIRINGS: usize = 16;
const MAX_SETUP_ATTEMPTS: u32 = 100;
const MAX_REQUEST: usize = 64 * 1024;

const STATUS_READ_ONLY: i64 = -70404;
const STATUS_WRITE_ONLY: i64 = -70405;
const STATUS_NO_NOTIFICATION: i64 = -70406;
const STATUS_NOT_FOUND: i64 = -70409;
const STATUS_INVALID_VALUE: i64 = -70410;
const STATUS_INSUFFICIENT_PRIVILEGES: i64 = -70401;

#[derive(Default)]
struct LightState {
    pending_on: Option<tokio::task::JoinHandle<()>>,
    suppress_until: Option<Instant>,
}

pub struct Shared {
    pub info: Mutex<AccessoryInfo>,
    pub db: RwLock<Database>,
    pin: String,
    debug: bool,
    runner: Runner,
    lights: Mutex<HashMap<usize, LightState>>,
    connections: Mutex<HashMap<u64, Arc<Connection>>>,
    next_connection: AtomicU64,
    failed_setups: AtomicU32,
    /// Fired when pairing state changes, so the advertisement's status flag follows.
    pub pairing_changed: Notify,
    pub cancel: CancellationToken,
}

struct Writer {
    half: OwnedWriteHalf,
    cipher: Option<FrameCipher>,
}

struct Connection {
    writer: tokio::sync::Mutex<Writer>,
    controller: Mutex<Option<String>>,
    events: Mutex<HashSet<(u64, u64)>>,
    close: CancellationToken,
}

impl Connection {
    async fn send(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().await;
        let out = match w.cipher.as_mut() {
            Some(c) => c.encrypt(bytes),
            None => bytes.to_vec(),
        };
        w.half.write_all(&out).await
    }
}

impl Shared {
    pub fn new(info: AccessoryInfo, db: Database, pin: String, debug: bool, runner: Runner) -> Arc<Self> {
        Arc::new(Self {
            info: Mutex::new(info),
            db: RwLock::new(db),
            pin,
            debug,
            runner,
            lights: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(1),
            failed_setups: AtomicU32::new(0),
            pairing_changed: Notify::new(),
            cancel: CancellationToken::new(),
        })
    }

    fn log(&self, message: impl FnOnce() -> String) {
        if self.debug {
            tracing::info!("[HomeKit] {}", message());
        }
    }

    /// Swap in a rebuilt database. Pending "on" timers belong to the old layout and are dropped.
    pub fn replace_database(&self, db: Database) {
        for (_, state) in self.lights.lock().unwrap().drain() {
            if let Some(t) = state.pending_on {
                t.abort();
            }
        }
        *self.db.write().unwrap() = db;
    }

    pub fn paired(&self) -> bool {
        self.info.lock().unwrap().is_paired()
    }

    pub fn connection_count(&self) -> usize {
        self.connections.lock().unwrap().len()
    }
}

/// Accept connections until cancelled.
pub async fn serve(shared: Arc<Shared>, listener: TcpListener) {
    loop {
        let accepted = tokio::select! {
            _ = shared.cancel.cancelled() => break,
            a = listener.accept() => a,
        };
        let Ok((stream, peer)) = accepted else { continue };
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            shared.log(|| format!("Connection from {peer}"));
            handle_connection(shared.clone(), stream).await;
            shared.log(|| format!("Connection from {peer} closed"));
        });
    }
    let all: Vec<_> = shared.connections.lock().unwrap().drain().map(|(_, c)| c).collect();
    for c in all {
        c.close.cancel();
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

fn parse_request(buf: &mut Vec<u8>) -> Result<Option<Request>, ()> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    let head = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return if buf.len() > MAX_REQUEST { Err(()) } else { Ok(None) },
        Err(_) => return Err(()),
    };
    let length = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-length"))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST {
        return Err(());
    }
    if buf.len() < head + length {
        return Ok(None);
    }
    let target = req.path.unwrap_or("/").to_string();
    let method = req.method.unwrap_or("GET").to_string();
    let (path, query) =
        target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target, String::new()));
    let body = buf[head..head + length].to_vec();
    buf.drain(..head + length);
    Ok(Some(Request { method, path, query, body }))
}

fn http(status: u16, reason: &str, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status} {reason}\r\n");
    if let Some(ct) = content_type {
        out.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn tlv_response(t: Tlv) -> Vec<u8> {
    http(200, "OK", Some("application/pairing+tlv8"), &t.encode())
}

fn json_response(status: u16, reason: &str, body: &Value) -> Vec<u8> {
    http(status, reason, Some("application/hap+json"), body.to_string().as_bytes())
}

fn no_content() -> Vec<u8> {
    b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()
}

fn event_frame(body: &Value) -> Vec<u8> {
    let body = body.to_string();
    format!("EVENT/1.0 200 OK\r\nContent-Type: application/hap+json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
        .into_bytes()
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS has randomness");
    b
}

#[derive(Default)]
struct ConnState {
    setup: Option<(srp::Server, Option<Vec<u8>>)>,
    verify: Option<VerifyState>,
}

struct VerifyState {
    shared_secret: [u8; 32],
    accessory_public: [u8; 32],
    controller_public: [u8; 32],
}

/// What a request does to the connection besides its response.
enum After {
    Nothing,
    StartEncryption { read: [u8; 32], write: [u8; 32], controller: String },
    CloseControllers(Vec<String>),
}

async fn handle_connection(shared: Arc<Shared>, stream: TcpStream) {
    let (mut reader, half) = stream.into_split();
    let id = shared.next_connection.fetch_add(1, Ordering::SeqCst);
    let conn = Arc::new(Connection {
        writer: tokio::sync::Mutex::new(Writer { half, cipher: None }),
        controller: Mutex::new(None),
        events: Mutex::new(HashSet::new()),
        close: shared.cancel.child_token(),
    });
    shared.connections.lock().unwrap().insert(id, conn.clone());

    let mut state = ConnState::default();
    let mut inbound: Option<FrameCipher> = None;
    let mut raw = Vec::new();
    let mut plain = Vec::new();
    let mut chunk = vec![0u8; 4096];
    'outer: loop {
        let n = tokio::select! {
            _ = conn.close.cancelled() => break,
            r = reader.read(&mut chunk) => match r { Ok(0) | Err(_) => break, Ok(n) => n },
        };
        match inbound.as_mut() {
            Some(cipher) => {
                raw.extend_from_slice(&chunk[..n]);
                match cipher.decrypt(&mut raw) {
                    Ok(bytes) => plain.extend(bytes),
                    Err(_) => break,
                }
            }
            None => plain.extend_from_slice(&chunk[..n]),
        }
        loop {
            let request = match parse_request(&mut plain) {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(()) => break 'outer,
            };
            let verified = inbound.is_some();
            let (response, after) = route(&shared, &conn, &mut state, verified, request).await;
            if conn.send(&response).await.is_err() {
                break 'outer;
            }
            match after {
                After::Nothing => {}
                After::StartEncryption { read, write, controller } => {
                    conn.writer.lock().await.cipher = Some(FrameCipher::new(read));
                    inbound = Some(FrameCipher::new(write));
                    // Anything already buffered arrived encrypted.
                    raw.append(&mut plain);
                    *conn.controller.lock().unwrap() = Some(controller);
                    if let Some(cipher) = inbound.as_mut() {
                        match cipher.decrypt(&mut raw) {
                            Ok(bytes) => plain.extend(bytes),
                            Err(_) => break 'outer,
                        }
                    }
                }
                After::CloseControllers(ids) => close_controllers(&shared, &ids),
            }
        }
    }
    shared.connections.lock().unwrap().remove(&id);
}

fn close_controllers(shared: &Shared, ids: &[String]) {
    for c in shared.connections.lock().unwrap().values() {
        let controller = c.controller.lock().unwrap().clone();
        if controller.is_some_and(|id| ids.contains(&id)) {
            c.close.cancel();
        }
    }
}

async fn route(
    shared: &Arc<Shared>,
    conn: &Arc<Connection>,
    state: &mut ConnState,
    verified: bool,
    req: Request,
) -> (Vec<u8>, After) {
    shared.log(|| {
        format!(
            "{} {}{}",
            req.method,
            req.path,
            if req.query.is_empty() { String::new() } else { format!("?{}", req.query) }
        )
    });
    let Ok(body) = (if req.path.starts_with("/pair") { Tlv::decode(&req.body).map(Some) } else { Ok(None) }) else {
        return (http(400, "Bad Request", None, b""), After::Nothing);
    };
    match (req.method.as_str(), req.path.as_str(), body) {
        ("POST", "/pair-setup", Some(t)) => (tlv_response(pair_setup(shared, state, &t)), After::Nothing),
        ("POST", "/pair-verify", Some(t)) => pair_verify(shared, state, &t),
        ("POST", "/identify", _) if !shared.paired() => {
            tracing::info!("[HomeKit] Identify requested");
            (no_content(), After::Nothing)
        }
        _ if !verified => (
            json_response(470, "Connection Authorization Required", &json!({"status": STATUS_INSUFFICIENT_PRIVILEGES})),
            After::Nothing,
        ),
        ("POST", "/pairings", Some(t)) => pairings(shared, conn, &t),
        ("GET", "/accessories", _) => {
            let body = shared.db.read().unwrap().to_json();
            (json_response(200, "OK", &body), After::Nothing)
        }
        ("GET", "/characteristics", _) => (read_characteristics(shared, conn, &req.query), After::Nothing),
        ("PUT", "/characteristics", _) => (write_characteristics(shared, conn, &req.body), After::Nothing),
        _ => (http(404, "Not Found", None, b""), After::Nothing),
    }
}

// ---------- pair-setup ----------

fn pair_setup(shared: &Shared, state: &mut ConnState, t: &Tlv) -> Tlv {
    let step = t.byte(tlv::STATE).unwrap_or(0);
    match step {
        1 => {
            if shared.paired() {
                return Tlv::error(2, tlv::ERROR_UNAVAILABLE);
            }
            if shared.failed_setups.load(Ordering::SeqCst) >= MAX_SETUP_ATTEMPTS {
                return Tlv::error(2, 0x05);
            }
            let salt = random::<16>();
            let server = srp::Server::new(&salt, shared.pin.as_bytes(), &random::<32>());
            let reply = Tlv::state(2).with(tlv::SALT, salt).with(tlv::PUBLIC_KEY, server.public_key());
            state.setup = Some((server, None));
            shared.log(|| "Pair setup: M1 → M2".into());
            reply
        }
        3 => {
            let (Some((server, key)), Some(a), Some(proof)) =
                (state.setup.as_mut(), t.get(tlv::PUBLIC_KEY), t.get(tlv::PROOF))
            else {
                return Tlv::error(4, tlv::ERROR_UNKNOWN);
            };
            match server.verify(a, proof) {
                Some(session) => {
                    *key = Some(session.key);
                    shared.log(|| "Pair setup: M3 → M4 (code accepted)".into());
                    Tlv::state(4).with(tlv::PROOF, session.server_proof)
                }
                None => {
                    shared.failed_setups.fetch_add(1, Ordering::SeqCst);
                    state.setup = None;
                    tracing::warn!("[HomeKit] Pairing refused: wrong setup code.");
                    Tlv::error(4, tlv::ERROR_AUTHENTICATION)
                }
            }
        }
        5 => {
            let Some((_, Some(key))) = state.setup.take() else { return Tlv::error(6, tlv::ERROR_UNKNOWN) };
            match finish_setup(shared, &key, t) {
                Some(reply) => reply,
                None => Tlv::error(6, tlv::ERROR_AUTHENTICATION),
            }
        }
        _ => Tlv::error(step.wrapping_add(1), tlv::ERROR_UNKNOWN),
    }
}

fn finish_setup(shared: &Shared, key: &[u8], t: &Tlv) -> Option<Tlv> {
    let session_key = hkdf_sha512(key, "Pair-Setup-Encrypt-Salt", "Pair-Setup-Encrypt-Info");
    let sub = Tlv::decode(&open(&session_key, &label_nonce(b"PS-Msg05"), b"", t.get(tlv::ENCRYPTED_DATA)?)?).ok()?;
    let controller_id = sub.get(tlv::IDENTIFIER)?.to_vec();
    let controller_ltpk: [u8; 32] = sub.get(tlv::PUBLIC_KEY)?.try_into().ok()?;
    let signature = Signature::from_slice(sub.get(tlv::SIGNATURE)?).ok()?;
    let controller_x = hkdf_sha512(key, "Pair-Setup-Controller-Sign-Salt", "Pair-Setup-Controller-Sign-Info");
    let mut signed = controller_x.to_vec();
    signed.extend_from_slice(&controller_id);
    signed.extend_from_slice(&controller_ltpk);
    VerifyingKey::from_bytes(&controller_ltpk).ok()?.verify(&signed, &signature).ok()?;

    let id = String::from_utf8(controller_id).ok()?;
    let (username, ltpk, signature) = {
        let mut info = shared.info.lock().unwrap();
        info.paired.insert(id.clone(), Pairing { public_key: controller_ltpk, permission: PERMISSION_ADMIN });
        if let Err(e) = info.save() {
            tracing::warn!("[HomeKit] Could not save the pairing: {e}");
        }
        let accessory_x = hkdf_sha512(key, "Pair-Setup-Accessory-Sign-Salt", "Pair-Setup-Accessory-Sign-Info");
        let ltpk = info.signing.verifying_key().to_bytes();
        let mut material = accessory_x.to_vec();
        material.extend_from_slice(info.username.as_bytes());
        material.extend_from_slice(&ltpk);
        (info.username.clone(), ltpk, info.signing.sign(&material).to_bytes())
    };
    tracing::info!("[HomeKit] Paired with controller {id}.");
    shared.pairing_changed.notify_waiters();
    let sub = Tlv::new()
        .with(tlv::IDENTIFIER, username.as_bytes())
        .with(tlv::PUBLIC_KEY, ltpk)
        .with(tlv::SIGNATURE, signature);
    let encrypted = seal(&session_key, &label_nonce(b"PS-Msg06"), b"", &sub.encode());
    Some(Tlv::state(6).with(tlv::ENCRYPTED_DATA, encrypted))
}

// ---------- pair-verify ----------

fn pair_verify(shared: &Shared, state: &mut ConnState, t: &Tlv) -> (Vec<u8>, After) {
    match t.byte(tlv::STATE).unwrap_or(0) {
        1 => {
            let Some(controller_public) = t.get(tlv::PUBLIC_KEY).and_then(|k| <[u8; 32]>::try_from(k).ok()) else {
                return (tlv_response(Tlv::error(2, tlv::ERROR_UNKNOWN)), After::Nothing);
            };
            let secret = x25519_dalek::StaticSecret::from(random::<32>());
            let accessory_public = x25519_dalek::PublicKey::from(&secret).to_bytes();
            let shared_secret = secret.diffie_hellman(&x25519_dalek::PublicKey::from(controller_public)).to_bytes();
            let (username, signature) = {
                let info = shared.info.lock().unwrap();
                let mut material = accessory_public.to_vec();
                material.extend_from_slice(info.username.as_bytes());
                material.extend_from_slice(&controller_public);
                (info.username.clone(), info.signing.sign(&material).to_bytes())
            };
            let session_key = hkdf_sha512(&shared_secret, "Pair-Verify-Encrypt-Salt", "Pair-Verify-Encrypt-Info");
            let sub = Tlv::new().with(tlv::IDENTIFIER, username.as_bytes()).with(tlv::SIGNATURE, signature);
            let encrypted = seal(&session_key, &label_nonce(b"PV-Msg02"), b"", &sub.encode());
            state.verify = Some(VerifyState { shared_secret, accessory_public, controller_public });
            let reply = Tlv::state(2).with(tlv::PUBLIC_KEY, accessory_public).with(tlv::ENCRYPTED_DATA, encrypted);
            (tlv_response(reply), After::Nothing)
        }
        3 => {
            let Some(v) = state.verify.take() else {
                return (tlv_response(Tlv::error(4, tlv::ERROR_UNKNOWN)), After::Nothing);
            };
            match finish_verify(shared, &v, t) {
                Some(controller) => {
                    shared.log(|| format!("Pair verify: session with {controller}"));
                    let read = hkdf_sha512(&v.shared_secret, "Control-Salt", "Control-Read-Encryption-Key");
                    let write = hkdf_sha512(&v.shared_secret, "Control-Salt", "Control-Write-Encryption-Key");
                    (tlv_response(Tlv::state(4)), After::StartEncryption { read, write, controller })
                }
                None => (tlv_response(Tlv::error(4, tlv::ERROR_AUTHENTICATION)), After::Nothing),
            }
        }
        other => (tlv_response(Tlv::error(other.wrapping_add(1), tlv::ERROR_UNKNOWN)), After::Nothing),
    }
}

fn finish_verify(shared: &Shared, v: &VerifyState, t: &Tlv) -> Option<String> {
    let session_key = hkdf_sha512(&v.shared_secret, "Pair-Verify-Encrypt-Salt", "Pair-Verify-Encrypt-Info");
    let sub = Tlv::decode(&open(&session_key, &label_nonce(b"PV-Msg03"), b"", t.get(tlv::ENCRYPTED_DATA)?)?).ok()?;
    let id = String::from_utf8(sub.get(tlv::IDENTIFIER)?.to_vec()).ok()?;
    let signature = Signature::from_slice(sub.get(tlv::SIGNATURE)?).ok()?;
    let ltpk = shared.info.lock().unwrap().paired.get(&id)?.public_key;
    let mut material = v.controller_public.to_vec();
    material.extend_from_slice(id.as_bytes());
    material.extend_from_slice(&v.accessory_public);
    VerifyingKey::from_bytes(&ltpk).ok()?.verify(&material, &signature).ok()?;
    Some(id)
}

// ---------- pairings ----------

fn pairings(shared: &Shared, conn: &Connection, t: &Tlv) -> (Vec<u8>, After) {
    let caller = conn.controller.lock().unwrap().clone().unwrap_or_default();
    let mut info = shared.info.lock().unwrap();
    if info.paired.get(&caller).map(|p| p.permission) != Some(PERMISSION_ADMIN) {
        return (tlv_response(Tlv::error(2, tlv::ERROR_AUTHENTICATION)), After::Nothing);
    }
    match t.byte(tlv::METHOD) {
        Some(3) => {
            let (Some(id), Some(pk)) =
                (t.get(tlv::IDENTIFIER), t.get(tlv::PUBLIC_KEY).and_then(|k| <[u8; 32]>::try_from(k).ok()))
            else {
                return (tlv_response(Tlv::error(2, tlv::ERROR_UNKNOWN)), After::Nothing);
            };
            let id = String::from_utf8_lossy(id).into_owned();
            let permission = t.byte(tlv::PERMISSIONS).unwrap_or(0);
            let count = info.paired.len();
            match info.paired.get_mut(&id) {
                Some(existing) if existing.public_key != pk => {
                    return (tlv_response(Tlv::error(2, tlv::ERROR_UNKNOWN)), After::Nothing);
                }
                Some(existing) => existing.permission = permission,
                None if count >= MAX_PAIRINGS => {
                    return (tlv_response(Tlv::error(2, tlv::ERROR_MAX_PEERS)), After::Nothing);
                }
                None => {
                    info.paired.insert(id.clone(), Pairing { public_key: pk, permission });
                }
            }
            let _ = info.save();
            tracing::info!("[HomeKit] Added pairing {id}.");
            (tlv_response(Tlv::state(2)), After::Nothing)
        }
        Some(4) => {
            let id = t.get(tlv::IDENTIFIER).map(|i| String::from_utf8_lossy(i).into_owned()).unwrap_or_default();
            let mut removed = Vec::new();
            if info.paired.remove(&id).is_some() {
                removed.push(id.clone());
            }
            if !info.has_admin() {
                // The last admin is gone: the bridge is unpaired and can pair again.
                removed.extend(std::mem::take(&mut info.paired).into_keys());
            }
            let _ = info.save();
            drop(info);
            tracing::info!("[HomeKit] Removed pairing {id}.");
            shared.pairing_changed.notify_waiters();
            (tlv_response(Tlv::state(2)), After::CloseControllers(removed))
        }
        Some(5) => {
            let mut reply = Tlv::state(2);
            for (i, (id, p)) in info.paired.iter().enumerate() {
                if i > 0 {
                    reply.push(tlv::SEPARATOR, []);
                }
                reply.push(tlv::IDENTIFIER, id.as_bytes());
                reply.push(tlv::PUBLIC_KEY, p.public_key);
                reply.push(tlv::PERMISSIONS, [p.permission]);
            }
            (tlv_response(reply), After::Nothing)
        }
        _ => (tlv_response(Tlv::error(2, tlv::ERROR_UNKNOWN)), After::Nothing),
    }
}

// ---------- characteristics ----------

fn parse_ids(query: &str) -> (Vec<(u64, u64)>, HashSet<String>) {
    let mut ids = Vec::new();
    let mut flags = HashSet::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "id" {
            for pair in value.split(',') {
                if let Some((a, i)) = pair.split_once('.')
                    && let (Ok(a), Ok(i)) = (a.trim().parse(), i.trim().parse())
                {
                    ids.push((a, i));
                }
            }
        } else if value == "1" {
            flags.insert(key.into_owned());
        }
    }
    (ids, flags)
}

fn read_characteristics(shared: &Shared, conn: &Connection, query: &str) -> Vec<u8> {
    let (ids, flags) = parse_ids(query);
    let db = shared.db.read().unwrap();
    let events = conn.events.lock().unwrap();
    let mut all_ok = true;
    let items: Vec<Value> = ids
        .iter()
        .map(|&(aid, iid)| {
            let mut item = json!({"aid": aid, "iid": iid});
            match db.find(aid, iid) {
                None => {
                    all_ok = false;
                    item["status"] = json!(STATUS_NOT_FOUND);
                }
                Some(c) if !c.readable() => {
                    all_ok = false;
                    item["status"] = json!(STATUS_WRITE_ONLY);
                }
                Some(c) => {
                    item["value"] = c.value.clone();
                    item["status"] = json!(0);
                    if flags.contains("type") {
                        item["type"] = json!(c.kind);
                    }
                    if flags.contains("perms") {
                        item["perms"] = json!(c.perms);
                    }
                    if flags.contains("meta") {
                        item["format"] = json!(c.format);
                    }
                    if flags.contains("ev") {
                        item["ev"] = json!(events.contains(&(aid, iid)));
                    }
                }
            }
            item
        })
        .collect();
    let items: Vec<Value> = if all_ok {
        items
            .into_iter()
            .map(|mut v| {
                v.as_object_mut().unwrap().remove("status");
                v
            })
            .collect()
    } else {
        items
    };
    let body = json!({ "characteristics": items });
    if all_ok { json_response(200, "OK", &body) } else { json_response(207, "Multi-Status", &body) }
}

fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => match s.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn write_characteristics(shared: &Arc<Shared>, conn: &Arc<Connection>, body: &[u8]) -> Vec<u8> {
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return json_response(400, "Bad Request", &json!({"status": STATUS_INVALID_VALUE}));
    };
    let writes = request.get("characteristics").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut results = Vec::new();
    let mut all_ok = true;
    let mut want_values = false;
    let mut touched = Vec::new();
    for w in &writes {
        let (Some(aid), Some(iid)) = (w.get("aid").and_then(Value::as_u64), w.get("iid").and_then(Value::as_u64))
        else {
            continue;
        };
        let status = {
            let db = shared.db.read().unwrap();
            match db.find(aid, iid).cloned() {
                None => STATUS_NOT_FOUND,
                Some(c) => {
                    let mut status = 0;
                    if let Some(ev) = w.get("ev").and_then(as_bool) {
                        if c.notifies() {
                            let mut events = conn.events.lock().unwrap();
                            if ev {
                                events.insert((aid, iid))
                            } else {
                                events.remove(&(aid, iid))
                            };
                        } else {
                            status = STATUS_NO_NOTIFICATION;
                        }
                    }
                    if let Some(value) = w.get("value") {
                        if !c.writable() {
                            status = STATUS_READ_ONLY;
                        } else {
                            status = match (&c.role, as_bool(value)) {
                                (Role::Identify, _) => {
                                    tracing::info!("[HomeKit] Identify {aid}");
                                    0
                                }
                                (Role::On(index), Some(on)) => {
                                    touched.push(*index);
                                    light_on(shared, &db, *index, on);
                                    0
                                }
                                (Role::Brightness(index), _) => match value.as_f64() {
                                    Some(level) => {
                                        touched.push(*index);
                                        light_brightness(shared, &db, *index, level.clamp(0.0, 100.0).round() as u8);
                                        0
                                    }
                                    None => STATUS_INVALID_VALUE,
                                },
                                (Role::On(_), None) => STATUS_INVALID_VALUE,
                                (Role::Static, _) => STATUS_READ_ONLY,
                            };
                        }
                    }
                    status
                }
            }
        };
        let mut item = json!({"aid": aid, "iid": iid, "status": status});
        if w.get("r").and_then(as_bool) == Some(true) {
            want_values = true;
            if let Some(c) = shared.db.read().unwrap().find(aid, iid) {
                item["value"] = c.value.clone();
            }
        }
        all_ok &= status == 0;
        results.push(item);
    }
    touched.sort_unstable();
    touched.dedup();
    for index in touched {
        schedule_reset(shared.clone(), index);
    }
    if all_ok && !want_values {
        no_content()
    } else {
        json_response(207, "Multi-Status", &json!({ "characteristics": results }))
    }
}

/// On: off runs right away; on runs right away unless the item takes parameters, in which
/// case it waits 800 ms for a possible brightness change.
fn light_on(shared: &Arc<Shared>, db: &Database, index: usize, on: bool) {
    let Some(entry) = db.lights.get(index).cloned() else { return };
    let mut lights = shared.lights.lock().unwrap();
    let state = lights.entry(index).or_default();
    if let Some(t) = state.pending_on.take() {
        t.abort();
    }
    if !on {
        (shared.runner)(entry, false, None);
    } else if entry.allow_params {
        if state.suppress_until.is_some_and(|t| Instant::now() < t) {
            return;
        }
        let runner = shared.runner.clone();
        state.pending_on = Some(tokio::spawn(async move {
            tokio::time::sleep(ON_DELAY).await;
            runner(entry, true, None);
        }));
    } else {
        (shared.runner)(entry, true, None);
    }
}

/// Brightness passes the level to items that take parameters.
fn light_brightness(shared: &Arc<Shared>, db: &Database, index: usize, level: u8) {
    let Some(entry) = db.lights.get(index).cloned() else { return };
    if !entry.allow_params {
        return;
    }
    let mut lights = shared.lights.lock().unwrap();
    let state = lights.entry(index).or_default();
    if let Some(t) = state.pending_on.take() {
        t.abort();
    }
    state.suppress_until = Some(Instant::now() + SUPPRESS_ON);
    (shared.runner)(entry, true, Some(level));
}

/// Snap the light back to off / full brightness and tell every subscriber.
fn schedule_reset(shared: Arc<Shared>, index: usize) {
    tokio::spawn(async move {
        tokio::time::sleep(RESET_AFTER).await;
        let Some((aid, on_iid, brightness_iid)) = shared.db.read().unwrap().light_ids(index) else { return };
        notify(&shared, &[(aid, on_iid, json!(RESTING_ON)), (aid, brightness_iid, json!(RESTING_BRIGHTNESS))]).await;
    });
}

async fn notify(shared: &Shared, changes: &[(u64, u64, Value)]) {
    let targets: Vec<(Arc<Connection>, Vec<Value>)> = shared
        .connections
        .lock()
        .unwrap()
        .values()
        .filter_map(|c| {
            let events = c.events.lock().unwrap();
            let items: Vec<Value> = changes
                .iter()
                .filter(|(a, i, _)| events.contains(&(*a, *i)))
                .map(|(a, i, v)| json!({"aid": a, "iid": i, "value": v}))
                .collect();
            (!items.is_empty()).then(|| (c.clone(), items))
        })
        .collect();
    for (conn, items) in targets {
        let _ = conn.send(&event_frame(&json!({ "characteristics": items }))).await;
    }
}
