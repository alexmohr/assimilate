// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Backup reports, from pending to finished, their archives, canary results
//! and retention.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::{AcknowledgedFilter, BackupStatus, ReportStatus};
use sqlx::PgPool;

use crate::error::ApiError;

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn insert_canary_result(
    pool: &PgPool,
    schedule_id: i64,
    success: bool,
    canary_filename: &str,
    error_message: Option<&str>,
    archive_name: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO canary_results (schedule_id, success, canary_filename, error_message, \
         archive_name) VALUES ($1, $2, $3, $4, $5)",
        schedule_id,
        success,
        canary_filename,
        error_message,
        archive_name,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// A row from the `canary_results` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CanaryResultRow {
    /// Unique identifier.
    pub id: i64,
    /// Schedule ID this result belongs to.
    pub schedule_id: Option<i64>,
    /// When the canary was verified.
    pub verified_at: DateTime<Utc>,
    /// Whether the canary check succeeded.
    pub success: bool,
    /// Name of the canary file that was checked.
    pub canary_filename: Option<String>,
    /// Error message if the check failed.
    pub error_message: Option<String>,
    /// Archive name created by the canary run.
    pub archive_name: Option<String>,
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_latest_canary_result(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Option<CanaryResultRow>, ApiError> {
    let row = sqlx::query_as!(
        CanaryResultRow,
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT 1",
        schedule_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_canary_results(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<CanaryResultRow>, ApiError> {
    let rows = sqlx::query_as!(
        CanaryResultRow,
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT \
         $2",
        schedule_id,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// A row from the `backup_reports` table with joined repo/schedule names.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ReportRow {
    /// Unique identifier.
    pub id: i64,
    /// Agent that ran the backup.
    pub agent_id: i64,
    /// Repository that was backed up.
    pub repo_id: i64,
    /// Repository display name.
    pub repo_name: String,
    /// Schedule ID that triggered the backup, if any.
    pub schedule_id: Option<i64>,
    /// Schedule display name, if any.
    pub schedule_name: Option<String>,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status (e.g. "success", "failed", "warning").
    pub status: ReportStatus,
    /// Total original size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Warning messages, if any.
    pub warnings: Vec<String>,
    /// Borg version used.
    pub borg_version: Option<String>,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Borg command that was executed.
    pub borg_command: Option<String>,
    /// Correlates to `backup_run_events.run_id` for this run's
    /// power-management timeline, when one was recorded.
    pub run_id: Option<String>,
}

/// Storage statistics grouped by agent and repo.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct StorageStatRow {
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// Total original size in bytes.
    pub total_original_size: i64,
    /// Total compressed size in bytes.
    pub total_compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub total_deduplicated_size: i64,
    /// Number of backup reports.
    pub report_count: i64,
}

/// An activity feed entry representing a backup run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ActivityRow {
    /// Report ID.
    pub id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status.
    pub status: ReportStatus,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Repository ID.
    pub repo_id: Option<i64>,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Schedule ID, if any.
    #[serde(default)]
    pub schedule_id: Option<i64>,
    /// Schedule display name, if any.
    #[serde(default)]
    pub schedule_name: Option<String>,
    /// Run ID for tracking multi-step backups.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Whether a human has acknowledged this run's warning/failure.
    #[serde(default)]
    pub acknowledged: bool,
}

/// Health summary for a schedule-agent-repo combination.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HealthRow {
    /// Repository ID.
    pub repo_id: i64,
    /// Schedule ID.
    pub schedule_id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// Status of the most recent run of any kind, including one still in
    /// flight (in which case this is `None` - see `get_health_summary`).
    pub last_status: Option<ReportStatus>,
    /// When the last *completed* backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Outcome of the last *completed* backup (`last_backup_at`'s run).
    pub last_backup_status: Option<ReportStatus>,
    /// When the most recent backup that produced an archive (success or
    /// warning) finished - older than `last_backup_at` when the latest
    /// completed run failed.
    pub last_success_at: Option<DateTime<Utc>>,
    /// Error message from the last failure.
    pub last_error_message: Option<String>,
    /// Schedule cron expression.
    pub cron_expression: Option<String>,
    /// Whether the schedule is enabled.
    pub schedule_enabled: Option<bool>,
    /// How many consecutive scheduled runs this schedule has missed since its
    /// last success.
    pub consecutive_missed_backups: i32,
    /// The schedule's configured missed-backup threshold.
    pub missed_backup_threshold: i32,
}

/// Parameters for inserting or upserting a backup report.
#[derive(Clone)]
pub struct InsertReportParams {
    /// Agent that ran the backup.
    pub agent_id: i64,
    /// Repository that was backed up.
    pub repo_id: i64,
    /// Schedule ID, if any.
    pub schedule_id: Option<i64>,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status.
    pub status: BackupStatus,
    /// Total original size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Repository-level unique compressed size.
    pub repo_unique_csize: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Warning messages, if any.
    pub warnings: Vec<String>,
    /// Borg version used.
    pub borg_version: Option<String>,
    /// Whether the agent has been matched to a known host.
    pub matched: bool,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Borg command that was executed.
    pub borg_command: Option<String>,
    /// Run ID for tracking multi-step backups.
    pub run_id: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_pending(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    run_id: &str,
    triggered_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, run_id) VALUES ($1, $2, $3, $4, $4, 'pending', $5) ON CONFLICT (repo_id, \
         agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
        agent_id,
        repo_id,
        schedule_id,
        triggered_at,
        run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Settles the `pending` report row a run inserted for one target, for a
/// target that was never dispatched: left `pending`, it would be replayed as a
/// backup the next time the agent reconnects.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn settle_pending_report_for_target(
    pool: &PgPool,
    run_id: &str,
    agent_id: i64,
    repo_id: i64,
    status: ReportStatus,
    reason: &str,
) -> Result<(), ApiError> {
    let status = status.to_string();
    sqlx::query!(
        "UPDATE backup_reports SET status = $4, finished_at = NOW(), error_message = $5 WHERE \
         run_id = $1 AND agent_id = $2 AND repo_id = $3 AND status = 'pending'",
        run_id,
        agent_id,
        repo_id,
        status,
        reason,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_started(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    started_at: DateTime<Utc>,
    borg_command: Option<&str>,
    run_id: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(rid) = run_id {
        sqlx::query!(
            "UPDATE backup_reports SET started_at = $1, status = 'started', borg_command = $2 \
             WHERE run_id = $3 AND agent_id = $4 AND status = 'pending'",
            started_at,
            borg_command,
            rid,
            agent_id,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    } else {
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, borg_command) VALUES ($1, $2, $3, $4, $4, 'started', $5) ON CONFLICT \
             (repo_id, agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
            agent_id,
            repo_id,
            schedule_id,
            started_at,
            borg_command,
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
pub async fn cancel_backup_report(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW(), \
         cancellation_acknowledged = false WHERE agent_id = $1 AND repo_id = $2 AND status IN \
         ('pending', 'started')",
        agent_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn cancel_all_active_backups(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW(), \
         cancellation_acknowledged = false WHERE status IN ('pending', 'started')",
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_cancellation(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET cancellation_acknowledged = true WHERE agent_id = $1 AND \
         repo_id = $2 AND status = 'cancelled'",
        agent_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn fail_other_started_backups(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    current_run_id: Option<&str>,
    hostname: &str,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "UPDATE backup_reports SET status = 'failed', finished_at = NOW(), error_message = $1 \
         WHERE agent_id = $2 AND repo_id = $3 AND status IN ('pending', 'started') AND ($4::text \
         IS NULL OR run_id IS DISTINCT FROM $4)",
        format!("Agent '{hostname}' restarted; backup abandoned"),
        agent_id,
        repo_id,
        current_run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Marks every in-flight (`pending`/`started`) backup report for an agent as
/// abandoned, across all repos - called when the agent's connection is
/// replaced by a new one (see [`crate::ws::registry::AgentRegistry::register`]),
/// since a reconnect means the previous session, and anything it was in the
/// middle of, is gone for good regardless of which repo it targeted. Returns
/// the distinct repo IDs that had a row updated, so the caller can wake up
/// anything still waiting on those operations via the completion bus instead
/// of leaving it blocked on a session that will never report back.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn fail_started_backups_for_agent_reconnect(
    pool: &PgPool,
    agent_id: i64,
    hostname: &str,
) -> Result<Vec<i64>, ApiError> {
    let mut repo_ids: Vec<i64> = sqlx::query_scalar!(
        "UPDATE backup_reports SET status = 'failed', finished_at = NOW(), error_message = $1 \
         WHERE agent_id = $2 AND status IN ('pending', 'started') RETURNING repo_id",
        format!("Agent '{hostname}' reconnected; previous backup abandoned"),
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    repo_ids.sort_unstable();
    repo_ids.dedup();
    Ok(repo_ids)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_report(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    if let Some(ref run_id) = params.run_id {
        update_backup_report_by_run_id(pool, params, run_id).await
    } else if params.archive_name.is_some() {
        upsert_backup_report_with_archive_name(pool, params).await
    } else {
        upsert_backup_report_without_archive_name(pool, params).await
    }
}

async fn update_backup_report_by_run_id(
    pool: &PgPool,
    params: &InsertReportParams,
    run_id: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET schedule_id = COALESCE($1, schedule_id), finished_at = $2, \
         status = $3, original_size = $4, compressed_size = $5, deduplicated_size = $6, \
         repo_unique_csize = $7, files_processed = $8, duration_secs = $9, error_message = $10, \
         warnings = $11, borg_version = $12, matched = $13, archive_name = $14, borg_command = \
         COALESCE($15, borg_command), started_at = $16 WHERE run_id = $17 AND agent_id = $18 AND \
         status IN ('pending', 'started')",
        params.schedule_id,
        params.finished_at,
        &params.status.to_string(),
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
        params.started_at,
        run_id,
        params.agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

async fn upsert_backup_report_with_archive_name(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    let status_str = params.status.to_string();
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14, $15, $16, $17, $18) ON CONFLICT (repo_id, agent_id, started_at, archive_name) \
         WHERE archive_name IS NOT NULL DO UPDATE SET schedule_id = \
         COALESCE(EXCLUDED.schedule_id, backup_reports.schedule_id), finished_at = \
         EXCLUDED.finished_at, status = EXCLUDED.status, original_size = EXCLUDED.original_size, \
         compressed_size = EXCLUDED.compressed_size, deduplicated_size = \
         EXCLUDED.deduplicated_size, repo_unique_csize = EXCLUDED.repo_unique_csize, \
         files_processed = EXCLUDED.files_processed, duration_secs = EXCLUDED.duration_secs, \
         error_message = EXCLUDED.error_message, warnings = EXCLUDED.warnings, borg_version = \
         EXCLUDED.borg_version, matched = EXCLUDED.matched, archive_name = EXCLUDED.archive_name, \
         borg_command = COALESCE(EXCLUDED.borg_command, backup_reports.borg_command)",
        params.agent_id,
        params.repo_id,
        params.schedule_id,
        params.started_at,
        params.finished_at,
        &status_str,
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

async fn upsert_backup_report_without_archive_name(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    let status_str = params.status.to_string();
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14, $15, $16, $17, $18) ON CONFLICT (repo_id, agent_id, started_at) WHERE \
         archive_name IS NULL DO UPDATE SET schedule_id = COALESCE(EXCLUDED.schedule_id, \
         backup_reports.schedule_id), finished_at = EXCLUDED.finished_at, status = \
         EXCLUDED.status, original_size = EXCLUDED.original_size, compressed_size = \
         EXCLUDED.compressed_size, deduplicated_size = EXCLUDED.deduplicated_size, \
         repo_unique_csize = EXCLUDED.repo_unique_csize, files_processed = \
         EXCLUDED.files_processed, duration_secs = EXCLUDED.duration_secs, error_message = \
         EXCLUDED.error_message, warnings = EXCLUDED.warnings, borg_version = \
         EXCLUDED.borg_version, matched = EXCLUDED.matched, archive_name = EXCLUDED.archive_name, \
         borg_command = COALESCE(EXCLUDED.borg_command, backup_reports.borg_command)",
        params.agent_id,
        params.repo_id,
        params.schedule_id,
        params.started_at,
        params.finished_at,
        &status_str,
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn bulk_insert_backup_reports(
    pool: &PgPool,
    params: &[InsertReportParams],
) -> Result<u64, ApiError> {
    if params.is_empty() {
        return Ok(0);
    }

    let mut agent_ids = Vec::with_capacity(params.len());
    let mut repo_ids = Vec::with_capacity(params.len());
    let mut started_ats = Vec::with_capacity(params.len());
    let mut finished_ats = Vec::with_capacity(params.len());
    let mut statuses: Vec<String> = Vec::with_capacity(params.len());
    let mut original_sizes = Vec::with_capacity(params.len());
    let mut compressed_sizes = Vec::with_capacity(params.len());
    let mut deduplicated_sizes = Vec::with_capacity(params.len());
    let mut repo_unique_csizes = Vec::with_capacity(params.len());
    let mut files_processed_v = Vec::with_capacity(params.len());
    let mut duration_secs_v = Vec::with_capacity(params.len());
    let mut error_messages: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut borg_versions: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut matcheds = Vec::with_capacity(params.len());
    let mut archive_names: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut borg_commands: Vec<Option<&str>> = Vec::with_capacity(params.len());

    for p in params {
        agent_ids.push(p.agent_id);
        repo_ids.push(p.repo_id);
        started_ats.push(p.started_at);
        finished_ats.push(p.finished_at);
        statuses.push(p.status.to_string());
        original_sizes.push(p.original_size);
        compressed_sizes.push(p.compressed_size);
        deduplicated_sizes.push(p.deduplicated_size);
        repo_unique_csizes.push(p.repo_unique_csize);
        files_processed_v.push(p.files_processed);
        duration_secs_v.push(p.duration_secs);
        error_messages.push(p.error_message.as_deref());
        borg_versions.push(p.borg_version.as_deref());
        matcheds.push(p.matched);
        archive_names.push(p.archive_name.as_deref());
        borg_commands.push(p.borg_command.as_deref());
    }

    let status_strs: Vec<&str> = statuses.iter().map(String::as_str).collect();

    let result = sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
         original_size, compressed_size, deduplicated_size, repo_unique_csize, files_processed, \
         duration_secs, error_message, warnings, borg_version, matched, archive_name, \
         borg_command) SELECT t.agent_id, t.repo_id, t.started_at, t.finished_at, t.status, \
         t.original_size, t.compressed_size, t.deduplicated_size, t.repo_unique_csize, \
         t.files_processed, t.duration_secs, t.error_message, ARRAY[]::text[], t.borg_version, \
         t.matched, t.archive_name, t.borg_command FROM UNNEST($1::bigint[], $2::bigint[], \
         $3::timestamptz[], $4::timestamptz[], $5::text[], $6::bigint[], $7::bigint[], \
         $8::bigint[], $9::bigint[], $10::bigint[], $11::bigint[], $12::text[], $13::text[], \
         $14::bool[], $15::text[], $16::text[]) AS t(agent_id, repo_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, borg_version, matched, archive_name, \
         borg_command) ON CONFLICT (repo_id, agent_id, started_at, archive_name) WHERE \
         archive_name IS NOT NULL DO NOTHING",
        &agent_ids,
        &repo_ids,
        &started_ats,
        &finished_ats,
        &status_strs as &[&str],
        &original_sizes,
        &compressed_sizes,
        &deduplicated_sizes,
        &repo_unique_csizes,
        &files_processed_v,
        &duration_secs_v,
        &error_messages as &[Option<&str>],
        &borg_versions as &[Option<&str>],
        &matcheds,
        &archive_names as &[Option<&str>],
        &borg_commands as &[Option<&str>],
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(result.rows_affected())
}

/// Statistics for a single borg archive.
pub struct ArchiveStats {
    /// Original (uncompressed) size in bytes.
    pub original_size: i64,
    /// Compressed size in bytes.
    pub compressed_size: i64,
    /// Deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Repository-level unique compressed size.
    pub repo_unique_csize: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_backup_report_stats(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    stats: &ArchiveStats,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET original_size = $3, compressed_size = $4, deduplicated_size = \
         $5, files_processed = $6, duration_secs = $7 WHERE repo_id = $1 AND archive_name = $2 \
         AND original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0",
        repo_id,
        archive_name,
        stats.original_size,
        stats.compressed_size,
        stats.deduplicated_size,
        stats.files_processed,
        stats.duration_secs,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if stats.repo_unique_csize > 0 {
        sqlx::query!(
            "UPDATE backup_reports SET repo_unique_csize = $3 WHERE repo_id = $1 AND archive_name \
             = $2 AND repo_unique_csize = 0",
            repo_id,
            archive_name,
            stats.repo_unique_csize,
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
pub async fn list_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
    target: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    if let Some(target_name) = target {
        sqlx::query_as!(
            ReportRow,
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status AS \"status: ReportStatus\", \
             br.original_size, br.compressed_size, br.deduplicated_size, br.files_processed, \
             br.duration_secs, br.error_message, br.warnings, br.borg_version, br.archive_name, \
             br.borg_command, br.run_id FROM backup_reports br JOIN repos r ON r.id = br.repo_id \
             LEFT JOIN schedules s ON s.id = br.schedule_id WHERE br.agent_id = $1 AND r.name = \
             $2 ORDER by br.started_at DESC, br.id DESC LIMIT $3 OFFSET $4",
            agent_id,
            target_name,
            limit,
            offset,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            ReportRow,
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status AS \"status: ReportStatus\", \
             br.original_size, br.compressed_size, br.deduplicated_size, br.files_processed, \
             br.duration_secs, br.error_message, br.warnings, br.borg_version, br.archive_name, \
             br.borg_command, br.run_id FROM backup_reports br JOIN repos r ON r.id = br.repo_id \
             LEFT JOIN schedules s ON s.id = br.schedule_id WHERE br.agent_id = $1 ORDER BY \
             br.started_at DESC, br.id DESC LIMIT $2 OFFSET $3",
            agent_id,
            limit,
            offset,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
    target: Option<&str>,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = if let Some(target_name) = target {
        sqlx::query_as!(
            CountRow,
            "SELECT COUNT(*) as count FROM backup_reports br JOIN repos r ON r.id = br.repo_id \
             WHERE br.agent_id = $1 AND r.name = $2",
            agent_id,
            target_name,
        )
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?
    } else {
        sqlx::query_as!(
            CountRow,
            "SELECT COUNT(*) as count FROM backup_reports WHERE agent_id = $1",
            agent_id,
        )
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?
    };
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    sqlx::query_as!(
        ReportRow,
        "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE WHEN \
         s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS schedule_name, \
         br.started_at, br.finished_at, br.status AS \"status: ReportStatus\", br.original_size, \
         br.compressed_size, br.deduplicated_size, br.files_processed, br.duration_secs, \
         br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command, \
         br.run_id FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s \
         ON s.id = br.schedule_id WHERE br.schedule_id = $1 ORDER BY br.started_at DESC, br.id \
         DESC LIMIT $2 OFFSET $3",
        schedule_id,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_reports_for_schedule(pool: &PgPool, schedule_id: i64) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM backup_reports WHERE schedule_id = $1",
        schedule_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_stats(pool: &PgPool) -> Result<Vec<StorageStatRow>, ApiError> {
    sqlx::query_as!(
        StorageStatRow,
        "SELECT a.hostname, r.name AS target_name, COALESCE(SUM(br.original_size), 0)::INT8 AS \
         \"total_original_size!\", COALESCE(SUM(br.compressed_size), 0)::INT8 AS \
         \"total_compressed_size!\", COALESCE(SUM(br.deduplicated_size), 0)::INT8 AS \
         \"total_deduplicated_size!\", COUNT(br.id)::INT8 AS \"report_count!\" FROM \
         backup_reports br JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id \
         WHERE a.is_hidden = false GROUP BY a.hostname, r.name ORDER BY a.hostname, r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// The filters an activity-feed query narrows on. Both feed queries take the
/// same set, and passing them one by one outgrew a readable argument list.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityFeedFilters<'a> {
    /// Only runs against this repository.
    pub repo_id: Option<i64>,
    /// Only runs from this agent hostname.
    pub hostname: Option<&'a str>,
    /// Only runs of this schedule.
    pub schedule_id: Option<i64>,
    /// Only runs belonging to this run ID.
    pub run_id: Option<&'a str>,
    /// Which acknowledgment state to return.
    pub acknowledged: AcknowledgedFilter,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_activity_feed(
    pool: &PgPool,
    limit: i64,
    filters: ActivityFeedFilters<'_>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status AS \"status: ReportStatus\", br.duration_secs, br.repo_id, br.archive_name, \
         br.error_message, br.schedule_id, s.name AS \"schedule_name?\", br.run_id, \
         br.acknowledged FROM backup_reports br JOIN agents a ON a.id = br.agent_id JOIN repos r \
         ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = br.schedule_id WHERE a.is_hidden = \
         false AND COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND ($1::bigint IS NULL \
         OR br.repo_id = $1) AND ($2::text IS NULL OR a.hostname = $2) AND ($3::bigint IS NULL OR \
         br.schedule_id = $3) AND ($4::text IS NULL OR br.run_id = $4) AND ($5::bool IS NULL OR \
         br.acknowledged = $5) ORDER BY br.started_at DESC LIMIT $6",
        filters.repo_id,
        filters.hostname,
        filters.schedule_id,
        filters.run_id,
        filters.acknowledged.as_sql_predicate(),
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_health_summary(
    pool: &PgPool,
    schedule_id: Option<i64>,
) -> Result<Vec<HealthRow>, ApiError> {
    // Two LATERAL joins per (schedule, agent) row: `latest` is the most recent report
    // regardless of status, which is what tells the UI a backup is currently running
    // (status 'pending'/'started'); `completed` is the most recent *settled* report, whose
    // `finished_at`/`status` are the real "last backup" timestamp/outcome cadence, overdue
    // and coverage-staleness checks need. A single lookup used to serve both, so while a
    // backup was in flight its placeholder `finished_at` (set to the start time) displaced
    // the previous completed backup's timestamp entirely, making a host with an active
    // backup look like it had none. `last_backup_status` (from `completed`) is exposed
    // separately from `last_status` (from `latest`) so a consumer that must not treat a
    // failed run as coverage - see HostsView.vue's mostRecentBackupAt - has the real
    // completed-run outcome to gate on, even while a newer run is in flight and `latest`'s
    // own status can't represent that (pending/started isn't a `BackupStatus`).
    // `succeeded` is the most recent run that actually produced an archive, so a host
    // whose latest completed run failed still reports the backup it does have instead of
    // reading as never backed up. A skipped run (a dependency was away) never started, so it
    // is not a settled backup either: counting it would keep a host that skips every night
    // from ever reading as overdue.
    //
    // `schedule_id` narrows the whole thing to one schedule for a caller that only shows
    // that schedule's hosts. The filter sits on the base `schedules` scan, so the two
    // LATERAL lookups run once per target of that schedule instead of once per
    // (schedule, agent) pair in the installation.
    sqlx::query_as!(
        HealthRow,
        "SELECT r.id AS repo_id, s.id AS schedule_id, a.hostname, r.name AS target_name, \
         latest.status AS \"last_status?: ReportStatus\", completed.finished_at AS \
         \"last_backup_at?\", completed.status AS \"last_backup_status?: ReportStatus\", \
         succeeded.finished_at AS \"last_success_at?\", latest.error_message AS \
         \"last_error_message?\", s.cron_expression, s.enabled AS schedule_enabled, \
         s.consecutive_failures AS consecutive_missed_backups, s.missed_backup_threshold FROM \
         schedules s JOIN schedule_targets st ON st.schedule_id = s.id JOIN agents a ON a.id = \
         st.agent_id JOIN repos r ON r.id = s.repo_id LEFT JOIN LATERAL ( SELECT br.status, \
         br.error_message FROM backup_reports br WHERE br.schedule_id = s.id AND br.agent_id = \
         a.id AND br.repo_id = s.repo_id ORDER BY br.started_at DESC LIMIT 1 ) latest ON true \
         LEFT JOIN LATERAL ( SELECT br.status, br.finished_at FROM backup_reports br WHERE \
         br.schedule_id = s.id AND br.agent_id = a.id AND br.repo_id = s.repo_id AND br.status \
         NOT IN ('pending', 'started', 'skipped') ORDER BY br.started_at DESC LIMIT 1 ) completed \
         ON true LEFT JOIN LATERAL ( SELECT br.finished_at FROM backup_reports br WHERE \
         br.schedule_id = s.id AND br.agent_id = a.id AND br.repo_id = s.repo_id AND br.status IN \
         ('success', 'warning') ORDER BY br.started_at DESC LIMIT 1 ) succeeded ON true WHERE \
         a.is_hidden = false AND ($1::bigint IS NULL OR s.id = $1) ORDER BY a.hostname, r.name",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Prunes old backup-run history by age.
///
/// Reports that carry an `archive_name` represent an actual borg archive and
/// double as the archive list, so they must never be aged out here: imported
/// and synced archives keep their original (often very old) borg `start`
/// timestamp, and their lifecycle is governed by borg plus the sync stale
/// removal, not by the report-retention window. Only run history without an
/// archive (pending/started/failed/cancelled) is pruned.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_backup_reports_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE started_at < $1 AND archive_name IS NULL",
        before,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_backup_reports_with_archive_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE started_at < $1 AND archive_name IS NOT NULL",
        before,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Deletes all failed backup-run history for an agent, on demand.
///
/// A failed run *usually* carries no `archive_name` (no archive was
/// produced), but `run_backup`'s create step can succeed and a later
/// prune/compact/post-backup-hook step can still fail the run overall - so
/// this guards on `archive_name IS NULL` too, matching
/// [`delete_backup_reports_before`]'s age-based equivalent, rather than
/// ever discarding the only report row linking to a retained archive.
/// Unlike that age-based retention, this acts immediately regardless of
/// `failed_report_retention_days`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_failed_backup_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE agent_id = $1 AND status = 'failed' AND archive_name IS \
         NULL",
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Deletes all failed backup-run history for a schedule, on demand. See
/// [`delete_failed_backup_reports_for_agent`] - same rationale, scoped to a
/// schedule instead of an agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_failed_backup_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE schedule_id = $1 AND status = 'failed' AND archive_name \
         IS NULL",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Counts an agent's failed backup-run history, unbounded by the report-list
/// pagination `limit` a page's own display uses. A "clean up failed backups"
/// confirmation must state how many records [`delete_failed_backup_reports_for_agent`]
/// is actually about to remove, not how many happen to fall within the most
/// recently displayed page of reports - so this carries the same
/// `archive_name IS NULL` guard as that delete, or the count would overstate
/// what the delete actually removes.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_backup_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM backup_reports WHERE agent_id = $1 AND status = 'failed' \
         AND archive_name IS NULL",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Counts a schedule's failed backup-run history. See
/// [`count_failed_backup_reports_for_agent`] - same rationale, scoped to a
/// schedule instead of an agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_backup_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM backup_reports WHERE schedule_id = $1 AND status = \
         'failed' AND archive_name IS NULL",
        schedule_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Looks up the repository a backup report belongs to, so the caller can run
/// a repo-scoped permission check before acknowledging it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no report has this id, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_backup_report_repo_id(pool: &PgPool, id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar!("SELECT repo_id FROM backup_reports WHERE id = $1", id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("report id '{id}' not found")))
}

/// Looks up the repository a backup report belongs to, for the acknowledge
/// endpoint specifically - unlike [`get_backup_report_repo_id`], this also
/// rejects a report whose status isn't warning/failed, since a successful
/// run has nothing to review (matching the frontend's own `isAckable` gate,
/// which this backs up rather than duplicates trust in).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no report has this id,
/// [`ApiError::Unprocessable`] if the report's status isn't warning or
/// failed, or [`ApiError::Database`] if the query fails.
pub async fn get_ackable_backup_report_repo_id(pool: &PgPool, id: i64) -> Result<i64, ApiError> {
    let row = sqlx::query!(
        "SELECT repo_id, status FROM backup_reports WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("report id '{id}' not found")))?;

    let ackable = row
        .status
        .parse::<BackupStatus>()
        .is_ok_and(BackupStatus::is_acknowledgeable);
    if !ackable {
        return Err(ApiError::Unprocessable(format!(
            "report {id} cannot be acknowledged: only warning or failed runs can be reviewed"
        )));
    }
    Ok(row.repo_id)
}

/// Sets or clears the acknowledged flag on a backup report. Acknowledging is
/// a shared, all-users action - like the report itself, it isn't scoped to
/// whoever clicked it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_backup_report_acknowledged(
    pool: &PgPool,
    id: i64,
    acknowledged: bool,
) -> Result<(), ApiError> {
    let rows_affected = sqlx::query!(
        "UPDATE backup_reports SET acknowledged = $1 WHERE id = $2",
        acknowledged,
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("backup report {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_archive_names_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names: Vec<String> = sqlx::query_scalar!(
        "SELECT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS NOT NULL",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?
    .into_iter()
    .flatten()
    .collect();
    Ok(names.into_iter().collect())
}

/// Archive names that need a `borg info` run.
///
/// Covers two cases:
/// - All sizes are still zero (archive was imported but never enriched).
/// - `repo_unique_csize` is zero even though other sizes are populated (archive was enriched
///   before `repo_unique_csize` was tracked).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_archive_names_needing_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names: Vec<String> = sqlx::query_scalar!(
        "SELECT DISTINCT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS \
         NOT NULL AND ((original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0) OR \
         repo_unique_csize = 0)",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?
    .into_iter()
    .flatten()
    .collect();
    Ok(names.into_iter().collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_archive_reports_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)",
        repo_id,
        names
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Removes the directory paths among `candidate_ids` that no content index
/// references any more.
///
/// `archive_paths` is shared by every archive of a repository, so removing one
/// archive's index can only orphan the paths that index referenced. Callers
/// collect those as the candidates before deleting the index rows, and only
/// they are checked here, rather than scanning the whole table.
///
/// Returns the number of paths removed.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn gc_orphaned_archive_paths<'e, E>(
    executor: E,
    repo_id: i64,
    candidate_ids: &[i64],
) -> Result<u64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    if candidate_ids.is_empty() {
        return Ok(0);
    }
    sqlx::query!(
        "DELETE FROM archive_paths WHERE repo_id = $1 AND id = ANY($2) AND NOT EXISTS (SELECT 1 \
         FROM archive_dirs WHERE dir_path_id = archive_paths.id)",
        repo_id,
        candidate_ids,
    )
    .execute(executor)
    .await
    .map(|result| result.rows_affected())
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_archive_records_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)",
        repo_id,
        names,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Collect candidate path IDs before the cascade delete removes archive_dirs.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT DISTINCT dir_path_id AS \"dir_path_id!\" FROM archive_dirs WHERE archive_id IN \
         (SELECT id FROM archives WHERE repo_id = $1 AND name = ANY($2))",
        repo_id,
        names,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_dirs, archive_index_jobs, and archive_tags.
    sqlx::query!(
        "DELETE FROM archives WHERE repo_id = $1 AND name = ANY($2)",
        repo_id,
        names,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    gc_orphaned_archive_paths(&mut *tx, repo_id, &candidate_ids).await?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_all_repo_archive_data(pool: &PgPool, repo_id: i64) -> Result<u64, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    // Collect candidate path IDs before the cascade delete removes archive_dirs.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT DISTINCT dir_path_id AS \"dir_path_id!\" FROM archive_dirs WHERE archive_id IN \
         (SELECT id FROM archives WHERE repo_id = $1)",
        repo_id,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Delete all backup_reports for the repo.
    let result = sqlx::query!("DELETE FROM backup_reports WHERE repo_id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_dirs, archive_index_jobs, and archive_tags.
    sqlx::query!("DELETE FROM archives WHERE repo_id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    gc_orphaned_archive_paths(&mut *tx, repo_id, &candidate_ids).await?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}
