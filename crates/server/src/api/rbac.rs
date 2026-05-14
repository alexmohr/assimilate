// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use super::auth::{AuthUser, RequireAdmin};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetGroupMembersRequest {
    pub user_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub can_create_client: bool,
    pub can_delete_client: bool,
    pub can_delete_own_client: bool,
    pub can_create_repo: bool,
    pub can_delete_repo: bool,
    pub can_delete_own_repo: bool,
    pub can_create_schedule: bool,
    pub can_delete_schedule: bool,
    pub can_delete_own_schedule: bool,
    pub can_manage_tags: bool,
    pub can_view_all_repos: bool,
    pub can_manage_tunnels: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: String,
    pub can_create_client: bool,
    pub can_delete_client: bool,
    pub can_delete_own_client: bool,
    pub can_create_repo: bool,
    pub can_delete_repo: bool,
    pub can_delete_own_repo: bool,
    pub can_create_schedule: bool,
    pub can_delete_schedule: bool,
    pub can_delete_own_schedule: bool,
    pub can_manage_tags: bool,
    pub can_view_all_repos: bool,
    pub can_manage_tunnels: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetUserRolesRequest {
    pub role_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct GroupMembersResponse {
    pub user_ids: Vec<i64>,
}

pub async fn list_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::GroupRow>>, ApiError> {
    let groups = db::list_groups(&state.pool).await?;
    Ok(Json(groups))
}

pub async fn create_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateGroupRequest>,
) -> Result<(StatusCode, Json<db::GroupRow>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "group name must not be empty".to_string(),
        ));
    }
    let group = db::insert_group(&state.pool, name, req.description.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(group)))
}

pub async fn update_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateGroupRequest>,
) -> Result<Json<db::GroupRow>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "group name must not be empty".to_string(),
        ));
    }
    let group = db::update_group(&state.pool, id, name, req.description.as_deref()).await?;
    Ok(Json(group))
}

pub async fn delete_group(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::delete_group(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_group_members(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<GroupMembersResponse>, ApiError> {
    let group = db::get_group(&state.pool, id).await?;
    if group.is_none() {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    let user_ids = db::list_group_members(&state.pool, id).await?;
    Ok(Json(GroupMembersResponse { user_ids }))
}

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

pub async fn list_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::RoleRow>>, ApiError> {
    let roles = db::list_roles(&state.pool).await?;
    Ok(Json(roles))
}

pub async fn create_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateRoleRequest>,
) -> Result<(StatusCode, Json<db::RoleRow>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    let params = db::InsertRoleParams {
        name,
        can_create_client: req.can_create_client,
        can_delete_client: req.can_delete_client,
        can_delete_own_client: req.can_delete_own_client,
        can_create_repo: req.can_create_repo,
        can_delete_repo: req.can_delete_repo,
        can_delete_own_repo: req.can_delete_own_repo,
        can_create_schedule: req.can_create_schedule,
        can_delete_schedule: req.can_delete_schedule,
        can_delete_own_schedule: req.can_delete_own_schedule,
        can_manage_tags: req.can_manage_tags,
        can_view_all_repos: req.can_view_all_repos,
        can_manage_tunnels: req.can_manage_tunnels,
    };
    let role = db::insert_role(&state.pool, &params).await?;
    Ok((StatusCode::CREATED, Json(role)))
}

pub async fn update_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRoleRequest>,
) -> Result<Json<db::RoleRow>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    let params = db::InsertRoleParams {
        name,
        can_create_client: req.can_create_client,
        can_delete_client: req.can_delete_client,
        can_delete_own_client: req.can_delete_own_client,
        can_create_repo: req.can_create_repo,
        can_delete_repo: req.can_delete_repo,
        can_delete_own_repo: req.can_delete_own_repo,
        can_create_schedule: req.can_create_schedule,
        can_delete_schedule: req.can_delete_schedule,
        can_delete_own_schedule: req.can_delete_own_schedule,
        can_manage_tags: req.can_manage_tags,
        can_view_all_repos: req.can_view_all_repos,
        can_manage_tunnels: req.can_manage_tunnels,
    };
    let role = db::update_role(&state.pool, id, &params).await?;
    Ok(Json(role))
}

const PROTECTED_ROLE_NAMES: &[&str] = &["admin", "operator", "viewer"];

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

pub async fn list_user_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<db::RoleRow>>, ApiError> {
    let roles = db::list_user_roles(&state.pool, user_id).await?;
    Ok(Json(roles))
}

pub async fn set_user_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
    ApiJson(req): ApiJson<SetUserRolesRequest>,
) -> Result<StatusCode, ApiError> {
    db::set_user_roles(&state.pool, user_id, &req.role_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_user_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<db::GroupRow>>, ApiError> {
    let groups = db::list_user_groups(&state.pool, user_id).await?;
    Ok(Json(groups))
}

pub async fn get_effective_permissions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<Json<db::RoleRow>, ApiError> {
    if auth.role != super::auth::Role::Admin && auth.user_id != user_id {
        return Err(ApiError::Forbidden(
            "admin access required or must be own user".to_string(),
        ));
    }
    let perms = db::get_effective_permissions(&state.pool, user_id).await?;
    Ok(Json(perms))
}
