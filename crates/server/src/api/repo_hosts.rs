// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repository hosts: the machines borg writes to.
//!
//! Everything that is a fact about the machine rather than about one
//! repository on it is set here, once, for every repository on it: where it
//! is reached (hostname and port), the SSH host key it presents, and its
//! power settings. Its "when the host is offline" settings live beside these
//! in [`super::availability`].
//!
//! Admin only throughout: a host's list of repositories names every
//! repository on it regardless of who may see which, and its wake settings
//! let whoever has them power the machine on.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::responses::{
    HostWakeSettingsResponse, RepoHostKeyResponse, RepoHostResponse, RepoOnHostResponse,
};
use tracing::info;

use super::{agents::UpdateHostWakeRequest, auth::RequireAdmin, helpers};
use crate::{
    AppState, config_assembler,
    db::{
        self,
        repo_hosts::{RepoHostRow, RepoOnHostRow},
    },
    error::{ApiError, ApiJson},
};

impl From<RepoOnHostRow> for RepoOnHostResponse {
    fn from(row: RepoOnHostRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            ssh_user: row.ssh_user,
            repo_path: row.repo_path,
            enabled: row.enabled,
        }
    }
}

fn host_response(host: RepoHostRow, repositories: Vec<RepoOnHostRow>) -> RepoHostResponse {
    RepoHostResponse {
        id: host.id,
        ssh_host: host.ssh_host,
        ssh_port: host.ssh_port,
        ssh_host_key: host.ssh_host_key,
        intermittent: host.intermittent,
        power: HostWakeSettingsResponse {
            wake_enabled: host.wake_enabled,
            wake_mac_address: host.wake_mac_address,
            wake_broadcast_address: host.wake_broadcast_address,
            wake_timeout_seconds: host.wake_timeout_seconds,
            shutdown_after_backup: host.shutdown_after_backup,
        },
        repositories: repositories
            .into_iter()
            .map(RepoOnHostResponse::from)
            .collect(),
    }
}

async fn load_host_response(
    state: &AppState,
    host: RepoHostRow,
) -> Result<RepoHostResponse, ApiError> {
    let repositories = db::repo_hosts::list_repos_on_host(&state.pool, host.id).await?;
    Ok(host_response(host, repositories))
}

/// Re-sends config to every agent that writes to a repository on this host,
/// so a new address or key reaches them without waiting for a reconnect.
async fn push_config_for_host(state: &AppState, repo_host_id: i64) -> Result<(), ApiError> {
    let targets =
        db::repo_hosts::list_target_agents_for_repo_host(&state.pool, repo_host_id).await?;
    for target in &targets {
        config_assembler::push_config_to_agent(state, target.agent_id).await;
    }
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(())
}

fn validate_port(ssh_port: i32) -> Result<u16, ApiError> {
    u16::try_from(ssh_port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| ApiError::BadRequest("ssh_port must be between 1 and 65535".into()))
}

#[utoipa::path(
    get,
    path = "/api/repo-hosts",
    tag = "Repository hosts",
    operation_id = "listRepoHosts",
    responses(
        (status = 200, description = "Repository hosts", body = Vec<RepoHostResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// List every repository host with its repositories (admin only).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn list_repo_hosts(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<RepoHostResponse>>, ApiError> {
    let hosts = db::repo_hosts::list_repo_hosts(&state.pool).await?;
    let mut repos_by_host = db::repo_hosts::list_repos_by_host(&state.pool).await?;
    let responses = hosts
        .into_iter()
        .map(|summary| {
            let repositories = repos_by_host.remove(&summary.host.id).unwrap_or_default();
            host_response(summary.host, repositories)
        })
        .collect();
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/repo-hosts/{repo_host_id}",
    tag = "Repository hosts",
    operation_id = "getRepoHost",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    responses(
        (status = 200, description = "Repository host", body = RepoHostResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// One repository host with its repositories (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the host does not exist.
pub async fn get_repo_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
) -> Result<Json<RepoHostResponse>, ApiError> {
    let host = db::repo_hosts::get_repo_host(&state.pool, repo_host_id).await?;
    Ok(Json(load_host_response(&state, host).await?))
}

/// Request payload for changing where a repository host is reached.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRepoHostRequest {
    /// Hostname or IP borg connects to. Unique across hosts.
    pub ssh_host: String,
    /// SSH port (1-65535).
    pub ssh_port: i32,
}

#[utoipa::path(
    put,
    path = "/api/repo-hosts/{repo_host_id}",
    tag = "Repository hosts",
    operation_id = "updateRepoHost",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    request_body = UpdateRepoHostRequest,
    responses(
        (status = 200, description = "Updated repository host", body = RepoHostResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Another host already has this hostname"),
    )
)]
/// Change where a repository host is reached (admin only).
///
/// Applies to every repository on the host: each is marked as relocated, so
/// its agents confirm the move before writing to it again. The pinned host key
/// is kept - a renamed machine presents the same key, and one that does not is
/// refused rather than trusted.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an empty hostname or an invalid port,
/// [`ApiError::Conflict`] if another host already has the hostname, or
/// [`ApiError::NotFound`] if the host does not exist.
pub async fn update_repo_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRepoHostRequest>,
) -> Result<Json<RepoHostResponse>, ApiError> {
    let ssh_host = req.ssh_host.trim();
    helpers::validate_non_empty(ssh_host, "ssh_host")?;
    validate_port(req.ssh_port)?;

    let host =
        db::repo_hosts::update_repo_host_address(&state.pool, repo_host_id, ssh_host, req.ssh_port)
            .await?;
    info!(
        repo_host_id,
        ssh_host = %host.ssh_host,
        ssh_port = host.ssh_port,
        "repository host address updated"
    );
    push_config_for_host(&state, repo_host_id).await?;
    Ok(Json(load_host_response(&state, host).await?))
}

#[utoipa::path(
    delete,
    path = "/api/repo-hosts/{repo_host_id}",
    tag = "Repository hosts",
    operation_id = "deleteRepoHost",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Repositories still use this host"),
    )
)]
/// Remove a repository host no repository uses any more (admin only).
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if a repository still uses the host, or
/// [`ApiError::NotFound`] if it does not exist.
pub async fn delete_repo_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::repo_hosts::delete_repo_host(&state.pool, repo_host_id).await?;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/repo-hosts/{repo_host_id}/power",
    tag = "Repository hosts",
    operation_id = "updateRepoHostPower",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    request_body = UpdateHostWakeRequest,
    responses(
        (status = 200, description = "Updated repository host", body = RepoHostResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Update a repository host's power-management settings (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for invalid wake settings, or
/// [`ApiError::NotFound`] if the host does not exist.
pub async fn update_repo_host_power(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateHostWakeRequest>,
) -> Result<Json<RepoHostResponse>, ApiError> {
    crate::api::agents::validate_host_wake(&req)?;
    let host = db::repo_hosts::update_repo_host_power(
        &state.pool,
        repo_host_id,
        db::RepoPowerPatch {
            wake_enabled: req.wake_enabled,
            wake_mac_address: req.wake_mac_address.as_deref(),
            wake_broadcast_address: req.wake_broadcast_address.as_deref(),
            wake_timeout_seconds: req.wake_timeout_seconds,
            shutdown_after_backup: req.shutdown_after_backup,
        },
    )
    .await?;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(Json(load_host_response(&state, host).await?))
}

/// Request payload for accepting a scanned SSH host key.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AcceptRepoHostKeyRequest {
    /// The SSH host key string (e.g. "ssh-ed25519 AAAA...").
    pub ssh_host_key: String,
}

#[utoipa::path(
    post,
    path = "/api/repo-hosts/{repo_host_id}/ssh-host-key/scan",
    tag = "Repository hosts",
    operation_id = "scanRepoHostKey",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    responses(
        (status = 200, description = "Scanned SSH host key", body = RepoHostKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "SSH host key scan failed"),
    )
)]
/// Scan the key a repository host presents now, without pinning it (admin
/// only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the host does not exist, or
/// [`ApiError::BadGateway`] if the scan fails.
pub async fn scan_repo_host_key(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
) -> Result<Json<RepoHostKeyResponse>, ApiError> {
    let host = db::repo_hosts::get_repo_host(&state.pool, repo_host_id).await?;
    let ssh_host_key = crate::ssh::scan_host_key(&host.ssh_host, validate_port(host.ssh_port)?)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;
    Ok(Json(RepoHostKeyResponse { ssh_host_key }))
}

#[utoipa::path(
    post,
    path = "/api/repo-hosts/{repo_host_id}/ssh-host-key",
    tag = "Repository hosts",
    operation_id = "acceptRepoHostKey",
    params(("repo_host_id" = i64, Path, description = "Repository host ID")),
    request_body = AcceptRepoHostKeyRequest,
    responses(
        (status = 200, description = "SSH host key accepted", body = RepoHostKeyResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Pin a scanned SSH host key for every repository on the host, and push the
/// new key to the agents that write to them (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an empty key, or
/// [`ApiError::NotFound`] if the host does not exist.
pub async fn accept_repo_host_key(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_host_id): Path<i64>,
    ApiJson(req): ApiJson<AcceptRepoHostKeyRequest>,
) -> Result<Json<RepoHostKeyResponse>, ApiError> {
    let ssh_host_key = req.ssh_host_key.trim();
    helpers::validate_non_empty(ssh_host_key, "ssh_host_key")?;
    db::repo_hosts::update_repo_host_key(&state.pool, repo_host_id, ssh_host_key).await?;
    push_config_for_host(&state, repo_host_id).await?;
    info!(repo_host_id, "repository host SSH host key accepted");
    Ok(Json(RepoHostKeyResponse {
        ssh_host_key: ssh_host_key.to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_must_fit_the_tcp_range() {
        assert!(validate_port(0).is_err());
        assert!(validate_port(-1).is_err());
        assert!(validate_port(65_536).is_err());
        assert_eq!(validate_port(22).unwrap(), 22);
        assert_eq!(validate_port(65_535).unwrap(), 65_535);
    }
}
