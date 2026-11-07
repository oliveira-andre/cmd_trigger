//! The link to your CmdTrigger server. Signing in once registers this computer and stores an
//! agent token, which can only connect this computer and run what it publishes. The account
//! password and session are not kept. After that the agent keeps a WebSocket open, publishes
//! its catalog, and runs what the server sends.

use crate::config::ServerConfig;
use crate::runner::Runner;
use cmdtrigger_protocol::{
    API_PREFIX, AgentMessage, Credentials, RegisterComputer, Registered, RunStatus, ServerMessage, Session,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum LinkStatus {
    /// No server set up.
    Off,
    Connecting,
    Online,
    Offline {
        error: String,
    },
    /// The server refused this computer's token (deleted or revoked): sign in again.
    Rejected {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("{0}")]
    Invalid(String),
    #[error("could not reach the server: {0}")]
    Network(String),
    #[error("{0}")]
    Refused(String),
}

/// `https://host` → canonical form, or an error for anything that isn't http(s).
pub fn normalize_url(url: &str) -> Result<String, LinkError> {
    let url = url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(url)
        .map_err(|_| LinkError::Invalid("That isn't a web address like https://cmd.example.com".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(LinkError::Invalid(
            "The server address must start with https:// (or http:// on your own network)".into(),
        ));
    }
    Ok(url.to_string())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("CmdTrigger/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("static HTTP client settings")
}

async fn reply<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T, LinkError> {
    let status = response.status();
    let text = response.text().await.map_err(|e| LinkError::Network(e.to_string()))?;
    if !status.is_success() {
        let message = serde_json::from_str::<cmdtrigger_protocol::ApiError>(&text).map(|e| e.error).unwrap_or(text);
        return Err(LinkError::Refused(message));
    }
    serde_json::from_str(&text).map_err(|e| LinkError::Network(format!("unexpected reply: {e}")))
}

/// Sign in (or create the account), register this computer, and return what to save.
pub async fn sign_in(
    url: &str,
    email: &str,
    password: &str,
    create: bool,
    computer: &str,
) -> Result<ServerConfig, LinkError> {
    let url = normalize_url(url)?;
    let client = http();
    let credentials = Credentials {
        email: email.trim().into(),
        password: password.into(),
        device: format!("{computer} agent setup"),
    };
    let path = if create { "signup" } else { "login" };
    let session: Session = reply(
        client
            .post(format!("{url}{API_PREFIX}/{path}"))
            .json(&credentials)
            .send()
            .await
            .map_err(|e| LinkError::Network(e.to_string()))?,
    )
    .await?;
    let registered: Registered = reply(
        client
            .post(format!("{url}{API_PREFIX}/computers/register"))
            .bearer_auth(&session.token)
            .json(&RegisterComputer { name: computer.into(), os: std::env::consts::OS.into() })
            .send()
            .await
            .map_err(|e| LinkError::Network(e.to_string()))?,
    )
    .await?;
    let _ = client.post(format!("{url}{API_PREFIX}/logout")).bearer_auth(&session.token).send().await;
    Ok(ServerConfig {
        url,
        email: session.email,
        computer_id: registered.computer_id,
        agent_token: registered.agent_token,
    })
}

pub fn agent_url(base: &str) -> String {
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}{API_PREFIX}/agent")
}

pub struct Link {
    cancel: CancellationToken,
    status: watch::Receiver<LinkStatus>,
}

impl Link {
    pub fn start(
        config: ServerConfig,
        computer: String,
        runner: Arc<Runner>,
        changes: broadcast::Receiver<()>,
    ) -> Option<Self> {
        if !config.connected() {
            return None;
        }
        let cancel = CancellationToken::new();
        let (tx, status) = watch::channel(LinkStatus::Connecting);
        tokio::spawn(run(config, computer, runner, changes, tx, cancel.clone()));
        Some(Self { cancel, status })
    }

    pub fn status(&self) -> LinkStatus {
        self.status.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<LinkStatus> {
        self.status.clone()
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

enum End {
    Retry(String),
    Rejected(String),
    Cancelled,
}

async fn run(
    config: ServerConfig,
    computer: String,
    runner: Arc<Runner>,
    mut changes: broadcast::Receiver<()>,
    status: watch::Sender<LinkStatus>,
    cancel: CancellationToken,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        status.send_replace(LinkStatus::Connecting);
        let started = std::time::Instant::now();
        match session(&config, &computer, &runner, &mut changes, &status, &cancel).await {
            End::Cancelled => return,
            End::Rejected(message) => {
                tracing::warn!("The server refused this computer: {message}");
                status.send_replace(LinkStatus::Rejected { message });
                return;
            }
            End::Retry(error) => {
                // A link that worked (the server restarted or redeployed, the network
                // blipped) comes back quickly. One that keeps failing, or drops straight
                // after connecting, keeps backing off.
                let was_online = matches!(*status.borrow(), LinkStatus::Online);
                if was_online && started.elapsed() > Duration::from_secs(5) {
                    delay = Duration::from_secs(1);
                }
                tracing::info!("Server link: {error}; retrying in {} s", delay.as_secs());
                status.send_replace(LinkStatus::Offline { error });
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

async fn session(
    config: &ServerConfig,
    computer: &str,
    runner: &Arc<Runner>,
    changes: &mut broadcast::Receiver<()>,
    status: &watch::Sender<LinkStatus>,
    cancel: &CancellationToken,
) -> End {
    let mut request = match agent_url(&config.url).into_client_request() {
        Ok(r) => r,
        Err(e) => return End::Rejected(format!("bad server address: {e}")),
    };
    let auth = format!("Bearer {}", config.agent_token);
    request.headers_mut().insert("authorization", auth.parse().expect("token is header-safe"));
    let connecting = tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(request));
    let ws = tokio::select! {
        _ = cancel.cancelled() => return End::Cancelled,
        r = connecting => match r {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status() == 401 || response.status() == 403 => {
                let body = response.body().as_ref().map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
                return End::Rejected(serde_json::from_str::<cmdtrigger_protocol::ApiError>(&body).map(|e| e.error).unwrap_or_else(|_| "token refused".into()));
            }
            Ok(Err(e)) => return End::Retry(e.to_string()),
            Err(_) => return End::Retry("timed out connecting".into()),
        },
    };
    let (mut tx, mut rx) = ws.split();
    let send = |m: &AgentMessage| Message::Text(serde_json::to_string(m).expect("messages serialize").into());
    let hello = AgentMessage::Hello {
        name: computer.to_string(),
        os: std::env::consts::OS.into(),
        version: crate::VERSION.into(),
        ground: runner.ground().as_str().into(),
        catalog: runner.catalog(),
    };
    if let Err(e) = tx.send(send(&hello)).await {
        return End::Retry(e.to_string());
    }
    status.send_replace(LinkStatus::Online);
    tracing::info!("Connected to {}", config.url);

    let (results_tx, mut results) = mpsc::unbounded_channel::<AgentMessage>();
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = tx.close().await;
                return End::Cancelled;
            }
            changed = changes.recv() => {
                if matches!(changed, Err(broadcast::error::RecvError::Closed)) { return End::Cancelled }
                if let Err(e) = tx.send(send(&AgentMessage::Catalog { catalog: runner.catalog() })).await {
                    return End::Retry(e.to_string());
                }
            }
            Some(result) = results.recv() => {
                if let Err(e) = tx.send(send(&result)).await {
                    return End::Retry(e.to_string());
                }
            }
            _ = ping.tick() => {
                if awaiting_pong {
                    return End::Retry("the server stopped answering".into());
                }
                awaiting_pong = true;
                if let Err(e) = tx.send(send(&AgentMessage::Ping)).await {
                    return End::Retry(e.to_string());
                }
            }
            frame = rx.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; continue }
                    Some(Ok(Message::Close(_))) | None => return End::Retry("the server closed the connection".into()),
                    Some(Err(e)) => return End::Retry(e.to_string()),
                    Some(Ok(_)) => continue,
                };
                match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(ServerMessage::Pong) => awaiting_pong = false,
                    Ok(ServerMessage::Rejected { message }) => return End::Rejected(message),
                    Ok(ServerMessage::Run { run_id, kind, id, params, source }) => {
                        let (runner, results_tx) = (runner.clone(), results_tx.clone());
                        tokio::spawn(async move {
                            let _ = results_tx.send(AgentMessage::Result {
                                run_id: run_id.clone(), status: RunStatus::Running, exit_code: None, output: String::new(), duration_ms: 0,
                            });
                            let source = if source.is_empty() { "server".to_string() } else { format!("server/{source}") };
                            let record = runner.run_with_id(kind, &id, params, &source, run_id.clone()).await;
                            let _ = results_tx.send(AgentMessage::Result {
                                run_id, status: record.status, exit_code: record.exit_code, output: record.output, duration_ms: record.duration_ms,
                            });
                        });
                    }
                    Err(e) => tracing::warn!("Server link: unreadable message ({e})"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(normalize_url(" https://cmd.example.com/ ").unwrap(), "https://cmd.example.com");
        assert!(normalize_url("cmd.example.com").is_err());
        assert!(normalize_url("ftp://x").is_err());
        assert_eq!(agent_url("https://cmd.example.com"), "wss://cmd.example.com/api/v1/agent");
        assert_eq!(agent_url("http://192.168.1.5:8080"), "ws://192.168.1.5:8080/api/v1/agent");
    }
}
