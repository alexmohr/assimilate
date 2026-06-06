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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DeployAgentRequest {
    pub ssh_host: String,
    #[serde(default = "super::helpers::default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<u16>,
    pub server_url: String,
    pub install_path: Option<String>,
    #[serde(default)]
    pub use_sudo: bool,
    pub sudo_password: Option<String>,
    pub ssh_password: Option<String>,
    pub systemd_service_content: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeployAgentResponse {
    pub success: bool,
    pub skipped: bool,
    pub token: Option<String>,
    pub available_version: Option<String>,
    pub error: Option<String>,
}

fn tunnel_server_url(tunnel: &db::SshTunnel) -> Option<String> {
    tunnel
        .enabled
        .then(|| format!("ws://127.0.0.1:{}", tunnel.tunnel_port))
}

pub fn agent_binary_dir() -> PathBuf {
    if let Ok(path) = std::env::var("AGENT_BINARY_DIR") {
        return PathBuf::from(path);
    }

    let docker_path = PathBuf::from("/app");
    if std::fs::read_dir(&docker_path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().starts_with("agent-"))
        })
        .unwrap_or(false)
    {
        return docker_path;
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        return dir.to_path_buf();
    }

    docker_path
}

pub async fn query_available_agent_version(binary_dir: &std::path::Path) -> Option<String> {
    let server_arch = std::env::consts::ARCH;
    let candidates = [
        binary_dir.join(format!("agent-{server_arch}")),
        binary_dir.join("agent-x86_64"),
        binary_dir.join("agent-aarch64"),
    ];

    let binary_path = candidates.iter().find(|p| p.exists())?;
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
        .map(|v| v.to_owned())
        .or_else(|| Some(stdout.trim().to_owned()))
}

#[utoipa::path(
    post,
    path = "/api/clients/{hostname}/deploy",
    tag = "Deployment",
    operation_id = "deployAgent",
    summary = "Deploy the agent binary to a host via SSH (admin only)",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
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
pub async fn deploy_agent(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<DeployAgentRequest>,
) -> Result<Json<DeployAgentResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.server_url, "server_url")?;

    let binary_dir = agent_binary_dir();

    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;
    let tunnel_server_url = db::get_tunnel_by_client_id(&state.pool, client.id)
        .await
        .ok()
        .and_then(|tunnel| tunnel_server_url(&tunnel));
    let uses_tunnel = tunnel_server_url.is_some();
    let server_url = tunnel_server_url.unwrap_or(req.server_url);

    let available_version = query_available_agent_version(&binary_dir).await;

    if !uses_tunnel
        && let Some(ref available) = available_version
        && let Some(ref deployed) = client.agent_version
        && available == deployed
    {
        info!(
            hostname = %hostname,
            version = %available,
            "agent already at latest version, skipping deploy"
        );
        return Ok(Json(DeployAgentResponse {
            success: true,
            skipped: true,
            token: None,
            available_version: Some(available.clone()),
            error: None,
        }));
    }

    let token_hex = helpers::generate_random_hex(32);
    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    db::regenerate_client_token(&state.pool, &hostname, &token_hash).await?;

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
        use_sudo: req.use_sudo,
        sudo_password: req.sudo_password.as_deref(),
        password: req.ssh_password.as_deref(),
        systemd_service_content: req.systemd_service_content.as_deref(),
    })
    .await;

    match result {
        Ok(()) => {
            if let Some(ref version) = available_version {
                db::update_last_seen_and_version(&state.pool, client.id, version, None, None)
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_tunnel_uses_loopback_server_url() {
        let tunnel = db::SshTunnel {
            id: 1,
            client_id: 2,
            ssh_host: "agent.example.com".to_string(),
            ssh_user: "root".to_string(),
            ssh_port: 22,
            tunnel_port: 18080,
            enabled: true,
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
            client_id: 2,
            ssh_host: "agent.example.com".to_string(),
            ssh_user: "root".to_string(),
            ssh_port: 22,
            tunnel_port: 18080,
            enabled: false,
            created_at: chrono::Utc::now(),
        };

        assert_eq!(tunnel_server_url(&tunnel), None);
    }

    // Tests combined into one: both mutate AGENT_BINARY_DIR env var, causing races when parallel.
    #[test]
    fn agent_binary_dir_selection() {
        unsafe { std::env::set_var("AGENT_BINARY_DIR", "/custom/path") };
        let path = agent_binary_dir();
        assert_eq!(path, PathBuf::from("/custom/path"));

        unsafe { std::env::remove_var("AGENT_BINARY_DIR") };
        let path = agent_binary_dir();
        assert!(path.is_absolute());
    }
}
