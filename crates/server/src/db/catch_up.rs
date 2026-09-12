// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::ApiError;

/// One target of one schedule that has a run waiting to be caught up, together
/// with everything the reconnect handler needs to decide whether it may still
/// run - see `crate::catch_up`.
#[derive(Debug, Clone)]
pub struct CatchUpCandidate {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// Schedule display name, for the log line and system event.
    pub schedule_name: String,
    /// Agent that missed the occurrence.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    /// Schedule type, as stored - parsed at the call site.
    pub schedule_type: String,
    /// The schedule's cron expression, needed to advance `next_run_at` past
    /// this catch-up run the same way a scheduled tick does.
    pub cron_expression: String,
    /// The occurrence that was missed.
    pub pending_for: DateTime<Utc>,
    /// When the schedule next runs on its own, which the lead-time floor is
    /// measured against. Never `None` in practice for an enabled schedule, but
    /// the column is nullable, and a schedule with no computed next run has no
    /// regular run for a catch-up to collide with.
    pub next_run_at: Option<DateTime<Utc>>,
    /// The schedule's configured floor: how much time must be left before
    /// `next_run_at` for the catch-up to still be worth running.
    pub min_lead_minutes: i32,
}

/// The repositories a catch-up run for `schedule_id` writes to, in write
/// order, skipping any whose repository is disabled.
///
/// A catch-up has to cover the same targets the tick it stands in for would
/// have: resolving only the schedule's denormalised primary would quietly let
/// every secondary copy fall further behind on every miss.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_enabled_catch_up_repos(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT sr.repo_id FROM schedule_repos sr JOIN repos r ON r.id = sr.repo_id WHERE \
         sr.schedule_id = $1 AND r.enabled = true ORDER BY sr.execution_order, sr.repo_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records that `agent_id` missed `due_at` for `schedule_id`, so the run can be
/// caught up when that host reconnects.
///
/// Deliberately an overwrite rather than an accumulation: a host that misses
/// thirty-five occurrences ends up with the most recent one pending, and
/// therefore exactly one catch-up run - the "missed runs never stack" rule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    due_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = $3 WHERE schedule_id = $1 AND \
         agent_id = $2",
        schedule_id,
        agent_id,
        due_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Every run this agent has pending, restricted to schedules that could actually
/// run one right now: catch-up still enabled, schedule and repository enabled,
/// and the host not hidden - the same gates
/// [`super::list_due_schedules`](crate::db::list_due_schedules) applies to a
/// regular tick.
///
/// A pending marker on a schedule that fails those gates is not returned here and
/// is cleared by [`clear_catch_up_pending`] all the same, so a stale miss can
/// never resurface days later when the schedule is re-enabled for unrelated
/// reasons.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_catch_up_candidates_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<CatchUpCandidate>, ApiError> {
    sqlx::query_as!(
        CatchUpCandidate,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, st.agent_id, a.hostname, \
         s.schedule_type, s.cron_expression, st.catch_up_pending_for AS \"pending_for!\", \
         s.next_run_at, s.catch_up_min_lead_minutes AS min_lead_minutes FROM schedule_targets st \
         JOIN schedules s ON s.id = st.schedule_id JOIN agents a ON a.id = st.agent_id WHERE \
         st.agent_id = $1 AND st.catch_up_pending_for IS NOT NULL AND s.catch_up_missed_runs = \
         true AND s.enabled = true AND a.is_hidden = false AND EXISTS (SELECT 1 FROM \
         schedule_repos sr JOIN repos r ON r.id = sr.repo_id WHERE sr.schedule_id = s.id AND \
         r.enabled = true) ORDER BY s.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Clears every pending marker this agent carries, whether or not the run it
/// represents is about to happen. The decision is made once, at reconnect: a miss
/// that is skipped (too close to the next run, catch-up switched off since,
/// schedule disabled) is dropped rather than carried forward, since the next
/// scheduled run supersedes it anyway.
///
/// Returns the number of markers cleared.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_catch_up_pending(pool: &PgPool, agent_id: i64) -> Result<u64, ApiError> {
    let cleared = sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = NULL WHERE agent_id = $1 AND \
         catch_up_pending_for IS NOT NULL",
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(cleared)
}

/// Drops every pending marker on one schedule, used when its catch-up setting is
/// switched off: a miss recorded while the feature was on must not run days later
/// because somebody briefly re-enabled it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_catch_up_pending_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = NULL WHERE schedule_id = $1 AND \
         catch_up_pending_for IS NOT NULL",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
