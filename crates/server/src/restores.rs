// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The life of a restore onto an agent, from the request to the agent's
//! answer.
//!
//! A restore is recorded in the `restores` table before anything is sent, so
//! it survives a slow `borg extract`, a dropped agent connection and a server
//! restart. The agent's outbound queue outlives its WebSocket connection, so
//! an answer to a restore sent before a reconnect still arrives afterwards.
//! An agent that restarted lost the restore instead, so on every connect the
//! server sends each unfinished restore again; the agent ignores one it is
//! already working on.

use shared::{
    protocol::{ServerToAgent, ServerToUi},
    types::{RepoId, RestoreStatus, SystemEventType},
};

use crate::{
    AppState,
    db::{
        self,
        restores::{RestoreOutcome, RestoreRow},
    },
    error::ApiError,
};

fn announce(state: &AppState, restore_id: i64, status: RestoreStatus) {
    state
        .ui_broadcast
        .send(ServerToUi::RestoreUpdated { restore_id, status });
}

fn restore_message(restore: &RestoreRow) -> ServerToAgent {
    ServerToAgent::RestoreFiles {
        request_id: restore.request_id.clone(),
        repo_id: RepoId(restore.repo_id),
        archive_name: restore.archive_name.clone(),
        paths: restore.paths.clone(),
        target_path: restore.target_path.clone(),
    }
}

/// What a restore restores, in words.
fn describe_paths(paths: &[String]) -> String {
    match paths {
        [] => "the whole archive".to_owned(),
        [path] => path.clone(),
        many => format!("{} paths", many.len()),
    }
}

fn describe(restore: &RestoreRow) -> String {
    format!(
        "{} from {} to {} on {}",
        describe_paths(&restore.paths),
        restore.archive_name,
        restore.target_path,
        restore.hostname
    )
}

/// Sends a queued restore to its agent, if the agent is connected. A restore
/// whose agent is offline stays queued until [`resume_for_agent`] picks it
/// up on reconnect.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if recording the hand-over fails.
pub async fn dispatch(state: &AppState, restore: &RestoreRow) -> Result<(), ApiError> {
    if !state.registry.is_connected(restore.agent_id).await {
        return Ok(());
    }
    if !db::restores::mark_restore_dispatched(&state.pool, restore.id).await? {
        return Ok(());
    }
    if state
        .registry
        .send_to(restore.agent_id, restore_message(restore))
        .await
        .is_err()
    {
        db::restores::requeue_restore(&state.pool, restore.id).await?;
        return Ok(());
    }
    announce(state, restore.id, RestoreStatus::Dispatched);
    Ok(())
}

/// Sends an agent that just connected every restore it still owes an answer
/// for: the queued ones for the first time, the others again in case the
/// agent restarted and lost them.
pub async fn resume_for_agent(state: &AppState, agent_id: i64, hostname: &str) {
    let restores = match db::restores::list_unfinished_restores_for_agent(&state.pool, agent_id)
        .await
    {
        Ok(restores) => restores,
        Err(e) => {
            tracing::error!(hostname = %hostname, error = %e, "failed to load unfinished restores");
            return;
        }
    };
    for restore in &restores {
        match restore.status {
            RestoreStatus::Queued => {
                if let Err(e) = dispatch(state, restore).await {
                    tracing::error!(
                        hostname = %hostname,
                        restore_id = restore.id,
                        error = %e,
                        "failed to send a queued restore"
                    );
                }
            }
            RestoreStatus::Dispatched | RestoreStatus::Running => {
                if state
                    .registry
                    .send_to(agent_id, restore_message(restore))
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        hostname = %hostname,
                        restore_id = restore.id,
                        "agent went away before an unfinished restore could be sent again"
                    );
                }
            }
            RestoreStatus::Succeeded | RestoreStatus::Failed | RestoreStatus::Cancelled => {}
        }
    }
}

/// Runs [`resume_for_agent`] in the background for an agent that just
/// connected. Spawned because the messages go through that connection's
/// outbound queue, which only drains once its read/write loop runs.
pub fn spawn_resume_for_agent(state: &AppState, agent_id: i64, hostname: &str) {
    let state = state.clone();
    let hostname = hostname.to_owned();
    tokio::spawn(async move { resume_for_agent(&state, agent_id, &hostname).await });
}

/// Records that the agent started extracting a restore. Returns `false`
/// when the agent has no unfinished restore with this request id.
pub async fn record_started(state: &AppState, agent_id: i64, request_id: &str) -> bool {
    match db::restores::mark_restore_running(&state.pool, request_id, agent_id).await {
        Ok(Some(restore_id)) => {
            announce(state, restore_id, RestoreStatus::Running);
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::error!(request_id = %request_id, error = %e, "failed to record restore start");
            true
        }
    }
}

/// Records how a restore ended, adds it to the Activity Log and tells the
/// UI. Returns `false` when the agent has no unfinished restore with this
/// request id.
pub async fn record_finished(
    state: &AppState,
    agent_id: i64,
    request_id: &str,
    outcome: &RestoreOutcome,
) -> bool {
    let restore_id =
        match db::restores::finish_restore(&state.pool, request_id, agent_id, outcome).await {
            Ok(Some(restore_id)) => restore_id,
            Ok(None) => return false,
            Err(e) => {
                tracing::error!(
                    request_id = %request_id,
                    error = %e,
                    "failed to record how a restore ended"
                );
                return true;
            }
        };
    let restore = match db::restores::get_restore(&state.pool, restore_id).await {
        Ok(restore) => restore,
        Err(e) => {
            tracing::error!(restore_id, error = %e, "failed to load a finished restore");
            return true;
        }
    };
    let (event_type, message) = match outcome {
        RestoreOutcome::Succeeded { .. } => (
            SystemEventType::RestoreCompleted,
            format!("Restored {}", describe(&restore)),
        ),
        RestoreOutcome::Failed { error_message } => (
            SystemEventType::RestoreFailed,
            format!("Restore of {} failed: {error_message}", describe(&restore)),
        ),
    };
    if let Err(e) =
        db::insert_system_event(&state.pool, event_type, Some(&restore.hostname), &message).await
    {
        tracing::error!(restore_id, error = %e, "failed to record restore system event");
    }
    announce(state, restore.id, restore.status);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_what_a_restore_restores() {
        assert_eq!(describe_paths(&[]), "the whole archive");
        assert_eq!(describe_paths(&["etc/hosts".to_owned()]), "etc/hosts");
        assert_eq!(
            describe_paths(&["etc/hosts".to_owned(), "var/lib/app".to_owned()]),
            "2 paths"
        );
    }
}
