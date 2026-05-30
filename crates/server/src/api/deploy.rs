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

pub fn agent_binary_path() -> PathBuf {
    if let Ok(path) = std::env::var("AGENT_BINARY_PATH") {
        return PathBuf::from(path);
    }

    let docker_path = PathBuf::from("/app/agent");
    if docker_path.exists() {
        return docker_path;
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("agent");
        if sibling.exists() {
            return sibling;
        }
    }

    docker_path
}

pub async fn query_available_agent_version(binary_path: &std::path::Path) -> Option<String> {
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

    let binary_path = agent_binary_path();
    if !binary_path.exists() {
        return Err(ApiError::Internal(format!(
            "agent binary not found at {}. Set AGENT_BINARY_PATH to the correct location.",
            binary_path.display()
        )));
    }

    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;

    let available_version = query_available_agent_version(&binary_path).await;

    if let Some(ref available) = available_version
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
        local_binary: &binary_path,
        remote_path: install_path,
        server_url: &req.server_url,
        token: &token_hex,
        use_sudo: req.use_sudo,
        sudo_password: req.sudo_password.as_deref(),
        password: req.ssh_password.as_deref(),
        systemd_service_content: req.systemd_service_content.as_deref(),
    })
    .await;

    match result {
        Ok(()) => {
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

    // Tests combined into one: both mutate AGENT_BINARY_PATH env var, causing races when parallel.
    #[test]
    fn agent_binary_path_selection() {
        unsafe { std::env::set_var("AGENT_BINARY_PATH", "/custom/path/agent") };
        let path = agent_binary_path();
        assert_eq!(path, PathBuf::from("/custom/path/agent"));

        unsafe { std::env::remove_var("AGENT_BINARY_PATH") };
        let path = agent_binary_path();
        assert_eq!(path.file_name().unwrap(), "agent");
    }
}
