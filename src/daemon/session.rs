use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use bytes::Bytes;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};

use crate::config::{self, Coopfile, NetworkMode};
use base64::Engine;
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

impl PtyState {
    fn new(id: u32, role: PtyRole, command: String, pid: u32, master_fd: RawFd) -> (Self, oneshot::Receiver<()>) {
        let (output_tx, _) = broadcast::channel(256);
        let scrollback = Arc::new(Mutex::new(Vec::new()));
        let exit_rx = spawn_pty_reader(master_fd, output_tx.clone(), scrollback.clone());
        let state = Self {
            id,
            role,
            command,
            pid: Some(pid),
            master_fd: Some(master_fd),
            output_tx: Some(output_tx),
            scrollback: Some(scrollback),
        };
        (state, exit_rx)
    }
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
    /// Default shell command from config
    pub default_shell: String,
    /// Whether the session's network is isolated
    pub network_isolated: bool,
    /// Home directory inside the sandbox
    pub sandbox_home: String,
    /// Sandbox user name
    pub sandbox_user: String,
    /// User-defined env vars from config
    pub user_env: Vec<(String, String)>,
    /// Workspace path inside the sandbox (e.g. /workspace)
    pub sandbox_workspace: String,
    /// Auto-restart agent on exit
    pub auto_restart: bool,
    /// Delay before restarting agent (ms)
    pub restart_delay_ms: u64,
}

impl Session {
    /// Remove dead PTY processes (both agent and shell roles).
    fn prune_dead_ptys(&mut self) {
        self.ptys.retain(|p| match p.pid {
            Some(pid) => is_pid_alive(pid),
            None => true,
        });
    }

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
                    pid: p.pid,
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

/// Spawn a persistent PTY reader task that reads from master_fd, broadcasts
/// output to all subscribers, and appends to the scrollback buffer.
/// Returns a oneshot receiver that fires when the reader exits (EOF).
fn spawn_pty_reader(
    master_fd: RawFd,
    output_tx: broadcast::Sender<Bytes>,
    scrollback: Arc<Mutex<Vec<u8>>>,
) -> oneshot::Receiver<()> {
    let (exit_tx, exit_rx) = oneshot::channel();

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
                let _ = exit_tx.send(());
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
                        let mut sb = scrollback.lock().await;
                        sb.extend_from_slice(&buf[..n]);
                        // Trim to max size (keep the tail)
                        if sb.len() > SCROLLBACK_MAX {
                            let excess = sb.len() - SCROLLBACK_MAX;
                            sb.drain(..excess);
                        }
                    }

                    // Broadcast to any connected clients (ignore if none)
                    let _ = output_tx.send(data);
                }
                Ok(Err(_)) => break, // EOF or error
                Err(_would_block) => continue,
            }
        }

        let _ = exit_tx.send(());
    });

    exit_rx
}

/// Check if a process is still alive via kill(pid, 0)
fn is_pid_alive(pid: u32) -> bool {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        None,
    )
    .is_ok()
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
                    default_shell: "/bin/bash".to_string(),
                    network_isolated: false,
                    sandbox_home: "/home/coop".to_string(),
                    sandbox_user: "coop".to_string(),
                    user_env: vec![],
                    sandbox_workspace: "/workspace".to_string(),
                    auto_restart: true,
                    restart_delay_ms: 1000,
                },
            );
        }
    }

    pub async fn create_session(
        self: &Arc<Self>,
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
            .agent_command()
            .unwrap_or("claude")
            .to_string();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let sandbox_user = config.sandbox.user.clone();
        let sandbox_home = format!("/home/{}", sandbox_user);
        let network_isolated = config.network.mode != NetworkMode::Host;
        let default_shell = config.sandbox.shell_command().to_string();
        let sandbox_workspace = config.workspace.path.clone();
        let user_env: Vec<(String, String)> = config.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let auto_restart = config.session.auto_restart;
        let restart_delay_ms = config.session.restart_delay_ms;

        let (agent_pty, exit_rx) = PtyState::new(
            0,
            PtyRole::Agent,
            agent_cmd,
            ns_result.child_pid,
            ns_result.pty_master_fd,
        );
        let output_tx = agent_pty.output_tx.clone().unwrap();

        let session = Session {
            name: name.clone(),
            workspace: workspace.clone(),
            namespace_pid: ns_result.child_pid,
            created: now,
            ptys: vec![agent_pty],
            local_clients: 0,
            web_clients: 0,
            default_shell,
            network_isolated,
            sandbox_home,
            sandbox_user,
            user_env,
            sandbox_workspace,
            auto_restart,
            restart_delay_ms,
        };

        tracing::info!(
            session = %name,
            workspace = %workspace,
            pid = ns_result.child_pid,
            "Created session"
        );

        let mut sessions = self.sessions.write().await;
        sessions.insert(name.clone(), session);
        drop(sessions);

        self.spawn_exit_watcher(
            exit_rx,
            name.clone(),
            0,
            PtyRole::Agent,
            ns_result.child_pid,
            output_tx,
            auto_restart,
            restart_delay_ms,
        );

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
        self: &Arc<Self>,
        session_name: &str,
        command: Option<String>,
        force_new: bool,
        _cols: u16,
        _rows: u16,
    ) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_name).ok_or_else(|| {
            anyhow::anyhow!("Session '{}' not found", session_name)
        })?;

        let cmd = command.unwrap_or_else(|| session.default_shell.clone());

        session.prune_dead_ptys();

        // Unless forced, try to find an existing live shell running the same command
        if !force_new {
            if let Some(existing) = session.ptys.iter().find(|p| {
                p.role == PtyRole::Shell && p.command == cmd
            }) {
                return Ok(Response::ok_with(ResponseData {
                    pty: Some(existing.id),
                    ..Default::default()
                }));
            }
        }

        let pty_id = session.ptys.iter().map(|p| p.id).max().map_or(1, |m| m + 1);

        let env_vars = session.user_env.clone();
        let ns_pid = session.namespace_pid;
        let network_isolated = session.network_isolated;
        let sandbox_user = session.sandbox_user.clone();
        let sandbox_home = session.sandbox_home.clone();
        let sandbox_workspace = session.sandbox_workspace.clone();

        let shell_ns = namespace::nsenter_shell(
            ns_pid,
            &cmd,
            &env_vars,
            network_isolated,
            &sandbox_user,
            &sandbox_home,
            &sandbox_workspace,
        )?;

        let (shell_pty, exit_rx) = PtyState::new(
            pty_id,
            PtyRole::Shell,
            cmd,
            shell_ns.shell_pid,
            shell_ns.pty_master_fd,
        );
        let output_tx = shell_pty.output_tx.clone().unwrap();
        session.ptys.push(shell_pty);
        let session_name_owned = session_name.to_string();
        drop(sessions);

        self.spawn_exit_watcher(
            exit_rx,
            session_name_owned,
            pty_id,
            PtyRole::Shell,
            shell_ns.shell_pid,
            output_tx,
            false, // shells don't auto-restart
            0,
        );

        Ok(Response::ok_with(ResponseData {
            pty: Some(pty_id),
            ..Default::default()
        }))
    }

    /// Kill a specific PTY session within a box
    pub async fn kill_pty(&self, session_name: &str, pty_id: u32) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_name).ok_or_else(|| {
            anyhow::anyhow!("Session '{}' not found", session_name)
        })?;

        let pty_idx = session
            .ptys
            .iter()
            .position(|p| p.id == pty_id)
            .ok_or_else(|| anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name))?;

        let pty = &session.ptys[pty_idx];

        // Send SIGTERM to the shell process
        if let Some(pid) = pty.pid {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }

        // Close master fd
        if let Some(fd) = pty.master_fd {
            unsafe { nix::libc::close(fd) };
        }

        // Remove from the ptys list
        session.ptys.remove(pty_idx);

        tracing::info!(session = %session_name, pty = pty_id, "Killed PTY session");
        Ok(Response::ok())
    }

    /// List PTY sessions within a specific box
    pub async fn session_ls(&self, session_name: &str) -> Result<Response> {
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(session_name) {
                session.prune_dead_ptys();
            }
        }

        let sessions = self.sessions.read().await;
        let session = self.resolve_session(&sessions, session_name)?;
        let ptys: Vec<crate::ipc::PtyInfo> = session
            .ptys
            .iter()
            .map(|p| crate::ipc::PtyInfo {
                id: p.id,
                role: p.role.clone(),
                command: p.command.clone(),
                pid: p.pid,
            })
            .collect();

        Ok(Response::ok_with(ResponseData {
            session: Some(session.name.clone()),
            ptys: Some(ptys),
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

    /// Get scrollback logs for a PTY, optionally tail N lines.
    pub async fn get_logs(
        &self,
        session_name: &str,
        pty_id: u32,
        tail_lines: Option<usize>,
    ) -> Result<Response> {
        let sessions = self.sessions.read().await;
        let session = self.resolve_session(&sessions, session_name)?;

        let pty = session
            .ptys
            .iter()
            .find(|p| p.id == pty_id)
            .ok_or_else(|| anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name))?;

        let scrollback = pty
            .scrollback
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no scrollback buffer", pty_id))?;

        let data = scrollback.lock().await;
        let bytes = if let Some(n) = tail_lines {
            if n == 0 {
                data.clone()
            } else {
                // Scan backwards for N newlines
                let mut count = 0;
                let mut start = data.len();
                for i in (0..data.len()).rev() {
                    if data[i] == b'\n' {
                        count += 1;
                        if count >= n {
                            start = i + 1;
                            break;
                        }
                    }
                }
                data[start..].to_vec()
            }
        } else {
            data.clone()
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Response::ok_with(ResponseData {
            log_data: Some(encoded),
            ..Default::default()
        }))
    }

    /// Restart a PTY process (agent or shell). Reuses the same broadcast
    /// channel and scrollback so connected clients stay connected.
    pub async fn restart_pty(
        self: &Arc<Self>,
        session_name: &str,
        pty_id: u32,
    ) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_name).ok_or_else(|| {
            anyhow::anyhow!("Session '{}' not found", session_name)
        })?;

        let pty = session
            .ptys
            .iter()
            .find(|p| p.id == pty_id)
            .ok_or_else(|| anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name))?;

        let command = pty.command.clone();
        let role = pty.role.clone();
        let old_pid = pty.pid;
        let old_master_fd = pty.master_fd;
        let output_tx = pty.output_tx.clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no output channel", pty_id))?;
        let scrollback = pty.scrollback.clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no scrollback buffer", pty_id))?;

        let auto_restart = session.auto_restart;
        let restart_delay_ms = session.restart_delay_ms;

        // nsenter new process FIRST (keeps namespace alive)
        let env_vars = session.user_env.clone();
        let ns_pid = session.namespace_pid;
        let network_isolated = session.network_isolated;
        let sandbox_user = session.sandbox_user.clone();
        let sandbox_home = session.sandbox_home.clone();
        let sandbox_workspace = session.sandbox_workspace.clone();

        let shell_ns = namespace::nsenter_shell(
            ns_pid,
            &command,
            &env_vars,
            network_isolated,
            &sandbox_user,
            &sandbox_home,
            &sandbox_workspace,
        )?;

        // Kill old process
        if let Some(pid) = old_pid {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }

        // Close old master fd
        if let Some(fd) = old_master_fd {
            unsafe { nix::libc::close(fd) };
        }

        // Start new pty_reader with SAME output_tx and scrollback
        let exit_rx = spawn_pty_reader(shell_ns.pty_master_fd, output_tx.clone(), scrollback);

        // Update PtyState in-place
        let pty = session
            .ptys
            .iter_mut()
            .find(|p| p.id == pty_id)
            .unwrap();
        pty.pid = Some(shell_ns.shell_pid);
        pty.master_fd = Some(shell_ns.pty_master_fd);

        // If this was the agent (PTY 0), update namespace_pid
        if pty_id == 0 {
            session.namespace_pid = shell_ns.shell_pid;
        }

        let session_name_owned = session_name.to_string();
        drop(sessions);

        // Spawn watcher for the new process
        let watcher_auto_restart = matches!(role, PtyRole::Agent) && auto_restart;
        self.spawn_exit_watcher(
            exit_rx,
            session_name_owned,
            pty_id,
            role,
            shell_ns.shell_pid,
            output_tx,
            watcher_auto_restart,
            restart_delay_ms,
        );

        tracing::info!(
            session = %session_name,
            pty = pty_id,
            old_pid = ?old_pid,
            new_pid = shell_ns.shell_pid,
            "Restarted PTY"
        );

        Ok(Response::ok_with(ResponseData {
            pid: Some(shell_ns.shell_pid),
            pty: Some(pty_id),
            ..Default::default()
        }))
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

    /// Get the current PID of a PTY (used by watcher to detect stale restarts).
    async fn get_pty_pid(&self, session_name: &str, pty_id: u32) -> Option<u32> {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_name)
            .and_then(|s| s.ptys.iter().find(|p| p.id == pty_id))
            .and_then(|p| p.pid)
    }

    /// Spawn a background task that watches for a PTY reader to exit and takes
    /// the appropriate action: auto-restart for agents, cleanup for shells.
    fn spawn_exit_watcher(
        self: &Arc<Self>,
        exit_rx: oneshot::Receiver<()>,
        session_name: String,
        pty_id: u32,
        role: PtyRole,
        expected_pid: u32,
        output_tx: broadcast::Sender<Bytes>,
        auto_restart: bool,
        restart_delay_ms: u64,
    ) {
        let sm = Arc::clone(self);
        tokio::spawn(async move {
            // Wait for the PTY reader to exit
            let _ = exit_rx.await;

            // Check if someone already restarted this PTY (e.g. manual `coop restart`)
            if sm.get_pty_pid(&session_name, pty_id).await != Some(expected_pid) {
                return;
            }

            match role {
                PtyRole::Agent if auto_restart => {
                    // Notify connected clients via the broadcast channel
                    let msg = format!(
                        "\r\n\x1b[2m[agent exited, restarting in {}ms...]\x1b[0m\r\n",
                        restart_delay_ms
                    );
                    let _ = output_tx.send(Bytes::from(msg));

                    tokio::time::sleep(std::time::Duration::from_millis(restart_delay_ms)).await;

                    // Check again after delay
                    if sm.get_pty_pid(&session_name, pty_id).await != Some(expected_pid) {
                        return;
                    }

                    match sm.restart_pty(&session_name, pty_id).await {
                        Ok(_) => tracing::info!(
                            session = %session_name,
                            pty = pty_id,
                            "Auto-restarted agent"
                        ),
                        Err(e) => tracing::error!(
                            session = %session_name,
                            pty = pty_id,
                            error = %e,
                            "Failed to auto-restart agent"
                        ),
                    }
                }
                PtyRole::Agent => {
                    // auto_restart disabled — just notify
                    let msg = "\r\n\x1b[2m[agent exited]\x1b[0m\r\n";
                    let _ = output_tx.send(Bytes::from(msg));
                }
                PtyRole::Shell => {
                    // Clean up the dead shell PTY
                    tracing::info!(session = %session_name, pty = pty_id, "Shell exited, cleaning up");
                    let _ = sm.kill_pty(&session_name, pty_id).await;
                }
            }
        });
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
