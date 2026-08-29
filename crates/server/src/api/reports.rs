// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::responses::{DeleteFailedReportsResponse, FailedReportCountResponse, ReportResponse};
use tracing::warn;

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::DomainQuery,
};
use crate::{AppState, db, error::ApiError};

#[cfg(test)]
mod tests {
    use shared::types::BackupStatus;

    use super::*;

    fn make_row(status: &str) -> db::ReportRow {
        db::ReportRow {
            id: 1,
            agent_id: 1,
            repo_id: 1,
            repo_name: "test-repo".to_owned(),
            schedule_id: Some(1),
            schedule_name: None,
            started_at: chrono::Utc::now(),
            finished_at: chrono::Utc::now(),
            status: status.to_owned(),
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            archive_name: None,
            borg_command: None,
        }
    }

    #[test]
    fn row_to_report_response_parses_valid_status() {
        let row = make_row("success");
        let resp = row_to_report_response(row, "myhost".to_owned());
        assert_eq!(resp.status, BackupStatus::Success);
    }

    #[test]
    fn row_to_report_response_parses_failed_status() {
        let row = make_row("failed");
        let resp = row_to_report_response(row, "myhost".to_owned());
        assert_eq!(resp.status, BackupStatus::Failed);
    }

    #[test]
    fn row_to_report_response_falls_back_to_success_on_invalid_status() {
        let row = make_row("corrupted_status_value");
        let resp = row_to_report_response(row, "myhost".to_owned());
        assert_eq!(resp.status, BackupStatus::Success);
    }

    #[test]
    fn row_to_report_response_hostname_is_set() {
        let row = make_row("success");
        let resp = row_to_report_response(row, "webserver-01".to_owned());
        assert_eq!(resp.hostname, Some("webserver-01".to_owned()));
    }
}

fn row_to_report_response(row: db::ReportRow, hostname: String) -> ReportResponse {
    ReportResponse {
        id: row.id,
        agent_id: row.agent_id,
        repo_id: row.repo_id,
        schedule_id: row.schedule_id,
        started_at: row.started_at,
        finished_at: row.finished_at,
        status: row.status.parse().unwrap_or_else(|e| {
            warn!(
                error = %e,
                raw_status = %row.status,
                "failed to parse backup status, defaulting to Success"
            );
            shared::types::BackupStatus::default()
        }),
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

/// Query parameters for listing backup reports.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListReportsQuery {
    /// Filter by target repository name.
    pub target: Option<String>,
    /// Maximum number of reports to return.
    pub limit: Option<i64>,
    /// Domain, required if the hostname is shared by multiple agents.
    pub domain: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/reports",
    tag = "Reports",
    operation_id = "listReports",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("target" = Option<String>, Query, description = "Filter by target repo name"),
        ("limit" = Option<i64>, Query, description = "Max entries to return"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "List of backup reports", body = Vec<ReportResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Agent not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// List backup reports for an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<ListReportsQuery>,
) -> Result<Json<Vec<ReportResponse>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
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

#[utoipa::path(
    delete,
    path = "/api/agents/{hostname}/reports/failed",
    tag = "Reports",
    operation_id = "deleteFailedReports",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Failed reports deleted", body = DeleteFailedReportsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Agent not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// Delete all failed backup reports for an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_failed_reports(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<DeleteFailedReportsResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let deleted = db::delete_failed_backup_reports_for_agent(&state.pool, agent.id).await?;
    Ok(Json(DeleteFailedReportsResponse { deleted }))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/reports/failed/count",
    tag = "Reports",
    operation_id = "countFailedReports",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Failed report count", body = FailedReportCountResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Agent not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// Count an agent's failed backup reports, unbounded by the report list's own
/// pagination window - the true number a "clean up failed backups"
/// confirmation is about to delete.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn count_failed_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<FailedReportCountResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let count = db::count_failed_backup_reports_for_agent(&state.pool, agent.id).await?;
    Ok(Json(FailedReportCountResponse { count }))
}
