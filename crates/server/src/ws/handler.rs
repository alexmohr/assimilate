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
    AppState,
    api::repos::sync_new_archives,
    config_assembler, db,
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
                agent_git_sha,
                agent_build_time,
                supports_restart,
                restart_unavailable_reason,
            }) => Some((
                hostname,
                token,
                agent_version,
                agent_git_sha,
                agent_build_time,
                supports_restart,
                restart_unavailable_reason,
            )),
            Ok(_) | Err(_) => None,
        },
        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => None,
        Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => None,
    };

    let Some((
        hostname,
        token,
        agent_version,
        agent_git_sha,
        agent_build_time,
        supports_restart,
        restart_unavailable_reason,
    )) = hello
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

    if let Err(e) = db::update_last_seen_and_version(
        &state.pool,
        client_id,
        &agent_version,
        agent_git_sha.as_deref(),
        agent_build_time.as_deref(),
    )
    .await
    {
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
    match db::get_schedule_ids_for_hostname(&state.pool, &hostname).await {
        Ok(ids) => {
            let mut running = state.running_schedules.write().await;
            for id in ids {
                running.remove(&id);
            }
        }
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "failed to clear running schedules on disconnect");
        }
    }
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
            schedule_id,
            started_at,
            borg_command,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                started_at = %started_at,
                "backup started"
            );
            if let Err(e) = db::insert_backup_started(
                &state.pool,
                client_id,
                repo_id.0,
                schedule_id,
                started_at,
                borg_command.as_deref(),
            )
            .await
            {
                tracing::error!(hostname = %hostname, error = %e, "failed to insert backup started row");
            }
            match db::get_backup_schedule_for_hostname_repo(&state.pool, hostname, repo_id.0).await
            {
                Ok(Some(schedule)) => {
                    state.running_schedules.write().await.insert(schedule.id);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(hostname = %hostname, error = %e, "failed to look up schedule for BackupStarted");
                }
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::BackupCompleted { report } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?report.repo_id,
                status = ?report.status,
                "backup completed"
            );
            match db::get_backup_schedule_for_hostname_repo(&state.pool, hostname, report.repo_id.0)
                .await
            {
                Ok(Some(schedule)) => {
                    state.running_schedules.write().await.remove(&schedule.id);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(hostname = %hostname, error = %e, "failed to look up schedule for BackupCompleted");
                }
            }
            let status = match report.status {
                shared::types::BackupStatus::Success => "success",
                shared::types::BackupStatus::Warning => "warning",
                shared::types::BackupStatus::Failed => "failed",
            };
            let notification_error_message = report.error_message.clone();
            let notification_archive_name = report.archive_name.clone();
            let params = db::InsertReportParams {
                client_id,
                repo_id: report.repo_id.0,
                schedule_id: report.schedule_id,
                started_at: report.started_at,
                finished_at: report.finished_at,
                status: status.to_string(),
                original_size: report.original_size,
                compressed_size: report.compressed_size,
                deduplicated_size: report.deduplicated_size,
                repo_unique_csize: report.repo_unique_csize,
                files_processed: report.files_processed,
                duration_secs: report.duration_secs,
                error_message: report.error_message,
                warnings: report.warnings,
                borg_version: report.borg_version,
                matched: true,
                archive_name: report.archive_name,
                borg_command: report.borg_command,
            };
            if let Err(e) = db::insert_backup_report(&state.pool, &params).await {
                tracing::error!(hostname = %hostname, error = %e, "failed to persist backup report");
            }

            if matches!(
                report.status,
                shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
            ) && let Err(e) = db::clear_relocation_pending(&state.pool, report.repo_id.0).await
            {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to clear relocation_pending"
                );
            }

            if let Ok(Some(quota)) = db::quota::get_quota(&state.pool, report.repo_id.0).await {
                let quota_status = db::quota::evaluate_quota(&quota, report.deduplicated_size);
                if !matches!(quota_status, db::quota::QuotaStatus::Ok) {
                    let quota_label = match quota_status {
                        db::quota::QuotaStatus::Ok => "ok",
                        db::quota::QuotaStatus::Warning => "warning",
                        db::quota::QuotaStatus::Critical => "critical",
                    };
                    tracing::warn!(
                        hostname = %hostname,
                        repo_id = ?report.repo_id,
                        deduplicated_size = report.deduplicated_size,
                        quota_status = quota_label,
                        "repository quota exceeded"
                    );

                    let event_type = match quota_status {
                        db::quota::QuotaStatus::Ok => EventType::BackupSuccess,
                        db::quota::QuotaStatus::Warning => EventType::BackupWarning,
                        db::quota::QuotaStatus::Critical => EventType::BackupFailed,
                    };
                    let message = format!(
                        "Repository quota {quota_label} for repo {}: deduplicated size {} bytes \
                         exceeds configured limits",
                        report.repo_id.0, report.deduplicated_size,
                    );
                    let quota_event = NotificationEvent {
                        event_type,
                        hostname: hostname.to_owned(),
                        repo_name: report.repo_id.0.to_string(),
                        status: quota_label.to_owned(),
                        error_message: Some(message),
                        timestamp: chrono::Utc::now(),
                        repo_id: Some(report.repo_id.0),
                        client_id: Some(client_id),
                        schedule_id: None,
                        archive_name: None,
                    };
                    let service = state.notification_service.clone();
                    tokio::spawn(async move {
                        if let Err(e) = notifications::dispatch(&service, quota_event).await {
                            tracing::error!(error = %e, "notification dispatch failed");
                        }
                    });
                }
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
                archive_name: notification_archive_name,
            };
            let service = state.notification_service.clone();
            tokio::spawn(async move {
                if let Err(e) = notifications::dispatch(&service, event).await {
                    tracing::error!(error = %e, "notification dispatch failed");
                }
            });

            if matches!(
                report.status,
                shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
            ) {
                let sync_pool = state.pool.clone();
                let sync_key = state.encryption_key;
                let sync_repo_id = report.repo_id.0;
                let sync_broadcast = state.ui_broadcast.clone();
                tokio::spawn(async move {
                    if let Err(e) = db::set_repo_importing(&sync_pool, sync_repo_id, true).await {
                        tracing::error!(
                            repo_id = sync_repo_id,
                            error = %e,
                            "post-backup sync: failed to set importing flag"
                        );
                        return;
                    }
                    match sync_new_archives(&sync_pool, &sync_key, sync_repo_id, &sync_broadcast)
                        .await
                    {
                        Ok((added, removed)) => {
                            if let Err(e) =
                                db::update_repo_last_synced(&sync_pool, sync_repo_id).await
                            {
                                tracing::error!(
                                    repo_id = sync_repo_id,
                                    error = %e,
                                    "post-backup sync: failed to update last_synced_at"
                                );
                            }
                            if let Err(e) =
                                db::set_repo_importing(&sync_pool, sync_repo_id, false).await
                            {
                                tracing::error!(
                                    repo_id = sync_repo_id,
                                    error = %e,
                                    "post-backup sync: failed to clear importing flag"
                                );
                            }
                            if let Err(e) =
                                db::set_repo_import_error(&sync_pool, sync_repo_id, None).await
                            {
                                tracing::error!(
                                    repo_id = sync_repo_id,
                                    error = %e,
                                    "post-backup sync: failed to clear import_error"
                                );
                            }
                            if added > 0 || removed > 0 {
                                sync_broadcast.send(ServerToUi::DataChanged);
                            }
                            tracing::debug!(
                                repo_id = sync_repo_id,
                                added,
                                removed,
                                "post-backup sync completed"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                repo_id = sync_repo_id,
                                error = %e,
                                "post-backup sync failed"
                            );
                            if let Err(e2) =
                                db::set_repo_importing(&sync_pool, sync_repo_id, false).await
                            {
                                tracing::error!(
                                    repo_id = sync_repo_id,
                                    error = %e2,
                                    "post-backup sync: failed to clear importing flag"
                                );
                            }
                            if let Err(e2) = db::set_repo_import_error(
                                &sync_pool,
                                sync_repo_id,
                                Some(&format!("{e}")),
                            )
                            .await
                            {
                                tracing::error!(
                                    repo_id = sync_repo_id,
                                    error = %e2,
                                    "post-backup sync: failed to set import_error"
                                );
                            }
                        }
                    }
                });
            }

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
            match db::get_backup_schedule_for_hostname_repo(&state.pool, hostname, repo_id.0).await
            {
                Ok(Some(schedule)) => {
                    state.running_schedules.write().await.remove(&schedule.id);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(hostname = %hostname, error = %e, "failed to look up schedule for BackupRejected");
                }
            }
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
                archive_name: None,
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
        AgentToServer::SearchResult { .. }
        | AgentToServer::ExportReady { .. }
        | AgentToServer::KeyExportResult { .. }
        | AgentToServer::KeyImportResult { .. }
        | AgentToServer::PassphraseChanged { .. }
        | AgentToServer::OperationProgress { .. } => {
            tracing::warn!(hostname = %hostname, "unexpected agent response");
        }
        AgentToServer::RestoreCompleted {
            request_id,
            success,
            files_restored,
            error_message,
        } => {
            if let Some(tx) = state.pending_restores.lock().await.remove(&request_id) {
                let _ = tx.send((success, files_restored, error_message));
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    request_id = %request_id,
                    "unexpected RestoreCompleted with no pending request"
                );
            }
        }
        AgentToServer::MigrateEncryptionCompleted {
            request_id,
            success,
            error_message,
        } => {
            if let Some(tx) = state.pending_migrations.lock().await.remove(&request_id) {
                let _ = tx.send((success, error_message));
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    request_id = %request_id,
                    "unexpected MigrateEncryptionCompleted with no pending request"
                );
            }
        }
        AgentToServer::DryRunResult {
            request_id,
            files,
            total_size,
            error_message,
        } => {
            if let Some(tx) = state.pending_dryruns.lock().await.remove(&request_id) {
                let _ = tx.send((files, total_size, error_message));
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    request_id = %request_id,
                    "unexpected DryRunResult with no pending request"
                );
            }
        }
        AgentToServer::OperationFailed { request_id, error } => {
            if let Some(tx) = state.pending_dryruns.lock().await.remove(&request_id) {
                let _ = tx.send((Vec::new(), 0, Some(error)));
            } else if let Some(tx) = state.pending_restores.lock().await.remove(&request_id) {
                let _ = tx.send((false, 0, Some(error)));
            } else if let Some(tx) = state.pending_deletes.lock().await.remove(&request_id) {
                let _ = tx.send((false, 0, Some(error)));
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    request_id = %request_id,
                    "unexpected OperationFailed with no pending request"
                );
            }
        }
        AgentToServer::DeleteArchivesResult {
            request_id,
            success,
            deleted_count,
            error_message,
        } => {
            if let Some(tx) = state.pending_deletes.lock().await.remove(&request_id) {
                let _ = tx.send((success, deleted_count, error_message));
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    request_id = %request_id,
                    "unexpected DeleteArchivesResult with no pending request"
                );
            }
        }
        AgentToServer::BackupCancelled { repo_id } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                "backup cancelled by agent"
            );
            if let Err(e) = db::cancel_backup_report(&state.pool, client_id, repo_id.0).await {
                tracing::error!(
                    hostname = %hostname,
                    repo_id = ?repo_id,
                    error = %e,
                    "failed to update cancelled backup report"
                );
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        net::SocketAddr,
        sync::Arc,
    };

    use chrono::Utc;
    use shared::{
        protocol::AgentToServer,
        types::{BackupReport, BackupStatus, ClientId, RepoId, ReportId},
    };
    use sqlx::PgPool;
    use tokio::sync::{Mutex, RwLock};

    use crate::{
        AppState,
        db::{self, InsertRepoParams, ScheduleParams},
        log_buffer::LogBuffer,
        notifications::NotificationService,
        tunnel::TunnelManager,
        ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
    };

    fn build_test_state(pool: PgPool) -> AppState {
        let ui_broadcast = UiBroadcast::new();
        let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let tunnel_manager = TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr);
        AppState {
            encryption_key: shared::crypto::derive_key(b"test-handler-key"),
            registry: AgentRegistry::new(),
            ui_broadcast,
            tunnel_manager,
            log_buffer: LogBuffer::default(),
            notification_service: NotificationService::new(pool.clone(), reqwest::Client::new()),
            pending_dryruns: Arc::new(Mutex::new(HashMap::new())),
            pending_restores: Arc::new(Mutex::new(HashMap::new())),
            pending_migrations: Arc::new(Mutex::new(HashMap::new())),
            pending_deletes: Arc::new(Mutex::new(HashMap::new())),
            running_schedules: Arc::new(RwLock::new(HashSet::new())),
            pool,
        }
    }

    async fn create_handler_test_data(pool: &PgPool) -> (i64, i64, i64) {
        let client = db::insert_client(pool, "handler-host", None, "hash", None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "handler-repo",
                repo_path: "/backups/handler",
                ssh_user: "user",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: b"enc",
                compression: "none",
                encryption: "none",
                owner_id: None,
            },
        )
        .await
        .unwrap();
        let schedule = db::insert_schedule(
            pool,
            repo.id,
            &ScheduleParams {
                name: "handler-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                ignore_global_excludes: false,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "[]",
                post_backup_commands: "[]",
                execution_mode: "parallel",
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(client.id, 0)])
            .await
            .unwrap();
        (client.id, repo.id, schedule.id)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn backup_started_sets_running_schedule(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (client_id, repo_id, schedule_id) = create_handler_test_data(&pool).await;

        let msg = serde_json::to_string(&AgentToServer::BackupStarted {
            repo_id: RepoId(repo_id),
            started_at: Utc::now(),
        })
        .unwrap();
        super::handle_agent_message(&msg, "handler-host", client_id, &state).await;

        assert!(state.running_schedules.read().await.contains(&schedule_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn backup_rejected_clears_running_schedule(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (client_id, repo_id, schedule_id) = create_handler_test_data(&pool).await;

        state.running_schedules.write().await.insert(schedule_id);

        let msg = serde_json::to_string(&AgentToServer::BackupRejected {
            repo_id: RepoId(repo_id),
            reason: "already running".to_string(),
        })
        .unwrap();
        super::handle_agent_message(&msg, "handler-host", client_id, &state).await;

        assert!(!state.running_schedules.read().await.contains(&schedule_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_clears_running_schedule(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (client_id, repo_id, schedule_id) = create_handler_test_data(&pool).await;

        state.running_schedules.write().await.insert(schedule_id);

        let report = BackupReport {
            id: ReportId(0),
            client_id: ClientId(client_id),
            repo_id: RepoId(repo_id),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            status: BackupStatus::Success,
            original_size: 1000,
            compressed_size: 500,
            deduplicated_size: 250,
            repo_unique_csize: 250,
            files_processed: 10,
            duration_secs: 5,
            error_message: None,
            warnings: Vec::new(),
            borg_version: None,
            archive_name: Some("handler-host-2026-01-01".to_string()),
        };
        let msg = serde_json::to_string(&AgentToServer::BackupCompleted { report }).unwrap();
        super::handle_agent_message(&msg, "handler-host", client_id, &state).await;

        assert!(!state.running_schedules.read().await.contains(&schedule_id));
    }
}
