// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, time::Duration};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    archives::{LOCK_WAIT_SECS, classify_borg_error, get_repo_env},
    auth::AuthUser,
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, error::ApiError};

const SEARCH_TIMEOUT: Duration = Duration::from_mins(1);
const PER_ARCHIVE_TIMEOUT: Duration = Duration::from_mins(1);
const OVERALL_TIMEOUT: Duration = Duration::from_mins(5);
const DEFAULT_MAX_ARCHIVES: usize = 20;
const MAX_ARCHIVES_CAP: usize = 100;

/// Query parameters for searching files within an archive.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Glob pattern to match file paths.
    pub pattern: String,
    /// Optional path prefix to filter results.
    pub path_prefix: Option<String>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Number of results to skip.
    pub offset: Option<usize>,
}

/// A single file entry matching a search query.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SearchEntry {
    /// Relative file path.
    pub path: String,
    /// File size in bytes.
    pub size: i64,
    /// Last modification timestamp.
    pub mtime: DateTime<Utc>,
    /// Entry type ("-" for file, "d" for directory).
    #[serde(rename = "type")]
    pub entry_type: String,
}

/// Paginated search results for a single archive.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SearchResponse {
    /// Matching file entries.
    pub items: Vec<SearchEntry>,
    /// Total matching entries before pagination.
    pub total_matched: usize,
    /// Maximum entries returned.
    pub limit: usize,
    /// Number of entries skipped.
    pub offset: usize,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/search",
    tag = "Archives",
    operation_id = "searchArchive",
    summary = "Search files in an archive by glob pattern",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        ("pattern" = String, Query, description = "Glob pattern to match files"),
        ("path_prefix" = Option<String>, Query,
            description = "Filter results to paths starting with this prefix"),
        ("limit" = Option<usize>, Query,
            description = "Max entries to return (default: 100, max: 1000)"),
        ("offset" = Option<usize>, Query, description = "Number of entries to skip (default: 0)"),
    ),
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 400, description = "Empty pattern"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::BadGateway`]: the upstream operation (e.g. SSH or borg) fails
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn search_archive(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;

    if query.pattern.is_empty() {
        return Err(ApiError::BadRequest(
            "pattern must not be empty".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(100).min(1000);
    let offset = query.offset.unwrap_or(0);

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let borg_pattern = format!("sh:{}", query.pattern);

    let output = tokio::time::timeout(
        SEARCH_TIMEOUT,
        Borg::new().run(
            &[
                "list",
                "--json-lines",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--pattern",
                borg_pattern.as_str(),
                "--",
                repo_archive.as_str(),
            ],
            &env,
        ),
    )
    .await
    .map_err(|_| ApiError::BadGateway("borg search timed out after 60s".to_string()))?
    .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);

        if stderr.contains("Archive") && stderr.contains("does not exist") {
            return Err(ApiError::NotFound(format!(
                "archive '{archive_name}' not found"
            )));
        }

        return Err(classify_borg_error(code, &stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all_entries: Vec<SearchEntry> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .inspect_err(|e| {
                    tracing::trace!(error = %e, "skipping unparseable borg output line");
                })
                .ok()?;

            let path = v
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();

            if let Some(ref prefix) = query.path_prefix
                && !path.starts_with(prefix.as_str())
            {
                return None;
            }

            let mtime_str = v
                .get("mtime")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let mtime = DateTime::parse_from_rfc3339(mtime_str)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| mtime_str.parse::<DateTime<Utc>>())
                .unwrap_or_default();

            Some(SearchEntry {
                path,
                size: v
                    .get("size")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                mtime,
                entry_type: v
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();

    let total_matched = all_entries.len();
    let items: Vec<SearchEntry> = all_entries.into_iter().skip(offset).take(limit).collect();

    Ok(Json(SearchResponse {
        items,
        total_matched,
        limit,
        offset,
    }))
}

/// Query parameters for cross-archive search.
#[derive(Debug, Deserialize)]
pub struct CrossSearchQuery {
    /// Glob pattern to match file paths.
    pub pattern: String,
    /// Maximum number of archives to search (default 20, max 100).
    pub max_archives: Option<usize>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Number of results to skip.
    pub offset: Option<usize>,
}

/// A file entry in a cross-archive search result, annotated with the archive name.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CrossSearchEntry {
    /// Relative file path.
    pub path: String,
    /// File size in bytes.
    pub size: i64,
    /// Last modification timestamp.
    pub mtime: DateTime<Utc>,
    /// Entry type ("-" for file, "d" for directory).
    #[serde(rename = "type")]
    pub entry_type: String,
    /// Name of the archive containing this file.
    pub archive_name: String,
}

/// Paginated cross-archive search results.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CrossSearchResponse {
    /// Matching file entries.
    pub items: Vec<CrossSearchEntry>,
    /// Number of archives searched.
    pub total_archives_searched: usize,
    /// Maximum entries returned.
    pub limit: usize,
    /// Number of entries skipped.
    pub offset: usize,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/search",
    tag = "Search",
    operation_id = "crossArchiveSearch",
    summary = "Search for files across multiple archives (most recent first)",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("pattern" = String, Query, description = "Glob pattern to search for"),
        ("max_archives" = Option<usize>, Query,
            description = "Max archives to search (default 20, max 100)"),
        ("limit" = Option<usize>, Query, description = "Max results to return (default 200)"),
        ("offset" = Option<usize>, Query, description = "Offset for pagination (default 0)"),
    ),
    responses(
        (status = 200, description = "Search results", body = CrossSearchResponse),
        (status = 400, description = "Invalid pattern"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn cross_archive_search(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(repo_id): AxumPath<i64>,
    Query(query): Query<CrossSearchQuery>,
) -> Result<Json<CrossSearchResponse>, ApiError> {
    if query.pattern.is_empty() {
        return Err(ApiError::BadRequest(
            "pattern must not be empty".to_string(),
        ));
    }

    let max_archives = query
        .max_archives
        .unwrap_or(DEFAULT_MAX_ARCHIVES)
        .min(MAX_ARCHIVES_CAP);
    let limit = query.limit.unwrap_or(200).min(1000);
    let offset = query.offset.unwrap_or(0);

    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let archives = list_archives_sorted(&borg_repo, &env).await?;
    let archives_to_search: Vec<&ArchiveEntryBrief> = archives.iter().take(max_archives).collect();
    let total_archives_searched = archives_to_search.len();

    let overall_deadline = tokio::time::Instant::now()
        .checked_add(OVERALL_TIMEOUT)
        .unwrap_or_else(tokio::time::Instant::now);
    let mut seen: HashMap<String, CrossSearchEntry> = HashMap::new();

    let borg_pattern = format!("sh:{}", query.pattern);

    for archive in &archives_to_search {
        if tokio::time::Instant::now() >= overall_deadline {
            break;
        }

        let entries = search_in_archive(&borg_repo, &archive.name, &borg_pattern, &env).await?;

        for entry in entries {
            seen.entry(entry.path.clone()).or_insert(entry);
        }
    }

    let mut items: Vec<CrossSearchEntry> = seen.into_values().collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));

    let items: Vec<CrossSearchEntry> = items.into_iter().skip(offset).take(limit).collect();

    Ok(Json(CrossSearchResponse {
        items,
        total_archives_searched,
        limit,
        offset,
    }))
}

#[derive(Debug)]
struct ArchiveEntryBrief {
    name: String,
    start: String,
}

async fn list_archives_sorted(
    borg_repo: &str,
    env: &HashMap<String, String>,
) -> Result<Vec<ArchiveEntryBrief>, ApiError> {
    let output = Borg::new()
        .run(
            &[
                "list",
                "--json",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--",
                borg_repo,
            ],
            env,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg output: {e}")))?;

    let mut archives: Vec<ArchiveEntryBrief> = json_output
        .get("archives")
        .and_then(serde_json::Value::as_array)
        .map_or_else(Vec::new, |arr| {
            arr.iter()
                .map(|a| ArchiveEntryBrief {
                    name: a
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    start: a
                        .get("start")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        });

    // Sort by start time descending (most recent first)
    archives.sort_by(|a, b| b.start.cmp(&a.start));

    Ok(archives)
}

async fn search_in_archive(
    borg_repo: &str,
    archive_name: &str,
    borg_pattern: &str,
    env: &HashMap<String, String>,
) -> Result<Vec<CrossSearchEntry>, ApiError> {
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let result = tokio::time::timeout(
        PER_ARCHIVE_TIMEOUT,
        Borg::new().run(
            &[
                "list",
                "--json-lines",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--pattern",
                borg_pattern,
                "--",
                repo_archive.as_str(),
            ],
            env,
        ),
    )
    .await;

    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(ApiError::Internal(format!("failed to execute borg: {e}")));
        }
        Err(_) => {
            return Err(ApiError::BadGateway(format!(
                "search timed out for archive '{archive_name}'"
            )));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<CrossSearchEntry> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .inspect_err(|e| {
                    tracing::trace!(error = %e, "skipping unparseable borg output line");
                })
                .ok()?;

            let mtime_str = v
                .get("mtime")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let mtime = DateTime::parse_from_rfc3339(mtime_str)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| mtime_str.parse::<DateTime<Utc>>())
                .unwrap_or_default();

            Some(CrossSearchEntry {
                path: v
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                size: v
                    .get("size")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                mtime,
                entry_type: v
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                archive_name: archive_name.to_string(),
            })
        })
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pattern_is_rejected() {
        let query = SearchQuery {
            pattern: String::new(),
            path_prefix: None,
            limit: None,
            offset: None,
        };
        assert!(query.pattern.is_empty());
    }

    #[test]
    fn pagination_defaults() {
        let query = SearchQuery {
            pattern: "*.txt".to_string(),
            path_prefix: None,
            limit: None,
            offset: None,
        };
        let limit = query.limit.unwrap_or(100).min(1000);
        let offset = query.offset.unwrap_or(0);
        assert_eq!(limit, 100);
        assert_eq!(offset, 0);
    }

    #[test]
    fn limit_capped_at_1000() {
        let query = SearchQuery {
            pattern: "*.txt".to_string(),
            path_prefix: None,
            limit: Some(5000),
            offset: None,
        };
        let limit = query.limit.unwrap_or(100).min(1000);
        assert_eq!(limit, 1000);
    }

    #[test]
    fn search_entry_serialization() {
        let entry = SearchEntry {
            path: "etc/hosts".to_string(),
            size: 220,
            mtime: DateTime::default(),
            entry_type: "r".to_string(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json.get("path").unwrap(), "etc/hosts");
        assert_eq!(json.get("size").unwrap(), 220);
        assert_eq!(json.get("type").unwrap(), "r");
    }
}
