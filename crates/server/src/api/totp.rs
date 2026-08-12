// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::Response,
};
use rand::rngs::OsRng;
use serde::Deserialize;
use shared::responses::{LoginResponse, TotpSetupResponse, TotpVerifyResponse};
use totp_rs::{Algorithm, Secret, TOTP};

use super::auth::AuthUser;
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

const RECOVERY_CODE_COUNT: usize = 10;
const RECOVERY_CODE_BYTES: usize = 8;

/// Account-level brute-force protection for TOTP code / recovery-code
/// verification, independent of the coarser per-IP `totp_rate_limiter` - an
/// attacker who already has a valid password could otherwise distribute
/// guesses across many source IPs with no per-account backstop. Mirrors
/// the password step's `MAX_LOGIN_ATTEMPTS`/`LOGIN_WINDOW_MINUTES` in
/// `api::auth`.
const TOTP_MAX_ATTEMPTS: i64 = 5;
const TOTP_ATTEMPTS_WINDOW_MINUTES: i32 = 15;

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

/// Returns the exact stored hash that matched `input`, if any. The hash
/// (rather than an index into the caller's snapshot of the code list) is
/// what callers should use to consume the code, so the consumption can be
/// an atomic "remove this exact value" DB operation instead of a
/// read-modify-write of the whole array.
async fn verify_recovery_code(
    input: &str,
    hashed_codes: &[String],
) -> Result<Option<String>, ApiError> {
    let normalized_input = normalize_recovery_code(input);
    let hashes = hashed_codes.to_vec();
    tokio::task::spawn_blocking(move || -> Result<Option<String>, ApiError> {
        for hash in hashes {
            if bcrypt::verify(&normalized_input, &hash)
                .map_err(|e| ApiError::Internal(format!("failed to verify recovery code: {e}")))?
            {
                return Ok(Some(hash));
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
    create_totp_with_label(secret, None, String::new())
}

/// Same RFC 6238 parameters as [`create_totp`], but with an issuer/account
/// name label for the `otpauth://` URI shown during enrollment.
fn create_totp_with_label(
    secret: &[u8],
    issuer: Option<String>,
    account_name: String,
) -> Result<TOTP, ApiError> {
    // Use RFC 6238 defaults: 30-second period, 6 digits, SHA-1
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret.to_vec())
            .to_bytes()
            .map_err(|e| ApiError::Internal(format!("failed to parse TOTP secret: {e}")))?,
        issuer,
        account_name,
    )
    .map_err(|e| ApiError::Internal(format!("failed to create TOTP: {e}")))
}

fn generate_totp_secret() -> Vec<u8> {
    let mut secret = vec![0u8; 20];
    rand::RngCore::fill_bytes(&mut OsRng, &mut secret);
    secret
}

/// Compares two strings in constant time (with respect to their contents),
/// to avoid leaking how many leading bytes of a guessed TOTP code matched
/// the real one through response-timing differences.
fn constant_time_str_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |diff, (x, y)| diff | (x ^ y))
        == 0
}

/// Verify a TOTP code against the user's stored encrypted secret.
///
/// Returns the matched time-step index (`unix_time / totp.step`) if the
/// code is valid for one of the steps within the allowed skew, or `None`
/// if it doesn't match any of them. The step index (rather than just a
/// bool) lets callers implement precise replay protection: reject only a
/// code whose step was already consumed, not every code submitted within
/// some wall-clock window.
fn verify_totp_code(
    state: &AppState,
    encrypted_secret: &[u8],
    code: &str,
) -> Result<Option<i64>, ApiError> {
    let decrypted = shared::crypto::decrypt_passphrase(encrypted_secret, &state.encryption_key)?;
    let secret = hex::decode(&decrypted)
        .map_err(|e| ApiError::Internal(format!("failed to decode TOTP secret: {e}")))?;

    let totp = create_totp(&secret)?;
    let code = code.trim();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ApiError::Internal(format!("system time error: {e}")))?
        .as_secs();
    let current_step = i64::try_from(now.checked_div(totp.step).unwrap_or(0)).unwrap_or(i64::MAX);
    let skew = i64::from(totp.skew);
    let neg_skew = skew.checked_neg().unwrap_or(i64::MIN);

    for offset in neg_skew..=skew {
        let Some(step) = current_step.checked_add(offset) else {
            continue;
        };
        let Ok(step_u64) = u64::try_from(step) else {
            continue;
        };
        let Some(step_time) = step_u64.checked_mul(totp.step) else {
            continue;
        };
        if constant_time_str_eq(&totp.generate(step_time), code) {
            return Ok(Some(step));
        }
    }
    Ok(None)
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
    // Re-running setup while TOTP is already enabled would silently overwrite
    // the working secret and recovery codes for anyone with API access to an
    // authenticated session (e.g. via XSS) - require disabling first, which
    // is password-gated.
    let existing = db::get_user_totp_fields(&state.pool, auth.user_id).await?;
    if existing.is_some_and(|f| f.enabled) {
        return Err(ApiError::BadRequest(
            "TOTP is already enabled; disable it before setting it up again".to_string(),
        ));
    }

    // Generate a new secret
    let secret = generate_totp_secret();

    // Create TOTP object with issuer and account name for the URL
    let totp = create_totp_with_label(
        &secret,
        Some("Assimilate".to_string()),
        auth.username.clone(),
    )?;

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
/// is invalid. Returns [`ApiError::TooManyRequests`] if the account has
/// too many recent failed attempts. Returns [`ApiError::Internal`] if
/// decryption or TOTP verification fails. Returns [`ApiError::Database`]
/// if the DB query fails.
pub async fn totp_verify(
    State(state): State<AppState>,
    auth: AuthUser,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<TotpVerifyRequest>,
) -> Result<Json<TotpVerifyResponse>, ApiError> {
    // Same account-level lockout as totp_disable: a hijacked authenticated
    // session (stolen cookie, XSS) belonging to a user who started but
    // hasn't finished enrollment could otherwise brute-force this
    // guessable 6-digit code with no rate limit at all.
    let failed_count =
        db::count_failed_totp_attempts(&state.pool, auth.user_id, TOTP_ATTEMPTS_WINDOW_MINUTES)
            .await?;
    if failed_count >= TOTP_MAX_ATTEMPTS {
        return Err(ApiError::TooManyRequests(
            "Too many failed attempts. Try again later.".to_string(),
        ));
    }

    let totp_fields = db::get_user_totp_fields(&state.pool, auth.user_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    let Some(ref encrypted) = totp_fields.secret_encrypted else {
        return Err(ApiError::BadRequest("TOTP not set up".to_string()));
    };

    let Some(step) = verify_totp_code(&state, encrypted, &req.code)? else {
        let ip = state
            .client_ip_resolver
            .resolve(peer.ip(), &headers)
            .to_string();
        db::insert_totp_attempt(&state.pool, auth.user_id, &ip, false).await?;
        return Err(ApiError::BadRequest(
            "Invalid verification code".to_string(),
        ));
    };

    db::enable_user_totp(&state.pool, auth.user_id, step).await?;

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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<TotpLoginVerifyRequest>,
) -> Result<Response, ApiError> {
    // Verify the temp token session exists
    let temp_hashed = crate::api::tokens::hash_token(&req.temp_token);
    let temp_session = db::get_session(&state.pool, &temp_hashed)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid or expired temp token".to_string()))?;

    // A temp_token must be a pending_totp session - a fully authenticated
    // session's raw ID should never satisfy this endpoint's contract, even
    // though ownership already requires possessing a valid session token.
    if !temp_session.pending_totp {
        return Err(ApiError::Unauthorized(
            "invalid or expired temp token".to_string(),
        ));
    }

    let user = db::get_user_by_id(&state.pool, temp_session.user_id).await?;
    let ip = state
        .client_ip_resolver
        .resolve(peer.ip(), &headers)
        .to_string();

    let failed_count =
        db::count_failed_totp_attempts(&state.pool, user.id, TOTP_ATTEMPTS_WINDOW_MINUTES).await?;
    if failed_count >= TOTP_MAX_ATTEMPTS {
        return Err(ApiError::TooManyRequests(
            "Too many failed verification attempts. Try again later.".to_string(),
        ));
    }

    let totp_fields = db::get_user_totp_fields(&state.pool, user.id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    if !totp_fields.enabled {
        return Err(ApiError::BadRequest("TOTP not enabled".to_string()));
    }

    // Verify the TOTP code and find which time-step it matches.
    let Some(ref encrypted) = totp_fields.secret_encrypted else {
        return Err(ApiError::BadRequest("TOTP not configured".to_string()));
    };

    let Some(step) = verify_totp_code(&state, encrypted, &req.code)? else {
        db::insert_totp_attempt(&state.pool, user.id, &ip, false).await?;
        return Err(ApiError::Unauthorized(
            "invalid verification code".to_string(),
        ));
    };

    // TOTP replay protection: reject only if this code's time-step is at or
    // before the last one actually consumed - not merely because a prior
    // login happened recently. A wall-clock-only window would also reject a
    // second, different, currently-valid code from a different device
    // logging in shortly after the first (a real scenario given this PR's
    // multi-device session support), even though it isn't a replay at all.
    //
    // The check and the write happen in one atomic statement (rather than
    // read-then-write) so two requests racing the same code can't both
    // observe "not yet used" before either write lands.
    if !db::try_consume_totp_step(&state.pool, user.id, step).await? {
        return Err(ApiError::Unauthorized("TOTP code already used".to_string()));
    }

    // The TOTP step is the actual completion of login for a TOTP-enabled
    // account -- the password step (login()) deliberately deferred clearing
    // the account lockout / recording success until here, so a correct
    // password alone (without the TOTP code) can't reset the password-
    // lockout escalation tier or be recorded as a successful login.
    db::record_successful_login(&state.pool, &user.username, &ip).await?;

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
/// Returns [`ApiError::TooManyRequests`] if the account has too many recent
/// failed attempts.
/// Returns [`ApiError::Database`] if the DB query fails.
pub async fn totp_disable(
    State(state): State<AppState>,
    auth: AuthUser,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<TotpDisableRequest>,
) -> Result<Json<TotpVerifyResponse>, ApiError> {
    // Same account-level lockout as the TOTP/recovery login steps: this
    // endpoint accepts a password guess from an already-authenticated
    // session, which an attacker with a hijacked session (stolen cookie,
    // XSS) could otherwise use to brute-force the real password with no
    // rate limit at all, since it isn't behind the login router either.
    let failed_count =
        db::count_failed_totp_attempts(&state.pool, auth.user_id, TOTP_ATTEMPTS_WINDOW_MINUTES)
            .await?;
    if failed_count >= TOTP_MAX_ATTEMPTS {
        return Err(ApiError::TooManyRequests(
            "Too many failed attempts. Try again later.".to_string(),
        ));
    }

    let hash = db::get_user_password_hash_by_id(&state.pool, auth.user_id).await?;

    let valid = super::helpers::verify_password(req.password.clone(), hash)
        .await
        .map_err(|_| ApiError::BadRequest("incorrect password".to_string()))?;

    if !valid {
        let ip = state
            .client_ip_resolver
            .resolve(peer.ip(), &headers)
            .to_string();
        db::insert_totp_attempt(&state.pool, auth.user_id, &ip, false).await?;
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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<TotpRecoveryRequest>,
) -> Result<Response, ApiError> {
    // Validate the temp_token and get the user
    let temp_hashed = crate::api::tokens::hash_token(&req.temp_token);
    let session = db::get_session(&state.pool, &temp_hashed)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid temp token".to_string()))?;

    // A temp_token must be a pending_totp session - a fully authenticated
    // session's raw ID should never satisfy this endpoint's contract, even
    // though ownership already requires possessing a valid session token.
    if !session.pending_totp {
        return Err(ApiError::Unauthorized("invalid temp token".to_string()));
    }

    let ip = state
        .client_ip_resolver
        .resolve(peer.ip(), &headers)
        .to_string();

    let failed_count =
        db::count_failed_totp_attempts(&state.pool, session.user_id, TOTP_ATTEMPTS_WINDOW_MINUTES)
            .await?;
    if failed_count >= TOTP_MAX_ATTEMPTS {
        return Err(ApiError::TooManyRequests(
            "Too many failed verification attempts. Try again later.".to_string(),
        ));
    }

    let totp_fields = db::get_user_totp_fields(&state.pool, session.user_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("TOTP not set up".to_string()))?;

    let matched_hash = verify_recovery_code(&req.code, &totp_fields.recovery_codes).await?;
    let Some(matched_hash) = matched_hash else {
        db::insert_totp_attempt(&state.pool, session.user_id, &ip, false).await?;
        return Err(ApiError::Unauthorized("invalid recovery code".to_string()));
    };

    // Consume the matched code atomically (remove-if-present in one
    // statement) so a second request racing the same code can't also
    // observe it as still present and consume it a second time.
    if !db::try_consume_totp_recovery_code(&state.pool, session.user_id, &matched_hash).await? {
        return Err(ApiError::Unauthorized("invalid recovery code".to_string()));
    }

    // Recovery-code verification completes login the same way the TOTP-code
    // path does -- reset the password-lockout state and record the success,
    // matching totp_verify_login (see the reasoning in login()'s TOTP
    // branch: only a fully-completed login should do either).
    let user = db::get_user_by_id(&state.pool, session.user_id).await?;
    db::record_successful_login(&state.pool, &user.username, &ip).await?;

    // Delete the temp session
    db::delete_session(&state.pool, &temp_hashed).await?;

    // Create the real session using the shared helper
    let user_resp = super::users::user_row_to_response(&state.pool, user).await?;
    let response =
        super::auth::create_session_response(&state.pool, user_resp, session.remember_me).await?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_str_eq_matches_string_equality() {
        assert!(constant_time_str_eq("123456", "123456"));
        assert!(!constant_time_str_eq("123456", "123457"));
        assert!(!constant_time_str_eq("123456", "12345"));
        assert!(!constant_time_str_eq("", "123456"));
        assert!(constant_time_str_eq("", ""));
    }

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
        let matched = verify_recovery_code("abcd-1234-ef56-7890", std::slice::from_ref(&hashed))
            .await
            .unwrap();
        assert_eq!(matched, Some(hashed));
    }

    #[tokio::test]
    async fn recovery_code_verify_is_case_insensitive_and_ignores_dashes() {
        let code = "ABCD-1234-EF56-7890";
        let hashed = hash_recovery_code(code).await.unwrap();
        let matched = verify_recovery_code("abcd1234ef567890", std::slice::from_ref(&hashed))
            .await
            .unwrap();
        assert_eq!(matched, Some(hashed));
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
