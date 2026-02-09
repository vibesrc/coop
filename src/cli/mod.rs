use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "coop", version, about = "Sandboxed AI agent sessions with remote access")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Create session but don't attach
    #[arg(short, long)]
    pub detach: bool,

    /// Workspace directory (default: cwd)
    #[arg(short, long)]
    pub workspace: Option<String>,

    /// Session name (default: derived from workspace basename)
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
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Attach to an existing session's agent PTY
    Attach {
        /// Session name or workspace path
        session: Option<String>,
    },

    /// Spawn a new shell PTY inside a session
    Shell {
        /// Session name
        session: Option<String>,

        /// Shell command
        #[arg(short, long, default_value = "/bin/sh")]
        command: String,
    },

    /// List all running sessions
    Ls {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Kill a session and all its processes
    Kill {
        /// Session name
        session: Option<String>,

        /// Kill all sessions
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
            crate::sandbox::init::ensure_rootfs(cli.build).await?;

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
                        tracing::error!(error = %e, "Failed to create/attach session");
                        return Err(e);
                    }
                }
            }
        }
        Some(Commands::Attach { session }) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            let session = session.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            });
            client.attach(&session, 0).await?;
        }
        Some(Commands::Shell { session, command }) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            let session = session.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            });
            client.shell(&session, Some(&command)).await?;
        }
        Some(Commands::Ls { json }) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            client.list_sessions(json).await?;
        }
        Some(Commands::Kill { session, all, force }) => {
            let client = crate::daemon::client::DaemonClient::connect().await?;
            if all {
                client.kill_all(force).await?;
            } else {
                let session = session.unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap()
                        .to_string_lossy()
                        .to_string()
                });
                client.kill(&session, force).await?;
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
