//! Run only the HomeKit bridge against a data folder, for pairing tests:
//!   cargo run -p cmdtrigger-core --example homekit_bridge -- <data folder> [port]
//! Lights are logged instead of run.

use cmdtrigger_core::homekit::HomeKitBridge;
use cmdtrigger_core::{DataDir, Ground};

#[tokio::main]
async fn main() {
    cmdtrigger_core::logging::init(None);
    let mut args = std::env::args().skip(1);
    let data = DataDir::at(args.next().expect("data folder"));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(cmdtrigger_core::homekit::PORT);
    let (changes, _) = tokio::sync::broadcast::channel(4);
    let runner: cmdtrigger_core::homekit::server::Runner = std::sync::Arc::new(|entry, on, level| {
        println!("RAN {} on={on} level={level:?}", entry.name);
    });
    let catalog_runner = cmdtrigger_core::runner::Runner::new(data.clone(), Ground::Foreground);
    let catalog: cmdtrigger_core::homekit::Catalog = std::sync::Arc::new(move || catalog_runner.catalog());
    let bridge = HomeKitBridge::start(data, catalog, changes.subscribe(), port, runner).await;
    println!("STATUS {}", serde_json::to_string(&bridge.status()).unwrap());
    let _ = tokio::signal::ctrl_c().await;
}
