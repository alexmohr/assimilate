// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{audit::AuditEvent, responses::UserResponse};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    auth::RequireAdmin,
    helpers,
};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

pub(crate) async fn get_user_role_string(
    pool: &sqlx::PgPool,
    user_id: i64,
) -> Result<String, ApiError> {
    let role_names: Vec<String> = db::list_user_roles(pool, user_id)
        .await?
        .into_iter()
        .map(|r| r.name)
        .collect();
    Ok(role_names.join(","))
}

pub(crate) async fn user_row_to_response(
    pool: &sqlx::PgPool,
    row: db::UserRow,
) -> Result<UserResponse, ApiError> {
    let role = get_user_role_string(pool, row.id).await?;
    Ok(UserResponse {
        id: row.id,
        username: row.username,
        role,
        created_at: row.created_at,
        last_login_at: row.last_login_at,
        must_change_password: row.must_change_password,
    })
}

/// Request payload for creating a new user.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    /// Username.
    pub username: String,
    /// Password (minimum 8 characters).
    pub password: String,
}

/// Request payload for updating a user's password.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePasswordRequest {
    /// New password (minimum 8 characters).
    pub password: String,
}

#[utoipa::path(
    get,
    path = "/api/users",
    tag = "Users",
    operation_id = "list_users",
    responses(
        (status = 200, description = "List of users", body = Vec<UserResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 500, description = "Internal server error"),
    )
)]
/// List all users (admin only).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_users(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let rows = db::list_users(&state.pool).await?;
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        users.push(user_row_to_response(&state.pool, row).await?);
    }
    Ok(Json(users))
}

#[utoipa::path(
    post,
    path = "/api/users",
    tag = "Users",
    operation_id = "create_user",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = UserResponse),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 500, description = "Internal server error"),
    )
)]
/// Create a new user (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn create_user(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    ApiJson(req): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    helpers::validate_non_empty(&req.username, "username")?;

    if req.password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }

    let hash = helpers::hash_password(req.password.clone()).await?;

    let user = db::insert_user(&state.pool, &req.username, &hash).await?;
    let user = user_row_to_response(&state.pool, user).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::User(user.id)),
        AuditEvent::CreateUser {
            username: user.username.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(user)))
}

#[utoipa::path(
    put,
    path = "/api/users/{user_id}/password",
    tag = "Users",
    operation_id = "update_password",
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
/// Update a user's password (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn update_password(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
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
    let user = db::get_user_by_id(&state.pool, user_id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::User(user_id)),
        AuditEvent::ResetPassword {
            username: user.username,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/users/{user_id}",
    tag = "Users",
    operation_id = "delete_user",
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
/// Delete a user (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn delete_user(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if admin.user_id == user_id {
        return Err(ApiError::BadRequest(
            "cannot delete own account".to_string(),
        ));
    }

    let user = db::get_user_by_id(&state.pool, user_id).await?;
    db::delete_user(&state.pool, user_id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::User(user_id)),
        AuditEvent::DeleteUser {
            username: user.username,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{audit_entries, audit_events, build_test_state, insert_auth_user};

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn creating_resetting_and_deleting_a_user_is_audited(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"users-audit-test-key");
        let admin = insert_auth_user(&pool, "root-admin").await;
        let ip = ClientIp(Some("192.0.2.4".parse().unwrap()));

        let (_, Json(created)) = create_user(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ip,
            ApiJson(CreateUserRequest {
                username: "new-operator".to_owned(),
                password: "correct-horse-battery".to_owned(),
            }),
        )
        .await
        .unwrap();
        update_password(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ip,
            Path(created.id),
            ApiJson(UpdatePasswordRequest {
                password: "another-long-secret".to_owned(),
            }),
        )
        .await
        .unwrap();
        delete_user(
            State(state),
            RequireAdmin(admin.clone()),
            ip,
            Path(created.id),
        )
        .await
        .unwrap();

        let entries = audit_entries(&pool).await;
        let events: Vec<_> = entries.iter().map(|entry| entry.event.clone()).collect();
        assert_eq!(
            events,
            [
                AuditEvent::DeleteUser {
                    username: "new-operator".to_owned()
                },
                AuditEvent::ResetPassword {
                    username: "new-operator".to_owned()
                },
                AuditEvent::CreateUser {
                    username: "new-operator".to_owned()
                },
            ]
        );
        assert!(entries.iter().all(|entry| {
            entry.user_id == Some(admin.user_id)
                && entry.target_type.as_deref() == Some("user")
                && entry.target_id == Some(created.id)
                && entry.ip_address.as_deref() == Some("192.0.2.4")
        }));
        let stored = serde_json::to_string(&entries).unwrap();
        assert!(
            !stored.contains("correct-horse-battery") && !stored.contains("another-long-secret"),
            "a password must never reach the audit log"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_self_delete_is_not_audited(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"users-audit-test-key");
        let admin = insert_auth_user(&pool, "lonely-admin").await;

        let result = delete_user(
            State(state),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            Path(admin.user_id),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest(_))));
        assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
    }
}
