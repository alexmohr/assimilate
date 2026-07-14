// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, fmt, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::DateTime;
use futures_util::future::join_all;
use serde::Deserialize;
use shared::{
    crypto::encrypt_passphrase,
    responses::{
        BreakLockResponse, ConfirmRelocationResponse, ExecBorgResponse, InitRepoResponse,
        MigrateEncryptionResponse, PassphraseResponse, RepoHostKeyResponse, RepoResponse,
        RepoWithStatsResponse, RescanResponse, SyncResponse,
    },
    types::{BORG_REPO_ENV_KEY, BorgEncryption, build_repo_url},
};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{
    archives::LOCK_WAIT_SECS,
    auth::{AuthUser, RequireAdmin},
    helpers,
    permissions::is_visible_to_user,
};
use crate::{
    AppState, RepoLock, archive_index,
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
            compression: row.compression.parse().unwrap_or_default(),
            encryption: row.encryption.parse().unwrap_or_default(),
            enabled: row.enabled,
            owner_id: row.owner_id,
            visibility: row.visibility,
            sync_schedule: row.sync_schedule,
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
            compression: row.compression.parse().unwrap_or_default(),
            encryption: row.encryption.parse().unwrap_or_default(),
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
            last_op_kind: row.last_op_kind.and_then(|s| s.parse().ok()),
            last_op_at: row.last_op_at,
            last_op_by: row.last_op_by,
            current_op: None,
        }
    }
}

/// Extracts a concise, user-facing error message from borg stderr.
///
/// Borg sometimes outputs a full Python traceback; in that case the actual
/// exception is on a line matching `SomeError: description`. When no traceback
/// is present the first line is usually the useful message.
fn extract_borg_error(stderr: &str) -> &str {
    let first = stderr.lines().next().unwrap_or(stderr);
    if !first.starts_with("Traceback") {
        return first;
    }
    stderr
        .lines()
        .take_while(|l| !l.starts_with("Platform:") && !l.starts_with("Borg server: Platform:"))
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with(' ')
                && !l.starts_with("Traceback")
                && !l.starts_with("Borg server:")
        })
        .last()
        .unwrap_or(first)
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
    let mut visible = Vec::with_capacity(repos.len());
    for repo in repos {
        if is_visible_to_user(
            &state.pool,
            auth.user_id,
            repo.owner_id,
            &repo.visibility,
            is_admin,
        )
        .await?
        {
            visible.push(RepoResponse::from(repo));
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
    ),
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
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
) -> Result<Json<Vec<RepoResponse>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
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

    let ssh_port = req.ssh_port.unwrap_or(22);
    let ssh_port_u16 = u16::try_from(ssh_port)
        .map_err(|_| ApiError::BadRequest("ssh_port out of range".into()))?;
    helpers::validate_path_exists(&req.ssh_host, &req.ssh_user, ssh_port_u16, &req.repo_path)
        .await?;
    let ssh_host_key = crate::ssh::scan_host_key(&req.ssh_host, ssh_port_u16)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;

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

    let repo = db::insert_repo(
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
    )
    .await?;
    db::update_repo_ssh_host_key(&state.pool, repo.id, &ssh_host_key).await?;

    let repo_id = repo.id;
    let pool = state.pool.clone();
    let encryption_key = state.encryption_key;
    let ui_broadcast = state.ui_broadcast.clone();
    let state_repo_lock = state.repo_lock.clone();
    let need_borg_info = info_result.is_none();
    let bg_repo_url = repo_url.clone();
    let bg_passphrase = req.passphrase.clone();
    let task_state = state.clone();

    db::set_repo_importing(&state.pool, repo_id, true).await?;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
    let (task_id, cancel) = state.import_tasks.start(repo_id).await;

    tokio::spawn(run_initial_import_task(InitialImportTask {
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
    }));

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
    } = work;

    // Detect encryption before syncing rather than concurrently: both borg
    // info and the sync's borg list contend for the same repository lock, so
    // running them in parallel would force the list into the lock-retry path.
    if need_borg_info {
        let timeout = get_borg_timeout(pool).await;
        match run_borg_info_with_retry(
            bg_repo_url,
            bg_passphrase,
            timeout,
            &task_state.task_registry,
        )
        .await
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

/// Request payload for accepting a scanned SSH host key.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AcceptRepoHostKeyRequest {
    /// The SSH host key string (e.g. "ssh-rsa AAAA...").
    pub ssh_host_key: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/ssh-host-key/scan",
    tag = "Repositories",
    operation_id = "scanRepoHostKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Scanned SSH host key", body = RepoHostKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "SSH host key scan failed"),
    )
)]
/// Scan the repository host key without saving it.
///
/// # Errors
///
/// Returns [`ApiError::BadGateway`] if the upstream operation (e.g. SSH or borg) fails.
pub async fn scan_repo_host_key(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoHostKeyResponse>, ApiError> {
    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let ssh_port = u16::try_from(repo.ssh_port).unwrap_or(22);
    let ssh_host_key = crate::ssh::scan_host_key(&repo.ssh_host, ssh_port)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;
    Ok(Json(RepoHostKeyResponse { ssh_host_key }))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/ssh-host-key",
    tag = "Repositories",
    operation_id = "acceptRepoHostKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = AcceptRepoHostKeyRequest,
    responses(
        (status = 200, description = "SSH host key accepted", body = RepoHostKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Accept a scanned SSH host key and push updated config.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn accept_repo_host_key(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<AcceptRepoHostKeyRequest>,
) -> Result<Json<RepoHostKeyResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host_key, "ssh_host_key")?;

    let repo = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    db::update_repo_ssh_host_key(&state.pool, repo_id, &req.ssh_host_key).await?;

    let hostnames = db::get_schedule_target_hostnames_for_repo(&state.pool, repo_id).await?;
    for hostname in &hostnames {
        config_assembler::push_config_to_agent(&state, hostname).await;
    }

    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);

    info!(repo_id, name = %repo.name, "repository SSH host key accepted");

    Ok(Json(RepoHostKeyResponse {
        ssh_host_key: req.ssh_host_key,
    }))
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
    let mut visible = Vec::with_capacity(repos.len());
    for repo in repos {
        if is_visible_to_user(
            &state.pool,
            auth.user_id,
            repo.owner_id,
            &repo.visibility,
            is_admin,
        )
        .await?
        {
            let response = RepoWithStatsResponse::from(repo);
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
    _auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoWithStatsResponse>, ApiError> {
    let mut res: RepoWithStatsResponse =
        db::get_repo_with_stats(&state.pool, repo_id).await?.into();
    res.current_op = state.repo_op_tracker.get(repo_id).await;
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

    let ssh_port = req.ssh_port.unwrap_or(22);
    let ssh_port_u16 = u16::try_from(ssh_port)
        .map_err(|_| ApiError::BadRequest("ssh_port out of range".into()))?;
    let ssh_host_key = crate::ssh::scan_host_key(&req.ssh_host, ssh_port_u16)
        .await
        .map_err(|e| ApiError::BadGateway(e.to_string()))?;
    let repo_url = build_repo_url(&req.ssh_user, &req.ssh_host, ssh_port_u16, &req.repo_path);

    let borg_output = run_borg_init(
        &repo_url,
        &req.passphrase,
        req.encryption,
        &state.task_registry,
    )
    .await?;

    let passphrase_encrypted = encrypt_passphrase(&req.passphrase, &state.encryption_key)?;
    let compression = helpers::validate_compression(req.compression.as_deref())?;

    let encryption = req.encryption.to_string();

    let repo = db::insert_repo(
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
    )
    .await?;
    db::update_repo_ssh_host_key(&state.pool, repo.id, &ssh_host_key).await?;

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

    let hostnames = db::get_schedule_target_hostnames_for_repo(&state.pool, repo_id).await?;
    for hostname in &hostnames {
        config_assembler::push_config_to_agent(&state, hostname).await;
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
            &s.visibility,
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
        )
        .await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });

    let result = run_borg_break_lock(&repo_url, &passphrase, &state.task_registry).await;

    state.repo_op_tracker.clear(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });

    let borg_output = result?;

    let now = chrono::Utc::now();
    if let Err(e) = db::update_repo_last_op(&state.pool, repo_id, "break_lock", now, "server").await
    {
        warn!(repo_id, error = %e, "failed to persist last_op for break_lock");
    }

    info!(repo_id, name = %repo.name, "repository lock broken");

    Ok(Json(BreakLockResponse {
        message: format!("lock broken on repository '{}'", repo.name),
        borg_output,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorgSubcommand {
    Info,
    List,
    Check,
    Compact,
    Prune,
    Delete,
    Diff,
    Rename,
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
        match self {
            BorgSubcommand::Info => "info",
            BorgSubcommand::List => "list",
            BorgSubcommand::Check => "check",
            BorgSubcommand::Compact => "compact",
            BorgSubcommand::Prune => "prune",
            BorgSubcommand::Delete => "delete",
            BorgSubcommand::Diff => "diff",
            BorgSubcommand::Rename => "rename",
            BorgSubcommand::Recreate => "recreate",
        }
    }

    fn permitted_list() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for BorgSubcommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BorgSubcommand {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "info" => Ok(BorgSubcommand::Info),
            "list" => Ok(BorgSubcommand::List),
            "check" => Ok(BorgSubcommand::Check),
            "compact" => Ok(BorgSubcommand::Compact),
            "prune" => Ok(BorgSubcommand::Prune),
            "delete" => Ok(BorgSubcommand::Delete),
            "diff" => Ok(BorgSubcommand::Diff),
            "rename" => Ok(BorgSubcommand::Rename),
            "recreate" => Ok(BorgSubcommand::Recreate),
            _ => Err(()),
        }
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

    let subcommand = raw_subcommand.parse::<BorgSubcommand>().map_err(|()| {
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

    let mut env = helpers::borg_base_env(&passphrase);
    env.insert(BORG_REPO_ENV_KEY.to_owned(), repo_url);

    info!(repo_id, name = %repo.name, subcommand = %subcommand, "admin executing borg command");

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

    let current_encryption: BorgEncryption = repo
        .encryption
        .parse()
        .map_err(|_| ApiError::Internal("invalid current encryption in database".to_owned()))?;

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

    let date_suffix = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let migrated_path = format!("{}.migrated-{date_suffix}", repo.repo_path);

    let ssh_rename_result = run_ssh_command(
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
    .await;

    if let Err(e) = ssh_rename_result {
        return Err(ApiError::BadGateway(format!(
            "failed to rename old repository: {e}"
        )));
    }

    info!(repo_id, %migrated_path, "renamed old repo for migration");

    let init_result = run_borg_init(
        &repo_url,
        &passphrase,
        req.target_encryption,
        &state.task_registry,
    )
    .await;
    if let Err(e) = init_result {
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

    db::update_repo_encryption(&state.pool, repo_id, req.target_encryption.as_borg_arg()).await?;

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            action: "migrate_encryption",
            target_type: Some("repo"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({
                "from": repo.encryption,
                "to": req.target_encryption.as_borg_arg(),
                "migrated_path": migrated_path,
            })),
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

struct BorgInfoResult {
    encryption: BorgEncryption,
}

fn is_lock_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("failed to create/acquire the lock")
        || lower.contains("lock.exclusive")
        || lower.contains("lockroster")
}

async fn run_borg_info(
    repo_url: &str,
    passphrase: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    run_borg_info_once(repo_url, passphrase, task_registry).await
}

const LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const LOCK_RETRY_MAX_ATTEMPTS: u32 = 60;
const DEFAULT_BORG_QUERY_TIMEOUT_SECS: u64 = 300;
const DEFAULT_BORG_LIST_STAGE_TIMEOUT_SECS: u64 = 1800;

/// Upper bound for a single `borg list`/`borg info` invocation. `--lock-wait`
/// only bounds lock contention; a hung SSH connection would otherwise keep the
/// process (and the import) waiting forever, leaving the repo stuck at "Listing
/// archives". On timeout the process is killed and the operation fails so the
/// importing state is cleared.
///
/// Resolution order: DB setting `borg_query_timeout_secs`, then env var
/// `ASSIMILATE_BORG_QUERY_TIMEOUT_SECS`, then 300 s default.
async fn get_borg_timeout(pool: &PgPool) -> Duration {
    if let Ok(Some(v)) = db::get_setting(pool, "borg_query_timeout_secs").await
        && let Ok(secs) = v.parse::<u64>()
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    std::env::var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or_else(
            || Duration::from_secs(DEFAULT_BORG_QUERY_TIMEOUT_SECS),
            Duration::from_secs,
        )
}

/// Upper bound on the *total* duration of all `borg list` attempts in a single
/// listing stage, including per-attempt timeouts and lock-retry sleeps. Without
/// this, a repository locked by a long-running backup can keep the import stuck
/// at "Listing archives" for up to 90 minutes (60 retries, roughly 90s each).
///
/// Resolution order: DB setting `borg_list_stage_timeout_secs`, then env var
/// `ASSIMILATE_BORG_LIST_STAGE_TIMEOUT_SECS`, then 1800 s default.
async fn get_borg_list_stage_timeout(pool: &PgPool) -> Duration {
    if let Ok(Some(v)) = db::get_setting(pool, "borg_list_stage_timeout_secs").await
        && let Ok(secs) = v.parse::<u64>()
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    std::env::var("ASSIMILATE_BORG_LIST_STAGE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or_else(
            || Duration::from_secs(DEFAULT_BORG_LIST_STAGE_TIMEOUT_SECS),
            Duration::from_secs,
        )
}

async fn run_borg_info_with_retry(
    repo_url: &str,
    passphrase: &str,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let attempt_result = match tokio::time::timeout(
            timeout,
            run_borg_info_once(repo_url, passphrase, task_registry),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(ApiError::Internal(format!(
                "borg info timed out after {}s; the repository may be unreachable",
                timeout.as_secs()
            ))),
        };
        match attempt_result {
            Ok(result) => return Ok(result),
            Err(e) => {
                let is_lock = matches!(&e, ApiError::BadGateway(msg) if is_lock_error(msg));
                if !is_lock || attempt == LOCK_RETRY_MAX_ATTEMPTS {
                    return Err(e);
                }
                warn!(
                    attempt,
                    max = LOCK_RETRY_MAX_ATTEMPTS,
                    "borg info lock contention, retrying in {}s",
                    LOCK_RETRY_INTERVAL.as_secs()
                );
                tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
            }
        }
    }
    Err(ApiError::Internal(
        "borg info failed after maximum retries".to_owned(),
    ))
}

async fn run_borg_info_once(
    repo_url: &str,
    passphrase: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
        .with_registry(task_registry.clone())
        .run(&["info", "--json", repo_url], &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg info failed");

        let lower = stderr.to_lowercase();
        if lower.contains("passphrase") || lower.contains("decrypt") {
            return Err(ApiError::BadRequest(
                "passphrase is incorrect for this repository".to_string(),
            ));
        }
        if lower.contains("not a valid repository")
            || lower.contains("does not exist")
            || lower.contains("failed to open repository")
        {
            return Err(ApiError::BadRequest(
                "path does not contain a valid borg repository".to_string(),
            ));
        }

        let summary = extract_borg_error(&stderr);
        return Err(ApiError::BadGateway(format!(
            "borg info failed (exit {exit_code}): {summary}"
        )));
    }

    let json: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg info JSON: {e}")))?;

    let mode_str = json
        .get("encryption")
        .and_then(|e| e.get("mode"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ApiError::Internal("borg info JSON missing encryption.mode field".to_string())
        })?;

    let encryption = mode_str.parse::<BorgEncryption>().map_err(|e| {
        ApiError::Internal(format!("unsupported encryption mode from borg info: {e}"))
    })?;

    Ok(BorgInfoResult { encryption })
}

async fn run_borg_break_lock(
    repo_url: &str,
    passphrase: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<String, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
        .with_registry(task_registry.clone())
        .run(
            &["break-lock", "--lock-wait", LOCK_WAIT_SECS, repo_url],
            &env,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg break-lock failed");
        let summary = extract_borg_error(&stderr);
        return Err(ApiError::BadGateway(format!(
            "borg break-lock failed (exit {exit_code}): {summary}"
        )));
    }

    let combined = if stdout.is_empty() {
        stderr
    } else {
        format!("{stdout}\n{stderr}")
    };

    Ok(combined.trim().to_string())
}

async fn run_borg_init(
    repo_url: &str,
    passphrase: &str,
    encryption: BorgEncryption,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<String, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
        .with_registry(task_registry.clone())
        .run(
            &["init", "--encryption", encryption.as_borg_arg(), repo_url],
            &env,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg init failed");

        if stderr.contains("repository already exists") {
            return Err(ApiError::Conflict(
                "repository already exists at this path".to_string(),
            ));
        }

        let summary = extract_borg_error(&stderr);
        return Err(ApiError::BadGateway(format!(
            "borg init failed (exit {exit_code}): {summary}"
        )));
    }

    let combined = if stdout.is_empty() {
        stderr
    } else {
        format!("{stdout}\n{stderr}")
    };

    Ok(combined.trim().to_string())
}

struct BorgListRetryArgs<'a> {
    borg_repo: &'a str,
    env: &'a std::collections::HashMap<String, String>,
    timeout: Duration,
    stage_timeout: Duration,
    pool: &'a PgPool,
    ui_broadcast: &'a UiBroadcast,
    repo_id: i64,
    task_registry: &'a shared::task_registry::TaskRegistry,
}

async fn run_borg_list_with_retry(
    args: BorgListRetryArgs<'_>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let BorgListRetryArgs {
        borg_repo,
        env,
        timeout,
        stage_timeout,
        pool,
        ui_broadcast,
        repo_id,
        task_registry,
    } = args;
    let borg = Borg::new().with_registry(task_registry.clone());
    // Plain `borg list --json` reads the repository manifest and returns
    // immediately; adding `--format` forces per-archive metadata loads and
    // makes full resyncs crawl on large repositories.
    let args = borg_list_args(borg_repo);
    let stage_deadline = tokio::time::Instant::now()
        .checked_add(stage_timeout)
        .unwrap_or_else(tokio::time::Instant::now);

    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let remaining = stage_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(ApiError::Internal(format!(
                "borg list stage timed out after {}s; repository may be locked by a long-running \
                 backup",
                stage_timeout.as_secs()
            )));
        }
        let attempt_timeout = timeout.min(remaining);

        let output = match tokio::time::timeout(attempt_timeout, borg.run(&args, env)).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ApiError::Internal(format!(
                    "failed to execute borg list: {e}"
                )));
            }
            Err(_) => {
                return Err(ApiError::Internal(format!(
                    "borg list timed out after {}s; the repository may be unreachable",
                    attempt_timeout.as_secs()
                )));
            }
        };

        if output.status.success() {
            // A successful exit with unparseable output is a hard error: silently
            // treating it as empty would prune every existing archive record.
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
                ApiError::Internal(format!("borg list returned malformed JSON: {e}"))
            })?;
            let archives = json
                .get("archives")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    ApiError::Internal("borg list JSON missing 'archives' array".to_string())
                })?;
            return Ok(archives);
        }

        let stderr_str = String::from_utf8_lossy(&output.stderr);
        if !is_lock_error(&stderr_str) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            return Err(ApiError::Internal(format!(
                "borg list failed: {stderr_str}"
            )));
        }

        let now = tokio::time::Instant::now();
        if now
            .checked_add(LOCK_RETRY_INTERVAL)
            .is_none_or(|next| next >= stage_deadline)
        {
            return Err(ApiError::Internal(format!(
                "borg list stage timed out after {}s; repository may be locked by a long-running \
                 backup",
                stage_timeout.as_secs()
            )));
        }

        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            "borg list lock contention, retrying in {}s",
            LOCK_RETRY_INTERVAL.as_secs()
        );
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            0,
            0,
            Some("Waiting for lock\u{2026}"),
        )
        .await;
        tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
    }

    Err(ApiError::Internal(
        "borg list failed after maximum retries".to_owned(),
    ))
}

fn borg_list_args(borg_repo: &str) -> [&str; 6] {
    [
        "list",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        borg_repo,
    ]
}

/// Refreshes the authoritative repo statistics from `borg info --json`
/// (`cache.stats`) plus the archive count from the just-completed `borg list`.
/// These are the single source of truth for repo size/archive numbers; failures
/// are logged but never abort a sync.
async fn refresh_repo_info_stats(
    pool: &PgPool,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_count: i64,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) {
    let args = [
        "info",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        borg_repo,
    ];
    let output = match tokio::time::timeout(
        timeout,
        Borg::new()
            .with_registry(task_registry.clone())
            .run(&args, env),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!(repo_id, error = %e, "failed to run borg info for repo stats");
            return;
        }
        Err(_) => {
            warn!(
                repo_id,
                timeout_secs = timeout.as_secs(),
                "borg info timed out refreshing repo stats"
            );
            return;
        }
    };

    if !output.status.success() {
        warn!(
            repo_id,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "borg info (repo stats) failed"
        );
        return;
    }

    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            warn!(repo_id, error = %e, "failed to parse borg info repo stats");
            return;
        }
    };

    let stats = json.get("cache").and_then(|v| v.get("stats"));
    let info_stats = db::RepoInfoStats {
        original_size: stats
            .and_then(|s| s.get("total_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        compressed_size: stats
            .and_then(|s| s.get("total_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        deduplicated_size: stats
            .and_then(|s| s.get("unique_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        total_chunks: stats
            .and_then(|s| s.get("total_chunks"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        unique_chunks: stats
            .and_then(|s| s.get("total_unique_chunks"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        archive_count,
    };

    if let Err(e) = db::update_repo_info_stats(pool, repo_id, &info_stats).await {
        warn!(repo_id, error = %e, "failed to persist repo info stats");
    }
}

fn parse_borg_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
        .map(|dt| dt.to_utc())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
                .map(|naive| naive.and_utc())
        })
        .ok()
}

fn archive_hostname(archive: &serde_json::Value) -> Option<&str> {
    archive["hostname"]
        .as_str()
        .filter(|hostname| !hostname.is_empty())
}

fn archive_finish_time(
    archive: &serde_json::Value,
    started_at: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    parse_borg_timestamp(archive["end"].as_str().unwrap_or_default()).unwrap_or_else(|| {
        archive["duration"]
            .as_f64()
            .and_then(|duration| std::time::Duration::try_from_secs_f64(duration).ok())
            .and_then(|duration| chrono::Duration::from_std(duration).ok())
            .and_then(|duration| started_at.checked_add_signed(duration))
            .unwrap_or(started_at)
    })
}

fn archive_metadata_missing(archive: &serde_json::Value) -> bool {
    archive_hostname(archive).is_none() || archive["end"].as_str().is_none_or(str::is_empty)
}

async fn fetch_archive_metadata_with_retry(
    borg_repo: &str,
    env: &HashMap<String, String>,
    archive_name: &str,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<serde_json::Value, ApiError> {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let args = [
        "info",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        repo_archive.as_str(),
    ];

    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let output = match tokio::time::timeout(
            timeout,
            Borg::new()
                .with_registry(task_registry.clone())
                .run(&args, env),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ApiError::Internal(format!(
                    "failed to execute borg info for archive '{archive_name}': {e}"
                )));
            }
            Err(_) => {
                return Err(ApiError::Internal(format!(
                    "borg info timed out after {}s while reading archive '{archive_name}'",
                    timeout.as_secs()
                )));
            }
        };

        if output.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
                ApiError::Internal(format!(
                    "borg info returned malformed JSON for archive '{archive_name}': {e}"
                ))
            })?;
            return json
                .get("archives")
                .and_then(serde_json::Value::as_array)
                .and_then(|archives| archives.first())
                .cloned()
                .ok_or_else(|| {
                    ApiError::Internal(format!(
                        "borg info JSON missing archive entry for '{archive_name}'"
                    ))
                });
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !is_lock_error(&stderr) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            let summary = extract_borg_error(&stderr);
            return Err(ApiError::Internal(format!(
                "borg info failed for archive '{archive_name}': {summary}"
            )));
        }

        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            archive = archive_name,
            "borg info lock contention, retrying in {}s",
            LOCK_RETRY_INTERVAL.as_secs()
        );
        tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
    }

    Err(ApiError::Internal(format!(
        "borg info failed after maximum retries for archive '{archive_name}'"
    )))
}

struct HydrateArchivesArgs<'a> {
    pool: &'a PgPool,
    ui_broadcast: &'a UiBroadcast,
    repo_id: i64,
    borg_repo: &'a str,
    env: &'a HashMap<String, String>,
    timeout: Duration,
    archives: &'a [&'a serde_json::Value],
    task_registry: &'a shared::task_registry::TaskRegistry,
}

/// Fills in `hostname`/`end` for a single archive if missing, via a fresh `borg info`
/// lookup. Archives that already have both, or have no `name`, are returned unchanged.
async fn hydrate_one_archive(
    archive: serde_json::Value,
    borg_repo: &str,
    env: &HashMap<String, String>,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<serde_json::Value, ApiError> {
    if !archive_metadata_missing(&archive) {
        return Ok(archive);
    }

    let archive_name = archive
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if archive_name.is_empty() {
        return Ok(archive);
    }

    let metadata =
        fetch_archive_metadata_with_retry(borg_repo, env, archive_name, timeout, task_registry)
            .await?;

    let mut merged = archive;
    for field in ["hostname", "end"] {
        let needs_update = merged
            .get(field)
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(str::is_empty));
        if needs_update
            && let Some(new_value) = metadata.get(field)
            && let Some(obj) = merged.as_object_mut()
        {
            obj.insert(field.to_string(), new_value.clone());
        }
    }
    Ok(merged)
}

async fn hydrate_archives_with_metadata(
    args: HydrateArchivesArgs<'_>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let HydrateArchivesArgs {
        pool,
        ui_broadcast,
        repo_id,
        borg_repo,
        env,
        timeout,
        archives,
        task_registry,
    } = args;
    let total = archives.len();
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);

    let mut hydrated = Vec::with_capacity(total);
    let mut completed = 0usize;
    #[allow(
        clippy::unnecessary_to_owned,
        reason = "false positive: dropping .copied() makes `archive` a &&Value, and the .clone() \
                  inside hydrate_one_archive then clones the outer reference instead of producing \
                  an owned Value, which fails to type-check"
    )]
    for archive in archives.iter().copied() {
        let archive_name = archive
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let next_position = completed.saturating_add(1);
        let loading_msg = if archive_name.is_empty() {
            format!("Loading archive metadata... ({next_position}/{total})")
        } else {
            format!("Loading metadata for '{archive_name}' ({next_position}/{total})")
        };
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            i32::try_from(next_position).unwrap_or(i32::MAX),
            total_i32,
            Some(&loading_msg),
        )
        .await;

        let archive =
            hydrate_one_archive(archive.clone(), borg_repo, env, timeout, task_registry).await?;

        completed = completed.saturating_add(1);
        let loaded_msg = if archive_name.is_empty() {
            format!("Loading archive metadata... ({completed}/{total})")
        } else {
            format!("Loaded metadata for '{archive_name}' ({completed}/{total})")
        };
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            i32::try_from(completed).unwrap_or(i32::MAX),
            total_i32,
            Some(&loaded_msg),
        )
        .await;
        hydrated.push(archive);
    }

    Ok(hydrated)
}

async fn publish_import_progress(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    progress: i32,
    total: i32,
    message: Option<&str>,
) {
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress,
        total,
        message: message.map(str::to_owned),
    });
    let _ =
        db::update_repo_import_progress(pool, repo_id, i64::from(progress), i64::from(total)).await;
    let _ = db::set_import_status_message(pool, repo_id, message).await;
}

/// Clear the import progress state for a repository.
pub async fn clear_import_progress_state(pool: &PgPool, ui_broadcast: &UiBroadcast, repo_id: i64) {
    ui_broadcast.clear_import_progress(repo_id);
    let _ = db::update_repo_import_progress(pool, repo_id, 0, 0).await;
    let _ = db::set_import_status_message(pool, repo_id, None).await;
}

/// Mark a repository as being synced by the server, returning a guard that
/// clears the marker when dropped (including on panic), so a cancelled or
/// panicking task doesn't leave the entry permanently "active". Callers must
/// clear via the guard (`clear_now`), not [`clear_server_sync_op`]: the
/// guard's token-checked clear can't clobber a *different* `ServerSync` op
/// that's since claimed the same `repo_id` (e.g. a scheduled sync started
/// after this manual one was cancelled before ever acquiring `repo_lock`),
/// whereas an unconditional plain clear could.
#[must_use]
pub async fn set_server_sync_op(
    state: &AppState,
    repo_id: i64,
) -> crate::repo_op_tracker::RepoOpGuard {
    let guard = state
        .repo_op_tracker
        .set_guarded(
            repo_id,
            shared::protocol::RepoOpKind::ServerSync,
            "server".to_owned(),
            state.task_registry.clone(),
        )
        .await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });
    guard
}

/// Shared tail for every task that claimed [`set_server_sync_op`]'s guard:
/// marks the import task finished, clears the guard's own entry (a no-op if
/// a different operation already reclaimed this `repo_id`), and broadcasts
/// the cleared state to the UI.
pub async fn finish_server_sync_task(
    import_tasks: &crate::ImportTaskRegistry,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    task_id: u64,
    op_clear_guard: crate::repo_op_tracker::RepoOpGuard,
) {
    import_tasks.finish(repo_id, task_id).await;
    op_clear_guard.clear_now().await;
    ui_broadcast.send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });
}

/// Forcibly clear the server-sync operation marker for a repository,
/// regardless of which operation currently holds it. Only for
/// [`reset_import`]'s "unstick a stuck import" recovery path, which is
/// meant to override whatever's there - anything holding its own
/// [`set_server_sync_op`] guard should clear via `clear_now` instead so it
/// only ever clears its own operation.
async fn clear_server_sync_op(state: &AppState, repo_id: i64) {
    state.repo_op_tracker.clear(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });
}

/// Builds backup-report rows for a set of borg archives, resolving each archive's
/// owning agent by hostname (creating an imported placeholder when unmatched).
///
/// Shared by full and incremental sync so agent resolution lives in one place.
/// Hostname and end time are expected to be present by the time this runs.
/// Progress is published per archive against `total = archives.len()`.
async fn build_import_reports(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    archives: &[&serde_json::Value],
) -> Result<Vec<db::InsertReportParams>, ApiError> {
    let total = archives.len();

    let mut hostname_cache: HashMap<String, (i64, bool)> = HashMap::new();
    let mut report_params = Vec::with_capacity(total);
    for (processed, archive) in archives.iter().enumerate() {
        let name = archive
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let Some(hostname) = archive_hostname(archive) else {
            warn!(repo_id, archive = %name, "skipping archive with missing hostname metadata");
            continue;
        };

        let (agent_id, matched) = if let Some(&cached) = hostname_cache.get(hostname) {
            cached
        } else {
            let (agent, matched) = match db::resolve_agent_for_hostname(pool, hostname).await? {
                db::ResolveResult::ExactMatch(c) | db::ResolveResult::PatternMatch(c) => (c, true),
                db::ResolveResult::Unmatched => {
                    let c = db::get_or_create_agent_by_hostname(pool, hostname).await?;
                    (c, false)
                }
            };
            let entry = (agent.id, matched);
            hostname_cache.insert(hostname.to_string(), entry);
            entry
        };

        let Some(started_at) = parse_borg_timestamp(
            archive
                .get("start")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        ) else {
            warn!(repo_id, archive = %name, "skipping archive with unparseable start timestamp");
            continue;
        };
        let finished_at = archive_finish_time(archive, started_at);
        let duration_secs = finished_at
            .signed_duration_since(started_at)
            .num_seconds()
            .max(0);

        let processed_count = i32::try_from(processed.saturating_add(1)).unwrap_or(i32::MAX);
        info!(repo_id, archive = %name, processed = processed_count, total, "archive imported");
        let progress_msg = format!("Imported \u{2018}{name}\u{2019} ({processed_count}/{total})");
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            processed_count,
            i32::try_from(total).unwrap_or(i32::MAX),
            Some(&progress_msg),
        )
        .await;

        report_params.push(db::InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: None,
            started_at,
            finished_at,
            status: "success".to_string(),
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched,
            archive_name: Some(name.to_string()),
            borg_command: None,
            run_id: None,
        });
    }

    Ok(report_params)
}

#[derive(Clone, Copy)]
enum SyncMode<'a> {
    /// Full sync: import every archive and prune DB records for archives no
    /// longer present in the repository.
    Existing,
    /// Incremental sync: import only archives not already known, and queue
    /// content indexing for the newly imported archives.
    New { repo_lock: &'a RepoLock },
}

/// Returns `true` when a borg archive JSON entry has a non-empty name that is
/// not already recorded for the repository (i.e. it is new and should be
/// imported during an incremental sync).
fn is_unknown_archive(
    archive: &serde_json::Value,
    known_names: &std::collections::HashSet<String>,
) -> bool {
    archive["name"]
        .as_str()
        .is_some_and(|n| !n.is_empty() && !known_names.contains(n))
}

struct ArchiveSyncDiff<'a> {
    borg_names: std::collections::HashSet<String>,
    removed: u64,
    to_import: Vec<&'a serde_json::Value>,
}

/// Compares the archives reported by `borg list` against what's already
/// known for this repository. In [`SyncMode::Existing`] mode, also prunes
/// local records for archives that no longer exist upstream.
async fn partition_archives_to_sync<'a>(
    pool: &PgPool,
    repo_id: i64,
    archives: &'a [serde_json::Value],
    mode: SyncMode<'_>,
) -> Result<ArchiveSyncDiff<'a>, ApiError> {
    let borg_names: std::collections::HashSet<String> = archives
        .iter()
        .filter_map(|a| a["name"].as_str())
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect();

    let known_names = db::list_archive_names_for_repo(pool, repo_id).await?;

    let removed = match mode {
        SyncMode::Existing => {
            let stale: Vec<String> = known_names.difference(&borg_names).cloned().collect();
            let removed = db::delete_archive_records_by_names(pool, repo_id, &stale).await?;
            if removed > 0 {
                info!(repo_id, removed, "removed stale archives during full sync");
            }
            removed
        }
        SyncMode::New { .. } => 0,
    };

    let to_import: Vec<&serde_json::Value> = match mode {
        SyncMode::Existing => archives.iter().collect(),
        SyncMode::New { .. } => archives
            .iter()
            .filter(|a| is_unknown_archive(a, &known_names))
            .collect(),
    };

    Ok(ArchiveSyncDiff {
        borg_names,
        removed,
        to_import,
    })
}

struct ImportOutcome {
    processed: u64,
    archive_names: Vec<String>,
}

/// Hydrates metadata for the archives selected for import, builds and
/// persists their backup report rows, and kicks off background stat
/// enrichment. Returns how many reports were saved and the archive names
/// that were imported.
#[allow(
    clippy::too_many_arguments,
    reason = "grouping these into a struct would obscure the call site more than it would clarify \
              it; all params are single-use scalars/refs from the caller's own locals"
)]
async fn import_and_persist_archives(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    timeout: Duration,
    to_import: &[&serde_json::Value],
    importing_msg: &str,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<ImportOutcome, ApiError> {
    let total = to_import.len();
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        i32::try_from(total).unwrap_or(i32::MAX),
        Some(importing_msg),
    )
    .await;

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        i32::try_from(total).unwrap_or(i32::MAX),
        Some("Loading archive metadata..."),
    )
    .await;
    let hydrated_archives = hydrate_archives_with_metadata(HydrateArchivesArgs {
        pool,
        ui_broadcast,
        repo_id,
        borg_repo,
        env,
        timeout,
        archives: to_import,
        task_registry,
    })
    .await?;
    let hydrated_refs: Vec<&serde_json::Value> = hydrated_archives.iter().collect();

    let report_params = build_import_reports(pool, ui_broadcast, repo_id, &hydrated_refs).await?;

    let processed = u64::try_from(report_params.len()).unwrap_or(u64::MAX);
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);
    let save_msg = format!("Saving {processed} backup reports\u{2026}");
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some(&save_msg),
    )
    .await;

    let archive_names: Vec<String> = report_params
        .iter()
        .filter_map(|params| params.archive_name.clone())
        .collect();
    db::bulk_insert_backup_reports(pool, &report_params).await?;
    enrich_archive_stats_background(
        pool.clone(),
        borg_repo.to_owned(),
        env.clone(),
        repo_id,
        archive_names.clone(),
        background_task_tracker,
        task_registry.clone(),
    );

    Ok(ImportOutcome {
        processed,
        archive_names,
    })
}

async fn sync_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    mode: SyncMode<'_>,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(u64, u64), ApiError> {
    let (borg_repo, env) = super::archives::get_repo_env(pool, encryption_key, repo_id).await?;
    let timeout = get_borg_timeout(pool).await;
    let stage_timeout = get_borg_list_stage_timeout(pool).await;

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        0,
        Some("Listing archives\u{2026}"),
    )
    .await;

    let archives = run_borg_list_with_retry(BorgListRetryArgs {
        borg_repo: &borg_repo,
        env: &env,
        timeout,
        stage_timeout,
        pool,
        ui_broadcast,
        repo_id,
        task_registry,
    })
    .await?;

    let ArchiveSyncDiff {
        borg_names,
        removed,
        to_import,
    } = partition_archives_to_sync(pool, repo_id, &archives, mode).await?;

    let repo_archive_count = i64::try_from(borg_names.len()).unwrap_or(i64::MAX);

    if to_import.is_empty() {
        refresh_repo_info_stats(
            pool,
            &borg_repo,
            &env,
            repo_id,
            repo_archive_count,
            timeout,
            task_registry,
        )
        .await;
        return Ok((0, removed));
    }

    let total = to_import.len();
    let importing_msg = match mode {
        SyncMode::Existing => format!("Importing {total} archives\u{2026}"),
        SyncMode::New { .. } => format!("Importing {total} new archives\u{2026}"),
    };

    let ImportOutcome {
        processed,
        archive_names,
    } = import_and_persist_archives(
        pool,
        ui_broadcast,
        repo_id,
        &borg_repo,
        &env,
        timeout,
        &to_import,
        &importing_msg,
        background_task_tracker,
        task_registry,
    )
    .await?;

    finish_archive_sync(FinishArchiveSyncArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        borg_repo: &borg_repo,
        env: &env,
        mode,
        background_task_tracker,
        task_registry,
        archive_names,
        repo_archive_count,
        timeout,
        total,
        processed,
    })
    .await;

    Ok((processed, removed))
}

struct FinishArchiveSyncArgs<'a> {
    pool: &'a PgPool,
    encryption_key: &'a [u8; 32],
    repo_id: i64,
    ui_broadcast: &'a UiBroadcast,
    borg_repo: &'a str,
    env: &'a std::collections::HashMap<String, String>,
    mode: SyncMode<'a>,
    background_task_tracker: &'a crate::background_tasks::BackgroundTaskTracker,
    task_registry: &'a shared::task_registry::TaskRegistry,
    archive_names: Vec<String>,
    repo_archive_count: i64,
    timeout: Duration,
    total: usize,
    processed: u64,
}

/// Queues content indexing for newly-imported archives (incremental sync only),
/// refreshes the repo's authoritative stats, and logs completion. Split out of
/// [`sync_archives`] to stay under clippy's function-length limit.
async fn finish_archive_sync(args: FinishArchiveSyncArgs<'_>) {
    let FinishArchiveSyncArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        borg_repo,
        env,
        mode,
        background_task_tracker,
        task_registry,
        archive_names,
        repo_archive_count,
        timeout,
        total,
        processed,
    } = args;

    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);

    if let SyncMode::New { repo_lock } = mode {
        queue_archive_indexing(QueueArchiveIndexingArgs {
            pool,
            encryption_key,
            repo_id,
            archive_names: &archive_names,
            repo_lock,
            background_task_tracker,
            task_registry,
            sync_kind: "incremental sync",
        })
        .await;
    }

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some("Refreshing repository statistics\u{2026}"),
    )
    .await;
    refresh_repo_info_stats(
        pool,
        borg_repo,
        env,
        repo_id,
        repo_archive_count,
        timeout,
        task_registry,
    )
    .await;

    match mode {
        SyncMode::Existing => {
            info!(
                repo_id,
                imported = processed,
                total,
                "synced existing archives"
            );
        }
        SyncMode::New { .. } => {
            info!(
                repo_id,
                added = processed,
                total,
                "incremental sync complete"
            );
        }
    }
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn sync_existing_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        SyncMode::Existing,
        background_task_tracker,
        task_registry,
    )
    .await
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn sync_new_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    repo_lock: &RepoLock,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        SyncMode::New { repo_lock },
        background_task_tracker,
        task_registry,
    )
    .await
}

fn enrich_archive_stats_background(
    pool: PgPool,
    borg_repo: String,
    env: std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_names: Vec<String>,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: shared::task_registry::TaskRegistry,
) {
    // Guard is claimed synchronously, before the task is spawned - any_active()
    // must read true the instant this function returns, not merely once the
    // scheduler gets around to polling the new task for the first time (which
    // depends on incidental yield points elsewhere in the caller, not on any
    // synchronization guarantee).
    let task_guard = background_task_tracker.begin();
    tokio::spawn(async move {
        let _task_guard = task_guard;
        // Immutable archives that already have stats never change, so only query
        // borg for the ones still missing them.
        let needing = match db::list_archive_names_needing_stats(&pool, repo_id).await {
            Ok(names) => names,
            Err(e) => {
                warn!(repo_id, error = %e, "stat enrichment: no archives needing stats");
                return;
            }
        };
        let archive_names: Vec<String> = archive_names
            .into_iter()
            .filter(|name| needing.contains(name))
            .collect();
        if archive_names.is_empty() {
            return;
        }
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        let futures: Vec<_> = archive_names
            .into_iter()
            .map(|archive_name| {
                let pool = pool.clone();
                let borg_repo = borg_repo.clone();
                let env = env.clone();
                let semaphore = std::sync::Arc::clone(&semaphore);
                let task_registry = task_registry.clone();
                async move {
                    let _permit = semaphore.acquire_owned().await;
                    enrich_single_archive_stats(
                        &pool,
                        &borg_repo,
                        &env,
                        repo_id,
                        &archive_name,
                        &task_registry,
                    )
                    .await;
                }
            })
            .collect();
        join_all(futures).await;
        info!(repo_id, "stat enrichment: completed");
    });
}

/// Runs `borg info --json` for a single archive and returns the parsed
/// top-level JSON document, logging and returning `None` on any failure
/// (spawn error, non-zero exit, unparseable output).
async fn run_borg_info_for_stats(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_name: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Option<serde_json::Value> {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let output = match Borg::new()
        .with_registry(task_registry.clone())
        .run(
            &[
                "info",
                "--json",
                "--lock-wait",
                LOCK_WAIT_SECS,
                &repo_archive,
            ],
            env,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => {
            warn!(
                repo_id,
                archive = %archive_name,
                error = %e,
                "stat enrichment: failed to run borg info"
            );
            return None;
        }
    };

    if !output.status.success() {
        warn!(
            repo_id,
            archive = %archive_name,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "stat enrichment: borg info non-zero exit"
        );
        return None;
    }

    match serde_json::from_slice(&output.stdout) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(
                repo_id,
                archive = %archive_name,
                error = %e,
                "stat enrichment: failed to parse borg info output"
            );
            None
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "borg durations are small positive second counts; removal tracked in #284"
)]
fn parse_archive_stats(json: &serde_json::Value, info: &serde_json::Value) -> db::ArchiveStats {
    let raw_stats = info.get("stats");
    db::ArchiveStats {
        original_size: raw_stats
            .and_then(|s| s.get("original_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        compressed_size: raw_stats
            .and_then(|s| s.get("compressed_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        deduplicated_size: raw_stats
            .and_then(|s| s.get("deduplicated_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        files_processed: raw_stats
            .and_then(|s| s.get("nfiles"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        duration_secs: info
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as i64,
        repo_unique_csize: json
            .get("cache")
            .and_then(|v| v.get("stats"))
            .and_then(|v| v.get("unique_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
    }
}

async fn enrich_single_archive_stats(
    pool: &PgPool,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_name: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) {
    let Some(json) =
        run_borg_info_for_stats(borg_repo, env, repo_id, archive_name, task_registry).await
    else {
        return;
    };

    let Some(info) = json
        .get("archives")
        .and_then(serde_json::Value::as_array)
        .and_then(|a| a.first())
    else {
        warn!(
            repo_id,
            archive = %archive_name,
            "stat enrichment: borg info returned no archive entry"
        );
        return;
    };

    let archive_stats = parse_archive_stats(&json, info);

    if let Err(e) =
        db::update_backup_report_stats(pool, repo_id, archive_name, &archive_stats).await
    {
        warn!(
            repo_id,
            archive = %archive_name,
            error = %e,
            "stat enrichment: failed to update stats"
        );
    } else {
        info!(
            repo_id,
            archive = %archive_name,
            original_size = archive_stats.original_size,
            "stat enrichment: updated archive stats"
        );
    }
}

struct QueueArchiveIndexingArgs<'a> {
    pool: &'a PgPool,
    encryption_key: &'a [u8; 32],
    repo_id: i64,
    archive_names: &'a [String],
    repo_lock: &'a RepoLock,
    background_task_tracker: &'a crate::background_tasks::BackgroundTaskTracker,
    task_registry: &'a shared::task_registry::TaskRegistry,
    sync_kind: &'a str,
}

async fn queue_archive_indexing(args: QueueArchiveIndexingArgs<'_>) {
    let QueueArchiveIndexingArgs {
        pool,
        encryption_key,
        repo_id,
        archive_names,
        repo_lock,
        background_task_tracker,
        task_registry,
        sync_kind,
    } = args;
    join_all(archive_names.iter().map(|archive_name| {
        let pool = pool.clone();
        let repo_lock = repo_lock.clone();
        let task_registry = task_registry.clone();
        async move {
            if let Err(e) = archive_index::ensure_indexed(
                pool,
                *encryption_key,
                repo_id,
                archive_name.clone(),
                repo_lock,
                background_task_tracker,
                task_registry,
            )
            .await
            {
                warn!(
                    repo_id,
                    archive = %archive_name,
                    sync_kind,
                    error = %e,
                    "failed to queue archive indexing"
                );
            }
        }
    }))
    .await;
}

/// Builds the content index for every archive in the repository that is not
/// already fully indexed, one archive at a time, broadcasting progress so the
/// UI can show which archive (and how many files) is currently being processed.
/// Already-indexed archives are skipped: borg archives are immutable, so a
/// finished index never needs rebuilding.
async fn index_archives_with_progress(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    ui_broadcast: UiBroadcast,
    repo_lock: RepoLock,
    repo_lock_held: bool,
    task_registry: shared::task_registry::TaskRegistry,
) {
    let all = match db::list_archive_names_for_repo(&pool, repo_id).await {
        Ok(names) => names,
        Err(e) => {
            warn!(repo_id, error = %e, "content indexing: failed to list archives");
            return;
        }
    };
    let done = match archive_index::list_indexed_archive_names(&pool, repo_id).await {
        Ok(names) => names,
        Err(e) => {
            warn!(repo_id, error = %e, "content indexing: failed to list indexed archives");
            return;
        }
    };

    let mut pending: Vec<String> = all.difference(&done).cloned().collect();
    pending.sort_unstable();
    let total = pending.len();
    if total == 0 {
        return;
    }
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);
    info!(
        repo_id,
        total, "content indexing: starting full resync index"
    );

    for (index, archive_name) in pending.iter().enumerate() {
        index_one_archive(IndexOneArchiveArgs {
            pool: &pool,
            encryption_key: &encryption_key,
            repo_id,
            ui_broadcast: &ui_broadcast,
            repo_lock: &repo_lock,
            repo_lock_held,
            archive_name,
            index,
            total,
            total_i32,
            task_registry: &task_registry,
        })
        .await;
    }

    publish_import_progress(
        &pool,
        &ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some("Indexing complete"),
    )
    .await;
    info!(repo_id, total, "content indexing: completed");
}

struct IndexOneArchiveArgs<'a> {
    pool: &'a PgPool,
    encryption_key: &'a [u8; 32],
    repo_id: i64,
    ui_broadcast: &'a UiBroadcast,
    repo_lock: &'a RepoLock,
    repo_lock_held: bool,
    archive_name: &'a str,
    index: usize,
    total: usize,
    total_i32: i32,
    task_registry: &'a shared::task_registry::TaskRegistry,
}

/// Indexes a single archive's contents, broadcasting progress as it goes.
///
/// The progress bar tracks *completed* archives, so it never reaches 100%
/// while an archive (including the last one) is still being scanned. The
/// live file count and current path keep the badge visibly moving even
/// while a single large archive is being scanned.
async fn index_one_archive(args: IndexOneArchiveArgs<'_>) {
    let IndexOneArchiveArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        repo_lock,
        repo_lock_held,
        archive_name,
        index,
        total,
        total_i32,
        task_registry,
    } = args;

    let completed = i32::try_from(index).unwrap_or(i32::MAX);
    let human_position = index.saturating_add(1);
    let archive_msg =
        format!("Indexing contents of \u{2018}{archive_name}\u{2019} ({human_position}/{total})");
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        completed,
        total_i32,
        Some(&archive_msg),
    )
    .await;

    if let Err(e) = archive_index::ensure_index_job(pool, repo_id, archive_name).await {
        warn!(repo_id, archive = %archive_name, error = %e, "content index job failed");
        return;
    }

    let mut on_progress = |file_count: u64, current: Option<&str>| {
        let message = current.map_or_else(
            || {
                format!(
                    "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \u{2014} \
                     {file_count} files"
                )
            },
            |path| {
                format!(
                    "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \u{2014} \
                     {file_count} files \u{00b7} {path}"
                )
            },
        );
        ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
            repo_id,
            progress: completed,
            total: total_i32,
            message: Some(message),
        });
    };

    let result = if repo_lock_held {
        archive_index::run_indexing_with_lock_held(
            pool,
            encryption_key,
            repo_id,
            archive_name,
            &mut on_progress,
            task_registry,
        )
        .await
    } else {
        archive_index::run_indexing(
            pool,
            encryption_key,
            repo_id,
            archive_name,
            repo_lock,
            &mut on_progress,
            task_registry,
        )
        .await
    };

    if let Err(e) = result {
        warn!(repo_id, archive = %archive_name, error = %e, "content indexing: archive failed");
    }
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
    tokio::spawn(run_repo_sync_task(RepoSyncTask {
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
    }));

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
    let _ = db::insert_system_event(pool, "repo_sync_failed", None, &msg).await;
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
        let _ = db::insert_system_event(pool, "repo_sync_slow", None, &warn_msg).await;
    }

    info!("{msg}");
    let _ = db::insert_system_event(pool, "repo_sync", None, &msg).await;
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
    db::get_repo_with_stats(&state.pool, repo_id).await?;
    let cancelled = state.import_tasks.cancel(repo_id).await;
    if cancelled {
        info!(repo_id, "cancelled active import task");
    }
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
    tokio::spawn(run_repo_sync_task(RepoSyncTask {
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
    }));

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
    use chrono::Datelike as _;

    use super::*;

    #[test]
    fn is_lock_error_detects_lock_create_message() {
        assert!(is_lock_error(
            "Failed to create/acquire the lock /repo/lock.exclusive"
        ));
    }

    #[test]
    fn is_lock_error_detects_lock_exclusive() {
        assert!(is_lock_error("waiting for lock.exclusive to be released"));
    }

    #[test]
    fn is_lock_error_detects_lockroster() {
        assert!(is_lock_error("LockRoster: another process holds the lock"));
    }

    #[test]
    fn is_lock_error_is_case_insensitive() {
        assert!(is_lock_error("FAILED TO CREATE/ACQUIRE THE LOCK"));
        assert!(is_lock_error("LOCK.EXCLUSIVE"));
        assert!(is_lock_error("LOCKROSTER"));
    }

    #[test]
    fn is_lock_error_returns_false_for_unrelated_errors() {
        assert!(!is_lock_error("Repository does not exist"));
        assert!(!is_lock_error("passphrase is incorrect"));
        assert!(!is_lock_error("Connection refused"));
        assert!(!is_lock_error(""));
    }

    #[test]
    fn is_lock_error_returns_false_for_partial_matches() {
        assert!(!is_lock_error("lock timeout after 60 seconds"));
        assert!(!is_lock_error("unlock successful"));
    }

    #[test]
    fn parse_borg_timestamp_rfc3339() {
        let ts = parse_borg_timestamp("2024-03-15T10:30:00+00:00");
        assert!(ts.is_some());
        let dt = ts.unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 3);
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn parse_borg_timestamp_naive_with_fraction() {
        let ts = parse_borg_timestamp("2024-06-01T08:00:00.123456");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_borg_timestamp_naive_without_fraction() {
        let ts = parse_borg_timestamp("2024-06-01T08:00:00");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_borg_timestamp_empty_returns_none() {
        assert!(parse_borg_timestamp("").is_none());
    }

    #[test]
    fn parse_borg_timestamp_invalid_returns_none() {
        assert!(parse_borg_timestamp("not-a-date").is_none());
        assert!(parse_borg_timestamp("2024-13-01T00:00:00").is_none());
    }

    #[test]
    fn borg_list_args_do_not_request_archive_metadata_format() {
        let args = borg_list_args("ssh://borg@example.test/repo");
        assert_eq!(
            args,
            [
                "list",
                "--json",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--",
                "ssh://borg@example.test/repo"
            ]
        );
    }

    #[test]
    fn extract_borg_error_returns_first_line_when_no_traceback() {
        let stderr = "Repository does not exist.\nsome extra context\n";
        assert_eq!(extract_borg_error(stderr), "Repository does not exist.");
    }

    #[test]
    fn extract_borg_error_returns_whole_input_when_single_line() {
        assert_eq!(extract_borg_error("boom"), "boom");
    }

    #[test]
    fn extract_borg_error_picks_exception_line_from_traceback() {
        let stderr = "Traceback (most recent call last):\n  File \"borg/archiver.py\", line 1, in \
                      main\n    do_thing()\nValueError: invalid passphrase\nPlatform: Linux x86_64";
        assert_eq!(extract_borg_error(stderr), "ValueError: invalid passphrase");
    }

    #[test]
    fn extract_borg_error_stops_at_platform_marker() {
        let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 2\nRuntimeError: \
                      lock timeout\nPlatform: Linux\nLater: noise";
        assert_eq!(extract_borg_error(stderr), "RuntimeError: lock timeout");
    }

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
    fn is_unknown_archive_detects_new_and_known() {
        use std::collections::HashSet;
        let known: HashSet<String> = ["host-2024-01-01".to_string()].into_iter().collect();
        assert!(is_unknown_archive(
            &serde_json::json!({"name": "host-2024-02-02"}),
            &known
        ));
        assert!(!is_unknown_archive(
            &serde_json::json!({"name": "host-2024-01-01"}),
            &known
        ));
    }

    #[test]
    fn is_unknown_archive_rejects_empty_or_missing_name() {
        use std::collections::HashSet;
        let known: HashSet<String> = HashSet::new();
        assert!(!is_unknown_archive(
            &serde_json::json!({"name": ""}),
            &known
        ));
        assert!(!is_unknown_archive(&serde_json::json!({"size": 1}), &known));
    }

    #[test]
    fn archive_hostname_returns_hostname_when_present() {
        let archive = serde_json::json!({"hostname": "web-01.example.com"});
        assert_eq!(archive_hostname(&archive), Some("web-01.example.com"));
    }

    #[test]
    fn archive_hostname_returns_none_when_missing_or_empty() {
        assert_eq!(archive_hostname(&serde_json::json!({})), None);
        assert_eq!(archive_hostname(&serde_json::json!({"hostname": ""})), None);
        assert_eq!(
            archive_hostname(&serde_json::json!({"hostname": null})),
            None
        );
    }

    #[test]
    fn archive_finish_time_uses_end_timestamp_when_present() {
        let archive = serde_json::json!({"end": "2024-06-01T08:00:00+00:00"});
        let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
            .unwrap()
            .to_utc();
        let finished = archive_finish_time(&archive, started_at);
        assert_eq!(finished, started_at + chrono::Duration::hours(1));
    }

    #[test]
    fn archive_finish_time_falls_back_to_duration_when_end_missing() {
        let archive = serde_json::json!({"duration": 3600.0});
        let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
            .unwrap()
            .to_utc();
        let finished = archive_finish_time(&archive, started_at);
        assert_eq!(finished, started_at + chrono::Duration::hours(1));
    }

    #[test]
    fn archive_finish_time_falls_back_to_started_at_when_no_end_or_duration() {
        let archive = serde_json::json!({});
        let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
            .unwrap()
            .to_utc();
        let finished = archive_finish_time(&archive, started_at);
        assert_eq!(finished, started_at);
    }

    #[test]
    fn archive_metadata_missing_true_when_hostname_or_end_missing() {
        assert!(archive_metadata_missing(&serde_json::json!({})));
        assert!(archive_metadata_missing(
            &serde_json::json!({"hostname": "h1"})
        ));
        assert!(archive_metadata_missing(
            &serde_json::json!({"end": "2024-01-01T00:00:00Z"})
        ));
    }

    #[test]
    fn archive_metadata_missing_false_when_hostname_and_end_present() {
        assert!(!archive_metadata_missing(&serde_json::json!({
            "hostname": "h1",
            "end": "2024-01-01T00:00:00Z"
        })));
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
            compression: "lz4".into(),
            encryption: "repokey".into(),
            enabled: true,
            owner_id: None,
            visibility: "private".into(),
            sync_schedule: None,
        };
        let resp = RepoResponse::from(row);
        assert_eq!(resp.compression, shared::types::Compression::Lz4);
        assert_eq!(resp.encryption, shared::types::BorgEncryption::Repokey);
    }

    #[test]
    fn repo_row_from_invalid_compression_falls_back_to_default() {
        let row = db::RepoRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            compression: "garbage_algorithm".into(),
            encryption: "repokey_blake2".into(),
            enabled: true,
            owner_id: None,
            visibility: "private".into(),
            sync_schedule: None,
        };
        let resp = RepoResponse::from(row);
        assert_eq!(resp.compression, shared::types::Compression::Lz4);
    }

    #[test]
    fn repo_row_from_invalid_encryption_falls_back_to_default() {
        let row = db::RepoRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            compression: "lz4".into(),
            encryption: "bogus_encryption".into(),
            enabled: true,
            owner_id: None,
            visibility: "private".into(),
            sync_schedule: None,
        };
        let resp = RepoResponse::from(row);
        assert_eq!(resp.encryption, shared::types::BorgEncryption::Repokey);
    }

    #[test]
    fn repo_with_stats_row_from_invalid_last_op_kind_silently_drops() {
        let row = db::RepoWithStatsRow {
            id: 1,
            name: "test".into(),
            repo_path: "/repo".into(),
            ssh_user: "borg".into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            ssh_host_key: None,
            compression: "lz4".into(),
            encryption: "repokey".into(),
            enabled: true,
            importing: false,
            import_error: None,
            import_progress: 0,
            import_total: 0,
            import_status_message: None,
            owner_id: None,
            visibility: "private".into(),
            sync_schedule: None,
            last_synced_at: None,
            archive_count: 0,
            last_backup_at: None,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            agent_count: 0,
            unmatched_count: 0,
            last_op_kind: Some("bogus_op".into()),
            relocation_pending: false,
            last_op_at: None,
            last_op_by: None,
        };
        let resp = RepoWithStatsResponse::from(row);
        assert_eq!(resp.last_op_kind, None);
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
            compression: "lz4".into(),
            encryption: "repokey".into(),
            enabled: true,
            importing: false,
            import_error: None,
            import_progress: 0,
            import_total: 0,
            import_status_message: None,
            owner_id: None,
            visibility: "private".into(),
            sync_schedule: None,
            last_synced_at: None,
            archive_count: 0,
            last_backup_at: None,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            agent_count: 0,
            unmatched_count: 0,
            last_op_kind: Some("agent_backup".into()),
            relocation_pending: false,
            last_op_at: None,
            last_op_by: None,
        };
        let resp = RepoWithStatsResponse::from(row);
        assert_eq!(
            resp.last_op_kind,
            Some(shared::protocol::RepoOpKind::AgentBackup)
        );
    }
}
