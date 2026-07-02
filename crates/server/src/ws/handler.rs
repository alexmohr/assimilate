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
use shared::{
    protocol::{AgentToServer, ServerToAgent, ServerToUi},
    types::ScheduleType,
};
use tokio::sync::mpsc;

use crate::{
    AppState,
    api::repos::sync_new_archives,
    archive_index, config_assembler, db,
    notifications::{self, EventType, NotificationEvent},
    ws::{completion_bus::OperationOutcome, ui_broadcast::ActiveBackupSnapshot},
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
                agent_commit_count,
                supports_restart,
                restart_unavailable_reason,
            }) => Some((
                hostname,
                token,
                agent_version,
                agent_git_sha,
                agent_build_time,
                agent_commit_count,
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
        agent_commit_count,
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

    let (agent_id, token_hash) = match db::get_agent_token_hash(&state.pool, &hostname).await {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "unknown agent attempted connection");
            if let Err(e) = db::insert_system_event(
                &state.pool,
                "auth_failed",
                Some(&hostname),
                &format!("Unknown agent '{hostname}' attempted connection"),
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

    let verify_result = tokio::task::spawn_blocking(move || bcrypt::verify(&token, &token_hash))
        .await
        .map_err(|e| {
            tracing::error!(hostname = %hostname, error = %e, "bcrypt task panicked");
        });
    let token_valid = match verify_result {
        Ok(Ok(valid)) => valid,
        Ok(Err(e)) => {
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
        Err(()) => {
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
            &format!("Invalid token for agent '{hostname}'"),
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
        agent_id,
        &agent_version,
        agent_git_sha.as_deref(),
        agent_build_time.as_deref(),
        agent_commit_count.map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
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
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to assemble config on connect"
            );
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
                        handle_agent_message(text.as_str(), &hostname, agent_id, &state).await;
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
    let cleared = state.repo_op_tracker.clear_for_agent(&hostname).await;
    for repo_id in cleared {
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });
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

fn extract_archive_name(borg_command: &str) -> Option<String> {
    borg_command
        .split_whitespace()
        .find(|s| s.starts_with("::"))
        .map(|s| s.trim_start_matches("::").to_owned())
        .filter(|s| !s.is_empty())
}

async fn handle_agent_message(text: &str, hostname: &str, agent_id: i64, state: &AppState) {
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
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to update last_seen_at"
                );
            }
        }
        AgentToServer::BackupStarted {
            repo_id,
            schedule_id,
            started_at,
            borg_command,
            run_id,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                started_at = %started_at,
                "backup started"
            );
            if let Err(e) = db::insert_backup_started(
                &state.pool,
                agent_id,
                repo_id.0,
                schedule_id,
                started_at,
                borg_command.as_deref(),
                run_id.as_deref(),
            )
            .await
            {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to insert backup started row"
                );
            }
            state
                .repo_op_tracker
                .set(
                    repo_id.0,
                    shared::protocol::RepoOpKind::AgentBackup,
                    hostname.to_owned(),
                )
                .await;
            state.ui_broadcast.send(ServerToUi::RepoOpChanged {
                repo_id: repo_id.0,
                op: state.repo_op_tracker.get(repo_id.0).await,
            });
            if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
                let archive_name = borg_command.as_deref().and_then(extract_archive_name);
                state.ui_broadcast.set_active_backup(ActiveBackupSnapshot {
                    hostname: hostname.to_owned(),
                    target_name: target_name.clone(),
                    archive_name: archive_name.clone(),
                    schedule_id,
                    repo_id: repo_id.0,
                    progress_line: None,
                });
                state.ui_broadcast.send(ServerToUi::BackupStarted {
                    hostname: hostname.to_owned(),
                    target_name,
                    archive_name,
                    schedule_id,
                });
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        AgentToServer::BackupCompleted { report } => {
            let report_for_ui = report.clone();
            tracing::info!(
                hostname = %hostname,
                repo_id = ?report.repo_id,
                status = ?report.status,
                "backup completed"
            );
            let outcome_success = !matches!(&report.status, shared::types::BackupStatus::Failed);
            let status = match report.status {
                shared::types::BackupStatus::Success => "success",
                shared::types::BackupStatus::Warning => "warning",
                shared::types::BackupStatus::Failed => "failed",
            };
            state.completion_bus.publish(OperationOutcome {
                hostname: hostname.to_owned(),
                repo_id: report.repo_id.0,
                success: outcome_success,
            });
            let notification_error_message = report.error_message.clone();
            let notification_archive_name = report.archive_name.clone();
            let index_archive_name = notification_archive_name.clone();
            let params = db::InsertReportParams {
                agent_id,
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
                run_id: report.run_id.clone(),
            };
            let report_persisted = match db::insert_backup_report(&state.pool, &params).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(
                        hostname = %hostname,
                        error = %e,
                        "failed to persist backup report"
                    );
                    false
                }
            };

            if report_persisted
                && matches!(
                    report.status,
                    shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
                )
                && let Some(archive_name) = index_archive_name
            {
                let index_pool = state.pool.clone();
                let index_key = state.encryption_key;
                let index_repo_id = report.repo_id.0;
                let index_repo_lock = state.repo_lock.clone();
                tokio::spawn(async move {
                    match archive_index::ensure_indexed(
                        index_pool,
                        index_key,
                        index_repo_id,
                        archive_name.clone(),
                        index_repo_lock,
                    )
                    .await
                    {
                        Ok(status) => {
                            tracing::debug!(
                                repo_id = index_repo_id,
                                archive_name = %archive_name,
                                status = ?status,
                                "queued archive indexing after backup"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                repo_id = index_repo_id,
                                archive_name = %archive_name,
                                error = %e,
                                "failed to queue archive indexing after backup"
                            );
                        }
                    }
                });
            }

            if matches!(
                report.status,
                shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
            ) && let Err(e) =
                db::clear_relocation_for_host(&state.pool, report.repo_id.0, hostname).await
            {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to clear relocation_pending for host"
                );
            }

            let repo_name = db::get_repo_name(&state.pool, report.repo_id.0)
                .await
                .unwrap_or_else(|_| report.repo_id.0.to_string());
            let completed_repo_name = repo_name.clone();

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
                        repo_name, report.deduplicated_size,
                    );
                    let quota_event = NotificationEvent {
                        event_type,
                        hostname: hostname.to_owned(),
                        repo_name: repo_name.clone(),
                        status: quota_label.to_owned(),
                        error_message: Some(message),
                        timestamp: chrono::Utc::now(),
                        repo_id: Some(report.repo_id.0),
                        agent_id: Some(agent_id),
                        schedule_id: None,
                        schedule_name: None,
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
            let schedule_name = match report.schedule_id {
                Some(sid) => db::get_schedule_display_name(&state.pool, sid, &repo_name)
                    .await
                    .ok(),
                None => None,
            };
            let event = NotificationEvent {
                event_type,
                hostname: hostname.to_owned(),
                repo_name,
                status: status.to_string(),
                error_message: notification_error_message,
                timestamp: chrono::Utc::now(),
                repo_id: Some(report.repo_id.0),
                agent_id: Some(agent_id),
                schedule_id: report.schedule_id,
                schedule_name,
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
                let sync_repo_lock = state.repo_lock.clone();
                tokio::spawn(async move {
                    if let Err(e) = db::set_repo_importing(&sync_pool, sync_repo_id, true).await {
                        tracing::error!(
                            repo_id = sync_repo_id,
                            error = %e,
                            "post-backup sync: failed to set importing flag"
                        );
                        return;
                    }
                    match sync_new_archives(
                        &sync_pool,
                        &sync_key,
                        sync_repo_id,
                        &sync_broadcast,
                        &sync_repo_lock,
                    )
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
                            crate::api::repos::clear_import_progress_state(
                                &sync_pool,
                                &sync_broadcast,
                                sync_repo_id,
                            )
                            .await;
                            sync_broadcast.send(ServerToUi::DataChanged);
                            if added > 0 || removed > 0 {
                                tracing::debug!(
                                    repo_id = sync_repo_id,
                                    added,
                                    removed,
                                    "post-backup sync changed repo contents"
                                );
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
                            crate::api::repos::clear_import_progress_state(
                                &sync_pool,
                                &sync_broadcast,
                                sync_repo_id,
                            )
                            .await;
                            sync_broadcast.send(ServerToUi::DataChanged);
                        }
                    }
                });
            }

            let finished_repo_id = report.repo_id.0;
            state.repo_op_tracker.clear(finished_repo_id).await;
            if let Err(e) = db::update_repo_last_op(
                &state.pool,
                finished_repo_id,
                "agent_backup",
                chrono::Utc::now(),
                hostname,
            )
            .await
            {
                tracing::warn!(
                    repo_id = finished_repo_id,
                    error = %e,
                    "failed to persist last_op for backup"
                );
            }
            state.ui_broadcast.send(ServerToUi::RepoOpChanged {
                repo_id: finished_repo_id,
                op: None,
            });
            state.ui_broadcast.send(ServerToUi::BackupCompleted {
                hostname: hostname.to_owned(),
                target_name: completed_repo_name,
                report: Box::new(report_for_ui),
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
            state.completion_bus.publish(OperationOutcome {
                hostname: hostname.to_owned(),
                repo_id: repo_id.0,
                success: false,
            });
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
            state.completion_bus.publish(OperationOutcome {
                hostname: hostname.to_owned(),
                repo_id: repo_id.0,
                success,
            });
            let repo_name = db::get_repo_name(&state.pool, repo_id.0)
                .await
                .unwrap_or_else(|_| repo_id.0.to_string());
            state.ui_broadcast.send(ServerToUi::CheckCompleted {
                hostname: hostname.to_owned(),
                target_name: repo_name.clone(),
                success,
                error_message: error_message.clone(),
            });

            // The agent only reports a repo id, not which schedule triggered the
            // check, so infer it from the host/repo/type combination.
            let schedule = db::get_schedule_for_hostname_repo(
                &state.pool,
                hostname,
                repo_id.0,
                ScheduleType::Check,
            )
            .await
            .ok()
            .flatten();
            let schedule_name = match &schedule {
                Some(s) if !s.name.trim().is_empty() => Some(s.name.clone()),
                Some(_) => Some(repo_name.clone()),
                None => None,
            };

            let event_type = if success {
                EventType::CheckSuccess
            } else {
                EventType::CheckFailed
            };
            let event = NotificationEvent {
                event_type,
                hostname: hostname.to_owned(),
                repo_name,
                status: if success { "success" } else { "failed" }.to_owned(),
                error_message,
                timestamp: chrono::Utc::now(),
                repo_id: Some(repo_id.0),
                agent_id: Some(agent_id),
                schedule_id: schedule.map(|s| s.id),
                schedule_name,
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
            state.completion_bus.publish(OperationOutcome {
                hostname: hostname.to_owned(),
                repo_id: repo_id.0,
                success,
            });
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
            let schedule_id = db::get_schedule_for_hostname_repo(
                &state.pool,
                hostname,
                repo_id.0,
                ScheduleType::Backup,
            )
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
        AgentToServer::BackupLog {
            repo_id,
            schedule_id,
            line,
        } => {
            state.ui_broadcast.send(ServerToUi::BackupLog {
                hostname: hostname.to_owned(),
                schedule_id,
                repo_id: repo_id.0,
                line,
            });
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
            state.completion_bus.publish(OperationOutcome {
                hostname: hostname.to_owned(),
                repo_id: repo_id.0,
                success: false,
            });
            if let Err(e) = db::cancel_backup_report(&state.pool, agent_id, repo_id.0).await {
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
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use chrono::{TimeZone, Utc};
    use shared::{
        crypto::{derive_key, encrypt_passphrase},
        protocol::AgentToServer,
        types::{AgentId, BackupReport, BackupStatus, RepoId, ReportId},
    };
    use sqlx::PgPool;
    use tokio::time::timeout;

    use super::*;

    static TEST_BORG_BINARY_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn build_test_state(pool: PgPool) -> AppState {
        let ui_broadcast = crate::ws::ui_broadcast::UiBroadcast::new();
        let tunnel_manager = crate::tunnel::TunnelManager::new(
            pool.clone(),
            ui_broadcast.clone(),
            "127.0.0.1:0".parse().expect("valid socket address"),
        );

        AppState {
            pool: pool.clone(),
            encryption_key: derive_key(b"handler-test-secret-key").unwrap(),
            registry: crate::ws::registry::AgentRegistry::new(),
            ui_broadcast,
            tunnel_manager,
            log_buffer: crate::log_buffer::LogBuffer::default(),
            notification_service: crate::notifications::NotificationService::new(
                pool,
                reqwest::Client::new(),
            ),
            completion_bus: crate::ws::completion_bus::CompletionBus::new(),
            repo_op_tracker: crate::repo_op_tracker::RepoOpTracker::default(),
            repo_lock: crate::RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_restores: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_migrations: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_deletes: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
        }
    }

    async fn write_fake_borg_binary() -> PathBuf {
        let counter = TEST_BORG_BINARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "borg-fake-handler-{pid}-{counter}",
            pid = std::process::id()
        ));
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("create fake borg dir");

        let binary = dir.join("borg");
        let script = r#"#!/usr/bin/env bash
set -eu

for arg in "$@"; do
  case "$arg" in
    --json-lines)
      cat <<'JSON'
{
  "type": "file",
  "path": "dir/file.txt",
  "size": 123,
  "mtime": "2026-06-05T12:00:00",
  "mode": "100644"
}
JSON
      exit 0
      ;;
    --glob-archives)
      cat <<'JSON'
{
  "archives": [
    {
      "name": "archive-1",
      "hostname": "agent-1",
      "start": "2026-06-05T12:00:00+00:00",
      "end": "2026-06-05T12:05:00+00:00",
      "duration": 5.0,
      "stats": {
        "original_size": 1000,
        "compressed_size": 500,
        "deduplicated_size": 250,
        "nfiles": 3
      }
    }
  ]
}
JSON
      exit 0
      ;;
  esac
done

if [ "${1:-}" = "info" ]; then
  cat <<'JSON'
{
  "cache": {
    "stats": {
      "total_size": 1000,
      "total_csize": 500,
      "unique_csize": 250,
      "total_chunks": 10,
      "total_unique_chunks": 5
    }
  },
  "encryption": {
    "mode": "repokey"
  }
}
JSON
  exit 0
fi

if [ "${1:-}" = "list" ]; then
  cat <<'JSON'
{
  "archives": [
    {
      "name": "archive-1",
      "hostname": "agent-1",
      "start": "2026-06-05T12:00:00+00:00",
      "end": "2026-06-05T12:05:00+00:00"
    }
  ]
}
JSON
  exit 0
fi

exit 0
"#;

        tokio::fs::write(&binary, script)
            .await
            .expect("write fake borg binary");
        let mut permissions = tokio::fs::metadata(&binary)
            .await
            .expect("read fake borg metadata")
            .permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&binary, permissions)
            .await
            .expect("mark fake borg executable");
        binary
    }

    #[ignore]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_queues_archive_indexing(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "handler-repo",
                repo_path: "/backups/handler",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .expect("insert repo");

        let borg_binary = write_fake_borg_binary().await;
        let _borg_guard = crate::borg::override_binary_for_tests(borg_binary);
        let state = build_test_state(pool.clone());

        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent.id),
            repo_id: RepoId(repo.id),
            schedule_id: None,
            started_at,
            finished_at: started_at + chrono::Duration::minutes(5),
            status: BackupStatus::Success,
            original_size: 1_000,
            compressed_size: 500,
            deduplicated_size: 250,
            repo_unique_csize: 250,
            files_processed: 3,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: Some("archive-2".to_string()),
            borg_command: Some("borg create".to_string()),
            run_id: None,
        };
        let msg = serde_json::to_string(&AgentToServer::BackupCompleted { report })
            .expect("serialize message");

        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        timeout(Duration::from_secs(5), async {
            loop {
                let archive_2 = crate::archive_index::get_index_status(&pool, repo.id, "archive-2")
                    .await
                    .expect("archive-2 status query");
                if matches!(archive_2, Some(crate::archive_index::IndexStatus::Done)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("timed out waiting for archive indexing");
    }
}
