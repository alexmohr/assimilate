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
    types::{BackupStatus, ScheduleType, SystemEventType},
};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AppState,
    api::repos::sync_new_archives,
    archive_index, catch_up, config_assembler, db,
    notifications::{self, EventType, NotificationEvent},
    pending::{Claim, PendingRequests},
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

/// Verifies `token` against every agent registered under `hostname` and
/// returns the ID of the one it matches, if any.
///
/// More than one agent can share a hostname (agents in different domains),
/// and a connecting agent only ever reports its OS hostname -- never which
/// domain it belongs to -- so the presented token, not the hostname alone,
/// is what identifies which specific agent this connection is for.
pub(crate) async fn verify_agent_token(pool: &PgPool, hostname: &str, token: &str) -> Option<i64> {
    let candidates = db::get_agent_token_hashes(pool, hostname).await.ok()?;
    for candidate in candidates {
        let token = token.to_owned();
        let hash = candidate.agent_token_hash;
        let verified = tokio::task::spawn_blocking(move || bcrypt::verify(&token, &hash))
            .await
            .unwrap_or(Ok(false))
            .unwrap_or(false);
        if verified {
            return Some(candidate.id);
        }
    }
    None
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
    let Some(agent_id) = verify_agent_token(pool, hostname, &token).await else {
        tracing::warn!(hostname = %hostname, "invalid agent token or unknown agent");
        if let Err(e) = db::insert_system_event(
            pool,
            SystemEventType::AuthFailed,
            Some(hostname),
            &format!("Invalid token for agent '{hostname}'"),
        )
        .await
        {
            tracing::error!(error = %e, "failed to insert system event");
        }
        send_close(ws_sink, "authentication failed").await;
        return None;
    };

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
    let replaced_connection = state
        .registry
        .register(
            agent_id,
            outbound_tx,
            supports_restart,
            restart_unavailable_reason,
        )
        .await;

    tracing::info!(hostname = %hostname, "agent connected");

    reenable_system_disabled_schedules_on_reconnect(&state, agent_id, &hostname).await;

    // After the re-enable pass, never before it: a schedule this host's outage
    // auto-disabled is only eligible for its pending catch-up once it is enabled
    // again.
    catch_up::run_catch_ups_on_reconnect(&state, agent_id, &hostname).await;

    if replaced_connection {
        abandon_stale_operations_on_reconnect(&state, agent_id, &hostname).await;
    }

    state.ui_broadcast.send(ServerToUi::AgentConnected {
        hostname: hostname.clone(),
    });

    send_reconnect_catchup(&state, &mut ws_sink, &hostname, agent_id).await;

    tokio::spawn(ping_loop(ping_tx, state.shutdown_token.clone()));

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
            () = state.shutdown_token.cancelled() => {
                tracing::debug!(hostname = %hostname, "shutdown, closing agent connection");
                break;
            }
        }
    }

    state.registry.unregister(agent_id).await;
    let cleared = state.repo_op_tracker.clear_for_agent(agent_id).await;
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

/// Called right after [`AgentRegistry::register`] reports that this
/// connection replaced an already-registered one for the same hostname. The
/// previous session is gone for good, so any backup it had in flight (on any
/// repo, not just whatever this new connection does next) is abandoned:
///
/// - Marks the stale `backup_reports` rows `failed` (already visible on the
///   dashboard's Needs Attention panel via the existing `backup_failed`
///   finding, and in the activity log - no separate alerting needed).
/// - Publishes a failure to the completion bus for each affected repo, so a
///   scheduler task still parked in `wait_for_completion` for the old
///   session wakes up immediately instead of waiting forever. Without this,
///   `AgentRegistry::is_connected` can't tell the new session apart from the
///   old one (same hostname key), so the connectivity poll in
///   `wait_for_completion` never notices anything is wrong - the task keeps
///   holding its `RepoLock` guard indefinitely, which can wedge the entire
///   scheduler (`tick()` awaits each due schedule's dispatch in turn) even
///   for repos this agent has nothing to do with.
async fn abandon_stale_operations_on_reconnect(state: &AppState, agent_id: i64, hostname: &str) {
    match db::fail_started_backups_for_agent_reconnect(&state.pool, agent_id, hostname).await {
        Ok(repo_ids) => {
            for repo_id in repo_ids {
                tracing::warn!(
                    hostname = %hostname,
                    repo_id,
                    "agent reconnected while a backup for this repo was still in flight; \
                     abandoning it"
                );
                state.completion_bus.publish(OperationOutcome {
                    agent_id,
                    repo_id,
                    success: false,
                });
            }
        }
        Err(e) => {
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to abandon stale in-flight backups on agent reconnect"
            );
        }
    }
}

/// Called on every successful agent connection. If the scheduler had auto-disabled
/// any schedule this agent targets after repeated unreachable-agent failures (see
/// `scheduler::record_schedule_failure_once`), re-enables it and makes it due again
/// immediately, so backups resume on their own instead of staying off until a human
/// notices and flips the switch back on. Never touches a schedule a human or quota
/// enforcement disabled for an unrelated reason.
///
/// Considers a candidate schedule on *any* of its targets reconnecting, not just the
/// one `auto_disabled_by_agent_id` happens to name (see
/// [`db::list_auto_disabled_schedule_ids_for_agent`]'s doc comment for why), but only
/// actually re-enables it once every one of its targets is currently connected -
/// `auto_disabled_by_agent_id` alone isn't enough to know that, since it only names
/// whichever target happened to be first-recorded on the disabling tick, not every
/// target that contributed to the (schedule-wide) failure streak.
async fn reenable_system_disabled_schedules_on_reconnect(
    state: &AppState,
    agent_id: i64,
    hostname: &str,
) {
    let candidates =
        match db::list_auto_disabled_schedule_ids_for_agent(&state.pool, agent_id).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to list auto-disabled schedules on agent reconnect"
                );
                return;
            }
        };
    if candidates.is_empty() {
        return;
    }

    // A multi-target schedule's auto_disabled_by_agent_id only ever names whichever
    // target happened to be first-recorded on the disabling tick, not every target
    // that contributed to the (schedule-wide) failure streak. Re-enabling here
    // unconditionally could resume a schedule whose *other* target is still fully
    // unreachable, reproducing the same unbounded-retry problem for that target - so
    // only actually re-enable a candidate once every one of its targets is currently
    // connected, not just this reconnecting one.
    let targets_by_schedule =
        match db::get_schedule_target_agent_ids_by_schedule(&state.pool, &candidates).await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to look up target agents for auto-disabled schedules"
                );
                return;
            }
        };
    let mut safe_to_reenable = Vec::with_capacity(candidates.len());
    for schedule_id in candidates {
        let targets = targets_by_schedule
            .get(&schedule_id)
            .map_or(&[][..], |v| v.as_slice());
        let mut all_connected = true;
        for target_agent_id in targets {
            if !state.registry.is_connected(*target_agent_id).await {
                all_connected = false;
                break;
            }
        }
        if all_connected {
            safe_to_reenable.push(schedule_id);
        } else {
            tracing::info!(
                hostname = %hostname,
                schedule_id,
                "not re-enabling on reconnect - at least one other target of this schedule is \
                 still unreachable"
            );
        }
    }
    if safe_to_reenable.is_empty() {
        return;
    }

    match db::reenable_specific_schedules(&state.pool, &safe_to_reenable, chrono::Utc::now()).await
    {
        Ok(schedule_ids) if !schedule_ids.is_empty() => {
            tracing::info!(
                hostname = %hostname,
                ?schedule_ids,
                "agent reconnected; re-enabling schedules that were auto-disabled while it was \
                 unreachable"
            );
            let msg = format!(
                "{} schedule(s) re-enabled now that agent '{hostname}' reconnected: {:?}",
                schedule_ids.len(),
                schedule_ids
            );
            if let Err(e) = db::insert_system_event(
                &state.pool,
                SystemEventType::ScheduleReenabled,
                Some(hostname),
                &msg,
            )
            .await
            {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to record schedule-reenabled system event"
                );
            }
            state.ui_broadcast.send(ServerToUi::DataChanged);
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to re-enable auto-disabled schedules on agent reconnect"
            );
        }
    }
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
    match config_assembler::assemble_config(&state.pool, &state.encryption_key, agent_id).await {
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

async fn ping_loop(sender: mpsc::Sender<ServerToAgent>, shutdown_token: CancellationToken) {
    let mut interval = tokio::time::interval(PING_INTERVAL);
    loop {
        tokio::select! {
            biased;
            () = shutdown_token.cancelled() => return,
            _ = interval.tick() => {}
        }
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
        SystemEventType::SecurityViolation,
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
        run_id: None,
        duration_secs: None,
        original_size: None,
        compressed_size: None,
        deduplicated_size: None,
        files_processed: None,
        warnings: Vec::new(),
        next_run_at: None,
        activity_url: None,
    };
    spawn_notification_dispatch(state, quota_event);
}

/// Spawns `notifications::dispatch` for `event` as a detached task registered with
/// `task_registry`, so shutdown can join it instead of the runtime abandoning a
/// still-in-flight webhook/email/push delivery. Shared by every call site that builds
/// a [`NotificationEvent`] and fires it off in the background.
fn spawn_notification_dispatch(state: &AppState, event: NotificationEvent) {
    let service = state.notification_service.clone();
    let task_registry = state.task_registry.clone();
    state.background_task_tracker.spawn_tracked(async move {
        if let Err(e) = notifications::dispatch(&service, event, &task_registry).await {
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
        AgentToServer::VmScanResult {
            request_id,
            vms,
            error,
        } => {
            handle_vm_scan_result(hostname, agent_id, state, request_id, vms, error).await;
        }
        AgentToServer::VmSnapshotReport {
            schedule_id,
            outcomes,
        } => {
            tracing::info!(
                hostname = %hostname,
                schedule_id = ?schedule_id,
                domains = outcomes.len(),
                "agent staged virtual machines"
            );
            if let Err(e) = db::vms::record_outcomes(&state.pool, agent_id, &outcomes).await {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to record virtual machine snapshot outcomes"
                );
            }
        }
        AgentToServer::VmBuildResult {
            request_id,
            outcome,
            error,
        } => {
            if let Some(reason) = error.as_deref() {
                tracing::warn!(
                    hostname = %hostname,
                    error = %reason,
                    "virtual machine build failed on the agent"
                );
            }
            answer_pending(
                &state.pending_vm_builds,
                &request_id,
                agent_id,
                hostname,
                (outcome, error),
            )
            .await;
        }
        AgentToServer::VmStageResult {
            request_id,
            outcome,
        } => {
            handle_vm_stage_result(hostname, agent_id, state, request_id, outcome).await;
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
                agent_id,
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
                agent_id,
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
                agent_id,
                state,
                request_id,
                files,
                total_size,
                error_message,
            )
            .await;
        }
        AgentToServer::OperationFailed { request_id, error } => {
            handle_operation_failed(hostname, agent_id, state, request_id, error).await;
        }
        AgentToServer::DeleteArchivesResult {
            request_id,
            success,
            deleted_count,
            error_message,
        } => {
            handle_delete_archives_result(
                hostname,
                agent_id,
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

/// Records the domains an agent reported and hands them to whoever asked for
/// the scan. A scan that failed still resolves the waiter, so the UI reports
/// the reason rather than timing out.
async fn handle_vm_scan_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: Option<String>,
    vms: Vec<shared::vm::DiscoveredVm>,
    error: Option<String>,
) {
    if let Some(reason) = error.as_deref() {
        tracing::warn!(
            hostname = %hostname,
            error = %reason,
            "virtual machine scan failed on the agent"
        );
    } else if let Err(e) = db::vms::record_scan(&state.pool, agent_id, &vms).await {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to record the virtual machines an agent reported"
        );
    }

    if let Some(request_id) = request_id {
        answer_pending(
            &state.pending_vm_scans,
            &request_id,
            agent_id,
            hostname,
            (vms, error),
        )
        .await;
    }
}

/// Records what staging one domain right now did to it, and hands the
/// outcome to whoever asked for the snapshot. Recorded whether or not it
/// carries an error, so the domain's row shows the failure on the next load
/// even though the request itself reports it as failed.
async fn handle_vm_stage_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: Option<String>,
    outcome: shared::vm::VmSnapshotOutcome,
) {
    if let Some(reason) = outcome.error.as_deref() {
        tracing::warn!(
            hostname = %hostname,
            domain = %outcome.name,
            error = %reason,
            "manual virtual machine snapshot failed on the agent"
        );
    }
    if let Err(e) =
        db::vms::record_outcomes(&state.pool, agent_id, std::slice::from_ref(&outcome)).await
    {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to record a manual virtual machine snapshot outcome"
        );
    }

    if let Some(request_id) = request_id {
        answer_pending(
            &state.pending_vm_stages,
            &request_id,
            agent_id,
            hostname,
            outcome,
        )
        .await;
    }
}

/// Hands `answer` to whoever waits on `request_id` in `pending`, provided
/// `agent_id` is the agent the request was sent to. Returns `false` when no
/// request is waiting there under that id.
///
/// An answer to a request that was sent to another agent is dropped with a
/// warning and leaves that request pending for its own agent, so one agent
/// cannot fulfill or fail another agent's operation by sending its request
/// id.
async fn answer_pending<T>(
    pending: &PendingRequests<T>,
    request_id: &str,
    agent_id: i64,
    hostname: &str,
    answer: T,
) -> bool {
    match pending.claim(request_id, agent_id).await {
        Claim::Claimed(tx) => {
            let _ = tx.send(answer);
            true
        }
        Claim::WrongAgent { expected_agent_id } => {
            tracing::warn!(
                hostname = %hostname,
                agent_id,
                expected_agent_id,
                request_id = %request_id,
                "ignoring an answer to a request that was sent to another agent"
            );
            true
        }
        Claim::Unknown => false,
    }
}

async fn handle_restore_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    files_restored: u64,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_restores,
        &request_id,
        agent_id,
        hostname,
        (success, files_restored, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected RestoreCompleted with no pending request"
        );
    }
}

async fn handle_migrate_encryption_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_migrations,
        &request_id,
        agent_id,
        hostname,
        (success, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected MigrateEncryptionCompleted with no pending request"
        );
    }
}

async fn handle_dry_run_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    files: Vec<shared::types::DryRunFile>,
    total_size: i64,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_dryruns,
        &request_id,
        agent_id,
        hostname,
        (files, total_size, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected DryRunResult with no pending request"
        );
    }
}

async fn handle_operation_failed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    error: String,
) {
    let answered = answer_pending(
        &state.pending_dryruns,
        &request_id,
        agent_id,
        hostname,
        (Vec::new(), 0, Some(error.clone())),
    )
    .await
        || answer_pending(
            &state.pending_restores,
            &request_id,
            agent_id,
            hostname,
            (false, 0, Some(error.clone())),
        )
        .await
        || answer_pending(
            &state.pending_deletes,
            &request_id,
            agent_id,
            hostname,
            (false, 0, Some(error)),
        )
        .await;
    if !answered {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected OperationFailed with no pending request"
        );
    }
}

async fn handle_delete_archives_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    deleted_count: u32,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_deletes,
        &request_id,
        agent_id,
        hostname,
        (success, deleted_count, error_message),
    )
    .await
    {
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
            Some(agent_id),
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
            started_at,
        });
        state.ui_broadcast.send(ServerToUi::BackupStarted {
            hostname: hostname.to_owned(),
            target_name,
            archive_name,
            schedule_id,
            started_at,
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
    let task_registry = state.task_registry.clone();
    state.background_task_tracker.spawn_tracked(async move {
        match archive_index::ensure_indexed(
            pool,
            encryption_key,
            repo_id,
            archive_name.clone(),
            repo_lock,
            &background_task_tracker,
            task_registry,
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

/// Runs both quota checks (the repo's own quota and, if it shares an SSH host with other
/// repos, the combined server quota) after a backup completes. The two are independent --
/// each reads its own settings and dispatches its own notification/enforcement -- so they
/// run concurrently via [`tokio::join!`] instead of paying their DB/notification latency
/// twice in sequence.
async fn check_quotas_after_backup(
    state: &AppState,
    hostname: &str,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    repo_unique_csize: i64,
    repo_name: &str,
) {
    tokio::join!(
        check_repo_quota_after_backup(
            state,
            hostname,
            agent_id,
            repo_id,
            schedule_id,
            repo_unique_csize,
            repo_name,
        ),
        check_server_quota_after_backup(
            state,
            hostname,
            agent_id,
            repo_id,
            schedule_id,
            repo_unique_csize,
            repo_name,
        ),
    );
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
    repo_unique_csize: i64,
    repo_name: &str,
) {
    let Ok(Some(quota)) = db::quota::get_quota(&state.pool, repo_id).await else {
        return;
    };
    let quota_status = db::quota::evaluate_quota(&quota, repo_unique_csize);
    if matches!(quota_status, db::quota::QuotaStatus::Ok) {
        return;
    }
    tracing::warn!(
        hostname = %hostname,
        repo_id,
        repo_unique_csize,
        quota_status = quota_status_label(quota_status),
        "repository quota exceeded"
    );

    let message = format!(
        "Repository quota {} for repo {repo_name}: current size {repo_unique_csize} bytes exceeds \
         configured limits",
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
    repo_unique_csize: i64,
    repo_name: &str,
) {
    let Ok(ssh_host) = db::get_repo_ssh_host(&state.pool, repo_id).await else {
        return;
    };
    let Ok(Some(server_quota)) = db::server_quota::get_server_quota(&state.pool, &ssh_host).await
    else {
        return;
    };
    let Ok(siblings_deduplicated_size) = db::server_quota::total_deduplicated_size_for_ssh_host(
        &state.pool,
        &ssh_host,
        Some(repo_id),
    )
    .await
    else {
        return;
    };

    // Combine the just-completed backup's fresh `repo_unique_csize` (this repo's
    // current repo-wide deduplicated size) with the (possibly stale, since
    // `repo_stats` is only refreshed by a sync/rescan) snapshot for sibling repos
    // on the host, so a breach on an otherwise idle host is caught immediately
    // rather than only after an unrelated rescan.
    let total_deduplicated_size = siblings_deduplicated_size.saturating_add(repo_unique_csize);
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

/// Context for [`dispatch_backup_completion_notification`], gathered by its caller from a
/// `BackupReport` before that report is moved into `persist_backup_completed_report`.
struct BackupCompletionNotificationArgs<'a> {
    status: shared::types::BackupStatus,
    hostname: &'a str,
    repo_name: String,
    status_str: &'a str,
    error_message: Option<String>,
    repo_id: i64,
    agent_id: i64,
    schedule_id: Option<i64>,
    archive_name: Option<String>,
    run_id: Option<String>,
    duration_secs: i64,
    original_size: i64,
    compressed_size: i64,
    deduplicated_size: i64,
    files_processed: i64,
    warnings: Vec<String>,
}

/// Which event a *failed* backup is reported as.
///
/// Only a repository marked as not always online can turn a failure into
/// anything else. For one that is not marked - a server that should always be
/// there - an unreachable host is exactly the failure borg reported, and is
/// left as one without so much as a probe. For one that is marked, a failure
/// whose host does not answer SSH is a skipped backup rather than a failed
/// one: its absence was expected, and the run is caught up once it is back.
///
/// Decided here rather than alongside the failure, so exactly one event
/// describes the run. Reporting the skip separately would leave every such
/// failure firing twice - once as `BackupFailed` from this same dispatch, and
/// again as the skip - which is two alerts and two activity rows for one
/// backup.
///
/// The probe only ever runs once a backup has already failed. Asking on the
/// way *in* and refusing to dispatch would mean any hiccup reaching the host -
/// a slow answer, a refused key, a momentary blip - turned a backup that would
/// have run into one that never ran, which is worse than the mislabelling it
/// would be curing.
///
/// Falls back to [`EventType::BackupFailed`] whenever the repository's row
/// can't be read: a repository this server cannot even look up is not one it
/// can call absent, however it is marked.
///
/// Runs on a background task, never on the caller's thread of control: the
/// probe is a live SSH round-trip, and every path here is reached from
/// `handle_agent_message`, which the agent's websocket loop awaits inline.
async fn classify_failed_backup(
    pool: &PgPool,
    repo_id: i64,
    repo_name: &str,
    hostname: &str,
    schedule: Option<(i64, &str)>,
    run_id: Option<&str>,
) -> FailedBackupReport {
    let schedule_name = schedule.map(|(_, name)| name);
    let intermittent = db::catch_up::get_repo_availability(pool, repo_id)
        .await
        .is_ok_and(|availability| availability.intermittent);
    if !intermittent {
        return FailedBackupReport::plain_failure();
    }
    let Ok(repo) = db::get_repo_by_id(pool, repo_id).await else {
        return FailedBackupReport::plain_failure();
    };
    if crate::power::repo_reachable(&repo).await {
        return FailedBackupReport::plain_failure();
    }

    tracing::warn!(
        hostname = %hostname,
        repo_id,
        "backup failed and the repository's host is not answering SSH; reporting it as skipped"
    );
    // The bare reason is what every channel shows, matching what the
    // agent-offline sibling puts in its own `error_message`; the activity log
    // gets the same reason behind a sentence that names the run.
    let reason = format!("the host for repository '{repo_name}' did not answer SSH");
    let msg = schedule_name.map_or_else(
        || format!("Backup failed: {reason}"),
        |name| format!("Backup for schedule '{name}' failed: {reason}"),
    );
    if let Err(e) = db::insert_system_event(
        pool,
        shared::types::SystemEventType::BackupSkippedRepoOffline,
        Some(hostname),
        &msg,
    )
    .await
    {
        tracing::error!(
            repo_id,
            error = %e,
            "failed to record backup-skipped-repo-offline system event"
        );
    }
    if let Some((schedule_id, _)) = schedule {
        mark_repo_catch_up_pending(pool, schedule_id, repo_id, repo_name, run_id).await;
    }
    FailedBackupReport {
        event_type: EventType::BackupSkippedRepoOffline,
        reason: Some(reason),
    }
}

/// Records that this repository owes the schedule a run, so
/// [`crate::repo_catch_up`] picks it up once the host answers again.
///
/// This is the only place that knows a run failed *because* the repository was
/// away, which is why the marker is written here rather than in the scheduler:
/// by the time the tick dispatched, the host still looked fine.
///
/// Only reached for a repository marked as not always online - the caller has
/// already decided that. A run with no schedule behind it (Run now) has
/// nothing to catch up to, so it never gets here either.
async fn mark_repo_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
    repo_name: &str,
    run_id: Option<&str>,
) {
    // The occurrence this run stood for, not the moment it gave up: the
    // give-up window is the wait an operator asked for, measured from when the
    // backup should have happened. Read off the run's own report rather than
    // the schedule's `last_run_at`, which by the time a slow failure comes back
    // may already name a later run of the same schedule.
    let run_started = match run_id {
        Some(run_id) => db::catch_up::run_started_at(pool, run_id, repo_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let due_at = if let Some(started) = run_started {
        started
    } else {
        // A run with no report of its own falls back to the schedule's last
        // run, and a schedule that has somehow never run has no occurrence to
        // name, so the failure itself becomes one.
        let Ok(schedule) = db::get_schedule_by_id(pool, schedule_id).await else {
            return;
        };
        schedule.last_run_at.unwrap_or_else(chrono::Utc::now)
    };
    if let Err(e) =
        db::catch_up::mark_repo_catch_up_pending(pool, schedule_id, repo_id, due_at).await
    {
        tracing::error!(
            schedule_id,
            repo_id,
            error = %e,
            "failed to record a pending repository catch-up"
        );
        return;
    }
    tracing::info!(
        schedule_id,
        repo_id,
        missed_occurrence = %due_at,
        "repository '{repo_name}' was away; the run will be caught up when it answers"
    );
}

/// How a failed backup is reported: the event it is raised as, plus the
/// explanation that stands in for borg's own error when the failure turns out
/// to be a host that was not there.
struct FailedBackupReport {
    event_type: EventType,
    /// `None` leaves the agent's reported error in place, which is the right
    /// thing to show when borg's own message is the explanation. `Some` is the
    /// reason the run is being reported as a skip instead, and replaces it on
    /// every channel - otherwise a "Backup skipped" alert would carry the same
    /// raw connection error a plain failure did, and say nothing the label
    /// didn't already.
    reason: Option<String>,
}

impl FailedBackupReport {
    /// A failure borg itself is the best witness to.
    const fn plain_failure() -> Self {
        Self {
            event_type: EventType::BackupFailed,
            reason: None,
        }
    }
}

/// Dispatches a [`NotificationEvent`] for a completed backup.
async fn dispatch_backup_completion_notification(
    state: &AppState,
    args: BackupCompletionNotificationArgs<'_>,
) {
    let BackupCompletionNotificationArgs {
        status,
        hostname,
        repo_name,
        status_str,
        error_message,
        repo_id,
        agent_id,
        schedule_id,
        archive_name,
        run_id,
        duration_secs,
        original_size,
        compressed_size,
        deduplicated_size,
        files_processed,
        warnings,
    } = args;

    let (schedule_name, next_run_at) = match schedule_id {
        Some(sid) => db::get_schedule_name_and_next_run_at(&state.pool, sid, &repo_name)
            .await
            .map_or((None, None), |(name, next_run_at)| {
                (Some(name), next_run_at)
            }),
        None => (None, None),
    };
    // Captured before the task below, so the notification still carries the
    // moment the backup was reported rather than whenever its classification
    // happened to finish.
    let timestamp = chrono::Utc::now();
    let hostname = hostname.to_owned();
    let status_str = status_str.to_owned();
    let pool = state.pool.clone();
    let service = state.notification_service.clone();
    let task_registry = state.task_registry.clone();
    // Spawned rather than awaited, because classifying a *failed* backup means
    // probing the repository's host, and that is a live SSH round-trip. Every
    // path to here runs inside `handle_agent_message`, which the agent's
    // websocket loop awaits inline, so waiting here would hold up both the
    // next inbound frame from that agent and anything already queued outbound
    // to it - on every failed backup, for a connection that has done nothing
    // wrong. The same reasoning that keeps the probe off the dispatch path
    // keeps it off this one.
    state.background_task_tracker.spawn_tracked(async move {
        let (event_type, error_message) = match status {
            shared::types::BackupStatus::Success => (EventType::BackupSuccess, error_message),
            shared::types::BackupStatus::Warning => (EventType::BackupWarning, error_message),
            shared::types::BackupStatus::Failed => {
                let schedule = schedule_id.zip(schedule_name.as_deref());
                let report = classify_failed_backup(
                    &pool,
                    repo_id,
                    &repo_name,
                    &hostname,
                    schedule,
                    run_id.as_deref(),
                )
                .await;
                // The classified reason wins where there is one: it is why the
                // run is being called a skip at all.
                (report.event_type, report.reason.or(error_message))
            }
        };
        let event = NotificationEvent {
            event_type,
            hostname,
            repo_name,
            status: status_str,
            error_message,
            timestamp,
            repo_id: Some(repo_id),
            agent_id: Some(agent_id),
            schedule_id,
            schedule_name,
            archive_name,
            run_id,
            duration_secs: Some(duration_secs),
            original_size: Some(original_size),
            compressed_size: Some(compressed_size),
            deduplicated_size: Some(deduplicated_size),
            files_processed: Some(files_processed),
            warnings,
            next_run_at,
            activity_url: None,
        };
        if let Err(e) = notifications::dispatch(&service, event, &task_registry).await {
            tracing::error!(error = %e, "notification dispatch failed");
        }
    });
}

/// Runs the post-backup archive sync in the background: marks the repo as
/// importing, syncs any new archives, and clears importing/error state
/// regardless of outcome. Tracking is the caller's job (see
/// `spawn_post_backup_sync`): claiming the guard in this body would only
/// happen once the runtime first polls the spawned task, which is the race
/// the tracker exists to close.
async fn run_post_backup_sync(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    ui_broadcast: crate::ws::ui_broadcast::UiBroadcast,
    repo_lock: crate::RepoLock,
    background_task_tracker: crate::background_tasks::BackgroundTaskTracker,
    task_registry: shared::task_registry::TaskRegistry,
) {
    // Held for the rest of this function so a panic inside sync_new_archives
    // still clears repo_import_state.importing (via spawned cleanup, since
    // Drop can't await) instead of leaving it permanently "importing" - see
    // db::ImportingGuard.
    let importing_guard = match db::ImportingGuard::acquire(&pool, repo_id, task_registry.clone())
        .await
    {
        Ok(guard) => guard,
        Err(e) => {
            tracing::error!(repo_id, error = %e, "post-backup sync: failed to set importing flag");
            return;
        }
    };
    match sync_new_archives(
        &pool,
        &encryption_key,
        repo_id,
        &ui_broadcast,
        &repo_lock,
        &background_task_tracker,
        &task_registry,
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
            importing_guard.clear_now().await;
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
            importing_guard.clear_now().await;
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
    state
        .background_task_tracker
        .spawn_tracked(run_post_backup_sync(
            state.pool.clone(),
            state.encryption_key,
            repo_id,
            state.ui_broadcast.clone(),
            state.repo_lock.clone(),
            state.background_task_tracker.clone(),
            state.task_registry.clone(),
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
    status: BackupStatus,
    report: shared::types::BackupReport,
) -> bool {
    let params = db::InsertReportParams {
        agent_id,
        repo_id: report.repo_id.0,
        schedule_id: report.schedule_id,
        started_at: report.started_at,
        finished_at: report.finished_at,
        status,
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
    let repo_unique_csize = report.repo_unique_csize;
    let report_status = report.status;

    let outcome_success = !matches!(report_status, shared::types::BackupStatus::Failed);
    state.completion_bus.publish(OperationOutcome {
        agent_id,
        repo_id,
        success: outcome_success,
    });

    let notification_error_message = report.error_message.clone();
    let notification_archive_name = report.archive_name.clone();
    let index_archive_name = notification_archive_name.clone();
    let notification_run_id = report.run_id.clone();
    let notification_duration_secs = report.duration_secs;
    let notification_original_size = report.original_size;
    let notification_compressed_size = report.compressed_size;
    let notification_deduplicated_size = report.deduplicated_size;
    let notification_files_processed = report.files_processed;
    let notification_warnings = report.warnings.clone();
    let succeeded_or_warned = matches!(
        report_status,
        shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
    );

    let report_persisted =
        persist_backup_completed_report(state, hostname, agent_id, report_status, report).await;

    let status_str = &report_status.to_string();

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

    check_quotas_after_backup(
        state,
        hostname,
        agent_id,
        repo_id,
        schedule_id,
        repo_unique_csize,
        &repo_name,
    )
    .await;

    dispatch_backup_completion_notification(
        state,
        BackupCompletionNotificationArgs {
            status: report_status,
            hostname,
            repo_name,
            status_str,
            error_message: notification_error_message,
            repo_id,
            agent_id,
            schedule_id,
            archive_name: notification_archive_name,
            run_id: notification_run_id,
            duration_secs: notification_duration_secs,
            original_size: notification_original_size,
            compressed_size: notification_compressed_size,
            deduplicated_size: notification_deduplicated_size,
            files_processed: notification_files_processed,
            warnings: notification_warnings,
        },
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
        agent_id,
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
        agent_id,
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
    let next_run_at = schedule.as_ref().and_then(|s| s.next_run_at);

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
        run_id: None,
        duration_secs: Some(duration_secs),
        original_size: None,
        compressed_size: None,
        deduplicated_size: None,
        files_processed: None,
        warnings: Vec::new(),
        next_run_at,
        activity_url: None,
    };
    spawn_notification_dispatch(state, event);

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
        agent_id,
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
        agent_id,
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
        types::{
            AcknowledgedFilter, AgentId, BackupReport, BackupStatus, QuotaAction, RepoId, ReportId,
            ScheduleWakeOverride,
        },
    };
    use sqlx::PgPool;
    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test]
    async fn ping_loop_exits_promptly_when_shutdown_token_cancelled() {
        let (tx, mut rx) = mpsc::channel::<ServerToAgent>(4);
        let token = CancellationToken::new();

        let handle = tokio::spawn(ping_loop(tx, token.clone()));
        token.cancel();

        let result = timeout(Duration::from_secs(5), handle).await;
        assert!(
            result.is_ok(),
            "ping_loop did not exit within 5s after shutdown_token cancellation"
        );
        assert!(rx.try_recv().is_err(), "no ping should have been sent");
    }

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
        crate::test_support::build_test_state(pool, b"handler-test-secret-key")
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
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");

        // Link agent to repo via a schedule so the validate_agent_repo guard passes
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "test-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule target");

        let _gate = crate::borg::acquire_test_binary_gate().await;
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
                if matches!(archive_2, Some(shared::types::IndexStatus::Done)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("timed out waiting for archive indexing");

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
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
        let agent = crate::db::insert_agent(pool, "test-handler-host", None, "hash", None, None)
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "test-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
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
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-agent", None, "rogue-hash", None, None)
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
        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
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
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-backup-agent", None, "hash", None, None)
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

    /// A manual snapshot's result is recorded into `agent_vms` and handed to
    /// whoever is waiting on the request, the same way a scheduled backup's
    /// `VmSnapshotReport` is recorded - but resolving the one-shot besides,
    /// since a manual snapshot has a caller actually waiting on the answer.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_vm_stage_result_records_and_resolves(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-stage-test-host", None, "hash", None, None)
            .await
            .expect("insert agent");

        let state = build_test_state(pool.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_stages
            .insert("req-stage-test".to_owned(), agent.id, tx)
            .await;

        let msg = serde_json::to_string(&AgentToServer::VmStageResult {
            request_id: Some("req-stage-test".into()),
            outcome: shared::vm::VmSnapshotOutcome {
                name: "web01".into(),
                action: shared::vm::VmRunAction::Increment,
                mode: shared::vm::VmSnapshotMode::Incremental,
                staged_bytes: 4096,
                chain_length: 2,
                error: None,
            },
        })
        .expect("serialize");

        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let outcome = rx.await.expect("the waiter is resolved");
        assert_eq!(outcome.name, "web01");
        assert_eq!(outcome.staged_bytes, 4096);

        let row = sqlx::query!(
            "SELECT staged_bytes, chain_length FROM agent_vms WHERE agent_id = $1 AND name = $2",
            agent.id,
            "web01",
        )
        .fetch_one(&pool)
        .await
        .expect("outcome recorded");
        assert_eq!(row.staged_bytes, 4096);
        assert_eq!(row.chain_length, 2);
    }

    /// An answer carrying another agent's request id - a result or an
    /// `OperationFailed` - must not resolve that agent's pending request,
    /// and must leave it pending for the agent it was sent to.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_ignores_answers_from_another_agent(pool: PgPool) {
        let target = crate::db::insert_agent(&pool, "answer-target-host", None, "hash", None, None)
            .await
            .expect("insert target agent");
        let other = crate::db::insert_agent(&pool, "answer-other-host", None, "hash", None, None)
            .await
            .expect("insert other agent");

        let state = build_test_state(pool);
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        state
            .pending_dryruns
            .insert("req-dry-run".to_owned(), target.id, tx)
            .await;

        let forged_result = serde_json::to_string(&AgentToServer::DryRunResult {
            request_id: "req-dry-run".into(),
            files: Vec::new(),
            total_size: 1,
            error_message: None,
        })
        .expect("serialize");
        handle_agent_message(&forged_result, &other.hostname, other.id, &state).await;
        let forged_failure = serde_json::to_string(&AgentToServer::OperationFailed {
            request_id: "req-dry-run".into(),
            error: "forged".into(),
        })
        .expect("serialize");
        handle_agent_message(&forged_failure, &other.hostname, other.id, &state).await;
        assert!(
            rx.try_recv().is_err(),
            "another agent's answers must not resolve the request"
        );

        let genuine = serde_json::to_string(&AgentToServer::DryRunResult {
            request_id: "req-dry-run".into(),
            files: Vec::new(),
            total_size: 42,
            error_message: None,
        })
        .expect("serialize");
        handle_agent_message(&genuine, &target.hostname, target.id, &state).await;
        let (_, total_size, error) = rx.await.expect("the target agent resolves the request");
        assert_eq!(total_size, 42);
        assert_eq!(error, None);
    }

    /// `spawn_post_backup_sync` must mark the task in flight before it returns.
    /// Claiming the guard as the first statement of `run_post_backup_sync`'s own
    /// body looked equivalent but wasn't: calling an async fn runs none of it, so
    /// `any_active()` only turned true once the runtime first polled the spawned
    /// task - and whether that happens before a caller (or a test's runtime
    /// teardown) looks is a scheduling race, not a guarantee.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn spawn_post_backup_sync_is_tracked_before_the_task_is_first_polled(pool: PgPool) {
        let state = build_test_state(pool.clone());

        // No repo with this id exists, so the task fails out immediately without
        // running borg - what it does is irrelevant here, only when it starts
        // counting is.
        spawn_post_backup_sync(&state, 987_654);

        // Deliberately no await between the spawn and this assertion.
        assert!(
            state.background_task_tracker.any_active(),
            "post-backup sync must be tracked the moment it is spawned"
        );

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_log_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-log-agent", None, "hash", None, None)
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
        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_cancelled_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-cancel-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupCancelled {
            repo_id: RepoId(assigned_repo.id),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_rejected_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-reject-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupRejected {
            repo_id: RepoId(assigned_repo.id),
            reason: "security test".into(),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    fn backup_completed_message(agent_id: i64, repo_id: i64, deduplicated_size: i64) -> String {
        backup_completed_message_with_repo_size(
            agent_id,
            repo_id,
            deduplicated_size,
            deduplicated_size,
        )
    }

    /// Like [`backup_completed_message`], but lets the archive-level `deduplicated_size`
    /// (this backup's own new unique data) and the repo-wide `repo_unique_csize` (the
    /// repo's total current usage) diverge, matching what a real backup with prior
    /// archives reports.
    fn backup_completed_message_with_repo_size(
        agent_id: i64,
        repo_id: i64,
        deduplicated_size: i64,
        repo_unique_csize: i64,
    ) -> String {
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
            repo_unique_csize,
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

    /// A failed backup report, the shape a run against a host that is not
    /// there actually produces.
    fn backup_failed_message(
        agent_id: i64,
        repo_id: i64,
        schedule_id: Option<i64>,
        run_id: Option<&str>,
    ) -> String {
        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent_id),
            repo_id: RepoId(repo_id),
            schedule_id,
            started_at,
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(1))
                .unwrap(),
            status: BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 60,
            error_message: Some("Connection closed by remote host".to_owned()),
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: None,
            borg_command: None,
            run_id: run_id.map(str::to_owned),
        };
        serde_json::to_string(&AgentToServer::BackupCompleted { report })
            .expect("serialize message")
    }

    /// An agent owning one repository whose host never answers, through one
    /// schedule. `intermittent` is the repository's own "host is not always
    /// online" switch, which is the only thing the tests below differ on - and
    /// far too much setup to write out for each.
    #[cfg(test)]
    async fn absent_repo_fixture(
        pool: &PgPool,
        intermittent: bool,
    ) -> (
        crate::db::AgentRow,
        crate::db::RepoRow,
        crate::db::ScheduleRow,
    ) {
        let agent = crate::db::insert_agent(pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        // `storage.local` never answers from a test, which is exactly the
        // state this reports on.
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "absent-repo",
                repo_path: "/backups/absent",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        if intermittent {
            sqlx::query!(
                "UPDATE repos SET intermittent = true WHERE id = $1",
                repo.id
            )
            .execute(pool)
            .await
            .expect("mark the repository as not always online");
        }
        // The agent has to own this repository through a schedule, or the
        // report is rejected before any of this is reached.
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "absent-repo-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        (agent, repo, schedule)
    }

    /// Reports a failed backup of `repo` from `agent` - through `schedule_id`, or
    /// as a manual run when it is `None` - with a webhook subscribed to both a
    /// plain failure and the repo-offline skip, and returns the channel and the
    /// event types delivered to it once every background task has finished.
    ///
    /// Subscribed to both so a double-fire shows up as two rows rather than
    /// being masked by only one of them having a rule.
    #[cfg(test)]
    async fn report_failed_backup(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        repo_id: i64,
        schedule_id: Option<i64>,
    ) -> (i64, Vec<String>) {
        report_failed_run(pool, agent, repo_id, schedule_id, None).await
    }

    /// [`report_failed_backup`] for a run that carries its own `run_id`, as a
    /// scheduled one does.
    #[cfg(test)]
    async fn report_failed_run(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        repo_id: i64,
        schedule_id: Option<i64>,
        run_id: Option<&str>,
    ) -> (i64, Vec<String>) {
        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'webhook', $2, true) RETURNING id",
            "test-webhook",
            serde_json::json!({ "url": "http://127.0.0.1:1/unreachable" }),
        )
        .fetch_one(pool)
        .await
        .unwrap();
        for event_type in ["backup_failed", "backup_skipped_repo_offline"] {
            sqlx::query!(
                "INSERT INTO notification_rules (channel_id, event_type, enabled) VALUES ($1, $2, \
                 true)",
                channel_id,
                event_type,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        let state = build_test_state(pool.clone());
        let msg = backup_failed_message(agent.id, repo_id, schedule_id, run_id);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        // The notification is spawned on the background tracker, and only the
        // delivery attempt itself lands on the task registry, so both have to
        // be drained before the rows exist.
        assert!(
            state
                .background_task_tracker
                .wait_until_idle(std::time::Duration::from_secs(60))
                .await,
            "the backup-completed background work must finish"
        );
        let outstanding = state
            .task_registry
            .shutdown(std::time::Duration::from_secs(30))
            .await;
        assert_eq!(
            outstanding, 0,
            "notification delivery must have been joined"
        );

        let deliveries: Vec<String> = sqlx::query_scalar!(
            "SELECT event_type FROM notification_deliveries WHERE channel_id = $1",
            channel_id,
        )
        .fetch_all(pool)
        .await
        .unwrap();
        (channel_id, deliveries)
    }

    #[cfg(test)]
    async fn skipped_repo_offline_events(pool: &PgPool) -> usize {
        sqlx::query_scalar!(
            "SELECT event_type FROM system_events WHERE event_type = 'backup_skipped_repo_offline'",
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .len()
    }

    #[cfg(test)]
    async fn pending_repo_catch_ups(
        pool: &PgPool,
    ) -> Vec<crate::db::catch_up::RepoCatchUpCandidate> {
        crate::db::catch_up::list_repo_catch_up_candidates(
            pool,
            crate::db::catch_up::RepoCatchUpFilter::All,
        )
        .await
        .unwrap()
    }

    /// A backup that failed against a repository marked as not always online,
    /// whose host is not answering, must be reported as a repo-offline skip -
    /// and as *only* that. The failure notification has to become the skip
    /// rather than fire alongside it, or one backup leaves two alerts and two
    /// activity rows.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_absent_repository_host_is_reported_as_skipped_once(
        pool: PgPool,
    ) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        let (channel_id, deliveries) =
            report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        assert_eq!(
            deliveries,
            vec!["backup_skipped_repo_offline".to_owned()],
            "exactly one notification may describe the run, and it must be the skip rather than a \
             bare failure"
        );

        // Asserted on the delivered payload, not on a hand-built one: the
        // point of collapsing to a single event is that the alert itself says
        // why, so it must carry the reason rather than repeat the raw borg
        // error a plain failure would have shown.
        let payload: serde_json::Value = sqlx::query_scalar!(
            r#"SELECT payload AS "payload!" FROM notification_deliveries WHERE channel_id = $1"#,
            channel_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            payload
                .get("error_message")
                .and_then(serde_json::Value::as_str),
            Some("the host for repository 'absent-repo' did not answer SSH"),
            "the outbound notification must explain the absent host, not echo borg's own \
             connection error"
        );
        assert_eq!(
            skipped_repo_offline_events(&pool).await,
            1,
            "the Activity Log must carry the reason the backup had nowhere to write"
        );
    }

    /// The same failure against a repository that is *not* marked as not always
    /// online: a server that should have been there, so an unreachable host is
    /// exactly the failure borg reported. No skip, no probe-driven relabelling,
    /// and nothing waiting for it to come back.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_always_online_repository_stays_a_failure(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, false).await;
        let (_, deliveries) = report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        assert_eq!(
            deliveries,
            vec!["backup_failed".to_owned()],
            "an always-online repository's unreachable host is a failed backup, never a skip"
        );
        assert_eq!(skipped_repo_offline_events(&pool).await, 0);
        assert!(
            pending_repo_catch_ups(&pool).await.is_empty(),
            "nothing waits for a repository that is expected to always be there"
        );
    }

    /// The other half of the skip: the repository is also waited for, so the
    /// run happens once its host answers again instead of at the schedule's
    /// next occurrence.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_absent_repository_leaves_a_catch_up_pending(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        // The occurrence the run stood in for, which is what the marker names
        // and what the give-up window is measured from. Truncated to what
        // Postgres itself stores, so the round-trip below is exact whichever
        // way the server version handles the sub-microsecond tail.
        let missed = chrono::SubsecRound::trunc_subsecs(chrono::Utc::now(), 6)
            .checked_sub_signed(chrono::Duration::hours(2))
            .unwrap();
        sqlx::query!(
            "UPDATE schedules SET last_run_at = $2 WHERE id = $1",
            schedule.id,
            missed,
        )
        .execute(&pool)
        .await
        .unwrap();

        report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        let pending = pending_repo_catch_ups(&pool).await;
        assert_eq!(pending.len(), 1, "the absent repository must be waited on");
        let candidate = pending.first().unwrap();
        assert_eq!(candidate.repo_id, repo.id);
        assert_eq!(candidate.schedule_id, schedule.id);
        assert_eq!(
            candidate.pending_for, missed,
            "the marker names the occurrence the run stood in for, not the moment it gave up"
        );
        assert_eq!(candidate.last_probe_at, None);
    }

    /// A failure that comes back slowly, after the same schedule has already
    /// run again, must still name the occurrence it stood for - its own run's
    /// start - rather than whatever `last_run_at` says by then.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_slow_failure_names_its_own_run_not_the_schedules_latest(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        // The run that failed, as the scheduler recorded it when it dispatched.
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, run_id) VALUES ($1, $2, $3, NOW() - interval '2 hours', NOW() - interval '2 \
             hours', 'pending', 'slow-run')",
            agent.id,
            repo.id,
            schedule.id,
        )
        .execute(&pool)
        .await
        .unwrap();
        // ...and the schedule has moved on to a later run since.
        sqlx::query!(
            "UPDATE schedules SET last_run_at = NOW() WHERE id = $1",
            schedule.id,
        )
        .execute(&pool)
        .await
        .unwrap();

        report_failed_run(&pool, &agent, repo.id, Some(schedule.id), Some("slow-run")).await;

        let run_started = crate::db::catch_up::run_started_at(&pool, "slow-run", repo.id)
            .await
            .unwrap()
            .expect("the run keeps its report");
        let pending = pending_repo_catch_ups(&pool).await;
        let candidate = pending
            .first()
            .expect("the absent repository must be waited on");
        assert_eq!(
            candidate.pending_for, run_started,
            "the marker must name the run that failed, not the schedule's latest one"
        );
        let last_run_at = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .unwrap()
            .last_run_at;
        assert_ne!(Some(candidate.pending_for), last_run_at);
    }

    /// A manual Run now has no occurrence behind it, so there is nothing to
    /// catch up to - only a schedule that was due leaves a marker.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_manual_run_against_an_absent_repository_leaves_nothing_pending(pool: PgPool) {
        let (agent, repo, _) = absent_repo_fixture(&pool, true).await;

        report_failed_backup(&pool, &agent, repo.id, None).await;

        assert!(
            pending_repo_catch_ups(&pool).await.is_empty(),
            "a run with no schedule behind it has no occurrence to catch up"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_repo_quota_breach(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "quota-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
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

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Regression test for the archive-level-vs-repo-wide size mixup: a backup that adds
    /// almost no new unique data (`deduplicated_size`) to an already-large repo must still
    /// breach the repo's quota, because enforcement is decided by `repo_unique_csize` (the
    /// repo's actual current total), not by how much this one archive contributed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_uses_repo_unique_csize_not_archive_delta_for_repo_quota(
        pool: PgPool,
    ) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "quota-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
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
        // This archive itself only added 5 bytes of new unique data (well under the
        // warn/critical thresholds), but the repo's actual current total is 150 bytes,
        // over the critical threshold of 100.
        let msg = backup_completed_message_with_repo_size(agent.id, repo.id, 5, 150);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_server_quota_breach(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
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
                sync_schedule: None,
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo b");
        let schedule_a = crate::db::insert_schedule(
            &pool,
            repo_a.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-a",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
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
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-b",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
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

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Regression test for the archive-level-vs-repo-wide size mixup in the *server* quota
    /// path: repo A's own backup only added 5 bytes of new unique data (its archive-level
    /// `deduplicated_size`), well under what's needed to breach the shared host quota when
    /// combined with sibling repo B's 60-byte snapshot. But repo A's actual current total
    /// (`repo_unique_csize`) is 45 bytes, which combined with B's 60 bytes breaches the
    /// critical threshold of 100. Enforcement must use `repo_unique_csize`, not
    /// `deduplicated_size`, for the just-completed repo's contribution.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_uses_repo_unique_csize_not_archive_delta_for_server_quota(
        pool: PgPool,
    ) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
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
                sync_schedule: None,
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
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo b");
        // Repo B's sibling snapshot, as if left over from an earlier sync/rescan.
        crate::db::update_repo_info_stats(
            &pool,
            repo_b.id,
            &crate::db::RepoInfoStats {
                deduplicated_size: 60,
                ..Default::default()
            },
        )
        .await
        .expect("seed repo b stats");
        let schedule_a = crate::db::insert_schedule(
            &pool,
            repo_a.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-a",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule a");
        crate::db::insert_schedule_targets(&pool, schedule_a.id, &[(agent.id, 0)])
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
        // Archive-level delta (5) + sibling snapshot (60) = 65, under the critical
        // threshold of 100. Only the repo-wide total (45) + sibling snapshot (60) = 105
        // breaches it.
        let msg = backup_completed_message_with_repo_size(agent.id, repo_a.id, 5, 45);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule_a.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// The agent reconnecting must both re-enable a schedule the scheduler auto-disabled
    /// for it, and record a `ScheduleReenabled` system event, so the reconnect isn't
    /// invisible outside server logs - matching the `ScheduleAutoDisabled` event the
    /// scheduler already records when it disables the schedule in the first place.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_reenables_schedule_and_records_system_event(pool: PgPool) {
        let agent =
            crate::db::insert_agent(&pool, "reconnect-event-agent", None, "hash", None, None)
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
                name: "reconnect-event-repo",
                repo_path: "/backups/reconnect-event",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "reconnect-event-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        for _ in 0..3 {
            crate::db::record_schedule_failure(&pool, schedule.id, agent.id, next, 3, true)
                .await
                .expect("record schedule failure");
        }
        let disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!disabled.enabled && disabled.auto_disabled_agent_unreachable);

        let state = build_test_state(pool.clone());
        let mut ui_rx = state.ui_broadcast.subscribe();
        // The real connection handler registers the agent before calling this - the
        // reconnect check now requires every target of a candidate schedule to be
        // currently connected, which for this single-target schedule means just the
        // reconnecting agent itself.
        let (tx, _rx) = mpsc::channel(1);
        state.registry.register(agent.id, tx, false, None).await;
        reenable_system_disabled_schedules_on_reconnect(&state, agent.id, &agent.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(reenabled.enabled);
        assert!(!reenabled.auto_disabled_agent_unreachable);

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let event = events
            .iter()
            .find(|e| matches!(e.event_type, SystemEventType::ScheduleReenabled))
            .expect("a ScheduleReenabled system event was recorded");
        assert_eq!(event.hostname.as_deref(), Some(agent.hostname.as_str()));
        assert!(event.message.contains(&schedule.id.to_string()));

        // Without this, SchedulesView.vue (which only refetches on mount or on a
        // DataChanged message) would never show the re-enabled schedule until a
        // manual reload.
        let mut saw_data_changed = false;
        while let Ok(msg) = ui_rx.try_recv() {
            if matches!(msg, ServerToUi::DataChanged) {
                saw_data_changed = true;
            }
        }
        assert!(
            saw_data_changed,
            "re-enabling a schedule on reconnect must broadcast DataChanged so it shows up live \
             in the UI"
        );
    }

    /// `auto_disabled_by_agent_id` isn't exposed on `ScheduleRow` (it's internal
    /// bookkeeping, not part of the schedule API), so tests that need to assert on it
    /// read it directly.
    async fn auto_disabled_by_agent_id(pool: &PgPool, schedule_id: i64) -> Option<i64> {
        sqlx::query_scalar!(
            "SELECT auto_disabled_by_agent_id FROM schedules WHERE id = $1",
            schedule_id,
        )
        .fetch_one(pool)
        .await
        .expect("select auto_disabled_by_agent_id")
    }

    /// Reproduces the exact bug the registry-connectivity check exists to prevent: for a
    /// multi-target schedule, `auto_disabled_by_agent_id` only ever names whichever
    /// target happened to be first-recorded when the threshold was crossed, not every
    /// target that contributed to the (schedule-wide) failure streak. Here "flaky"
    /// reconnects routinely while "broken" stays down the whole time, but the failure
    /// that actually crosses the threshold is recorded against "flaky" - so a reconnect
    /// handler that only checked `auto_disabled_by_agent_id` would re-enable the
    /// schedule the moment flaky reconnects, immediately reproducing the unbounded-retry
    /// problem for "broken", which this PR exists to fix.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_only_reenables_multi_target_schedule_once_every_target_is_connected(
        pool: PgPool,
    ) {
        let flaky = crate::db::insert_agent(&pool, "flaky-agent", None, "hash", None, None)
            .await
            .expect("insert flaky agent");
        let broken = crate::db::insert_agent(&pool, "broken-agent", None, "hash", None, None)
            .await
            .expect("insert broken agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "multi-target-repo",
                repo_path: "/backups/multi-target",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "multi-target-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "continue",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(flaky.id, 0), (broken.id, 1)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        // Two failures attributed to "broken", then the third (threshold-crossing) one
        // attributed to "flaky" - so `auto_disabled_by_agent_id` ends up naming flaky,
        // even though broken is the target that's actually still down.
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 1");
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 2");
        crate::db::record_schedule_failure(&pool, schedule.id, flaky.id, next, 3, true)
            .await
            .expect("record schedule failure 3");
        let disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!disabled.enabled && disabled.auto_disabled_agent_unreachable);
        assert_eq!(
            auto_disabled_by_agent_id(&pool, schedule.id).await,
            Some(flaky.id)
        );

        let state = build_test_state(pool.clone());

        // Flaky reconnects, but broken is still down - must not re-enable the schedule.
        let (flaky_tx, _flaky_rx) = mpsc::channel(1);
        state
            .registry
            .register(flaky.id, flaky_tx, false, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, flaky.id, &flaky.hostname).await;

        let still_disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(
            !still_disabled.enabled && still_disabled.auto_disabled_agent_unreachable,
            "must not re-enable a multi-target schedule while another target is still \
             unreachable, even if the reconnecting agent is the one named by \
             auto_disabled_by_agent_id"
        );

        // Broken reconnects too - now every target is connected, so the schedule
        // resumes.
        let (broken_tx, _broken_rx) = mpsc::channel(1);
        state
            .registry
            .register(broken.id, broken_tx, false, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, flaky.id, &flaky.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(reenabled.enabled);
        assert!(!reenabled.auto_disabled_agent_unreachable);
    }

    /// `auto_disabled_by_agent_id` only ever names whichever target happened to be
    /// first-recorded on the disabling tick - not necessarily the target that's
    /// actually been down the whole streak. If reconnect handling only reconsidered a
    /// schedule when *that specific* agent reconnects, a schedule could stay disabled
    /// forever once the credited agent has already recovered and stopped generating
    /// new reconnect events, even after the truly-broken target comes back too.
    /// Reconnect handling must reconsider a candidate schedule on *any* of its
    /// targets reconnecting - here "broken" reconnects (not "flaky", the one credited
    /// via `auto_disabled_by_agent_id`), and that alone must be enough to re-enable
    /// the schedule once every target, including the already-connected "flaky", is up.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_from_uncredited_target_reenables_schedule_once_all_connected(pool: PgPool) {
        let flaky = crate::db::insert_agent(&pool, "flaky-agent-2", None, "hash", None, None)
            .await
            .expect("insert flaky agent");
        let broken = crate::db::insert_agent(&pool, "broken-agent-2", None, "hash", None, None)
            .await
            .expect("insert broken agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "uncredited-target-repo",
                repo_path: "/backups/uncredited-target",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "uncredited-target-schedule",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "continue",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(flaky.id, 0), (broken.id, 1)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        // Same attribution as the sibling test: the threshold-crossing failure is
        // recorded against "flaky", so auto_disabled_by_agent_id ends up naming flaky
        // even though "broken" is the one reconnecting below.
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 1");
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 2");
        crate::db::record_schedule_failure(&pool, schedule.id, flaky.id, next, 3, true)
            .await
            .expect("record schedule failure 3");
        assert_eq!(
            auto_disabled_by_agent_id(&pool, schedule.id).await,
            Some(flaky.id)
        );

        let state = build_test_state(pool.clone());

        // Flaky already recovered and is connected, but generates no new reconnect
        // event here - only "broken" does.
        let (flaky_tx, _flaky_rx) = mpsc::channel(1);
        state
            .registry
            .register(flaky.id, flaky_tx, false, None)
            .await;
        let (broken_tx, _broken_rx) = mpsc::channel(1);
        state
            .registry
            .register(broken.id, broken_tx, false, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, broken.id, &broken.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(
            reenabled.enabled,
            "the uncredited target's own reconnect must still re-enable the schedule once every \
             target is connected"
        );
        assert!(!reenabled.auto_disabled_agent_unreachable);
    }

    /// Agent, repository and catch-up-enabled schedule for the reconnect catch-up
    /// tests below, with `agent_id` already carrying a missed occurrence.
    async fn insert_catch_up_fixture(
        pool: &PgPool,
        name: &str,
        next_run_at: chrono::DateTime<chrono::Utc>,
    ) -> (crate::db::AgentRow, i64) {
        let agent = crate::db::insert_agent(pool, name, None, "hash", None, None)
            .await
            .expect("insert agent");
        // Only a host marked as not always online is waited for.
        sqlx::query!(
            "UPDATE agents SET intermittent = true WHERE id = $1",
            agent.id
        )
        .execute(pool)
        .await
        .expect("mark the agent as not always online");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: &format!("{name}-repo"),
                repo_path: "/backups/catch-up",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        crate::db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAACATCHUP")
            .await
            .expect("set repo host key");
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: &format!("{name}-schedule"),
                schedule_type: "backup",
                cron_expression: "0 2 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        crate::db::set_next_run_at(pool, schedule.id, next_run_at)
            .await
            .expect("set next run");
        crate::db::catch_up::mark_catch_up_pending(
            pool,
            schedule.id,
            agent.id,
            chrono::Utc::now()
                .checked_sub_signed(chrono::Duration::days(35))
                .unwrap(),
        )
        .await
        .expect("mark catch-up pending");
        (agent, schedule.id)
    }

    async fn pending_catch_ups(pool: &PgPool, agent_id: i64) -> usize {
        crate::db::catch_up::list_catch_up_candidates_for_agent(pool, agent_id)
            .await
            .expect("list catch-up candidates")
            .len()
    }

    async fn recorded_catch_up_events(pool: &PgPool) -> usize {
        crate::db::get_system_events(pool, 20, AcknowledgedFilter::All)
            .await
            .expect("get system events")
            .iter()
            .filter(|e| matches!(e.event_type, SystemEventType::ScheduleCatchUp))
            .count()
    }

    /// The happy path: a host that missed a run comes back with plenty of time before
    /// the next scheduled one, so the run it missed is dispatched to it.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_runs_the_occurrence_the_host_missed(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        let (agent, schedule_id) = insert_catch_up_fixture(&pool, "catch-up-runs", next_run).await;

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state.registry.register(agent.id, tx, false, None).await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a catch-up run must be dispatched on reconnect")
            .expect("registry channel open");
        assert!(
            matches!(first, ServerToAgent::ConfigUpdate(_)),
            "the catch-up must push a fresh config first, got: {first:?}"
        );
        let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the run trigger must follow the config push")
            .expect("registry channel open");
        assert!(
            matches!(second, ServerToAgent::RunBackupNow { .. }),
            "the catch-up must trigger the missed backup, got: {second:?}"
        );

        assert_eq!(
            pending_catch_ups(&pool, agent.id).await,
            0,
            "the marker must be cleared, so a second reconnect cannot run it again"
        );
        assert_eq!(recorded_catch_up_events(&pool).await, 1);
        assert!(
            crate::db::get_schedule_by_id(&pool, schedule_id)
                .await
                .expect("get schedule")
                .enabled
        );

        // Lets the dispatch task's completion wait finish instead of outliving the test.
        state.registry.unregister(agent.id).await;
    }

    /// The regular run is 30 minutes out and the floor is two hours: catching up now
    /// would only duplicate it, so the miss is dropped instead.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_skips_a_catch_up_when_the_next_run_is_close(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::minutes(30))
            .unwrap();
        let (agent, _) = insert_catch_up_fixture(&pool, "catch-up-skips", next_run).await;

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state.registry.register(agent.id, tx, false, None).await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        assert!(
            rx.try_recv().is_err(),
            "nothing may be dispatched when the next scheduled run is inside the floor"
        );
        assert_eq!(
            pending_catch_ups(&pool, agent.id).await,
            0,
            "a skipped miss is dropped, not carried forward to the next reconnect"
        );
        assert_eq!(recorded_catch_up_events(&pool).await, 0);
    }

    /// Marking the agent as always online between the miss and the reconnect means
    /// the run is no longer wanted - the marker still goes, so it cannot resurface
    /// later.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_drops_a_catch_up_the_agent_no_longer_wants(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        let (agent, _) = insert_catch_up_fixture(&pool, "catch-up-off", next_run).await;
        sqlx::query!(
            "UPDATE agents SET intermittent = false WHERE id = $1",
            agent.id,
        )
        .execute(&pool)
        .await
        .expect("mark the agent as always online");

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state.registry.register(agent.id, tx, false, None).await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        assert!(
            rx.try_recv().is_err(),
            "an agent no longer marked as not always online runs nothing"
        );
        let still_pending = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM schedule_targets WHERE agent_id = $1 AND catch_up_pending_for \
             IS NOT NULL",
            agent.id,
        )
        .fetch_one(&pool)
        .await
        .expect("count pending");
        assert_eq!(still_pending, Some(0), "the stale marker must be cleared");
    }
}
