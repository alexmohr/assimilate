// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    protocol::ServerToAgent,
    responses::{
        AgentResponse, CreateAgentResponse, DeleteAgentArchivesResponse, MergeAgentResponse,
    },
};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers,
    permissions::is_visible_to_user,
};
use crate::{
    AppState, config_assembler,
    db::{self, AgentRow, IMPORTED_TOKEN_HASH, patterns::HostnamePatternRow},
    error::{ApiError, ApiJson},
};

/// Request payload for registering a new agent.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAgentRequest {
    /// Agent hostname.
    pub hostname: String,
    /// Optional display name.
    pub display_name: Option<String>,
}

/// Request payload for updating an agent's configuration.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateAgentRequest {
    /// New hostname (if changing).
    pub hostname: Option<String>,
    /// New display name.
    pub display_name: Option<String>,
    /// Default backup source paths.
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns.
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
    /// Default pre-backup commands.
    #[serde(default)]
    pub default_pre_backup_commands: Vec<String>,
    /// Default post-backup commands.
    #[serde(default)]
    pub default_post_backup_commands: Vec<String>,
    /// Default file change detection patterns.
    #[serde(default)]
    pub default_file_change_patterns_raw: String,
}

/// Builds an [`AgentResponse`] for `agent`, resolving live connection and
/// restart capability from the registry by the agent's own hostname.
async fn build_agent_response(state: &AppState, agent: AgentRow) -> AgentResponse {
    let is_connected = state.registry.is_connected(&agent.hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&agent.hostname).await;
    AgentResponse {
        id: agent.id,
        hostname: agent.hostname,
        display_name: agent.display_name,
        agent_version: agent.agent_version,
        agent_git_sha: agent.agent_git_sha,
        agent_build_time: agent.agent_build_time,
        agent_commit_count: agent.agent_commit_count,
        created_at: agent.created_at,
        last_seen_at: agent.last_seen_at,
        default_backup_paths: agent.default_backup_paths,
        default_exclude_patterns: agent.default_exclude_patterns,
        default_pre_backup_commands: agent.default_pre_backup_commands,
        default_post_backup_commands: agent.default_post_backup_commands,
        default_file_change_patterns_raw: agent.default_file_change_patterns_raw,
        is_connected,
        is_imported: agent.agent_token_hash == IMPORTED_TOKEN_HASH,
        is_hidden: agent.is_hidden,
        supports_restart,
        owner_id: agent.owner_id,
        visibility: agent.visibility.parse().unwrap_or_default(),
        restart_unavailable_reason,
        last_ssh_user: agent.last_ssh_user,
    }
}

#[utoipa::path(
    post,
    path = "/api/agents",
    tag = "Agents",
    operation_id = "createAgent",
    request_body = CreateAgentRequest,
    responses(
        (status = 201, description = "Agent registered", body = CreateAgentResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
/// Register a new agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn create_agent(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateAgentRequest>,
) -> Result<(StatusCode, Json<CreateAgentResponse>), ApiError> {
    helpers::validate_non_empty(&req.hostname, "hostname")?;

    let token_hex = helpers::generate_random_hex(32);

    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    let agent = db::insert_agent(
        &state.pool,
        &req.hostname,
        req.display_name.as_deref(),
        &token_hash,
        Some(admin.user_id),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateAgentResponse {
            agent: build_agent_response(&state, agent).await,
            token: token_hex,
        }),
    ))
}

/// Query parameters for listing agents.
#[derive(Debug, Deserialize)]
pub struct ListAgentsQuery {
    /// Whether to include hidden agents in the response.
    #[serde(default)]
    pub include_hidden: bool,
}

#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "Agents",
    operation_id = "listAgents",
    responses(
        (status = 200, description = "List of agents", body = Vec<AgentResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
/// List all agents.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_agents(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<Vec<AgentResponse>>, ApiError> {
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let is_admin = effective.can_delete_repo;
    let include_hidden = query.include_hidden && is_admin;
    let agents = db::list_agents(&state.pool, include_hidden).await?;
    let mut responses = Vec::with_capacity(agents.len());
    for a in agents {
        if !is_visible_to_user(
            &state.pool,
            auth.user_id,
            a.owner_id,
            &a.visibility,
            is_admin,
        )
        .await?
        {
            continue;
        }
        responses.push(build_agent_response(&state, a).await);
    }
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "getAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent details", body = AgentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Get an agent by hostname.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    Ok(Json(build_agent_response(&state, agent).await))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "updateAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    request_body = UpdateAgentRequest,
    responses(
        (status = 200, description = "Updated agent", body = AgentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Update an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn update_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<UpdateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    let new_hostname = req.hostname.as_deref().unwrap_or(&hostname);
    let pre_cmds = serde_json::to_string(&req.default_pre_backup_commands)
        .unwrap_or_else(|_| "[]".to_string());
    let post_cmds = serde_json::to_string(&req.default_post_backup_commands)
        .unwrap_or_else(|_| "[]".to_string());
    let agent = db::update_agent(
        &state.pool,
        &hostname,
        new_hostname,
        db::AgentDefaults {
            display_name: req.display_name.as_deref(),
            default_backup_paths: &req.default_backup_paths,
            default_exclude_patterns: &req.default_exclude_patterns,
            default_pre_backup_commands: &pre_cmds,
            default_post_backup_commands: &post_cmds,
            default_file_change_patterns_raw: &req.default_file_change_patterns_raw,
        },
    )
    .await?;
    config_assembler::push_config_to_agent(&state, new_hostname).await;
    Ok(Json(build_agent_response(&state, agent).await))
}

#[utoipa::path(
    delete,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "deleteAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Delete an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<StatusCode, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;

    if let Ok(tunnel) = db::get_tunnel_by_agent_id(&state.pool, agent.id).await {
        state.tunnel_manager.stop_tunnel(tunnel.id).await;
    }

    db::delete_agent(&state.pool, &hostname).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/regenerate-token",
    tag = "Agents",
    operation_id = "regenerateAgentToken",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "New token issued", body = CreateAgentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Regenerate the agent token for an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn regenerate_token(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<CreateAgentResponse>, ApiError> {
    let existing = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let was_imported = existing.agent_token_hash == IMPORTED_TOKEN_HASH;

    let token_hex = helpers::generate_random_hex(32);

    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    let agent = db::regenerate_agent_token(&state.pool, &hostname, &token_hash).await?;

    if was_imported {
        db::mark_agent_reports_matched(&state.pool, agent.id).await?;
    }

    Ok(Json(CreateAgentResponse {
        agent: shared::responses::AgentResponse {
            is_connected: false,
            is_imported: false,
            supports_restart: false,
            restart_unavailable_reason: None,
            ..build_agent_response(&state, agent).await
        },
        token: token_hex,
    }))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/restart",
    tag = "Agents",
    operation_id = "restartAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 202, description = "Restart accepted"),
        (status = 400, description = "Restart not supported"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Send a restart command to the agent.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn restart_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (supports_restart, reason) = state.registry.restart_capability(&hostname).await;

    if !supports_restart {
        return Err(ApiError::BadRequest(
            reason.unwrap_or_else(|| "restart not supported".to_owned()),
        ));
    }

    state
        .registry
        .send_to(&hostname, ServerToAgent::RestartAgent)
        .await
        .map_err(|e| ApiError::Internal(format!("agent not connected: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/hostname-patterns",
    tag = "Agents",
    operation_id = "listHostnamePatterns",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "List of patterns", body = Vec<HostnamePatternRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List hostname patterns for an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_hostname_patterns(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<HostnamePatternRow>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let patterns = db::patterns::list_patterns_for_agent(&state.pool, agent.id).await?;
    Ok(Json(patterns))
}

/// Request payload for adding a hostname pattern to an agent.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AddPatternRequest {
    /// Glob pattern for matching archive hostnames.
    pub pattern: String,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/hostname-patterns",
    tag = "Agents",
    operation_id = "addHostnamePattern",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    request_body = AddPatternRequest,
    responses(
        (status = 201, description = "Pattern created", body = HostnamePatternRow),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Duplicate pattern"),
    )
)]
/// Add a hostname pattern to an agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn add_hostname_pattern(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<AddPatternRequest>,
) -> Result<(StatusCode, Json<HostnamePatternRow>), ApiError> {
    helpers::validate_non_empty(&req.pattern, "pattern")?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let row = db::patterns::add_hostname_pattern(&state.pool, agent.id, &req.pattern).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    delete,
    path = "/api/agents/{hostname}/hostname-patterns/{pattern_id}",
    tag = "Agents",
    operation_id = "deleteHostnamePattern",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("pattern_id" = i64, Path, description = "Pattern ID"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Delete a hostname pattern.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_hostname_pattern(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path((hostname, pattern_id)): Path<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    db::get_agent_by_hostname(&state.pool, &hostname).await?;
    db::patterns::delete_hostname_pattern(&state.pool, pattern_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Request payload for merging a placeholder agent into a real one.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MergeAgentRequest {
    /// Optional hostname pattern to create on the target agent.
    pub create_pattern: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/merge-from/{source_id}",
    tag = "Agents",
    operation_id = "mergeAgent",
    params(
        ("hostname" = String, Path, description = "Target agent hostname"),
        ("source_id" = i64, Path, description = "Source placeholder agent ID"),
    ),
    request_body(content = Option<MergeAgentRequest>, content_type = "application/json"),
    responses(
        (status = 200, description = "Merge completed", body = MergeAgentResponse),
        (status = 400, description = "Source is not a placeholder"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Merge a placeholder agent into this agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn merge_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path((hostname, source_id)): Path<(String, i64)>,
    ApiJson(req): ApiJson<MergeAgentRequest>,
) -> Result<Json<MergeAgentResponse>, ApiError> {
    let target = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    db::merge_agent(&state.pool, source_id, target.id).await?;

    if let Some(pattern) = &req.create_pattern
        && !pattern.is_empty()
    {
        db::patterns::add_hostname_pattern(&state.pool, target.id, pattern).await?;
    }

    Ok(Json(MergeAgentResponse { merged: true }))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/hide",
    tag = "Agents",
    operation_id = "hideAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent hidden"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Hide an agent from all views.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn hide_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let a = db::set_agent_hidden(&state.pool, &hostname, true).await?;
    Ok(Json(build_agent_response(&state, a).await))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/unhide",
    tag = "Agents",
    operation_id = "unhideAgent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent unhidden"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Unhide a previously hidden agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn unhide_agent(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let a = db::set_agent_hidden(&state.pool, &hostname, false).await?;
    Ok(Json(build_agent_response(&state, a).await))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/delete-archives",
    tag = "Agents",
    operation_id = "deleteAgentArchives",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Archives deleted", body = DeleteAgentArchivesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Agent offline"),
    )
)]
/// Delete all borg archives belonging to this agent and remove the agent.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_agent_archives(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<DeleteAgentArchivesResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;

    let archives_by_repo = db::get_archives_for_agent_with_patterns(&state.pool, agent.id).await?;

    let mut total_deleted: u32 = 0;
    let mut errors: Vec<String> = Vec::new();

    for (repo_id, archive_names) in &archives_by_repo {
        let targets = db::get_schedule_target_hostnames_for_repo(&state.pool, repo_id.0).await?;

        let mut connected_host = None;
        for h in &targets {
            if state.registry.is_connected(h).await {
                connected_host = Some(h.clone());
                break;
            }
        }

        let Some(agent_hostname) = connected_host else {
            errors.push(format!(
                "no connected agent for repo {} -- skipped {} archives",
                repo_id.0,
                archive_names.len()
            ));
            continue;
        };

        let request_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();

        state
            .pending_deletes
            .lock()
            .await
            .insert(request_id.clone(), tx);

        let msg = ServerToAgent::DeleteArchives {
            request_id: request_id.clone(),
            repo_id: *repo_id,
            archive_names: archive_names.clone(),
        };

        if state.registry.send_to(&agent_hostname, msg).await.is_err() {
            state.pending_deletes.lock().await.remove(&request_id);
            errors.push(format!("failed to send to agent for repo {}", repo_id.0));
            continue;
        }

        match tokio::time::timeout(Duration::from_mins(5), rx).await {
            Ok(Ok((true, count, _))) => {
                total_deleted = total_deleted.saturating_add(count);
            }
            Ok(Ok((false, count, Some(err)))) => {
                total_deleted = total_deleted.saturating_add(count);
                errors.push(format!("repo {}: {err}", repo_id.0));
            }
            Ok(Ok((false, count, None))) => {
                total_deleted = total_deleted.saturating_add(count);
                errors.push(format!("repo {}: unknown error", repo_id.0));
            }
            Ok(Err(_)) => {
                errors.push(format!("repo {}: response channel closed", repo_id.0));
            }
            Err(_) => {
                state.pending_deletes.lock().await.remove(&request_id);
                errors.push(format!("repo {}: timed out", repo_id.0));
            }
        }
    }

    if errors.is_empty() {
        db::delete_agent(&state.pool, &hostname).await?;
    }

    Ok(Json(DeleteAgentArchivesResponse {
        success: errors.is_empty(),
        total_deleted,
        errors,
    }))
}
