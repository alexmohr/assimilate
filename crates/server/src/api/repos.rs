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
    audit::AuditEvent,
    crypto::encrypt_passphrase,
    responses::{
        BreakLockResponse, ConfirmRelocationResponse, ExecBorgResponse, InitRepoResponse,
        MigrateEncryptionResponse, PassphraseResponse, RepoQuotaSummaryResponse, RepoResponse,
        RepoWithStatsResponse, RescanResponse, SyncResponse,
    },
    types::{BORG_REPO_ENV_KEY, BorgEncryption, SystemEventType, build_repo_url},
};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

mod archive_metadata;
mod archive_stats;
mod borg_ops;
mod indexing;
mod sync;
#[cfg(test)]
mod test_helpers;
mod text_limits;

pub(super) use self::borg_ops::verify_repo_access;
pub use self::sync::{
    clear_import_progress_state, finish_server_sync_task, set_server_sync_op,
    sync_existing_archives, sync_new_archives,
};
use self::{
    borg_ops::{
        BorgInfoResult, get_borg_timeout, is_lock_error, run_borg_break_lock, run_borg_info,
        run_borg_info_with_retry, run_borg_init,
    },
    indexing::index_archives_with_progress,
    sync::clear_server_sync_op,
    text_limits::RepoTextFields,
};
use super::{
    auth::{AuthUser, RequireAdmin},
    helpers::{self, DomainQuery},
    permissions::is_visible_to_user,
};
use crate::{
    AppState, RepoLock,
    borg::Borg,
    config_assembler,
    db::{self, InsertRepoParams, RepoRow, RepoWithStatsRow, UpdateRepoParams},
    error::{ApiError, ApiJson},
    ssh::shell_escape,
    ws::ui_broadcast::UiBroadcast,
};

impl From<RepoRow> for RepoResponse {
    fn from(row: RepoRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            repo_path: row.repo_path,
            ssh_user: row.ssh_user,
            ssh_host: row.ssh_host,
            ssh_port: row.ssh_port,
            compression: row.compression,
            encryption: row.encryption,
            enabled: row.enabled,
            owner_id: row.owner_id,
            visibility: row.visibility,
            sync_schedule: row.sync_schedule,
            repo_host_id: row.repo_host_id,
            machine_id: row.machine_id,
            power: shared::responses::HostWakeSettingsResponse {
                wake_enabled: row.wake_enabled,
                wake_mac_address: row.wake_mac_address,
                wake_broadcast_address: row.wake_broadcast_address,
                wake_timeout_seconds: row.wake_timeout_seconds,
                shutdown_after_backup: row.shutdown_after_backup,
            },
        }
    }
}

impl From<RepoWithStatsRow> for RepoWithStatsResponse {
    fn from(row: RepoWithStatsRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            repo_path: row.repo_path,
            ssh_user: row.ssh_user,
            ssh_host: row.ssh_host,
            ssh_port: row.ssh_port,
            ssh_host_key: row.ssh_host_key,
            compression: row.compression,
            encryption: row.encryption,
            enabled: row.enabled,
            importing: row.importing,
            import_error: row.import_error,
            import_progress: row.import_progress,
            import_total: row.import_total,
            import_status_message: row.import_status_message,
            owner_id: row.owner_id,
            visibility: row.visibility,
            sync_schedule: row.sync_schedule,
            last_synced_at: row.last_synced_at,
            archive_count: row.archive_count,
            last_backup_at: row.last_backup_at,
            total_original_size: row.total_original_size,
            total_compressed_size: row.total_compressed_size,
            total_deduplicated_size: row.total_deduplicated_size,
            agent_count: row.agent_count,
            unmatched_count: row.unmatched_count,
            relocation_pending: row.relocation_pending,
            last_op_kind: row.last_op_kind,
            last_op_at: row.last_op_at,
            last_op_by: row.last_op_by,
            current_op: None,
            // The quota columns come from a LEFT JOIN: all set when the repo
            // has a quota row, all NULL when it has none.
            quota: match (
                row.quota_enabled,
                row.quota_warn_action,
                row.quota_critical_action,
            ) {
                (Some(enabled), Some(warn_action), Some(critical_action)) => {
                    Some(RepoQuotaSummaryResponse {
                        warn_bytes: row.quota_warn_bytes,
                        critical_bytes: row.quota_critical_bytes,
                        warn_action,
                        critical_action,
                        enabled,
                    })
                }
                _ => None,
            },
            repo_host: shared::responses::RepoHostRefResponse {
                id: row.repo_host_id,
                machine_id: row.host_machine_id,
                intermittent: row.host_intermittent,
            },
            power: shared::responses::HostWakeSettingsResponse {
                wake_enabled: row.wake_enabled,
                wake_mac_address: row.wake_mac_address,
                wake_broadcast_address: row.wake_broadcast_address,
                wake_timeout_seconds: row.wake_timeout_seconds,
                shutdown_after_backup: row.shutdown_after_backup,
            },
        }
    }
}

/// A new repository on a host that is already known joins it: it has to use
/// the host's port, and the key the host presents now has to be the one
/// already pinned for it. A different key is refused rather than pinned over
/// the old one, which would silently re-trust every other repository on the
/// host - accepting a new key is a decision made on the host, deliberately.
async fn check_known_repo_host(
    pool: &PgPool,
    ssh_host: &str,
    ssh_port: i32,
    scanned_key: &str,
) -> Result<(), ApiError> {
    let Some(host) = db::repo_hosts::find_repo_host_by_name(pool, ssh_host).await? else {
        return Ok(());
    };
    if host.ssh_port != ssh_port {
        return Err(ApiError::BadRequest(format!(
            "repository host {ssh_host} uses SSH port {}; use that port, or change it on the host",
            host.ssh_port
        )));
    }
    match host.ssh_host_key.as_deref() {
        Some(pinned) if pinned != scanned_key => Err(db::repo_hosts::host_key_mismatch(ssh_host)),
        Some(_) | None => Ok(()),
    }
}

/// An edit that moves a repository onto another host is held to the same
/// rule as adding one there: a known host that has a key pinned must present
/// that key now, or the move is refused - otherwise re-pointing a repository
/// would be a way around it. `scan` is only called for such a host, so a move
/// to a new host, or to one with no key yet, costs no connection and pins on
/// first use as before.
async fn check_move_to_repo_host<F, Fut>(
    pool: &PgPool,
    ssh_host: &str,
    ssh_port: i32,
    scan: F,
) -> Result<(), ApiError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, ApiError>>,
{
    let Some(host) = db::repo_hosts::find_repo_host_by_name(pool, ssh_host).await? else {
        return Ok(());
    };
    if host.ssh_host_key.is_none() {
        return Ok(());
    }
    let scanned_key = scan().await?;
    check_known_repo_host(pool, ssh_host, ssh_port, &scanned_key).await
}

#[utoipa::path(
    get,
    path = "/api/repos",
    tag = "Repositories",
    operation_id = "listRepos",
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
/// List all repositories.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_repos(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepoResponse>>, ApiError> {
    let repos = db::list_all_repos(&state.pool).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let is_admin = effective.can_delete_repo;
    let can_view_wake_secrets = effective.can_view_wake_secrets();
    let mut visible = Vec::with_capacity(repos.len());
    for repo in repos {
        if is_visible_to_user(
            &state.pool,
            auth.user_id,
            repo.owner_id,
            repo.visibility,
            is_admin,
        )
        .await?
        {
            let mut response = RepoResponse::from(repo);
            if !can_view_wake_secrets {
                response.power.wake_mac_address = None;
                response.power.wake_broadcast_address = None;
            }
            visible.push(response);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/repos",
    tag = "Repositories",
    operation_id = "getAgentRepos",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
        ("domain" = Option<String>, Query, description = "Required if the hostname is ambiguous"),
    ),
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Hostname is ambiguous; specify a domain"),
    )
)]
/// List repositories for a specific host.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_agent_repos(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    Query(query): Query<DomainQuery>,
) -> Result<Json<Vec<RepoResponse>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname, query.domain.as_deref()).await?;
    let repos = db::list_repos_for_agent_public(&state.pool, agent.id).await?;
    Ok(Json(repos.into_iter().map(RepoResponse::from).collect()))
}

/// Request payload for adding an existing borg repository.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRepoRequest {
    /// Display name for the repository.
    pub name: String,
    /// Remote path on the SSH host.
    pub repo_path: String,
    /// SSH user (defaults to "borg").
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    /// SSH hostname or IP.
    pub ssh_host: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<i32>,
    /// Repository passphrase.
    pub passphrase: String,
    /// Compression algorithm (defaults to "lz4").
    pub compression: Option<String>,
}

/// Request payload for updating a repository.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRepoRequest {
    /// New display name.
    pub name: Option<String>,
    /// Remote path on the SSH host.
    pub repo_path: String,
    /// SSH user (defaults to "borg").
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    /// SSH hostname or IP.
    pub ssh_host: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<i32>,
    /// Compression algorithm.
    pub compression: Option<String>,
    /// Encryption mode.
    #[schema(value_type = Option<String>)]
    pub encryption: Option<BorgEncryption>,
    /// Whether the repository is enabled.
    pub enabled: Option<bool>,
    /// Sync schedule cron expression (None = disable).
    pub sync_schedule: Option<Option<String>>,
}

#[utoipa::path(
    post,
    path = "/api/repos",
    tag = "Repositories",
    operation_id = "createRepo",
    request_body = CreateRepoRequest,
    responses(
        (status = 201, description = "Repository created", body = RepoResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
/// Create a new repository.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::BadGateway`]: the upstream operation (e.g. SSH or borg) fails
pub async fn create_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateRepoRequest>,
) -> Result<(StatusCode, Json<RepoResponse>), ApiError> {
    helpers::validate_non_empty(&req.name, "name")?;
    helpers::validate_non_empty(&req.repo_path, "repo_path")?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    RepoTextFields::from(&req).validate()?;

    let ssh_port = req.ssh_port.unwrap_or(22);
    let ssh_port_u16 = u16::try_from(ssh_port)
        .map_err(|_| ApiError::BadRequest("ssh_port out of range".into()))?;
    helpers::validate_path_exists(&req.ssh_host, &req.ssh_user, ssh_port_u16, &req.repo_path)
        .await?;
    let ssh_host_key = crate::ssh::scan_host_key(&req.ssh_host, ssh_port_u16)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;
    check_known_repo_host(&state.pool, &req.ssh_host, ssh_port, &ssh_host_key).await?;

    let repo_url = build_repo_url(&req.ssh_user, &req.ssh_host, ssh_port_u16, &req.repo_path);

    let info_timeout = Duration::from_secs(30);
    let info_result: Option<BorgInfoResult> = match tokio::time::timeout(
        info_timeout,
        run_borg_info(&repo_url, &req.passphrase, &state.task_registry),
    )
    .await
    {
        Err(_) => None,
        Ok(Ok(info)) => Some(info),
        Ok(Err(ApiError::BadGateway(ref msg))) if is_lock_error(msg) => None,
        Ok(Err(e)) => return Err(e),
    };

    let encryption = info_result
        .as_ref()
        .map_or("unknown".to_string(), |info| info.encryption.to_string());

    let passphrase_encrypted = encrypt_passphrase(&req.passphrase, &state.encryption_key)?;
    let compression = helpers::validate_compression(req.compression.as_deref())?;

    let repo = db::insert_repo_pinning_host_key(
        &state.pool,
        &InsertRepoParams {
            name: &req.name,
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            ssh_port,
            passphrase_encrypted: &passphrase_encrypted,
            compression: &compression,
            encryption: &encryption,
            owner_id: None,
            sync_schedule: None,
        },
        &ssh_host_key,
    )
    .await?;

    let repo_id = repo.id;
    let pool = state.pool.clone();
    let encryption_key = state.encryption_key;
    let ui_broadcast = state.ui_broadcast.clone();
    let state_repo_lock = state.repo_lock.clone();
    let need_borg_info = info_result.is_none();
    let bg_repo_url = repo_url.clone();
    let bg_passphrase = req.passphrase.clone();
    let bg_ssh_host = req.ssh_host.clone();
    let bg_ssh_host_key = ssh_host_key.clone();
    let task_state = state.clone();

    db::set_repo_importing(&state.pool, repo_id, true).await?;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
    let (task_id, cancel) = state.import_tasks.start(repo_id).await;

    // Tracked like the stat-enrichment spawn below: this task acquires the
    // repo lock and runs borg via sync_existing_archives, so a test that
    // returns without waiting for it leaves it running into whatever test
    // comes next.
    let import = run_initial_import_task(InitialImportTask {
        pool,
        encryption_key,
        ui_broadcast,
        repo_lock: state_repo_lock,
        task_state,
        repo_id,
        task_id,
        cancel,
        need_borg_info,
        bg_repo_url,
        bg_passphrase,
        bg_ssh_host,
        bg_ssh_port: ssh_port_u16,
        bg_ssh_host_key,
    });
    state.background_task_tracker.spawn_tracked(import);

    Ok((StatusCode::CREATED, Json(RepoResponse::from(repo))))
}

struct InitialImportTask {
    pool: PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: UiBroadcast,
    repo_lock: RepoLock,
    task_state: AppState,
    repo_id: i64,
    task_id: u64,
    cancel: CancellationToken,
    need_borg_info: bool,
    bg_repo_url: String,
    bg_passphrase: String,
    bg_ssh_host: String,
    bg_ssh_port: u16,
    bg_ssh_host_key: String,
}

/// Arguments for [`run_initial_import_work`], bundled (rather than passed
/// individually) to stay under clippy's argument-count limit.
struct InitialImportWork<'a> {
    pool: &'a PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: &'a UiBroadcast,
    state_repo_lock: RepoLock,
    task_state: &'a AppState,
    repo_id: i64,
    task_id: u64,
    need_borg_info: bool,
    bg_repo_url: &'a str,
    bg_passphrase: &'a str,
    bg_ssh_host: &'a str,
    bg_ssh_port: u16,
    bg_ssh_host_key: &'a str,
}

/// The actual sync work `run_initial_import_task` races against
/// cancellation: deferred encryption detection (if `borg info` couldn't be
/// reached synchronously during creation), the initial archive sync, and
/// content indexing. Split out so the outer task stays under clippy's
/// function-length limit.
async fn run_initial_import_work(work: InitialImportWork<'_>) {
    let InitialImportWork {
        pool,
        encryption_key,
        ui_broadcast,
        state_repo_lock,
        task_state,
        repo_id,
        task_id,
        need_borg_info,
        bg_repo_url,
        bg_passphrase,
        bg_ssh_host,
        bg_ssh_port,
        bg_ssh_host_key,
    } = work;

    // Detect encryption before syncing rather than concurrently: both borg
    // info and the sync's borg list contend for the same repository lock, so
    // running them in parallel would force the list into the lock-retry path.
    if need_borg_info {
        let timeout = get_borg_timeout(pool).await;
        // The repo row (and its ssh_host_key) was already persisted before this
        // background task was spawned, so - unlike the synchronous attempt in
        // create_repo, which runs before any repo exists to pin against - this
        // deferred retry can and should pin against the accepted host key.
        let env = helpers::borg_env_for_repo(
            bg_passphrase,
            repo_id,
            bg_ssh_host,
            bg_ssh_port,
            Some(bg_ssh_host_key),
        )
        .await;
        match run_borg_info_with_retry(bg_repo_url, &env, timeout, &task_state.task_registry).await
        {
            Ok(info) => {
                if let Err(e) =
                    db::update_repo_encryption(pool, repo_id, &info.encryption.to_string()).await
                {
                    warn!(
                        repo_id,
                        error = %e,
                        "failed to update encryption after deferred borg info"
                    );
                }
            }
            Err(e) => {
                warn!(
                    repo_id,
                    error = %e,
                    "deferred borg info failed, continuing to archive sync"
                );
            }
        }
    }

    let _import_lock = state_repo_lock.acquire(repo_id).await;
    let sync_ok = match sync_existing_archives(
        pool,
        &encryption_key,
        repo_id,
        ui_broadcast,
        &task_state.background_task_tracker,
        &task_state.task_registry,
    )
    .await
    {
        Ok(_) => {
            if let Err(e) = db::update_repo_last_synced(pool, repo_id).await {
                warn!(
                    repo_id,
                    error = %e,
                    "failed to set last_synced_at after initial import"
                );
            }
            true
        }
        Err(e) => {
            warn!(repo_id, error = %e, "failed to sync existing archives on import");
            if task_state.import_tasks.is_current(repo_id, task_id).await {
                let _ = db::set_repo_import_error(pool, repo_id, Some(&format!("{e}"))).await;
            }
            false
        }
    };

    if sync_ok && task_state.import_tasks.is_current(repo_id, task_id).await {
        index_archives_with_progress(
            pool.clone(),
            encryption_key,
            repo_id,
            ui_broadcast.clone(),
            state_repo_lock,
            false,
            task_state.task_registry.clone(),
        )
        .await;
    }

    if task_state.import_tasks.is_current(repo_id, task_id).await {
        if let Err(e) = db::set_repo_importing(pool, repo_id, false).await {
            warn!(repo_id, error = %e, "failed to clear importing flag");
        }
        clear_import_progress_state(pool, ui_broadcast, repo_id).await;
        ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
    }
}

/// Runs the background work kicked off when a repository is first created:
/// deferred encryption detection (if `borg info` couldn't be reached
/// synchronously during creation), the initial archive sync, and content
/// indexing.
async fn run_initial_import_task(task: InitialImportTask) {
    let InitialImportTask {
        pool,
        encryption_key,
        ui_broadcast,
        repo_lock: state_repo_lock,
        task_state,
        repo_id,
        task_id,
        cancel,
        need_borg_info,
        bg_repo_url,
        bg_passphrase,
        bg_ssh_host,
        bg_ssh_port,
        bg_ssh_host_key,
    } = task;

    let op_clear_guard = set_server_sync_op(&task_state, repo_id).await;
    tokio::select! {
        () = cancel.cancelled() => {
            info!(repo_id, "initial import cancelled");
        }
        () = run_initial_import_work(InitialImportWork {
            pool: &pool,
            encryption_key,
            ui_broadcast: &ui_broadcast,
            state_repo_lock,
            task_state: &task_state,
            repo_id,
            task_id,
            need_borg_info,
            bg_repo_url: &bg_repo_url,
            bg_passphrase: &bg_passphrase,
            bg_ssh_host: &bg_ssh_host,
            bg_ssh_port,
            bg_ssh_host_key: &bg_ssh_host_key,
        }) => {}
    }

    finish_server_sync_task(
        &task_state.import_tasks,
        &ui_broadcast,
        repo_id,
        task_id,
        op_clear_guard,
    )
    .await;
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "updateRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = UpdateRepoRequest,
    responses(
        (status = 200, description = "Updated repository", body = RepoResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Update a repository (admin only).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn update_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRepoRequest>,
) -> Result<Json<RepoResponse>, ApiError> {
    helpers::validate_non_empty(&req.repo_path, "repo_path")?;
    if let Some(ref n) = req.name {
        helpers::validate_non_empty(n, "name")?;
    }
    RepoTextFields::from(&req).validate()?;

    let compression = helpers::validate_compression(req.compression.as_deref())?;

    let existing = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let location_changed = existing.repo_path != req.repo_path
        || existing.ssh_host != req.ssh_host
        || existing.ssh_port != req.ssh_port.unwrap_or(22);

    let encryption = req
        .encryption
        .map_or_else(|| "repokey-blake2".to_string(), |e| e.to_string());

    let name = req.name.unwrap_or(existing.name);
    let ssh_port = req.ssh_port.unwrap_or(22);
    if existing.ssh_host != req.ssh_host {
        let ssh_port_u16 = u16::try_from(ssh_port)
            .map_err(|_| ApiError::BadRequest("ssh_port out of range".into()))?;
        check_move_to_repo_host(&state.pool, &req.ssh_host, ssh_port, || async {
            crate::ssh::scan_host_key(&req.ssh_host, ssh_port_u16)
                .await
                .map_err(|e| ApiError::BadGateway(e.to_string()))
        })
        .await?;
    }

    let update_params = UpdateRepoParams {
        repo_id,
        name: &name,
        repo_path: &req.repo_path,
        ssh_user: &req.ssh_user,
        ssh_host: &req.ssh_host,
        ssh_port,
        compression: &compression,
        encryption: &encryption,
        enabled: req.enabled.unwrap_or(true),
        sync_schedule: req.sync_schedule.as_ref().map(|v| v.as_deref()),
    };

    let repo = if location_changed {
        db::update_repo_and_set_relocation_pending(&state.pool, &update_params).await?
    } else {
        db::update_repo(&state.pool, &update_params).await?
    };

    Ok(Json(RepoResponse::from(repo)))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "deleteRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Remove a repository from the database (admin only).
///
/// Removes the repository record and associated schedules/reports from the database. Does NOT
/// delete any data on disk.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::delete_repo(&state.pool, repo_id).await?;
    crate::ssh::remove_pinned_known_hosts(repo_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/destroy",
    tag = "Repositories",
    operation_id = "destroyRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 204, description = "Destroyed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Failed to delete from disk"),
    )
)]
/// Destroy a repository from disk and remove from database (admin only).
///
/// DANGEROUS: Permanently deletes the repository data from the remote filesystem via SSH (rm -rf)
/// and then removes the database record. This action is irreversible.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn destroy_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let conn = db::get_repo_connection(&state.pool, repo_id).await?;
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;

    let command = format!("rm -rf '{}'", repo.repo_path.replace('\'', "'\\''"));
    info!(
        repo_id,
        repo_name = %repo.name,
        ssh_host = %conn.ssh_host,
        path = %repo.repo_path,
        "destroying repository from disk"
    );

    run_ssh_command(
        &conn.ssh_user,
        &conn.ssh_host,
        conn.ssh_port,
        &command,
        repo.ssh_host_key.clone(),
    )
    .await
    .map_err(|e| {
        error!(repo_id, error = %e, "failed to destroy repository on disk");
        ApiError::Internal(format!("failed to delete repository from disk: {e}"))
    })?;

    db::delete_repo(&state.pool, repo_id).await?;
    crate::ssh::remove_pinned_known_hosts(repo_id).await;

    info!(repo_id, "repository destroyed successfully");
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/passphrase",
    tag = "Repositories",
    operation_id = "getRepoPassphrase",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Passphrase", body = PassphraseResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Get the decrypted passphrase for a repository (admin only).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_passphrase(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<PassphraseResponse>, ApiError> {
    let encrypted = db::get_repo_passphrase(&state.pool, repo_id).await?;
    let passphrase = shared::crypto::decrypt_passphrase(&encrypted, &state.encryption_key)?;
    Ok(Json(PassphraseResponse { passphrase }))
}

#[utoipa::path(
    get,
    path = "/api/repos/stats",
    tag = "Repositories",
    operation_id = "listReposWithStats",
    responses(
        (status = 200, description = "Repositories with stats", body = Vec<RepoWithStatsResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
/// List repositories with backup statistics.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_repos_with_stats(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepoWithStatsResponse>>, ApiError> {
    let repos = db::list_repos_with_stats(&state.pool).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let is_admin = effective.can_delete_repo;
    // Quota configuration is otherwise gated to operators/admins (see
    // `require_operator_or_admin` in api/quota.rs) - strip it here too, so embedding it on the
    // list response doesn't leak it to a viewer who can see the repo but not its quota.
    let can_view_quota = effective.can_delete_repo || effective.can_view_all_repos;
    let mut visible = Vec::with_capacity(repos.len());
    for repo in repos {
        if is_visible_to_user(
            &state.pool,
            auth.user_id,
            repo.owner_id,
            repo.visibility,
            is_admin,
        )
        .await?
        {
            let mut response = RepoWithStatsResponse::from(repo);
            if !can_view_quota {
                response.quota = None;
                // Wake-on-LAN's MAC/broadcast address let anyone who has
                // them power the host on remotely, so they're gated to the
                // same audience as quota rather than embedded for any
                // viewer who can merely see the repo.
                response.power.wake_mac_address = None;
                response.power.wake_broadcast_address = None;
            }
            visible.push(response);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "getRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Repository with stats", body = RepoWithStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Get a repository with statistics.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoWithStatsResponse>, ApiError> {
    let mut res: RepoWithStatsResponse =
        db::get_repo_with_stats(&state.pool, repo_id).await?.into();
    res.current_op = state.repo_op_tracker.get(repo_id).await;
    // Quota configuration is otherwise gated to operators/admins (see
    // `require_operator_or_admin` in api/quota.rs) - strip it here too, so embedding it on this
    // response doesn't leak it to a viewer who can see the repo but not its quota.
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if !(effective.can_delete_repo || effective.can_view_all_repos) {
        res.quota = None;
        res.power.wake_mac_address = None;
        res.power.wake_broadcast_address = None;
    }
    Ok(Json(res))
}

/// Request payload for initializing a new borg repository.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InitRepoRequest {
    /// Display name for the repository.
    pub name: String,
    /// Remote path on the SSH host.
    pub repo_path: String,
    /// SSH user (defaults to "borg").
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    /// SSH hostname or IP.
    pub ssh_host: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<i32>,
    /// Repository passphrase.
    pub passphrase: String,
    /// Encryption mode for the new repository.
    #[schema(value_type = String)]
    pub encryption: BorgEncryption,
    /// Compression algorithm (defaults to "lz4").
    pub compression: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/init",
    tag = "Repositories",
    operation_id = "initRepo",
    request_body = InitRepoRequest,
    responses(
        (status = 201, description = "Repository initialized", body = InitRepoResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Repository already exists"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Initialize a new borg repository and register it.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::BadGateway`]: the upstream operation (e.g. SSH or borg) fails
pub async fn init_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<InitRepoRequest>,
) -> Result<(StatusCode, Json<InitRepoResponse>), ApiError> {
    helpers::validate_non_empty(&req.name, "name")?;
    helpers::validate_non_empty(&req.repo_path, "repo_path")?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    RepoTextFields::from(&req).validate()?;

    let ssh_port = req.ssh_port.unwrap_or(22);
    let ssh_port_u16 = u16::try_from(ssh_port)
        .map_err(|_| ApiError::BadRequest("ssh_port out of range".into()))?;
    let ssh_host_key = crate::ssh::scan_host_key(&req.ssh_host, ssh_port_u16)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;
    check_known_repo_host(&state.pool, &req.ssh_host, ssh_port, &ssh_host_key).await?;
    let repo_url = build_repo_url(&req.ssh_user, &req.ssh_host, ssh_port_u16, &req.repo_path);

    let env = helpers::borg_base_env(&req.passphrase);
    let borg_output = run_borg_init(&repo_url, &env, req.encryption, &state.task_registry).await?;

    let passphrase_encrypted = encrypt_passphrase(&req.passphrase, &state.encryption_key)?;
    let compression = helpers::validate_compression(req.compression.as_deref())?;

    let encryption = req.encryption.to_string();

    let repo = db::insert_repo_pinning_host_key(
        &state.pool,
        &InsertRepoParams {
            name: &req.name,
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            ssh_port,
            passphrase_encrypted: &passphrase_encrypted,
            compression: &compression,
            encryption: &encryption,
            owner_id: None,
            sync_schedule: None,
        },
        &ssh_host_key,
    )
    .await?;

    info!(repo_id = repo.id, name = %req.name, "repository initialized");

    Ok((
        StatusCode::CREATED,
        Json(InitRepoResponse {
            repo: repo.into(),
            borg_output,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/confirm-relocation",
    tag = "Repositories",
    operation_id = "confirmRepoRelocation",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Relocation accepted", body = ConfirmRelocationResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Accept a borg repository relocation for the next backup run.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn confirm_relocation(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<ConfirmRelocationResponse>, ApiError> {
    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    db::set_relocation_pending(&state.pool, repo_id).await?;
    info!(repo_id, name = %repo.name, "relocation confirmation set");

    let targets = db::get_schedule_target_agents_for_repo(&state.pool, repo_id).await?;
    for target in &targets {
        config_assembler::push_config_to_agent(&state, target.agent_id).await;
    }

    Ok(Json(ConfirmRelocationResponse {
        message: format!(
            "Relocation accepted for '{}'. The next backup will set \
             BORG_RELOCATED_REPO_ACCESS_IS_OK=yes.",
            repo.name
        ),
    }))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/schedules",
    tag = "Repositories",
    operation_id = "listSchedulesForRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "List of schedules", body = Vec<crate::db::ScheduleRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List schedules for a repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedules_for_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<db::ScheduleRow>>, ApiError> {
    let schedules = db::list_schedules_for_repo(&state.pool, repo_id).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let is_admin = effective.can_delete_repo;
    let mut visible = Vec::with_capacity(schedules.len());
    for s in schedules {
        if super::permissions::is_visible_to_user(
            &state.pool,
            auth.user_id,
            s.owner_id,
            s.visibility,
            is_admin,
        )
        .await?
        {
            visible.push(s);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/break-lock",
    tag = "Repositories",
    operation_id = "breakRepoLock",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Lock broken", body = BreakLockResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Break a stale lock on a borg repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn break_lock(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<BreakLockResponse>, ApiError> {
    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let passphrase =
        shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, &state.encryption_key)?;

    let repo_url = build_repo_url(
        &repo.ssh_user,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        &repo.repo_path,
    );

    state
        .repo_op_tracker
        .set(
            repo_id,
            shared::protocol::RepoOpKind::BreakLock,
            "server".to_owned(),
            None,
        )
        .await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });

    let result = run_borg_break_lock(
        repo_id,
        &repo_url,
        &passphrase,
        &state.task_registry,
        &state.repo_lock,
    )
    .await;

    state.repo_op_tracker.clear(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });

    let borg_output = result?;

    let now = chrono::Utc::now();
    if let Err(e) = db::update_repo_last_op(
        &state.pool,
        repo_id,
        shared::protocol::RepoOpKind::BreakLock,
        now,
        "server",
    )
    .await
    {
        warn!(repo_id, error = %e, "failed to persist last_op for break_lock");
    }

    info!(repo_id, name = %repo.name, "repository lock broken");

    Ok(Json(BreakLockResponse {
        message: format!("lock broken on repository '{}'", repo.name),
        borg_output,
    }))
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
enum BorgSubcommand {
    #[strum(serialize = "info")]
    Info,
    #[strum(serialize = "list")]
    List,
    #[strum(serialize = "check")]
    Check,
    #[strum(serialize = "compact")]
    Compact,
    #[strum(serialize = "prune")]
    Prune,
    #[strum(serialize = "delete")]
    Delete,
    #[strum(serialize = "diff")]
    Diff,
    #[strum(serialize = "rename")]
    Rename,
    #[strum(serialize = "recreate")]
    Recreate,
}

impl BorgSubcommand {
    const ALL: [BorgSubcommand; 9] = [
        BorgSubcommand::Info,
        BorgSubcommand::List,
        BorgSubcommand::Check,
        BorgSubcommand::Compact,
        BorgSubcommand::Prune,
        BorgSubcommand::Delete,
        BorgSubcommand::Diff,
        BorgSubcommand::Rename,
        BorgSubcommand::Recreate,
    ];

    fn as_str(self) -> &'static str {
        (&self).into()
    }

    fn permitted_list() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Request payload for executing an ad-hoc borg command.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ExecBorgRequest {
    /// Borg subcommand and arguments (e.g. `["info", "--json"]`).
    pub args: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/exec",
    tag = "Repositories",
    operation_id = "execBorgCommand",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = ExecBorgRequest,
    responses(
        (status = 200, description = "Command output", body = ExecBorgResponse),
        (status = 400, description = "Invalid subcommand or arguments"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Not found"),
    )
)]
/// Execute a borg command against the repository (admin only).
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn exec_borg(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<ExecBorgRequest>,
) -> Result<Json<ExecBorgResponse>, ApiError> {
    let raw_subcommand = req
        .args
        .first()
        .ok_or_else(|| ApiError::BadRequest("args must not be empty".to_owned()))?;

    let subcommand = raw_subcommand.parse::<BorgSubcommand>().map_err(|_| {
        ApiError::BadRequest(format!(
            "subcommand '{raw_subcommand}' is not allowed; permitted: {}",
            BorgSubcommand::permitted_list()
        ))
    })?;

    if req.args.len() > 32 {
        return Err(ApiError::BadRequest("too many arguments".to_owned()));
    }

    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let passphrase =
        shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, &state.encryption_key)?;

    let repo_url = build_repo_url(
        &repo.ssh_user,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        &repo.repo_path,
    );

    let mut env = helpers::borg_env_for_repo(
        &passphrase,
        repo_id,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        repo.ssh_host_key.as_deref(),
    )
    .await;
    env.insert(BORG_REPO_ENV_KEY.to_owned(), repo_url);

    info!(repo_id, name = %repo.name, subcommand = %subcommand, "admin executing borg command");

    // Serializes against every other operation that touches this repo's local borg cache
    // (backup, delete, import, rescan, reset & sync, and Break Lock's own cache-lock
    // clearing) - an ad-hoc admin command like `compact`/`prune`/`recreate` genuinely uses
    // the cache exactly like those do, and without this an unrelated concurrent operation
    // could corrupt state.
    let _repo_lock_guard = state.repo_lock.acquire(repo_id).await;

    let output = Borg::new()
        .with_registry(state.task_registry.clone())
        .run(&req.args, &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok(Json(ExecBorgResponse {
        stdout,
        stderr,
        exit_code,
    }))
}

/// Request payload for migrating a repository's encryption mode.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MigrateEncryptionRequest {
    /// Target encryption mode.
    #[schema(value_type = String)]
    pub target_encryption: BorgEncryption,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/migrate-encryption",
    tag = "Repositories",
    operation_id = "migrateRepoEncryption",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = MigrateEncryptionRequest,
    responses(
        (status = 200, description = "Migration completed", body = MigrateEncryptionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Migration failed"),
    )
)]
/// Migrate repository to a different encryption mode.
///
/// Renames the existing repository and creates a new one at the original path with the target
/// encryption. Old repo preserved at .migrated-<date> path.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::BadGateway`]: the upstream operation (e.g. SSH or borg) fails
pub async fn migrate_encryption(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<MigrateEncryptionRequest>,
) -> Result<Json<MigrateEncryptionResponse>, ApiError> {
    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let passphrase =
        shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, &state.encryption_key)?;

    let current_encryption = repo.encryption;

    if current_encryption == req.target_encryption {
        return Err(ApiError::BadRequest(
            "repository already uses the target encryption mode".to_owned(),
        ));
    }

    let repo_url = build_repo_url(
        &repo.ssh_user,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        &repo.repo_path,
    );

    // Serializes against every other operation that touches this repo's local borg cache
    // (backup, delete, import, rescan, reset & sync, Break Lock's cache-lock clearing) for
    // the whole migration, not just the `borg init` call below - the SSH rename that
    // precedes it already mutates the repository a concurrent operation could be using.
    let _repo_lock_guard = state.repo_lock.acquire(repo_id).await;

    let migrated_path = rename_and_reinit_repo(RenameAndReinitParams {
        state: &state,
        repo_id,
        repo: &repo,
        repo_url: &repo_url,
        passphrase: &passphrase,
        target_encryption: req.target_encryption,
    })
    .await?;

    db::update_repo_encryption(&state.pool, repo_id, req.target_encryption.as_borg_arg()).await?;

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            event: AuditEvent::MigrateEncryption {
                from: current_encryption,
                to: req.target_encryption,
                migrated_path: migrated_path.clone(),
            },
            target_type: Some("repo"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    info!(
        repo_id,
        encryption = req.target_encryption.as_borg_arg(),
        "encryption migration completed"
    );

    Ok(Json(MigrateEncryptionResponse {
        success: true,
        message: format!(
            "migrated to {}; old repo preserved at {migrated_path}",
            req.target_encryption.as_borg_arg()
        ),
        migrated_path: Some(migrated_path),
    }))
}

/// Arguments for [`rename_and_reinit_repo`], bundled (rather than passed
/// individually) to stay under clippy's argument-count limit.
struct RenameAndReinitParams<'a> {
    state: &'a AppState,
    repo_id: i64,
    repo: &'a db::RepoWithPassphraseRow,
    repo_url: &'a str,
    passphrase: &'a str,
    target_encryption: BorgEncryption,
}

/// Renames the repository's on-disk directory to a `.migrated-<date>` path
/// and runs `borg init` at the original path with the target encryption,
/// rolling the rename back if `borg init` fails. Returns the migrated path
/// on success.
async fn rename_and_reinit_repo(params: RenameAndReinitParams<'_>) -> Result<String, ApiError> {
    let RenameAndReinitParams {
        state,
        repo_id,
        repo,
        repo_url,
        passphrase,
        target_encryption,
    } = params;

    let date_suffix = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let migrated_path = format!("{}.migrated-{date_suffix}", repo.repo_path);

    run_ssh_command(
        &repo.ssh_user,
        &repo.ssh_host,
        repo.ssh_port,
        &format!(
            "mv {} {}",
            shell_escape(&repo.repo_path),
            shell_escape(&migrated_path)
        ),
        repo.ssh_host_key.clone(),
    )
    .await
    .map_err(|e| ApiError::BadGateway(format!("failed to rename old repository: {e}")))?;

    info!(repo_id, %migrated_path, "renamed old repo for migration");

    let env = helpers::borg_env_for_repo(
        passphrase,
        repo_id,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        repo.ssh_host_key.as_deref(),
    )
    .await;

    if let Err(e) = run_borg_init(repo_url, &env, target_encryption, &state.task_registry).await {
        warn!(repo_id, %e, "borg init failed during migration, rolling back");
        let _ = run_ssh_command(
            &repo.ssh_user,
            &repo.ssh_host,
            repo.ssh_port,
            &format!(
                "mv {} {}",
                shell_escape(&migrated_path),
                shell_escape(&repo.repo_path)
            ),
            repo.ssh_host_key.clone(),
        )
        .await;
        return Err(ApiError::BadGateway(format!(
            "borg init failed during migration (rolled back): {e}"
        )));
    }

    Ok(migrated_path)
}

async fn run_ssh_command(
    ssh_user: &str,
    ssh_host: &str,
    ssh_port: i32,
    command: &str,
    expected_host_key: Option<String>,
) -> Result<(), String> {
    let port = u16::try_from(ssh_port).map_err(|e| format!("invalid SSH port: {e}"))?;

    let session = crate::ssh::connect_with_key(ssh_host, ssh_user, port, expected_host_key)
        .await
        .map_err(|e| e.to_string())?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        crate::ssh::exec_command(&session, command),
    )
    .await
    .map_err(|_| "SSH command timed out after 30 seconds".to_owned())?
    .map_err(|e| e.to_string())?;

    let (exit_code, _stdout, stderr) = result;
    if exit_code != 0 {
        return Err(stderr);
    }

    Ok(())
}

#[derive(sqlx::FromRow)]
struct UnmatchedRow {
    report_id: i64,
    hostname: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/rescan",
    tag = "Repositories",
    operation_id = "rescanRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Rescan results", body = RescanResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Re-scan unmatched archives against hostname patterns.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn rescan_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<RescanResponse>, ApiError> {
    db::get_repo_with_stats(&state.pool, repo_id).await?;

    let unmatched_rows = sqlx::query_as!(
        UnmatchedRow,
        "SELECT br.id AS report_id, c.hostname FROM backup_reports br JOIN agents c ON c.id = \
         br.agent_id WHERE br.repo_id = $1 AND br.matched = false",
        repo_id,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let mut matched_count = 0u64;

    for row in &unmatched_rows {
        let result = db::resolve_agent_for_hostname(&state.pool, &row.hostname).await?;
        let new_agent_id = match result {
            db::ResolveResult::ExactMatch(c) | db::ResolveResult::PatternMatch(c) => Some(c.id),
            db::ResolveResult::Unmatched => None,
        };

        if let Some(agent_id) = new_agent_id {
            sqlx::query!(
                "UPDATE backup_reports SET agent_id = $1, matched = true WHERE id = $2",
                agent_id,
                row.report_id,
            )
            .execute(&state.pool)
            .await
            .map_err(ApiError::Database)?;
            matched_count = matched_count.saturating_add(1);
        }
    }

    sqlx::query!(
        "DELETE FROM agents WHERE agent_token_hash = 'imported:no-auth' AND NOT EXISTS (SELECT 1 \
         FROM backup_reports WHERE agent_id = agents.id)",
    )
    .execute(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let remaining_unmatched = u64::try_from(unmatched_rows.len())
        .unwrap_or(0)
        .saturating_sub(matched_count);

    Ok(Json(RescanResponse {
        matched: matched_count,
        remaining_unmatched,
    }))
}

/// Query parameters for the repo sync endpoint.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SyncQuery {
    /// Whether to rebuild the archive index after sync.
    #[serde(default)]
    pub build_index: bool,
}

const SYNC_WARN_DURATION: Duration = Duration::from_mins(5);

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/sync",
    tag = "Repositories",
    operation_id = "syncRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("build_index" = bool, Query, description = "Also build archive indexes while syncing"),
    ),
    responses(
        (status = 202, description = "Sync accepted, progress via WebSocket", body = SyncResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Sync already in progress"),
    )
)]
/// Full repository sync - re-reads all archives from borg.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the request conflicts with the current state.
pub async fn sync_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<SyncQuery>,
    Path(repo_id): Path<i64>,
) -> Result<(StatusCode, Json<SyncResponse>), ApiError> {
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;
    if repo.importing {
        return Err(ApiError::Conflict("sync already in progress".to_string()));
    }

    // Set importing immediately so the scheduler and other callers know.
    db::set_repo_importing(&state.pool, repo_id, true).await?;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);

    let repo_name = repo.name.clone();
    let (task_id, cancel) = state.import_tasks.start(repo_id).await;

    // Spawn the sync in a background task so client/proxy disconnects (e.g.
    // nginx 504 after 60s) do not cancel the cleanup -- the task owns the full
    // lifecycle and always clears importing + broadcasts DataChanged.
    // Tracked like the `enrich_archive_stats_background` spawn above: this task
    // acquires the repo lock and runs borg via sync_existing_archives, so a test
    // that returns without waiting for it leaves it running into whatever test
    // comes next.
    let sync = run_repo_sync_task(RepoSyncTask {
        task_state: state.clone(),
        pool: state.pool.clone(),
        encryption_key: state.encryption_key,
        ui_broadcast: state.ui_broadcast.clone(),
        repo_lock: state.repo_lock.clone(),
        repo_id,
        repo_name,
        build_index: query.build_index,
        task_id,
        cancel,
        reset_first: false,
        operation_label: "repo sync",
    });
    state.background_task_tracker.spawn_tracked(sync);

    Ok((
        StatusCode::ACCEPTED,
        Json(SyncResponse {
            imported: 0,
            removed: 0,
            duration_secs: 0,
        }),
    ))
}

struct RepoSyncTask {
    task_state: AppState,
    pool: PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: UiBroadcast,
    repo_lock: RepoLock,
    repo_id: i64,
    repo_name: String,
    build_index: bool,
    task_id: u64,
    cancel: CancellationToken,
    /// Whether to delete all existing archive metadata before syncing
    /// (`reset-and-sync`) or sync incrementally (`sync`).
    reset_first: bool,
    /// Used only in log messages and system-event text to distinguish the
    /// two operations ("repo sync" vs "reset-and-sync").
    operation_label: &'static str,
}

/// Arguments for [`run_repo_sync_work`], bundled (rather than passed
/// individually) to stay under clippy's argument-count limit.
struct RepoSyncWork<'a> {
    task_state: &'a AppState,
    pool: &'a PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: &'a UiBroadcast,
    repo_lock: RepoLock,
    repo_id: i64,
    repo_name: &'a str,
    build_index: bool,
    task_id: u64,
    reset_first: bool,
    operation_label: &'static str,
}

/// The actual sync work `run_repo_sync_task` races against cancellation.
/// Split out so the outer task stays under clippy's function-length limit.
async fn run_repo_sync_work(work: RepoSyncWork<'_>) {
    let RepoSyncWork {
        task_state,
        pool,
        encryption_key,
        ui_broadcast,
        repo_lock,
        repo_id,
        repo_name,
        build_index,
        task_id,
        reset_first,
        operation_label,
    } = work;

    let _sync_guard = repo_lock.acquire(repo_id).await;
    let start = std::time::Instant::now();

    if reset_first {
        let reset_ok =
            reset_repo_archive_data_before_sync(task_state, pool, ui_broadcast, repo_id, task_id)
                .await;
        if !reset_ok {
            return;
        }
    }

    let result = sync_existing_archives(
        pool,
        &encryption_key,
        repo_id,
        ui_broadcast,
        &task_state.background_task_tracker,
        &task_state.task_registry,
    )
    .await;
    let elapsed = start.elapsed();

    let (imported, removed) = match result {
        Ok(counts) => {
            let _ = db::update_repo_last_synced(pool, repo_id).await;
            counts
        }
        Err(e) => {
            handle_repo_sync_failure(
                task_state,
                pool,
                ui_broadcast,
                repo_id,
                repo_name,
                operation_label,
                task_id,
                elapsed,
                &e,
            )
            .await;
            return;
        }
    };

    log_repo_sync_completion(
        pool,
        repo_id,
        repo_name,
        operation_label,
        imported,
        removed,
        elapsed,
    )
    .await;

    if !task_state.import_tasks.is_current(repo_id, task_id).await {
        return;
    }

    let _ = db::set_repo_import_error(pool, repo_id, None).await;
    if build_index {
        index_archives_with_progress(
            pool.clone(),
            encryption_key,
            repo_id,
            ui_broadcast.clone(),
            repo_lock,
            true,
            task_state.task_registry.clone(),
        )
        .await;
    }

    let _ = db::set_repo_importing(pool, repo_id, false).await;
    clear_import_progress_state(pool, ui_broadcast, repo_id).await;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
}

/// Shared background task body for both `sync_repo` and `reset_and_sync_repo`;
/// the only behavioral difference is whether archive metadata is wiped first.
async fn run_repo_sync_task(task: RepoSyncTask) {
    let RepoSyncTask {
        task_state,
        pool,
        encryption_key,
        ui_broadcast,
        repo_lock,
        repo_id,
        repo_name,
        build_index,
        task_id,
        cancel,
        reset_first,
        operation_label,
    } = task;

    let op_clear_guard = set_server_sync_op(&task_state, repo_id).await;

    tokio::select! {
        () = cancel.cancelled() => {
            info!(repo_id, "{operation_label} cancelled");
        }
        () = run_repo_sync_work(RepoSyncWork {
            task_state: &task_state,
            pool: &pool,
            encryption_key,
            ui_broadcast: &ui_broadcast,
            repo_lock,
            repo_id,
            repo_name: &repo_name,
            build_index,
            task_id,
            reset_first,
            operation_label,
        }) => {}
    }

    finish_server_sync_task(
        &task_state.import_tasks,
        &ui_broadcast,
        repo_id,
        task_id,
        op_clear_guard,
    )
    .await;
}

/// Wipes existing archive metadata for a `reset-and-sync` before the
/// incremental sync runs. Returns `false` (after cleaning up importing
/// state) if the deletion itself failed, signalling the caller to abort.
async fn reset_repo_archive_data_before_sync(
    task_state: &AppState,
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    task_id: u64,
) -> bool {
    if let Err(e) = db::delete_all_repo_archive_data(pool, repo_id).await {
        error!(repo_id, error = %e, "failed to delete archive data for reset-and-sync");
        if task_state.import_tasks.is_current(repo_id, task_id).await {
            let _ = db::set_repo_importing(pool, repo_id, false).await;
            let _ = db::set_repo_import_error(pool, repo_id, Some(&format!("{e}"))).await;
            clear_import_progress_state(pool, ui_broadcast, repo_id).await;
            ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
        }
        return false;
    }
    if let Err(e) = db::delete_orphaned_placeholder_agents(pool).await {
        warn!(repo_id, error = %e, "failed to clean up orphaned placeholder agents");
    }
    true
}

#[allow(
    clippy::too_many_arguments,
    reason = "grouping these into a struct would obscure the call site more than it would clarify \
              it; all params are single-use scalars/refs from the caller's own locals"
)]
async fn handle_repo_sync_failure(
    task_state: &AppState,
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    repo_name: &str,
    operation_label: &str,
    task_id: u64,
    elapsed: Duration,
    e: &ApiError,
) {
    let msg = format!(
        "{operation_label} failed for '{repo_name}' after {:.1}s: {e}",
        elapsed.as_secs_f64()
    );
    error!("{msg}");
    let _ = db::insert_system_event(pool, SystemEventType::RepoSyncFailed, None, &msg).await;
    if task_state.import_tasks.is_current(repo_id, task_id).await {
        let _ = db::set_repo_import_error(pool, repo_id, Some(&format!("{e}"))).await;
        let _ = db::set_repo_importing(pool, repo_id, false).await;
        clear_import_progress_state(pool, ui_broadcast, repo_id).await;
        ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
    }
}

async fn log_repo_sync_completion(
    pool: &PgPool,
    repo_id: i64,
    repo_name: &str,
    operation_label: &str,
    imported: u64,
    removed: u64,
    elapsed: Duration,
) {
    let duration_secs = elapsed.as_secs();
    let msg = format!(
        "{operation_label} completed for '{repo_name}': imported {imported}, removed {removed} \
         archives in {duration_secs}s"
    );

    if elapsed > SYNC_WARN_DURATION {
        error!(
            repo_id,
            duration_secs,
            "{operation_label} exceeded {}s threshold",
            SYNC_WARN_DURATION.as_secs()
        );
        let warn_msg = format!(
            "{operation_label} for '{repo_name}' took {duration_secs}s (exceeds {}s threshold)",
            SYNC_WARN_DURATION.as_secs()
        );
        let _ = db::insert_system_event(pool, SystemEventType::RepoSyncSlow, None, &warn_msg).await;
    }

    info!("{msg}");
    let _ = db::insert_system_event(pool, SystemEventType::RepoSync, None, &msg).await;
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/reset-and-sync",
    tag = "Repositories",
    operation_id = "resetImport",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 204, description = "Import state reset"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Reset a stuck importing state (admin only).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn reset_import(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;
    let cancelled = state.import_tasks.cancel(repo_id).await;
    if cancelled {
        info!(repo_id, repo_name = %repo.name, "repo sync cancelled");
        let msg = format!("repo sync cancelled for '{}'", repo.name);
        if let Err(e) =
            db::insert_system_event(&state.pool, SystemEventType::RepoSyncCancelled, None, &msg)
                .await
        {
            tracing::error!(repo_id, error = %e, "failed to log sync cancelled event");
        }
    }
    // A reset ends a passphrase hold too: once `importing` is clear a sync can
    // start, and a hold outliving it would be released under that sync. The
    // hold goes first, clearing both flags at once, so a held repository is
    // never briefly not importing while still held.
    db::release_passphrase_hold(&state.pool, repo_id).await?;
    db::set_repo_importing(&state.pool, repo_id, false).await?;
    db::set_repo_import_error(&state.pool, repo_id, None).await?;
    clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id).await;
    clear_server_sync_op(&state, repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/reset-and-sync",
    tag = "Repositories",
    operation_id = "resetAndSyncRepo",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("build_index" = bool, Query, description = "Also build archive indexes while syncing"),
    ),
    responses(
        (status = 202, description = "Reset accepted, progress via WebSocket", body = SyncResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Sync already in progress"),
    )
)]
/// Delete all archive metadata and re-import from borg (admin only).
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the request conflicts with the current state.
pub async fn reset_and_sync_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<SyncQuery>,
    Path(repo_id): Path<i64>,
) -> Result<(StatusCode, Json<SyncResponse>), ApiError> {
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;
    if repo.importing {
        return Err(ApiError::Conflict("sync already in progress".to_string()));
    }

    // Set importing immediately so the scheduler and other callers know.
    db::set_repo_importing(&state.pool, repo_id, true).await?;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);

    let repo_name = repo.name.clone();
    let (task_id, cancel) = state.import_tasks.start(repo_id).await;

    // Spawn the reset + sync in a background task so client/proxy disconnects
    // do not cancel the cleanup.
    // Tracked like the `enrich_archive_stats_background` spawn above: this task
    // acquires the repo lock and runs borg via sync_existing_archives, so a test
    // that returns without waiting for it leaves it running into whatever test
    // comes next.
    let sync = run_repo_sync_task(RepoSyncTask {
        task_state: state.clone(),
        pool: state.pool.clone(),
        encryption_key: state.encryption_key,
        ui_broadcast: state.ui_broadcast.clone(),
        repo_lock: state.repo_lock.clone(),
        repo_id,
        repo_name,
        build_index: query.build_index,
        task_id,
        cancel,
        reset_first: true,
        operation_label: "reset-and-sync",
    });
    state.background_task_tracker.spawn_tracked(sync);

    Ok((
        StatusCode::ACCEPTED,
        Json(SyncResponse {
            imported: 0,
            removed: 0,
            duration_secs: 0,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borg_subcommand_from_str_round_trips_all_variants() {
        for sub in BorgSubcommand::ALL {
            let parsed: BorgSubcommand = sub.as_str().parse().unwrap();
            assert_eq!(parsed, sub);
            assert_eq!(sub.to_string(), sub.as_str());
        }
    }

    #[test]
    fn borg_subcommand_from_str_rejects_unknown() {
        assert!("rm".parse::<BorgSubcommand>().is_err());
        assert!("".parse::<BorgSubcommand>().is_err());
        assert!("INFO".parse::<BorgSubcommand>().is_err());
    }

    #[test]
    fn borg_subcommand_permitted_list_contains_all() {
        let list = BorgSubcommand::permitted_list();
        for sub in BorgSubcommand::ALL {
            assert!(
                list.contains(sub.as_str()),
                "missing {} in {list}",
                sub.as_str()
            );
        }
        assert_eq!(list.matches(',').count(), BorgSubcommand::ALL.len() - 1);
    }

    #[test]
    fn repo_row_from_valid_compression_lz4() {
        let row = db::RepoRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            compression: shared::types::Compression::Lz4,
            encryption: shared::types::BorgEncryption::Repokey,
            enabled: true,
            owner_id: None,
            visibility: shared::types::Visibility::Private,
            sync_schedule: None,
            repo_host_id: 1,
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
            machine_id: 1,
        };
        let resp = RepoResponse::from(row);
        assert_eq!(resp.compression, shared::types::Compression::Lz4);
        assert_eq!(resp.encryption, shared::types::BorgEncryption::Repokey);
    }

    #[test]
    fn repo_with_stats_row_from_valid_last_op_kind() {
        let row = db::RepoWithStatsRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            ssh_host_key: None,
            compression: shared::types::Compression::Lz4,
            encryption: shared::types::BorgEncryption::Repokey,
            enabled: true,
            importing: false,
            import_error: None,
            import_progress: 0,
            import_total: 0,
            import_status_message: None,
            owner_id: None,
            visibility: shared::types::Visibility::Private,
            sync_schedule: None,
            last_synced_at: None,
            archive_count: 0,
            last_backup_at: None,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            agent_count: 0,
            unmatched_count: 0,
            last_op_kind: Some(shared::protocol::RepoOpKind::AgentBackup),
            relocation_pending: false,
            last_op_at: None,
            last_op_by: None,
            quota_warn_bytes: None,
            quota_critical_bytes: None,
            quota_warn_action: None,
            quota_critical_action: None,
            quota_enabled: None,
            repo_host_id: 1,
            host_intermittent: false,
            host_machine_id: 1,
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
        };
        let resp = RepoWithStatsResponse::from(row);
        assert_eq!(
            resp.last_op_kind,
            Some(shared::protocol::RepoOpKind::AgentBackup)
        );
    }

    #[test]
    fn repo_with_stats_row_from_no_quota_row_yields_none() {
        let row = db::RepoWithStatsRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            ssh_host_key: None,
            compression: shared::types::Compression::Lz4,
            encryption: shared::types::BorgEncryption::Repokey,
            enabled: true,
            importing: false,
            import_error: None,
            import_progress: 0,
            import_total: 0,
            import_status_message: None,
            owner_id: None,
            visibility: shared::types::Visibility::Private,
            sync_schedule: None,
            last_synced_at: None,
            archive_count: 0,
            last_backup_at: None,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            agent_count: 0,
            unmatched_count: 0,
            last_op_kind: None,
            relocation_pending: false,
            last_op_at: None,
            last_op_by: None,
            quota_warn_bytes: None,
            quota_critical_bytes: None,
            quota_warn_action: None,
            quota_critical_action: None,
            quota_enabled: None,
            repo_host_id: 1,
            host_intermittent: false,
            host_machine_id: 1,
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
        };
        let resp = RepoWithStatsResponse::from(row);
        assert!(resp.quota.is_none());
    }

    #[test]
    fn repo_with_stats_row_from_quota_row_maps_fields() {
        let row = db::RepoWithStatsRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            ssh_host_key: None,
            compression: shared::types::Compression::Lz4,
            encryption: shared::types::BorgEncryption::Repokey,
            enabled: true,
            importing: false,
            import_error: None,
            import_progress: 0,
            import_total: 0,
            import_status_message: None,
            owner_id: None,
            visibility: shared::types::Visibility::Private,
            sync_schedule: None,
            last_synced_at: None,
            archive_count: 0,
            last_backup_at: None,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            agent_count: 0,
            unmatched_count: 0,
            last_op_kind: None,
            relocation_pending: false,
            last_op_at: None,
            last_op_by: None,
            quota_warn_bytes: Some(500),
            quota_critical_bytes: Some(600),
            quota_warn_action: Some(shared::types::QuotaAction::NotifyOnly),
            quota_critical_action: Some(shared::types::QuotaAction::BlockBackups),
            quota_enabled: Some(true),
            repo_host_id: 1,
            host_intermittent: false,
            host_machine_id: 1,
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
        };
        let resp = RepoWithStatsResponse::from(row);
        let quota = resp.quota.expect("quota should be present");
        assert_eq!(quota.warn_bytes, Some(500));
        assert_eq!(quota.critical_bytes, Some(600));
        assert_eq!(quota.warn_action, shared::types::QuotaAction::NotifyOnly);
        assert_eq!(
            quota.critical_action,
            shared::types::QuotaAction::BlockBackups
        );
        assert!(quota.enabled);
    }

    async fn insert_repo_host(pool: &PgPool, ssh_host: &str, ssh_port: i32, key: Option<&str>) {
        let mut tx = pool.begin().await.expect("begin");
        let id = db::repo_hosts::resolve_repo_host(&mut tx, ssh_host, ssh_port)
            .await
            .expect("insert repo host");
        tx.commit().await.expect("commit");
        if let Some(key) = key {
            db::repo_hosts::update_repo_host_key(pool, id, key)
                .await
                .expect("pin host key");
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn check_known_repo_host_lets_an_unknown_or_matching_host_through(pool: PgPool) {
        insert_repo_host(&pool, "nas.lan", 22, Some("ssh-ed25519 AAAAPINNED")).await;
        insert_repo_host(&pool, "fresh.lan", 22, None).await;

        check_known_repo_host(&pool, "elsewhere.lan", 2222, "ssh-ed25519 AAAAANY")
            .await
            .expect("an unknown host becomes a new one");
        check_known_repo_host(&pool, "nas.lan", 22, "ssh-ed25519 AAAAPINNED")
            .await
            .expect("the pinned key matches");
        check_known_repo_host(&pool, "fresh.lan", 22, "ssh-ed25519 AAAAFIRST")
            .await
            .expect("a host with no pinned key has nothing to refuse");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn check_known_repo_host_refuses_another_port_or_key(pool: PgPool) {
        insert_repo_host(&pool, "nas.lan", 22, Some("ssh-ed25519 AAAAPINNED")).await;

        let port = check_known_repo_host(&pool, "nas.lan", 2222, "ssh-ed25519 AAAAPINNED").await;
        assert!(
            matches!(&port, Err(ApiError::BadRequest(msg)) if msg.contains("uses SSH port 22")),
            "{port:?}"
        );

        let key = check_known_repo_host(&pool, "nas.lan", 22, "ssh-ed25519 AAAACHANGED").await;
        let Err(ApiError::Conflict(msg)) = key else {
            panic!("expected a conflict, got {key:?}");
        };
        let expected = concat!(
            "nas.lan presents a different SSH host key than the one pinned on its ",
            "repository host; check the host and accept the new key there first",
        );
        assert_eq!(msg, expected);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn moving_a_repository_to_a_known_host_checks_its_pinned_key(pool: PgPool) {
        insert_repo_host(&pool, "nas.lan", 22, Some("ssh-ed25519 AAAAPINNED")).await;
        insert_repo_host(&pool, "fresh.lan", 22, None).await;

        let key = check_move_to_repo_host(&pool, "nas.lan", 22, || async {
            Ok("ssh-ed25519 AAAACHANGED".to_owned())
        })
        .await;
        assert!(matches!(key, Err(ApiError::Conflict(_))), "{key:?}");

        check_move_to_repo_host(&pool, "nas.lan", 22, || async {
            Ok("ssh-ed25519 AAAAPINNED".to_owned())
        })
        .await
        .expect("the pinned key matches");

        // No key to hold it to, or no host yet: nothing is scanned.
        for ssh_host in ["fresh.lan", "unknown.lan"] {
            check_move_to_repo_host(&pool, ssh_host, 22, || async {
                panic!("{ssh_host} has no pinned key, so nothing may be scanned")
            })
            .await
            .expect("a host without a pinned key is joined as before");
        }
    }
}
