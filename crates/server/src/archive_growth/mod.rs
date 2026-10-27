// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Where an archive's deduplicated size sits.
//!
//! `borg list` reports, per file, the deduplicated compressed bytes no other
//! file in the repository shares (`dcsize`); summed over an archive they are
//! exactly the archive's deduplicated size. Reading them makes borg check
//! every chunk of the archive against the repository's chunk index, which on
//! a big archive takes minutes, so the breakdown is worked out in the
//! background, one archive at a time per repository, and kept in
//! `archive_growth` until it is recomputed.

pub mod tree;

use std::{collections::HashMap, panic::AssertUnwindSafe, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use serde::Deserialize;
use shared::{
    archive_growth::{ArchiveGrowth, ArchiveGrowthResponse, ArchiveGrowthStatus},
    task_registry::TaskRegistry,
};
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};

use crate::{
    AppState, RepoLock,
    api::archives::{LOCK_WAIT_SECS, classify_borg_error, get_repo_env},
    archive_growth::tree::GrowthBuilder,
    archive_index::{archive_id, get_or_create_archive_id, panic_reason},
    borg::Borg,
    error::ApiError,
};

/// How long borg may stay silent. Before its first line it brings its chunk
/// index up to date with the repository, which on a large repository the
/// server has not read in a while takes long.
const LINE_READ_TIMEOUT: Duration = Duration::from_mins(30);

/// One file as `borg list --json-lines --format '{size}{dcsize}'` prints it.
#[derive(Debug, Deserialize)]
struct ListedFile {
    path: String,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    dcsize: i64,
}

#[derive(sqlx::FromRow)]
struct GrowthRow {
    status: ArchiveGrowthStatus,
    result: Option<String>,
    error_message: Option<String>,
    finished_at: Option<DateTime<Utc>>,
}

impl GrowthRow {
    fn into_response(self) -> Result<ArchiveGrowthResponse, ApiError> {
        let growth = self
            .result
            .as_deref()
            .map(serde_json::from_str::<ArchiveGrowth>)
            .transpose()
            .map_err(|e| ApiError::Internal(format!("corrupt archive growth breakdown: {e}")))?;
        Ok(ArchiveGrowthResponse {
            status: self.status,
            growth,
            computed_at: self.finished_at.map(|at| at.to_rfc3339()),
            error: self.error_message,
        })
    }
}

/// The breakdown of `archive_name` as stored, or `None` when none was asked for.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails, or [`ApiError::Internal`]
/// if the stored breakdown cannot be read.
pub async fn read(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Option<ArchiveGrowthResponse>, ApiError> {
    let Some(archive_id) = archive_id(pool, repo_id, archive_name).await? else {
        return Ok(None);
    };
    sqlx::query_as!(
        GrowthRow,
        "SELECT status AS \"status: ArchiveGrowthStatus\", result, error_message, finished_at \
         FROM archive_growth WHERE archive_id = $1",
        archive_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .map(GrowthRow::into_response)
    .transpose()
}

/// The breakdown of `archive_name`, starting to work it out if nobody has.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn ensure(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
) -> Result<ArchiveGrowthResponse, ApiError> {
    let archive_id = get_or_create_archive_id(&state.pool, repo_id, archive_name).await?;
    let claimed = sqlx::query!(
        "INSERT INTO archive_growth (archive_id, status) VALUES ($1, 'pending') ON CONFLICT DO \
         NOTHING",
        archive_id,
    )
    .execute(&state.pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected()
        == 1;
    if claimed {
        spawn(state, repo_id, archive_id, archive_name);
    }
    current(&state.pool, repo_id, archive_name).await
}

/// Works the breakdown of `archive_name` out again, unless that is already
/// under way.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn recompute(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
) -> Result<ArchiveGrowthResponse, ApiError> {
    let archive_id = get_or_create_archive_id(&state.pool, repo_id, archive_name).await?;
    let claimed = sqlx::query!(
        "INSERT INTO archive_growth (archive_id, status) VALUES ($1, 'pending') ON CONFLICT \
         (archive_id) DO UPDATE SET status = 'pending', result = NULL, error_message = NULL, \
         requested_at = NOW(), finished_at = NULL WHERE archive_growth.status IN ('done', \
         'failed')",
        archive_id,
    )
    .execute(&state.pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected()
        == 1;
    if claimed {
        spawn(state, repo_id, archive_id, archive_name);
    }
    current(&state.pool, repo_id, archive_name).await
}

async fn current(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<ArchiveGrowthResponse, ApiError> {
    read(pool, repo_id, archive_name).await?.ok_or_else(|| {
        ApiError::Internal("archive growth row vanished right after it was claimed".to_owned())
    })
}

fn spawn(state: &AppState, repo_id: i64, archive_id: i64, archive_name: &str) {
    let pool = state.pool.clone();
    let encryption_key = state.encryption_key;
    let repo_lock = state.repo_lock.clone();
    let task_registry = state.task_registry.clone();
    let archive_name = archive_name.to_owned();
    state.background_task_tracker.spawn_tracked(async move {
        let job = run(
            &pool,
            &encryption_key,
            &repo_lock,
            &task_registry,
            repo_id,
            archive_id,
            &archive_name,
        );
        let error = match AssertUnwindSafe(job).catch_unwind().await {
            Ok(Ok(())) => return,
            Ok(Err(e)) => e.to_string(),
            Err(payload) => panic_reason(payload.as_ref()),
        };
        tracing::warn!(archive_id, archive_name, error = %error, "archive growth failed");
        if let Err(e) = sqlx::query!(
            "UPDATE archive_growth SET status = 'failed', finished_at = NOW(), error_message = $2 \
             WHERE archive_id = $1 AND status IN ('pending', 'computing')",
            archive_id,
            error,
        )
        .execute(&pool)
        .await
        {
            tracing::error!(archive_id, error = %e, "failed to record archive growth failure");
        }
    });
}

async fn run(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_lock: &RepoLock,
    task_registry: &TaskRegistry,
    repo_id: i64,
    archive_id: i64,
    archive_name: &str,
) -> Result<(), ApiError> {
    let (borg_repo, env) = get_repo_env(pool, encryption_key, repo_id).await?;
    // borg would wait for the repository lock anyway; queueing here keeps the
    // row reading "pending" until the repository is actually being read.
    let _repo_guard = repo_lock.acquire(repo_id).await;
    sqlx::query!(
        "UPDATE archive_growth SET status = 'computing' WHERE archive_id = $1",
        archive_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    let growth = list_growth(&borg_repo, &env, archive_name, task_registry).await?;
    let result = serde_json::to_string(&growth)
        .map_err(|e| ApiError::Internal(format!("serialising archive growth: {e}")))?;
    sqlx::query!(
        "UPDATE archive_growth SET status = 'done', result = $2, finished_at = NOW() WHERE \
         archive_id = $1",
        archive_id,
        result,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Reads every file of the archive with the bytes only it holds into a
/// breakdown. Drains stderr alongside stdout, so a chatty borg cannot fill
/// that pipe and stall with the repository lock held.
async fn list_growth(
    borg_repo: &str,
    env: &HashMap<String, String>,
    archive_name: &str,
    task_registry: &TaskRegistry,
) -> Result<ArchiveGrowth, ApiError> {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let mut child = Borg::new()
        .with_registry(task_registry.clone())
        .spawn(
            &[
                "list",
                "--json-lines",
                "--format",
                "{size}{dcsize}",
                "--lock-wait",
                LOCK_WAIT_SECS,
                &repo_archive,
            ],
            env,
        )
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;
    let Some(stdout) = child.take_stdout() else {
        return Err(ApiError::Internal("no stdout from borg".to_owned()));
    };
    let stderr = child.take_stderr();
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut buf).await;
        }
        buf
    });

    let mut builder = GrowthBuilder::default();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = tokio::time::timeout(LINE_READ_TIMEOUT, lines.next_line())
        .await
        .map_err(|_| ApiError::Internal("timed out reading borg output".to_owned()))?
        .map_err(|e| ApiError::Internal(format!("reading borg output: {e}")))?
    {
        add_line(&mut builder, &line);
    }

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .map_err(|_| ApiError::Internal("borg wait timed out".to_owned()))?
        .map_err(|e| ApiError::Internal(format!("borg wait failed: {e}")))?;
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() {
        return Err(classify_borg_error(status.code().unwrap_or(1), &stderr));
    }
    Ok(builder.finish())
}

/// Adds one line of borg's listing, skipping (rather than failing on) a line
/// that is not a listed entry.
fn add_line(builder: &mut GrowthBuilder, line: &str) {
    if line.is_empty() {
        return;
    }
    let Ok(file) = serde_json::from_str::<ListedFile>(line).inspect_err(|e| {
        tracing::trace!(error = %e, "skipping unparseable borg list line");
    }) else {
        return;
    };
    builder.add(&file.path, file.dcsize, file.size);
}

/// Fails the breakdowns an earlier server process left queued or running.
/// They only ever run inside the server process, so at startup nothing is
/// still working on them, yet they would keep reading as under way and keep
/// [`recompute`] from starting them again.
///
/// Returns how many were failed.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn release_interrupted(pool: &PgPool) -> Result<u64, ApiError> {
    sqlx::query!(
        "UPDATE archive_growth SET status = 'failed', finished_at = NOW(), error_message = \
         'interrupted by a server restart' WHERE status IN ('pending', 'computing')"
    )
    .execute(pool)
    .await
    .map(|result| result.rows_affected())
    .map_err(ApiError::Database)
}

#[cfg(test)]
mod tests;
