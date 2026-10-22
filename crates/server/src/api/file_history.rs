// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A file's versions across one host's archives in a repository, read from
//! the archive index rather than from borg.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use shared::{
    responses::{
        FileHistoryArchive, FileHistoryEntry, FileHistoryIndexResponse, FileHistoryResponse,
    },
    types::IndexStatus,
};
use sqlx::PgPool;

use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{
    AppState,
    api::archives::validate_extract_path,
    archive_index::{self, codec::DirEntry},
    error::ApiError,
};

/// Query parameters naming the file whose history is wanted.
#[derive(Debug, Deserialize)]
pub struct FileHistoryQuery {
    /// The file's path within the archives.
    pub path: String,
}

/// The agent that wrote an archive.
pub(crate) struct ArchiveOwner {
    pub(crate) agent_id: i64,
    pub(crate) hostname: String,
}

/// The agent whose backup wrote `archive_name` into `repo_id`.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no successful backup recorded the
/// archive, or [`ApiError::Database`] if the query fails.
pub(crate) async fn archive_owner(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<ArchiveOwner, ApiError> {
    sqlx::query_as!(
        ArchiveOwner,
        "SELECT br.agent_id, c.hostname FROM backup_reports br JOIN agents c ON c.id = \
         br.agent_id WHERE br.repo_id = $1 AND br.archive_name = $2 AND br.status IN ('success', \
         'warning') ORDER BY br.started_at DESC, br.id DESC LIMIT 1",
        repo_id,
        archive_name,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("no backup recorded archive {archive_name}")))
}

/// Every archive `agent_id` wrote into `repo_id`, newest first.
async fn host_archives(
    pool: &PgPool,
    repo_id: i64,
    agent_id: i64,
) -> Result<Vec<FileHistoryArchive>, ApiError> {
    let rows = sqlx::query!(
        "SELECT name AS \"name!\", started_at AS \"started_at!\" FROM (SELECT DISTINCT ON \
         (br.archive_name) br.archive_name AS name, br.started_at FROM backup_reports br WHERE \
         br.repo_id = $1 AND br.agent_id = $2 AND br.archive_name IS NOT NULL AND br.status IN \
         ('success', 'warning') ORDER BY br.archive_name, br.started_at DESC, br.id DESC) latest \
         ORDER BY started_at DESC, name DESC",
        repo_id,
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| FileHistoryArchive {
            name: row.name,
            start: rfc3339(row.started_at),
        })
        .collect())
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// What borg calls a regular file in a listing's type column.
enum EntryKind {
    RegularFile,
    Other,
}

impl From<&str> for EntryKind {
    fn from(entry_type: &str) -> Self {
        match entry_type {
            "-" => Self::RegularFile,
            _ => Self::Other,
        }
    }
}

/// What one archive's index says about the file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Version { size: i64, mtime: String },
    Absent,
    NotIndexed,
}

impl From<Option<&DirEntry>> for Seen {
    fn from(entry: Option<&DirEntry>) -> Self {
        match entry.map(|entry| (EntryKind::from(entry.entry_type.as_str()), entry)) {
            Some((EntryKind::RegularFile, entry)) => Self::Version {
                size: entry.size,
                mtime: entry.mtime.clone(),
            },
            Some((EntryKind::Other, _)) | None => Self::Absent,
        }
    }
}

/// Folds consecutive archives that saw the file the same way into one entry.
fn group(archives: Vec<(FileHistoryArchive, Seen)>) -> Vec<FileHistoryEntry> {
    let mut runs: Vec<(Seen, Vec<FileHistoryArchive>)> = Vec::new();
    for (archive, seen) in archives {
        match runs.last_mut() {
            Some((last, members)) if *last == seen => members.push(archive),
            Some(_) | None => runs.push((seen, vec![archive])),
        }
    }
    runs.into_iter()
        .map(|(seen, archives)| match seen {
            Seen::Version { size, mtime } => FileHistoryEntry::Version {
                archives,
                size,
                mtime,
            },
            Seen::Absent => FileHistoryEntry::Absent { archives },
            Seen::NotIndexed => FileHistoryEntry::NotIndexed { archives },
        })
        .collect()
}

/// The directory a path is in and its own name; the root's children have
/// the empty directory, as the index stores it.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The path as the index stores it: no leading `./` or `/`.
fn index_path(path: &str) -> &str {
    path.trim_start_matches("./").trim_start_matches('/')
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/file-history",
    tag = "Archives",
    operation_id = "fileHistory",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path,
            description = "An archive holding the file; its host's archives are searched"),
        ("path" = String, Query, description = "File path within the archives"),
    ),
    responses(
        (status = 200, description = "The file's versions, newest first",
            body = FileHistoryResponse),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No backup recorded the archive"),
    )
)]
/// List every version of one file across the archives its host wrote into
/// the repository, from the archive index.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid path, the repository's
/// permission errors, [`ApiError::NotFound`] for an archive no backup
/// recorded, and database errors.
pub async fn file_history(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<FileHistoryQuery>,
) -> Result<Json<FileHistoryResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    validate_extract_path(&query.path)?;
    let path = index_path(&query.path);
    let (parent, name) = split_path(path);

    let owner = archive_owner(&state.pool, repo_id, &archive_name).await?;
    let archives = host_archives(&state.pool, repo_id, owner.agent_id).await?;
    let names: Vec<String> = archives.iter().map(|a| a.name.clone()).collect();
    let statuses = archive_index::index_statuses(&state.pool, repo_id, &names).await?;
    let indexed: Vec<String> = names
        .iter()
        .filter(|name| matches!(statuses.get(*name), Some(IndexStatus::Done)))
        .cloned()
        .collect();
    let found = archive_index::find_entries(&state.pool, repo_id, &indexed, parent, name).await?;

    let seen = archives
        .into_iter()
        .map(|archive| {
            let seen = match statuses.get(&archive.name) {
                Some(IndexStatus::Done) => Seen::from(found.get(&archive.name)),
                Some(IndexStatus::Pending | IndexStatus::Indexing | IndexStatus::Failed) | None => {
                    Seen::NotIndexed
                }
            };
            (archive, seen)
        })
        .collect();

    Ok(Json(FileHistoryResponse {
        path: path.to_owned(),
        hostname: owner.hostname,
        agent_online: state.registry.is_connected(owner.agent_id).await,
        entries: group(seen),
    }))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/file-history/index",
    tag = "Archives",
    operation_id = "indexFileHistory",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path,
            description = "An archive whose host's unindexed archives to index"),
    ),
    responses(
        (status = 200, description = "Indexing started", body = FileHistoryIndexResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No backup recorded the archive"),
    )
)]
/// Start indexing every archive of the same host that has no index yet, so
/// a file's history can cover them. Indexing runs in the background, one
/// archive after another.
///
/// # Errors
///
/// Returns the repository's permission errors, [`ApiError::NotFound`] for
/// an archive no backup recorded, and database errors.
pub async fn index_file_history(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<FileHistoryIndexResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let owner = archive_owner(&state.pool, repo_id, &archive_name).await?;
    let names: Vec<String> = host_archives(&state.pool, repo_id, owner.agent_id)
        .await?
        .into_iter()
        .map(|archive| archive.name)
        .collect();
    let statuses: HashMap<String, IndexStatus> =
        archive_index::index_statuses(&state.pool, repo_id, &names).await?;
    let missing: Vec<String> = names
        .into_iter()
        .filter(|name| !statuses.contains_key(name))
        .collect();
    for name in &missing {
        archive_index::ensure_indexed(
            state.pool.clone(),
            state.encryption_key,
            repo_id,
            name.clone(),
            state.repo_lock.clone(),
            &state.background_task_tracker,
            state.task_registry.clone(),
        )
        .await?;
    }
    Ok(Json(FileHistoryIndexResponse {
        queued: missing.len(),
    }))
}

#[cfg(test)]
mod tests;
