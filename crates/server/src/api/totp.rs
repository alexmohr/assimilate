// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, extract::State, response::Response};
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
            hex_grouped(&bytes)
        })
        .collect()
}

fn hex_grouped(bytes: &[u8]) -> String {
    let capacity = bytes
        .len()
        .saturating_mul(2)
        .saturating_add(bytes.len() / 4);
    let mut out = String::with_capacity(capacity);
    for (i, chunk) in bytes.chunks(4).enumerate() {
        if i > 0 {
            out.push('-');
        }
        for b in chunk {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
        }
    }
    out
}

fn normalize_recovery_code(code: &str) -> String {
    code.replace('-', "").to_lowercase()
}

async fn hash_recovery_code(code: &str) -> Result<String, ApiError> {
    let normalized = normalize_recovery_code(code);
    tokio::task::spawn_blocking(move || {
        bcrypt::hash(&normalized, bcrypt::DEFAULT_COST)
            .map_err(|e| ApiError::Internal(format!("failed to hash recovery code: {e}")))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn blocking failed: {e}")))?
}

async fn verify_recovery_code(
    input: &str,
    hashed_codes: &[String],
) -> Result<Option<usize>, ApiError> {
    let normalized_input = normalize_recovery_code(input);
    let hashes = hashed_codes.to_vec();
    tokio::task::spawn_blocking(move || -> Result<Option<usize>, ApiError> {
        for (idx, hash) in hashes.iter().enumerate() {
            if bcrypt::verify(&normalized_input, hash)
                .map_err(|e| ApiError::Internal(format!("failed to verify recovery code: {e}")))?
            {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn blocking failed: {e}")))?
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

/// Verify a TOTP code against the user's stored encrypted secret.
///
/// Returns `Ok(true)` if the code is valid, `Ok(false)` if invalid.
async fn verify_totp_code(
    state: &AppState,
    encrypted_secret: &[u8],
    code: &str,
) -> Result<bool, ApiError> {
    let decrypted = shared::crypto::decrypt_passphrase(encrypted_secret, &state.encryption_key)?;
    let secret = hex::decode(&decrypted)
        .map_err(|e| ApiError::Internal(format!("failed to decode TOTP secret: {e}")))?;

    let totp = create_totp(&secret)?;
    let is_valid = totp
        .check_current(code.trim())
        .map_err(|_| ApiError::Internal("TOTP check failed".to_string()))?;

    Ok(is_valid)
}

/// Request payload for TOTP verification during setup.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpVerifyRequest {
    /// The TOTP code to verify.
    pub code: String,
}

/// Request payload for disabling TOTP.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpDisableRequest {
    /// The user's current password (required for security).
    pub password: String,
}

/// Request payload for logging in with a recovery code.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpRecoveryRequest {
    /// The recovery code.
    pub code: String,
    /// The temporary token from the first login step.
    pub temp_token: String,
}

/// Request payload for completing login with a TOTP code.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TotpLoginVerifyRequest {
    /// The TOTP code.
    pub code: String,
    /// The temporary token from the first login step.
    pub temp_token: String,
}

/// Response indicating that TOTP is required to complete login.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TempTokenResponse {
    /// Whether TOTP is required.
    pub totp_required: bool,
    /// The temporary token for the second login step.
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
/// # Errors
///
/// Returns [`ApiError::Internal`] if TOTP creation, QR generation, or
/// encryption fails. Returns [`ApiError::Database`] if the DB query fails.
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
        Secret::Raw(secret.clone())
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
    let mut hashed_codes = Vec::with_capacity(recovery_codes.len());
    for code in &recovery_codes {
        hashed_codes.push(hash_recovery_code(code).await?);
    }

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
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if TOTP is not set up or the code
/// is invalid. Returns [`ApiError::Internal`] if decryption or TOTP
/// verification fails. Returns [`ApiError::Database`] if the DB query fails.
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

    if !verify_totp_code(&state, encrypted, &req.code).await? {
        return Err(ApiError::BadRequest(
            "Invalid verification code".to_string(),
        ));
    }

    db::enable_user_totp(&state.pool, auth.user_id).await?;

    let backup_codes_remaining =
        Some(i32::try_from(totp_fields.recovery_codes.len()).unwrap_or(i32::MAX));

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
/// # Errors
///
/// Returns [`ApiError::Unauthorized`] if the temp token or TOTP code is
/// invalid. Returns [`ApiError::BadRequest`] if TOTP is not configured.
/// Returns [`ApiError::Internal`] if session creation fails.
/// Returns [`ApiError::Database`] if the DB query fails.
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

    // TOTP replay protection: reject if last_verified_at is within the current
    // 30-second TOTP window, preventing the same code from being reused.
    if let Some(last_verified) = totp_fields.last_verified_at {
        let within_window = chrono::Utc::now()
            .signed_duration_since(last_verified)
            .num_seconds()
            < 30;
        if within_window {
            return Err(ApiError::Unauthorized("TOTP code already used".to_string()));
        }
    }

    // Verify TOTP code
    let Some(ref encrypted) = totp_fields.secret_encrypted else {
        return Err(ApiError::BadRequest("TOTP not configured".to_string()));
    };

    if !verify_totp_code(&state, encrypted, &req.code).await? {
        return Err(ApiError::Unauthorized(
            "invalid verification code".to_string(),
        ));
    }

    // Record the verification time to prevent replay of the same code
    db::update_totp_last_verified_at(&state.pool, user.id).await?;

    // Delete the temp session
    db::delete_session(&state.pool, &temp_hashed).await?;

    // Create the real session using the shared helper
    let user_resp = super::users::user_row_to_response(&state.pool, user).await?;
    let response =
        super::auth::create_session_response(&state.pool, user_resp, temp_session.remember_me)
            .await?;
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
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the password is incorrect.
/// Returns [`ApiError::Database`] if the DB query fails.
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
/// # Errors
///
/// Returns [`ApiError::Unauthorized`] if the temp token or recovery code
/// is invalid. Returns [`ApiError::BadRequest`] if TOTP is not configured.
/// Returns [`ApiError::Internal`] if session creation fails.
/// Returns [`ApiError::Database`] if the DB query fails.
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

    let idx = verify_recovery_code(&req.code, &totp_fields.recovery_codes).await?;
    let Some(idx) = idx else {
        return Err(ApiError::Unauthorized("invalid recovery code".to_string()));
    };

    // Remove the used recovery code
    let mut remaining = totp_fields.recovery_codes.clone();
    remaining.remove(idx);
    db::replace_totp_recovery_codes(&state.pool, session.user_id, &remaining).await?;

    // Delete the temp session
    db::delete_session(&state.pool, &temp_hashed).await?;

    // Create the real session using the shared helper
    let user = db::get_user_by_id(&state.pool, session.user_id).await?;
    let user_resp = super::users::user_row_to_response(&state.pool, user).await?;
    let response =
        super::auth::create_session_response(&state.pool, user_resp, session.remember_me).await?;
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

    #[tokio::test]
    async fn recovery_code_hash_and_verify_roundtrip() {
        let code = "abcd-1234-ef56-7890";
        let hashed = hash_recovery_code(code).await.unwrap();
        let idx = verify_recovery_code("abcd-1234-ef56-7890", &[hashed])
            .await
            .unwrap();
        assert_eq!(idx, Some(0));
    }

    #[tokio::test]
    async fn recovery_code_verify_is_case_insensitive_and_ignores_dashes() {
        let code = "ABCD-1234-EF56-7890";
        let hashed = hash_recovery_code(code).await.unwrap();
        let idx = verify_recovery_code("abcd1234ef567890", &[hashed])
            .await
            .unwrap();
        assert_eq!(idx, Some(0));
    }

    #[tokio::test]
    async fn recovery_code_wrong_code_returns_none() {
        let code = "abcd-1234-ef56-7890";
        let hashed = hash_recovery_code(code).await.unwrap();
        let idx = verify_recovery_code("zzzz-zzzz-zzzz-zzzz", &[hashed])
            .await
            .unwrap();
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

    #[tokio::test]
    async fn hash_recovery_code_uses_bcrypt() {
        let code = "test-code-1234";
        let hashed = hash_recovery_code(code).await.unwrap();
        // bcrypt hashes start with $2b$ or $2a$
        assert!(
            hashed.starts_with("$2"),
            "expected bcrypt hash, got: {hashed}"
        );
    }
}
