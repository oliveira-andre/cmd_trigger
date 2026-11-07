//! The data folder: `~/Library/Application Support/CmdTrigger` on macOS, `%APPDATA%\CmdTrigger`
//! on Windows, `~/.config/cmdtrigger` on Linux. `CMDTRIGGER_DATA_DIR` overrides it, and the
//! background service is told its folder explicitly.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DataDir {
    root: PathBuf,
}

/// The platform's default folder.
pub fn default_root() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        return dirs::config_dir().map(|d| d.join("cmdtrigger"));
    }
    dirs::config_dir().map(|d| d.join("CmdTrigger"))
}

impl DataDir {
    pub fn resolve(explicit: Option<&Path>) -> anyhow::Result<Self> {
        if let Some(path) = explicit {
            return Ok(Self::at(path));
        }
        if let Some(path) = std::env::var_os("CMDTRIGGER_DATA_DIR").filter(|p| !p.is_empty()) {
            return Ok(Self::at(PathBuf::from(path)));
        }
        Ok(Self::at(default_root().ok_or_else(|| anyhow::anyhow!("no home folder"))?))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn commands(&self) -> PathBuf {
        self.file("commands.json")
    }
    pub fn commands_backup(&self) -> PathBuf {
        self.file("commands.json.backup")
    }
    pub fn automations(&self) -> PathBuf {
        self.file("automations.json")
    }
    pub fn history(&self) -> PathBuf {
        self.file("history.jsonl")
    }
    pub fn homekit(&self) -> PathBuf {
        self.file("homekit.json")
    }
    /// HomeKit pairing state (long-term keys, paired controllers, accessory IDs).
    pub fn homekit_cache(&self) -> PathBuf {
        self.file("homekit")
    }
    pub fn mqtt(&self) -> PathBuf {
        self.file("mqtt.json")
    }
    pub fn local_api(&self) -> PathBuf {
        self.file("api.json")
    }
    /// Paired phones and their keys (owner-only).
    pub fn phones(&self) -> PathBuf {
        self.file("phones.json")
    }
    pub fn server(&self) -> PathBuf {
        self.file("server.json")
    }
    pub fn settings(&self) -> PathBuf {
        self.file("settings.json")
    }
    pub fn debug_log(&self) -> PathBuf {
        self.file("debug.log")
    }

    pub fn read_value(&self, path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

/// The computer's name as the OS reports it, without a `.local` suffix.
pub fn hostname() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    name.trim_end_matches(".local").to_string()
}
