// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dependency hosts: machines a backup needs besides its agent and its
//! repository, such as the server whose share a pre-backup command mounts.
//!
//! Anyone signed in may list them and see which schedules need them - a
//! schedule's own Dependencies pane names them - but only an admin may change
//! or test one. How one is woken and whether it is always online are its
//! machine's settings, in [`super::machines`].

use std::collections::{BTreeSet, HashMap};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    dependency_hosts::{
        AgentDependenciesBody, DependencyHostResponse, DependencyRepoHostRef, DependencySourceKind,
        DependencyTestResponse, DependencyUsageResponse, ScheduleDependenciesResponse,
        ScheduleDependencyResponse, UpdateDependencyMachineRequest,
        UpdateScheduleDependenciesRequest,
    },
    protocol::ServerToUi,
};

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::{self, DomainQuery},
};
use crate::{
    AppState,
    db::{
        self,
        dependency_hosts::{
            DependencyConnectionPatch, DependencyHostRow, DependencySource, NewDependencyHost,
        },
    },
    error::{ApiError, ApiJson},
};

/// Longest name accepted, so a card and a run timeline stay readable.
const MAX_NAME_LEN: usize = 100;
/// Longest description accepted.
const MAX_DESCRIPTION_LEN: usize = 500;
/// Longest address accepted - a fully qualified domain name's limit.
const MAX_ADDRESS_LEN: usize = 253;

/// Request payload for a new dependency.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDependencyHostRequest {
    /// Display name, unique.
    pub name: String,
    /// Hostname or IP address, as the Assimilate server reaches it.
    pub address: String,
    /// TCP port whose answer means the machine is up (445 for SMB, 2049 for
    /// NFS, 22 for SSH).
    pub port: i32,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// The repository host this is the same machine as, to share its
    /// machine - how it is woken and whether it is always online.
    #[serde(default)]
    pub repo_host_id: Option<i64>,
}

/// Request payload for a dependency's name, address, port and description.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateDependencyConnectionRequest {
    /// Display name, unique.
    pub name: String,
    /// Hostname or IP address.
    pub address: String,
    /// TCP port to check.
    pub port: i32,
    /// What it is for.
    #[serde(default)]
    pub description: String,
}

/// Request payload for testing an address and port before saving them.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TestDependencyRequest {
    /// Hostname or IP address.
    pub address: String,
    /// TCP port.
    pub port: i32,
}

/// Trims and checks a name, returning the trimmed value.
fn validate_name(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    helpers::validate_non_empty(name, "name")?;
    if name.chars().count() > MAX_NAME_LEN {
        return Err(ApiError::BadRequest(format!(
            "name must be at most {MAX_NAME_LEN} characters"
        )));
    }
    Ok(name)
}

/// Trims and checks an address: a hostname or IP, so no whitespace and no
/// scheme or path.
fn validate_address(address: &str) -> Result<&str, ApiError> {
    let address = address.trim();
    helpers::validate_non_empty(address, "address")?;
    if address.len() > MAX_ADDRESS_LEN
        || address
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '?' | '#'))
    {
        return Err(ApiError::BadRequest(
            "address must be a hostname or IP address".to_owned(),
        ));
    }
    Ok(address)
}

fn validate_port(port: i32) -> Result<i32, ApiError> {
    if (1..=65_535).contains(&port) {
        Ok(port)
    } else {
        Err(ApiError::BadRequest(
            "port must be between 1 and 65535".to_owned(),
        ))
    }
}

fn validate_description(description: &str) -> Result<&str, ApiError> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(ApiError::BadRequest(format!(
            "description must be at most {MAX_DESCRIPTION_LEN} characters"
        )));
    }
    Ok(description)
}

/// Confirms a repository host to share settings with exists, so a typo is a
/// 400 naming the field rather than an opaque foreign-key error.
async fn validate_repo_host(state: &AppState, repo_host_id: Option<i64>) -> Result<(), ApiError> {
    let Some(id) = repo_host_id else {
        return Ok(());
    };
    match db::repo_hosts::get_repo_host(&state.pool, id).await {
        Ok(_) => Ok(()),
        Err(ApiError::NotFound(_)) => Err(ApiError::BadRequest(format!(
            "repository host {id} does not exist"
        ))),
        Err(e) => Err(e),
    }
}

/// Builds the response for one dependency.
async fn host_response(
    state: &AppState,
    host: DependencyHostRow,
    counts: (i64, i64, i64),
) -> Result<DependencyHostResponse, ApiError> {
    let repo_host = match host.repo_host_id {
        Some(id) => match db::repo_hosts::get_repo_host(&state.pool, id).await {
            Ok(row) => Some(DependencyRepoHostRef {
                id: row.id,
                ssh_host: row.ssh_host,
            }),
            Err(ApiError::NotFound(_)) => None,
            Err(e) => return Err(e),
        },
        None => None,
    };
    let (schedule_count, agent_default_count, waiting_count) = counts;
    Ok(DependencyHostResponse {
        id: host.id,
        name: host.name,
        address: host.address,
        port: host.port,
        description: host.description,
        machine_id: host.machine_id,
        repo_host,
        intermittent: host.intermittent,
        last_checked_at: host.last_checked_at,
        last_check_reachable: host.last_check_reachable,
        schedule_count,
        agent_default_count,
        waiting_count,
    })
}

/// Loads one dependency with its counts, for every handler that returns one.
async fn load_response(state: &AppState, id: i64) -> Result<DependencyHostResponse, ApiError> {
    let summary = db::dependency_hosts::get_dependency_host_summary(&state.pool, id).await?;
    let counts = (
        summary.schedule_count,
        summary.agent_default_count,
        summary.waiting_count,
    );
    host_response(state, summary.host, counts).await
}

fn data_changed(state: &AppState) {
    state.ui_broadcast.send(ServerToUi::DataChanged);
}

#[utoipa::path(
    get,
    path = "/api/dependency-hosts",
    tag = "Dependency hosts",
    operation_id = "listDependencyHosts",
    responses(
        (status = 200, description = "Dependency hosts", body = Vec<DependencyHostResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
/// List every dependency host.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn list_dependency_hosts(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<DependencyHostResponse>>, ApiError> {
    let mut out = Vec::new();
    for summary in db::dependency_hosts::list_dependency_hosts(&state.pool).await? {
        let counts = (
            summary.schedule_count,
            summary.agent_default_count,
            summary.waiting_count,
        );
        out.push(host_response(&state, summary.host, counts).await?);
    }
    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/api/dependency-hosts",
    tag = "Dependency hosts",
    operation_id = "createDependencyHost",
    request_body = CreateDependencyHostRequest,
    responses(
        (status = 201, description = "Created dependency host", body = DependencyHostResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "Name already taken"),
    )
)]
/// Create a dependency host (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid field, or
/// [`ApiError::Conflict`] if the name is taken.
pub async fn create_dependency_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateDependencyHostRequest>,
) -> Result<(StatusCode, Json<DependencyHostResponse>), ApiError> {
    let name = validate_name(&req.name)?;
    let address = validate_address(&req.address)?;
    let port = validate_port(req.port)?;
    let description = validate_description(&req.description)?;
    validate_repo_host(&state, req.repo_host_id).await?;
    let host = db::dependency_hosts::insert_dependency_host(
        &state.pool,
        &NewDependencyHost {
            name,
            address,
            port,
            description,
            repo_host_id: req.repo_host_id,
        },
    )
    .await?;
    tracing::info!(dependency = %host.name, address = %host.address, port, "dependency host created");
    data_changed(&state);
    Ok((
        StatusCode::CREATED,
        Json(host_response(&state, host, (0, 0, 0)).await?),
    ))
}

#[utoipa::path(
    post,
    path = "/api/dependency-hosts/test",
    tag = "Dependency hosts",
    operation_id = "testDependencyAddress",
    request_body = TestDependencyRequest,
    responses(
        (status = 200, description = "What the test found", body = DependencyTestResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Check whether an address answers on a port, before it is saved (admin
/// only - it makes the server connect wherever it is told to).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid address or port.
pub async fn test_dependency_address(
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<TestDependencyRequest>,
) -> Result<Json<DependencyTestResponse>, ApiError> {
    let address = validate_address(&req.address)?;
    let port = validate_port(req.port)?;
    let reachable = crate::dependencies::probe(address, port).await;
    Ok(Json(DependencyTestResponse {
        reachable,
        address: address.to_owned(),
        port,
        timeout_seconds: crate::dependencies::probe_timeout_seconds(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/dependency-hosts/{dependency_host_id}",
    tag = "Dependency hosts",
    operation_id = "getDependencyHost",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    responses(
        (status = 200, description = "Dependency host", body = DependencyHostResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Get one dependency host.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn get_dependency_host(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<DependencyHostResponse>, ApiError> {
    Ok(Json(load_response(&state, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/dependency-hosts/{dependency_host_id}",
    tag = "Dependency hosts",
    operation_id = "updateDependencyHost",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    request_body = UpdateDependencyConnectionRequest,
    responses(
        (status = 200, description = "Updated dependency host", body = DependencyHostResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Name already taken"),
    )
)]
/// Change a dependency host's name, address, port and description (admin
/// only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid field,
/// [`ApiError::Conflict`] if the name is taken, or [`ApiError::NotFound`].
pub async fn update_dependency_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateDependencyConnectionRequest>,
) -> Result<Json<DependencyHostResponse>, ApiError> {
    let patch = DependencyConnectionPatch {
        name: validate_name(&req.name)?,
        address: validate_address(&req.address)?,
        port: validate_port(req.port)?,
        description: validate_description(&req.description)?,
    };
    db::dependency_hosts::update_dependency_host_connection(&state.pool, id, &patch).await?;
    data_changed(&state);
    Ok(Json(load_response(&state, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/dependency-hosts/{dependency_host_id}",
    tag = "Dependency hosts",
    operation_id = "deleteDependencyHost",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Remove a dependency host (admin only). Schedules and agent defaults that
/// needed it stop needing it, and runs waiting on it stop waiting.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn delete_dependency_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::dependency_hosts::delete_dependency_host(&state.pool, id).await?;
    tracing::info!(dependency_host_id = id, "dependency host deleted");
    data_changed(&state);
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/dependency-hosts/{dependency_host_id}/machine",
    tag = "Dependency hosts",
    operation_id = "updateDependencyHostMachine",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    request_body = UpdateDependencyMachineRequest,
    responses(
        (status = 200, description = "Updated dependency host", body = DependencyHostResponse),
        (status = 400, description = "Unknown repository host"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Move a dependency host onto the machine of the repository host it is the
/// same machine as, or onto a machine of its own (admin only).
///
/// Moving it onto a machine of its own starts that machine with default
/// settings: never woken, always online.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an unknown repository host, or
/// [`ApiError::NotFound`].
pub async fn update_dependency_host_machine(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateDependencyMachineRequest>,
) -> Result<Json<DependencyHostResponse>, ApiError> {
    validate_repo_host(&state, req.repo_host_id).await?;
    db::dependency_hosts::set_dependency_host_repo_host(&state.pool, id, req.repo_host_id).await?;
    data_changed(&state);
    Ok(Json(load_response(&state, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/dependency-hosts/{dependency_host_id}/test",
    tag = "Dependency hosts",
    operation_id = "testDependencyHost",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    responses(
        (status = 200, description = "What the test found", body = DependencyTestResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Check whether a saved dependency host answers right now, and remember the
/// answer (admin only). Never wakes it and never starts a catch-up - Check now
/// does that.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn test_dependency_host(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<DependencyTestResponse>, ApiError> {
    let dependency = db::dependency_hosts::get_required_dependency(&state.pool, id).await?;
    let reachable = crate::dependencies::probe_and_record(&state.pool, &dependency).await;
    data_changed(&state);
    Ok(Json(DependencyTestResponse {
        reachable,
        address: dependency.address,
        port: dependency.port,
        timeout_seconds: crate::dependencies::probe_timeout_seconds(),
    }))
}

impl From<DependencySource> for DependencySourceKind {
    fn from(source: DependencySource) -> Self {
        match source {
            DependencySource::Schedule => Self::Schedule,
            DependencySource::AgentDefault => Self::AgentDefault,
            DependencySource::RepoHost => Self::RepoHost,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/dependency-hosts/{dependency_host_id}/usage",
    tag = "Dependency hosts",
    operation_id = "listDependencyHostUsage",
    params(("dependency_host_id" = i64, Path, description = "Dependency host ID")),
    responses(
        (status = 200, description = "Schedules that need it", body = Vec<DependencyUsageResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Every schedule and agent that needs a dependency host.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn list_dependency_host_usage(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Vec<DependencyUsageResponse>>, ApiError> {
    db::dependency_hosts::get_dependency_host(&state.pool, id).await?;
    let usage = db::dependency_hosts::list_dependency_usage(&state.pool, id)
        .await?
        .into_iter()
        .map(|row| DependencyUsageResponse {
            schedule_id: row.schedule_id,
            schedule_name: row.schedule_name,
            agent_id: row.agent_id,
            hostname: row.hostname,
            source: row.source.into(),
        })
        .collect();
    Ok(Json(usage))
}

/// Every (agent, dependency) of one schedule - what it sets itself, what its
/// agents' defaults require, and what the repository hosts it writes to
/// require - plus the runs waiting on one.
async fn schedule_dependencies_response(
    state: &AppState,
    schedule_id: i64,
) -> Result<ScheduleDependenciesResponse, ApiError> {
    let agent_ids: Vec<i64> = db::list_schedule_targets(&state.pool, schedule_id)
        .await?
        .into_iter()
        .map(|t| t.agent_id)
        .collect();
    let defaults = db::dependency_hosts::list_all_agent_default_dependencies(&state.pool).await?;
    let from_repo_hosts =
        db::dependency_hosts::list_schedule_repo_host_dependencies(&state.pool, schedule_id)
            .await?;
    let set_here =
        db::dependency_hosts::list_schedule_dependencies(&state.pool, schedule_id).await?;
    let hosts: HashMap<i64, DependencyHostRow> =
        db::dependency_hosts::list_dependency_hosts(&state.pool)
            .await?
            .into_iter()
            .map(|s| (s.host.id, s.host))
            .collect();
    let mut repo_hosts: HashMap<i64, DependencyRepoHostRef> = HashMap::new();
    for (repo_host_id, _) in &from_repo_hosts {
        if repo_hosts.contains_key(repo_host_id) {
            continue;
        }
        let host = db::repo_hosts::get_repo_host(&state.pool, *repo_host_id).await?;
        repo_hosts.insert(
            host.id,
            DependencyRepoHostRef {
                id: host.id,
                ssh_host: host.ssh_host,
            },
        );
    }

    // Inherited first, so a pair both set here and inherited reads as the one
    // it cannot be removed as. A repository host's dependencies apply to every
    // agent of the schedule.
    let mut seen = BTreeSet::new();
    let mut dependencies = Vec::new();
    let from_agents = defaults
        .iter()
        .filter(|(agent_id, _)| agent_ids.contains(agent_id))
        .map(|pair| (*pair, DependencySourceKind::AgentDefault, None));
    let from_hosts = agent_ids.iter().flat_map(|agent_id| {
        from_repo_hosts
            .iter()
            .map(move |(repo_host_id, dependency_host_id)| {
                (
                    (*agent_id, *dependency_host_id),
                    DependencySourceKind::RepoHost,
                    Some(*repo_host_id),
                )
            })
    });
    let own = set_here
        .iter()
        .filter(|(agent_id, _)| agent_ids.contains(agent_id))
        .map(|pair| (*pair, DependencySourceKind::Schedule, None));
    for ((agent_id, dependency_host_id), source, repo_host_id) in
        from_agents.chain(from_hosts).chain(own)
    {
        if !seen.insert((agent_id, dependency_host_id)) {
            continue;
        }
        let Some(host) = hosts.get(&dependency_host_id) else {
            continue;
        };
        dependencies.push(ScheduleDependencyResponse {
            agent_id,
            dependency_host_id,
            dependency_name: host.name.clone(),
            source,
            repo_host: repo_host_id.and_then(|id| repo_hosts.get(&id).cloned()),
            last_check_reachable: host.last_check_reachable,
        });
    }
    dependencies.sort_by(|a, b| {
        let order = |id: i64| agent_ids.iter().position(|a| *a == id);
        order(a.agent_id)
            .cmp(&order(b.agent_id))
            .then_with(|| a.dependency_name.cmp(&b.dependency_name))
    });
    Ok(ScheduleDependenciesResponse {
        dependencies,
        waiting: crate::dependencies::catch_up::waiting_for_schedule(state, schedule_id).await?,
    })
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/dependencies",
    tag = "Schedules",
    operation_id = "getScheduleDependencies",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Dependencies per agent", body = ScheduleDependenciesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// The dependency hosts each agent of a schedule needs, and the runs waiting
/// on one.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the schedule does not exist.
pub async fn get_schedule_dependencies(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScheduleDependenciesResponse>, ApiError> {
    db::get_schedule_by_id(&state.pool, id).await?;
    Ok(Json(schedule_dependencies_response(&state, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/schedules/{id}/dependencies",
    tag = "Schedules",
    operation_id = "updateScheduleDependencies",
    params(("id" = i64, Path, description = "Schedule ID")),
    request_body = UpdateScheduleDependenciesRequest,
    responses(
        (status = 200, description = "Dependencies per agent", body = ScheduleDependenciesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Replace the dependency hosts a schedule sets for its agents. Those an
/// agent's backup defaults require apply on top and need not be listed.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller may not edit the schedule,
/// or [`ApiError::BadRequest`] for an agent that is not one of its targets or
/// an unknown dependency.
pub async fn update_schedule_dependencies(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateScheduleDependenciesRequest>,
) -> Result<Json<ScheduleDependenciesResponse>, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    super::schedules::check_schedule_edit_permission(&state, &auth, &schedule).await?;
    let agent_ids: Vec<i64> = db::list_schedule_targets(&state.pool, id)
        .await?
        .into_iter()
        .map(|t| t.agent_id)
        .collect();
    if let Some(stray) = req
        .dependencies
        .iter()
        .find(|d| !agent_ids.contains(&d.agent_id))
    {
        return Err(ApiError::BadRequest(format!(
            "agent {} is not a target of this schedule",
            stray.agent_id
        )));
    }
    let pairs: Vec<(i64, i64)> = req
        .dependencies
        .iter()
        .map(|d| (d.agent_id, d.dependency_host_id))
        .collect();
    db::dependency_hosts::replace_schedule_dependencies(&state.pool, id, &pairs).await?;
    data_changed(&state);
    Ok(Json(schedule_dependencies_response(&state, id).await?))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/dependencies",
    tag = "Agents",
    operation_id = "getAgentDependencies",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Default dependencies", body = AgentDependenciesBody),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// The dependency hosts an agent's backup defaults require of every schedule
/// on it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the agent does not exist.
pub async fn get_agent_dependencies(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<AgentDependenciesBody>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    Ok(Json(AgentDependenciesBody {
        dependency_host_ids: db::dependency_hosts::list_agent_default_dependencies(
            &state.pool,
            agent.id,
        )
        .await?,
    }))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/dependencies",
    tag = "Agents",
    operation_id = "updateAgentDependencies",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = AgentDependenciesBody,
    responses(
        (status = 200, description = "Default dependencies", body = AgentDependenciesBody),
        (status = 400, description = "Unknown dependency"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Replace the dependency hosts an agent's backup defaults require (admin
/// only, like the rest of the agent's backup defaults).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an unknown dependency, or
/// [`ApiError::NotFound`] if the agent does not exist.
pub async fn update_agent_dependencies(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<AgentDependenciesBody>,
) -> Result<Json<AgentDependenciesBody>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    db::dependency_hosts::replace_agent_default_dependencies(
        &state.pool,
        agent.id,
        &req.dependency_host_ids,
    )
    .await?;
    data_changed(&state);
    Ok(Json(AgentDependenciesBody {
        dependency_host_ids: db::dependency_hosts::list_agent_default_dependencies(
            &state.pool,
            agent.id,
        )
        .await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_bounded() {
        assert_eq!(validate_name("  nas-media ").unwrap(), "nas-media");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn an_address_is_a_bare_hostname_or_ip() {
        assert_eq!(
            validate_address(" nas-media.lan ").unwrap(),
            "nas-media.lan"
        );
        assert_eq!(validate_address("10.0.20.14").unwrap(), "10.0.20.14");
        for bad in ["", "smb://nas", "nas media", "user@nas", "nas/share"] {
            assert!(validate_address(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn a_port_must_fit_the_tcp_range() {
        assert!(validate_port(445).is_ok());
        assert!(validate_port(0).is_err());
        assert!(validate_port(65_536).is_err());
    }

    fn power(wake_enabled: bool, mac: Option<&str>) -> shared::machines::UpdateMachinePowerRequest {
        shared::machines::UpdateMachinePowerRequest {
            wake_enabled,
            wake_mac_address: mac.map(str::to_owned),
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
        }
    }

    /// A dependency's machine is woken by the same rules as any other.
    #[test]
    fn waking_needs_a_well_formed_mac() {
        use crate::api::agents::validate_host_wake as validate_power;
        assert!(validate_power(&power(true, Some("9C:B6:D0:1A:44:7F"))).is_ok());
        assert!(validate_power(&power(true, None)).is_err());
        assert!(validate_power(&power(false, Some("not-a-mac"))).is_err());
        assert!(validate_power(&power(false, None)).is_ok());
    }

    #[test]
    fn a_broadcast_address_must_be_ipv4_and_the_timeout_positive() {
        use crate::api::agents::validate_host_wake as validate_power;
        let mut req = power(true, Some("9C:B6:D0:1A:44:7F"));
        req.wake_broadcast_address = Some("ff02::1".to_owned());
        assert!(validate_power(&req).is_err());
        let mut req = power(false, None);
        req.wake_timeout_seconds = 0;
        assert!(validate_power(&req).is_err());
    }

    #[test]
    fn a_long_description_is_refused() {
        assert!(validate_description("Media share").is_ok());
        assert!(validate_description(&"x".repeat(MAX_DESCRIPTION_LEN + 1)).is_err());
    }
}
