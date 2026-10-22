// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Users, sessions, TOTP, login attempts and lockout, and preferences.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::SystemEventType;
use sqlx::PgPool;

use super::insert_system_event;
use crate::error::ApiError;

/// Exponential backoff durations for account lockout (indexed by escalation level).
pub const LOCKOUT_DURATIONS: &[i64] = &[1, 5, 15, 60, 1440];

/// A row from the `users` table (excluding the password hash).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct UserRow {
    /// Unique identifier.
    pub id: i64,
    /// Username for login.
    pub username: String,
    /// Whether the user must change their password on next login.
    pub must_change_password: bool,
    /// When the user was created.
    pub created_at: DateTime<Utc>,
    /// When the user last logged in.
    pub last_login_at: Option<DateTime<Utc>>,
    /// When the account is locked until (if applicable).
    pub locked_until: Option<DateTime<Utc>>,
}

/// A row from the `sessions` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    /// Session ID (token).
    pub id: String,
    /// User ID this session belongs to.
    pub user_id: i64,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session expires.
    pub expires_at: DateTime<Utc>,
    /// Whether the "remember me" flag was set.
    pub remember_me: bool,
    /// When the session was last used.
    pub last_seen_at: DateTime<Utc>,
    /// Whether this session is pending TOTP verification (pre-login temp session).
    pub pending_totp: bool,
}

/// A session row returned for user-facing session listing.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionForUser {
    /// Hashed session ID.
    pub id: String,
    /// User ID the session belongs to.
    pub user_id: i64,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session expires.
    pub expires_at: DateTime<Utc>,
    /// When the session was last used.
    pub last_seen_at: DateTime<Utc>,
    /// Whether the "remember me" flag was set.
    pub remember_me: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "INSERT INTO users (username, password_hash) VALUES ($1, $2) RETURNING id, username, \
         must_change_password, created_at, last_login_at, locked_until",
        username,
        password_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_username(pool: &PgPool, username: &str) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users WHERE username = $1",
        username,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_password_hash(
    pool: &PgPool,
    username: &str,
) -> Result<(UserRow, String), ApiError> {
    #[derive(sqlx::FromRow)]
    struct FullRow {
        id: i64,
        username: String,
        password_hash: String,
        must_change_password: bool,
        created_at: DateTime<Utc>,
        last_login_at: Option<DateTime<Utc>>,
        locked_until: Option<DateTime<Utc>>,
    }

    let row = sqlx::query_as!(
        FullRow,
        "SELECT id, username, password_hash, must_change_password, created_at, last_login_at, \
         locked_until FROM users WHERE username = $1",
        username,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })?;

    let user = UserRow {
        id: row.id,
        username: row.username,
        must_change_password: row.must_change_password,
        created_at: row.created_at,
        last_login_at: row.last_login_at,
        locked_until: row.locked_until,
    };
    Ok((user, row.password_hash))
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_id(pool: &PgPool, user_id: i64) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users WHERE id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_users(pool: &PgPool) -> Result<Vec<UserRow>, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// The `(id, username)` of each of `user_ids` that exists, ordered by ID.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_usernames_by_ids(
    pool: &PgPool,
    user_ids: &[i64],
) -> Result<Vec<(i64, String)>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, username FROM users WHERE id = ANY($1) ORDER BY id",
        user_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows.into_iter().map(|row| (row.id, row.username)).collect())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_user(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM users WHERE id = $1", user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_user_password(
    pool: &PgPool,
    user_id: i64,
    password_hash: &str,
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET password_hash = $2, must_change_password = false WHERE id = $1",
        user_id,
        password_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_login(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET last_login_at = NOW() WHERE id = $1",
        user_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_user_totp_fields(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<UserTotpFields>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        secret_encrypted: Option<Vec<u8>>,
        enabled: bool,
        recovery_codes: Option<Vec<String>>,
        last_verified_step: Option<i64>,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT totp_secret_encrypted AS secret_encrypted, totp_enabled AS enabled, \
         totp_recovery_codes AS recovery_codes, totp_last_verified_step AS last_verified_step \
         FROM users WHERE id = $1",
        user_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(match row {
        Some(r) => {
            if r.secret_encrypted.is_some() {
                Some(UserTotpFields {
                    secret_encrypted: r.secret_encrypted,
                    enabled: r.enabled,
                    recovery_codes: r.recovery_codes.unwrap_or_default(),
                    last_verified_step: r.last_verified_step,
                })
            } else {
                None
            }
        }
        None => None,
    })
}

/// TOTP configuration fields for a user.
pub struct UserTotpFields {
    /// Encrypted TOTP secret (AES-256-GCM).
    pub secret_encrypted: Option<Vec<u8>>,
    /// Whether TOTP is enabled for this user.
    pub enabled: bool,
    /// Hashed recovery codes.
    pub recovery_codes: Vec<String>,
    /// The most recent TOTP time-step (`unix_time / step`) successfully
    /// consumed during login, used for replay protection.
    pub last_verified_step: Option<i64>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_user_totp_secret(
    pool: &PgPool,
    user_id: i64,
    encrypted_secret: &[u8],
    recovery_codes: &[String],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_secret_encrypted = $2, totp_recovery_codes = $3 WHERE id = $1",
        user_id,
        encrypted_secret,
        recovery_codes,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn enable_user_totp(
    pool: &PgPool,
    user_id: i64,
    verified_step: i64,
) -> Result<(), ApiError> {
    // Record the step consumed by the enrollment code itself, so it can't be
    // replayed against the login endpoint for the rest of its validity window.
    sqlx::query!(
        "UPDATE users SET totp_enabled = true, totp_last_verified_step = $2 WHERE id = $1",
        user_id,
        verified_step,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn disable_user_totp(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_enabled = false, totp_secret_encrypted = NULL, totp_recovery_codes \
         = NULL, totp_last_verified_step = NULL WHERE id = $1",
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn replace_totp_recovery_codes(
    pool: &PgPool,
    user_id: i64,
    recovery_codes: &[String],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_recovery_codes = $2 WHERE id = $1",
        user_id,
        recovery_codes,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Atomically removes exactly one recovery code (matched by its stored
/// hash) from a user's recovery-code list, in a single statement rather
/// than a read-modify-write of the whole array, so two requests racing the
/// same code can't both observe it as still present before either write
/// commits. Returns `true` if the hash was found and removed, `false` if it
/// was already gone (e.g. consumed by a concurrent request).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn try_consume_totp_recovery_code(
    pool: &PgPool,
    user_id: i64,
    code_hash: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET totp_recovery_codes = array_remove(totp_recovery_codes, $2) WHERE id = \
         $1 AND $2 = ANY(totp_recovery_codes)",
        user_id,
        code_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Atomically checks and records the TOTP step consumed by a login, in a
/// single statement rather than a separate read-then-write, so two
/// concurrent requests racing the same code can't both pass the replay
/// check before either write commits. Returns `true` if `step` was newer
/// than whatever was previously recorded (and is now recorded), `false` if
/// it was a replay (at or before the recorded step) and the row was left
/// unchanged.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn try_consume_totp_step(
    pool: &PgPool,
    user_id: i64,
    step: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET totp_last_verified_step = $2 WHERE id = $1 AND (totp_last_verified_step \
         IS NULL OR totp_last_verified_step < $2)",
        user_id,
        step,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_session_last_seen(pool: &PgPool, session_id: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = NOW() WHERE id = $1",
        session_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_sessions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<SessionForUser>, ApiError> {
    sqlx::query_as!(
        SessionForUser,
        "SELECT id, user_id, created_at, expires_at, last_seen_at, remember_me FROM sessions \
         WHERE user_id = $1 AND expires_at > NOW() AND pending_totp = false ORDER BY created_at \
         DESC",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_session_by_id(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2",
        session_id,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_user_password_hash_by_id(pool: &PgPool, user_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        password_hash: String,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT password_hash FROM users WHERE id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })?;
    Ok(row.password_hash)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_session(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
    remember_me: bool,
    pending_totp: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, expires_at, remember_me, last_seen_at, pending_totp) \
         VALUES ($1, $2, $3, $4, NOW(), $5)",
        session_id,
        user_id,
        expires_at,
        remember_me,
        pending_totp,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Unauthorized`]: the caller is not authenticated
/// - [`ApiError::Database`]: the database query fails
pub async fn get_session(pool: &PgPool, session_id: &str) -> Result<SessionRow, ApiError> {
    sqlx::query_as!(
        SessionRow,
        "SELECT id, user_id, created_at, expires_at, remember_me, last_seen_at, pending_totp FROM \
         sessions WHERE id = $1 AND expires_at > NOW()",
        session_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::Unauthorized("session expired or invalid".to_string())
        }
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn extend_session(
    pool: &PgPool,
    session_id: &str,
    new_expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE sessions SET expires_at = $1 WHERE id = $2",
        new_expires_at,
        session_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM sessions WHERE id = $1", session_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_expired_sessions(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM sessions WHERE expires_at <= NOW()")
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn user_count(pool: &PgPool) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(CountRow, "SELECT COUNT(*) as count FROM users")
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_login_attempts(
    pool: &PgPool,
    username: &str,
    ip: &str,
    window_minutes: i32,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND ip = $2 AND success \
         = false AND attempted_at > NOW() - ($3 || ' minutes')::INTERVAL",
        username,
        ip,
        window_minutes.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_login_attempt(
    pool: &PgPool,
    username: &str,
    ip: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, $3)",
        username,
        ip,
        success,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_totp_attempts(
    pool: &PgPool,
    user_id: i64,
    window_minutes: i32,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM totp_attempts WHERE user_id = $1 AND success = false AND \
         attempted_at > NOW() - ($2 || ' minutes')::INTERVAL",
        user_id,
        window_minutes.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Clear the lockout for an account and reset its lockout-escalation
/// counter. Called after a successful login so a future lockout starts
/// back at the shortest tier.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn clear_account_lockout<'e, E>(executor: E, username: &str) -> Result<(), ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        "UPDATE users SET locked_until = NULL, lockout_escalation_level = 0 WHERE username = $1",
        username,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records a fully-completed successful login: clears any account lockout
/// and resets the escalation counter, and inserts a `success = true`
/// `login_attempts` row. Wraps both writes in one transaction (matching
/// [`record_failed_login_and_check_lockout`]'s treatment of the failure
/// path) so a mid-write DB error can't wipe the lockout state without also
/// recording the successful login that justified clearing it.
///
/// Callers must only invoke this once authentication has *fully* completed
/// -- i.e. after any required TOTP step, not merely after the password
/// check -- since this both resets the password-lockout escalation tier and
/// records the attempt as successful in the audit trail.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn record_successful_login(
    pool: &PgPool,
    username: &str,
    ip: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    clear_account_lockout(&mut *tx, username).await?;

    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, true)",
        username,
        ip,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Count failed login attempts since the last successful login for the given
/// username. If there has never been a successful login, counts all failures.
/// Generic over the executor so callers running inside a transaction (e.g.
/// [`record_failed_login_and_check_lockout`]) can reuse this instead of
/// re-embedding the same query against a `&mut Transaction`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn count_failed_attempts_since_last_success<'e, E>(
    executor: E,
    username: &str,
) -> Result<i64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND success = false AND \
         attempted_at > COALESCE((SELECT MAX(attempted_at) FROM login_attempts WHERE username = \
         $1 AND success = true), '-infinity'::TIMESTAMPTZ)",
        username,
    )
    .fetch_one(executor)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.count.unwrap_or(0))
}

/// Counts failed attempts that belong to the *current* lockout cycle --
/// i.e. failures recorded after the later of the last successful login or
/// `locked_until` (the account's most recent lock, whether still active or
/// already expired).
///
/// This is deliberately narrower than
/// [`count_failed_attempts_since_last_success`]: `login()`'s locked-account
/// branch calls [`record_failed_login_and_check_lockout`] on every attempt
/// against a locked account (for timing-uniformity reasons), so those
/// attempts get recorded as failures too. If the escalation gate counted
/// *all* failures since the last success, that count would never reset on
/// its own -- once an account first crosses `max_account_failures`, the
/// count is already inflated past the threshold forever, so the very next
/// failed attempt after any future lockout expires (not a fresh batch of
/// `max_account_failures`) would immediately re-trigger escalation. That
/// both lets an attacker keep an account locked at the maximum tier
/// indefinitely with roughly one low-frequency attempt per cycle, and lets
/// a locked-out legitimate user ratchet their own account up through
/// repeated retries. Excluding everything up to and including
/// `locked_until` means each lock's window (which is always at least
/// `LOCKOUT_DURATIONS[0]` long) guarantees every attempt from the prior
/// cycle -- including ones made while locked -- falls at or before the
/// cutoff, so a genuinely fresh `max_account_failures` is required to
/// escalate again after each lock expires.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
async fn count_failed_attempts_in_current_cycle<'e, E>(
    executor: E,
    username: &str,
    locked_until: Option<DateTime<Utc>>,
) -> Result<i64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND success = false AND \
         attempted_at > GREATEST(COALESCE((SELECT MAX(attempted_at) FROM login_attempts WHERE \
         username = $1 AND success = true), '-infinity'::TIMESTAMPTZ), COALESCE($2::TIMESTAMPTZ, \
         '-infinity'::TIMESTAMPTZ))",
        username,
        locked_until,
    )
    .fetch_one(executor)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.count.unwrap_or(0))
}

/// Record a failed login attempt and check if the account should be locked.
///
/// Escalates the lockout duration by lockout *cycle*, not by raw failure
/// count: `users.lockout_escalation_level` only advances when this call
/// actually establishes a new lockout (the account isn't already locked),
/// and the threshold check itself only counts failures from the current
/// cycle (see [`count_failed_attempts_in_current_cycle`]) so a fresh
/// `max_account_failures` is required every cycle, not just once ever.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn record_failed_login_and_check_lockout(
    pool: &PgPool,
    username: &str,
    ip: &str,
    max_account_failures: i64,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, false)",
        username,
        ip,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Lock the user row (a no-op if the username doesn't exist -- the
    // dummy-hash path) so concurrent failed attempts for the same account
    // serialize here: without this, two concurrent transactions could each
    // count the failures committed so far, both land just under
    // `max_account_failures`, and both skip escalation even though their
    // combined total already crossed the threshold. Also read back
    // `locked_until` while holding the lock, needed to scope the count
    // below to the current cycle.
    let locked_until_before: Option<DateTime<Utc>> = sqlx::query_scalar!(
        "SELECT locked_until FROM users WHERE username = $1 FOR UPDATE",
        username
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::Database)?
    .flatten();

    // Count failures in the current cycle only -- see
    // count_failed_attempts_in_current_cycle's doc comment for why this
    // must exclude attempts made before/during the most recent lock rather
    // than counting everything since the last successful login.
    let count =
        count_failed_attempts_in_current_cycle(&mut *tx, username, locked_until_before).await?;

    if count >= max_account_failures {
        // Advance the per-cycle escalation counter only if the account isn't
        // already locked. `login`'s locked-account branch calls this
        // function on every attempt (to keep DB work -- and therefore
        // response timing -- identical to the wrong-password branch), so
        // this guard is load-bearing: without it, continued brute-forcing
        // against an already-locked account would keep extending
        // `locked_until` and advancing the escalation tier further on every
        // single attempt, rather than only once per lockout cycle.
        let escalated: Option<i32> = sqlx::query_scalar!(
            "UPDATE users SET lockout_escalation_level = lockout_escalation_level + 1 WHERE \
             username = $1 AND (locked_until IS NULL OR locked_until <= NOW()) RETURNING \
             lockout_escalation_level",
            username,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        // `None` means either the user doesn't exist (the bcrypt dummy-hash
        // path) or the account is already locked -- nothing further to do.
        let Some(escalated) = escalated else {
            tx.commit().await.map_err(ApiError::Database)?;
            return Ok(());
        };

        // The counter was just incremented, so this lockout's tier is one
        // less than the new value -- the first lockout uses index 0, the
        // shortest duration.
        let escalation_level = i64::from(escalated).saturating_sub(1);
        let duration_minutes = LOCKOUT_DURATIONS
            .get(usize::try_from(escalation_level).unwrap_or(0))
            .copied()
            .unwrap_or(*LOCKOUT_DURATIONS.last().unwrap_or(&1));
        let locked_until = Utc::now()
            .checked_add_signed(chrono::Duration::try_minutes(duration_minutes).unwrap_or_default())
            .unwrap_or(Utc::now());

        sqlx::query!(
            "UPDATE users SET locked_until = $1 WHERE username = $2",
            locked_until,
            username,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        tx.commit().await.map_err(ApiError::Database)?;

        // Spawned rather than awaited: this insert is pure audit logging,
        // not part of the login response's correctness, and awaiting it
        // here would make the threshold-crossing attempt measurably slower
        // than every attempt before it -- a narrow timing signal an
        // attacker could use to detect exactly which attempt locked the
        // account, in the same spirit as the timing-uniformity work
        // elsewhere in this function.
        let pool = pool.clone();
        let message = format!(
            "Account '{username}' locked until {locked_until} after {count} failed attempts"
        );
        tokio::spawn(async move {
            let _ =
                insert_system_event(&pool, SystemEventType::AccountLocked, None, &message).await;
        });

        return Ok(());
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_totp_attempt(
    pool: &PgPool,
    user_id: i64,
    ip: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO totp_attempts (user_id, ip, success) VALUES ($1, $2, $3)",
        user_id,
        ip,
        success,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Prunes old login-attempt history by age. `record_failed_login_and_check_lockout`
/// now also records attempts against nonexistent usernames (needed for the
/// constant-time dummy-hash login path), so this table grows without bound
/// otherwise. A 90-day cutoff is far longer than the longest lockout tier
/// (24h), so this cannot interfere with an in-progress lockout escalation in
/// practice.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_login_attempts_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM login_attempts WHERE attempted_at < $1", before)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Get user preferences.
///
/// A user's stored preferences, or the defaults when they have none.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails, or [`ApiError::Internal`] if
/// the stored preferences cannot be read.
pub async fn get_user_preferences(
    pool: &PgPool,
    user_id: i64,
) -> Result<shared::responses::UserPreferences, ApiError> {
    let row: Option<serde_json::Value> =
        sqlx::query_scalar!("SELECT preferences FROM users WHERE id = $1", user_id)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    row.map_or_else(
        || Ok(shared::responses::UserPreferences::default()),
        |stored| {
            serde_json::from_value(stored).map_err(|e| {
                ApiError::Internal(format!(
                    "user {user_id} has invalid stored preferences: {e}"
                ))
            })
        },
    )
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails, or [`ApiError::Internal`] if
/// the preferences cannot be serialized.
pub async fn set_user_preferences(
    pool: &PgPool,
    user_id: i64,
    preferences: &shared::responses::UserPreferences,
) -> Result<(), ApiError> {
    let stored = serde_json::to_value(preferences)
        .map_err(|e| ApiError::Internal(format!("failed to serialize preferences: {e}")))?;
    sqlx::query!(
        "UPDATE users SET preferences = $1 WHERE id = $2",
        stored,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
