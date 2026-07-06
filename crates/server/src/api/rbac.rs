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
        }
    }
}

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
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct CreateRoleRequest {
    pub name: String,
    pub can_create_agent: bool,
    pub can_delete_agent: bool,
    pub can_delete_own_agent: bool,
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct UpdateRoleRequest {
    pub name: String,
    pub can_create_agent: bool,
    pub can_delete_agent: bool,
    pub can_delete_own_agent: bool,
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
) -> Result<Json<shared::responses::GroupMembersResponse>, ApiError> {
    let group = db::get_group(&state.pool, id).await?;
    if group.is_none() {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    let user_ids = db::list_group_members(&state.pool, id).await?;
    Ok(Json(shared::responses::GroupMembersResponse { user_ids }))
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
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_roles(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(roles))
}

pub async fn create_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleResponse>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    let params = db::InsertRoleParams {
        name,
        can_create_agent: req.can_create_agent,
        can_delete_agent: req.can_delete_agent,
        can_delete_own_agent: req.can_delete_own_agent,
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
    let role: RoleResponse = db::insert_role(&state.pool, &params).await?.into();
    Ok((StatusCode::CREATED, Json(role)))
}

pub async fn update_role(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRoleRequest>,
) -> Result<Json<RoleResponse>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    let params = db::InsertRoleParams {
        name,
        can_create_agent: req.can_create_agent,
        can_delete_agent: req.can_delete_agent,
        can_delete_own_agent: req.can_delete_own_agent,
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
    let role: RoleResponse = db::update_role(&state.pool, id, &params).await?.into();
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
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_user_roles(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
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
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let groups: Vec<GroupResponse> = db::list_user_groups(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(groups))
}

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
