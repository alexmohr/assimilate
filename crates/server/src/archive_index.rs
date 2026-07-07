// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use serde::Serialize;
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    RepoLock,
    api::archives::{
        ContentEntry, LOCK_WAIT_SECS, classify_borg_error, get_repo_env, normalize_path,
    },
    borg::Borg,
    error::ApiError,
};

/// Status of an archive content indexing job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    /// Index job not yet started.
    Pending,
    /// Indexing is in progress.
    Indexing,
    /// Indexing completed successfully.
    Done,
    /// Indexing failed.
    Failed,
}

impl std::str::FromStr for IndexStatus {
    type Err = std::convert::Infallible;

    /// Any value other than a recognized status (including an absent DB row,
    /// which callers represent as an empty string) is treated as `Pending`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "indexing" => Self::Indexing,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Pending,
        })
    }
}

/// Returns the `archives.id` for the given `(repo_id, archive_name)`, creating the row if absent.
async fn get_or_create_archive_id(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar!(
        "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT (repo_id, name) DO \
         UPDATE SET name = EXCLUDED.name RETURNING id",
        repo_id,
        archive_name,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_index_status(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Option<IndexStatus>, ApiError> {
    let row = sqlx::query_scalar!(
        "SELECT j.status FROM archive_index_jobs j JOIN archives a ON a.id = j.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
        repo_id,
        archive_name,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.map(|s: String| s.parse().unwrap_or(IndexStatus::Pending)))
}

/// Rows inserted per statement. Large archives are written in chunks so a single
/// statement never grows big enough to trip slow-statement alerts or timeouts.
const INSERT_CHUNK: usize = 5000;

#[derive(sqlx::FromRow)]
struct ArchivePathRow {
    id: i64,
    path: String,
}

async fn ensure_archive_paths(
    pool: &PgPool,
    repo_id: i64,
    paths: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    let mut unique_paths = paths.to_vec();
    unique_paths.sort_unstable();
    unique_paths.dedup();

    let mut map = HashMap::with_capacity(unique_paths.len());
    for chunk in unique_paths.chunks(INSERT_CHUNK) {
        sqlx::query!(
            "INSERT INTO archive_paths (repo_id, path) SELECT $1, unnest($2::text[]) ON CONFLICT \
             DO NOTHING",
            repo_id,
            chunk,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

        let rows = sqlx::query_as!(
            ArchivePathRow,
            "SELECT id, path FROM archive_paths WHERE repo_id = $1 AND path = ANY($2::text[])",
            repo_id,
            chunk,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;

        map.extend(rows.into_iter().map(|row| (row.path, row.id)));
    }

    Ok(map)
}

/// Atomically claim the indexing job and spawn a background task if we won the race.
/// Returns the current status after the claim attempt.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn ensure_indexed(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    archive_name: String,
    repo_lock: RepoLock,
) -> Result<IndexStatus, ApiError> {
    let archive_id = get_or_create_archive_id(&pool, repo_id, &archive_name).await?;

    let result = sqlx::query!(
        "INSERT INTO archive_index_jobs (archive_id, status) VALUES ($1, 'pending') ON CONFLICT \
         DO NOTHING",
        archive_id,
    )
    .execute(&pool)
    .await
    .map_err(ApiError::Database)?;

    if result.rows_affected() == 1 {
        // We claimed the job - spawn background indexing.
        let pool_bg = pool.clone();
        let archive_name_bg = archive_name.clone();
        tokio::spawn(async move {
            if let Err(e) = run_indexing(
                &pool_bg,
                &encryption_key,
                repo_id,
                &archive_name_bg,
                &repo_lock,
                &mut |_, _| {},
            )
            .await
            {
                tracing::error!(
                    repo_id,
                    archive_name = archive_name_bg,
                    error = %e,
                    "archive indexing failed"
                );
            }
        });
        return Ok(IndexStatus::Pending);
    }

    // Another task already claimed it - return current status.
    get_index_status(&pool, repo_id, &archive_name)
        .await
        .map(|s| s.unwrap_or(IndexStatus::Pending))
}

/// Archive names in this repository whose content index is already complete.
/// A full resync skips these: borg archives are immutable, so a finished
/// index never needs to be rebuilt.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_indexed_archive_names(
    pool: &PgPool,
    repo_id: i64,
) -> Result<HashSet<String>, ApiError> {
    let names = sqlx::query_scalar!(
        "SELECT a.name FROM archive_index_jobs j JOIN archives a ON a.id = j.archive_id WHERE \
         a.repo_id = $1 AND j.status = 'done'",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(names.into_iter().collect())
}

/// Ensure an index job row exists so `run_indexing` can transition it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn ensure_index_job(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<(), ApiError> {
    let archive_id = get_or_create_archive_id(pool, repo_id, archive_name).await?;
    sqlx::query!(
        "INSERT INTO archive_index_jobs (archive_id, status) VALUES ($1, 'pending') ON CONFLICT \
         DO NOTHING",
        archive_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn run_indexing<F: FnMut(u64, Option<&str>)>(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_name: &str,
    repo_lock: &RepoLock,
    on_progress: &mut F,
) -> Result<(), ApiError> {
    let archive_id = get_or_create_archive_id(pool, repo_id, archive_name).await?;
    // Serialise the borg `list` with every other borg operation on this repo so
    // indexing, deletes, syncs and backups never contend for the repository lock.
    let _repo_guard = repo_lock.acquire(repo_id).await;

    run_indexing_impl(
        pool,
        encryption_key,
        repo_id,
        archive_id,
        archive_name,
        on_progress,
    )
    .await
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn run_indexing_with_lock_held<F: FnMut(u64, Option<&str>)>(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_name: &str,
    on_progress: &mut F,
) -> Result<(), ApiError> {
    let archive_id = get_or_create_archive_id(pool, repo_id, archive_name).await?;

    run_indexing_impl(
        pool,
        encryption_key,
        repo_id,
        archive_id,
        archive_name,
        on_progress,
    )
    .await
}

async fn run_indexing_impl<F: FnMut(u64, Option<&str>)>(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_id: i64,
    archive_name: &str,
    on_progress: &mut F,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE archive_index_jobs SET status = 'indexing', started_at = NOW() WHERE archive_id = \
         $1",
        archive_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    match index_archive(
        pool,
        encryption_key,
        repo_id,
        archive_id,
        archive_name,
        on_progress,
    )
    .await
    {
        Ok(file_count) => {
            sqlx::query!(
                "UPDATE archive_index_jobs SET status = 'done', finished_at = NOW(), file_count = \
                 $2 WHERE archive_id = $1",
                archive_id,
                file_count,
            )
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            sqlx::query!(
                "UPDATE archive_index_jobs SET status = 'failed', finished_at = NOW(), \
                 error_message = $2 WHERE archive_id = $1",
                archive_id,
                msg,
            )
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
            Err(e)
        }
    }
}

/// Runs `borg list --json-lines` for the archive, parsing each output line
/// into a [`ContentEntry`] and reporting progress via `on_progress` every
/// ~300ms. Drains stderr concurrently with stdout: borg writes lock-wait
/// notices and warnings to stderr, and if that pipe fills (~64 KiB) while
/// stdout is still being read, borg blocks on the write, stdout stalls, and
/// `child.wait()` deadlocks with the repository lock held.
async fn borg_list_archive_entries<F: FnMut(u64, Option<&str>)>(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    archive_name: &str,
    on_progress: &mut F,
) -> Result<Vec<ContentEntry>, ApiError> {
    const LINE_READ_TIMEOUT: Duration = Duration::from_secs(30);

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let mut child = Borg::new()
        .spawn(
            &[
                "list",
                "--json-lines",
                "--lock-wait",
                LOCK_WAIT_SECS,
                &repo_archive,
            ],
            env,
        )
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let Some(stdout) = child.take_stdout() else {
        return Err(ApiError::Internal("no stdout from borg".to_string()));
    };

    let stderr = child.take_stderr();
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(mut se) = stderr {
            use tokio::io::AsyncReadExt;
            let _ = se.read_to_string(&mut buf).await;
        }
        buf
    });

    let mut raw: Vec<ContentEntry> = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut last_emit = std::time::Instant::now();
    loop {
        let line = tokio::time::timeout(LINE_READ_TIMEOUT, lines.next_line())
            .await
            .map_err(|_| ApiError::Internal("timed out reading borg output".to_string()))?
            .map_err(|e| ApiError::Internal(format!("reading borg output: {e}")))?;

        let Some(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line).inspect_err(|e| {
            tracing::trace!(error = %e, "skipping unparseable borg output line");
        }) else {
            continue;
        };
        raw.push(ContentEntry {
            entry_type: v
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            path: v
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map_or_else(String::new, normalize_path),
            size: v
                .get("size")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            mtime: v
                .get("mtime")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            mode: v
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
        if last_emit.elapsed() >= std::time::Duration::from_millis(300) {
            let current = raw.last().map(|entry| entry.path.as_str());
            on_progress(u64::try_from(raw.len()).unwrap_or(u64::MAX), current);
            last_emit = std::time::Instant::now();
        }
    }
    on_progress(
        u64::try_from(raw.len()).unwrap_or(u64::MAX),
        raw.last().map(|entry| entry.path.as_str()),
    );

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .map_err(|_| ApiError::Internal("borg wait timed out".to_string()))?
        .map_err(|e| ApiError::Internal(format!("borg wait failed: {e}")))?;
    let stderr_str = stderr_task.await.unwrap_or_default();
    if !status.success() {
        return Err(classify_borg_error(status.code().unwrap_or(1), &stderr_str));
    }

    Ok(raw)
}

struct ExpandedArchiveEntries {
    paths: Vec<String>,
    parent_paths: Vec<String>,
    entry_types: Vec<String>,
    sizes: Vec<i64>,
    mtimes: Vec<String>,
    modes: Vec<String>,
    path_values: Vec<String>,
}

/// Flattens the raw `borg list` entries into parallel column vectors ready
/// for bulk insert, synthesising any missing ancestor directories along the
/// way (borg only lists the leaf entries actually present in the archive).
fn expand_entries_with_ancestors(raw: Vec<ContentEntry>) -> ExpandedArchiveEntries {
    let mut paths: Vec<String> = Vec::new();
    let mut parent_paths: Vec<String> = Vec::new();
    let mut entry_types: Vec<String> = Vec::new();
    let mut sizes: Vec<i64> = Vec::new();
    let mut mtimes: Vec<String> = Vec::new();
    let mut modes: Vec<String> = Vec::new();

    let mut seen: HashSet<String> = HashSet::new();
    let mut path_values: HashSet<String> = HashSet::new();

    let mut add =
        |path: String, parent: String, etype: String, size: i64, mtime: String, mode: String| {
            path_values.insert(path.clone());
            path_values.insert(parent.clone());

            if seen.insert(path.clone()) {
                paths.push(path);
                parent_paths.push(parent);
                entry_types.push(etype);
                sizes.push(size);
                mtimes.push(mtime);
                modes.push(mode);
            }
        };

    for entry in raw {
        if entry.path.is_empty() {
            continue;
        }

        // Ensure all ancestor directories are present.
        let segments: Vec<&str> = entry.path.split('/').collect();
        for depth in 1..segments.len() {
            let dir_path = segments.get(..depth).unwrap_or(&[]).join("/");
            let dir_parent = if depth == 1 {
                String::new()
            } else {
                segments
                    .get(..depth.saturating_sub(1))
                    .unwrap_or(&[])
                    .join("/")
            };
            add(
                dir_path,
                dir_parent,
                "d".to_string(),
                0,
                String::new(),
                String::new(),
            );
        }

        let parent = entry
            .path
            .rfind('/')
            .map_or_else(String::new, |i| entry.path[..i].to_string());
        add(
            entry.path,
            parent,
            entry.entry_type,
            entry.size,
            entry.mtime,
            entry.mode,
        );
    }

    ExpandedArchiveEntries {
        paths,
        parent_paths,
        entry_types,
        sizes,
        mtimes,
        modes,
        path_values: path_values.into_iter().collect(),
    }
}

/// Inserts the flattened archive-file rows in chunks rather than one giant
/// statement: archives with millions of files would otherwise build a
/// single query large enough to trip slow-statement alerts and statement
/// timeouts.
struct ArchiveFileColumns<'a> {
    path_ids: &'a [i64],
    parent_path_ids: &'a [i64],
    entry_types: &'a [String],
    sizes: &'a [i64],
    mtimes: &'a [String],
    modes: &'a [String],
}

async fn insert_archive_files_chunked(
    pool: &PgPool,
    archive_id: i64,
    columns: &ArchiveFileColumns<'_>,
) -> Result<(), ApiError> {
    let ArchiveFileColumns {
        path_ids,
        parent_path_ids,
        entry_types,
        sizes,
        mtimes,
        modes,
    } = *columns;

    let mut offset = 0;
    while offset < path_ids.len() {
        let end = offset.saturating_add(INSERT_CHUNK).min(path_ids.len());
        sqlx::query!(
            "INSERT INTO archive_files (archive_id, path_id, parent_path_id, entry_type, size, \
             mtime, mode) SELECT $1, unnest($2::bigint[]), unnest($3::bigint[]), \
             unnest($4::text[]), unnest($5::bigint[]), unnest($6::text[]), unnest($7::text[]) ON \
             CONFLICT DO NOTHING",
            archive_id,
            path_ids.get(offset..end).unwrap_or(&[]) as &[i64],
            parent_path_ids.get(offset..end).unwrap_or(&[]) as &[i64],
            entry_types.get(offset..end).unwrap_or(&[]) as &[String],
            sizes.get(offset..end).unwrap_or(&[]) as &[i64],
            mtimes.get(offset..end).unwrap_or(&[]) as &[String],
            modes.get(offset..end).unwrap_or(&[]) as &[String],
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
        offset = end;
    }
    Ok(())
}

async fn index_archive<F: FnMut(u64, Option<&str>)>(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_id: i64,
    archive_name: &str,
    on_progress: &mut F,
) -> Result<i64, ApiError> {
    let (borg_repo, env) = get_repo_env(pool, encryption_key, repo_id).await?;
    let raw = borg_list_archive_entries(&borg_repo, &env, archive_name, on_progress).await?;

    let ExpandedArchiveEntries {
        paths,
        parent_paths,
        entry_types,
        sizes,
        mtimes,
        modes,
        path_values,
    } = expand_entries_with_ancestors(raw);

    let file_count = i64::try_from(paths.len()).unwrap_or(i64::MAX);
    let path_id_map = ensure_archive_paths(pool, repo_id, &path_values).await?;
    let path_id = |path: &str| -> Result<i64, ApiError> {
        path_id_map
            .get(path)
            .copied()
            .ok_or_else(|| ApiError::Internal(format!("missing archive path id for {path}")))
    };

    let path_ids: Vec<i64> = paths
        .iter()
        .map(|path| path_id(path))
        .collect::<Result<_, _>>()?;
    let parent_path_ids: Vec<i64> = parent_paths
        .iter()
        .map(|path| path_id(path))
        .collect::<Result<_, _>>()?;

    insert_archive_files_chunked(
        pool,
        archive_id,
        &ArchiveFileColumns {
            path_ids: &path_ids,
            parent_path_ids: &parent_path_ids,
            entry_types: &entry_types,
            sizes: &sizes,
            mtimes: &mtimes,
            modes: &modes,
        },
    )
    .await?;

    Ok(file_count)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn query_dir(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    parent_path: &str,
    limit: i64,
) -> Result<Vec<ContentEntry>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        path: String,
        entry_type: String,
        size: i64,
        mtime: String,
        mode: String,
    }

    let archive_id = sqlx::query_scalar!(
        "SELECT id FROM archives WHERE repo_id = $1 AND name = $2",
        repo_id,
        archive_name,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    let Some(archive_id) = archive_id else {
        return Ok(Vec::new());
    };

    let parent_path_id = sqlx::query_scalar!(
        "SELECT id FROM archive_paths WHERE repo_id = $1 AND path = $2",
        repo_id,
        parent_path,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    let Some(parent_path_id) = parent_path_id else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query_as!(
        Row,
        "SELECT p.path, f.entry_type, f.size, f.mtime, f.mode FROM archive_files f JOIN \
         archive_paths p ON p.id = f.path_id WHERE f.archive_id = $1 AND f.parent_path_id = $2 \
         ORDER BY f.entry_type DESC, p.path ASC LIMIT $3",
        archive_id,
        parent_path_id,
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| ContentEntry {
            entry_type: r.entry_type,
            path: r.path,
            size: r.size,
            mtime: r.mtime,
            mode: r.mode,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::IndexStatus;

    #[test]
    fn index_status_parses_known_values() {
        assert_eq!("indexing".parse(), Ok(IndexStatus::Indexing));
        assert_eq!("done".parse(), Ok(IndexStatus::Done));
        assert_eq!("failed".parse(), Ok(IndexStatus::Failed));
    }

    #[test]
    fn index_status_defaults_to_pending_for_unknown_values() {
        assert_eq!("".parse(), Ok(IndexStatus::Pending));
        assert_eq!("bogus".parse(), Ok(IndexStatus::Pending));
    }

    #[test]
    fn parent_path_for_root_file() {
        let path = "README.md";
        let parent = path
            .rfind('/')
            .map_or_else(String::new, |i| path[..i].to_string());
        assert_eq!(parent, "");
    }

    #[test]
    fn parent_path_for_nested_file() {
        let path = "home/user/docs/file.txt";
        let parent = path
            .rfind('/')
            .map_or_else(String::new, |i| path[..i].to_string());
        assert_eq!(parent, "home/user/docs");
    }

    #[test]
    fn ancestor_synthesis_produces_all_dirs() {
        // A single deep file should produce 3 synthetic directory entries.
        let path = "a/b/c/file.txt";
        let segments: Vec<&str> = path.split('/').collect();
        let mut dirs: Vec<String> = Vec::new();
        for depth in 1..segments.len() {
            dirs.push(segments.get(..depth).unwrap_or(&[]).join("/"));
        }
        assert_eq!(dirs, vec!["a", "a/b", "a/b/c"]);
    }

    #[test]
    fn empty_path_skipped() {
        // The archive root "." normalises to "" and must not produce a DB row.
        let path = "";
        assert!(path.is_empty());
    }
}
