// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Backup runs starting, being rejected and being cancelled, and the content
//! indexing that follows a finished run.

use shared::protocol::ServerToUi;

use super::validate_agent_repo;
use crate::{
    AppState, archive_index, db,
    ws::{completion_bus::OperationOutcome, post_backup_sync, ui_broadcast::ActiveBackupSnapshot},
};

fn extract_archive_name(borg_command: &str) -> Option<String> {
    borg_command
        .split_whitespace()
        .find(|s| s.starts_with("::"))
        .map(|s| s.trim_start_matches("::").to_owned())
        .filter(|s| !s.is_empty())
}

pub(super) struct BackupStartedArgs<'a> {
    pub(super) hostname: &'a str,
    pub(super) agent_id: i64,
    pub(super) state: &'a AppState,
    pub(super) repo_id: shared::types::RepoId,
    pub(super) schedule_id: Option<i64>,
    pub(super) started_at: chrono::DateTime<chrono::Utc>,
    pub(super) borg_command: Option<String>,
    pub(super) run_id: Option<String>,
}

pub(super) async fn handle_backup_started(args: BackupStartedArgs<'_>) {
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
pub(super) fn spawn_post_backup_indexing(state: &AppState, repo_id: i64, archive_name: String) {
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

pub(super) async fn handle_backup_rejected(
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

pub(super) async fn handle_backup_cancelled(
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
    // An archive written before the abort, or one a prune removed, has
    // changed the repository as much as a run that finished.
    post_backup_sync::spawn(
        state,
        post_backup_sync::cancelled(agent_id, hostname, repo_id.0),
    );
    state.ui_broadcast.send(ServerToUi::DataChanged);
}
