// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The rows the Prometheus `/metrics` endpoint is rendered from.
//!
//! Each query leaves a value it does not know as `NULL` rather than folding it
//! into a zero, so the renderer can omit the sample and an `absent()` alert
//! still fires on it.

use chrono::{DateTime, Utc};
use shared::types::{ReportStatus, ScheduleType};
use sqlx::PgPool;

use crate::error::ApiError;

/// One schedule target: a schedule writing one host's sources to one
/// repository.
#[derive(Debug, Clone)]
pub struct ScheduleTargetMetricsRow {
    /// Schedule primary key.
    pub schedule_id: i64,
    /// Schedule display name: its name, else its primary repository's, else
    /// `Schedule #<id>` as the UI shows it. The same on every target row.
    pub schedule_name: String,
    /// What the schedule runs.
    pub schedule_type: ScheduleType,
    /// Whether the schedule is enabled.
    pub schedule_enabled: bool,
    /// Cron expression the schedule runs on.
    pub cron_expression: String,
    /// The target host's hostname.
    pub hostname: String,
    /// The target host's DNS domain, when an admin set one.
    pub domain: Option<String>,
    /// The repository written to.
    pub repo_name: String,
    /// The repository host's address.
    pub repo_host: String,
    /// The status of the target's latest finished run, if it ever finished one.
    pub latest_status: Option<ReportStatus>,
    /// When the target's latest successful run finished, if it ever had one.
    pub last_success_at: Option<DateTime<Utc>>,
}

/// One repository with its statistics and quota.
#[derive(Debug, Clone)]
pub struct RepositoryMetricsRow {
    /// Repository name.
    pub repo_name: String,
    /// The repository host's address.
    pub repo_host: String,
    /// Total size of every archive before compression and deduplication.
    pub original_size: Option<i64>,
    /// Size after compression and deduplication.
    pub deduplicated_size: Option<i64>,
    /// Number of archives.
    pub archive_count: Option<i32>,
    /// When the statistics were last read from borg, if they ever were.
    pub stats_updated_at: Option<DateTime<Utc>>,
    /// Quota warn threshold in bytes, when a quota is enabled and sets one.
    pub quota_warn_bytes: Option<i64>,
    /// Quota critical threshold in bytes, when a quota is enabled and sets one.
    pub quota_critical_bytes: Option<i64>,
}

/// One agent with what it reported about itself.
#[derive(Debug, Clone)]
pub struct AgentMetricsRow {
    /// Agent primary key.
    pub agent_id: i64,
    /// Hostname.
    pub hostname: String,
    /// DNS domain, when an admin set one.
    pub domain: Option<String>,
    /// Agent version it last reported.
    pub agent_version: Option<String>,
    /// Borg version from its most recent report that carried one.
    pub borg_version: Option<String>,
    /// When the agent was last seen.
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// Every schedule target of an enabled repository on a visible, real host.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn schedule_targets(pool: &PgPool) -> Result<Vec<ScheduleTargetMetricsRow>, ApiError> {
    sqlx::query_as!(
        ScheduleTargetMetricsRow,
        r#"
        SELECT s.id AS schedule_id,
               COALESCE(NULLIF(s.name, ''), pr.name, 'Schedule #' || s.id) AS "schedule_name!",
               s.schedule_type AS "schedule_type: ScheduleType",
               s.enabled AS schedule_enabled,
               s.cron_expression,
               a.hostname,
               a.domain,
               r.name AS repo_name,
               rh.ssh_host AS repo_host,
               latest.status AS "latest_status?: ReportStatus",
               success.finished_at AS "last_success_at?"
        FROM schedules s
        JOIN schedule_targets st ON st.schedule_id = s.id
        JOIN agents a ON a.id = st.agent_id
        JOIN schedule_repos sr ON sr.schedule_id = s.id
        JOIN repos r ON r.id = sr.repo_id
        JOIN repo_hosts rh ON rh.id = r.repo_host_id
        LEFT JOIN repos pr ON pr.id = s.repo_id
        LEFT JOIN LATERAL (
            SELECT br.status
            FROM backup_reports br
            WHERE br.schedule_id = s.id AND br.agent_id = a.id AND br.repo_id = r.id
              AND br.status IN ('success', 'warning', 'failed')
            ORDER BY br.finished_at DESC
            LIMIT 1
        ) latest ON true
        LEFT JOIN LATERAL (
            SELECT br.finished_at
            FROM backup_reports br
            WHERE br.schedule_id = s.id AND br.agent_id = a.id AND br.repo_id = r.id
              AND br.status = 'success'
            ORDER BY br.finished_at DESC
            LIMIT 1
        ) success ON true
        WHERE a.is_hidden = false
          AND a.agent_token_hash <> 'imported:no-auth'
          AND r.enabled = true
        ORDER BY s.id, a.id, r.id
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every enabled repository with its statistics and quota.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn repositories(pool: &PgPool) -> Result<Vec<RepositoryMetricsRow>, ApiError> {
    sqlx::query_as!(
        RepositoryMetricsRow,
        r#"
        SELECT r.name AS repo_name,
               rh.ssh_host AS repo_host,
               CASE WHEN rs.updated_at IS NULL THEN NULL ELSE rs.original_size END
                   AS "original_size?",
               CASE WHEN rs.updated_at IS NULL THEN NULL ELSE rs.deduplicated_size END
                   AS "deduplicated_size?",
               CASE WHEN rs.updated_at IS NULL THEN NULL ELSE rs.archive_count END
                   AS "archive_count?",
               rs.updated_at AS "stats_updated_at?",
               CASE WHEN q.enabled THEN q.warn_bytes END AS "quota_warn_bytes?",
               CASE WHEN q.enabled THEN q.critical_bytes END AS "quota_critical_bytes?"
        FROM repos r
        JOIN repo_hosts rh ON rh.id = r.repo_host_id
        LEFT JOIN repo_stats rs ON rs.repo_id = r.id
        LEFT JOIN repo_quotas q ON q.repo_id = r.id
        WHERE r.enabled = true
        ORDER BY r.name
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every visible agent that is a real host rather than an imported placeholder.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn agents(pool: &PgPool) -> Result<Vec<AgentMetricsRow>, ApiError> {
    sqlx::query_as!(
        AgentMetricsRow,
        r#"
        SELECT a.id AS agent_id,
               a.hostname,
               a.domain,
               a.agent_version,
               (
                   SELECT br.borg_version
                   FROM backup_reports br
                   WHERE br.agent_id = a.id AND br.borg_version IS NOT NULL
                     AND br.borg_version <> ''
                   ORDER BY br.started_at DESC
                   LIMIT 1
               ) AS "borg_version?",
               a.last_seen_at
        FROM agents a
        WHERE a.is_hidden = false
          AND a.agent_token_hash <> 'imported:no-auth'
        ORDER BY a.hostname, a.domain
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
