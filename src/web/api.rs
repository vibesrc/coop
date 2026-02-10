use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::Deserialize;

use super::server::WebState;
use crate::ipc::SessionInfo;

/// API routes
pub fn api_routes() -> Router<Arc<WebState>> {
    Router::new()
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions/{name}", delete(kill_session))
        .route("/api/sessions/{name}/shell", post(spawn_shell))
}

#[derive(Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

/// Verify the auth token from query param or Authorization header
fn verify_token(state: &WebState, token: Option<&str>) -> bool {
    token.map_or(false, |t| t == state.token)
}

async fn list_sessions(
    State(state): State<Arc<WebState>>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Vec<SessionInfo>>, StatusCode> {
    if !verify_token(&state, query.token.as_deref()) {
        return Err(StatusCode::FORBIDDEN);
    }

    match state.session_manager.list_sessions().await {
        Ok(resp) => {
            if let Some(sessions) = resp.data.sessions {
                Ok(Json(sessions))
            } else {
                Ok(Json(vec![]))
            }
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub name: Option<String>,
    pub workspace: String,
}

async fn create_session(
    State(state): State<Arc<WebState>>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !verify_token(&state, query.token.as_deref()) {
        return Err(StatusCode::FORBIDDEN);
    }

    match state
        .session_manager
        .create_session(body.name, body.workspace, None, true)
        .await
    {
        Ok(resp) => Ok(Json(serde_json::to_value(resp).unwrap())),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn kill_session(
    State(state): State<Arc<WebState>>,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !verify_token(&state, query.token.as_deref()) {
        return Err(StatusCode::FORBIDDEN);
    }

    match state.session_manager.kill_session(&name, false).await {
        Ok(resp) => Ok(Json(serde_json::to_value(resp).unwrap())),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
pub struct SpawnShellRequest {
    pub command: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

async fn spawn_shell(
    State(state): State<Arc<WebState>>,
    Query(query): Query<TokenQuery>,
    Path(name): Path<String>,
    Json(body): Json<SpawnShellRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !verify_token(&state, query.token.as_deref()) {
        return Err(StatusCode::FORBIDDEN);
    }

    match state
        .session_manager
        .spawn_shell(&name, body.command, false, body.cols.unwrap_or(120), body.rows.unwrap_or(40))
        .await
    {
        Ok(resp) => Ok(Json(serde_json::to_value(resp).unwrap())),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
