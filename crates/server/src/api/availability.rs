// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Whether a host is expected to be reachable, and how long to wait for it
//! when it is not - the "When the host is offline" section of an agent's and a
//! repository's Power pane.
//!
//! A separate endpoint from the power settings beside it rather than more
//! fields on them: those carry their own validation (a MAC address to wake
//! with, an SSH user to shut down with), and marking a laptop as "not always
//! online" should not fail because its wake settings are incomplete.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use shared::responses::{CatchUpWaitResponse, HostAvailabilityResponse, RepoCatchUpCheckResponse};

use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::DomainQuery,
};
use crate::{
    AppState,
    catch_up::give_up_deadline,
    db::{
        self,
        catch_up::{AgentAvailabilityRow, RepoAvailabilityRow},
    },
    error::{ApiError, ApiJson},
};

/// Upper bound on the re-check interval: a week. Past that the probe is slower
/// than any schedule it could serve, so the catch-up would only ever be found
/// after a regular run had already covered the gap.
pub(crate) const MAX_CATCH_UP_RECHECK_MINUTES: i32 = 10_080;

/// Upper bound on the give-up window: thirty days. A backup nobody has managed
/// to take in a month is not waiting on a transient outage.
pub(crate) const MAX_CATCH_UP_GIVE_UP_MINUTES: i32 = 43_200;

/// Request payload for a repository's "when the host is offline" settings.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRepoAvailabilityRequest {
    /// Whether the host is marked as not always online. Off, an unreachable
    /// host is a failed backup; on, it is a skipped one that is caught up once
    /// the host answers again.
    pub intermittent: bool,
    /// How often, in minutes, the host is asked whether it is back while a
    /// catch-up waits on it (1-10080).
    pub catch_up_recheck_minutes: i32,
    /// How long, in minutes, the host is waited for before the run is
    /// abandoned and reported as failed. Zero waits indefinitely; anything
    /// else must be at least one re-check interval (up to 43200).
    pub catch_up_give_up_minutes: i32,
}

/// Request payload for an agent's "when the host is offline" settings. No
/// re-check interval: an agent announces its own return by reconnecting.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateAgentAvailabilityRequest {
    /// Whether the host is marked as not always online. Off, an agent that is
    /// not connected when its backup comes due is a failed backup; on, it is a
    /// skipped one that is caught up once the agent reconnects.
    pub intermittent: bool,
    /// How long, in minutes, the agent is waited for before the run is
    /// abandoned and reported as failed. Zero waits indefinitely (otherwise up
    /// to 43200).
    pub catch_up_give_up_minutes: i32,
}

fn validate_recheck_minutes(minutes: i32) -> Result<i32, ApiError> {
    if !(1..=MAX_CATCH_UP_RECHECK_MINUTES).contains(&minutes) {
        return Err(ApiError::BadRequest(format!(
            "catch_up_recheck_minutes must be between 1 and {MAX_CATCH_UP_RECHECK_MINUTES}"
        )));
    }
    Ok(minutes)
}

/// Zero means "wait indefinitely" and is always allowed. Any other value is
/// bounded, and - for a repository, which has to be asked - has to leave room
/// for at least one probe: a window shorter than the interval that fills it
/// would abandon every catch-up without ever having asked whether the host was
/// back, which from the outside looks like the feature silently not working.
fn validate_give_up_minutes(minutes: i32, recheck_minutes: Option<i32>) -> Result<i32, ApiError> {
    if minutes == 0 {
        return Ok(0);
    }
    if !(1..=MAX_CATCH_UP_GIVE_UP_MINUTES).contains(&minutes) {
        return Err(ApiError::BadRequest(format!(
            "catch_up_give_up_minutes must be 0 (wait indefinitely) or between 1 and \
             {MAX_CATCH_UP_GIVE_UP_MINUTES}"
        )));
    }
    if let Some(recheck) = recheck_minutes
        && minutes < recheck
    {
        return Err(ApiError::BadRequest(format!(
            "catch_up_give_up_minutes ({minutes}) must leave room for at least one re-check \
             ({recheck} minutes)"
        )));
    }
    Ok(minutes)
}

async fn repo_availability_response(
    state: &AppState,
    repo_id: i64,
    settings: RepoAvailabilityRow,
) -> Result<HostAvailabilityResponse, ApiError> {
    Ok(HostAvailabilityResponse {
        intermittent: settings.intermittent,
        catch_up_recheck_minutes: Some(settings.recheck_minutes),
        catch_up_give_up_minutes: settings.give_up_minutes,
        waiting: crate::repo_catch_up::waiting_for_repo(state, repo_id).await?,
    })
}

async fn agent_availability_response(
    state: &AppState,
    agent_id: i64,
    settings: AgentAvailabilityRow,
) -> Result<HostAvailabilityResponse, ApiError> {
    let waiting = db::catch_up::list_agent_catch_up_waits(&state.pool, agent_id)
        .await?
        .into_iter()
        .map(|wait| CatchUpWaitResponse {
            schedule_id: wait.schedule_id,
            schedule_name: wait.schedule_name,
            pending_for: wait.pending_for,
            last_probe_at: None,
            next_probe_at: None,
            give_up_at: give_up_deadline(wait.pending_for, settings.give_up_minutes),
        })
        .collect();
    Ok(HostAvailabilityResponse {
        intermittent: settings.intermittent,
        catch_up_recheck_minutes: None,
        catch_up_give_up_minutes: settings.give_up_minutes,
        waiting,
    })
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/availability",
    tag = "Repositories",
    operation_id = "getRepoAvailability",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Availability settings", body = HostAvailabilityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// A repository's "when the host is offline" settings, and the schedules
/// currently waiting on it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist.
pub async fn get_repo_availability(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<HostAvailabilityResponse>, ApiError> {
    let settings = db::catch_up::get_repo_availability(&state.pool, repo_id).await?;
    Ok(Json(
        repo_availability_response(&state, repo_id, settings).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/availability",
    tag = "Repositories",
    operation_id = "updateRepoAvailability",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = UpdateRepoAvailabilityRequest,
    responses(
        (status = 200, description = "Updated settings", body = HostAvailabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Update a repository's "when the host is offline" settings (admin only).
///
/// Switching the host back to "always online" drops whatever was waiting on
/// it: nothing is waiting any more, and a miss recorded while it was marked
/// must not run days later because somebody switched it on again.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an out-of-range value, or
/// [`ApiError::NotFound`] if the repository does not exist.
pub async fn update_repo_availability(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRepoAvailabilityRequest>,
) -> Result<Json<HostAvailabilityResponse>, ApiError> {
    let recheck_minutes = validate_recheck_minutes(req.catch_up_recheck_minutes)?;
    let give_up_minutes =
        validate_give_up_minutes(req.catch_up_give_up_minutes, Some(recheck_minutes))?;
    let settings = db::catch_up::update_repo_availability(
        &state.pool,
        repo_id,
        RepoAvailabilityRow {
            intermittent: req.intermittent,
            recheck_minutes,
            give_up_minutes,
        },
    )
    .await?;
    if !settings.intermittent {
        db::catch_up::clear_repo_catch_up_pending_for_repo(&state.pool, repo_id).await?;
    }
    Ok(Json(
        repo_availability_response(&state, repo_id, settings).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/availability/check",
    tag = "Repositories",
    operation_id = "checkRepoAvailabilityNow",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "What the check did", body = RepoCatchUpCheckResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Ask this repository's host whether it is back, without waiting for the next
/// scheduled re-check, and catch up every schedule waiting on it if it is
/// (admin only - it can start backups on every host that writes here).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist.
pub async fn check_repo_availability_now(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoCatchUpCheckResponse>, ApiError> {
    // Confirms the repository exists, so an unknown id is a 404 rather than a
    // successful check of nothing.
    db::catch_up::get_repo_availability(&state.pool, repo_id).await?;
    let outcome = crate::repo_catch_up::check_repo_now(&state, repo_id).await?;
    Ok(Json(RepoCatchUpCheckResponse {
        probed: outcome.probed,
        reachable: outcome.reachable,
        started: outcome.started,
        abandoned: outcome.abandoned,
        dropped: outcome.dropped,
    }))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/availability",
    tag = "Agents",
    operation_id = "getAgentAvailability",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "Availability settings", body = HostAvailabilityResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// An agent's "when the host is offline" settings, and the schedules currently
/// waiting for it to reconnect.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the agent does not exist.
pub async fn get_agent_availability(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<HostAvailabilityResponse>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let settings = db::catch_up::get_agent_availability(&state.pool, agent.id).await?;
    Ok(Json(
        agent_availability_response(&state, agent.id, settings).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/availability",
    tag = "Agents",
    operation_id = "updateAgentAvailability",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    request_body = UpdateAgentAvailabilityRequest,
    responses(
        (status = 200, description = "Updated settings", body = HostAvailabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// Update an agent's "when the host is offline" settings (admin only).
///
/// Switching the host back to "always online" drops whatever was waiting for
/// it to reconnect, for the same reason the repository endpoint does.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an out-of-range value, or
/// [`ApiError::NotFound`] if the agent does not exist.
pub async fn update_agent_availability(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
    ApiJson(req): ApiJson<UpdateAgentAvailabilityRequest>,
) -> Result<Json<HostAvailabilityResponse>, ApiError> {
    let give_up_minutes = validate_give_up_minutes(req.catch_up_give_up_minutes, None)?;
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let settings = db::catch_up::update_agent_availability(
        &state.pool,
        agent.id,
        AgentAvailabilityRow {
            intermittent: req.intermittent,
            give_up_minutes,
        },
    )
    .await?;
    if !settings.intermittent {
        db::catch_up::clear_catch_up_pending(&state.pool, agent.id).await?;
    }
    Ok(Json(
        agent_availability_response(&state, agent.id, settings).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recheck_interval_is_bounded_on_both_sides() {
        assert!(validate_recheck_minutes(0).is_err());
        assert_eq!(validate_recheck_minutes(1).unwrap(), 1);
        assert_eq!(
            validate_recheck_minutes(MAX_CATCH_UP_RECHECK_MINUTES).unwrap(),
            MAX_CATCH_UP_RECHECK_MINUTES
        );
        assert!(validate_recheck_minutes(MAX_CATCH_UP_RECHECK_MINUTES + 1).is_err());
    }

    /// Zero is how every host behaved before the window existed, so it has to
    /// stay valid whatever the interval is.
    #[test]
    fn zero_is_always_a_valid_give_up_window() {
        assert_eq!(validate_give_up_minutes(0, Some(60)).unwrap(), 0);
        assert_eq!(validate_give_up_minutes(0, None).unwrap(), 0);
    }

    #[test]
    fn a_repository_window_must_hold_at_least_one_recheck() {
        assert!(validate_give_up_minutes(10, Some(15)).is_err());
        assert_eq!(validate_give_up_minutes(15, Some(15)).unwrap(), 15);
    }

    /// An agent is never asked, so there is no interval to leave room for.
    #[test]
    fn an_agent_window_only_has_to_be_in_range() {
        assert_eq!(validate_give_up_minutes(1, None).unwrap(), 1);
        assert!(validate_give_up_minutes(-1, None).is_err());
        assert!(validate_give_up_minutes(MAX_CATCH_UP_GIVE_UP_MINUTES + 1, None).is_err());
    }
}
