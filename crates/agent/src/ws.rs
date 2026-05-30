// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{process, time::Duration};

use futures_util::{SinkExt, StreamExt};
use shared::protocol::{AgentToServer, ServerToAgent};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, protocol::frame::coding::CloseCode};
use tracing::{error, info, warn};

use crate::{Args, executor::ExecutorCommand, systemd::RestartCapability};

const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_mins(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLOSE_CODE_AUTH_FAILED: u16 = 4001;

pub async fn run_ws_client(
    args: &Args,
    exec_cmd_tx: mpsc::Sender<ExecutorCommand>,
    mut outbound_rx: mpsc::Receiver<AgentToServer>,
    restart_capability: &RestartCapability,
) {
    let mut backoff = BACKOFF_BASE;

    loop {
        match connect_and_run(args, &exec_cmd_tx, &mut outbound_rx, restart_capability).await {
            Ok(()) => {
                info!("WebSocket connection closed gracefully");
            }
            Err(WsError::AuthRejected(reason)) => {
                error!("Authentication rejected by server: {reason}");
                process::exit(1);
            }
            Err(e) => {
                error!("WebSocket connection error: {e}");
            }
        }

        info!("Reconnecting in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(BACKOFF_CAP);
    }
}

async fn connect_and_run(
    args: &Args,
    exec_cmd_tx: &mpsc::Sender<ExecutorCommand>,
    outbound_rx: &mut mpsc::Receiver<AgentToServer>,
    restart_capability: &RestartCapability,
) -> Result<(), WsError> {
    let url = format!("{}/ws/agent", args.server_url.trim_end_matches('/'));
    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(WsError::Connect)?;

    info!("Connected to {url}");

    let (mut sink, mut stream) = ws_stream.split();

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();

    let version = env!("CARGO_PKG_VERSION");
    let git_sha = env!("GIT_SHA");
    let agent_version = if git_sha.is_empty() {
        version.to_owned()
    } else {
        format!("{version}+{git_sha}")
    };

    let hello = AgentToServer::Hello {
        hostname,
        token: args.token.clone(),
        agent_version,
        supports_restart: restart_capability.supported,
        restart_unavailable_reason: restart_capability.unavailable_reason.clone(),
    };

    let hello_json = serde_json::to_string(&hello).map_err(WsError::Serialize)?;
    sink.send(Message::Text(hello_json.into()))
        .await
        .map_err(WsError::Send)?;

    info!("Sent Hello message");

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let pong = serde_json::to_string(&AgentToServer::Pong).map_err(WsError::Serialize)?;
                sink.send(Message::Text(pong.into())).await.map_err(WsError::Send)?;
            }
            Some(msg) = outbound_rx.recv() => {
                let json = serde_json::to_string(&msg).map_err(WsError::Serialize)?;
                sink.send(Message::Text(json.into())).await.map_err(WsError::Send)?;
            }
            inbound = stream.next() => {
                let Some(msg_result) = inbound else {
                    return Ok(());
                };
                let msg = msg_result.map_err(WsError::Receive)?;
                match msg {
                    Message::Text(text) => {
                        handle_text_message(
                            &text,
                            exec_cmd_tx,
                            &mut sink,
                        ).await?;
                    }
                    Message::Close(frame) => {
                        if let Some(ref f) = frame
                            && f.code == CloseCode::from(CLOSE_CODE_AUTH_FAILED)
                        {
                            return Err(WsError::AuthRejected(f.reason.to_string()));
                        }
                        info!("Received close frame");
                        return Ok(());
                    }
                    Message::Ping(data) => {
                        sink.send(Message::Pong(data)).await.map_err(WsError::Send)?;
                    }
                    Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn handle_text_message(
    text: &str,
    exec_cmd_tx: &mpsc::Sender<ExecutorCommand>,
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
) -> Result<(), WsError> {
    let server_msg: ServerToAgent = serde_json::from_str(text).map_err(WsError::Deserialize)?;

    match server_msg {
        ServerToAgent::ConfigUpdate(config) => {
            info!("Received config update");
            if exec_cmd_tx
                .send(ExecutorCommand::UpdateConfig(config))
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::RunBackupNow { repo_id, .. } => {
            info!("Received RunBackupNow for repo {repo_id:?}");
            if exec_cmd_tx
                .send(ExecutorCommand::RunNow { repo_id })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::RunCheckNow { repo_id, .. } => {
            info!("Received RunCheckNow for repo {repo_id:?}");
            if exec_cmd_tx
                .send(ExecutorCommand::RunCheckNow { repo_id })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::RunVerifyNow { repo_id, .. } => {
            info!("Received RunVerifyNow for repo {repo_id:?}");
            if exec_cmd_tx
                .send(ExecutorCommand::RunVerifyNow { repo_id })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::InitRepo {
            repo_path,
            ssh_user,
            ssh_host,
            ssh_port,
            passphrase,
            encryption,
            ..
        } => {
            info!("Received InitRepo for {repo_path}");
            if exec_cmd_tx
                .send(ExecutorCommand::InitRepo {
                    repo_path,
                    ssh_user,
                    ssh_host,
                    ssh_port,
                    passphrase,
                    encryption,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::Ping => {
            let pong = serde_json::to_string(&AgentToServer::Pong).map_err(WsError::Serialize)?;
            sink.send(Message::Text(pong.into()))
                .await
                .map_err(WsError::Send)?;
        }
        ServerToAgent::RestartAgent => {
            info!("Received RestartAgent command, exiting for systemd restart");
            process::exit(0);
        }
        ServerToAgent::SearchArchive { .. } => {
            warn!("SearchArchive not yet implemented in agent");
        }
        ServerToAgent::RestoreFiles { .. } => {
            warn!("RestoreFiles not yet implemented in agent");
        }
        ServerToAgent::DryRun {
            request_id,
            repo_id,
            schedule_id,
        } => {
            info!("Received DryRun for repo {repo_id:?} schedule {schedule_id}");
            if exec_cmd_tx
                .send(ExecutorCommand::DryRun {
                    repo_id,
                    schedule_id,
                    request_id,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::ExportArchive { .. } => {
            warn!("ExportArchive not yet implemented in agent");
        }
        ServerToAgent::KeyExport { .. } => {
            warn!("KeyExport not yet implemented in agent");
        }
        ServerToAgent::KeyImport { .. } => {
            warn!("KeyImport not yet implemented in agent");
        }
        ServerToAgent::ChangePassphrase { .. } => {
            warn!("ChangePassphrase not yet implemented in agent");
        }
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum WsError {
    #[error("connection failed: {0}")]
    Connect(tokio_tungstenite::tungstenite::Error),
    #[error("send failed: {0}")]
    Send(tokio_tungstenite::tungstenite::Error),
    #[error("receive failed: {0}")]
    Receive(tokio_tungstenite::tungstenite::Error),
    #[error("serialization failed: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialization failed: {0}")]
    Deserialize(serde_json::Error),
    #[error("authentication rejected: {0}")]
    AuthRejected(String),
}
