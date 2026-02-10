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

    /// Spawn a new shell PTY inside a box
    Shell {
        /// Box name
        name: Option<String>,

        /// Shell command (default: from config shell_command, or /bin/bash)
        #[arg(short, long)]
        command: Option<String>,
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

    /// Build the rootfs from the Coopfile
    Init {
        /// Coopfile path
        #[arg(short, long, default_value = "./coop.toml")]
        file: String,

        /// Ignore cached OCI layers
        #[arg(long)]
        no_cache: bool,
    },

    /// Rebuild the rootfs
    Rebuild,

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

    /// Manage named volumes
    Volume {
        #[command(subcommand)]
        action: VolumeAction,
    },

    /// Manage rootfs images and cache
    Image {
        #[command(subcommand)]
        action: ImageAction,
    },

    /// Gracefully shut down the daemon
    Shutdown,

    /// Tail the daemon log
    Logs {
        /// Follow log output
        #[arg(short, long)]
        follow: bool,

        /// Number of lines
        #[arg(short, default_value_t = 50)]
        n: usize,
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
    /// Spawn a new shell PTY inside a box
    Shell {
        /// Box name
        name: Option<String>,
        /// Shell command (default: from config shell_command, or /bin/bash)
        #[arg(short, long)]
        command: Option<String>,
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
pub enum VolumeAction {
    /// List named volumes
    Ls,
    /// Remove a named volume
    Rm {
        /// Volume name
        name: String,
    },
    /// Remove all named volumes
    Prune,
}

#[derive(Subcommand, Debug)]
pub enum ImageAction {
    /// Show rootfs and cache info
    Info,
    /// Remove the built rootfs (add --all to also remove OCI layer cache)
    Rm {
        /// Also remove OCI layer cache
        #[arg(long)]
        all: bool,
    },
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
        Some(Commands::Shell { name, command }) => cmd_shell(name, command.as_deref()).await?,
        Some(Commands::Ls { json }) => cmd_ls(json).await?,
        Some(Commands::Kill { name, all, force }) => cmd_kill(name, all, force).await?,
        Some(Commands::Box { action }) => {
            match action {
                BoxAction::Ls { json } => cmd_ls(json).await?,
                BoxAction::Attach { name } => cmd_attach(name).await?,
                BoxAction::Shell { name, command } => cmd_shell(name, command.as_deref()).await?,
                BoxAction::Kill { name, all, force } => cmd_kill(name, all, force).await?,
            }
        }
        Some(Commands::Init { file, no_cache }) => {
            crate::sandbox::init::build_rootfs(&file, no_cache).await?;
        }
        Some(Commands::Rebuild) => {
            crate::sandbox::init::build_rootfs("./coop.toml", true).await?;
        }
        Some(Commands::Status) => {
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
        Some(Commands::Volume { action }) => {
            let volumes_dir = crate::config::coop_dir()?.join("volumes");
            match action {
                VolumeAction::Ls => {
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
                VolumeAction::Rm { name } => {
                    let vol_path = volumes_dir.join(&name);
                    if vol_path.exists() {
                        std::fs::remove_dir_all(&vol_path)?;
                        println!("Removed volume: {}", name);
                    } else {
                        anyhow::bail!("Volume '{}' not found", name);
                    }
                }
                VolumeAction::Prune => {
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
            }
        }
        Some(Commands::Image { action }) => {
            let rootfs_path = crate::config::rootfs_base_path()?;
            let oci_path = crate::config::oci_cache_dir()?;
            match action {
                ImageAction::Info => {
                    if rootfs_path.exists() {
                        let size = dir_size(&rootfs_path);
                        println!("Rootfs:     {} ({})", rootfs_path.display(), format_size(size));
                        // Show manifest hash
                        let manifest = crate::config::coop_dir()?.join("rootfs").join("manifest");
                        if let Ok(hash) = std::fs::read_to_string(&manifest) {
                            println!("Config hash: {}", hash.trim());
                        }
                    } else {
                        println!("Rootfs:     not built");
                    }
                    if oci_path.exists() {
                        let size = dir_size(&oci_path);
                        println!("OCI cache:  {} ({})", oci_path.display(), format_size(size));
                    } else {
                        println!("OCI cache:  empty");
                    }
                }
                ImageAction::Rm { all } => {
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
            }
        }
        Some(Commands::Shutdown) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.shutdown().await?;
        }
        Some(Commands::Logs { follow, n }) => {
            crate::daemon::logs::tail_logs(follow, n).await?;
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

async fn cmd_attach(name: Option<String>) -> Result<()> {
    let client = crate::daemon::client::DaemonClient::connect().await?;
    let name = name.unwrap_or_else(default_box_name);
    client.attach(&name, 0).await
}

async fn cmd_shell(name: Option<String>, command: Option<&str>) -> Result<()> {
    let workspace = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Ensure rootfs exists
    crate::sandbox::init::ensure_rootfs(false, false).await?;

    let client = crate::daemon::client::DaemonClient::connect().await?;
    client
        .shell_or_create(name.as_deref(), &workspace, command)
        .await
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
