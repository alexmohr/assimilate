// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use chrono::{DateTime, Utc};
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    schedule::calculate_next_run,
    types::{OnFailure, RepoId, ScheduleType, SystemEventType},
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    AppState, RepoLock,
    api::repos::sync_existing_archives,
    config_assembler, db,
    db::DueScheduleRow,
    power,
    repo_op_tracker::RepoOpTracker,
    tunnel::TunnelManager,
    ws::{
        completion_bus, completion_bus::CompletionBus, registry::AgentRegistry,
        ui_broadcast::UiBroadcast,
    },
};

const TICK_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_hours(1);
const SYNC_CHECK_INTERVAL: Duration = Duration::from_mins(1);
const SESSION_CLEANUP_INTERVAL: Duration = Duration::from_hours(1);
const SYNC_WARN_DURATION: Duration = Duration::from_mins(5);
/// Default number of consecutive failures to reach a schedule's agent (offline or
/// unreachable at trigger time) tolerated before the schedule is disabled rather than
/// retried again - see [`db::record_schedule_failure`]. Matches the `schedules
/// .missed_backup_threshold` column's DB default; each schedule's own configured
/// value (`DueScheduleRow::missed_backup_threshold`) is used at the actual call site
/// in [`record_schedule_failure_once`], so this constant only backs schedules with no
/// targets (never actually reached) and tests exercising `db::record_schedule_failure`
/// directly. A disabled schedule is re-enabled automatically once every one of its
/// targets reconnects (see
/// `ws::handler::reenable_system_disabled_schedules_on_reconnect`).
const MAX_CONSECUTIVE_FAILURES: i32 = 3;
/// Fallback backoff (hours), applied by [`record_schedule_failure_once`] when the
/// schedule's own cron expression can't be evaluated for `next_run_at` (an invalid
/// expression, one with no next occurrence, or an ambiguous/invalid local time for this
/// particular tick). Without some fixed backoff here, a schedule in that state would
/// never get a real `next_run_at` to fall back on and would keep re-triggering on every
/// scheduler tick forever - the exact unbounded-retry problem this whole mechanism
/// exists to prevent.
const CRON_EVAL_FAILURE_BACKOFF_HOURS: i64 = 1;

/// Ticks due schedules on `TICK_INTERVAL` until `shutdown_token` fires. Split out of
/// `run()` (rather than an inline closure) purely to keep `run()` itself under the
/// line-count limit now that `TickDeps` carries the notification fields too.
async fn run_schedule_ticks(state: AppState, shutdown_token: tokio_util::sync::CancellationToken) {
    let mut interval = tokio::time::interval(TICK_INTERVAL);
    loop {
        tokio::select! {
            biased;
            () = shutdown_token.cancelled() => return,
            _ = interval.tick() => {}
        }
        if let Err(e) = tick(&TickDeps {
            pool: &state.pool,
            registry: &state.registry,
            encryption_key: &state.encryption_key,
            tunnel_manager: &state.tunnel_manager,
            completion_bus: &state.completion_bus,
            repo_lock: &state.repo_lock,
            repo_op_tracker: &state.repo_op_tracker,
            ui_broadcast: &state.ui_broadcast,
            background_task_tracker: &state.background_task_tracker,
            power_sessions: &state.power_sessions,
            notification_service: &state.notification_service,
            task_registry: &state.task_registry,
        })
        .await
        {
            tracing::error!(error = %e, "scheduler tick failed");
        }
    }
}

/// Main scheduler loop: ticks schedules, runs retention, syncs repos, and cleans up sessions.
/// Every inner loop races its interval tick against `state.shutdown_token`, so the whole
/// function returns promptly once shutdown starts instead of keeping the process alive
/// (and, in a coverage build, keeping LLVM's atexit-flush from ever running) until the
/// runtime is torn down out from under it.
pub async fn run(state: AppState) {
    let _receiver = state.completion_bus.subscribe();
    let schedule_state = state.clone();
    let retention_pool = state.pool.clone();
    let sync_state = state.clone();
    let session_pool = state.pool.clone();
    let shutdown_token = state.shutdown_token.clone();

    let schedule_task = run_schedule_ticks(schedule_state, shutdown_token.clone());

    let retention_task = {
        let shutdown_token = shutdown_token.clone();
        async move {
            let mut interval = tokio::time::interval(RETENTION_INTERVAL);
            loop {
                tokio::select! {
                    biased;
                    () = shutdown_token.cancelled() => return,
                    _ = interval.tick() => {}
                }
                if let Err(e) = run_retention_cleanup(&retention_pool).await {
                    tracing::error!(error = %e, "retention cleanup failed");
                }
            }
        }
    };

    let sync_task = {
        let shutdown_token = shutdown_token.clone();
        async move {
            let mut interval = tokio::time::interval(SYNC_CHECK_INTERVAL);
            loop {
                tokio::select! {
                    biased;
                    () = shutdown_token.cancelled() => return,
                    _ = interval.tick() => {}
                }
                run_repo_sync(
                    &sync_state.pool,
                    &sync_state.encryption_key,
                    &sync_state.ui_broadcast,
                    &sync_state.repo_op_tracker,
                    &sync_state.repo_lock,
                    &sync_state.background_task_tracker,
                    &sync_state.task_registry,
                )
                .await;
            }
        }
    };

    let session_cleanup_task = async move {
        let mut interval = tokio::time::interval(SESSION_CLEANUP_INTERVAL);
        loop {
            tokio::select! {
                biased;
                () = shutdown_token.cancelled() => return,
                _ = interval.tick() => {}
            }
            match db::delete_expired_sessions(&session_pool).await {
                Ok(count) if count > 0 => {
                    tracing::debug!(count, "deleted expired sessions");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "session cleanup failed");
                }
            }
        }
    };

    tokio::join!(
        schedule_task,
        retention_task,
        sync_task,
        session_cleanup_task
    );
}

/// Check every repo with a `sync_schedule` cron and trigger a sync if due.
pub async fn run_repo_sync(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    ui_broadcast: &UiBroadcast,
    repo_op_tracker: &RepoOpTracker,
    repo_lock: &RepoLock,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) {
    let repos = match db::list_repos_with_sync_schedule(pool).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to list repos for sync");
            return;
        }
    };

    let tz = db::get_schedule_timezone(pool)
        .await
        .unwrap_or(chrono_tz::Tz::UTC);
    let now = Utc::now();

    let importing_ids: std::collections::HashSet<i64> =
        match db::list_importing_repo_ids(pool).await {
            Ok(ids) => ids.into_iter().collect(),
            Err(e) => {
                tracing::error!(error = %e, "failed to list importing repos for sync guard");
                return;
            }
        };

    for repo in repos {
        if !repo.enabled {
            continue;
        }

        if importing_ids.contains(&repo.id) {
            tracing::debug!(
                repo_id = repo.id,
                "skipping scheduled sync: import in progress"
            );
            continue;
        }

        let Some(ref cron_expr) = repo.sync_schedule else {
            continue;
        };

        let from = repo.last_synced_at.unwrap_or(DateTime::UNIX_EPOCH);
        let next_run = match calculate_next_run(cron_expr, from, tz) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    repo_id = repo.id,
                    cron = %cron_expr,
                    error = %e,
                    "invalid sync_schedule cron, skipping"
                );
                continue;
            }
        };

        if next_run > now {
            continue;
        }

        // ImportingGuard::acquire's returned guard is held for the task's lifetime
        // so a panic inside sync_existing_archives still clears
        // repo_import_state.importing (via spawned cleanup, since Drop can't
        // await) instead of leaving it permanently "importing" - which would
        // also block this repo's own periodic sync forever, since it's skipped
        // above whenever `importing_ids` contains it.
        let importing_guard =
            match db::ImportingGuard::acquire(pool, repo.id, task_registry.clone()).await {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(
                        repo_id = repo.id,
                        error = %e,
                        "failed to set importing flag for scheduled sync"
                    );
                    continue;
                }
            };

        // set_guarded's returned guard is held for the task's lifetime so a panic
        // inside sync_existing_archives still clears this repo's entry (via
        // spawned cleanup, since Drop can't await) instead of leaving it
        // permanently "active" - see RepoOpGuard. The guard only ever clears the
        // exact operation it was created for, so it can't clobber a later
        // operation that reuses this repo_id after this one's own clear_now()
        // already ran.
        let op_clear_guard = repo_op_tracker
            .set_guarded(
                repo.id,
                shared::protocol::RepoOpKind::ServerSync,
                "server".to_owned(),
                task_registry.clone(),
            )
            .await;
        ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id: repo.id,
            op: repo_op_tracker.get(repo.id).await,
        });

        // Claimed synchronously (before spawning) rather than inside run_scheduled_repo_sync,
        // so background_task_tracker.any_active() reflects this task immediately instead of
        // only once it gets its first poll - the same race class fixed for
        // enrich_archive_stats_background in #371. Without this, a test that only waits on
        // repo_op_tracker (which clears earlier, right after sync_existing_archives returns)
        // can observe run_scheduled_repo_sync's own bookkeeping - handle_scheduled_sync_success
        // in particular - as a scheduling coincidence, which is exactly what produced
        // non-deterministic coverage on scheduler.rs.
        let task_guard = background_task_tracker.begin();
        let task = ScheduledRepoSync {
            pool: pool.clone(),
            encryption_key: *encryption_key,
            ui_broadcast: ui_broadcast.clone(),
            op_clear_guard,
            importing_guard,
            repo_lock: repo_lock.clone(),
            background_task_tracker: background_task_tracker.clone(),
            task_guard,
            task_registry: task_registry.clone(),
            repo_id: repo.id,
            repo_name: repo.name.clone(),
        };
        tokio::spawn(run_scheduled_repo_sync(task));
    }
}

struct ScheduledRepoSync {
    pool: PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: UiBroadcast,
    op_clear_guard: crate::repo_op_tracker::RepoOpGuard,
    importing_guard: db::ImportingGuard,
    repo_lock: RepoLock,
    background_task_tracker: crate::background_tasks::BackgroundTaskTracker,
    task_guard: crate::background_tasks::BackgroundTaskGuard,
    task_registry: shared::task_registry::TaskRegistry,
    repo_id: i64,
    repo_name: String,
}

async fn run_scheduled_repo_sync(task: ScheduledRepoSync) {
    let ScheduledRepoSync {
        pool,
        encryption_key,
        ui_broadcast,
        op_clear_guard,
        importing_guard,
        repo_lock,
        background_task_tracker,
        task_guard: _task_guard,
        task_registry,
        repo_id,
        repo_name,
    } = task;

    let _repo_guard = repo_lock.acquire(repo_id).await;
    let start = std::time::Instant::now();
    let sync_result = sync_existing_archives(
        &pool,
        &encryption_key,
        repo_id,
        &ui_broadcast,
        &background_task_tracker,
        &task_registry,
    )
    .await;

    op_clear_guard.clear_now().await;
    ui_broadcast.send(ServerToUi::RepoOpChanged { repo_id, op: None });

    match sync_result {
        Ok((added, removed)) => {
            handle_scheduled_sync_success(ScheduledSyncSuccess {
                pool: &pool,
                ui_broadcast: &ui_broadcast,
                importing_guard,
                repo_id,
                repo_name: &repo_name,
                elapsed: start.elapsed(),
                added,
                removed,
            })
            .await;
        }
        Err(crate::error::ApiError::NotFound(ref reason)) => {
            tracing::warn!(
                repo_id,
                repo_name = %repo_name,
                reason = %reason,
                "skipping sync for repo that no longer exists"
            );
            importing_guard.clear_now().await;
            crate::api::repos::clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
            ui_broadcast.send(ServerToUi::DataChanged);
        }
        Err(e) => {
            let elapsed = start.elapsed();
            let msg = format!(
                "periodic sync failed for '{repo_name}' after {:.1}s: {e}",
                elapsed.as_secs_f64()
            );
            tracing::error!("{msg}");
            if let Err(log_err) =
                db::insert_system_event(&pool, SystemEventType::RepoSyncFailed, None, &msg).await
            {
                tracing::error!(error = %log_err, "failed to log sync event");
            }
            importing_guard.clear_now().await;
            if let Err(e2) = db::set_repo_import_error(&pool, repo_id, Some(&format!("{e}"))).await
            {
                tracing::error!(repo_id, error = %e2, "failed to set import_error");
            }
            crate::api::repos::clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
            ui_broadcast.send(ServerToUi::DataChanged);
        }
    }
}

/// Arguments for [`handle_scheduled_sync_success`], bundled (rather than
/// passed individually) to stay under clippy's argument-count limit.
struct ScheduledSyncSuccess<'a> {
    pool: &'a PgPool,
    ui_broadcast: &'a UiBroadcast,
    importing_guard: db::ImportingGuard,
    repo_id: i64,
    repo_name: &'a str,
    elapsed: std::time::Duration,
    added: u64,
    removed: u64,
}

/// Handles bookkeeping and logging after a scheduled sync completed
/// successfully: clears importing/error state and records a system event for
/// content changes or a slow-sync warning.
async fn handle_scheduled_sync_success(success: ScheduledSyncSuccess<'_>) {
    let ScheduledSyncSuccess {
        pool,
        ui_broadcast,
        importing_guard,
        repo_id,
        repo_name,
        elapsed,
        added,
        removed,
    } = success;
    let duration_secs = elapsed.as_secs();

    if let Err(e) = db::update_repo_last_synced(pool, repo_id).await {
        tracing::error!(repo_id, error = %e, "failed to update last_synced_at");
    }
    if let Err(e) =
        db::update_repo_last_op(pool, repo_id, "server_sync", Utc::now(), "server").await
    {
        tracing::error!(repo_id, error = %e, "failed to update last_op after sync");
    }

    importing_guard.clear_now().await;
    if let Err(e) = db::set_repo_import_error(pool, repo_id, None).await {
        tracing::error!(repo_id, error = %e, "failed to clear import_error after sync");
    }
    crate::api::repos::clear_import_progress_state(pool, ui_broadcast, repo_id).await;
    ui_broadcast.send(ServerToUi::DataChanged);

    if added > 0 || removed > 0 {
        let msg = format!(
            "periodic sync for '{repo_name}': added {added}, removed {removed} archives in \
             {duration_secs}s",
        );
        tracing::info!("{msg}");
        if let Err(e) = db::insert_system_event(pool, SystemEventType::RepoSync, None, &msg).await {
            tracing::error!(error = %e, "failed to log sync event");
        }
    }

    if elapsed > SYNC_WARN_DURATION {
        let msg = format!(
            "periodic sync for '{repo_name}' took {duration_secs}s (exceeds {}s threshold)",
            SYNC_WARN_DURATION.as_secs()
        );
        tracing::error!("{msg}");
        if let Err(e) =
            db::insert_system_event(pool, SystemEventType::RepoSyncSlow, None, &msg).await
        {
            tracing::error!(error = %e, "failed to log slow sync event");
        }
    }
}

/// Not user-configurable (unlike the settings-driven cutoffs in
/// `run_retention_cleanup`): this is background account-security bookkeeping
/// rather than a report/event retention policy a user would want to tune.
const LOGIN_ATTEMPT_RETENTION_DAYS: i64 = 90;

/// Reads a `*_retention_days` setting, falling back to `legacy` (the old
/// single `retention_days` setting) and then `default` if unset or
/// unparseable.
async fn retention_days_setting(
    pool: &PgPool,
    key: &str,
    legacy: Option<i64>,
    default: i64,
) -> Result<i64, crate::error::ApiError> {
    Ok(db::get_setting(pool, key)
        .await?
        .and_then(|v| {
            v.parse::<i64>()
                .inspect_err(|e| {
                    tracing::warn!(setting = key, value = %v, error = %e, "failed to parse retention setting");
                })
                .ok()
        })
        .or(legacy)
        .unwrap_or(default))
}

async fn run_retention_cleanup(pool: &PgPool) -> Result<(), crate::error::ApiError> {
    let legacy_retention = db::get_setting(pool, "retention_days")
        .await?
        .and_then(|v| {
            v.parse::<i64>().inspect_err(|e| {
                tracing::warn!(value = %v, error = %e, "failed to parse retention_days setting");
            }).ok()
        });

    let report_days = retention_days_setting(pool, "report_retention_days", None, 0).await?;
    let failed_days =
        retention_days_setting(pool, "failed_report_retention_days", legacy_retention, 365).await?;
    let event_days =
        retention_days_setting(pool, "system_event_retention_days", legacy_retention, 90).await?;
    let notification_delivery_days = retention_days_setting(
        pool,
        "notification_delivery_retention_days",
        legacy_retention,
        30,
    )
    .await?;

    let mut events_deleted: u64 = 0;
    let mut reports_deleted: u64 = 0;
    let mut archive_reports_deleted: u64 = 0;
    let mut login_attempts_deleted: u64 = 0;
    let mut notification_deliveries_deleted: u64 = 0;

    if let Some(cutoff) =
        Utc::now().checked_sub_signed(chrono::Duration::days(LOGIN_ATTEMPT_RETENTION_DAYS))
    {
        login_attempts_deleted = db::delete_login_attempts_before(pool, cutoff).await?;
    }

    if report_days > 0 {
        let Some(cutoff) = Utc::now().checked_sub_signed(chrono::Duration::days(report_days))
        else {
            return Ok(()); // clock went backwards, skip this cycle
        };
        archive_reports_deleted =
            db::delete_backup_reports_with_archive_before(pool, cutoff).await?;
    }

    if failed_days > 0 {
        let Some(cutoff) = Utc::now().checked_sub_signed(chrono::Duration::days(failed_days))
        else {
            return Ok(());
        };
        reports_deleted = db::delete_backup_reports_before(pool, cutoff).await?;
    }

    if event_days > 0 {
        let Some(cutoff) = Utc::now().checked_sub_signed(chrono::Duration::days(event_days)) else {
            return Ok(());
        };
        events_deleted = db::delete_system_events_before(pool, cutoff).await?;
    }

    if notification_delivery_days > 0 {
        let Some(cutoff) =
            Utc::now().checked_sub_signed(chrono::Duration::days(notification_delivery_days))
        else {
            return Ok(());
        };
        notification_deliveries_deleted =
            db::delete_notification_deliveries_before(pool, cutoff).await?;
    }

    if events_deleted > 0
        || reports_deleted > 0
        || archive_reports_deleted > 0
        || login_attempts_deleted > 0
        || notification_deliveries_deleted > 0
    {
        tracing::info!(
            events_deleted,
            reports_deleted,
            archive_reports_deleted,
            login_attempts_deleted,
            notification_deliveries_deleted,
            report_days,
            failed_days,
            event_days,
            notification_delivery_days,
            "retention cleanup completed"
        );
    }

    Ok(())
}

/// Dependencies needed to evaluate and trigger due schedules. Bundled into one
/// struct (rather than passed as individual arguments) to keep `tick`'s
/// signature manageable as the scheduler grows more cross-cutting concerns
/// (op tracking, broadcasts) beyond the original trigger/wait logic.
#[derive(Clone, Copy)]
struct TickDeps<'a> {
    pool: &'a PgPool,
    registry: &'a AgentRegistry,
    encryption_key: &'a [u8; 32],
    tunnel_manager: &'a TunnelManager,
    completion_bus: &'a CompletionBus,
    repo_lock: &'a RepoLock,
    repo_op_tracker: &'a RepoOpTracker,
    ui_broadcast: &'a UiBroadcast,
    background_task_tracker: &'a crate::background_tasks::BackgroundTaskTracker,
    power_sessions: &'a crate::power::PowerSessionTracker,
    notification_service: &'a crate::notifications::NotificationService,
    task_registry: &'a shared::task_registry::TaskRegistry,
}

async fn tick(deps: &TickDeps<'_>) -> Result<(), crate::error::ApiError> {
    let TickDeps {
        pool,
        registry,
        encryption_key,
        tunnel_manager,
        completion_bus,
        repo_lock,
        repo_op_tracker,
        ui_broadcast,
        background_task_tracker,
        power_sessions,
        notification_service,
        task_registry,
    } = *deps;
    let now = Utc::now();
    let due = db::list_due_schedules(pool, now).await?;

    if due.is_empty() {
        return Ok(());
    }

    let tz = db::get_schedule_timezone(pool).await?;

    // Group rows by schedule_id, preserving ORDER BY s.id, st.execution_order from the query.
    let mut schedule_groups: Vec<(i64, String, Vec<DueScheduleRow>)> = Vec::new();
    for row in due {
        match schedule_groups.last_mut() {
            Some((id, _, targets)) if *id == row.schedule_id => {
                targets.push(row);
            }
            _ => {
                let cron = row.cron_expression.clone();
                schedule_groups.push((row.schedule_id, cron, vec![row]));
            }
        }
    }

    for (schedule_id, cron, targets) in schedule_groups {
        let Some(first) = targets.first() else {
            continue;
        };
        let on_failure = first.on_failure.parse::<OnFailure>().unwrap_or_else(|_| {
            tracing::warn!(
                schedule_id,
                value = %first.on_failure,
                "invalid on_failure value in database; defaulting to Stop"
            );
            OnFailure::default()
        });

        let run_id = Uuid::new_v4().to_string();

        for target in &targets {
            if let Err(e) = db::insert_backup_pending(
                pool,
                target.agent_id,
                target.repo_id,
                Some(schedule_id),
                &run_id,
                now,
            )
            .await
            {
                tracing::warn!(
                    schedule_id,
                    hostname = %target.hostname,
                    error = %e,
                    "failed to insert pending record"
                );
            }
        }

        let (triggered_tx, triggered_rx) = tokio::sync::oneshot::channel();
        // Claimed synchronously (before spawning) rather than inside run_sequential_schedule,
        // so background_task_tracker.any_active() reflects this task immediately instead of
        // only once it gets its first poll - same pattern as run_scheduled_repo_sync above,
        // fixed for the same reason (#371). Without this, await_target_completion (which can
        // run for as long as every target's backup takes) is a fully untracked tokio::spawn:
        // whether it finishes before e2e teardown polls background_ops_in_flight and stops
        // containers is a scheduling race, producing non-deterministic coverage on this
        // function whenever a multi-target schedule actually fires during a run.
        let task_guard = background_task_tracker.begin();
        let ctx = SequentialExecution {
            pool: pool.clone(),
            registry: registry.clone(),
            encryption_key: *encryption_key,
            tunnel_manager: tunnel_manager.clone(),
            completion_bus: completion_bus.clone(),
            repo_lock: repo_lock.clone(),
            repo_op_tracker: repo_op_tracker.clone(),
            ui_broadcast: ui_broadcast.clone(),
            power_sessions: power_sessions.clone(),
            notification_service: notification_service.clone(),
            task_registry: task_registry.clone(),
            schedule_id,
            cron,
            targets,
            on_failure,
            triggered_at: now,
            tz,
            run_id,
            triggered_tx,
            task_guard,
        };
        tokio::spawn(async move {
            run_sequential_schedule(ctx).await;
        });
        // Yield so the spawned task can run and send the initial messages before tick returns.
        // This ensures callers can observe messages immediately after tick() completes.
        let _ = triggered_rx.await;
    }

    Ok(())
}

fn build_trigger_msg(
    schedule_type: ScheduleType,
    repo_id: RepoId,
    schedule_id: i64,
    run_id: &str,
) -> ServerToAgent {
    match schedule_type {
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
            run_id: Some(run_id.to_string()),
        },
    }
}

fn schedule_type_label(schedule_type: ScheduleType) -> &'static str {
    match schedule_type {
        ScheduleType::Check => "check",
        ScheduleType::Verify => "verify",
        ScheduleType::Backup => "backup",
    }
}

/// The op kind to record while a triggered schedule target is in flight, so the
/// repo detail page can show that the repository is actually locked right now
/// rather than only ever showing the last completed operation.
pub(crate) fn repo_op_kind_for(schedule_type: ScheduleType) -> shared::protocol::RepoOpKind {
    match schedule_type {
        ScheduleType::Backup => shared::protocol::RepoOpKind::AgentBackup,
        ScheduleType::Check => shared::protocol::RepoOpKind::AgentCheck,
        ScheduleType::Verify => shared::protocol::RepoOpKind::AgentVerify,
    }
}

struct SequentialExecution {
    pool: PgPool,
    registry: AgentRegistry,
    encryption_key: [u8; 32],
    tunnel_manager: TunnelManager,
    completion_bus: CompletionBus,
    repo_lock: RepoLock,
    repo_op_tracker: RepoOpTracker,
    ui_broadcast: UiBroadcast,
    power_sessions: crate::power::PowerSessionTracker,
    notification_service: crate::notifications::NotificationService,
    task_registry: shared::task_registry::TaskRegistry,
    schedule_id: i64,
    cron: String,
    targets: Vec<DueScheduleRow>,
    on_failure: OnFailure,
    triggered_at: DateTime<Utc>,
    tz: chrono_tz::Tz,
    run_id: String,
    /// Signalled once the first target's messages have been sent (or skipped).
    /// Allows `tick()` to wait briefly so callers using `try_recv()` see messages.
    triggered_tx: tokio::sync::oneshot::Sender<()>,
    /// Held for the lifetime of `run_sequential_schedule`; dropping it (task
    /// completion or panic) is what `background_task_tracker.any_active()`
    /// observes - see the claim site in `tick()` above.
    task_guard: crate::background_tasks::BackgroundTaskGuard,
}

struct SequentialTargetCtx<'a> {
    pool: &'a PgPool,
    registry: &'a AgentRegistry,
    encryption_key: &'a [u8; 32],
    tunnel_manager: &'a TunnelManager,
    completion_bus: &'a CompletionBus,
    repo_lock: &'a RepoLock,
    repo_op_tracker: &'a RepoOpTracker,
    ui_broadcast: &'a UiBroadcast,
    power_sessions: &'a crate::power::PowerSessionTracker,
    notification_service: &'a crate::notifications::NotificationService,
    task_registry: &'a shared::task_registry::TaskRegistry,
    schedule_id: i64,
    schedule_name: &'a str,
    cron: &'a str,
    on_failure: OnFailure,
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
    run_id: &'a str,
    missed_backup_threshold: i32,
}

impl<'a> SequentialTargetCtx<'a> {
    fn power_ctx(&self) -> power::PowerCtx<'a> {
        power::PowerCtx {
            pool: self.pool,
            registry: self.registry,
            ui_broadcast: self.ui_broadcast,
            power_sessions: self.power_sessions,
        }
    }
}

enum TargetControl {
    Continue,
    Stop,
}

async fn run_sequential_schedule(ctx: SequentialExecution) {
    let SequentialExecution {
        pool,
        registry,
        encryption_key,
        tunnel_manager,
        completion_bus,
        repo_lock,
        repo_op_tracker,
        ui_broadcast,
        power_sessions,
        notification_service,
        task_registry,
        schedule_id,
        cron,
        targets,
        on_failure,
        triggered_at: now,
        tz,
        run_id,
        triggered_tx,
        task_guard: _task_guard,
    } = ctx;
    let mut marked_triggered = false;
    let mut recorded_failure = false;
    let mut triggered_tx = Some(triggered_tx);

    let schedule_name = targets.first().map_or("", |t| t.schedule_name.as_str());
    let missed_backup_threshold = targets
        .first()
        .map_or(MAX_CONSECUTIVE_FAILURES, |t| t.missed_backup_threshold);
    let target_ctx = SequentialTargetCtx {
        pool: &pool,
        registry: &registry,
        encryption_key: &encryption_key,
        tunnel_manager: &tunnel_manager,
        completion_bus: &completion_bus,
        repo_lock: &repo_lock,
        repo_op_tracker: &repo_op_tracker,
        ui_broadcast: &ui_broadcast,
        power_sessions: &power_sessions,
        notification_service: &notification_service,
        task_registry: &task_registry,
        schedule_id,
        schedule_name,
        cron: &cron,
        on_failure,
        now,
        tz,
        run_id: &run_id,
        missed_backup_threshold,
    };

    for target in &targets {
        match run_sequential_target(
            &target_ctx,
            target,
            &mut marked_triggered,
            &mut recorded_failure,
            &mut triggered_tx,
        )
        .await
        {
            TargetControl::Continue => {}
            TargetControl::Stop => break,
        }
    }

    // Only a tick where every attempted target reached its agent counts as fully
    // healthy. A single schedule-wide counter can't track per-target failure state,
    // so if this tick recorded a failure for one target, a different target
    // succeeding in the same tick must not wipe out that count - otherwise a
    // permanently unreachable target in an `on_failure: Continue` schedule would
    // never reach MAX_CONSECUTIVE_FAILURES as long as some other target keeps
    // succeeding. mark_schedule_triggered_once (called from the per-target success
    // arm above) only ever advances next_run_at/last_run_at now; this is the sole
    // place consecutive_failures resets to 0.
    if !recorded_failure
        && let Err(e) = db::reset_schedule_consecutive_failures(&pool, schedule_id).await
    {
        tracing::error!(
            schedule_id,
            error = %e,
            "sequential: failed to reset schedule failure count"
        );
    }
}

/// Records a target's failure and signals `tick()` that the first target has been
/// attempted, in that order - the signal lets `tick()` stop waiting as soon as the
/// first target has been attempted, and if it fired first, the DB write would race the
/// caller reading the schedule's updated failure count right after `tick()` returns.
/// Shared by all three `run_sequential_target` failure paths (config-push
/// unreachable/error, and trigger-send failure), which differ only in whether the
/// failure was a connectivity problem (`agent_unreachable`).
async fn fail_target(
    ctx: &SequentialTargetCtx<'_>,
    agent_id: i64,
    repo_id: i64,
    hostname: &str,
    agent_unreachable: bool,
    recorded_failure: &mut bool,
    triggered_tx: &mut Option<tokio::sync::oneshot::Sender<()>>,
) -> TargetControl {
    record_schedule_failure_once(
        ctx,
        agent_id,
        repo_id,
        hostname,
        agent_unreachable,
        recorded_failure,
    )
    .await;
    signal_first_target_attempted(triggered_tx);
    match ctx.on_failure {
        OnFailure::Stop => TargetControl::Stop,
        OnFailure::Continue => TargetControl::Continue,
    }
}

async fn run_sequential_target(
    ctx: &SequentialTargetCtx<'_>,
    target: &DueScheduleRow,
    marked_triggered: &mut bool,
    recorded_failure: &mut bool,
    triggered_tx: &mut Option<tokio::sync::oneshot::Sender<()>>,
) -> TargetControl {
    let schedule_id = ctx.schedule_id;
    let Ok(schedule_type) = target.schedule_type.parse::<ScheduleType>() else {
        tracing::error!(
            schedule_id,
            schedule_type = %target.schedule_type,
            "sequential: invalid schedule type in database, skipping target"
        );
        return match ctx.on_failure {
            OnFailure::Stop => TargetControl::Stop,
            OnFailure::Continue => TargetControl::Continue,
        };
    };

    // Subscribe before sending so we don't miss the completion event.
    let rx = ctx.completion_bus.subscribe();

    // Make sure the source and repository hosts are reachable before anything
    // else that assumes they are, waking each independently and concurrently
    // (one being slow doesn't hold up the other). Every target that reaches
    // here is matched by exactly one `teardown_power_for_target` call on
    // every exit path below, so a host this run wakes is never left running
    // because of an early return. Done before acquiring the repo lock below,
    // since waking doesn't touch the repo itself - a slow wake (up to
    // wake_timeout_seconds) must not hold the lock and block an unrelated,
    // already-reachable target from starting.
    let (agent_row, repo_row) = ensure_target_power(ctx, target).await;
    let power = TargetPowerState {
        ctx: ctx.power_ctx(),
        agent: agent_row.as_ref(),
        repo: repo_row.as_ref(),
        agent_id: target.agent_id,
        repo_id: target.repo_id,
        run_id: ctx.run_id,
        hostname: &target.hostname,
    };

    // Acquire the per-repo lock to prevent concurrent backups across schedules.
    let _repo_guard = ctx.repo_lock.acquire(target.repo_id).await;

    ctx.tunnel_manager
        .ensure_agent_tunnel_connected(target.agent_id)
        .await;

    match push_pre_run_config(ctx, target).await {
        PreRunConfigOutcome::Sent => {}
        // recorded_failure is not guarded on marked_triggered: an earlier target in
        // this same tick succeeding must not suppress recording a later target's
        // failure - each is a distinct target that can fail independently, and a
        // schedule with `on_failure: Continue` processes every target regardless of
        // earlier outcomes.
        PreRunConfigOutcome::AgentUnreachable => {
            return fail_target_with_teardown(
                ctx,
                power,
                target,
                true,
                recorded_failure,
                triggered_tx,
            )
            .await;
        }
        // A persistent config-assembly error (e.g. a corrupted encrypted passphrase)
        // is just as capable of retrying forever on every tick as an unreachable
        // agent - route it through the same backoff/auto-disable path rather than
        // leaving next_run_at untouched. Passed as a local/data failure (not
        // agent_unreachable), so the disable it may cause won't be silently cleared
        // by an unrelated agent reconnect - see the doc comment on
        // db::record_schedule_failure.
        PreRunConfigOutcome::ConfigError => {
            return fail_target_with_teardown(
                ctx,
                power,
                target,
                false,
                recorded_failure,
                triggered_tx,
            )
            .await;
        }
    }

    let repo_id = RepoId(target.repo_id);
    let msg = build_trigger_msg(schedule_type, repo_id, schedule_id, ctx.run_id);
    let action = schedule_type_label(schedule_type);

    match ctx.registry.send_to(target.agent_id, msg).await {
        Ok(()) => {
            record_target_dispatched(ctx, target, schedule_type, action, marked_triggered).await;
            signal_first_target_attempted(triggered_tx);
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                repo_id = target.repo_id,
                action,
                schedule_id,
                error = %e,
                "sequential: agent not connected, skipping target"
            );
            return fail_target_with_teardown(
                ctx,
                power,
                target,
                true,
                recorded_failure,
                triggered_tx,
            )
            .await;
        }
    }

    let control = await_target_completion(ctx, target, rx).await;
    teardown_power_for_target(power).await;
    control
}

/// Records a successfully dispatched target: logs it, marks the repo as
/// actively in use for the lifetime of the lock guard (not just while the
/// agent happens to be reporting progress, so the repo detail page can show
/// it's locked right now rather than only ever showing the last completed
/// operation), and marks the schedule triggered. Split out of
/// [`run_sequential_target`] purely to keep that function's line count down.
async fn record_target_dispatched(
    ctx: &SequentialTargetCtx<'_>,
    target: &DueScheduleRow,
    schedule_type: ScheduleType,
    action: &str,
    marked_triggered: &mut bool,
) {
    let schedule_id = ctx.schedule_id;
    tracing::info!(
        hostname = %target.hostname,
        repo_id = target.repo_id,
        action,
        schedule_id,
        "sequential: triggered"
    );
    ctx.repo_op_tracker
        .set(
            target.repo_id,
            repo_op_kind_for(schedule_type),
            target.hostname.clone(),
            Some(target.agent_id),
        )
        .await;
    ctx.ui_broadcast.send(ServerToUi::RepoOpChanged {
        repo_id: target.repo_id,
        op: ctx.repo_op_tracker.get(target.repo_id).await,
    });
    if !*marked_triggered {
        mark_schedule_triggered_once(ctx, marked_triggered).await;
    }
}

/// The rows and reachability context [`run_sequential_target`] needs to pass
/// on to [`teardown_power_for_target`] once it's done with a target,
/// bundled to keep both functions' argument counts down.
struct TargetPowerState<'a> {
    ctx: power::PowerCtx<'a>,
    agent: Option<&'a db::AgentRow>,
    repo: Option<&'a db::RepoRow>,
    /// This target's pairing, independent of whether `agent`/`repo` above
    /// were actually fetched -- teardown needs the sibling ID even when its
    /// own row lookup failed.
    agent_id: i64,
    repo_id: i64,
    run_id: &'a str,
    hostname: &'a str,
}

/// [`fail_target`], but first tears down anything this target's own
/// [`ensure_target_power`] call turned on -- the shared tail of every
/// pre-dispatch failure path in [`run_sequential_target`].
async fn fail_target_with_teardown(
    ctx: &SequentialTargetCtx<'_>,
    power: TargetPowerState<'_>,
    target: &DueScheduleRow,
    agent_unreachable: bool,
    recorded_failure: &mut bool,
    triggered_tx: &mut Option<tokio::sync::oneshot::Sender<()>>,
) -> TargetControl {
    teardown_power_for_target(power).await;
    fail_target(
        ctx,
        target.agent_id,
        target.repo_id,
        &target.hostname,
        agent_unreachable,
        recorded_failure,
        triggered_tx,
    )
    .await
}

/// Fetches the source and repository host rows, makes sure each is
/// reachable (waking it first if it's configured to and isn't already), and
/// registers this target's participation in each host's
/// [`PowerSessionTracker`](crate::power::PowerSessionTracker) session.
/// Returns the rows (or `None` for one the DB lookup failed for) so the
/// caller can pass them on to [`teardown_power_for_target`] later. The two
/// hosts are checked concurrently -- one being slow to wake doesn't hold up
/// the other.
async fn ensure_target_power(
    ctx: &SequentialTargetCtx<'_>,
    target: &DueScheduleRow,
) -> (Option<db::AgentRow>, Option<db::RepoRow>) {
    let power_ctx = ctx.power_ctx();
    let agent_row = match db::get_agent_by_id(ctx.pool, target.agent_id).await {
        Ok(row) => Some(row),
        Err(e) => {
            tracing::warn!(
                agent_id = target.agent_id,
                error = %e,
                "sequential: failed to load agent for power management, skipping wake"
            );
            None
        }
    };
    let repo_row = match db::get_repo_by_id(ctx.pool, target.repo_id).await {
        Ok(row) => Some(row),
        Err(e) => {
            tracing::warn!(
                repo_id = target.repo_id,
                error = %e,
                "sequential: failed to load repo for power management, skipping wake"
            );
            None
        }
    };

    // Reserve both hosts *before* attempting to reach them, not once the
    // attempt resolves: two targets concurrently waking the same host must
    // both be counted for the whole waking window, or the one that resolves
    // first can tear the host down (see teardown_power_for_target) while the
    // other is still relying on it being up.
    if agent_row.is_some() {
        ctx.power_sessions
            .reserve(power::PowerHostKey::Agent(target.agent_id))
            .await;
    }
    if repo_row.is_some() {
        ctx.power_sessions
            .reserve(power::PowerHostKey::Repo(target.repo_id))
            .await;
    }

    let (agent_outcome, repo_outcome) = tokio::join!(
        async {
            match &agent_row {
                Some(agent) => {
                    power::ensure_agent_online(power_ctx, agent, target.repo_id, ctx.run_id).await
                }
                None => power::AgentPowerOutcome::default(),
            }
        },
        async {
            match &repo_row {
                Some(repo) => {
                    power::ensure_repo_online(
                        power_ctx,
                        repo,
                        target.agent_id,
                        ctx.run_id,
                        &target.hostname,
                    )
                    .await
                }
                None => power::RepoPowerOutcome::default(),
            }
        },
    );

    if agent_row.is_some() {
        ctx.power_sessions
            .record_outcome(
                power::PowerHostKey::Agent(target.agent_id),
                agent_outcome.woke,
                agent_outcome.started_agent,
            )
            .await;
    }
    if repo_row.is_some() {
        ctx.power_sessions
            .record_outcome(
                power::PowerHostKey::Repo(target.repo_id),
                repo_outcome.woke,
                false,
            )
            .await;
    }
    (agent_row, repo_row)
}

/// Shuts down / stops each of `agent_row`/`repo_row`'s hosts if this run's
/// [`PowerSessionTracker`](crate::power::PowerSessionTracker) says it's both
/// safe (no other concurrently-running target still relying on the host) and
/// warranted (this session actually woke or started it). A no-op for a host
/// whose row was never fetched -- see the `is_some()` guards around the
/// matching `begin()` calls in [`run_sequential_target`].
async fn teardown_power_for_target(power: TargetPowerState<'_>) {
    if let Some(agent) = power.agent {
        power::teardown_agent_power(power.ctx, agent, power.repo_id, power.run_id).await;
    }
    if let Some(repo) = power.repo {
        power::teardown_repo_power(
            power.ctx,
            repo,
            power.agent_id,
            power.run_id,
            power.hostname,
        )
        .await;
    }
}

fn signal_first_target_attempted(triggered_tx: &mut Option<tokio::sync::oneshot::Sender<()>>) {
    if let Some(tx) = triggered_tx.take() {
        let _ = tx.send(());
    }
}

/// Outcome of [`push_pre_run_config`], distinguishing an unreachable agent (which
/// counts toward the schedule's consecutive-failure backoff) from a config assembly
/// error (a local/db problem, not a sign the agent is offline).
enum PreRunConfigOutcome {
    Sent,
    AgentUnreachable,
    ConfigError,
}

/// Pushes a fresh config to the target agent before triggering the run.
async fn push_pre_run_config(
    ctx: &SequentialTargetCtx<'_>,
    target: &DueScheduleRow,
) -> PreRunConfigOutcome {
    let schedule_id = ctx.schedule_id;
    match config_assembler::assemble_config(ctx.pool, ctx.encryption_key, target.agent_id).await {
        Ok(config) => {
            let config_msg = ServerToAgent::ConfigUpdate(config);
            if let Err(e) = ctx.registry.send_to(target.agent_id, config_msg).await {
                tracing::warn!(
                    hostname = %target.hostname,
                    schedule_id,
                    error = %e,
                    "sequential: agent not connected for pre-run config push, skipping target"
                );
                return PreRunConfigOutcome::AgentUnreachable;
            }
            PreRunConfigOutcome::Sent
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                schedule_id,
                error = %e,
                "sequential: failed to assemble config, skipping target"
            );
            PreRunConfigOutcome::ConfigError
        }
    }
}

/// Computes the schedule's next cron occurrence from `ctx`, logging and returning
/// `None` on an invalid cron expression - shared by [`mark_schedule_triggered_once`]
/// and [`record_schedule_failure_once`], which both need this same "advance
/// `next_run_at`" starting point.
fn calculate_next_run_or_log(ctx: &SequentialTargetCtx<'_>) -> Option<DateTime<Utc>> {
    calculate_next_run(ctx.cron, ctx.now, ctx.tz)
        .inspect_err(|e| {
            tracing::error!(
                schedule_id = ctx.schedule_id,
                cron = %ctx.cron,
                error = %e,
                "sequential: invalid cron expression"
            );
        })
        .ok()
}

async fn mark_schedule_triggered_once(ctx: &SequentialTargetCtx<'_>, marked_triggered: &mut bool) {
    let Some(next) = calculate_next_run_or_log(ctx) else {
        return;
    };
    let schedule_id = ctx.schedule_id;
    if let Err(e) = db::mark_schedule_triggered(ctx.pool, schedule_id, ctx.now, next).await {
        tracing::error!(
            schedule_id,
            error = %e,
            "sequential: failed to mark schedule triggered"
        );
    } else {
        *marked_triggered = true;
    }
}

/// Records that this schedule failed to reach `agent_id`, backing off `next_run_at`
/// to the next scheduled occurrence (instead of the every-30s tick cadence) and
/// auto-disabling the schedule once its configured `missed_backup_threshold`
/// (`ctx.missed_backup_threshold`) is reached. Only the first target failure of a
/// tick counts, mirroring [`mark_schedule_triggered_once`],
/// since a schedule with multiple targets shouldn't be double-counted for one tick -
/// callers don't need to guard this themselves, it early-returns if `recorded_failure`
/// is already `true`.
/// `agent_id` is recorded as the schedule's `auto_disabled_by_agent_id` only when
/// `agent_unreachable` is `true`, so a reconnect only ever re-enables a schedule that
/// was disabled for connectivity reasons - see
/// `ws::handler::reenable_system_disabled_schedules_on_reconnect` and the doc comment
/// on [`db::record_schedule_failure`] for why a local/data failure (`agent_unreachable
/// = false`, e.g. a config-assembly error) must not be cleared by an unrelated
/// reconnect.
///
/// `recorded_failure` is set to `true` as soon as a failure is being processed for
/// this tick, regardless of whether the DB write or cron calculation below actually
/// succeeds: "a failure occurred but couldn't be recorded" must still suppress the
/// post-loop [`db::reset_schedule_consecutive_failures`] call the same way a
/// successfully-recorded failure does, or a transient DB error would wipe a real,
/// already-persisted failure count.
///
/// Unlike [`mark_schedule_triggered_once`] (which just skips its DB write on an
/// unevaluatable cron, since there's nothing useful to persist on a bare trigger), this
/// function always records the failure - falling back to
/// [`CRON_EVAL_FAILURE_BACKOFF_HOURS`] for `next_run_at` when the cron itself can't be
/// evaluated. Skipping the DB write here entirely, the way the pre-existing
/// success-path helper does, would mean `consecutive_failures` never increments and
/// `next_run_at` never advances, so the schedule stays "due" and gets retried on every
/// tick forever - reproducing, for an unparseable/unsatisfiable cron, the exact
/// unbounded-retry problem this whole mechanism exists to fix for an unreachable agent.
async fn record_schedule_failure_once(
    ctx: &SequentialTargetCtx<'_>,
    agent_id: i64,
    repo_id: i64,
    hostname: &str,
    agent_unreachable: bool,
    recorded_failure: &mut bool,
) {
    if *recorded_failure {
        return;
    }
    *recorded_failure = true;
    let next = calculate_next_run_or_log(ctx).unwrap_or_else(|| {
        ctx.now
            .checked_add_signed(chrono::Duration::hours(CRON_EVAL_FAILURE_BACKOFF_HOURS))
            .unwrap_or(ctx.now)
    });
    let schedule_id = ctx.schedule_id;
    match db::record_schedule_failure(
        ctx.pool,
        schedule_id,
        agent_id,
        next,
        ctx.missed_backup_threshold,
        agent_unreachable,
    )
    .await
    {
        Ok(outcome) => {
            if outcome.auto_disabled {
                tracing::error!(
                    schedule_id,
                    consecutive_failures = outcome.consecutive_failures,
                    agent_unreachable,
                    persisted_auto_disabled_agent_unreachable =
                        outcome.auto_disabled_agent_unreachable,
                    "sequential: schedule auto-disabled after repeated failures"
                );
                // Derived from what record_schedule_failure actually persisted, not
                // from this call's own `agent_unreachable` argument: the two can
                // diverge when an earlier local/data failure in the same streak
                // already marked failure_streak_pure_connectivity false, in which case
                // auto_disabled_agent_unreachable stays false even though this
                // specific call was a connectivity failure - reporting "unreachable"
                // here would contradict both the UI's "Auto-disabled - error" label
                // and the fact that a reconnect will never auto-heal it.
                let reason = if outcome.auto_disabled_agent_unreachable {
                    format!("agent '{hostname}' stayed unreachable")
                } else {
                    "a local or configuration problem".to_owned()
                };
                let msg = format!(
                    "Schedule '{}' auto-disabled after {} consecutive failures: {reason}",
                    ctx.schedule_name, outcome.consecutive_failures
                );
                if let Err(e) = db::insert_system_event(
                    ctx.pool,
                    SystemEventType::ScheduleAutoDisabled,
                    Some(hostname),
                    &msg,
                )
                .await
                {
                    tracing::error!(
                        schedule_id,
                        error = %e,
                        "sequential: failed to record schedule-auto-disabled system event"
                    );
                }
                dispatch_schedule_auto_disabled_notification(
                    ctx,
                    agent_id,
                    repo_id,
                    hostname,
                    schedule_id,
                    &reason,
                )
                .await;
                ctx.ui_broadcast.send(ServerToUi::DataChanged);
            } else {
                tracing::warn!(
                    schedule_id,
                    consecutive_failures = outcome.consecutive_failures,
                    max = ctx.missed_backup_threshold,
                    agent_unreachable,
                    "sequential: target failed, backing off to next scheduled run"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                schedule_id,
                error = %e,
                "sequential: failed to record schedule failure"
            );
        }
    }
}

/// Dispatches a [`notifications::EventType::ScheduleAutoDisabled`] event so a
/// configured channel (email/webhook/push) can alert on this the same way it
/// already can for a failed/warning backup - `record_schedule_failure_once`
/// only ever wrote a system event before, which never leaves the Activity page.
/// Looks up the repo name for the event payload rather than threading it all
/// the way through `SequentialTargetCtx`/`DueScheduleRow`, since this is the
/// only place along the sequential-execution path that needs it.
async fn dispatch_schedule_auto_disabled_notification(
    ctx: &SequentialTargetCtx<'_>,
    agent_id: i64,
    repo_id: i64,
    hostname: &str,
    schedule_id: i64,
    reason: &str,
) {
    let repo_name = db::get_repo_name(ctx.pool, repo_id)
        .await
        .unwrap_or_default();
    let event = crate::notifications::NotificationEvent {
        event_type: crate::notifications::EventType::ScheduleAutoDisabled,
        hostname: hostname.to_owned(),
        repo_name,
        status: "auto_disabled".to_owned(),
        error_message: Some(reason.to_owned()),
        timestamp: ctx.now,
        repo_id: Some(repo_id),
        agent_id: Some(agent_id),
        schedule_id: Some(schedule_id),
        schedule_name: Some(ctx.schedule_name.to_owned()),
        archive_name: None,
    };
    if let Err(e) =
        crate::notifications::dispatch(ctx.notification_service, event, ctx.task_registry).await
    {
        tracing::error!(
            schedule_id,
            error = %e,
            "sequential: failed to dispatch schedule-auto-disabled notification"
        );
    }
}

async fn await_target_completion(
    ctx: &SequentialTargetCtx<'_>,
    target: &DueScheduleRow,
    rx: tokio::sync::broadcast::Receiver<completion_bus::OperationOutcome>,
) -> TargetControl {
    let schedule_id = ctx.schedule_id;
    let repo_id_val = target.repo_id;

    let outcome =
        completion_bus::wait_for_completion(ctx.registry, rx, target.agent_id, repo_id_val).await;

    ctx.repo_op_tracker.clear(repo_id_val).await;
    ctx.ui_broadcast.send(ServerToUi::RepoOpChanged {
        repo_id: repo_id_val,
        op: None,
    });

    let success = match outcome {
        completion_bus::CompletionOutcome::Success => true,
        completion_bus::CompletionOutcome::Failed => false,
        completion_bus::CompletionOutcome::AgentDisconnected => {
            tracing::error!(
                schedule_id,
                hostname = %target.hostname,
                repo_id = target.repo_id,
                "sequential: agent disconnected before reporting completion"
            );
            false
        }
    };

    if !success {
        match ctx.on_failure {
            OnFailure::Stop => {
                tracing::warn!(
                    schedule_id,
                    hostname = %target.hostname,
                    "sequential: stopping remaining targets due to failure"
                );
                return TargetControl::Stop;
            }
            OnFailure::Continue => {}
        }
    }

    TargetControl::Continue
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, sync::OnceLock, time::Duration};

    use chrono::TimeZone;
    use tempfile::TempDir;
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use crate::{
        db::{self, InsertRepoParams, ScheduleParams},
        repo_op_tracker::RepoOpTracker,
        tunnel::TunnelManager,
        ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
    };

    /// `run()`'s four inner loops each race their interval tick against
    /// `shutdown_token.cancelled()` with `biased;` ordering the cancellation arm
    /// first, so cancelling immediately after spawning always wins even on a
    /// freshly-created `interval` (whose first `tick()` resolves immediately,
    /// same as the cancellation future, rather than after a full period) - without
    /// `biased`, `tokio::select!` would break that tie randomly instead of
    /// deterministically preferring shutdown. `run()` returns without ever
    /// touching the DB, so a lazily-connected, never-reachable pool is fine here.
    #[tokio::test]
    async fn run_returns_promptly_when_shutdown_token_is_cancelled() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let ui_broadcast = UiBroadcast::new();
        let state = AppState {
            pool: pool.clone(),
            encryption_key: shared::crypto::derive_key(b"scheduler-shutdown-test-key").unwrap(),
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
            repo_lock: RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: crate::new_pending_map(),
            pending_restores: crate::new_pending_map(),
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
            power_sessions: crate::power::PowerSessionTracker::default(),
        };
        let shutdown_token = state.shutdown_token.clone();

        let handle = tokio::spawn(run(state));
        shutdown_token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            result.is_ok(),
            "scheduler::run did not exit within 5s after shutdown_token cancellation"
        );
    }

    static BORG_BINARY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct BorgBinaryGuard {
        previous: Option<String>,
    }

    impl Drop for BorgBinaryGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.clone() {
                // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
                unsafe { std::env::set_var("BORG_BINARY", previous) };
            } else {
                // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
                unsafe { std::env::remove_var("BORG_BINARY") };
            }
        }
    }

    async fn borg_binary_lock() -> tokio::sync::MutexGuard<'static, ()> {
        BORG_BINARY_LOCK.get_or_init(|| Mutex::new(())).lock().await
    }

    async fn install_fake_borg(
        list_json: &str,
        info_all_json: &str,
        info_repo_json: &str,
    ) -> (TempDir, BorgBinaryGuard) {
        let tempdir = tempfile::tempdir().unwrap();
        let script = format!(
            r#"#!/bin/sh
set -eu
case "$1" in
  list)
    case " $* " in
      *" --json "*) cat <<'EOF'
{list_json}
EOF
        ;;
      *) ;;
    esac
    ;;
  info)
    case " $* " in
      *" --glob-archives "*) cat <<'EOF'
{info_all_json}
EOF
        ;;
      *"::"*) cat <<'EOF'
{info_all_json}
EOF
        ;;
      *) cat <<'EOF'
{info_repo_json}
EOF
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
"#
        );

        let borg_path = tempdir.path().join("borg");
        tokio::fs::write(&borg_path, script).await.unwrap();
        let mut permissions = tokio::fs::metadata(&borg_path).await.unwrap().permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&borg_path, permissions)
            .await
            .unwrap();

        let previous = std::env::var("BORG_BINARY").ok();
        // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
        unsafe { std::env::set_var("BORG_BINARY", &borg_path) };

        (tempdir, BorgBinaryGuard { previous })
    }

    #[test]
    fn sync_due_when_next_run_in_past() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 13, 0, 0).unwrap();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next <= now);
    }

    #[test]
    fn sync_not_due_when_next_run_in_future() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next > now);
    }

    #[test]
    fn sync_due_when_never_synced() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = DateTime::UNIX_EPOCH;
        let now = Utc::now();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next <= now);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn run_repo_sync_full_reimports_and_prunes_stale_archives(pool: sqlx::PgPool) {
        let _borg_lock = borg_binary_lock().await;
        let list_json: &str = concat!(
            r#"{"archives":[{"name":"fresh-archive","hostname":"scheduler-test-host","#,
            r#""start":"2026-06-05T10:00:00Z","end":"2026-06-05T10:05:00Z","#,
            r#""duration":300.0,"stats":{"original_size":1000,"compressed_size":500,"#,
            r#""deduplicated_size":250,"nfiles":2}}]}"#,
        );
        let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 1000,
      "total_csize": 600,
      "unique_csize": 500,
      "total_chunks": 10,
      "unique_chunks": 8
    }
  }
}"#;

        let (_borg_dir, _borg_guard) =
            install_fake_borg(list_json, list_json, info_repo_json).await;
        let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-scheduler").unwrap();
        let passphrase_encrypted =
            shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
        let agent = db::insert_agent(
            &pool,
            "scheduler-test-host",
            Some("Scheduler Test Host"),
            "hash",
            None,
            None,
        )
        .await
        .unwrap();
        let repo = db::insert_repo(
            &pool,
            &InsertRepoParams {
                name: "scheduler-test-repo",
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "storage.local",
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

        let stale_started_at = Utc::now()
            .checked_sub_signed(chrono::Duration::days(1))
            .unwrap();
        let stale_finished_at = stale_started_at
            .checked_add_signed(chrono::Duration::minutes(5))
            .unwrap();
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
             files_processed, duration_secs, error_message, warnings, borg_version, matched, \
             archive_name, borg_command) VALUES ($1, $2, NULL, $3, $4, 'success', 10, 5, 5, 5, 1, \
             300, NULL, '{}'::text[], NULL, true, $5, NULL)",
            agent.id,
            repo.id,
            stale_started_at,
            stale_finished_at,
            "stale-archive",
        )
        .execute(&pool)
        .await
        .unwrap();

        let background_task_tracker = crate::background_tasks::BackgroundTaskTracker::default();
        let task_registry = shared::task_registry::TaskRegistry::default();
        sync_existing_archives(
            &pool,
            &encryption_key,
            repo.id,
            &UiBroadcast::new(),
            &background_task_tracker,
            &task_registry,
        )
        .await
        .expect("sync_existing_archives failed");

        // sync_existing_archives fires archive-stat enrichment in the background
        // (enrich_archive_stats_background) rather than awaiting it, so the assertions
        // below would otherwise race that task's completion - whether it finishes before
        // this test function returns and tears down its tokio runtime is a scheduling
        // coincidence, which is exactly what produces non-deterministic coverage on the
        // functions it calls (parse_archive_stats, enrich_single_archive_stats, ...).
        background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;

        let stale_count = sqlx::query_scalar!(
            "SELECT COUNT(*)::BIGINT FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
            repo.id,
            "stale-archive",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(stale_count, 0);

        let fresh_count = sqlx::query_scalar!(
            "SELECT COUNT(*)::BIGINT FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
            repo.id,
            "fresh-archive",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(fresh_count, 1);
    }

    /// Exercises `run_repo_sync` end-to-end (not just the `sync_existing_archives`
    /// call it eventually makes) so the due-sync-check loop, the `RepoOpTracker`
    /// guard construction, and `run_scheduled_repo_sync`'s bookkeeping are all
    /// covered by something other than a direct call to the inner sync function.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn run_repo_sync_triggers_due_scheduled_sync_and_clears_repo_op(pool: sqlx::PgPool) {
        let _borg_lock = borg_binary_lock().await;
        let empty_list_json = r#"{"archives":[]}"#;
        let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 0,
      "total_csize": 0,
      "unique_csize": 0,
      "total_chunks": 0,
      "unique_chunks": 0
    }
  }
}"#;
        let (_borg_dir, _borg_guard) =
            install_fake_borg(empty_list_json, empty_list_json, info_repo_json).await;
        let encryption_key =
            shared::crypto::derive_key(b"test-secret-key-for-scheduled-sync").unwrap();
        let passphrase_encrypted =
            shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
        let repo = db::insert_repo(
            &pool,
            &InsertRepoParams {
                name: "scheduled-sync-test-repo",
                repo_path: "/backup/scheduled-sync-test",
                ssh_user: "borg",
                ssh_host: "storage.local",
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

        // Every-minute cron with no prior sync (repo.last_synced_at is NULL) is
        // immediately due, so run_repo_sync must pick this repo up on this tick.
        sqlx::query!(
            "UPDATE repos SET sync_schedule = $2 WHERE id = $1",
            repo.id,
            "* * * * *",
        )
        .execute(&pool)
        .await
        .unwrap();

        let ui_broadcast = UiBroadcast::new();
        let repo_op_tracker = RepoOpTracker::default();
        let repo_lock = RepoLock::default();
        let background_task_tracker = crate::background_tasks::BackgroundTaskTracker::default();
        let task_registry = shared::task_registry::TaskRegistry::default();

        run_repo_sync(
            &pool,
            &encryption_key,
            &ui_broadcast,
            &repo_op_tracker,
            &repo_lock,
            &background_task_tracker,
            &task_registry,
        )
        .await;

        // run_repo_sync only starts run_scheduled_repo_sync in the background. Its
        // background_task_tracker guard is now claimed synchronously before the spawn and held
        // for the whole task (including handle_scheduled_sync_success, which runs after
        // repo_op_tracker has already cleared), so assert_idle alone is sufficient to know
        // everything - not just repo_op_tracker's earlier-clearing entry - has finished.
        background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
        assert!(
            !repo_op_tracker.any_active().await,
            "repo_op_tracker should have cleared the scheduled sync's entry"
        );

        let last_synced_at = sqlx::query_scalar!(
            "SELECT last_synced_at FROM repo_stats WHERE repo_id = $1",
            repo.id,
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
        .flatten();
        assert!(
            last_synced_at.is_some(),
            "run_repo_sync should have run the due sync and recorded last_synced_at"
        );
    }

    // tick() integration tests
    // Run with:
    //   DATABASE_URL=postgres://borg:borg_secret@localhost:5432/borg \
    //     cargo test -p server --test-threads=1

    const TICK_TEST_HOSTNAME: &str = "tick-test-agent";
    const TICK_TEST_KEY_MATERIAL: &[u8] = b"tick-test-scheduler-secret-key";

    fn tick_test_key() -> [u8; 32] {
        shared::crypto::derive_key(TICK_TEST_KEY_MATERIAL).unwrap()
    }

    fn dummy_tunnel(pool: sqlx::PgPool) -> TunnelManager {
        TunnelManager::new(pool, UiBroadcast::new(), "127.0.0.1:0".parse().unwrap())
    }

    async fn setup_due_schedule(pool: &sqlx::PgPool, key: &[u8; 32]) -> (i64, i64, i64) {
        let passphrase_enc = shared::crypto::encrypt_passphrase("test-pass", key).unwrap();
        let agent = db::insert_agent(pool, TICK_TEST_HOSTNAME, None, "hash", None, None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "tick-repo",
                repo_path: "/backup/tick",
                ssh_user: "borg",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_enc,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAATICKTEST")
            .await
            .unwrap();
        let schedule = db::insert_schedule(
            pool,
            repo.id,
            &ScheduleParams {
                name: "tick-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
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
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .unwrap();
        let past = Utc::now()
            .checked_sub_signed(chrono::Duration::hours(1))
            .unwrap();
        db::set_next_run_at(pool, schedule.id, past).await.unwrap();
        (repo.id, schedule.id, agent.id)
    }

    async fn register_fake_agent(
        registry: &AgentRegistry,
        agent_id: i64,
    ) -> mpsc::Receiver<shared::protocol::ServerToAgent> {
        let (tx, rx) = mpsc::channel(32);
        registry.register(agent_id, tx, false, None).await;
        rx
    }

    /// `tick()` must send `ConfigUpdate` *before* the run trigger so the agent
    /// always executes with the current config.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_sends_config_update_before_run_trigger(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (repo_id, _, agent_id) = setup_due_schedule(&pool, &key).await;

        let registry = AgentRegistry::new();
        let mut rx = register_fake_agent(&registry, agent_id).await;
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        let first = rx
            .try_recv()
            .expect("expected ConfigUpdate as first message");
        assert!(
            matches!(first, shared::protocol::ServerToAgent::ConfigUpdate(_)),
            "first message must be ConfigUpdate, got: {first:?}"
        );

        let second = rx
            .try_recv()
            .expect("expected RunBackupNow as second message");
        match second {
            shared::protocol::ServerToAgent::RunBackupNow { repo_id: rid, .. } => {
                assert_eq!(rid.0, repo_id, "RunBackupNow repo_id mismatch");
            }
            other => panic!("expected RunBackupNow, got: {other:?}"),
        }
    }

    /// `ConfigUpdate` sent before each trigger must reflect the *current* global
    /// excludes, not those that were in place when the schedule was created.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_config_carries_updated_global_excludes(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, _, agent_id) = setup_due_schedule(&pool, &key).await;

        // Set global excludes raw text; tick must deliver the current value.
        db::set_global_excludes_raw(&pool, "*.tmp").await.unwrap();

        let registry = AgentRegistry::new();
        let mut rx = register_fake_agent(&registry, agent_id).await;
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        let msg = rx.try_recv().expect("expected ConfigUpdate");
        match msg {
            shared::protocol::ServerToAgent::ConfigUpdate(config) => {
                let all_excludes: Vec<_> = config
                    .repos
                    .iter()
                    .flat_map(|r| r.schedules.iter())
                    .flat_map(|s| s.exclude_patterns.iter().cloned())
                    .collect();
                assert!(
                    all_excludes.iter().any(|p| p == "*.tmp"),
                    "exclude '*.tmp' missing; got: {all_excludes:?}"
                );
                assert!(
                    !all_excludes.iter().any(|p| p == "*.log"),
                    "stale exclude '*.log' present; got: {all_excludes:?}"
                );
            }
            other => panic!("expected ConfigUpdate, got: {other:?}"),
        }
    }

    /// Fetches the failure-tracking columns added for the agent-offline backoff, for
    /// assertions in the tests below.
    async fn schedule_failure_state(pool: &sqlx::PgPool, schedule_id: i64) -> (i32, bool, bool) {
        let row = sqlx::query!(
            "SELECT consecutive_failures, enabled, auto_disabled_agent_unreachable FROM schedules \
             WHERE id = $1",
            schedule_id,
        )
        .fetch_one(pool)
        .await
        .unwrap();
        (
            row.consecutive_failures,
            row.enabled,
            row.auto_disabled_agent_unreachable,
        )
    }

    /// When the target agent is not connected, `tick()` must not error, must back off
    /// `next_run_at` to the next scheduled occurrence (instead of retrying on every 30s
    /// tick forever), and must record the failure so repeated misses can eventually
    /// auto-disable the schedule.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_backs_off_and_records_failure_when_agent_disconnected(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        let due = db::list_due_schedules(&pool, Utc::now()).await.unwrap();
        assert!(
            !due.iter().any(|s| s.schedule_id == schedule_id),
            "schedule must back off to the next scheduled run, not stay due for the next tick"
        );

        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, 1, "one failure must be recorded");
        assert!(enabled, "a single failure must not disable the schedule");
        assert!(!auto_disabled);
    }

    /// A cron expression that can't be evaluated (here: syntactically invalid, but the
    /// same gap applies to one with no next occurrence or an ambiguous local time) must
    /// still count as a recorded failure with `next_run_at` pushed forward - not
    /// silently skipped. Otherwise `consecutive_failures` never increments and
    /// `next_run_at` never advances, so the schedule stays "due" and gets retried on
    /// every 30s tick forever, reproducing the exact unbounded-retry problem this whole
    /// mechanism exists to prevent, just triggered by a bad cron instead of an
    /// unreachable agent.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_backs_off_and_records_failure_on_unevaluatable_cron(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        sqlx::query!(
            "UPDATE schedules SET cron_expression = $1 WHERE id = $2",
            "not-a-valid-cron",
            schedule_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        let due = db::list_due_schedules(&pool, Utc::now()).await.unwrap();
        assert!(
            !due.iter().any(|s| s.schedule_id == schedule_id),
            "an unevaluatable cron must still back off next_run_at, not stay due for the next \
             tick forever"
        );

        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(
            consecutive_failures, 1,
            "the failure must still be recorded even though next_run_at couldn't be computed from \
             the cron itself"
        );
        assert!(enabled, "a single failure must not disable the schedule");
        assert!(!auto_disabled);
    }

    /// After `MAX_CONSECUTIVE_FAILURES` consecutive misses, the scheduler must give up
    /// and disable the schedule rather than retrying forever.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_auto_disables_schedule_after_max_consecutive_failures(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let ui_broadcast = UiBroadcast::new();
        let mut ui_rx = ui_broadcast.subscribe();

        for attempt in 1..=MAX_CONSECUTIVE_FAILURES {
            // Simulate the missed run's cron occurrence having arrived again.
            let past = Utc::now()
                .checked_sub_signed(chrono::Duration::hours(1))
                .unwrap();
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &ui_broadcast,
                background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &crate::notifications::NotificationService::new(pool.clone()),
                task_registry: &shared::task_registry::TaskRegistry::default(),
            })
            .await
            .unwrap();

            let (consecutive_failures, enabled, auto_disabled) =
                schedule_failure_state(&pool, schedule_id).await;
            assert_eq!(consecutive_failures, attempt);
            if attempt < MAX_CONSECUTIVE_FAILURES {
                assert!(enabled, "must stay enabled before the threshold is reached");
                assert!(!auto_disabled);
            } else {
                assert!(!enabled, "must auto-disable once the threshold is reached");
                assert!(auto_disabled);
            }
        }

        let due = db::list_due_schedules(&pool, Utc::now()).await.unwrap();
        assert!(
            !due.iter().any(|s| s.schedule_id == schedule_id),
            "a disabled schedule must never be selected as due again"
        );

        // The auto-disable must not be invisible outside server logs.
        let events = db::get_system_events(&pool, 10).await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e.event_type, SystemEventType::ScheduleAutoDisabled)),
            "auto-disabling a schedule must record a ScheduleAutoDisabled system event"
        );

        // Nor invisible to a browser already looking at the schedules list: without a
        // broadcast, SchedulesView.vue (which only refetches on mount or on a
        // DataChanged message) would never show the new status until a manual reload.
        let mut saw_data_changed = false;
        while let Ok(msg) = ui_rx.try_recv() {
            if matches!(msg, ServerToUi::DataChanged) {
                saw_data_changed = true;
            }
        }
        assert!(
            saw_data_changed,
            "auto-disabling a schedule must broadcast DataChanged so it shows up live in the UI"
        );
    }

    /// Auto-disabling a schedule must not be visible only via a system event
    /// (which never leaves the Activity page) - a configured notification
    /// channel with a `schedule_auto_disabled` rule must actually be dispatched
    /// to, the same way a failed/warning backup already notifies.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_auto_disable_dispatches_a_schedule_auto_disabled_notification(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;

        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'webhook', $2, true) RETURNING id",
            "test-webhook",
            serde_json::json!({ "url": "http://127.0.0.1:1/unreachable" }),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO notification_rules (channel_id, event_type, enabled) VALUES ($1, \
             'schedule_auto_disabled', true)",
            channel_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let notification_service = crate::notifications::NotificationService::new(pool.clone());
        let task_registry = shared::task_registry::TaskRegistry::default();

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let past = Utc::now()
                .checked_sub_signed(chrono::Duration::hours(1))
                .unwrap();
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &UiBroadcast::new(),
                background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &notification_service,
                task_registry: &task_registry,
            })
            .await
            .unwrap();
        }

        let outstanding = task_registry
            .shutdown(std::time::Duration::from_secs(5))
            .await;
        assert_eq!(
            outstanding, 0,
            "task_registry.shutdown must join the notification delivery task"
        );

        let delivery_event_types: Vec<String> = sqlx::query_scalar!(
            "SELECT event_type FROM notification_deliveries WHERE channel_id = $1",
            channel_id,
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            delivery_event_types,
            vec!["schedule_auto_disabled".to_owned()],
            "auto-disabling the schedule must dispatch a schedule_auto_disabled notification"
        );
    }

    /// A schedule's own `missed_backup_threshold` - not the `MAX_CONSECUTIVE_FAILURES`
    /// default - must govern when the scheduler gives up and disables it. Below that
    /// custom threshold, misses must keep accumulating with the schedule still enabled
    /// (the "warning" zone the UI surfaces); at it, the schedule must be disabled, even
    /// though that count is well past the old hardcoded default of 3.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_honors_a_custom_missed_backup_threshold(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let custom_threshold = MAX_CONSECUTIVE_FAILURES + 2;
        sqlx::query!(
            "UPDATE schedules SET missed_backup_threshold = $2 WHERE id = $1",
            schedule_id,
            custom_threshold,
        )
        .execute(&pool)
        .await
        .unwrap();

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        for attempt in 1..=custom_threshold {
            let past = Utc::now()
                .checked_sub_signed(chrono::Duration::hours(1))
                .unwrap();
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &UiBroadcast::new(),
                background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &crate::notifications::NotificationService::new(pool.clone()),
                task_registry: &shared::task_registry::TaskRegistry::default(),
            })
            .await
            .unwrap();

            let (consecutive_failures, enabled, _) =
                schedule_failure_state(&pool, schedule_id).await;
            assert_eq!(consecutive_failures, attempt);
            if attempt <= MAX_CONSECUTIVE_FAILURES {
                assert!(
                    enabled,
                    "must stay enabled past the old hardcoded default of \
                     {MAX_CONSECUTIVE_FAILURES} misses, since this schedule's own threshold is \
                     higher"
                );
            }
            if attempt < custom_threshold {
                assert!(
                    enabled,
                    "must stay enabled before this schedule's own threshold is reached"
                );
            } else {
                assert!(
                    !enabled,
                    "must auto-disable once this schedule's own threshold is reached"
                );
            }
        }
    }

    /// `db::reset_schedule_consecutive_failures` (called from the post-loop `if
    /// !recorded_failure` arm in `run_sequential_schedule`) is the sole place
    /// `consecutive_failures` resets to 0, and is distinct from the reconnect/retarget
    /// paths that clear the *auto-disable* bookkeeping - it fires for the ordinary
    /// "agent recovers and a tick fully succeeds" case. A schedule that failed once
    /// and later recovers must start counting from zero again, not carry a stale
    /// failure count into whatever transient hiccup happens next.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_success_resets_consecutive_failures(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (repo_id, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;

        let next = Utc::now()
            .checked_add_signed(chrono::Duration::hours(1))
            .unwrap();
        db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            true,
        )
        .await
        .unwrap();
        let (consecutive_failures, enabled, _) = schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, 1);
        assert!(enabled, "a single failure must not disable the schedule");

        // Make the schedule due again for the recovering tick.
        let past = Utc::now()
            .checked_sub_signed(chrono::Duration::hours(1))
            .unwrap();
        db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

        let registry = AgentRegistry::new();
        let mut rx = register_fake_agent(&registry, agent_id).await;
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let background_task_tracker = crate::background_tasks::BackgroundTaskTracker::default();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &background_task_tracker,
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        let trigger = loop {
            match rx.recv().await.expect("expected messages for the target") {
                shared::protocol::ServerToAgent::ConfigUpdate(_) => {}
                other => break other,
            }
        };
        assert!(
            matches!(
                trigger,
                shared::protocol::ServerToAgent::RunBackupNow { .. }
            ),
            "expected RunBackupNow, got: {trigger:?}"
        );
        bus.publish(completion_bus::OperationOutcome {
            agent_id,
            repo_id,
            success: true,
        });
        assert!(
            background_task_tracker
                .wait_until_idle(std::time::Duration::from_secs(5))
                .await,
            "the tick's background task must finish"
        );

        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(
            consecutive_failures, 0,
            "a fully successful tick must reset the failure count back to zero"
        );
        assert!(enabled);
        assert!(!auto_disabled);
    }

    /// A config-assembly failure (e.g. a corrupted encrypted passphrase) must still
    /// count toward the failure threshold and auto-disable the schedule, but - unlike
    /// an unreachable-agent failure - must never be recorded as
    /// `auto_disabled_agent_unreachable`: the agent reconnecting over the websocket
    /// says nothing about whether the underlying data problem was fixed, so an
    /// unrelated reconnect must not silently re-enable a schedule that's still broken.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_auto_disables_but_does_not_mark_unreachable_on_config_error(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;

        // A different encryption key than the one the passphrase was encrypted with
        // makes config assembly fail deterministically every tick - simulating a
        // corrupted encrypted passphrase without touching the DB directly.
        let wrong_key = [7u8; 32];
        let registry = AgentRegistry::new();
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();

        for attempt in 1..=MAX_CONSECUTIVE_FAILURES {
            let past = Utc::now()
                .checked_sub_signed(chrono::Duration::hours(1))
                .unwrap();
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &wrong_key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &UiBroadcast::new(),
                background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &crate::notifications::NotificationService::new(pool.clone()),
                task_registry: &shared::task_registry::TaskRegistry::default(),
            })
            .await
            .unwrap();

            let (consecutive_failures, enabled, auto_disabled) =
                schedule_failure_state(&pool, schedule_id).await;
            assert_eq!(consecutive_failures, attempt);
            assert!(
                !auto_disabled,
                "a config-assembly failure must never set auto_disabled_agent_unreachable"
            );
            if attempt < MAX_CONSECUTIVE_FAILURES {
                assert!(enabled, "must stay enabled before the threshold is reached");
            } else {
                assert!(
                    !enabled,
                    "must still auto-disable once the threshold is reached"
                );
            }
        }

        // The agent "reconnecting" must not resurrect a schedule that's disabled for a
        // data/config reason, since nothing about the broken config actually changed.
        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(reenabled, Vec::<i64>::new());
        let (_, still_enabled, _) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !still_enabled,
            "must remain disabled until a human fixes it"
        );
    }

    /// A streak that ends on a config-error failure must never be marked
    /// `auto_disabled_agent_unreachable`, even if earlier failures in that same
    /// streak were connectivity failures - an unrelated agent reconnect must not
    /// re-enable it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn record_schedule_failure_only_marks_unreachable_on_the_disabling_call(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        // Two connectivity failures, then a config-error failure that crosses the
        // threshold and actually disables the schedule.
        for _ in 0..MAX_CONSECUTIVE_FAILURES - 1 {
            db::record_schedule_failure(
                &pool,
                schedule_id,
                agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let outcome = db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            false,
        )
        .await
        .unwrap();
        assert!(outcome.auto_disabled);

        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(!enabled, "must be disabled once the threshold is crossed");
        assert!(
            !auto_disabled,
            "the disabling call was a config error, so the streak must not be marked \
             agent-unreachable just because earlier failures in it were"
        );

        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(
            reenabled,
            Vec::<i64>::new(),
            "an unrelated reconnect must not clear a streak that ended on a config error"
        );
    }

    /// `ScheduleFailureOutcome::auto_disabled` must reflect whether *this specific
    /// call* crossed the auto-disable threshold, not just whether the schedule ends
    /// up `enabled = false` - those can disagree if a concurrent write (e.g. a human
    /// PUT disabling the schedule for an unrelated reason) races an in-flight
    /// failure recording. Simulated here without an actual second connection racing:
    /// after the schedule was already disabled and had its failure count reset by
    /// something else, a single new failure lands - it must not be reported as the
    /// call that disabled the schedule, or the caller would log a misleading
    /// "auto-disabled after repeated failures" message and insert a
    /// `ScheduleAutoDisabled` system event that misattributes the real cause.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn record_schedule_failure_does_not_report_auto_disabled_for_an_unrelated_prior_disable(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        // Simulates a concurrent human/quota disable landing between this schedule
        // being selected as due and an in-flight failure write reaching the DB:
        // enabled=false and consecutive_failures reset to 0, same as
        // set_schedule_enabled/update_schedule leave it.
        sqlx::query!(
            "UPDATE schedules SET enabled = false, consecutive_failures = 0 WHERE id = $1",
            schedule_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            true,
        )
        .await
        .unwrap();
        assert_eq!(outcome.consecutive_failures, 1);
        assert!(
            !outcome.auto_disabled,
            "one failure after an unrelated disable must not be reported as the call that \
             auto-disabled the schedule"
        );
    }

    /// A streak that contains even one config-error failure must never be marked
    /// `auto_disabled_agent_unreachable`, even when the specific failure that crosses
    /// the threshold is itself a connectivity failure - the mid-streak data problem
    /// was never confirmed fixed, so an unrelated agent reconnect must not silently
    /// re-enable the schedule.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn record_schedule_failure_requires_the_whole_streak_to_be_pure_connectivity(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        // Connectivity failure, then a config-error failure, then a connectivity
        // failure again that crosses the threshold and disables the schedule.
        db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            true,
        )
        .await
        .unwrap();
        db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            false,
        )
        .await
        .unwrap();
        let outcome = db::record_schedule_failure(
            &pool,
            schedule_id,
            agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            true,
        )
        .await
        .unwrap();
        assert!(outcome.auto_disabled);
        assert!(
            !outcome.auto_disabled_agent_unreachable,
            "the returned outcome must reflect what was actually persisted (impure streak), not \
             this call's own agent_unreachable=true argument - callers deriving a disable reason \
             from this field must not report \"agent unreachable\" here"
        );

        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(!enabled, "must be disabled once the threshold is crossed");
        assert!(
            !auto_disabled,
            "the streak contained a config error, so it must not be marked agent-unreachable even \
             though the disabling call itself was a connectivity failure"
        );

        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(
            reenabled,
            Vec::<i64>::new(),
            "an unrelated reconnect must not clear a streak that contained a config error"
        );
    }

    /// The persisted `ScheduleAutoDisabled` system event's message must describe the
    /// reason actually recorded in the database (`auto_disabled_agent_unreachable`),
    /// not just whether the specific tick that crossed the threshold happened to be a
    /// connectivity failure. A config-error failure earlier in the same streak marks
    /// it impure, so even though the disabling tick here is a connectivity failure,
    /// the schedule ends up `auto_disabled_agent_unreachable = false` - the event text
    /// must say so ("a local or configuration problem"), not "stayed unreachable",
    /// since a reconnect will never auto-heal this schedule and an operator reading
    /// the event needs to know that.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn auto_disable_event_reason_reflects_persisted_state_not_the_disabling_ticks_own_kind(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let wrong_key = [7u8; 32];
        let registry = AgentRegistry::new(); // no agent registered - every tick is a
        // connectivity failure once the key is
        // correct again
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let ui_broadcast = UiBroadcast::new();

        let past = Utc::now()
            .checked_sub_signed(chrono::Duration::hours(1))
            .unwrap();

        // Tick 1: config error (wrong key) - marks the streak impure.
        db::set_next_run_at(&pool, schedule_id, past).await.unwrap();
        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &wrong_key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &ui_broadcast,
            background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        // Ticks 2 and 3: connectivity failures (correct key, no agent connected) -
        // tick 3 crosses MAX_CONSECUTIVE_FAILURES and disables the schedule.
        for _ in 2..=MAX_CONSECUTIVE_FAILURES {
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();
            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &ui_broadcast,
                background_task_tracker: &crate::background_tasks::BackgroundTaskTracker::default(),
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &crate::notifications::NotificationService::new(pool.clone()),
                task_registry: &shared::task_registry::TaskRegistry::default(),
            })
            .await
            .unwrap();
        }

        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, MAX_CONSECUTIVE_FAILURES);
        assert!(!enabled, "must be disabled once the threshold is crossed");
        assert!(
            !auto_disabled,
            "the streak contained a config error, so it must not be marked agent-unreachable"
        );

        let events = db::get_system_events(&pool, 10).await.unwrap();
        let event = events
            .iter()
            .find(|e| matches!(e.event_type, SystemEventType::ScheduleAutoDisabled))
            .expect("a ScheduleAutoDisabled system event was recorded");
        assert!(
            event.message.contains("a local or configuration problem"),
            "event message must reflect the persisted (impure) reason: {}",
            event.message
        );
        assert!(
            !event.message.contains("stayed unreachable"),
            "event message must not claim the agent was unreachable when \
             auto_disabled_agent_unreachable ended up false: {}",
            event.message
        );
    }

    /// Retargeting an auto-disabled schedule away from the agent that caused the
    /// disable must clear the stale auto-disable bookkeeping - otherwise the old
    /// agent, no longer even a target, would incorrectly re-enable this schedule if
    /// it ever reconnects again, and the schedule would have no legitimate path back
    /// to enabled through the normal failure/reconnect cycle.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retargeting_clears_auto_disable_bookkeeping_for_the_dropped_agent(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let old_agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let new_agent =
            db::insert_agent(&pool, "retarget-test-new-agent", None, "hash", None, None)
                .await
                .unwrap();
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            db::record_schedule_failure(
                &pool,
                schedule_id,
                old_agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !enabled && auto_disabled,
            "setup must have auto-disabled it"
        );

        // Retarget the schedule away from the broken agent, the realistic remediation.
        db::delete_schedule_targets(&pool, schedule_id)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(new_agent.id, 0)])
            .await
            .unwrap();
        db::reset_schedule_failure_tracking_if_target_dropped(
            &pool,
            schedule_id,
            &[old_agent_id],
            &[new_agent.id],
        )
        .await
        .unwrap();

        let (consecutive_failures, still_enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(!still_enabled, "retargeting alone must not re-enable it");
        assert!(
            !auto_disabled,
            "the stale bookkeeping pointing at the dropped agent must be cleared"
        );
        assert_eq!(consecutive_failures, 0);

        // The old (now-unrelated) agent reconnecting must never re-enable this
        // schedule again - it isn't even a target of it anymore.
        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, old_agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(reenabled, Vec::<i64>::new());
    }

    /// Retargeting away from an agent a schedule is *partway* through a connectivity
    /// failure streak against - before it fully auto-disables - must reset the
    /// carried-over failure count too, not just once the schedule is already
    /// disabled. Otherwise a single failure from a brand new agent that has never
    /// failed could immediately cross the threshold on top of the stale count,
    /// wrongly auto-disabling the schedule and blaming the new agent.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retargeting_before_full_disable_resets_partial_connectivity_failures(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let old_agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let new_agent = db::insert_agent(
            &pool,
            "retarget-partial-new-agent",
            None,
            "hash",
            None,
            None,
        )
        .await
        .unwrap();
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        // One failure short of the threshold - still enabled, not yet auto-disabled.
        for _ in 0..MAX_CONSECUTIVE_FAILURES - 1 {
            db::record_schedule_failure(
                &pool,
                schedule_id,
                old_agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, MAX_CONSECUTIVE_FAILURES - 1);
        assert!(enabled && !auto_disabled, "must not be disabled yet");

        db::delete_schedule_targets(&pool, schedule_id)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(new_agent.id, 0)])
            .await
            .unwrap();
        db::reset_schedule_failure_tracking_if_target_dropped(
            &pool,
            schedule_id,
            &[old_agent_id],
            &[new_agent.id],
        )
        .await
        .unwrap();

        let (consecutive_failures, enabled, _) = schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(
            consecutive_failures, 0,
            "the stale failure count against the dropped agent must not carry over to the new one"
        );
        assert!(enabled);
    }

    /// The opposite of the above: retargeting must never silently clear a failure
    /// streak that contains a local/config error, even partway through it - that
    /// state is documented as requiring a human to fix the actual cause and
    /// re-enable the schedule themselves, so an unrelated retarget must not give it
    /// a free pass.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retargeting_does_not_clear_a_config_error_failure_streak(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let old_agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let new_agent =
            db::insert_agent(&pool, "retarget-config-new-agent", None, "hash", None, None)
                .await
                .unwrap();
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        // A config/data failure taints the streak's purity, one short of the threshold.
        db::record_schedule_failure(
            &pool,
            schedule_id,
            old_agent_id,
            next,
            MAX_CONSECUTIVE_FAILURES,
            false,
        )
        .await
        .unwrap();
        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, 1);
        assert!(enabled && !auto_disabled);

        db::delete_schedule_targets(&pool, schedule_id)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(new_agent.id, 0)])
            .await
            .unwrap();
        db::reset_schedule_failure_tracking_if_target_dropped(
            &pool,
            schedule_id,
            &[old_agent_id],
            &[new_agent.id],
        )
        .await
        .unwrap();

        let (consecutive_failures, enabled, _) = schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(
            consecutive_failures, 1,
            "a config-error-tainted streak must survive an unrelated retarget"
        );
        assert!(enabled);
    }

    /// For a multi-target schedule, editing the target list must not forgive a
    /// still-targeted, still-broken agent just because an unrelated sibling target
    /// was dropped in the same edit - only dropping the agent actually recorded as
    /// the cause (`auto_disabled_by_agent_id`) may reset the bookkeeping.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retargeting_a_multi_target_schedule_does_not_reset_if_causing_agent_still_targeted(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let causing_agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let sibling_agent = db::insert_agent(&pool, "multi-drop-sibling", None, "hash", None, None)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(sibling_agent.id, 1)])
            .await
            .unwrap();
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            db::record_schedule_failure(
                &pool,
                schedule_id,
                causing_agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(!enabled && auto_disabled, "setup must auto-disable it");

        // Drop only the unrelated sibling; the causing agent stays a target.
        db::delete_schedule_targets(&pool, schedule_id)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(causing_agent_id, 0)])
            .await
            .unwrap();
        db::reset_schedule_failure_tracking_if_target_dropped(
            &pool,
            schedule_id,
            &[causing_agent_id, sibling_agent.id],
            &[causing_agent_id],
        )
        .await
        .unwrap();

        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !enabled && auto_disabled,
            "dropping an unrelated sibling must not clear bookkeeping the causing agent is still \
             responsible for"
        );
        assert_eq!(consecutive_failures, MAX_CONSECUTIVE_FAILURES);
    }

    /// The inverse of the above: dropping the agent actually recorded as the cause of
    /// an auto-disable must clear the bookkeeping, even if an unrelated sibling target
    /// stays on the schedule.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn retargeting_a_multi_target_schedule_resets_when_the_causing_agent_is_dropped(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let causing_agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;
        let sibling_agent = db::insert_agent(&pool, "multi-drop-causing", None, "hash", None, None)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(sibling_agent.id, 1)])
            .await
            .unwrap();
        let next = Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            db::record_schedule_failure(
                &pool,
                schedule_id,
                causing_agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(!enabled && auto_disabled, "setup must auto-disable it");

        // Drop the causing agent; the unrelated sibling stays a target.
        db::delete_schedule_targets(&pool, schedule_id)
            .await
            .unwrap();
        db::insert_schedule_targets(&pool, schedule_id, &[(sibling_agent.id, 0)])
            .await
            .unwrap();
        db::reset_schedule_failure_tracking_if_target_dropped(
            &pool,
            schedule_id,
            &[causing_agent_id, sibling_agent.id],
            &[sibling_agent.id],
        )
        .await
        .unwrap();

        let (consecutive_failures, still_enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !still_enabled,
            "dropping the causing agent alone must not re-enable the schedule"
        );
        assert!(
            !auto_disabled,
            "dropping the causing agent must clear the stale bookkeeping"
        );
        assert_eq!(consecutive_failures, 0);
    }

    /// A schedule the scheduler auto-disabled after repeated unreachable-agent
    /// failures must come back on its own once the agent reconnects - never stay off
    /// until a human notices, and never touch a schedule disabled for another reason.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn reconnect_reenables_only_auto_disabled_schedules(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, auto_disabled_schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let next = Utc::now()
                .checked_add_signed(chrono::Duration::days(1))
                .unwrap();
            db::record_schedule_failure(
                &pool,
                auto_disabled_schedule_id,
                agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) =
            schedule_failure_state(&pool, auto_disabled_schedule_id).await;
        assert!(!enabled && auto_disabled, "setup must have disabled it");

        // A schedule disabled for an unrelated reason (e.g. a human, or quota
        // enforcement) targeting the same agent must be left untouched.
        let human_disabled_schedule_id = db::insert_schedule(
            &pool,
            db::get_schedule_by_id(&pool, auto_disabled_schedule_id)
                .await
                .unwrap()
                .repo_id
                .unwrap(),
            &ScheduleParams {
                name: "human-disabled-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: false,
                canary_enabled: false,
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
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap()
        .id;
        db::insert_schedule_targets(&pool, human_disabled_schedule_id, &[(agent_id, 0)])
            .await
            .unwrap();

        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();

        assert_eq!(reenabled, vec![auto_disabled_schedule_id]);
        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, auto_disabled_schedule_id).await;
        assert_eq!(consecutive_failures, 0);
        assert!(enabled);
        assert!(!auto_disabled);
        let (_, human_still_disabled, _) =
            schedule_failure_state(&pool, human_disabled_schedule_id).await;
        assert!(
            !human_still_disabled,
            "a schedule disabled for another reason must not be re-enabled"
        );
    }

    /// A schedule the scheduler auto-disabled must not be silently re-enabled on
    /// reconnect once something else has since taken over its `enabled` state -
    /// `set_schedule_enabled` must clear `auto_disabled_agent_unreachable`, or a
    /// later disable-for-another-reason (e.g. a human re-enables it, then quota
    /// enforcement disables it again) would look indistinguishable from the
    /// original auto-disable to the reconnect handler.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn set_schedule_enabled_clears_stale_auto_disabled_flag(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let next = Utc::now()
                .checked_add_signed(chrono::Duration::days(1))
                .unwrap();
            db::record_schedule_failure(
                &pool,
                schedule_id,
                agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !enabled && auto_disabled,
            "setup must have auto-disabled it"
        );

        // A human re-enables it, then something else (e.g. quota enforcement)
        // disables it again for an unrelated reason.
        db::set_schedule_enabled(&pool, schedule_id, true)
            .await
            .unwrap();
        db::set_schedule_enabled(&pool, schedule_id, false)
            .await
            .unwrap();

        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(!enabled);
        assert!(
            !auto_disabled,
            "a direct enabled write must clear the stale auto-disabled flag"
        );

        // Reconnect must not silently lift this unrelated disable.
        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(reenabled, Vec::<i64>::new());
        let (_, still_enabled, _) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !still_enabled,
            "schedule must stay disabled after reconnect"
        );
    }

    /// A schedule can have one target that's permanently unreachable and another
    /// that's always reachable. With `on_failure: Continue`, every tick processes
    /// both: the failing target's failure must keep counting toward auto-disable
    /// even though the other target succeeds in the very same tick and every tick
    /// after it - a schedule-wide reset triggered by the succeeding target must not
    /// erase the other target's failure count, or the unreachable target would
    /// retry forever exactly like the bug this feature fixes, just masked by its
    /// sibling target's success.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_disables_for_unreachable_target_despite_sibling_target_succeeding(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let passphrase_enc = shared::crypto::encrypt_passphrase("test-pass", &key).unwrap();
        let unreachable_agent =
            db::insert_agent(&pool, "continue-test-unreachable", None, "hash", None, None)
                .await
                .unwrap();
        let reachable_agent =
            db::insert_agent(&pool, "continue-test-reachable", None, "hash", None, None)
                .await
                .unwrap();
        let repo = db::insert_repo(
            &pool,
            &InsertRepoParams {
                name: "continue-test-repo",
                repo_path: "/backup/continue-test",
                ssh_user: "borg",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_enc,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::update_repo_ssh_host_key(&pool, repo.id, "ssh-ed25519 AAAACONTINUETEST")
            .await
            .unwrap();
        let schedule = db::insert_schedule(
            &pool,
            repo.id,
            &ScheduleParams {
                name: "continue-test-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
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
                on_failure: "continue",
            },
            None,
        )
        .await
        .unwrap();
        let schedule_id = schedule.id;
        db::insert_schedule_targets(
            &pool,
            schedule_id,
            &[(unreachable_agent.id, 0), (reachable_agent.id, 1)],
        )
        .await
        .unwrap();

        let registry = AgentRegistry::new();
        let (tx, mut rx) = mpsc::channel(32);
        registry.register(reachable_agent.id, tx, false, None).await;
        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let background_task_tracker = crate::background_tasks::BackgroundTaskTracker::default();

        for attempt in 1..=MAX_CONSECUTIVE_FAILURES {
            let past = Utc::now()
                .checked_sub_signed(chrono::Duration::hours(1))
                .unwrap();
            db::set_next_run_at(&pool, schedule_id, past).await.unwrap();

            tick(&TickDeps {
                pool: &pool,
                registry: &registry,
                encryption_key: &key,
                tunnel_manager: &tunnel,
                completion_bus: &bus,
                repo_lock: &RepoLock::default(),
                repo_op_tracker: &RepoOpTracker::default(),
                ui_broadcast: &UiBroadcast::new(),
                background_task_tracker: &background_task_tracker,
                power_sessions: &crate::power::PowerSessionTracker::default(),
                notification_service: &crate::notifications::NotificationService::new(pool.clone()),
                task_registry: &shared::task_registry::TaskRegistry::default(),
            })
            .await
            .unwrap();

            // tick() only waits for the first target (the unreachable one, which is
            // what lets the assertions below run right after it returns) - the
            // reachable target's own trigger/completion runs in the background. Drive
            // it to completion and wait for the whole tick's background task to go
            // idle before the next iteration; otherwise its lingering
            // mark_schedule_triggered_once/await_target_completion call can straggle
            // into the next iteration and race this test's own forced next_run_at,
            // which is what made this test flake in CI.
            let trigger = loop {
                match rx
                    .recv()
                    .await
                    .expect("expected messages for the reachable target")
                {
                    shared::protocol::ServerToAgent::ConfigUpdate(_) => {}
                    other => break other,
                }
            };
            assert!(
                matches!(
                    trigger,
                    shared::protocol::ServerToAgent::RunBackupNow { .. }
                ),
                "expected RunBackupNow for the reachable target, got: {trigger:?}"
            );
            bus.publish(completion_bus::OperationOutcome {
                agent_id: reachable_agent.id,
                repo_id: repo.id,
                success: true,
            });
            assert!(
                background_task_tracker
                    .wait_until_idle(std::time::Duration::from_secs(5))
                    .await,
                "the tick's background task must finish before the next iteration"
            );

            let (consecutive_failures, enabled, auto_disabled) =
                schedule_failure_state(&pool, schedule_id).await;
            assert_eq!(
                consecutive_failures, attempt,
                "the reachable target succeeding must not reset the unreachable target's failure \
                 count"
            );
            if attempt < MAX_CONSECUTIVE_FAILURES {
                assert!(enabled);
                assert!(!auto_disabled);
            } else {
                assert!(!enabled, "must auto-disable once the threshold is reached");
                assert!(auto_disabled);
            }
        }
    }

    /// A multi-target schedule auto-disabled because of one specific unreachable
    /// target must not be re-enabled just because a *different* target of the same
    /// schedule reconnects - only the target whose failures actually caused the
    /// disable should bring it back. Without this, a schedule with one permanently-
    /// broken target and one merely-flaky-but-fine target would have its failure
    /// count reset every time the flaky target's routine reconnects happened, and
    /// the broken target would never actually reach the auto-disable threshold for
    /// good.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn reconnect_only_reenables_for_the_agent_that_caused_the_disable(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let passphrase_enc = shared::crypto::encrypt_passphrase("test-pass", &key).unwrap();
        let broken_agent =
            db::insert_agent(&pool, "scoped-reconnect-broken", None, "hash", None, None)
                .await
                .unwrap();
        let flaky_agent =
            db::insert_agent(&pool, "scoped-reconnect-flaky", None, "hash", None, None)
                .await
                .unwrap();
        let repo = db::insert_repo(
            &pool,
            &InsertRepoParams {
                name: "scoped-reconnect-repo",
                repo_path: "/backup/scoped-reconnect",
                ssh_user: "borg",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_enc,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::update_repo_ssh_host_key(&pool, repo.id, "ssh-ed25519 AAAASCOPEDTEST")
            .await
            .unwrap();
        let schedule = db::insert_schedule(
            &pool,
            repo.id,
            &ScheduleParams {
                name: "scoped-reconnect-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
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
                on_failure: "continue",
            },
            None,
        )
        .await
        .unwrap();
        let schedule_id = schedule.id;
        db::insert_schedule_targets(
            &pool,
            schedule_id,
            &[(broken_agent.id, 0), (flaky_agent.id, 1)],
        )
        .await
        .unwrap();

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let next = Utc::now()
                .checked_add_signed(chrono::Duration::days(1))
                .unwrap();
            db::record_schedule_failure(
                &pool,
                schedule_id,
                broken_agent.id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !enabled && auto_disabled,
            "setup must have auto-disabled it"
        );

        let reenabled_by_flaky =
            db::reenable_system_disabled_schedules_for_agent(&pool, flaky_agent.id, Utc::now())
                .await
                .unwrap();
        assert_eq!(
            reenabled_by_flaky,
            Vec::<i64>::new(),
            "a different target of the schedule reconnecting must not re-enable it"
        );
        let (consecutive_failures, still_enabled, still_auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(!still_enabled);
        assert!(still_auto_disabled);
        assert_eq!(consecutive_failures, MAX_CONSECUTIVE_FAILURES);

        let reenabled_by_broken =
            db::reenable_system_disabled_schedules_for_agent(&pool, broken_agent.id, Utc::now())
                .await
                .unwrap();
        assert_eq!(reenabled_by_broken, vec![schedule_id]);
        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert_eq!(consecutive_failures, 0);
        assert!(enabled);
        assert!(!auto_disabled);
    }

    /// `update_schedule` backs the general edit-schedule form, which resubmits
    /// `enabled` unchanged on every save of any other field. It must preserve the
    /// auto-disable bookkeeping when `enabled` doesn't actually change, or an
    /// unrelated edit (e.g. a retention tweak) made while the agent is still offline
    /// would permanently strand the schedule - the reconnect handler matches on
    /// `auto_disabled_by_agent_id`, which this same edit would otherwise have wiped.
    /// An edit that *does* toggle `enabled` must still clear it, exactly like
    /// `set_schedule_enabled`.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn update_schedule_preserves_auto_disabled_state_unless_enabled_changes(
        pool: sqlx::PgPool,
    ) {
        let key = tick_test_key();
        let (_, schedule_id, _) = setup_due_schedule(&pool, &key).await;
        let agent_id = db::get_agent_by_hostname(&pool, TICK_TEST_HOSTNAME, None)
            .await
            .unwrap()
            .id;

        let unrelated_edit_params = |enabled: bool| ScheduleParams {
            name: "tick-sched-renamed",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled,
            canary_enabled: false,
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
            on_failure: "stop",
        };

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let next = Utc::now()
                .checked_add_signed(chrono::Duration::days(1))
                .unwrap();
            db::record_schedule_failure(
                &pool,
                schedule_id,
                agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        let (_, enabled, auto_disabled) = schedule_failure_state(&pool, schedule_id).await;
        assert!(
            !enabled && auto_disabled,
            "setup must have auto-disabled it"
        );

        // Edit an unrelated field, resubmitting the current (disabled) `enabled`
        // value unchanged.
        db::update_schedule(&pool, schedule_id, &unrelated_edit_params(false))
            .await
            .unwrap();
        let (consecutive_failures, still_enabled, still_auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(!still_enabled);
        assert!(
            still_auto_disabled,
            "an unrelated edit that leaves `enabled` unchanged must not clear the auto-disabled \
             flag"
        );
        assert_eq!(consecutive_failures, MAX_CONSECUTIVE_FAILURES);

        // Reconnect must still be able to re-enable it after that unrelated edit.
        let reenabled =
            db::reenable_system_disabled_schedules_for_agent(&pool, agent_id, Utc::now())
                .await
                .unwrap();
        assert_eq!(reenabled, vec![schedule_id]);

        // Auto-disable it again, then this time actually flip `enabled` through the
        // same edit form - a real transition must still clear the bookkeeping.
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let next = Utc::now()
                .checked_add_signed(chrono::Duration::days(1))
                .unwrap();
            db::record_schedule_failure(
                &pool,
                schedule_id,
                agent_id,
                next,
                MAX_CONSECUTIVE_FAILURES,
                true,
            )
            .await
            .unwrap();
        }
        db::update_schedule(&pool, schedule_id, &unrelated_edit_params(true))
            .await
            .unwrap();
        let (consecutive_failures, enabled, auto_disabled) =
            schedule_failure_state(&pool, schedule_id).await;
        assert!(enabled);
        assert!(
            !auto_disabled,
            "an edit that actually toggles `enabled` must still clear the flag"
        );
        assert_eq!(consecutive_failures, 0);
    }

    /// Like `setup_due_schedule`, but with two targets so `tick()` dispatches
    /// through `run_sequential_schedule` for more than a single agent.
    async fn setup_due_sequential_schedule(pool: &sqlx::PgPool, key: &[u8; 32]) -> (i64, i64, i64) {
        let passphrase_enc = shared::crypto::encrypt_passphrase("test-pass", key).unwrap();
        let agent1 = db::insert_agent(pool, "tick-test-agent-1", None, "hash", None, None)
            .await
            .unwrap();
        let agent2 = db::insert_agent(pool, "tick-test-agent-2", None, "hash", None, None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "tick-sequential-repo",
                repo_path: "/backup/tick-sequential",
                ssh_user: "borg",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_enc,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAATICKTEST")
            .await
            .unwrap();
        let schedule = db::insert_schedule(
            pool,
            repo.id,
            &ScheduleParams {
                name: "tick-sequential-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
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
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(agent1.id, 0), (agent2.id, 1)])
            .await
            .unwrap();
        let past = Utc::now()
            .checked_sub_signed(chrono::Duration::hours(1))
            .unwrap();
        db::set_next_run_at(pool, schedule.id, past).await.unwrap();
        (repo.id, agent1.id, agent2.id)
    }

    /// Regression test for the untracked `tokio::spawn(run_sequential_schedule)` in
    /// `tick()`: without a `background_task_tracker` guard claimed before the spawn,
    /// whether `await_target_completion` finishes before e2e teardown polls
    /// `background_ops_in_flight` is a scheduling race - which is what produced the
    /// non-deterministic coverage on this function described in #366/#378.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_tracks_sequential_schedule_via_background_task_tracker(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (repo_id, agent1_id, agent2_id) = setup_due_sequential_schedule(&pool, &key).await;

        let registry = AgentRegistry::new();
        let (tx1, mut rx1) = mpsc::channel(32);
        registry.register(agent1_id, tx1, false, None).await;
        let (tx2, mut rx2) = mpsc::channel(32);
        registry.register(agent2_id, tx2, false, None).await;

        let tunnel = dummy_tunnel(pool.clone());
        let bus = CompletionBus::new();
        let background_task_tracker = crate::background_tasks::BackgroundTaskTracker::default();

        tick(&TickDeps {
            pool: &pool,
            registry: &registry,
            encryption_key: &key,
            tunnel_manager: &tunnel,
            completion_bus: &bus,
            repo_lock: &RepoLock::default(),
            repo_op_tracker: &RepoOpTracker::default(),
            ui_broadcast: &UiBroadcast::new(),
            background_task_tracker: &background_task_tracker,
            power_sessions: &crate::power::PowerSessionTracker::default(),
            notification_service: &crate::notifications::NotificationService::new(pool.clone()),
            task_registry: &shared::task_registry::TaskRegistry::default(),
        })
        .await
        .unwrap();

        assert!(
            background_task_tracker.any_active(),
            "the spawned sequential-schedule task must be tracked immediately after tick() \
             returns, not only once it gets its first poll"
        );

        // Each target gets a ConfigUpdate before its trigger message; skip past it to
        // the actual RunBackupNow so the first target can be reported complete.
        let first_trigger = loop {
            match rx1
                .recv()
                .await
                .expect("expected messages for first target")
            {
                shared::protocol::ServerToAgent::ConfigUpdate(_) => {}
                other => break other,
            }
        };
        assert!(
            matches!(
                first_trigger,
                shared::protocol::ServerToAgent::RunBackupNow { .. }
            ),
            "expected RunBackupNow for the first target, got: {first_trigger:?}"
        );
        bus.publish(completion_bus::OperationOutcome {
            agent_id: agent1_id,
            repo_id,
            success: true,
        });

        let second_trigger = loop {
            match rx2
                .recv()
                .await
                .expect("expected messages for second target")
            {
                shared::protocol::ServerToAgent::ConfigUpdate(_) => {}
                other => break other,
            }
        };
        assert!(
            matches!(
                second_trigger,
                shared::protocol::ServerToAgent::RunBackupNow { .. }
            ),
            "expected RunBackupNow for the second target, got: {second_trigger:?}"
        );
        bus.publish(completion_bus::OperationOutcome {
            agent_id: agent2_id,
            repo_id,
            success: true,
        });

        assert!(
            background_task_tracker
                .wait_until_idle(std::time::Duration::from_secs(5))
                .await,
            "background_task_tracker must go idle once both targets have completed"
        );
    }
}
