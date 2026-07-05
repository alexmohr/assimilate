// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{responses::ServerQuotaResponse, types::QuotaAction};

use super::auth::RequireAdmin;
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::server_quota::ServerQuotaWithUsage> for ServerQuotaResponse {
    fn from(row: db::server_quota::ServerQuotaWithUsage) -> Self {
        match row.quota {
            Some(quota) => Self {
                ssh_host: row.ssh_host,
                repo_count: row.repo_count,
                total_deduplicated_size: row.total_deduplicated_size,
                configured: true,
                warn_bytes: quota.warn_bytes,
                critical_bytes: quota.critical_bytes,
                warn_action: quota.warn_action.parse().unwrap_or_default(),
                critical_action: quota.critical_action.parse().unwrap_or_default(),
                enabled: quota.enabled,
                updated_at: Some(quota.updated_at),
            },
            None => Self {
                ssh_host: row.ssh_host,
                repo_count: row.repo_count,
                total_deduplicated_size: row.total_deduplicated_size,
                configured: false,
                warn_bytes: None,
                critical_bytes: None,
                warn_action: QuotaAction::default(),
                critical_action: QuotaAction::default(),
                enabled: false,
                updated_at: None,
            },
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpsertServerQuotaRequest {
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    #[serde(default)]
    pub warn_action: QuotaAction,
    #[serde(default)]
    pub critical_action: QuotaAction,
    pub enabled: bool,
}

#[utoipa::path(
    get,
    path = "/api/server-quotas",
    tag = "Quota",
    operation_id = "listServerQuotas",
    summary = "List every SSH host hosting a repo, with usage and quota configuration",
    responses(
        (status = 200, description = "Server quotas", body = Vec<ServerQuotaResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn list_server_quotas(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<ServerQuotaResponse>>, ApiError> {
    let rows = db::server_quota::list_server_quotas_with_usage(&state.pool)
        .await
        .map_err(ApiError::Database)?;

    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    put,
    path = "/api/server-quotas/{ssh_host}",
    tag = "Quota",
    operation_id = "upsertServerQuota",
    summary = "Create or update the quota configuration for an SSH host",
    params(("ssh_host" = String, Path, description = "SSH host shared by one or more repos")),
    request_body = UpsertServerQuotaRequest,
    responses(
        (status = 200, description = "Server quota configuration", body = ServerQuotaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn upsert_server_quota(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(ssh_host): Path<String>,
    ApiJson(req): ApiJson<UpsertServerQuotaRequest>,
) -> Result<Json<ServerQuotaResponse>, ApiError> {
    db::server_quota::upsert_server_quota(
        &state.pool,
        &ssh_host,
        req.warn_bytes,
        req.critical_bytes,
        req.warn_action,
        req.critical_action,
        req.enabled,
    )
    .await
    .map_err(ApiError::Database)?;

    let total_deduplicated_size =
        db::server_quota::total_deduplicated_size_for_ssh_host(&state.pool, &ssh_host)
            .await
            .map_err(ApiError::Database)?;
    let repo_count = db::server_quota::repo_count_for_ssh_host(&state.pool, &ssh_host)
        .await
        .map_err(ApiError::Database)?;
    let quota = db::server_quota::get_server_quota(&state.pool, &ssh_host)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("server quota for {ssh_host} not found")))?;

    let response: ServerQuotaResponse = db::server_quota::ServerQuotaWithUsage {
        ssh_host,
        repo_count,
        total_deduplicated_size,
        quota: Some(quota),
    }
    .into();

    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/server-quotas/{ssh_host}",
    tag = "Quota",
    operation_id = "deleteServerQuota",
    summary = "Remove the quota configuration for an SSH host",
    params(("ssh_host" = String, Path, description = "SSH host shared by one or more repos")),
    responses(
        (status = 204, description = "Server quota deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Quota not configured"),
    )
)]
pub async fn delete_server_quota(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(ssh_host): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = db::server_quota::delete_server_quota(&state.pool, &ssh_host)
        .await
        .map_err(ApiError::Database)?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!(
            "server quota for {ssh_host} not found"
        )))
    }
}
