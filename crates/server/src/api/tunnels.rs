// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use shared::protocol::TunnelStatus;

use super::auth::RequireAdmin;
use crate::{
    AppState,
    db::{self, NewSshTunnel, SshTunnel, UpdateSshTunnel},
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize)]
pub struct CreateTunnelRequest {
    pub client_id: i64,
    pub ssh_host: String,
    #[serde(default = "super::helpers::default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<i32>,
    pub tunnel_port: i32,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTunnelRequest {
    pub ssh_host: Option<String>,
    pub ssh_user: Option<String>,
    pub ssh_port: Option<i32>,
    pub tunnel_port: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TunnelResponse {
    #[serde(flatten)]
    pub tunnel: SshTunnel,
    pub status: TunnelStatus,
}

fn validate_port(port: i32, field: &str) -> Result<(), ApiError> {
    if port <= 0 || port > 65535 {
        return Err(ApiError::BadRequest(format!(
            "{field} must be between 1 and 65535"
        )));
    }
    Ok(())
}

fn validate_non_empty(value: &str, field: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{field} must not be empty")));
    }
    Ok(())
}

pub async fn list_tunnels(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<TunnelResponse>>, ApiError> {
    let tunnels = db::list_all_tunnels(&state.pool).await?;
    let statuses = state.tunnel_manager.all_statuses().await;
    let responses = tunnels
        .into_iter()
        .map(|tunnel| {
            let status = statuses
                .iter()
                .find(|(id, _)| *id == tunnel.id)
                .map_or(TunnelStatus::Disconnected, |(_, s)| s.clone());
            TunnelResponse { tunnel, status }
        })
        .collect();
    Ok(Json(responses))
}

pub async fn get_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<TunnelResponse>, ApiError> {
    let tunnel = db::get_tunnel_by_id(&state.pool, id).await?;
    let status = state
        .tunnel_manager
        .tunnel_status(id)
        .await
        .unwrap_or(TunnelStatus::Disconnected);
    Ok(Json(TunnelResponse { tunnel, status }))
}

pub async fn create_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    ApiJson(req): ApiJson<CreateTunnelRequest>,
) -> Result<(StatusCode, Json<SshTunnel>), ApiError> {
    validate_non_empty(&req.ssh_host, "ssh_host")?;
    let ssh_port = req.ssh_port.unwrap_or(22);
    validate_port(ssh_port, "ssh_port")?;
    validate_port(req.tunnel_port, "tunnel_port")?;

    let tunnel = db::insert_tunnel(
        &state.pool,
        &NewSshTunnel {
            client_id: req.client_id,
            ssh_host: req.ssh_host,
            ssh_user: req.ssh_user,
            ssh_port: req.ssh_port,
            tunnel_port: req.tunnel_port,
            enabled: req.enabled,
        },
    )
    .await?;

    if tunnel.enabled {
        state.tunnel_manager.start_tunnel(tunnel.id).await;
    }

    Ok((StatusCode::CREATED, Json(tunnel)))
}

pub async fn update_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateTunnelRequest>,
) -> Result<Json<SshTunnel>, ApiError> {
    if let Some(ref host) = req.ssh_host {
        validate_non_empty(host, "ssh_host")?;
    }
    if let Some(port) = req.ssh_port {
        validate_port(port, "ssh_port")?;
    }
    if let Some(port) = req.tunnel_port {
        validate_port(port, "tunnel_port")?;
    }

    let tunnel = db::update_tunnel(
        &state.pool,
        id,
        &UpdateSshTunnel {
            ssh_host: req.ssh_host,
            ssh_user: req.ssh_user,
            ssh_port: req.ssh_port,
            tunnel_port: req.tunnel_port,
            enabled: req.enabled,
        },
    )
    .await?;

    state.tunnel_manager.stop_tunnel(id).await;
    if tunnel.enabled {
        state.tunnel_manager.start_tunnel(id).await;
    }

    Ok(Json(tunnel))
}

pub async fn delete_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.tunnel_manager.stop_tunnel(id).await;
    db::delete_tunnel(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn enable_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<SshTunnel>, ApiError> {
    let tunnel = db::update_tunnel(
        &state.pool,
        id,
        &UpdateSshTunnel {
            ssh_host: None,
            ssh_user: None,
            ssh_port: None,
            tunnel_port: None,
            enabled: Some(true),
        },
    )
    .await?;
    state.tunnel_manager.start_tunnel(id).await;
    Ok(Json(tunnel))
}

pub async fn disable_tunnel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<SshTunnel>, ApiError> {
    state.tunnel_manager.stop_tunnel(id).await;
    let tunnel = db::update_tunnel(
        &state.pool,
        id,
        &UpdateSshTunnel {
            ssh_host: None,
            ssh_user: None,
            ssh_port: None,
            tunnel_port: None,
            enabled: Some(false),
        },
    )
    .await?;
    Ok(Json(tunnel))
}
