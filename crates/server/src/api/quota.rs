// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;

use super::auth::{AuthUser, Role};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpsertQuotaRequest {
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub enabled: bool,
}

async fn require_operator_or_admin(state: &AppState, auth: &AuthUser) -> Result<(), ApiError> {
    if auth.role == Role::Admin {
        return Ok(());
    }

    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if effective.can_view_all_repos {
        return Ok(());
    }

    Err(ApiError::Forbidden("operator access required".to_owned()))
}

#[utoipa::path(
    get,
    path = "/api/repos/{id}/quota",
    tag = "Quota",
    operation_id = "getRepoQuota",
    summary = "Get a repository quota configuration",
    params(("id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Quota configuration", body = crate::db::quota::RepoQuota),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Quota not configured"),
    )
)]
pub async fn get_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<crate::db::quota::RepoQuota>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota = db::quota::get_quota(&state.pool, repo_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("quota for repo {repo_id} not found")))?;

    Ok(Json(quota))
}

#[utoipa::path(
    put,
    path = "/api/repos/{id}/quota",
    tag = "Quota",
    operation_id = "upsertRepoQuota",
    summary = "Create or update a repository quota configuration",
    params(("id" = i64, Path, description = "Repository ID")),
    request_body = UpsertQuotaRequest,
    responses(
        (status = 200, description = "Quota configuration", body = crate::db::quota::RepoQuota),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn upsert_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpsertQuotaRequest>,
) -> Result<Json<crate::db::quota::RepoQuota>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota = db::quota::upsert_quota(
        &state.pool,
        repo_id,
        req.warn_bytes,
        req.critical_bytes,
        req.enabled,
    )
    .await?;

    Ok(Json(quota))
}
