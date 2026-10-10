// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Handing restores to agents and recording how they end.
//!
//! A restore is recorded before anything is sent, so the API can return at
//! once: `borg extract` on the agent may take far longer than any request
//! should wait. It goes to the agent right away if the agent is connected,
//! otherwise when it next connects, and the agent's `RestoreCompleted` (or
//! `OperationFailed`) settles it whenever it arrives. Every change is
//! pushed to the UI as [`ServerToUi::RestoreRunChanged`].

use std::time::Duration;

use shared::{
    protocol::{ServerToAgent, ServerToUi},
    types::RestoreRun,
};
use uuid::Uuid;

use crate::{
    AppState,
    db::restore_runs::{self, RestoreOutcome},
    error::ApiError,
};

/// Hands the pending restore `id` to its agent, if the agent is connected.
/// Leaves it pending, to go out on the agent's next connection, if not.
/// Either way the restore, as it then stands, is pushed to the UI, so a
/// restore waiting for an offline agent shows up as soon as it is recorded,
/// and returned; `None` when there is no restore `id`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn dispatch(
    state: &AppState,
    id: Uuid,
    agent_id: i64,
) -> Result<Option<RestoreRun>, ApiError> {
    hand_over(state, id, agent_id).await?;
    broadcast(state, id).await
}

/// How often a restore is offered to its agent in one go. The second try
/// covers a connection replaced between claiming the restore and sending it.
const HAND_OVER_ATTEMPTS: usize = 2;

/// Claims the pending restore `id` for its connected agent process and
/// sends it to that process. The connection can be replaced (the agent
/// restarted) between the two; the restore is then put back and offered to
/// the new process, so it is never recorded against a process that is not
/// the one running it.
async fn hand_over(state: &AppState, id: Uuid, agent_id: i64) -> Result<(), ApiError> {
    for _ in 0..HAND_OVER_ATTEMPTS {
        let Some(instance_id) = state.registry.instance_id(agent_id).await else {
            return Ok(());
        };
        let Some(restore) =
            restore_runs::claim_restore_run(&state.pool, id, instance_id.as_deref()).await?
        else {
            return Ok(());
        };
        let msg = ServerToAgent::RestoreFiles {
            request_id: id.to_string(),
            repo_id: shared::types::RepoId(restore.repo_id),
            archive_name: restore.archive_name,
            paths: restore.paths,
            target_path: restore.target_path,
        };
        if state
            .registry
            .send_to_instance(agent_id, instance_id.as_deref(), msg)
            .await
            .is_ok()
        {
            return Ok(());
        }
        // The agent went away, or is a different process, since the lookup.
        restore_runs::return_restore_run_to_pending(&state.pool, id).await?;
    }
    Ok(())
}

/// Called when an agent connects: fails the restores the agent can no
/// longer answer for, then hands it the ones waiting for it.
///
/// `instance_id` is the agent process that connected, from its Hello. A
/// restore handed to another process was lost when that process ended. An
/// agent that names no instance cannot tell a reconnect from a restart, so
/// every restore it was running is failed: its outcome can no longer be
/// told apart from a lost one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn on_agent_connected(
    state: &AppState,
    agent_id: i64,
    hostname: &str,
    instance_id: Option<&str>,
) -> Result<(), ApiError> {
    let lost = restore_runs::fail_lost_restore_runs(
        &state.pool,
        agent_id,
        instance_id,
        instance_id.is_none(),
        &format!("Agent '{hostname}' restarted while the restore was running"),
    )
    .await?;
    for id in lost {
        broadcast(state, id).await?;
    }
    for id in restore_runs::pending_restore_run_ids(&state.pool, agent_id).await? {
        dispatch(state, id, agent_id).await?;
    }
    Ok(())
}

/// What became of an agent's answer for a restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// The outcome is recorded.
    Recorded,
    /// The answer names no restore running on that agent.
    NotRunning,
    /// The database rejected the outcome. It is retried in the background
    /// until the database takes it, so the restore does not stay running
    /// for an answer that already arrived.
    Retrying,
}

/// First wait before an outcome the database rejected is offered again.
const RETRY_FIRST_DELAY: Duration = Duration::from_secs(1);
/// Longest wait between two offers of a rejected outcome.
const RETRY_MAX_DELAY: Duration = Duration::from_mins(1);

/// Records how a restore `agent_id` ran ended, as [`finish`] does. An
/// outcome the database rejects is not dropped: it is offered again, with
/// a growing delay, until it is recorded, turns out to be for no running
/// restore, or the server shuts down.
pub async fn settle(
    state: &AppState,
    agent_id: i64,
    request_id: &str,
    outcome: RestoreOutcome,
) -> Settlement {
    match finish(state, agent_id, request_id, &outcome).await {
        Ok(true) => Settlement::Recorded,
        Ok(false) => Settlement::NotRunning,
        Err(e) => {
            tracing::error!(
                agent_id,
                request_id = %request_id,
                error = %e,
                "failed to record how a restore ended; retrying in the background"
            );
            let task_state = state.clone();
            let request_id = request_id.to_owned();
            state.background_task_tracker.spawn_tracked(async move {
                retry_finish(&task_state, agent_id, &request_id, &outcome).await;
            });
            Settlement::Retrying
        }
    }
}

async fn retry_finish(state: &AppState, agent_id: i64, request_id: &str, outcome: &RestoreOutcome) {
    let mut delay = RETRY_FIRST_DELAY;
    loop {
        tokio::select! {
            biased;
            () = state.shutdown_token.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        match finish(state, agent_id, request_id, outcome).await {
            Ok(true) => {
                tracing::info!(
                    agent_id,
                    request_id = %request_id,
                    "recorded how a restore ended after the database rejected it"
                );
                return;
            }
            // Settled some other way meanwhile, e.g. failed as lost when the
            // agent came back as another process.
            Ok(false) => return,
            Err(e) => tracing::warn!(
                agent_id,
                request_id = %request_id,
                error = %e,
                "still failing to record how a restore ended"
            ),
        }
        delay = delay.saturating_mul(2).min(RETRY_MAX_DELAY);
    }
}

/// Records how a restore `agent_id` ran ended. Returns `false` when
/// `request_id` names no restore running on that agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if recording the outcome fails. Failing
/// to push the recorded restore to the UI is only logged, since the outcome
/// is stored either way.
pub async fn finish(
    state: &AppState,
    agent_id: i64,
    request_id: &str,
    outcome: &RestoreOutcome,
) -> Result<bool, ApiError> {
    let Ok(id) = Uuid::parse_str(request_id) else {
        return Ok(false);
    };
    if !restore_runs::finish_restore_run(&state.pool, id, agent_id, outcome).await? {
        return Ok(false);
    }
    if let Err(e) = broadcast(state, id).await {
        tracing::error!(
            restore_id = %id,
            error = %e,
            "failed to push a finished restore to the UI"
        );
    }
    Ok(true)
}

/// Pushes the restore `id`, as it now stands, to the UI, and returns it;
/// `None` when there is no restore `id`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn broadcast(state: &AppState, id: Uuid) -> Result<Option<RestoreRun>, ApiError> {
    let run = restore_runs::get_restore_run(&state.pool, id).await?;
    if let Some(run) = &run {
        state
            .ui_broadcast
            .send(ServerToUi::RestoreRunChanged { run: run.clone() });
    }
    Ok(run)
}

/// Makes the database reject restore outcomes, as a database that is down
/// or misbehaving would, and take them again.
#[cfg(test)]
pub(crate) mod test_support {
    use sqlx::PgPool;

    /// Rejects every outcome, successful or not.
    pub(crate) async fn reject_restore_outcomes(pool: &PgPool) {
        sqlx::query!(
            "ALTER TABLE restore_runs ADD CONSTRAINT test_rejects_outcomes CHECK (status IN \
             ('pending', 'running', 'cancelled')) NOT VALID"
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Rejects successful outcomes only.
    pub(crate) async fn reject_successful_restores(pool: &PgPool) {
        sqlx::query!(
            "ALTER TABLE restore_runs ADD CONSTRAINT test_rejects_outcomes CHECK (status <> \
             'success') NOT VALID"
        )
        .execute(pool)
        .await
        .unwrap();
    }

    pub(crate) async fn accept_restore_outcomes(pool: &PgPool) {
        sqlx::query!("ALTER TABLE restore_runs DROP CONSTRAINT test_rejects_outcomes")
            .execute(pool)
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use shared::types::RestoreRunStatus;
    use sqlx::PgPool;
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::db;

    struct Fixture {
        state: AppState,
        agent_id: i64,
        repo_id: i64,
    }

    async fn fixture(pool: PgPool) -> Fixture {
        let state = crate::test_support::build_test_state(pool.clone(), b"restore-runs-test-key");
        let agent = db::insert_agent(&pool, "restore-agent", None, "hash", None, None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            &pool,
            &db::InsertRepoParams {
                name: "restore-repo",
                repo_path: "/backups/restore",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"encrypted",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        Fixture {
            state,
            agent_id: agent.id,
            repo_id: repo.id,
        }
    }

    impl Fixture {
        async fn record_restore(&self) -> Uuid {
            restore_runs::insert_restore_run(
                &self.state.pool,
                &restore_runs::NewRestoreRun {
                    agent_id: self.agent_id,
                    repo_id: self.repo_id,
                    archive_name: "nightly",
                    paths: &["etc/hosts".to_owned()],
                    target_path: "/restore",
                    requested_by: "admin",
                },
            )
            .await
            .unwrap()
        }

        async fn connect(&self, instance_id: Option<&str>) -> mpsc::Receiver<ServerToAgent> {
            let (tx, rx) = mpsc::channel(8);
            self.state
                .registry
                .register(
                    self.agent_id,
                    tx,
                    false,
                    None,
                    instance_id.map(str::to_owned),
                )
                .await;
            rx
        }

        async fn status(&self, id: Uuid) -> RestoreRunStatus {
            restore_runs::get_restore_run(&self.state.pool, id)
                .await
                .unwrap()
                .unwrap()
                .status
        }
    }

    fn next_restore_change(rx: &mut broadcast::Receiver<ServerToUi>) -> shared::types::RestoreRun {
        let message = serde_json::to_value(rx.try_recv().unwrap()).unwrap();
        assert_eq!(
            message.pointer("/type"),
            Some(&serde_json::json!("RestoreRunChanged"))
        );
        serde_json::from_value(message.pointer("/payload/run").cloned().unwrap()).unwrap()
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_goes_to_a_connected_agent_at_once(pool: PgPool) {
        let fx = fixture(pool).await;
        let mut agent_rx = fx.connect(Some("instance-a")).await;
        let mut ui = fx.state.ui_broadcast.subscribe();
        let id = fx.record_restore().await;

        dispatch(&fx.state, id, fx.agent_id).await.unwrap();

        let Some(ServerToAgent::RestoreFiles {
            request_id,
            repo_id,
            archive_name,
            paths,
            target_path,
        }) = agent_rx.try_recv().ok()
        else {
            panic!("the agent was not handed the restore");
        };
        assert_eq!(request_id, id.to_string());
        assert_eq!(repo_id.0, fx.repo_id);
        assert_eq!(archive_name, "nightly");
        assert_eq!(paths, vec!["etc/hosts".to_owned()]);
        assert_eq!(target_path, "/restore");
        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
        assert_eq!(
            next_restore_change(&mut ui).status,
            RestoreRunStatus::Running
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_for_an_offline_agent_waits(pool: PgPool) {
        let fx = fixture(pool).await;
        let mut ui = fx.state.ui_broadcast.subscribe();
        let id = fx.record_restore().await;

        dispatch(&fx.state, id, fx.agent_id).await.unwrap();

        assert_eq!(fx.status(id).await, RestoreRunStatus::Pending);
        // Pushed all the same, so the UI lists it while it waits.
        let run = next_restore_change(&mut ui);
        assert_eq!(run.id, id.to_string());
        assert_eq!(run.status, RestoreRunStatus::Pending);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_no_longer_waiting_is_not_sent(pool: PgPool) {
        let fx = fixture(pool).await;
        let mut agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        restore_runs::cancel_restore_run(&fx.state.pool, id)
            .await
            .unwrap();

        dispatch(&fx.state, id, fx.agent_id).await.unwrap();

        assert!(agent_rx.try_recv().is_err());
        assert_eq!(fx.status(id).await, RestoreRunStatus::Cancelled);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_the_agent_could_not_take_waits_again(pool: PgPool) {
        let fx = fixture(pool).await;
        // Registered, but the connection is already gone.
        drop(fx.connect(Some("instance-a")).await);
        let id = fx.record_restore().await;

        dispatch(&fx.state, id, fx.agent_id).await.unwrap();

        assert_eq!(fx.status(id).await, RestoreRunStatus::Pending);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connecting_agent_gets_its_waiting_restores(pool: PgPool) {
        let fx = fixture(pool).await;
        let id = fx.record_restore().await;
        let mut agent_rx = fx.connect(Some("instance-a")).await;

        on_agent_connected(&fx.state, fx.agent_id, "restore-agent", Some("instance-a"))
            .await
            .unwrap();

        assert!(matches!(
            agent_rx.try_recv(),
            Ok(ServerToAgent::RestoreFiles { request_id, .. }) if request_id == id.to_string()
        ));
        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restarted_agent_fails_the_restore_it_lost(pool: PgPool) {
        let fx = fixture(pool).await;
        let _first = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        let _second = fx.connect(Some("instance-b")).await;
        let mut ui = fx.state.ui_broadcast.subscribe();

        on_agent_connected(&fx.state, fx.agent_id, "restore-agent", Some("instance-b"))
            .await
            .unwrap();

        let run = next_restore_change(&mut ui);
        assert_eq!(run.status, RestoreRunStatus::Failed);
        assert_eq!(
            run.error_message.as_deref(),
            Some("Agent 'restore-agent' restarted while the restore was running")
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_reconnected_agent_keeps_the_restore_it_is_running(pool: PgPool) {
        let fx = fixture(pool).await;
        let _first = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        let _second = fx.connect(Some("instance-a")).await;

        on_agent_connected(&fx.state, fx.agent_id, "restore-agent", Some("instance-a"))
            .await
            .unwrap();

        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_agents_answer_settles_the_restore(pool: PgPool) {
        let fx = fixture(pool).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        let mut ui = fx.state.ui_broadcast.subscribe();
        let outcome = RestoreOutcome {
            success: true,
            files_restored: 1,
            error_message: None,
        };

        let finished = finish(&fx.state, fx.agent_id, &id.to_string(), &outcome)
            .await
            .unwrap();

        assert!(finished);
        let run = next_restore_change(&mut ui);
        assert_eq!(run.status, RestoreRunStatus::Success);
        assert_eq!(run.files_restored, Some(1));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_answer_for_no_running_restore_is_not_taken(pool: PgPool) {
        let fx = fixture(pool).await;
        let outcome = RestoreOutcome {
            success: true,
            files_restored: 1,
            error_message: None,
        };

        let not_an_id = finish(&fx.state, fx.agent_id, "dry-run-17", &outcome)
            .await
            .unwrap();
        let unknown = finish(
            &fx.state,
            fx.agent_id,
            &Uuid::new_v4().to_string(),
            &outcome,
        )
        .await
        .unwrap();

        assert!(!not_an_id);
        assert!(!unknown);
    }

    fn success() -> RestoreOutcome {
        RestoreOutcome {
            success: true,
            files_restored: 1,
            error_message: None,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn settling_records_the_outcome_or_reports_no_restore(pool: PgPool) {
        let fx = fixture(pool).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();

        let recorded = settle(&fx.state, fx.agent_id, &id.to_string(), success()).await;
        let repeat = settle(&fx.state, fx.agent_id, &id.to_string(), success()).await;

        assert_eq!(recorded, Settlement::Recorded);
        assert_eq!(repeat, Settlement::NotRunning);
        assert_eq!(fx.status(id).await, RestoreRunStatus::Success);
        assert!(!fx.state.background_task_tracker.any_active());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_outcome_the_database_rejects_is_recorded_once_it_takes_it(pool: PgPool) {
        let fx = fixture(pool.clone()).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        test_support::reject_restore_outcomes(&pool).await;
        let mut ui = fx.state.ui_broadcast.subscribe();

        let settled = settle(&fx.state, fx.agent_id, &id.to_string(), success()).await;

        assert_eq!(settled, Settlement::Retrying);
        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
        // Long enough for the first retry to be rejected too.
        tokio::time::sleep(RETRY_FIRST_DELAY.saturating_mul(2)).await;
        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
        test_support::accept_restore_outcomes(&pool).await;
        assert!(
            fx.state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
        assert_eq!(fx.status(id).await, RestoreRunStatus::Success);
        let run = next_restore_change(&mut ui);
        assert_eq!(run.status, RestoreRunStatus::Success);
        assert_eq!(run.files_restored, Some(1));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_outcome_is_dropped_once_the_restore_is_settled_otherwise(pool: PgPool) {
        let fx = fixture(pool.clone()).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        test_support::reject_successful_restores(&pool).await;

        let settled = settle(&fx.state, fx.agent_id, &id.to_string(), success()).await;
        on_agent_connected(&fx.state, fx.agent_id, "restore-agent", Some("instance-b"))
            .await
            .unwrap();

        assert_eq!(settled, Settlement::Retrying);
        assert!(
            fx.state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
        assert_eq!(fx.status(id).await, RestoreRunStatus::Failed);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retrying_a_rejected_outcome_stops_at_shutdown(pool: PgPool) {
        let fx = fixture(pool.clone()).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        test_support::reject_restore_outcomes(&pool).await;

        let settled = settle(&fx.state, fx.agent_id, &id.to_string(), success()).await;
        fx.state.shutdown_token.cancel();

        assert_eq!(settled, Settlement::Retrying);
        assert!(
            fx.state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
        assert_eq!(fx.status(id).await, RestoreRunStatus::Running);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn dispatch_and_broadcast_return_the_restore(pool: PgPool) {
        let fx = fixture(pool).await;
        let id = fx.record_restore().await;

        let dispatched = dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        let broadcast_run = broadcast(&fx.state, id).await.unwrap();
        let missing = broadcast(&fx.state, Uuid::new_v4()).await.unwrap();

        assert_eq!(
            dispatched.as_ref().map(|run| run.status),
            Some(RestoreRunStatus::Pending)
        );
        assert_eq!(dispatched, broadcast_run);
        assert_eq!(missing, None);
    }

    /// The outcome is stored even when the restore cannot be read back to
    /// push it to the UI.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_outcome_is_recorded_even_if_the_ui_push_fails(pool: PgPool) {
        let fx = fixture(pool.clone()).await;
        let _agent_rx = fx.connect(Some("instance-a")).await;
        let id = fx.record_restore().await;
        dispatch(&fx.state, id, fx.agent_id).await.unwrap();
        // A NULL path cannot be read back as a `String`, so fetching the
        // restore for the UI fails while recording its outcome does not.
        sqlx::query!(
            "UPDATE restore_runs SET paths = ARRAY[NULL]::TEXT[] WHERE id = $1",
            id
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut ui = fx.state.ui_broadcast.subscribe();

        let finished = finish(&fx.state, fx.agent_id, &id.to_string(), &success())
            .await
            .unwrap();

        assert!(finished);
        assert!(ui.try_recv().is_err());
        let status = sqlx::query_scalar!("SELECT status FROM restore_runs WHERE id = $1", id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, RestoreRunStatus::Success.to_string());
    }
}
