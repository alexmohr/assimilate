// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use shared::protocol::ServerToAgent;

use super::{
    auth::{AuthUser, Role},
    helpers,
    permissions::is_visible_to_user,
};
use crate::{
    AppState, config_assembler,
    db::{self, ClientRow},
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateClientRequest {
    pub hostname: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateClientRequest {
    pub display_name: Option<String>,
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClientResponse {
    #[serde(flatten)]
    pub client: ClientRow,
    pub is_connected: bool,
    pub supports_restart: bool,
    pub restart_unavailable_reason: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreateClientResponse {
    pub client: ClientRow,
    pub token: String,
}

#[utoipa::path(
    post,
    path = "/api/clients",
    tag = "Hosts",
    operation_id = "createClient",
    summary = "Register a new host/client",
    request_body = CreateClientRequest,
    responses(
        (status = 201, description = "Client created", body = CreateClientResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn create_client(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<CreateClientRequest>,
) -> Result<(StatusCode, Json<CreateClientResponse>), ApiError> {
    helpers::validate_non_empty(&req.hostname, "hostname")?;

    let token_hex = helpers::generate_random_hex(32);

    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    let client = db::insert_client(
        &state.pool,
        &req.hostname,
        req.display_name.as_deref(),
        &token_hash,
        Some(auth.user_id),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateClientResponse {
            client,
            token: token_hex,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/clients",
    tag = "Hosts",
    operation_id = "listClients",
    summary = "List all hosts/clients",
    responses(
        (status = 200, description = "List of clients", body = Vec<ClientResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_clients(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ClientResponse>>, ApiError> {
    let clients = db::list_clients(&state.pool).await?;
    let is_admin = auth.role == Role::Admin;
    let mut responses = Vec::with_capacity(clients.len());
    for c in clients {
        if !is_visible_to_user(
            &state.pool,
            auth.user_id,
            c.owner_id,
            &c.visibility,
            is_admin,
        )
        .await?
        {
            continue;
        }
        let is_connected = state.registry.is_connected(&c.hostname).await;
        let (supports_restart, restart_unavailable_reason) =
            state.registry.restart_capability(&c.hostname).await;
        responses.push(ClientResponse {
            client: c,
            is_connected,
            supports_restart,
            restart_unavailable_reason,
        });
    }
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/clients/{hostname}",
    tag = "Hosts",
    operation_id = "getClient",
    summary = "Get a host/client by hostname",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
    ),
    responses(
        (status = 200, description = "Client details", body = ClientResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_client(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<ClientResponse>, ApiError> {
    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;
    let is_connected = state.registry.is_connected(&hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&hostname).await;
    Ok(Json(ClientResponse {
        client,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[utoipa::path(
    put,
    path = "/api/clients/{hostname}",
    tag = "Hosts",
    operation_id = "updateClient",
    summary = "Update a host/client",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
    ),
    request_body = UpdateClientRequest,
    responses(
        (status = 200, description = "Updated client", body = ClientResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_client(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<UpdateClientRequest>,
) -> Result<Json<ClientResponse>, ApiError> {
    let client = db::update_client(
        &state.pool,
        &hostname,
        req.display_name.as_deref(),
        &req.default_backup_paths,
        &req.default_exclude_patterns,
    )
    .await?;
    config_assembler::push_config_to_agent(&state, &hostname).await;
    let is_connected = state.registry.is_connected(&hostname).await;
    let (supports_restart, restart_unavailable_reason) =
        state.registry.restart_capability(&hostname).await;
    Ok(Json(ClientResponse {
        client,
        is_connected,
        supports_restart,
        restart_unavailable_reason,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/clients/{hostname}",
    tag = "Hosts",
    operation_id = "deleteClient",
    summary = "Delete a host/client",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_client(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<StatusCode, ApiError> {
    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;

    if let Ok(tunnel) = db::get_tunnel_by_client_id(&state.pool, client.id).await {
        state.tunnel_manager.stop_tunnel(tunnel.id).await;
    }

    db::delete_client(&state.pool, &hostname).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/clients/{hostname}/regenerate-token",
    tag = "Hosts",
    operation_id = "regenerateClientToken",
    summary = "Regenerate the agent token for a host",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
    ),
    responses(
        (status = 200, description = "New token issued", body = CreateClientResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn regenerate_token(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<CreateClientResponse>, ApiError> {
    let token_hex = helpers::generate_random_hex(32);

    let token_hash = bcrypt::hash(&token_hex, bcrypt::DEFAULT_COST)?;

    let client = db::regenerate_client_token(&state.pool, &hostname, &token_hash).await?;

    Ok(Json(CreateClientResponse {
        client,
        token: token_hex,
    }))
}

#[utoipa::path(
    post,
    path = "/api/clients/{hostname}/restart",
    tag = "Hosts",
    operation_id = "restartAgent",
    summary = "Send a restart command to the agent",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
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
