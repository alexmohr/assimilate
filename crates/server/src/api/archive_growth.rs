// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use shared::archive_growth::ArchiveGrowthResponse;

use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{AppState, archive_growth, error::ApiError};

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/growth",
    tag = "Archives",
    operation_id = "getArchiveGrowth",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "The breakdown, or where working it out stands",
            body = ArchiveGrowthResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Where an archive's deduplicated size sits: per folder and per file, the
/// data no other archive in the repository holds.
///
/// The first request starts working it out in the background and answers
/// `pending`; poll until the status is `done` or `failed`. The breakdown is
/// kept, and reflects the repository at `computed_at`: archives written since
/// that keep the same data share it, so recomputing can show less.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Forbidden`]: the user may not view the repository
/// - [`ApiError::Database`]: a query fails
pub async fn get_archive_growth(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<ArchiveGrowthResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    archive_growth::ensure(&state, repo_id, &archive_name)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/growth",
    tag = "Archives",
    operation_id = "recomputeArchiveGrowth",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 202, description = "Recomputing, or already under way",
            body = ArchiveGrowthResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Work an archive's breakdown out again against the repository as it is
/// now. Does nothing while it is already being worked out.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Forbidden`]: the user may not view the repository
/// - [`ApiError::Database`]: a query fails
pub async fn recompute_archive_growth(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<(StatusCode, Json<ArchiveGrowthResponse>), ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    archive_growth::recompute(&state, repo_id, &archive_name)
        .await
        .map(|response| (StatusCode::ACCEPTED, Json(response)))
}
