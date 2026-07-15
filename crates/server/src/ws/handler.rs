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
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use shared::{
    protocol::{AgentToServer, ServerToAgent, ServerToUi},
    types::ScheduleType,
};
use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::{
    AppState,
    api::repos::sync_new_archives,
    archive_index, config_assembler, db,
    notifications::{self, EventType, NotificationEvent},
    quota_enforcement,
    ws::{completion_bus::OperationOutcome, ui_broadcast::ActiveBackupSnapshot},
};

const PING_INTERVAL: Duration = Duration::from_secs(30);
const CHANNEL_BUFFER: usize = 32;

/// WebSocket upgrade handler for agent connections.
///
/// Expects a `Hello` message as the first frame, authenticates the agent,
/// registers it in the agent registry, and starts the bidirectional message loop.
pub fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl std::future::Future<Output = impl IntoResponse> {
    std::future::ready(ws.on_upgrade(|socket| handle_socket(socket, state)))
}

#[derive(sqlx::FromRow)]
#[allow(
    clippy::struct_field_names,
    reason = "these are distinct foreign-key/identifier columns from the backup_reports table, \
              not a repeated affix to strip"
)]
struct PendingBackupRow {
    repo_id: i64,
    schedule_id: Option<i64>,
    run_id: Option<String>,
}

struct HelloFields {
    hostname: String,
    token: String,
    agent_version: String,
    agent_git_sha: Option<String>,
    agent_build_time: Option<String>,
    agent_commit_count: Option<u32>,
    supports_restart: bool,
    restart_unavailable_reason: Option<String>,
}

async fn read_hello_message(ws_stream: &mut SplitStream<WebSocket>) -> Option<HelloFields> {
    match ws_stream.next().await {
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
            }) => Some(HelloFields {
                hostname,
                token,
                agent_version,
                agent_git_sha,
                agent_build_time,
                agent_commit_count,
                supports_restart,
                restart_unavailable_reason,
            }),
            Ok(_) | Err(_) => None,
        },
        Some(
            Ok(Message::Close(_) | Message::Binary(_) | Message::Ping(_) | Message::Pong(_))
            | Err(_),
        )
        | None => None,
    }
}

async fn send_close(ws_sink: &mut SplitSink<WebSocket, Message>, reason: &'static str) {
    let close = Message::Close(Some(CloseFrame {
        code: 4001,
        reason: reason.into(),
    }));
    if let Err(e) = ws_sink.send(close).await {
        tracing::debug!(error = %e, "ws send failed");
    }
}

/// Looks up the agent's token hash and verifies the presented token.
/// Sends a close frame and logs a system event on any failure. Returns the
/// agent ID on success.
async fn authenticate_agent(
    pool: &PgPool,
    ws_sink: &mut SplitSink<WebSocket, Message>,
    hostname: &str,
    token: String,
) -> Option<i64> {
    let (agent_id, token_hash) = match db::get_agent_token_hash(pool, hostname).await {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "unknown agent attempted connection");
            if let Err(e) = db::insert_system_event(
                pool,
                "auth_failed",
                Some(hostname),
                &format!("Unknown agent '{hostname}' attempted connection"),
            )
            .await
            {
                tracing::error!(error = %e, "failed to insert system event");
            }
            send_close(ws_sink, "authentication failed").await;
            return None;
        }
    };

    let hostname_owned = hostname.to_owned();
    let verify_result = tokio::task::spawn_blocking(move || bcrypt::verify(&token, &token_hash))
        .await
        .map_err(|e| {
            tracing::error!(hostname = %hostname_owned, error = %e, "bcrypt task panicked");
        });
    let token_valid = match verify_result {
        Ok(Ok(valid)) => valid,
        Ok(Err(e)) => {
            tracing::error!(hostname = %hostname, error = %e, "bcrypt verification failed");
            send_close(ws_sink, "authentication failed").await;
            return None;
        }
        Err(()) => {
            send_close(ws_sink, "authentication failed").await;
            return None;
        }
    };

    if !token_valid {
        tracing::warn!(hostname = %hostname, "invalid agent token");
        if let Err(e) = db::insert_system_event(
            pool,
            "auth_failed",
            Some(hostname),
            &format!("Invalid token for agent '{hostname}'"),
        )
        .await
        {
            tracing::error!(error = %e, "failed to insert system event");
        }
        send_close(ws_sink, "authentication failed").await;
        return None;
    }

    Some(agent_id)
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    let Some(hello) = read_hello_message(&mut ws_stream).await else {
        send_close(&mut ws_sink, "expected Hello message").await;
        return;
    };
    let HelloFields {
        hostname,
        token,
        agent_version,
        agent_git_sha,
        agent_build_time,
        agent_commit_count,
        supports_restart,
        restart_unavailable_reason,
    } = hello;

    tracing::info!(
        hostname = %hostname,
        agent_version = %agent_version,
        "agent attempting connection"
    );

    let Some(agent_id) = authenticate_agent(&state.pool, &mut ws_sink, &hostname, token).await
    else {
        return;
    };

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

    send_reconnect_catchup(&state, &mut ws_sink, &hostname, agent_id).await;

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

async fn send_ws_message(ws_sink: &mut SplitSink<WebSocket, Message>, msg: &ServerToAgent) -> bool {
    let Ok(json) = serde_json::to_string(msg) else {
        return false;
    };
    ws_sink.send(Message::Text(json.into())).await.is_ok()
}

/// Catches the agent up on state it may have missed while offline: pushes a
/// fresh config, notifies it of any backups the user cancelled while it was
/// disconnected, and re-triggers any backup runs that were queued (e.g. via
/// "Run Now") but never dispatched.
async fn send_reconnect_catchup(
    state: &AppState,
    ws_sink: &mut SplitSink<WebSocket, Message>,
    hostname: &str,
    agent_id: i64,
) {
    match config_assembler::assemble_config(&state.pool, &state.encryption_key, hostname).await {
        Ok(config) => {
            if !send_ws_message(ws_sink, &ServerToAgent::ConfigUpdate(config)).await {
                tracing::debug!(hostname = %hostname, "ws send failed");
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

    // If the user cancelled a backup while this agent was offline, notify the
    // agent now so it can kill the corresponding borg process.
    if let Ok(rows) = sqlx::query_scalar!(
        "SELECT DISTINCT repo_id FROM backup_reports WHERE agent_id = $1 AND status = 'cancelled' \
         AND NOT cancellation_acknowledged",
        agent_id,
    )
    .fetch_all(&state.pool)
    .await
    {
        for repo_id in rows {
            let msg = ServerToAgent::CancelBackup {
                repo_id: shared::types::RepoId(repo_id),
            };
            if !send_ws_message(ws_sink, &msg).await {
                tracing::warn!(
                    hostname = %hostname,
                    repo_id,
                    "failed to notify agent of cancelled backup on reconnect"
                );
            }
        }
    }

    // Execute any pending backup runs that were triggered (e.g. "Run Now") while
    // this agent was offline. The backup_report is already in the DB as 'pending'
    // and will be updated to 'started' when the agent reports BackupStarted.
    if let Ok(rows) = sqlx::query_as!(
        PendingBackupRow,
        "SELECT repo_id, schedule_id, run_id FROM backup_reports WHERE agent_id = $1 AND status = \
         'pending' ORDER BY started_at ASC",
        agent_id,
    )
    .fetch_all(&state.pool)
    .await
    {
        for row in rows {
            let PendingBackupRow {
                repo_id,
                schedule_id,
                run_id,
            } = row;
            let msg = ServerToAgent::RunBackupNow {
                repo_id: shared::types::RepoId(repo_id),
                schedule_id,
                request_id: None,
                run_id,
            };
            if send_ws_message(ws_sink, &msg).await {
                tracing::info!(
                    hostname = %hostname,
                    repo_id,
                    schedule_id,
                    "sent pending RunBackupNow on reconnect"
                );
            } else {
                tracing::warn!(
                    hostname = %hostname,
                    repo_id,
                    "failed to send pending RunBackupNow on reconnect"
                );
            }
        }
    }
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

pub(crate) async fn validate_agent_repo(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    hostname: &str,
    msg_type: &str,
) -> bool {
    let Ok(has_access) = db::check_agent_repo_access(pool, agent_id, repo_id).await else {
        tracing::error!(
            hostname = %hostname,
            agent_id = agent_id,
            repo_id = repo_id,
            msg_type = %msg_type,
            "validate_agent_repo: DB error checking agent repo access"
        );
        return false;
    };
    if has_access {
        return true;
    }
    tracing::warn!(
        security = true,
        hostname = %hostname,
        agent_id = agent_id,
        repo_id = repo_id,
        msg_type = %msg_type,
        "agent tried to report on a repo it is not assigned to"
    );
    if let Err(e) = db::insert_system_event(
        pool,
        "security_violation",
        Some(hostname),
        &format!(
            "Agent '{hostname}' (id={agent_id}) tried to report on repo {repo_id} without \
             assignment (msg={msg_type})"
        ),
    )
    .await
    {
        tracing::error!(error = %e, "failed to insert security_violation system event");
    }
    false
}

fn quota_status_label(status: db::quota::QuotaStatus) -> &'static str {
    match status {
        db::quota::QuotaStatus::Ok => "ok",
        db::quota::QuotaStatus::Warning => "warning",
        db::quota::QuotaStatus::Critical => "critical",
    }
}

fn quota_event_type(status: db::quota::QuotaStatus) -> EventType {
    match status {
        db::quota::QuotaStatus::Ok => EventType::BackupSuccess,
        db::quota::QuotaStatus::Warning => EventType::BackupWarning,
        db::quota::QuotaStatus::Critical => EventType::BackupFailed,
    }
}

/// Dispatches the notification for a repo- or server-quota breach. Shared by both the
/// per-repo and per-host (shared SSH host) quota checks in [`handle_agent_message`], which
/// otherwise duplicate this status-to-label, status-to-`EventType`, and dispatch logic almost
/// verbatim.
fn dispatch_quota_breach_notification(
    state: &AppState,
    hostname: &str,
    agent_id: i64,
    repo_name: &str,
    repo_id: i64,
    quota_status: db::quota::QuotaStatus,
    message: String,
) {
    let quota_event = NotificationEvent {
        event_type: quota_event_type(quota_status),
        hostname: hostname.to_owned(),
        repo_name: repo_name.to_owned(),
        status: quota_status_label(quota_status).to_owned(),
        error_message: Some(message),
        timestamp: chrono::Utc::now(),
        repo_id: Some(repo_id),
        agent_id: Some(agent_id),
        schedule_id: None,
        schedule_name: None,
        archive_name: None,
    };
    let service = state.notification_service.clone();
    let task_guard = state.background_task_tracker.begin();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        if let Err(e) = notifications::dispatch(&service, quota_event).await {
            tracing::error!(error = %e, "notification dispatch failed");
        }
    });
}

/// Dispatches a decoded [`AgentToServer`] message to its handler.
///
/// Every variant carrying real logic is delegated to its own `handle_*`
/// function below (each independently under the line-count limit); what
/// remains here is a flat, one-arm-per-variant routing table over the full
/// protocol enum. Splitting that table across multiple functions would not
/// remove any logic -- it would just force a reader chasing "what happens
/// for message X" to jump between several partial matches instead of
/// reading one.
#[allow(
    clippy::too_many_lines,
    reason = "flat 19-variant protocol dispatch table; every arm with real logic already \
              delegates to its own handle_* function, so splitting this further would fragment \
              routing logic across multiple partial matches without removing any code"
)]
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
            handle_backup_started(BackupStartedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                schedule_id,
                started_at,
                borg_command,
                run_id,
            })
            .await;
        }
        AgentToServer::BackupCompleted { report } => {
            handle_backup_completed(hostname, agent_id, state, report).await;
        }
        AgentToServer::StatusUpdate { repo_id, status } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                status = ?status,
                "status update"
            );
            // Guard is applied for future-proofing even though StatusUpdate
            // is currently a no-op.
            let _ = validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "StatusUpdate")
                .await;
        }
        AgentToServer::BackupRejected { repo_id, reason } => {
            handle_backup_rejected(hostname, agent_id, state, repo_id, &reason).await;
        }
        AgentToServer::CheckCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
        } => {
            handle_check_completed(CheckCompletedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                duration_secs,
                error_message,
            })
            .await;
        }
        AgentToServer::VerifyCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
            files_verified,
        } => {
            handle_verify_completed(VerifyCompletedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                duration_secs,
                error_message,
                files_verified,
            })
            .await;
        }
        AgentToServer::CanaryVerified {
            repo_id,
            success,
            nonce,
            archive_name,
            error_message,
        } => {
            handle_canary_verified(CanaryVerifiedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                nonce,
                archive_name,
                error_message,
            })
            .await;
        }
        AgentToServer::BackupLog {
            repo_id,
            schedule_id,
            line,
        } => {
            handle_backup_log(hostname, agent_id, state, repo_id, schedule_id, line).await;
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
            handle_restore_completed(
                hostname,
                state,
                request_id,
                success,
                files_restored,
                error_message,
            )
            .await;
        }
        AgentToServer::MigrateEncryptionCompleted {
            request_id,
            success,
            error_message,
        } => {
            handle_migrate_encryption_completed(
                hostname,
                state,
                request_id,
                success,
                error_message,
            )
            .await;
        }
        AgentToServer::DryRunResult {
            request_id,
            files,
            total_size,
            error_message,
        } => {
            handle_dry_run_result(
                hostname,
                state,
                request_id,
                files,
                total_size,
                error_message,
            )
            .await;
        }
        AgentToServer::OperationFailed { request_id, error } => {
            handle_operation_failed(hostname, state, request_id, error).await;
        }
        AgentToServer::DeleteArchivesResult {
            request_id,
            success,
            deleted_count,
            error_message,
        } => {
            handle_delete_archives_result(
                hostname,
                state,
                request_id,
                success,
                deleted_count,
                error_message,
            )
            .await;
        }
        AgentToServer::BackupCancelled { repo_id } => {
            handle_backup_cancelled(hostname, agent_id, state, repo_id).await;
        }
    }
}

async fn handle_backup_log(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    repo_id: shared::types::RepoId,
    schedule_id: Option<i64>,
    line: String,
) {
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "BackupLog").await {
        return;
    }
    state.ui_broadcast.send(ServerToUi::BackupLog {
        hostname: hostname.to_owned(),
        schedule_id,
        repo_id: repo_id.0,
        line,
    });
}

async fn handle_restore_completed(
    hostname: &str,
    state: &AppState,
    request_id: String,
    success: bool,
    files_restored: u64,
    error_message: Option<String>,
) {
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

async fn handle_migrate_encryption_completed(
    hostname: &str,
    state: &AppState,
    request_id: String,
    success: bool,
    error_message: Option<String>,
) {
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

async fn handle_dry_run_result(
    hostname: &str,
    state: &AppState,
    request_id: String,
    files: Vec<shared::types::DryRunFile>,
    total_size: i64,
    error_message: Option<String>,
) {
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

async fn handle_operation_failed(
    hostname: &str,
    state: &AppState,
    request_id: String,
    error: String,
) {
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

async fn handle_delete_archives_result(
    hostname: &str,
    state: &AppState,
    request_id: String,
    success: bool,
    deleted_count: u32,
    error_message: Option<String>,
) {
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

struct BackupStartedArgs<'a> {
    hostname: &'a str,
    agent_id: i64,
    state: &'a AppState,
    repo_id: shared::types::RepoId,
    schedule_id: Option<i64>,
    started_at: chrono::DateTime<chrono::Utc>,
    borg_command: Option<String>,
    run_id: Option<String>,
}

async fn handle_backup_started(args: BackupStartedArgs<'_>) {
    let BackupStartedArgs {
        hostname,
        agent_id,
        state,
        repo_id,
        schedule_id,
        started_at,
        borg_command,
        run_id,
    } = args;

    tracing::info!(
        hostname = %hostname,
        repo_id = ?repo_id,
        started_at = %started_at,
        "backup started"
    );
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "BackupStarted").await {
        return;
    }
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
    // If the agent restarted and is starting a new backup, any existing
    // 'started' rows for this agent+repo are orphaned - fail them.
    if let Err(e) = db::fail_other_started_backups(
        &state.pool,
        agent_id,
        repo_id.0,
        run_id.as_deref(),
        hostname,
    )
    .await
    {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to clean up orphaned backup rows"
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

/// Queues content indexing for the archive just backed up, running in the
/// background so the caller doesn't block on it.
fn spawn_post_backup_indexing(state: &AppState, repo_id: i64, archive_name: String) {
    let pool = state.pool.clone();
    let encryption_key = state.encryption_key;
    let repo_lock = state.repo_lock.clone();
    let background_task_tracker = state.background_task_tracker.clone();
    let task_guard = state.background_task_tracker.begin();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        match archive_index::ensure_indexed(
            pool,
            encryption_key,
            repo_id,
            archive_name.clone(),
            repo_lock,
            &background_task_tracker,
        )
        .await
        {
            Ok(status) => {
                tracing::debug!(
                    repo_id,
                    archive_name = %archive_name,
                    status = ?status,
                    "queued archive indexing after backup"
                );
            }
            Err(e) => {
                tracing::error!(
                    repo_id,
                    archive_name = %archive_name,
                    error = %e,
                    "failed to queue archive indexing after backup"
                );
            }
        }
    });
}

/// Checks whether the just-completed backup pushed the repository's own
/// quota over a warning/critical threshold, dispatching a notification and
/// enforcement action if so.
async fn check_repo_quota_after_backup(
    state: &AppState,
    hostname: &str,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    deduplicated_size: i64,
    repo_name: &str,
) {
    let Ok(Some(quota)) = db::quota::get_quota(&state.pool, repo_id).await else {
        return;
    };
    let quota_status = db::quota::evaluate_quota(&quota, deduplicated_size);
    if matches!(quota_status, db::quota::QuotaStatus::Ok) {
        return;
    }
    tracing::warn!(
        hostname = %hostname,
        repo_id,
        deduplicated_size,
        quota_status = quota_status_label(quota_status),
        "repository quota exceeded"
    );

    let message = format!(
        "Repository quota {} for repo {repo_name}: deduplicated size {deduplicated_size} bytes \
         exceeds configured limits",
        quota_status_label(quota_status),
    );
    dispatch_quota_breach_notification(
        state,
        hostname,
        agent_id,
        repo_name,
        repo_id,
        quota_status,
        message,
    );

    if let Some(action) = quota.action_for(quota_status) {
        quota_enforcement::enforce_repo_quota_action(state, repo_id, schedule_id, action).await;
    }
}

/// Checks whether the just-completed backup, combined with its sibling
/// repos on the same SSH host, pushed a shared server-wide quota over a
/// warning/critical threshold, dispatching a notification and enforcement
/// action if so.
async fn check_server_quota_after_backup(
    state: &AppState,
    hostname: &str,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    deduplicated_size: i64,
    repo_name: &str,
) {
    let Ok(ssh_host) = db::get_repo_ssh_host(&state.pool, repo_id).await else {
        return;
    };
    let Ok(Some(server_quota)) = db::server_quota::get_server_quota(&state.pool, &ssh_host).await
    else {
        return;
    };
    let Ok(siblings_deduplicated_size) =
        db::server_quota::total_deduplicated_size_for_ssh_host_excluding(
            &state.pool,
            &ssh_host,
            repo_id,
        )
        .await
    else {
        return;
    };

    // Combine the just-completed backup's fresh `deduplicated_size` with the
    // (possibly stale, since `repo_stats` is only refreshed by a sync/rescan) snapshot
    // for sibling repos on the host, so a breach on an otherwise idle host is caught
    // immediately rather than only after an unrelated rescan.
    let total_deduplicated_size = siblings_deduplicated_size.saturating_add(deduplicated_size);
    let quota_status = server_quota.status(total_deduplicated_size);
    if matches!(quota_status, db::quota::QuotaStatus::Ok) {
        return;
    }
    tracing::warn!(
        hostname = %hostname,
        ssh_host = %ssh_host,
        total_deduplicated_size,
        quota_status = quota_status_label(quota_status),
        "server quota exceeded"
    );

    let message = format!(
        "Server quota {} for host {ssh_host}: combined deduplicated size \
         {total_deduplicated_size} bytes exceeds configured limits",
        quota_status_label(quota_status),
    );
    dispatch_quota_breach_notification(
        state,
        hostname,
        agent_id,
        repo_name,
        repo_id,
        quota_status,
        message,
    );

    if let Some(action) = server_quota.action_for(quota_status) {
        quota_enforcement::enforce_server_quota_action(state, &ssh_host, schedule_id, action).await;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "grouping these into a struct would obscure the call site more than it would clarify \
              it; all params are single-use scalars/refs from the caller's own locals"
)]
async fn dispatch_backup_completion_notification(
    state: &AppState,
    status: shared::types::BackupStatus,
    hostname: &str,
    repo_name: String,
    status_str: &str,
    error_message: Option<String>,
    repo_id: i64,
    agent_id: i64,
    schedule_id: Option<i64>,
    archive_name: Option<String>,
) {
    let event_type = match status {
        shared::types::BackupStatus::Success => EventType::BackupSuccess,
        shared::types::BackupStatus::Warning => EventType::BackupWarning,
        shared::types::BackupStatus::Failed => EventType::BackupFailed,
    };
    let schedule_name = match schedule_id {
        Some(sid) => db::get_schedule_display_name(&state.pool, sid, &repo_name)
            .await
            .ok(),
        None => None,
    };
    let event = NotificationEvent {
        event_type,
        hostname: hostname.to_owned(),
        repo_name,
        status: status_str.to_string(),
        error_message,
        timestamp: chrono::Utc::now(),
        repo_id: Some(repo_id),
        agent_id: Some(agent_id),
        schedule_id,
        schedule_name,
        archive_name,
    };
    let service = state.notification_service.clone();
    let task_guard = state.background_task_tracker.begin();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        if let Err(e) = notifications::dispatch(&service, event).await {
            tracing::error!(error = %e, "notification dispatch failed");
        }
    });
}

/// Runs the post-backup archive sync in the background: marks the repo as
/// importing, syncs any new archives, and clears importing/error state
/// regardless of outcome.
async fn run_post_backup_sync(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    ui_broadcast: crate::ws::ui_broadcast::UiBroadcast,
    repo_lock: crate::RepoLock,
    background_task_tracker: crate::background_tasks::BackgroundTaskTracker,
) {
    let _task_guard = background_task_tracker.begin();
    if let Err(e) = db::set_repo_importing(&pool, repo_id, true).await {
        tracing::error!(repo_id, error = %e, "post-backup sync: failed to set importing flag");
        return;
    }
    match sync_new_archives(
        &pool,
        &encryption_key,
        repo_id,
        &ui_broadcast,
        &repo_lock,
        &background_task_tracker,
    )
    .await
    {
        Ok((added, removed)) => {
            if let Err(e) = db::update_repo_last_synced(&pool, repo_id).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "post-backup sync: failed to update last_synced_at"
                );
            }
            if let Err(e) = db::set_repo_importing(&pool, repo_id, false).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "post-backup sync: failed to clear importing flag"
                );
            }
            if let Err(e) = db::set_repo_import_error(&pool, repo_id, None).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "post-backup sync: failed to clear import_error"
                );
            }
            crate::api::repos::clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
            ui_broadcast.send(ServerToUi::DataChanged);
            if added > 0 || removed > 0 {
                tracing::debug!(
                    repo_id,
                    added,
                    removed,
                    "post-backup sync changed repo contents"
                );
            }
            tracing::debug!(repo_id, added, removed, "post-backup sync completed");
        }
        Err(e) => {
            tracing::error!(repo_id, error = %e, "post-backup sync failed");
            if let Err(e2) = db::set_repo_importing(&pool, repo_id, false).await {
                tracing::error!(
                    repo_id,
                    error = %e2,
                    "post-backup sync: failed to clear importing flag"
                );
            }
            if let Err(e2) = db::set_repo_import_error(&pool, repo_id, Some(&format!("{e}"))).await
            {
                tracing::error!(
                    repo_id,
                    error = %e2,
                    "post-backup sync: failed to set import_error"
                );
            }
            crate::api::repos::clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
            ui_broadcast.send(ServerToUi::DataChanged);
        }
    }
}

fn spawn_post_backup_sync(state: &AppState, repo_id: i64) {
    tokio::spawn(run_post_backup_sync(
        state.pool.clone(),
        state.encryption_key,
        repo_id,
        state.ui_broadcast.clone(),
        state.repo_lock.clone(),
        state.background_task_tracker.clone(),
    ));
}

async fn finalize_backup_completion(
    state: &AppState,
    hostname: &str,
    repo_id: i64,
    report_for_ui: shared::types::BackupReport,
    repo_name: String,
) {
    state.repo_op_tracker.clear(repo_id).await;
    if let Err(e) = db::update_repo_last_op(
        &state.pool,
        repo_id,
        "agent_backup",
        chrono::Utc::now(),
        hostname,
    )
    .await
    {
        tracing::warn!(repo_id, error = %e, "failed to persist last_op for backup");
    }
    state
        .ui_broadcast
        .send(ServerToUi::RepoOpChanged { repo_id, op: None });
    state.ui_broadcast.send(ServerToUi::BackupCompleted {
        hostname: hostname.to_owned(),
        target_name: repo_name,
        report: Box::new(report_for_ui),
    });
    state.ui_broadcast.send(ServerToUi::DataChanged);
}

async fn persist_backup_completed_report(
    state: &AppState,
    hostname: &str,
    agent_id: i64,
    status: &str,
    report: shared::types::BackupReport,
) -> bool {
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
        run_id: report.run_id,
    };
    match db::insert_backup_report(&state.pool, &params).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(hostname = %hostname, error = %e, "failed to persist backup report");
            false
        }
    }
}

/// Queues content indexing for the just-persisted archive (if the backup
/// succeeded or warned and actually persisted), and clears any pending
/// relocation flag for the host on the same condition.
async fn handle_post_backup_report_side_effects(
    state: &AppState,
    hostname: &str,
    repo_id: i64,
    report_persisted: bool,
    succeeded_or_warned: bool,
    index_archive_name: Option<String>,
) {
    if report_persisted
        && succeeded_or_warned
        && let Some(archive_name) = index_archive_name
    {
        spawn_post_backup_indexing(state, repo_id, archive_name);
    }

    if succeeded_or_warned
        && let Err(e) = db::clear_relocation_for_host(&state.pool, repo_id, hostname).await
    {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to clear relocation_pending for host"
        );
    }
}

async fn handle_backup_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    report: shared::types::BackupReport,
) {
    let report_for_ui = report.clone();
    tracing::info!(
        hostname = %hostname,
        repo_id = ?report.repo_id,
        status = ?report.status,
        "backup completed"
    );
    if !validate_agent_repo(
        &state.pool,
        agent_id,
        report.repo_id.0,
        hostname,
        "BackupCompleted",
    )
    .await
    {
        return;
    }

    let repo_id = report.repo_id.0;
    let schedule_id = report.schedule_id;
    let deduplicated_size = report.deduplicated_size;
    let report_status = report.status;

    let outcome_success = !matches!(report_status, shared::types::BackupStatus::Failed);
    let status = match report_status {
        shared::types::BackupStatus::Success => "success",
        shared::types::BackupStatus::Warning => "warning",
        shared::types::BackupStatus::Failed => "failed",
    };
    state.completion_bus.publish(OperationOutcome {
        hostname: hostname.to_owned(),
        repo_id,
        success: outcome_success,
    });

    let notification_error_message = report.error_message.clone();
    let notification_archive_name = report.archive_name.clone();
    let index_archive_name = notification_archive_name.clone();
    let succeeded_or_warned = matches!(
        report_status,
        shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
    );

    let report_persisted =
        persist_backup_completed_report(state, hostname, agent_id, status, report).await;

    handle_post_backup_report_side_effects(
        state,
        hostname,
        repo_id,
        report_persisted,
        succeeded_or_warned,
        index_archive_name,
    )
    .await;

    let repo_name = db::get_repo_name(&state.pool, repo_id)
        .await
        .unwrap_or_else(|_| repo_id.to_string());
    let completed_repo_name = repo_name.clone();

    check_repo_quota_after_backup(
        state,
        hostname,
        agent_id,
        repo_id,
        schedule_id,
        deduplicated_size,
        &repo_name,
    )
    .await;
    check_server_quota_after_backup(
        state,
        hostname,
        agent_id,
        repo_id,
        schedule_id,
        deduplicated_size,
        &repo_name,
    )
    .await;

    dispatch_backup_completion_notification(
        state,
        report_status,
        hostname,
        repo_name,
        status,
        notification_error_message,
        repo_id,
        agent_id,
        schedule_id,
        notification_archive_name,
    )
    .await;

    if succeeded_or_warned {
        spawn_post_backup_sync(state, repo_id);
    }

    finalize_backup_completion(state, hostname, repo_id, report_for_ui, completed_repo_name).await;
}

async fn handle_backup_rejected(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    repo_id: shared::types::RepoId,
    reason: &str,
) {
    tracing::warn!(
        hostname = %hostname,
        repo_id = ?repo_id,
        reason = %reason,
        "backup rejected by agent"
    );
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "BackupRejected").await {
        return;
    }
    state.completion_bus.publish(OperationOutcome {
        hostname: hostname.to_owned(),
        repo_id: repo_id.0,
        success: false,
    });
}

struct CheckCompletedArgs<'a> {
    hostname: &'a str,
    agent_id: i64,
    state: &'a AppState,
    repo_id: shared::types::RepoId,
    success: bool,
    duration_secs: i64,
    error_message: Option<String>,
}

async fn handle_check_completed(args: CheckCompletedArgs<'_>) {
    let CheckCompletedArgs {
        hostname,
        agent_id,
        state,
        repo_id,
        success,
        duration_secs,
        error_message,
    } = args;

    tracing::info!(
        hostname = %hostname,
        repo_id = ?repo_id,
        success,
        duration_secs,
        "check completed"
    );
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "CheckCompleted").await {
        return;
    }
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
    let schedule =
        db::get_schedule_for_hostname_repo(&state.pool, hostname, repo_id.0, ScheduleType::Check)
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
    let task_guard = state.background_task_tracker.begin();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        if let Err(e) = notifications::dispatch(&service, event).await {
            tracing::error!(error = %e, "notification dispatch failed");
        }
    });

    state.ui_broadcast.send(ServerToUi::DataChanged);
}

struct VerifyCompletedArgs<'a> {
    hostname: &'a str,
    agent_id: i64,
    state: &'a AppState,
    repo_id: shared::types::RepoId,
    success: bool,
    duration_secs: i64,
    error_message: Option<String>,
    files_verified: i64,
}

async fn handle_verify_completed(args: VerifyCompletedArgs<'_>) {
    let VerifyCompletedArgs {
        hostname,
        agent_id,
        state,
        repo_id,
        success,
        duration_secs,
        error_message,
        files_verified,
    } = args;

    tracing::info!(
        hostname = %hostname,
        repo_id = ?repo_id,
        success,
        duration_secs,
        files_verified,
        "verify completed"
    );
    if !validate_agent_repo(
        &state.pool,
        agent_id,
        repo_id.0,
        hostname,
        "VerifyCompleted",
    )
    .await
    {
        return;
    }
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

struct CanaryVerifiedArgs<'a> {
    hostname: &'a str,
    agent_id: i64,
    state: &'a AppState,
    repo_id: shared::types::RepoId,
    success: bool,
    nonce: String,
    archive_name: String,
    error_message: Option<String>,
}

async fn handle_canary_verified(args: CanaryVerifiedArgs<'_>) {
    let CanaryVerifiedArgs {
        hostname,
        agent_id,
        state,
        repo_id,
        success,
        nonce,
        archive_name,
        error_message,
    } = args;

    tracing::info!(
        hostname = %hostname,
        repo_id = ?repo_id,
        success,
        "canary verification completed"
    );
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "CanaryVerified").await {
        return;
    }
    let schedule_id =
        db::get_schedule_for_hostname_repo(&state.pool, hostname, repo_id.0, ScheduleType::Backup)
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

async fn handle_backup_cancelled(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    repo_id: shared::types::RepoId,
) {
    tracing::info!(
        hostname = %hostname,
        repo_id = ?repo_id,
        "backup cancelled by agent"
    );
    if !validate_agent_repo(
        &state.pool,
        agent_id,
        repo_id.0,
        hostname,
        "BackupCancelled",
    )
    .await
    {
        return;
    }
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
    if let Err(e) = db::acknowledge_cancellation(&state.pool, agent_id, repo_id.0).await {
        tracing::error!(
            hostname = %hostname,
            repo_id = ?repo_id,
            error = %e,
            "failed to acknowledge cancellation"
        );
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
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
        types::{AgentId, BackupReport, BackupStatus, QuotaAction, RepoId, ReportId},
    };
    use sqlx::PgPool;
    use tokio::time::timeout;

    use super::*;

    #[test]
    fn quota_status_label_covers_every_status() {
        assert_eq!(quota_status_label(db::quota::QuotaStatus::Ok), "ok");
        assert_eq!(
            quota_status_label(db::quota::QuotaStatus::Warning),
            "warning"
        );
        assert_eq!(
            quota_status_label(db::quota::QuotaStatus::Critical),
            "critical"
        );
    }

    #[test]
    fn quota_event_type_covers_every_status() {
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Ok),
            EventType::BackupSuccess
        ));
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Warning),
            EventType::BackupWarning
        ));
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Critical),
            EventType::BackupFailed
        ));
    }

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
            notification_service: crate::notifications::NotificationService::new(pool),
            completion_bus: crate::ws::completion_bus::CompletionBus::new(),
            repo_op_tracker: crate::repo_op_tracker::RepoOpTracker::default(),
            background_task_tracker: crate::background_tasks::BackgroundTaskTracker::default(),
            repo_lock: crate::RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: crate::new_pending_map(),
            pending_restores: crate::new_pending_map(),
            pending_migrations: crate::new_pending_map(),
            pending_deletes: crate::new_pending_map(),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            client_ip_resolver: crate::client_ip::ClientIpResolver::new(),
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

    #[ignore = "requires DATABASE_URL"]
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

        // Link agent to repo via a schedule so the validate_agent_repo guard passes
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                name: "test-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule target");

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
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(5))
                .unwrap(),
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

        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(5))
                .await,
            "timed out waiting for background tasks to finish"
        );
    }

    /// Create a test agent+repo+schedule triple linked via `schedule_targets`.
    /// Returns (agent, repo, schedule).
    async fn create_agent_repo_schedule(
        pool: &PgPool,
    ) -> (
        crate::db::AgentRow,
        crate::db::RepoRow,
        crate::db::ScheduleRow,
    ) {
        let agent = crate::db::insert_agent(pool, "test-handler-host", None, "hash", None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "handler-test-repo",
                repo_path: "/backups/handler-test",
                ssh_user: "user",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                name: "test-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule target");
        (agent, repo, schedule)
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn validate_agent_repo_rejects_rogue_agent(pool: PgPool) {
        let (assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent = crate::db::insert_agent(&pool, "rogue-agent", None, "rogue-hash", None)
            .await
            .expect("insert rogue agent");

        // Assigned agent passes
        assert!(
            validate_agent_repo(
                &pool,
                assigned_agent.id,
                assigned_repo.id,
                &assigned_agent.hostname,
                "Test",
            )
            .await
        );

        // Rogue agent is rejected
        assert!(
            !validate_agent_repo(
                &pool,
                rogue_agent.id,
                assigned_repo.id,
                &rogue_agent.hostname,
                "BackupStarted",
            )
            .await
        );

        // A security_violation event was logged
        let events = crate::db::get_system_events(&pool, 10)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "security_violation")
            .collect();
        assert_eq!(security_events.len(), 1);
        assert!(
            security_events
                .first()
                .unwrap()
                .message
                .contains("rogue-agent")
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_started_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent = crate::db::insert_agent(&pool, "rogue-backup-agent", None, "hash", None)
            .await
            .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupStarted {
            repo_id: RepoId(assigned_repo.id),
            schedule_id: None,
            started_at: chrono::Utc::now(),
            borg_command: Some("borg create --compression lz4 ::archive-name".into()),
            run_id: None,
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        // Verify no backup_report was created for the rogue agent
        let reports = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM backup_reports WHERE agent_id = $1",
            rogue_agent.id,
        )
        .fetch_one(&pool)
        .await
        .expect("query reports");
        assert_eq!(reports.unwrap_or(0), 0);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_log_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent = crate::db::insert_agent(&pool, "rogue-log-agent", None, "hash", None)
            .await
            .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupLog {
            repo_id: RepoId(assigned_repo.id),
            schedule_id: None,
            line: "some log line".into(),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        // Verify a security_violation was logged (BackupLog is guarded)
        let events = crate::db::get_system_events(&pool, 10)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "security_violation")
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_cancelled_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent = crate::db::insert_agent(&pool, "rogue-cancel-agent", None, "hash", None)
            .await
            .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupCancelled {
            repo_id: RepoId(assigned_repo.id),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "security_violation")
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_rejected_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent = crate::db::insert_agent(&pool, "rogue-reject-agent", None, "hash", None)
            .await
            .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupRejected {
            repo_id: RepoId(assigned_repo.id),
            reason: "security test".into(),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "security_violation")
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    fn backup_completed_message(agent_id: i64, repo_id: i64, deduplicated_size: i64) -> String {
        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent_id),
            repo_id: RepoId(repo_id),
            schedule_id: None,
            started_at,
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(5))
                .unwrap(),
            status: BackupStatus::Success,
            original_size: deduplicated_size,
            compressed_size: deduplicated_size,
            deduplicated_size,
            repo_unique_csize: deduplicated_size,
            files_processed: 3,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: None,
            borg_command: None,
            run_id: None,
        };
        serde_json::to_string(&AgentToServer::BackupCompleted { report })
            .expect("serialize message")
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_repo_quota_breach(pool: PgPool) {
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
                name: "quota-repo",
                repo_path: "/backups/quota",
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
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                name: "quota-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::quota::upsert_quota(
            &pool,
            repo.id,
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert quota");

        let state = build_test_state(pool.clone());
        let msg = backup_completed_message(agent.id, repo.id, 200);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(5))
                .await,
            "timed out waiting for background tasks to finish"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_server_quota_breach(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo_a = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-a",
                repo_path: "/backups/shared-a",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .expect("insert repo a");
        let repo_b = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-b",
                repo_path: "/backups/shared-b",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .expect("insert repo b");
        let schedule_a = crate::db::insert_schedule(
            &pool,
            repo_a.id,
            &crate::db::ScheduleParams {
                name: "shared-schedule-a",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule a");
        crate::db::insert_schedule_targets(&pool, schedule_a.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        let schedule_b = crate::db::insert_schedule(
            &pool,
            repo_b.id,
            &crate::db::ScheduleParams {
                name: "shared-schedule-b",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule b");
        crate::db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::server_quota::upsert_server_quota(
            &pool,
            "shared.local",
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert server quota");

        let state = build_test_state(pool.clone());
        let msg = backup_completed_message(agent.id, repo_a.id, 200);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule_b.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(5))
                .await,
            "timed out waiting for background tasks to finish"
        );
    }
}
