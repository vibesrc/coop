use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio_util::codec::{Framed, FramedParts};

use crate::config;
use crate::ipc::{
    Command, DaemonEvent, MessageCodec, Response, ResponseData, StreamCodec, StreamFrame,
    VersionHandshake, VersionResponse, FRAME_CONTROL, FRAME_PTY_DATA, PROTOCOL_VERSION,
};

use super::session::SessionManager;

/// The daemon server that listens on the unix socket and manages sessions.
pub struct DaemonServer {
    session_manager: Arc<SessionManager>,
    shutdown_tx: broadcast::Sender<()>,
}

impl DaemonServer {
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            session_manager: Arc::new(SessionManager::new()),
            shutdown_tx,
        }
    }

    /// Run the daemon server event loop.
    pub async fn run(&self) -> Result<()> {
        let sock_path = config::socket_path()?;

        // Safety: ensure we don't follow symlinks
        if sock_path.is_symlink() {
            anyhow::bail!(
                "Socket path is a symlink, refusing to continue: {}",
                sock_path.display()
            );
        }

        // Clean up stale socket
        if sock_path.exists() {
            std::fs::remove_file(&sock_path)?;
        }

        // Set restrictive umask before binding
        let old_umask = unsafe { nix::libc::umask(0o177) };

        let listener = UnixListener::bind(&sock_path)
            .with_context(|| format!("Failed to bind {}", sock_path.display()))?;

        // Restore umask
        unsafe {
            nix::libc::umask(old_umask);
        }

        // Write PID file
        if let Ok(pid_path) = config::pid_file_path() {
            let _ = std::fs::write(&pid_path, std::process::id().to_string());
        }

        tracing::info!(socket = %sock_path.display(), "Daemon listening");

        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let idle_timeout = Duration::from_secs(30);
        let mut idle_since = tokio::time::Instant::now();

        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _)) => {
                            idle_since = tokio::time::Instant::now();
                            let session_manager = self.session_manager.clone();
                            let shutdown_tx = self.shutdown_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, session_manager, shutdown_tx).await {
                                    tracing::error!(error = %e, "Client handler error");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Accept error");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Received shutdown signal");
                    break;
                }
                _ = tokio::time::sleep(idle_timeout) => {
                    if self.session_manager.session_count().await == 0
                        && idle_since.elapsed() >= idle_timeout
                    {
                        tracing::info!("Idle timeout reached, shutting down");
                        break;
                    }
                }
            }
        }

        // Cleanup
        let _ = std::fs::remove_file(&sock_path);
        if let Ok(pid_path) = config::pid_file_path() {
            let _ = std::fs::remove_file(&pid_path);
        }

        tracing::info!("Daemon exited cleanly");
        Ok(())
    }
}

/// Info about a stream mode session that the client should upgrade to
struct StreamTarget {
    session: String,
    pty: u32,
    cols: u16,
    rows: u16,
    /// When true, client is read-only (no stdin writes, no resize)
    readonly: bool,
}

async fn handle_client(
    stream: tokio::net::UnixStream,
    session_manager: Arc<SessionManager>,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<()> {
    // Verify peer credentials
    let cred = stream.peer_cred()?;
    let my_uid = nix::unistd::getuid();
    if cred.uid() != my_uid.as_raw() {
        tracing::warn!(peer_uid = ?cred.uid(), "Rejecting connection from different UID");
        return Ok(());
    }

    let mut framed = Framed::new(stream, MessageCodec);

    // Protocol version handshake
    let msg = framed
        .next()
        .await
        .context("Connection closed before handshake")?
        .context("Read error during handshake")?;

    let handshake: VersionHandshake = serde_json::from_slice(&msg)?;

    if handshake.version != PROTOCOL_VERSION {
        let resp = VersionResponse {
            version: PROTOCOL_VERSION,
            ok: false,
            error: Some("VERSION_MISMATCH".to_string()),
            message: Some(format!(
                "Unsupported protocol version {}",
                handshake.version
            )),
        };
        let json = serde_json::to_vec(&resp)?;
        framed.send(Bytes::from(json)).await?;
        return Ok(());
    }

    let resp = VersionResponse {
        version: PROTOCOL_VERSION,
        ok: true,
        error: None,
        message: None,
    };
    let json = serde_json::to_vec(&resp)?;
    framed.send(Bytes::from(json)).await?;

    // Command loop
    let mut stream_target: Option<StreamTarget> = None;

    while let Some(msg) = framed.next().await {
        let msg = msg.context("Read error")?;
        let cmd: Command = match serde_json::from_slice(&msg) {
            Ok(cmd) => cmd,
            Err(e) => {
                let resp = Response::err("INVALID_COMMAND", format!("Parse error: {}", e));
                let json = serde_json::to_vec(&resp)?;
                framed.send(Bytes::from(json)).await?;
                continue;
            }
        };

        let is_detach = matches!(cmd, Command::Detach);

        // Track attach/shell targets for stream mode upgrade
        let pending_stream = match &cmd {
            Command::Attach {
                session,
                pty,
                cols,
                rows,
            } => Some(StreamTarget {
                session: session.clone(),
                pty: *pty,
                cols: *cols,
                rows: *rows,
                readonly: false,
            }),
            Command::Shell {
                session,
                cols,
                rows,
                ..
            } => Some(StreamTarget {
                session: session.clone(),
                pty: 0, // will be filled from response
                cols: *cols,
                rows: *rows,
                readonly: false,
            }),
            Command::Logs {
                session,
                pty,
                follow,
                ..
            } if *follow => Some(StreamTarget {
                session: session.clone(),
                pty: *pty,
                cols: 0,
                rows: 0,
                readonly: true,
            }),
            _ => None,
        };

        let is_shell = matches!(cmd, Command::Shell { .. });

        let resp = match cmd {
            Command::Create {
                name,
                workspace,
                coopfile,
                detach,
            } => {
                session_manager
                    .create_session(name, workspace, coopfile, detach)
                    .await
            }
            Command::Attach {
                session,
                pty,
                cols,
                rows,
            } => session_manager.attach(&session, pty, cols, rows).await,
            Command::Shell {
                session,
                command,
                force_new,
                cols,
                rows,
            } => {
                session_manager
                    .spawn_shell(&session, command, force_new, cols, rows)
                    .await
            }
            Command::Ls => session_manager.list_sessions().await,
            Command::Kill {
                session,
                all,
                force,
            } => {
                if all {
                    session_manager.kill_all(force).await
                } else {
                    session_manager.kill_session(&session, force).await
                }
            }
            Command::Serve { port, host, token } => {
                let token = token.unwrap_or_else(generate_token);
                let sm = session_manager.clone();
                let token_clone = token.clone();
                let host_clone = host.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::web::server::start_web_server(
                        &host_clone,
                        port,
                        sm,
                        token_clone,
                    )
                    .await
                    {
                        tracing::error!(error = %e, "Web server error");
                    }
                });
                Ok(Response::ok_with(ResponseData {
                    port: Some(port),
                    host: Some(host),
                    token: Some(token),
                    ..Default::default()
                }))
            }
            Command::SessionLs { session } => {
                session_manager.session_ls(&session).await
            }
            Command::SessionKill { session, pty } => {
                session_manager.kill_pty(&session, pty).await
            }
            Command::Logs { session, pty, tail_lines, .. } => {
                session_manager.get_logs(&session, pty, tail_lines).await
            }
            Command::Restart { session, pty } => {
                session_manager.restart_pty(&session, pty).await
            }
            Command::Shutdown => {
                let _ = shutdown_tx.send(());
                Ok(Response::ok())
            }
            Command::Resize { .. } => Ok(Response::err(
                "INVALID_COMMAND",
                "Resize is only valid in stream mode",
            )),
            Command::Detach => Ok(Response::ok()),
            Command::Tunnel { .. } => Ok(Response::err(
                "INVALID_COMMAND",
                "Tunnel not yet implemented",
            )),
        };

        let resp = resp.unwrap_or_else(|e| Response::err("INTERNAL_ERROR", e.to_string()));

        // If attach/shell succeeded, prepare for stream mode upgrade
        if resp.ok {
            if let Some(mut target) = pending_stream {
                // For shell commands, the assigned PTY id is in the response
                if is_shell {
                    if let Some(pty_id) = resp.data.pty {
                        target.pty = pty_id;
                    }
                }
                stream_target = Some(target);
            }
        }

        let json = serde_json::to_vec(&resp)?;
        framed.send(Bytes::from(json)).await?;

        if is_detach {
            break;
        }

        // If we have a stream target, upgrade to stream mode
        if let Some(target) = stream_target.take() {
            // Use into_parts to preserve any buffered bytes from client
            let parts = framed.into_parts();
            handle_stream_mode(parts, &target, session_manager).await?;
            return Ok(());
        }
    }

    Ok(())
}

/// Handle a client in stream mode: bridge between the client's framed stream
/// and the PTY master fd, using the broadcast channel for fan-out.
async fn handle_stream_mode(
    msg_parts: FramedParts<tokio::net::UnixStream, MessageCodec>,
    target: &StreamTarget,
    session_manager: Arc<SessionManager>,
) -> Result<()> {
    session_manager.add_local_client(&target.session).await;

    let result =
        handle_stream_mode_inner(msg_parts, target, session_manager.clone()).await;

    session_manager.remove_local_client(&target.session).await;

    result
}

async fn handle_stream_mode_inner(
    msg_parts: FramedParts<tokio::net::UnixStream, MessageCodec>,
    target: &StreamTarget,
    session_manager: Arc<SessionManager>,
) -> Result<()> {
    // Carry over any buffered bytes from the MessageCodec framed reader
    // into the new StreamCodec framed reader, so we don't lose data that
    // was already read from the socket (e.g. scrollback replay).
    let mut new_parts = FramedParts::new(msg_parts.io, StreamCodec);
    new_parts.read_buf = msg_parts.read_buf;
    let stream_framed = Framed::from_parts(new_parts);
    let (mut sink, mut client_stream) = stream_framed.split();

    // Get the PTY broadcast channel, master fd, and scrollback buffer
    let (master_fd, output_tx, scrollback) = session_manager
        .get_pty_handle(&target.session, target.pty)
        .await?;

    // Subscribe BEFORE replaying scrollback so we don't miss anything
    let mut output_rx = output_tx.subscribe();
    // Drop our sender clone so the channel properly closes when the PTY exits
    drop(output_tx);

    // If we have a real master fd and not readonly, set initial window size
    if !target.readonly {
        let fd = master_fd.load(Ordering::SeqCst);
        if fd >= 0 {
            set_pty_size(fd, target.cols, target.rows);
        }
    }

    // Replay scrollback buffer so the client sees previous terminal state
    if let Some(sb) = &scrollback {
        let data = sb.lock().await;
        if !data.is_empty() {
            sink.send(StreamFrame::pty_data(Bytes::copy_from_slice(&data))).await?;
        }
    }

    // The persistent PTY reader (spawned in create_session) handles reading
    // from the master fd and broadcasting. We just bridge broadcast -> client.

    // Main stream mode loop
    loop {
        tokio::select! {
            // Client -> daemon: PTY data or control frames
            frame = client_stream.next() => {
                match frame {
                    Some(Ok(frame)) => {
                        match frame.frame_type {
                            FRAME_PTY_DATA => {
                                // Write input to PTY master (skip if readonly).
                                // Read fd atomically so we always use the current
                                // fd even after a PTY restart.
                                if !target.readonly {
                                    let fd = master_fd.load(Ordering::SeqCst);
                                    if fd >= 0 {
                                        let data = &frame.payload;
                                        unsafe {
                                            nix::libc::write(
                                                fd,
                                                data.as_ptr() as *const _,
                                                data.len(),
                                            );
                                        }
                                    }
                                }
                            }
                            FRAME_CONTROL => {
                                match serde_json::from_slice::<Command>(&frame.payload) {
                                    Ok(Command::Resize { cols, rows }) => {
                                        if !target.readonly {
                                            let fd = master_fd.load(Ordering::SeqCst);
                                            if fd >= 0 {
                                                set_pty_size(fd, cols, rows);
                                            }
                                        }
                                    }
                                    Ok(Command::Detach) => {
                                        // Send detached event and close
                                        let event = serde_json::to_vec(&DaemonEvent::Detached)?;
                                        let _ = sink.send(StreamFrame::control(Bytes::from(event))).await;
                                        break;
                                    }
                                    _ => {
                                        // Unknown control command in stream mode, ignore
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "Stream read error from client");
                        break;
                    }
                    None => {
                        // Client disconnected
                        break;
                    }
                }
            }

            // PTY output -> client
            data = output_rx.recv() => {
                match data {
                    Ok(bytes) => {
                        if sink.send(StreamFrame::pty_data(bytes)).await.is_err() {
                            // Client write failed, disconnect
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(dropped = n, "Client lagging, dropped frames");
                        // Continue — client will see a gap in output
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // PTY output channel closed (PTY exited)
                        let event = serde_json::to_vec(&DaemonEvent::PtyExited { code: 0 })?;
                        let _ = sink.send(StreamFrame::control(Bytes::from(event))).await;
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Set the window size on a PTY master fd
fn set_pty_size(fd: std::os::unix::io::RawFd, cols: u16, rows: u16) {
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

fn generate_token() -> String {
    use base64::Engine;
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
