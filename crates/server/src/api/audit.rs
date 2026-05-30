// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;

use super::auth::RequireAdmin;
use crate::{AppState, db, error::ApiError};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AuditLogQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub user_id: Option<i64>,
    pub action: Option<String>,
    pub target_type: Option<String>,
    #[schema(value_type = Option<String>)]
    pub from: Option<String>,
    #[schema(value_type = Option<String>)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AuditLogResponse {
    pub items: Vec<db::audit::AuditEntry>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

fn parse_iso_datetime(value: &str) -> Result<DateTime<Utc>, ApiError> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .map_err(|_| ApiError::BadRequest(format!("invalid datetime: {value}")))?;
    Ok(Utc.from_utc_datetime(&parsed))
}

#[utoipa::path(
    get,
    path = "/api/audit-log",
    tag = "System",
    operation_id = "listAuditLog",
    summary = "List audit log entries",
    params(
        ("page" = Option<i64>, Query, description = "Page number, default 1"),
        ("per_page" = Option<i64>, Query, description = "Page size, default 50, max 200"),
        ("user_id" = Option<i64>, Query, description = "Filter by user ID"),
        ("action" = Option<String>, Query, description = "Filter by action"),
        ("target_type" = Option<String>, Query, description = "Filter by target type"),
        ("from" = Option<String>, Query, description = "Filter from ISO datetime"),
        ("to" = Option<String>, Query, description = "Filter to ISO datetime"),
    ),
    responses(
        (status = 200, description = "Audit log entries", body = AuditLogResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn list_audit_log(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<AuditLogResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 200);
    let filter_from = query.from.as_deref().map(parse_iso_datetime).transpose()?;
    let filter_to = query.to.as_deref().map(parse_iso_datetime).transpose()?;

    let filters = db::audit::AuditEntryFilters {
        page,
        per_page,
        filter_user_id: query.user_id,
        filter_action: query.action.as_deref(),
        filter_target_type: query.target_type.as_deref(),
        filter_from,
        filter_to,
    };

    let (items, total) = db::audit::list_audit_entries(&state.pool, &filters).await?;

    Ok(Json(AuditLogResponse {
        items,
        total,
        page,
        per_page,
    }))
}
