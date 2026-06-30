// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::responses::UserResponse;

use super::{auth::RequireAdmin, helpers};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::UserRow> for UserResponse {
    fn from(row: db::UserRow) -> Self {
        Self {
            id: row.id,
            username: row.username,
            role: row.role,
            created_at: row.created_at,
            last_login_at: row.last_login_at,
            must_change_password: row.must_change_password,
        }
    }
}

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
        (status = 200, description = "List of users", body = Vec<UserResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let users: Vec<UserResponse> = db::list_users(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
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
        (status = 201, description = "User created", body = UserResponse),
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
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    helpers::validate_non_empty(&req.username, "username")?;

    if req.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }

    validate_role(&req.role)?;

    let hash = helpers::hash_password(req.password.clone()).await?;

    let user: UserResponse = db::insert_user(&state.pool, &req.username, &hash, &req.role)
        .await?
        .into();
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
        (status = 200, description = "Updated user", body = UserResponse),
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
) -> Result<Json<UserResponse>, ApiError> {
    if admin.user_id == user_id {
        return Err(ApiError::BadRequest("cannot change own role".to_string()));
    }

    validate_role(&req.role)?;

    let user: UserResponse = db::update_user_role(&state.pool, user_id, &req.role)
        .await?
        .into();
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
    role.parse::<super::auth::Role>().map_err(|_| {
        ApiError::BadRequest(format!("invalid role '{role}', must be 'admin' or 'user'"))
    })?;
    Ok(())
}
