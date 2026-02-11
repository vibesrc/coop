use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "coop", version, about = "Sandboxed AI agent sessions with remote access")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Create box but don't attach
    #[arg(short, long)]
    pub detach: bool,

    /// Workspace directory (default: cwd)
    #[arg(short, long)]
    pub workspace: Option<String>,

    /// Box name (default: derived from workspace basename)
    #[arg(short, long)]
    pub name: Option<String>,

    /// Global config path
    #[arg(long, default_value = "~/.config/coop/default.toml")]
    pub config: String,

    /// Daemon socket path
    #[arg(long)]
    pub socket: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(short, long)]
    pub quiet: bool,

    /// Force rebuild rootfs before starting
    #[arg(short, long)]
    pub build: bool,

    /// Ignore cached rootfs and rebuild from scratch (use with --build)
    #[arg(long)]
    pub no_cache: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Attach to a running box's agent PTY
    Attach {
        /// Box name or workspace path
        name: Option<String>,
    },

    /// Open or manage shell sessions inside a box
    Shell {
        #[command(subcommand)]
        action: Option<ShellAction>,

        /// Shell command (default: from config shell_command, or /bin/bash)
        #[arg(short, long)]
        command: Option<String>,

        /// Force create a new shell (don't reattach to existing)
        #[arg(long)]
        new: bool,
    },

    /// List all running boxes
    Ls {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Kill a box and all its processes
    Kill {
        /// Box name
        name: Option<String>,

        /// Kill all boxes
        #[arg(long)]
        all: bool,

        /// Force kill (SIGKILL, no grace period)
        #[arg(short)]
        force: bool,
    },

    /// Initialize a new coop.toml in the current directory
    Init,

    /// Build (or rebuild) the rootfs from the Coopfile
    Build {
        /// Ignore cached OCI layers and rootfs
        #[arg(long)]
        no_cache: bool,
    },

    /// Show status
    Status,

    /// Start the embedded web UI server
    Serve {
        /// Port number
        #[arg(short, long, default_value_t = 8888)]
        port: u16,

        /// Bind address
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Use a specific auth token
        #[arg(long)]
        token: Option<String>,

        /// Stop the running web server
        #[arg(long)]
        stop: bool,
    },

    /// Create a P2P WebRTC tunnel
    Tunnel {
        /// Custom STUN server
        #[arg(long)]
        stun: Option<String>,

        /// Disable STUN (LAN only)
        #[arg(long)]
        no_stun: bool,

        /// Don't display QR code
        #[arg(long)]
        no_qr: bool,
    },

    /// Manage boxes
    Box {
        #[command(subcommand)]
        action: BoxAction,
    },

    /// Manage PTY sessions within a box
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Manage the coop system (daemon, volumes, images, cache)
    System {
        #[command(subcommand)]
        action: SystemAction,
    },

    /// View agent (PTY 0) scrollback logs
    Logs {
        /// Follow output live
        #[arg(short, long)]
        follow: bool,

        /// Show last N lines (0 = all)
        #[arg(short, default_value_t = 0)]
        n: usize,
    },

    /// Restart the agent process (PTY 0)
    Restart,

    /// Update coop to the latest release
    Update {
        /// Check for updates without installing
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum BoxAction {
    /// List all running boxes
    Ls {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Attach to a box's agent PTY
    Attach {
        /// Box name
        name: Option<String>,
    },
    /// Open a shell in a box
    Shell {
        /// Box name
        name: Option<String>,
        /// Shell command (default: from config shell_command, or /bin/bash)
        #[arg(short, long)]
        command: Option<String>,
        /// Force create a new shell (don't reattach to existing)
        #[arg(long)]
        new: bool,
    },
    /// Kill a box
    Kill {
        /// Box name
        name: Option<String>,
        /// Kill all boxes
        #[arg(long)]
        all: bool,
        /// Force kill (SIGKILL, no grace period)
        #[arg(short)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ShellAction {
    /// List shell sessions in the current box
    Ls,
    /// Attach to an existing shell session by ID
    Attach {
        /// PTY session ID
        id: u32,
    },
    /// Kill a shell session
    Kill {
        /// PTY session ID
        id: u32,
    },
    /// View shell PTY scrollback logs
    Logs {
        /// PTY session ID
        id: u32,
        /// Follow output live
        #[arg(short, long)]
        follow: bool,
        /// Show last N lines (0 = all)
        #[arg(short, default_value_t = 0)]
        n: usize,
    },
    /// Restart a shell process
    Restart {
        /// PTY session ID (default: first shell)
        id: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
pub enum SessionAction {
    /// List PTY sessions in a box
    Ls {
        /// Box name (default: derived from cwd)
        name: Option<String>,
    },
    /// Kill a specific PTY session
    Kill {
        /// Box name
        name: String,
        /// PTY session ID
        pty: u32,
    },
}

#[derive(Subcommand, Debug)]
pub enum SystemAction {
    /// Show daemon and system status
    Status,
    /// Tail the daemon log
    Logs {
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
        /// Number of lines
        #[arg(short, default_value_t = 50)]
        n: usize,
    },
    /// Gracefully shut down the daemon
    Shutdown,
    /// List named volumes
    Volumes,
    /// Remove a named volume
    #[command(name = "volume-rm")]
    VolumeRm {
        /// Volume name
        name: String,
    },
    /// Remove all unused volumes
    #[command(name = "volume-prune")]
    VolumePrune,
    /// Show rootfs and cache disk usage
    Df,
    /// Remove rootfs and/or OCI cache
    Clean {
        /// Also remove OCI layer cache
        #[arg(long)]
        all: bool,
    },
    /// Remove everything (rootfs, cache, volumes, sessions)
    Prune,
}

pub async fn run(cli: Cli) -> Result<()> {
    crate::config::ensure_dirs()?;

    match cli.command {
        None => {
            // Smart default: create or attach
            let workspace = cli
                .workspace
                .unwrap_or_else(|| std::env::current_dir().unwrap().to_string_lossy().to_string());
            tracing::info!(workspace = %workspace, "Smart default: create or attach");

            // Ensure rootfs exists (first run auto-builds, --build forces rebuild)
            crate::sandbox::init::ensure_rootfs(cli.build, cli.no_cache).await?;

            let client = crate::daemon::client::DaemonClient::connect().await?;

            if cli.detach {
                client
                    .create_session(cli.name.as_deref(), &workspace, true)
                    .await?;
            } else {
                // Try attach first, create if not found
                match client.attach_or_create(cli.name.as_deref(), &workspace).await {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to create/attach box");
                        return Err(e);
                    }
                }
            }
        }
        Some(Commands::Attach { name }) => cmd_attach(name).await?,
        Some(Commands::Shell { action, command, new }) => {
            match action {
                None => cmd_shell(None, command.as_deref(), new).await?,
                Some(ShellAction::Ls) => cmd_shell_ls().await?,
                Some(ShellAction::Attach { id }) => cmd_shell_attach(id).await?,
                Some(ShellAction::Kill { id }) => cmd_shell_kill(id).await?,
                Some(ShellAction::Logs { id, follow, n }) => {
                    let box_name = default_box_name();
                    let tail = if n > 0 { Some(n) } else { None };
                    let client = crate::daemon::client::DaemonClient::connect().await?;
                    client.logs(&box_name, id, follow, tail).await?;
                }
                Some(ShellAction::Restart { id }) => {
                    let box_name = default_box_name();
                    let pty_id = id.unwrap_or(1); // Default to first shell (PTY 1)
                    let client = crate::daemon::client::DaemonClient::connect().await?;
                    client.restart(&box_name, pty_id).await?;
                }
            }
        }
        Some(Commands::Ls { json }) => cmd_ls(json).await?,
        Some(Commands::Kill { name, all, force }) => cmd_kill(name, all, force).await?,
        Some(Commands::Box { action }) => {
            match action {
                BoxAction::Ls { json } => cmd_ls(json).await?,
                BoxAction::Attach { name } => cmd_attach(name).await?,
                BoxAction::Shell { name, command, new } => cmd_shell(name, command.as_deref(), new).await?,
                BoxAction::Kill { name, all, force } => cmd_kill(name, all, force).await?,
            }
        }
        Some(Commands::Init) => {
            cmd_init().await?;
        }
        Some(Commands::Build { no_cache }) => {
            crate::sandbox::init::build_rootfs("./coop.toml", no_cache).await?;
        }
        Some(Commands::Status) => {
            // Keep as a convenience alias for `coop system status`
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.status().await?;
        }
        Some(Commands::Serve {
            port,
            host,
            token,
            stop,
        }) => {
            if stop {
                let client = crate::daemon::client::DaemonClient::connect().await?;
                client.stop_serve().await?;
            } else {
                let client = crate::daemon::client::DaemonClient::connect().await?;
                client.serve(port, &host, token.as_deref()).await?;
            }
        }
        Some(Commands::Tunnel {
            stun,
            no_stun,
            no_qr,
        }) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.tunnel(stun.as_deref(), no_stun, no_qr).await?;
        }
        Some(Commands::Session { action }) => {
            match action {
                SessionAction::Ls { name } => {
                    let client = crate::daemon::client::DaemonClient::connect().await?;
                    match name {
                        Some(n) => client.session_ls(&n).await?,
                        None => client.session_ls_all().await?,
                    }
                }
                SessionAction::Kill { name, pty } => {
                    let client = crate::daemon::client::DaemonClient::connect().await?;
                    client.session_kill(&name, pty).await?;
                }
            }
        }
        Some(Commands::System { action }) => {
            cmd_system(action).await?;
        }
        Some(Commands::Logs { follow, n }) => {
            let box_name = default_box_name();
            let tail = if n > 0 { Some(n) } else { None };
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.logs(&box_name, 0, follow, tail).await?;
        }
        Some(Commands::Restart) => {
            let box_name = default_box_name();
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.restart(&box_name, 0).await?;
        }
        Some(Commands::Update { check }) => {
            cmd_update(check)?;
        }
    }

    Ok(())
}

fn default_box_name() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

const DEFAULT_COOP_TOML: &str = r#"[sandbox]
image = "debian:latest"
agent = "claude"
shell = "bash"
user = "coop"

setup = [
  "DEBIAN_FRONTEND=noninteractive apt-get update && apt-get install -y bash curl git ca-certificates",
]

mounts = [
  "claude-config:~/.claude",
]
"#;

async fn cmd_init() -> Result<()> {
    let path = std::path::Path::new("coop.toml");
    if path.exists() {
        anyhow::bail!("coop.toml already exists in this directory");
    }
    std::fs::write(path, DEFAULT_COOP_TOML)?;
    println!("Created coop.toml");
    Ok(())
}

async fn cmd_attach(name: Option<String>) -> Result<()> {
    let client = crate::daemon::client::DaemonClient::connect().await?;
    let name = name.unwrap_or_else(default_box_name);
    client.attach(&name, 0).await
}

async fn cmd_shell(name: Option<String>, command: Option<&str>, force_new: bool) -> Result<()> {
    let workspace = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Ensure rootfs exists
    crate::sandbox::init::ensure_rootfs(false, false).await?;

    let client = crate::daemon::client::DaemonClient::connect().await?;
    client
        .shell_or_create(name.as_deref(), &workspace, command, force_new)
        .await
}

async fn cmd_shell_attach(id: u32) -> Result<()> {
    let box_name = default_box_name();
    let client = crate::daemon::client::DaemonClient::connect().await?;
    client.attach(&box_name, id).await
}

async fn cmd_shell_ls() -> Result<()> {
    let box_name = default_box_name();
    let client = crate::daemon::client::DaemonClient::connect().await?;
    client.session_ls(&box_name).await
}

async fn cmd_shell_kill(id: u32) -> Result<()> {
    let box_name = default_box_name();
    let client = crate::daemon::client::DaemonClient::connect().await?;
    client.session_kill(&box_name, id).await
}

async fn cmd_ls(json: bool) -> Result<()> {
    let client = crate::daemon::client::DaemonClient::connect().await?;
    client.list_sessions(json).await
}

async fn cmd_kill(name: Option<String>, all: bool, force: bool) -> Result<()> {
    let client = crate::daemon::client::DaemonClient::connect().await?;
    if all {
        client.kill_all(force).await
    } else {
        let name = name.unwrap_or_else(default_box_name);
        client.kill(&name, force).await
    }
}

async fn cmd_system(action: SystemAction) -> Result<()> {
    match action {
        SystemAction::Status => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.status().await?;
        }
        SystemAction::Logs { follow, n } => {
            crate::daemon::logs::tail_logs(follow, n).await?;
        }
        SystemAction::Shutdown => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.shutdown().await?;
        }
        SystemAction::Volumes => {
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            if !volumes_dir.exists() {
                println!("No volumes.");
                return Ok(());
            }
            let mut found = false;
            for entry in std::fs::read_dir(&volumes_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let name = entry.file_name();
                    let size = dir_size(&entry.path());
                    println!("{:<30} {}", name.to_string_lossy(), format_size(size));
                    found = true;
                }
            }
            if !found {
                println!("No volumes.");
            }
        }
        SystemAction::VolumeRm { name } => {
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            let vol_path = volumes_dir.join(&name);
            if vol_path.exists() {
                std::fs::remove_dir_all(&vol_path)?;
                println!("Removed volume: {}", name);
            } else {
                anyhow::bail!("Volume '{}' not found", name);
            }
        }
        SystemAction::VolumePrune => {
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            if volumes_dir.exists() {
                let mut count = 0;
                for entry in std::fs::read_dir(&volumes_dir)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        std::fs::remove_dir_all(entry.path())?;
                        count += 1;
                    }
                }
                println!("Removed {} volume(s).", count);
            } else {
                println!("No volumes to prune.");
            }
        }
        SystemAction::Df => {
            let rootfs_path = crate::config::rootfs_base_path()?;
            let oci_path = crate::config::oci_cache_dir()?;
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            let sessions_dir = crate::config::sessions_dir()?;

            let mut total = 0u64;

            if rootfs_path.exists() {
                let size = dir_size(&rootfs_path);
                total += size;
                println!("Rootfs:     {}", format_size(size));
            } else {
                println!("Rootfs:     not built");
            }
            if oci_path.exists() {
                let size = dir_size(&oci_path);
                total += size;
                println!("OCI cache:  {}", format_size(size));
            } else {
                println!("OCI cache:  empty");
            }
            if volumes_dir.exists() {
                let size = dir_size(&volumes_dir);
                total += size;
                println!("Volumes:    {}", format_size(size));
            } else {
                println!("Volumes:    empty");
            }
            if sessions_dir.exists() {
                let size = dir_size(&sessions_dir);
                total += size;
                println!("Sessions:   {}", format_size(size));
            }
            println!("Total:      {}", format_size(total));
        }
        SystemAction::Clean { all } => {
            let rootfs_path = crate::config::rootfs_base_path()?;
            let oci_path = crate::config::oci_cache_dir()?;
            let mut removed = false;
            if rootfs_path.exists() {
                std::fs::remove_dir_all(&rootfs_path)?;
                let manifest = crate::config::coop_dir()?.join("rootfs").join("manifest");
                let _ = std::fs::remove_file(&manifest);
                println!("Removed rootfs.");
                removed = true;
            }
            if all && oci_path.exists() {
                std::fs::remove_dir_all(&oci_path)?;
                println!("Removed OCI cache.");
                removed = true;
            }
            if !removed {
                println!("Nothing to remove.");
            }
        }
        SystemAction::Prune => {
            let rootfs_path = crate::config::rootfs_base_path()?;
            let oci_path = crate::config::oci_cache_dir()?;
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            let sessions_dir = crate::config::sessions_dir()?;

            let mut removed = false;
            if rootfs_path.exists() {
                std::fs::remove_dir_all(&rootfs_path)?;
                let manifest = crate::config::coop_dir()?.join("rootfs").join("manifest");
                let _ = std::fs::remove_file(&manifest);
                println!("Removed rootfs.");
                removed = true;
            }
            if oci_path.exists() {
                std::fs::remove_dir_all(&oci_path)?;
                println!("Removed OCI cache.");
                removed = true;
            }
            if volumes_dir.exists() {
                std::fs::remove_dir_all(&volumes_dir)?;
                println!("Removed all volumes.");
                removed = true;
            }
            if sessions_dir.exists() {
                std::fs::remove_dir_all(&sessions_dir)?;
                println!("Removed all session data.");
                removed = true;
            }
            if !removed {
                println!("Nothing to remove.");
            }
        }
    }
    Ok(())
}

fn cmd_update(check: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let updater = self_update::backends::github::Update::configure()
        .repo_owner("vibesrc")
        .repo_name("coop")
        .bin_name("coop")
        .target(self_update::get_target())
        .current_version(current)
        .show_download_progress(true)
        .no_confirm(true)
        .build()?;

    if check {
        match updater.get_latest_release() {
            Ok(latest) => {
                println!("Current: v{}", current);
                println!("Latest:  {}", latest.version);
                if latest.version == current {
                    println!("Already up to date.");
                }
            }
            Err(e) => {
                println!("Current: v{}", current);
                println!("Could not check for updates: {}", e);
            }
        }
    } else {
        match updater.update() {
            Ok(status) => {
                println!("Updated to v{}!", status.version());
            }
            Err(e) => {
                eprintln!("Update failed: {}", e);
                return Err(e.into());
            }
        }
    }
    Ok(())
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                } else if meta.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
