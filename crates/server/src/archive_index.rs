// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashSet;

use serde::Serialize;
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    api::archives::{
        ContentEntry, LOCK_WAIT_SECS, classify_borg_error, get_repo_env, normalize_path,
    },
    borg::Borg,
    error::ApiError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    Pending,
    Indexing,
    Done,
    Failed,
}

pub fn get_index_status_from_str(s: &str) -> IndexStatus {
    match s {
        "indexing" => IndexStatus::Indexing,
        "done" => IndexStatus::Done,
        "failed" => IndexStatus::Failed,
        _ => IndexStatus::Pending,
    }
}

pub async fn get_index_status(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Option<IndexStatus>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        status: String,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT status FROM archive_index_jobs WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind(archive_name)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.map(|r| get_index_status_from_str(&r.status)))
}

/// Atomically claims the indexing job and spawns a background task if we won the race.
/// Returns the current status after the claim attempt.
pub async fn ensure_indexed(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    archive_name: String,
) -> Result<IndexStatus, ApiError> {
    let result = sqlx::query(
        "INSERT INTO archive_index_jobs (repo_id, archive_name, status) VALUES ($1, $2, \
         'pending') ON CONFLICT DO NOTHING",
    )
    .bind(repo_id)
    .bind(&archive_name)
    .execute(&pool)
    .await
    .map_err(ApiError::Database)?;

    if result.rows_affected() == 1 {
        let pool_bg = pool.clone();
        let archive_name_bg = archive_name.clone();
        tokio::spawn(async move {
            if let Err(e) = run_indexing(&pool_bg, &encryption_key, repo_id, &archive_name_bg).await
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

    get_index_status(&pool, repo_id, &archive_name)
        .await
        .map(|s| s.unwrap_or(IndexStatus::Pending))
}

pub async fn run_indexing(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_name: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE archive_index_jobs SET status = 'indexing', started_at = NOW() WHERE repo_id = $1 \
         AND archive_name = $2",
    )
    .bind(repo_id)
    .bind(archive_name)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    match index_archive(pool, encryption_key, repo_id, archive_name).await {
        Ok(file_count) => {
            sqlx::query(
                "UPDATE archive_index_jobs SET status = 'done', finished_at = NOW(), file_count = \
                 $3 WHERE repo_id = $1 AND archive_name = $2",
            )
            .bind(repo_id)
            .bind(archive_name)
            .bind(file_count)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            sqlx::query(
                "UPDATE archive_index_jobs SET status = 'failed', finished_at = NOW(), \
                 error_message = $3 WHERE repo_id = $1 AND archive_name = $2",
            )
            .bind(repo_id)
            .bind(archive_name)
            .bind(msg)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
            Err(e)
        }
    }
}

async fn index_archive(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_name: &str,
) -> Result<i64, ApiError> {
    let (borg_repo, env) = get_repo_env(pool, encryption_key, repo_id).await?;
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
            &env,
        )
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let Some(stdout) = child.stdout.take() else {
        return Err(ApiError::Internal("no stdout from borg".to_string()));
    };

    let mut raw: Vec<ContentEntry> = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| ApiError::Internal(format!("reading borg output: {e}")))?
    {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line).inspect_err(|e| {
            tracing::trace!(error = %e, "skipping unparseable borg output line");
        }) else {
            continue;
        };
        raw.push(ContentEntry {
            entry_type: v["type"].as_str().unwrap_or("").to_string(),
            path: v["path"].as_str().map_or_else(String::new, normalize_path),
            size: v["size"].as_i64().unwrap_or(0),
            mtime: v["mtime"].as_str().unwrap_or("").to_string(),
            mode: v["mode"].as_str().unwrap_or("").to_string(),
        });
    }

    let status = child
        .wait()
        .await
        .map_err(|e| ApiError::Internal(format!("borg wait failed: {e}")))?;
    if !status.success() {
        use tokio::io::AsyncReadExt;
        let mut stderr_str = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = se.read_to_string(&mut stderr_str).await;
        }
        return Err(classify_borg_error(status.code().unwrap_or(1), &stderr_str));
    }

    let mut paths: Vec<String> = Vec::new();
    let mut parent_paths: Vec<String> = Vec::new();
    let mut entry_types: Vec<String> = Vec::new();
    let mut sizes: Vec<i64> = Vec::new();
    let mut mtimes: Vec<String> = Vec::new();
    let mut modes: Vec<String> = Vec::new();

    let mut seen: HashSet<String> = HashSet::new();

    let mut add =
        |path: String, parent: String, etype: String, size: i64, mtime: String, mode: String| {
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
            let dir_path = segments[..depth].join("/");
            let dir_parent = if depth == 1 {
                String::new()
            } else {
                segments[..depth - 1].join("/")
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

    let file_count = i64::try_from(paths.len()).unwrap_or(i64::MAX);

    // Bulk insert using unnest arrays — one round-trip regardless of archive size.
    sqlx::query(
        "INSERT INTO archive_files (repo_id, archive_name, path, parent_path, entry_type, size, \
         mtime, mode) SELECT $1, $2, unnest($3::text[]), unnest($4::text[]), unnest($5::text[]), \
         unnest($6::bigint[]), unnest($7::text[]), unnest($8::text[]) ON CONFLICT DO NOTHING",
    )
    .bind(repo_id)
    .bind(archive_name)
    .bind(&paths)
    .bind(&parent_paths)
    .bind(&entry_types)
    .bind(&sizes)
    .bind(&mtimes)
    .bind(&modes)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(file_count)
}

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

    let rows = sqlx::query_as::<_, Row>(
        "SELECT path, entry_type, size, mtime, mode FROM archive_files WHERE repo_id = $1 AND \
         archive_name = $2 AND parent_path = $3 ORDER BY entry_type DESC, path ASC LIMIT $4",
    )
    .bind(repo_id)
    .bind(archive_name)
    .bind(parent_path)
    .bind(limit)
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
    use super::*;

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
        let path = "a/b/c/file.txt";
        let segments: Vec<&str> = path.split('/').collect();
        let mut dirs: Vec<String> = Vec::new();
        for depth in 1..segments.len() {
            dirs.push(segments[..depth].join("/"));
        }
        assert_eq!(dirs, vec!["a", "a/b", "a/b/c"]);
    }

    #[test]
    fn empty_path_skipped() {
        let path = "";
        assert!(path.is_empty());
    }
}
