//! The real agent against the real server: sign-in, registration, catalog, runs from an API
//! token (waiting for output), MCP over HTTP, offline handling, and removal.
#![cfg(unix)]

use cmdtrigger_core::agent::Options;
use cmdtrigger_core::link::{self, LinkStatus};
use cmdtrigger_core::{Agent, DataDir, Ground};
use cmdtrigger_server::{Server, Signup, router};
use serde_json::{Value, json};
use std::time::Duration;

async fn eventually<F: FnMut() -> bool>(what: &str, mut check: F) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn start_server(dir: &std::path::Path) -> String {
    let server = Server::open(&dir.join("server.db"), Signup::First).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router(server)).await.unwrap() });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_and_server_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let url = start_server(dir.path()).await;
    let http = reqwest::Client::new();
    let post = |path: &str, token: Option<&str>, body: Value| {
        let mut r = http.post(format!("{url}/api/v1{path}")).json(&body);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        r.send()
    };

    // The first account is open; the second is refused.
    let session: Value =
        post("/signup", None, json!({"email": "Me@Example.com", "password": "correct horse", "device": "test"}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    let session = session["token"].as_str().unwrap().to_string();
    let second =
        post("/signup", None, json!({"email": "other@example.com", "password": "correct horse"})).await.unwrap();
    assert_eq!(second.status(), 403);
    let wrong = post("/login", None, json!({"email": "me@example.com", "password": "nope nope"})).await.unwrap();
    assert_eq!(wrong.status(), 401);

    // A computer with two commands and an automation.
    let data = DataDir::at(dir.path().join("agent"));
    std::fs::create_dir_all(data.root()).unwrap();
    let out = dir.path().join("out.txt");
    std::fs::write(
        data.commands(),
        json!([
            {"name": "Write", "command": format!("printf %s \"$CMDTRIGGER_SOURCE:$CMDTRIGGER_PARAMS\" > '{}'; echo written; true", out.display()), "allowParams": true, "icon": "✍️"},
            {"name": "Fail", "command": "echo broken >&2; exit 2"}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        data.automations(),
        json!([{"id": "a1", "name": "Both", "steps": [{"type": "command", "command": "Write", "params": "auto"}]}])
            .to_string(),
    )
    .unwrap();
    std::fs::write(data.settings(), json!({"computerName": "Test PC"}).to_string()).unwrap();

    let config = link::sign_in(&url, "me@example.com", "correct horse", false, "Test PC").await.unwrap();
    assert_eq!(config.email, "me@example.com");
    config.save(&data).unwrap();
    let agent = Agent::start_with(
        data.clone(),
        Ground::Foreground,
        Options { homekit_port: 0, api_port: Some(0), phone_port: Some(0) },
    )
    .await;
    eventually("agent online", || agent.link_status() == LinkStatus::Online).await;

    let computers: Value =
        http.get(format!("{url}/api/v1/computers")).bearer_auth(&session).send().await.unwrap().json().await.unwrap();
    assert_eq!(computers[0]["name"], "Test PC");
    assert_eq!(computers[0]["online"], json!(["foreground"]));
    let names: Vec<&str> =
        computers[0]["catalog"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Write", "Fail", "Both"]);

    // An API token runs things and waits for the result.
    let token: Value =
        post("/tokens", Some(&session), json!({"name": "Shortcuts"})).await.unwrap().json().await.unwrap();
    let token = token["token"].as_str().unwrap().to_string();
    let run: Value =
        post("/run", Some(&token), json!({"computer": "test pc", "target": "write", "params": "hello", "wait": 10}))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(run["status"], "succeeded", "{run}");
    assert_eq!(run["output"], "written");
    assert_eq!(run["source"], "api");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "server/api:hello");

    let failed: Value = post("/run", Some(&token), json!({"computer": "Test PC", "target": "Fail", "wait": 10}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (failed["status"].as_str(), failed["exitCode"].as_i64(), failed["output"].as_str()),
        (Some("failed"), Some(2), Some("broken"))
    );

    let automation: Value = post("/run", Some(&token), json!({"computer": "Test PC", "target": "Both", "wait": 10}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(automation["status"], "succeeded");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "server/api:auto");

    // Without waiting, the run is queued and finishes on its own.
    let queued: Value = post("/run", Some(&token), json!({"computer": "Test PC", "target": "Write"}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(queued["status"], "queued");
    let id = queued["id"].as_str().unwrap().to_string();
    let mut finished = Value::Null;
    for _ in 0..100 {
        finished =
            http.get(format!("{url}/api/v1/runs/{id}")).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
        if finished["status"] == "succeeded" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(finished["status"], "succeeded");

    // A token can't manage the account.
    assert_eq!(post("/tokens", Some(&token), json!({"name": "x"})).await.unwrap().status(), 403);

    // MCP over HTTP: one tool per item, named after the computer.
    let init: Value = post(
        "/mcp",
        Some(&token),
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "cmdtrigger-server");
    let tools: Value = post("/mcp", Some(&token), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tool_names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tool_names, ["test_pc__write", "test_pc__fail", "test_pc__automation_both"]);
    let called: Value = post("/mcp", Some(&token), json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "test_pc__write", "arguments": {"params": "from ai"}}}))
        .await.unwrap().json().await.unwrap();
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "server/mcp:from ai");

    // History has everything, newest first.
    let runs: Value =
        http.get(format!("{url}/api/v1/runs")).bearer_auth(&session).send().await.unwrap().json().await.unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 5);
    assert_eq!(runs[0]["source"], "mcp");

    // Offline: the run is recorded as such.
    agent.shutdown().await;
    eventually("agent gone", || true).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let offline: Value = post("/run", Some(&token), json!({"computer": "Test PC", "target": "Write"}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(offline["status"], "offline");

    // Removing the computer cuts off its agent for good.
    let agent = Agent::start_with(
        data.clone(),
        Ground::Foreground,
        Options { homekit_port: 0, api_port: Some(0), phone_port: Some(0) },
    )
    .await;
    eventually("agent back", || agent.link_status() == LinkStatus::Online).await;
    let id = computers[0]["id"].as_str().unwrap();
    let removed = http.delete(format!("{url}/api/v1/computers/{id}")).bearer_auth(&session).send().await.unwrap();
    assert_eq!(removed.status(), 204);
    eventually("agent rejected", || matches!(agent.link_status(), LinkStatus::Rejected { .. })).await;
    agent.shutdown().await;
}
