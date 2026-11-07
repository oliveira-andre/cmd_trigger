//! The main window. It opens on demand from the tray; the URL hash picks the view.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";

/// Show the main window at `route` (e.g. "commands", "settings/server").
pub fn open(app: &AppHandle, route: &str) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to(MAIN, "navigate", route);
        return;
    }
    let url = WebviewUrl::App(format!("index.html#{route}").into());
    let built = WebviewWindowBuilder::new(app, MAIN, url)
        .title("CmdTrigger")
        .inner_size(1080.0, 740.0)
        .min_inner_size(760.0, 520.0)
        .background_color(tauri::window::Color(11, 14, 17, 255))
        .center()
        .focused(true)
        .build();
    match built {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => tracing::warn!("Could not open the window: {e}"),
    }
}
