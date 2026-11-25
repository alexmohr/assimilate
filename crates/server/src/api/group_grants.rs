// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A group's per-repository permissions, which every member of the group
//! holds on top of their own grants (admin only).

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use shared::{
    audit::{AuditEvent, RepoPermission},
    responses::GroupRepoPermissionResponse,
};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    auth::RequireAdmin,
};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

/// Request payload for setting what a group may do on a repository.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetGroupRepoPermissionRequest {
    /// What the group's members may do on the repository. Empty revokes
    /// the group's grant.
    pub permissions: Vec<RepoPermission>,
}

async fn require_group(state: &AppState, group_id: i64) -> Result<db::GroupRow, ApiError> {
    db::get_group(&state.pool, group_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("group {group_id} not found")))
}

#[utoipa::path(
    get,
    path = "/api/groups/{group_id}/permissions",
    tag = "Permissions",
    operation_id = "listPermissionsForGroup",
    params(("group_id" = i64, Path, description = "Group ID")),
    responses(
        (status = 200, description = "The group's repository grants",
            body = Vec<GroupRepoPermissionResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Group not found"),
    )
)]
/// List every repository grant of a group.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the group does not exist.
pub async fn list_for_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(group_id): Path<i64>,
) -> Result<Json<Vec<GroupRepoPermissionResponse>>, ApiError> {
    require_group(&state, group_id).await?;
    let grants = db::list_group_repo_permissions(&state.pool, group_id)
        .await?
        .into_iter()
        .map(|grant| GroupRepoPermissionResponse {
            group_id,
            repo_id: grant.repo_id,
            permissions: grant.permissions,
        })
        .collect();
    Ok(Json(grants))
}

#[utoipa::path(
    put,
    path = "/api/groups/{group_id}/permissions/{repo_id}",
    tag = "Permissions",
    operation_id = "setGroupPermission",
    params(
        ("group_id" = i64, Path, description = "Group ID"),
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = SetGroupRepoPermissionRequest,
    responses(
        (status = 200, description = "What the group now holds on the repository",
            body = GroupRepoPermissionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Group or repository not found"),
    )
)]
/// Set what a group's members may do on a repository.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the group or the repository does not
/// exist.
pub async fn set_for_group(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path((group_id, repo_id)): Path<(i64, i64)>,
    ApiJson(req): ApiJson<SetGroupRepoPermissionRequest>,
) -> Result<Json<GroupRepoPermissionResponse>, ApiError> {
    let group = require_group(&state, group_id).await?;
    db::get_repo_by_id(&state.pool, repo_id).await?;
    let before = db::get_group_repo_permission(&state.pool, group_id, repo_id).await?;
    db::set_group_repo_permission(&state.pool, group_id, repo_id, &req.permissions).await?;
    let after = db::get_group_repo_permission(&state.pool, group_id, repo_id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Repo(repo_id)),
        AuditEvent::SetGroupRepoPermission {
            group: group.name,
            before,
            after: after.clone(),
        },
    )
    .await;
    Ok(Json(GroupRepoPermissionResponse {
        group_id,
        repo_id,
        permissions: after,
    }))
}
