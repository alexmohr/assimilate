// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{process, time::Duration};

use futures_util::{Sink, SinkExt, StreamExt};
use shared::protocol::{AgentToServer, ServerToAgent};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message, protocol::frame::coding::CloseCode};
use tracing::{error, info, warn};

use self::inbound::{Inbound, UnrecognisedMessage};
use crate::{Args, executor::ExecutorCommand, systemd::RestartCapability};

mod inbound;

const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_mins(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLOSE_CODE_AUTH_FAILED: u16 = 4001;

pub async fn run_ws_client(
    args: &Args,
    exec_cmd_tx: mpsc::Sender<ExecutorCommand>,
    mut outbound_rx: mpsc::Receiver<AgentToServer>,
    restart_capability: &RestartCapability,
) -> Result<(), WsError> {
    let mut backoff = BACKOFF_BASE;
    // Identifies this process to the server across reconnects, so it can
    // tell a reconnect, after which answers to operations it handed over
    // still arrive (they wait in `outbound_rx`), from a restart, which lost
    // them.
    let instance_id = uuid::Uuid::new_v4().to_string();

    loop {
        match connect_and_run(
            args,
            &exec_cmd_tx,
            &mut outbound_rx,
            restart_capability,
            &instance_id,
        )
        .await
        {
            Ok(()) => {
                info!("WebSocket connection closed gracefully");
            }
            Err(WsError::AuthRejected(reason)) => {
                error!("Authentication rejected by server: {reason}");
                return Err(WsError::AuthRejected(reason));
            }
            Err(WsError::ServerShutdown) => {
                info!("Server shutting down, reconnecting in 60s");
                tokio::time::sleep(Duration::from_mins(1)).await;
                backoff = BACKOFF_BASE;
                continue;
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
    instance_id: &str,
) -> Result<(), WsError> {
    let url = format!("{}/ws/agent", args.server_url.trim_end_matches('/'));
    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| WsError::Connect(Box::new(e)))?;

    info!("Connected to {url}");

    let (mut sink, mut stream) = ws_stream.split();

    let hostname = std::env::var("BORG_HOSTNAME")
        .unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().into_owned());

    let version = env!("APP_VERSION");
    let git_sha = env!("GIT_SHA");
    let build_timestamp = env!("BUILD_TIMESTAMP");
    let commit_count_str = env!("GIT_COMMIT_COUNT");
    let agent_version = if git_sha.is_empty() {
        version.to_owned()
    } else {
        format!("{version}+{git_sha}")
    };
    let agent_commit_count = commit_count_str.parse::<u32>().ok().filter(|&n| n > 0);

    let hello = AgentToServer::Hello {
        hostname,
        token: args.token.clone(),
        agent_version,
        agent_git_sha: if git_sha.is_empty() {
            None
        } else {
            Some(git_sha.to_owned())
        },
        agent_build_time: if build_timestamp.is_empty() {
            None
        } else {
            Some(build_timestamp.to_owned())
        },
        agent_commit_count,
        supports_restart: restart_capability.supported,
        restart_unavailable_reason: restart_capability.unavailable_reason.clone(),
        instance_id: Some(instance_id.to_owned()),
    };

    let hello_json = serde_json::to_string(&hello).map_err(WsError::Serialize)?;
    sink.send(Message::Text(hello_json.into()))
        .await
        .map_err(|e| WsError::Send(Box::new(e)))?;

    info!("Sent Hello message");

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let pong = serde_json::to_string(&AgentToServer::Pong).map_err(WsError::Serialize)?;
                sink.send(Message::Text(pong.into()))
                    .await
                    .map_err(|e| WsError::Send(Box::new(e)))?;
            }
            Some(msg) = outbound_rx.recv() => {
                let json = serde_json::to_string(&msg).map_err(WsError::Serialize)?;
                sink.send(Message::Text(json.into()))
                    .await
                    .map_err(|e| WsError::Send(Box::new(e)))?;
            }
            inbound = stream.next() => {
                let Some(msg_result) = inbound else {
                    return Ok(());
                };
                let msg = msg_result.map_err(|e| WsError::Receive(Box::new(e)))?;
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
                        sink.send(Message::Pong(data))
                            .await
                            .map_err(|e| WsError::Send(Box::new(e)))?;
                    }
                    Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "message dispatch match; split tracked in #116"
)]
async fn handle_text_message<S>(
    text: &str,
    exec_cmd_tx: &mpsc::Sender<ExecutorCommand>,
    sink: &mut S,
) -> Result<(), WsError>
where
    S: Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let server_msg = match inbound::decode(text) {
        Ok(Inbound::Known(msg)) => msg,
        Ok(Inbound::Unrecognised(msg)) => return reject_unrecognised(msg, sink).await,
        Err(_) => {
            // The serde error can quote the payload, which may carry secrets.
            warn!("Ignoring a server message that is not a tagged JSON envelope");
            return Ok(());
        }
    };

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
        ServerToAgent::RunBackupNow {
            repo_id,
            schedule_id,
            run_id,
            ..
        } => {
            info!("Received RunBackupNow for repo {repo_id:?}");
            if exec_cmd_tx
                .send(ExecutorCommand::RunNow {
                    repo_id,
                    schedule_id,
                    run_id,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::ScanVms { request_id } => {
            info!("Received ScanVms");
            if exec_cmd_tx
                .send(ExecutorCommand::ScanVms { request_id })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::BuildVm {
            request_id,
            request,
        } => {
            info!("Received BuildVm for {}", request.name);
            if exec_cmd_tx
                .send(ExecutorCommand::BuildVm {
                    request_id,
                    request,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::StageVm { request_id, domain } => {
            info!("Received StageVm for {domain}");
            if exec_cmd_tx
                .send(ExecutorCommand::StageVm { request_id, domain })
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
                .map_err(|e| WsError::Send(Box::new(e)))?;
        }
        ServerToAgent::ShuttingDown => {
            info!("Server shutting down, disconnecting");
            return Err(WsError::ServerShutdown);
        }
        ServerToAgent::RestartAgent => {
            info!("Received RestartAgent command, exiting for systemd restart");
            process::exit(0);
        }
        ServerToAgent::SearchArchive { .. } => {
            warn!("SearchArchive not yet implemented in agent");
        }
        ServerToAgent::RestoreFiles {
            request_id,
            repo_id,
            archive_name,
            paths,
            target_path,
        } => {
            info!("Received RestoreFiles for repo {repo_id:?} archive {archive_name}");
            if exec_cmd_tx
                .send(ExecutorCommand::RestoreFiles {
                    repo_id,
                    archive_name,
                    paths,
                    target_path,
                    request_id,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
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
        ServerToAgent::DeleteArchives {
            request_id,
            repo_id,
            archive_names,
        } => {
            if exec_cmd_tx
                .send(ExecutorCommand::DeleteArchives {
                    repo_id,
                    archive_names,
                    request_id,
                })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
        ServerToAgent::CancelBackup { repo_id } => {
            info!("Received CancelBackup for repo {repo_id:?}");
            if exec_cmd_tx
                .send(ExecutorCommand::CancelBackup { repo_id })
                .await
                .is_err()
            {
                warn!("Executor command channel closed");
            }
        }
    }

    Ok(())
}

/// Logs a message this agent cannot handle and keeps the connection open, so
/// a newer server does not make an older agent reconnect in a loop. When the
/// message carries a request id the server is told, so the request fails
/// right away instead of timing out.
async fn reject_unrecognised<S>(msg: UnrecognisedMessage, sink: &mut S) -> Result<(), WsError>
where
    S: Sink<Message, Error = tungstenite::Error> + Unpin,
{
    warn!(
        message_type = %msg.message_type,
        "Ignoring a server message this agent cannot handle; update the agent"
    );
    let Some(request_id) = msg.request_id else {
        return Ok(());
    };
    let reply = AgentToServer::UnsupportedMessage {
        request_id,
        message_type: msg.message_type,
    };
    let json = serde_json::to_string(&reply).map_err(WsError::Serialize)?;
    sink.send(Message::Text(json.into()))
        .await
        .map_err(|e| WsError::Send(Box::new(e)))
}

#[derive(Debug, thiserror::Error)]
pub enum WsError {
    #[error("connection failed: {0}")]
    Connect(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("send failed: {0}")]
    Send(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("receive failed: {0}")]
    Receive(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("serialization failed: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialization failed: {0}")]
    Deserialize(serde_json::Error),
    #[error("authentication rejected: {0}")]
    AuthRejected(String),
    #[error("server is shutting down")]
    ServerShutdown,
}

/// Returns `true` for errors that should terminate the agent process rather
/// than trigger a reconnect. Authentication rejection is fatal because retrying
/// with the same credentials would loop indefinitely.
pub(crate) fn is_fatal(err: &WsError) -> bool {
    matches!(err, WsError::AuthRejected(_))
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use super::*;

    /// Keeps every frame sent to it, so a test can see what the agent replied.
    #[derive(Default)]
    struct RecordingSink(Vec<Message>);

    impl Sink<Message> for RecordingSink {
        type Error = tungstenite::Error;

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.get_mut().0.push(item);
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Feeds `text` to `handle_text_message` and returns its result together
    /// with every frame it sent back.
    async fn handle(text: &str) -> (Result<(), WsError>, Vec<Message>) {
        let (exec_cmd_tx, _exec_cmd_rx) = mpsc::channel(1);
        let mut sink = RecordingSink::default();
        let result = handle_text_message(text, &exec_cmd_tx, &mut sink).await;
        sink.close().await.unwrap();
        (result, sink.0)
    }

    fn text_frame(msg: &AgentToServer) -> Message {
        Message::Text(serde_json::to_string(msg).unwrap().into())
    }

    #[tokio::test]
    async fn ping_is_answered_with_pong() {
        let (result, sent) = handle(r#"{"type":"Ping"}"#).await;
        assert!(result.is_ok());
        assert_eq!(sent, vec![text_frame(&AgentToServer::Pong)]);
    }

    #[tokio::test]
    async fn unknown_request_is_answered_unsupported_and_keeps_the_connection() {
        let (result, sent) = handle(
            r#"{"type":"SomeFutureRequest","payload":{"request_id":"req-1","passphrase":"x"}}"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(
            sent,
            vec![text_frame(&AgentToServer::UnsupportedMessage {
                request_id: "req-1".into(),
                message_type: "SomeFutureRequest".into(),
            })]
        );
    }

    #[tokio::test]
    async fn unknown_notice_without_request_id_is_ignored() {
        let (result, sent) = handle(r#"{"type":"SomeFutureNotice"}"#).await;
        assert!(result.is_ok());
        assert_eq!(sent, Vec::new());
    }

    #[tokio::test]
    async fn text_that_is_not_an_envelope_is_ignored() {
        let (result, sent) = handle("not json").await;
        assert!(result.is_ok());
        assert_eq!(sent, Vec::new());
    }

    #[test]
    fn auth_rejected_is_fatal() {
        assert!(is_fatal(&WsError::AuthRejected("invalid token".into())));
    }

    #[test]
    fn serialization_errors_are_not_fatal() {
        let serde_err = serde_json::from_str::<AgentToServer>("not json").unwrap_err();
        assert!(!is_fatal(&WsError::Deserialize(serde_err)));
    }

    #[test]
    fn protocol_errors_are_not_fatal() {
        let proto_err = tokio_tungstenite::tungstenite::Error::ConnectionClosed;
        assert!(!is_fatal(&WsError::Send(Box::new(proto_err))));
    }

    #[test]
    fn auth_rejected_display_includes_reason() {
        let err = WsError::AuthRejected("token expired".into());
        assert_eq!(err.to_string(), "authentication rejected: token expired");
    }

    #[test]
    fn auth_rejected_debug_includes_variant() {
        let err = WsError::AuthRejected("nope".into());
        assert!(format!("{err:?}").contains("AuthRejected"));
    }

    #[test]
    fn server_shutdown_is_not_fatal() {
        assert!(!is_fatal(&WsError::ServerShutdown));
    }

    #[test]
    fn server_shutdown_display() {
        let err = WsError::ServerShutdown;
        assert_eq!(err.to_string(), "server is shutting down");
    }

    /// The server tells a reconnect from a restart by the instance id in the
    /// agent's Hello, so it must stay the same for as long as the process
    /// runs.
    #[tokio::test]
    async fn a_reconnect_names_the_same_instance_in_its_hello() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let args = Args {
            server_url: format!("ws://{}", listener.local_addr().unwrap()),
            token: "token".to_owned(),
        };
        let (exec_cmd_tx, _exec_cmd_rx) = mpsc::channel(1);
        let (_outbound_tx, outbound_rx) = mpsc::channel(1);
        let capability = RestartCapability {
            supported: false,
            unavailable_reason: None,
        };

        // Takes each connection's Hello, then drops the connection, so the
        // agent reconnects.
        let server = async {
            let mut instance_ids = [None, None];
            for slot in &mut instance_ids {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let Some(Ok(Message::Text(text))) = ws.next().await else {
                    panic!("the agent sent no Hello");
                };
                let AgentToServer::Hello { instance_id, .. } = serde_json::from_str(&text).unwrap()
                else {
                    panic!("the agent's first message is not a Hello: {text}");
                };
                *slot = instance_id;
            }
            instance_ids
        };

        let [first, second] = tokio::select! {
            ids = server => ids,
            result = run_ws_client(&args, exec_cmd_tx, outbound_rx, &capability) => {
                panic!("the agent stopped: {result:?}")
            }
        };

        assert!(first.is_some());
        assert_eq!(first, second);
    }
}
