// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Line-by-line comparison of one file between two archives.

mod compute;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use serde::Deserialize;
use shared::responses::{FileDiffResponse, FileDiffSide};

use self::compute::{DiffScope, diff_text};
use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{
    AppState,
    api::archives::{get_repo_env, stream_extract, validate_extract_path},
    borg::Borg,
    error::ApiError,
};

/// The largest version of a file, in bytes, the server reads to compare. Both
/// versions are held in memory and their diff is returned as JSON, so this is
/// sized for configuration files and scripts, not data.
const FILE_DIFF_MAX_BYTES: usize = 1024 * 1024;

/// Query parameters for comparing one file between two archives.
#[derive(Debug, Deserialize)]
pub struct FileDiffQuery {
    /// Path of the file within both archives.
    pub path: String,
    /// The archive to compare against: the base, whose lines the diff removes.
    pub against: String,
    /// Repository `against` is in, when it is not the request path's.
    pub against_repo_id: Option<i64>,
    /// Return every line instead of only the changes and their context.
    #[serde(default)]
    pub whole_file: bool,
}

/// What reading one version of the file produced.
enum Version {
    Text(String),
    Binary,
    TooLarge,
}

impl From<Vec<u8>> for Version {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.contains(&0) {
            return Self::Binary;
        }
        String::from_utf8(bytes).map_or(Self::Binary, Self::Text)
    }
}

impl Version {
    /// The text of this version, or the response that explains why there is none.
    fn into_text(self, side: FileDiffSide) -> Result<String, FileDiffResponse> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Binary => Err(FileDiffResponse::Binary { side }),
            Self::TooLarge => Err(FileDiffResponse::TooLarge {
                side,
                limit_bytes: FILE_DIFF_MAX_BYTES,
            }),
        }
    }
}

async fn read_version(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
    path: &str,
) -> Result<Version, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let contents = stream_extract(
        &Borg::new().with_registry(state.task_registry.clone()),
        &repo_archive,
        path,
        &env,
        &state.task_registry,
    )?
    .read_capped(FILE_DIFF_MAX_BYTES)
    .await?;
    Ok(contents.map_or(Version::TooLarge, Version::from))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/file-diff",
    tag = "Archives",
    operation_id = "diffArchiveFile",
    params(
        ("repo_id" = i64, Path, description = "Repository of the target archive"),
        ("archive_name" = String, Path,
            description = "Target archive: the version the diff adds lines to"),
        ("path" = String, Query, description = "File path within both archives"),
        ("against" = String, Query,
            description = "Base archive: the version the diff removes lines from"),
        ("against_repo_id" = Option<i64>, Query,
            description = "Repository of the base archive (default: the target's)"),
        ("whole_file" = Option<bool>, Query,
            description = "Return every line, not only the changes (default: false)"),
    ),
    responses(
        (status = 200, description = "The comparison, or why the file cannot be compared",
            body = FileDiffResponse),
        (status = 400, description = "Invalid path, or the file compared with itself"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive or file not found"),
        (status = 409, description = "Repository is locked by another operation"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Compare one file between two archives, line by line.
///
/// Both versions are read with `borg extract`, so the caller needs extract
/// permission on both repositories, the same as downloading the file.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid path or a file compared
/// with itself, the permission errors of either repository, and the
/// classified borg error if reading a version failed.
pub async fn diff_file(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<FileDiffQuery>,
) -> Result<Json<FileDiffResponse>, ApiError> {
    let against_repo_id = query.against_repo_id.unwrap_or(repo_id);
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;
    if against_repo_id != repo_id {
        check_repo_permission(&state.pool, &auth, against_repo_id, |p| p.can_extract).await?;
    }
    validate_extract_path(&query.path)?;
    if against_repo_id == repo_id && query.against == archive_name {
        return Err(ApiError::BadRequest(
            "a file cannot be compared with itself; pick another archive".to_string(),
        ));
    }

    let (base, target) = tokio::try_join!(
        read_version(&state, against_repo_id, &query.against, &query.path),
        read_version(&state, repo_id, &archive_name, &query.path),
    )?;

    let texts = base.into_text(FileDiffSide::Base).and_then(|base| {
        target
            .into_text(FileDiffSide::Target)
            .map(|target| (base, target))
    });
    let response = match texts {
        Ok((base, target)) => {
            let scope = DiffScope::from(query.whole_file);
            tokio::task::spawn_blocking(move || diff_text(&base, &target, scope))
                .await
                .map_err(|e| ApiError::Internal(format!("file diff task failed: {e}")))?
        }
        Err(response) => response,
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests;
