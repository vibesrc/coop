use std::path::PathBuf;

use anyhow::{Context, Result};

/// Returns the base coop directory: ~/.coop
pub fn coop_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".coop"))
}

/// Returns the daemon socket path: ~/.coop/sock
pub fn socket_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("sock"))
}

/// Returns the daemon PID file path: ~/.coop/daemon.pid
pub fn pid_file_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("daemon.pid"))
}

/// Returns the daemon lock file path: ~/.coop/daemon.lock
pub fn lock_file_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("daemon.lock"))
}

/// Returns the daemon log file path: ~/.coop/logs/daemon.log
pub fn log_file_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("logs").join("daemon.log"))
}

/// Returns the base rootfs path: ~/.coop/rootfs/base
pub fn rootfs_base_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("rootfs").join("base"))
}

/// Returns the sessions directory: ~/.coop/sessions
pub fn sessions_dir() -> Result<PathBuf> {
    Ok(coop_dir()?.join("sessions"))
}

/// Returns a specific session directory: ~/.coop/sessions/<name>
pub fn session_dir(name: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(name))
}

/// Returns the OCI cache directory: ~/.coop/cache/oci
pub fn oci_cache_dir() -> Result<PathBuf> {
    Ok(coop_dir()?.join("cache").join("oci"))
}

/// Returns the machine ID file path: ~/.coop/machine_id
pub fn machine_id_path() -> Result<PathBuf> {
    Ok(coop_dir()?.join("machine_id"))
}

/// Returns the global config path: ~/.config/coop/default.toml
pub fn global_config_path() -> Result<PathBuf> {
    let config = dirs::config_dir().context("Could not determine config directory")?;
    Ok(config.join("coop").join("default.toml"))
}

/// Ensures all required directories exist
pub fn ensure_dirs() -> Result<()> {
    let dirs = [
        coop_dir()?,
        coop_dir()?.join("logs"),
        coop_dir()?.join("rootfs"),
        sessions_dir()?,
        oci_cache_dir()?,
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    }
    Ok(())
}
