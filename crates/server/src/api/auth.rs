// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, Path, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use shared::responses::{
    LoginResponse, MeResponse, PreferencesResponse, RefreshSessionResponse, SessionListResponse,
    SessionResponse,
};
use uuid::Uuid;

use super::{helpers, users};
use crate::{
    AppState,
    api::tokens::hash_token,
    db,
    error::{ApiError, ApiJson},
};

const MAX_LOGIN_ATTEMPTS: i64 = 5;
const LOGIN_WINDOW_MINUTES: i32 = 15;

/// Whether the session cookie should carry the `Secure` attribute.
///
/// Defaults to `Secure` fail-safe: only an explicit `ASSIMILATE_SECURE_COOKIES=false`
/// disables it (e.g. for local HTTP development).
enum CookieSecurity {
    Secure,
    Insecure,
}

impl From<Option<String>> for CookieSecurity {
    fn from(env_value: Option<String>) -> Self {
        match env_value.as_deref() {
            Some("false") => Self::Insecure,
            _ => Self::Secure,
        }
    }
}

impl CookieSecurity {
    fn cookie_flag(self) -> &'static str {
        match self {
            Self::Secure => "; Secure",
            Self::Insecure => "",
        }
    }
}

pub fn secure_cookie_flag() -> &'static str {
    CookieSecurity::from(std::env::var("ASSIMILATE_SECURE_COOKIES").ok()).cookie_flag()
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub session_id: Option<String>,
}

const ALLOWED_PATHS_DURING_PASSWORD_CHANGE: &[&str] = &[
    "/api/auth/change-password",
    "/api/auth/logout",
    "/api/auth/me",
    "/api/auth/totp/setup",
    "/api/auth/totp/verify",
    "/api/auth/totp/disable",
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
        let hashed_id = hash_token(&session_id);
        let session = db::get_session(&state.pool, &hashed_id).await?;
        let user = db::get_user_by_id(&state.pool, session.user_id).await?;

        // Idle timeout check
        let idle_timeout_minutes: i64 =
            db::get_setting(&state.pool, "session_idle_timeout_minutes")
                .await?
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(480);

        let idle_duration = Utc::now() - session.last_seen_at;
        if idle_duration.num_minutes() > idle_timeout_minutes {
            db::delete_session(&state.pool, &hashed_id).await?;
            return Err(ApiError::Unauthorized(
                "session expired due to inactivity".to_string(),
            ));
        }

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

    Ok(Some(AuthUser {
        user_id: user.id,
        username: user.username,
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
        let effective = db::get_effective_permissions(&state.pool, auth_user.user_id).await?;
        if !effective.can_delete_repo {
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
pub struct RefreshResponse {
    pub session_expires_at: DateTime<Utc>,
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let ip = state
        .client_ip_resolver
        .resolve(peer.ip(), &headers)
        .to_string();

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

    let user_resp = users::user_row_to_response(&state.pool, user).await?;

    // Check if TOTP is enabled for this user
    let totp_fields = db::get_user_totp_fields(&state.pool, user_resp.id).await?;
    let totp_enabled = totp_fields.is_some_and(|f| f.enabled);

    if totp_enabled {
        // Create a short-lived temp token session for TOTP verification
        let temp_token = Uuid::new_v4().to_string();
        let temp_hashed = hash_token(&temp_token);
        let temp_expires = Utc::now() + Duration::minutes(10);
        db::insert_session(&state.pool, &temp_hashed, user_resp.id, temp_expires, false).await?;

        let body = Json(LoginResponse {
            user: user_resp,
            session_expires_at: temp_expires,
            remember_me: req.remember_me,
            totp_required: true,
            temp_token: Some(temp_token),
        });
        return Ok(body.into_response());
    }

    let session_id = Uuid::new_v4().to_string();
    let (ttl_hours, max_age_secs) = if req.remember_me {
        (24 * 7, 7 * 86400)
    } else {
        (24, 86400)
    };
    let expires_at = Utc::now() + Duration::hours(ttl_hours);

    let hashed_id = hash_token(&session_id);
    db::insert_session(
        &state.pool,
        &hashed_id,
        user_resp.id,
        expires_at,
        req.remember_me,
    )
    .await?;
    db::update_last_login(&state.pool, user_resp.id).await?;

    let secure_flag = secure_cookie_flag();
    let cookie = format!(
        "session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure_flag}"
    );

    let body = Json(LoginResponse {
        user: user_resp,
        session_expires_at: expires_at,
        remember_me: req.remember_me,
        totp_required: false,
        temp_token: None,
    });
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
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
    db::delete_session(&state.pool, &hash_token(session_id)).await?;

    let secure_flag = secure_cookie_flag();
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
    let (session_expires_at, remember_me) = if let Some(ref session_id) = auth.session_id {
        let hashed_id = hash_token(session_id);
        let session = db::get_session(&state.pool, &hashed_id).await?;
        // Update last_seen_at on me requests to slide the idle window
        db::update_session_last_seen(&state.pool, &hashed_id).await?;
        (Some(session.expires_at), session.remember_me)
    } else {
        (None, false)
    };
    let role = users::get_user_role_string(&state.pool, auth.user_id).await?;

    let totp_fields = db::get_user_totp_fields(&state.pool, auth.user_id).await?;
    let totp_enabled = totp_fields.is_some_and(|f| f.enabled);

    Ok(Json(MeResponse {
        id: auth.user_id,
        username: auth.username,
        role,
        must_change_password: user.must_change_password,
        session_expires_at,
        remember_me,
        totp_enabled,
    }))
}

#[utoipa::path(
    post,
    path = "/api/auth/refresh",
    tag = "Authentication",
    operation_id = "refresh_session",
    summary = "Extend a remember-me session before it expires",
    responses(
        (status = 200, description = "Session extended", body = RefreshSessionResponse),
        (status = 400, description = "Not a remember-me session or token auth"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn refresh_session(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let Some(ref session_id) = auth.session_id else {
        return Err(ApiError::BadRequest(
            "cannot refresh with token auth".to_string(),
        ));
    };

    let hashed_id = hash_token(session_id);
    let session = db::get_session(&state.pool, &hashed_id).await?;
    if !session.remember_me {
        return Err(ApiError::BadRequest(
            "not a remember-me session".to_string(),
        ));
    }

    let new_expires_at = Utc::now() + Duration::days(7);
    db::extend_session(&state.pool, &hashed_id, new_expires_at).await?;
    // Update last_seen_at to slide idle window
    db::update_session_last_seen(&state.pool, &hashed_id).await?;

    let secure_flag = secure_cookie_flag();
    let max_age_secs = 7 * 86400_i64;
    let cookie = format!(
        "session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure_flag}"
    );

    let mut response = Json(RefreshSessionResponse {
        session_expires_at: new_expires_at,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
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
) -> Result<Json<PreferencesResponse>, ApiError> {
    let prefs = db::get_user_preferences(&state.pool, auth.user_id).await?;
    Ok(Json(PreferencesResponse { inner: prefs }))
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
) -> Result<Json<PreferencesResponse>, ApiError> {
    if !body.is_object() {
        return Err(ApiError::BadRequest(
            "preferences must be a JSON object".to_string(),
        ));
    }
    db::set_user_preferences(&state.pool, auth.user_id, &body).await?;
    Ok(Json(PreferencesResponse { inner: body }))
}

#[utoipa::path(
    get,
    path = "/api/auth/sessions",
    tag = "Authentication",
    operation_id = "list_sessions",
    summary = "List all active sessions for the current user",
    responses(
        (status = 200, description = "List of active sessions", body = SessionListResponse),
        (status = 401, description = "Not authenticated"),
    )
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = db::list_sessions_for_user(&state.pool, auth.user_id).await?;

    let current_session_id = auth.session_id.map(|s| hash_token(&s));
    let sessions: Vec<SessionResponse> = sessions
        .into_iter()
        .map(|s| SessionResponse {
            current: current_session_id.as_deref() == Some(&s.id),
            id: s.id,
            user_id: s.user_id,
            created_at: s.created_at,
            expires_at: s.expires_at,
            last_seen_at: s.last_seen_at,
            remember_me: s.remember_me,
        })
        .collect();

    Ok(Json(SessionListResponse { sessions }))
}

#[utoipa::path(
    delete,
    path = "/api/auth/sessions/{session_id}",
    tag = "Authentication",
    operation_id = "revoke_session",
    summary = "Revoke another active session (cannot revoke own current session)",
    responses(
        (status = 204, description = "Session revoked"),
        (status = 400, description = "Cannot revoke own current session"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Session not found"),
    )
)]
pub async fn revoke_session(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(session_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let current_hashed = auth.session_id.map(|s| hash_token(&s));
    let target_hashed = hash_token(&session_id);

    if current_hashed.as_deref() == Some(&target_hashed) {
        return Err(ApiError::BadRequest(
            "cannot revoke your own current session".to_string(),
        ));
    }

    let deleted = db::delete_session_by_id(&state.pool, &target_hashed).await?;
    if !deleted {
        return Err(ApiError::NotFound("session not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::CookieSecurity;

    #[test]
    fn cookie_security_defaults_to_secure_when_unset() {
        assert_eq!(CookieSecurity::from(None).cookie_flag(), "; Secure");
    }

    #[test]
    fn cookie_security_is_insecure_when_explicitly_false() {
        assert_eq!(
            CookieSecurity::from(Some("false".to_string())).cookie_flag(),
            ""
        );
    }

    #[test]
    fn cookie_security_is_secure_for_any_other_value() {
        assert_eq!(
            CookieSecurity::from(Some("0".to_string())).cookie_flag(),
            "; Secure"
        );
    }
}
