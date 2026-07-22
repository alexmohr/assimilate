// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::responses::{GroupResponse, RoleResponse};

use super::auth::{AuthUser, RequireAdmin};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::GroupRow> for GroupResponse {
    fn from(g: db::GroupRow) -> Self {
        Self {
            id: g.id,
            name: g.name,
            description: g.description,
            created_at: g.created_at,
        }
    }
}

impl From<db::RoleRow> for RoleResponse {
    fn from(r: db::RoleRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            created_at: r.created_at,
            can_create_agent: r.can_create_agent,
            can_delete_agent: r.can_delete_agent,
            can_delete_own_agent: r.can_delete_own_agent,
            can_create_repo: r.can_create_repo,
            can_delete_repo: r.can_delete_repo,
            can_delete_own_repo: r.can_delete_own_repo,
            can_create_schedule: r.can_create_schedule,
            can_delete_schedule: r.can_delete_schedule,
            can_delete_own_schedule: r.can_delete_own_schedule,
            can_manage_tags: r.can_manage_tags,
            can_view_all_repos: r.can_view_all_repos,
            can_manage_tunnels: r.can_manage_tunnels,
            can_upgrade_agent: r.can_upgrade_agent,
        }
    }
}

/// Request payload for creating a new group.
#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    /// Group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
}

/// Request payload for updating a group.
#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    /// Updated group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
}

/// Request payload for setting group membership.
#[derive(Debug, Deserialize)]
pub struct SetGroupMembersRequest {
    /// User IDs to include in the group.
    pub user_ids: Vec<i64>,
}

/// Shared permission fields used in role create/update payloads.
#[derive(Debug, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RolePermissionFields {
    /// Permission to create agents.
    pub can_create_agent: bool,
    /// Permission to delete any agent.
    pub can_delete_agent: bool,
    /// Permission to delete own agents.
    pub can_delete_own_agent: bool,
    /// Permission to create repositories.
    pub can_create_repo: bool,
    /// Permission to delete any repository.
    pub can_delete_repo: bool,
    /// Permission to delete own repositories.
    pub can_delete_own_repo: bool,
    /// Permission to create schedules.
    pub can_create_schedule: bool,
    /// Permission to delete any schedule.
    pub can_delete_schedule: bool,
    /// Permission to delete own schedules.
    pub can_delete_own_schedule: bool,
    /// Permission to manage tags.
    pub can_manage_tags: bool,
    /// Permission to view all repositories.
    pub can_view_all_repos: bool,
    /// Permission to manage tunnels.
    pub can_manage_tunnels: bool,
    /// Permission to upgrade agents.
    pub can_upgrade_agent: bool,
}

/// Request payload for creating a new role.
#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    /// Role name.
    pub name: String,
    /// Role permissions (flattened so they appear at the same JSON level as `name`).
    #[serde(flatten)]
    pub perms: RolePermissionFields,
}

/// Request payload for updating a role. Shares the same shape as
/// [`CreateRoleRequest`] - both are flattened around [`RolePermissionFields`].
pub type UpdateRoleRequest = CreateRoleRequest;

/// Request payload for setting a user's role assignments.
#[derive(Debug, Deserialize)]
pub struct SetUserRolesRequest {
    /// Role IDs to assign to the user.
    pub role_ids: Vec<i64>,
}

/// List all groups (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let groups: Vec<GroupResponse> = db::list_groups(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(groups))
}

/// Create a new group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the group name is empty.
pub async fn create_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateGroupRequest>,
) -> Result<(StatusCode, Json<GroupResponse>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "group name must not be empty".to_string(),
        ));
    }
    let group: GroupResponse = db::insert_group(&state.pool, name, req.description.as_deref())
        .await?
        .into();
    Ok((StatusCode::CREATED, Json(group)))
}

/// Update a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the updated name is empty.
pub async fn update_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateGroupRequest>,
) -> Result<Json<GroupResponse>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "group name must not be empty".to_string(),
        ));
    }
    let group: GroupResponse = db::update_group(&state.pool, id, name, req.description.as_deref())
        .await?
        .into();
    Ok(Json(group))
}

/// Delete a group (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn delete_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::delete_group(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List members of a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn list_group_members(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<shared::responses::GroupMembersResponse>, ApiError> {
    let group = db::get_group(&state.pool, id).await?;
    if group.is_none() {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    let user_ids = db::list_group_members(&state.pool, id).await?;
    Ok(Json(shared::responses::GroupMembersResponse { user_ids }))
}

/// Set the member list of a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn set_group_members(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<SetGroupMembersRequest>,
) -> Result<StatusCode, ApiError> {
    let group = db::get_group(&state.pool, id).await?;
    if group.is_none() {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    db::set_group_members(&state.pool, id, &req.user_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List all roles (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_roles(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(roles))
}

/// Validate role name and build [`InsertRoleParams`] from a request with [`RolePermissionFields`].
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the role name is empty.
fn build_role_params<'a>(
    name: &'a str,
    perms: &'a RolePermissionFields,
) -> Result<db::InsertRoleParams<'a>, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    Ok(db::InsertRoleParams {
        name,
        can_create_agent: perms.can_create_agent,
        can_delete_agent: perms.can_delete_agent,
        can_delete_own_agent: perms.can_delete_own_agent,
        can_create_repo: perms.can_create_repo,
        can_delete_repo: perms.can_delete_repo,
        can_delete_own_repo: perms.can_delete_own_repo,
        can_create_schedule: perms.can_create_schedule,
        can_delete_schedule: perms.can_delete_schedule,
        can_delete_own_schedule: perms.can_delete_own_schedule,
        can_manage_tags: perms.can_manage_tags,
        can_view_all_repos: perms.can_view_all_repos,
        can_manage_tunnels: perms.can_manage_tunnels,
        can_upgrade_agent: perms.can_upgrade_agent,
    })
}

/// Create a new role (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the role name is empty.
pub async fn create_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleResponse>), ApiError> {
    let params = build_role_params(&req.name, &req.perms)?;
    let role: RoleResponse = db::insert_role(&state.pool, &params).await?.into();
    Ok((StatusCode::CREATED, Json(role)))
}

/// Update a role (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the updated name is empty.
pub async fn update_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRoleRequest>,
) -> Result<Json<RoleResponse>, ApiError> {
    let params = build_role_params(&req.name, &req.perms)?;
    let role: RoleResponse = db::update_role(&state.pool, id, &params).await?.into();
    Ok(Json(role))
}

const PROTECTED_ROLE_NAMES: &[&str] = &["admin", "operator", "viewer"];

/// Delete a role (admin only). Built-in roles cannot be deleted.
/// # Errors
/// [`ApiError::NotFound`] if the role does not exist.
/// [`ApiError::BadRequest`] if the role is built-in.
pub async fn delete_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let role = db::get_role(&state.pool, id).await?;
    let Some(role) = role else {
        return Err(ApiError::NotFound(format!("role {id} not found")));
    };
    if PROTECTED_ROLE_NAMES.contains(&role.name.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "cannot delete built-in role '{}'",
            role.name
        )));
    }
    db::delete_role(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List roles assigned to a user (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_user_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_user_roles(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(roles))
}

/// Set roles for a user (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn set_user_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
    ApiJson(req): ApiJson<SetUserRolesRequest>,
) -> Result<StatusCode, ApiError> {
    db::set_user_roles(&state.pool, user_id, &req.role_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List groups a user belongs to (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_user_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let groups: Vec<GroupResponse> = db::list_user_groups(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(groups))
}

/// Get effective permissions for a user. Admins see any user; users see only themselves.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller lacks permission.
pub async fn get_effective_permissions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<Json<RoleResponse>, ApiError> {
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if !effective.can_delete_repo && auth.user_id != user_id {
        return Err(ApiError::Forbidden(
            "admin access required or must be own user".to_string(),
        ));
    }
    let perms: RoleResponse = db::get_effective_permissions(&state.pool, user_id)
        .await?
        .into();
    Ok(Json(perms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_role_request_includes_can_upgrade_agent() {
        let json = serde_json::json!({
            "name": "custom-role",
            "can_create_agent": true,
            "can_delete_agent": false,
            "can_delete_own_agent": true,
            "can_create_repo": false,
            "can_delete_repo": true,
            "can_delete_own_repo": false,
            "can_create_schedule": true,
            "can_delete_schedule": false,
            "can_delete_own_schedule": true,
            "can_manage_tags": false,
            "can_view_all_repos": true,
            "can_manage_tunnels": false,
            "can_upgrade_agent": true
        });
        let req: CreateRoleRequest = serde_json::from_value(json).unwrap();
        assert!(req.perms.can_upgrade_agent);
        assert_eq!(req.name, "custom-role");
        assert!(req.perms.can_create_agent);
    }

    #[test]
    fn update_role_request_includes_can_upgrade_agent() {
        let json = serde_json::json!({
            "name": "updated-role",
            "can_create_agent": false,
            "can_delete_agent": true,
            "can_delete_own_agent": false,
            "can_create_repo": true,
            "can_delete_repo": false,
            "can_delete_own_repo": true,
            "can_create_schedule": false,
            "can_delete_schedule": true,
            "can_delete_own_schedule": false,
            "can_manage_tags": true,
            "can_view_all_repos": false,
            "can_manage_tunnels": true,
            "can_upgrade_agent": true
        });
        let req: UpdateRoleRequest = serde_json::from_value(json).unwrap();
        assert!(req.perms.can_upgrade_agent);
        assert_eq!(req.name, "updated-role");
        assert!(!req.perms.can_create_agent);
    }
}
