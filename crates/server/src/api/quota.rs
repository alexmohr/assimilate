// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use shared::responses::RepoQuotaResponse;

use super::auth::{AuthUser, Role};
use crate::{
    AppState, db,
    db::quota::{QuotaAction, RepoQuota, ServerQuota},
    error::{ApiError, ApiJson},
};

impl From<db::quota::RepoQuota> for RepoQuotaResponse {
    fn from(q: db::quota::RepoQuota) -> Self {
        Self {
            repo_id: q.repo_id,
            warn_bytes: q.warn_bytes,
            critical_bytes: q.critical_bytes,
            enabled: q.enabled,
            updated_at: q.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpsertQuotaRequest {
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub warn_action: Option<QuotaAction>,
    pub critical_action: Option<QuotaAction>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpsertServerQuotaRequest {
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub warn_action: Option<QuotaAction>,
    pub critical_action: Option<QuotaAction>,
    pub enabled: bool,
}

async fn require_operator_or_admin(state: &AppState, auth: &AuthUser) -> Result<(), ApiError> {
    if auth.role == Role::Admin {
        return Ok(());
    }

    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if effective.can_view_all_repos {
        return Ok(());
    }

    Err(ApiError::Forbidden("operator access required".to_owned()))
}

#[utoipa::path(
    get,
    path = "/api/repos/{id}/quota",
    tag = "Quota",
    operation_id = "getRepoQuota",
    summary = "Get a repository quota configuration",
    params(("id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Quota configuration", body = RepoQuotaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Quota not configured"),
    )
)]
pub async fn get_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoQuotaResponse>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota: RepoQuotaResponse = db::quota::get_quota(&state.pool, repo_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("quota for repo {repo_id} not found")))?
        .into();

    Ok(Json(quota))
}

#[utoipa::path(
    put,
    path = "/api/repos/{id}/quota",
    tag = "Quota",
    operation_id = "upsertRepoQuota",
    summary = "Create or update a repository quota configuration",
    params(("id" = i64, Path, description = "Repository ID")),
    request_body = UpsertQuotaRequest,
    responses(
        (status = 200, description = "Quota configuration", body = RepoQuotaResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn upsert_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpsertQuotaRequest>,
) -> Result<Json<RepoQuotaResponse>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota: RepoQuotaResponse = db::quota::upsert_quota(
        &state.pool,
        repo_id,
        req.warn_bytes,
        req.critical_bytes,
        req.warn_action.unwrap_or(QuotaAction::NotifyOnly),
        req.critical_action.unwrap_or(QuotaAction::NotifyOnly),
        req.enabled,
    )
    .await?
    .into();

    Ok(Json(quota))
}

#[utoipa::path(
    get,
    path = "/api/server-quotas",
    tag = "Quota",
    operation_id = "listServerQuotas",
    summary = "List all server quota configurations",
    responses(
        (status = 200, description = "List of server quotas", body = Vec<ServerQuota>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn list_server_quotas(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ServerQuota>>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quotas = db::quota::list_server_quotas(&state.pool).await?;
    Ok(Json(quotas))
}

#[utoipa::path(
    get,
    path = "/api/server-quotas/{ssh_host}",
    tag = "Quota",
    operation_id = "getServerQuota",
    summary = "Get a server quota configuration",
    params(("ssh_host" = String, Path, description = "SSH host")),
    responses(
        (status = 200, description = "Server quota configuration", body = ServerQuota),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Quota not configured"),
    )
)]
pub async fn get_server_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ssh_host): Path<String>,
) -> Result<Json<ServerQuota>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota = db::quota::get_server_quota(&state.pool, &ssh_host)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("quota for server {ssh_host} not found")))?;

    Ok(Json(quota))
}

#[utoipa::path(
    put,
    path = "/api/server-quotas/{ssh_host}",
    tag = "Quota",
    operation_id = "upsertServerQuota",
    summary = "Create or update a server quota configuration",
    params(("ssh_host" = String, Path, description = "SSH host")),
    request_body = UpsertServerQuotaRequest,
    responses(
        (status = 200, description = "Server quota configuration", body = ServerQuota),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn upsert_server_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ssh_host): Path<String>,
    ApiJson(req): ApiJson<UpsertServerQuotaRequest>,
) -> Result<Json<ServerQuota>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let quota = db::quota::upsert_server_quota(
        &state.pool,
        &ssh_host,
        req.warn_bytes,
        req.critical_bytes,
        req.warn_action.unwrap_or(QuotaAction::NotifyOnly),
        req.critical_action.unwrap_or(QuotaAction::NotifyOnly),
        req.enabled,
    )
    .await?;

    Ok(Json(quota))
}

#[utoipa::path(
    delete,
    path = "/api/server-quotas/{ssh_host}",
    tag = "Quota",
    operation_id = "deleteServerQuota",
    summary = "Delete a server quota configuration",
    params(("ssh_host" = String, Path, description = "SSH host")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_server_quota(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ssh_host): Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let deleted = db::quota::delete_server_quota(&state.pool, &ssh_host).await?;
    if deleted {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!(
            "quota for server {ssh_host} not found"
        )))
    }
}

#[utoipa::path(
    get,
    path = "/api/server-quotas/hosts",
    tag = "Quota",
    operation_id = "listSshHosts",
    summary = "List distinct SSH hosts from configured repositories",
    responses(
        (status = 200, description = "List of SSH hosts", body = Vec<String>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn list_ssh_hosts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<String>>, ApiError> {
    require_operator_or_admin(&state, &auth).await?;

    let hosts = db::quota::list_ssh_hosts(&state.pool).await?;
    Ok(Json(hosts))
}
