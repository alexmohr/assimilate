// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use shared::protocol::ServerToAgent;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{
    auth::{AuthUser, RequireAdmin, Role},
    helpers,
    permissions::is_visible_to_user,
};
use crate::{
    AppState, config_assembler,
    db::{self, AgentRow, patterns::HostnamePatternRow},
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAgentRequest {
    pub hostname: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateAgentRequest {
    pub hostname: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AgentResponse {
    #[serde(flatten)]
    pub agent: AgentRow,
    pub is_connected: bool,
    pub is_imported: bool,
    pub supports_restart: bool,
    pub restart_unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreateAgentResponse {
    pub agent: AgentRow,
    pub token: String,
}

#[utoipa::path(
    post,
    path = "/api/agents",
    tag = "Agents",
    operation_id = "createAgent",
    summary = "Register a new agent",
    request_body = CreateAgentRequest,
    responses(
        (status = 201, description = "Agent registered", body = CreateAgentResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
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
            agent,
            token: token_hex,
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ListAgentsQuery {
    #[serde(default)]
    pub include_hidden: bool,
}

#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "Agents",
    operation_id = "listAgents",
    summary = "List all agents",
    responses(
        (status = 200, description = "List of agents", body = Vec<AgentResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_agents(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<Vec<AgentResponse>>, ApiError> {
    let is_admin = auth.role == Role::Admin;
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
        let is_connected = state.registry.is_connected(&a.hostname).await;
        let (supports_restart, restart_unavailable_reason) =
            state.registry.restart_capability(&a.hostname).await;
        responses.push(AgentResponse {
            is_imported: a.agent_token_hash == "imported:no-auth",
            agent: a,
            is_connected,
            supports_restart,
            restart_unavailable_reason,
        });
    }
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "getAgent",
    summary = "Get an agent by hostname",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent details", body = AgentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let is_connected = state.registry.is_connected(&hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&hostname).await;
    Ok(Json(AgentResponse {
        is_imported: agent.agent_token_hash == "imported:no-auth",
        agent,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "updateAgent",
    summary = "Update an agent",
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
pub async fn update_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<UpdateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    let new_hostname = req.hostname.as_deref().unwrap_or(&hostname);
    let agent = db::update_agent(
        &state.pool,
        &hostname,
        new_hostname,
        req.display_name.as_deref(),
        &req.default_backup_paths,
        &req.default_exclude_patterns,
    )
    .await?;
    config_assembler::push_config_to_agent(&state, new_hostname).await;
    let is_connected = state.registry.is_connected(&hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&hostname).await;
    Ok(Json(AgentResponse {
        is_imported: agent.agent_token_hash == "imported:no-auth",
        agent,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/agents/{hostname}",
    tag = "Agents",
    operation_id = "deleteAgent",
    summary = "Delete an agent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
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
    summary = "Regenerate the agent token for an agent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "New token issued", body = CreateAgentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn regenerate_token(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<CreateAgentResponse>, ApiError> {
    let existing = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let was_imported = existing.agent_token_hash == "imported:no-auth";

    let token_hex = helpers::generate_random_hex(32);

    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    let agent = db::regenerate_agent_token(&state.pool, &hostname, &token_hash).await?;

    if was_imported {
        db::mark_agent_reports_matched(&state.pool, agent.id).await?;
    }

    Ok(Json(CreateAgentResponse {
        agent,
        token: token_hex,
    }))
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/restart",
    tag = "Agents",
    operation_id = "restartAgent",
    summary = "Send a restart command to the agent",
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
pub async fn restart_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
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
    summary = "List hostname patterns for an agent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "List of patterns", body = Vec<HostnamePatternRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn list_hostname_patterns(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<HostnamePatternRow>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let patterns = db::patterns::list_patterns_for_agent(&state.pool, agent.id).await?;
    Ok(Json(patterns))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AddPatternRequest {
    pub pattern: String,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/hostname-patterns",
    tag = "Agents",
    operation_id = "addHostnamePattern",
    summary = "Add a hostname pattern to an agent",
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
pub async fn add_hostname_pattern(
    State(state): State<AppState>,
    _auth: AuthUser,
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
    summary = "Delete a hostname pattern",
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
pub async fn delete_hostname_pattern(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((hostname, pattern_id)): Path<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    db::get_agent_by_hostname(&state.pool, &hostname).await?;
    db::patterns::delete_hostname_pattern(&state.pool, pattern_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MergeAgentRequest {
    pub create_pattern: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MergeAgentResponse {
    pub merged: bool,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/merge-from/{source_id}",
    tag = "Agents",
    operation_id = "mergeAgent",
    summary = "Merge a placeholder agent into this agent",
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
pub async fn merge_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
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
    summary = "Hide an agent from all views",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent hidden"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn hide_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let a = db::set_agent_hidden(&state.pool, &hostname, true).await?;
    let is_connected = state.registry.is_connected(&a.hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&a.hostname).await;
    Ok(Json(AgentResponse {
        is_imported: a.agent_token_hash == "imported:no-auth",
        agent: a,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/unhide",
    tag = "Agents",
    operation_id = "unhideAgent",
    summary = "Unhide a previously hidden agent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Agent unhidden"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn unhide_agent(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<AgentResponse>, ApiError> {
    let a = db::set_agent_hidden(&state.pool, &hostname, false).await?;
    let is_connected = state.registry.is_connected(&a.hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&a.hostname).await;
    Ok(Json(AgentResponse {
        is_imported: a.agent_token_hash == "imported:no-auth",
        agent: a,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeleteArchivesResponse {
    pub success: bool,
    pub total_deleted: u32,
    pub errors: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/agents/{hostname}/delete-archives",
    tag = "Agents",
    operation_id = "deleteAgentArchives",
    summary = "Delete all borg archives belonging to this agent and remove the agent",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "Archives deleted", body = DeleteArchivesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Agent offline"),
    )
)]
pub async fn delete_agent_archives(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<DeleteArchivesResponse>, ApiError> {
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

        match tokio::time::timeout(Duration::from_secs(300), rx).await {
            Ok(Ok((true, count, _))) => {
                total_deleted += count;
            }
            Ok(Ok((false, count, Some(err)))) => {
                total_deleted += count;
                errors.push(format!("repo {}: {err}", repo_id.0));
            }
            Ok(Ok((false, count, None))) => {
                total_deleted += count;
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

    Ok(Json(DeleteArchivesResponse {
        success: errors.is_empty(),
        total_deleted,
        errors,
    }))
}
