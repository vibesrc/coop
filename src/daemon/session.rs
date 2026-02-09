use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use bytes::Bytes;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::config::{self, Coopfile};
use crate::ipc::{
    PtyInfo, PtyRole, Response, ResponseData, SessionInfo, ERR_SESSION_EXISTS,
    ERR_SESSION_NOT_FOUND,
};
use crate::sandbox::namespace;

/// Max scrollback buffer size (256KB)
const SCROLLBACK_MAX: usize = 256 * 1024;

/// State of a single PTY
#[derive(Debug, Clone)]
pub struct PtyState {
    pub id: u32,
    pub role: PtyRole,
    pub command: String,
    pub pid: Option<u32>,
    /// PTY master file descriptor (owned by daemon). None if PTY not yet allocated.
    pub master_fd: Option<RawFd>,
    /// Broadcast channel for fan-out of PTY output to all attached clients.
    pub output_tx: Option<broadcast::Sender<Bytes>>,
    /// Shared scrollback buffer for replay on re-attach.
    pub scrollback: Option<Arc<Mutex<Vec<u8>>>>,
}

/// State of a running session
#[derive(Debug)]
pub struct Session {
    pub name: String,
    pub workspace: String,
    pub namespace_pid: u32,
    pub created: u64,
    pub ptys: Vec<PtyState>,
    pub local_clients: u32,
    pub web_clients: u32,
}

impl Session {
    pub fn to_info(&self) -> SessionInfo {
        SessionInfo {
            name: self.name.clone(),
            workspace: self.workspace.clone(),
            pid: self.namespace_pid,
            created: self.created,
            ptys: self
                .ptys
                .iter()
                .map(|p| PtyInfo {
                    id: p.id,
                    role: p.role.clone(),
                    command: p.command.clone(),
                })
                .collect(),
            web_clients: self.web_clients,
            local_clients: self.local_clients,
        }
    }
}

/// Manages all active sessions.
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Session>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Rediscover orphaned sessions from a previous daemon by scanning /proc.
    pub async fn rediscover_sessions(&self) {
        let discovered = namespace::discover_sessions();
        let mut sessions = self.sessions.write().await;

        for ds in discovered {
            if sessions.contains_key(&ds.name) {
                continue;
            }

            tracing::info!(
                session = %ds.name,
                workspace = %ds.workspace,
                pid = ds.pid,
                "Rediscovered orphaned session"
            );

            sessions.insert(
                ds.name.clone(),
                Session {
                    name: ds.name,
                    workspace: ds.workspace,
                    namespace_pid: ds.pid,
                    created: ds.created,
                    ptys: vec![],
                    local_clients: 0,
                    web_clients: 0,
                },
            );
        }
    }

    pub async fn create_session(
        &self,
        name: Option<String>,
        workspace: String,
        _coopfile: Option<String>,
        _detach: bool,
    ) -> Result<Response> {
        let name = name.unwrap_or_else(|| {
            std::path::Path::new(&workspace)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

        // Check if session already exists (by name or by workspace path)
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&name) {
                return Ok(Response::err_with(
                    ERR_SESSION_EXISTS,
                    format!("Session '{}' already exists", name),
                    ResponseData {
                        session: Some(name),
                        ..Default::default()
                    },
                ));
            }
            // Also check by workspace path
            if let Some(existing) = sessions.values().find(|s| s.workspace == workspace) {
                return Ok(Response::err_with(
                    ERR_SESSION_EXISTS,
                    format!("Session '{}' already exists for this workspace", existing.name),
                    ResponseData {
                        session: Some(existing.name.clone()),
                        ..Default::default()
                    },
                ));
            }
        }

        // Parse and merge Coopfile from the workspace
        let workspace_path = PathBuf::from(&workspace);
        let mut config = Coopfile::resolve(&workspace_path, None).unwrap_or_default();
        config.expand_env();

        // Verify base rootfs exists
        let base_path = config::rootfs_base_path()?;
        if !base_path.exists() {
            return Ok(Response::err(
                "ROOTFS_NOT_FOUND",
                "Rootfs not found. Run `coop init` first.",
            ));
        }

        // Create the namespace
        let ns_result = match namespace::create_session(&name, &config, &workspace_path) {
            Ok(ns) => ns,
            Err(e) => {
                return Ok(Response::err(
                    "NAMESPACE_ERROR",
                    format!("Failed to create namespace: {}", e),
                ));
            }
        };

        let agent_cmd = config
            .sandbox
            .command
            .as_deref()
            .unwrap_or("claude")
            .to_string();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let (output_tx, _) = broadcast::channel(256);
        let scrollback = Arc::new(Mutex::new(Vec::new()));

        // Spawn persistent PTY reader that runs for the lifetime of the session.
        // This ensures output is captured even when no client is attached.
        let master_fd = ns_result.pty_master_fd;
        {
            let output_tx_clone = output_tx.clone();
            let scrollback_clone = scrollback.clone();

            // Set non-blocking so AsyncFd works
            unsafe {
                let flags = nix::libc::fcntl(master_fd, nix::libc::F_GETFL);
                nix::libc::fcntl(master_fd, nix::libc::F_SETFL, flags | nix::libc::O_NONBLOCK);
            }

            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let async_fd = match tokio::io::unix::AsyncFd::new(master_fd) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to create AsyncFd for PTY master");
                        return;
                    }
                };

                loop {
                    let mut guard = match async_fd.readable().await {
                        Ok(g) => g,
                        Err(_) => break,
                    };

                    match guard.try_io(|inner| {
                        let fd = inner.as_raw_fd();
                        let n = unsafe {
                            nix::libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len())
                        };
                        if n < 0 {
                            Err(std::io::Error::last_os_error())
                        } else if n == 0 {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "PTY EOF",
                            ))
                        } else {
                            Ok(n as usize)
                        }
                    }) {
                        Ok(Ok(n)) => {
                            let data = Bytes::copy_from_slice(&buf[..n]);

                            // Append to scrollback buffer
                            {
                                let mut sb = scrollback_clone.lock().await;
                                sb.extend_from_slice(&buf[..n]);
                                // Trim to max size (keep the tail)
                                if sb.len() > SCROLLBACK_MAX {
                                    let excess = sb.len() - SCROLLBACK_MAX;
                                    sb.drain(..excess);
                                }
                            }

                            // Broadcast to any connected clients (ignore if none)
                            let _ = output_tx_clone.send(data);
                        }
                        Ok(Err(_)) => break, // EOF or error
                        Err(_would_block) => continue,
                    }
                }
            });
        }

        let session = Session {
            name: name.clone(),
            workspace: workspace.clone(),
            namespace_pid: ns_result.child_pid,
            created: now,
            ptys: vec![PtyState {
                id: 0,
                role: PtyRole::Agent,
                command: agent_cmd,
                pid: Some(ns_result.child_pid),
                master_fd: Some(master_fd),
                output_tx: Some(output_tx),
                scrollback: Some(scrollback),
            }],
            local_clients: 0,
            web_clients: 0,
        };

        tracing::info!(
            session = %name,
            workspace = %workspace,
            pid = ns_result.child_pid,
            "Created session"
        );

        let mut sessions = self.sessions.write().await;
        sessions.insert(name.clone(), session);

        Ok(Response::ok_with(ResponseData {
            session: Some(name),
            pid: Some(ns_result.child_pid),
            ..Default::default()
        }))
    }

    pub async fn attach(
        &self,
        session: &str,
        pty: u32,
        _cols: u16,
        _rows: u16,
    ) -> Result<Response> {
        let sessions = self.sessions.read().await;
        let session = self.resolve_session(&sessions, session)?;

        if pty as usize >= session.ptys.len() {
            return Ok(Response::err(
                "PTY_NOT_FOUND",
                format!("PTY {} not found in session '{}'", pty, session.name),
            ));
        }

        // TODO: upgrade to stream mode
        Ok(Response::ok())
    }

    pub async fn spawn_shell(
        &self,
        session_name: &str,
        command: Option<String>,
        _cols: u16,
        _rows: u16,
    ) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_name).ok_or_else(|| {
            anyhow::anyhow!("Session '{}' not found", session_name)
        })?;

        let cmd = command.unwrap_or_else(|| "/bin/sh".to_string());
        let pty_id = session.ptys.len() as u32;

        let (output_tx, _) = broadcast::channel(256);

        // TODO: actually nsenter and forkpty
        session.ptys.push(PtyState {
            id: pty_id,
            role: PtyRole::Shell,
            command: cmd,
            pid: None,
            master_fd: None,
            output_tx: Some(output_tx),
            scrollback: None,
        });

        Ok(Response::ok_with(ResponseData {
            pty: Some(pty_id),
            ..Default::default()
        }))
    }

    pub async fn list_sessions(&self) -> Result<Response> {
        let sessions = self.sessions.read().await;
        let infos: Vec<SessionInfo> = sessions.values().map(|s| s.to_info()).collect();

        Ok(Response::ok_with(ResponseData {
            sessions: Some(infos),
            ..Default::default()
        }))
    }

    pub async fn kill_session(&self, session_name: &str, force: bool) -> Result<Response> {
        let mut sessions = self.sessions.write().await;

        // Resolve session name (could be workspace path)
        let name = if session_name.contains('/') {
            sessions
                .values()
                .find(|s| s.workspace == session_name)
                .map(|s| s.name.clone())
        } else {
            Some(session_name.to_string())
        };

        let name = match name {
            Some(n) => n,
            None => {
                return Ok(Response::err(
                    ERR_SESSION_NOT_FOUND,
                    format!("Session '{}' not found", session_name),
                ))
            }
        };

        if let Some(session) = sessions.remove(&name) {
            // Kill the namespace init process
            if session.namespace_pid > 0 {
                if let Err(e) = namespace::kill_session(session.namespace_pid, force) {
                    tracing::warn!(
                        session = %name,
                        pid = session.namespace_pid,
                        error = %e,
                        "Failed to kill namespace process"
                    );
                }

                // If not force, wait briefly then force kill
                if !force {
                    let pid = session.namespace_pid;
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        let _ = namespace::kill_session(pid, true);
                    });
                }
            }

            // Clean up session directory (preserve persist/)
            if let Ok(session_dir) = config::session_dir(&name) {
                let _ = std::fs::remove_dir_all(session_dir.join("upper"));
                let _ = std::fs::remove_dir_all(session_dir.join("work"));
                let _ = std::fs::remove_dir_all(session_dir.join("merged"));
            }

            tracing::info!(session = %name, "Killed session");
            Ok(Response::ok())
        } else {
            Ok(Response::err(
                ERR_SESSION_NOT_FOUND,
                format!("Session '{}' not found", name),
            ))
        }
    }

    pub async fn kill_all(&self, force: bool) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let count = sessions.len();

        for (name, session) in sessions.drain() {
            if session.namespace_pid > 0 {
                if let Err(e) = namespace::kill_session(session.namespace_pid, force) {
                    tracing::warn!(
                        session = %name,
                        error = %e,
                        "Failed to kill namespace process"
                    );
                }
            }

            // Clean up session directory
            if let Ok(session_dir) = config::session_dir(&name) {
                let _ = std::fs::remove_dir_all(session_dir.join("upper"));
                let _ = std::fs::remove_dir_all(session_dir.join("work"));
                let _ = std::fs::remove_dir_all(session_dir.join("merged"));
            }
        }

        tracing::info!(count = count, "Killed all sessions");
        Ok(Response::ok())
    }

    /// Get the broadcast sender and master fd for a PTY in a session.
    /// Used by stream mode to bridge client connections to the PTY.
    pub async fn get_pty_handle(
        &self,
        session_name: &str,
        pty_id: u32,
    ) -> Result<(Option<RawFd>, broadcast::Sender<Bytes>, Option<Arc<Mutex<Vec<u8>>>>)> {
        let sessions = self.sessions.read().await;
        let session = self.resolve_session(&sessions, session_name)?;

        let pty = session
            .ptys
            .iter()
            .find(|p| p.id == pty_id)
            .ok_or_else(|| anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name))?;

        let output_tx = pty
            .output_tx
            .clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no output channel", pty_id))?;

        Ok((pty.master_fd, output_tx, pty.scrollback.clone()))
    }

    /// Increment the local client count for a session
    pub async fn add_local_client(&self, session_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_name) {
            s.local_clients += 1;
        }
    }

    /// Decrement the local client count for a session
    pub async fn remove_local_client(&self, session_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_name) {
            s.local_clients = s.local_clients.saturating_sub(1);
        }
    }

    /// Increment the web client count for a session
    pub async fn add_web_client(&self, session_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_name) {
            s.web_clients += 1;
        }
    }

    /// Decrement the web client count for a session
    pub async fn remove_web_client(&self, session_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_name) {
            s.web_clients = s.web_clients.saturating_sub(1);
        }
    }

    fn resolve_session<'a>(
        &self,
        sessions: &'a HashMap<String, Session>,
        name_or_path: &str,
    ) -> Result<&'a Session> {
        // Direct name lookup
        if let Some(s) = sessions.get(name_or_path) {
            return Ok(s);
        }

        // Workspace path lookup
        if name_or_path.contains('/') {
            if let Some(s) = sessions.values().find(|s| s.workspace == name_or_path) {
                return Ok(s);
            }
        }

        bail!("Session '{}' not found", name_or_path);
    }
}
