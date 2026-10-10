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

use shared::protocol::{ServerToAgent, ServerToUi};
use uuid::Uuid;

use crate::{
    AppState,
    db::restore_runs::{self, RestoreOutcome},
    error::ApiError,
};

/// Hands the pending restore `id` to its agent, if the agent is connected.
/// Leaves it pending, to go out on the agent's next connection, if not.
/// Either way the restore, as it then stands, is pushed to the UI, so a
/// restore waiting for an offline agent shows up as soon as it is recorded.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn dispatch(state: &AppState, id: Uuid, agent_id: i64) -> Result<(), ApiError> {
    hand_over(state, id, agent_id).await?;
    broadcast(state, id).await
}

/// Claims the pending restore `id` for its connected agent and sends it.
async fn hand_over(state: &AppState, id: Uuid, agent_id: i64) -> Result<(), ApiError> {
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
    if state.registry.send_to(agent_id, msg).await.is_err() {
        // The agent went away between the lookup and the send.
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

/// Records how a restore `agent_id` ran ended. Returns `false` when
/// `request_id` names no restore running on that agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
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
    broadcast(state, id).await?;
    Ok(true)
}

/// Pushes the restore `id`, as it now stands, to the UI.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn broadcast(state: &AppState, id: Uuid) -> Result<(), ApiError> {
    if let Some(run) = restore_runs::get_restore_run(&state.pool, id).await? {
        state
            .ui_broadcast
            .send(ServerToUi::RestoreRunChanged { run });
    }
    Ok(())
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
}
