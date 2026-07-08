// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{auth::RequireAdmin, helpers};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
    ssh,
};

/// Request payload for deploying the agent binary via SSH.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DeployAgentRequest {
    /// SSH hostname or IP address.
    pub ssh_host: String,
    /// SSH user (defaults to "borg").
    #[serde(default = "super::helpers::default_ssh_user")]
    pub ssh_user: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<u16>,
    /// Server URL the agent should connect to (e.g. `<ws://host:8080>`).
    pub server_url: String,
    /// Remote install path for the binary.
    pub install_path: Option<String>,
    /// Optional SSH password (key-based auth preferred).
    pub ssh_password: Option<String>,
    /// Optional custom systemd service unit content.
    pub systemd_service_content: Option<String>,
}

/// Result of an agent deployment attempt.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeployAgentResponse {
    /// Whether the deployment succeeded.
    pub success: bool,
    /// Whether the agent was already at the latest version and skipped.
    pub skipped: bool,
    /// The generated agent token (shown once).
    pub token: Option<String>,
    /// Latest available agent version.
    pub available_version: Option<String>,
    /// Error message if the deployment failed.
    pub error: Option<String>,
}

fn tunnel_server_url(tunnel: &db::SshTunnel) -> Option<String> {
    tunnel
        .enabled
        .then(|| format!("ws://127.0.0.1:{}", tunnel.tunnel_port))
}

/// Resolve the directory containing agent binaries.
pub async fn agent_binary_dir() -> PathBuf {
    if let Ok(path) = std::env::var("AGENT_BINARY_DIR") {
        return PathBuf::from(path);
    }

    let docker_path = PathBuf::from("/app");
    if !ssh::list_agent_binaries(&docker_path).await.is_empty() {
        return docker_path;
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        return dir.to_path_buf();
    }

    docker_path
}

/// Query the available agent version from the binary directory.
pub async fn query_available_agent_version(binary_dir: &std::path::Path) -> Option<String> {
    let server_arch = std::env::consts::ARCH;
    let candidates = [
        binary_dir.join(format!("agent-{server_arch}")),
        binary_dir.join("agent-x86_64"),
        binary_dir.join("agent-aarch64"),
    ];

    let mut binary_path = None;
    for candidate in &candidates {
        if tokio::fs::try_exists(candidate).await.unwrap_or(false) {
            binary_path = Some(candidate);
            break;
        }
    }
    let binary_path = binary_path?;
    let output = tokio::process::Command::new(binary_path)
        .arg("--version")
        .output()
        .await
        .inspect_err(|e| {
            tracing::debug!(path = ?binary_path, error = %e, "failed to query agent version");
        })
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .strip_prefix("agent ")
        .map(str::to_owned)
        .or_else(|| Some(stdout.trim().to_owned()))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/deploy",
    tag = "Deployment",
    operation_id = "deployAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    request_body = DeployAgentRequest,
    responses(
        (status = 200, description = "Deploy result", body = DeployAgentResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Agent binary not found or internal error"),
    )
)]
/// Deploy the agent binary to a host via SSH (admin only).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn deploy_agent(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<DeployAgentRequest>,
) -> Result<Json<DeployAgentResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.server_url, "server_url")?;

    let binary_dir = agent_binary_dir().await;

    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let tunnel_server_url = db::get_tunnel_by_agent_id(&state.pool, agent.id)
        .await
        .ok()
        .and_then(|tunnel| tunnel_server_url(&tunnel));
    let uses_tunnel = tunnel_server_url.is_some();
    let server_url = tunnel_server_url.unwrap_or(req.server_url);

    let available_version = query_available_agent_version(&binary_dir).await;
    let server_commit_count = option_env!("GIT_COMMIT_COUNT")
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|&n| n > 0);

    let already_current = agent_is_current(
        server_commit_count,
        agent.agent_commit_count,
        available_version.as_deref(),
        agent.agent_version.as_deref(),
    );

    if !uses_tunnel && already_current {
        let version = available_version
            .clone()
            .or_else(|| agent.agent_version.clone());
        info!(
            hostname = %hostname,
            "agent already at latest version, skipping deploy"
        );
        return Ok(Json(DeployAgentResponse {
            success: true,
            skipped: true,
            token: None,
            available_version: version,
            error: None,
        }));
    }

    let token_hex = helpers::generate_random_hex(32);
    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    db::regenerate_agent_token(&state.pool, &hostname, &token_hash).await?;

    let install_path = req
        .install_path
        .as_deref()
        .unwrap_or("/usr/local/bin/assimilate-agent");

    let port = req.ssh_port.unwrap_or(22);

    let result = ssh::deploy_agent(&ssh::DeployAgentParams {
        host: &req.ssh_host,
        user: &req.ssh_user,
        port,
        binary_dir: &binary_dir,
        remote_path: install_path,
        server_url: &server_url,
        token: &token_hex,
        password: req.ssh_password.as_deref(),
        systemd_service_content: req.systemd_service_content.as_deref(),
    })
    .await;

    match result {
        Ok(()) => {
            if let Some(ref version) = available_version {
                db::update_last_seen_and_version(
                    &state.pool,
                    agent.id,
                    version,
                    None,
                    None,
                    server_commit_count,
                )
                .await?;
            }
            info!(hostname = %hostname, ssh_host = %req.ssh_host, "agent deployed successfully");
            Ok(Json(DeployAgentResponse {
                success: true,
                skipped: false,
                token: Some(token_hex),
                available_version,
                error: None,
            }))
        }
        Err(e) => Ok(Json(DeployAgentResponse {
            success: false,
            skipped: false,
            token: Some(token_hex),
            available_version,
            error: Some(e.to_string()),
        })),
    }
}
/// Request payload for fetching a systemd service unit from a remote host.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FetchServiceUnitRequest {
    /// SSH hostname or IP address.
    pub ssh_host: String,
    /// SSH user (defaults to "borg").
    #[serde(default = "super::helpers::default_ssh_user")]
    pub ssh_user: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<u16>,
    /// Optional SSH password.
    pub ssh_password: Option<String>,
}

/// Content of a remote systemd service unit file.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FetchServiceUnitResponse {
    /// Service unit file content, or None if not found.
    pub content: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/service-unit",
    tag = "Deployment",
    operation_id = "fetchServiceUnit",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    request_body = FetchServiceUnitRequest,
    responses(
        (status = 200, description = "Service unit content or null if not present", body =
            FetchServiceUnitResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Read the existing systemd service unit from a remote host via SSH (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadGateway`] if the upstream operation (e.g. SSH or borg) fails.
pub async fn fetch_service_unit(
    _admin: RequireAdmin,
    Path(_hostname): Path<String>,
    ApiJson(req): ApiJson<FetchServiceUnitRequest>,
) -> Result<Json<FetchServiceUnitResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;

    let port = req.ssh_port.unwrap_or(22);
    let content = ssh::read_remote_file(&ssh::ReadFileParams {
        host: &req.ssh_host,
        user: &req.ssh_user,
        port,
        password: req.ssh_password.as_deref(),
        path: "/etc/systemd/system/assimilate-agent.service",
    })
    .await
    .map_err(|e| ApiError::BadGateway(e.to_string()))?;

    Ok(Json(FetchServiceUnitResponse { content }))
}

fn agent_is_current(
    server_commit_count: Option<i32>,
    agent_commit_count: Option<i32>,
    available_version: Option<&str>,
    agent_version: Option<&str>,
) -> bool {
    if let (Some(server_count), Some(agent_count)) = (server_commit_count, agent_commit_count) {
        agent_count >= server_count
    } else {
        available_version
            .zip(agent_version)
            .is_some_and(|(av, dv)| av == dv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_tunnel_uses_loopback_server_url() {
        let tunnel = db::SshTunnel {
            id: 1,
            agent_id: 2,
            ssh_host: "agent.example.com".to_string(),
            ssh_user: "root".to_string(),
            ssh_port: 22,
            tunnel_port: 18080,
            enabled: true,
            ssh_host_key: None,
            created_at: chrono::Utc::now(),
        };

        assert_eq!(
            tunnel_server_url(&tunnel),
            Some("ws://127.0.0.1:18080".to_string())
        );
    }

    #[test]
    fn disabled_tunnel_does_not_override_server_url() {
        let tunnel = db::SshTunnel {
            id: 1,
            agent_id: 2,
            ssh_host: "agent.example.com".to_string(),
            ssh_user: "root".to_string(),
            ssh_port: 22,
            tunnel_port: 18080,
            enabled: false,
            ssh_host_key: None,
            created_at: chrono::Utc::now(),
        };

        assert_eq!(tunnel_server_url(&tunnel), None);
    }

    // Tests combined into one: both mutate AGENT_BINARY_DIR env var, causing races when parallel.
    #[tokio::test]
    async fn agent_binary_dir_selection() {
        unsafe { std::env::set_var("AGENT_BINARY_DIR", "/custom/path") };
        assert_eq!(agent_binary_dir().await, PathBuf::from("/custom/path"));

        unsafe { std::env::remove_var("AGENT_BINARY_DIR") };
        assert!(agent_binary_dir().await.is_absolute());
    }

    #[test]
    fn agent_is_current_uses_commit_count_when_both_present() {
        assert!(agent_is_current(Some(10), Some(10), None, None));
        assert!(agent_is_current(Some(10), Some(11), None, None));
        assert!(!agent_is_current(Some(11), Some(10), None, None));
    }

    #[test]
    fn agent_is_current_falls_back_to_version_string() {
        assert!(agent_is_current(None, None, Some("1.2.3"), Some("1.2.3")));
        assert!(!agent_is_current(None, None, Some("1.2.4"), Some("1.2.3")));
        assert!(!agent_is_current(None, None, None, Some("1.2.3")));
        // falls back to version when only one count is available
        assert!(agent_is_current(
            Some(5),
            None,
            Some("1.2.3"),
            Some("1.2.3")
        ));
        assert!(!agent_is_current(
            Some(5),
            None,
            Some("1.2.4"),
            Some("1.2.3")
        ));
    }
}
