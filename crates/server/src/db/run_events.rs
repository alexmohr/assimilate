// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::{RunEventTarget, RunEventType};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `backup_run_events` table -- one step of a run's
/// power-management timeline (reachability check, wake, agent start,
/// shutdown). `target`/`event_type` are stored as `TEXT` (CHECK-constrained
/// at the DB layer, same convention as `schedules.schedule_type` and the
/// other enum-over-TEXT columns) and parsed at the API boundary rather than
/// bound directly to the enum type.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RunEventRow {
    /// Unique identifier.
    pub id: i64,
    /// Correlates to the run's `backup_reports.run_id`.
    pub run_id: String,
    /// Which target pairing (within a possibly multi-target schedule
    /// sharing this `run_id`) this event belongs to.
    pub agent_id: i64,
    /// Which target pairing (within a possibly multi-target schedule
    /// sharing this `run_id`) this event belongs to.
    pub repo_id: i64,
    /// Which host this event happened to, as stored (`"source"` /
    /// `"repository"`).
    pub target: String,
    /// What happened, as stored (e.g. `"wake_sent"`).
    pub event_type: String,
    /// Human-readable description of the event.
    pub message: String,
    /// When the event occurred.
    pub occurred_at: DateTime<Utc>,
}

/// Records one step of a run's power-management timeline.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_run_event(
    pool: &PgPool,
    run_id: &str,
    agent_id: i64,
    repo_id: i64,
    target: RunEventTarget,
    event_type: RunEventType,
    message: &str,
) -> Result<RunEventRow, ApiError> {
    sqlx::query_as!(
        RunEventRow,
        "INSERT INTO backup_run_events (run_id, agent_id, repo_id, target, event_type, message) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, run_id, agent_id, repo_id, target, \
         event_type, message, occurred_at",
        run_id,
        agent_id,
        repo_id,
        target.to_string(),
        event_type.to_string(),
        message,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Lists a run's power-management timeline in chronological order, scoped
/// to one target pairing -- `run_id` alone spans every target of a
/// multi-target schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_run_events(
    pool: &PgPool,
    run_id: &str,
    agent_id: i64,
    repo_id: i64,
) -> Result<Vec<RunEventRow>, ApiError> {
    sqlx::query_as!(
        RunEventRow,
        "SELECT id, run_id, agent_id, repo_id, target, event_type, message, occurred_at FROM \
         backup_run_events WHERE run_id = $1 AND agent_id = $2 AND repo_id = $3 ORDER BY \
         occurred_at, id",
        run_id,
        agent_id,
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Prunes old run events by age, the same way every other historical/event
/// table in this app is bounded (`system_events`, `backup_reports`, ...).
/// `run_id` is deliberately not a foreign key to `backup_reports` (see the
/// migration's own comment -- one `run_id` fans out to several reports), so
/// this table isn't cleaned up as a side effect of report retention and
/// needs its own cutoff, or its rows outlive the reports they describe.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_run_events_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_run_events WHERE occurred_at < $1",
        before
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}
