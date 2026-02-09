use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio_util::codec::Framed;

use crate::ipc::{
    Command, DaemonEvent, MessageCodec, Response, StreamCodec, StreamFrame, VersionHandshake,
    VersionResponse, FRAME_CONTROL, FRAME_PTY_DATA, PROTOCOL_VERSION,
};

/// Client for communicating with the coop daemon over the unix socket.
pub struct DaemonClient {
    framed: Framed<UnixStream, MessageCodec>,
}

impl DaemonClient {
    /// Connect to the daemon, auto-spawning if necessary.
    pub async fn connect() -> Result<Self> {
        let sock_path = crate::config::socket_path()?;

        // Try connecting first
        let stream = match UnixStream::connect(&sock_path).await {
            Ok(s) => s,
            Err(_) => {
                // Daemon not running, spawn it
                crate::daemon::spawn::ensure_daemon().await?;
                UnixStream::connect(&sock_path)
                    .await
                    .context("Failed to connect to daemon after spawn")?
            }
        };

        let mut framed = Framed::new(stream, MessageCodec);

        // Protocol version handshake
        let handshake = serde_json::to_vec(&VersionHandshake {
            version: PROTOCOL_VERSION,
        })?;
        framed.send(Bytes::from(handshake)).await?;

        let resp = framed
            .next()
            .await
            .context("Daemon closed connection during handshake")?
            .context("Read error during handshake")?;

        let version_resp: VersionResponse = serde_json::from_slice(&resp)?;
        if !version_resp.ok {
            bail!(
                "Protocol version mismatch: {}",
                version_resp.message.unwrap_or_default()
            );
        }

        Ok(Self { framed })
    }

    /// Send a command and receive the response.
    async fn send_command(&mut self, cmd: &Command) -> Result<Response> {
        let json = serde_json::to_vec(cmd)?;
        self.framed.send(Bytes::from(json)).await?;

        let resp = self
            .framed
            .next()
            .await
            .context("Daemon closed connection")?
            .context("Read error")?;

        let response: Response = serde_json::from_slice(&resp)?;
        Ok(response)
    }

    pub async fn create_session(
        mut self,
        name: Option<&str>,
        workspace: &str,
        detach: bool,
    ) -> Result<()> {
        let cmd = Command::Create {
            name: name.map(|s| s.to_string()),
            workspace: workspace.to_string(),
            coopfile: None,
            detach,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!(
                "Failed to create session: {}",
                resp.message.unwrap_or_default()
            );
        }
        if let Some(session) = &resp.data.session {
            println!("Session '{}' created", session);
        }
        Ok(())
    }

    pub async fn attach_or_create(mut self, name: Option<&str>, workspace: &str) -> Result<()> {
        // Try create with attach
        let cmd = Command::Create {
            name: name.map(|s| s.to_string()),
            workspace: workspace.to_string(),
            coopfile: None,
            detach: false,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            // If session exists, try attach (use session name from response if available)
            if resp.error.as_deref() == Some("SESSION_EXISTS") {
                let session_name = resp.data.session.unwrap_or_else(|| {
                    name.map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            std::path::Path::new(workspace)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        })
                });
                return self.attach(&session_name, 0).await;
            }
            bail!(
                "Failed to create session: {}",
                resp.message.unwrap_or_default()
            );
        }

        // Session created, now attach to it (this triggers stream mode on the server)
        if let Some(session) = resp.data.session {
            return self.attach(&session, 0).await;
        }
        Ok(())
    }

    pub async fn attach(mut self, session: &str, pty: u32) -> Result<()> {
        let (cols, rows) = terminal_size();
        let cmd = Command::Attach {
            session: session.to_string(),
            pty,
            cols,
            rows,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!("Failed to attach: {}", resp.message.unwrap_or_default());
        }

        self.enter_stream_mode(session, pty).await
    }

    pub async fn shell(mut self, session: &str, command: Option<&str>) -> Result<()> {
        let (cols, rows) = terminal_size();
        let cmd = Command::Shell {
            session: session.to_string(),
            command: command.map(|s| s.to_string()),
            cols,
            rows,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!("Failed to spawn shell: {}", resp.message.unwrap_or_default());
        }

        if let Some(pty_id) = resp.data.pty {
            self.enter_stream_mode(session, pty_id).await?;
        }
        Ok(())
    }

    pub async fn list_sessions(mut self, json: bool) -> Result<()> {
        let resp = self.send_command(&Command::Ls).await?;
        if !resp.ok {
            bail!("Failed to list sessions: {}", resp.message.unwrap_or_default());
        }

        if let Some(sessions) = &resp.data.sessions {
            if json {
                println!("{}", serde_json::to_string_pretty(sessions)?);
            } else if sessions.is_empty() {
                println!("No running sessions.");
            } else {
                println!(
                    "{:<12} {:<30} {:<10} {:<6} {:<15} {}",
                    "SESSION", "WORKSPACE", "STATE", "PTYS", "CLIENTS", "AGE"
                );
                for s in sessions {
                    println!(
                        "{:<12} {:<30} {:<10} {:<6} {:<15} {}",
                        s.name,
                        truncate(&s.workspace, 28),
                        "running",
                        s.ptys.len(),
                        format!("{} local, {} web", s.local_clients, s.web_clients),
                        format_age(s.created),
                    );
                }
            }
        }
        Ok(())
    }

    pub async fn kill(mut self, session: &str, force: bool) -> Result<()> {
        let cmd = Command::Kill {
            session: session.to_string(),
            all: false,
            force,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!("Failed to kill session: {}", resp.message.unwrap_or_default());
        }
        println!("Session '{}' killed", session);
        Ok(())
    }

    pub async fn kill_all(mut self, force: bool) -> Result<()> {
        let cmd = Command::Kill {
            session: String::new(),
            all: true,
            force,
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!("Failed to kill sessions: {}", resp.message.unwrap_or_default());
        }
        println!("All sessions killed");
        Ok(())
    }

    pub async fn serve(mut self, port: u16, host: &str, token: Option<&str>) -> Result<()> {
        let cmd = Command::Serve {
            port,
            host: host.to_string(),
            token: token.map(|s| s.to_string()),
        };
        let resp = self.send_command(&cmd).await?;
        if !resp.ok {
            bail!("Failed to start serve: {}", resp.message.unwrap_or_default());
        }

        let token = resp.data.token.unwrap_or_default();
        let host = resp.data.host.unwrap_or_else(|| host.to_string());
        let port = resp.data.port.unwrap_or(port);

        println!();
        println!("  \u{1f414} Coop web UI");
        println!();
        println!("  Local:   http://{}:{}?token={}", host, port, token);
        // TODO: detect network address for LAN URL
        println!();

        // Block until Ctrl+C
        tokio::signal::ctrl_c().await?;
        Ok(())
    }

    pub async fn tunnel(
        mut self,
        _stun: Option<&str>,
        _no_stun: bool,
        _no_qr: bool,
    ) -> Result<()> {
        // TODO: implement WebRTC tunnel
        bail!("Tunnel support not yet implemented");
    }

    pub async fn shutdown(mut self) -> Result<()> {
        let resp = self.send_command(&Command::Shutdown).await?;
        if !resp.ok {
            bail!("Failed to shutdown: {}", resp.message.unwrap_or_default());
        }
        println!("Daemon shutting down");
        Ok(())
    }

    pub async fn status(mut self) -> Result<()> {
        let resp = self.send_command(&Command::Ls).await?;
        if !resp.ok {
            bail!("Failed to get status: {}", resp.message.unwrap_or_default());
        }

        let session_count = resp.data.sessions.as_ref().map(|s| s.len()).unwrap_or(0);
        println!("Daemon:     running");
        println!("Sessions:   {} running", session_count);
        Ok(())
    }

    pub async fn stop_serve(mut self) -> Result<()> {
        // TODO: implement serve stop
        bail!("Serve stop not yet implemented");
    }

    /// Enter stream mode for an attached PTY session.
    ///
    /// This upgrades the connection from MessageCodec to StreamCodec and bridges
    /// the local terminal (stdin/stdout) to the remote PTY via tagged frames.
    async fn enter_stream_mode(self, _session: &str, _pty: u32) -> Result<()> {
        // Save original terminal settings so we can restore on exit
        let stdin_handle = std::io::stdin();
        let orig_termios = nix::sys::termios::tcgetattr(&stdin_handle)
            .context("Failed to get terminal attributes (not a terminal?)")?;

        // Set terminal to raw mode
        let mut raw = orig_termios.clone();
        nix::sys::termios::cfmakeraw(&mut raw);
        nix::sys::termios::tcsetattr(
            &stdin_handle,
            nix::sys::termios::SetArg::TCSANOW,
            &raw,
        )?;


        // Consume self to extract the UnixStream, carrying over any buffered
        // bytes (the server may have already sent StreamCodec frames like scrollback
        // replay that got read-ahead into the MessageCodec's buffer).
        let parts = self.framed.into_parts();
        let mut new_parts = tokio_util::codec::FramedParts::new(parts.io, StreamCodec);
        new_parts.read_buf = parts.read_buf;
        let stream_framed = Framed::from_parts(new_parts);
        let (mut sink, mut stream) = stream_framed.split();

        // Run the bidirectional bridge
        let result = run_stream_bridge(&mut sink, &mut stream).await;

        // Restore terminal settings regardless of how we exited
        let _ = nix::sys::termios::tcsetattr(
            &stdin_handle,
            nix::sys::termios::SetArg::TCSANOW,
            &orig_termios,
        );

        eprintln!("[detached]");

        result
    }
}

/// The escape character: Ctrl+] (0x1D)
const ESCAPE_CHAR: u8 = 0x1D;

/// Run the bidirectional stream bridge between local terminal and daemon PTY.
async fn run_stream_bridge(
    sink: &mut futures_util::stream::SplitSink<
        Framed<UnixStream, StreamCodec>,
        StreamFrame,
    >,
    stream: &mut futures_util::stream::SplitStream<Framed<UnixStream, StreamCodec>>,
) -> Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut stdin_buf = [0u8; 4096];

    // Set up SIGWINCH handler for terminal resize
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;

    loop {
        tokio::select! {
            // stdin -> daemon (PTY data frames)
            n = stdin.read(&mut stdin_buf) => {
                let n = n.context("stdin read error")?;
                if n == 0 {
                    // stdin closed, send detach
                    let detach = serde_json::to_vec(&Command::Detach)?;
                    sink.send(StreamFrame::control(Bytes::from(detach))).await?;
                    break;
                }

                // Check for escape character (Ctrl+])
                if let Some(pos) = stdin_buf[..n].iter().position(|&b| b == ESCAPE_CHAR) {
                    // Send any bytes before the escape char
                    if pos > 0 {
                        sink.send(StreamFrame::pty_data(
                            Bytes::copy_from_slice(&stdin_buf[..pos]),
                        )).await?;
                    }
                    // Send detach control
                    let detach = serde_json::to_vec(&Command::Detach)?;
                    sink.send(StreamFrame::control(Bytes::from(detach))).await?;
                    break;
                }

                sink.send(StreamFrame::pty_data(
                    Bytes::copy_from_slice(&stdin_buf[..n]),
                )).await?;
            }

            // daemon -> stdout (PTY data or control frames)
            frame = stream.next() => {
                match frame {
                    Some(Ok(frame)) => {
                        match frame.frame_type {
                            FRAME_PTY_DATA => {
                                stdout.write_all(&frame.payload).await?;
                                stdout.flush().await?;
                            }
                            FRAME_CONTROL => {
                                match serde_json::from_slice::<DaemonEvent>(&frame.payload) {
                                    Ok(DaemonEvent::PtyExited { code }) => {
                                        let msg = format!(
                                            "\r\n\x1b[2m[process exited (code {})]\x1b[0m\r\n",
                                            code
                                        );
                                        stdout.write_all(msg.as_bytes()).await?;
                                        stdout.flush().await?;
                                    }
                                    Ok(DaemonEvent::PtyRestarting { delay_ms }) => {
                                        let msg = format!(
                                            "\x1b[2m[restarting in {}ms...]\x1b[0m\r\n",
                                            delay_ms
                                        );
                                        stdout.write_all(msg.as_bytes()).await?;
                                        stdout.flush().await?;
                                        // Stay connected -- new PTY output will follow
                                    }
                                    Ok(DaemonEvent::Detached) => {
                                        break;
                                    }
                                    Err(_) => {
                                        // Unknown control frame, ignore
                                    }
                                }
                            }
                            _ => {
                                // Unknown frame type, ignore
                            }
                        }
                    }
                    Some(Err(e)) => {
                        return Err(e).context("Stream read error");
                    }
                    None => {
                        // Connection closed
                        break;
                    }
                }
            }

            // SIGWINCH -> send resize control frame
            _ = sigwinch.recv() => {
                let (cols, rows) = terminal_size();
                let resize = serde_json::to_vec(&Command::Resize { cols, rows })?;
                sink.send(StreamFrame::control(Bytes::from(resize))).await?;
            }
        }
    }

    Ok(())
}

fn terminal_size() -> (u16, u16) {
    let mut ws = nix::libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        nix::libc::ioctl(0, nix::libc::TIOCGWINSZ, &mut ws);
    }
    (ws.ws_col, ws.ws_row)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}

fn format_age(created: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(created);

    if elapsed < 60 {
        format!("{}s", elapsed)
    } else if elapsed < 3600 {
        format!("{}m", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60)
    } else {
        format!("{}d {}h", elapsed / 86400, (elapsed % 86400) / 3600)
    }
}
