//! The contract between a CmdTrigger agent (one per computer), the self-hosted CmdTrigger
//! server, and the server's clients (the web dashboard, scripts, iPhone Shortcuts, MCP).
//!
//! ```text
//!  phone / Shortcuts / scripts ──HTTPS + API token──┐
//!  web dashboard ─────────────HTTPS + session───────┤
//!  AI assistant (MCP) ────────HTTPS + API token─────┤
//!                                                    ▼
//!                                         cmdtrigger-server
//!                                                    ▲  WebSocket, agent token
//!                                     agent (tray app / CLI / service) on each PC
//! ```
//!
//! Commands and automations are defined on each computer; the agent publishes a *catalog*
//! (names, icons, descriptions — never the command lines) and the server can only ask it to
//! run something from that catalog.
//!
//! REST endpoints, all JSON under `/api/v1`:
//!
//! | method | path | auth | body → reply |
//! | --- | --- | --- | --- |
//! | POST | `/signup`, `/login` | – | [`Credentials`] → [`Session`] |
//! | POST | `/logout` | session | – |
//! | GET | `/me` | session/token | → `{email}` |
//! | GET | `/computers` | session/token | → `[`[`ComputerView`]`]` |
//! | DELETE | `/computers/{id}` | session | – |
//! | POST | `/computers/register` | session | [`RegisterComputer`] → [`Registered`] |
//! | POST | `/run` | session/token | [`RunRequest`] → [`RunView`] |
//! | GET | `/runs`, `/runs/{id}` | session/token | → [`RunView`]s |
//! | GET/POST | `/tokens` | session | → [`TokenView`]s / [`NewToken`] → [`CreatedToken`] |
//! | DELETE | `/tokens/{id}` | session | – |
//! | GET | `/agent` | agent token | WebSocket of [`AgentMessage`] / [`ServerMessage`] |
//! | POST | `/mcp` | token | MCP JSON-RPC (see [`mcp`]) |

pub mod mcp;

use serde::{Deserialize, Serialize};

pub const API_PREFIX: &str = "/api/v1";

/// What can be run on a computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Command,
    Automation,
}

/// One runnable thing, as the agent publishes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub kind: TargetKind,
    /// Commands are identified by name, automations by their id.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub allow_params: bool,
    /// Has an off command (on/off style).
    #[serde(default)]
    pub has_off: bool,
    /// "foreground" or "background": which of the computer's agents runs it.
    #[serde(default = "foreground")]
    pub ground: String,
}

fn foreground() -> String {
    "foreground".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Accepted, waiting for the agent.
    Queued,
    Running,
    Succeeded,
    Failed,
    /// No agent for that computer (and ground) is connected.
    Offline,
    /// The agent didn't answer in time; it may still finish.
    TimedOut,
}

impl RunStatus {
    pub fn finished(self) -> bool {
        !matches!(self, RunStatus::Queued | RunStatus::Running)
    }
}

/// Agent → server, over the agent WebSocket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    /// First message on every connection.
    Hello {
        name: String,
        os: String,
        version: String,
        ground: String,
        catalog: Vec<CatalogEntry>,
    },
    /// The commands or automations changed.
    Catalog {
        catalog: Vec<CatalogEntry>,
    },
    /// A run the server asked for finished (or started, with `Running`).
    Result {
        run_id: String,
        status: RunStatus,
        exit_code: Option<i32>,
        #[serde(default)]
        output: String,
        #[serde(default)]
        duration_ms: u64,
    },
    Ping,
}

/// Server → agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Run {
        run_id: String,
        kind: TargetKind,
        id: String,
        params: Option<String>,
        /// Who asked the server: "dashboard", "api", "mcp".
        #[serde(default)]
        source: String,
    },
    Pong,
    /// The token was revoked or the computer deleted; the agent should stop retrying.
    Rejected {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
    /// A label for the session ("Dashboard on iPhone", "Den PC agent setup").
    #[serde(default)]
    pub device: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterComputer {
    pub name: String,
    #[serde(default)]
    pub os: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registered {
    pub computer_id: String,
    pub agent_token: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerView {
    pub id: String,
    pub name: String,
    pub os: String,
    /// Which agents are connected: "foreground", "background".
    pub online: Vec<String>,
    pub last_seen: Option<u64>,
    pub catalog: Vec<CatalogEntry>,
}

/// Run a command or automation. `computer` is an id or a name (case-insensitive);
/// `target` is a command name or an automation id or name.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunRequest {
    pub computer: String,
    pub target: String,
    #[serde(default)]
    pub kind: Option<TargetKind>,
    #[serde(default)]
    pub params: Option<String>,
    /// Seconds to wait for the result (0–60). Without it the reply is the queued run.
    #[serde(default)]
    pub wait: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: String,
    pub computer_id: String,
    pub computer_name: String,
    pub kind: TargetKind,
    pub target: String,
    pub target_name: String,
    pub params: Option<String>,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub output: String,
    /// "dashboard", "api", "mcp".
    pub source: String,
    pub created_at: u64,
    pub finished_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewToken {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenView {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedToken {
    pub id: String,
    pub name: String,
    /// Shown once.
    pub token: String,
}

/// `{"error": "...", "code": "..."}` on every 4xx/5xx.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    pub code: String,
}

/// A name in the form AI tools and Home Assistant accept: lowercase letters, digits and `_`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() { "item".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let hello = AgentMessage::Hello {
            name: "Den".into(),
            os: "macos".into(),
            version: "0.2.0".into(),
            ground: "foreground".into(),
            catalog: vec![CatalogEntry {
                kind: TargetKind::Command,
                id: "Calculator".into(),
                name: "Calculator".into(),
                icon: "🧮".into(),
                description: String::new(),
                allow_params: false,
                has_off: false,
                ground: "foreground".into(),
            }],
        };
        let text = serde_json::to_string(&hello).unwrap();
        assert!(text.starts_with(r#"{"type":"hello""#));
        assert_eq!(serde_json::from_str::<AgentMessage>(&text).unwrap(), hello);
        let run: ServerMessage =
            serde_json::from_str(r#"{"type":"run","run_id":"r1","kind":"command","id":"Calculator","params":null}"#)
                .unwrap();
        // `source` is optional, so older servers still work.
        assert!(matches!(run, ServerMessage::Run { .. }));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Living room TV!"), "living_room_tv");
        assert_eq!(slug("  --  "), "item");
        assert_eq!(slug("Café 2"), "caf_2");
    }
}
