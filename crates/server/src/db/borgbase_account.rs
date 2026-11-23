// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `BorgBase` account the server manages repositories on: its encrypted API token.

use sqlx::PgPool;

use crate::error::ApiError;

/// Stores the encrypted API token, replacing any earlier one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn set_api_token(pool: &PgPool, token_encrypted: &[u8]) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO borgbase_account (singleton, api_token_encrypted) VALUES (TRUE, $1) ON \
         CONFLICT (singleton) DO UPDATE SET api_token_encrypted = EXCLUDED.api_token_encrypted, \
         updated_at = NOW()",
        token_encrypted,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// The encrypted API token, if one is stored.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_api_token(pool: &PgPool) -> Result<Option<Vec<u8>>, ApiError> {
    sqlx::query_scalar!("SELECT api_token_encrypted FROM borgbase_account")
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)
}

/// Removes the stored API token. Returns whether there was one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn clear_api_token(pool: &PgPool) -> Result<bool, ApiError> {
    let result = sqlx::query!("DELETE FROM borgbase_account")
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}
