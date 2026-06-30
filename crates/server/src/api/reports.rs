// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::responses::ReportResponse;

use super::auth::AuthUser;
use crate::{AppState, db, error::ApiError};

fn row_to_report_response(row: db::ReportRow, hostname: String) -> ReportResponse {
    ReportResponse {
        id: row.id,
        agent_id: row.agent_id,
        repo_id: row.repo_id,
        schedule_id: row.schedule_id,
        started_at: row.started_at,
        finished_at: row.finished_at,
        status: row.status,
        original_size: row.original_size,
        compressed_size: row.compressed_size,
        deduplicated_size: row.deduplicated_size,
        files_processed: row.files_processed,
        duration_secs: row.duration_secs,
        error_message: row.error_message,
        warnings: row.warnings,
        borg_version: row.borg_version,
        archive_name: row.archive_name,
        borg_command: row.borg_command,
        hostname: Some(hostname),
        repo_name: Some(row.repo_name),
        schedule_name: row.schedule_name,
    }
}

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
        ("hostname" = String, Path, description = "Agent hostname"),
        ("target" = Option<String>, Query, description = "Filter by target repo name"),
        ("limit" = Option<i64>, Query, description = "Max entries to return"),
    ),
    responses(
        (status = 200, description = "List of backup reports", body = Vec<ReportResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Agent not found"),
    )
)]
pub async fn list_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<ListReportsQuery>,
) -> Result<Json<Vec<ReportResponse>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let limit = query.limit.unwrap_or(50);
    let hostname_clone = hostname.clone();
    let reports: Vec<ReportResponse> =
        db::list_reports_for_agent(&state.pool, agent.id, query.target.as_deref(), limit)
            .await?
            .into_iter()
            .map(|r| row_to_report_response(r, hostname_clone.clone()))
            .collect();
    Ok(Json(reports))
}
