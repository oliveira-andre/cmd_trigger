//! The slice of the Model Context Protocol CmdTrigger speaks: JSON-RPC 2.0 with
//! `initialize`, `tools/list` and `tools/call`. The agent serves it over stdio
//! (`cmdtrigger mcp`); the server serves it over HTTP (`POST /api/v1/mcp`).
//! Every command and automation becomes one tool.

use crate::{CatalogEntry, TargetKind, slug};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// A request the caller has to act on.
#[derive(Debug, PartialEq)]
pub enum Call {
    /// Answer with [`tools`].
    ListTools,
    /// Run the tool named `name` with these arguments.
    CallTool { name: String, params: Option<String> },
}

/// Handle one JSON-RPC message. Returns `Ok(reply)` for messages the protocol layer answers
/// itself (initialize, ping, unknown methods), `Err((id, call))` when the caller must act,
/// and `Ok(None)` for notifications.
pub fn dispatch(message: &Value, server_name: &str, version: &str) -> Result<Option<Value>, (Value, Call)> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = id else { return Ok(None) }; // a notification (initialized, cancelled…)
    match method {
        "initialize" => {
            let requested =
                message.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
            Ok(Some(result(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": server_name, "version": version },
                    "instructions": "Each tool runs a command or automation on the user's computer. \
                                     Use a tool only when the user asks for that action.",
                }),
            )))
        }
        "ping" => Ok(Some(result(id, json!({})))),
        "tools/list" => Err((id, Call::ListTools)),
        "tools/call" => {
            let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or("").to_string();
            let params = message
                .pointer("/params/arguments/params")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Null => None,
                    other => Some(other.to_string()),
                })
                .filter(|s| !s.is_empty());
            Err((id, Call::CallTool { name, params }))
        }
        _ => Ok(Some(error(id, -32601, &format!("method not found: {method}")))),
    }
}

pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The reply to `tools/call`: text content, flagged as an error when the run failed.
pub fn tool_result(id: Value, text: &str, is_error: bool) -> Value {
    result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

/// A tool for each entry. `prefix` namespaces tools by computer when one server lists many.
pub fn tool_for(entry: &CatalogEntry, prefix: Option<&str>, computer_name: Option<&str>) -> (String, Value) {
    let base = match entry.kind {
        TargetKind::Command => slug(&entry.name),
        TargetKind::Automation => format!("automation_{}", slug(&entry.name)),
    };
    let mut name = match prefix {
        Some(p) => format!("{}__{base}", slug(p)),
        None => base,
    };
    name.truncate(64);
    let what = match entry.kind {
        TargetKind::Command => "Runs the command",
        TargetKind::Automation => "Runs the automation",
    };
    let on = computer_name.map(|c| format!(" on the computer \"{c}\"")).unwrap_or_default();
    let mut description = format!("{what} \"{}\"{on}.", entry.name);
    if !entry.description.trim().is_empty() {
        description.push(' ');
        description.push_str(entry.description.trim());
    }
    if entry.has_off {
        description.push_str(" Pass params \"on\" or \"off\".");
    }
    let schema = if entry.allow_params {
        json!({
            "type": "object",
            "properties": { "params": { "type": "string", "description": "Parameters for the command" } },
        })
    } else {
        json!({ "type": "object", "properties": {} })
    };
    let tool = json!({ "name": name, "description": description, "inputSchema": schema });
    (name, tool)
}

pub fn tools_list(id: Value, tools: Vec<Value>) -> Value {
    result(id, json!({ "tools": tools }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches() {
        let init =
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}});
        let reply = dispatch(&init, "cmdtrigger", "1").unwrap().unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(
            dispatch(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), "c", "1").unwrap().is_none()
        );
        let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "say", "arguments": {"params": "hi"}}});
        assert_eq!(
            dispatch(&call, "c", "1").unwrap_err().1,
            Call::CallTool { name: "say".into(), params: Some("hi".into()) }
        );
    }

    #[test]
    fn tools_are_named_for_ai() {
        let entry = CatalogEntry {
            kind: TargetKind::Command,
            id: "TV".into(),
            name: "Living room TV".into(),
            icon: String::new(),
            description: "Turns the TV on or off".into(),
            allow_params: true,
            has_off: true,
            ground: "foreground".into(),
        };
        let (name, tool) = tool_for(&entry, Some("Den PC"), Some("Den PC"));
        assert_eq!(name, "den_pc__living_room_tv");
        assert!(tool["description"].as_str().unwrap().contains("\"on\" or \"off\""));
        assert!(tool["inputSchema"]["properties"]["params"].is_object());
    }
}
