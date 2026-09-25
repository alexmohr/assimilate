// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde_json::Value;
use shared::{audit::AuditEvent, responses::AuditEntryResponse};
use sqlx::PgPool;

/// A row from the `audit_log` table, before its `action` and `details` are parsed into an
/// [`AuditEvent`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditEntryRow {
    /// Primary key.
    pub id: i64,
    /// User who performed the action, if authenticated.
    pub user_id: Option<i64>,
    /// Username at the time of the action.
    pub username: String,
    /// The audited action's name.
    pub action: String,
    /// Type of target resource, if applicable.
    pub target_type: Option<String>,
    /// ID of target resource, if applicable.
    pub target_id: Option<i64>,
    /// The audited action's details.
    pub details: Option<Value>,
    /// IP address from which the request originated.
    pub ip_address: Option<String>,
    /// When the entry was created.
    pub created_at: DateTime<Utc>,
}

impl TryFrom<AuditEntryRow> for AuditEntryResponse {
    type Error = serde_json::Error;

    fn try_from(row: AuditEntryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            user_id: row.user_id,
            username: row.username,
            event: AuditEvent::from_stored(&row.action, row.details)?,
            target_type: row.target_type,
            target_id: row.target_id,
            ip_address: row.ip_address,
            created_at: row.created_at,
        })
    }
}

/// Input parameters for inserting a new audit log entry.
pub struct NewAuditEntry<'a> {
    /// User who performed the action, if authenticated.
    pub user_id: Option<i64>,
    /// Username at the time of the action.
    pub username: &'a str,
    /// The audited action and its details.
    pub event: AuditEvent,
    /// Type of target resource, if applicable.
    pub target_type: Option<&'a str>,
    /// ID of target resource, if applicable.
    pub target_id: Option<i64>,
    /// Client IP address.
    pub ip_address: Option<&'a str>,
}

/// Errors from reading or writing the audit log.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// The database query failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// An audit event could not be stored, or a stored one could not be read back.
    #[error("invalid audit event: {0}")]
    Event(#[from] serde_json::Error),
}

impl From<AuditError> for crate::error::ApiError {
    fn from(err: AuditError) -> Self {
        match err {
            AuditError::Database(e) => Self::Database(e),
            AuditError::Event(e) => Self::Internal(format!("invalid audit event: {e}")),
        }
    }
}

/// # Errors
///
/// Returns an error if the event cannot be serialized or the database query fails.
pub async fn insert_audit_entry(
    pool: &PgPool,
    entry: &NewAuditEntry<'_>,
) -> Result<(), AuditError> {
    let (action, details) = entry.event.to_stored()?;
    sqlx::query!(
        "INSERT INTO audit_log (user_id, username, action, target_type, target_id, details, \
         ip_address) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        entry.user_id,
        entry.username,
        action,
        entry.target_type,
        entry.target_id,
        details,
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
/// Returns an error if the database query fails or a stored entry cannot be read.
pub async fn list_audit_entries(
    pool: &PgPool,
    filters: &AuditEntryFilters<'_>,
) -> Result<(Vec<AuditEntryResponse>, i64), AuditError> {
    let offset = filters
        .page
        .saturating_sub(1)
        .saturating_mul(filters.per_page);
    let rows = sqlx::query_as!(
        AuditEntryRow,
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

    let entries = rows
        .into_iter()
        .map(AuditEntryResponse::try_from)
        .collect::<Result<_, _>>()?;
    Ok((entries, total.unwrap_or(0)))
}
