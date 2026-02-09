use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::net::UnixStream;

use crate::config;

/// Ensure the daemon is running, spawning it if necessary.
pub async fn ensure_daemon() -> Result<()> {
    let sock_path = config::socket_path()?;

    // Check if daemon is already running
    if sock_path.exists() {
        match UnixStream::connect(&sock_path).await {
            Ok(_) => return Ok(()), // Daemon is alive
            Err(_) => {
                // Stale socket, clean up
                tracing::info!("Removing stale daemon socket");
                let _ = std::fs::remove_file(&sock_path);
            }
        }
    }

    spawn_daemon()?;

    // Wait for daemon to be ready
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("Daemon failed to start within 2.5 seconds");
        }

        if sock_path.exists() {
            if UnixStream::connect(&sock_path).await.is_ok() {
                return Ok(());
            }
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Fork self as a daemon process.
fn spawn_daemon() -> Result<()> {
    tracing::info!("Spawning daemon");

    let exe = std::env::current_exe().context("Failed to get current executable path")?;

    // Use double-fork to detach from terminal
    match unsafe { fork::fork() } {
        Ok(fork::Fork::Parent(_)) => {
            // Parent: return and wait for socket
            return Ok(());
        }
        Ok(fork::Fork::Child) => {
            // Child: setsid and fork again
            let _ = unsafe { nix::libc::setsid() };
            match unsafe { fork::fork() } {
                Ok(fork::Fork::Parent(_)) => {
                    // Intermediate child: exit
                    std::process::exit(0);
                }
                Ok(fork::Fork::Child) => {
                    // Grandchild: this is the daemon
                    run_daemon_process(&exe);
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Second fork failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            bail!("Fork failed: {}", e);
        }
    }
}

/// The daemon process entry point (runs in the grandchild after double-fork).
fn run_daemon_process(exe: &Path) {
    // Redirect stdout/stderr to log file
    if let Ok(log_path) = config::log_file_path() {
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Re-exec ourselves in daemon mode using an env var marker
        let status = std::process::Command::new(exe)
            .env("COOP_DAEMON_MODE", "1")
            .stdout(std::fs::File::create(&log_path).unwrap_or_else(|_| {
                std::fs::File::open("/dev/null").unwrap()
            }))
            .stderr(std::fs::File::create(&log_path).unwrap_or_else(|_| {
                std::fs::File::open("/dev/null").unwrap()
            }))
            .spawn();

        if let Err(e) = status {
            eprintln!("Failed to exec daemon: {}", e);
        }
    }
}

/// Check if we're running in daemon mode (called from main).
pub fn is_daemon_mode() -> bool {
    std::env::var("COOP_DAEMON_MODE").is_ok()
}
