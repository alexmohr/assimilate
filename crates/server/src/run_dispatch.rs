// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dispatching a schedule's targets outside the scheduler's own tick: a human
//! pressing Run now, and a host reconnecting with a run it missed while it was
//! unreachable. Both push a fresh config, send the run-now command, hold the
//! repository lock for the duration, and take part in power-session bookkeeping
//! identically - what differs is only how the run reads in the log, which is
//! what [`RunOrigin`] carries.

use std::fmt;

use shared::{
    protocol::{ServerToAgent, ServerToUi},
    types::{RepoId, ScheduleType},
};

use crate::{
    AppState, config_assembler, db, power,
    ws::{completion_bus, ui_broadcast::ActiveBackupSnapshot},
};

/// What asked for this run. Dispatch is identical either way; reading the log
/// afterwards, the difference is the whole story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOrigin {
    /// A human pressed Run now.
    Manual,
    /// A host reconnected carrying a run it missed while it was unreachable.
    CatchUp,
}

impl fmt::Display for RunOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => write!(f, "manual run"),
            Self::CatchUp => write!(f, "catch-up run"),
        }
    }
}

/// Everything about a run that is the same for every target of it.
pub struct RunRequest {
    /// Repository the run writes to.
    pub repo_id: RepoId,
    /// What the targets are being asked to do.
    pub schedule_type: ScheduleType,
    /// Schedule the run belongs to.
    pub schedule_id: i64,
    /// Correlates this run's `backup_reports` rows and run events.
    pub run_id: String,
    /// What asked for the run.
    pub origin: RunOrigin,
}

/// Runs each target in turn, returning how many of them the command actually
/// reached. Targets are never run concurrently: they share one repository, and
/// the repo lock would serialise them anyway.
pub async fn run_targets_sequential(
    state: AppState,
    targets: Vec<db::ScheduleRunTarget>,
    request: RunRequest,
) -> usize {
    let mut dispatched: usize = 0;
    for target in &targets {
        if run_target(&state, target, &request).await {
            dispatched = dispatched.saturating_add(1);
        }
    }
    dispatched
}

/// Returns whether the run-now command actually reached this target's agent.
async fn run_target(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    request: &RunRequest,
) -> bool {
    let repo_id = request.repo_id;
    let schedule_id = request.schedule_id;
    let origin = request.origin;
    let rx = state.completion_bus.subscribe();

    // Reserved for the whole duration of this run, before dispatch, for the same
    // reason the scheduler reserves before its own wake attempt: a scheduled run
    // sharing this agent's host must not have it shut down out from under it just
    // because this run raced to finish and release first. Dispatch here never
    // wakes anything itself (unlike the scheduler's ensure_target_power) -- it
    // only participates in the tracker so it can't be the reason a host it's
    // still using gets torn down, and hands off actual teardown below once it's
    // done.
    state
        .power_sessions
        .reserve(power::PowerHostKey::Agent(target.agent_id))
        .await;
    state
        .power_sessions
        .reserve(power::PowerHostKey::Repo(repo_id.0))
        .await;

    let _repo_guard = state.repo_lock.acquire(repo_id.0).await;

    let command_sent = push_config_and_trigger_target(state, target, request).await;

    // For backup schedules, broadcast BackupStarted even when the agent is
    // offline so the UI can immediately show the "Cancel Backup" button. The
    // backup_report is already in the DB as 'pending' from the caller.
    let is_backup = matches!(request.schedule_type, ScheduleType::Backup);

    if command_sent || is_backup {
        state
            .repo_op_tracker
            .set(
                repo_id.0,
                crate::scheduler::repo_op_kind_for(request.schedule_type),
                target.hostname.clone(),
                Some(target.agent_id),
            )
            .await;
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id: repo_id.0,
            op: state.repo_op_tracker.get(repo_id.0).await,
        });
    }

    if is_backup {
        broadcast_backup_started(state, target, repo_id, schedule_id).await;
    }

    if command_sent {
        let hostname = target.hostname.clone();
        let repo_id_val = repo_id.0;
        let outcome =
            completion_bus::wait_for_completion(&state.registry, rx, target.agent_id, repo_id_val)
                .await;

        state.repo_op_tracker.clear(repo_id_val).await;
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id: repo_id_val,
            op: None,
        });

        if outcome == completion_bus::CompletionOutcome::AgentDisconnected {
            tracing::warn!(
                hostname = %hostname,
                schedule_id,
                %origin,
                "agent disconnected before reporting completion"
            );
        }
    }

    release_target_power(state, target.agent_id, repo_id.0, request, &target.hostname).await;
    command_sent
}

/// Releases this run's `PowerSessionTracker` reservations, tearing down
/// whichever host(s) it was the last participant on -- mirroring `scheduler.rs`'s
/// `teardown_power_for_target`, but dispatch here never wakes anything itself, so
/// there's no pre-run snapshot to re-fetch: the current agent/repo rows are simply
/// fetched fresh. A row fetch failure is logged and that host's teardown skipped,
/// matching `ensure_target_power`'s existing best-effort convention.
async fn release_target_power(
    state: &AppState,
    agent_id: i64,
    repo_id: i64,
    request: &RunRequest,
    hostname: &str,
) {
    let origin = request.origin;
    let run_id = request.run_id.as_str();
    let power_ctx = power::PowerCtx {
        pool: &state.pool,
        registry: &state.registry,
        ui_broadcast: &state.ui_broadcast,
        power_sessions: &state.power_sessions,
    };
    match db::get_agent_by_id(&state.pool, agent_id).await {
        Ok(agent) => power::teardown_agent_power(power_ctx, &agent, repo_id, run_id).await,
        Err(e) => {
            tracing::warn!(agent_id, error = %e, %origin, "failed to load agent for power teardown");
            // teardown_agent_power would otherwise have released this --
            // without it, the reservation's count never reaches zero again,
            // permanently disabling shutdown/stop for this host. There's no
            // fresh row to act on, so this is a best-effort release of the
            // bookkeeping alone, with no SSH action attempted.
            state
                .power_sessions
                .end(power::PowerHostKey::Agent(agent_id))
                .await;
        }
    }
    match db::get_repo_by_id(&state.pool, repo_id).await {
        Ok(repo) => power::teardown_repo_power(power_ctx, &repo, agent_id, run_id, hostname).await,
        Err(e) => {
            tracing::warn!(repo_id, error = %e, %origin, "failed to load repo for power teardown");
            state
                .power_sessions
                .end(power::PowerHostKey::Repo(repo_id))
                .await;
        }
    }
}

/// Pushes a fresh config to the target agent, then sends the run-now command for
/// the schedule type. Returns whether the command was actually sent (i.e. the
/// agent was reachable for both steps).
async fn push_config_and_trigger_target(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    request: &RunRequest,
) -> bool {
    let origin = request.origin;
    let schedule_id = request.schedule_id;
    let agent_reachable = match config_assembler::assemble_config(
        &state.pool,
        &state.encryption_key,
        target.agent_id,
    )
    .await
    {
        Ok(config) => {
            if state
                .registry
                .send_to(target.agent_id, ServerToAgent::ConfigUpdate(config))
                .await
                .is_ok()
            {
                true
            } else {
                tracing::warn!(
                    hostname = %target.hostname,
                    %origin,
                    "agent not connected for config push"
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                %origin,
                "failed to assemble config"
            );
            false
        }
    };

    if !agent_reachable {
        return false;
    }

    let repo_id = request.repo_id;
    let msg = match request.schedule_type {
        ScheduleType::Check => ServerToAgent::RunCheckNow {
            repo_id,
            request_id: None,
        },
        ScheduleType::Verify => ServerToAgent::RunVerifyNow {
            repo_id,
            request_id: None,
        },
        ScheduleType::Backup => ServerToAgent::RunBackupNow {
            repo_id,
            schedule_id: Some(schedule_id),
            request_id: None,
            run_id: Some(request.run_id.clone()),
        },
    };

    match state.registry.send_to(target.agent_id, msg).await {
        Ok(()) => {
            tracing::info!(
                hostname = %target.hostname,
                schedule_id,
                %origin,
                "triggered"
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                %origin,
                "agent not connected"
            );
            false
        }
    }
}

async fn broadcast_backup_started(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    repo_id: RepoId,
    schedule_id: i64,
) {
    if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
        let started_at = chrono::Utc::now();
        state.ui_broadcast.set_active_backup(ActiveBackupSnapshot {
            hostname: target.hostname.clone(),
            target_name: target_name.clone(),
            archive_name: None,
            schedule_id: Some(schedule_id),
            repo_id: repo_id.0,
            progress_line: None,
            started_at,
        });
        state.ui_broadcast.send(ServerToUi::BackupStarted {
            hostname: target.hostname.clone(),
            target_name,
            archive_name: None,
            schedule_id: Some(schedule_id),
            started_at,
        });
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
}

#[cfg(test)]
mod tests {
    use db::InsertRepoParams;

    use super::*;
    use crate::{
        repo_op_tracker::RepoOpTracker,
        tunnel::TunnelManager,
        ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
    };

    #[test]
    fn run_origin_reads_as_the_kind_of_run_it_names() {
        assert_eq!(RunOrigin::Manual.to_string(), "manual run");
        assert_eq!(RunOrigin::CatchUp.to_string(), "catch-up run");
    }

    /// Builds an `AppState` around `pool` for tests that only need
    /// `release_target_power`'s dependencies (pool, registry,
    /// `ui_broadcast`, `power_sessions`) -- the rest are populated with inert
    /// defaults, matching `scheduler.rs`'s own test `AppState` boilerplate.
    fn test_app_state(pool: sqlx::PgPool) -> AppState {
        let ui_broadcast = UiBroadcast::new();
        AppState {
            pool: pool.clone(),
            encryption_key: shared::crypto::derive_key(b"schedules-power-test-key").unwrap(),
            registry: AgentRegistry::new(),
            ui_broadcast: ui_broadcast.clone(),
            tunnel_manager: TunnelManager::new(
                pool.clone(),
                ui_broadcast,
                "127.0.0.1:0".parse().unwrap(),
            ),
            log_buffer: crate::log_buffer::LogBuffer::default(),
            notification_service: crate::notifications::NotificationService::new(pool),
            completion_bus: CompletionBus::new(),
            repo_op_tracker: RepoOpTracker::default(),
            background_task_tracker: crate::background_tasks::BackgroundTaskTracker::default(),
            repo_lock: crate::RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: crate::new_pending_map(),
            pending_restores: crate::new_pending_map(),
            pending_vm_scans: crate::new_pending_map(),
            pending_vm_builds: crate::new_pending_map(),
            pending_migrations: crate::new_pending_map(),
            pending_deletes: crate::new_pending_map(),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            client_ip_resolver: crate::client_ip::ClientIpResolver::new(),
            task_registry: shared::task_registry::TaskRegistry::default(),
            user_rate_limiter: crate::rate_limit::UserRateLimiter::new(
                60,
                std::time::Duration::from_mins(1),
            ),
            session_idle_timeout_minutes: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
                480,
            )),
            power_sessions: power::PowerSessionTracker::default(),
        }
    }

    async fn insert_power_enabled_agent_and_repo(
        pool: &sqlx::PgPool,
    ) -> (db::AgentRow, db::RepoRow) {
        let agent = db::insert_agent(pool, "manual-power-host", None, "hash", None, None)
            .await
            .unwrap();
        let agent = db::update_agent_power(
            pool,
            agent.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
                start_agent_enabled: false,
                stop_agent_after_backup: false,
                // Nothing listens here -- the SSH attempt is expected to
                // fail, only the run-event trail is under test.
                ssh_host: Some("127.0.0.1"),
                ssh_port: 1,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();

        let passphrase_encrypted = shared::crypto::encrypt_passphrase(
            "test-pass",
            &shared::crypto::derive_key(b"test-secret-key-for-schedules").unwrap(),
        )
        .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "manual-power-repo",
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "127.0.0.1",
                ssh_port: 1,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        let repo = db::update_repo_power(
            pool,
            repo.id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
            },
        )
        .await
        .unwrap();

        (agent, repo)
    }

    /// Regression test for the "Run Now doesn't participate in
    /// `PowerSessionTracker`" bug: a sole reservation on both hosts must be
    /// torn down once a dispatched run releases it, exactly like the
    /// scheduler's own targets are.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn release_target_power_tears_down_sole_participant(pool: sqlx::PgPool) {
        let (agent, repo) = insert_power_enabled_agent_and_repo(&pool).await;
        let state = test_app_state(pool.clone());

        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Agent(agent.id), true, false)
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Repo(repo.id), true, false)
            .await;

        release_target_power(
            &state,
            agent.id,
            repo.id,
            &RunRequest {
                repo_id: RepoId(repo.id),
                schedule_type: ScheduleType::Backup,
                schedule_id: 1,
                run_id: "run-manual-1".to_owned(),
                origin: RunOrigin::Manual,
            },
            "manual-power-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-manual-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            event_types,
            vec!["shutdown_sent", "shutdown_sent"],
            "release as the sole participant must attempt shutdown for both the agent and repo \
             hosts"
        );
    }

    /// Regression test for the same bug's other half: releasing while a
    /// sibling schedule's reservation is still held on both hosts must do
    /// nothing, since a concurrent run is still relying on them staying up.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn release_target_power_is_a_noop_with_a_sibling_reservation_held(pool: sqlx::PgPool) {
        let (agent, repo) = insert_power_enabled_agent_and_repo(&pool).await;
        let state = test_app_state(pool.clone());

        // Two reservations on each host, simulating this manual run racing
        // a concurrent scheduled run that targets the same agent/repo.
        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Agent(agent.id), true, false)
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Repo(repo.id), true, false)
            .await;

        release_target_power(
            &state,
            agent.id,
            repo.id,
            &RunRequest {
                repo_id: RepoId(repo.id),
                schedule_type: ScheduleType::Backup,
                schedule_id: 1,
                run_id: "run-manual-2".to_owned(),
                origin: RunOrigin::Manual,
            },
            "manual-power-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-manual-2", agent.id, repo.id)
            .await
            .unwrap();
        assert!(
            events.is_empty(),
            "release while a sibling reservation is still held must not tear anything down: \
             {events:?}"
        );
    }

    /// Regression test: if the agent/repo row re-fetch inside
    /// `release_target_power` fails (transient DB error, or the row
    /// was deleted mid-run), the `PowerSessionTracker` reservation must
    /// still be released. Otherwise the session's count never returns to
    /// zero, silently and permanently disabling `shutdown_after_backup`/
    /// `stop_agent_after_backup` for that host until the server restarts.
    /// Uses a lazily-connected pool to a nonexistent database (matching
    /// `scheduler.rs`'s own `run_returns_promptly_when_shutdown_token_is_cancelled`
    /// test) so both row fetches fail deterministically without needing
    /// `DATABASE_URL`.
    #[tokio::test]
    async fn release_target_power_releases_the_reservation_even_when_the_row_fetch_fails() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let state = test_app_state(pool);
        let agent_id = 999_999;
        let repo_id = 888_888;

        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent_id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo_id))
            .await;

        release_target_power(
            &state,
            agent_id,
            repo_id,
            &RunRequest {
                repo_id: RepoId(repo_id),
                schedule_type: ScheduleType::Backup,
                schedule_id: 1,
                run_id: "run-manual-leak".to_owned(),
                origin: RunOrigin::Manual,
            },
            "leak-host",
        )
        .await;

        // If the reservation had leaked (the bug this regresses), this
        // would be the *first* decrement and return Some(..) instead of
        // None -- the tracker would still think a participant is present.
        assert!(
            state
                .power_sessions
                .end(power::PowerHostKey::Agent(agent_id))
                .await
                .is_none(),
            "the agent reservation must already be released by the failed fetch's fallback"
        );
        assert!(
            state
                .power_sessions
                .end(power::PowerHostKey::Repo(repo_id))
                .await
                .is_none(),
            "the repo reservation must already be released by the failed fetch's fallback"
        );
    }

    /// Regression test: when `assemble_config` fails - a transient DB error,
    /// or the agent row deleted mid-run - `push_config_and_trigger_target`
    /// must report the target unreachable and send nothing, rather than
    /// triggering a run against a config the agent never received.
    ///
    /// This arm was previously covered only by chance: no test drove it, and
    /// it registered as covered only when some unrelated test happened to
    /// fail a config assembly first. That made the line flap between covered
    /// and uncovered from run to run and moved the repository's aggregate
    /// coverage by a few hundredths of a percent either way, which is enough
    /// to fail `analyze-coverage-diff.js`'s strict comparison on an unrelated
    /// pull request.
    ///
    /// Uses a lazily-connected pool to a nonexistent database - the same
    /// deterministic, no-`DATABASE_URL` pattern as
    /// `release_target_power_releases_the_reservation_even_when_the_row_fetch_fails`
    /// above - so the fetch inside `assemble_config` fails on every run.
    #[tokio::test]
    async fn push_config_and_trigger_target_reports_unreachable_when_config_assembly_fails() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let state = test_app_state(pool);
        let target = db::ScheduleRunTarget {
            agent_id: 999_999,
            hostname: "unreachable-host".to_owned(),
        };

        let reachable = push_config_and_trigger_target(
            &state,
            &target,
            &RunRequest {
                repo_id: RepoId(888_888),
                schedule_type: ScheduleType::Backup,
                schedule_id: 777_777,
                run_id: "run-manual-config-assembly-failure".to_owned(),
                origin: RunOrigin::Manual,
            },
        )
        .await;

        assert!(
            !reachable,
            "a target whose config could not be assembled must be reported unreachable"
        );
    }
}
