// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Settling a backup that ends without a completion report of its own: one
//! the user cancelled, or one that was lost when its agent stopped or
//! restarted. Nothing else will ever announce its end, so whatever still
//! treats it as running - a dispatch task waiting on the completion bus, the
//! repository's op tracker, the UI's progress card - has to be told here.

use shared::protocol::ServerToUi;

use crate::{AppState, db, error::ApiError, ws::completion_bus::OperationOutcome};

/// Whether [`cancel`] announces the end of a backup it found nothing in
/// flight for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announce {
    /// Always: the user asked for this cancel, and a card still showing a
    /// backup for the repository must go even if its report was settled.
    Always,
    /// Only when a report was actually cancelled. For the agent's own
    /// "nothing running here" answer, which the system reset draws for every
    /// repository at once.
    IfReportCancelled,
}

/// Cancels `agent_id`'s in-flight backup report for `repo_id` and announces
/// the backup's end per `announce`. Returns whether a report was cancelled.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the report could not be updated.
pub async fn cancel(
    state: &AppState,
    agent_id: i64,
    repo_id: i64,
    announce: Announce,
) -> Result<bool, ApiError> {
    let schedules = db::cancel_backup_report(&state.pool, agent_id, repo_id).await?;
    let cancelled = !schedules.is_empty();
    if cancelled || matches!(announce, Announce::Always) {
        announce_stopped(
            state,
            agent_id,
            repo_id,
            schedules.into_iter().flatten().next(),
        )
        .await;
    }
    Ok(cancelled)
}

/// Announces that `agent_id`'s backup of `repo_id` is over without a
/// completion: wakes anything waiting on it, releases the repository's op
/// tracker entry if this backup still holds it, and tells the UI to drop it.
pub async fn announce_stopped(
    state: &AppState,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
) {
    state.completion_bus.publish(OperationOutcome {
        agent_id,
        repo_id,
        success: false,
    });
    if state
        .repo_op_tracker
        .clear_agent_backup(repo_id, agent_id)
        .await
    {
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });
    }
    match stopped_event(state, agent_id, repo_id, schedule_id).await {
        Ok(event) => state.ui_broadcast.send(event),
        Err(e) => {
            tracing::warn!(
                agent_id,
                repo_id,
                error = %e,
                "failed to name a stopped backup for the UI; clearing it silently"
            );
            state.ui_broadcast.clear_active_backup(repo_id);
        }
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
}

async fn stopped_event(
    state: &AppState,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
) -> Result<ServerToUi, ApiError> {
    let agent = db::get_agent_by_id(&state.pool, agent_id).await?;
    let repo = db::get_repo_by_id(&state.pool, repo_id).await?;
    Ok(ServerToUi::BackupStopped {
        hostname: agent.hostname,
        target_name: repo.name,
        repo_id,
        schedule_id,
    })
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::ws::{completion_bus::OperationOutcome, ui_broadcast::ActiveBackupSnapshot};

    const KEY: &[u8] = b"stopped-backup-test-secret-key";

    fn build_test_state(pool: PgPool) -> AppState {
        crate::test_support::build_test_state(pool, KEY)
    }

    async fn agent_and_repo(pool: &PgPool) -> (i64, i64) {
        let agent = db::insert_agent(pool, "stopped-host", None, "hash", None, None)
            .await
            .unwrap();
        let passphrase_encrypted = shared::crypto::encrypt_passphrase(
            "test-passphrase",
            &shared::crypto::derive_key(KEY).unwrap(),
        )
        .unwrap();
        let repo = db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: "stopped-repo",
                repo_path: "/backups/stopped",
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
        .unwrap();
        (agent.id, repo.id)
    }

    fn snapshot(repo_id: i64) -> ActiveBackupSnapshot {
        ActiveBackupSnapshot {
            hostname: "stopped-host".to_owned(),
            target_name: "stopped-repo".to_owned(),
            archive_name: None,
            schedule_id: None,
            repo_id,
            progress_line: None,
            started_at: chrono::Utc::now(),
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn cancel_settles_a_started_report_and_tells_everyone(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (agent_id, repo_id) = agent_and_repo(&pool).await;
        db::insert_backup_started(
            &pool,
            agent_id,
            repo_id,
            None,
            chrono::Utc::now(),
            None,
            None,
        )
        .await
        .unwrap();
        state
            .repo_op_tracker
            .set(
                repo_id,
                shared::protocol::RepoOpKind::AgentBackup,
                "stopped-host".to_owned(),
                Some(agent_id),
            )
            .await;
        state.ui_broadcast.set_active_backup(snapshot(repo_id));
        let mut ui = state.ui_broadcast.subscribe();
        let mut outcomes = state.completion_bus.subscribe();

        let cancelled = cancel(&state, agent_id, repo_id, Announce::IfReportCancelled)
            .await
            .unwrap();

        assert!(cancelled);
        let reports = db::list_reports_for_agent(&pool, agent_id, None, 10, 0)
            .await
            .unwrap();
        assert!(matches!(
            reports.first().map(|r| &r.status),
            Some(shared::types::ReportStatus::Cancelled)
        ));
        let OperationOutcome {
            agent_id: outcome_agent,
            repo_id: outcome_repo,
            success,
        } = outcomes.try_recv().unwrap();
        assert_eq!(
            (outcome_agent, outcome_repo, success),
            (agent_id, repo_id, false)
        );
        assert!(state.repo_op_tracker.get(repo_id).await.is_none());
        assert!(state.ui_broadcast.current_active_backups().is_empty());
        let events: Vec<ServerToUi> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            ServerToUi::BackupStopped { hostname, target_name, repo_id: r, .. }
                if hostname == "stopped-host" && target_name == "stopped-repo" && *r == repo_id
        )));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ServerToUi::RepoOpChanged { op: None, .. }))
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn cancel_with_nothing_in_flight_stays_quiet_unless_asked(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (agent_id, repo_id) = agent_and_repo(&pool).await;
        let mut ui = state.ui_broadcast.subscribe();

        let cancelled = cancel(&state, agent_id, repo_id, Announce::IfReportCancelled)
            .await
            .unwrap();
        assert!(!cancelled);
        assert!(ui.try_recv().is_err());

        state.ui_broadcast.set_active_backup(snapshot(repo_id));
        let cancelled = cancel(&state, agent_id, repo_id, Announce::Always)
            .await
            .unwrap();
        assert!(!cancelled);
        assert!(state.ui_broadcast.current_active_backups().is_empty());
        let events: Vec<ServerToUi> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ServerToUi::BackupStopped { .. }))
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn announce_leaves_another_operation_on_the_repo_alone(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let (agent_id, repo_id) = agent_and_repo(&pool).await;
        state
            .repo_op_tracker
            .set(
                repo_id,
                shared::protocol::RepoOpKind::ServerSync,
                "server".to_owned(),
                None,
            )
            .await;

        announce_stopped(&state, agent_id, repo_id, None).await;

        assert!(state.repo_op_tracker.get(repo_id).await.is_some());
    }
}
