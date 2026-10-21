// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repository maintenance results: `borg check`, archive verification and
//! canary restores.

use shared::{protocol::ServerToUi, types::ScheduleType};

use super::{notify::spawn_notification_dispatch, validate_agent_repo};
use crate::{
    AppState, db,
    notifications::{EventType, NotificationEvent},
    ws::completion_bus::OperationOutcome,
};

pub(super) struct CheckCompletedArgs<'a> {
    pub(super) hostname: &'a str,
    pub(super) agent_id: i64,
    pub(super) state: &'a AppState,
    pub(super) repo_id: shared::types::RepoId,
    pub(super) success: bool,
    pub(super) duration_secs: i64,
    pub(super) error_message: Option<String>,
}

pub(super) async fn handle_check_completed(args: CheckCompletedArgs<'_>) {
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

pub(super) struct VerifyCompletedArgs<'a> {
    pub(super) hostname: &'a str,
    pub(super) agent_id: i64,
    pub(super) state: &'a AppState,
    pub(super) repo_id: shared::types::RepoId,
    pub(super) success: bool,
    pub(super) duration_secs: i64,
    pub(super) error_message: Option<String>,
    pub(super) files_verified: i64,
}

pub(super) async fn handle_verify_completed(args: VerifyCompletedArgs<'_>) {
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

pub(super) struct CanaryVerifiedArgs<'a> {
    pub(super) hostname: &'a str,
    pub(super) agent_id: i64,
    pub(super) state: &'a AppState,
    pub(super) repo_id: shared::types::RepoId,
    pub(super) success: bool,
    pub(super) nonce: String,
    pub(super) archive_name: String,
    pub(super) error_message: Option<String>,
}

pub(super) async fn handle_canary_verified(args: CanaryVerifiedArgs<'_>) {
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
