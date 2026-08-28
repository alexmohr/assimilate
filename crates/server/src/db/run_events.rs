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
    target: RunEventTarget,
    event_type: RunEventType,
    message: &str,
) -> Result<RunEventRow, ApiError> {
    sqlx::query_as!(
        RunEventRow,
        "INSERT INTO backup_run_events (run_id, target, event_type, message) VALUES ($1, $2, $3, \
         $4) RETURNING id, run_id, target, event_type, message, occurred_at",
        run_id,
        target.to_string(),
        event_type.to_string(),
        message,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Lists a run's power-management timeline in chronological order.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_run_events(pool: &PgPool, run_id: &str) -> Result<Vec<RunEventRow>, ApiError> {
    sqlx::query_as!(
        RunEventRow,
        "SELECT id, run_id, target, event_type, message, occurred_at FROM backup_run_events WHERE \
         run_id = $1 ORDER BY occurred_at, id",
        run_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
