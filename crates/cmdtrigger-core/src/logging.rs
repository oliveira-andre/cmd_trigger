//! Logging to the terminal and, for the tray app, to `debug.log` in the data folder (started
//! fresh on every launch, as the original does).

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct Tee {
    file: Option<Arc<Mutex<std::fs::File>>>,
}

struct TeeWriter {
    file: Option<Arc<Mutex<std::fs::File>>>,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stdout().write_all(buf);
        if let Some(file) = &self.file {
            let _ = file.lock().unwrap().write_all(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = &self.file {
            let _ = file.lock().unwrap().flush();
        }
        std::io::stdout().flush()
    }
}

impl<'a> MakeWriter<'a> for Tee {
    type Writer = TeeWriter;
    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter { file: self.file.clone() }
    }
}

/// Install the global logger. `log_file` truncates and tees into that file. `RUST_LOG`
/// overrides the default `info` level.
pub fn init(log_file: Option<&Path>) {
    let file = log_file.and_then(|p| std::fs::File::create(p).ok()).map(|f| Arc::new(Mutex::new(f)));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,mdns_sd=warn,tungstenite=warn,reqwest=warn,hyper=warn")
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .with_writer(Tee { file })
        .try_init();
}
