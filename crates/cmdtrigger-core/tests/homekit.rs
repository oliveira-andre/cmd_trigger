//! The HomeKit bridge end to end, with a test controller doing pair setup, pair verify and an
//! encrypted session. Runs on every platform (no shell commands).

use cmdtrigger_core::config::{HomeKitConfig, Settings};
use cmdtrigger_core::homekit::crypto::{FrameCipher, hkdf_sha512, label_nonce, open, seal};
use cmdtrigger_core::homekit::tlv::{self, Tlv};
use cmdtrigger_core::homekit::{HomeKitBridge, srp};
use cmdtrigger_core::runner::Runner;
use cmdtrigger_core::{Command, DataDir, Ground, store};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn eventually<F: FnMut() -> bool>(what: &str, mut check: F) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

// ---------- HomeKit ----------

struct Hap {
    stream: TcpStream,
    read: Option<FrameCipher>,
    write: Option<FrameCipher>,
    buffer: Vec<u8>,
}

struct Reply {
    status_line: String,
    body: Vec<u8>,
}

impl Hap {
    async fn connect(port: u16) -> Self {
        Self {
            stream: TcpStream::connect(("127.0.0.1", port)).await.unwrap(),
            read: None,
            write: None,
            buffer: Vec::new(),
        }
    }

    async fn send(&mut self, method: &str, path: &str, content_type: &str, body: &[u8]) {
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: bridge\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        req.extend_from_slice(body);
        let wire = match self.write.as_mut() {
            Some(c) => c.encrypt(&req),
            None => req,
        };
        self.stream.write_all(&wire).await.unwrap();
    }

    async fn next_message(&mut self) -> Reply {
        let mut raw = Vec::new();
        loop {
            if let Some(end) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&self.buffer[..end]).to_string();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if self.buffer.len() >= end + 4 + length {
                    let body = self.buffer[end + 4..end + 4 + length].to_vec();
                    self.buffer.drain(..end + 4 + length);
                    return Reply { status_line: head.lines().next().unwrap().to_string(), body };
                }
            }
            let mut chunk = [0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(10), self.stream.read(&mut chunk)).await.unwrap().unwrap();
            assert!(n > 0, "bridge closed the connection");
            match self.read.as_mut() {
                Some(c) => {
                    raw.extend_from_slice(&chunk[..n]);
                    self.buffer.extend(c.decrypt(&mut raw).unwrap());
                }
                None => self.buffer.extend_from_slice(&chunk[..n]),
            }
        }
    }

    async fn tlv(&mut self, path: &str, t: Tlv) -> Tlv {
        self.send("POST", path, "application/pairing+tlv8", &t.encode()).await;
        let reply = self.next_message().await;
        assert!(reply.status_line.contains("200"), "{}", reply.status_line);
        Tlv::decode(&reply.body).unwrap()
    }

    async fn json(&mut self, method: &str, path: &str, body: Option<Value>) -> (String, Value) {
        let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        self.send(method, path, "application/hap+json", &bytes).await;
        let reply = self.next_message().await;
        let value = if reply.body.is_empty() { Value::Null } else { serde_json::from_slice(&reply.body).unwrap() };
        (reply.status_line, value)
    }
}

const PIN: &str = "031-45-154";
const CONTROLLER_ID: &str = "1B6E5D3C-AAAA-BBBB-CCCC-123456789ABC";

async fn pair(port: u16, controller: &SigningKey) -> Result<(), u8> {
    let mut hap = Hap::connect(port).await;
    let m2 = hap.tlv("/pair-setup", Tlv::state(1).with(tlv::METHOD, [0])).await;
    if let Some(e) = m2.byte(tlv::ERROR) {
        return Err(e);
    }
    let client = srp::Client::new(&[0x42; 32]);
    let (proof, key) = client.proof(m2.get(tlv::SALT).unwrap(), PIN.as_bytes(), m2.get(tlv::PUBLIC_KEY).unwrap());
    let m4 =
        hap.tlv("/pair-setup", Tlv::state(3).with(tlv::PUBLIC_KEY, &client.public_a).with(tlv::PROOF, &proof)).await;
    if let Some(e) = m4.byte(tlv::ERROR) {
        return Err(e);
    }
    let mut expected = client.public_a.clone();
    expected.extend(&proof);
    expected.extend(&key);
    assert_eq!(m4.get(tlv::PROOF).unwrap(), &<sha2::Sha512 as sha2::Digest>::digest(&expected)[..]);

    let session_key = hkdf_sha512(&key, "Pair-Setup-Encrypt-Salt", "Pair-Setup-Encrypt-Info");
    let x = hkdf_sha512(&key, "Pair-Setup-Controller-Sign-Salt", "Pair-Setup-Controller-Sign-Info");
    let ltpk = controller.verifying_key().to_bytes();
    let mut material = x.to_vec();
    material.extend(CONTROLLER_ID.as_bytes());
    material.extend(ltpk);
    let sub = Tlv::new()
        .with(tlv::IDENTIFIER, CONTROLLER_ID.as_bytes())
        .with(tlv::PUBLIC_KEY, ltpk)
        .with(tlv::SIGNATURE, controller.sign(&material).to_bytes());
    let encrypted = seal(&session_key, &label_nonce(b"PS-Msg05"), b"", &sub.encode());
    let m6 = hap.tlv("/pair-setup", Tlv::state(5).with(tlv::ENCRYPTED_DATA, encrypted)).await;
    if let Some(e) = m6.byte(tlv::ERROR) {
        return Err(e);
    }
    let inner =
        Tlv::decode(&open(&session_key, &label_nonce(b"PS-Msg06"), b"", m6.get(tlv::ENCRYPTED_DATA).unwrap()).unwrap())
            .unwrap();
    let accessory_id = inner.get(tlv::IDENTIFIER).unwrap();
    let accessory_ltpk: [u8; 32] = inner.get(tlv::PUBLIC_KEY).unwrap().try_into().unwrap();
    let mut signed = hkdf_sha512(&key, "Pair-Setup-Accessory-Sign-Salt", "Pair-Setup-Accessory-Sign-Info").to_vec();
    signed.extend(accessory_id);
    signed.extend(accessory_ltpk);
    let sig = ed25519_dalek::Signature::from_slice(inner.get(tlv::SIGNATURE).unwrap()).unwrap();
    VerifyingKey::from_bytes(&accessory_ltpk).unwrap().verify(&signed, &sig).expect("accessory signature");
    Ok(())
}

async fn verify(port: u16, controller: &SigningKey) -> Hap {
    let mut hap = Hap::connect(port).await;
    let secret = x25519_dalek::StaticSecret::from([0x24u8; 32]);
    let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let m2 = hap.tlv("/pair-verify", Tlv::state(1).with(tlv::PUBLIC_KEY, public)).await;
    let accessory_public: [u8; 32] = m2.get(tlv::PUBLIC_KEY).unwrap().try_into().unwrap();
    let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(accessory_public)).to_bytes();
    let session_key = hkdf_sha512(&shared, "Pair-Verify-Encrypt-Salt", "Pair-Verify-Encrypt-Info");
    let inner =
        Tlv::decode(&open(&session_key, &label_nonce(b"PV-Msg02"), b"", m2.get(tlv::ENCRYPTED_DATA).unwrap()).unwrap())
            .unwrap();
    assert!(inner.get(tlv::SIGNATURE).is_some());
    let mut material = public.to_vec();
    material.extend(CONTROLLER_ID.as_bytes());
    material.extend(accessory_public);
    let sub = Tlv::new()
        .with(tlv::IDENTIFIER, CONTROLLER_ID.as_bytes())
        .with(tlv::SIGNATURE, controller.sign(&material).to_bytes());
    let m4 = hap
        .tlv(
            "/pair-verify",
            Tlv::state(3).with(tlv::ENCRYPTED_DATA, seal(&session_key, &label_nonce(b"PV-Msg03"), b"", &sub.encode())),
        )
        .await;
    assert_eq!(m4.byte(tlv::STATE), Some(4));
    assert!(m4.byte(tlv::ERROR).is_none());
    hap.write = Some(FrameCipher::new(hkdf_sha512(&shared, "Control-Salt", "Control-Write-Encryption-Key")));
    hap.read = Some(FrameCipher::new(hkdf_sha512(&shared, "Control-Salt", "Control-Read-Encryption-Key")));
    hap
}

#[tokio::test(flavor = "multi_thread")]
async fn homekit_pairs_and_turns_commands_into_lights() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(dir.path());
    store::init_data_dir(&data).unwrap();
    Settings { computer_name: "Den".into(), language: "en".into() }.save(&data).unwrap();
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"trigger": "Calculator", "command": "calc", "ground": "foreground"},
        {"trigger": "Volume", "command": "vol", "ground": "foreground", "allowParams": "true"},
        {"trigger": "Reboot", "command": "reboot", "ground": "background"}
    ]))
    .unwrap();
    store::save_commands(&data.commands(), &commands).unwrap();
    let mut config = HomeKitConfig::load(&data);
    config.enabled = true;
    config.pin = PIN.into();
    config.save(&data).unwrap();

    let (ran_tx, mut ran) = tokio::sync::mpsc::unbounded_channel::<(String, bool, Option<u8>)>();
    let runner: cmdtrigger_core::homekit::server::Runner =
        std::sync::Arc::new(move |entry, on, level| ran_tx.send((entry.name, on, level)).unwrap());
    let (changes, _) = tokio::sync::broadcast::channel(4);
    // The foreground agent's catalog: the background command ("Reboot") gets no light.
    let agent_runner = Runner::new(data.clone(), Ground::Foreground);
    let catalog: cmdtrigger_core::homekit::Catalog = std::sync::Arc::new(move || agent_runner.catalog());
    let mut bridge = HomeKitBridge::start(data.clone(), catalog, changes.subscribe(), 0, runner).await;
    let status = bridge.status();
    assert!(status.running, "{status:?}");
    assert_eq!(status.lights, 2);
    let port = status.port;

    // Unpaired: the accessory database is off limits.
    let mut anonymous = Hap::connect(port).await;
    let (line, body) = anonymous.json("GET", "/accessories", None).await;
    assert!(line.contains("470"), "{line}");
    assert_eq!(body["status"], -70401);

    let controller = SigningKey::from_bytes(&[7u8; 32]);
    pair(port, &controller).await.expect("pairing works");
    assert!(bridge.status().paired);
    // A second pairing attempt is refused while paired.
    assert_eq!(pair(port, &SigningKey::from_bytes(&[8u8; 32])).await, Err(tlv::ERROR_UNAVAILABLE));
    // The pairing is stored where hap-nodejs keeps it.
    let username = config.username.replace(':', "");
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(data.homekit_cache().join(format!("AccessoryInfo.{username}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["pairedClients"][CONTROLLER_ID], hex::encode(controller.verifying_key().to_bytes()));

    let mut hap = verify(port, &controller).await;
    let (line, accessories) = hap.json("GET", "/accessories", None).await;
    assert!(line.contains("200"));
    let list = accessories["accessories"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[1]["services"][1]["characteristics"][0]["value"], "Den Calculator");
    let (calc_aid, volume_aid) = (list[1]["aid"].as_u64().unwrap(), list[2]["aid"].as_u64().unwrap());

    // Subscribe to Calculator's On, switch it on: the command runs and the light snaps back.
    let (line, _) = hap
        .json("PUT", "/characteristics", Some(json!({"characteristics": [{"aid": calc_aid, "iid": 10, "ev": true}]})))
        .await;
    assert!(line.contains("204"), "{line}");
    let (line, _) = hap
        .json("PUT", "/characteristics", Some(json!({"characteristics": [{"aid": calc_aid, "iid": 10, "value": 1}]})))
        .await;
    assert!(line.contains("204"), "{line}");
    assert_eq!(ran.recv().await.unwrap(), ("Calculator".into(), true, None));
    let event = hap.next_message().await;
    assert!(event.status_line.starts_with("EVENT/1.0 200"), "{}", event.status_line);
    let event: Value = serde_json::from_slice(&event.body).unwrap();
    assert_eq!(event["characteristics"][0], json!({"aid": calc_aid, "iid": 10, "value": false}));

    // A command with parameters gets the brightness; the "on" that comes with it is folded in.
    let (line, _) = hap
        .json("PUT", "/characteristics", Some(json!({"characteristics": [{"aid": volume_aid, "iid": 11, "value": 40}, {"aid": volume_aid, "iid": 10, "value": true}]})))
        .await;
    assert!(line.contains("204"));
    assert_eq!(ran.recv().await.unwrap(), ("Volume".into(), true, Some(40)));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(ran.try_recv().is_err(), "the on was folded into the brightness change");
    // Off runs right away.
    hap.json(
        "PUT",
        "/characteristics",
        Some(json!({"characteristics": [{"aid": volume_aid, "iid": 10, "value": false}]})),
    )
    .await;
    assert_eq!(ran.recv().await.unwrap(), ("Volume".into(), false, None));

    // Reads, and errors for read-only and unknown characteristics.
    let (line, values) = hap.json("GET", &format!("/characteristics?id={calc_aid}.10,{calc_aid}.11"), None).await;
    assert!(line.contains("200"));
    assert_eq!(values["characteristics"][1]["value"], 100);
    let (line, values) =
        hap.json("PUT", "/characteristics", Some(json!({"characteristics": [{"aid": calc_aid, "iid": 9, "value": "x"}, {"aid": 99, "iid": 1, "value": 1}]}))).await;
    assert!(line.contains("207"));
    assert_eq!(values["characteristics"][0]["status"], -70404);
    assert_eq!(values["characteristics"][1]["status"], -70409);

    // Editing commands.json rebuilds the lights.
    let mut list2 = store::load_commands(&data.commands()).unwrap();
    list2.push(Command::new("Lamp", "lamp", "foreground"));
    store::save_commands(&data.commands(), &list2).unwrap();
    changes.send(()).unwrap();
    eventually("rebuilt", || bridge.status().lights == 3).await;
    let (_, accessories) = hap.json("GET", "/accessories", None).await;
    assert_eq!(accessories["accessories"].as_array().unwrap().len(), 4);

    // Pairings: list, then remove the only admin, which unpairs the bridge.
    let listed = hap.tlv("/pairings", Tlv::state(1).with(tlv::METHOD, [5])).await;
    assert_eq!(listed.get(tlv::IDENTIFIER).unwrap(), CONTROLLER_ID.as_bytes());
    let removed = hap
        .tlv("/pairings", Tlv::state(1).with(tlv::METHOD, [4]).with(tlv::IDENTIFIER, CONTROLLER_ID.as_bytes()))
        .await;
    assert_eq!(removed.byte(tlv::STATE), Some(2));
    eventually("unpaired", || !bridge.status().paired).await;

    // And it can pair again, with a fresh controller.
    pair(port, &SigningKey::from_bytes(&[9u8; 32])).await.expect("pairs again");
    bridge.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn homekit_refuses_a_wrong_code() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(dir.path());
    store::init_data_dir(&data).unwrap();
    let mut config = HomeKitConfig::load(&data);
    config.enabled = true;
    config.pin = "111-22-333".into();
    config.save(&data).unwrap();
    let (changes, _) = tokio::sync::broadcast::channel(4);
    let catalog: cmdtrigger_core::homekit::Catalog = std::sync::Arc::new(Vec::new);
    let bridge = HomeKitBridge::start(data, catalog, changes.subscribe(), 0, std::sync::Arc::new(|_, _, _| {})).await;
    assert_eq!(pair(bridge.status().port, &SigningKey::from_bytes(&[7u8; 32])).await, Err(tlv::ERROR_AUTHENTICATION));
    assert!(!bridge.status().paired);
}
