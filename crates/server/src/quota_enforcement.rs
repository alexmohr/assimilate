// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Applies a [`QuotaAction`] once a repo or server quota threshold is breached.
//! `NotifyOnly` is a no-op here since the caller already dispatches a
//! notification for every breach regardless of the configured action.

use shared::types::QuotaAction;

use crate::{AppState, config_assembler, db};

async fn disable_schedule_and_push(state: &AppState, schedule_id: i64) {
    if let Err(e) = db::set_schedule_enabled(&state.pool, schedule_id, false).await {
        tracing::error!(schedule_id, error = %e, "failed to disable schedule for quota action");
        return;
    }
    config_assembler::push_config_to_all_schedule_targets(state, schedule_id).await;
}

/// Enforces a repo-level quota action: `block_backups` disables every schedule for the
/// repo, `disable_schedule` disables only `triggering_schedule_id`.
pub async fn enforce_repo_quota_action(
    state: &AppState,
    repo_id: i64,
    triggering_schedule_id: Option<i64>,
    action: QuotaAction,
) {
    match action {
        QuotaAction::NotifyOnly => {}
        QuotaAction::BlockBackups => {
            match db::list_schedules_for_repo(&state.pool, repo_id).await {
                Ok(schedules) => {
                    for schedule in schedules {
                        disable_schedule_and_push(state, schedule.id).await;
                    }
                }
                Err(e) => {
                    tracing::error!(repo_id, error = %e, "failed to list schedules for quota action");
                }
            }
        }
        QuotaAction::DisableSchedule => {
            if let Some(schedule_id) = triggering_schedule_id {
                disable_schedule_and_push(state, schedule_id).await;
            }
        }
    }
}

/// Enforces a server-level (shared SSH host) quota action: `block_backups` disables every
/// schedule for every repo on `ssh_host`, `disable_schedule` disables only
/// `triggering_schedule_id`.
pub async fn enforce_server_quota_action(
    state: &AppState,
    ssh_host: &str,
    triggering_schedule_id: Option<i64>,
    action: QuotaAction,
) {
    match action {
        QuotaAction::NotifyOnly => {}
        QuotaAction::BlockBackups => {
            match db::list_schedule_ids_for_ssh_host(&state.pool, ssh_host).await {
                Ok(schedule_ids) => {
                    for schedule_id in schedule_ids {
                        disable_schedule_and_push(state, schedule_id).await;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        ssh_host,
                        error = %e,
                        "failed to list schedules for server quota action"
                    );
                }
            }
        }
        QuotaAction::DisableSchedule => {
            if let Some(schedule_id) = triggering_schedule_id {
                disable_schedule_and_push(state, schedule_id).await;
            }
        }
    }
}
