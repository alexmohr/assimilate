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

#[must_use]
pub fn agent_version_string() -> &'static str {
    concat!(env!("APP_VERSION"), "+", env!("GIT_SHA"))
}

#[derive(Parser)]
#[command(version = agent_version_string())]
pub struct Args {
    #[arg(long, env = "BORG_SERVER_URL")]
    server_url: String,

    #[arg(long, env = "BORG_AGENT_TOKEN")]
    token: String,
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

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
        res = tokio::signal::ctrl_c() => {
            if let Err(e) = res {
                tracing::error!("Failed to listen for Ctrl+C: {e}");
            }
            tracing::info!("Received Ctrl+C, shutting down");
        }
    }
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
