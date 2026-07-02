// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::helpers;
use crate::{
    AppState,
    api::tokens::hash_token,
    db,
    error::{ApiError, ApiJson},
};

const MAX_LOGIN_ATTEMPTS: i64 = 5;
const LOGIN_WINDOW_MINUTES: i32 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Admin,
    User,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Admin => "admin",
            Self::User => "user",
        };
        f.write_str(s)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid role: {0}")]
pub struct InvalidRole(pub String);

impl std::str::FromStr for Role {
    type Err = InvalidRole;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            other => Err(InvalidRole(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub session_id: Option<String>,
}

const ALLOWED_PATHS_DURING_PASSWORD_CHANGE: &[&str] = &[
    "/api/auth/change-password",
    "/api/auth/logout",
    "/api/auth/me",
];

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(token_user) = try_bearer_auth(parts, state).await? {
            return Ok(token_user);
        }

        let session_id = extract_session_cookie(parts)?;
        let session = db::get_session(&state.pool, &session_id).await?;
        let user = db::get_user_by_id(&state.pool, session.user_id).await?;
        let role = user
            .role
            .parse::<Role>()
            .map_err(|_| ApiError::Internal(format!("invalid role in database: {}", user.role)))?;

        if user.must_change_password {
            let path = parts.uri.path();
            if !ALLOWED_PATHS_DURING_PASSWORD_CHANGE.contains(&path) {
                return Err(ApiError::Forbidden(
                    "password change required before accessing this resource".to_string(),
                ));
            }
        }

        Ok(Self {
            user_id: user.id,
            username: user.username,
            role,
            session_id: Some(session_id),
        })
    }
}

async fn try_bearer_auth(parts: &Parts, state: &AppState) -> Result<Option<AuthUser>, ApiError> {
    let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let Some(auth_str) = auth_header.to_str().ok() else {
        return Ok(None);
    };
    let Some(token) = auth_str.strip_prefix("Bearer ") else {
        return Ok(None);
    };

    let token_hash = hash_token(token);
    let lookup = db::get_user_by_token_hash(&state.pool, &token_hash).await?;
    db::update_api_token_last_used(&state.pool, &token_hash).await?;

    let user = db::get_user_by_id(&state.pool, lookup.user_id).await?;
    let role = user
        .role
        .parse::<Role>()
        .map_err(|_| ApiError::Internal(format!("invalid role in database: {}", user.role)))?;

    Ok(Some(AuthUser {
        user_id: user.id,
        username: user.username,
        role,
        session_id: None,
    }))
}

pub struct RequireAdmin(pub AuthUser);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_user = AuthUser::from_request_parts(parts, state).await?;
        if auth_user.role != Role::Admin {
            return Err(ApiError::Forbidden("admin access required".to_string()));
        }
        Ok(Self(auth_user))
    }
}

fn extract_session_cookie(parts: &Parts) -> Result<String, ApiError> {
    let cookie_header = parts
        .headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".to_string()))?;

    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix("session=")
            && !value.is_empty()
        {
            return Ok(value.to_string());
        }
    }

    Err(ApiError::Unauthorized("not authenticated".to_string()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub remember_me: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub user: db::UserRow,
}

#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "Authentication",
    operation_id = "login",
    summary = "Log in with username and password",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 429, description = "Too many failed login attempts"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let ip = extract_client_ip(&headers);

    let failed_count =
        db::count_failed_login_attempts(&state.pool, &req.username, &ip, LOGIN_WINDOW_MINUTES)
            .await?;
    if failed_count >= MAX_LOGIN_ATTEMPTS {
        return Err(ApiError::TooManyRequests(
            "Too many failed login attempts. Try again later.".to_string(),
        ));
    }

    let (user, hash) = db::get_user_password_hash(&state.pool, &req.username)
        .await
        .map_err(|e| match e {
            ApiError::NotFound(_) => ApiError::Unauthorized("invalid credentials".to_string()),
            other => other,
        })?;

    let password = req.password.clone();
    let valid = helpers::verify_password(password, hash)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid credentials".to_string()))?;

    if !valid {
        db::insert_login_attempt(&state.pool, &req.username, &ip, false).await?;
        return Err(ApiError::Unauthorized("invalid credentials".to_string()));
    }

    let session_id = Uuid::new_v4().to_string();
    let (ttl_hours, max_age_secs) = if req.remember_me {
        (24 * 30, 30 * 86400)
    } else {
        (24, 86400)
    };
    let expires_at = Utc::now() + Duration::hours(ttl_hours);

    db::insert_session(&state.pool, &session_id, user.id, expires_at).await?;
    db::update_last_login(&state.pool, user.id).await?;

    let secure_flag = if std::env::var("ASSIMILATE_SECURE_COOKIES").map_or(true, |v| v != "false") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure_flag}"
    );

    let body = Json(LoginResponse { user });
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
}

fn extract_client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "Authentication",
    operation_id = "logout",
    summary = "Log out and invalidate the current session",
    responses(
        (status = 204, description = "Logged out successfully"),
        (status = 400, description = "Cannot logout with token auth"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn logout(State(state): State<AppState>, auth: AuthUser) -> Result<Response, ApiError> {
    let Some(session_id) = &auth.session_id else {
        return Err(ApiError::BadRequest(
            "cannot logout with token auth".to_string(),
        ));
    };
    db::delete_session(&state.pool, session_id).await?;

    let secure_flag = if std::env::var("ASSIMILATE_SECURE_COOKIES").map_or(true, |v| v != "false") {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!("session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure_flag}");
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "Authentication",
    operation_id = "me",
    summary = "Get the currently authenticated user",
    responses(
        (status = 200, description = "Current user info", body = MeResponse),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<MeResponse>, ApiError> {
    let user = db::get_user_by_id(&state.pool, auth.user_id).await?;
    Ok(Json(MeResponse {
        id: auth.user_id,
        username: auth.username,
        role: auth.role.to_string(),
        must_change_password: user.must_change_password,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MeResponse {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangePasswordRequest {
    pub new_password: String,
}

#[utoipa::path(
    post,
    path = "/api/auth/change-password",
    tag = "Authentication",
    operation_id = "change_password",
    summary = "Change the current user's password",
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed successfully"),
        (status = 400, description = "Password too short"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if req.new_password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".to_string(),
        ));
    }

    let password = req.new_password;
    let hash = helpers::hash_password(password).await?;

    db::update_user_password(&state.pool, auth.user_id, &hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/auth/preferences",
    tag = "Authentication",
    operation_id = "get_preferences",
    summary = "Get the current user's preferences",
    responses(
        (status = 200, description = "User preferences as JSON object"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn get_preferences(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let prefs = db::get_user_preferences(&state.pool, auth.user_id).await?;
    Ok(Json(prefs))
}

#[utoipa::path(
    put,
    path = "/api/auth/preferences",
    tag = "Authentication",
    operation_id = "update_preferences",
    summary = "Update the current user's preferences",
    request_body(content = serde_json::Value, description = "Preferences JSON object"),
    responses(
        (status = 200, description = "Updated preferences"),
        (status = 400, description = "Preferences must be a JSON object"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn update_preferences(
    auth: AuthUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !body.is_object() {
        return Err(ApiError::BadRequest(
            "preferences must be a JSON object".to_string(),
        ));
    }
    db::set_user_preferences(&state.pool, auth.user_id, &body).await?;
    Ok(Json(body))
}
