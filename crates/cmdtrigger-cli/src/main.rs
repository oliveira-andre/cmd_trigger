//! `cmdtrigger`: the agent without a tray, for servers and Raspberry Pis (the original's
//! `node src/agent.js --console` and `daemon.js --run`).

use cmdtrigger_core::headless::{self, Mode};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match headless::parse(&args) {
        // Without a mode flag the CLI behaves like `--console`.
        Mode::Gui if args.is_empty() => Mode::Console,
        Mode::Gui => Mode::Help,
        other => other,
    };
    std::process::exit(headless::run(mode).await);
}
