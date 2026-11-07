//! Reading and writing the data files: `commands.json` (with its backup), `automations.json`,
//! and the small settings files.

use crate::automation::Automation;
use crate::model::Command;
use crate::paths::DataDir;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::Write;
use std::path::Path;

/// Write to a sibling temp file, then rename it over the target, so a crash never leaves a
/// half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_with_mode(path, bytes, None)
}

/// Like [`write_atomic`], readable only by the owner (tokens, passwords, API keys).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_with_mode(path, bytes, Some(0o600))
}

fn write_with_mode(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".new");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut file = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// JSON with a one-space indent (the command files).
pub fn to_json_1(value: &impl Serialize) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    value.serialize(&mut ser).expect("JSON values always serialize");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

/// JSON with a two-space indent (settings and automations).
pub fn to_json_2(value: &impl Serialize) -> String {
    serde_json::to_string_pretty(value).expect("JSON values always serialize")
}

/// A settings file, or the default when it is missing or unreadable.
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not read {0}: {1}")]
    Read(String, std::io::Error),
    #[error("{0} is not valid: {1}")]
    Parse(String, String),
    #[error("could not write {0}: {1}")]
    Write(String, std::io::Error),
}

pub fn parse_commands(text: &str) -> Result<Vec<Command>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Value::Array(items) = value else { return Err("the top level must be a list".into()) };
    let commands: Vec<Command> = items
        .into_iter()
        .map(|item| match item {
            Value::Object(map) => Ok(Command(map).normalize()),
            other => Err(format!("every command must be an object, found {other}")),
        })
        .collect::<Result<_, _>>()?;
    for (i, c) in commands.iter().enumerate() {
        if c.name().trim().is_empty() {
            return Err(format!("command {} has no name", i + 1));
        }
        if commands[..i].iter().any(|o| o.name().eq_ignore_ascii_case(c.name())) {
            return Err(format!("two commands are named \"{}\"", c.name()));
        }
    }
    Ok(commands)
}

pub fn load_commands(path: &Path) -> Result<Vec<Command>, StoreError> {
    let text = std::fs::read_to_string(path).map_err(|e| StoreError::Read(path.display().to_string(), e))?;
    parse_commands(&text).map_err(|e| StoreError::Parse(path.display().to_string(), e))
}

/// For readers that must not fail: a missing or broken file reads as no commands.
pub fn load_commands_lenient(path: &Path) -> Vec<Command> {
    load_commands(path).unwrap_or_default()
}

pub fn save_commands(path: &Path, commands: &[Command]) -> Result<(), StoreError> {
    write_atomic(path, to_json_1(&commands).as_bytes()).map_err(|e| StoreError::Write(path.display().to_string(), e))
}

/// Read `commands.json`; when it doesn't parse, put the last good copy back and use that.
/// When it does, refresh the backup.
pub fn load_with_backup(data: &DataDir) -> Result<Vec<Command>, StoreError> {
    let path = data.commands();
    let backup = data.commands_backup();
    let text = std::fs::read_to_string(&path).map_err(|e| StoreError::Read(path.display().to_string(), e))?;
    match parse_commands(&text) {
        Ok(commands) => {
            let _ = write_atomic(&backup, text.as_bytes());
            Ok(commands)
        }
        Err(problem) => {
            tracing::warn!("{} is not valid ({problem}). Restoring the last good copy.", path.display());
            let good =
                std::fs::read_to_string(&backup).map_err(|e| StoreError::Read(backup.display().to_string(), e))?;
            let commands = parse_commands(&good).map_err(|e| StoreError::Parse(backup.display().to_string(), e))?;
            write_atomic(&path, good.as_bytes()).map_err(|e| StoreError::Write(path.display().to_string(), e))?;
            Ok(commands)
        }
    }
}

pub fn load_automations(path: &Path) -> Result<Vec<Automation>, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| StoreError::Parse(path.display().to_string(), e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(StoreError::Read(path.display().to_string(), e)),
    }
}

pub fn load_automations_lenient(path: &Path) -> Vec<Automation> {
    load_automations(path).unwrap_or_default()
}

pub fn save_automations(path: &Path, automations: &[Automation]) -> Result<(), StoreError> {
    write_atomic(path, to_json_2(&automations).as_bytes()).map_err(|e| StoreError::Write(path.display().to_string(), e))
}

pub fn default_commands_for_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        include_str!("../defaults/mac.json")
    } else if cfg!(windows) {
        include_str!("../defaults/windows.json")
    } else if std::fs::read_to_string("/etc/os-release").is_ok_and(|s| s.to_lowercase().contains("raspbian")) {
        include_str!("../defaults/rpi.json")
    } else {
        include_str!("../defaults/linux.json")
    }
}

/// The built-in example library for this platform.
pub fn examples() -> Vec<Value> {
    let os = std::env::consts::OS;
    let all: Vec<Value> = serde_json::from_str(include_str!("../defaults/examples.json")).expect("valid examples");
    all.into_iter()
        .filter(|e| matches!(e.get("os").and_then(Value::as_str), Some(o) if o == os || o == "any"))
        .collect()
}

/// First run: make the folder and seed `commands.json` (imported from a TRIGGERcmd agent's
/// `~/.TRIGGERcmdData` when there is one, otherwise the platform's starter list).
pub fn init_data_dir(data: &DataDir) -> std::io::Result<()> {
    data.ensure()?;
    let commands = data.commands();
    if !commands.exists() {
        let legacy = dirs::home_dir().map(|h| h.join(".TRIGGERcmdData").join("commands.json"));
        let imported = legacy.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse_commands(&t).ok());
        match imported {
            Some(list) if !list.is_empty() => {
                tracing::info!("Imported {} commands from ~/.TRIGGERcmdData", list.len());
                write_atomic(&commands, to_json_1(&list).as_bytes())?;
            }
            _ => write_atomic(&commands, default_commands_for_platform().as_bytes())?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_examples_parse() {
        for text in [
            include_str!("../defaults/mac.json"),
            include_str!("../defaults/windows.json"),
            include_str!("../defaults/linux.json"),
            include_str!("../defaults/rpi.json"),
        ] {
            assert!(!parse_commands(text).unwrap().is_empty());
        }
        assert!(
            parse_commands(include_str!("../defaults/examples.json")).is_err(),
            "examples repeat names across OSes"
        );
        assert!(!examples().is_empty() || cfg!(not(any(target_os = "macos", target_os = "linux", windows))));
    }

    #[test]
    fn duplicate_and_blank_names_are_refused() {
        assert!(parse_commands(r#"[{"name":"A","command":"a"},{"name":"a","command":"b"}]"#).is_err());
        assert!(parse_commands(r#"[{"name":" ","command":"a"}]"#).is_err());
    }

    #[test]
    fn broken_file_is_restored_from_backup() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        std::fs::write(data.commands(), include_str!("../defaults/linux.json")).unwrap();
        let good = load_with_backup(&data).unwrap();
        std::fs::write(data.commands(), "[{ not json").unwrap();
        assert_eq!(load_with_backup(&data).unwrap(), good);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_private(&path, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
