// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `assimilate-agent` binary: connects to the server over WebSocket,
//! receives backup configuration, and runs `borg` on the local machine.

mod backup;
mod borg;
mod executor;
mod ssh_forward;
mod systemd;
mod task_registry;
mod ws;

use std::{process, time::Duration};

use clap::Parser;
use executor::{Executor, ExecutorCommand};
use shared::protocol::AgentToServer;
use task_registry::TaskRegistry;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

/// Extra time beyond borg's own SIGKILL-escalation delay ([`borg::kill_escalation_delay`])
/// that shutdown waits for the task registry (queued operations, plus each borg child's
/// reaper) to drain, before giving up and letting the process exit anyway.
const SHUTDOWN_GRACE_BUFFER: Duration = Duration::from_secs(10);

/// How long shutdown waits for the spawned `Executor::run` task to return once its
/// command channel has been closed. That loop only breaks on channel closure - it
/// doesn't itself wait on any in-flight operations - so this only needs to cover
/// ordinary task-scheduling latency, not a full backup.
const EXECUTOR_JOIN_GRACE: Duration = Duration::from_secs(5);

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

/// Version string reported to the server and by `--version`: the crate's
/// version, plus the build's git SHA suffix when one was embedded.
#[must_use]
pub fn agent_version_string() -> &'static str {
    if env!("GIT_SHA").is_empty() {
        env!("APP_VERSION")
    } else {
        concat!(env!("APP_VERSION"), "+", env!("GIT_SHA"))
    }
}

/// Command-line/environment arguments the agent needs to connect to its server.
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

    let task_registry = TaskRegistry::default();
    let executor = Executor::new(&args.server_url, &args.token, task_registry.clone());

    let executor_handle = tokio::spawn(async move {
        executor.run(exec_cmd_rx, outbound_tx).await;
    });

    // Either way this select resolves, the ws_client future (which owns exec_cmd_tx) is
    // gone once it returns below - completed on its own, or dropped as the losing branch -
    // closing the channel executor.run() is reading. That's the standard tokio idiom for
    // signalling a spawned task to wind down.
    let fatal = tokio::select! {
        result = ws::run_ws_client(&args, exec_cmd_tx, outbound_rx, &restart_capability) => {
            result.as_ref().err().is_some_and(ws::is_fatal)
        }
        () = shutdown_signal() => false,
    };

    // Give executor.run() a chance to actually return (it will, promptly, once the
    // channel above closes) and let every task it and borg's GracefulChild reapers
    // spawned finish cleanly, instead of the process exiting out from under them and
    // silently abandoning whatever they were mid-way through.
    match tokio::time::timeout(EXECUTOR_JOIN_GRACE, executor_handle).await {
        Ok(Err(e)) => tracing::warn!(error = %e, "executor task panicked during shutdown"),
        Ok(Ok(())) => {}
        Err(_) => tracing::warn!("executor task did not shut down within the grace period"),
    }
    let outstanding = task_registry
        .shutdown(borg::kill_escalation_delay().saturating_add(SHUTDOWN_GRACE_BUFFER))
        .await;
    if outstanding > 0 {
        tracing::warn!(
            outstanding,
            "background tasks still running at shutdown deadline"
        );
    }

    if fatal {
        process::exit(1);
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
