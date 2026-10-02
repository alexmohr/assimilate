// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Runs a target skipped because a dependency host was away, waiting to be
//! caught up once it answers - see `crate::dependencies::catch_up`.
//!
//! One marker per (schedule, agent). Unlike the agent and repository markers
//! it is not cleared when its catch-up is dispatched, only handed to that run:
//! the agent's report for the schedule settles it, and a catch-up that finds a
//! dependency away again hands it back. Retries therefore keep the original
//! occurrence, and the give-up window is never pushed forward.

use chrono::{DateTime, Utc};
use shared::types::ScheduleType;
use sqlx::PgPool;

use crate::error::ApiError;

/// One target waiting on a dependency, with everything the poller needs to
/// decide what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCatchUpCandidate {
    /// Schedule whose run was skipped.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    /// What the schedule runs.
    pub schedule_type: ScheduleType,
    /// Its cron expression, needed to advance `next_run_at` past a catch-up.
    pub cron_expression: String,
    /// Agent whose run was skipped.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    /// The dependency that was away.
    pub dependency_host_id: i64,
    /// Its name.
    pub dependency_name: String,
    /// The occurrence that was skipped.
    pub pending_for: DateTime<Utc>,
    /// When the dependency was last asked whether it is back.
    pub last_probe_at: Option<DateTime<Utc>>,
    /// The catch-up run this marker is handed to, while one is in flight.
    pub dispatched_run_id: Option<String>,
    /// The schedule's next regular run.
    pub next_run_at: Option<DateTime<Utc>>,
    /// The schedule's catch-up floor.
    pub min_lead_minutes: i32,
    /// How often the dependency is asked whether it is back.
    pub recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub give_up_minutes: i32,
    /// Whether the dependency is still marked as not always online.
    pub intermittent: bool,
    /// Whether the schedule is enabled.
    pub schedule_enabled: bool,
}

/// Which markers to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyCatchUpFilter {
    /// Every one - the poller's pass.
    All,
    /// Those of one schedule - its Overview.
    Schedule(i64),
    /// Those waiting on one dependency - its Power pane and Check now.
    Dependency(i64),
}

/// Records that `agent_id` skipped `due_at` of `schedule_id` because
/// `dependency_host_id` was away.
///
/// A scheduled miss overwrites an older marker, the same way agent markers
/// do: however many occurrences a dependency misses, at most one catch-up
/// follows, and the wait is measured from the latest of them.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the write fails.
pub async fn mark_dependency_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    dependency_host_id: i64,
    due_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO dependency_catch_ups (schedule_id, agent_id, dependency_host_id, \
         pending_for) VALUES ($1, $2, $3, $4) ON CONFLICT (schedule_id, agent_id) DO UPDATE SET \
         dependency_host_id = EXCLUDED.dependency_host_id, pending_for = EXCLUDED.pending_for, \
         last_probe_at = NULL, dispatched_run_id = NULL",
        schedule_id,
        agent_id,
        dependency_host_id,
        due_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records a skip only when the target is not already waiting on a
/// dependency - for a catch-up run of another kind (the agent came back, the
/// repository came back) that finds a dependency away: the occurrence it
/// stands for is not known here, so an existing marker's is kept and a new
/// one starts at `at`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the write fails.
pub async fn mark_dependency_catch_up_if_absent(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    dependency_host_id: i64,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO dependency_catch_ups (schedule_id, agent_id, dependency_host_id, \
         pending_for) VALUES ($1, $2, $3, $4) ON CONFLICT (schedule_id, agent_id) DO NOTHING",
        schedule_id,
        agent_id,
        dependency_host_id,
        at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Hands a marker to the catch-up run `run_id`, unless another pass already
/// did. Returns whether this call took it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn hand_off_dependency_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    run_id: &str,
) -> Result<bool, ApiError> {
    let taken = sqlx::query!(
        "UPDATE dependency_catch_ups SET dispatched_run_id = $3 WHERE schedule_id = $1 AND \
         agent_id = $2 AND dispatched_run_id IS NULL",
        schedule_id,
        agent_id,
        run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(taken > 0)
}

/// Takes a marker back from the catch-up run `run_id` that could not go ahead,
/// because a dependency was away again or the agent could not be reached, so
/// the poller waits on `dependency_host_id` again, from the original
/// occurrence.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn release_dependency_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    run_id: &str,
    dependency_host_id: Option<i64>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE dependency_catch_ups SET dispatched_run_id = NULL, dependency_host_id = \
         COALESCE($4, dependency_host_id) WHERE schedule_id = $1 AND agent_id = $2 AND \
         dispatched_run_id = $3",
        schedule_id,
        agent_id,
        run_id,
        dependency_host_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Drops a marker - its run went ahead, it was abandoned, or it no longer
/// qualifies. Returns whether this call is the one that dropped it, so two
/// passes racing for it act on it once.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the delete fails.
pub async fn clear_dependency_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<bool, ApiError> {
    let cleared = sqlx::query!(
        "DELETE FROM dependency_catch_ups WHERE schedule_id = $1 AND agent_id = $2",
        schedule_id,
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(cleared > 0)
}

/// Records that a dependency was asked, at `at`, whether it is back - for
/// every marker waiting on it, since the one probe answers all of them.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn record_dependency_catch_up_probe(
    pool: &PgPool,
    dependency_host_id: i64,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE dependency_catch_ups SET last_probe_at = $2 WHERE dependency_host_id = $1",
        dependency_host_id,
        at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Every marker narrowed by `filter`, ordered by dependency so the poller can
/// probe each one once however many targets wait on it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_dependency_catch_up_candidates(
    pool: &PgPool,
    filter: DependencyCatchUpFilter,
) -> Result<Vec<DependencyCatchUpCandidate>, ApiError> {
    let (schedule_id, dependency_host_id) = match filter {
        DependencyCatchUpFilter::All => (None, None),
        DependencyCatchUpFilter::Schedule(id) => (Some(id), None),
        DependencyCatchUpFilter::Dependency(id) => (None, Some(id)),
    };
    sqlx::query_as!(
        DependencyCatchUpCandidate,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, s.schedule_type AS \"schedule_type: \
         ScheduleType\", s.cron_expression, c.agent_id, a.hostname, c.dependency_host_id, d.name \
         AS dependency_name, c.pending_for, c.last_probe_at, c.dispatched_run_id, s.next_run_at, \
         s.catch_up_min_lead_minutes AS min_lead_minutes, d.catch_up_recheck_minutes AS \
         recheck_minutes, d.catch_up_give_up_minutes AS give_up_minutes, d.intermittent, \
         s.enabled AS schedule_enabled FROM dependency_catch_ups c JOIN schedules s ON s.id = \
         c.schedule_id JOIN agents a ON a.id = c.agent_id JOIN dependency_hosts d ON d.id = \
         c.dependency_host_id WHERE ($1::BIGINT IS NULL OR c.schedule_id = $1) AND ($2::BIGINT IS \
         NULL OR c.dependency_host_id = $2) ORDER BY c.dependency_host_id, c.schedule_id, \
         c.agent_id",
        schedule_id,
        dependency_host_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
