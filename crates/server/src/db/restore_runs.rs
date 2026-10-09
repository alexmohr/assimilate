// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Restores of archive files onto an agent (the `restore_runs` table).
//!
//! A restore is `pending` until it is handed to its agent, then `running`
//! until the agent reports the outcome. Every transition is a conditional
//! `UPDATE` on the current status, so two paths racing for the same restore
//! (the API and an agent's reconnect, say) cannot both hand it over, and a
//! late answer cannot reopen a restore that is already over.

use chrono::{DateTime, Utc};
use shared::types::{RestoreRun, RestoreRunStatus};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

/// A restore to record, before it is handed to the agent.
#[derive(Debug)]
pub struct NewRestoreRun<'a> {
    /// The agent the files are restored onto.
    pub agent_id: i64,
    /// The repository holding the archive.
    pub repo_id: i64,
    /// The archive the files come from.
    pub archive_name: &'a str,
    /// Paths within the archive; empty restores the whole archive.
    pub paths: &'a [String],
    /// Directory on the agent the files are extracted into.
    pub target_path: &'a str,
    /// Username of whoever started the restore.
    pub requested_by: &'a str,
}

/// What the agent needs to run a restore it is handed.
#[derive(Debug, PartialEq, Eq)]
pub struct ClaimedRestore {
    /// The repository holding the archive.
    pub repo_id: i64,
    /// The archive the files come from.
    pub archive_name: String,
    /// Paths within the archive; empty restores the whole archive.
    pub paths: Vec<String>,
    /// Directory on the agent the files are extracted into.
    pub target_path: String,
}

/// How a restore the agent ran ended.
#[derive(Debug)]
pub struct RestoreOutcome {
    /// Whether the agent extracted the files.
    pub success: bool,
    /// Number of requested paths restored.
    pub files_restored: i64,
    /// Why the restore failed, if it did.
    pub error_message: Option<String>,
}

struct RestoreRunRow {
    id: Uuid,
    agent_id: i64,
    hostname: String,
    repo_id: i64,
    repo_name: String,
    archive_name: String,
    paths: Vec<String>,
    target_path: String,
    status: RestoreRunStatus,
    files_restored: Option<i64>,
    error_message: Option<String>,
    requested_by: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl From<RestoreRunRow> for RestoreRun {
    fn from(row: RestoreRunRow) -> Self {
        Self {
            id: row.id.to_string(),
            agent_id: row.agent_id,
            hostname: row.hostname,
            repo_id: row.repo_id,
            repo_name: row.repo_name,
            archive_name: row.archive_name,
            paths: row.paths,
            target_path: row.target_path,
            status: row.status,
            files_restored: row.files_restored,
            error_message: row.error_message,
            requested_by: row.requested_by,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}

/// Records a restore as `pending` and returns its id.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_restore_run(pool: &PgPool, run: &NewRestoreRun<'_>) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!(
        "INSERT INTO restore_runs (id, agent_id, repo_id, archive_name, paths, target_path, \
         status, requested_by) VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7) RETURNING id",
        Uuid::new_v4(),
        run.agent_id,
        run.repo_id,
        run.archive_name,
        run.paths,
        run.target_path,
        run.requested_by,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Returns the restore with `id`, if there is one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_restore_run(pool: &PgPool, id: Uuid) -> Result<Option<RestoreRun>, ApiError> {
    let row = sqlx::query_as!(
        RestoreRunRow,
        "SELECT r.id, r.agent_id, a.hostname, r.repo_id, p.name AS repo_name, r.archive_name, \
         r.paths, r.target_path, r.status AS \"status: RestoreRunStatus\", r.files_restored, \
         r.error_message, r.requested_by, r.created_at, r.started_at, r.finished_at FROM \
         restore_runs r JOIN agents a ON a.id = r.agent_id JOIN repos p ON p.id = r.repo_id WHERE \
         r.id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.map(RestoreRun::from))
}

/// Lists the most recent restores, newest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_restore_runs(pool: &PgPool, limit: i64) -> Result<Vec<RestoreRun>, ApiError> {
    let rows = sqlx::query_as!(
        RestoreRunRow,
        "SELECT r.id, r.agent_id, a.hostname, r.repo_id, p.name AS repo_name, r.archive_name, \
         r.paths, r.target_path, r.status AS \"status: RestoreRunStatus\", r.files_restored, \
         r.error_message, r.requested_by, r.created_at, r.started_at, r.finished_at FROM \
         restore_runs r JOIN agents a ON a.id = r.agent_id JOIN repos p ON p.id = r.repo_id ORDER \
         BY r.created_at DESC, r.id LIMIT $1",
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows.into_iter().map(RestoreRun::from).collect())
}

/// Ids of `agent_id`'s restores still waiting to be handed to it, oldest
/// first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn pending_restore_run_ids(pool: &PgPool, agent_id: i64) -> Result<Vec<Uuid>, ApiError> {
    sqlx::query_scalar!(
        "SELECT id FROM restore_runs WHERE agent_id = $1 AND status = 'pending' ORDER BY \
         created_at, id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Marks a `pending` restore `running`, handed to the agent process
/// `agent_instance`, and returns what the agent needs to run it. Returns
/// `None` when the restore is not pending (any more), so whoever gets
/// `Some` is the only one to send it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn claim_restore_run(
    pool: &PgPool,
    id: Uuid,
    agent_instance: Option<&str>,
) -> Result<Option<ClaimedRestore>, ApiError> {
    sqlx::query_as!(
        ClaimedRestore,
        "UPDATE restore_runs SET status = 'running', started_at = NOW(), agent_instance = $2 \
         WHERE id = $1 AND status = 'pending' RETURNING repo_id, archive_name, paths, target_path",
        id,
        agent_instance,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Puts a restore that could not be sent after all back to `pending`, so it
/// goes out when the agent next connects.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn return_restore_run_to_pending(pool: &PgPool, id: Uuid) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE restore_runs SET status = 'pending', started_at = NULL, agent_instance = NULL \
         WHERE id = $1 AND status = 'running'",
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records how a `running` restore handed to `agent_id` ended. Returns
/// `false`, changing nothing, when there is no such restore: the id is
/// unknown, belongs to another agent's restore, or the restore is already
/// over.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn finish_restore_run(
    pool: &PgPool,
    id: Uuid,
    agent_id: i64,
    outcome: &RestoreOutcome,
) -> Result<bool, ApiError> {
    let status = if outcome.success {
        RestoreRunStatus::Success
    } else {
        RestoreRunStatus::Failed
    };
    let updated = sqlx::query!(
        "UPDATE restore_runs SET status = $3, files_restored = $4, error_message = $5, \
         finished_at = NOW() WHERE id = $1 AND agent_id = $2 AND status = 'running'",
        id,
        agent_id,
        status.to_string(),
        outcome.files_restored,
        outcome.error_message,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(updated.rows_affected() > 0)
}

/// Fails `agent_id`'s running restores whose answer can no longer arrive,
/// with `message` as the reason, and returns their ids.
///
/// Those are the restores handed to an agent process other than
/// `current_instance`, the one that just connected: the earlier process is
/// gone, and with it the restore and its answer. With `fail_all`, every
/// running restore of the agent is failed instead, for an agent that names
/// no instance and so cannot tell a restart from a reconnect.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn fail_lost_restore_runs(
    pool: &PgPool,
    agent_id: i64,
    current_instance: Option<&str>,
    fail_all: bool,
    message: &str,
) -> Result<Vec<Uuid>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE restore_runs SET status = 'failed', error_message = $4, finished_at = NOW() WHERE \
         agent_id = $1 AND status = 'running' AND ($3 OR agent_instance IS DISTINCT FROM $2) \
         RETURNING id",
        agent_id,
        current_instance,
        fail_all,
        message,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Cancels a restore that has not been handed to the agent yet. Returns
/// `false`, changing nothing, when it is not pending.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn cancel_restore_run(pool: &PgPool, id: Uuid) -> Result<bool, ApiError> {
    let updated = sqlx::query!(
        "UPDATE restore_runs SET status = 'cancelled', finished_at = NOW() WHERE id = $1 AND \
         status = 'pending'",
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(updated.rows_affected() > 0)
}
