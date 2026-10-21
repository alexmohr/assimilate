// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Backup-completion notifications and the shared background dispatch every
//! notification goes out through.

use super::failure::{
    classify_failed_backup, clear_repo_catch_up_on_success, settle_dependency_catch_up,
};
use crate::{
    AppState, db,
    notifications::{self, EventType, NotificationEvent},
};

/// Which event a warning-status backup goes out as. A run that only saw files
/// change under it gets an event of its own, so a channel can follow real
/// warnings without also hearing about every busy log file.
pub(super) fn warning_event_type(kind: shared::types::BackupWarningKind) -> EventType {
    match kind {
        shared::types::BackupWarningKind::General => EventType::BackupWarning,
        shared::types::BackupWarningKind::FileChanged => EventType::BackupFileChanged,
    }
}

/// Spawns `notifications::dispatch` for `event` as a detached task registered with
/// `task_registry`, so shutdown can join it instead of the runtime abandoning a
/// still-in-flight webhook/email/push delivery. Shared by every call site that builds
/// a [`NotificationEvent`] and fires it off in the background.
pub(super) fn spawn_notification_dispatch(state: &AppState, event: NotificationEvent) {
    let service = state.notification_service.clone();
    let task_registry = state.task_registry.clone();
    state.background_task_tracker.spawn_tracked(async move {
        if let Err(e) = notifications::dispatch(&service, event, &task_registry).await {
            tracing::error!(error = %e, "notification dispatch failed");
        }
    });
}

/// Context for [`dispatch_backup_completion_notification`], gathered by its caller from a
/// `BackupReport` before that report is moved into `persist_backup_completed_report`.
pub(super) struct BackupCompletionNotificationArgs<'a> {
    pub(super) status: shared::types::BackupStatus,
    pub(super) warning_kind: shared::types::BackupWarningKind,
    pub(super) hostname: &'a str,
    pub(super) repo_name: String,
    pub(super) status_str: &'a str,
    pub(super) error_message: Option<String>,
    pub(super) repo_id: i64,
    pub(super) agent_id: i64,
    pub(super) schedule_id: Option<i64>,
    pub(super) archive_name: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) duration_secs: i64,
    pub(super) original_size: i64,
    pub(super) compressed_size: i64,
    pub(super) deduplicated_size: i64,
    pub(super) files_processed: i64,
    pub(super) warnings: Vec<String>,
}

/// Dispatches a [`NotificationEvent`] for a completed backup.
pub(super) async fn dispatch_backup_completion_notification(
    state: &AppState,
    args: BackupCompletionNotificationArgs<'_>,
) {
    let BackupCompletionNotificationArgs {
        status,
        warning_kind,
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
        if matches!(
            status,
            shared::types::BackupStatus::Success | shared::types::BackupStatus::Warning
        ) && let Some(schedule_id) = schedule_id
        {
            clear_repo_catch_up_on_success(&pool, schedule_id, repo_id).await;
        }
        // Whatever borg made of it, the run reached its agent: the dependency
        // it was waiting on was there, so the wait is over. A failure from
        // here on is the backup's own, not a skip to catch up.
        if let Some(schedule_id) = schedule_id {
            settle_dependency_catch_up(&pool, schedule_id, agent_id).await;
        }
        let (event_type, error_message) = match status {
            shared::types::BackupStatus::Success => (EventType::BackupSuccess, error_message),
            shared::types::BackupStatus::Warning => {
                (warning_event_type(warning_kind), error_message)
            }
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
