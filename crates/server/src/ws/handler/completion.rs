// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A finished backup run: persisting its report, its side effects and the
//! final UI update.

use shared::{protocol::ServerToUi, types::BackupStatus};

use super::{
    backup::spawn_post_backup_indexing,
    notify::{BackupCompletionNotificationArgs, dispatch_backup_completion_notification},
    quota::check_quotas_after_backup,
    validate_agent_repo,
};
use crate::{
    AppState, db,
    ws::{completion_bus::OperationOutcome, post_backup_sync},
};

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
        shared::protocol::RepoOpKind::AgentBackup,
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

pub(super) async fn persist_backup_completed_report(
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

pub(super) async fn handle_backup_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    report: shared::types::BackupReport,
    warning_kind: shared::types::BackupWarningKind,
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

    // Before the completion is published: that is what lets the run's own
    // power teardown go ahead, and it must not shut the host down first.
    let finished_run = post_backup_sync::prepare(state, agent_id, hostname, &report).await;

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
            warning_kind,
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

    // Whatever the outcome: a run that failed after `borg create` has still
    // changed the repository.
    post_backup_sync::spawn(state, finished_run);

    finalize_backup_completion(state, hostname, repo_id, report_for_ui, completed_repo_name).await;
}
