//! The tray (menu bar) icon: open the app, run any command or automation in one click, see
//! the server link, and quit.

use crate::state::AppState;
use crate::windows;
use cmdtrigger_core::config::{self, Lang};
use cmdtrigger_core::i18n::t;
use cmdtrigger_core::link::LinkStatus;
use cmdtrigger_core::service;
use cmdtrigger_protocol::{CatalogEntry, TargetKind};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

const TRAY: &str = "main";

/// What the menu shows; kept current by the agent's event tasks.
#[derive(Default)]
pub struct TrayInfo {
    pub catalog: Mutex<Vec<CatalogEntry>>,
    pub link: Mutex<Option<LinkStatus>>,
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY)
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?)
        .icon_as_template(true)
        .tooltip("CmdTrigger")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY) else { return };
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
    let lang = app.state::<AppState>().lang();
    let _ = tray.set_tooltip(Some(format!(
        "CmdTrigger — {}",
        t(lang, link_label(&app.state::<TrayInfo>().link.lock().unwrap()))
    )));
}

fn link_label(link: &Option<LinkStatus>) -> &'static str {
    match link {
        Some(LinkStatus::Online) => "Server: connected",
        Some(LinkStatus::Connecting) => "Server: connecting…",
        Some(LinkStatus::Offline { .. }) => "Server: offline",
        Some(LinkStatus::Rejected { .. }) => "Server: sign in again",
        Some(LinkStatus::Off) | None => "Server: not set up",
    }
}

fn item(app: &AppHandle, id: &str, text: &str, enabled: bool) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let info = app.state::<TrayInfo>();
    let lang = state.lang();
    let version = item(app, "version", &format!("CmdTrigger {}", cmdtrigger_core::VERSION), false)?;
    let server = item(app, "server", t(lang, link_label(&info.link.lock().unwrap())), true)?;
    let open = item(app, "open", t(lang, "Open CmdTrigger"), true)?;

    let catalog = info.catalog.lock().unwrap().clone();
    let mut run_items: Vec<MenuItem<Wry>> = Vec::new();
    for e in &catalog {
        let kind = if e.kind == TargetKind::Command { "c" } else { "a" };
        let label = if e.icon.is_empty() { e.name.clone() } else { format!("{}  {}", e.icon, e.name) };
        let label = if e.allow_params { format!("{label}…") } else { label };
        run_items.push(item(app, &format!("run:{kind}:{}", e.id), &label, true)?);
    }
    if run_items.is_empty() {
        run_items.push(item(app, "none", t(lang, "Nothing yet"), false)?);
    }
    let run_refs: Vec<&dyn IsMenuItem<Wry>> = run_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let run = Submenu::with_items(app, t(lang, "Run"), true, &run_refs)?;

    let languages: Vec<CheckMenuItem<Wry>> = Lang::ALL
        .iter()
        .map(|l| {
            CheckMenuItem::with_id(app, format!("lang-{}", l.code()), l.native_name(), true, *l == lang, None::<&str>)
        })
        .collect::<tauri::Result<_>>()?;
    let language_refs: Vec<&dyn IsMenuItem<Wry>> = languages.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let language = Submenu::with_items(app, t(lang, "Language"), true, &language_refs)?;
    let quit = item(app, "quit", t(lang, "Quit"), true)?;
    let (sep1, sep2, sep3) =
        (PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?);

    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&version, &server, &sep1, &open, &run, &sep2];
    let background;
    if service::supported() {
        let install = item(app, "svc-install", t(lang, "Install Background Service"), true)?;
        let remove = item(app, "svc-remove", t(lang, "Remove Background Service"), true)?;
        background = Submenu::with_items(app, t(lang, "Background Service"), true, &[&install, &remove])?;
        items.push(&background);
    }
    items.extend([&language as &dyn IsMenuItem<Wry>, &sep3, &quit]);
    Menu::with_items(app, &items)
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => windows::open(app, "commands"),
        "server" => windows::open(app, "settings/server"),
        "svc-install" | "svc-remove" => run_service_action(app, id == "svc-install"),
        "quit" => crate::quit(app),
        lang if lang.starts_with("lang-") => set_language(app, Lang::from_code(&lang[5..])),
        run if run.starts_with("run:") => {
            let mut parts = run.splitn(3, ':').skip(1);
            let (Some(kind), Some(target)) = (parts.next(), parts.next()) else { return };
            let kind = if kind == "c" { TargetKind::Command } else { TargetKind::Automation };
            let entry = app
                .state::<TrayInfo>()
                .catalog
                .lock()
                .unwrap()
                .iter()
                .find(|e| e.kind == kind && e.id == target)
                .cloned();
            let Some(entry) = entry else { return };
            if entry.allow_params {
                // Ask for the parameters in the app.
                windows::open(
                    app,
                    &format!("run/{}/{}", if kind == TargetKind::Command { "command" } else { "automation" }, entry.id),
                );
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(agent) = app.state::<AppState>().agent().await {
                    agent.runner().run(kind, &entry.id, None, "tray").await;
                }
            });
        }
        _ => {}
    }
}

pub fn set_language(app: &AppHandle, lang: Lang) {
    let state = app.state::<AppState>();
    *state.lang.lock().unwrap() = lang;
    if let Err(e) = config::save_language(&state.data, lang) {
        tracing::warn!("Could not save the language: {e}");
    }
    refresh(app);
    let _ = app.emit("language-changed", lang.code());
}

/// Install or remove the background service with admin rights (pkexec / UAC).
fn run_service_action(app: &AppHandle, install: bool) {
    let folder = app.state::<AppState>().data.root().display().to_string();
    let args: Vec<&str> = if install { vec!["--install-daemon", &folder] } else { vec!["--remove-daemon"] };
    match service::elevated_command(&args) {
        Ok(mut command) => {
            std::thread::spawn(move || match command.status() {
                Ok(status) => tracing::info!("Background service command finished: {status}"),
                Err(e) => tracing::warn!("Background service command failed: {e}"),
            });
        }
        Err(e) => tracing::warn!("{e}"),
    }
}
