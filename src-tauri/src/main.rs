// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cmdtrigger_core::headless::{self, Mode};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match headless::parse(&args) {
        // No headless flag (or only macOS's -psn_…): the tray app.
        Mode::Gui => cmdtrigger_lib::run(),
        // --console, --run, --trigger …: the same agent without a window, like the original.
        mode => {
            let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
            std::process::exit(runtime.block_on(headless::run(mode)));
        }
    }
}
