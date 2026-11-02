// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Append-only repositories: turning it on and off for a repository's login,
//! checking it, and the maintenance that prunes and compacts such a
//! repository.
//!
//! Admin only throughout. Enforcement is the repository host's: see
//! [`crate::ssh::append_only`].

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use shared::append_only::{
    AppendOnlyCheckResponse, AppendOnlyKeyResponse, AppendOnlyLoginResponse,
    AppendOnlySetupFailure, AppendOnlySiblingResponse, AppendOnlyStatusResponse,
    MaintenanceRunResponse, MaintenanceSettingsResponse, MaintenanceStoppedResponse,
    PendingDeletionResponse, ReapplyAppendOnlyRequest, SetAppendOnlyRequest,
    UpdateMaintenanceRequest,
};

use super::{auth::RequireAdmin, helpers};
use crate::{
    AppState,
    db::{
        self,
        append_only::{DEFAULT_MAINTENANCE_CRON, LoginRepoRow},
    },
    error::{ApiError, ApiJson},
    ssh::{
        self,
        append_only::{Access, Login, SetupError},
        authorized_keys, keys,
    },
};

/// How many maintenance runs the status lists.
const RECENT_RUNS: i64 = 10;

/// A repository's SSH login and everything on it.
struct Context {
    repo_id: i64,
    ssh_host: String,
    ssh_port: u16,
    ssh_host_key: Option<String>,
    ssh_user: String,
    repo_host_id: i64,
    repos: Vec<LoginRepoRow>,
}

impl Context {
    async fn load(state: &AppState, repo_id: i64) -> Result<Self, ApiError> {
        let repo = db::get_repo_by_id(&state.pool, repo_id).await?;
        let host = db::repo_hosts::get_repo_host(&state.pool, repo.repo_host_id).await?;
        let repos = db::append_only::list_login_repos(&state.pool, host.id, &repo.ssh_user).await?;
        Ok(Self {
            repo_id,
            ssh_port: u16::try_from(host.ssh_port).map_err(|_| {
                ApiError::Internal(format!("ssh_port {} out of u16 range", host.ssh_port))
            })?,
            ssh_host: host.ssh_host,
            ssh_host_key: host.ssh_host_key,
            ssh_user: repo.ssh_user,
            repo_host_id: host.id,
            repos,
        })
    }

    fn login(&self) -> Login<'_> {
        Login {
            host: &self.ssh_host,
            user: &self.ssh_user,
            port: self.ssh_port,
            host_key: self.ssh_host_key.as_deref(),
        }
    }

    /// The path of every repository on the login, sorted so the forced
    /// command comes out the same every time.
    fn repo_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.repos.iter().map(|r| r.repo_path.clone()).collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

fn setup_failure(error: &SetupError) -> AppendOnlySetupFailure {
    AppendOnlySetupFailure {
        needs_password: matches!(error, SetupError::NeedsPassword),
        message: error.to_string(),
    }
}

fn key_response(key: &ssh_key::PublicKey, line: String) -> AppendOnlyKeyResponse {
    AppendOnlyKeyResponse {
        fingerprint: keys::fingerprint(key),
        authorized_keys_line: line,
    }
}

async fn status(
    state: &AppState,
    ctx: &Context,
    setup_failure: Option<AppendOnlySetupFailure>,
) -> Result<AppendOnlyStatusResponse, ApiError> {
    let login = db::append_only::get_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user).await?;
    let enabled = login.is_some();

    let backup = keys::read_backup_public_key()
        .await
        .map_err(|e| ApiError::Internal(format!("the server's SSH key is not usable: {e}")))?;
    let backup_line = authorized_keys::restricted_line(&backup, &ctx.repo_paths())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let maintenance = keys::read_maintenance_key()
        .await
        .map_err(|e| ApiError::Internal(format!("the maintenance key is not usable: {e}")))?
        .map(|key| {
            let public = key.public_key().clone();
            let line = authorized_keys::maintenance_line(&public);
            key_response(&public, line)
        });

    let check = login.as_ref().and_then(|row| {
        Some(AppendOnlyCheckResponse {
            checked_at: row.checked_at?,
            backup_key_ok: row.backup_key_ok.unwrap_or(false),
            maintenance_key_ok: row.maintenance_key_ok.unwrap_or(false),
            restriction_in_place: row.restriction_in_place.unwrap_or(false),
            error: row.check_error.clone(),
        })
    });

    let maintenance_settings = if enabled {
        maintenance_response(state, ctx.repo_id).await?
    } else {
        None
    };
    let pending = db::append_only::list_pending_deletions(&state.pool, ctx.repo_id).await?;
    let runs = db::append_only::list_recent_runs(&state.pool, ctx.repo_id, RECENT_RUNS).await?;

    Ok(AppendOnlyStatusResponse {
        enabled,
        login: AppendOnlyLoginResponse {
            ssh_host: ctx.ssh_host.clone(),
            ssh_user: ctx.ssh_user.clone(),
            sibling_repos: ctx
                .repos
                .iter()
                .filter(|r| r.id != ctx.repo_id)
                .map(|r| AppendOnlySiblingResponse {
                    id: r.id,
                    name: r.name.clone(),
                })
                .collect(),
        },
        installed_at: login.map(|row| row.installed_at),
        check,
        backup_key: key_response(&backup, backup_line),
        maintenance_key: maintenance,
        maintenance: maintenance_settings,
        pending_deletions: pending
            .into_iter()
            .map(|row| PendingDeletionResponse {
                archive_name: row.archive_name,
                reason: shared::append_only::DeletionReason::Manual,
                requested_by: row.requested_by,
                requested_at: row.requested_at,
            })
            .collect(),
        recent_runs: runs
            .into_iter()
            .map(|row| MaintenanceRunResponse {
                id: row.id,
                started_at: row.started_at,
                finished_at: row.finished_at,
                outcome: row.outcome,
                archives_removed: row.archives_removed,
                bytes_freed: row.bytes_freed,
                detail: row.detail,
            })
            .collect(),
        setup_failure,
    })
}

async fn maintenance_response(
    state: &AppState,
    repo_id: i64,
) -> Result<Option<MaintenanceSettingsResponse>, ApiError> {
    let Some(row) = db::append_only::get_maintenance(&state.pool, repo_id).await? else {
        return Ok(None);
    };
    let stopped = match row.stopped_at {
        Some(since) => Some(MaintenanceStoppedResponse {
            since,
            vanished_archives: db::append_only::list_vanished(&state.pool, repo_id).await?,
        }),
        None => None,
    };
    Ok(Some(MaintenanceSettingsResponse {
        cron_expression: row.cron_expression,
        check_before_prune: row.check_before_prune,
        next_run_at: row.next_run_at,
        last_run_at: row.last_run_at,
        stopped,
    }))
}

/// Makes the login's keys match what the host should hold and records the
/// outcome of trying both.
async fn check_login(state: &AppState, ctx: &Context) -> Result<(), ApiError> {
    let result = ssh::append_only::check(ctx.login(), &ctx.repo_paths()).await;
    db::append_only::record_check(
        &state.pool,
        ctx.repo_host_id,
        &ctx.ssh_user,
        db::append_only::CheckRecord {
            backup_key_ok: result.backup_key_ok,
            maintenance_key_ok: result.maintenance_key_ok,
            restriction_in_place: result.restriction_in_place,
            error: result.error.as_deref(),
        },
    )
    .await?;
    Ok(())
}

async fn changed(state: &AppState) {
    helpers::push_config_to_all_agents(state).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/append-only",
    tag = "Append-only",
    operation_id = "getAppendOnly",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Append-only status", body = AppendOnlyStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
    )
)]
/// The repository's append-only settings and state (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist.
pub async fn get_append_only(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    let ctx = Context::load(&state, repo_id).await?;
    Ok(Json(status(&state, &ctx, None).await?))
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/append-only",
    tag = "Append-only",
    operation_id = "setAppendOnly",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = SetAppendOnlyRequest,
    responses(
        (status = 200, description = "Append-only status, with the failure if it did not work",
            body = AppendOnlyStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
        (status = 409, description = "Archive deletions are still queued"),
    )
)]
/// Turn append-only on or off for the repository's SSH login (admin only).
///
/// Every repository that logs in the same way switches with it. The host's
/// `authorized_keys` is edited over SFTP; a failure comes back in the status,
/// not as an error, with whether the SSH password is needed.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] when turning it off while deletions are
/// queued, since nothing would carry them out any more.
pub async fn set_append_only(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<SetAppendOnlyRequest>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    let ctx = Context::load(&state, repo_id).await?;
    let access = req
        .password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map_or(Access::ServerKey, Access::Password);

    let failure = if req.enabled {
        enable(&state, &ctx, access).await?
    } else {
        disable(&state, &ctx, access).await?
    };
    changed(&state).await;
    Ok(Json(status(&state, &ctx, failure).await?))
}

async fn enable(
    state: &AppState,
    ctx: &Context,
    access: Access<'_>,
) -> Result<Option<AppendOnlySetupFailure>, ApiError> {
    if let Err(e) = ssh::append_only::enable(ctx.login(), &ctx.repo_paths(), access).await {
        tracing::warn!(repo_id = ctx.repo_id, error = %e, "append-only setup failed");
        return Ok(Some(setup_failure(&e)));
    }

    db::append_only::upsert_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user).await?;
    ssh::set_login_append_only(&ctx.ssh_host, &ctx.ssh_user, true);
    for repo in &ctx.repos {
        let next = crate::maintenance::next_cron_run(state, DEFAULT_MAINTENANCE_CRON).await;
        db::append_only::ensure_maintenance(&state.pool, repo.id, next).await?;
    }
    check_login(state, ctx).await?;
    tracing::info!(repo_id = ctx.repo_id, host = %ctx.ssh_host, "append-only turned on");
    Ok(None)
}

async fn disable(
    state: &AppState,
    ctx: &Context,
    access: Access<'_>,
) -> Result<Option<AppendOnlySetupFailure>, ApiError> {
    for repo in &ctx.repos {
        if !db::append_only::list_pending_deletions(&state.pool, repo.id)
            .await?
            .is_empty()
        {
            return Err(ApiError::Conflict(format!(
                "repository {} has archive deletions queued; run maintenance or cancel them \
                 first, because nothing would carry them out once append-only is off",
                repo.name
            )));
        }
    }

    if let Err(e) = ssh::append_only::disable(ctx.login(), access).await {
        tracing::warn!(repo_id = ctx.repo_id, error = %e, "removing append-only failed");
        return Ok(Some(setup_failure(&e)));
    }

    db::append_only::delete_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user).await?;
    ssh::set_login_append_only(&ctx.ssh_host, &ctx.ssh_user, false);
    for repo in &ctx.repos {
        db::append_only::clear_all_vanished(&state.pool, repo.id).await?;
        db::append_only::delete_maintenance(&state.pool, repo.id).await?;
    }
    tracing::info!(repo_id = ctx.repo_id, host = %ctx.ssh_host, "append-only turned off");
    Ok(None)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/append-only/check",
    tag = "Append-only",
    operation_id = "checkAppendOnly",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Append-only status after the check",
            body = AppendOnlyStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
    )
)]
/// Try both keys against the host and read its `authorized_keys` (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist.
pub async fn check_append_only(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    let ctx = Context::load(&state, repo_id).await?;
    if db::append_only::get_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user)
        .await?
        .is_some()
    {
        check_login(&state, &ctx).await?;
    }
    Ok(Json(status(&state, &ctx, None).await?))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/append-only/reapply",
    tag = "Append-only",
    operation_id = "reapplyAppendOnly",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = ReapplyAppendOnlyRequest,
    responses(
        (status = 200, description = "Append-only status, with the failure if it did not work",
            body = AppendOnlyStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
        (status = 409, description = "Append-only is not on for this repository"),
    )
)]
/// Rewrite the host's `authorized_keys` for the login's current repositories
/// (admin only).
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if append-only is off.
pub async fn reapply_append_only(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<ReapplyAppendOnlyRequest>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    let ctx = Context::load(&state, repo_id).await?;
    if db::append_only::get_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user)
        .await?
        .is_none()
    {
        return Err(ApiError::Conflict(
            "append-only is not turned on for this repository".to_owned(),
        ));
    }
    let access = req
        .password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map_or(Access::ServerKey, Access::Password);
    let failure = enable(&state, &ctx, access).await?;
    changed(&state).await;
    Ok(Json(status(&state, &ctx, failure).await?))
}

fn validate_maintenance(req: &UpdateMaintenanceRequest) -> Result<(), ApiError> {
    helpers::validate_non_empty(&req.cron_expression, "cron_expression")?;
    shared::schedule::validate_cron(&req.cron_expression).map_err(ApiError::BadRequest)
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/append-only/maintenance",
    tag = "Append-only",
    operation_id = "updateMaintenance",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = UpdateMaintenanceRequest,
    responses(
        (status = 200, description = "Append-only status", body = AppendOnlyStatusResponse),
        (status = 400, description = "Invalid cron expression"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
        (status = 409, description = "Append-only is not on for this repository"),
    )
)]
/// Change when maintenance runs (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid cron expression and
/// [`ApiError::Conflict`] if append-only is off.
pub async fn update_maintenance(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateMaintenanceRequest>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    validate_maintenance(&req)?;
    let ctx = Context::load(&state, repo_id).await?;
    let existing = db::append_only::get_maintenance(&state.pool, repo_id)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict("append-only is not turned on for this repository".to_owned())
        })?;
    // A stopped repository stays stopped until an admin resumes it.
    let next = if existing.stopped_at.is_some() {
        None
    } else {
        crate::maintenance::next_cron_run(&state, &req.cron_expression).await
    };
    db::append_only::update_maintenance_settings(
        &state.pool,
        repo_id,
        &req.cron_expression,
        req.check_before_prune,
        next,
    )
    .await?;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(Json(status(&state, &ctx, None).await?))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/append-only/maintenance/run",
    tag = "Append-only",
    operation_id = "runMaintenance",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 202, description = "Maintenance started"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
        (status = 409, description = "Append-only is not on for this repository"),
    )
)]
/// Run maintenance now (admin only). It runs in the background, behind any
/// other operation on the repository.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if append-only is off.
pub async fn run_maintenance(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if !db::append_only::is_repo_append_only(&state.pool, repo_id).await? {
        return Err(ApiError::Conflict(
            "append-only is not turned on for this repository".to_owned(),
        ));
    }
    let runner = state.clone();
    state.background_task_tracker.spawn_tracked(async move {
        crate::maintenance::run(&runner, repo_id, &admin.username).await;
    });
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/append-only/maintenance/resume",
    tag = "Append-only",
    operation_id = "resumeMaintenance",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "Append-only status", body = AppendOnlyStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Repository not found"),
        (status = 409, description = "Append-only is not on for this repository"),
    )
)]
/// Accept that the archives which vanished are gone and let maintenance run
/// again (admin only).
///
/// The records of those archives are removed. Their data is only really gone
/// once maintenance next writes to the repository, so roll the repository back
/// on the host first if that was not intended.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if append-only is off.
pub async fn resume_maintenance(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<AppendOnlyStatusResponse>, ApiError> {
    let ctx = Context::load(&state, repo_id).await?;
    let settings = db::append_only::get_maintenance(&state.pool, repo_id)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict("append-only is not turned on for this repository".to_owned())
        })?;

    let vanished = db::append_only::list_vanished(&state.pool, repo_id).await?;
    db::delete_archive_records_by_names(&state.pool, repo_id, &vanished).await?;
    db::append_only::clear_all_vanished(&state.pool, repo_id).await?;
    let next = crate::maintenance::next_cron_run(&state, &settings.cron_expression).await;
    db::append_only::resume_maintenance(&state.pool, repo_id, next).await?;
    tracing::info!(repo_id, archives = vanished.len(), "maintenance resumed");
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(Json(status(&state, &ctx, None).await?))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}/archives/{archive_name}/deletion",
    tag = "Append-only",
    operation_id = "cancelArchiveDeletion",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 204, description = "The deletion is no longer queued"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "The archive has no queued deletion"),
    )
)]
/// Take an archive off the queue of deletions waiting for maintenance.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the archive is not queued.
pub async fn cancel_archive_deletion(
    State(state): State<AppState>,
    auth: super::auth::AuthUser,
    Path((repo_id, archive_name)): Path<(i64, String)>,
) -> Result<StatusCode, ApiError> {
    super::permissions::check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_delete)
        .await?;
    if !db::append_only::cancel_deletion(&state.pool, repo_id, &archive_name).await? {
        return Err(ApiError::NotFound(format!(
            "archive {archive_name} has no deletion queued"
        )));
    }
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(StatusCode::NO_CONTENT)
}

/// Lets a repository that is about to be added through an append-only login's
/// restricted key: the host's forced command names every repository the key
/// may reach, so it is rewritten with the new path before borg is run against
/// it. Does nothing for a login that is not append-only.
///
/// # Errors
///
/// Returns [`ApiError::BadGateway`] if the host's `authorized_keys` could not
/// be updated, since borg would be refused there.
pub(super) async fn allow_new_repository(
    state: &AppState,
    ssh_host: &str,
    ssh_user: &str,
    repo_path: &str,
) -> Result<(), ApiError> {
    let Some(host) = db::repo_hosts::find_repo_host_by_name(&state.pool, ssh_host).await? else {
        return Ok(());
    };
    if db::append_only::get_login(&state.pool, host.id, ssh_user)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let port = u16::try_from(host.ssh_port)
        .map_err(|_| ApiError::Internal(format!("ssh_port {} out of u16 range", host.ssh_port)))?;
    let mut paths: Vec<String> = db::append_only::list_login_repos(&state.pool, host.id, ssh_user)
        .await?
        .into_iter()
        .map(|r| r.repo_path)
        .chain(std::iter::once(repo_path.to_owned()))
        .collect();
    paths.sort();
    paths.dedup();
    let login = Login {
        host: &host.ssh_host,
        user: ssh_user,
        port,
        host_key: host.ssh_host_key.as_deref(),
    };
    ssh::append_only::enable(login, &paths, Access::ServerKey)
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!(
                "{ssh_host} is append-only and its authorized_keys could not be updated to allow \
                 the new repository: {e}"
            ))
        })
}

/// Gives a repository just added on an append-only login its maintenance
/// schedule. Does nothing for a login that is not append-only.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the schedule cannot be stored.
pub(super) async fn adopt_new_repository(state: &AppState, repo_id: i64) -> Result<(), ApiError> {
    if db::append_only::is_repo_append_only(&state.pool, repo_id).await? {
        let next = crate::maintenance::next_cron_run(state, DEFAULT_MAINTENANCE_CRON).await;
        db::append_only::ensure_maintenance(&state.pool, repo_id, next).await?;
    }
    Ok(())
}
