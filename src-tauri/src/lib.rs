//! The CmdTrigger tray app: the foreground agent with a menu bar icon and its window.

mod commands;
mod state;
mod tray;
mod windows;

use cmdtrigger_core::{Agent, DataDir, Ground, config, logging, store};
use state::AppState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tray::TrayInfo;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second launch opens the window of the one already running.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| windows::open(app, "commands")))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let data = DataDir::resolve(None).map_err(|e| e.to_string())?;
            let first_run = !data.commands().exists();
            store::init_data_dir(&data)?;
            logging::init(Some(&data.debug_log()));
            tracing::info!("CmdTrigger {} using {}", cmdtrigger_core::VERSION, data.root().display());
            let lang = config::load_language(&data);
            app.manage(AppState::new(data.clone(), lang));
            app.manage(TrayInfo::default());
            tray::setup(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(start_agent(handle, data, first_run));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::set_language,
            commands::open_url,
            commands::general_get,
            commands::general_save,
            commands::load_commands,
            commands::save_commands,
            commands::read_commands_text,
            commands::write_commands_text,
            commands::examples,
            commands::run_item,
            commands::history,
            commands::clear_history,
            commands::load_automations,
            commands::save_automations,
            commands::new_id,
            commands::current_network,
            commands::installed_apps,
            commands::usb_devices,
            commands::wake,
            commands::homekit_get,
            commands::homekit_save,
            commands::mqtt_get,
            commands::mqtt_save,
            commands::api_get,
            commands::api_save,
            commands::phone_get,
            commands::phone_save,
            commands::phone_pair_start,
            commands::phone_pair_cancel,
            commands::phone_remove,
            commands::wake_access_enable,
            commands::server_get,
            commands::server_sign_in,
            commands::server_sign_out,
        ])
        .build(tauri::generate_context!())
        .expect("error while building CmdTrigger")
        .run(|app, event| {
            // A tray app keeps running with no windows open; only Quit exits.
            if let RunEvent::ExitRequested { api, code, .. } = event
                && code.is_none()
                && !app.state::<AppState>().quitting.load(Ordering::SeqCst)
            {
                api.prevent_exit();
            }
        });
}

/// Start the foreground agent, then keep the tray and the window in step with it.
async fn start_agent(app: AppHandle, data: DataDir, first_run: bool) {
    let agent = Agent::start(data, Ground::Foreground).await;
    *app.state::<AppState>().agent.lock().await = Some(agent.clone());
    *app.state::<TrayInfo>().catalog.lock().unwrap() = agent.catalog();
    *app.state::<TrayInfo>().link.lock().unwrap() = Some(agent.link_status());
    tray::refresh(&app);
    if first_run {
        enable_autostart(&app);
        windows::open(&app, "commands");
    }

    // Runs → the window's activity list.
    let mut runs = agent.runner().subscribe();
    let to_window = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(record) = runs.recv().await {
            let _ = to_window.emit("run", record);
        }
    });
    // Command or automation changes → the tray's Run menu and the window.
    let mut changes = agent.subscribe_changes();
    let (to_tray, for_catalog) = (app.clone(), agent.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            match changes.recv().await {
                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
            *to_tray.state::<TrayInfo>().catalog.lock().unwrap() = for_catalog.catalog();
            tray::refresh(&to_tray);
            let _ = to_tray.emit("catalog-changed", ());
        }
    });
    // The server link's status → the tray and Settings. (Restarting the link replaces the
    // watch, so this polls cheaply instead.)
    let (to_tray, for_link) = (app.clone(), agent.clone());
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        loop {
            let now = for_link.link_status();
            if last.as_ref() != Some(&now) {
                *to_tray.state::<TrayInfo>().link.lock().unwrap() = Some(now.clone());
                tray::refresh(&to_tray);
                let _ = to_tray.emit("server-status", &now);
                last = Some(now);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// Start at login, set once on the first run of an installed build (Settings can turn it off).
fn enable_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    if cfg!(debug_assertions) {
        return;
    }
    if let Err(e) = app.autolaunch().enable() {
        tracing::warn!("Could not enable start at login: {e}");
    }
}

pub fn quit(app: &AppHandle) {
    app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(agent) = app.state::<AppState>().agent.lock().await.take() {
            agent.shutdown().await;
        }
        app.exit(0);
    });
}
