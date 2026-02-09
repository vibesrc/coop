use std::sync::Arc;

use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;

use super::server::WebState;

#[derive(Embed)]
#[folder = "web/dist/"]
struct Asset;

pub fn asset_routes() -> Router<Arc<WebState>> {
    Router::new()
        .route("/", get(index_handler))
        .route("/assets/{*path}", get(static_handler))
        .route("/connect", get(index_handler))
}

async fn index_handler() -> impl IntoResponse {
    match Asset::get("index.html") {
        Some(content) => {
            Html(String::from_utf8_lossy(content.data.as_ref()).to_string()).into_response()
        }
        None => Html(
            "<html><body><h1>Coop Web UI</h1><p>Web assets not built. Run <code>npm run build</code> in the <code>web/</code> directory.</p></body></html>"
                .to_string(),
        )
        .into_response(),
    }
}

async fn static_handler(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    let path = format!("assets/{}", path);
    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            (
                [(axum::http::header::CONTENT_TYPE, mime)],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}
