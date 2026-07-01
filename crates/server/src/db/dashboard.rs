// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::ApiError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TargetRow {
    pub schedule_id: i64,
    pub schedule_name: Option<String>,
    pub cron_expression: String,
    pub schedule_enabled: bool,
    pub schedule_last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub agent_id: i64,
    pub hostname: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub latest_report_id: Option<i64>,
    pub latest_started_at: Option<DateTime<Utc>>,
    pub latest_finished_at: Option<DateTime<Utc>>,
    pub latest_failed: Option<bool>,
    pub latest_warning: Option<bool>,
    pub latest_started: Option<bool>,
    pub latest_message: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EligibleAgentRow {
    pub agent_id: i64,
    pub hostname: String,
    pub enabled_assignment_count: Option<i64>,
    pub disabled_assignment_count: Option<i64>,
    pub successful_enabled_assignment_count: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UpcomingScheduleRow {
    pub schedule_id: i64,
    pub schedule_name: Option<String>,
    pub repo_id: i64,
    pub repo_name: String,
    pub next_run_at: Option<DateTime<Utc>>,
    pub target_count: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepositoryRow {
    pub repo_id: i64,
    pub repo_name: String,
    pub deduplicated_size: i64,
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub quota_enabled: Option<bool>,
    pub enabled_schedule_count: Option<i64>,
    pub importing: bool,
    pub import_error: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

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
               latest.id AS latest_report_id,
               latest.started_at AS latest_started_at,
               latest.finished_at AS latest_finished_at,
               latest.status = 'failed' AS latest_failed,
               latest.status = 'warning' AS latest_warning,
               latest.status = 'started' AS latest_started,
               CASE
                   WHEN latest.status = 'warning' THEN latest.warnings[1]
                   ELSE latest.error_message
               END AS latest_message,
               success.finished_at AS last_success_at
         FROM schedules s
         JOIN schedule_targets st ON st.schedule_id = s.id
         JOIN agents c ON c.id = st.agent_id
         JOIN repos r ON r.id = s.repo_id
         LEFT JOIN LATERAL (
             SELECT br.id, br.started_at, br.finished_at, br.status,
                    br.error_message, br.warnings
             FROM backup_reports br
             WHERE br.schedule_id = s.id AND br.agent_id = c.id
             ORDER BY br.started_at DESC
             LIMIT 1
         ) latest ON true
         LEFT JOIN LATERAL (
             SELECT br.finished_at
             FROM backup_reports br
             WHERE br.schedule_id = s.id AND br.agent_id = c.id
               AND br.status = 'success'
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

pub async fn repositories(pool: &PgPool) -> Result<Vec<RepositoryRow>, ApiError> {
    sqlx::query_as!(
        RepositoryRow,
        r#"
        SELECT r.id AS repo_id,
               r.name AS repo_name,
               r.info_deduplicated_size AS deduplicated_size,
               q.warn_bytes,
               q.critical_bytes,
               q.enabled AS quota_enabled,
               COUNT(DISTINCT s.id) FILTER (WHERE s.enabled = true) AS enabled_schedule_count,
               r.importing,
               r.import_error,
               r.last_synced_at
         FROM repos r
         LEFT JOIN repo_quotas q ON q.repo_id = r.id
         LEFT JOIN schedules s ON s.repo_id = r.id
         WHERE r.enabled = true
         GROUP BY r.id, r.name, r.info_deduplicated_size, q.warn_bytes,
                  q.critical_bytes, q.enabled, r.importing, r.import_error, r.last_synced_at
         ORDER BY r.info_deduplicated_size DESC, r.name
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

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
