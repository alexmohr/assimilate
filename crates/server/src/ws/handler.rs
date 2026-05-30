// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use shared::protocol::{AgentToServer, ServerToAgent, ServerToUi};
use tokio::sync::mpsc;

use crate::{
    AppState, config_assembler, db,
    notifications::{self, EventType, NotificationEvent},
};

const PING_INTERVAL: Duration = Duration::from_secs(30);
const CHANNEL_BUFFER: usize = 32;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    let hello = match ws_stream.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<AgentToServer>(text.as_str())
        {
            Ok(AgentToServer::Hello {
                hostname,
                token,
                agent_version,
                supports_restart,
                restart_unavailable_reason,
            }) => Some((
                hostname,
                token,
                agent_version,
                supports_restart,
                restart_unavailable_reason,
            )),
            Ok(_) | Err(_) => None,
        },
        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => None,
        Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => None,
    };

    let Some((hostname, token, agent_version, supports_restart, restart_unavailable_reason)) =
        hello
    else {
        let close = Message::Close(Some(CloseFrame {
            code: 4001,
            reason: "expected Hello message".into(),
        }));
        if let Err(e) = ws_sink.send(close).await {
            tracing::debug!(error = %e, "ws send failed");
        }
        return;
    };

    tracing::info!(
        hostname = %hostname,
        agent_version = %agent_version,
        "agent attempting connection"
    );

    let (client_id, token_hash) = match db::get_client_token_hash(&state.pool, &hostname).await {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "unknown client attempted connection");
            if let Err(e) = db::insert_system_event(
                &state.pool,
                "auth_failed",
                Some(&hostname),
                &format!("Unknown client '{hostname}' attempted connection"),
            )
            .await
            {
                tracing::error!(error = %e, "failed to insert system event");
            }
            let close = Message::Close(Some(CloseFrame {
                code: 4001,
                reason: "authentication failed".into(),
            }));
            if let Err(e) = ws_sink.send(close).await {
                tracing::debug!(error = %e, "ws send failed");
            }
            return;
        }
    };

    let token_valid = match bcrypt::verify(&token, &token_hash) {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!(hostname = %hostname, error = %e, "bcrypt verification failed");
            let close = Message::Close(Some(CloseFrame {
                code: 4001,
                reason: "authentication failed".into(),
            }));
            if let Err(e) = ws_sink.send(close).await {
                tracing::debug!(error = %e, "ws send failed");
            }
            return;
        }
    };

    if !token_valid {
        tracing::warn!(hostname = %hostname, "invalid agent token");
        if let Err(e) = db::insert_system_event(
            &state.pool,
            "auth_failed",
            Some(&hostname),
            &format!("Invalid token for client '{hostname}'"),
        )
        .await
        {
            tracing::error!(error = %e, "failed to insert system event");
        }
        let close = Message::Close(Some(CloseFrame {
            code: 4001,
            reason: "authentication failed".into(),
        }));
        if let Err(e) = ws_sink.send(close).await {
            tracing::debug!(error = %e, "ws send failed");
        }
        return;
    }

    if let Err(e) = db::update_last_seen_and_version(&state.pool, client_id, &agent_version).await {
        tracing::error!(hostname = %hostname, error = %e, "failed to update last_seen_at");
    }

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<ServerToAgent>(CHANNEL_BUFFER);
    let ping_tx = outbound_tx.clone();
    state
        .registry
        .register(
            hostname.clone(),
            outbound_tx,
            supports_restart,
            restart_unavailable_reason,
        )
        .await;

    tracing::info!(hostname = %hostname, "agent connected");

    state.ui_broadcast.send(ServerToUi::AgentConnected {
        hostname: hostname.clone(),
    });

    match config_assembler::assemble_config(&state.pool, &state.encryption_key, &hostname).await {
        Ok(config) => {
            let config_msg = ServerToAgent::ConfigUpdate(config);
            if let Ok(json) = serde_json::to_string(&config_msg)
                && let Err(e) = ws_sink.send(Message::Text(json.into())).await
            {
                tracing::debug!(error = %e, "ws send failed");
            }
        }
        Err(e) => {
            tracing::error!(hostname = %hostname, error = %e, "failed to assemble config on connect");
        }
    }

    tokio::spawn(ping_loop(ping_tx));

    loop {
        tokio::select! {
            outbound = outbound_rx.recv() => {
                let Some(msg) = outbound else { break };
                let Ok(json) = serde_json::to_string(&msg) else { continue };
                if ws_sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            inbound = ws_stream.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        handle_agent_message(text.as_str(), &hostname, client_id, &state).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        tracing::warn!(hostname = %hostname, error = %e, "ws read error");
                        break;
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                }
            }
        }
    }

    state.registry.unregister(&hostname).await;
    state.ui_broadcast.send(ServerToUi::AgentDisconnected {
        hostname: hostname.clone(),
    });
    tracing::info!(hostname = %hostname, "agent disconnected");
}

async fn ping_loop(sender: mpsc::Sender<ServerToAgent>) {
    let mut interval = tokio::time::interval(PING_INTERVAL);
    loop {
        interval.tick().await;
        if sender.send(ServerToAgent::Ping).await.is_err() {
            break;
        }
    }
}

async fn handle_agent_message(text: &str, hostname: &str, client_id: i64, state: &AppState) {
    let msg = match serde_json::from_str::<AgentToServer>(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "invalid message from agent");
            return;
        }
    };

    match msg {
        AgentToServer::Pong => {
            if let Err(e) = db::update_last_seen_by_hostname(&state.pool, hostname).await {
                tracing::error!(hostname = %hostname, error = %e, "failed to update last_seen_at");
            }
        }
        AgentToServer::BackupStarted {
            repo_id,
            started_at,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                started_at = %started_at,
                "backup started"
            );
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::BackupCompleted { report } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?report.repo_id,
                status = ?report.status,
                "backup completed"
            );
            let status = match report.status {
                shared::types::BackupStatus::Success => "success",
                shared::types::BackupStatus::Warning => "warning",
                shared::types::BackupStatus::Failed => "failed",
            };
            let notification_error_message = report.error_message.clone();
            let params = db::InsertReportParams {
                client_id,
                repo_id: report.repo_id.0,
                started_at: report.started_at,
                finished_at: report.finished_at,
                status: status.to_string(),
                original_size: report.original_size,
                compressed_size: report.compressed_size,
                deduplicated_size: report.deduplicated_size,
                files_processed: report.files_processed,
                duration_secs: report.duration_secs,
                error_message: report.error_message,
                warnings: report.warnings,
                borg_version: report.borg_version,
            };
            if let Err(e) = db::insert_backup_report(&state.pool, &params).await {
                tracing::error!(hostname = %hostname, error = %e, "failed to persist backup report");
            }

            let event_type = match report.status {
                shared::types::BackupStatus::Success => EventType::BackupSuccess,
                shared::types::BackupStatus::Warning => EventType::BackupWarning,
                shared::types::BackupStatus::Failed => EventType::BackupFailed,
            };
            let event = NotificationEvent {
                event_type,
                hostname: hostname.to_owned(),
                repo_name: report.repo_id.0.to_string(),
                status: status.to_string(),
                error_message: notification_error_message,
                timestamp: chrono::Utc::now(),
                repo_id: Some(report.repo_id.0),
                client_id: Some(client_id),
                schedule_id: None,
            };
            let service = state.notification_service.clone();
            tokio::spawn(async move {
                if let Err(e) = notifications::dispatch(&service, event).await {
                    tracing::error!(error = %e, "notification dispatch failed");
                }
            });

            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::StatusUpdate { repo_id, status } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                status = ?status,
                "status update"
            );
        }
        AgentToServer::BackupRejected { repo_id, reason } => {
            tracing::warn!(
                hostname = %hostname,
                repo_id = ?repo_id,
                reason = %reason,
                "backup rejected by agent"
            );
        }
        AgentToServer::CheckCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                success,
                duration_secs,
                "check completed"
            );
            if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
                state.ui_broadcast.send(ServerToUi::CheckCompleted {
                    hostname: hostname.to_owned(),
                    target_name,
                    success,
                    error_message: error_message.clone(),
                });
            }

            let event_type = if success {
                EventType::CheckSuccess
            } else {
                EventType::CheckFailed
            };
            let event = NotificationEvent {
                event_type,
                hostname: hostname.to_owned(),
                repo_name: repo_id.0.to_string(),
                status: if success { "success" } else { "failed" }.to_owned(),
                error_message,
                timestamp: chrono::Utc::now(),
                repo_id: Some(repo_id.0),
                client_id: Some(client_id),
                schedule_id: None,
            };
            let service = state.notification_service.clone();
            tokio::spawn(async move {
                if let Err(e) = notifications::dispatch(&service, event).await {
                    tracing::error!(error = %e, "notification dispatch failed");
                }
            });

            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::VerifyCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
            files_verified,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                success,
                duration_secs,
                files_verified,
                "verify completed"
            );
            if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
                state.ui_broadcast.send(ServerToUi::VerifyCompleted {
                    hostname: hostname.to_owned(),
                    target_name,
                    success,
                    error_message,
                });
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::CanaryVerified {
            repo_id,
            success,
            nonce,
            archive_name,
            error_message,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                success,
                "canary verification completed"
            );
            let schedule_id =
                db::get_backup_schedule_for_hostname_repo(&state.pool, hostname, repo_id.0)
                    .await
                    .map_err(|e| {
                        tracing::warn!(
                            hostname = %hostname,
                            repo_id = ?repo_id,
                            error = %e,
                            "failed to look up schedule for canary result"
                        );
                    })
                    .ok()
                    .flatten()
                    .map(|s| s.id);

            if let Some(sid) = schedule_id {
                if let Err(e) = db::insert_canary_result(
                    &state.pool,
                    sid,
                    success,
                    &nonce,
                    error_message.as_deref(),
                    if archive_name.is_empty() {
                        None
                    } else {
                        Some(&archive_name)
                    },
                )
                .await
                {
                    tracing::error!(error = %e, "failed to insert canary result");
                }
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    repo_id = ?repo_id,
                    "no backup schedule found for canary result, skipping insert"
                );
            }
            if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
                state.ui_broadcast.send(ServerToUi::CanaryVerified {
                    hostname: hostname.to_owned(),
                    target_name,
                    success,
                    error_message,
                });
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::Hello { .. } => {
            tracing::warn!(hostname = %hostname, "unexpected Hello after handshake");
        }
        AgentToServer::RestartFailed { error_message } => {
            tracing::error!(
                hostname = %hostname,
                error = %error_message,
                "agent restart failed"
            );
        }
        AgentToServer::InitRepoCompleted {
            repo_path,
            success,
            error_message,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_path = %repo_path,
                success,
                error_message = ?error_message,
                "init repo completed"
            );
        }
    }
}
