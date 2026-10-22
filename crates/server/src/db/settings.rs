// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! System settings, global excludes and database storage figures.

use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiError;

/// Global exclude patterns applied to all schedules.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GlobalExcludesConfig {
    /// Raw exclude pattern text.
    pub raw_text: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_global_excludes_raw(pool: &PgPool) -> Result<String, ApiError> {
    let row: Option<String> =
        sqlx::query_scalar!("SELECT raw_text FROM excludes_global_config LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row.unwrap_or_default())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_global_excludes_raw(pool: &PgPool, raw_text: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO excludes_global_config (raw_text) VALUES ($1) ON CONFLICT (id) DO UPDATE SET \
         raw_text = EXCLUDED.raw_text",
        raw_text,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_setting(pool: &PgPool, key: &str) -> Result<Option<String>, ApiError> {
    let row: Option<String> =
        sqlx::query_scalar!("SELECT value FROM system_settings WHERE key = $1", key)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row)
}

/// Reads a setting and parses it, logging (without failing) if the stored
/// value is present but not parseable as `T`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_parsed_setting<T: std::str::FromStr>(
    pool: &PgPool,
    key: &str,
) -> Result<Option<T>, ApiError>
where
    T::Err: std::fmt::Display,
{
    Ok(get_setting(pool, key).await?.and_then(|v| {
        v.parse::<T>()
            .inspect_err(|e| tracing::warn!(setting = key, value = %v, error = %e, "failed to parse setting"))
            .ok()
    }))
}

/// Writes several settings in one statement, so a reader or a concurrent
/// writer sees all of them or none of them. Two concurrent calls over the same
/// keys leave one call's values for every key, never a mix of both: the later
/// statement waits on the earlier one's rows and then overwrites them all.
///
/// Each key may appear only once in `entries`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_settings(pool: &PgPool, entries: &[(&str, &str)]) -> Result<(), ApiError> {
    let (keys, values): (Vec<String>, Vec<String>) = entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .unzip();
    sqlx::query!(
        "INSERT INTO system_settings (key, value, updated_at) SELECT key, value, NOW() FROM \
         UNNEST($1::text[], $2::text[]) AS entry(key, value) ON CONFLICT (key) DO UPDATE SET \
         value = EXCLUDED.value, updated_at = NOW()",
        &keys,
        &values,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO system_settings (key, value, updated_at) VALUES ($1, $2, NOW()) ON CONFLICT \
         (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
        key,
        value,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Size breakdown of a database relation (table + indexes + TOAST).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DatabaseRelationSizeRow {
    /// Table name.
    pub table_name: String,
    /// Bytes used by the main table.
    pub table_bytes: i64,
    /// Bytes used by indexes.
    pub index_bytes: i64,
    /// Bytes used by TOAST storage.
    pub toast_bytes: i64,
    /// Total bytes (table + indexes + TOAST).
    pub total_bytes: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_database_storage(
    pool: &PgPool,
) -> Result<(i64, Vec<DatabaseRelationSizeRow>), ApiError> {
    let total_bytes: Option<i64> =
        sqlx::query_scalar!("SELECT pg_database_size(current_database())::BIGINT",)
            .fetch_one(pool)
            .await
            .map_err(ApiError::Database)?;

    let relations = sqlx::query_as!(
        DatabaseRelationSizeRow,
        "WITH sizes AS ( SELECT relname::TEXT AS table_name, pg_relation_size(relid)::BIGINT AS \
         table_bytes, pg_indexes_size(relid)::BIGINT AS index_bytes, \
         (pg_total_relation_size(relid) - pg_relation_size(relid) - \
         pg_indexes_size(relid))::BIGINT AS toast_bytes, pg_total_relation_size(relid)::BIGINT AS \
         total_bytes FROM pg_catalog.pg_statio_user_tables ) SELECT table_name AS \
         \"table_name!\", table_bytes AS \"table_bytes!\", index_bytes AS \"index_bytes!\", \
         toast_bytes AS \"toast_bytes!\", total_bytes AS \"total_bytes!\" FROM sizes ORDER BY \
         total_bytes DESC, table_name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok((total_bytes.unwrap_or(0), relations))
}

/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn get_schedule_timezone(pool: &PgPool) -> Result<chrono_tz::Tz, ApiError> {
    let tz_str = get_setting(pool, "timezone").await?.unwrap_or_default();
    shared::schedule::parse_timezone(&tz_str)
        .map_err(|e| ApiError::Internal(format!("invalid timezone setting: {e}")))
}
