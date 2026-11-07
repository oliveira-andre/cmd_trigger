//! The phone link end to end, with this test playing the iPhone.

use super::*;
use crate::agent::Ground;

struct Phone {
    base: String,
    id: String,
    key: [u8; 32],
    http: reqwest::Client,
}

impl Phone {
    async fn call_raw(&self, request: Value) -> (StatusCode, Value, Vec<u8>) {
        let sealed = crypto::seal(&self.key, "call", &self.id, request.to_string().as_bytes());
        let reply = self
            .http
            .post(format!("{}/phone/v1/call", self.base))
            .json(&json!({ "phone": self.id, "sealed": B64.encode(&sealed) }))
            .send()
            .await
            .unwrap();
        let status = StatusCode::from_u16(reply.status().as_u16()).unwrap();
        let body: Value = reply.json().await.unwrap();
        let opened = body["sealed"]
            .as_str()
            .and_then(|s| B64.decode(s).ok())
            .and_then(|s| crypto::open(&self.key, "reply", &self.id, &s))
            .map(|(plain, _)| serde_json::from_slice(&plain).unwrap())
            .unwrap_or(body);
        (status, opened, sealed)
    }

    async fn call(&self, mut request: Value) -> Value {
        request["at"] = json!(now_ms() / 1000);
        let (status, reply, _) = self.call_raw(request).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        reply
    }
}

async fn pair(
    http: &reqwest::Client,
    base: &str,
    code: &str,
    phone_id: &str,
) -> (StatusCode, Value, [u8; 32], [u8; 32]) {
    let secret = x25519_dalek::StaticSecret::from(crypto::random::<32>());
    let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let code_key = crypto::code_key(&crypto::normalize_code(code).unwrap());
    let proof = crypto::phone_proof(&code_key, &public, phone_id);
    let reply = http
        .post(format!("{base}/phone/v1/pair"))
        .json(&json!({ "phoneId": phone_id, "phoneName": "Test iPhone", "phonePublic": B64.encode(public), "proof": B64.encode(proof) }))
        .send()
        .await
        .unwrap();
    let status = StatusCode::from_u16(reply.status().as_u16()).unwrap();
    let body: Value = reply.json().await.unwrap();
    if status != StatusCode::OK {
        return (status, body, [0; 32], code_key);
    }
    let mac_public: [u8; 32] = B64.decode(body["macPublic"].as_str().unwrap()).unwrap().try_into().unwrap();
    assert_eq!(
        B64.decode(body["proof"].as_str().unwrap()).unwrap(),
        crypto::mac_proof(&code_key, &mac_public, &public),
        "the computer proves it knows the code too"
    );
    let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(mac_public)).to_bytes();
    (status, body, crypto::derive_key(&code_key, &shared, &public, &mac_public), code_key)
}

#[tokio::test]
async fn pairs_and_runs_over_sealed_calls() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(dir.path());
    std::fs::write(
        data.commands(),
        r#"[{"name": "Say", "command": "echo \"from:$CMDTRIGGER_SOURCE\"; true", "allowParams": true},
            {"name": "Fail", "command": "exit 4"}]"#,
    )
    .unwrap();
    std::fs::write(data.settings(), r#"{"computerName": "Test Mac"}"#).unwrap();
    // No local API: nothing to reach for a background service.
    std::fs::write(data.local_api(), r#"{"enabled": false}"#).unwrap();
    let runner = Runner::new(data.clone(), Ground::Foreground);
    let link = PhoneLink::start(&data, runner, Some(0)).await.unwrap().unwrap();
    let base = format!("http://127.0.0.1:{}", link.addr.port());
    let http = reqwest::Client::new();

    let hello: Value = http.get(format!("{base}/phone/v1/hello")).send().await.unwrap().json().await.unwrap();
    assert_eq!(hello["name"], "Test Mac");
    assert_eq!(hello["pairing"], false);

    // No window open → refused, whatever the code.
    let (status, _, _, _) = pair(&http, &base, "0000-0000-0000-0000", "phone-1").await;
    assert_eq!(status, StatusCode::CONFLICT);

    let info = link.start_pairing().await;
    assert!(info.url.starts_with("cmdtrigger://pair?v=1&id="), "{}", info.url);
    assert!(info.qr_svg.contains("<svg"), "QR code rendered");
    let (status, body, _, _) = pair(&http, &base, "0000-0000-0000-0000", "phone-1").await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("wrong code")));

    let (status, body, key, _) = pair(&http, &base, &info.code.to_lowercase(), "phone-1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["macName"], "Test Mac");
    assert_eq!(link.pairing().unwrap().paired_with.as_deref(), Some("Test iPhone"));
    let (status, _, _, _) = pair(&http, &base, &info.code, "phone-2").await;
    assert_eq!(status, StatusCode::CONFLICT, "a code works once");
    let saved = PhoneConfig::load(&data);
    assert_eq!(saved.phones.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(data.phones()).unwrap().permissions().mode() & 0o777, 0o600);
    }

    let phone = Phone { base: base.clone(), id: "phone-1".into(), key, http: http.clone() };
    let status = phone.call(json!({ "op": "status" })).await;
    assert_eq!(status["name"], "Test Mac");
    let names: Vec<&str> = status["catalog"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Say", "Fail"]);
    assert!(status["interfaces"].is_array());

    let ran = phone.call(json!({ "op": "run", "kind": "command", "target": "Say", "params": "hi" })).await;
    assert_eq!(ran["run"]["status"], "succeeded");
    assert_eq!(ran["run"]["output"].as_str().unwrap().trim(), "from:phone");
    let failed = phone.call(json!({ "op": "run", "kind": "command", "target": "Fail" })).await;
    assert_eq!((failed["run"]["status"].as_str(), failed["run"]["exitCode"].as_i64()), (Some("failed"), Some(4)));
    let missing = phone.call(json!({ "op": "run", "kind": "command", "target": "Nope" })).await;
    assert!(missing["error"].as_str().unwrap().contains("Nope"));
    let runs = phone.call(json!({ "op": "runs", "limit": 10 })).await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 2);
    assert_eq!(runs["runs"][0]["source"], "phone");

    // A recorded call can't be played again.
    let request = json!({ "op": "status", "at": now_ms() / 1000 });
    let (status, _, sealed) = phone.call_raw(request).await;
    assert_eq!(status, StatusCode::OK);
    let replay = http
        .post(format!("{base}/phone/v1/call"))
        .json(&json!({ "phone": "phone-1", "sealed": B64.encode(&sealed) }))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status().as_u16(), 409);

    // Old timestamps, strangers and wrong keys are refused.
    let (status, _, _) = phone.call_raw(json!({ "op": "status", "at": now_ms() / 1000 - 3600 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let stranger = Phone { base: base.clone(), id: "phone-9".into(), key, http: http.clone() };
    assert_eq!(stranger.call_raw(json!({ "op": "status", "at": now_ms() / 1000 })).await.0, StatusCode::UNAUTHORIZED);
    let forger = Phone { base: base.clone(), id: "phone-1".into(), key: [7; 32], http: http.clone() };
    assert_eq!(forger.call_raw(json!({ "op": "status", "at": now_ms() / 1000 })).await.0, StatusCode::UNAUTHORIZED);

    // Unpairing from the phone forgets it.
    phone.call(json!({ "op": "unpair" })).await;
    assert!(PhoneConfig::load(&data).phones.is_empty());
    assert_eq!(phone.call_raw(json!({ "op": "status", "at": now_ms() / 1000 })).await.0, StatusCode::UNAUTHORIZED);
    link.stop();
}

#[tokio::test]
async fn too_many_wrong_codes_close_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(dir.path());
    std::fs::write(data.commands(), "[]").unwrap();
    let link = PhoneLink::start(&data, Runner::new(data.clone(), Ground::Foreground), Some(0)).await.unwrap().unwrap();
    let base = format!("http://127.0.0.1:{}", link.addr.port());
    let http = reqwest::Client::new();
    let info = link.start_pairing().await;
    for _ in 0..MAX_ATTEMPTS - 1 {
        assert_eq!(pair(&http, &base, "0000-0000-0000-0000", "p").await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(pair(&http, &base, "0000-0000-0000-0000", "p").await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(pair(&http, &base, &info.code, "p").await.0, StatusCode::CONFLICT, "even the right code is too late");
    assert!(link.pairing().is_none());
}

#[test]
fn a_disabled_link_does_not_listen() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::at(dir.path());
    PhoneConfig { enabled: false, ..PhoneConfig::default() }.save(&data).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let link = rt.block_on(PhoneLink::start(&data, Runner::new(data.clone(), Ground::Foreground), Some(0))).unwrap();
    assert!(link.is_none());
    assert_eq!(PhoneConfig::load(&data).mac_id.len(), 16, "the computer still gets an id");
}
