// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use shared::responses::{LoginResponse, TotpSetupResponse, TotpVerifyResponse};
use totp_rs::{Algorithm, Secret, TOTP};

use super::auth::AuthUser;
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

const RECOVERY_CODE_COUNT: usize = 10;
const RECOVERY_CODE_BYTES: usize = 8;

fn generate_recovery_codes() -> Vec<String> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let mut bytes = vec![0u8; RECOVERY_CODE_BYTES];
            rand::RngCore::fill_bytes(&mut OsRng, &mut bytes);
            // Format as hex groups for readability
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            hex.chars()
                .collect::<Vec<_>>()
                .chunks(4)
                .map(|c| c.iter().collect::<String>())
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect()
}

fn hash_recovery_code(code: &str) -> String {
    let normalized = code.replace('-', "").to_lowercase();
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(normalized.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

fn verify_recovery_code(input: &str, hashed_codes: &[String]) -> Option<usize> {
    let normalized_input = input.replace('-', "").to_lowercase();
    let input_hash = {
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(normalized_input.as_bytes());
        let result = hasher.finalize();
        result
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    hashed_codes.iter().position(|c| c == &input_hash)
}

fn generate_qr_code_uri(uri: &str) -> Result<String, ApiError> {
    use image::Luma;
    use qrcode::{EcLevel, QrCode};

    let qr = QrCode::with_error_correction_level(uri, EcLevel::M)
        .map_err(|e| ApiError::Internal(format!("failed to generate QR code: {e}")))?;

    let image = qr.render::<Luma<u8>>().min_dimensions(400, 400).build();

    let mut png_bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| ApiError::Internal(format!("failed to encode QR code: {e}")))?;

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

fn create_totp(secret: &[u8]) -> Result<TOTP, ApiError> {
    // Use RFC 6238 defaults: 30-second period, 6 digits, SHA-1
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret.to_vec())
            .to_bytes()
            .map_err(|e| ApiError::Internal(format!("failed to parse TOTP secret: {e}")))?,
        None,
        String::new(),
    )
    .map_err(|e| ApiError::Internal(format!("failed to create TOTP: {e}")))
}

fn generate_totp_secret() -> Vec<u8> {
    let mut secret = vec![0u8; 20];
    rand::RngCore::fill_bytes(&mut OsRng, &mut secret);
    secret
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpVerifyRequest {
    pub code: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpDisableRequest {
    pub password: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpRecoveryRequest {
    pub code: String,
    pub temp_token: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpLoginVerifyRequest {
    pub code: String,
    pub temp_token: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TempTokenResponse {
    pub totp_required: bool,
    pub temp_token: String,
}

#[utoipa::path(
    post,
    path = "/api/auth/totp/setup",
    tag = "Authentication",
    operation_id = "totp_setup",
    summary = "Generate TOTP setup info (secret + QR code + recovery codes)",
    responses(
        (status = 200, description = "TOTP setup info", body = TotpSetupResponse),
        (status = 401, description = "Not authenticated"),
    )
)]
pub async fn totp_setup(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<TotpSetupResponse>, ApiError> {
    // Generate a new secret
    let secret = generate_totp_secret();

    // Create TOTP object with issuer and account name for the URL
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret.to_vec())
            .to_bytes()
            .map_err(|e| ApiError::Internal(format!("failed to parse TOTP secret: {e}")))?,
        Some("Assimilate".to_string()),
        auth.username.clone(),
    )
    .map_err(|e| ApiError::Internal(format!("failed to create TOTP: {e}")))?;

    let otpauth_uri = totp.get_url();

    // Encrypt the secret
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key)?;

    // Generate recovery codes
    let recovery_codes = generate_recovery_codes();
    let hashed_codes: Vec<String> = recovery_codes
        .iter()
        .map(|c| hash_recovery_code(c))
        .collect();

    // Store encrypted secret and hashed recovery codes (but don't enable yet)
    db::set_user_totp_secret(&state.pool, auth.user_id, &encrypted, &hashed_codes).await?;

    // Generate QR code as base64 PNG
    let qr_uri = generate_qr_code_uri(&otpauth_uri)?;

    Ok(Json(TotpSetupResponse {
        secret: hex::encode(&secret),
        qr_uri,
        recovery_codes,
    }))
}

#[utoipa::path(
    post,
    path = "/api/auth/totp/verify",
    tag = "Authentication",
    operation_id = "totp_verify",
    summary = "Verify a TOTP code and enable 2FA",
    responses(
        (status = 200, description = "TOTP verification result", body = TotpVerifyResponse),
        (status = 401, description = "Not authenticated"),
        (status = 400, description = "Invalid code"),
    )
)]
pub async fn totp_verify(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<TotpVerifyRequest>,
) -> Result<Json<TotpVerifyResponse>, ApiError> {
    let totp_fields = db::get_user_totp_fields(&state.pool, auth.user_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    let Some(ref encrypted) = totp_fields.secret_encrypted else {
        return Err(ApiError::BadRequest("TOTP not set up".to_string()));
    };

    let decrypted = shared::crypto::decrypt_passphrase(encrypted, &state.encryption_key)?;
    let secret = hex::decode(&decrypted)
        .map_err(|e| ApiError::Internal(format!("failed to decode TOTP secret: {e}")))?;

    let totp = create_totp(&secret)?;
    let is_valid = totp
        .check_current(req.code.trim())
        .map_err(|_| ApiError::Internal("TOTP check failed".to_string()))?;

    if !is_valid {
        return Err(ApiError::BadRequest(
            "Invalid verification code".to_string(),
        ));
    }

    db::enable_user_totp(&state.pool, auth.user_id).await?;

    let backup_codes_remaining = Some(totp_fields.recovery_codes.len() as i32);

    Ok(Json(TotpVerifyResponse {
        success: true,
        backup_codes_remaining,
    }))
}

#[utoipa::path(
    post,
    path = "/api/auth/totp/verify-login",
    tag = "Authentication",
    operation_id = "totp_verify_login",
    summary = "Complete login with TOTP code (two-step login)",
    responses(
        (status = 200, description = "Login complete", body = LoginResponse),
        (status = 401, description = "Invalid code or temp token"),
    )
)]
pub async fn totp_verify_login(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<TotpLoginVerifyRequest>,
) -> Result<Response, ApiError> {
    // Verify the temp token session exists
    let temp_hashed = crate::api::tokens::hash_token(&req.temp_token);
    let temp_session = db::get_session(&state.pool, &temp_hashed)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid or expired temp token".to_string()))?;

    let user = db::get_user_by_id(&state.pool, temp_session.user_id).await?;
    let totp_fields = db::get_user_totp_fields(&state.pool, user.id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    if !totp_fields.enabled {
        return Err(ApiError::BadRequest("TOTP not enabled".to_string()));
    }

    // Verify TOTP code
    let Some(ref encrypted) = totp_fields.secret_encrypted else {
        return Err(ApiError::BadRequest("TOTP not configured".to_string()));
    };

    let decrypted = shared::crypto::decrypt_passphrase(encrypted, &state.encryption_key)?;
    let secret = hex::decode(&decrypted)
        .map_err(|e| ApiError::Internal(format!("failed to decode TOTP secret: {e}")))?;

    let totp = create_totp(&secret)?;
    let is_valid = totp
        .check_current(req.code.trim())
        .map_err(|_| ApiError::Internal("TOTP check failed".to_string()))?;

    if !is_valid {
        return Err(ApiError::Unauthorized(
            "invalid verification code".to_string(),
        ));
    }

    // Delete the temp session
    db::delete_session(&state.pool, &temp_hashed).await?;

    // Create the real session
    let remember_me = temp_session.remember_me;
    let session_id = uuid::Uuid::new_v4().to_string();
    let (ttl_hours, max_age_secs) = if remember_me {
        (24 * 7, 7 * 86400)
    } else {
        (24, 86400)
    };
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(ttl_hours);

    let hashed_id = crate::api::tokens::hash_token(&session_id);
    db::insert_session(&state.pool, &hashed_id, user.id, expires_at, remember_me).await?;
    db::update_last_login(&state.pool, user.id).await?;

    let secure_flag = super::auth::secure_cookie_flag();
    let cookie = format!(
        "session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure_flag}"
    );

    let user_resp = super::users::user_row_to_response(&state.pool, user).await?;
    let body = Json(LoginResponse {
        user: user_resp,
        session_expires_at: expires_at,
        remember_me,
        totp_required: false,
        temp_token: None,
    });
    let mut response = body.into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/auth/totp/disable",
    tag = "Authentication",
    operation_id = "totp_disable",
    summary = "Disable TOTP/2FA (requires current password)",
    responses(
        (status = 200, description = "TOTP disabled"),
        (status = 401, description = "Not authenticated"),
        (status = 400, description = "Incorrect password"),
    )
)]
pub async fn totp_disable(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<TotpDisableRequest>,
) -> Result<Json<TotpVerifyResponse>, ApiError> {
    let hash = db::get_user_password_hash_by_id(&state.pool, auth.user_id).await?;

    let valid = super::helpers::verify_password(req.password.clone(), hash)
        .await
        .map_err(|_| ApiError::BadRequest("incorrect password".to_string()))?;

    if !valid {
        return Err(ApiError::BadRequest("incorrect password".to_string()));
    }

    db::disable_user_totp(&state.pool, auth.user_id).await?;

    Ok(Json(TotpVerifyResponse {
        success: true,
        backup_codes_remaining: None,
    }))
}

#[utoipa::path(
    post,
    path = "/api/auth/totp/recovery",
    tag = "Authentication",
    operation_id = "totp_recovery",
    summary = "Verify a recovery code during login (completes the login)",
    responses(
        (status = 200, description = "Recovery accepted, login complete", body = LoginResponse),
        (status = 401, description = "Invalid recovery code or temp token"),
    )
)]
pub async fn totp_recovery(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<TotpRecoveryRequest>,
) -> Result<Response, ApiError> {
    // Validate the temp_token and get the user
    let temp_hashed = crate::api::tokens::hash_token(&req.temp_token);
    let session = db::get_session(&state.pool, &temp_hashed)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid temp token".to_string()))?;

    let totp_fields = db::get_user_totp_fields(&state.pool, session.user_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    let idx = verify_recovery_code(&req.code, &totp_fields.recovery_codes);
    let Some(idx) = idx else {
        return Err(ApiError::Unauthorized("invalid recovery code".to_string()));
    };

    // Remove the used recovery code
    let mut remaining = totp_fields.recovery_codes.clone();
    remaining.remove(idx);
    db::replace_totp_recovery_codes(&state.pool, session.user_id, &remaining).await?;

    // Delete the temp session
    db::delete_session(&state.pool, &temp_hashed).await?;

    // Create the real session
    let user = db::get_user_by_id(&state.pool, session.user_id).await?;
    let remember_me = session.remember_me;
    let session_id = uuid::Uuid::new_v4().to_string();
    let (ttl_hours, max_age_secs) = if remember_me {
        (24 * 7, 7 * 86400)
    } else {
        (24, 86400)
    };
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(ttl_hours);

    let hashed_id = crate::api::tokens::hash_token(&session_id);
    db::insert_session(&state.pool, &hashed_id, user.id, expires_at, remember_me).await?;
    db::update_last_login(&state.pool, user.id).await?;

    let secure_flag = super::auth::secure_cookie_flag();
    let cookie = format!(
        "session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure_flag}"
    );

    let user_resp = super::users::user_row_to_response(&state.pool, user).await?;
    let body = Json(LoginResponse {
        user: user_resp,
        session_expires_at: expires_at,
        remember_me,
        totp_required: false,
        temp_token: None,
    });
    let mut response = body.into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|e| ApiError::Internal(format!("failed to build cookie header: {e}")))?,
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_recovery_codes_produces_unique_codes() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let mut sorted = codes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "recovery codes must be unique");
    }

    #[test]
    fn recovery_code_hash_and_verify_roundtrip() {
        let code = "abcd-1234-ef56-7890";
        let hashed = hash_recovery_code(code);
        let idx = verify_recovery_code("abcd-1234-ef56-7890", &[hashed]);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn recovery_code_verify_is_case_insensitive_and_ignores_dashes() {
        let code = "ABCD-1234-EF56-7890";
        let hashed = hash_recovery_code(code);
        let idx = verify_recovery_code("abcd1234ef567890", &[hashed]);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn recovery_code_wrong_code_returns_none() {
        let code = "abcd-1234-ef56-7890";
        let hashed = hash_recovery_code(code);
        let idx = verify_recovery_code("zzzz-zzzz-zzzz-zzzz", &[hashed]);
        assert!(idx.is_none());
    }

    #[test]
    fn totp_secret_generation_returns_random_bytes() {
        let secret1 = generate_totp_secret();
        let secret2 = generate_totp_secret();
        assert_eq!(secret1.len(), 20);
        assert_ne!(secret1, secret2);
    }

    #[test]
    fn create_totp_from_secret_succeeds() {
        let secret = generate_totp_secret();
        let totp = create_totp(&secret);
        assert!(totp.is_ok());
    }

    #[test]
    fn hash_recovery_code_produces_hex_string() {
        let code = "test-code-1234";
        let hashed = hash_recovery_code(code);
        assert!(!hashed.is_empty());
        assert!(hashed.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
