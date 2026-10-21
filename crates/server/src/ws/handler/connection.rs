// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Agent connection lifecycle: the `Hello` handshake and authentication, the
//! socket loop, reconnect catch-up and the keepalive ping.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use shared::{
    protocol::{AgentToServer, ServerToAgent, ServerToUi},
    types::SystemEventType,
};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::handle_agent_message;
use crate::{
    AppState, catch_up, config_assembler, db,
    restore_runs::{self},
    ws::completion_bus::OperationOutcome,
};

const PING_INTERVAL: Duration = Duration::from_secs(30);

const CHANNEL_BUFFER: usize = 32;

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

pub(super) struct HelloFields {
    pub(super) hostname: String,
    pub(super) token: String,
    pub(super) agent_version: String,
    pub(super) agent_git_sha: Option<String>,
    pub(super) agent_build_time: Option<String>,
    pub(super) agent_commit_count: Option<u32>,
    pub(super) supports_restart: bool,
    pub(super) restart_unavailable_reason: Option<String>,
    pub(super) instance_id: Option<String>,
    pub(super) snapshot_tools: Vec<shared::snapshot::FsSnapshotKind>,
}

pub(super) async fn read_hello_message<St, E>(ws_stream: &mut St) -> Option<HelloFields>
where
    St: Stream<Item = Result<Message, E>> + Unpin,
{
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
                instance_id,
                snapshot_tools,
            }) => Some(HelloFields {
                hostname,
                token,
                agent_version,
                agent_git_sha,
                agent_build_time,
                agent_commit_count,
                supports_restart,
                restart_unavailable_reason,
                instance_id,
                snapshot_tools,
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

pub(super) async fn send_close<Si>(ws_sink: &mut Si, reason: &'static str)
where
    Si: Sink<Message> + Unpin,
    Si::Error: std::fmt::Display,
{
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
pub(super) async fn authenticate_agent<Si>(
    pool: &PgPool,
    ws_sink: &mut Si,
    hostname: &str,
    token: &str,
) -> Option<i64>
where
    Si: Sink<Message> + Unpin,
    Si::Error: std::fmt::Display,
{
    let Some(agent_id) = verify_agent_token(pool, hostname, token).await else {
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

/// Records that the agent just connected, and the build it runs.
pub(super) async fn record_agent_seen(pool: &PgPool, agent_id: i64, hello: &HelloFields) {
    if let Err(e) = db::update_last_seen_and_version(
        pool,
        agent_id,
        &hello.agent_version,
        hello.agent_git_sha.as_deref(),
        hello.agent_build_time.as_deref(),
        hello
            .agent_commit_count
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
    )
    .await
    {
        tracing::error!(hostname = %hello.hostname, error = %e, "failed to update last_seen_at");
    }
}

pub(super) async fn handle_socket(socket: WebSocket, state: AppState) {
    let (ws_sink, ws_stream) = socket.split();
    Box::pin(serve_agent(ws_sink, ws_stream, state)).await;
}

/// The agent connection behind [`handle_socket`]: the Hello handshake, then
/// the message loop until either side ends it. Generic over the socket's two
/// halves so tests can drive it without a network.
pub(super) async fn serve_agent<Si, St, E>(mut ws_sink: Si, mut ws_stream: St, state: AppState)
where
    Si: Sink<Message> + Unpin,
    Si::Error: std::fmt::Display,
    St: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    let Some(hello) = read_hello_message(&mut ws_stream).await else {
        send_close(&mut ws_sink, "expected Hello message").await;
        return;
    };
    tracing::info!(
        hostname = %hello.hostname,
        agent_version = %hello.agent_version,
        "agent attempting connection"
    );

    let Some(agent_id) =
        authenticate_agent(&state.pool, &mut ws_sink, &hello.hostname, &hello.token).await
    else {
        return;
    };

    record_agent_seen(&state.pool, agent_id, &hello).await;
    let HelloFields {
        hostname,
        supports_restart,
        restart_unavailable_reason,
        instance_id,
        snapshot_tools,
        ..
    } = hello;

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<ServerToAgent>(CHANNEL_BUFFER);
    let ping_tx = outbound_tx.clone();
    let replaced_connection = state
        .registry
        .register(
            agent_id,
            outbound_tx,
            supports_restart,
            restart_unavailable_reason,
            instance_id.clone(),
        )
        .await;
    state
        .registry
        .set_snapshot_tools(agent_id, snapshot_tools)
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

    send_reconnect_catchup(&state, &mut ws_sink, &hostname, agent_id, instance_id).await;

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
pub(super) async fn reenable_system_disabled_schedules_on_reconnect(
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

pub(super) async fn send_ws_message<Si, M>(ws_sink: &mut Si, msg: &M) -> bool
where
    Si: Sink<Message> + Unpin,
    M: serde::Serialize,
{
    let Ok(json) = serde_json::to_string(msg) else {
        return false;
    };
    ws_sink.send(Message::Text(json.into())).await.is_ok()
}

/// Catches the agent up on state it may have missed while offline: pushes a
/// fresh config, notifies it of any backups the user cancelled while it was
/// disconnected, re-triggers any backup runs that were queued (e.g. via
/// "Run Now") but never dispatched, and settles and hands over its restores.
pub(super) async fn send_reconnect_catchup<Si>(
    state: &AppState,
    ws_sink: &mut Si,
    hostname: &str,
    agent_id: i64,
    instance_id: Option<String>,
) where
    Si: Sink<Message> + Unpin,
{
    match config_assembler::assemble_config(&state.pool, &state.encryption_key, agent_id).await {
        Ok(config) => {
            if !send_ws_message(ws_sink, &ServerToAgent::ConfigUpdate(Box::new(config))).await {
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

    // After the config push above: the agent needs the repo's config to run
    // a restore handed to it.
    spawn_restore_catchup(state, agent_id, hostname, instance_id);
}

/// Catches a newly connected agent up on its restores, in the background:
/// the restores go out through the connection's outbound channel, which
/// only the connection's own loop drains.
pub(super) fn spawn_restore_catchup(
    state: &AppState,
    agent_id: i64,
    hostname: &str,
    instance_id: Option<String>,
) {
    let task_state = state.clone();
    let hostname = hostname.to_owned();
    state.background_task_tracker.spawn_tracked(async move {
        if let Err(e) = restore_runs::on_agent_connected(
            &task_state,
            agent_id,
            &hostname,
            instance_id.as_deref(),
        )
        .await
        {
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to catch the agent up on restores"
            );
        }
    });
}

pub(super) async fn ping_loop(
    sender: mpsc::Sender<ServerToAgent>,
    shutdown_token: CancellationToken,
) {
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
