// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

mod backup;
mod borg;
mod executor;
mod ssh_forward;
mod systemd;
mod ws;

use std::process;

use clap::Parser;
use executor::{Executor, ExecutorCommand};
use shared::protocol::AgentToServer;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

// Resolves when SIGINT or SIGTERM is received.
// SIGTERM is critical for coverage builds: docker compose stop sends SIGTERM,
// and LLVM's atexit handler only runs when the process exits via exit(), not
// when terminated by an unhandled signal.
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal as unix_signal};
    let Ok(mut sigterm) = unix_signal(SignalKind::terminate()) else {
        tracing::error!("failed to install SIGTERM handler, relying on Ctrl+C only");
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("Failed to listen for Ctrl+C: {e}");
        }
        tracing::info!("Received Ctrl+C, shutting down");
        return;
    };
    tokio::select! {
        _ = sigterm.recv() => tracing::info!("Received SIGTERM, shutting down"),
        res = tokio::signal::ctrl_c() => {
            if let Err(e) = res {
                tracing::error!("Failed to listen for Ctrl+C: {e}");
            }
            tracing::info!("Received Ctrl+C, shutting down");
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!("Failed to listen for Ctrl+C: {e}");
    }
    tracing::info!("Received Ctrl+C, shutting down");
}

#[must_use]
pub fn agent_version_string() -> &'static str {
    if env!("GIT_SHA").is_empty() {
        env!("APP_VERSION")
    } else {
        concat!(env!("APP_VERSION"), "+", env!("GIT_SHA"))
    }
}

#[derive(Parser)]
#[command(version = agent_version_string())]
pub struct Args {
    #[arg(long, env = "BORG_SERVER_URL")]
    server_url: String,

    #[arg(long, env = "BORG_AGENT_TOKEN")]
    token: String,
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("failed to install rustls crypto provider")]
    RustlsProvider,
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| StartupError::RustlsProvider)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut args = Args::parse();
    args.server_url = normalize_ws_url(&args.server_url);

    let restart_capability = systemd::detect_restart_capability().await;

    let (exec_cmd_tx, exec_cmd_rx) = mpsc::channel::<ExecutorCommand>(16);
    let (outbound_tx, outbound_rx) = mpsc::channel::<AgentToServer>(64);

    let executor = Executor::new(&args.server_url, &args.token);

    tokio::spawn(async move {
        executor.run(exec_cmd_rx, outbound_tx).await;
    });

    tokio::select! {
        result = ws::run_ws_client(&args, exec_cmd_tx, outbound_rx, &restart_capability) => {
            if result.as_ref().err().is_some_and(ws::is_fatal) {
                process::exit(1);
            }
        }
        () = shutdown_signal() => {}
    }
    Ok(())
}

fn normalize_ws_url(url: &str) -> String {
    if url.starts_with("ws://") || url.starts_with("wss://") {
        return url.to_owned();
    }
    if let Some(rest) = url.strip_prefix("https://") {
        return format!("wss://{rest}");
    }
    if let Some(rest) = url.strip_prefix("http://") {
        return format!("ws://{rest}");
    }
    format!("ws://{url}")
}
