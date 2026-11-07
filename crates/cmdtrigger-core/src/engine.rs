//! Fires automations on their own: schedules, intervals, startup, wake from sleep, network
//! changes, USB devices, displays and file changes. (Webhooks arrive through the local API.)
//!
//! A 5-second tick drives the clock-based triggers and wake detection; the system monitors
//! (network, USB, displays) are polled every 10 seconds, and only while some enabled
//! automation uses them. The first poll only records the starting state.

use crate::automation::{Automation, DeviceEvent, NetworkEvent, Trigger, Weekday, parse_hhmm};
use crate::monitors::{self, NetworkInfo, WakeDetector};
use crate::runner::Runner;
use chrono::{Datelike, Timelike};
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

pub const TICK: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_secs(3);
const SLEEP_THRESHOLD: Duration = Duration::from_secs(30);
const FILE_SETTLE: Duration = Duration::from_millis(800);

pub struct Engine {
    cancel: CancellationToken,
}

impl Engine {
    /// Start firing. `changes` fires when commands.json or automations.json changes.
    pub fn start(runner: Arc<Runner>, changes: broadcast::Receiver<()>) -> Self {
        let cancel = CancellationToken::new();
        tokio::spawn(run(runner, changes, cancel.clone()));
        Self { cancel }
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn mine(runner: &Runner) -> Vec<Automation> {
    let ground = runner.ground().as_str();
    runner.automations().into_iter().filter(|a| a.enabled && a.ground == ground).collect()
}

fn fire(runner: &Arc<Runner>, a: &Automation, params: Option<String>, source: &str) {
    let (runner, a, source) = (runner.clone(), a.clone(), source.to_string());
    tokio::spawn(async move {
        runner.trigger(&a, params, &source).await;
    });
}

#[derive(Default)]
struct Seen {
    network: Option<Option<NetworkInfo>>,
    usb: Option<Vec<String>>,
    displays: Option<u32>,
}

async fn run(runner: Arc<Runner>, mut changes: broadcast::Receiver<()>, cancel: CancellationToken) {
    let mut automations = mine(&runner);
    let (file_tx, mut file_rx) = mpsc::unbounded_channel::<String>();
    let mut watchers = watch_files(&automations, &file_tx);
    let mut last_scheduled: HashMap<String, String> = HashMap::new();
    let mut last_interval: HashMap<String, Instant> = HashMap::new();
    let mut wake = WakeDetector::default();
    let mut seen = Seen::default();
    let mut file_pending: HashMap<String, Instant> = HashMap::new();
    let mut ticks: u64 = 0;

    // Startup triggers, once the rest of the agent is up.
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(STARTUP_DELAY) => {}
    }
    for a in &automations {
        if a.triggers.iter().any(|t| matches!(t, Trigger::Startup)) {
            fire(&runner, a, None, "startup");
        }
    }

    let mut interval = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = changes.recv() => {
                if matches!(changed, Err(broadcast::error::RecvError::Closed)) { return }
                automations = mine(&runner);
                watchers = watch_files(&automations, &file_tx);
                continue;
            }
            Some(id) = file_rx.recv() => {
                file_pending.insert(id, Instant::now());
                continue;
            }
            _ = interval.tick() => {}
        }
        ticks += 1;
        let _ = &watchers;

        if wake.check(SLEEP_THRESHOLD) {
            tracing::info!("The computer woke from sleep.");
            for a in automations.iter().filter(|a| a.triggers.iter().any(|t| matches!(t, Trigger::Wake))) {
                fire(&runner, a, None, "wake");
            }
        }

        // Schedules: at most once per matching minute.
        let now = chrono::Local::now();
        let minute = now.hour() * 60 + now.minute();
        let stamp = now.format("%Y-%m-%d %H:%M").to_string();
        let today = Weekday::from_chrono(now.weekday());
        for a in &automations {
            for (i, t) in a.triggers.iter().enumerate() {
                let key = format!("{}#{i}", a.id);
                match t {
                    Trigger::Schedule { at, days } => {
                        let due = parse_hhmm(at) == Some(minute) && (days.is_empty() || days.contains(&today));
                        if due && last_scheduled.get(&key) != Some(&stamp) {
                            last_scheduled.insert(key, stamp.clone());
                            fire(&runner, a, None, "schedule");
                        }
                    }
                    Trigger::Interval { minutes } if *minutes > 0 => {
                        let last = *last_interval.entry(key.clone()).or_insert_with(Instant::now);
                        if last.elapsed() >= Duration::from_secs(*minutes as u64 * 60) {
                            last_interval.insert(key, Instant::now());
                            fire(&runner, a, None, "schedule");
                        }
                    }
                    _ => {}
                }
            }
        }

        // File changes, once they've settled.
        let settled: Vec<String> =
            file_pending.iter().filter(|(_, t)| t.elapsed() >= FILE_SETTLE).map(|(k, _)| k.clone()).collect();
        for id in settled {
            file_pending.remove(&id);
            if let Some(a) = automations.iter().find(|a| a.id == id) {
                fire(&runner, a, None, "file");
            }
        }

        if ticks.is_multiple_of(2) {
            poll_monitors(&runner, &automations, &mut seen).await;
        }
    }
}

async fn poll_monitors(runner: &Arc<Runner>, automations: &[Automation], seen: &mut Seen) {
    let uses = |f: fn(&Trigger) -> bool| automations.iter().any(|a| a.triggers.iter().any(f));

    if uses(|t| matches!(t, Trigger::Network { .. })) {
        let now = monitors::network().await;
        if let Some(before) = seen.network.replace(now.clone())
            && before.as_ref().and_then(NetworkInfo::id) != now.as_ref().and_then(NetworkInfo::id)
        {
            tracing::info!(
                "Network changed: {} → {}",
                before.as_ref().and_then(NetworkInfo::id).unwrap_or_else(|| "offline".into()),
                now.as_ref().and_then(NetworkInfo::id).unwrap_or_else(|| "offline".into())
            );
            for a in automations {
                let hit = a.triggers.iter().any(|t| match t {
                    Trigger::Network { network, event } => {
                        let side = if *event == NetworkEvent::Joined { &now } else { &before };
                        side.as_ref().is_some_and(|n| network.trim().is_empty() || n.matches(network))
                    }
                    _ => false,
                });
                if hit {
                    let id = now.as_ref().and_then(NetworkInfo::id);
                    fire(runner, a, id, "network");
                }
            }
        }
    } else {
        seen.network = None;
    }

    if uses(|t| matches!(t, Trigger::Usb { .. })) {
        let now = monitors::usb_devices().await;
        if let Some(before) = seen.usb.replace(now.clone()) {
            let added = difference(&now, &before);
            let removed = difference(&before, &now);
            for (names, event) in [(added, DeviceEvent::Connected), (removed, DeviceEvent::Disconnected)] {
                for device in names {
                    tracing::info!(
                        "USB device {}: {device}",
                        if event == DeviceEvent::Connected { "connected" } else { "removed" }
                    );
                    for a in automations {
                        let hit = a.triggers.iter().any(|t| {
                            matches!(t, Trigger::Usb { event: e, name } if *e == event
                                && (name.trim().is_empty() || device.to_lowercase().contains(&name.trim().to_lowercase())))
                        });
                        if hit {
                            fire(runner, a, Some(device.clone()), "usb");
                        }
                    }
                }
            }
        }
    } else {
        seen.usb = None;
    }

    if uses(|t| matches!(t, Trigger::Display { .. })) {
        if let Some(now) = monitors::display_count().await
            && let Some(before) = seen.displays.replace(now)
            && before != now
        {
            let event = if now > before { DeviceEvent::Connected } else { DeviceEvent::Disconnected };
            tracing::info!("Displays: {before} → {now}");
            for a in automations {
                if a.triggers.iter().any(|t| matches!(t, Trigger::Display { event: e } if *e == event)) {
                    fire(runner, a, Some(now.to_string()), "display");
                }
            }
        }
    } else {
        seen.displays = None;
    }
}

/// Items in `a` not matched one-for-one in `b` (devices can share a name).
fn difference(a: &[String], b: &[String]) -> Vec<String> {
    let mut rest = b.to_vec();
    a.iter()
        .filter(|x| match rest.iter().position(|y| y == *x) {
            Some(i) => {
                rest.remove(i);
                false
            }
            None => true,
        })
        .cloned()
        .collect()
}

/// One watcher per file trigger; events carry the automation id.
fn watch_files(automations: &[Automation], tx: &mpsc::UnboundedSender<String>) -> Vec<notify::RecommendedWatcher> {
    let mut watchers = Vec::new();
    for a in automations {
        for t in &a.triggers {
            let Trigger::File { path, recursive } = t else { continue };
            let path = expand_home(path);
            let (tx, id) = (tx.clone(), a.id.clone());
            let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && !matches!(event.kind, notify::EventKind::Access(_))
                {
                    let _ = tx.send(id.clone());
                }
            });
            let mode = if *recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            match watcher {
                Ok(mut w) => match w.watch(&path, mode) {
                    Ok(()) => watchers.push(w),
                    Err(e) => tracing::warn!("\"{}\": can't watch {}: {e}", a.name, path.display()),
                },
                Err(e) => tracing::warn!("File watching unavailable: {e}"),
            }
        }
    }
    watchers
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_differences_count_duplicates() {
        let before = vec!["Hub".to_string(), "Hub".into(), "Mouse".into()];
        let after = vec!["Hub".to_string(), "Keyboard".into(), "Mouse".into()];
        assert_eq!(difference(&after, &before), vec!["Keyboard"]);
        assert_eq!(difference(&before, &after), vec!["Hub"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_and_file_triggers_fire() {
        let dir = tempfile::tempdir().unwrap();
        let data = crate::DataDir::at(dir.path());
        let watched = dir.path().join("inbox");
        std::fs::create_dir_all(&watched).unwrap();
        std::fs::write(data.commands(), "[]").unwrap();
        let log = dir.path().join("log.txt");
        let automations = serde_json::json!([
            {"id": "s", "name": "At start", "triggers": [{"type": "startup"}],
             "steps": [{"type": "shell", "line": format!("echo start >> '{}'", log.display())}]},
            {"id": "f", "name": "On file", "triggers": [{"type": "file", "path": watched.display().to_string()}],
             "steps": [{"type": "shell", "line": format!("echo file >> '{}'", log.display())}]}
        ]);
        std::fs::write(data.automations(), automations.to_string()).unwrap();
        let runner = Runner::new(data, crate::Ground::Foreground);
        let (tx, _) = broadcast::channel(4);
        let _engine = Engine::start(runner.clone(), tx.subscribe());
        tokio::time::sleep(STARTUP_DELAY + Duration::from_millis(500)).await;
        std::fs::write(watched.join("new.txt"), "x").unwrap();
        for _ in 0..40 {
            if std::fs::read_to_string(&log).is_ok_and(|s| s.contains("file")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("start") && text.contains("file"), "{text}");
        let sources: Vec<String> = runner.recent(10).into_iter().map(|r| r.source).collect();
        assert!(sources.contains(&"startup".to_string()) && sources.contains(&"file".to_string()));
    }
}
