// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Schedules: their repositories and targets, due runs, failure tracking and
//! auto-disable.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::{
    hooks::HookCommand,
    types::{ExecutionMode, OnFailure, ScheduleType, ScheduleWakeOverride, Visibility},
};
use sqlx::PgPool;

use super::HookCommands;
use crate::error::ApiError;

/// Takes a `SELECT ... FOR UPDATE` lock on every schedule that currently targets
/// `agent_id`, so a concurrent single-statement `UPDATE schedules ...` for one of
/// them - most importantly `record_schedule_failure`, run by an in-flight scheduler
/// tick - blocks until this transaction commits or rolls back, instead of racing it.
///
/// Without this, a schedule that crosses the auto-disable threshold for this agent
/// (setting `auto_disabled_by_agent_id = agent_id`, `auto_disabled_agent_unreachable =
/// true`, `consecutive_failures = N`) in a transaction that commits after this
/// transaction's own bookkeeping-reset check ran but before its destructive step
/// (deleting or merging away the agent) would end up permanently stale: the
/// destructive step's `ON DELETE SET NULL` FK only clears `auto_disabled_by_agent_id`,
/// leaving `auto_disabled_agent_unreachable`/`consecutive_failures` stuck, and since
/// the causing agent is gone, no reconnect can ever un-stick it.
///
/// Must run before anything in the same transaction that would stop `agent_id` from
/// still being a target of these schedules (e.g. retargeting `schedule_targets`) -
/// otherwise the lookup that finds which rows to lock races the same way.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub(super) async fn lock_schedules_targeting_agent(
    tx: &mut sqlx::PgConnection,
    agent_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "SELECT s.id FROM schedules s JOIN schedule_targets st ON st.schedule_id = s.id WHERE \
         st.agent_id = $1 FOR UPDATE OF s",
        agent_id,
    )
    .fetch_all(tx)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `schedules` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct ScheduleRow {
    /// Unique identifier.
    pub id: i64,
    /// Target repository ID.
    pub repo_id: Option<i64>,
    /// Schedule display name.
    pub name: String,
    /// Schedule type (e.g. "cron", "interval").
    pub schedule_type: ScheduleType,
    /// Cron expression for scheduling.
    pub cron_expression: String,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// Whether canary backups are enabled.
    pub canary_enabled: bool,
    /// Whether this schedule stages the host's virtual machines before
    /// backing up. Requires the host itself to have staging enabled.
    pub vm_snapshot_enabled: bool,
    /// Whether this schedule dumps the host's databases into its archive.
    /// Requires the host itself to have database dumps enabled.
    pub database_backup_enabled: bool,
    /// When the schedule last ran.
    pub last_run_at: Option<DateTime<Utc>>,
    /// When the schedule is next due.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: String,
    /// Raw include pattern text, rescuing paths from a broader exclude.
    pub include_patterns_raw: String,
    /// Raw file-change detection pattern text.
    pub file_change_patterns_raw: String,
    /// Whether to ignore global exclude patterns.
    pub ignore_global_excludes: bool,
    /// Number of hourly backups to keep.
    pub keep_hourly: i32,
    /// Number of daily backups to keep.
    pub keep_daily: i32,
    /// Number of weekly backups to keep.
    pub keep_weekly: i32,
    /// Number of monthly backups to keep.
    pub keep_monthly: i32,
    /// Number of yearly backups to keep.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in KB/s, if any.
    pub rate_limit_kbps: Option<i32>,
    /// Pre-backup commands.
    #[schema(value_type = Vec<HookCommand>)]
    pub pre_backup_commands: HookCommands,
    /// Post-backup commands.
    #[schema(value_type = Vec<HookCommand>)]
    pub post_backup_commands: HookCommands,
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: i32,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled. Below this count, a miss only
    /// shows as a warning.
    pub missed_backup_threshold: i32,
    /// How much time must be left before `next_run_at` for a catch-up to still
    /// start; a host coming back closer than this drops the pending miss
    /// instead. The only catch-up setting a schedule has: whether a host is
    /// waited for at all is the host's own setting.
    pub catch_up_min_lead_minutes: i32,
    /// Execution mode (e.g. "sequential").
    pub execution_mode: ExecutionMode,
    /// On-failure behaviour (e.g. "continue", "abort").
    pub on_failure: OnFailure,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: Visibility,
    /// Whether this schedule wakes the hosts it needs, overriding what those
    /// hosts default to. One of `host_default`, `enabled`, `disabled` -
    /// parsed into [`shared::types::ScheduleWakeOverride`] at the point of
    /// use.
    pub wake_override: ScheduleWakeOverride,
    /// Hostnames of target agents, resolved at query time.
    #[serde(default)]
    #[sqlx(default)]
    pub target_hostnames: Vec<String>,
    /// How many of this schedule's targets have a run waiting to be caught up
    /// once their host reconnects, resolved at query time. At most one per
    /// target: misses overwrite rather than accumulate.
    pub catch_up_pending_count: i64,
    /// How many consecutive attempts have failed to reach the schedule's target
    /// agent(s) since the last success (or reconnect). Reset to 0 on success, on
    /// reconnect from the causing agent, or on any direct edit of `enabled`.
    pub consecutive_failures: i32,
    /// Whether the scheduler itself disabled this schedule after
    /// `consecutive_failures` reached the threshold, specifically because its
    /// target agent stayed unreachable - as opposed to a local/data failure (e.g. a
    /// corrupted passphrase), a human, or quota enforcement. Only ever true while
    /// `enabled` is false; the schedule re-enables automatically once the causing
    /// agent reconnects.
    pub auto_disabled_agent_unreachable: bool,
}

/// A row from the `schedule_targets` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleTargetRow {
    /// Agent ID.
    pub agent_id: i64,
    /// Execution order among targets for the same schedule.
    pub execution_order: i32,
    /// The occurrence this target missed while its host was unreachable, waiting to
    /// be caught up when that host reconnects. `None` when nothing is pending.
    pub catch_up_pending_for: Option<DateTime<Utc>>,
}

/// A row from the `schedule_repos` join table: one repository a schedule
/// writes into.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleRepoRow {
    /// Repository ID.
    pub repo_id: i64,
    /// Write order among the schedule's repositories.
    pub execution_order: i32,
    /// Whether a failure on this repository fails the whole run.
    pub required: bool,
    /// The occurrence this repository missed, waiting to be caught up.
    pub catch_up_pending_for: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedule_repos(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleRepoRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRepoRow,
        "SELECT repo_id, execution_order, required, catch_up_pending_for FROM schedule_repos \
         WHERE schedule_id = $1 ORDER BY execution_order, repo_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Replaces a schedule's repository targets, in the order given, and
/// re-points `schedules.repo_id` at the first of them.
///
/// One transaction: the denormalised primary target on `schedules` and the
/// rows it summarises must never be observed disagreeing by a scheduler tick
/// running concurrently.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if any of the queries fails.
pub async fn replace_schedule_repos(
    pool: &PgPool,
    schedule_id: i64,
    targets: &[(i64, bool)],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    sqlx::query!(
        "DELETE FROM schedule_repos WHERE schedule_id = $1",
        schedule_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    for (order, (repo_id, required)) in targets.iter().enumerate() {
        let execution_order = i32::try_from(order).unwrap_or(i32::MAX);
        sqlx::query!(
            "INSERT INTO schedule_repos (schedule_id, repo_id, execution_order, required) VALUES \
             ($1, $2, $3, $4)",
            schedule_id,
            *repo_id,
            execution_order,
            *required,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    // The first *required* target, not simply the first: the denormalised
    // primary is what health, quota and reports key off, so it must not point
    // at a copy the schedule is allowed to lose. The fallback only covers an
    // empty list - the API never accepts one with no required target.
    let primary = targets
        .iter()
        .find(|(_, required)| *required)
        .or_else(|| targets.first())
        .map(|(repo_id, _)| *repo_id);
    sqlx::query!(
        "UPDATE schedules SET repo_id = $2 WHERE id = $1",
        schedule_id,
        primary,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)
}

/// Number of schedules targeting a specific agent.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleCountByAgent {
    /// Agent ID.
    pub agent_id: i64,
    /// Number of distinct schedules targeting this agent.
    pub count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_counts_by_agent(
    pool: &PgPool,
) -> Result<Vec<ScheduleCountByAgent>, ApiError> {
    sqlx::query_as!(
        ScheduleCountByAgent,
        "SELECT agent_id, COUNT(DISTINCT schedule_id)::bigint AS \"count!\" FROM schedule_targets \
         GROUP BY agent_id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_agents_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<ScheduleRunTarget>, ApiError> {
    sqlx::query_as!(
        ScheduleRunTarget,
        "SELECT DISTINCT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id JOIN schedule_repos sr ON sr.schedule_id = st.schedule_id WHERE \
         sr.repo_id = $1",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules(pool: &PgPool) -> Result<Vec<ScheduleRow>, ApiError> {
    let rows = sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type AS \"schedule_type: ScheduleType\", \
         s.cron_expression, s.enabled, s.canary_enabled, s.vm_snapshot_enabled, \
         s.database_backup_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.include_patterns_raw, s.file_change_patterns_raw, s.ignore_global_excludes, \
         s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, s.keep_yearly, \
         s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", s.post_backup_commands AS \"post_backup_commands: HookCommands\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.catch_up_min_lead_minutes, \
         s.execution_mode AS \"execution_mode: ExecutionMode\", s.on_failure AS \"on_failure: \
         OnFailure\", s.owner_id, s.visibility AS \"visibility: Visibility\", s.wake_override AS \
         \"wake_override: ScheduleWakeOverride\", s.consecutive_failures, \
         s.auto_disabled_agent_unreachable, ARRAY(SELECT a.hostname FROM schedule_targets st JOIN \
         agents a ON a.id = st.agent_id WHERE st.schedule_id = s.id ORDER BY st.execution_order, \
         a.hostname) AS \"target_hostnames!\", (SELECT COUNT(*) FROM schedule_targets stc WHERE \
         stc.schedule_id = s.id AND stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM \
         schedule_repos src WHERE src.schedule_id = s.id AND src.catch_up_pending_for IS NOT \
         NULL) AS \"catch_up_pending_count!\" FROM schedules s ORDER BY s.id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}

/// Parameters for creating or updating a schedule.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct ScheduleParams<'a> {
    /// Schedule display name.
    pub name: &'a str,
    /// What the schedule runs.
    pub schedule_type: ScheduleType,
    /// Cron expression.
    pub cron_expression: &'a str,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// Whether canary backups are enabled.
    pub canary_enabled: bool,
    /// Whether this schedule stages the host's virtual machines first.
    pub vm_snapshot_enabled: bool,
    /// Whether this schedule dumps the host's databases into its archive.
    pub database_backup_enabled: bool,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: &'a str,
    /// Raw include pattern text, rescuing paths from a broader exclude.
    pub include_patterns_raw: &'a str,
    /// Whether to ignore global excludes.
    pub ignore_global_excludes: bool,
    /// Hourly retention count.
    pub keep_hourly: i32,
    /// Daily retention count.
    pub keep_daily: i32,
    /// Weekly retention count.
    pub keep_weekly: i32,
    /// Monthly retention count.
    pub keep_monthly: i32,
    /// Yearly retention count.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in KB/s.
    pub rate_limit_kbps: Option<i32>,
    /// Pre-backup commands.
    pub pre_backup_commands: &'a [HookCommand],
    /// Post-backup commands.
    pub post_backup_commands: &'a [HookCommand],
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: i32,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled.
    pub missed_backup_threshold: i32,
    /// How much time must be left before the next scheduled run for a catch-up
    /// to still start.
    pub catch_up_min_lead_minutes: i32,
    /// On-failure behaviour.
    pub on_failure: OnFailure,
    /// Raw file-change detection pattern text.
    pub file_change_patterns_raw: &'a str,
    /// Whether this schedule wakes the hosts it needs, overriding the hosts'
    /// own defaults.
    pub wake_override: ScheduleWakeOverride,
}

#[cfg(test)]
impl<'a> ScheduleParams<'a> {
    /// The plain nightly backup schedule a test fixture wants when it only
    /// cares that *a* schedule exists.
    ///
    /// Spelled out in full at each fixture until the duplicate-code check
    /// caught two of them being the same thirty lines; anything a test does
    /// care about it overrides on the returned value.
    #[must_use]
    pub fn for_test(name: &'a str, cron_expression: &'a str) -> Self {
        Self {
            wake_override: ScheduleWakeOverride::HostDefault,
            name,
            schedule_type: ScheduleType::Backup,
            cron_expression,
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            database_backup_enabled: false,
            exclude_patterns_raw: "",
            include_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            catch_up_min_lead_minutes: 120,
            on_failure: OnFailure::Stop,
        }
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_schedule(
    pool: &PgPool,
    repo_id: i64,
    params: &ScheduleParams<'_>,
    owner_id: Option<i64>,
) -> Result<ScheduleRow, ApiError> {
    // One transaction: `list_due_schedules` and `config_assembler` dispatch
    // strictly off `schedule_repos`, so a schedules row that lost its target
    // row to a failed or cancelled second statement would silently never run
    // and never reach an agent's config, with no error surfaced to anyone.
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let schedule = sqlx::query_as!(
        ScheduleRow,
        "INSERT INTO schedules (repo_id, name, schedule_type, cron_expression, enabled, \
         canary_enabled, exclude_patterns_raw, file_change_patterns_raw, ignore_global_excludes, \
         keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, \
         rate_limit_kbps, pre_backup_commands, post_backup_commands, execution_mode, on_failure, \
         owner_id, hook_timeout_seconds, missed_backup_threshold, vm_snapshot_enabled, \
         catch_up_min_lead_minutes, wake_override, include_patterns_raw, database_backup_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, \
         'sequential', $19, $20, $21, $22, $23, $24, $25, $26, $27) RETURNING id, repo_id, name, \
         schedule_type AS \"schedule_type: ScheduleType\", cron_expression, enabled, \
         canary_enabled, vm_snapshot_enabled, database_backup_enabled, last_run_at, next_run_at, \
         exclude_patterns_raw, include_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", post_backup_commands AS \"post_backup_commands: HookCommands\", \
         hook_timeout_seconds, missed_backup_threshold, catch_up_min_lead_minutes, execution_mode \
         AS \"execution_mode: ExecutionMode\", on_failure AS \"on_failure: OnFailure\", owner_id, \
         visibility AS \"visibility: Visibility\", wake_override AS \"wake_override: \
         ScheduleWakeOverride\", consecutive_failures, auto_disabled_agent_unreachable, \
         ARRAY[]::TEXT[] AS \"target_hostnames!\", (SELECT COUNT(*) FROM schedule_targets stc \
         WHERE stc.schedule_id = schedules.id AND stc.catch_up_pending_for IS NOT NULL) + (SELECT \
         COUNT(*) FROM schedule_repos src WHERE src.schedule_id = schedules.id AND \
         src.catch_up_pending_for IS NOT NULL) AS \"catch_up_pending_count!\"",
        repo_id,
        params.name,
        params.schedule_type.to_string(),
        params.cron_expression,
        params.enabled,
        params.canary_enabled,
        params.exclude_patterns_raw,
        params.file_change_patterns_raw,
        params.ignore_global_excludes,
        params.keep_hourly,
        params.keep_daily,
        params.keep_weekly,
        params.keep_monthly,
        params.keep_yearly,
        params.compact_enabled,
        params.rate_limit_kbps,
        sqlx::types::Json(params.pre_backup_commands) as _,
        sqlx::types::Json(params.post_backup_commands) as _,
        params.on_failure.to_string(),
        owner_id,
        params.hook_timeout_seconds,
        params.missed_backup_threshold,
        params.vm_snapshot_enabled,
        params.catch_up_min_lead_minutes,
        params.wake_override.to_string(),
        params.include_patterns_raw,
        params.database_backup_enabled,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Every schedule owns at least the repository it was created with, so
    // callers that know nothing about multiple targets (config import, the
    // agent-side auto-provisioning in `ws::handler`) still produce a schedule
    // the scheduler can dispatch.
    sqlx::query!(
        "INSERT INTO schedule_repos (schedule_id, repo_id, execution_order, required) VALUES ($1, \
         $2, 0, TRUE)",
        schedule.id,
        repo_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;

    Ok(schedule)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_schedule(
    pool: &PgPool,
    id: i64,
    params: &ScheduleParams<'_>,
) -> Result<ScheduleRow, ApiError> {
    // Unlike set_schedule_enabled (only ever called to explicitly flip `enabled`),
    // this backs the general edit-schedule form, which resubmits `enabled` unchanged
    // on every save of any other field (rename, retention tweak, cron change, ...).
    // Clearing the auto-disable bookkeeping unconditionally here would let an
    // unrelated edit permanently strand an auto-disabled schedule: the reconnect
    // handler only matches on auto_disabled_by_agent_id, so once this cleared it,
    // reconnecting would never re-enable the schedule again. Only clear it when
    // `enabled` is actually transitioning - i.e. a human explicitly toggled it
    // through this same form - the same "direct write from outside the failure-
    // tracking path" trigger set_schedule_enabled uses.
    sqlx::query_as!(
        ScheduleRow,
        "UPDATE schedules SET name = $2, cron_expression = $3, enabled = $4, canary_enabled = $5, \
         exclude_patterns_raw = $6, file_change_patterns_raw = $7, ignore_global_excludes = $8, \
         keep_hourly = $9, keep_daily = $10, keep_weekly = $11, keep_monthly = $12, keep_yearly = \
         $13, compact_enabled = $14, rate_limit_kbps = $15, pre_backup_commands = $16, \
         post_backup_commands = $17, execution_mode = 'sequential', on_failure = $18, \
         hook_timeout_seconds = $19, missed_backup_threshold = $20, vm_snapshot_enabled = $21, \
         catch_up_min_lead_minutes = $22, wake_override = $23, auto_disabled_agent_unreachable = \
         CASE WHEN enabled IS DISTINCT FROM $4 THEN false ELSE auto_disabled_agent_unreachable \
         END, auto_disabled_by_agent_id = CASE WHEN enabled IS DISTINCT FROM $4 THEN NULL ELSE \
         auto_disabled_by_agent_id END, consecutive_failures = CASE WHEN enabled IS DISTINCT FROM \
         $4 THEN 0 ELSE consecutive_failures END, failure_streak_pure_connectivity = CASE WHEN \
         enabled IS DISTINCT FROM $4 THEN true ELSE failure_streak_pure_connectivity END, \
         include_patterns_raw = $24, database_backup_enabled = $25 WHERE id = $1 RETURNING id, \
         repo_id, name, schedule_type AS \"schedule_type: ScheduleType\", cron_expression, \
         enabled, canary_enabled, vm_snapshot_enabled, database_backup_enabled, last_run_at, \
         next_run_at, exclude_patterns_raw, include_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", post_backup_commands AS \"post_backup_commands: HookCommands\", \
         hook_timeout_seconds, missed_backup_threshold, catch_up_min_lead_minutes, execution_mode \
         AS \"execution_mode: ExecutionMode\", on_failure AS \"on_failure: OnFailure\", owner_id, \
         visibility AS \"visibility: Visibility\", wake_override AS \"wake_override: \
         ScheduleWakeOverride\", consecutive_failures, auto_disabled_agent_unreachable, \
         ARRAY[]::TEXT[] AS \"target_hostnames!\", (SELECT COUNT(*) FROM schedule_targets stc \
         WHERE stc.schedule_id = schedules.id AND stc.catch_up_pending_for IS NOT NULL) + (SELECT \
         COUNT(*) FROM schedule_repos src WHERE src.schedule_id = schedules.id AND \
         src.catch_up_pending_for IS NOT NULL) AS \"catch_up_pending_count!\"",
        id,
        params.name,
        params.cron_expression,
        params.enabled,
        params.canary_enabled,
        params.exclude_patterns_raw,
        params.file_change_patterns_raw,
        params.ignore_global_excludes,
        params.keep_hourly,
        params.keep_daily,
        params.keep_weekly,
        params.keep_monthly,
        params.keep_yearly,
        params.compact_enabled,
        params.rate_limit_kbps,
        sqlx::types::Json(params.pre_backup_commands) as _,
        sqlx::types::Json(params.post_backup_commands) as _,
        params.on_failure.to_string(),
        params.hook_timeout_seconds,
        params.missed_backup_threshold,
        params.vm_snapshot_enabled,
        params.catch_up_min_lead_minutes,
        params.wake_override.to_string(),
        params.include_patterns_raw,
        params.database_backup_enabled,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

/// Points a schedule at a single repository, replacing whatever target list
/// it had.
///
/// This is the pre-multi-target contract of `PUT /schedules/{id}` with a bare
/// `repo_id`: a caller that says "this schedule backs up to X" and knows
/// nothing about a list is asserting the whole list. Callers that do know send
/// `repo_targets`, which goes through [`replace_schedule_repos`] instead.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_schedule_repo(pool: &PgPool, id: i64, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query_scalar!("SELECT id FROM schedules WHERE id = $1", id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("schedule {id} not found")))?;
    replace_schedule_repos(pool, id, &[(repo_id, true)]).await
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT id, repo_id, name, schedule_type AS \"schedule_type: ScheduleType\", \
         cron_expression, enabled, canary_enabled, vm_snapshot_enabled, database_backup_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, include_patterns_raw, \
         file_change_patterns_raw, ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, pre_backup_commands AS \
         \"pre_backup_commands: HookCommands\", post_backup_commands AS \"post_backup_commands: \
         HookCommands\", hook_timeout_seconds, missed_backup_threshold, \
         catch_up_min_lead_minutes, execution_mode AS \"execution_mode: ExecutionMode\", \
         on_failure AS \"on_failure: OnFailure\", owner_id, visibility AS \"visibility: \
         Visibility\", wake_override AS \"wake_override: ScheduleWakeOverride\", \
         consecutive_failures, auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\", (SELECT COUNT(*) FROM schedule_targets stc WHERE stc.schedule_id \
         = schedules.id AND stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM \
         schedule_repos src WHERE src.schedule_id = schedules.id AND src.catch_up_pending_for IS \
         NOT NULL) AS \"catch_up_pending_count!\" FROM schedules WHERE repo_id = $1",
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Finds the schedule (of the given type) that targets `hostname` and `repo_id`.
/// Used to attribute a completion reported by the agent (which only carries a
/// repo id, not a schedule id) back to the schedule that most likely triggered
/// it. If multiple schedules of the same type target the same host/repo pair,
/// an arbitrary one is returned.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_for_hostname_repo(
    pool: &PgPool,
    hostname: &str,
    repo_id: i64,
    schedule_type: ScheduleType,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type AS \"schedule_type: ScheduleType\", \
         s.cron_expression, s.enabled, s.canary_enabled, s.vm_snapshot_enabled, \
         s.database_backup_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.include_patterns_raw, s.file_change_patterns_raw, s.ignore_global_excludes, \
         s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, s.keep_yearly, \
         s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", s.post_backup_commands AS \"post_backup_commands: HookCommands\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.catch_up_min_lead_minutes, \
         s.execution_mode AS \"execution_mode: ExecutionMode\", s.on_failure AS \"on_failure: \
         OnFailure\", s.owner_id, s.visibility AS \"visibility: Visibility\", s.wake_override AS \
         \"wake_override: ScheduleWakeOverride\", s.consecutive_failures, \
         s.auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \"target_hostnames!\", (SELECT \
         COUNT(*) FROM schedule_targets stc WHERE stc.schedule_id = s.id AND \
         stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM schedule_repos src WHERE \
         src.schedule_id = s.id AND src.catch_up_pending_for IS NOT NULL) AS \
         \"catch_up_pending_count!\" FROM schedules s JOIN schedule_targets st ON st.schedule_id \
         = s.id JOIN agents m ON st.agent_id = m.id WHERE m.hostname = $1 AND s.repo_id = $2 AND \
         s.schedule_type = $3 LIMIT 1",
        hostname,
        repo_id,
        schedule_type.to_string(),
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every schedule that writes into `repo_id`, whether as its primary target or
/// as one of the further copies it also writes.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type AS \"schedule_type: ScheduleType\", \
         s.cron_expression, s.enabled, s.canary_enabled, s.vm_snapshot_enabled, \
         s.database_backup_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.include_patterns_raw, s.file_change_patterns_raw, s.ignore_global_excludes, \
         s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, s.keep_yearly, \
         s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", s.post_backup_commands AS \"post_backup_commands: HookCommands\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.catch_up_min_lead_minutes, \
         s.execution_mode AS \"execution_mode: ExecutionMode\", s.on_failure AS \"on_failure: \
         OnFailure\", s.owner_id, s.visibility AS \"visibility: Visibility\", s.wake_override AS \
         \"wake_override: ScheduleWakeOverride\", s.consecutive_failures, \
         s.auto_disabled_agent_unreachable, COALESCE(ARRAY(SELECT a.hostname FROM \
         schedule_targets st JOIN agents a ON a.id = st.agent_id WHERE st.schedule_id = s.id \
         ORDER BY st.execution_order, a.hostname), ARRAY[]::TEXT[]) AS \"target_hostnames!\", \
         (SELECT COUNT(*) FROM schedule_targets stc WHERE stc.schedule_id = s.id AND \
         stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM schedule_repos src WHERE \
         src.schedule_id = s.id AND src.catch_up_pending_for IS NOT NULL) AS \
         \"catch_up_pending_count!\" FROM schedules s WHERE EXISTS (SELECT 1 FROM schedule_repos \
         sr WHERE sr.schedule_id = s.id AND sr.repo_id = $1) ORDER BY s.id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_schedule(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM schedules WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("schedule {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type AS \"schedule_type: ScheduleType\", \
         s.cron_expression, s.enabled, s.canary_enabled, s.vm_snapshot_enabled, \
         s.database_backup_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.include_patterns_raw, s.file_change_patterns_raw, s.ignore_global_excludes, \
         s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, s.keep_yearly, \
         s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands AS \"pre_backup_commands: \
         HookCommands\", s.post_backup_commands AS \"post_backup_commands: HookCommands\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.catch_up_min_lead_minutes, \
         s.execution_mode AS \"execution_mode: ExecutionMode\", s.on_failure AS \"on_failure: \
         OnFailure\", s.owner_id, s.visibility AS \"visibility: Visibility\", s.wake_override AS \
         \"wake_override: ScheduleWakeOverride\", s.consecutive_failures, \
         s.auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \"target_hostnames!\", (SELECT \
         COUNT(*) FROM schedule_targets stc WHERE stc.schedule_id = s.id AND \
         stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM schedule_repos src WHERE \
         src.schedule_id = s.id AND src.catch_up_pending_for IS NOT NULL) AS \
         \"catch_up_pending_count!\" FROM schedules s JOIN schedule_targets st ON st.schedule_id \
         = s.id WHERE st.agent_id = $1 ORDER by s.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A schedule that is due to run, joined with its target agent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DueScheduleRow {
    /// Schedule ID.
    pub schedule_id: i64,
    /// Schedule display name.
    pub schedule_name: String,
    /// Repository ID.
    pub repo_id: i64,
    /// Target agent ID.
    pub agent_id: i64,
    /// Target agent hostname.
    pub hostname: String,
    /// Schedule type.
    pub schedule_type: ScheduleType,
    /// Cron expression.
    pub cron_expression: String,
    /// On-failure behaviour.
    pub on_failure: OnFailure,
    /// Execution order among targets.
    pub execution_order: i32,
    /// Whether a failure writing to this repository fails the whole run. A
    /// best-effort target only warns and never stops the remaining targets.
    pub required: bool,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled.
    pub missed_backup_threshold: i32,
    /// Whether the target agent is marked as not always online. Decides what
    /// its being unreachable means: an expected skip that is caught up on
    /// reconnect, or - when it is not marked - a failed backup.
    pub agent_intermittent: bool,
    /// Whether this schedule wakes the hosts it needs, overriding what those
    /// hosts default to - parsed into
    /// [`shared::types::ScheduleWakeOverride`] by the scheduler.
    pub wake_override: ScheduleWakeOverride,
    /// The occurrence this tick is running: the `next_run_at` that came due. Recorded
    /// as the target's pending catch-up when the host turns out to be unreachable.
    pub due_at: DateTime<Utc>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_due_schedules(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<DueScheduleRow>, ApiError> {
    sqlx::query_as!(
        DueScheduleRow,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, sr.repo_id, st.agent_id, \
         a.hostname, s.schedule_type AS \"schedule_type: ScheduleType\", s.cron_expression, \
         s.on_failure AS \"on_failure: OnFailure\", st.execution_order, sr.required, \
         s.missed_backup_threshold, am.intermittent AS agent_intermittent, s.wake_override AS \
         \"wake_override: ScheduleWakeOverride\", s.next_run_at AS \"due_at!\" FROM schedules s \
         JOIN schedule_repos sr ON sr.schedule_id = s.id JOIN repos r ON r.id = sr.repo_id JOIN \
         schedule_targets st ON st.schedule_id = s.id JOIN agents a ON a.id = st.agent_id JOIN \
         machines am ON am.id = a.machine_id WHERE s.enabled = true AND r.enabled = true AND \
         a.is_hidden = false AND s.next_run_at IS NOT NULL AND s.next_run_at <= $1 ORDER BY s.id, \
         st.execution_order, sr.execution_order",
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_schedule_triggered(
    pool: &PgPool,
    schedule_id: i64,
    now: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET last_run_at = $2, next_run_at = $3 WHERE id = $1",
        schedule_id,
        now,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Advances `schedule_id`'s `last_run_at` to `now` and `next_run_at` to the
/// next cron occurrence from it - the bookkeeping a successful dispatch
/// performs, whether it came from a scheduled tick, a manual "Run now", or a
/// caught-up run. The single implementation both `scheduler::mark_schedule_triggered_once`
/// and `run_dispatch`'s manual/catch-up dispatch call, so the two paths can't
/// drift out of sync with each other.
///
/// Returns whether the write actually landed, so a caller that only wants to
/// count a run as "triggered" once this bookkeeping truly succeeded can gate
/// that decision on it, rather than assuming success: an invalid cron
/// expression or a transient database error must not be silently treated as
/// "done".
pub async fn advance_schedule_run(
    pool: &PgPool,
    schedule_id: i64,
    cron_expression: &str,
    tz: chrono_tz::Tz,
    now: DateTime<Utc>,
) -> bool {
    let next = match shared::schedule::calculate_next_run(cron_expression, now, tz) {
        Ok(next) => next,
        Err(e) => {
            tracing::error!(
                schedule_id,
                cron = %cron_expression,
                error = %e,
                "invalid cron expression, not advancing schedule run"
            );
            return false;
        }
    };
    match mark_schedule_triggered(pool, schedule_id, now, next).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(schedule_id, error = %e, "failed to advance schedule run");
            false
        }
    }
}

/// Resets a schedule's consecutive-failure count - once a tick completes having
/// recorded no failure for any of its targets, or as soon as a run first reaches
/// a host while none of the schedule's other hosts is unreachable (see
/// `run_dispatch::reset_missed_streak_if_every_target_is_back`). Deliberately
/// *not* folded into [`mark_schedule_triggered`], which runs on each individual
/// target's success: a multi-target schedule can have one target succeed while
/// another fails in the same tick, and that success must not erase the other
/// target's failure count - see the call sites in `scheduler.rs`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reset_schedule_consecutive_failures(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET consecutive_failures = 0, failure_streak_pure_connectivity = true \
         WHERE id = $1",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Outcome of [`record_schedule_failure`]: the schedule's updated consecutive-failure
/// count, and whether this call was the one that crossed the auto-disable threshold.
pub struct ScheduleFailureOutcome {
    /// The schedule's consecutive-failure count after this call.
    pub consecutive_failures: i32,
    /// Whether this call disabled the schedule (crossed `max_consecutive_failures`).
    pub auto_disabled: bool,
    /// The schedule's `auto_disabled_agent_unreachable` value after this call - only
    /// ever true if the *whole* failure streak was pure connectivity, which can differ
    /// from this call's own `agent_unreachable` argument when an earlier local/data
    /// failure in the same streak already marked it impure. Callers reporting *why* a
    /// schedule was disabled (e.g. a log line or system event) must derive the reason
    /// from this field, not from their own `agent_unreachable` argument, or the
    /// message can claim "agent unreachable" for a call that actually left
    /// `auto_disabled_agent_unreachable` false in the database.
    pub auto_disabled_agent_unreachable: bool,
}

/// Records one failed attempt to reach a schedule's agent (config push or trigger
/// send). Advances `next_run_at` to the next scheduled occurrence - same as a
/// successful trigger - so the scheduler backs off to the normal cron cadence
/// instead of retrying every tick, and disables the schedule once
/// `consecutive_failures` reaches `max_consecutive_failures`, so an agent that stays
/// offline indefinitely doesn't generate failures forever.
///
/// `agent_unreachable` distinguishes *why* this attempt failed: only a connectivity
/// failure (the agent itself unreachable) marks `auto_disabled_agent_unreachable` and
/// records `agent_id` as `auto_disabled_by_agent_id`, so the reconnect handler (see
/// [`list_auto_disabled_schedule_ids_for_agent`]/[`reenable_specific_schedules`])
/// re-enables the schedule once every one of its targets reconnects. A local/data
/// failure (e.g. config assembly, a corrupted encrypted passphrase) still counts
/// toward `consecutive_failures` and still disables
/// the schedule at the threshold, but deliberately leaves that bookkeeping untouched:
/// the agent reconnecting over the websocket says nothing about whether the underlying
/// data problem was fixed, so it must not silently self-heal a disable that was never
/// about connectivity - a human has to fix it and re-enable the schedule.
///
/// `failure_streak_pure_connectivity` tracks this across the *whole* streak, not just
/// this one call: it's only ever true if every failure since the last reset was
/// `agent_unreachable`, so a streak with even one local/data failure in it can never
/// mark `auto_disabled_agent_unreachable` - even if the specific call that happens to
/// cross the threshold is itself a connectivity failure.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn record_schedule_failure(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    next_run_at: DateTime<Utc>,
    max_consecutive_failures: i32,
    agent_unreachable: bool,
) -> Result<ScheduleFailureOutcome, ApiError> {
    let row = sqlx::query!(
        "UPDATE schedules SET consecutive_failures = consecutive_failures + 1, next_run_at = $2, \
         enabled = enabled AND consecutive_failures + 1 < $3, failure_streak_pure_connectivity = \
         failure_streak_pure_connectivity AND $5, auto_disabled_agent_unreachable = CASE WHEN \
         (failure_streak_pure_connectivity AND $5) AND consecutive_failures + 1 >= $3 THEN true \
         ELSE auto_disabled_agent_unreachable END, auto_disabled_by_agent_id = CASE WHEN \
         (failure_streak_pure_connectivity AND $5) AND consecutive_failures + 1 >= $3 THEN $4 \
         ELSE auto_disabled_by_agent_id END WHERE id = $1 RETURNING consecutive_failures, \
         enabled, auto_disabled_agent_unreachable",
        schedule_id,
        next_run_at,
        max_consecutive_failures,
        agent_id,
        agent_unreachable,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(ScheduleFailureOutcome {
        consecutive_failures: row.consecutive_failures,
        // Derived from the threshold check itself, not from `enabled`'s final value:
        // if a concurrent human/quota write disabled this schedule (and reset
        // consecutive_failures) between it being selected as due and this write
        // landing, `enabled` is already false for an unrelated reason and staying
        // false here doesn't mean *this* call crossed the threshold.
        auto_disabled: row.consecutive_failures >= max_consecutive_failures,
        auto_disabled_agent_unreachable: row.auto_disabled_agent_unreachable,
    })
}

/// Re-enables every schedule that the scheduler had auto-disabled after repeated
/// unreachable-agent failures *from this specific `agent_id`* (never a schedule a
/// human or quota enforcement disabled), resetting its failure count and making it due
/// again immediately - matches only on `auto_disabled_by_agent_id`, an atomic
/// single-step primitive kept as a lower-level building block and exercised directly
/// by tests.
///
/// Not used by the production reconnect path (see
/// `ws::handler::connection::reenable_system_disabled_schedules_on_reconnect`) - a multi-target
/// schedule's `auto_disabled_by_agent_id` only ever names whichever target happened to
/// be first-recorded on the disabling tick, not every target that contributed to the
/// streak, so gating solely on that one credited agent's reconnect (as this function
/// does) can leave a schedule with an unreachable *other* target disabled forever. The
/// production path instead reconsiders on any target's reconnect and only actually
/// re-enables once every target is independently confirmed connected - see
/// [`list_auto_disabled_schedule_ids_for_agent`] and [`reenable_specific_schedules`].
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reenable_system_disabled_schedules_for_agent(
    pool: &PgPool,
    agent_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE schedules SET enabled = true, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true, next_run_at = $2 WHERE \
         auto_disabled_agent_unreachable = true AND auto_disabled_by_agent_id = $1 RETURNING id",
        agent_id,
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Schedules currently auto-disabled where `agent_id` is one of the schedule's
/// targets - the candidate set for [`reenable_specific_schedules`]. Split out as a
/// read so a caller (the reconnect handler) can filter the candidates before
/// committing to the write.
///
/// Deliberately matches on *any* target of the schedule, not just the one recorded in
/// `auto_disabled_by_agent_id`: for a multi-target schedule, that column only ever
/// names whichever target happened to be first-recorded on the disabling tick, not
/// every target that contributed to the streak (`consecutive_failures` is
/// schedule-wide). Gating solely on that one credited agent's reconnect would mean a
/// schedule could stay disabled forever if *that* agent never reconnects again after
/// the disable (e.g. it already recovered and stayed connected while a different
/// target was the one still down) - even once every target is actually back.
/// Reconsidering on any target's reconnect is safe because
/// [`reenable_specific_schedules`]'s caller only ever includes a schedule here once it
/// has independently verified every one of its targets is currently connected.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_auto_disabled_schedule_ids_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT s.id FROM schedules s JOIN schedule_targets st ON st.schedule_id = s.id WHERE \
         s.auto_disabled_agent_unreachable = true AND st.agent_id = $1",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Re-enables exactly the given `schedule_ids` (a caller-filtered subset of
/// [`list_auto_disabled_schedule_ids_for_agent`]'s result), the same way
/// [`reenable_system_disabled_schedules_for_agent`] does. Matches only on
/// `auto_disabled_agent_unreachable = true`, not on which agent is recorded in
/// `auto_disabled_by_agent_id` - the reconnecting agent that produced `schedule_ids`
/// may not be the one originally credited with the disable, see
/// [`list_auto_disabled_schedule_ids_for_agent`] - so a schedule whose state changed
/// between the two calls (e.g. a human disabled it for an unrelated reason in between)
/// is safely skipped rather than blindly overwritten.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reenable_specific_schedules(
    pool: &PgPool,
    schedule_ids: &[i64],
    now: DateTime<Utc>,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE schedules SET enabled = true, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true, next_run_at = $2 WHERE id = ANY($1) AND \
         auto_disabled_agent_unreachable = true RETURNING id",
        schedule_ids,
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_next_run_at(
    pool: &PgPool,
    schedule_id: i64,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET next_run_at = $2 WHERE id = $1",
        schedule_id,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_schedule_enabled(
    pool: &PgPool,
    schedule_id: i64,
    enabled: bool,
) -> Result<(), ApiError> {
    // Clears the agent-unreachable failure-tracking columns whenever something other
    // than record_schedule_failure/reenable_system_disabled_schedules_for_agent writes
    // `enabled` directly - otherwise auto_disabled_agent_unreachable can outlive the
    // auto-disable it was set for (e.g. a human re-enables the schedule, then quota
    // enforcement disables it again for an unrelated reason: without this, the stale
    // `true` flag would make a later agent reconnect silently lift the quota block).
    sqlx::query!(
        "UPDATE schedules SET enabled = $2, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true WHERE id = $1",
        schedule_id,
        enabled,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Resets a schedule's connectivity-failure bookkeeping (`consecutive_failures`,
/// `auto_disabled_agent_unreachable`, `auto_disabled_by_agent_id`) when retargeting
/// drops the specific agent a failure streak is attributable to - never merely
/// because *some* target changed, which would wrongly forgive a still-targeted,
/// still-broken agent just because an unrelated sibling target was added or
/// removed. Two cases, based on whether `auto_disabled_by_agent_id` has attribution
/// recorded yet (it's only ever set once the schedule fully auto-disables):
/// - **Already auto-disabled**: resets only if `auto_disabled_by_agent_id` itself is
///   no longer in the new target list. Without this, retargeting away from the
///   causing agent - the realistic way an admin fixes this - would leave
///   `auto_disabled_by_agent_id` pointing at an agent that isn't a target anymore, so
///   [`reenable_system_disabled_schedules_for_agent`] could never match it again and
///   the schedule would stay disabled forever despite the admin's fix.
/// - **Not yet disabled** (a partial streak, e.g. `consecutive_failures = 2` against
///   threshold 3): there's no per-agent attribution to check yet, so this only
///   resets if *every* previously-targeted agent is gone from the new list - the one
///   case where whichever agent(s) were actually accruing the streak are
///   unambiguously no longer targeted. If even one old target remains, the streak
///   might still belong to it, so it's left untouched.
///
/// Deliberately a no-op either way when the streak contains a local/config failure
/// (`failure_streak_pure_connectivity = false`): that state must only ever be
/// cleared by a human fixing the actual cause and re-enabling the schedule
/// themselves, never implicitly by a retarget.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reset_schedule_failure_tracking_if_target_dropped(
    pool: &PgPool,
    schedule_id: i64,
    old_agent_ids: &[i64],
    new_agent_ids: &[i64],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET auto_disabled_agent_unreachable = false, auto_disabled_by_agent_id \
         = NULL, consecutive_failures = 0 WHERE id = $1 AND failure_streak_pure_connectivity = \
         true AND ((auto_disabled_by_agent_id IS NOT NULL AND NOT (auto_disabled_by_agent_id = \
         ANY($2))) OR (auto_disabled_by_agent_id IS NULL AND NOT ($3::bigint[] && $2::bigint[])))",
        schedule_id,
        new_agent_ids,
        old_agent_ids,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// IDs of every schedule belonging to a repo whose `ssh_host` matches, used to enforce a
/// `server_quotas` `block_backups` action across all repos sharing that host.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedule_ids_for_ssh_host(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<Vec<i64>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT s.id FROM schedules s JOIN repos r ON r.id = s.repo_id JOIN repo_hosts h ON h.id \
         = r.repo_host_id WHERE h.ssh_host = $1",
        ssh_host,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_schedule_by_id(pool: &PgPool, id: i64) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT id, repo_id, name, schedule_type AS \"schedule_type: ScheduleType\", \
         cron_expression, enabled, canary_enabled, vm_snapshot_enabled, database_backup_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, include_patterns_raw, \
         file_change_patterns_raw, ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, pre_backup_commands AS \
         \"pre_backup_commands: HookCommands\", post_backup_commands AS \"post_backup_commands: \
         HookCommands\", hook_timeout_seconds, missed_backup_threshold, \
         catch_up_min_lead_minutes, execution_mode AS \"execution_mode: ExecutionMode\", \
         on_failure AS \"on_failure: OnFailure\", owner_id, visibility AS \"visibility: \
         Visibility\", wake_override AS \"wake_override: ScheduleWakeOverride\", \
         consecutive_failures, auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\", (SELECT COUNT(*) FROM schedule_targets stc WHERE stc.schedule_id \
         = schedules.id AND stc.catch_up_pending_for IS NOT NULL) + (SELECT COUNT(*) FROM \
         schedule_repos src WHERE src.schedule_id = schedules.id AND src.catch_up_pending_for IS \
         NOT NULL) AS \"catch_up_pending_count!\" FROM schedules WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

/// Reads a schedule's display name (falling back to `default_name` when unset) and its next
/// scheduled run time in a single round trip, for a backup-completion notification -- both
/// come from the same `schedules` row, so there is no reason to query it twice.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the schedule doesn't exist, or [`ApiError::Database`] if
/// the query fails.
pub async fn get_schedule_name_and_next_run_at(
    pool: &PgPool,
    schedule_id: i64,
    default_name: &str,
) -> Result<(String, Option<DateTime<Utc>>), ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
        next_run_at: Option<DateTime<Utc>>,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT name, next_run_at FROM schedules WHERE id = $1",
        schedule_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {schedule_id} not found")),
        other => ApiError::Database(other),
    })?;

    let name = if row.name.trim().is_empty() {
        default_name.to_owned()
    } else {
        row.name
    };
    Ok((name, row.next_run_at))
}

/// Batched form of [`get_schedule_targets_for_run`] for callers that need target hostnames
/// for many schedules at once (e.g. projecting calendar events for every schedule in a
/// fleet) -- one round trip instead of one query per schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_hostnames_by_schedule(
    pool: &PgPool,
    schedule_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<String>>, ApiError> {
    struct Row {
        schedule_id: i64,
        hostname: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT st.schedule_id, a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id \
         = a.id WHERE st.schedule_id = ANY($1) AND a.is_hidden = false ORDER BY st.schedule_id, \
         st.execution_order",
        schedule_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut by_schedule: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    for row in rows {
        by_schedule
            .entry(row.schedule_id)
            .or_default()
            .push(row.hostname);
    }
    Ok(by_schedule)
}

/// Batched form of [`get_schedule_target_hostnames_by_schedule`] returning agent IDs
/// instead of hostnames, for callers that need to key live-connection state (which is
/// tracked per agent ID, not per hostname, since a hostname can be shared by more than
/// one agent).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_agent_ids_by_schedule(
    pool: &PgPool,
    schedule_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<i64>>, ApiError> {
    struct Row {
        schedule_id: i64,
        agent_id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT st.schedule_id, st.agent_id FROM schedule_targets st JOIN agents a ON a.id = \
         st.agent_id WHERE st.schedule_id = ANY($1) AND a.is_hidden = false ORDER BY \
         st.schedule_id, st.execution_order",
        schedule_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut by_schedule: std::collections::HashMap<i64, Vec<i64>> =
        std::collections::HashMap::new();
    for row in rows {
        by_schedule
            .entry(row.schedule_id)
            .or_default()
            .push(row.agent_id);
    }
    Ok(by_schedule)
}

/// A target agent for a schedule run.
#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRunTarget {
    /// Agent ID.
    pub agent_id: i64,
    /// Agent hostname.
    pub hostname: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_targets_for_run(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleRunTarget>, ApiError> {
    sqlx::query_as!(
        ScheduleRunTarget,
        "SELECT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id WHERE st.schedule_id = $1 AND a.is_hidden = false ORDER BY \
         st.execution_order",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
    targets: &[(i64, i32)],
) -> Result<(), ApiError> {
    for (agent_id, execution_order) in targets {
        sqlx::query!(
            "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, \
             $2, $3)",
            schedule_id,
            *agent_id,
            *execution_order,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_schedule_targets(pool: &PgPool, schedule_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM schedule_targets WHERE schedule_id = $1",
        schedule_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleTargetRow>, ApiError> {
    sqlx::query_as!(
        ScheduleTargetRow,
        "SELECT agent_id, execution_order, catch_up_pending_for FROM schedule_targets WHERE \
         schedule_id = $1 ORDER BY execution_order",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
