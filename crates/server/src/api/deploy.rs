// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{path::PathBuf, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use shared::{
    responses::{
        DeployJobState, DeployStatusResponse, DeployStep, DeployStepResponse, DeployStepStatus,
    },
    types::DeploymentMode,
};
use strum::IntoEnumIterator;
use tracing::{info, warn};

use super::{
    auth::AuthUser,
    helpers::{self, DomainQuery, MaxLen},
};
use crate::{
    AgentRegistry, AppState, db,
    deploy_jobs::{DeployJobs, Outcome},
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
    /// Redeploy even if the agent already reports the latest version --
    /// e.g. after the host was reimaged or the installation was otherwise lost.
    #[serde(default)]
    pub force: bool,
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
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = DeployAgentRequest,
    responses(
        (status = 202, description = "The deploy was started and keeps running on the server; \
            poll its status", body = DeployStatusResponse),
        (status = 200, description = "Nothing to do: the agent already runs the latest version",
            body = DeployStatusResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- upgrade agent permission required, or the \
            server runs in desktop mode"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain, or a deploy of \
            this agent is already running"),
        (status = 500, description = "Agent binary not found or internal error"),
    )
)]
/// Start deploying the agent binary to a host via SSH (requires `can_upgrade_agent`
/// permission).
///
/// The deploy runs on the server, not for the length of this request: read its progress
/// from `GET /api/agents/{hostname}/deploy`.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if a deploy of this agent is already running.
pub async fn deploy_agent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<DeployAgentRequest>,
) -> Result<(StatusCode, Json<DeployStatusResponse>), ApiError> {
    ensure_agent_deploy_allowed(state.deployment_mode)?;
    require_upgrade_agent(&state.pool, auth.user_id).await?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;
    helpers::validate_non_empty(&req.server_url, "server_url")?;
    validate_deploy_lengths(&req)?;

    let binary_dir = agent_binary_dir().await;

    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
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

    if should_skip_deploy(uses_tunnel, already_current, req.force) {
        let version = available_version
            .clone()
            .or_else(|| agent.agent_version.clone());
        info!(
            hostname = %hostname,
            "agent already at latest version, skipping deploy"
        );
        return Ok((StatusCode::OK, Json(skipped_status(version))));
    }

    let server_key = state
        .channel_key
        .public_key()
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .to_string();

    state
        .deploy_jobs
        .start(agent.id, auth.user_id)
        .map_err(|e| ApiError::Conflict(e.to_string()))?;

    let token = match issue_agent_token(&state.pool, agent.id).await {
        Ok(token) => token,
        Err(e) => {
            state.deploy_jobs.remove(agent.id);
            return Err(e);
        }
    };

    let run = DeployRun {
        pool: state.pool.clone(),
        registry: state.registry.clone(),
        jobs: state.deploy_jobs.clone(),
        agent_id: agent.id,
        hostname,
        ssh_host: req.ssh_host,
        ssh_user: req.ssh_user,
        ssh_port: req.ssh_port.unwrap_or(22),
        ssh_password: req.ssh_password,
        binary_dir,
        install_path: req
            .install_path
            .unwrap_or_else(|| DEFAULT_INSTALL_PATH.to_owned()),
        server_url,
        token,
        server_key,
        systemd_service_content: req.systemd_service_content,
        available_version,
        server_commit_count,
    };
    state.task_registry.register(tokio::spawn(run_deploy(run)));

    let status = state
        .deploy_jobs
        .status(agent.id, auth.user_id)
        .ok_or_else(|| ApiError::Internal("the deploy was not recorded".to_owned()))?;
    Ok((StatusCode::ACCEPTED, Json(status)))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/deploy",
    tag = "Deployment",
    operation_id = "getAgentDeployStatus",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "The agent's current deploy, or the last one", body =
            DeployStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- upgrade agent permission required, or the \
            server runs in desktop mode"),
        (status = 404, description = "No deploy of this agent is running or recent"),
    )
)]
/// Read how the agent's deploy is going (requires `can_upgrade_agent` permission).
///
/// The new agent token is included once the deploy has finished, for the user who
/// started it only.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the agent has no running or recent deploy.
pub async fn deploy_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<DeployStatusResponse>, ApiError> {
    ensure_agent_deploy_allowed(state.deployment_mode)?;
    require_upgrade_agent(&state.pool, auth.user_id).await?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    state
        .deploy_jobs
        .status(agent.id, auth.user_id)
        .map(Json)
        .ok_or_else(|| {
            ApiError::NotFound("no deploy of this agent is running or recent".to_owned())
        })
}

#[utoipa::path(
    delete,
    path = "/api/agents/{hostname}/deploy",
    tag = "Deployment",
    operation_id = "dismissAgentDeploy",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 204, description = "The result was dismissed, or there was none"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- upgrade agent permission required, or the \
            server runs in desktop mode"),
    )
)]
/// Dismiss the result of the agent's last deploy, dropping the token it holds (requires
/// `can_upgrade_agent` permission). Only the user who started the deploy can.
///
/// # Errors
///
/// Returns an error if the agent cannot be resolved.
pub async fn dismiss_deploy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<StatusCode, ApiError> {
    ensure_agent_deploy_allowed(state.deployment_mode)?;
    require_upgrade_agent(&state.pool, auth.user_id).await?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    state.deploy_jobs.dismiss(agent.id, auth.user_id);
    Ok(StatusCode::NO_CONTENT)
}

/// Where the agent binary is installed unless the request says otherwise.
const DEFAULT_INSTALL_PATH: &str = "/usr/local/bin/assimilate-agent";

/// How long a restarted agent has to connect back before the deploy fails.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(90);
/// How often the server looks for the restarted agent.
const RECONNECT_POLL: Duration = Duration::from_millis(500);
/// How long after the restart an agent that names no process instance has to
/// have been connected before it counts as the restarted one: it cannot be told
/// from the old process by its identity, only by outlasting its shutdown.
const RECONNECT_GRACE: Duration = Duration::from_secs(3);

fn skipped_status(version: Option<String>) -> DeployStatusResponse {
    DeployStatusResponse {
        state: DeployJobState::Succeeded,
        steps: DeployStep::iter()
            .map(|step| DeployStepResponse {
                step,
                status: DeployStepStatus::Pending,
                duration_ms: None,
            })
            .collect(),
        skipped: true,
        token: None,
        available_version: version,
        error: None,
    }
}

/// Generates a new agent token and stores its hash, so the agent that holds
/// it is the only one the server accepts. Returns the token, which only the
/// host being deployed to and the user deploying ever see.
async fn issue_agent_token(pool: &sqlx::PgPool, agent_id: i64) -> Result<String, ApiError> {
    let token = helpers::generate_random_hex(32);
    let hash = bcrypt::hash(&token, bcrypt::DEFAULT_COST)?;
    db::regenerate_agent_token(pool, agent_id, &hash).await?;
    Ok(token)
}

/// Everything a deploy needs once it runs on its own, away from the request.
struct DeployRun {
    pool: sqlx::PgPool,
    registry: AgentRegistry,
    jobs: DeployJobs,
    agent_id: i64,
    hostname: String,
    ssh_host: String,
    ssh_user: String,
    ssh_port: u16,
    ssh_password: Option<String>,
    binary_dir: PathBuf,
    install_path: String,
    server_url: String,
    token: String,
    server_key: String,
    systemd_service_content: Option<String>,
    available_version: Option<String>,
    server_commit_count: Option<i32>,
}

/// Why a deploy stopped.
#[derive(Debug, thiserror::Error)]
enum DeployError {
    #[error("{0}")]
    Ssh(#[from] ssh::SshError),
    #[error(
        "the agent was installed and restarted but did not connect back to the server within \
         {} seconds; check the agent's logs on the host",
        RECONNECT_TIMEOUT.as_secs()
    )]
    NotReconnected,
}

/// Runs a deploy to its end and records how it went. Never fails: whatever
/// goes wrong is the deploy's result.
async fn run_deploy(run: DeployRun) {
    let before = presence(&run.registry, run.agent_id).await;
    let result = execute_deploy(&run, &before).await;
    if let Err(e) = &result {
        warn!(hostname = %run.hostname, error = %e, "agent deploy failed");
    }
    run.jobs.finish(
        run.agent_id,
        Outcome {
            skipped: false,
            token: Some(run.token),
            available_version: run.available_version,
            error: result.err().map(|e| e.to_string()),
        },
    );
}

async fn execute_deploy(run: &DeployRun, before: &Presence) -> Result<(), DeployError> {
    ssh::deploy_agent(
        &ssh::DeployAgentParams {
            host: &run.ssh_host,
            user: &run.ssh_user,
            port: run.ssh_port,
            binary_dir: &run.binary_dir,
            remote_path: &run.install_path,
            server_url: &run.server_url,
            token: &run.token,
            server_key: &run.server_key,
            password: run.ssh_password.as_deref(),
            systemd_service_content: run.systemd_service_content.as_deref(),
        },
        &|step| run.jobs.begin_step(run.agent_id, step),
    )
    .await?;

    // The host is deployed, so a failure to record it must not fail the deploy.
    if let Some(version) = &run.available_version
        && let Err(e) = db::update_last_seen_and_version(
            &run.pool,
            run.agent_id,
            version,
            None,
            None,
            run.server_commit_count,
        )
        .await
    {
        warn!(hostname = %run.hostname, error = %e, "failed to record the deployed agent version");
    }
    if let Err(e) = db::update_last_ssh_user(&run.pool, run.agent_id, &run.ssh_user).await {
        warn!(hostname = %run.hostname, error = %e, "failed to record the deploy's SSH user");
    }
    info!(hostname = %run.hostname, ssh_host = %run.ssh_host, "agent deployed, waiting for it to reconnect");

    run.jobs.begin_step(run.agent_id, DeployStep::Reconnect);
    if wait_for_reconnect(
        &run.registry,
        run.agent_id,
        before,
        RECONNECT_TIMEOUT,
        RECONNECT_POLL,
    )
    .await
    {
        info!(hostname = %run.hostname, "agent reconnected after deploy");
        Ok(())
    } else {
        Err(DeployError::NotReconnected)
    }
}

/// Whether an agent is connected to the server, and as which process.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Presence {
    /// Not connected.
    Absent,
    /// Connected as the process named by the instance id, or as one that names
    /// none.
    Present(Option<String>),
}

async fn presence(registry: &AgentRegistry, agent_id: i64) -> Presence {
    match registry.instance_id(agent_id).await {
        None => Presence::Absent,
        Some(instance) => Presence::Present(instance),
    }
}

/// Whether the agent connected now is the one the deploy restarted, rather than
/// the process that was running before it: `before` is the agent's presence
/// before the restart, `now` its presence now, `since_restart` how long the
/// restart has been under way.
fn is_restarted_agent(before: &Presence, now: &Presence, since_restart: Duration) -> bool {
    match (before, now) {
        (_, Presence::Absent) => false,
        // It was not connected, so any connection is the new one.
        (Presence::Absent, Presence::Present(_)) => true,
        (Presence::Present(Some(previous)), Presence::Present(current)) => {
            current.as_ref() != Some(previous)
        }
        // The old process named no instance, so only its shutdown tells the two
        // apart: a connection that is still there after a grace period is not it.
        (Presence::Present(None), Presence::Present(_)) => since_restart >= RECONNECT_GRACE,
    }
}

/// Waits up to `timeout` for the deployed agent to connect back. Returns
/// whether it did.
async fn wait_for_reconnect(
    registry: &AgentRegistry,
    agent_id: i64,
    before: &Presence,
    timeout: Duration,
    poll: Duration,
) -> bool {
    let started = tokio::time::Instant::now();
    loop {
        let now = presence(registry, agent_id).await;
        if is_restarted_agent(before, &now, started.elapsed()) {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(poll).await;
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
        (status = 403, description = "Forbidden -- upgrade agent permission required, or the \
            server runs in desktop mode"),
    )
)]
/// Read the existing systemd service unit from a remote host via SSH
/// (requires `can_upgrade_agent` permission).
///
/// # Errors
///
/// Returns [`ApiError::BadGateway`] if the upstream operation (e.g. SSH or borg) fails.
pub async fn fetch_service_unit(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(_hostname): Path<String>,
    ApiJson(req): ApiJson<FetchServiceUnitRequest>,
) -> Result<Json<FetchServiceUnitResponse>, ApiError> {
    ensure_agent_deploy_allowed(state.deployment_mode)?;
    require_upgrade_agent(&state.pool, auth.user_id).await?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;
    helpers::validate_max_len(&req.ssh_host, "ssh_host", MaxLen::Hostname)?;
    helpers::validate_max_len(&req.ssh_user, "ssh_user", MaxLen::Name)?;

    let port = req.ssh_port.unwrap_or(22);
    let content = ssh::read_remote_file(&ssh::ReadFileParams {
        host: &req.ssh_host,
        user: &req.ssh_user,
        port,
        password: req.ssh_password.as_deref(),
        path: "/etc/systemd/system/assimilate-agent.service",
    })
    .await
    .map_err(|e| ApiError::BadGateway(e.to_string()))?
    .map(|content| ssh::redact_agent_token(&content));

    Ok(Json(FetchServiceUnitResponse { content }))
}

/// Whether a deploy request can be skipped as a no-op: the agent is already
/// current, it isn't reached through a tunnel (which always redeploys, since
/// the tunnel itself may need re-establishing), and the caller didn't force it.
fn should_skip_deploy(uses_tunnel: bool, already_current: bool, force: bool) -> bool {
    !uses_tunnel && already_current && !force
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

/// Refuses to deploy an agent over SSH in desktop mode. The desktop app ships
/// its own agent and updates it with the app; installing one as a systemd
/// service on top of it would leave two agents for the same computer.
fn ensure_agent_deploy_allowed(mode: DeploymentMode) -> Result<(), ApiError> {
    match mode {
        DeploymentMode::Server => Ok(()),
        DeploymentMode::Desktop => Err(ApiError::Forbidden(
            "The desktop app's agent can't be deployed over SSH; it's updated with the app"
                .to_string(),
        )),
    }
}

/// Require the calling user to have the `can_upgrade_agent` permission.
async fn require_upgrade_agent(pool: &sqlx::PgPool, user_id: i64) -> Result<(), ApiError> {
    let effective = db::get_effective_permissions(pool, user_id).await?;
    if effective.can_delete_repo || effective.can_upgrade_agent {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "upgrade agent permission required".to_string(),
    ))
}

/// Rejects a deploy request whose strings exceed their [`MaxLen`] caps.
fn validate_deploy_lengths(req: &DeployAgentRequest) -> Result<(), ApiError> {
    helpers::validate_max_len(&req.ssh_host, "ssh_host", MaxLen::Hostname)?;
    helpers::validate_max_len(&req.ssh_user, "ssh_user", MaxLen::Name)?;
    helpers::validate_max_len(&req.server_url, "server_url", MaxLen::Url)?;
    helpers::validate_opt_max_len(req.install_path.as_deref(), "install_path", MaxLen::Path)?;
    helpers::validate_opt_max_len(
        req.systemd_service_content.as_deref(),
        "systemd_service_content",
        MaxLen::Text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy_request(overrides: &serde_json::Value) -> DeployAgentRequest {
        let mut body = serde_json::json!({
            "ssh_host": "web-server-01.example.com",
            "ssh_user": "root",
            "server_url": "https://backup.example.com",
        });
        body.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn deploy_lengths_accept_every_field_at_its_limit() {
        let req = deploy_request(&serde_json::json!({
            "ssh_host": "a".repeat(MaxLen::Hostname.chars()),
            "ssh_user": "a".repeat(MaxLen::Name.chars()),
            "server_url": "a".repeat(MaxLen::Url.chars()),
            "install_path": "a".repeat(MaxLen::Path.chars()),
            "systemd_service_content": "a".repeat(MaxLen::Text.chars()),
        }));
        assert!(validate_deploy_lengths(&req).is_ok());
    }

    #[test]
    fn deploy_lengths_reject_each_over_limit_field_by_name() {
        let cases = [
            ("ssh_host", MaxLen::Hostname),
            ("ssh_user", MaxLen::Name),
            ("server_url", MaxLen::Url),
            ("install_path", MaxLen::Path),
            ("systemd_service_content", MaxLen::Text),
        ];
        for (field, max) in cases {
            let req = deploy_request(
                &serde_json::json!({ field: "a".repeat(max.chars().saturating_add(1)) }),
            );
            let message = helpers::rejection_message(validate_deploy_lengths(&req));
            assert!(message.starts_with(&format!("{field} ")), "{message}");
        }
    }

    fn present(id: &str) -> Presence {
        Presence::Present(Some(id.to_owned()))
    }

    #[test]
    fn an_agent_that_was_not_connected_counts_as_restarted_once_it_connects() {
        assert!(is_restarted_agent(
            &Presence::Absent,
            &Presence::Present(None),
            Duration::ZERO
        ));
        assert!(is_restarted_agent(
            &Presence::Absent,
            &present("new"),
            Duration::ZERO
        ));
        assert!(!is_restarted_agent(
            &Presence::Absent,
            &Presence::Absent,
            Duration::from_mins(1)
        ));
    }

    #[test]
    fn a_connection_from_the_old_process_is_not_the_restarted_agent() {
        let before = present("old");
        assert!(!is_restarted_agent(
            &before,
            &present("old"),
            Duration::from_mins(1)
        ));
        assert!(!is_restarted_agent(
            &before,
            &Presence::Absent,
            Duration::from_mins(1)
        ));
        assert!(is_restarted_agent(&before, &present("new"), Duration::ZERO));
        assert!(
            is_restarted_agent(&before, &Presence::Present(None), Duration::ZERO),
            "an agent that now names no instance is a different process"
        );
    }

    #[test]
    fn an_agent_without_an_instance_must_outlast_the_grace_period() {
        let before = Presence::Present(None);
        assert!(!is_restarted_agent(
            &before,
            &Presence::Present(None),
            RECONNECT_GRACE.saturating_sub(Duration::from_millis(1))
        ));
        assert!(is_restarted_agent(
            &before,
            &Presence::Present(None),
            RECONNECT_GRACE
        ));
    }

    async fn connect(registry: &AgentRegistry, agent_id: i64, instance_id: &str) {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        registry
            .register(agent_id, sender, false, None, Some(instance_id.to_owned()))
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_returns_once_the_restarted_agent_connects() {
        let registry = AgentRegistry::new();
        connect(&registry, 1, "old").await;
        let before = presence(&registry, 1).await;

        let reconnecting = registry.clone();
        let wait = tokio::spawn(async move {
            wait_for_reconnect(
                &reconnecting,
                1,
                &before,
                Duration::from_secs(30),
                Duration::from_millis(500),
            )
            .await
        });
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!wait.is_finished(), "the old process is still connected");

        connect(&registry, 1, "new").await;
        assert!(wait.await.expect("the wait ends"));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_gives_up_when_the_agent_never_comes_back() {
        let registry = AgentRegistry::new();
        connect(&registry, 1, "old").await;
        let before = presence(&registry, 1).await;
        registry.unregister(1).await;

        let reconnected = wait_for_reconnect(
            &registry,
            1,
            &before,
            Duration::from_secs(30),
            Duration::from_millis(500),
        )
        .await;

        assert!(!reconnected);
    }

    #[test]
    fn a_skipped_deploy_reports_success_without_running_any_step() {
        let status = skipped_status(Some("1.2.0".to_owned()));

        assert_eq!(status.state, DeployJobState::Succeeded);
        assert!(status.skipped);
        assert_eq!(status.available_version.as_deref(), Some("1.2.0"));
        assert_eq!(status.token, None);
        assert_eq!(status.steps.len(), DeployStep::iter().count());
        assert!(
            status
                .steps
                .iter()
                .all(|s| s.status == DeployStepStatus::Pending)
        );
    }

    #[test]
    fn a_deploy_that_never_reconnects_says_how_long_it_waited() {
        let message = DeployError::NotReconnected.to_string();

        assert!(message.contains("90 seconds"), "{message}");
        assert!(message.contains("did not connect back"), "{message}");
    }

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
    fn should_skip_deploy_skips_only_when_current_untunneled_and_not_forced() {
        assert!(should_skip_deploy(false, true, false));
        assert!(
            !should_skip_deploy(false, true, true),
            "force must bypass the skip"
        );
        assert!(
            !should_skip_deploy(false, false, false),
            "not current -> never skip"
        );
        assert!(
            !should_skip_deploy(true, true, false),
            "a tunnel must always redeploy so it can be re-established"
        );
        assert!(!should_skip_deploy(true, true, true));
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

    #[test]
    fn agent_deploy_is_refused_only_in_desktop_mode() {
        assert!(ensure_agent_deploy_allowed(DeploymentMode::Server).is_ok());
        assert!(matches!(
            ensure_agent_deploy_allowed(DeploymentMode::Desktop),
            Err(ApiError::Forbidden(msg)) if msg.contains("desktop app")
        ));
    }

    mod desktop_mode {
        use axum::extract::{Path, Query, State};
        use shared::types::DeploymentMode;
        use sqlx::PgPool;

        use super::{
            super::{
                DeployAgentRequest, FetchServiceUnitRequest, deploy_agent, fetch_service_unit,
            },
            empty_role_params,
        };
        use crate::{
            AppState,
            api::{auth::AuthUser, helpers::DomainQuery},
            db,
            error::{ApiError, ApiJson},
            test_support::{build_test_state, insert_auth_user},
        };

        fn desktop_state(pool: &PgPool) -> AppState {
            AppState {
                deployment_mode: DeploymentMode::Desktop,
                ..build_test_state(pool.clone(), b"agent-deploy-test-key")
            }
        }

        /// A user who may deploy agents in server mode, so only the mode can
        /// refuse them.
        async fn upgrader(pool: &PgPool, username: &str) -> AuthUser {
            let auth = insert_auth_user(pool, username).await;
            let mut params = empty_role_params(username);
            params.can_upgrade_agent = true;
            let role = db::insert_role(pool, &params).await.expect("insert role");
            db::set_user_roles(pool, auth.user_id, &[role.id])
                .await
                .expect("assign role");
            auth
        }

        fn refused_for_desktop<T>(result: Result<T, ApiError>) -> bool {
            matches!(result, Err(ApiError::Forbidden(msg)) if msg.contains("desktop app"))
        }

        // Refused before any SSH connection is attempted: nothing listens
        // on the requested host.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn deploy_is_refused_in_desktop_mode(pool: PgPool) {
            let auth = upgrader(&pool, "desktop-deploy-user").await;
            let request = DeployAgentRequest {
                ssh_host: "192.0.2.1".to_owned(),
                ssh_user: "borg".to_owned(),
                ssh_port: None,
                ssh_password: None,
                server_url: "http://127.0.0.1:3000".to_owned(),
                install_path: None,
                systemd_service_content: None,
                force: false,
            };

            let result = deploy_agent(
                State(desktop_state(&pool)),
                auth,
                Path("desktop-host".to_owned()),
                Query(DomainQuery { domain: None }),
                ApiJson(request),
            )
            .await;

            assert!(refused_for_desktop(result));
        }

        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn reading_the_service_unit_is_refused_in_desktop_mode(pool: PgPool) {
            let auth = upgrader(&pool, "desktop-unit-user").await;
            let request = FetchServiceUnitRequest {
                ssh_host: "192.0.2.1".to_owned(),
                ssh_user: "borg".to_owned(),
                ssh_port: None,
                ssh_password: None,
            };

            let result = fetch_service_unit(
                State(desktop_state(&pool)),
                auth,
                Path("desktop-host".to_owned()),
                ApiJson(request),
            )
            .await;

            assert!(refused_for_desktop(result));
        }
    }

    fn empty_role_params(name: &str) -> db::InsertRoleParams<'_> {
        db::InsertRoleParams {
            name,
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: false,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn require_upgrade_agent_rejects_user_without_the_permission(pool: sqlx::PgPool) {
        let user = db::insert_user(&pool, "no-perms", "hash")
            .await
            .expect("insert user");
        let role = db::insert_role(&pool, &empty_role_params("no-perms-role"))
            .await
            .expect("insert role");
        db::set_user_roles(&pool, user.id, &[role.id])
            .await
            .expect("assign role");

        let result = require_upgrade_agent(&pool, user.id).await;
        assert!(matches!(result, Err(ApiError::Forbidden(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn require_upgrade_agent_allows_user_with_can_upgrade_agent(pool: sqlx::PgPool) {
        let user = db::insert_user(&pool, "upgrader", "hash")
            .await
            .expect("insert user");
        let mut params = empty_role_params("upgrader-role");
        params.can_upgrade_agent = true;
        let role = db::insert_role(&pool, &params).await.expect("insert role");
        db::set_user_roles(&pool, user.id, &[role.id])
            .await
            .expect("assign role");

        require_upgrade_agent(&pool, user.id)
            .await
            .expect("user with can_upgrade_agent should pass");
    }

    // Regression test for the missing admin-bypass clause: a role with
    // can_delete_repo (the codebase-wide "admin" bit - see
    // docs/access-control.md) but without can_upgrade_agent explicitly set
    // must still pass, matching every other per-capability check
    // (tags.rs::ensure_manage_tags and friends).
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn require_upgrade_agent_allows_admin_bypass_via_can_delete_repo(pool: sqlx::PgPool) {
        let user = db::insert_user(&pool, "admin-like", "hash")
            .await
            .expect("insert user");
        let mut params = empty_role_params("admin-like-role");
        params.can_delete_repo = true;
        let role = db::insert_role(&pool, &params).await.expect("insert role");
        db::set_user_roles(&pool, user.id, &[role.id])
            .await
            .expect("assign role");

        require_upgrade_agent(&pool, user.id)
            .await
            .expect("can_delete_repo should bypass the upgrade-agent check");
    }
}
