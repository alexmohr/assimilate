// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

/// A row from the `audit_log` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AuditEntry {
    /// Primary key.
    pub id: i64,
    /// User who performed the action, if authenticated.
    pub user_id: Option<i64>,
    /// Username at the time of the action.
    pub username: String,
    /// Action identifier (e.g. "login", "repo.create").
    pub action: String,
    /// Type of target resource, if applicable.
    pub target_type: Option<String>,
    /// ID of target resource, if applicable.
    pub target_id: Option<i64>,
    /// Arbitrary JSON payload with action-specific details.
    pub details: Option<Value>,
    /// IP address from which the request originated.
    pub ip_address: Option<String>,
    /// When the entry was created.
    pub created_at: DateTime<Utc>,
}

/// Input parameters for inserting a new audit log entry.
pub struct NewAuditEntry<'a> {
    /// User who performed the action, if authenticated.
    pub user_id: Option<i64>,
    /// Username at the time of the action.
    pub username: &'a str,
    /// Action identifier.
    pub action: &'a str,
    /// Type of target resource, if applicable.
    pub target_type: Option<&'a str>,
    /// ID of target resource, if applicable.
    pub target_id: Option<i64>,
    /// Arbitrary JSON payload.
    pub details: Option<Value>,
    /// Client IP address.
    pub ip_address: Option<&'a str>,
}

/// # Errors
///
/// Returns an error if the database query fails.
pub async fn insert_audit_entry(
    pool: &PgPool,
    entry: &NewAuditEntry<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO audit_log (user_id, username, action, target_type, target_id, details, \
         ip_address) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        entry.user_id,
        entry.username,
        entry.action,
        entry.target_type,
        entry.target_id,
        entry.details,
        entry.ip_address,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Filtering and pagination parameters for listing audit log entries.
pub struct AuditEntryFilters<'a> {
    /// Page number (1-indexed).
    pub page: i64,
    /// Number of entries per page.
    pub per_page: i64,
    /// Filter by user ID.
    pub filter_user_id: Option<i64>,
    /// Filter by action name.
    pub filter_action: Option<&'a str>,
    /// Filter by target type.
    pub filter_target_type: Option<&'a str>,
    /// Only include entries created at or after this time.
    pub filter_from: Option<DateTime<Utc>>,
    /// Only include entries created at or before this time.
    pub filter_to: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns an error if the database query fails.
pub async fn list_audit_entries(
    pool: &PgPool,
    filters: &AuditEntryFilters<'_>,
) -> Result<(Vec<AuditEntry>, i64), sqlx::Error> {
    let offset = filters
        .page
        .saturating_sub(1)
        .saturating_mul(filters.per_page);
    let rows = sqlx::query_as!(
        AuditEntry,
        "SELECT id, user_id, username, action, target_type, target_id, details, ip_address, \
         created_at
         FROM audit_log
         WHERE ($1::BIGINT IS NULL OR user_id = $1)
           AND ($2::TEXT IS NULL OR action = $2)
           AND ($3::TEXT IS NULL OR target_type = $3)
           AND ($4::TIMESTAMPTZ IS NULL OR created_at >= $4)
           AND ($5::TIMESTAMPTZ IS NULL OR created_at <= $5)
         ORDER BY created_at DESC, id DESC
         LIMIT $6 OFFSET $7",
        filters.filter_user_id,
        filters.filter_action,
        filters.filter_target_type,
        filters.filter_from,
        filters.filter_to,
        filters.per_page,
        offset,
    )
    .fetch_all(pool)
    .await?;

    let total = sqlx::query_scalar!(
        "SELECT COUNT(*)
         FROM audit_log
         WHERE ($1::BIGINT IS NULL OR user_id = $1)
           AND ($2::TEXT IS NULL OR action = $2)
           AND ($3::TEXT IS NULL OR target_type = $3)
           AND ($4::TIMESTAMPTZ IS NULL OR created_at >= $4)
           AND ($5::TIMESTAMPTZ IS NULL OR created_at <= $5)",
        filters.filter_user_id,
        filters.filter_action,
        filters.filter_target_type,
        filters.filter_from,
        filters.filter_to,
    )
    .fetch_one(pool)
    .await?;

    Ok((rows, total.unwrap_or(0)))
}
