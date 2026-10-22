// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! SSH tunnels to repository hosts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `ssh_tunnels` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SshTunnel {
    /// Unique identifier.
    pub id: i64,
    /// Agent that owns this tunnel.
    pub agent_id: i64,
    /// SSH hostname for the tunnel destination.
    pub ssh_host: String,
    /// SSH user for the tunnel destination.
    pub ssh_user: String,
    /// SSH port for the tunnel destination.
    pub ssh_port: i32,
    /// Local port the tunnel listens on.
    pub tunnel_port: i32,
    /// Known host key of the destination.
    pub ssh_host_key: Option<String>,
    /// Whether the tunnel is enabled.
    pub enabled: bool,
    /// When the tunnel was created.
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating a new SSH tunnel.
#[derive(Debug, Deserialize)]
pub struct NewSshTunnel {
    /// Agent that will own this tunnel.
    pub agent_id: i64,
    /// SSH hostname for the tunnel destination.
    pub ssh_host: String,
    /// SSH user for the tunnel destination.
    pub ssh_user: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<i32>,
    /// Local port the tunnel will listen on.
    pub tunnel_port: i32,
    /// Whether the tunnel is enabled (defaults to true).
    pub enabled: Option<bool>,
    /// Known host key of the destination.
    pub ssh_host_key: Option<String>,
}

/// Parameters for updating an existing SSH tunnel (all fields optional).
#[derive(Debug, Deserialize)]
pub struct UpdateSshTunnel {
    /// New SSH hostname.
    pub ssh_host: Option<String>,
    /// New SSH user.
    pub ssh_user: Option<String>,
    /// New SSH port.
    pub ssh_port: Option<i32>,
    /// New local tunnel port.
    pub tunnel_port: Option<i32>,
    /// New enabled state.
    pub enabled: Option<bool>,
    /// New known host key.
    pub ssh_host_key: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_enabled_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE enabled = true ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_tunnel_by_id(pool: &PgPool, id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_tunnel_by_agent_id(pool: &PgPool, agent_id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE agent_id = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("ssh tunnel for agent {agent_id} not found"))
        }
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_tunnel(pool: &PgPool, params: &NewSshTunnel) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "INSERT INTO ssh_tunnels (agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, \
         ssh_host_key) VALUES ($1, $2, $3, COALESCE($4, 22), $5, COALESCE($6, true), $7) \
         RETURNING id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, \
         enabled, created_at",
        params.agent_id,
        params.ssh_host,
        params.ssh_user,
        params.ssh_port,
        params.tunnel_port,
        params.enabled,
        params.ssh_host_key,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_tunnel(
    pool: &PgPool,
    id: i64,
    params: &UpdateSshTunnel,
) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "UPDATE ssh_tunnels SET ssh_host = COALESCE($2, ssh_host), ssh_user = COALESCE($3, \
         ssh_user), ssh_port = COALESCE($4, ssh_port), tunnel_port = COALESCE($5, tunnel_port), \
         enabled = COALESCE($6, enabled), ssh_host_key = COALESCE($7, ssh_host_key) WHERE id = $1 \
         RETURNING id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, \
         enabled, created_at",
        id,
        params.ssh_host,
        params.ssh_user,
        params.ssh_port,
        params.tunnel_port,
        params.enabled,
        params.ssh_host_key,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_tunnel_ssh_host_key(
    pool: &PgPool,
    id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE ssh_tunnels SET ssh_host_key = $2 WHERE id = $1",
        id,
        ssh_host_key,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_tunnel(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM ssh_tunnels WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("ssh tunnel {id} not found")));
    }
    Ok(())
}
