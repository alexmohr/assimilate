// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use serde::{Deserialize, Serialize};

use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{AppState, api::archives::get_repo_env, borg::Borg, error::ApiError};

const DIFF_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_WAIT_SECS: &str = "60";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DiffResponse {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    pub total_changes: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub archive1: String,
    pub archive2: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangeCategory {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BorgDiffChangeType {
    Added,
    Removed,
    Modified,
    ModeChanged,
    OwnerChanged,
    LinkTargetChanged,
    TimeChanged,
    Unknown(String),
}

impl From<&str> for BorgDiffChangeType {
    fn from(value: &str) -> Self {
        match value {
            "added" => Self::Added,
            "removed" => Self::Removed,
            "modified" => Self::Modified,
            "mode changed" => Self::ModeChanged,
            "owner changed" => Self::OwnerChanged,
            "link target changed" => Self::LinkTargetChanged,
            "time changed" => Self::TimeChanged,
            other => Self::Unknown(other.to_string()),
        }
    }
}

fn classify_change(change_type: &BorgDiffChangeType) -> ChangeCategory {
    match change_type {
        BorgDiffChangeType::Added => ChangeCategory::Added,
        BorgDiffChangeType::Removed => ChangeCategory::Removed,
        BorgDiffChangeType::Modified
        | BorgDiffChangeType::ModeChanged
        | BorgDiffChangeType::OwnerChanged
        | BorgDiffChangeType::LinkTargetChanged
        | BorgDiffChangeType::TimeChanged
        | BorgDiffChangeType::Unknown(_) => ChangeCategory::Modified,
    }
}

fn classify_borg_diff_error(exit_code: i32, stderr: &str) -> ApiError {
    if stderr.contains("Archive") && stderr.contains("does not exist") {
        return ApiError::NotFound(format!("archive not found: {stderr}"));
    }
    if exit_code == 1 && stderr.to_lowercase().contains("lock") {
        return ApiError::Conflict("repository is locked by another operation".to_string());
    }
    if stderr.contains("Connection refused")
        || stderr.contains("Connection timed out")
        || stderr.contains("ssh: connect to host")
        || stderr.contains("Could not resolve hostname")
    {
        return ApiError::BadGateway(format!("SSH connection failed: {stderr}"));
    }
    ApiError::Internal(format!("borg diff failed (exit {exit_code}): {stderr}"))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/diff",
    tag = "Archives",
    operation_id = "diffArchives",
    summary = "Diff two archives in a repository",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive1" = String, Query, description = "First archive name"),
        ("archive2" = String, Query, description = "Second archive name"),
        ("limit" = Option<usize>, Query,
            description = "Max entries to return per category (default: 100)"),
        ("offset" = Option<usize>, Query,
            description = "Offset into combined sorted change list (default: 0)"),
    ),
    responses(
        (status = 200, description = "Diff result", body = DiffResponse),
        (status = 400, description = "archive1 and archive2 are the same"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
pub async fn diff_archives(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(repo_id): AxumPath<i64>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<DiffResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;

    if query.archive1 == query.archive2 {
        return Err(ApiError::BadRequest(
            "archive1 and archive2 must be different".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let repo_archive1 = format!("{borg_repo}::{}", query.archive1);

    let mut child = Borg::new()
        .spawn(
            &[
                "diff",
                "--json-lines",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--",
                repo_archive1.as_str(),
                query.archive2.as_str(),
            ],
            &env,
        )
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let output = tokio::time::timeout(DIFF_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ApiError::Internal("borg diff timed out".to_string()))?
        .map_err(|e| ApiError::Internal(format!("failed to wait for borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_diff_error(code, &stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut all_entries: Vec<(String, ChangeCategory)> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .inspect_err(|e| {
                    tracing::trace!(error = %e, "skipping unparseable borg diff line");
                })
                .ok()?;
            let path = v
                .get("path")
                .and_then(serde_json::Value::as_str)?
                .to_string();
            let category = v
                .get("changes")
                .and_then(serde_json::Value::as_array)
                .and_then(|changes| changes.first())
                .and_then(|c| c.get("type"))
                .and_then(serde_json::Value::as_str)
                .map_or(ChangeCategory::Modified, |change_type| {
                    classify_change(&BorgDiffChangeType::from(change_type))
                });
            Some((path, category))
        })
        .collect();

    all_entries.sort_by(|a, b| a.0.cmp(&b.0));

    let total_changes = all_entries.len();

    let page: Vec<_> = all_entries.into_iter().skip(offset).take(limit).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    for (path, category) in page {
        match category {
            ChangeCategory::Added => added.push(path),
            ChangeCategory::Removed => removed.push(path),
            ChangeCategory::Modified => modified.push(path),
        }
    }

    Ok(Json(DiffResponse {
        added,
        removed,
        modified,
        total_changes,
        limit,
        offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_known_types() {
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("added")),
            ChangeCategory::Added
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("removed")),
            ChangeCategory::Removed
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("modified")),
            ChangeCategory::Modified
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("mode changed")),
            ChangeCategory::Modified
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("owner changed")),
            ChangeCategory::Modified
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("link target changed")),
            ChangeCategory::Modified
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("time changed")),
            ChangeCategory::Modified
        );
        assert_eq!(
            classify_change(&BorgDiffChangeType::from("unknown type")),
            ChangeCategory::Modified
        );
    }

    #[test]
    fn parse_diff_lines() {
        let lines = [
            r#"{"path": "etc/hosts", "changes": [{"type": "modified"}]}"#,
            r#"{"path": "etc/new-file", "changes": [{"type": "added"}]}"#,
            r#"{"path": "etc/old-file", "changes": [{"type": "removed"}]}"#,
        ];

        let mut entries: Vec<(String, ChangeCategory)> = lines
            .iter()
            .filter_map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).ok()?;
                let path = v
                    .get("path")
                    .and_then(serde_json::Value::as_str)?
                    .to_string();
                let category = v
                    .get("changes")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|c| c.first())
                    .and_then(|c| c.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .map_or(ChangeCategory::Modified, |change_type| {
                        classify_change(&BorgDiffChangeType::from(change_type))
                    });
                Some((path, category))
            })
            .collect();

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0],
            ("etc/hosts".to_string(), ChangeCategory::Modified)
        );
        assert_eq!(
            entries[1],
            ("etc/new-file".to_string(), ChangeCategory::Added)
        );
        assert_eq!(
            entries[2],
            ("etc/old-file".to_string(), ChangeCategory::Removed)
        );
    }
}
