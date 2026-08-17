// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod codec;

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use shared::types::IndexStatus;
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    RepoLock,
    api::archives::{
        ContentEntry, LOCK_WAIT_SECS, classify_borg_error, get_repo_env, normalize_path,
    },
    archive_index::codec::DirEntry,
    background_tasks::BackgroundTaskTracker,
    borg::Borg,
    error::ApiError,
};

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

    row.map(|s: String| {
        s.parse()
            .map_err(|_| ApiError::Internal(format!("invalid index status: {s}")))
    })
    .transpose()
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
    background_task_tracker: &BackgroundTaskTracker,
    task_registry: shared::task_registry::TaskRegistry,
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
        let task_guard = background_task_tracker.begin();
        tokio::spawn(async move {
            let _task_guard = task_guard;
            if let Err(e) = run_indexing(
                &pool_bg,
                &encryption_key,
                repo_id,
                &archive_name_bg,
                &repo_lock,
                &mut |_, _| {},
                &task_registry,
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
        .map(Option::unwrap_or_default)
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
    task_registry: &shared::task_registry::TaskRegistry,
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
        task_registry,
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
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(), ApiError> {
    let archive_id = get_or_create_archive_id(pool, repo_id, archive_name).await?;

    run_indexing_impl(
        pool,
        encryption_key,
        repo_id,
        archive_id,
        archive_name,
        on_progress,
        task_registry,
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
    task_registry: &shared::task_registry::TaskRegistry,
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
        task_registry,
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
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<Vec<ContentEntry>, ApiError> {
    const LINE_READ_TIMEOUT: Duration = Duration::from_secs(30);

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let mut child = Borg::new()
        .with_registry(task_registry.clone())
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

/// Directory children grouped by their parent directory path, ready to be
/// encoded into one blob per directory.
struct ExpandedArchiveEntries {
    /// `(directory path, children in listing order)` for every directory that
    /// has at least one child.
    dirs: Vec<(String, Vec<DirEntry>)>,
    /// Number of distinct entries seen, including synthesised ancestors.
    entry_count: i64,
}

/// Splits a normalised path into its parent directory and final segment.
fn split_parent(path: &str) -> (&str, &str) {
    path.rfind('/')
        .map_or(("", path), |i| (&path[..i], &path[i.saturating_add(1)..]))
}

/// Groups the raw `borg list` entries under their parent directories,
/// synthesising any missing ancestor directories along the way (borg only lists
/// the leaf entries actually present in the archive) and sorting each
/// directory's children into listing order.
fn expand_entries_with_ancestors(raw: Vec<ContentEntry>) -> ExpandedArchiveEntries {
    let mut children: HashMap<String, Vec<DirEntry>> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut add = |path: &str, entry_type: String, size: i64, mtime: String, mode: String| {
        if !seen.insert(path.to_owned()) {
            return;
        }
        let (parent, name) = split_parent(path);
        children
            .entry(parent.to_owned())
            .or_default()
            .push(DirEntry {
                name: name.to_owned(),
                entry_type,
                size,
                mtime,
                mode,
            });
    };

    raw.into_iter()
        .filter(|entry| !entry.path.is_empty())
        .for_each(|entry| {
            // Ensure all ancestor directories are present.
            entry.path.match_indices('/').for_each(|(index, _)| {
                add(
                    entry.path.get(..index).unwrap_or_default(),
                    "d".to_owned(),
                    0,
                    String::new(),
                    String::new(),
                );
            });

            add(
                &entry.path,
                entry.entry_type,
                entry.size,
                entry.mtime,
                entry.mode,
            );
        });

    let entry_count = i64::try_from(seen.len()).unwrap_or(i64::MAX);
    let dirs = children
        .into_iter()
        .map(|(dir, mut entries)| {
            codec::sort_for_listing(&mut entries);
            (dir, entries)
        })
        .collect();

    ExpandedArchiveEntries { dirs, entry_count }
}

/// Maximum number of entries packed into a single `archive_dirs` row. A
/// directory with more children than this spills into further chunks, so no
/// single row grows unbounded and a limited listing only reads the chunks it
/// needs.
const CHUNK_ENTRIES: usize = 2000;

/// One encoded chunk, ready for insertion.
struct DirChunk {
    dir_path_id: i64,
    chunk_no: i32,
    entries: Vec<u8>,
}

/// Encodes each directory's children into [`CHUNK_ENTRIES`]-sized compressed
/// chunks.
fn encode_dir_chunks(dir_path_id: i64, entries: &[DirEntry]) -> Vec<DirChunk> {
    entries
        .chunks(CHUNK_ENTRIES)
        .enumerate()
        .map(|(index, chunk)| DirChunk {
            dir_path_id,
            chunk_no: i32::try_from(index).unwrap_or(i32::MAX),
            entries: codec::encode(chunk),
        })
        .collect()
}

/// Replaces every stored directory blob for `archive_id`.
///
/// Indexing an archive is a *replace*, not a merge. A re-index is a supported
/// path - a `failed` or interrupted job is picked up again by a later full
/// resync - and simply upserting would leave behind chunk rows that the new pass
/// no longer produces. A directory that previously spanned more chunks than it
/// does now (which happens when [`CHUNK_ENTRIES`] changes between deploys, the
/// same skew [`fetch_dir_entries`] is written to tolerate on the read side)
/// would keep its trailing rows, and a read has no way to tell a stale chunk
/// from a live one: it would append the leftovers to the end of the listing.
/// Clearing the archive's rows first makes that impossible.
///
/// The delete and the inserts are deliberately not wrapped in one transaction:
/// an archive with millions of entries would hold it open for the whole indexing
/// run. It does not need to be atomic, because `run_indexing_impl` holds the job
/// in `indexing` for the duration and the API only serves the stored index for
/// archives whose job reached `done` - anything else falls back to reading the
/// archive live from borg, so a half-written index is never observable.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn replace_archive_dirs(
    pool: &PgPool,
    archive_id: i64,
    dirs: &[(i64, Vec<DirEntry>)],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM archive_dirs WHERE archive_id = $1", archive_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    let chunks: Vec<DirChunk> = dirs
        .iter()
        .flat_map(|(dir_path_id, entries)| encode_dir_chunks(*dir_path_id, entries))
        .collect();

    insert_archive_dirs_chunked(pool, archive_id, &chunks).await
}

/// Inserts the encoded directory blobs in chunks rather than one giant
/// statement: archives with millions of files would otherwise build a
/// single query large enough to trip slow-statement alerts and statement
/// timeouts.
async fn insert_archive_dirs_chunked(
    pool: &PgPool,
    archive_id: i64,
    chunks: &[DirChunk],
) -> Result<(), ApiError> {
    for batch in chunks.chunks(INSERT_CHUNK) {
        let dir_path_ids: Vec<i64> = batch.iter().map(|chunk| chunk.dir_path_id).collect();
        let chunk_nos: Vec<i32> = batch.iter().map(|chunk| chunk.chunk_no).collect();
        let entries: Vec<Vec<u8>> = batch.iter().map(|chunk| chunk.entries.clone()).collect();

        sqlx::query!(
            "INSERT INTO archive_dirs (archive_id, dir_path_id, chunk_no, entries) SELECT $1, \
             unnest($2::bigint[]), unnest($3::int[]), unnest($4::bytea[]) ON CONFLICT \
             (archive_id, dir_path_id, chunk_no) DO UPDATE SET entries = EXCLUDED.entries",
            archive_id,
            &dir_path_ids,
            &chunk_nos,
            &entries,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
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
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<i64, ApiError> {
    let (borg_repo, env) = get_repo_env(pool, encryption_key, repo_id).await?;
    let raw = borg_list_archive_entries(&borg_repo, &env, archive_name, on_progress, task_registry)
        .await?;

    let ExpandedArchiveEntries { dirs, entry_count } = expand_entries_with_ancestors(raw);

    // Only directory paths need an id: a file's name lives inside its parent's blob.
    let dir_paths: Vec<String> = dirs.iter().map(|(dir, _)| dir.clone()).collect();
    let path_id_map = ensure_archive_paths(pool, repo_id, &dir_paths).await?;

    let resolved: Vec<(i64, Vec<DirEntry>)> = dirs
        .into_iter()
        .map(|(dir, entries)| {
            path_id_map
                .get(&dir)
                .copied()
                .ok_or_else(|| ApiError::Internal(format!("missing archive path id for {dir}")))
                .map(|dir_path_id| (dir_path_id, entries))
        })
        .collect::<Result<_, _>>()?;

    replace_archive_dirs(pool, archive_id, &resolved).await?;

    Ok(entry_count)
}

/// Fetches the stored chunks for one directory in listing order, stopping as
/// soon as `limit` entries have been collected.
///
/// Chunks are read in batches sized from [`CHUNK_ENTRIES`], so the common case
/// of a directory that fits in one chunk costs exactly one round trip. The loop
/// keeps going if a batch turns out to hold fewer entries than expected, which
/// keeps the read correct for rows written when [`CHUNK_ENTRIES`] had a
/// different value.
async fn fetch_dir_entries(
    pool: &PgPool,
    archive_id: i64,
    dir_path_id: i64,
    limit: usize,
) -> Result<Vec<DirEntry>, ApiError> {
    let mut entries: Vec<DirEntry> = Vec::new();
    let mut offset: i64 = 0;

    while entries.len() < limit {
        let remaining = limit.saturating_sub(entries.len());
        let wanted = remaining.div_ceil(CHUNK_ENTRIES).max(1);
        let batch = i64::try_from(wanted).unwrap_or(i64::MAX);

        let blobs = sqlx::query_scalar!(
            "SELECT entries FROM archive_dirs WHERE archive_id = $1 AND dir_path_id = $2 ORDER BY \
             chunk_no OFFSET $3 LIMIT $4",
            archive_id,
            dir_path_id,
            offset,
            batch,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;

        let fetched = i64::try_from(blobs.len()).unwrap_or(i64::MAX);

        entries.extend(
            blobs
                .iter()
                .map(|blob| codec::decode(blob))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ApiError::Internal(format!("corrupt archive directory index: {e}")))?
                .into_iter()
                .flatten(),
        );

        if fetched < batch {
            break;
        }
        offset = offset.saturating_add(fetched);
    }

    entries.truncate(limit);
    Ok(entries)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails, or
/// [`ApiError::Internal`] if a stored directory blob cannot be decoded.
pub async fn query_dir(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    parent_path: &str,
    limit: i64,
) -> Result<Vec<ContentEntry>, ApiError> {
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

    let dir_path_id = sqlx::query_scalar!(
        "SELECT id FROM archive_paths WHERE repo_id = $1 AND path = $2",
        repo_id,
        parent_path,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    let Some(dir_path_id) = dir_path_id else {
        return Ok(Vec::new());
    };

    let limit = usize::try_from(limit).unwrap_or(0);
    let entries = fetch_dir_entries(pool, archive_id, dir_path_id, limit).await?;

    Ok(entries
        .into_iter()
        .map(|entry| ContentEntry {
            path: if parent_path.is_empty() {
                entry.name
            } else {
                format!("{parent_path}/{}", entry.name)
            },
            entry_type: entry.entry_type,
            size: entry.size,
            mtime: entry.mtime,
            mode: entry.mode,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use shared::types::IndexStatus;

    use super::*;

    fn content_entry(path: &str, entry_type: &str) -> ContentEntry {
        ContentEntry {
            entry_type: entry_type.to_owned(),
            path: path.to_owned(),
            size: 10,
            mtime: "2026-06-05T12:00:00.000000".to_owned(),
            mode: "-rw-r--r--".to_owned(),
        }
    }

    fn dir_names(expanded: &ExpandedArchiveEntries, dir: &str) -> Vec<String> {
        expanded
            .dirs
            .iter()
            .find(|(path, _)| path == dir)
            .map(|(_, entries)| entries.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The entry stored for `name` inside `dir`, so a test can assert on the
    /// metadata that was kept and not just on which paths survived.
    fn dir_entry_named(expanded: &ExpandedArchiveEntries, dir: &str, name: &str) -> DirEntry {
        expanded
            .dirs
            .iter()
            .find(|(path, _)| path == dir)
            .and_then(|(_, entries)| entries.iter().find(|entry| entry.name == name))
            .cloned()
            .unwrap()
    }

    #[test]
    fn index_status_parses_known_values() {
        assert_eq!("indexing".parse(), Ok(IndexStatus::Indexing));
        assert_eq!("done".parse(), Ok(IndexStatus::Done));
        assert_eq!("failed".parse(), Ok(IndexStatus::Failed));
    }

    #[test]
    fn index_status_returns_error_for_unknown_values() {
        assert!("".parse::<IndexStatus>().is_err());
        assert!("bogus".parse::<IndexStatus>().is_err());
    }

    #[test]
    fn parent_path_for_root_file() {
        assert_eq!(split_parent("README.md"), ("", "README.md"));
    }

    #[test]
    fn parent_path_for_nested_file() {
        assert_eq!(
            split_parent("home/user/docs/file.txt"),
            ("home/user/docs", "file.txt")
        );
    }

    #[test]
    fn ancestor_synthesis_produces_all_dirs() {
        // A single deep file should produce 3 synthetic directory entries.
        let expanded = expand_entries_with_ancestors(vec![content_entry("a/b/c/file.txt", "-")]);

        let mut dirs: Vec<&str> = expanded.dirs.iter().map(|(dir, _)| dir.as_str()).collect();
        dirs.sort_unstable();

        assert_eq!(dirs, ["", "a", "a/b", "a/b/c"]);
        assert_eq!(dir_names(&expanded, ""), ["a"]);
        assert_eq!(dir_names(&expanded, "a"), ["b"]);
        assert_eq!(dir_names(&expanded, "a/b"), ["c"]);
        assert_eq!(dir_names(&expanded, "a/b/c"), ["file.txt"]);
        assert_eq!(expanded.entry_count, 4);
    }

    #[test]
    fn empty_path_skipped() {
        // The archive root "." normalises to "" and must not produce a DB row.
        let expanded = expand_entries_with_ancestors(vec![content_entry("", "d")]);

        assert_eq!(expanded.dirs.len(), 0);
        assert_eq!(expanded.entry_count, 0);
    }

    #[test]
    fn duplicate_paths_are_indexed_once() {
        let expanded = expand_entries_with_ancestors(vec![
            content_entry("dir/file.txt", "-"),
            content_entry("dir/file.txt", "-"),
        ]);

        assert_eq!(dir_names(&expanded, "dir"), ["file.txt"]);
        assert_eq!(expanded.entry_count, 2);
    }

    #[test]
    fn borg_listing_a_directory_before_its_contents_keeps_the_real_metadata() {
        // The normal ordering: borg lists "a" itself, then what is inside it, so the
        // placeholder synthesised while expanding "a/f" loses to the entry already
        // recorded and the directory keeps borg's own metadata.
        let expanded =
            expand_entries_with_ancestors(vec![content_entry("a", "d"), content_entry("a/f", "-")]);

        let stored = dir_entry_named(&expanded, "", "a");
        assert_eq!(stored.mtime, "2026-06-05T12:00:00.000000");
        assert_eq!(stored.mode, "-rw-r--r--");
        assert_eq!(stored.size, 10);
        assert_eq!(expanded.entry_count, 2);
    }

    #[test]
    fn a_synthesised_ancestor_wins_when_it_is_recorded_first() {
        // The reverse ordering: "a" is synthesised as a placeholder while expanding
        // "a/f", so borg's own later entry for "a" is dropped entirely - metadata
        // included - because `add` keeps the first write for a path. Such a directory
        // lists with a blank mtime/mode instead of borg's values.
        //
        // This precedence predates the per-directory blob layout: the old code had the
        // same `seen` short-circuit, and its insert used ON CONFLICT DO NOTHING. It is
        // pinned here rather than changed.
        let expanded =
            expand_entries_with_ancestors(vec![content_entry("a/f", "-"), content_entry("a", "d")]);

        let stored = dir_entry_named(&expanded, "", "a");
        assert_eq!(stored.entry_type, "d");
        assert_eq!(stored.mtime, "");
        assert_eq!(stored.mode, "");
        assert_eq!(stored.size, 0);

        // Either way the path is recorded exactly once.
        assert_eq!(dir_names(&expanded, ""), ["a"]);
        assert_eq!(expanded.entry_count, 2);
    }

    #[test]
    fn children_are_sorted_directories_first_then_by_name() {
        let expanded = expand_entries_with_ancestors(vec![
            content_entry("top/zebra.txt", "-"),
            content_entry("top/alpha.txt", "-"),
            content_entry("top/sub/nested.txt", "-"),
        ]);

        assert_eq!(
            dir_names(&expanded, "top"),
            ["sub", "alpha.txt", "zebra.txt"]
        );
    }

    #[test]
    fn entries_are_split_into_bounded_chunks() {
        let entries: Vec<DirEntry> = (0..CHUNK_ENTRIES.saturating_mul(2).saturating_add(1))
            .map(|i| DirEntry {
                name: format!("file_{i}"),
                entry_type: "-".to_owned(),
                size: 0,
                mtime: String::new(),
                mode: String::new(),
            })
            .collect();

        let chunks = encode_dir_chunks(7, &entries);

        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks.iter().map(|c| c.chunk_no).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(chunks.iter().all(|c| c.dir_path_id == 7));
        assert_eq!(
            chunks
                .iter()
                .map(|c| codec::decode(&c.entries).unwrap().len())
                .sum::<usize>(),
            entries.len()
        );
    }

    #[test]
    fn a_directory_that_fits_produces_one_chunk() {
        let entries = vec![DirEntry {
            name: "only".to_owned(),
            entry_type: "-".to_owned(),
            size: 1,
            mtime: String::new(),
            mode: String::new(),
        }];

        assert_eq!(encode_dir_chunks(1, &entries).len(), 1);
    }
}
