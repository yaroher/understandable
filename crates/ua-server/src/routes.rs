//! HTTP route definitions split into `api` (JSON) and assets (static).

pub mod api;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::assets::Dashboard;
use crate::state::AppState;

pub fn assets_router() -> Router<Arc<AppState>> {
    // Explicit `/assets/*path` covers Vite's hashed bundle paths; anything
    // else falls through to `index.html` so client-side routes work on
    // hard reload.
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset_relative))
        .fallback(serve_index)
}

async fn serve_index() -> Response {
    serve_named("index.html")
}

async fn serve_asset_relative(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return serve_named("index.html");
    }
    serve_named(path)
}

fn serve_named(name: &str) -> Response {
    let Some(file) = Dashboard::get(name) else {
        return (
            StatusCode::NOT_FOUND,
            format!("dashboard asset not found: {name}"),
        )
            .into_response();
    };
    let mime = mime_for(name);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(file.data.into_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else if lower.ends_with(".woff") {
        "font/woff"
    } else if lower.ends_with(".ttf") {
        "font/ttf"
    } else {
        "application/octet-stream"
    }
}
