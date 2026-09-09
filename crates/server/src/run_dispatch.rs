// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dispatching a schedule's targets outside the scheduler's own tick: a human
//! pressing Run now, and a host reconnecting with a run it missed while it was
//! unreachable. Both push a fresh config, send the run-now command, hold the
//! repository lock for the duration, and take part in power-session bookkeeping
//! identically - what differs is only how the run reads in the log, which is
//! what [`RunOrigin`] carries.

use std::fmt;

use chrono::{DateTime, Utc};
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    schedule::calculate_next_run,
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
    /// Repositories the run writes to, in write order. A schedule with several
    /// targets writes each of them, the same as its scheduler tick does.
    pub repo_ids: Vec<RepoId>,
    /// What the targets are being asked to do.
    pub schedule_type: ScheduleType,
    /// Schedule the run belongs to.
    pub schedule_id: i64,
    /// The schedule's cron expression, needed to advance `next_run_at` past
    /// this run the same way a scheduled tick does.
    pub cron_expression: String,
    /// When this run was asked for, recorded as the schedule's `last_run_at`
    /// once dispatch reaches a target - the moment the request was made, not
    /// whenever that target's backup happens to finish.
    pub now: DateTime<Utc>,
    /// Correlates this run's `backup_reports` rows and run events.
    pub run_id: String,
    /// What asked for the run.
    pub origin: RunOrigin,
}

/// Runs each (host, repository) pair in turn, returning how many of them the
/// command actually reached. Nothing runs concurrently: pairs sharing a
/// repository would be serialised by the repo lock anyway, and the order here -
/// host-major, then write order - is the one `list_due_schedules` hands the
/// scheduler, so a manual run writes the same copies in the same sequence a
/// scheduled one does.
pub async fn run_targets_sequential(
    state: AppState,
    targets: Vec<db::ScheduleRunTarget>,
    request: RunRequest,
) -> usize {
    let mut dispatched: usize = 0;
    let mut marked_triggered = false;
    for target in &targets {
        for repo_id in &request.repo_ids {
            if run_target(&state, target, &request, *repo_id, &mut marked_triggered).await {
                dispatched = dispatched.saturating_add(1);
            }
        }
    }
    dispatched
}

/// Returns whether the run-now command actually reached this target's agent.
async fn run_target(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    request: &RunRequest,
    repo_id: RepoId,
    marked_triggered: &mut bool,
) -> bool {
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

    let command_sent = push_config_and_trigger_target(state, target, request, repo_id).await;

    // Reaching the first target of the run is what a scheduled tick's own
    // success path counts as "triggered", so this mirrors it here rather than
    // waiting for the whole run to finish - the schedule's `last_run_at`/
    // `next_run_at` must not still read as it did before this run started
    // just because the backup itself is still in flight.
    if command_sent && !*marked_triggered {
        record_schedule_triggered(state, request).await;
        *marked_triggered = true;
    }

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

/// Advances the schedule's `last_run_at` to `request.now` and `next_run_at` to
/// the next cron occurrence from it, the same bookkeeping
/// `scheduler::mark_schedule_triggered_once` does for a scheduled tick's own
/// successful dispatch. Without this, a manual "Run now" or a caught-up run
/// left the schedule looking exactly as overdue as before it ran: the
/// Overview page's "Last run" never moved, and a later scheduled tick could
/// re-trigger the very occurrence this run just covered.
async fn record_schedule_triggered(state: &AppState, request: &RunRequest) {
    let schedule_id = request.schedule_id;
    let tz = match db::get_schedule_timezone(&state.pool).await {
        Ok(tz) => tz,
        Err(e) => {
            tracing::error!(
                schedule_id,
                error = %e,
                "failed to load timezone, not marking schedule triggered"
            );
            return;
        }
    };
    let next = match calculate_next_run(&request.cron_expression, request.now, tz) {
        Ok(next) => next,
        Err(e) => {
            tracing::error!(
                schedule_id,
                cron = %request.cron_expression,
                error = %e,
                "invalid cron expression, not marking schedule triggered"
            );
            return;
        }
    };
    if let Err(e) = db::mark_schedule_triggered(&state.pool, schedule_id, request.now, next).await {
        tracing::error!(schedule_id, error = %e, "failed to mark schedule triggered");
    }
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
    repo_id: RepoId,
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

    #[test]
    fn run_origin_reads_as_the_kind_of_run_it_names() {
        assert_eq!(RunOrigin::Manual.to_string(), "manual run");
        assert_eq!(RunOrigin::CatchUp.to_string(), "catch-up run");
    }

    /// The key material this module's tests derive their encryption key from.
    const TEST_KEY_MATERIAL: &[u8] = b"run-dispatch-power-test-key";

    fn test_app_state(pool: sqlx::PgPool) -> AppState {
        crate::test_support::build_test_state(pool, TEST_KEY_MATERIAL)
    }

    /// Puts both of a target's hosts in the state a finished run leaves behind:
    /// reserved and recorded as woken. `siblings` is how many *other* runs are
    /// holding the same two hosts at the same time.
    async fn reserve_woken_hosts(state: &AppState, agent_id: i64, repo_id: i64, siblings: usize) {
        for key in [
            power::PowerHostKey::Agent(agent_id),
            power::PowerHostKey::Repo(repo_id),
        ] {
            for _ in 0..=siblings {
                state.power_sessions.reserve(key).await;
            }
            state.power_sessions.record_outcome(key, true, false).await;
        }
    }

    /// Releases the pair the way a finished run does.
    async fn release_after_run(state: &AppState, agent_id: i64, repo_id: i64, run_id: &str) {
        release_target_power(
            state,
            agent_id,
            repo_id,
            &RunRequest {
                repo_ids: vec![RepoId(repo_id)],
                schedule_type: ScheduleType::Backup,
                schedule_id: 1,
                cron_expression: "0 2 * * *".to_owned(),
                now: Utc::now(),
                run_id: run_id.to_owned(),
                origin: RunOrigin::Manual,
            },
            "manual-power-host",
        )
        .await;
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

    /// An agent, a repository `assemble_config` can actually build a config
    /// for (passphrase encrypted with this module's own `TEST_KEY_MATERIAL`,
    /// matching `test_app_state`'s encryption key, and an SSH host key set -
    /// mirroring `scheduler.rs`'s own `setup_due_schedule`), and a schedule
    /// with one target on that agent.
    async fn insert_schedule_with_target(
        pool: &sqlx::PgPool,
        hostname: &str,
        cron_expression: &str,
    ) -> (db::AgentRow, db::RepoRow, db::ScheduleRow) {
        let agent = db::insert_agent(pool, hostname, None, "hash", None, None)
            .await
            .unwrap();
        let passphrase_encrypted = shared::crypto::encrypt_passphrase(
            "test-pass",
            &shared::crypto::derive_key(TEST_KEY_MATERIAL).unwrap(),
        )
        .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: &format!("{hostname}-repo"),
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "127.0.0.1",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAATEST")
            .await
            .unwrap();
        let schedule = db::insert_schedule(
            pool,
            repo.id,
            &db::ScheduleParams {
                wake_override: shared::types::ScheduleWakeOverride::HostDefault,
                name: "run-dispatch-test-schedule",
                schedule_type: "backup",
                cron_expression,
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 0,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_missed_runs: false,
                catch_up_min_lead_minutes: 120,
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .unwrap();
        (agent, repo, schedule)
    }

    /// Regression test for the "manual runs don't update the schedule's last
    /// run" bug: `record_schedule_triggered` is the piece `run_target` calls
    /// once dispatch reaches a target, and on its own must advance both
    /// `last_run_at` (to the moment the run was asked for) and `next_run_at`
    /// (to the next cron occurrence from it) - exactly what
    /// `scheduler::mark_schedule_triggered_once` does for a scheduled tick.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn record_schedule_triggered_advances_last_run_and_next_run_at(pool: sqlx::PgPool) {
        let (_agent, _repo, schedule) =
            insert_schedule_with_target(&pool, "record-triggered-host", "0 2 * * *").await;
        let state = test_app_state(pool.clone());
        let now = Utc::now();

        record_schedule_triggered(
            &state,
            &RunRequest {
                repo_ids: vec![],
                schedule_type: ScheduleType::Backup,
                schedule_id: schedule.id,
                cron_expression: schedule.cron_expression.clone(),
                now,
                run_id: "run-record-triggered".to_owned(),
                origin: RunOrigin::Manual,
            },
        )
        .await;

        let updated = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
        assert_eq!(
            updated.last_run_at.map(|t| t.timestamp()),
            Some(now.timestamp()),
            "last_run_at must advance to the moment the run was asked for"
        );
        assert!(
            updated.next_run_at.is_some_and(|next| next > now),
            "next_run_at must advance past this run, not stay at whatever it was before"
        );
    }

    /// Regression test for the same bug, one level up: `run_target` - what a
    /// manual "Run now" and a caught-up run both dispatch through - must
    /// itself call `record_schedule_triggered` once it actually reaches the
    /// agent, not just when the whole run finishes. The agent's completion is
    /// published concurrently with `run_target` rather than awaited
    /// afterwards, since `run_target` doesn't return until the run completes
    /// and nothing here ever reports one back on its own.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn run_target_marks_the_schedule_triggered_once_dispatch_reaches_the_agent(
        pool: sqlx::PgPool,
    ) {
        let (agent, repo, schedule) =
            insert_schedule_with_target(&pool, "run-target-triggers-host", "0 2 * * *").await;
        let state = test_app_state(pool.clone());
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        state.registry.register(agent.id, tx, false, None).await;

        let target = db::ScheduleRunTarget {
            agent_id: agent.id,
            hostname: agent.hostname.clone(),
        };
        let now = Utc::now();
        let request = RunRequest {
            repo_ids: vec![RepoId(repo.id)],
            schedule_type: ScheduleType::Backup,
            schedule_id: schedule.id,
            cron_expression: schedule.cron_expression.clone(),
            now,
            run_id: "run-target-triggers".to_owned(),
            origin: RunOrigin::Manual,
        };
        let mut marked_triggered = false;

        tokio::join!(
            run_target(
                &state,
                &target,
                &request,
                RepoId(repo.id),
                &mut marked_triggered
            ),
            async {
                state
                    .completion_bus
                    .publish(completion_bus::OperationOutcome {
                        agent_id: agent.id,
                        repo_id: repo.id,
                        success: true,
                    });
            }
        );

        assert!(
            marked_triggered,
            "run_target must mark the schedule triggered once it reaches the agent"
        );
        let updated = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
        assert!(
            updated.last_run_at.is_some(),
            "a manual run that reached the agent must advance last_run_at"
        );
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
        reserve_woken_hosts(&state, agent.id, repo.id, 0).await;

        release_after_run(&state, agent.id, repo.id, "run-manual-1").await;

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
        // One sibling reservation on each host, simulating this run racing a
        // concurrent scheduled run that targets the same agent and repo.
        reserve_woken_hosts(&state, agent.id, repo.id, 1).await;

        release_after_run(&state, agent.id, repo.id, "run-manual-2").await;

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
                repo_ids: vec![RepoId(repo_id)],
                schedule_type: ScheduleType::Backup,
                schedule_id: 1,
                cron_expression: "0 2 * * *".to_owned(),
                now: Utc::now(),
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
                repo_ids: vec![RepoId(888_888)],
                schedule_type: ScheduleType::Backup,
                schedule_id: 777_777,
                cron_expression: "0 2 * * *".to_owned(),
                now: Utc::now(),
                run_id: "run-manual-config-assembly-failure".to_owned(),
                origin: RunOrigin::Manual,
            },
            RepoId(888_888),
        )
        .await;

        assert!(
            !reachable,
            "a target whose config could not be assembled must be reported unreachable"
        );
    }
}
