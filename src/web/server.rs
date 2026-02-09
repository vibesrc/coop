use std::sync::Arc;

use anyhow::Result;
use axum::Router;

use crate::daemon::session::SessionManager;

/// State shared across all web request handlers
pub struct WebState {
    pub session_manager: Arc<SessionManager>,
    pub token: String,
}

/// Create the axum router for the web UI
pub fn create_router(state: Arc<WebState>) -> Router {
    Router::new()
        .merge(super::api::api_routes())
        .merge(super::websocket::ws_routes())
        .merge(super::assets::asset_routes())
        .with_state(state)
}

/// Start the web server
pub async fn start_web_server(
    host: &str,
    port: u16,
    session_manager: Arc<SessionManager>,
    token: String,
) -> Result<()> {
    let state = Arc::new(WebState {
        session_manager,
        token,
    });

    let app = create_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!(addr = %addr, "Web server listening");

    axum::serve(listener, app).await?;

    Ok(())
}
