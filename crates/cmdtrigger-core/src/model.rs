//! One entry of `commands.json`:
//!
//! ```json
//! {"name": "Living room TV", "command": "~/bin/tv.sh on", "offCommand": "~/bin/tv.sh off",
//!  "ground": "foreground", "allowParams": true, "quoteParams": false,
//!  "description": "Turns the TV on or off", "icon": "📺"}
//! ```
//!
//! A command is kept as its JSON object, so fields this version doesn't know survive edits.
//! Files written by the TRIGGERcmd agent (`trigger`, `mcpToolDescription`, `"true"` strings)
//! are normalized on load (see [`Command::normalize`]).

use cmdtrigger_protocol::{CatalogEntry, TargetKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Command(pub Map<String, Value>);

impl Command {
    pub fn new(name: &str, command: &str, ground: &str) -> Self {
        let mut map = Map::new();
        map.insert("name".into(), Value::String(name.into()));
        map.insert("command".into(), Value::String(command.into()));
        map.insert("ground".into(), Value::String(ground.into()));
        Self(map)
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    fn text(&self, key: &str) -> &str {
        self.0.get(key).and_then(Value::as_str).unwrap_or("")
    }

    pub fn name(&self) -> &str {
        self.text("name")
    }
    pub fn command(&self) -> &str {
        self.text("command")
    }
    pub fn ground(&self) -> &str {
        match self.text("ground") {
            "background" => "background",
            _ => "foreground",
        }
    }
    /// Runs instead of `command` when the parameter is "off". Blank means none.
    pub fn off_command(&self) -> Option<&str> {
        Some(self.text("offCommand")).filter(|s| !s.trim().is_empty())
    }
    pub fn description(&self) -> &str {
        self.text("description")
    }
    pub fn icon(&self) -> &str {
        self.text("icon")
    }
    pub fn allow_params(&self) -> bool {
        flag(self.0.get("allowParams"))
    }
    pub fn quote_params(&self) -> bool {
        flag(self.0.get("quoteParams"))
    }

    /// Bring older field names and string flags to the current form.
    pub fn normalize(mut self) -> Self {
        if !self.0.contains_key("name")
            && let Some(trigger) = self.0.remove("trigger")
        {
            self.0.insert("name".into(), trigger);
        }
        if !self.0.contains_key("description")
            && let Some(d) = self.0.remove("mcpToolDescription")
        {
            self.0.insert("description".into(), d);
        }
        for key in ["allowParams", "quoteParams"] {
            if let Some(v) = self.0.get(key) {
                let b = flag(Some(v));
                self.0.insert(key.into(), Value::Bool(b));
            }
        }
        self
    }

    pub fn catalog_entry(&self) -> CatalogEntry {
        CatalogEntry {
            kind: TargetKind::Command,
            id: self.name().to_string(),
            name: self.name().to_string(),
            icon: self.icon().to_string(),
            description: self.description().to_string(),
            allow_params: self.allow_params(),
            has_off: self.off_command().is_some(),
            ground: self.ground().to_string(),
        }
    }
}

/// `true`, `"true"`, `1` and `"1"` are on; anything else is off.
pub fn flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true" || s == "1",
        Some(Value::Number(n)) => n.as_f64() == Some(1.0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(v: Value) -> Command {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn flags() {
        assert!(cmd(json!({"allowParams": "true"})).allow_params());
        assert!(cmd(json!({"allowParams": 1})).allow_params());
        assert!(!cmd(json!({"allowParams": "false"})).allow_params());
        assert!(cmd(json!({"quoteParams": true})).quote_params());
    }

    #[test]
    fn older_files_are_normalized() {
        let old = cmd(
            json!({"trigger": "Calc", "command": "calc", "mcpToolDescription": "math", "allowParams": "false", "voice": "calc"}),
        );
        let new = old.normalize();
        assert_eq!(new.name(), "Calc");
        assert_eq!(new.description(), "math");
        assert_eq!(new.get("allowParams"), Some(&json!(false)));
        assert_eq!(new.get("voice"), Some(&json!("calc")), "unknown fields stay");
    }

    #[test]
    fn unknown_fields_survive() {
        let text = r#"{"name":"A","command":"b","ground":"foreground","myField":42}"#;
        let c: Command = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), text);
    }
}
