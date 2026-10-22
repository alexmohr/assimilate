// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dashboard, trend, storage and calendar figures.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::{
    ExecutionMode, OnFailure, ReportStatus, ScheduleType, ScheduleWakeOverride, Visibility,
};
use sqlx::PgPool;

use super::{ActivityFeedFilters, ActivityRow, HookCommands, ScheduleRow};
use crate::error::ApiError;

/// Dashboard summary aggregated from all tables.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DashboardSummaryRow {
    /// Total non-hidden agents.
    pub total_agents: i64,
    /// Total repositories.
    pub total_repos: i64,
    /// Number of enabled schedules.
    pub active_schedules: i64,
    /// Total number of schedules.
    pub total_schedules: i64,
    /// Total deduplicated storage across all repos.
    pub total_storage_bytes: i64,
    /// When the last successful backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// When the next backup is scheduled.
    pub next_backup_at: Option<DateTime<Utc>>,
    /// Schedule ID of the last backup.
    pub last_backup_schedule_id: Option<i64>,
    /// Repo ID of the last backup.
    pub last_backup_repo_id: Option<i64>,
    /// Archive name of the last backup.
    pub last_backup_archive_name: Option<String>,
    /// Schedule ID of the next backup.
    pub next_backup_schedule_id: Option<i64>,
    /// Successful backups in the last 30 days.
    pub success_30d: i64,
    /// Failed backups in the last 30 days.
    pub failed_30d: i64,
    /// Total backups in the last 30 days.
    pub total_30d: i64,
    /// When the last failure occurred.
    pub last_failure_at: Option<DateTime<Utc>>,
    /// When the last warning occurred.
    pub last_warning_at: Option<DateTime<Utc>>,
    /// Schedule ID of the last failure.
    pub last_failure_schedule_id: Option<i64>,
    /// Schedule ID of the last warning.
    pub last_warning_schedule_id: Option<i64>,
    /// Error message from the last failure.
    pub last_failure_message: Option<String>,
    /// Warning message from the last warning.
    pub last_warning_message: Option<String>,
    /// Repo ID of the last failure.
    pub last_failure_repo_id: Option<i64>,
    /// Repo ID of the last warning.
    pub last_warning_repo_id: Option<i64>,
    /// Repo name of the last failure.
    pub last_failure_repo_name: Option<String>,
    /// Repo name of the last warning.
    pub last_warning_repo_name: Option<String>,
    /// Schedule name (cron expression) of the last failure.
    pub last_failure_schedule_name: Option<String>,
    /// Schedule name (cron expression) of the last warning.
    pub last_warning_schedule_name: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_dashboard_summary(pool: &PgPool) -> Result<DashboardSummaryRow, ApiError> {
    // The last_failure_*/last_warning_* CTEs skip acknowledged reports: once someone has
    // reviewed a warning or failure in the Activity Log, the dashboard's "Last failure" and
    // "Last warning" tiles fall back to the most recent one still awaiting review.
    //
    // Rewritten from ~18 independent correlated "last matching row" subqueries (each a
    // full sort of the filtered backup_reports rows, run on every dashboard load) into a
    // handful of CTEs that each scan/sort once. Every CTE's WHERE clause is copied verbatim
    // from the subquery(s) it replaces -- including the couple of intentional asymmetries
    // in the original (e.g. last_backup_at applies the epoch-sentinel guard but
    // last_backup_repo_id/last_backup_archive_name don't; the *_schedule_id/*_schedule_name
    // fields require a resolvable schedule_id while the sibling *_at/*_message/*_repo_*
    // fields don't) -- so the result set is identical, just computed more cheaply now that
    // `idx_backup_reports_status_finished_at` covers the ORDER BY.
    sqlx::query_as!(
        DashboardSummaryRow,
        "WITH last_success_at AS ( SELECT MAX(finished_at) AS finished_at FROM backup_reports \
         WHERE status = 'success' AND finished_at > '1970-01-01T00:00:00Z' ), last_success_row AS \
         ( SELECT br.repo_id, br.archive_name FROM backup_reports br WHERE br.status = 'success' \
         ORDER BY br.finished_at DESC LIMIT 1 ), last_backup_row AS ( SELECT br.schedule_id FROM \
         backup_reports br WHERE br.schedule_id IS NOT NULL ORDER BY br.finished_at DESC LIMIT 1 \
         ), next_backup_row AS ( SELECT s.id, s.next_run_at FROM schedules s JOIN repos r ON r.id \
         = s.repo_id WHERE s.enabled = true AND r.enabled = true AND s.next_run_at IS NOT NULL \
         AND s.next_run_at > NOW() ORDER BY s.next_run_at LIMIT 1 ), last_failure_general AS ( \
         SELECT br.finished_at, br.error_message, br.repo_id, r.name AS repo_name FROM \
         backup_reports br JOIN repos r ON r.id = br.repo_id WHERE br.status = 'failed' AND \
         br.acknowledged = false AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY \
         br.finished_at DESC LIMIT 1 ), last_failure_scheduled AS ( SELECT br.schedule_id, \
         s.cron_expression AS schedule_name FROM backup_reports br JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.status = 'failed' AND br.acknowledged = false AND br.finished_at \
         > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1 ), last_warning_general AS \
         ( SELECT br.finished_at, br.warnings[1] AS warning_message, br.repo_id, r.name AS \
         repo_name FROM backup_reports br JOIN repos r ON r.id = br.repo_id WHERE br.status = \
         'warning' AND br.acknowledged = false AND br.finished_at > '1970-01-01T00:00:00Z' ORDER \
         BY br.finished_at DESC LIMIT 1 ), last_warning_scheduled AS ( SELECT br.schedule_id, \
         s.cron_expression AS schedule_name FROM backup_reports br JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.status = 'warning' AND br.acknowledged = false AND \
         br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1 ) SELECT \
         (SELECT COUNT(*) FROM agents WHERE is_hidden = false) AS \"total_agents!\", (SELECT \
         COUNT(*) FROM repos) AS \"total_repos!\", (SELECT COUNT(*) FROM schedules WHERE enabled \
         = true) AS \"active_schedules!\", (SELECT COUNT(*) FROM schedules) AS \
         \"total_schedules!\", COALESCE((SELECT SUM(deduplicated_size) FROM repo_stats), 0)::INT8 \
         AS \"total_storage_bytes!\", last_success_at.finished_at AS last_backup_at, \
         next_backup_row.next_run_at AS next_backup_at, last_backup_row.schedule_id AS \
         last_backup_schedule_id, last_success_row.repo_id AS last_backup_repo_id, \
         last_success_row.archive_name AS last_backup_archive_name, next_backup_row.id AS \
         next_backup_schedule_id, (SELECT COUNT(*) FROM backup_reports WHERE status = 'success' \
         AND started_at > NOW() - INTERVAL '30 days') AS \"success_30d!\", (SELECT COUNT(*) FROM \
         backup_reports WHERE status != 'success' AND started_at > NOW() - INTERVAL '30 days') AS \
         \"failed_30d!\", (SELECT COUNT(*) FROM backup_reports WHERE started_at > NOW() - \
         INTERVAL '30 days') AS \"total_30d!\", last_failure_general.finished_at AS \
         last_failure_at, last_warning_general.finished_at AS last_warning_at, \
         last_failure_scheduled.schedule_id AS last_failure_schedule_id, \
         last_warning_scheduled.schedule_id AS last_warning_schedule_id, \
         last_failure_general.error_message AS last_failure_message, \
         last_warning_general.warning_message AS last_warning_message, \
         last_failure_general.repo_id AS last_failure_repo_id, last_warning_general.repo_id AS \
         last_warning_repo_id, last_failure_general.repo_name AS last_failure_repo_name, \
         last_warning_general.repo_name AS last_warning_repo_name, \
         last_failure_scheduled.schedule_name AS last_failure_schedule_name, \
         last_warning_scheduled.schedule_name AS last_warning_schedule_name FROM (SELECT 1) AS \
         one LEFT JOIN last_success_at ON true LEFT JOIN last_success_row ON true LEFT JOIN \
         last_backup_row ON true LEFT JOIN next_backup_row ON true LEFT JOIN last_failure_general \
         ON true LEFT JOIN last_failure_scheduled ON true LEFT JOIN last_warning_general ON true \
         LEFT JOIN last_warning_scheduled ON true",
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Storage breakdown by repository.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageBreakdownRow {
    /// Repository name.
    pub name: String,
    /// Compressed size in bytes.
    pub compressed_size: i64,
    /// Deduplicated size in bytes.
    pub deduplicated_size: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_breakdown(pool: &PgPool) -> Result<Vec<StorageBreakdownRow>, ApiError> {
    sqlx::query_as!(
        StorageBreakdownRow,
        "SELECT r.name, COALESCE(rs.compressed_size, 0)::INT8 AS \"compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0)::INT8 AS \"deduplicated_size!\" FROM repos r LEFT JOIN \
         repo_stats rs ON rs.repo_id = r.id ORDER BY rs.deduplicated_size DESC NULLS LAST",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_activity_feed_days(
    pool: &PgPool,
    days: i64,
    // Caps *runs* per schedule, not rows in the result set overall - a plain
    // global LIMIT would let one frequently-running schedule's reports crowd
    // out every row belonging to a less-frequent one in the ranked window.
    // A run is every report sharing a `run_id` (one per target of a
    // multi-agent schedule), so capping runs rather than rows means a
    // schedule with N targets still gets its last `per_schedule_limit`
    // firings instead of only `per_schedule_limit / N` of them. A report
    // without a `run_id` counts as a run of its own. The `days` window is
    // applied per run too: a run with any report inside it comes back with
    // all of its reports, so one straddling the cutoff is never drawn with
    // only the targets that happen to fall inside. Only a report that passes
    // every other filter (visible agent, repository, host, schedule,
    // acknowledgment) can hold a run inside the window - one the feed would
    // not show must not pull a stale sibling back in.
    per_schedule_limit: Option<i64>,
    filters: ActivityFeedFilters<'_>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT id, hostname, target_name, started_at, finished_at, status AS \"status!: \
         ReportStatus\", duration_secs AS \"duration_secs!\", repo_id, archive_name, \
         error_message, schedule_id, schedule_name AS \"schedule_name?\", run_id, acknowledged AS \
         \"acknowledged!\" FROM ( SELECT *, DENSE_RANK() OVER (PARTITION BY schedule_id ORDER BY \
         run_started_at DESC, run_key) AS run_rank FROM ( SELECT br.id, a.hostname, r.name AS \
         target_name, br.started_at, br.finished_at, br.status, br.duration_secs, br.repo_id, \
         br.archive_name, br.error_message, br.schedule_id, s.name AS schedule_name, br.run_id, \
         br.acknowledged, COALESCE(br.run_id, br.id::text) AS run_key, MIN(br.started_at) OVER \
         (PARTITION BY br.schedule_id, COALESCE(br.run_id, br.id::text)) AS run_started_at FROM \
         backup_reports br JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id \
         LEFT JOIN schedules s ON s.id = br.schedule_id WHERE a.is_hidden = false AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND (br.started_at > NOW() - \
         make_interval(days => $1::int) OR br.run_id IN ( SELECT w.run_id FROM backup_reports w \
         JOIN agents wa ON wa.id = w.agent_id WHERE w.run_id IS NOT NULL AND w.started_at > NOW() \
         - make_interval(days => $1::int) AND wa.is_hidden = false AND COALESCE(wa.display_name, \
         '') NOT ILIKE '%(imported)%' AND ($2::bigint IS NULL OR w.repo_id = $2) AND ($3::text IS \
         NULL OR wa.hostname = $3) AND ($4::bigint IS NULL OR w.schedule_id = $4) AND ($6::bool \
         IS NULL OR w.acknowledged = $6) )) AND ($2::bigint IS NULL OR br.repo_id = $2) AND \
         ($3::text IS NULL OR a.hostname = $3) AND ($4::bigint IS NULL OR br.schedule_id = $4) \
         AND ($5::text IS NULL OR br.run_id = $5) AND ($6::bool IS NULL OR br.acknowledged = $6) \
         ) reports ) ranked WHERE $7::bigint IS NULL OR run_rank <= $7 ORDER BY started_at DESC",
        i32::try_from(days).unwrap_or(14),
        filters.repo_id,
        filters.hostname,
        filters.schedule_id,
        filters.run_id,
        filters.acknowledged.as_sql_predicate(),
        per_schedule_limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A single day's aggregated backup trend data.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TrendRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Average original size.
    pub original_size: i64,
    /// Average compressed size.
    pub compressed_size: i64,
    /// Average deduplicated size.
    pub deduplicated_size: i64,
    /// Average file count.
    pub file_count: i64,
    /// Average duration in seconds.
    pub duration_seconds: i64,
    /// Number of backups on this date.
    pub backup_count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_backup_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<TrendRow>, ApiError> {
    let days = i32::try_from(days).unwrap_or(30);
    if let Some(rid) = repo_id {
        sqlx::query_as!(
            TrendRow,
            "SELECT started_at::date AS \"date!\", COALESCE(AVG(original_size), 0)::INT8 AS \
             \"original_size!\", COALESCE(AVG(compressed_size), 0)::INT8 AS \"compressed_size!\", \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS \"deduplicated_size!\", \
             COALESCE(AVG(files_processed), 0)::INT8 AS \"file_count!\", \
             COALESCE(AVG(duration_secs), 0)::INT8 AS \"duration_seconds!\", COUNT(*)::INT8 AS \
             \"backup_count!\" FROM backup_reports WHERE repo_id = $1 AND started_at > NOW() - \
             make_interval(days => $2) GROUP BY started_at::date ORDER BY 1",
            rid,
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            TrendRow,
            "SELECT started_at::date AS \"date!\", COALESCE(AVG(original_size), 0)::INT8 AS \
             \"original_size!\", COALESCE(AVG(compressed_size), 0)::INT8 AS \"compressed_size!\", \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS \"deduplicated_size!\", \
             COALESCE(AVG(files_processed), 0)::INT8 AS \"file_count!\", \
             COALESCE(AVG(duration_secs), 0)::INT8 AS \"duration_seconds!\", COUNT(*)::INT8 AS \
             \"backup_count!\" FROM backup_reports WHERE started_at > NOW() - make_interval(days \
             => $1) GROUP BY started_at::date ORDER BY 1",
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// A calendar event representing a backup run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CalendarEventRow {
    /// Event date.
    pub date: chrono::NaiveDate,
    /// Event type (e.g. "backup").
    pub event_type: String,
    /// Backup status.
    pub status: String,
    /// Repository name.
    pub repo_name: String,
    /// Agent hostname.
    pub hostname: String,
    /// Event time string (HH:MM).
    pub time: String,
    /// Report ID, if any.
    pub report_id: Option<i64>,
    /// Repository ID, if any.
    pub repo_id: Option<i64>,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Archive name, if any.
    pub archive_name: Option<String>,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Database`]: the database query fails
pub async fn get_calendar_events(
    pool: &PgPool,
    year: i32,
    month: u32,
    repo_id: Option<i64>,
    tz: chrono_tz::Tz,
) -> Result<Vec<CalendarEventRow>, ApiError> {
    let start = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;
    let end = if month == 12 {
        year.checked_add(1)
            .and_then(|y| chrono::NaiveDate::from_ymd_opt(y, 1, 1))
    } else {
        month
            .checked_add(1)
            .and_then(|m| chrono::NaiveDate::from_ymd_opt(year, m, 1))
    }
    .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;

    let tz_name = tz.name();

    if let Some(rid) = repo_id {
        sqlx::query_as!(
            CalendarEventRow,
            "SELECT (br.started_at AT TIME ZONE $4)::date AS \"date!\", 'backup' AS \
             \"event_type!\", CASE WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS \
             \"status!\", r.name AS \"repo_name!\", a.hostname AS \"hostname!\", \
             to_char(br.started_at AT TIME ZONE $4, 'HH24:MI') AS \"time!\", br.id AS \
             \"report_id?\", br.repo_id AS \"repo_id?\", br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $4)::date >= \
             $1 AND (br.started_at AT TIME ZONE $4)::date < $2 AND br.repo_id = $3 ORDER BY \
             br.started_at",
            start,
            end,
            rid,
            tz_name,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            CalendarEventRow,
            "SELECT (br.started_at AT TIME ZONE $3)::date AS \"date!\", 'backup' AS \
             \"event_type!\", CASE WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS \
             \"status!\", r.name AS \"repo_name!\", a.hostname AS \"hostname!\", \
             to_char(br.started_at AT TIME ZONE $3, 'HH24:MI') AS \"time!\", br.id AS \
             \"report_id?\", br.repo_id AS \"repo_id?\", br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $3)::date >= \
             $1 AND (br.started_at AT TIME ZONE $3)::date < $2 ORDER BY br.started_at",
            start,
            end,
            tz_name,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// A single day's storage trend data (cumulative across all repos).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Cumulative original size up to this date.
    pub original_size: i64,
    /// Cumulative compressed size up to this date.
    pub compressed_size: i64,
    /// Latest deduplicated (``repo_unique_csize``) as of this date.
    pub deduplicated_size: Option<i64>,
}

/// A single day's storage trend data, per repository.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendByRepoRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Repository ID.
    pub repo_id: i64,
    /// Repository name.
    pub repo_name: String,
    /// Cumulative original size up to this date.
    pub original_size: i64,
    /// Cumulative compressed size up to this date.
    pub compressed_size: i64,
    /// Latest deduplicated size as of this date.
    pub deduplicated_size: Option<i64>,
}

/// `original_size`/`compressed_size` are the cumulative sum, across every archive taken up to
/// that date, of that archive's (pre-deduplication) size; this mirrors how borg itself defines
/// a repository's total (non-deduplicated) size. `deduplicated_size` is the repository's actual
/// unique compressed size (`repo_unique_csize`) as of the most recent archive on or before that
/// date. Mixing a single archive's per-archive size with the repo-wide deduplicated size would
/// make the deduplicated line exceed the original/compressed lines, which is impossible.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<StorageTrendRow>, ApiError> {
    let days = i32::try_from(days).unwrap_or(30);
    if let Some(rid) = repo_id {
        // Single-pass rewrite: aggregate each day's reports once (`daily`), then derive the
        // cumulative original/compressed totals and the forward-filled latest dedup snapshot
        // via window functions over just the (bounded) `days` series, instead of re-scanning
        // the entire report history with a correlated subquery per displayed day.
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ), daily AS ( SELECT \
             br.started_at::date AS date, SUM(br.original_size) AS day_original, \
             SUM(br.compressed_size) AS day_compressed, (ARRAY_AGG(br.repo_unique_csize ORDER BY \
             br.started_at DESC))[1] AS day_csize FROM backup_reports br WHERE br.repo_id = $2 \
             AND br.status = 'success' GROUP BY br.started_at::date ), joined AS ( SELECT d.date, \
             dl.day_original, dl.day_compressed, dl.day_csize, COUNT(dl.date) OVER (ORDER BY \
             d.date) AS fill_grp FROM days d LEFT JOIN daily dl ON dl.date = d.date ) SELECT date \
             AS \"date!\", COALESCE(SUM(day_original) OVER (ORDER BY date), 0)::INT8 AS \
             \"original_size!\", COALESCE(SUM(day_compressed) OVER (ORDER BY date), 0)::INT8 AS \
             \"compressed_size!\", NULLIF(MAX(day_csize) OVER (PARTITION BY fill_grp), 0)::INT8 \
             AS \"deduplicated_size?\" FROM joined ORDER BY date",
            days,
            rid,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        // Same single-pass approach, but the daily rollup and cumulative window are computed
        // per repo first (`per_repo`) -- matching the original per-repo "latest known dedup
        // size" semantics -- then summed across repos per day. `days LEFT JOIN fleet_by_date`
        // (rather than driving from `days CROSS JOIN repos_list`) keeps the "always emit one
        // row per requested day" behaviour even when no repos have any reports yet.
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( \
             SELECT DISTINCT br.repo_id FROM backup_reports br WHERE br.status = 'success' ), \
             daily AS ( SELECT br.repo_id, br.started_at::date AS date, SUM(br.original_size) AS \
             day_original, SUM(br.compressed_size) AS day_compressed, \
             (ARRAY_AGG(br.repo_unique_csize ORDER BY br.started_at DESC))[1] AS day_csize FROM \
             backup_reports br WHERE br.status = 'success' GROUP BY br.repo_id, \
             br.started_at::date ), joined AS ( SELECT rl.repo_id, d.date, dl.day_original, \
             dl.day_compressed, dl.day_csize, COUNT(dl.date) OVER (PARTITION BY rl.repo_id ORDER \
             BY d.date) AS fill_grp FROM repos_list rl CROSS JOIN days d LEFT JOIN daily dl ON \
             dl.repo_id = rl.repo_id AND dl.date = d.date ), per_repo AS ( SELECT repo_id, date, \
             COALESCE(SUM(day_original) OVER (PARTITION BY repo_id ORDER BY date), 0) AS \
             cum_original, COALESCE(SUM(day_compressed) OVER (PARTITION BY repo_id ORDER BY \
             date), 0) AS cum_compressed, MAX(day_csize) OVER (PARTITION BY repo_id, fill_grp) AS \
             cum_csize FROM joined ), fleet_by_date AS ( SELECT date, SUM(cum_original) AS \
             original_size, SUM(cum_compressed) AS compressed_size, SUM(COALESCE(cum_csize, 0)) \
             AS csize_sum FROM per_repo GROUP BY date ) SELECT d.date AS \"date!\", \
             COALESCE(f.original_size, 0)::INT8 AS \"original_size!\", \
             COALESCE(f.compressed_size, 0)::INT8 AS \"compressed_size!\", \
             NULLIF(COALESCE(f.csize_sum, 0), 0)::INT8 AS \"deduplicated_size?\" FROM days d LEFT \
             JOIN fleet_by_date f ON f.date = d.date ORDER BY d.date",
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// See [`get_storage_trends`] for why `original_size`/`compressed_size` are a cumulative sum
/// over all archives up to that date while `deduplicated_size` is the latest repo-wide
/// `repo_unique_csize` snapshot.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_trends_by_repo(
    pool: &PgPool,
    days: i64,
) -> Result<Vec<StorageTrendByRepoRow>, ApiError> {
    let days_i32 = i32::try_from(days).unwrap_or(30);
    // Same single-pass rewrite as `get_storage_trends`: aggregate each repo's reports per day
    // once (`daily`), then derive per-(repo, day) cumulative totals and the forward-filled
    // latest dedup snapshot via window functions, instead of a correlated subquery per
    // (day, repo) pair that re-scans that repo's entire history each time.
    sqlx::query_as!(
        StorageTrendByRepoRow,
        "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => $1))::date, \
         CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( SELECT DISTINCT r.id \
         AS repo_id, r.name AS repo_name FROM repos r JOIN backup_reports br ON br.repo_id = r.id \
         ), daily AS ( SELECT br.repo_id, br.started_at::date AS date, SUM(br.original_size) AS \
         day_original, SUM(br.compressed_size) AS day_compressed, (ARRAY_AGG(br.repo_unique_csize \
         ORDER BY br.started_at DESC))[1] AS day_csize FROM backup_reports br WHERE br.status = \
         'success' GROUP BY br.repo_id, br.started_at::date ), joined AS ( SELECT rl.repo_id, \
         rl.repo_name, d.date, dl.day_original, dl.day_compressed, dl.day_csize, COUNT(dl.date) \
         OVER (PARTITION BY rl.repo_id ORDER BY d.date) AS fill_grp FROM repos_list rl CROSS JOIN \
         days d LEFT JOIN daily dl ON dl.repo_id = rl.repo_id AND dl.date = d.date ) SELECT date \
         AS \"date!\", repo_id AS \"repo_id!\", repo_name AS \"repo_name!\", \
         COALESCE(SUM(day_original) OVER (PARTITION BY repo_id ORDER BY date), 0)::INT8 AS \
         \"original_size!\", COALESCE(SUM(day_compressed) OVER (PARTITION BY repo_id ORDER BY \
         date), 0)::INT8 AS \"compressed_size!\", NULLIF(MAX(day_csize) OVER (PARTITION BY \
         repo_id, fill_grp), 0)::INT8 AS \"deduplicated_size?\" FROM joined ORDER BY date, \
         repo_name",
        days_i32,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_enabled_schedules_for_calendar(
    pool: &PgPool,
) -> Result<Vec<ScheduleRow>, ApiError> {
    let rows = sqlx::query_as!(
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
         NOT NULL) AS \"catch_up_pending_count!\" FROM schedules WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}
