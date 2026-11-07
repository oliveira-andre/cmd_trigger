//! The command line, shared by the tray app's binary and the standalone `cmdtrigger` CLI.
//!
//! ```text
//! cmdtrigger [console]              run the agent in this terminal (foreground commands)
//! cmdtrigger run NAME [PARAMS…]     run a command or automation now (through the running agent)
//! cmdtrigger list                   list commands and automations
//! cmdtrigger mcp                    MCP server on stdio for AI assistants
//! cmdtrigger connect URL [--email E] [--create]   link this computer to your server
//! cmdtrigger disconnect             unlink it
//! cmdtrigger wake MAC [BROADCAST]   send a Wake-on-LAN packet
//! cmdtrigger --run [FOLDER]         run the background agent (what the service runs)
//! cmdtrigger --install-daemon [FOLDER] / --remove-daemon   the background service
//! ```

use crate::config::{ApiConfig, ServerConfig, Settings};
use crate::paths::DataDir;
use crate::{Agent, Ground, logging, service, store};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Mode {
    /// No headless arguments: the caller decides (the tray app shows its UI).
    Gui,
    Help,
    Version,
    Console,
    Run {
        target: String,
        params: Option<String>,
    },
    List,
    Mcp,
    /// Pair an iPhone: the running app (or console agent) shows a QR code here.
    Pair {
        json: bool,
    },
    Connect {
        url: String,
        email: Option<String>,
        create: bool,
    },
    Disconnect,
    Wake {
        mac: String,
        broadcast: String,
    },
    Background(Option<PathBuf>),
    /// Started by the Windows service manager.
    Service(Option<PathBuf>),
    InstallDaemon(Option<PathBuf>),
    RemoveDaemon,
}

/// Unknown arguments (macOS's `-psn_…`) are ignored.
pub fn parse(args: &[String]) -> Mode {
    let positional: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with("-psn_")).collect();
    let value_after = |flag: &str| {
        positional.iter().position(|a| *a == flag).and_then(|i| positional.get(i + 1)).map(|s| s.to_string())
    };
    let rest = |from: usize| Some(positional[from..].join(" ")).filter(|s| !s.trim().is_empty());
    match positional.first().copied() {
        None => Mode::Gui,
        Some("-h" | "--help" | "help") => Mode::Help,
        Some("-V" | "--version" | "version") => Mode::Version,
        Some("console" | "--console") => Mode::Console,
        Some("run") => match positional.get(1) {
            Some(target) => Mode::Run { target: target.to_string(), params: rest(2) },
            None => Mode::Help,
        },
        Some("list" | "ls") => Mode::List,
        Some("mcp") => Mode::Mcp,
        Some("pair") => Mode::Pair { json: positional.contains(&"--json") },
        Some("connect") => match positional.get(1).filter(|u| !u.starts_with('-')) {
            Some(url) => Mode::Connect {
                url: url.to_string(),
                email: value_after("--email"),
                create: positional.contains(&"--create"),
            },
            None => Mode::Help,
        },
        Some("disconnect") => Mode::Disconnect,
        Some("wake") => match positional.get(1) {
            Some(mac) => Mode::Wake {
                mac: mac.to_string(),
                broadcast: positional.get(2).map(|s| s.to_string()).unwrap_or_default(),
            },
            None => Mode::Help,
        },
        Some("--run") => Mode::Background(positional.get(1).map(PathBuf::from)),
        Some("--service") => Mode::Service(positional.get(1).map(PathBuf::from)),
        Some("--install-daemon") => Mode::InstallDaemon(positional.get(1).map(PathBuf::from)),
        Some("--remove-daemon") => Mode::RemoveDaemon,
        Some(_) => Mode::Gui,
    }
}

pub const HELP: &str = "\
CmdTrigger: run your own commands and automations from HomeKit, Home Assistant, webhooks,
AI assistants, your phone, or on a schedule.

  cmdtrigger [console]              Run the agent in this terminal
  cmdtrigger run NAME [PARAMS…]     Run a command or automation now
  cmdtrigger list                   List commands and automations
  cmdtrigger mcp                    MCP server on stdin/stdout for AI assistants
  cmdtrigger pair [--json]          Pair the iPhone app (shows a QR code; needs the app running)
  cmdtrigger connect URL [--email ADDRESS] [--create]
                                    Link this computer to your CmdTrigger server
  cmdtrigger disconnect             Unlink it
  cmdtrigger wake MAC [BROADCAST]   Send a Wake-on-LAN packet
  cmdtrigger --run [FOLDER]         Run the background agent (what the service runs)
  cmdtrigger --install-daemon [FOLDER]   Install and start the background service (root/admin)
  cmdtrigger --remove-daemon        Remove it
  cmdtrigger --version";

fn ask(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok()?;
    Some(line.trim().to_string()).filter(|l| !l.is_empty())
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn data_or_exit(folder: Option<&std::path::Path>) -> Result<DataDir, i32> {
    DataDir::resolve(folder).map_err(|e| {
        eprintln!("{e}");
        1
    })
}

/// Run a headless mode to completion. Returns the process exit code.
pub async fn run(mode: Mode) -> i32 {
    match run_inner(mode).await {
        Ok(code) | Err(code) => code,
    }
}

async fn run_inner(mode: Mode) -> Result<i32, i32> {
    match mode {
        Mode::Gui | Mode::Help => println!("{HELP}"),
        Mode::Version => println!("cmdtrigger {}", crate::VERSION),
        Mode::Console | Mode::Background(_) => {
            let (folder, ground) = match &mode {
                Mode::Background(f) => (f.clone(), Ground::Background),
                _ => (None, Ground::Foreground),
            };
            logging::init(None);
            let data = data_or_exit(folder.as_deref())?;
            let agent = Agent::start(data, ground).await;
            println!(
                "CmdTrigger {} agent running for \"{}\". Ctrl+C stops it.",
                ground.as_str(),
                agent.computer_name()
            );
            wait_for_shutdown().await;
            agent.shutdown().await;
        }
        #[cfg(windows)]
        Mode::Service(folder) => {
            return match tokio::task::spawn_blocking(move || crate::winservice::run(folder)).await {
                Ok(Ok(())) => Ok(0),
                _ => Err(1),
            };
        }
        #[cfg(not(windows))]
        Mode::Service(folder) => return Box::pin(run_inner(Mode::Background(folder))).await,
        Mode::Run { target, params } => {
            let data = data_or_exit(None)?;
            return run_now(&data, &target, params).await;
        }
        Mode::Pair { json } => {
            let data = data_or_exit(None)?;
            return pair(&data, json).await;
        }
        Mode::List => {
            let data = data_or_exit(None)?;
            let _ = store::init_data_dir(&data);
            for c in store::load_commands_lenient(&data.commands()) {
                let flags = [
                    (c.allow_params(), "params"),
                    (c.off_command().is_some(), "on/off"),
                    (c.ground() == "background", "background"),
                ];
                let flags: Vec<&str> = flags.iter().filter(|(on, _)| *on).map(|(_, f)| *f).collect();
                let icon = if c.icon().is_empty() { "•" } else { c.icon() };
                let flags = if flags.is_empty() { String::new() } else { format!("  ({})", flags.join(", ")) };
                println!("{icon} {}{flags}", c.name());
            }
            for a in store::load_automations_lenient(&data.automations()) {
                let icon = if a.icon.is_empty() { "⚙" } else { &a.icon };
                println!("{icon} {}  (automation{})", a.name, if a.enabled { "" } else { ", off" });
            }
        }
        Mode::Mcp => {
            let data = data_or_exit(None)?;
            if let Err(e) = crate::mcp_stdio::serve_stdio(data).await {
                eprintln!("{e}");
                return Err(1);
            }
        }
        Mode::Connect { url, email, create } => {
            let data = data_or_exit(None)?;
            let _ = store::init_data_dir(&data);
            let email = email.or_else(|| ask("Email: ")).ok_or(1)?;
            let password = ask(if create { "Choose a password: " } else { "Password: " }).ok_or(1)?;
            let computer = Settings::load(&data).computer_name();
            match crate::link::sign_in(&url, &email, &password, create, &computer).await {
                Ok(config) => {
                    config.save(&data).map_err(|e| {
                        eprintln!("{e}");
                        1
                    })?;
                    println!("Linked \"{computer}\" to {}. Restart the agent (or the app) to connect.", config.url);
                }
                Err(e) => {
                    eprintln!("{e}");
                    return Err(1);
                }
            }
        }
        Mode::Disconnect => {
            let data = data_or_exit(None)?;
            ServerConfig::clear(&data).map_err(|_| 1)?;
            println!("Unlinked. Remove the computer on the server's dashboard too.");
        }
        Mode::Wake { mac, broadcast } => {
            if let Err(e) = crate::wol::wake(&mac, &broadcast).await {
                eprintln!("{e}");
                return Err(1);
            }
            println!("Sent a wake-up packet to {mac}.");
        }
        Mode::InstallDaemon(folder) => match service::install(folder) {
            Ok(message) => println!("{message}"),
            Err(e) => {
                eprintln!("{e}");
                return Err(1);
            }
        },
        Mode::RemoveDaemon => match service::remove() {
            Ok(message) => println!("{message}"),
            Err(e) => {
                eprintln!("{e}");
                return Err(1);
            }
        },
    }
    Ok(0)
}

/// `cmdtrigger run`: through a running agent's local API when there is one (so it shows in
/// the app), otherwise directly.
async fn run_now(data: &DataDir, target: &str, params: Option<String>) -> Result<i32, i32> {
    let config = ApiConfig::load(data);
    let http = reqwest::Client::new();
    for port in [config.port, config.port.saturating_add(1)] {
        let body = serde_json::json!({ "target": target, "params": params, "wait": true, "source": "cli" });
        let url = format!("http://127.0.0.1:{port}/api/v1/run");
        let Ok(response) = http.post(url).bearer_auth(&config.key).json(&body).send().await else { continue };
        if response.status() == 404 {
            continue; // maybe the other agent has it
        }
        let record: serde_json::Value = response.json().await.unwrap_or_default();
        return Ok(report(&record));
    }
    // No agent running: run it here.
    let _ = store::init_data_dir(data);
    let runner = crate::runner::Runner::new(data.clone(), Ground::Foreground);
    let Some((kind, id)) = crate::local_api::resolve(&runner, target, None) else {
        eprintln!("There is no command or automation called \"{target}\".");
        return Err(1);
    };
    let record = runner.run(kind, &id, params, "cli").await;
    Ok(report(&serde_json::to_value(record).unwrap_or_default()))
}

/// Opens a pairing window in the running foreground agent and shows its QR code.
async fn pair(data: &DataDir, json: bool) -> Result<i32, i32> {
    let config = ApiConfig::load(data);
    let base = format!("http://127.0.0.1:{}/api/v1/pair", config.port_for(Ground::Foreground));
    let http = reqwest::Client::new();
    let reply = http.post(&base).bearer_auth(&config.key).send().await.map_err(|_| {
        eprintln!("CmdTrigger isn't running. Open the app (or run `cmdtrigger console`) and try again.");
        1
    })?;
    let ok = reply.status().is_success();
    let body: serde_json::Value = reply.json().await.unwrap_or_default();
    if !ok {
        eprintln!("{}", body.get("error").and_then(|e| e.as_str()).unwrap_or("Pairing couldn't start."));
        return Err(1);
    }
    if json {
        let mut body = body;
        if let Some(o) = body.as_object_mut() {
            o.remove("qrSvg");
        }
        println!("{body}");
        return Ok(0);
    }
    let url = body["url"].as_str().unwrap_or_default();
    if let Ok(code) = qrcode::QrCode::new(url.as_bytes()) {
        let art = code
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build();
        println!("{art}");
    }
    println!("Scan this with your iPhone's camera, or in the CmdTrigger app type the code:\n");
    println!("    {}\n", body["code"].as_str().unwrap_or_default());
    println!("Waiting for the iPhone… (Ctrl+C to stop)");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(reply) = http.get(&base).bearer_auth(&config.key).send().await else { break };
        let now: serde_json::Value = reply.json().await.unwrap_or_default();
        if let Some(name) = now.get("pairedWith").and_then(|n| n.as_str()) {
            println!("Paired with {name}.");
            return Ok(0);
        }
        if now.is_null() {
            break;
        }
    }
    eprintln!("The code expired. Run `cmdtrigger pair` again.");
    Err(1)
}

fn report(record: &serde_json::Value) -> i32 {
    let output = record.get("output").and_then(|v| v.as_str()).unwrap_or("");
    if !output.is_empty() {
        println!("{output}");
    }
    match record.get("status").and_then(|v| v.as_str()) {
        Some("succeeded") => 0,
        other => {
            eprintln!("{}", record.get("error").and_then(|v| v.as_str()).unwrap_or(other.unwrap_or("failed")));
            record.get("exitCode").and_then(|c| c.as_i64()).map(|c| c as i32).filter(|c| *c != 0).unwrap_or(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses() {
        assert_eq!(parse(&args(&[])), Mode::Gui);
        assert_eq!(parse(&args(&["-psn_0_12345"])), Mode::Gui);
        assert_eq!(parse(&args(&["console"])), Mode::Console);
        assert_eq!(
            parse(&args(&["run", "Say", "hello", "world"])),
            Mode::Run { target: "Say".into(), params: Some("hello world".into()) }
        );
        assert_eq!(parse(&args(&["run", "Calculator"])), Mode::Run { target: "Calculator".into(), params: None });
        assert_eq!(parse(&args(&["--run", "/srv/ct"])), Mode::Background(Some("/srv/ct".into())));
        assert_eq!(
            parse(&args(&["connect", "https://cmd.example.com", "--email", "me@example.com", "--create"])),
            Mode::Connect { url: "https://cmd.example.com".into(), email: Some("me@example.com".into()), create: true }
        );
        assert_eq!(
            parse(&args(&["wake", "aa:bb:cc:dd:ee:ff"])),
            Mode::Wake { mac: "aa:bb:cc:dd:ee:ff".into(), broadcast: String::new() }
        );
        assert_eq!(parse(&args(&["mcp"])), Mode::Mcp);
    }
}
