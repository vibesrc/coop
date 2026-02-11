use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use bytes::Bytes;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};

use crate::config::{self, Coopfile};
use crate::ipc::{
    PtyInfo, PtyRole, Response, ResponseData, SessionInfo, ERR_SESSION_EXISTS,
    ERR_SESSION_NOT_FOUND,
};
use crate::sandbox::namespace;
use base64::Engine;

/// Max scrollback buffer size (256KB)
const SCROLLBACK_MAX: usize = 256 * 1024;

/// State of a single PTY
#[derive(Debug, Clone)]
pub struct PtyState {
    pub id: u32,
    pub role: PtyRole,
    pub command: String,
    pub pid: Option<u32>,
    /// PTY master file descriptor (owned by daemon). Shared atomically so
    /// stream handlers always read the current fd after restarts. -1 = closed.
    pub master_fd: Arc<AtomicI32>,
    /// Broadcast channel for fan-out of PTY output to all attached clients.
    pub output_tx: Option<broadcast::Sender<Bytes>>,
    /// Shared scrollback buffer for replay on re-attach.
    pub scrollback: Option<Arc<Mutex<Vec<u8>>>>,
    /// Whether this PTY auto-restarts on exit
    pub auto_restart: bool,
}

impl PtyState {
    fn new(
        id: u32,
        role: PtyRole,
        command: String,
        pid: u32,
        master_fd: RawFd,
        auto_restart: bool,
    ) -> (Self, oneshot::Receiver<()>) {
        let (output_tx, _) = broadcast::channel(256);
        let scrollback = Arc::new(Mutex::new(Vec::new()));
        let exit_rx = spawn_pty_reader(master_fd, output_tx.clone(), scrollback.clone());
        let state = Self {
            id,
            role,
            command,
            pid: Some(pid),
            master_fd: Arc::new(AtomicI32::new(master_fd)),
            output_tx: Some(output_tx),
            scrollback: Some(scrollback),
            auto_restart,
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
    /// Home directory inside the sandbox
    pub sandbox_home: String,
    /// Sandbox user name
    pub sandbox_user: String,
    /// User-defined env vars from config
    pub user_env: Vec<(String, String)>,
    /// Workspace path inside the sandbox (e.g. /workspace)
    pub sandbox_workspace: String,
    /// Delay before restarting PTYs with auto_restart (ms)
    pub restart_delay_ms: u64,
    /// Pinned namespace fds — keep the namespace alive for restart support.
    /// -1 means not set (e.g. rediscovered sessions without namespace fds).
    pub ns_user_fd: RawFd,
    pub ns_mnt_fd: RawFd,
    pub ns_uts_fd: RawFd,
    pub ns_net_fd: Option<RawFd>,
    pub ns_root_fd: RawFd,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Close pinned namespace fds to release the namespace
        for fd in [
            self.ns_user_fd,
            self.ns_mnt_fd,
            self.ns_uts_fd,
            self.ns_root_fd,
        ] {
            if fd >= 0 {
                unsafe {
                    nix::libc::close(fd);
                }
            }
        }
        if let Some(fd) = self.ns_net_fd {
            if fd >= 0 {
                unsafe {
                    nix::libc::close(fd);
                }
            }
        }
    }
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
                let n = unsafe { nix::libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
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
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
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
    #[allow(dead_code)]
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
                    sandbox_home: "/home/coop".to_string(),
                    sandbox_user: "coop".to_string(),
                    user_env: vec![],
                    sandbox_workspace: "/workspace".to_string(),
                    restart_delay_ms: 1000,
                    // Rediscovered sessions don't have pinned fds — restart won't work
                    ns_user_fd: -1,
                    ns_mnt_fd: -1,
                    ns_uts_fd: -1,
                    ns_net_fd: None,
                    ns_root_fd: -1,
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
                    format!(
                        "Session '{}' already exists for this workspace",
                        existing.name
                    ),
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
        let default_shell = config.sandbox.shell_command().to_string();
        let sandbox_workspace = config.workspace.path.clone();
        let user_env: Vec<(String, String)> = config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let auto_restart = config.session.auto_restart;
        let restart_delay_ms = config.session.restart_delay_ms;

        let (agent_pty, exit_rx) = PtyState::new(
            0,
            PtyRole::Agent,
            agent_cmd,
            ns_result.child_pid,
            ns_result.pty_master_fd,
            auto_restart,
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
            sandbox_home,
            sandbox_user,
            user_env,
            sandbox_workspace,
            restart_delay_ms,
            ns_user_fd: ns_result.ns_user_fd,
            ns_mnt_fd: ns_result.ns_mnt_fd,
            ns_uts_fd: ns_result.ns_uts_fd,
            ns_net_fd: ns_result.ns_net_fd,
            ns_root_fd: ns_result.ns_root_fd,
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
        let name = Self::resolve_name(&sessions, session_name)?;
        let session = sessions.get_mut(&name).unwrap();

        let cmd = command.unwrap_or_else(|| session.default_shell.clone());

        session.prune_dead_ptys();

        // Unless forced, try to find an existing live shell running the same command
        if !force_new {
            if let Some(existing) = session
                .ptys
                .iter()
                .find(|p| p.role == PtyRole::Shell && p.command == cmd)
            {
                return Ok(Response::ok_with(ResponseData {
                    pty: Some(existing.id),
                    ..Default::default()
                }));
            }
        }

        let pty_id = session.ptys.iter().map(|p| p.id).max().map_or(1, |m| m + 1);

        let env_vars = session.user_env.clone();
        let ns_user_fd = session.ns_user_fd;
        let ns_mnt_fd = session.ns_mnt_fd;
        let ns_uts_fd = session.ns_uts_fd;
        let ns_net_fd = session.ns_net_fd;
        let ns_root_fd = session.ns_root_fd;
        let sandbox_user = session.sandbox_user.clone();
        let sandbox_home = session.sandbox_home.clone();
        let sandbox_workspace = session.sandbox_workspace.clone();

        let shell_ns = namespace::nsenter_shell(
            ns_user_fd,
            ns_mnt_fd,
            ns_uts_fd,
            ns_net_fd,
            ns_root_fd,
            &cmd,
            &env_vars,
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
            false,
        );
        let output_tx = shell_pty.output_tx.clone().unwrap();
        session.ptys.push(shell_pty);
        drop(sessions);

        self.spawn_exit_watcher(
            exit_rx,
            name,
            pty_id,
            shell_ns.shell_pid,
            output_tx,
            false,
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
        let name = Self::resolve_name(&sessions, session_name)?;
        let session = sessions.get_mut(&name).unwrap();

        let pty_idx = session
            .ptys
            .iter()
            .position(|p| p.id == pty_id)
            .ok_or_else(|| {
                anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name)
            })?;

        let pty = &session.ptys[pty_idx];

        // Send SIGTERM to the shell process
        if let Some(pid) = pty.pid {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }

        // Close master fd (atomic swap to -1)
        let fd = pty.master_fd.swap(-1, Ordering::SeqCst);
        if fd >= 0 {
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
            .ok_or_else(|| {
                anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name)
            })?;

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

    /// Restart a PTY process (agent or shell). Re-reads coop.toml to pick up
    /// config changes (agent command, env vars, etc.). Reuses the same broadcast
    /// channel and scrollback so connected clients stay connected.
    pub async fn restart_pty(
        self: &Arc<Self>,
        session_name: &str,
        pty_id: u32,
    ) -> Result<Response> {
        let mut sessions = self.sessions.write().await;
        let name = Self::resolve_name(&sessions, session_name)?;
        let session = sessions.get_mut(&name).unwrap();

        let pty = session
            .ptys
            .iter()
            .find(|p| p.id == pty_id)
            .ok_or_else(|| {
                anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name)
            })?;

        let old_pid = pty.pid;
        let master_fd_ref = pty.master_fd.clone();
        let output_tx = pty
            .output_tx
            .clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no output channel", pty_id))?;
        let scrollback = pty
            .scrollback
            .clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no scrollback buffer", pty_id))?;

        // Re-read coop.toml to pick up config changes
        let workspace_path = PathBuf::from(&session.workspace);
        let mut config = Coopfile::resolve(&workspace_path, None).unwrap_or_default();
        config.expand_env();

        // Update session-level settings from fresh config
        session.default_shell = config.sandbox.shell_command().to_string();
        session.user_env = config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        session.restart_delay_ms = config.session.restart_delay_ms;

        // Determine the command: agent (PTY 0) picks up new agent command,
        // shells keep their original command
        let command = if pty_id == 0 {
            let new_cmd = config
                .sandbox
                .agent_command()
                .unwrap_or("claude")
                .to_string();
            new_cmd
        } else {
            pty.command.clone()
        };

        // Update PTY-level settings
        let auto_restart = if pty_id == 0 {
            config.session.auto_restart
        } else {
            pty.auto_restart
        };
        let restart_delay_ms = session.restart_delay_ms;

        // nsenter new process using pinned namespace fds (works even after init dies)
        let env_vars = session.user_env.clone();
        let ns_user_fd = session.ns_user_fd;
        let ns_mnt_fd = session.ns_mnt_fd;
        let ns_uts_fd = session.ns_uts_fd;
        let ns_net_fd = session.ns_net_fd;
        let ns_root_fd = session.ns_root_fd;
        let sandbox_user = session.sandbox_user.clone();
        let sandbox_home = session.sandbox_home.clone();
        let sandbox_workspace = session.sandbox_workspace.clone();

        let shell_ns = namespace::nsenter_shell(
            ns_user_fd,
            ns_mnt_fd,
            ns_uts_fd,
            ns_net_fd,
            ns_root_fd,
            &command,
            &env_vars,
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

        // Close old master fd and atomically swap to new fd.
        // Stream handlers read from the same Arc<AtomicI32>, so they
        // immediately start writing to the new fd after this.
        let old_fd = master_fd_ref.swap(shell_ns.pty_master_fd, Ordering::SeqCst);
        if old_fd >= 0 {
            unsafe { nix::libc::close(old_fd) };
        }

        // Start new pty_reader with SAME output_tx and scrollback
        let exit_rx = spawn_pty_reader(shell_ns.pty_master_fd, output_tx.clone(), scrollback);

        // Update PtyState in-place
        let pty = session.ptys.iter_mut().find(|p| p.id == pty_id).unwrap();
        pty.pid = Some(shell_ns.shell_pid);
        pty.command = command;
        pty.auto_restart = auto_restart;

        // If this was the agent (PTY 0), update namespace_pid
        if pty_id == 0 {
            session.namespace_pid = shell_ns.shell_pid;
        }

        drop(sessions);

        // Spawn watcher for the new process (only auto-restart if the PTY had it before)
        self.spawn_exit_watcher(
            exit_rx,
            name,
            pty_id,
            shell_ns.shell_pid,
            output_tx,
            auto_restart,
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
    /// The master_fd is an `Arc<AtomicI32>` so stream handlers always
    /// read the current fd even after a PTY restart.
    pub async fn get_pty_handle(
        &self,
        session_name: &str,
        pty_id: u32,
    ) -> Result<(
        Arc<AtomicI32>,
        broadcast::Sender<Bytes>,
        Option<Arc<Mutex<Vec<u8>>>>,
    )> {
        let sessions = self.sessions.read().await;
        let session = self.resolve_session(&sessions, session_name)?;

        let pty = session
            .ptys
            .iter()
            .find(|p| p.id == pty_id)
            .ok_or_else(|| {
                anyhow::anyhow!("PTY {} not found in session '{}'", pty_id, session_name)
            })?;

        let output_tx = pty
            .output_tx
            .clone()
            .ok_or_else(|| anyhow::anyhow!("PTY {} has no output channel", pty_id))?;

        Ok((pty.master_fd.clone(), output_tx, pty.scrollback.clone()))
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

    /// Spawn a background task that watches for a PTY to exit.
    /// If auto_restart is true, restarts the process after a delay.
    /// Otherwise, cleans up the dead PTY.
    #[allow(clippy::too_many_arguments)]
    fn spawn_exit_watcher(
        self: &Arc<Self>,
        exit_rx: oneshot::Receiver<()>,
        session_name: String,
        pty_id: u32,
        expected_pid: u32,
        output_tx: broadcast::Sender<Bytes>,
        auto_restart: bool,
        restart_delay_ms: u64,
    ) {
        let sm = Arc::clone(self);
        tokio::spawn(async move {
            let _ = exit_rx.await;

            // Check if someone already restarted this PTY
            if sm.get_pty_pid(&session_name, pty_id).await != Some(expected_pid) {
                return;
            }

            if auto_restart {
                let msg = format!(
                    "\r\n\x1b[2m[process exited, restarting in {}ms...]\x1b[0m\r\n",
                    restart_delay_ms
                );
                let _ = output_tx.send(Bytes::from(msg));

                tokio::time::sleep(std::time::Duration::from_millis(restart_delay_ms)).await;

                if sm.get_pty_pid(&session_name, pty_id).await != Some(expected_pid) {
                    return;
                }

                match sm.restart_pty(&session_name, pty_id).await {
                    Ok(_) => {
                        tracing::info!(session = %session_name, pty = pty_id, "Auto-restarted PTY")
                    }
                    Err(e) => {
                        tracing::error!(session = %session_name, pty = pty_id, error = %e, "Failed to auto-restart PTY")
                    }
                }
            } else {
                tracing::info!(session = %session_name, pty = pty_id, "PTY exited, cleaning up");
                let _ = sm.kill_pty(&session_name, pty_id).await;
            }
        });
    }

    /// Resolve a session name or workspace path to the actual session key.
    fn resolve_name(sessions: &HashMap<String, Session>, name_or_path: &str) -> Result<String> {
        if sessions.contains_key(name_or_path) {
            return Ok(name_or_path.to_string());
        }
        if name_or_path.contains('/') {
            if let Some(s) = sessions.values().find(|s| s.workspace == name_or_path) {
                return Ok(s.name.clone());
            }
        }
        bail!("Session '{}' not found", name_or_path);
    }

    fn resolve_session<'a>(
        &self,
        sessions: &'a HashMap<String, Session>,
        name_or_path: &str,
    ) -> Result<&'a Session> {
        let key = Self::resolve_name(sessions, name_or_path)?;
        Ok(&sessions[&key])
    }
}
