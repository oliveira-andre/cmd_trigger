//! The local HTTP API, for scripts, Home Assistant `rest_command`s, Stream Deck plugins, other
//! apps, and `cmdtrigger mcp`. It listens on 127.0.0.1:7171 (the background service on 7172),
//! or on the network when allowed.
//!
//! ```text
//! GET  /api/v1/status                      → {computer, ground, version}
//! GET  /api/v1/catalog                     → [CatalogEntry]
//! POST /api/v1/run {target, kind?, params?, wait?, source?}  → RunRecord (wait=false: 202 {id})
//! GET  /api/v1/runs?limit=50               → [RunRecord], newest first
//! POST /hooks/<hook>  (or GET)             → runs the automation with that webhook trigger
//! POST /api/v1/pair                        → opens an iPhone pairing window (`cmdtrigger pair`)
//! GET  /api/v1/pair                        → the open window, or null
//! ```
//!
//! `/api/v1/*` needs the key from `api.json`: `Authorization: Bearer <key>`, `X-Api-Key`, or
//! `?key=`. Webhooks don't: the hook id is the secret, and the automation's conditions apply.

use crate::config::{ApiConfig, Settings};
use crate::phone::PairingInfo;
use crate::runner::{Runner, find_automation, find_command};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cmdtrigger_protocol::TargetKind;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(60);
const HOOK_WAIT: Duration = Duration::from_secs(30);

/// Opens a pairing window (`true`) or reads the open one (`false`), in the agent that serves
/// phones.
pub type PairingControl =
    Arc<dyn Fn(bool) -> Pin<Box<dyn Future<Output = Result<Option<PairingInfo>, String>> + Send>> + Send + Sync>;

struct Api {
    runner: Arc<Runner>,
    key: String,
    pairing: Option<PairingControl>,
}

pub struct LocalApi {
    cancel: CancellationToken,
    pub addr: SocketAddr,
}

impl LocalApi {
    /// Listen when enabled. Fails when the port is taken.
    pub async fn start(
        config: &ApiConfig,
        runner: Arc<Runner>,
        pairing: Option<PairingControl>,
    ) -> std::io::Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        let port = config.port_for(runner.ground());
        let host = if config.allow_lan { "0.0.0.0" } else { "127.0.0.1" };
        let listener = tokio::net::TcpListener::bind((host, port)).await?;
        let addr = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let app = router(Arc::new(Api { runner, key: config.key.clone(), pairing }));
        let stop = cancel.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async move { stop.cancelled().await }).await;
        });
        tracing::info!("Local API on http://{addr}");
        Ok(Some(Self { cancel, addr }))
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for LocalApi {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn router(api: Arc<Api>) -> Router {
    Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/catalog", get(catalog))
        .route("/api/v1/run", post(run))
        .route("/api/v1/runs", get(runs))
        .route("/hooks/{hook}", get(hook).post(hook))
        .route("/api/v1/pair", get(pair_get).post(pair_start))
        .with_state(api)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn authorized(api: &Api, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let given = header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| header("x-api-key"))
        .or_else(|| query.get("key").map(String::as_str));
    given.is_some_and(|k| crate::homekit::srp::constant_time_eq(k.trim().as_bytes(), api.key.as_bytes()))
}

macro_rules! guard {
    ($api:expr, $headers:expr, $query:expr) => {
        if !authorized(&$api, &$headers, &$query) {
            return error(StatusCode::UNAUTHORIZED, "missing or wrong API key");
        }
    };
}

async fn status(State(api): State<Arc<Api>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    guard!(api, headers, q);
    let computer = Settings::load(api.runner.data()).computer_name();
    Json(json!({ "computer": computer, "ground": api.runner.ground().as_str(), "version": crate::VERSION }))
        .into_response()
}

async fn catalog(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    guard!(api, headers, q);
    Json(api.runner.catalog()).into_response()
}

#[derive(Deserialize)]
struct RunBody {
    target: String,
    #[serde(default)]
    kind: Option<TargetKind>,
    #[serde(default)]
    params: Option<String>,
    #[serde(default = "yes")]
    wait: bool,
    /// "mcp", "cli" or "phone" when those call; anything else is "api".
    #[serde(default)]
    source: Option<String>,
}

fn yes() -> bool {
    true
}

/// A command name first, then an automation id or name.
pub fn resolve(runner: &Runner, target: &str, kind: Option<TargetKind>) -> Option<(TargetKind, String)> {
    let command = || find_command(&runner.commands(), target).map(|c| (TargetKind::Command, c.name().to_string()));
    let automation = || find_automation(&runner.automations(), target).map(|a| (TargetKind::Automation, a.id));
    match kind {
        Some(TargetKind::Command) => command(),
        Some(TargetKind::Automation) => automation(),
        None => command().or_else(automation),
    }
}

async fn run(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(body): Json<RunBody>,
) -> Response {
    guard!(api, headers, q);
    let Some((kind, id)) = resolve(&api.runner, &body.target, body.kind) else {
        return error(StatusCode::NOT_FOUND, &format!("nothing called \"{}\" on this computer", body.target));
    };
    let run_id = crate::automation::new_id(12);
    let runner = api.runner.clone();
    let (params, rid) = (body.params.clone(), run_id.clone());
    let source = match body.source.as_deref() {
        Some("mcp") => "mcp",
        Some("cli") => "cli",
        Some("phone") => "phone",
        _ => "api",
    };
    let task = tokio::spawn(async move { runner.run_with_id(kind, &id, params, source, rid).await });
    if !body.wait {
        return (StatusCode::ACCEPTED, Json(json!({ "id": run_id }))).into_response();
    }
    match tokio::time::timeout(WAIT, task).await {
        Ok(Ok(record)) => Json(record).into_response(),
        _ => (StatusCode::ACCEPTED, Json(json!({ "id": run_id, "status": "running" }))).into_response(),
    }
}

async fn pair_start(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    guard!(api, headers, q);
    pairing(&api, true).await
}

async fn pair_get(
    State(api): State<Arc<Api>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    guard!(api, headers, q);
    pairing(&api, false).await
}

async fn pairing(api: &Api, open: bool) -> Response {
    let Some(control) = &api.pairing else {
        return error(
            StatusCode::NOT_FOUND,
            "iPhones pair with the app (or `cmdtrigger console`), not the background service",
        );
    };
    match control(open).await {
        Ok(info) => Json(info).into_response(),
        Err(e) => error(StatusCode::CONFLICT, &e),
    }
}

async fn runs(State(api): State<Arc<Api>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    guard!(api, headers, q);
    let limit = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50).min(500);
    Json(api.runner.recent(limit)).into_response()
}

async fn hook(
    State(api): State<Arc<Api>>,
    Path(hook): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    body: String,
) -> Response {
    let automation = api
        .runner
        .automations()
        .into_iter()
        .find(|a| a.ground == api.runner.ground().as_str() && a.webhook() == Some(hook.as_str()));
    let Some(automation) = automation else { return error(StatusCode::NOT_FOUND, "no such webhook") };
    if !automation.enabled {
        return error(StatusCode::CONFLICT, "this automation is turned off");
    }
    // Parameters: ?params=…, a JSON body's "params", or a plain-text body.
    let params = q.get("params").cloned().or_else(|| {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(body) {
            Ok(Value::Object(map)) => {
                map.get("params").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
            }
            _ => Some(body.to_string()),
        }
    });
    let runner = api.runner.clone();
    let task = tokio::spawn(async move { runner.trigger(&automation, params, "webhook").await });
    match tokio::time::timeout(HOOK_WAIT, task).await {
        Ok(Ok(Some(record))) => Json(record).into_response(),
        Ok(Ok(None)) => error(StatusCode::CONFLICT, "skipped: the automation's conditions don't hold"),
        _ => (StatusCode::ACCEPTED, Json(json!({ "status": "running" }))).into_response(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::DataDir;

    #[tokio::test]
    async fn runs_commands_and_webhooks() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        std::fs::write(
            data.commands(),
            r#"[{"name": "Echo", "command": "printf", "allowParams": true, "quoteParams": true}]"#,
        )
        .unwrap();
        std::fs::write(
            data.automations(),
            r#"[{"id": "hooked", "name": "Hooked", "allowParams": true, "triggers": [{"type": "webhook", "hook": "secret-hook-1"}],
                 "steps": [{"type": "command", "command": "Echo", "params": "got {params}"}]}]"#,
        )
        .unwrap();
        let mut config = ApiConfig::load(&data);
        config.port = 0;
        let runner = Runner::new(data, crate::Ground::Foreground);
        let api = LocalApi::start(&config, runner, None).await.unwrap().unwrap();
        let base = format!("http://{}", api.addr);
        let http = reqwest::Client::new();

        let denied = http.get(format!("{base}/api/v1/catalog")).send().await.unwrap();
        assert_eq!(denied.status(), 401);
        let catalog: Value = http
            .get(format!("{base}/api/v1/catalog"))
            .bearer_auth(&config.key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(catalog[0]["name"], "Echo");

        let record: Value = http
            .post(format!("{base}/api/v1/run"))
            .header("x-api-key", &config.key)
            .json(&json!({"target": "echo", "params": "hello there"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!((record["status"].as_str(), record["output"].as_str()), (Some("succeeded"), Some("hello there")));

        let hooked: Value = http
            .post(format!("{base}/hooks/secret-hook-1"))
            .body(r#"{"params": "42"}"#)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(hooked["source"], "webhook");
        assert!(hooked["output"].as_str().unwrap().contains("got 42"), "{hooked}");
        assert_eq!(http.post(format!("{base}/hooks/nope")).send().await.unwrap().status(), 404);

        let runs: Value =
            http.get(format!("{base}/api/v1/runs?key={}", config.key)).send().await.unwrap().json().await.unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 2);
    }
}
