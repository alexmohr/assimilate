// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::ApiError;

/// A schedule target (agent + repo) with its latest backup status for the dashboard.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TargetRow {
    /// Schedule primary key.
    pub schedule_id: i64,
    /// Display name (falls back to repo name).
    pub schedule_name: Option<String>,
    /// Cron expression for schedule timing.
    pub cron_expression: String,
    /// Whether the schedule is enabled.
    pub schedule_enabled: bool,
    /// When the schedule last ran.
    pub schedule_last_run_at: Option<DateTime<Utc>>,
    /// When the schedule is next due.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Agent primary key.
    pub agent_id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Repository primary key.
    pub repo_id: i64,
    /// Repository name.
    pub repo_name: String,
    /// Most recent backup report ID for this target.
    pub latest_report_id: Option<i64>,
    /// When the latest backup started.
    pub latest_started_at: Option<DateTime<Utc>>,
    /// When the latest backup finished.
    pub latest_finished_at: Option<DateTime<Utc>>,
    /// Whether the latest backup failed.
    pub latest_failed: Option<bool>,
    /// Whether the latest backup completed with warnings.
    pub latest_warning: Option<bool>,
    /// Whether the latest backup is still running.
    pub latest_started: Option<bool>,
    /// Human-readable message from the latest backup.
    pub latest_message: Option<String>,
    /// When the last successful backup finished.
    pub last_success_at: Option<DateTime<Utc>>,
}

/// An agent with counts of enabled/disabled/successful schedule assignments.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EligibleAgentRow {
    /// Agent primary key.
    pub agent_id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Number of enabled schedule assignments for this agent.
    pub enabled_assignment_count: Option<i64>,
    /// Number of disabled schedule assignments.
    pub disabled_assignment_count: Option<i64>,
    /// Number of enabled assignments that have at least one success.
    pub successful_enabled_assignment_count: Option<i64>,
}

/// A schedule with its next run time and number of target agents.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UpcomingScheduleRow {
    /// Schedule primary key.
    pub schedule_id: i64,
    /// Schedule display name.
    pub schedule_name: Option<String>,
    /// Repository primary key.
    pub repo_id: i64,
    /// Repository name.
    pub repo_name: String,
    /// When the schedule is next due.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Number of target agents assigned to this schedule.
    pub target_count: Option<i64>,
}

/// A repository with its quota thresholds and import state for the dashboard.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepositoryRow {
    /// Repository primary key.
    pub repo_id: i64,
    /// Repository name.
    pub repo_name: String,
    /// Current deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Warn threshold in bytes.
    pub warn_bytes: Option<i64>,
    /// Critical threshold in bytes.
    pub critical_bytes: Option<i64>,
    /// Whether the repository quota is enabled.
    pub quota_enabled: Option<bool>,
    /// Number of enabled schedules for this repo.
    pub enabled_schedule_count: Option<i64>,
    /// Whether an import is in progress.
    pub importing: bool,
    /// Error message from a failed import, if any.
    pub import_error: Option<String>,
    /// When repo stats were last synced.
    pub last_synced_at: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn targets(pool: &PgPool) -> Result<Vec<TargetRow>, ApiError> {
    sqlx::query_as!(
        TargetRow,
        r#"
        SELECT s.id AS schedule_id,
               COALESCE(NULLIF(s.name, ''), r.name) AS schedule_name,
               s.cron_expression,
               s.enabled AS schedule_enabled,
               s.last_run_at AS schedule_last_run_at,
               s.next_run_at,
               c.id AS agent_id,
               c.hostname,
               r.id AS repo_id,
               r.name AS repo_name,
               COALESCE(latest.id, NULL) AS latest_report_id,
               COALESCE(latest.started_at, NULL) AS latest_started_at,
               COALESCE(latest.finished_at, NULL) AS latest_finished_at,
               COALESCE(latest.status = 'failed', NULL) AS latest_failed,
               COALESCE(latest.status = 'warning', NULL) AS latest_warning,
               COALESCE(latest.status = 'started', NULL) AS latest_started,
               CASE WHEN latest.status = 'warning' THEN latest.warnings[1]
                    ELSE latest.error_message END AS latest_message,
               COALESCE(success.finished_at, NULL) AS last_success_at
        FROM schedules s
        JOIN schedule_targets st ON st.schedule_id = s.id
        JOIN agents c ON c.id = st.agent_id
        JOIN repos r ON r.id = s.repo_id
        LEFT JOIN LATERAL (
            SELECT br.id, br.started_at, br.finished_at, br.status, br.error_message, br.warnings
            FROM backup_reports br
            WHERE br.schedule_id = s.id AND br.agent_id = c.id
            ORDER BY br.started_at DESC
            LIMIT 1
        ) latest ON true
        LEFT JOIN LATERAL (
            SELECT br.finished_at
            FROM backup_reports br
            WHERE br.schedule_id = s.id AND br.agent_id = c.id AND br.status = 'success'
            ORDER BY br.finished_at DESC
            LIMIT 1
        ) success ON true
        WHERE c.is_hidden = false
          AND c.agent_token_hash <> 'imported:no-auth'
          AND r.enabled = true
        ORDER BY s.id, c.id
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn eligible_hosts(pool: &PgPool) -> Result<Vec<EligibleAgentRow>, ApiError> {
    sqlx::query_as!(
        EligibleAgentRow,
        r#"
        SELECT c.id AS agent_id,
               c.hostname,
               COUNT(DISTINCT st.schedule_id) FILTER (
                   WHERE s.enabled = true AND r.enabled = true
               ) AS enabled_assignment_count,
               COUNT(DISTINCT st.schedule_id) FILTER (
                   WHERE s.enabled = false OR r.enabled = false
               ) AS disabled_assignment_count,
               COUNT(DISTINCT st.schedule_id) FILTER (
                   WHERE s.enabled = true AND r.enabled = true AND EXISTS (
                       SELECT 1 FROM backup_reports br
                       WHERE br.schedule_id = s.id AND br.agent_id = c.id
                         AND br.status = 'success'
                   )
               ) AS successful_enabled_assignment_count
         FROM agents c
         LEFT JOIN schedule_targets st ON st.agent_id = c.id
         LEFT JOIN schedules s ON s.id = st.schedule_id
         LEFT JOIN repos r ON r.id = s.repo_id
         WHERE c.is_hidden = false
           AND c.agent_token_hash <> 'imported:no-auth'
         GROUP BY c.id, c.hostname
         ORDER BY c.hostname
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upcoming_schedules(pool: &PgPool) -> Result<Vec<UpcomingScheduleRow>, ApiError> {
    sqlx::query_as!(
        UpcomingScheduleRow,
        r#"
        SELECT s.id AS schedule_id,
               COALESCE(NULLIF(s.name, ''), r.name) AS schedule_name,
               r.id AS repo_id,
               r.name AS repo_name,
               s.next_run_at,
               COUNT(DISTINCT c.id) AS target_count
         FROM schedules s
         JOIN repos r ON r.id = s.repo_id
         JOIN schedule_targets st ON st.schedule_id = s.id
         JOIN agents c ON c.id = st.agent_id
         WHERE s.enabled = true
           AND r.enabled = true
           AND s.next_run_at IS NOT NULL
           AND c.is_hidden = false
           AND c.agent_token_hash <> 'imported:no-auth'
         GROUP BY s.id, s.name, r.id, r.name, s.next_run_at
         ORDER BY s.next_run_at, s.id
         LIMIT 8
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn repositories(pool: &PgPool) -> Result<Vec<RepositoryRow>, ApiError> {
    sqlx::query_as!(
        RepositoryRow,
        r#"
        SELECT r.id AS repo_id,
               r.name AS repo_name,
               COALESCE(rs.deduplicated_size, 0)::INT8 AS "deduplicated_size!",
               q.warn_bytes,
               q.critical_bytes,
               COALESCE(q.enabled, false) AS quota_enabled,
               COUNT(DISTINCT s.id) FILTER (WHERE s.enabled = true) AS enabled_schedule_count,
               COALESCE(ris.importing, false) AS "importing!",
               ris.error AS import_error,
               rs.last_synced_at
         FROM repos r
         LEFT JOIN repo_stats rs ON rs.repo_id = r.id
         LEFT JOIN repo_import_state ris ON ris.repo_id = r.id
         LEFT JOIN repo_quotas q ON q.repo_id = r.id
         LEFT JOIN schedules s ON s.repo_id = r.id
         WHERE r.enabled = true
          GROUP BY r.id, r.name, rs.deduplicated_size, rs.last_synced_at,
                   q.warn_bytes, q.critical_bytes, COALESCE(q.enabled, false),
                   ris.importing, ris.error
         ORDER BY rs.deduplicated_size DESC NULLS LAST, r.name
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Database`]: the database query fails
pub async fn dismissed_finding_ids(
    pool: &PgPool,
    user_id: i64,
) -> Result<HashSet<String>, ApiError> {
    let uid =
        i32::try_from(user_id).map_err(|_| ApiError::BadRequest("user_id out of range".into()))?;
    sqlx::query_scalar!(
        "SELECT finding_id FROM dismissed_dashboard_findings WHERE user_id = $1",
        uid,
    )
    .fetch_all(pool)
    .await
    .map(HashSet::from_iter)
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Database`]: the database query fails
pub async fn dismiss_finding(
    pool: &PgPool,
    user_id: i64,
    finding_id: &str,
) -> Result<(), ApiError> {
    let uid =
        i32::try_from(user_id).map_err(|_| ApiError::BadRequest("user_id out of range".into()))?;
    sqlx::query!(
        "INSERT INTO dismissed_dashboard_findings (user_id, finding_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
        uid,
        finding_id,
    )
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Database`]: the database query fails
pub async fn undismiss_finding(
    pool: &PgPool,
    user_id: i64,
    finding_id: &str,
) -> Result<(), ApiError> {
    let uid =
        i32::try_from(user_id).map_err(|_| ApiError::BadRequest("user_id out of range".into()))?;
    sqlx::query!(
        "DELETE FROM dismissed_dashboard_findings WHERE user_id = $1 AND finding_id = $2",
        uid,
        finding_id,
    )
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(ApiError::Database)
}
