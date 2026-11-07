//! The run log: every command and automation run, newest last, in `history.jsonl`.

use cmdtrigger_protocol::{RunStatus, TargetKind};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

/// What the log keeps in memory and on disk.
pub const KEEP: usize = 500;
/// Output kept per run.
pub const MAX_OUTPUT: usize = 4000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    /// Unix milliseconds.
    pub at: u64,
    /// "tray", "homekit", "mqtt", "webhook", "api", "mcp", "server", "schedule", "startup"…
    pub source: String,
    pub kind: TargetKind,
    /// A command's name or an automation's id.
    pub target: String,
    pub name: String,
    pub params: Option<String>,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub output: String,
    pub duration_ms: u64,
}

/// Keep the end of long output (that's where errors are).
pub fn clip(text: &str) -> String {
    if text.len() <= MAX_OUTPUT {
        return text.to_string();
    }
    let mut start = text.len() - MAX_OUTPUT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct History {
    path: PathBuf,
    entries: VecDeque<RunRecord>,
    lines_on_disk: usize,
}

impl History {
    pub fn open(path: PathBuf) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut entries: VecDeque<RunRecord> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let lines_on_disk = entries.len();
        while entries.len() > KEEP {
            entries.pop_front();
        }
        Self { path, entries, lines_on_disk }
    }

    pub fn push(&mut self, record: RunRecord) {
        self.entries.push_back(record.clone());
        while self.entries.len() > KEEP {
            self.entries.pop_front();
        }
        if self.lines_on_disk >= KEEP * 2 {
            // Compact: rewrite with what's kept.
            let text: String = self.entries.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect();
            if crate::store::write_atomic(&self.path, text.as_bytes()).is_ok() {
                self.lines_on_disk = self.entries.len();
            }
            return;
        }
        let line = serde_json::to_string(&record).expect("records serialize") + "\n";
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if appended.is_ok() {
            self.lines_on_disk += 1;
        }
    }

    /// Newest first.
    pub fn recent(&self, limit: usize) -> Vec<RunRecord> {
        self.entries.iter().rev().take(limit).cloned().collect()
    }

    /// Forget every run. The file goes first, so a failed write leaves the two in step.
    pub fn clear(&mut self) -> std::io::Result<()> {
        crate::store::write_atomic(&self.path, b"")?;
        self.entries.clear();
        self.lines_on_disk = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(i: usize) -> RunRecord {
        RunRecord {
            id: i.to_string(),
            at: i as u64,
            source: "test".into(),
            kind: TargetKind::Command,
            target: "x".into(),
            name: "x".into(),
            params: None,
            status: RunStatus::Succeeded,
            exit_code: Some(0),
            output: String::new(),
            duration_ms: 1,
        }
    }

    #[test]
    fn keeps_the_newest_and_compacts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut h = History::open(path.clone());
        for i in 0..(KEEP * 2 + 10) {
            h.push(record(i));
        }
        assert_eq!(h.recent(1)[0].id, (KEEP * 2 + 9).to_string());
        let reopened = History::open(path.clone());
        assert_eq!(reopened.recent(KEEP).len(), KEEP);
        assert!(std::fs::read_to_string(&path).unwrap().lines().count() < KEEP * 2);
        assert_eq!(clip(&"é".repeat(5000)).chars().count(), MAX_OUTPUT / 2 + 1);
    }

    #[test]
    fn clear_forgets_everything_and_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut h = History::open(path.clone());
        for i in 0..3 {
            h.push(record(i));
        }
        h.clear().unwrap();
        assert!(h.recent(10).is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(History::open(path.clone()).recent(10).is_empty());

        h.push(record(9));
        let reopened = History::open(path);
        assert_eq!(reopened.recent(10).len(), 1);
        assert_eq!(reopened.recent(1)[0].id, "9");
    }

    #[test]
    fn a_failed_clear_keeps_the_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::open(dir.path().join("history.jsonl"));
        h.push(record(1));
        // The file's folder is really a file, so nothing can be written there.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "").unwrap();
        h.path = blocker.join("history.jsonl");
        assert!(h.clear().is_err());
        assert_eq!(h.recent(10).len(), 1);
    }
}
