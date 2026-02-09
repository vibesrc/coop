mod cli;
mod config;
mod daemon;
mod ipc;
mod pty;
mod sandbox;
mod tunnel;
mod web;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    // When re-execed as daemon, skip CLI parsing and run the server directly
    if daemon::spawn::is_daemon_mode() {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive("coop=info".parse()?),
            )
            .init();

        config::ensure_dirs()?;
        return daemon::server::DaemonServer::new().run().await;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("coop=info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    cli::run(cli).await
}
