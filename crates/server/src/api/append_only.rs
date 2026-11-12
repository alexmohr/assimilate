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
/// A stored SSH port as a port number. The database keeps it in range, so a
/// value outside it means the row was altered by hand.
fn ssh_port(stored: i32) -> Result<u16, ApiError> {
    u16::try_from(stored)
        .map_err(|_| ApiError::Internal(format!("ssh_port {stored} out of u16 range")))
}

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
            ssh_port: ssh_port(host.ssh_port)?,
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

/// The way into the host: the one-time password when one was given, else the
/// server's key. An empty password counts as none.
fn access_for(password: Option<&str>) -> Access<'_> {
    password
        .filter(|p| !p.is_empty())
        .map_or(Access::ServerKey, Access::Password)
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
    // A path the host cannot be told about is shown as a failure to read, not
    // as a failed request: the admin opening the pane is who can fix the path.
    let (backup_line, path_failure) =
        match authorized_keys::restricted_line(&backup, &ctx.repo_paths()) {
            Ok(line) => (line, None),
            Err(e) => (
                String::new(),
                Some(AppendOnlySetupFailure {
                    needs_password: false,
                    message: e.to_string(),
                }),
            ),
        };
    let setup_failure = setup_failure.or(path_failure);
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
    // The lock comes first: the repositories the login names are read after it,
    // so one added meanwhile is not left out of the restriction written below.
    let _login = lock_repo_login(&state, repo_id).await?;
    let ctx = Context::load(&state, repo_id).await?;
    let access = access_for(req.password.as_deref());

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

    // The host now restricts the agents' key, so the server uses the other key
    // for everything but borg before the database write, which can fail:
    // enabling again then picks up from the host's real state.
    ssh::set_login_append_only(&ctx.ssh_host, &ctx.ssh_user, true);
    db::append_only::upsert_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user).await?;
    let next = crate::maintenance::next_cron_run(state, DEFAULT_MAINTENANCE_CRON).await;
    for repo in &ctx.repos {
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
    // Turning it off forgets what maintenance kept for every repository on the
    // login, and takes the key a run uses off the host, so no run may be going.
    let mut not_during_maintenance = Vec::with_capacity(ctx.repos.len());
    for repo in &ctx.repos {
        not_during_maintenance.push(lock_unless_busy(state, repo.id).await?);
    }
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

    // The host no longer has the maintenance key, so the server stops using it
    // before the database write, which can fail: disabling again then picks up
    // from the host's real state instead of leaving the server on a dead key.
    ssh::set_login_append_only(&ctx.ssh_host, &ctx.ssh_user, false);
    db::append_only::delete_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user).await?;
    // The login is already gone, so a repository whose leftovers cannot be
    // cleared is logged and the rest are still cleaned, rather than leaving
    // every repository after it half done.
    for repo in &ctx.repos {
        let repo_id = repo.id;
        forget_repo_maintenance(state, repo_id)
            .await
            .inspect_err(|e| tracing::warn!(repo_id, error = %e, "append-only cleanup failed"))
            .ok();
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
    // The lock comes first: the repositories the login names are read after it,
    // so one added meanwhile is not left out of the restriction written below.
    let _login = lock_repo_login(&state, repo_id).await?;
    let ctx = Context::load(&state, repo_id).await?;
    if db::append_only::get_login(&state.pool, ctx.repo_host_id, &ctx.ssh_user)
        .await?
        .is_none()
    {
        return Err(ApiError::Conflict(
            "append-only is not turned on for this repository".to_owned(),
        ));
    }
    let access = access_for(req.password.as_deref());
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

/// Forgets what maintenance kept for a repository: the archives it had marked
/// as vanished, and its schedule.
async fn forget_repo_maintenance(state: &AppState, repo_id: i64) -> Result<(), ApiError> {
    let vanished = forget_vanished(state, repo_id).await?;
    // Turning append-only off ends the maintenance that archives vanishing had
    // stopped, so leave a trail of what was forgotten.
    tracing::info!(repo_id, ?vanished, "forgot maintenance state");
    db::append_only::delete_maintenance(&state.pool, repo_id).await
}

/// Accepts that the archives marked as vanished are gone: their records are
/// removed along with the marks. Returns the names.
async fn forget_vanished(state: &AppState, repo_id: i64) -> Result<Vec<String>, ApiError> {
    let vanished = db::append_only::list_vanished(&state.pool, repo_id).await?;
    db::delete_archive_records_by_names(&state.pool, repo_id, &vanished).await?;
    db::append_only::clear_all_vanished(&state.pool, repo_id).await?;
    Ok(vanished)
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

    // A run that is looking at the repository must not have the archives it has
    // just marked cleared from under it.
    let _not_during_maintenance = lock_unless_busy(&state, repo_id).await?;
    let vanished = forget_vanished(&state, repo_id).await?;
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
    // A maintenance run reads the queue once and then deletes from it for as
    // long as it takes; taking an archive off the queue meanwhile would say
    // it is kept when the run is about to delete it.
    let _not_during_maintenance = lock_unless_busy(&state, repo_id).await?;
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
    write_login_paths(state, ssh_host, ssh_user, Some(repo_path)).await
}

/// Waits for, then holds, the lock of the login the repository is on. The
/// archive deletion queue takes it so that a deletion cannot be queued while
/// append-only is being turned off, which would leave it with nothing to carry
/// it out.
///
/// # Errors
///
/// Returns an error if the repository or its host cannot be found.
pub(super) async fn lock_repo_login(
    state: &AppState,
    repo_id: i64,
) -> Result<tokio::sync::OwnedMutexGuard<()>, ApiError> {
    let repo = db::get_repo_by_id(&state.pool, repo_id).await?;
    let host = db::repo_hosts::get_repo_host(&state.pool, repo.repo_host_id).await?;
    Ok(ssh::lock_login(&host.ssh_host, &repo.ssh_user).await)
}

/// Takes a repository that is gone out of its login's restriction, so the
/// host stops listing its path. A failure is logged: the repository is already
/// gone, and re-applying append-only rewrites the list.
pub(super) async fn forget_repository_path(state: &AppState, ssh_host: &str, ssh_user: &str) {
    let _login = ssh::lock_login(ssh_host, ssh_user).await;
    if let Err(e) = write_login_paths(state, ssh_host, ssh_user, None).await {
        tracing::warn!(
            ssh_host,
            error = %e,
            "could not take a removed repository out of authorized_keys"
        );
    }
}

/// Runs `work`, which adds the repository at `repo_path`, with that path let
/// through the append-only login's restriction. If `work` fails, the host's
/// `authorized_keys` goes back to naming only the repositories Assimilate
/// knows, so a repository that was never added is not left reachable.
///
/// # Errors
///
/// Returns [`ApiError::BadGateway`] if the path could not be let in, or what
/// `work` returns.
pub(super) async fn with_new_repository_allowed<T>(
    state: &AppState,
    ssh_host: &str,
    ssh_user: &str,
    repo_path: &str,
    work: impl std::future::Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    let _login = ssh::lock_login(ssh_host, ssh_user).await;
    allow_new_repository(state, ssh_host, ssh_user, repo_path).await?;
    let result = work.await;
    if result.is_err()
        && let Err(e) = write_login_paths(state, ssh_host, ssh_user, None).await
    {
        tracing::warn!(
            ssh_host,
            error = %e,
            "could not take a repository that was not added back out of authorized_keys"
        );
    }
    result
}

/// Takes the repository's lock if nobody holds it. A maintenance run holds it
/// for the whole run, so what needs to stay out of a run's way - the queue of
/// deletions, the archives marked vanished - is changed only while holding it.
async fn lock_unless_busy(
    state: &AppState,
    repo_id: i64,
) -> Result<tokio::sync::OwnedMutexGuard<()>, ApiError> {
    state.repo_lock.try_acquire(repo_id).await.ok_or_else(|| {
        ApiError::Conflict(
            "the repository is busy, possibly with maintenance; try again once it has finished"
                .to_owned(),
        )
    })
}

/// Takes the repository lock for an append-only repository, whose maintenance
/// holds it for a whole run; nothing is taken for any other repository.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] when maintenance holds the repository.
pub(super) async fn lock_if_append_only(
    state: &AppState,
    repo_id: i64,
) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, ApiError> {
    if db::append_only::is_repo_append_only(&state.pool, repo_id).await? {
        lock_unless_busy(state, repo_id).await.map(Some)
    } else {
        Ok(None)
    }
}

/// Whether `ssh_user` on `ssh_host` is an append-only login.
async fn is_append_only_login(
    state: &AppState,
    ssh_host: &str,
    ssh_user: &str,
) -> Result<bool, ApiError> {
    let Some(host) = db::repo_hosts::find_repo_host_by_name(&state.pool, ssh_host).await? else {
        return Ok(false);
    };
    Ok(db::append_only::get_login(&state.pool, host.id, ssh_user)
        .await?
        .is_some())
}

/// Refuses moving an append-only repository to a login that is not
/// append-only while deletions are queued for it, which nothing would carry
/// out there, or archives are marked vanished, whose marks the move would
/// discard like **Accept and resume** does, without anyone having looked.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] when deletions are queued or archives are
/// marked vanished, or when maintenance holds the repository.
///
/// For an append-only repository the returned guard holds the repository lock;
/// keep it until the move is stored and followed, so maintenance cannot mark
/// an archive vanished in between and have the move discard that.
pub(super) async fn check_repository_move(
    state: &AppState,
    repo_id: i64,
    new_host: &str,
    new_user: &str,
) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, ApiError> {
    if !db::append_only::is_repo_append_only(&state.pool, repo_id).await? {
        return Ok(None);
    }
    let guard = lock_unless_busy(state, repo_id).await?;
    // Onto another append-only login nothing is lost: queued deletions and
    // vanished marks belong to the repository, and maintenance there carries
    // on with them. Only leaving append-only strands them.
    if is_append_only_login(state, new_host, new_user).await? {
        return Ok(Some(guard));
    }
    if !db::append_only::list_pending_deletions(&state.pool, repo_id)
        .await?
        .is_empty()
    {
        return Err(ApiError::Conflict(
            "archive deletions are queued for this repository; run maintenance or cancel them \
             first, because nothing would carry them out once it leaves an append-only login"
                .to_owned(),
        ));
    }
    if !db::append_only::list_vanished(&state.pool, repo_id)
        .await?
        .is_empty()
    {
        return Err(ApiError::Conflict(
            "archives are marked as vanished from this repository; look into why and use Accept \
             and resume first, because moving it off an append-only login would discard that"
                .to_owned(),
        ));
    }
    Ok(Some(guard))
}

/// Brings append-only in line after a repository moved: the login it left
/// stops naming its old path, and it is scheduled for maintenance if its new
/// login is append-only, or has its maintenance state dropped if it is not.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database cannot be updated.
pub(super) async fn follow_repository_move(
    state: &AppState,
    repo_id: i64,
    old_host: &str,
    old_user: &str,
) -> Result<(), ApiError> {
    forget_repository_path(state, old_host, old_user).await;
    if db::append_only::is_repo_append_only(&state.pool, repo_id).await? {
        adopt_new_repository(state, repo_id).await
    } else {
        // A deletion queued after the move was checked, but before it was
        // stored, has nothing left to carry it out: take it off the queue.
        for pending in db::append_only::list_pending_deletions(&state.pool, repo_id).await? {
            db::append_only::cancel_deletion(&state.pool, repo_id, &pending.archive_name).await?;
        }
        forget_repo_maintenance(state, repo_id).await
    }
}

/// Rewrites the append-only login's restriction to name the repositories
/// Assimilate knows, plus `extra`. Does nothing for a login that is not
/// append-only. A login left with no repository at all is turned off.
async fn write_login_paths(
    state: &AppState,
    ssh_host: &str,
    ssh_user: &str,
    extra: Option<&str>,
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
    let port = ssh_port(host.ssh_port)?;
    let mut paths: Vec<String> = db::append_only::list_login_repos(&state.pool, host.id, ssh_user)
        .await?
        .into_iter()
        .map(|r| r.repo_path)
        .chain(extra.map(str::to_owned))
        .collect();
    paths.sort();
    paths.dedup();
    let login = Login {
        host: &host.ssh_host,
        user: ssh_user,
        port,
        host_key: host.ssh_host_key.as_deref(),
    };
    if paths.is_empty() {
        // The last repository left the login, so there is nothing to restrict
        // the key to, and no repository left to turn it off through later:
        // take the restriction off now.
        ssh::append_only::disable(login, Access::ServerKey)
            .await
            .map_err(|e| {
                ApiError::BadGateway(format!(
                    "{ssh_host} is append-only and no repository is left on it, but its \
                     authorized_keys could not be put back: {e}"
                ))
            })?;
        ssh::set_login_append_only(ssh_host, ssh_user, false);
        return db::append_only::delete_login(&state.pool, host.id, ssh_user).await;
    }
    ssh::append_only::enable(login, &paths, Access::ServerKey)
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!(
                "{ssh_host} is append-only and its authorized_keys could not be updated for its \
                 repositories: {e}"
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

#[cfg(test)]
mod tests {
    //! The handlers against a stand-in SSH host, so that turning append-only on
    //! and off really edits an `authorized_keys` file.

    use axum::{
        extract::{Path, State},
        http::StatusCode,
    };
    use shared::append_only::{
        ReapplyAppendOnlyRequest, SetAppendOnlyRequest, UpdateMaintenanceRequest,
    };

    use super::*;
    use crate::{
        api::auth::AuthUser,
        ssh::append_only_tests::{Fixture, HostOptions},
        test_support,
    };

    /// A login of its own, so the process-wide record of append-only logins
    /// these tests change does not reach the stand-in host's own tests.
    const LOGIN_USER: &str = "api-tests";
    const ROLE_USER: &str = "api-tests-role";
    const ROLE_USER_UNREACHABLE: &str = "api-tests-role-unreachable";

    fn admin() -> RequireAdmin {
        RequireAdmin(AuthUser {
            user_id: 1,
            username: "admin".to_owned(),
            session_id: None,
        })
    }

    async fn add_repo(pool: &sqlx::PgPool, name: &str, path: &str, port: i32) -> db::RepoRow {
        add_repo_as(pool, LOGIN_USER, name, path, port).await
    }

    /// Like [`add_repo`], for a login of its own: the process-wide record of
    /// which logins are append-only is shared by tests running in parallel, so
    /// a test that reads it back must not share its login with the others.
    async fn add_repo_as(
        pool: &sqlx::PgPool,
        user: &str,
        name: &str,
        path: &str,
        port: i32,
    ) -> db::RepoRow {
        db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name,
                repo_path: path,
                ssh_user: user,
                ssh_host: "127.0.0.1",
                ssh_port: port,
                passphrase_encrypted: b"encrypted_data",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap()
    }

    fn state(pool: sqlx::PgPool) -> AppState {
        test_support::build_test_state(pool, b"append-only-api-test-key-material")
    }

    async fn set(state: &AppState, repo_id: i64, enabled: bool) -> AppendOnlyStatusResponse {
        let Json(status) = Box::pin(set_append_only(
            State(state.clone()),
            admin(),
            Path(repo_id),
            ApiJson(SetAppendOnlyRequest {
                enabled,
                password: None,
            }),
        ))
        .await
        .unwrap();
        status
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn turning_it_on_restricts_every_repository_on_the_login(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let port = i32::from(host.port);
        let first = add_repo(&pool, "first", "/srv/borg/first", port).await;
        let second = add_repo(&pool, "second", "/srv/borg/second", port).await;
        let state = state(pool);

        let status = host.scoped(set(&state, first.id, true)).await;

        assert!(status.enabled);
        assert!(status.setup_failure.is_none(), "{:?}", status.setup_failure);
        assert!(status.maintenance.is_some());
        let check = status.check.expect("a check right after setup");
        assert!(check.backup_key_ok && check.maintenance_key_ok && check.restriction_in_place);
        let file = host.authorized_keys();
        assert!(file.contains("--append-only"), "{file}");
        assert!(file.contains("/srv/borg/first"), "{file}");
        assert!(file.contains("/srv/borg/second"), "{file}");
        assert!(
            db::append_only::is_repo_append_only(&state.pool, second.id)
                .await
                .unwrap()
        );
        assert!(
            db::append_only::get_maintenance(&state.pool, second.id)
                .await
                .unwrap()
                .is_some(),
            "a sibling gets its own schedule"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn turning_it_off_lifts_the_restriction_and_forgets_the_schedule(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        db::append_only::mark_vanished(&state.pool, repo.id, &["gone".to_owned()])
            .await
            .unwrap();

        let status = host.scoped(set(&state, repo.id, false)).await;

        assert!(!status.enabled);
        assert!(status.setup_failure.is_none(), "{:?}", status.setup_failure);
        assert!(!host.authorized_keys().contains("--append-only"));
        assert!(
            !db::append_only::is_repo_append_only(&state.pool, repo.id)
                .await
                .unwrap()
        );
        assert!(
            db::append_only::get_maintenance(&state.pool, repo.id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db::append_only::list_vanished(&state.pool, repo.id)
                .await
                .unwrap(),
            Vec::<String>::new()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_host_that_refuses_the_setup_leaves_append_only_off(pool: sqlx::PgPool) {
        // The host's port is closed; the fixture only provides the server's keys.
        let keys = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "unreachable", "/srv/borg/x", 1).await;
        let state = state(pool);

        let status = keys.scoped(set(&state, repo.id, true)).await;

        assert!(!status.enabled);
        assert!(status.setup_failure.is_some());
        assert!(
            !db::append_only::is_repo_append_only(&state.pool, repo.id)
                .await
                .unwrap()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn checking_and_reapplying_report_what_the_host_holds(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;

        let Json(checked) = host
            .scoped(Box::pin(check_append_only(
                State(state.clone()),
                admin(),
                Path(repo.id),
            )))
            .await
            .unwrap();
        assert!(checked.check.expect("check").restriction_in_place);

        let Json(reapplied) = host
            .scoped(Box::pin(reapply_append_only(
                State(state.clone()),
                admin(),
                Path(repo.id),
                ApiJson(ReapplyAppendOnlyRequest { password: None }),
            )))
            .await
            .unwrap();
        assert!(reapplied.setup_failure.is_none());
        assert!(host.authorized_keys().contains("--append-only"));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn checking_a_repository_that_is_not_append_only_does_not_connect(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "plain", "/srv/borg/plain", 1).await;
        let state = state(pool);

        let Json(status) = host
            .scoped(Box::pin(check_append_only(
                State(state),
                admin(),
                Path(repo.id),
            )))
            .await
            .unwrap();

        assert!(!status.enabled);
        assert!(status.check.is_none());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn reapplying_where_append_only_is_off_is_refused(pool: sqlx::PgPool) {
        let repo = add_repo(&pool, "plain", "/srv/borg/plain", 1).await;
        let result = reapply_append_only(
            State(state(pool)),
            admin(),
            Path(repo.id),
            ApiJson(ReapplyAppendOnlyRequest { password: None }),
        )
        .await;
        assert!(matches!(result, Err(ApiError::Conflict(_))), "{result:?}");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_stopped_schedule_stays_stopped_when_it_is_edited(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        db::append_only::mark_vanished(&state.pool, repo.id, &["gone-1".to_owned()])
            .await
            .unwrap();
        db::append_only::finish_maintenance(&state.pool, repo.id, chrono::Utc::now(), None, true)
            .await
            .unwrap();

        let Json(status) = host
            .scoped(update_maintenance(
                State(state.clone()),
                admin(),
                Path(repo.id),
                ApiJson(UpdateMaintenanceRequest {
                    cron_expression: "0 3 * * *".to_owned(),
                    check_before_prune: false,
                }),
            ))
            .await
            .unwrap();

        let maintenance = status.maintenance.expect("schedule");
        assert_eq!(maintenance.cron_expression, "0 3 * * *");
        assert!(!maintenance.check_before_prune);
        assert!(maintenance.next_run_at.is_none(), "still stopped");
        let stopped = maintenance.stopped.expect("stopped");
        assert_eq!(stopped.vanished_archives, vec!["gone-1".to_owned()]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_login_with_no_schedule_for_this_repository_shows_none(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        db::append_only::delete_maintenance(&state.pool, repo.id)
            .await
            .unwrap();

        let Json(status) = host
            .scoped(get_append_only(State(state), admin(), Path(repo.id)))
            .await
            .unwrap();

        assert!(status.enabled);
        assert!(status.maintenance.is_none());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_host_that_cannot_be_reached_keeps_append_only_on_when_turning_it_off(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        db::repo_hosts::update_repo_host_address(&state.pool, repo.repo_host_id, "127.0.0.1", 1)
            .await
            .unwrap();

        let status = host.scoped(set(&state, repo.id, false)).await;

        assert!(status.enabled, "still on: the host was never changed");
        assert!(status.setup_failure.is_some());
        assert!(host.authorized_keys().contains("--append-only"));
    }

    /// An append-only login with one repository on the stand-in host, and the
    /// borg stand-in that answers `info` and `init`.
    async fn append_only_login(
        pool: sqlx::PgPool,
        host: &Fixture,
    ) -> (AppState, db::RepoRow, impl Sized) {
        let existing = add_repo(
            &pool,
            "existing",
            "/srv/borg/existing",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, existing.id, true)).await;
        let borg = crate::api::repos::test_helpers::install_fake_script(
            "#!/bin/sh\nprintf '{\"encryption\":{\"mode\":\"repokey\"}}'\n",
        )
        .await;
        (state, existing, borg)
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_initialised_on_an_append_only_login_is_let_in_and_scheduled(
        pool: sqlx::PgPool,
    ) {
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let host = Fixture::start(HostOptions::default()).await;
        let (state, existing, _borg) = append_only_login(pool, &host).await;

        let (status, Json(created)) = host
            .scoped(Box::pin(crate::api::repos::init_repo(
                State(state.clone()),
                admin(),
                ApiJson(crate::api::repos::InitRepoRequest {
                    name: "fresh".to_owned(),
                    repo_path: "/srv/borg/fresh".to_owned(),
                    ssh_user: LOGIN_USER.to_owned(),
                    ssh_host: "127.0.0.1".to_owned(),
                    ssh_port: Some(i32::from(host.port)),
                    passphrase: "pw".to_owned(),
                    encryption: shared::types::BorgEncryption::Repokey,
                    compression: None,
                }),
            )))
            .await
            .unwrap();

        assert_eq!(status, StatusCode::CREATED);
        let file = host.authorized_keys();
        assert!(file.contains("/srv/borg/fresh"), "{file}");
        assert!(file.contains("/srv/borg/existing"), "{file}");
        assert!(
            db::append_only::get_maintenance(&state.pool, created.repo.id)
                .await
                .unwrap()
                .is_some()
        );
        assert_ne!(created.repo.id, existing.id);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_added_on_an_append_only_login_is_let_in_and_scheduled(
        pool: sqlx::PgPool,
    ) {
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let host = Fixture::start(HostOptions::default()).await;
        let (state, _existing, _borg) = append_only_login(pool, &host).await;

        let (status, Json(created)) = host
            .scoped(Box::pin(crate::api::repos::create_repo(
                State(state.clone()),
                admin(),
                ApiJson(crate::api::repos::CreateRepoRequest {
                    name: "adopted".to_owned(),
                    repo_path: "/srv/borg/adopted".to_owned(),
                    ssh_user: LOGIN_USER.to_owned(),
                    ssh_host: "127.0.0.1".to_owned(),
                    ssh_port: Some(i32::from(host.port)),
                    passphrase: "pw".to_owned(),
                    compression: None,
                }),
            )))
            .await
            .unwrap();

        assert_eq!(status, StatusCode::CREATED);
        assert!(host.authorized_keys().contains("/srv/borg/adopted"));
        assert!(
            db::append_only::get_maintenance(&state.pool, created.id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_that_could_not_be_initialised_is_not_left_reachable(pool: sqlx::PgPool) {
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let host = Fixture::start(HostOptions::default()).await;
        let (state, _existing, _borg) = append_only_login(pool, &host).await;
        // borg refuses to initialise anything.
        let _failing = crate::api::repos::test_helpers::install_fake_script(
            "#!/bin/sh\necho 'no space left' >&2\nexit 2\n",
        )
        .await;

        let result = host
            .scoped(Box::pin(crate::api::repos::init_repo(
                State(state.clone()),
                admin(),
                ApiJson(crate::api::repos::InitRepoRequest {
                    name: "never".to_owned(),
                    repo_path: "/srv/borg/never".to_owned(),
                    ssh_user: LOGIN_USER.to_owned(),
                    ssh_host: "127.0.0.1".to_owned(),
                    ssh_port: Some(i32::from(host.port)),
                    passphrase: "pw".to_owned(),
                    encryption: shared::types::BorgEncryption::Repokey,
                    compression: None,
                }),
            )))
            .await;

        assert!(result.is_err());
        let file = host.authorized_keys();
        assert!(file.contains("/srv/borg/existing"), "{file}");
        assert!(
            !file.contains("/srv/borg/never"),
            "the path of a repository that was never added is taken back: {file}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_attempt_to_take_the_grant_back_is_reported_but_not_fatal(pool: sqlx::PgPool) {
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let host = Fixture::start(HostOptions::default()).await;
        let (state, _existing, _borg) = append_only_login(pool, &host).await;
        let _failing = crate::api::repos::test_helpers::install_fake_script(
            "#!/bin/sh\necho 'no space left' >&2\nexit 2\n",
        )
        .await;
        // Letting the path in writes once; taking it back out would write
        // again, and that write fails.
        host.fault(|f| f.fail_writes_to = Some((".ssh/authorized_keys".to_owned(), 1)));

        let result = host
            .scoped(Box::pin(crate::api::repos::init_repo(
                State(state.clone()),
                admin(),
                ApiJson(crate::api::repos::InitRepoRequest {
                    name: "never".to_owned(),
                    repo_path: "/srv/borg/never".to_owned(),
                    ssh_user: LOGIN_USER.to_owned(),
                    ssh_host: "127.0.0.1".to_owned(),
                    ssh_port: Some(i32::from(host.port)),
                    passphrase: "pw".to_owned(),
                    encryption: shared::types::BorgEncryption::Repokey,
                    compression: None,
                }),
            )))
            .await;

        // The caller still hears why the repository was not added, not about
        // the clean-up that went wrong afterwards.
        assert!(
            matches!(&result, Err(ApiError::BadGateway(m)) if m.contains("no space left")),
            "{result:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_a_host_forgets_that_its_login_was_append_only(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let host_id = db::repo_hosts::resolve_repo_host(&mut tx, "forgotten.lan", 22)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        db::append_only::upsert_login(&pool, host_id, "someone")
            .await
            .unwrap();
        crate::ssh::set_login_append_only("forgotten.lan", "someone", true);
        assert_eq!(
            crate::ssh::key_role_for("forgotten.lan", "someone"),
            crate::ssh::KeyRole::Maintenance
        );

        let status =
            crate::api::repo_hosts::delete_repo_host(State(state(pool)), admin(), Path(host_id))
                .await
                .unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            crate::ssh::key_role_for("forgotten.lan", "someone"),
            crate::ssh::KeyRole::Backup,
            "a host added again under the same name is not treated as append-only"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn renaming_a_host_moves_its_append_only_logins_with_it(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let host_id = db::repo_hosts::resolve_repo_host(&mut tx, "before.lan", 22)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        db::append_only::upsert_login(&pool, host_id, "renamer")
            .await
            .unwrap();
        crate::ssh::set_login_append_only("before.lan", "renamer", true);
        let state = state(pool);
        let rename = |ssh_host: &str, ssh_port: i32| {
            crate::api::repo_hosts::update_repo_host(
                State(state.clone()),
                admin(),
                Path(host_id),
                ApiJson(crate::api::repo_hosts::UpdateRepoHostRequest {
                    ssh_host: ssh_host.to_owned(),
                    ssh_port,
                }),
            )
        };

        // Only the port changes: the login stays append-only throughout.
        let Json(renamed) = rename("before.lan", 2222).await.unwrap();
        assert_eq!(renamed.ssh_port, 2222);
        assert_eq!(
            crate::ssh::key_role_for("before.lan", "renamer"),
            crate::ssh::KeyRole::Maintenance
        );

        let Json(renamed) = rename("after.lan", 2222).await.unwrap();
        assert_eq!(renamed.ssh_host, "after.lan");
        assert_eq!(
            crate::ssh::key_role_for("after.lan", "renamer"),
            crate::ssh::KeyRole::Maintenance
        );
        assert_eq!(
            crate::ssh::key_role_for("before.lan", "renamer"),
            crate::ssh::KeyRole::Backup,
            "the old name is not remembered as append-only"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_a_repository_takes_its_path_out_of_the_login(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let kept = add_repo(&pool, "kept", "/srv/borg/kept", i32::from(host.port)).await;
        let gone = add_repo(&pool, "gone", "/srv/borg/gone", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, kept.id, true)).await;
        assert!(host.authorized_keys().contains("/srv/borg/gone"));

        let status = host
            .scoped(Box::pin(crate::api::repos::delete_repo(
                State(state.clone()),
                admin(),
                Path(gone.id),
            )))
            .await
            .unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!host.authorized_keys().contains("/srv/borg/gone"));
        assert!(host.authorized_keys().contains("/srv/borg/kept"));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_the_last_repository_of_a_login_turns_append_only_off(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let only = add_repo_as(
            &pool,
            ROLE_USER,
            "only",
            "/srv/borg/only",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, only.id, true)).await;
        assert!(host.authorized_keys().contains("--append-only"));

        host.scoped(Box::pin(crate::api::repos::delete_repo(
            State(state.clone()),
            admin(),
            Path(only.id),
        )))
        .await
        .unwrap();

        let keys = host.authorized_keys();
        assert!(!keys.contains("--append-only"), "{keys}");
        assert!(!keys.contains("/srv/borg/only"), "{keys}");
        assert_eq!(
            crate::ssh::key_role_for("127.0.0.1", ROLE_USER),
            crate::ssh::KeyRole::Backup
        );
        let logins = db::append_only::list_logins_with_hosts(&state.pool)
            .await
            .unwrap();
        assert!(
            !logins.contains(&("127.0.0.1".to_owned(), ROLE_USER.to_owned())),
            "{logins:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_host_that_cannot_be_reached_keeps_the_login_of_a_deleted_last_repository(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let only = add_repo_as(
            &pool,
            ROLE_USER_UNREACHABLE,
            "only",
            "/srv/borg/only",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, only.id, true)).await;
        // From here on nothing answers on the host's port.
        db::repo_hosts::update_repo_host_address(&state.pool, only.repo_host_id, "127.0.0.1", 1)
            .await
            .unwrap();

        let status = host
            .scoped(Box::pin(crate::api::repos::delete_repo(
                State(state.clone()),
                admin(),
                Path(only.id),
            )))
            .await
            .unwrap();

        // The repository is gone either way; the login is left as it was, to
        // be put right by the next repository added on it.
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(host.authorized_keys().contains("--append-only"));
        assert_eq!(
            crate::ssh::key_role_for("127.0.0.1", ROLE_USER_UNREACHABLE),
            crate::ssh::KeyRole::Maintenance
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn destroying_a_repository_takes_its_path_out_of_the_login(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let kept = add_repo(&pool, "kept", "/srv/borg/kept", i32::from(host.port)).await;
        let gone = add_repo(&pool, "gone", "/srv/borg/gone", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, kept.id, true)).await;

        let status = host
            .scoped(Box::pin(crate::api::repos::destroy_repo(
                State(state.clone()),
                admin(),
                Path(gone.id),
            )))
            .await
            .unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(host.commands(), vec!["rm -rf '/srv/borg/gone'".to_owned()]);
        assert!(!host.authorized_keys().contains("/srv/borg/gone"));
        assert!(host.authorized_keys().contains("/srv/borg/kept"));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_host_that_cannot_be_updated_still_lets_a_repository_be_deleted(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let kept = add_repo(&pool, "kept", "/srv/borg/kept", i32::from(host.port)).await;
        let gone = add_repo(&pool, "gone", "/srv/borg/gone", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, kept.id, true)).await;
        // From here on nothing answers on the host's port.
        db::repo_hosts::update_repo_host_address(&state.pool, kept.repo_host_id, "127.0.0.1", 1)
            .await
            .unwrap();

        let status = host
            .scoped(Box::pin(crate::api::repos::delete_repo(
                State(state.clone()),
                admin(),
                Path(gone.id),
            )))
            .await
            .unwrap();

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(
            host.authorized_keys().contains("/srv/borg/gone"),
            "the host could not be reached, so its list is left as it was"
        );
    }

    fn move_to(
        repo: &db::RepoRow,
        ssh_user: &str,
        repo_path: &str,
    ) -> crate::api::repos::UpdateRepoRequest {
        crate::api::repos::UpdateRepoRequest {
            name: None,
            repo_path: repo_path.to_owned(),
            ssh_user: ssh_user.to_owned(),
            ssh_host: "127.0.0.1".to_owned(),
            ssh_port: Some(repo.ssh_port),
            compression: None,
            encryption: None,
            enabled: None,
            sync_schedule: None,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn changing_a_repositorys_path_changes_what_the_login_names(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let kept = add_repo(&pool, "kept", "/srv/borg/kept", i32::from(host.port)).await;
        let moved = add_repo(&pool, "moved", "/srv/borg/old", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, kept.id, true)).await;
        assert!(host.authorized_keys().contains("/srv/borg/old"));

        let Json(updated) = host
            .scoped(Box::pin(crate::api::repos::update_repo(
                State(state.clone()),
                admin(),
                Path(moved.id),
                ApiJson(move_to(&moved, LOGIN_USER, "/srv/borg/new")),
            )))
            .await
            .unwrap();
        assert_eq!(updated.repo_path, "/srv/borg/new");

        let keys = host.authorized_keys();
        assert!(keys.contains("/srv/borg/new"), "{keys}");
        assert!(!keys.contains("/srv/borg/old"), "{keys}");
        assert!(keys.contains("/srv/borg/kept"), "{keys}");
        assert!(
            db::append_only::get_maintenance(&state.pool, moved.id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resuming_and_turning_off_wait_for_a_run_that_holds_the_repository(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "busy", "/srv/borg/busy", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        db::append_only::mark_vanished(&state.pool, repo.id, &["gone".to_owned()])
            .await
            .unwrap();
        let keys_before = host.authorized_keys();

        // A maintenance run holds the repository's lock for the whole run.
        let run = state.repo_lock.acquire(repo.id).await;

        let resumed = resume_maintenance(State(state.clone()), admin(), Path(repo.id)).await;
        assert!(matches!(resumed, Err(ApiError::Conflict(_))), "{resumed:?}");
        let off = host
            .scoped(Box::pin(set_append_only(
                State(state.clone()),
                admin(),
                Path(repo.id),
                ApiJson(SetAppendOnlyRequest {
                    enabled: false,
                    password: None,
                }),
            )))
            .await;
        assert!(matches!(off, Err(ApiError::Conflict(_))), "{off:?}");

        // Nothing it was looking at changed, and the host is as it was.
        assert_eq!(
            db::append_only::list_vanished(&state.pool, repo.id)
                .await
                .unwrap(),
            vec!["gone".to_owned()]
        );
        assert!(
            db::append_only::is_repo_append_only(&state.pool, repo.id)
                .await
                .unwrap()
        );
        assert_eq!(host.authorized_keys(), keys_before);

        // Once the run is over, both go through.
        drop(run);
        let Json(resumed) = host
            .scoped(Box::pin(resume_maintenance(
                State(state.clone()),
                admin(),
                Path(repo.id),
            )))
            .await
            .unwrap();
        assert!(resumed.enabled);
        let status = host.scoped(set(&state, repo.id, false)).await;
        assert!(!status.enabled);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_with_vanished_archives_cannot_leave_an_append_only_login(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let moved = add_repo(&pool, "moved", "/srv/borg/old", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, moved.id, true)).await;
        db::append_only::mark_vanished(&state.pool, moved.id, &["gone".to_owned()])
            .await
            .unwrap();

        let refused = crate::api::repos::update_repo(
            State(state.clone()),
            admin(),
            Path(moved.id),
            ApiJson(move_to(&moved, "somebody-else", "/srv/borg/old")),
        )
        .await;

        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");
        assert_eq!(
            db::append_only::list_vanished(&state.pool, moved.id)
                .await
                .unwrap(),
            vec!["gone".to_owned()],
            "the evidence is still there"
        );
        // Staying on the login is fine: nothing is discarded.
        let Json(updated) = host
            .scoped(Box::pin(crate::api::repos::update_repo(
                State(state.clone()),
                admin(),
                Path(moved.id),
                ApiJson(move_to(&moved, LOGIN_USER, "/srv/borg/new")),
            )))
            .await
            .unwrap();
        assert_eq!(updated.repo_path, "/srv/borg/new");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_deletion_queued_as_a_repository_leaves_is_taken_off_the_queue(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let moved = add_repo(&pool, "moved", "/srv/borg/old", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, moved.id, true)).await;
        // The repository has moved to a login that is not append-only, and a
        // deletion was queued in the instant before.
        db::update_repo(
            &state.pool,
            &db::UpdateRepoParams {
                repo_id: moved.id,
                name: &moved.name,
                repo_path: &moved.repo_path,
                ssh_user: "somebody-else",
                ssh_host: "127.0.0.1",
                ssh_port: moved.ssh_port,
                compression: "lz4",
                encryption: "repokey",
                enabled: true,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::append_only::queue_deletion(&state.pool, moved.id, "a-1", "admin")
            .await
            .unwrap();

        host.scoped(Box::pin(follow_repository_move(
            &state,
            moved.id,
            "127.0.0.1",
            LOGIN_USER,
        )))
        .await
        .unwrap();

        let left = db::append_only::list_pending_deletions(&state.pool, moved.id)
            .await
            .unwrap();
        assert_eq!(left.len(), 0, "nothing is left waiting");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn editing_a_repository_in_place_leaves_its_login_alone(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "plain", "/srv/borg/plain", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;
        let before = host.authorized_keys();
        let mut request = move_to(&repo, LOGIN_USER, "/srv/borg/plain");
        request.name = Some("renamed".to_owned());

        let Json(updated) = host
            .scoped(Box::pin(crate::api::repos::update_repo(
                State(state.clone()),
                admin(),
                Path(repo.id),
                ApiJson(request),
            )))
            .await
            .unwrap();

        assert_eq!(updated.name, "renamed");
        assert_eq!(
            host.authorized_keys(),
            before,
            "nothing on the host changed"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn moving_a_repository_off_an_append_only_login_drops_it_from_the_restriction(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let kept = add_repo(&pool, "kept", "/srv/borg/kept", i32::from(host.port)).await;
        let moved = add_repo(&pool, "moved", "/srv/borg/old", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, kept.id, true)).await;

        let Json(updated) = host
            .scoped(Box::pin(crate::api::repos::update_repo(
                State(state.clone()),
                admin(),
                Path(moved.id),
                ApiJson(move_to(&moved, "somebody-else", "/srv/borg/old")),
            )))
            .await
            .unwrap();
        assert_eq!(updated.ssh_user, "somebody-else");

        let keys = host.authorized_keys();
        assert!(!keys.contains("/srv/borg/old"), "{keys}");
        assert!(keys.contains("/srv/borg/kept"), "{keys}");
        assert!(
            !db::append_only::is_repo_append_only(&state.pool, moved.id)
                .await
                .unwrap()
        );
        assert!(
            db::append_only::get_maintenance(&state.pool, moved.id)
                .await
                .unwrap()
                .is_none(),
            "it has nothing left to be maintained for"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_with_queued_deletions_cannot_leave_an_append_only_login(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let moved = add_repo(&pool, "moved", "/srv/borg/old", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, moved.id, true)).await;
        db::append_only::queue_deletion(&state.pool, moved.id, "a-1", "admin")
            .await
            .unwrap();

        let refused = crate::api::repos::update_repo(
            State(state.clone()),
            admin(),
            Path(moved.id),
            ApiJson(move_to(&moved, "somebody-else", "/srv/borg/old")),
        )
        .await;
        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");

        // A host Assimilate has no record of is not an append-only login either.
        let refused = check_repository_move(&state, moved.id, "no-such-host", LOGIN_USER).await;
        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");

        // Nothing moved, and the login still names the repository.
        assert!(host.authorized_keys().contains("/srv/borg/old"));
        assert!(
            db::append_only::is_repo_append_only(&state.pool, moved.id)
                .await
                .unwrap()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_status_of_a_repository_with_an_unusable_path_reports_it(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let odd = add_repo(&pool, "odd", "/srv/borg/o\"dd", i32::from(host.port)).await;
        let state = state(pool);

        let Json(status) = host
            .scoped(get_append_only(State(state.clone()), admin(), Path(odd.id)))
            .await
            .unwrap();

        // Opening the pane is not an error; the path is what to fix.
        let failure = status.setup_failure.expect("the path is reported");
        assert!(failure.message.contains("o\\\"dd"), "{}", failure.message);
        assert!(!failure.needs_password);
        assert_eq!(status.backup_key.authorized_keys_line, "");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_append_only_repository_cannot_move_while_maintenance_holds_it(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let moved = add_repo(
            &pool,
            "busy-move",
            "/srv/borg/busy-move",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, moved.id, true)).await;

        // Maintenance could mark archives vanished while the move discards
        // marks, so the move waits for it rather than racing it.
        let run = state.repo_lock.acquire(moved.id).await;
        let refused = crate::api::repos::update_repo(
            State(state.clone()),
            admin(),
            Path(moved.id),
            ApiJson(move_to(&moved, "somebody-else", "/srv/borg/busy-move")),
        )
        .await;
        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");
        drop(run);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_append_only_repository_cannot_be_deleted_or_destroyed_while_maintenance_holds_it(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let busy = add_repo(&pool, "busy", "/srv/borg/busy", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, busy.id, true)).await;

        let run = state.repo_lock.acquire(busy.id).await;
        let deleted =
            crate::api::repos::delete_repo(State(state.clone()), admin(), Path(busy.id)).await;
        assert!(matches!(deleted, Err(ApiError::Conflict(_))), "{deleted:?}");
        let destroyed =
            crate::api::repos::destroy_repo(State(state.clone()), admin(), Path(busy.id)).await;
        assert!(
            matches!(destroyed, Err(ApiError::Conflict(_))),
            "{destroyed:?}"
        );
        drop(run);

        // Nothing was removed, on the host or in the database.
        assert!(host.authorized_keys().contains("/srv/borg/busy"));
        assert!(db::get_repo_by_id(&state.pool, busy.id).await.is_ok());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn running_maintenance_is_accepted_for_an_append_only_repository(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "only", "/srv/borg/only", i32::from(host.port)).await;
        let state = state(pool);
        host.scoped(set(&state, repo.id, true)).await;

        let accepted = run_maintenance(State(state.clone()), admin(), Path(repo.id))
            .await
            .unwrap();
        assert_eq!(accepted, StatusCode::ACCEPTED);

        let refused = run_maintenance(State(state), admin(), Path(i64::MAX)).await;
        assert!(matches!(refused, Err(ApiError::Conflict(_))), "{refused:?}");
    }

    #[test]
    fn a_stored_port_outside_the_range_is_an_internal_error() {
        assert_eq!(ssh_port(22).unwrap(), 22);
        assert!(matches!(ssh_port(70_000), Err(ApiError::Internal(_))));
        assert!(matches!(ssh_port(-1), Err(ApiError::Internal(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_new_repository_on_an_append_only_login_is_let_in_and_scheduled(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let existing = add_repo(
            &pool,
            "existing",
            "/srv/borg/existing",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, existing.id, true)).await;

        host.scoped(Box::pin(allow_new_repository(
            &state,
            "127.0.0.1",
            LOGIN_USER,
            "/srv/borg/new",
        )))
        .await
        .unwrap();
        assert!(host.authorized_keys().contains("/srv/borg/new"));
        assert!(host.authorized_keys().contains("/srv/borg/existing"));

        let added = add_repo(&state.pool, "new", "/srv/borg/new", i32::from(host.port)).await;
        adopt_new_repository(&state, added.id).await.unwrap();
        assert!(
            db::append_only::get_maintenance(&state.pool, added.id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_new_repository_on_a_login_that_is_not_append_only_changes_nothing(
        pool: sqlx::PgPool,
    ) {
        let host = Fixture::start(HostOptions::default()).await;
        let repo = add_repo(&pool, "plain", "/srv/borg/plain", i32::from(host.port)).await;
        let state = state(pool);

        allow_new_repository(&state, "127.0.0.1", LOGIN_USER, "/srv/borg/more")
            .await
            .unwrap();
        allow_new_repository(&state, "no-such-host", LOGIN_USER, "/srv/borg/more")
            .await
            .unwrap();
        adopt_new_repository(&state, repo.id).await.unwrap();

        assert!(!host.authorized_keys().contains("--append-only"));
        assert!(
            db::append_only::get_maintenance(&state.pool, repo.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_host_that_cannot_be_updated_refuses_the_new_repository(pool: sqlx::PgPool) {
        let host = Fixture::start(HostOptions::default()).await;
        let existing = add_repo(
            &pool,
            "existing",
            "/srv/borg/existing",
            i32::from(host.port),
        )
        .await;
        let state = state(pool);
        host.scoped(set(&state, existing.id, true)).await;
        // From here on nothing answers on the host's port.
        db::repo_hosts::update_repo_host_address(
            &state.pool,
            existing.repo_host_id,
            "127.0.0.1",
            1,
        )
        .await
        .unwrap();

        let result = host
            .scoped(Box::pin(allow_new_repository(
                &state,
                "127.0.0.1",
                LOGIN_USER,
                "/srv/borg/new",
            )))
            .await;

        assert!(matches!(result, Err(ApiError::BadGateway(_))), "{result:?}");
    }
}
