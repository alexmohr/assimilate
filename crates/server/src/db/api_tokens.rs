// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! API tokens.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::ApiTokenScope;
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `api_tokens` table (excluding the token hash).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApiTokenRow {
    /// Unique identifier.
    pub id: i64,
    /// User ID that owns this token.
    pub user_id: i64,
    /// Human-readable token name.
    pub name: String,
    /// What the token may be used for.
    pub scope: ApiTokenScope,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// When the token was last used.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Inserts a full-scope API token.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_api_token(
    pool: &PgPool,
    user_id: i64,
    name: &str,
    token_hash: &str,
) -> Result<ApiTokenRow, ApiError> {
    insert_api_token_with_scope(pool, user_id, name, token_hash, ApiTokenScope::Full).await
}

/// Inserts an API token limited to `scope`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_api_token_with_scope(
    pool: &PgPool,
    user_id: i64,
    name: &str,
    token_hash: &str,
    scope: ApiTokenScope,
) -> Result<ApiTokenRow, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        r#"INSERT INTO api_tokens (user_id, name, token_hash, scope) VALUES ($1, $2, $3, $4)
           RETURNING id, user_id, name, scope AS "scope: ApiTokenScope", created_at, last_used_at"#,
        user_id,
        name,
        token_hash,
        scope.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_api_tokens_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        r#"SELECT id, user_id, name, scope AS "scope: ApiTokenScope", created_at, last_used_at
           FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC"#,
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_api_tokens(pool: &PgPool) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        r#"SELECT id, user_id, name, scope AS "scope: ApiTokenScope", created_at, last_used_at
           FROM api_tokens ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_api_token(pool: &PgPool, token_id: i64) -> Result<ApiTokenRow, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        r#"DELETE FROM api_tokens WHERE id = $1
           RETURNING id, user_id, name, scope AS "scope: ApiTokenScope", created_at, last_used_at"#,
        token_id
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("api token {token_id} not found")))
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_api_token_owner(pool: &PgPool, token_id: i64) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT user_id FROM api_tokens WHERE id = $1",
        token_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("api token {token_id} not found")),
        other => ApiError::Database(other),
    })?;
    Ok(row.user_id)
}

/// Minimal row for API token lookup: who owns the token and what it may do.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiTokenLookupRow {
    /// User ID that owns the token.
    pub user_id: i64,
    /// What the token may be used for.
    pub scope: ApiTokenScope,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Unauthorized`]: the caller is not authenticated
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<ApiTokenLookupRow, ApiError> {
    let row = sqlx::query_as!(
        ApiTokenLookupRow,
        r#"SELECT user_id, scope AS "scope: ApiTokenScope" FROM api_tokens WHERE token_hash = $1"#,
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::Unauthorized("invalid api token".to_string()),
        other => ApiError::Database(other),
    })?;
    Ok(row)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_api_token_last_used(pool: &PgPool, token_hash: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE api_tokens SET last_used_at = NOW() WHERE token_hash = $1",
        token_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records a scrape by the monitoring token with `token_hash`, returning
/// whether such a token exists. A full-scope token with the same hash does not
/// count: `/metrics` accepts monitoring tokens only.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn touch_monitoring_token(pool: &PgPool, token_hash: &str) -> Result<bool, ApiError> {
    let id = sqlx::query_scalar!(
        "UPDATE api_tokens SET last_used_at = NOW() WHERE token_hash = $1 AND scope = $2 \
         RETURNING id",
        token_hash,
        ApiTokenScope::Monitoring.to_string(),
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(id.is_some())
}
