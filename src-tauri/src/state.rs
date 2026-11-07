//! What the app holds while it runs.

use cmdtrigger_core::config::Lang;
use cmdtrigger_core::{Agent, DataDir};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

pub struct AppState {
    pub data: DataDir,
    pub lang: Mutex<Lang>,
    pub agent: tokio::sync::Mutex<Option<Agent>>,
    /// Set by Quit; any other exit request (last window closed) is ignored.
    pub quitting: AtomicBool,
}

impl AppState {
    pub fn new(data: DataDir, lang: Lang) -> Self {
        Self { data, lang: Mutex::new(lang), agent: tokio::sync::Mutex::new(None), quitting: AtomicBool::new(false) }
    }

    pub fn lang(&self) -> Lang {
        *self.lang.lock().unwrap()
    }

    pub async fn agent(&self) -> Option<Agent> {
        self.agent.lock().await.clone()
    }

    /// The agent, or an error the UI can show while it starts.
    pub async fn require_agent(&self) -> Result<Agent, String> {
        self.agent().await.ok_or_else(|| "CmdTrigger is still starting. Try again in a moment.".to_string())
    }
}
