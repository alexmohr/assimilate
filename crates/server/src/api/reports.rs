// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use super::auth::AuthUser;
use crate::{AppState, db, error::ApiError};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListReportsQuery {
    pub target: Option<String>,
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/reports",
    tag = "Reports",
    operation_id = "listReports",
    summary = "List backup reports for an agent",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
        ("target" = Option<String>, Query, description = "Filter by target repo name"),
        ("limit" = Option<i64>, Query, description = "Max entries to return"),
    ),
    responses(
        (status = 200, description = "List of backup reports", body = Vec<crate::db::ReportRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Client not found"),
    )
)]
pub async fn list_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<ListReportsQuery>,
) -> Result<Json<Vec<db::ReportRow>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let limit = query.limit.unwrap_or(50);
    let reports =
        db::list_reports_for_agent(&state.pool, agent.id, query.target.as_deref(), limit).await?;
    Ok(Json(reports))
}
