//! The CmdTrigger agent core: your own commands and automations, run from HomeKit (and Siri),
//! Home Assistant, webhooks, AI assistants (MCP), your phone through your own server, a
//! schedule or system events. Everything that isn't a window lives here, so the tray app, the
//! headless CLI and the background service share one implementation.
//!
//! - [`runner`]: runs commands and automations, keeps the history;
//! - [`automation`] + [`engine`] + [`monitors`]: triggers, conditions, steps;
//! - [`homekit`]: a native HomeKit bridge, every item a light;
//! - [`mqtt`]: Home Assistant entities through MQTT discovery;
//! - [`local_api`] + [`mcp_stdio`]: the local HTTP API, webhooks and MCP;
//! - [`link`]: the connection to your CmdTrigger server;
//! - [`agent`]: all of the above, running;
//! - [`headless`] + [`service`]: the command line and the background service.

pub mod agent;
pub mod apps;
pub mod automation;
pub mod config;
pub mod engine;
pub mod exec;
pub mod headless;
pub mod history;
pub mod homekit;
pub mod i18n;
pub mod link;
pub mod local_api;
pub mod logging;
pub mod mcp_stdio;
pub mod model;
pub mod monitors;
pub mod mqtt;
pub mod notify;
pub mod paths;
pub mod phone;
pub mod runner;
pub mod service;
pub mod store;
pub mod watch;
#[cfg(windows)]
pub mod winservice;
pub mod wol;

pub use agent::{Agent, Ground};
pub use model::Command;
pub use paths::DataDir;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
