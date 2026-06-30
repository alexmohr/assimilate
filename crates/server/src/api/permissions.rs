// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use shared::responses::RepoPermissionResponse;

use super::auth::{AuthUser, RequireAdmin, Role};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::RepoPermissionRow> for RepoPermissionResponse {
    fn from(row: db::RepoPermissionRow) -> Self {
        Self {
            user_id: row.user_id,
            repo_id: row.repo_id,
            can_view: row.can_view,
            can_backup: row.can_backup,
            can_modify_schedules: row.can_modify_schedules,
            can_extract: row.can_extract,
            can_delete: row.can_delete,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpsertPermissionRequest {
    pub can_view: bool,
    pub can_backup: bool,
    pub can_modify_schedules: bool,
    pub can_extract: bool,
    pub can_delete: bool,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/permissions",
    tag = "Permissions",
    operation_id = "listPermissionsForRepo",
    summary = "List all user permissions for a repository",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "List of permissions",
            body = Vec<RepoPermissionResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_for_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<RepoPermissionResponse>>, ApiError> {
    let perms: Vec<RepoPermissionResponse> =
        db::list_repo_permissions_for_repo(&state.pool, repo_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(perms))
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/permissions/{user_id}",
    tag = "Permissions",
    operation_id = "upsertPermission",
    summary = "Set or update a user's permissions for a repository",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("user_id" = i64, Path, description = "User ID"),
    ),
    request_body = UpsertPermissionRequest,
    responses(
        (status = 200, description = "Updated permission", body = RepoPermissionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn upsert(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path((repo_id, user_id)): Path<(i64, i64)>,
    ApiJson(req): ApiJson<UpsertPermissionRequest>,
) -> Result<Json<RepoPermissionResponse>, ApiError> {
    let perm: RepoPermissionResponse = db::upsert_repo_permission(
        &state.pool,
        &db::UpsertRepoPermissionParams {
            user_id,
            repo_id,
            can_view: req.can_view,
            can_backup: req.can_backup,
            can_modify_schedules: req.can_modify_schedules,
            can_extract: req.can_extract,
            can_delete: req.can_delete,
        },
    )
    .await?
    .into();
    Ok(Json(perm))
}

#[utoipa::path(
    get,
    path = "/api/users/{id}/permissions",
    tag = "Permissions",
    operation_id = "listPermissionsForUser",
    summary = "List all repository permissions for a user",
    params(("id" = i64, Path, description = "User ID")),
    responses(
        (status = 200, description = "List of permissions",
            body = Vec<RepoPermissionResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_for_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<RepoPermissionResponse>>, ApiError> {
    let perms: Vec<RepoPermissionResponse> =
        db::list_repo_permissions_for_user(&state.pool, user_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(perms))
}

pub async fn check_repo_permission(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    repo_id: i64,
    check: impl Fn(&db::RepoPermissionRow) -> bool,
) -> Result<(), ApiError> {
    if auth.role == Role::Admin {
        return Ok(());
    }

    let perm = db::get_repo_permission(pool, auth.user_id, repo_id).await?;
    if perm.is_some_and(|p| check(&p)) {
        return Ok(());
    }

    let effective = db::get_effective_permissions(pool, auth.user_id).await?;
    if effective.can_view_all_repos {
        let view_only = db::RepoPermissionRow {
            user_id: auth.user_id,
            repo_id,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract: false,
            can_delete: false,
        };
        if check(&view_only) {
            return Ok(());
        }
    }

    Err(ApiError::Forbidden(
        "insufficient repo permission".to_string(),
    ))
}

pub async fn is_visible_to_user(
    pool: &sqlx::PgPool,
    user_id: i64,
    owner_id: Option<i64>,
    visibility: &str,
    is_admin: bool,
) -> Result<bool, ApiError> {
    if is_admin {
        return Ok(true);
    }

    if owner_id == Some(user_id) {
        return Ok(true);
    }

    if visibility == "shared" {
        if let Some(owner) = owner_id {
            return db::user_shares_group_with(pool, user_id, owner).await;
        }
        return Ok(true);
    }

    Ok(false)
}
