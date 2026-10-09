// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use shared::{responses::RestoreResponse, types::RestoreStatus};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `restores` table, with the hostname of the agent it
/// restores to.
#[derive(Debug, Clone)]
pub struct RestoreRow {
    /// Unique identifier.
    pub id: i64,
    /// Id the server and agent use for this restore over the WebSocket.
    pub request_id: String,
    /// The repository holding the archive.
    pub repo_id: i64,
    /// The agent the files are restored to.
    pub agent_id: i64,
    /// Hostname of that agent.
    pub hostname: String,
    /// The archive the files come from.
    pub archive_name: String,
    /// Paths within the archive. Empty restores the whole archive.
    pub paths: Vec<String>,
    /// Directory on the agent the files are extracted into.
    pub target_path: String,
    /// Username of whoever started the restore.
    pub requested_by: String,
    /// Where the restore stands.
    pub status: RestoreStatus,
    /// Number of paths restored, once it succeeded.
    pub files_restored: Option<i64>,
    /// Why the restore failed, if it did.
    pub error_message: Option<String>,
    /// When the restore was requested.
    pub created_at: DateTime<Utc>,
    /// When the agent started extracting.
    pub started_at: Option<DateTime<Utc>>,
    /// When the restore reached its final state.
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<RestoreRow> for RestoreResponse {
    fn from(row: RestoreRow) -> Self {
        Self {
            id: row.id,
            repo_id: row.repo_id,
            archive_name: row.archive_name,
            paths: row.paths,
            target_path: row.target_path,
            agent_id: row.agent_id,
            hostname: row.hostname,
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

/// What a new restore restores, and where to.
#[derive(Debug)]
pub struct NewRestore<'a> {
    /// Id the server and agent use for this restore over the WebSocket.
    pub request_id: &'a str,
    /// The repository holding the archive.
    pub repo_id: i64,
    /// The agent the files are restored to.
    pub agent_id: i64,
    /// The archive the files come from.
    pub archive_name: &'a str,
    /// Paths within the archive. Empty restores the whole archive.
    pub paths: &'a [String],
    /// Directory on the agent the files are extracted into.
    pub target_path: &'a str,
    /// Username of whoever started the restore.
    pub requested_by: &'a str,
}

/// How a restore ended, as the agent reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The agent extracted the files.
    Succeeded {
        /// Number of paths restored.
        files_restored: i64,
    },
    /// The restore could not be carried out.
    Failed {
        /// Why.
        error_message: String,
    },
}

fn unfinished_statuses() -> Vec<String> {
    RestoreStatus::UNFINISHED
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Records a new restore. It starts out queued; sending it to the agent is
/// what moves it on.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_restore(pool: &PgPool, restore: &NewRestore<'_>) -> Result<i64, ApiError> {
    sqlx::query_scalar!(
        "INSERT INTO restores (request_id, repo_id, agent_id, archive_name, paths, target_path, \
         requested_by, status) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
        restore.request_id,
        restore.repo_id,
        restore.agent_id,
        restore.archive_name,
        restore.paths,
        restore.target_path,
        restore.requested_by,
        RestoreStatus::Queued.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Looks up a restore by id.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such restore, or
/// [`ApiError::Database`] if the database query fails.
pub async fn get_restore(pool: &PgPool, id: i64) -> Result<RestoreRow, ApiError> {
    sqlx::query_as!(
        RestoreRow,
        "SELECT r.id, r.request_id, r.repo_id, r.agent_id, a.hostname, r.archive_name, r.paths, \
         r.target_path, r.requested_by, r.status AS \"status: RestoreStatus\", r.files_restored, \
         r.error_message, r.created_at, r.started_at, r.finished_at FROM restores r JOIN agents a \
         ON a.id = r.agent_id WHERE r.id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("restore {id} not found")))
}

/// The restores an agent still owes an answer for, oldest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_unfinished_restores_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RestoreRow>, ApiError> {
    sqlx::query_as!(
        RestoreRow,
        "SELECT r.id, r.request_id, r.repo_id, r.agent_id, a.hostname, r.archive_name, r.paths, \
         r.target_path, r.requested_by, r.status AS \"status: RestoreStatus\", r.files_restored, \
         r.error_message, r.created_at, r.started_at, r.finished_at FROM restores r JOIN agents a \
         ON a.id = r.agent_id WHERE r.agent_id = $1 AND r.status = ANY($2) ORDER BY r.id",
        agent_id,
        &unfinished_statuses(),
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Marks a queued restore as sent to its agent. Returns `false` when the
/// restore was no longer queued - someone else already sent or cancelled it -
/// so exactly one caller sends it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_restore_dispatched(pool: &PgPool, id: i64) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE restores SET status = $2 WHERE id = $1 AND status = $3",
        id,
        RestoreStatus::Dispatched.to_string(),
        RestoreStatus::Queued.to_string(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Puts a restore that could not be sent back in the queue, unless the
/// agent already answered it in the meantime.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn requeue_restore(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE restores SET status = $2 WHERE id = $1 AND status = $3",
        id,
        RestoreStatus::Queued.to_string(),
        RestoreStatus::Dispatched.to_string(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records that `agent_id` started extracting the restore `request_id`
/// names. Returns the restore's id, or `None` when no unfinished restore of
/// that agent has this request id.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_restore_running(
    pool: &PgPool,
    request_id: &str,
    agent_id: i64,
) -> Result<Option<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE restores SET status = $3, started_at = NOW() WHERE request_id = $1 AND agent_id = \
         $2 AND status = ANY($4) RETURNING id",
        request_id,
        agent_id,
        RestoreStatus::Running.to_string(),
        &unfinished_statuses(),
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records how the restore `request_id` names ended. Returns the restore's
/// id, or `None` when no unfinished restore of `agent_id` has this request
/// id - an answer to a restore that already ended is ignored.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn finish_restore(
    pool: &PgPool,
    request_id: &str,
    agent_id: i64,
    outcome: &RestoreOutcome,
) -> Result<Option<i64>, ApiError> {
    let (status, files_restored, error_message) = match outcome {
        RestoreOutcome::Succeeded { files_restored } => {
            (RestoreStatus::Succeeded, Some(*files_restored), None)
        }
        RestoreOutcome::Failed { error_message } => {
            (RestoreStatus::Failed, None, Some(error_message.as_str()))
        }
    };
    sqlx::query_scalar!(
        "UPDATE restores SET status = $3, files_restored = $4, error_message = $5, finished_at = \
         NOW() WHERE request_id = $1 AND agent_id = $2 AND status = ANY($6) RETURNING id",
        request_id,
        agent_id,
        status.to_string(),
        files_restored,
        error_message,
        &unfinished_statuses(),
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Cancels a restore that has not been sent to its agent yet. Returns
/// `false` when it is no longer queued.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn cancel_queued_restore(pool: &PgPool, id: i64) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE restores SET status = $2, finished_at = NOW() WHERE id = $1 AND status = $3",
        id,
        RestoreStatus::Cancelled.to_string(),
        RestoreStatus::Queued.to_string(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Deletes restores that ended before `before`. Unfinished ones are kept
/// however old they are: an agent may still answer them.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_finished_restores_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM restores WHERE finished_at < $1", before)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}
