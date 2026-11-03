// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Append-only repositories: which logins are append-only, each repository's
//! maintenance schedule and runs, the archives that vanished and the
//! deletions waiting for maintenance.

use chrono::{DateTime, Utc};
use shared::append_only::MaintenanceOutcome;
use sqlx::PgPool;

use crate::error::ApiError;

/// The default maintenance schedule: Sunday 04:30.
pub const DEFAULT_MAINTENANCE_CRON: &str = "30 4 * * 0";

/// An append-only login and what its last check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOnlyLoginRow {
    /// The repository host.
    pub repo_host_id: i64,
    /// The SSH user.
    pub ssh_user: String,
    /// When the keys were installed.
    pub installed_at: DateTime<Utc>,
    /// When both keys were last tried.
    pub checked_at: Option<DateTime<Utc>>,
    /// Whether the agents' key was accepted.
    pub backup_key_ok: Option<bool>,
    /// Whether the maintenance key was accepted.
    pub maintenance_key_ok: Option<bool>,
    /// Whether `authorized_keys` carried the restriction at the last check.
    pub restriction_in_place: Option<bool>,
    /// What the last check reported.
    pub check_error: Option<String>,
}

/// A repository that logs in a given way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRepoRow {
    /// Repository ID.
    pub id: i64,
    /// Repository name.
    pub name: String,
    /// Path on the host.
    pub repo_path: String,
}

/// A repository's maintenance schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceRow {
    /// Repository ID.
    pub repo_id: i64,
    /// Cron expression, in the server's timezone.
    pub cron_expression: String,
    /// Whether to check the repository before writing.
    pub check_before_prune: bool,
    /// When maintenance runs next.
    pub next_run_at: Option<DateTime<Utc>>,
    /// When maintenance last ran.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Set while maintenance waits for an admin.
    pub stopped_at: Option<DateTime<Utc>>,
}

/// A maintenance run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceRunRow {
    /// Run ID.
    pub id: i64,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When it ended.
    pub finished_at: Option<DateTime<Utc>>,
    /// How it ended.
    pub outcome: MaintenanceOutcome,
    /// Archives removed.
    pub archives_removed: i64,
    /// Bytes freed.
    pub bytes_freed: i64,
    /// What happened.
    pub detail: Option<String>,
}

/// An archive deletion waiting for maintenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDeletionRow {
    /// Archive name.
    pub archive_name: String,
    /// When it was queued.
    pub requested_at: DateTime<Utc>,
    /// Who queued it.
    pub requested_by: Option<String>,
}

/// Whether the repository's login (host and SSH user) is append-only.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn is_repo_append_only(pool: &PgPool, repo_id: i64) -> Result<bool, ApiError> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM repos r
               JOIN append_only_logins l
                 ON l.repo_host_id = r.repo_host_id AND l.ssh_user = r.ssh_user
               WHERE r.id = $1
           ) AS "append_only!""#,
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// The IDs of every append-only repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_append_only_repo_ids(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        r#"SELECT r.id AS "id!"
           FROM repos r
           JOIN append_only_logins l
             ON l.repo_host_id = r.repo_host_id AND l.ssh_user = r.ssh_user"#
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// The append-only login row for a login, if there is one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_login(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_user: &str,
) -> Result<Option<AppendOnlyLoginRow>, ApiError> {
    sqlx::query_as!(
        AppendOnlyLoginRow,
        "SELECT repo_host_id, ssh_user, installed_at, checked_at, backup_key_ok, \
         maintenance_key_ok, restriction_in_place, check_error
         FROM append_only_logins WHERE repo_host_id = $1 AND ssh_user = $2",
        repo_host_id,
        ssh_user,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Marks a login append-only, or refreshes when it was installed.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn upsert_login(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_user: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO append_only_logins (repo_host_id, ssh_user) VALUES ($1, $2)
         ON CONFLICT (repo_host_id, ssh_user) DO UPDATE SET installed_at = NOW()",
        repo_host_id,
        ssh_user,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Marks a login as not append-only.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn delete_login(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_user: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM append_only_logins WHERE repo_host_id = $1 AND ssh_user = $2",
        repo_host_id,
        ssh_user,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// What a connection check found.
#[derive(Debug, Clone, Copy)]
pub struct CheckRecord<'a> {
    /// Whether the agents' key was accepted.
    pub backup_key_ok: bool,
    /// Whether the maintenance key was accepted.
    pub maintenance_key_ok: bool,
    /// Whether `authorized_keys` carries the restriction.
    pub restriction_in_place: bool,
    /// What went wrong, if anything.
    pub error: Option<&'a str>,
}

/// Records what the last connection check found.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn record_check(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_user: &str,
    check: CheckRecord<'_>,
) -> Result<DateTime<Utc>, ApiError> {
    sqlx::query_scalar!(
        r#"UPDATE append_only_logins
           SET checked_at = NOW(), backup_key_ok = $3, maintenance_key_ok = $4,
               restriction_in_place = $5, check_error = $6
           WHERE repo_host_id = $1 AND ssh_user = $2
           RETURNING checked_at AS "checked_at!""#,
        repo_host_id,
        ssh_user,
        check.backup_key_ok,
        check.maintenance_key_ok,
        check.restriction_in_place,
        check.error,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every repository that logs in as `ssh_user` on the host, by name.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_login_repos(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_user: &str,
) -> Result<Vec<LoginRepoRow>, ApiError> {
    sqlx::query_as!(
        LoginRepoRow,
        "SELECT id, name, repo_path FROM repos
         WHERE repo_host_id = $1 AND ssh_user = $2 ORDER BY name",
        repo_host_id,
        ssh_user,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Gives the repository a maintenance schedule if it has none. An existing
/// schedule is left as it is.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn ensure_maintenance(
    pool: &PgPool,
    repo_id: i64,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_maintenance (repo_id, cron_expression, next_run_at)
         VALUES ($1, $2, $3) ON CONFLICT (repo_id) DO NOTHING",
        repo_id,
        DEFAULT_MAINTENANCE_CRON,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Removes the repository's maintenance schedule, as when append-only is
/// turned off. Its run history stays.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn delete_maintenance(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM repo_maintenance WHERE repo_id = $1", repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

/// Lets a stopped repository's maintenance run again from `next_run_at`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn resume_maintenance(
    pool: &PgPool,
    repo_id: i64,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repo_maintenance SET stopped_at = NULL, next_run_at = $2 WHERE repo_id = $1",
        repo_id,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A repository's maintenance schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_maintenance(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Option<MaintenanceRow>, ApiError> {
    sqlx::query_as!(
        MaintenanceRow,
        "SELECT repo_id, cron_expression, check_before_prune, next_run_at, last_run_at, stopped_at
         FROM repo_maintenance WHERE repo_id = $1",
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Sets a repository's maintenance schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn update_maintenance_settings(
    pool: &PgPool,
    repo_id: i64,
    cron_expression: &str,
    check_before_prune: bool,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_maintenance (repo_id, cron_expression, check_before_prune, next_run_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (repo_id) DO UPDATE
         SET cron_expression = $2, check_before_prune = $3, next_run_at = $4",
        repo_id,
        cron_expression,
        check_before_prune,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Repositories whose maintenance is due: append-only, not stopped, and past
/// their next run.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_due_maintenance(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<MaintenanceRow>, ApiError> {
    sqlx::query_as!(
        MaintenanceRow,
        "SELECT m.repo_id, m.cron_expression, m.check_before_prune, m.next_run_at,
                m.last_run_at, m.stopped_at
         FROM repo_maintenance m
         JOIN repos r ON r.id = m.repo_id
         JOIN append_only_logins l
           ON l.repo_host_id = r.repo_host_id AND l.ssh_user = r.ssh_user
         WHERE m.stopped_at IS NULL AND m.next_run_at IS NOT NULL AND m.next_run_at <= $1
         ORDER BY m.next_run_at",
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records that maintenance ran and when it runs next. A stopped run keeps
/// `stopped_at` and has no next run until an admin resumes it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn finish_maintenance(
    pool: &PgPool,
    repo_id: i64,
    ran_at: DateTime<Utc>,
    next_run_at: Option<DateTime<Utc>>,
    stopped: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repo_maintenance
         SET last_run_at = $2,
             next_run_at = $3,
             stopped_at = CASE WHEN $4 THEN COALESCE(stopped_at, $2) ELSE NULL END
         WHERE repo_id = $1",
        repo_id,
        ran_at,
        next_run_at,
        stopped,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Starts a maintenance run's record.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn start_run(pool: &PgPool, repo_id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar!(
        "INSERT INTO repo_maintenance_runs (repo_id, outcome) VALUES ($1, 'failed') RETURNING id",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Ends a maintenance run's record.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn finish_run(
    pool: &PgPool,
    run_id: i64,
    outcome: MaintenanceOutcome,
    archives_removed: i64,
    bytes_freed: i64,
    detail: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repo_maintenance_runs
         SET finished_at = NOW(), outcome = $2, archives_removed = $3, bytes_freed = $4,
             detail = $5
         WHERE id = $1",
        run_id,
        outcome.as_ref(),
        archives_removed,
        bytes_freed,
        detail,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// The latest maintenance runs of a repository, newest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails or a stored outcome is
/// not one the server knows.
pub async fn list_recent_runs(
    pool: &PgPool,
    repo_id: i64,
    limit: i64,
) -> Result<Vec<MaintenanceRunRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, started_at, finished_at, outcome, archives_removed, bytes_freed, detail
         FROM repo_maintenance_runs WHERE repo_id = $1
         ORDER BY started_at DESC, id DESC LIMIT $2",
        repo_id,
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    rows.into_iter()
        .map(|row| {
            let outcome = row.outcome.parse().map_err(|_| {
                ApiError::Internal(format!(
                    "maintenance run {} has the unknown outcome {:?}",
                    row.id, row.outcome
                ))
            })?;
            Ok(MaintenanceRunRow {
                id: row.id,
                started_at: row.started_at,
                finished_at: row.finished_at,
                outcome,
                archives_removed: row.archives_removed,
                bytes_freed: row.bytes_freed,
                detail: row.detail,
            })
        })
        .collect()
}

/// Records archives that are gone from borg although nobody deleted them.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn mark_vanished(pool: &PgPool, repo_id: i64, names: &[String]) -> Result<(), ApiError> {
    if names.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        "INSERT INTO repo_vanished_archives (repo_id, archive_name)
         SELECT $1, UNNEST($2::text[]) ON CONFLICT DO NOTHING",
        repo_id,
        names,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Forgets archives that are listed by borg again, for instance after a
/// rollback.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn clear_vanished_present(
    pool: &PgPool,
    repo_id: i64,
    present: &[String],
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM repo_vanished_archives WHERE repo_id = $1 AND archive_name = ANY($2)",
        repo_id,
        present,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Brings the vanished marks in line with what a listing of the repository
/// shows: `gone` are the archives Assimilate has records of that are not
/// listed, `present` everything that is. The one place both the sync and the
/// maintenance run reconcile through, so the order and meaning cannot drift.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn reconcile_vanished(
    pool: &PgPool,
    repo_id: i64,
    gone: &[String],
    present: &[String],
) -> Result<(), ApiError> {
    mark_vanished(pool, repo_id, gone).await?;
    clear_vanished_present(pool, repo_id, present).await?;
    Ok(())
}

/// Archives recorded as vanished, by name.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_vanished(pool: &PgPool, repo_id: i64) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar!(
        "SELECT archive_name FROM repo_vanished_archives WHERE repo_id = $1 ORDER BY archive_name",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Forgets every vanished archive of the repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn clear_all_vanished(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM repo_vanished_archives WHERE repo_id = $1",
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Queues an archive deletion for the next maintenance run. Queuing twice is
/// the same as once.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn queue_deletion(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    requested_by: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_pending_deletions (repo_id, archive_name, requested_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        repo_id,
        archive_name,
        requested_by,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Takes an archive off the deletion queue. Returns whether it was queued.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn cancel_deletion(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM repo_pending_deletions WHERE repo_id = $1 AND archive_name = $2",
        repo_id,
        archive_name,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// The deletions waiting for maintenance, oldest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_pending_deletions(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<PendingDeletionRow>, ApiError> {
    sqlx::query_as!(
        PendingDeletionRow,
        "SELECT archive_name, requested_at, requested_by FROM repo_pending_deletions
         WHERE repo_id = $1 ORDER BY requested_at, archive_name",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every append-only login as (hostname, SSH user), for the SSH layer.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_logins_with_hosts(pool: &PgPool) -> Result<Vec<(String, String)>, ApiError> {
    let rows = sqlx::query!(
        "SELECT h.ssh_host, l.ssh_user FROM append_only_logins l
         JOIN repo_hosts h ON h.id = l.repo_host_id"
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows.into_iter().map(|r| (r.ssh_host, r.ssh_user)).collect())
}

/// One agent's retention on a repository: the keep counts of a schedule that
/// writes there, for the archives that schedule wrote under the agent's
/// hostname.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RetentionRow {
    /// The schedule, whose id is in the names of the archives it writes.
    pub schedule_id: i64,
    /// The agent's hostname, which archive names start with.
    pub hostname: String,
    /// Hourly archives to keep.
    pub keep_hourly: i32,
    /// Daily archives to keep.
    pub keep_daily: i32,
    /// Weekly archives to keep.
    pub keep_weekly: i32,
    /// Monthly archives to keep.
    pub keep_monthly: i32,
    /// Yearly archives to keep.
    pub keep_yearly: i32,
}

/// The retention every enabled backup schedule writing to the repository asks
/// for, once per agent. This is what the agent would have pruned with after
/// each backup, were the repository not append-only.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_retention(pool: &PgPool, repo_id: i64) -> Result<Vec<RetentionRow>, ApiError> {
    sqlx::query_as!(
        RetentionRow,
        "SELECT DISTINCT s.id AS schedule_id, a.hostname, s.keep_hourly, s.keep_daily, \
         s.keep_weekly, s.keep_monthly,
                s.keep_yearly
         FROM schedule_repos sr
         JOIN schedules s ON s.id = sr.schedule_id
         JOIN schedule_targets st ON st.schedule_id = s.id
         JOIN agents a ON a.id = st.agent_id
         WHERE sr.repo_id = $1 AND s.enabled AND s.schedule_type = 'backup'
         ORDER BY a.hostname, s.id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Sets when maintenance runs next, without touching anything else.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn set_next_run(
    pool: &PgPool,
    repo_id: i64,
    next_run_at: Option<DateTime<Utc>>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repo_maintenance SET next_run_at = $2 WHERE repo_id = $1",
        repo_id,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
