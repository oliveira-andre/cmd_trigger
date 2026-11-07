//! The web dashboard: one static page (served from the binary) that uses the JSON API.
//! It's installable on a phone's home screen.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use std::sync::Arc;

const INDEX: &str = include_str!("../dashboard/index.html");
const APP_JS: &str = include_str!("../dashboard/app.js");
const APP_CSS: &str = include_str!("../dashboard/app.css");
const ICON: &str = include_str!("../dashboard/icon.svg");
const MANIFEST: &str = include_str!("../dashboard/manifest.webmanifest");

fn file(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body)
}

pub fn routes() -> Router<Arc<crate::Server>> {
    Router::new()
        .route("/", get(|| async { file("text/html; charset=utf-8", INDEX) }))
        .route("/app.js", get(|| async { file("text/javascript; charset=utf-8", APP_JS) }))
        .route("/app.css", get(|| async { file("text/css; charset=utf-8", APP_CSS) }))
        .route("/icon.svg", get(|| async { file("image/svg+xml", ICON) }))
        .route("/manifest.webmanifest", get(|| async { file("application/manifest+json", MANIFEST) }))
}
