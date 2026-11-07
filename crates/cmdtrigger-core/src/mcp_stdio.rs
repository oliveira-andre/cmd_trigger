//! `cmdtrigger mcp`: an MCP server on stdin/stdout for AI assistants (Claude Desktop, Claude
//! Code, …). Each of this computer's commands and automations is a tool. It talks to the
//! running agent through the local API, so runs show up in the app's history and use the
//! same settings; the background service's items are included when it runs too.
//!
//! Claude Code: `claude mcp add cmdtrigger -- /path/to/cmdtrigger mcp`
//! Claude Desktop (`claude_desktop_config.json`):
//! `{"mcpServers": {"cmdtrigger": {"command": "/path/to/cmdtrigger", "args": ["mcp"]}}}`

use crate::config::ApiConfig;
use crate::paths::DataDir;
use cmdtrigger_protocol::{CatalogEntry, mcp};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct Agents {
    http: reqwest::Client,
    ports: Vec<u16>,
    key: String,
    tools: HashMap<String, (u16, CatalogEntry)>,
}

impl Agents {
    async fn catalog(&mut self) -> Vec<Value> {
        self.tools.clear();
        let mut list = Vec::new();
        for port in self.ports.clone() {
            let url = format!("http://127.0.0.1:{port}/api/v1/catalog");
            let Ok(response) = self.http.get(url).bearer_auth(&self.key).send().await else { continue };
            let Ok(entries) = response.json::<Vec<CatalogEntry>>().await else { continue };
            for entry in entries {
                let (name, tool) = mcp::tool_for(&entry, None, None);
                if self.tools.contains_key(&name) {
                    continue;
                }
                self.tools.insert(name, (port, entry));
                list.push(tool);
            }
        }
        list
    }

    async fn call(&mut self, name: &str, params: Option<String>) -> (String, bool) {
        if !self.tools.contains_key(name) {
            self.catalog().await;
        }
        let Some((port, entry)) = self.tools.get(name).cloned() else {
            return (format!("There is no tool called {name}. Is the CmdTrigger app running?"), true);
        };
        let body = json!({ "target": entry.id, "kind": entry.kind, "params": params, "wait": true, "source": "mcp" });
        let response = self
            .http
            .post(format!("http://127.0.0.1:{port}/api/v1/run"))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await;
        let record: Value = match response {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => return (format!("CmdTrigger is not reachable: {e}"), true),
        };
        let status = record.get("status").and_then(Value::as_str).unwrap_or("unknown");
        let output = record.get("output").and_then(Value::as_str).unwrap_or("").trim();
        let text = match (status, output.is_empty()) {
            ("succeeded", true) => format!("\"{}\" ran successfully.", entry.name),
            ("succeeded", false) => format!("\"{}\" ran successfully. Output:\n{output}", entry.name),
            (_, true) => format!("\"{}\" failed ({status}).", entry.name),
            (_, false) => format!("\"{}\" failed ({status}):\n{output}", entry.name),
        };
        (text, status != "succeeded")
    }
}

pub async fn serve_stdio(data: DataDir) -> anyhow::Result<()> {
    let config = ApiConfig::load(&data);
    let mut agents = Agents {
        http: reqwest::Client::builder().timeout(Duration::from_secs(90)).build()?,
        ports: vec![config.port, config.port.saturating_add(1)],
        key: config.key,
        tools: HashMap::new(),
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let reply = match mcp::dispatch(&message, "cmdtrigger", crate::VERSION) {
            Ok(reply) => reply,
            Err((id, mcp::Call::ListTools)) => Some(mcp::tools_list(id, agents.catalog().await)),
            Err((id, mcp::Call::CallTool { name, params })) => {
                let (text, is_error) = agents.call(&name, params).await;
                Some(mcp::tool_result(id, &text, is_error))
            }
        };
        if let Some(reply) = reply {
            stdout.write_all(format!("{reply}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
