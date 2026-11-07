//! Runs commands and automations, whoever asked: the tray, HomeKit, Home Assistant, a webhook,
//! the local API, an AI assistant, the server, a schedule or a system event. Every run lands in
//! the history and is broadcast to listeners (the UI, the server link).
//!
//! Commands run through the shell with these variables set:
//! `CMDTRIGGER_RUN_ID`, `CMDTRIGGER_SOURCE`, `CMDTRIGGER_PARAMS`, and inside automations
//! `CMDTRIGGER_AUTOMATION`.

use crate::agent::Ground;
use crate::automation::{Automation, Condition, Step, Weekday, parse_hhmm};
use crate::exec;
use crate::history::{History, RunRecord, clip, now_ms};
use crate::model::Command;
use crate::paths::DataDir;
use crate::store;
use chrono::{Datelike, Timelike};
use cmdtrigger_protocol::{CatalogEntry, RunStatus, TargetKind};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub struct Runner {
    data: DataDir,
    ground: Ground,
    history: Mutex<History>,
    events: broadcast::Sender<RunRecord>,
    busy: Mutex<HashSet<String>>,
}

/// The longest a "wait" step may pause.
const MAX_WAIT: f64 = 24.0 * 3600.0;

impl Runner {
    pub fn new(data: DataDir, ground: Ground) -> Arc<Self> {
        let history = History::open(data.history());
        Arc::new(Self {
            data,
            ground,
            history: Mutex::new(history),
            events: broadcast::channel(64).0,
            busy: Mutex::new(HashSet::new()),
        })
    }

    pub fn data(&self) -> &DataDir {
        &self.data
    }

    pub fn ground(&self) -> Ground {
        self.ground
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunRecord> {
        self.events.subscribe()
    }

    pub fn recent(&self, limit: usize) -> Vec<RunRecord> {
        self.history.lock().unwrap().recent(limit)
    }

    /// Forget every run, for everything that reads the history here.
    pub fn clear_history(&self) -> std::io::Result<()> {
        self.history.lock().unwrap().clear()
    }

    pub fn commands(&self) -> Vec<Command> {
        store::load_commands_lenient(&self.data.commands())
    }

    pub fn automations(&self) -> Vec<Automation> {
        store::load_automations_lenient(&self.data.automations())
    }

    /// What this agent offers: its ground's commands, then its automations.
    pub fn catalog(&self) -> Vec<CatalogEntry> {
        let ground = self.ground.as_str();
        let mut entries: Vec<CatalogEntry> =
            self.commands().iter().filter(|c| c.ground() == ground).map(Command::catalog_entry).collect();
        entries.extend(self.automations().iter().filter(|a| a.ground == ground).map(Automation::catalog_entry));
        entries
    }

    fn finish(&self, record: RunRecord) -> RunRecord {
        let mark = if record.status == RunStatus::Succeeded { "ok" } else { "failed" };
        tracing::info!(
            "[{}] {} \"{}\" {mark}{}",
            record.source,
            match record.kind {
                TargetKind::Command => "command",
                TargetKind::Automation => "automation",
            },
            record.name,
            record.exit_code.map(|c| format!(" (exit {c})")).unwrap_or_default()
        );
        self.history.lock().unwrap().push(record.clone());
        let _ = self.events.send(record.clone());
        record
    }

    fn refused(&self, kind: TargetKind, target: &str, params: Option<String>, source: &str, why: String) -> RunRecord {
        self.finish(RunRecord {
            id: crate::automation::new_id(12),
            at: now_ms(),
            source: source.to_string(),
            kind,
            target: target.to_string(),
            name: target.to_string(),
            params,
            status: RunStatus::Failed,
            exit_code: None,
            output: why,
            duration_ms: 0,
        })
    }

    /// Run something by hand (conditions don't apply). `target` is a command name, or an
    /// automation id or name.
    pub async fn run(&self, kind: TargetKind, target: &str, params: Option<String>, source: &str) -> RunRecord {
        self.run_with_id(kind, target, params, source, crate::automation::new_id(12)).await
    }

    pub async fn run_with_id(
        &self,
        kind: TargetKind,
        target: &str,
        params: Option<String>,
        source: &str,
        id: String,
    ) -> RunRecord {
        match kind {
            TargetKind::Command => {
                let Some(cmd) = find_command(&self.commands(), target) else {
                    return self.refused(
                        kind,
                        target,
                        params,
                        source,
                        format!("There is no command named \"{target}\"."),
                    );
                };
                if cmd.ground() != self.ground.as_str() {
                    return self.refused(kind, target, params, source, wrong_ground(cmd.ground()));
                }
                let line = exec::command_line(&cmd, params.as_deref());
                self.run_command(&cmd, &line, params, source, id).await
            }
            TargetKind::Automation => {
                let Some(automation) = find_automation(&self.automations(), target) else {
                    return self.refused(
                        kind,
                        target,
                        params,
                        source,
                        format!("There is no automation called \"{target}\"."),
                    );
                };
                if automation.ground != self.ground.as_str() {
                    return self.refused(kind, target, params, source, wrong_ground(&automation.ground));
                }
                let params = params.filter(|_| automation.allow_params);
                self.run_automation(&automation, params, source, id).await
            }
        }
    }

    /// A trigger fired: run the automation if it is enabled, not already running, and its
    /// conditions hold. `None` when it was skipped.
    pub async fn trigger(&self, automation: &Automation, params: Option<String>, source: &str) -> Option<RunRecord> {
        if !automation.enabled {
            return None;
        }
        if !conditions_hold(&automation.conditions).await {
            tracing::info!("[{source}] \"{}\" skipped: its conditions don't hold.", automation.name);
            return None;
        }
        let params = params.filter(|_| automation.allow_params);
        Some(self.run_automation(automation, params, source, crate::automation::new_id(12)).await)
    }

    /// A HomeKit light was switched (see [`exec::homekit_command_line`]).
    pub async fn run_homekit(&self, kind: TargetKind, target: &str, on: bool, level: Option<u8>) -> Option<RunRecord> {
        match kind {
            TargetKind::Command => {
                let cmd = find_command(&self.commands(), target)?;
                let line = exec::homekit_command_line(&cmd, on, level)?;
                let params = level
                    .map(|l| l.to_string())
                    .or_else(|| cmd.allow_params().then(|| (if on { "on" } else { "off" }).into()));
                Some(self.run_command(&cmd, &line, params, "homekit", crate::automation::new_id(12)).await)
            }
            TargetKind::Automation if on => Some(self.run(kind, target, level.map(|l| l.to_string()), "homekit").await),
            TargetKind::Automation => None,
        }
    }

    async fn run_command(
        &self,
        cmd: &Command,
        line: &str,
        params: Option<String>,
        source: &str,
        id: String,
    ) -> RunRecord {
        let started = Instant::now();
        let at = now_ms();
        let env = [
            ("CMDTRIGGER_RUN_ID", id.clone()),
            ("CMDTRIGGER_SOURCE", source.to_string()),
            ("CMDTRIGGER_PARAMS", params.clone().unwrap_or_default()),
        ];
        tracing::info!("[{source}] Running \"{}\": {line}", cmd.name());
        let outcome = exec::run_shell(line, &env).await;
        self.finish(RunRecord {
            id,
            at,
            source: source.to_string(),
            kind: TargetKind::Command,
            target: cmd.name().to_string(),
            name: cmd.name().to_string(),
            params,
            status: if outcome.success() { RunStatus::Succeeded } else { RunStatus::Failed },
            exit_code: outcome.exit_code,
            output: clip(&outcome.combined()),
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn run_automation(&self, a: &Automation, params: Option<String>, source: &str, id: String) -> RunRecord {
        if !self.busy.lock().unwrap().insert(a.id.clone()) {
            return self.refused(
                TargetKind::Automation,
                &a.id,
                params,
                source,
                format!("\"{}\" is already running.", a.name),
            );
        }
        let started = Instant::now();
        let at = now_ms();
        let p = params.clone().unwrap_or_default();
        let fill = |text: &str| text.replace("{params}", &p);
        let env = [
            ("CMDTRIGGER_RUN_ID", id.clone()),
            ("CMDTRIGGER_SOURCE", source.to_string()),
            ("CMDTRIGGER_PARAMS", p.clone()),
            ("CMDTRIGGER_AUTOMATION", a.name.clone()),
        ];
        tracing::info!("[{source}] Running automation \"{}\"", a.name);
        let mut log = String::new();
        let mut failed_code: Option<Option<i32>> = None;
        for (i, step) in a.steps.iter().enumerate() {
            let (label, ok, code, output) = match step {
                Step::Command { command, params } => match find_command(&self.commands(), command) {
                    Some(cmd) => {
                        let step_params = Some(fill(params)).filter(|s| !s.trim().is_empty());
                        let line = exec::command_line(&cmd, step_params.as_deref());
                        // A command sees the parameters it was given, like a direct run.
                        let mut step_env = env.clone();
                        step_env[2].1 = step_params.clone().unwrap_or_default();
                        let out = exec::run_shell(&line, &step_env).await;
                        (
                            format!("{} {}", cmd.icon(), cmd.name()).trim().to_string(),
                            out.success(),
                            out.exit_code,
                            out.combined(),
                        )
                    }
                    None => (command.clone(), false, None, format!("There is no command named \"{command}\".")),
                },
                Step::Shell { line } => {
                    let out = exec::run_shell(&fill(line), &env).await;
                    (format!("$ {}", fill(line)), out.success(), out.exit_code, out.combined())
                }
                Step::Wait { seconds } => {
                    let secs = seconds.clamp(0.0, MAX_WAIT);
                    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
                    (format!("wait {secs} s"), true, None, String::new())
                }
                Step::WakeOnLan { mac, broadcast } => match crate::wol::wake(mac, broadcast).await {
                    Ok(()) => (format!("wake {mac}"), true, None, String::new()),
                    Err(e) => (format!("wake {mac}"), false, None, e.to_string()),
                },
                Step::Notify { title, body } => {
                    crate::notify::show(&fill(title), &fill(body)).await;
                    (format!("notify \"{}\"", fill(title)), true, None, String::new())
                }
            };
            log.push_str(&format!("{}. {label} — {}\n", i + 1, if ok { "ok" } else { "failed" }));
            if !output.is_empty() {
                log.push_str(&output);
                log.push('\n');
            }
            if !ok {
                failed_code = Some(code);
                if a.stop_on_error {
                    log.push_str("Stopped: a step failed.\n");
                    break;
                }
            }
        }
        self.busy.lock().unwrap().remove(&a.id);
        self.finish(RunRecord {
            id,
            at,
            source: source.to_string(),
            kind: TargetKind::Automation,
            target: a.id.clone(),
            name: a.name.clone(),
            params,
            status: if failed_code.is_none() { RunStatus::Succeeded } else { RunStatus::Failed },
            exit_code: failed_code.flatten(),
            output: clip(log.trim_end()),
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn wrong_ground(ground: &str) -> String {
    if ground == "background" {
        "This runs in the background service, not in this app.".into()
    } else {
        "This runs in the tray app, not in the background service.".into()
    }
}

pub fn find_command(commands: &[Command], name: &str) -> Option<Command> {
    commands.iter().find(|c| c.name().eq_ignore_ascii_case(name.trim())).cloned()
}

pub fn find_automation(automations: &[Automation], target: &str) -> Option<Automation> {
    let target = target.trim();
    automations
        .iter()
        .find(|a| a.id == target)
        .or_else(|| automations.iter().find(|a| a.name.eq_ignore_ascii_case(target)))
        .cloned()
}

/// Do all conditions hold right now?
pub async fn conditions_hold(conditions: &[Condition]) -> bool {
    let now = chrono::Local::now();
    let minute = now.hour() * 60 + now.minute();
    let today = Weekday::from_chrono(now.weekday());
    let mut network = None;
    for condition in conditions {
        let ok = match condition {
            Condition::Time { after, before } => match (parse_hhmm(after), parse_hhmm(before)) {
                (Some(a), Some(b)) if a <= b => (a..b).contains(&minute),
                (Some(a), Some(b)) => minute >= a || minute < b,
                _ => true,
            },
            Condition::Days { days } => days.is_empty() || days.contains(&today),
            Condition::Network { network: wanted, not } => {
                if network.is_none() {
                    network = Some(crate::monitors::network().await);
                }
                let on = network.as_ref().unwrap().as_ref().is_some_and(|n| n.matches(wanted));
                on != *not
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, Arc<Runner>) {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        let out = dir.path().join("out.txt");
        std::fs::write(
            data.commands(),
            json!([
                {"name": "Write", "command": format!("printf %s \"$CMDTRIGGER_SOURCE:$CMDTRIGGER_PARAMS\" >> '{}'; true", out.display()), "allowParams": true},
                {"name": "Fail", "command": "echo nope; exit 4"},
                {"name": "Reads", "command": "printf %s \"$CMDTRIGGER_PARAMS\"", "allowParams": true},
                {"name": "Boot", "command": "true", "ground": "background"}
            ])
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            data.automations(),
            json!([{
                "id": "auto1", "name": "Two steps", "allowParams": true,
                "steps": [{"type": "command", "command": "Write", "params": "[{params}]"}, {"type": "shell", "line": "printf ' then' >> 'OUT'".replace("OUT", &out.display().to_string())}]
            }, {
                "id": "auto2", "name": "Stops", "steps": [{"type": "command", "command": "Fail"}, {"type": "shell", "line": "touch should-not-exist"}]
            }, {
                "id": "auto3", "name": "Never", "conditions": [{"type": "days", "days": []}, {"type": "time", "after": "00:00", "before": "00:00"}],
                "steps": [{"type": "wait", "seconds": 0}]
            }])
            .to_string(),
        )
        .unwrap();
        let runner = Runner::new(data, Ground::Foreground);
        (dir, runner)
    }

    #[tokio::test]
    async fn commands_and_automations_run_and_are_logged() {
        let (dir, runner) = setup();
        let out = dir.path().join("out.txt");
        let r = runner.run(TargetKind::Command, "write", Some("x y".into()), "api").await;
        assert_eq!(r.status, RunStatus::Succeeded);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "api:x y");

        let r = runner.run(TargetKind::Command, "Fail", None, "tray").await;
        assert_eq!((r.status, r.exit_code, r.output.as_str()), (RunStatus::Failed, Some(4), "nope"));

        let r = runner.run(TargetKind::Command, "Boot", None, "tray").await;
        assert!(r.output.contains("background service"));

        std::fs::write(&out, "").unwrap();
        let r = runner.run(TargetKind::Automation, "Two steps", Some("p".into()), "server").await;
        assert_eq!(r.status, RunStatus::Succeeded, "{}", r.output);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "server:[p] then");

        let r = runner.run(TargetKind::Automation, "auto2", None, "tray").await;
        assert_eq!(r.status, RunStatus::Failed);
        assert!(r.output.contains("Stopped"));
        assert!(!dir.path().join("should-not-exist").exists());

        let never = runner.automations().into_iter().find(|a| a.id == "auto3").unwrap();
        assert!(runner.trigger(&never, None, "schedule").await.is_none());
        assert_eq!(runner.recent(10).len(), 5);
        let catalog = runner.catalog();
        assert_eq!(
            catalog.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["Write", "Fail", "Reads", "Two steps", "Stops", "Never"]
        );
    }

    #[tokio::test]
    async fn a_command_reading_the_variable_gets_its_parameters_once() {
        let (_dir, runner) = setup();
        // Added to the line as well, printf would print them twice.
        let r = runner.run(TargetKind::Command, "Reads", Some("hi".into()), "api").await;
        assert_eq!((r.status, r.output.as_str()), (RunStatus::Succeeded, "hi"));
        assert_eq!(r.params.as_deref(), Some("hi"));
        let r = runner.run_homekit(TargetKind::Command, "Reads", true, Some(40)).await.unwrap();
        assert_eq!(r.output, "40");
        let r = runner.run_homekit(TargetKind::Command, "Reads", true, None).await.unwrap();
        assert_eq!(r.output, "on");
    }
}
