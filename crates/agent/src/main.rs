// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `assimilate-agent` binary: connects to the server over WebSocket,
//! receives backup configuration, and runs `borg` on the local machine.

mod backup;
mod borg;
mod database;
mod endpoint;
mod executor;
mod live_file;
#[cfg(windows)]
mod service;
mod ssh_forward;
mod systemd;
#[cfg(test)]
mod tests {
    #[cfg(unix)]
    pub(crate) mod fake_docker;
    pub(crate) mod fake_kube;
    pub(crate) mod fake_server;
    #[cfg(unix)]
    pub(crate) mod host_postgres;
    pub(crate) mod logs;
    pub(crate) mod messages;
}
mod vm;
mod ws;

use std::{future::Future, process::ExitCode, time::Duration};

use clap::Parser;
pub use endpoint::ServerEndpoint;
use executor::{Executor, ExecutorCommand};
use shared::{
    agent_channel::ChannelPublicKey, protocol::AgentToServer, task_registry::TaskRegistry,
};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

/// Extra time beyond borg's own SIGKILL-escalation delay ([`shared::borg::kill_escalation_delay`])
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

/// Where the agent reads its token. It is a secret, so it has no command-line
/// flag: other local users can read a process's command line.
const TOKEN_VAR: &str = "BORG_AGENT_TOKEN";

/// The command line. Without a subcommand the agent connects and runs in the
/// foreground; on Windows, `service` installs, removes or runs it as a
/// Windows service instead.
#[derive(Parser)]
#[command(
    version = agent_version_string(),
    subcommand_negates_reqs = true,
    after_help = "The agent token is read from the BORG_AGENT_TOKEN environment variable only."
)]
struct Cli {
    #[arg(long, env = "BORG_SERVER_URL", required = true)]
    server_url: Option<String>,

    /// The server's agent channel key, shown on the server's System page and
    /// with every new agent token. The agent only talks to a server holding the
    /// matching private key.
    #[arg(long, env = "BORG_SERVER_KEY", required = true)]
    server_key: Option<ChannelPublicKey>,

    #[cfg(windows)]
    #[command(subcommand)]
    command: Option<service::Command>,
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("failed to install rustls crypto provider")]
    RustlsProvider,
    #[error("failed to start the async runtime: {0}")]
    Runtime(std::io::Error),
    #[error("--server-url and --server-key are required, and BORG_AGENT_TOKEN must be set")]
    MissingArgs,
    #[cfg(windows)]
    #[error(transparent)]
    Service(#[from] service::ServiceError),
}

fn main() -> Result<ExitCode, StartupError> {
    // On Windows this process may be started to stop one of its own borg children.
    shared::console::run_interrupt_helper_if_requested();
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| StartupError::RustlsProvider)?;

    let cli = Cli::parse();
    #[cfg(windows)]
    if let Some(command) = cli.command {
        service::run(command)?;
        return Ok(ExitCode::SUCCESS);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = cli
        .connect_args(env_token())
        .ok_or(StartupError::MissingArgs)?;
    let fatal = runtime()?.block_on(async {
        let restart_capability = systemd::detect_restart_capability().await;
        run(args, shutdown_signal(), restart_capability).await
    });
    // Exit status 1 after a fatal error (one retrying won't fix).
    Ok(ExitCode::from(u8::from(fatal)))
}

/// The agent token from the environment, if it is set and not empty.
fn env_token() -> Option<String> {
    std::env::var(TOKEN_VAR)
        .ok()
        .filter(|token| !token.is_empty())
}

impl Cli {
    /// The server and server key to connect with, and `token`. Clap requires
    /// the server and key unless a subcommand is given, so only a
    /// subcommand's command line lacks them.
    fn connect_args(self, token: Option<String>) -> Option<ServerEndpoint> {
        Some(ServerEndpoint {
            url: self.server_url?,
            token: token?,
            key: self.server_key?,
        })
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, StartupError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(StartupError::Runtime)
}

/// Connects to the server and works until the connection fails for good or
/// `shutdown` resolves, then winds down. Returns whether the agent stopped
/// on a fatal error (one retrying won't fix, such as a rejected token).
async fn run(
    mut args: ServerEndpoint,
    shutdown: impl Future<Output = ()>,
    restart_capability: systemd::RestartCapability,
) -> bool {
    args.url = normalize_ws_url(&args.url);
    borg::locate_bundled_borg().await;

    let (exec_cmd_tx, exec_cmd_rx) = mpsc::channel::<ExecutorCommand>(16);
    let (outbound_tx, outbound_rx) = mpsc::channel::<AgentToServer>(64);

    let task_registry = TaskRegistry::default();
    let executor = Executor::new(args.clone(), task_registry.clone());

    let executor_handle = tokio::spawn(async move {
        executor.run(exec_cmd_rx, outbound_tx).await;
    });

    // Either way this select resolves, the ws_client future (which owns exec_cmd_tx) is
    // gone once it returns below - completed on its own, or dropped as the losing branch -
    // closing the channel executor.run() is reading. That's the standard tokio idiom for
    // signalling a spawned task to wind down.
    let ended = tokio::select! {
        result = ws::run_ws_client(&args, exec_cmd_tx, outbound_rx, &restart_capability) => {
            result.err()
        }
        () = shutdown => None,
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
    let outstanding = task_registry.shutdown(shutdown_deadline()).await;
    if outstanding > 0 {
        tracing::warn!(
            outstanding,
            "background tasks still running at shutdown deadline"
        );
    }
    // A restart exits only now, with borg stopped like on any shutdown: a
    // Windows service's processes outlive it, unlike a systemd unit's. The
    // service manager starts the agent again after this exit, which neither
    // a stop request nor a fatal error is.
    if matches!(ended, Some(ws::WsError::RestartRequested)) {
        tracing::info!("restarting: exiting for the service manager to start the agent again");
        std::process::exit(0);
    }
    ended.as_ref().is_some_and(ws::is_fatal)
}

/// How long shutdown waits for background work, borg's own escalation included.
fn shutdown_deadline() -> Duration {
    shared::borg::kill_escalation_delay().saturating_add(SHUTDOWN_GRACE_BUFFER)
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn a_server_url_becomes_a_websocket_url() {
        assert_eq!(
            normalize_ws_url("wss://backup.example"),
            "wss://backup.example"
        );
        assert_eq!(
            normalize_ws_url("ws://backup.example"),
            "ws://backup.example"
        );
        assert_eq!(
            normalize_ws_url("https://backup.example"),
            "wss://backup.example"
        );
        assert_eq!(
            normalize_ws_url("http://backup.example"),
            "ws://backup.example"
        );
        assert_eq!(
            normalize_ws_url("backup.example:8080"),
            "ws://backup.example:8080"
        );
    }

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn a_command_line_with_server_token_and_key_connects_with_them() {
        let cli = Cli::try_parse_from([
            "agent",
            "--server-url",
            "https://backup.example",
            "--server-key",
            KEY,
        ])
        .unwrap();

        let args = cli.connect_args(Some("secret".to_owned())).unwrap();

        assert_eq!(args.url, "https://backup.example");
        assert_eq!(args.token, "secret");
        assert_eq!(args.key.to_string(), KEY);
    }

    #[test]
    fn a_malformed_server_key_is_refused() {
        let parsed = Cli::try_parse_from([
            "agent",
            "--server-url",
            "https://backup.example",
            "--server-key",
            "not-a-key",
        ]);
        assert!(parsed.is_err());
    }

    /// The token is a secret, and a command line is readable by every local
    /// user, so there is no flag for it.
    #[test]
    fn the_token_cannot_be_given_on_the_command_line() {
        let parsed = Cli::try_parse_from([
            "agent",
            "--server-url",
            "https://backup.example",
            "--token",
            "secret",
            "--server-key",
            KEY,
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn without_a_server_token_or_key_there_is_nothing_to_connect_to() {
        let key: ChannelPublicKey = KEY.parse().unwrap();
        let cli = |server_url: Option<&str>, server_key| Cli {
            server_url: server_url.map(str::to_owned),
            server_key,
            #[cfg(windows)]
            command: None,
        };
        let secret = || Some("secret".to_owned());

        assert!(cli(None, Some(key)).connect_args(secret()).is_none());
        // Clap itself refuses a command line with none of them.
        assert!(Cli::try_parse_from(["agent"]).is_err());
        assert!(
            cli(Some("https://backup.example"), Some(key))
                .connect_args(None)
                .is_none()
        );
        assert!(
            cli(Some("https://backup.example"), None)
                .connect_args(secret())
                .is_none()
        );
        // An agent from before the encrypted channel, started with only a
        // server, is refused rather than connecting in the clear.
        assert!(Cli::try_parse_from(["agent", "--server-url", "https://backup.example"]).is_err());
    }

    /// `service` is the documented way to set the agent up on Windows, so its
    /// command lines must parse without the connect flags the agent otherwise
    /// requires.
    #[cfg(windows)]
    #[test]
    fn service_command_lines_parse_without_server_or_token() {
        use service::{Action, Command};

        let run = Cli::try_parse_from(["agent", "service", "run"]).unwrap();
        assert!(matches!(
            run.command,
            Some(Command::Service {
                action: Action::Run
            })
        ));
        let uninstall = Cli::try_parse_from(["agent", "service", "uninstall"]).unwrap();
        assert!(matches!(
            uninstall.command,
            Some(Command::Service {
                action: Action::Uninstall
            })
        ));
        let install = Cli::try_parse_from([
            "agent",
            "service",
            "install",
            "--server-url",
            "https://backup.example",
            "--server-key",
            KEY,
        ])
        .unwrap();
        assert!(matches!(
            install.command,
            Some(Command::Service {
                action: Action::Install { server_url, server_key },
            }) if server_url == "https://backup.example" && server_key.to_string() == KEY
        ));
        // As for the agent itself, the token is never a flag.
        assert!(
            Cli::try_parse_from([
                "agent",
                "service",
                "install",
                "--server-url",
                "https://backup.example",
                "--token",
                "secret",
                "--server-key",
                KEY,
            ])
            .is_err()
        );
    }
}
