//! What the window asks the app to do.

use crate::state::AppState;
use cmdtrigger_core::automation::{self, Automation};
use cmdtrigger_core::config::{self, ApiConfig, HomeKitConfig, MqttConfig, ServerConfig, Settings};
use cmdtrigger_core::history::RunRecord;
use cmdtrigger_core::{Command, monitors, store};
use cmdtrigger_protocol::TargetKind;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    platform: &'static str,
    language: &'static str,
    data_dir: String,
    computer_name: String,
    hostname: String,
    background_supported: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: cmdtrigger_core::VERSION,
        platform: std::env::consts::OS,
        language: state.lang().code(),
        data_dir: state.data.root().display().to_string(),
        computer_name: Settings::load(&state.data).computer_name(),
        hostname: cmdtrigger_core::paths::hostname(),
        background_supported: cmdtrigger_core::service::supported(),
    }
}

#[tauri::command]
pub fn set_language(app: AppHandle, code: String) {
    crate::tray::set_language(&app, config::Lang::from_code(&code));
}

/// http(s) links only.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only web links open from here".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

// ---------- general ----------

#[tauri::command]
pub fn general_get(app: AppHandle, state: State<'_, AppState>) -> Value {
    let settings = Settings::load(&state.data);
    json!({
        "computerName": settings.computer_name,
        "hostname": cmdtrigger_core::paths::hostname(),
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "macAddresses": cmdtrigger_core::wol::mac_addresses(),
    })
}

#[tauri::command]
pub async fn general_save(
    app: AppHandle,
    state: State<'_, AppState>,
    computer_name: String,
    autostart: bool,
) -> Result<()> {
    let mut settings = Settings::load(&state.data);
    let renamed = settings.computer_name.trim() != computer_name.trim();
    settings.computer_name = computer_name.trim().to_string();
    settings.save(&state.data).map_err(err)?;
    let launcher = app.autolaunch();
    if autostart != launcher.is_enabled().unwrap_or(false) {
        let result = if autostart { launcher.enable() } else { launcher.disable() };
        result.map_err(|e| format!("Could not change start at login: {e}"))?;
    }
    if renamed && let Some(agent) = state.agent().await {
        // The name shows in HomeKit, Home Assistant and on the server.
        agent.restart_mqtt();
        agent.restart_link();
        agent.restart_homekit().await;
        agent.restart_phone().await;
    }
    Ok(())
}

// ---------- commands ----------

#[tauri::command]
pub fn load_commands(state: State<'_, AppState>) -> Result<Vec<Value>> {
    let _ = store::init_data_dir(&state.data);
    let list = store::load_commands(&state.data.commands()).map_err(err)?;
    Ok(list.into_iter().map(|c| Value::Object(c.0)).collect())
}

#[tauri::command]
pub async fn save_commands(state: State<'_, AppState>, commands: Vec<Value>) -> Result<()> {
    let text = store::to_json_1(&commands);
    let list: Vec<Command> = store::parse_commands(&text)?;
    store::save_commands(&state.data.commands(), &list).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.changed();
    }
    Ok(())
}

#[tauri::command]
pub fn read_commands_text(state: State<'_, AppState>) -> Result<String> {
    let _ = store::init_data_dir(&state.data);
    std::fs::read_to_string(state.data.commands()).map_err(err)
}

/// Refuses text that isn't a valid command list.
#[tauri::command]
pub async fn write_commands_text(state: State<'_, AppState>, text: String) -> Result<usize> {
    let list = store::parse_commands(&text)?;
    store::write_atomic(&state.data.commands(), text.as_bytes()).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.changed();
    }
    Ok(list.len())
}

#[tauri::command]
pub fn examples() -> Vec<Value> {
    store::examples()
}

#[tauri::command]
pub async fn run_item(
    state: State<'_, AppState>,
    kind: TargetKind,
    target: String,
    params: Option<String>,
) -> Result<RunRecord> {
    let agent = state.require_agent().await?;
    let params = params.filter(|p| !p.trim().is_empty());
    Ok(agent.runner().run(kind, &target, params, "app").await)
}

#[tauri::command]
pub async fn history(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<RunRecord>> {
    let agent = state.require_agent().await?;
    Ok(agent.runner().recent(limit.unwrap_or(200)))
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>) -> Result<()> {
    let agent = state.require_agent().await?;
    agent.runner().clear_history().map_err(err)
}

// ---------- automations ----------

#[tauri::command]
pub fn load_automations(state: State<'_, AppState>) -> Result<Vec<Automation>> {
    store::load_automations(&state.data.automations()).map_err(err)
}

/// Save all automations. Refuses (with the problems listed) if one wouldn't work.
#[tauri::command]
pub async fn save_automations(state: State<'_, AppState>, automations: Vec<Automation>) -> Result<()> {
    let commands = store::load_commands_lenient(&state.data.commands());
    let names: Vec<&str> = commands.iter().map(Command::name).collect();
    for a in &automations {
        let problems = automation::validate(a, &names);
        if !problems.is_empty() {
            return Err(format!("“{}”: {}", a.name, problems.join(" ")));
        }
    }
    store::save_automations(&state.data.automations(), &automations).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.changed();
    }
    Ok(())
}

#[tauri::command]
pub fn new_id(length: Option<usize>) -> String {
    automation::new_id(length.unwrap_or(10).clamp(6, 32))
}

/// The network this computer is on, as automations identify it.
#[tauri::command]
pub async fn current_network() -> Option<Value> {
    let info = monitors::network().await?;
    let id = info.id()?;
    let label = match (&info.ssid, &info.gateway, &info.gateway_mac) {
        (Some(ssid), _, _) => format!("Wi-Fi “{ssid}”"),
        (None, Some(ip), Some(mac)) => format!("Router {ip} ({mac})"),
        (None, Some(ip), None) => format!("Router {ip}"),
        _ => id.clone(),
    };
    Some(json!({ "id": id, "label": label }))
}

#[tauri::command]
pub async fn installed_apps() -> Vec<cmdtrigger_core::apps::InstalledApp> {
    cmdtrigger_core::apps::installed().await
}

#[tauri::command]
pub async fn usb_devices() -> Vec<String> {
    let mut list = monitors::usb_devices().await;
    list.dedup();
    list
}

#[tauri::command]
pub async fn wake(mac: String) -> Result<()> {
    cmdtrigger_core::wol::wake(&mac, "").await.map_err(err)
}

// ---------- HomeKit ----------

#[tauri::command]
pub async fn homekit_get(state: State<'_, AppState>) -> Result<Value> {
    let config = HomeKitConfig::load(&state.data);
    let status = match state.agent().await {
        Some(agent) => serde_json::to_value(agent.homekit_status().await).map_err(err)?,
        None => Value::Null,
    };
    Ok(json!({ "config": config, "status": status }))
}

#[tauri::command]
pub async fn homekit_save(state: State<'_, AppState>, config: HomeKitConfig) -> Result<Value> {
    let config = HomeKitConfig {
        pin: config.pin.trim().into(),
        username: config.username.trim().to_uppercase(),
        bridge_name: config.bridge_name.trim().into(),
        ..config
    };
    if !config::valid_pin(&config.pin) {
        return Err("The PIN must look like 123-45-678 and not be a simple sequence.".into());
    }
    if !config::valid_username(&config.username) {
        return Err("The bridge ID must look like AA:BB:CC:DD:EE:FF.".into());
    }
    if config.bridge_name.is_empty() {
        return Err("The bridge needs a name.".into());
    }
    config.save(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_homekit().await;
    }
    homekit_get(state).await
}

// ---------- Home Assistant (MQTT) ----------

#[tauri::command]
pub async fn mqtt_get(state: State<'_, AppState>) -> Result<Value> {
    let config = MqttConfig::load(&state.data);
    let status = state.agent().await.and_then(|a| a.mqtt_status());
    Ok(json!({ "config": config, "status": status }))
}

#[tauri::command]
pub async fn mqtt_save(state: State<'_, AppState>, config: MqttConfig) -> Result<Value> {
    if config.enabled && config.host.trim().is_empty() {
        return Err("Enter the MQTT broker's address (your Home Assistant's, with the Mosquitto add-on).".into());
    }
    config.save(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_mqtt();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }
    mqtt_get(state).await
}

// ---------- local API, webhooks, MCP ----------

#[tauri::command]
pub async fn api_get(state: State<'_, AppState>) -> Result<Value> {
    let config = ApiConfig::load(&state.data);
    let status = match state.agent().await {
        Some(a) => serde_json::to_value(a.api_status().await).map_err(err)?,
        None => Value::Null,
    };
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "cmdtrigger".into());
    Ok(json!({ "config": config, "status": status, "executable": exe }))
}

#[tauri::command]
pub async fn api_save(
    state: State<'_, AppState>,
    enabled: bool,
    port: u16,
    allow_lan: bool,
    new_key: bool,
) -> Result<Value> {
    let mut config = ApiConfig::load(&state.data);
    if port < 1024 {
        return Err("Use a port from 1024 to 65534.".into());
    }
    config.enabled = enabled;
    config.port = port;
    config.allow_lan = allow_lan;
    if new_key {
        config.key = config::new_key();
    }
    config.save(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_api().await;
    }
    api_get(state).await
}

// ---------- iPhone ----------

#[tauri::command]
pub async fn phone_get(state: State<'_, AppState>) -> Result<Value> {
    let config = cmdtrigger_core::phone::PhoneConfig::load(&state.data);
    let status = match state.agent().await {
        Some(a) => serde_json::to_value(a.phone_status().await).map_err(err)?,
        None => Value::Null,
    };
    let interfaces = cmdtrigger_core::phone::host::interfaces().await;
    let wake = cmdtrigger_core::phone::host::wake_for_network().await;
    Ok(json!({
        "enabled": config.enabled,
        "port": config.port,
        "phones": config.views(),
        "status": status,
        "interfaces": interfaces,
        "wakeForNetwork": wake,
        "platform": std::env::consts::OS,
    }))
}

#[tauri::command]
pub async fn phone_save(state: State<'_, AppState>, enabled: bool) -> Result<Value> {
    let mut config = cmdtrigger_core::phone::PhoneConfig::load(&state.data);
    config.enabled = enabled;
    config.save(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_phone().await;
    }
    phone_get(state).await
}

#[tauri::command]
pub async fn phone_pair_start(state: State<'_, AppState>) -> Result<cmdtrigger_core::phone::PairingInfo> {
    state.require_agent().await?.start_phone_pairing().await
}

#[tauri::command]
pub async fn phone_pair_cancel(state: State<'_, AppState>) -> Result<()> {
    if let Some(agent) = state.agent().await {
        agent.cancel_phone_pairing().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn phone_remove(state: State<'_, AppState>, id: String) -> Result<Value> {
    cmdtrigger_core::phone::PhoneConfig::remove(&state.data, &id).map_err(err)?;
    phone_get(state).await
}

/// macOS "Wake for network access"; asks for an administrator's password.
#[tauri::command]
pub async fn wake_access_enable(state: State<'_, AppState>) -> Result<Value> {
    cmdtrigger_core::phone::host::enable_wake_for_network().await?;
    phone_get(state).await
}

// ---------- your server ----------

#[tauri::command]
pub async fn server_get(state: State<'_, AppState>) -> Result<Value> {
    let config = ServerConfig::load(&state.data);
    let status = state.agent().await.map(|a| a.link_status());
    Ok(json!({ "url": config.url, "email": config.email, "computerId": config.computer_id, "status": status }))
}

#[tauri::command]
pub async fn server_sign_in(
    state: State<'_, AppState>,
    url: String,
    email: String,
    password: String,
    create: bool,
) -> Result<Value> {
    let computer = Settings::load(&state.data).computer_name();
    let config = cmdtrigger_core::link::sign_in(&url, &email, &password, create, &computer).await.map_err(err)?;
    config.save(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_link();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    }
    server_get(state).await
}

#[tauri::command]
pub async fn server_sign_out(state: State<'_, AppState>) -> Result<Value> {
    ServerConfig::clear(&state.data).map_err(err)?;
    if let Some(agent) = state.agent().await {
        agent.restart_link();
    }
    server_get(state).await
}
