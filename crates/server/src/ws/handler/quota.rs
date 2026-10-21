// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repository and server quota checks after a backup, and their breach
//! notifications.

use super::notify::spawn_notification_dispatch;
use crate::{
    AppState, db,
    notifications::{EventType, NotificationEvent},
    quota_enforcement,
};

pub(super) fn quota_status_label(status: db::quota::QuotaStatus) -> &'static str {
    match status {
        db::quota::QuotaStatus::Ok => "ok",
        db::quota::QuotaStatus::Warning => "warning",
        db::quota::QuotaStatus::Critical => "critical",
    }
}

pub(super) fn quota_event_type(status: db::quota::QuotaStatus) -> EventType {
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

/// Runs both quota checks (the repo's own quota and, if it shares an SSH host with other
/// repos, the combined server quota) after a backup completes. The two are independent --
/// each reads its own settings and dispatches its own notification/enforcement -- so they
/// run concurrently via [`tokio::join!`] instead of paying their DB/notification latency
/// twice in sequence.
pub(super) async fn check_quotas_after_backup(
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
pub(super) async fn check_repo_quota_after_backup(
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
    // Only a breach has an action.
    let Some(action) = quota.action_for(quota_status) else {
        return;
    };
    let status_label = quota_status_label(quota_status);
    tracing::warn!(
        hostname = %hostname,
        repo_id,
        repo_unique_csize,
        quota_status = status_label,
        "repository quota exceeded"
    );

    let message = format!(
        "Repository quota {status_label} for repo {repo_name}: current size {repo_unique_csize} \
         bytes exceeds configured limits",
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

    quota_enforcement::enforce_repo_quota_action(state, repo_id, schedule_id, action).await;
}

/// Checks whether the just-completed backup, combined with its sibling
/// repos on the same SSH host, pushed a shared server-wide quota over a
/// warning/critical threshold, dispatching a notification and enforcement
/// action if so.
pub(super) async fn check_server_quota_after_backup(
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
    // Only a breach has an action.
    let Some(action) = server_quota.action_for(quota_status) else {
        return;
    };
    let status_label = quota_status_label(quota_status);
    tracing::warn!(
        hostname = %hostname,
        ssh_host = %ssh_host,
        total_deduplicated_size,
        quota_status = status_label,
        "server quota exceeded"
    );

    let message = format!(
        "Server quota {status_label} for host {ssh_host}: combined deduplicated size \
         {total_deduplicated_size} bytes exceeds configured limits",
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

    quota_enforcement::enforce_server_quota_action(state, &ssh_host, schedule_id, action).await;
}
