//! Watching `commands.json`. Editors save by writing a temp file and renaming it over the
//! original, so the folder is watched and events are filtered by name.

use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Send `()` on `changes` after `file` changes, coalescing bursts within `settle`.
pub fn watch_file(file: &Path, settle: Duration, changes: broadcast::Sender<()>) -> notify::Result<FileWatcher> {
    let target: PathBuf = file.to_path_buf();
    let name = target.file_name().map(|n| n.to_os_string());
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.paths.iter().any(|p| p.file_name().map(|n| n.to_os_string()) == name) {
            let _ = raw_tx.send(());
        }
    })?;
    let folder = target.parent().unwrap_or(Path::new(".")).to_path_buf();
    watcher.watch(&folder, RecursiveMode::NonRecursive)?;
    tokio::spawn(async move {
        while raw_rx.recv().await.is_some() {
            // Let the burst (temp write, rename, metadata) finish, then report it once.
            tokio::time::sleep(settle).await;
            while raw_rx.try_recv().is_ok() {}
            if changes.send(()).is_err() && changes.receiver_count() == 0 {
                // Nobody listens yet; keep watching anyway.
            }
        }
    });
    Ok(FileWatcher { _watcher: watcher })
}
