use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

use super::server::WebState;
use crate::pty::InputFilter;

pub fn ws_routes() -> Router<Arc<WebState>> {
    Router::new().route("/ws", get(ws_handler))
}

#[derive(Deserialize)]
pub struct WsQuery {
    pub session: String,
    pub pty: Option<u32>,
    pub token: Option<String>,
}

async fn ws_handler(
    State(state): State<Arc<WebState>>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Verify token
    if query.token.as_deref() != Some(state.token.as_str()) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state, query.session, query.pty.unwrap_or(0)))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: Arc<WebState>, session: String, pty: u32) {
    tracing::info!(session = %session, pty = pty, "WebSocket connected");

    if let Err(e) = handle_socket_inner(socket, &state, &session, pty).await {
        tracing::error!(session = %session, pty = pty, error = %e, "WebSocket handler error");
    }

    tracing::info!(session = %session, pty = pty, "WebSocket disconnected");
}

async fn handle_socket_inner(
    socket: WebSocket,
    state: &WebState,
    session: &str,
    pty: u32,
) -> anyhow::Result<()> {
    // Look up session and PTY handles
    let (master_fd, output_tx, scrollback) =
        state.session_manager.get_pty_handle(session, pty).await?;

    // Track web client
    state.session_manager.add_web_client(session).await;
    let _guard = WebClientGuard {
        session_manager: state.session_manager.clone(),
        session: session.to_string(),
    };

    // Subscribe to PTY output
    let mut output_rx = output_tx.subscribe();

    // Create input filter for agent PTYs (pty 0) on web connections
    let mut input_filter = if pty == 0 {
        Some(InputFilter::new(500, &[]))
    } else {
        None
    };

    let (mut ws_sink, mut ws_stream) = socket.split();

    // Replay scrollback buffer so the client sees previous terminal state
    if let Some(sb) = &scrollback {
        let data = sb.lock().await;
        if !data.is_empty() {
            let _ = ws_sink.send(Message::Binary(data.clone().into())).await;
        }
    }

    loop {
        tokio::select! {
            // WebSocket -> PTY (input)
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        // Apply input filter for agent PTYs
                        let (to_forward, warning) = if let Some(ref mut filter) = input_filter {
                            filter.filter(&data)
                        } else {
                            (data.to_vec(), None)
                        };

                        // Send warning back to client if blocked
                        if let Some(warning) = warning {
                            let _ = ws_sink.send(Message::Binary(warning.to_vec().into())).await;
                        }

                        // Write filtered input to PTY master
                        if !to_forward.is_empty() {
                            let fd = master_fd.load(std::sync::atomic::Ordering::SeqCst);
                            if fd >= 0 {
                                unsafe {
                                    nix::libc::write(
                                        fd,
                                        to_forward.as_ptr() as *const _,
                                        to_forward.len(),
                                    );
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        // JSON control message (e.g., resize)
                        if let Ok(control) = serde_json::from_str::<serde_json::Value>(&text) {
                            if control.get("type").and_then(|t| t.as_str()) == Some("resize") {
                                let cols = control.get("cols").and_then(|c| c.as_u64()).unwrap_or(120) as u16;
                                let rows = control.get("rows").and_then(|r| r.as_u64()).unwrap_or(40) as u16;
                                let fd = master_fd.load(std::sync::atomic::Ordering::SeqCst);
                                if fd >= 0 {
                                    let ws = nix::libc::winsize {
                                        ws_row: rows,
                                        ws_col: cols,
                                        ws_xpixel: 0,
                                        ws_ypixel: 0,
                                    };
                                    unsafe {
                                        nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &ws);
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "WebSocket receive error");
                        break;
                    }
                    _ => {}
                }
            }

            // PTY output -> WebSocket
            data = output_rx.recv() => {
                match data {
                    Ok(bytes) => {
                        if ws_sink.send(Message::Binary(bytes.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(dropped = n, "WebSocket client lagging");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // PTY exited
                        let msg =
                            "\r\n\x1b[2m[process exited]\x1b[0m\r\n".to_string();
                        let _ = ws_sink.send(Message::Binary(msg.into_bytes().into())).await;
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// RAII guard to decrement web client count on drop
struct WebClientGuard {
    session_manager: Arc<crate::daemon::session::SessionManager>,
    session: String,
}

impl Drop for WebClientGuard {
    fn drop(&mut self) {
        let sm = self.session_manager.clone();
        let session = self.session.clone();
        tokio::spawn(async move {
            sm.remove_web_client(&session).await;
        });
    }
}
