// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::{auth::RequireAdmin, helpers};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    pub role: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePasswordRequest {
    pub password: String,
}

#[utoipa::path(
    get,
    path = "/api/users",
    tag = "Users",
    operation_id = "list_users",
    summary = "List all users (admin only)",
    responses(
        (status = 200, description = "List of users", body = Vec<db::UserRow>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::UserRow>>, ApiError> {
    let users = db::list_users(&state.pool).await?;
    Ok(Json(users))
}

#[utoipa::path(
    post,
    path = "/api/users",
    tag = "Users",
    operation_id = "create_user",
    summary = "Create a new user (admin only)",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = db::UserRow),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<db::UserRow>), ApiError> {
    helpers::validate_non_empty(&req.username, "username")?;

    if req.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }

    validate_role(&req.role)?;

    let hash = helpers::hash_password(req.password.clone()).await?;

    let user = db::insert_user(&state.pool, &req.username, &hash, &req.role).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

#[utoipa::path(
    put,
    path = "/api/users/{user_id}/role",
    tag = "Users",
    operation_id = "update_role",
    summary = "Update a user's role (admin only)",
    params(
        ("user_id" = i64, Path, description = "User ID"),
    ),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "Updated user", body = db::UserRow),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn update_role(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(user_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRoleRequest>,
) -> Result<Json<db::UserRow>, ApiError> {
    if admin.user_id == user_id {
        return Err(ApiError::BadRequest("cannot change own role".to_string()));
    }

    validate_role(&req.role)?;

    let user = db::update_user_role(&state.pool, user_id, &req.role).await?;
    Ok(Json(user))
}

#[utoipa::path(
    put,
    path = "/api/users/{user_id}/password",
    tag = "Users",
    operation_id = "update_password",
    summary = "Update a user's password (admin only)",
    params(
        ("user_id" = i64, Path, description = "User ID"),
    ),
    request_body = UpdatePasswordRequest,
    responses(
        (status = 204, description = "Password updated"),
        (status = 400, description = "Password too short"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn update_password(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
    ApiJson(req): ApiJson<UpdatePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if req.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }

    let hash = helpers::hash_password(req.password.clone()).await?;

    db::update_user_password(&state.pool, user_id, &hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/users/{user_id}",
    tag = "Users",
    operation_id = "delete_user",
    summary = "Delete a user (admin only)",
    params(
        ("user_id" = i64, Path, description = "User ID"),
    ),
    responses(
        (status = 204, description = "User deleted"),
        (status = 400, description = "Cannot delete own account"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn delete_user(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if admin.user_id == user_id {
        return Err(ApiError::BadRequest(
            "cannot delete own account".to_string(),
        ));
    }

    db::delete_user(&state.pool, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_role(role: &str) -> Result<(), ApiError> {
    super::auth::Role::parse(role).ok_or_else(|| {
        ApiError::BadRequest(format!("invalid role '{role}', must be 'admin' or 'user'"))
    })?;
    Ok(())
}
