// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use futures_util::{Sink, SinkExt, StreamExt};
use shared::{
    agent_channel::ws::{
        DecryptedStream, EncryptedSink, TungsteniteCodec, WsChannelError, initiate,
    },
    protocol::{AgentToServer, ServerToAgent},
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message, protocol::frame::coding::CloseCode};
use tracing::{error, info, warn};

use self::inbound::{Inbound, UnrecognisedMessage};
use crate::{
    ServerEndpoint,
    executor::{ExecutorCommand, LiveFileRequest, RestoreDatabaseRequest, ScanDatabasesRequest},
    systemd::RestartCapability,
};

mod inbound;

const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_mins(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLOSE_CODE_AUTH_FAILED: u16 = 4001;

/// The error the encrypted channel's sink and stream report.
pub type ChannelFailure = WsChannelError<tungstenite::Error>;

pub async fn run_ws_client(
    args: &ServerEndpoint,
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
            Err(WsError::RestartRequested) => return Err(WsError::RestartRequested),
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
    args: &ServerEndpoint,
    exec_cmd_tx: &mpsc::Sender<ExecutorCommand>,
    outbound_rx: &mut mpsc::Receiver<AgentToServer>,
    restart_capability: &RestartCapability,
    instance_id: &str,
) -> Result<(), WsError> {
    let url = format!("{}/ws/agent", args.url.trim_end_matches('/'));
    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| WsError::Connect(Box::new(e)))?;

    info!("Connected to {url}");

    let (mut sink, mut stream) = ws_stream.split();
    let (sealer, opener) = initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &args.key)
        .await
        .map_err(|e| WsError::Handshake(Box::new(e)))?;
    info!(
        server_key = %args.key.fingerprint(),
        "Encrypted channel established"
    );
    let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
    let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);

    let hello = hello_message(&args.token, restart_capability, instance_id);
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

/// The `Hello` the agent opens every connection with, inside the encrypted
/// channel.
fn hello_message(
    token: &str,
    restart_capability: &RestartCapability,
    instance_id: &str,
) -> AgentToServer {
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

    AgentToServer::Hello {
        hostname,
        token: token.to_owned(),
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
    S: Sink<Message, Error = ChannelFailure> + Unpin,
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
        ServerToAgent::ScanDatabases {
            request_id,
            runtimes,
        } => {
            info!("Received ScanDatabases");
            if exec_cmd_tx
                .send(ExecutorCommand::ScanDatabases(ScanDatabasesRequest {
                    request_id,
                    runtimes,
                }))
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
        ServerToAgent::ReadLiveFile { request_id, path } => {
            info!("Received ReadLiveFile");
            if exec_cmd_tx
                .send(ExecutorCommand::ReadLiveFile(LiveFileRequest {
                    request_id,
                    path,
                }))
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
            info!("Received RestartAgent command, shutting down to be restarted");
            return Err(WsError::RestartRequested);
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
        ServerToAgent::RestoreDatabase {
            request_id,
            repo_id,
            archive_name,
            restore,
        } => {
            info!(
                "Received RestoreDatabase for repo {repo_id:?} archive {archive_name} source {}",
                restore.source
            );
            let request = RestoreDatabaseRequest {
                request_id,
                repo_id,
                archive_name,
                restore,
            };
            if exec_cmd_tx
                .send(ExecutorCommand::RestoreDatabase(request))
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
    S: Sink<Message, Error = ChannelFailure> + Unpin,
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
    /// The server did not complete the encrypted channel handshake: it does
    /// not hold the key this agent pins, or predates the encrypted channel.
    #[error(
        "encrypted channel handshake failed: {0}; check that BORG_SERVER_KEY matches the agent \
         channel key shown in the server's settings, and that the server is not older than this \
         agent"
    )]
    Handshake(Box<ChannelFailure>),
    #[error("send failed: {0}")]
    Send(Box<ChannelFailure>),
    #[error("receive failed: {0}")]
    Receive(Box<ChannelFailure>),
    #[error("serialization failed: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialization failed: {0}")]
    Deserialize(serde_json::Error),
    #[error("authentication rejected: {0}")]
    AuthRejected(String),
    #[error("server is shutting down")]
    ServerShutdown,
    /// The server asked the agent to restart: it shuts down like on a stop
    /// request, then exits for its service manager to start it again.
    #[error("the server asked the agent to restart")]
    RestartRequested,
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
        type Error = ChannelFailure;

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
    async fn a_database_scan_request_reaches_the_executor() {
        let (exec_cmd_tx, mut exec_cmd_rx) = mpsc::channel(1);
        let mut sink = RecordingSink::default();
        let request = serde_json::to_string(&ServerToAgent::ScanDatabases {
            request_id: "req-db".to_owned(),
            runtimes: Some(Box::new(shared::database::DatabaseRuntimes {
                kubernetes: true,
                kubeconfig_path: Some("/etc/assimilate/kubeconfig".to_owned()),
                ..shared::database::DatabaseRuntimes::default()
            })),
        })
        .unwrap();

        let result = handle_text_message(&request, &exec_cmd_tx, &mut sink).await;

        assert!(result.is_ok());
        let request = crate::tests::messages::scan_databases_request(exec_cmd_rx.recv().await);
        assert_eq!(request.request_id, "req-db");
        let runtimes = request.runtimes.unwrap();
        assert!(runtimes.kubernetes);
        assert_eq!(
            runtimes.kubeconfig_path.as_deref(),
            Some("/etc/assimilate/kubeconfig")
        );
        assert_eq!(sink.0.len(), 0, "the executor answers, not the socket loop");
    }

    #[tokio::test]
    async fn a_database_restore_request_reaches_the_executor() {
        let (exec_cmd_tx, mut exec_cmd_rx) = mpsc::channel(1);
        let mut sink = RecordingSink::default();
        let restore = shared::database::DatabaseRestore {
            source: shared::database::DatabaseSourceId {
                runtime: shared::database::DatabaseRuntime::Kubernetes,
                namespace: "shop".to_owned(),
                name: "db".to_owned(),
            },
            database: "orders".to_owned(),
            archive_path: "var/lib/assimilate/database-dumps/schedule-1/orders.pgdump".to_owned(),
            format: shared::database::DumpFormat::PgCustom,
            target: shared::database::DatabaseRestoreTarget::Original,
        };
        let request = serde_json::to_string(&ServerToAgent::RestoreDatabase {
            request_id: "req-restore".to_owned(),
            repo_id: shared::types::RepoId(3),
            archive_name: "host-2026-10-16".to_owned(),
            restore: restore.clone(),
        })
        .unwrap();

        let result = handle_text_message(&request, &exec_cmd_tx, &mut sink).await;

        assert!(result.is_ok());
        assert!(matches!(
            exec_cmd_rx.recv().await,
            Some(ExecutorCommand::RestoreDatabase(request))
                if request.request_id == "req-restore"
                    && request.repo_id == shared::types::RepoId(3)
                    && request.archive_name == "host-2026-10-16"
                    && request.restore == restore
        ));
        assert_eq!(sink.0.len(), 0, "the executor answers, not the socket loop");

        drop(exec_cmd_rx);
        let (result, logs) =
            crate::tests::logs::logs_while(handle_text_message(&request, &exec_cmd_tx, &mut sink))
                .await;
        assert!(result.is_ok());
        assert!(logs.contains("Executor command channel closed"), "{logs}");
    }

    #[tokio::test]
    async fn a_live_file_request_reaches_the_executor() {
        let (exec_cmd_tx, mut exec_cmd_rx) = mpsc::channel(1);
        let mut sink = RecordingSink::default();
        let request = serde_json::to_string(&ServerToAgent::ReadLiveFile {
            request_id: "req-live".to_owned(),
            path: "etc/nginx/nginx.conf".to_owned(),
        })
        .unwrap();

        let result = handle_text_message(&request, &exec_cmd_tx, &mut sink).await;

        assert!(result.is_ok());
        assert!(matches!(
            exec_cmd_rx.recv().await,
            Some(ExecutorCommand::ReadLiveFile(LiveFileRequest { request_id, path }))
                if request_id == "req-live" && path == "etc/nginx/nginx.conf"
        ));
        assert_eq!(sink.0.len(), 0, "the executor answers, not the socket loop");
    }

    #[tokio::test]
    async fn a_database_scan_request_without_an_executor_is_logged() {
        let (exec_cmd_tx, exec_cmd_rx) = mpsc::channel(1);
        drop(exec_cmd_rx);
        let mut sink = RecordingSink::default();
        let request = serde_json::to_string(&ServerToAgent::ScanDatabases {
            request_id: "req-db".to_owned(),
            runtimes: None,
        })
        .unwrap();

        let (result, logs) =
            crate::tests::logs::logs_while(handle_text_message(&request, &exec_cmd_tx, &mut sink))
                .await;

        assert!(result.is_ok());
        assert!(logs.contains("Executor command channel closed"), "{logs}");
    }

    #[tokio::test]
    async fn a_restart_request_ends_the_connection_to_restart() {
        let (result, sent) = handle(r#"{"type":"RestartAgent"}"#).await;
        assert!(matches!(result, Err(WsError::RestartRequested)));
        assert!(!is_fatal(&WsError::RestartRequested));
        assert_eq!(sent, Vec::<Message>::new());
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
        assert!(!is_fatal(&WsError::Send(Box::new(
            WsChannelError::Transport(proto_err)
        ))));
    }

    /// A server holding another key may be a misconfiguration that gets
    /// fixed, so the agent keeps retrying rather than exiting.
    #[test]
    fn a_failed_handshake_is_not_fatal_and_names_the_setting() {
        let err = WsError::Handshake(Box::new(WsChannelError::Channel(
            shared::agent_channel::ChannelError::Handshake,
        )));
        assert!(!is_fatal(&err));
        assert!(err.to_string().contains("BORG_SERVER_KEY"), "{err}");
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
    /// Accepts one connection on `listener`, completes the server side of
    /// the encrypted handshake, and returns the decrypted channel halves.
    async fn accept_encrypted(
        listener: &tokio::net::TcpListener,
    ) -> (
        EncryptedSink<
            TungsteniteCodec,
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
                Message,
            >,
        >,
        DecryptedStream<
            TungsteniteCodec,
            futures_util::stream::SplitStream<
                tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
            >,
        >,
    ) {
        let (tcp, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let (mut sink, mut stream) = ws.split();
        let key = crate::endpoint::test_support::server_key();
        let (sealer, opener) = shared::agent_channel::ws::accept::<TungsteniteCodec, _, _, _>(
            &mut sink,
            &mut stream,
            &key,
        )
        .await
        .unwrap();
        (
            EncryptedSink::new(sink, sealer),
            DecryptedStream::new(stream, opener),
        )
    }

    /// The server tells a reconnect from a restart by the instance id in the
    /// agent's Hello, so it must stay the same for as long as the process
    /// runs.
    #[tokio::test]
    async fn a_reconnect_names_the_same_instance_in_its_hello() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let args = crate::endpoint::test_support::endpoint(&format!(
            "ws://{}",
            listener.local_addr().unwrap()
        ));
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
                let (_sink, mut stream) = accept_encrypted(&listener).await;
                let text = stream
                    .next()
                    .await
                    .expect("a Hello")
                    .unwrap()
                    .into_text()
                    .unwrap();
                let hello: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(hello.pointer("/type"), Some(&serde_json::json!("Hello")));
                assert_eq!(
                    hello.pointer("/payload/token"),
                    Some(&serde_json::json!("token"))
                );
                *slot = hello
                    .pointer("/payload/instance_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            instance_ids
        };

        let client = run_ws_client(&args, exec_cmd_tx, outbound_rx, &capability);
        let ids = tokio::select! { ids = server => Some(ids), _ = client => None };
        let [first, second] = ids.expect("the agent stopped before it reconnected");

        assert!(first.is_some());
        assert_eq!(first, second);
    }

    /// Messages in both directions go through the encrypted channel: the
    /// server's Ping arrives and the agent's Pong comes back.
    #[tokio::test]
    async fn the_agent_answers_over_the_encrypted_channel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let args = crate::endpoint::test_support::endpoint(&format!(
            "ws://{}",
            listener.local_addr().unwrap()
        ));
        let (exec_cmd_tx, _exec_cmd_rx) = mpsc::channel(1);
        let (_outbound_tx, outbound_rx) = mpsc::channel(1);
        let capability = RestartCapability {
            supported: false,
            unavailable_reason: None,
        };

        let server = async {
            let (mut sink, mut stream) = accept_encrypted(&listener).await;
            let _hello = stream.next().await.unwrap().unwrap();
            let ping = serde_json::to_string(&ServerToAgent::Ping).unwrap();
            sink.send(Message::Text(ping.into())).await.unwrap();
            stream.next().await.unwrap().unwrap()
        };

        let client = run_ws_client(&args, exec_cmd_tx, outbound_rx, &capability);
        let reply = tokio::select! { reply = server => Some(reply), _ = client => None };

        assert_eq!(reply, Some(text_frame(&AgentToServer::Pong)));
    }

    /// An agent pinned to one key never sends its token to a server that
    /// cannot prove it holds that key.
    #[tokio::test]
    async fn a_server_with_another_key_never_sees_the_hello() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut args = crate::endpoint::test_support::endpoint(&format!(
            "ws://{}",
            listener.local_addr().unwrap()
        ));
        args.key = shared::agent_channel::ChannelPrivateKey::derive(b"some other server")
            .unwrap()
            .public_key()
            .unwrap();
        let (exec_cmd_tx, _exec_cmd_rx) = mpsc::channel(1);
        let (_outbound_tx, mut outbound_rx) = mpsc::channel(1);
        let capability = RestartCapability {
            supported: false,
            unavailable_reason: None,
        };

        let server = async {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let first = ws.next().await.unwrap().unwrap();
            let key = crate::endpoint::test_support::server_key();
            let refused = shared::agent_channel::respond(&key, &first.into_data()).is_err();
            // It answers with a handshake of its own making instead.
            let (_, own_first) =
                shared::agent_channel::Initiator::start(&key.public_key().unwrap()).unwrap();
            let (_, reply) = shared::agent_channel::respond(&key, &own_first).unwrap();
            ws.send(Message::Binary(reply.into())).await.unwrap();
            // Whatever else arrives before the agent gives up, none of it
            // may be a message.
            let rest: Vec<Message> = ws.filter_map(|m| async { m.ok() }).collect().await;
            (refused, rest)
        };

        let client = connect_and_run(
            &args,
            &exec_cmd_tx,
            &mut outbound_rx,
            &capability,
            "instance",
        );
        let (result, (refused, rest)) = tokio::join!(client, server);

        assert!(refused);
        assert!(matches!(result, Err(WsError::Handshake(_))), "{result:?}");
        assert!(
            rest.iter()
                .all(|m| !matches!(m, Message::Text(_) | Message::Binary(_))),
            "{rest:?}"
        );
    }

    /// A server from before the encrypted channel reads the handshake as a
    /// malformed Hello and closes; the agent reports that as a failed
    /// handshake and sends nothing more.
    #[tokio::test]
    async fn a_server_without_the_encrypted_channel_fails_the_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let args = crate::endpoint::test_support::endpoint(&format!(
            "ws://{}",
            listener.local_addr().unwrap()
        ));
        let (exec_cmd_tx, _exec_cmd_rx) = mpsc::channel(1);
        let (_outbound_tx, mut outbound_rx) = mpsc::channel(1);
        let capability = RestartCapability {
            supported: false,
            unavailable_reason: None,
        };

        let server = async {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let first = ws.next().await.unwrap().unwrap();
            ws.close(Some(tungstenite::protocol::CloseFrame {
                code: CloseCode::Policy,
                reason: "expected Hello message".into(),
            }))
            .await
            .unwrap();
            first.is_binary()
        };

        let client = connect_and_run(
            &args,
            &exec_cmd_tx,
            &mut outbound_rx,
            &capability,
            "instance",
        );
        let (result, first_was_binary) = tokio::join!(client, server);

        assert!(first_was_binary);
        assert!(
            matches!(&result, Err(WsError::Handshake(e)) if matches!(**e, WsChannelError::Closed)),
            "{result:?}"
        );
    }
}
