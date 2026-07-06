// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AuditEntry {
    pub id: i64,
    pub user_id: Option<i64>,
    pub username: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<i64>,
    pub details: Option<Value>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct NewAuditEntry<'a> {
    pub user_id: Option<i64>,
    pub username: &'a str,
    pub action: &'a str,
    pub target_type: Option<&'a str>,
    pub target_id: Option<i64>,
    pub details: Option<Value>,
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

pub struct AuditEntryFilters<'a> {
    pub page: i64,
    pub per_page: i64,
    pub filter_user_id: Option<i64>,
    pub filter_action: Option<&'a str>,
    pub filter_target_type: Option<&'a str>,
    pub filter_from: Option<DateTime<Utc>>,
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
