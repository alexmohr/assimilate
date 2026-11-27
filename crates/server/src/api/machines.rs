// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Machines: how an agent's, a repository host's or a dependency host's box
//! is woken and shut down, whether it is always online, and what waits on it -
//! the Power & dependencies pane every one of them shows.
//!
//! Anyone signed in may read a machine, as they could read the power and
//! availability of its roles before; only a viewer allowed to see wake secrets
//! sees its MAC and broadcast addresses, and only an admin sees which
//! schedules override it, changes it, or asks it whether it is back.
//!
//! A viewer who is not an admin sees a machine only through a role they may
//! see (see [`super::read_scope`]): its agent if they see the agent, its
//! repository host if they read a repository on it, and each dependency on
//! it they may see. Only those roles are listed, with only the waits of the
//! schedules, agents and repositories they see. A machine with none reads as
//! not found. A schedule's machines leave out those of the agents and
//! repositories the viewer does not see.

use axum::{
    Json,
    extract::{Path, State},
};
use shared::{
    machines::{
        MachineAgentRef, MachineAvailabilityResponse, MachineCheckResponse, MachineDependencyRef,
        MachineRepoHostRef, MachineResponse, MachineRoleKind, MachineRolesResponse,
        MachineScheduleOverrideResponse, MachineWaitResponse, ScheduleMachinePowerResponse,
        ScheduleMachineResponse, ScheduleMachinesResponse, UpdateMachineAvailabilityRequest,
        UpdateMachinePowerRequest, UpdateScheduleMachinesRequest,
    },
    protocol::ServerToUi,
    responses::HostWakeSettingsResponse,
    types::MachineWakeMode,
};

use super::{
    agents::validate_host_wake,
    auth::{AuthUser, RequireAdmin},
    availability::{validate_give_up_minutes, validate_recheck_minutes},
    read_scope::{self, ReadScope},
};
use crate::{
    AppState,
    catch_up::give_up_deadline,
    db::{
        self,
        machines::{
            MachineAvailabilityPatch, MachinePowerPatch, MachineRoles, MachineRow,
            ScheduleMachineWake,
        },
    },
    error::{ApiError, ApiJson},
};

/// A machine's wake settings as a response, with the addresses left out for a
/// viewer who may not see them.
pub(crate) fn wake_settings_response(
    machine: &MachineRow,
    show_secrets: bool,
) -> HostWakeSettingsResponse {
    let secret = |value: &Option<String>| value.clone().filter(|_| show_secrets);
    HostWakeSettingsResponse {
        wake_enabled: machine.wake_enabled,
        wake_mac_address: secret(&machine.wake_mac_address),
        wake_broadcast_address: secret(&machine.wake_broadcast_address),
        wake_timeout_seconds: machine.wake_timeout_seconds,
        shutdown_after_backup: machine.shutdown_after_backup,
    }
}

fn roles_response(roles: &MachineRoles) -> MachineRolesResponse {
    MachineRolesResponse {
        agent: roles.agent.as_ref().map(|a| MachineAgentRef {
            id: a.id,
            hostname: a.hostname.clone(),
            domain: a.domain.clone(),
        }),
        repo_host: roles.repo_host.as_ref().map(|h| MachineRepoHostRef {
            id: h.id,
            ssh_host: h.ssh_host.clone(),
        }),
        dependencies: roles
            .dependencies
            .iter()
            .map(|d| MachineDependencyRef {
                id: d.id,
                name: d.name.clone(),
            })
            .collect(),
    }
}

/// Every run waiting on any role of a machine, agents first, then the
/// repository host, then dependencies. A machine nothing waits for - not
/// marked, and no schedule set to wait for it - lists nothing for its agent:
/// switching the mark off already drops those waits.
async fn waiting(
    state: &AppState,
    machine: &MachineRow,
    roles: &MachineRoles,
    scope: &WaitScope,
) -> Result<Vec<MachineWaitResponse>, ApiError> {
    let mut out = Vec::new();
    if let Some(agent) = roles.agent.as_ref().filter(|_| machine.intermittent) {
        out.extend(
            db::catch_up::list_agent_catch_up_waits(&state.pool, agent.id)
                .await?
                .into_iter()
                .map(|wait| MachineWaitResponse {
                    role: MachineRoleKind::Agent,
                    schedule_id: wait.schedule_id,
                    schedule_name: wait.schedule_name,
                    hostname: Some(agent.hostname.clone()),
                    repo_id: None,
                    repo_name: None,
                    dependency_name: None,
                    pending_for: wait.pending_for,
                    last_probe_at: None,
                    next_probe_at: None,
                    give_up_at: give_up_deadline(
                        wait.pending_for,
                        machine.catch_up_give_up_minutes,
                    ),
                }),
        );
    }
    if let Some(host) = &roles.repo_host {
        out.extend(
            crate::repo_catch_up::waiting_for_host(state, host.id)
                .await?
                .into_iter()
                .map(|wait| MachineWaitResponse {
                    role: MachineRoleKind::RepoHost,
                    schedule_id: wait.schedule_id,
                    schedule_name: wait.schedule_name,
                    hostname: None,
                    repo_id: wait.repo_id,
                    repo_name: wait.repo_name,
                    dependency_name: None,
                    pending_for: wait.pending_for,
                    last_probe_at: wait.last_probe_at,
                    next_probe_at: wait.next_probe_at,
                    give_up_at: wait.give_up_at,
                }),
        );
    }
    for dependency in &roles.dependencies {
        out.extend(
            crate::dependencies::catch_up::waiting_for_dependency(state, dependency.id)
                .await?
                .into_iter()
                .filter(|wait| scope.agents.allows(wait.agent_id))
                .map(|wait| MachineWaitResponse {
                    role: MachineRoleKind::Dependency,
                    schedule_id: wait.schedule_id,
                    schedule_name: wait.schedule_name,
                    hostname: Some(wait.hostname),
                    repo_id: None,
                    repo_name: None,
                    dependency_name: Some(wait.dependency_name),
                    pending_for: wait.pending_for,
                    last_probe_at: wait.last_probe_at,
                    next_probe_at: wait.next_probe_at,
                    give_up_at: wait.give_up_at,
                }),
        );
    }
    out.retain(|wait| scope.allows(wait));
    Ok(out)
}

/// The schedules, agents and repositories whose waits on a machine a viewer
/// sees.
struct WaitScope {
    schedules: ReadScope,
    agents: ReadScope,
    repos: ReadScope,
}

impl WaitScope {
    /// Whether a wait on the machine, of `schedule_id` into `repo_id` if it
    /// names one, is the viewer's to see.
    fn allows(&self, wait: &MachineWaitResponse) -> bool {
        self.schedules.allows(wait.schedule_id)
            && wait
                .repo_id
                .is_none_or(|repo_id| self.repos.allows(repo_id))
    }
}

/// Leaves out of `roles` what `viewer` may not see - an agent they do not
/// see, a repository host they read no repository on, a dependency they may
/// not see - and returns what decides which waits on the rest they see. A
/// machine with no role left for them reads as not found.
async fn visible_roles(
    state: &AppState,
    machine_id: i64,
    mut roles: MachineRoles,
    viewer: &AuthUser,
) -> Result<(MachineRoles, WaitScope), ApiError> {
    let scope = WaitScope {
        schedules: read_scope::visible_schedules(&state.pool, viewer.user_id).await?,
        agents: read_scope::visible_agents(&state.pool, viewer.user_id).await?,
        repos: read_scope::readable_repos(&state.pool, viewer.user_id).await?,
    };
    let dependencies = read_scope::dependency_host_scope(&state.pool, viewer.user_id).await?;
    if matches!(dependencies, read_scope::DependencyHostScope::All) {
        return Ok((roles, scope));
    }
    let repo_hosts = read_scope::visible_repo_hosts(&state.pool, viewer.user_id).await?;
    roles.agent = roles.agent.filter(|a| scope.agents.allows(a.id));
    roles.repo_host = roles.repo_host.filter(|h| repo_hosts.allows(h.id));
    roles.dependencies.retain(|d| dependencies.allows(d.id));
    if roles.agent.is_none() && roles.repo_host.is_none() && roles.dependencies.is_empty() {
        return Err(ApiError::NotFound(format!(
            "machine {machine_id} not found"
        )));
    }
    Ok((roles, scope))
}

/// Builds the response for one machine, for `viewer` - its permissions decide
/// whether the wake addresses and the schedules that override it are shown,
/// and which of its dependencies are.
async fn machine_response(
    state: &AppState,
    machine_id: i64,
    viewer: &AuthUser,
) -> Result<MachineResponse, ApiError> {
    let machine = db::machines::get_machine(&state.pool, machine_id).await?;
    let roles = db::machines::get_machine_roles(&state.pool, machine_id).await?;
    let can_shut_down = roles.agent.is_some() || roles.repo_host.is_some();
    let (roles, wait_scope) = visible_roles(state, machine_id, roles, viewer).await?;
    let permissions = db::get_effective_permissions(&state.pool, viewer.user_id).await?;
    let schedule_overrides = if permissions.can_delete_repo {
        db::machines::list_machine_schedule_overrides(&state.pool, machine_id)
            .await?
            .into_iter()
            .map(|o| MachineScheduleOverrideResponse {
                schedule_id: o.schedule_id,
                schedule_name: o.schedule_name,
                wake_override: o.wake_override,
                mode: o.mode,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(MachineResponse {
        id: machine.id,
        label: roles.label(),
        roles: roles_response(&roles),
        power: wake_settings_response(&machine, permissions.can_view_wake_secrets()),
        can_shut_down,
        availability: MachineAvailabilityResponse {
            intermittent: machine.intermittent,
            catch_up_recheck_minutes: machine.catch_up_recheck_minutes,
            catch_up_give_up_minutes: machine.catch_up_give_up_minutes,
        },
        last_checked_at: machine.last_checked_at,
        last_check_reachable: machine.last_check_reachable,
        waiting: waiting(state, &machine, &roles, &wait_scope).await?,
        schedule_overrides,
    })
}

#[utoipa::path(
    get,
    path = "/api/machines/{machine_id}",
    tag = "Machines",
    operation_id = "getMachine",
    params(("machine_id" = i64, Path, description = "Machine ID")),
    responses(
        (status = 200, description = "Machine", body = MachineResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// A machine: its roles, how it is woken and shut down, whether it is always
/// online, and what waits on it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn get_machine(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<i64>,
) -> Result<Json<MachineResponse>, ApiError> {
    Ok(Json(machine_response(&state, machine_id, &auth).await?))
}

/// What a machine's roles add to its wake settings' own rules: only an agent's
/// or a repository host's machine can be shut down, since a dependency is
/// never shut down for its own sake, and an agent's is shut down over the
/// agent's SSH destination, which has to be set and authorized.
fn validate_shutdown(roles: &MachineRoles, shutdown_after_backup: bool) -> Result<(), ApiError> {
    if !shutdown_after_backup {
        return Ok(());
    }
    if roles.agent.is_none() && roles.repo_host.is_none() {
        return Err(ApiError::BadRequest(
            "a machine that is only a dependency is never shut down".to_owned(),
        ));
    }
    if let Some(agent) = &roles.agent {
        if agent.ssh_host.as_deref().is_none_or(str::is_empty) {
            return Err(ApiError::BadRequest(
                "set the agent's SSH host before shutting its machine down after backup".to_owned(),
            ));
        }
        if agent.last_ssh_user.is_none() {
            return Err(ApiError::BadRequest(
                "deploy the SSH key to this host (Deploy SSH key) before enabling shutdown after \
                 backup -- no SSH user on record for it yet"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

#[utoipa::path(
    put,
    path = "/api/machines/{machine_id}/power",
    tag = "Machines",
    operation_id = "updateMachinePower",
    params(("machine_id" = i64, Path, description = "Machine ID")),
    request_body = UpdateMachinePowerRequest,
    responses(
        (status = 200, description = "Updated machine", body = MachineResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Change how a machine is woken and shut down (admin only). It applies to
/// every role the machine plays.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for invalid wake settings or a shutdown
/// the machine's roles do not allow, or [`ApiError::NotFound`].
pub async fn update_machine_power(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(machine_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateMachinePowerRequest>,
) -> Result<Json<MachineResponse>, ApiError> {
    validate_host_wake(&req)?;
    db::machines::get_machine(&state.pool, machine_id).await?;
    let roles = db::machines::get_machine_roles(&state.pool, machine_id).await?;
    validate_shutdown(&roles, req.shutdown_after_backup)?;
    db::machines::update_machine_power(
        &state.pool,
        machine_id,
        &MachinePowerPatch {
            wake_enabled: req.wake_enabled,
            wake_mac_address: req.wake_mac_address.as_deref(),
            wake_broadcast_address: req.wake_broadcast_address.as_deref(),
            wake_timeout_seconds: req.wake_timeout_seconds,
            shutdown_after_backup: req.shutdown_after_backup,
        },
    )
    .await?;
    state.ui_broadcast.send(ServerToUi::DataChanged);
    Ok(Json(machine_response(&state, machine_id, &admin).await?))
}

#[utoipa::path(
    put,
    path = "/api/machines/{machine_id}/availability",
    tag = "Machines",
    operation_id = "updateMachineAvailability",
    params(("machine_id" = i64, Path, description = "Machine ID")),
    request_body = UpdateMachineAvailabilityRequest,
    responses(
        (status = 200, description = "Updated machine", body = MachineResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Change whether a machine is expected to be online (admin only).
///
/// Marking it as always online again drops what was waiting on it because of
/// the mark: nothing is waiting for it any more, and a miss recorded while it
/// was marked must not run days later because somebody marked it again. A
/// schedule set to wait for the machine itself keeps waiting.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an out-of-range value, or
/// [`ApiError::NotFound`].
pub async fn update_machine_availability(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(machine_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateMachineAvailabilityRequest>,
) -> Result<Json<MachineResponse>, ApiError> {
    let recheck_minutes = validate_recheck_minutes(req.catch_up_recheck_minutes)?;
    db::machines::get_machine(&state.pool, machine_id).await?;
    let roles = db::machines::get_machine_roles(&state.pool, machine_id).await?;
    // Only a repository host or a dependency is asked whether it is back; an
    // agent reconnects on its own, so there is no interval to leave room for.
    let asked = roles.repo_host.is_some() || !roles.dependencies.is_empty();
    let give_up_minutes = validate_give_up_minutes(
        req.catch_up_give_up_minutes,
        asked.then_some(recheck_minutes),
    )?;
    let machine = db::machines::update_machine_availability(
        &state.pool,
        machine_id,
        MachineAvailabilityPatch {
            intermittent: req.intermittent,
            catch_up_recheck_minutes: recheck_minutes,
            catch_up_give_up_minutes: give_up_minutes,
        },
    )
    .await?;
    if !machine.intermittent {
        if let Some(agent) = &roles.agent {
            db::catch_up::clear_catch_up_pending(&state.pool, agent.id).await?;
        }
        db::catch_up::clear_repo_catch_up_pending_for_machine(&state.pool, machine_id).await?;
        db::dependency_catch_ups::clear_dependency_catch_ups_for_machine(&state.pool, machine_id)
            .await?;
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
    Ok(Json(machine_response(&state, machine_id, &admin).await?))
}

#[utoipa::path(
    post,
    path = "/api/machines/{machine_id}/check",
    tag = "Machines",
    operation_id = "checkMachineNow",
    params(("machine_id" = i64, Path, description = "Machine ID")),
    responses(
        (status = 200, description = "What the check did", body = MachineCheckResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Ask a machine whether it is back now, without waking it, and catch up every
/// run waiting on it if it is (admin only - it can start backups).
///
/// A repository host is asked over SSH and a dependency on its port, each
/// once; an agent is back when it is connected, and catches its own runs up
/// when it reconnects.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist.
pub async fn check_machine_now(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(machine_id): Path<i64>,
) -> Result<Json<MachineCheckResponse>, ApiError> {
    db::machines::get_machine(&state.pool, machine_id).await?;
    let roles = db::machines::get_machine_roles(&state.pool, machine_id).await?;
    let mut response = MachineCheckResponse {
        reachable: false,
        started: 0,
        abandoned: 0,
        dropped: 0,
    };
    let mut add = |reachable: bool, started: usize, abandoned: usize, dropped: usize| {
        response.reachable |= reachable;
        response.started = response.started.saturating_add(started);
        response.abandoned = response.abandoned.saturating_add(abandoned);
        response.dropped = response.dropped.saturating_add(dropped);
    };
    if let Some(agent) = &roles.agent {
        add(state.registry.is_connected(agent.id).await, 0, 0, 0);
    }
    if let Some(host) = &roles.repo_host {
        let outcome = crate::repo_catch_up::check_host_now(&state, host.id).await?;
        // Nothing waiting means nothing was asked; ask anyway, so the button
        // answers the question it is labelled with.
        let reachable = if outcome.probed > 0 {
            outcome.reachable > 0
        } else {
            crate::repo_catch_up::probe_host(&state, host.id).await?
        };
        add(
            reachable,
            outcome.started,
            outcome.abandoned,
            outcome.dropped,
        );
    }
    for dependency in &roles.dependencies {
        let outcome =
            crate::dependencies::catch_up::check_dependency_now(&state, dependency.id).await?;
        let reachable = if outcome.probed > 0 {
            outcome.reachable > 0
        } else {
            let required =
                db::dependency_hosts::get_required_dependency(&state.pool, dependency.id).await?;
            crate::dependencies::probe_and_record(&state.pool, &required).await
        };
        add(
            reachable,
            outcome.started,
            outcome.abandoned,
            outcome.dropped,
        );
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
    Ok(Json(response))
}

/// What a schedule needs a machine as, accumulated over its roles.
struct ScheduleMachine {
    machine_id: i64,
    label: String,
    roles: Vec<MachineRoleKind>,
}

/// The agents and repositories whose machines a viewer sees among a
/// schedule's.
struct ScheduleMachineScope {
    agents: ReadScope,
    repos: ReadScope,
}

impl ScheduleMachineScope {
    /// Every agent and repository, for the schedule's machines as a run needs
    /// them.
    const fn everything() -> Self {
        Self {
            agents: ReadScope::All,
            repos: ReadScope::All,
        }
    }

    async fn load(state: &AppState, viewer: &AuthUser) -> Result<Self, ApiError> {
        Ok(Self {
            agents: read_scope::visible_agents(&state.pool, viewer.user_id).await?,
            repos: read_scope::readable_repos(&state.pool, viewer.user_id).await?,
        })
    }
}

/// Every machine a schedule needs, in the order a run meets them: its agents,
/// the repository hosts it writes to, then the dependencies its agents need -
/// of the agents and repositories `scope` allows.
async fn schedule_machines(
    state: &AppState,
    schedule_id: i64,
    scope: &ScheduleMachineScope,
) -> Result<Vec<ScheduleMachine>, ApiError> {
    let mut machines: Vec<ScheduleMachine> = Vec::new();
    let mut add = |machine_id: i64, label: String, role: MachineRoleKind| {
        if let Some(existing) = machines.iter_mut().find(|m| m.machine_id == machine_id) {
            if !existing.roles.contains(&role) {
                existing.roles.push(role);
            }
        } else {
            machines.push(ScheduleMachine {
                machine_id,
                label,
                roles: vec![role],
            });
        }
    };
    let targets = db::list_schedule_targets(&state.pool, schedule_id).await?;
    let mut agents = Vec::with_capacity(targets.len());
    for target in targets.iter().filter(|t| scope.agents.allows(t.agent_id)) {
        let agent = db::get_agent_by_id(&state.pool, target.agent_id).await?;
        add(
            agent.machine_id,
            agent.hostname.clone(),
            MachineRoleKind::Agent,
        );
        agents.push(agent.id);
    }
    for schedule_repo in db::list_schedule_repos(&state.pool, schedule_id)
        .await?
        .into_iter()
        .filter(|r| scope.repos.allows(r.repo_id))
    {
        let repo = db::get_repo_by_id(&state.pool, schedule_repo.repo_id).await?;
        add(repo.machine_id, repo.ssh_host, MachineRoleKind::RepoHost);
    }
    for agent_id in agents {
        for dependency in
            db::dependency_hosts::list_required_dependencies(&state.pool, schedule_id, agent_id)
                .await?
        {
            add(
                dependency.machine_id,
                dependency.name,
                MachineRoleKind::Dependency,
            );
        }
    }
    Ok(machines)
}

async fn schedule_machines_response(
    state: &AppState,
    schedule_id: i64,
    viewer: &AuthUser,
) -> Result<ScheduleMachinesResponse, ApiError> {
    let show_secrets = db::get_effective_permissions(&state.pool, viewer.user_id)
        .await?
        .can_view_wake_secrets();
    let choices = db::machines::list_schedule_machine_wake(&state.pool, schedule_id).await?;
    let mut out = Vec::new();
    let scope = ScheduleMachineScope::load(state, viewer).await?;
    for machine in schedule_machines(state, schedule_id, &scope).await? {
        let row = db::machines::get_machine(&state.pool, machine.machine_id).await?;
        out.push(ScheduleMachineResponse {
            machine_id: machine.machine_id,
            label: machine.label,
            can_wait: !machine.roles.contains(&MachineRoleKind::Agent),
            roles: machine.roles,
            power: ScheduleMachinePowerResponse {
                wake_enabled: row.wake_enabled,
                has_mac_address: !show_secrets || row.wake_mac_address.is_some(),
                shutdown_after_backup: row.shutdown_after_backup,
            },
            intermittent: row.intermittent,
            mode: choices
                .iter()
                .find(|c| c.machine_id == row.id)
                .map(|c| c.mode),
        });
    }
    Ok(ScheduleMachinesResponse { machines: out })
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/machines",
    tag = "Schedules",
    operation_id = "getScheduleMachines",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Machines it needs", body = ScheduleMachinesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Every machine a schedule needs - its agents, repository hosts and
/// dependencies - with their own settings and the schedule's choice for each,
/// leaving out those of the agents and repositories the caller does not see.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the schedule does not exist.
pub async fn get_schedule_machines(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScheduleMachinesResponse>, ApiError> {
    super::permissions::get_visible_schedule(&state.pool, &auth, id).await?;
    Ok(Json(schedule_machines_response(&state, id, &auth).await?))
}

#[utoipa::path(
    put,
    path = "/api/schedules/{id}/machines",
    tag = "Schedules",
    operation_id = "updateScheduleMachines",
    params(("id" = i64, Path, description = "Schedule ID")),
    request_body = UpdateScheduleMachinesRequest,
    responses(
        (status = 200, description = "Machines it needs", body = ScheduleMachinesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Replace the schedule's own choice for each machine it needs that the caller
/// sees. A machine left out follows the schedule-wide wake setting; the
/// choices for machines of agents and repositories the caller does not see
/// are kept.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller may not edit the schedule,
/// or [`ApiError::BadRequest`] for a machine the schedule does not need, a
/// machine listed twice, or waiting for an agent's machine.
pub async fn update_schedule_machines(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateScheduleMachinesRequest>,
) -> Result<Json<ScheduleMachinesResponse>, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    super::schedules::check_schedule_edit_permission(&state, &auth, &schedule).await?;
    let needed = schedule_machines(
        &state,
        id,
        &ScheduleMachineScope::load(&state, &auth).await?,
    )
    .await?;
    let mut choices: Vec<ScheduleMachineWake> = Vec::with_capacity(req.machines.len());
    for input in &req.machines {
        let Some(machine) = needed.iter().find(|m| m.machine_id == input.machine_id) else {
            return Err(ApiError::BadRequest(format!(
                "machine {} is not one this schedule needs",
                input.machine_id
            )));
        };
        if choices.iter().any(|c| c.machine_id == input.machine_id) {
            return Err(ApiError::BadRequest(format!(
                "machine {} is listed more than once",
                input.machine_id
            )));
        }
        if input.mode == MachineWakeMode::Wait && machine.roles.contains(&MachineRoleKind::Agent) {
            return Err(ApiError::BadRequest(format!(
                "{} runs an agent, which cannot be waited for: an agent that is not connected has \
                 nothing to run the backup - mark the machine as not always online instead",
                machine.label
            )));
        }
        choices.push(ScheduleMachineWake {
            machine_id: input.machine_id,
            mode: input.mode,
        });
    }
    // The choices for the machines of agents and repositories the caller does
    // not see stay as they are.
    let unseen: Vec<i64> = schedule_machines(&state, id, &ScheduleMachineScope::everything())
        .await?
        .into_iter()
        .map(|m| m.machine_id)
        .filter(|machine_id| !needed.iter().any(|m| m.machine_id == *machine_id))
        .collect();
    choices.extend(
        db::machines::list_schedule_machine_wake(&state.pool, id)
            .await?
            .into_iter()
            .filter(|c| unseen.contains(&c.machine_id)),
    );
    db::machines::replace_schedule_machine_wake(&state.pool, id, &choices).await?;
    state.ui_broadcast.send(ServerToUi::DataChanged);
    Ok(Json(schedule_machines_response(&state, id, &auth).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::machines::{MachineAgentRole, MachineDependencyRole, MachineRepoHostRole};

    fn agent(ssh_host: Option<&str>, last_ssh_user: Option<&str>) -> MachineAgentRole {
        MachineAgentRole {
            id: 1,
            hostname: "web-01".to_owned(),
            domain: None,
            ssh_host: ssh_host.map(str::to_owned),
            last_ssh_user: last_ssh_user.map(str::to_owned),
        }
    }

    #[test]
    fn a_dependency_only_machine_is_never_shut_down() {
        let roles = MachineRoles {
            dependencies: vec![MachineDependencyRole {
                id: 1,
                name: "nas-media".to_owned(),
            }],
            ..MachineRoles::default()
        };
        assert!(validate_shutdown(&roles, false).is_ok());
        assert!(validate_shutdown(&roles, true).is_err());
    }

    #[test]
    fn a_repository_host_machine_may_be_shut_down() {
        let roles = MachineRoles {
            repo_host: Some(MachineRepoHostRole {
                id: 1,
                ssh_host: "nas".to_owned(),
            }),
            ..MachineRoles::default()
        };
        assert!(validate_shutdown(&roles, true).is_ok());
    }

    #[test]
    fn an_agent_machine_is_shut_down_over_a_deployed_ssh_destination() {
        let with = |agent| MachineRoles {
            agent: Some(agent),
            ..MachineRoles::default()
        };
        assert!(validate_shutdown(&with(agent(None, Some("root"))), true).is_err());
        assert!(validate_shutdown(&with(agent(Some("web-01"), None)), true).is_err());
        assert!(validate_shutdown(&with(agent(Some("web-01"), Some("root"))), true).is_ok());
        assert!(validate_shutdown(&with(agent(None, None)), false).is_ok());
    }

    #[test]
    fn wake_addresses_are_hidden_from_a_viewer_who_may_not_see_them() {
        let machine = MachineRow {
            id: 1,
            wake_enabled: true,
            wake_mac_address: Some("9C:B6:D0:1A:44:7F".to_owned()),
            wake_broadcast_address: Some("10.0.20.255".to_owned()),
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
            intermittent: false,
            catch_up_recheck_minutes: 15,
            catch_up_give_up_minutes: 0,
            last_checked_at: None,
            last_check_reachable: None,
        };
        let hidden = wake_settings_response(&machine, false);
        assert_eq!(hidden.wake_mac_address, None);
        assert_eq!(hidden.wake_broadcast_address, None);
        assert!(hidden.wake_enabled);
        let shown = wake_settings_response(&machine, true);
        assert_eq!(shown.wake_mac_address.as_deref(), Some("9C:B6:D0:1A:44:7F"));
    }
}
