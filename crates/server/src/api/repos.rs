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
use serde::{Deserialize, Serialize};
use shared::{
    crypto::encrypt_passphrase,
    types::{BorgEncryption, build_repo_url},
};
use sqlx::PgPool;
use tracing::{error, info, warn};

use super::{
    archives::LOCK_WAIT_SECS,
    auth::{AuthUser, RequireAdmin, Role},
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RepoDetailResponse {
    #[serde(flatten)]
    pub stats: RepoWithStatsRow,
    pub current_op: Option<shared::protocol::ActiveRepoOp>,
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
    summary = "List all repositories",
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_repos(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepoRow>>, ApiError> {
    let repos = db::list_all_repos(&state.pool).await?;
    let is_admin = auth.role == Role::Admin;
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
            visible.push(repo);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/repos",
    tag = "Repositories",
    operation_id = "getAgentRepos",
    summary = "List repositories for a specific host",
    params(
        ("hostname" = String, Path, description = "Agent hostname"),
    ),
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_agent_repos(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<RepoRow>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let repos = db::list_repos_for_agent_public(&state.pool, agent.id).await?;
    Ok(Json(repos))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRepoRequest {
    pub name: String,
    pub repo_path: String,
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<i32>,
    pub passphrase: String,
    pub compression: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRepoRequest {
    pub name: Option<String>,
    pub repo_path: String,
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<i32>,
    pub compression: Option<String>,
    #[schema(value_type = Option<String>)]
    pub encryption: Option<BorgEncryption>,
    pub enabled: Option<bool>,
    pub sync_schedule: Option<Option<String>>,
}

#[utoipa::path(
    post,
    path = "/api/repos",
    tag = "Repositories",
    operation_id = "createRepo",
    summary = "Create a new repository",
    request_body = CreateRepoRequest,
    responses(
        (status = 201, description = "Repository created", body = RepoRow),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn create_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateRepoRequest>,
) -> Result<(StatusCode, Json<RepoRow>), ApiError> {
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
    let info_result: Option<BorgInfoResult> =
        match tokio::time::timeout(info_timeout, run_borg_info(&repo_url, &req.passphrase)).await {
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

    db::set_repo_importing(&state.pool, repo_id, true).await?;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);

    tokio::spawn(async move {
        // Detect encryption before syncing rather than concurrently: both borg
        // info and the sync's borg list contend for the same repository lock, so
        // running them in parallel would force the list into the lock-retry path.
        if need_borg_info {
            match run_borg_info_with_retry(&bg_repo_url, &bg_passphrase).await {
                Ok(info) => {
                    if let Err(e) =
                        db::update_repo_encryption(&pool, repo_id, &info.encryption.to_string())
                            .await
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

        let sync_ok = match sync_existing_archives(&pool, &encryption_key, repo_id, &ui_broadcast)
            .await
        {
            Ok(_) => {
                if let Err(e) = db::update_repo_last_synced(&pool, repo_id).await {
                    warn!(repo_id, error = %e, "failed to set last_synced_at after initial import");
                }
                true
            }
            Err(e) => {
                warn!(repo_id, error = %e, "failed to sync existing archives on import");
                let _ = db::set_repo_import_error(&pool, repo_id, Some(&format!("{e}"))).await;
                false
            }
        };

        if sync_ok {
            index_archives_with_progress(
                pool.clone(),
                encryption_key,
                repo_id,
                ui_broadcast.clone(),
                state_repo_lock,
            )
            .await;
        }

        if let Err(e) = db::set_repo_importing(&pool, repo_id, false).await {
            warn!(repo_id, error = %e, "failed to clear importing flag");
        }
        clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
        ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
    });

    Ok((StatusCode::CREATED, Json(repo)))
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "updateRepo",
    summary = "Update a repository (admin only)",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = UpdateRepoRequest,
    responses(
        (status = 200, description = "Updated repository", body = RepoRow),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRepoRequest>,
) -> Result<Json<RepoRow>, ApiError> {
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

    let sync_schedule = req.sync_schedule.unwrap_or(existing.sync_schedule);
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
        sync_schedule: sync_schedule.as_deref(),
    };

    let repo = if location_changed {
        db::update_repo_and_set_relocation_pending(&state.pool, &update_params).await?
    } else {
        db::update_repo(&state.pool, &update_params).await?
    };

    Ok(Json(repo))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "deleteRepo",
    summary = "Remove a repository from the database (admin only)",
    description = "Removes the repository record and associated schedules/reports from the \
                   database. Does NOT delete any data on disk.",
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
    summary = "Destroy a repository from disk and remove from database (admin only)",
    description = "DANGEROUS: Permanently deletes the repository data from the remote filesystem \
                   via SSH (rm -rf) and then removes the database record. This action is \
                   irreversible.",
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

#[derive(Serialize, utoipa::ToSchema)]
pub struct PassphraseResponse {
    pub passphrase: String,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/passphrase",
    tag = "Repositories",
    operation_id = "getRepoPassphrase",
    summary = "Get the decrypted passphrase for a repository (admin only)",
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
pub async fn get_passphrase(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<PassphraseResponse>, ApiError> {
    let encrypted = db::get_repo_passphrase(&state.pool, repo_id).await?;
    let passphrase = shared::crypto::decrypt_passphrase(&encrypted, &state.encryption_key)?;
    Ok(Json(PassphraseResponse { passphrase }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RepoHostKeyResponse {
    pub ssh_host_key: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AcceptRepoHostKeyRequest {
    pub ssh_host_key: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/ssh-host-key/scan",
    tag = "Repositories",
    operation_id = "scanRepoHostKey",
    summary = "Scan the repository host key without saving it",
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
    summary = "Accept a scanned SSH host key and push updated config",
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
    summary = "List repositories with backup statistics",
    responses(
        (status = 200, description = "Repositories with stats", body = Vec<RepoWithStatsRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_repos_with_stats(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<RepoWithStatsRow>>, ApiError> {
    let repos = db::list_repos_with_stats(&state.pool).await?;
    let is_admin = auth.role == Role::Admin;
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
            visible.push(repo);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}",
    tag = "Repositories",
    operation_id = "getRepo",
    summary = "Get a repository with statistics",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Repository with stats", body = RepoDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_repo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoDetailResponse>, ApiError> {
    let stats = db::get_repo_with_stats(&state.pool, repo_id).await?;
    let current_op = state.repo_op_tracker.get(repo_id).await;
    Ok(Json(RepoDetailResponse { stats, current_op }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InitRepoRequest {
    pub name: String,
    pub repo_path: String,
    #[serde(default = "helpers::default_ssh_user")]
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<i32>,
    pub passphrase: String,
    #[schema(value_type = String)]
    pub encryption: BorgEncryption,
    pub compression: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InitRepoResponse {
    pub repo: RepoRow,
    pub borg_output: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/init",
    tag = "Repositories",
    operation_id = "initRepo",
    summary = "Initialize a new borg repository and register it",
    request_body = InitRepoRequest,
    responses(
        (status = 201, description = "Repository initialized", body = InitRepoResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Repository already exists"),
        (status = 502, description = "Borg command failed"),
    )
)]
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

    let borg_output = run_borg_init(&repo_url, &req.passphrase, req.encryption).await?;

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
        },
    )
    .await?;
    db::update_repo_ssh_host_key(&state.pool, repo.id, &ssh_host_key).await?;

    info!(repo_id = repo.id, name = %req.name, "repository initialized");

    Ok((
        StatusCode::CREATED,
        Json(InitRepoResponse { repo, borg_output }),
    ))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConfirmRelocationResponse {
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/confirm-relocation",
    tag = "Repositories",
    operation_id = "confirmRepoRelocation",
    summary = "Accept a borg repository relocation for the next backup run",
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
    summary = "List schedules for a repository",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "List of schedules", body = Vec<crate::db::ScheduleRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn list_schedules_for_repo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<db::ScheduleRow>>, ApiError> {
    let schedules = db::list_schedules_for_repo(&state.pool, repo_id).await?;
    let is_admin = auth.role == Role::Admin;
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BreakLockResponse {
    pub message: String,
    pub borg_output: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/break-lock",
    tag = "Repositories",
    operation_id = "breakRepoLock",
    summary = "Break a stale lock on a borg repository",
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

    let result = run_borg_break_lock(&repo_url, &passphrase).await;

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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ExecBorgRequest {
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExecBorgResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/exec",
    tag = "Repositories",
    operation_id = "execBorgCommand",
    summary = "Execute a borg command against the repository (admin only)",
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
    env.insert("BORG_REPO".to_owned(), repo_url);

    info!(repo_id, name = %repo.name, subcommand = %subcommand, "admin executing borg command");

    let output = Borg::new()
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MigrateEncryptionRequest {
    #[schema(value_type = String)]
    pub target_encryption: BorgEncryption,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MigrateEncryptionResponse {
    pub success: bool,
    pub message: String,
    pub migrated_path: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/migrate-encryption",
    tag = "Repositories",
    operation_id = "migrateRepoEncryption",
    summary = "Migrate repository to a different encryption mode",
    description = "Renames the existing repository and creates a new one at the original path \
                   with the target encryption. Old repo preserved at .migrated-<date> path.",
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
pub async fn migrate_encryption(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
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

    let init_result = run_borg_init(&repo_url, &passphrase, req.target_encryption).await;
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
            user_id: Some(_admin.user_id),
            username: &_admin.username,
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

async fn run_borg_info(repo_url: &str, passphrase: &str) -> Result<BorgInfoResult, ApiError> {
    run_borg_info_once(repo_url, passphrase).await
}

const LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const LOCK_RETRY_MAX_ATTEMPTS: u32 = 60;
const DEFAULT_BORG_QUERY_TIMEOUT_SECS: u64 = 300;

/// Upper bound for a single `borg list`/`borg info` invocation. `--lock-wait`
/// only bounds lock contention; a hung SSH connection would otherwise keep the
/// process (and the import) waiting forever, leaving the repo stuck at "Listing
/// archives". On timeout the process is killed and the operation fails so the
/// importing state is cleared. Tunable via `ASSIMILATE_BORG_QUERY_TIMEOUT_SECS`.
fn borg_query_timeout() -> Duration {
    std::env::var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or_else(
            || Duration::from_secs(DEFAULT_BORG_QUERY_TIMEOUT_SECS),
            Duration::from_secs,
        )
}

async fn run_borg_info_with_retry(
    repo_url: &str,
    passphrase: &str,
) -> Result<BorgInfoResult, ApiError> {
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let attempt_result = match tokio::time::timeout(
            borg_query_timeout(),
            run_borg_info_once(repo_url, passphrase),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(ApiError::Internal(format!(
                "borg info timed out after {}s; the repository may be unreachable",
                borg_query_timeout().as_secs()
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

async fn run_borg_info_once(repo_url: &str, passphrase: &str) -> Result<BorgInfoResult, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
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

async fn run_borg_break_lock(repo_url: &str, passphrase: &str) -> Result<String, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
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
) -> Result<String, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    let output = Borg::new()
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

async fn run_borg_list_with_retry(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
) -> Result<Vec<serde_json::Value>, ApiError> {
    use tokio::io::AsyncReadExt as _;

    let borg = Borg::new();
    // --json (not --json-lines) is the correct flag for listing archives in a repo;
    // --json-lines is only valid when listing the contents of a specific archive.
    let args = ["list", "--json", "--lock-wait", LOCK_WAIT_SECS, borg_repo];
    let overall_start = std::time::Instant::now();

    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let mut child = borg
            .spawn(&args, env)
            .map_err(|e| ApiError::Internal(format!("failed to spawn borg list: {e}")))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| ApiError::Internal("borg list: no stdout pipe".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| ApiError::Internal("borg list: no stderr pipe".into()))?;

        let timed = tokio::time::timeout(borg_query_timeout(), async move {
            let stdout_task = async {
                let mut buf = String::new();
                stdout.read_to_string(&mut buf).await?;
                Ok::<String, std::io::Error>(buf)
            };
            let stderr_task = async {
                let mut buf = Vec::new();
                stderr.read_to_end(&mut buf).await?;
                Ok::<Vec<u8>, std::io::Error>(buf)
            };
            tokio::join!(stdout_task, stderr_task)
        })
        .await;

        let (json_str, stderr_bytes) = match timed {
            Err(_) => {
                let _ = child.kill().await;
                return Err(ApiError::Internal(format!(
                    "borg list timed out after {}s; the repository may be unreachable",
                    borg_query_timeout().as_secs()
                )));
            }
            Ok((Ok(j), Ok(s))) => (j, s),
            Ok((Err(e), _)) | Ok((_, Err(e))) => {
                let _ = child.kill().await;
                return Err(ApiError::Internal(format!("borg list IO error: {e}")));
            }
        };

        // Bound the wait too: borg may close its pipes (reads hit EOF above) yet
        // fail to exit, e.g. a defunct ssh child keeping the session open. Without
        // this guard the import would hang forever with importing = true.
        let status = match tokio::time::timeout(borg_query_timeout(), child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(ApiError::Internal(format!("failed to wait for borg: {e}")));
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err(ApiError::Internal(format!(
                    "borg list timed out after {}s waiting for process exit; the repository may \
                     be unreachable",
                    borg_query_timeout().as_secs()
                )));
            }
        };

        if status.success() {
            // A successful exit with unparseable output must be a hard error: silently
            // treating it as an empty repo would prune every existing archive record.
            let json: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
                ApiError::Internal(format!("borg list returned malformed JSON: {e}"))
            })?;
            let archives = json["archives"].as_array().cloned().ok_or_else(|| {
                ApiError::Internal("borg list JSON missing 'archives' array".to_string())
            })?;
            return Ok(archives);
        }

        let stderr_str = String::from_utf8_lossy(&stderr_bytes);
        if !is_lock_error(&stderr_str) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            return Err(ApiError::Internal(format!(
                "borg list failed: {stderr_str}"
            )));
        }

        let holder = parse_lock_holder(&stderr_str);
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            holder = holder.as_deref().unwrap_or("unknown"),
            "borg list lock contention, retrying in {}s",
            LOCK_RETRY_INTERVAL.as_secs()
        );

        let wait_start = std::time::Instant::now();
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let elapsed = overall_start.elapsed().as_secs();
                    let max = LOCK_RETRY_MAX_ATTEMPTS;
                    let holder_part = holder
                        .as_deref()
                        .map_or_else(String::new, |h| format!(", held by {h}"));
                    let msg = format!(
                        "Waiting for lock\u{2026} attempt {attempt}/{max}, {elapsed}s{holder_part}"
                    );
                    publish_import_progress(pool, ui_broadcast, repo_id, 0, 0, Some(&msg)).await;
                }
                _ = tokio::time::sleep(
                    LOCK_RETRY_INTERVAL.saturating_sub(wait_start.elapsed())
                ) => break,
            }
        }
    }
    Err(ApiError::Internal(
        "borg list failed after maximum retries".to_owned(),
    ))
}

/// Extracts lock holder info from borg's LockTimeout stderr output.
///
/// Borg may include a Python-dict-style `Holder:` entry, e.g.:
/// `Holder: {'hostname': 'web-01', 'pid': 1234, ...}`
fn parse_lock_holder(stderr: &str) -> Option<String> {
    let line = stderr.lines().find(|l| l.contains("Holder:"))?;
    let dict = line.split("Holder:").nth(1)?.trim();
    let hostname = extract_py_str(dict, "hostname");
    let pid = extract_py_num(dict, "pid");
    match (hostname, pid) {
        (Some(h), Some(p)) => Some(format!("{h} (PID {p})")),
        (Some(h), None) => Some(h),
        (None, Some(p)) => Some(format!("PID {p}")),
        (None, None) => None,
    }
}

fn extract_py_str(dict: &str, key: &str) -> Option<String> {
    let prefix = format!("'{key}': '");
    let pos = dict.find(&prefix)?;
    let rest = &dict[pos + prefix.len()..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn extract_py_num(dict: &str, key: &str) -> Option<String> {
    let prefix = format!("'{key}': ");
    let pos = dict.find(&prefix)?;
    let rest = &dict[pos + prefix.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    Some(rest[..end].to_string())
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
) {
    let args = ["info", "--json", "--lock-wait", LOCK_WAIT_SECS, borg_repo];
    let output = match tokio::time::timeout(borg_query_timeout(), Borg::new().run(&args, env)).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!(repo_id, error = %e, "failed to run borg info for repo stats");
            return;
        }
        Err(_) => {
            warn!(
                repo_id,
                timeout_secs = borg_query_timeout().as_secs(),
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

    let stats = &json["cache"]["stats"];
    let info_stats = db::RepoInfoStats {
        original_size: stats["total_size"].as_i64().unwrap_or(0),
        compressed_size: stats["total_csize"].as_i64().unwrap_or(0),
        deduplicated_size: stats["unique_csize"].as_i64().unwrap_or(0),
        total_chunks: stats["total_chunks"].as_i64().unwrap_or(0),
        unique_chunks: stats["total_unique_chunks"].as_i64().unwrap_or(0),
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

pub async fn clear_import_progress_state(pool: &PgPool, ui_broadcast: &UiBroadcast, repo_id: i64) {
    ui_broadcast.clear_import_progress(repo_id);
    let _ = db::update_repo_import_progress(pool, repo_id, 0, 0).await;
    let _ = db::set_import_status_message(pool, repo_id, None).await;
}

/// Fetches per-archive hostname metadata via `borg info --glob-archives * --json`.
///
/// Some borg versions omit the `hostname` field from `borg list --json` output.
/// This fallback issues a single `borg info` call that always populates hostname,
/// and returns a map of archive-name to hostname for use during archive import.
async fn fetch_hostname_fallback(
    borg_repo: &str,
    env: &HashMap<String, String>,
    repo_id: i64,
) -> HashMap<String, String> {
    let args = [
        "info",
        "--glob-archives",
        "*",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        borg_repo,
    ];
    let output = match tokio::time::timeout(borg_query_timeout(), Borg::new().run(&args, env)).await
    {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            warn!(
                repo_id,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "borg info hostname fallback exited non-zero"
            );
            return HashMap::new();
        }
        Ok(Err(e)) => {
            warn!(repo_id, error = %e, "borg info hostname fallback failed to run");
            return HashMap::new();
        }
        Err(_) => {
            warn!(
                repo_id,
                timeout_secs = borg_query_timeout().as_secs(),
                "borg info hostname fallback timed out"
            );
            return HashMap::new();
        }
    };

    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v["archives"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_string();
            let hostname = a["hostname"].as_str()?.to_string();
            (!hostname.is_empty()).then_some((name, hostname))
        })
        .collect()
}

/// Builds backup-report rows for a set of borg archives, resolving each archive's
/// owning agent by hostname (creating an imported placeholder when unmatched).
///
/// Shared by full and incremental sync so the hostname-fallback handling and agent
/// resolution live in one place. `archives` are borg `list --json` entries; progress
/// is published per archive against `total = archives.len()`.
async fn build_import_reports(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    borg_repo: &str,
    env: &HashMap<String, String>,
    archives: &[&serde_json::Value],
) -> Result<Vec<db::InsertReportParams>, ApiError> {
    let total = archives.len();

    // Some borg versions omit hostname from `list --json`; fetch it via `info` if needed.
    let hostname_fallback = if archives
        .iter()
        .any(|a| a["hostname"].as_str().is_none_or(|h| h.is_empty()))
    {
        warn!(
            repo_id,
            "borg list --json omitted hostname; running borg info fallback"
        );
        fetch_hostname_fallback(borg_repo, env, repo_id).await
    } else {
        HashMap::new()
    };

    let mut hostname_cache: HashMap<String, (i64, bool)> = HashMap::new();
    let mut report_params = Vec::with_capacity(total);
    for (processed, archive) in archives.iter().enumerate() {
        let name = archive["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let hostname = archive["hostname"]
            .as_str()
            .filter(|h| !h.is_empty())
            .or_else(|| hostname_fallback.get(name).map(String::as_str))
            .unwrap_or("unknown");

        let (agent_id, matched) = if let Some(&cached) = hostname_cache.get(hostname) {
            cached
        } else {
            let (agent, matched) = match db::resolve_agent_for_hostname(pool, hostname).await? {
                db::ResolveResult::ExactMatch(c) => (c, true),
                db::ResolveResult::PatternMatch(c) => (c, true),
                db::ResolveResult::Unmatched => {
                    let c = db::get_or_create_agent_by_hostname(pool, hostname).await?;
                    (c, false)
                }
            };
            let entry = (agent.id, matched);
            hostname_cache.insert(hostname.to_string(), entry);
            entry
        };

        let start = parse_borg_timestamp(archive["start"].as_str().unwrap_or_default());
        let end = parse_borg_timestamp(archive["end"].as_str().unwrap_or_default());
        let duration_secs = end
            .zip(start)
            .map_or(0, |(e, s)| e.signed_duration_since(s).num_seconds().max(0));

        let Some(started_at) = start else {
            warn!(repo_id, archive = %name, "skipping archive with unparseable start timestamp");
            continue;
        };
        let finished_at = end.unwrap_or(started_at);

        let processed_count = i32::try_from(processed + 1).unwrap_or(i32::MAX);
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

async fn sync_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    mode: SyncMode<'_>,
) -> Result<(u64, u64), ApiError> {
    let (borg_repo, env) = super::archives::get_repo_env(pool, encryption_key, repo_id).await?;

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        0,
        Some("Listing archives\u{2026}"),
    )
    .await;

    let archives = run_borg_list_with_retry(&borg_repo, &env, pool, ui_broadcast, repo_id).await?;

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
            .filter(|a| {
                a["name"]
                    .as_str()
                    .is_some_and(|n| !n.is_empty() && !known_names.contains(n))
            })
            .collect(),
    };

    let repo_archive_count = i64::try_from(borg_names.len()).unwrap_or(i64::MAX);

    if to_import.is_empty() {
        refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, repo_archive_count).await;
        return Ok((0, removed));
    }

    let total = to_import.len();
    let importing_msg = match mode {
        SyncMode::Existing => format!("Importing {total} archives\u{2026}"),
        SyncMode::New { .. } => format!("Importing {total} new archives\u{2026}"),
    };
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        i32::try_from(total).unwrap_or(i32::MAX),
        Some(&importing_msg),
    )
    .await;

    let report_params =
        build_import_reports(pool, ui_broadcast, repo_id, &borg_repo, &env, &to_import).await?;

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
        borg_repo.clone(),
        env.clone(),
        repo_id,
        archive_names.clone(),
    );

    if let SyncMode::New { repo_lock } = mode {
        queue_archive_indexing(
            pool,
            encryption_key,
            repo_id,
            &archive_names,
            repo_lock,
            "incremental sync",
        )
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
    refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, repo_archive_count).await;

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
    Ok((processed, removed))
}

pub async fn sync_existing_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        SyncMode::Existing,
    )
    .await
}

pub async fn sync_new_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    repo_lock: &RepoLock,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        SyncMode::New { repo_lock },
    )
    .await
}

fn enrich_archive_stats_background(
    pool: PgPool,
    borg_repo: String,
    env: std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_names: Vec<String>,
) {
    tokio::spawn(async move {
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
                let semaphore = semaphore.clone();
                async move {
                    let _permit = semaphore.acquire_owned().await;
                    let repo_archive = format!("{borg_repo}::{archive_name}");
                    let output = match Borg::new()
                        .run(
                            &[
                                "info",
                                "--json",
                                "--lock-wait",
                                LOCK_WAIT_SECS,
                                &repo_archive,
                            ],
                            &env,
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
                            return;
                        }
                    };

                    if !output.status.success() {
                        warn!(
                            repo_id,
                            archive = %archive_name,
                            stderr = %String::from_utf8_lossy(&output.stderr),
                            "stat enrichment: borg info non-zero exit"
                        );
                        return;
                    }

                    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(
                                repo_id,
                                archive = %archive_name,
                                error = %e,
                                "stat enrichment: failed to parse borg info output"
                            );
                            return;
                        }
                    };

                    let info = match json["archives"].as_array().and_then(|a| a.first()) {
                        Some(v) => v.clone(),
                        None => {
                            warn!(
                                repo_id,
                                archive = %archive_name,
                                "stat enrichment: borg info returned no archive entry"
                            );
                            return;
                        }
                    };

                    let raw_stats = &info["stats"];
                    #[allow(clippy::cast_possible_truncation)]
                    let archive_stats = db::ArchiveStats {
                        original_size: raw_stats["original_size"].as_i64().unwrap_or(0),
                        compressed_size: raw_stats["compressed_size"].as_i64().unwrap_or(0),
                        deduplicated_size: raw_stats["deduplicated_size"].as_i64().unwrap_or(0),
                        files_processed: raw_stats["nfiles"].as_i64().unwrap_or(0),
                        duration_secs: info["duration"].as_f64().unwrap_or(0.0) as i64,
                    };

                    if let Err(e) = db::update_backup_report_stats(
                        &pool,
                        repo_id,
                        &archive_name,
                        &archive_stats,
                    )
                    .await
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
            })
            .collect();
        join_all(futures).await;
        info!(repo_id, "stat enrichment: completed");
    });
}

async fn queue_archive_indexing(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_names: &[String],
    repo_lock: &RepoLock,
    sync_kind: &str,
) {
    join_all(archive_names.iter().map(|archive_name| {
        let pool = pool.clone();
        let repo_lock = repo_lock.clone();
        async move {
            if let Err(e) = archive_index::ensure_indexed(
                pool,
                *encryption_key,
                repo_id,
                archive_name.clone(),
                repo_lock,
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
        // The progress bar tracks *completed* archives, so it never reaches 100%
        // while an archive (including the last one) is still being scanned.
        let completed = i32::try_from(index).unwrap_or(i32::MAX);
        let human_position = index + 1;
        let archive_msg = format!(
            "Indexing contents of \u{2018}{archive_name}\u{2019} ({human_position}/{total})"
        );
        publish_import_progress(
            &pool,
            &ui_broadcast,
            repo_id,
            completed,
            total_i32,
            Some(&archive_msg),
        )
        .await;

        if let Err(e) = archive_index::ensure_index_job(&pool, repo_id, archive_name).await {
            warn!(repo_id, archive = %archive_name, error = %e, "content index job failed");
            continue;
        }

        // The live file count and current path keep the badge visibly moving even
        // while a single large archive is being scanned.
        let mut on_progress = |file_count: u64, current: Option<&str>| {
            let message = current.map_or_else(
                || {
                    format!(
                        "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \
                         \u{2014} {file_count} files"
                    )
                },
                |path| {
                    format!(
                        "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \
                         \u{2014} {file_count} files \u{00b7} {path}"
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

        if let Err(e) = archive_index::run_indexing(
            &pool,
            &encryption_key,
            repo_id,
            archive_name,
            &repo_lock,
            &mut on_progress,
        )
        .await
        {
            warn!(repo_id, archive = %archive_name, error = %e, "content indexing: archive failed");
        }
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RescanResponse {
    pub matched: u64,
    pub remaining_unmatched: u64,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/rescan",
    tag = "Repositories",
    operation_id = "rescanRepo",
    summary = "Re-scan unmatched archives against hostname patterns",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Rescan results", body = RescanResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn rescan_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<RescanResponse>, ApiError> {
    db::get_repo_with_stats(&state.pool, repo_id).await?;

    #[derive(sqlx::FromRow)]
    struct UnmatchedRow {
        report_id: i64,
        hostname: String,
    }

    let unmatched_rows = sqlx::query_as::<_, UnmatchedRow>(
        "SELECT br.id AS report_id, c.hostname FROM backup_reports br JOIN agents c ON c.id = \
         br.agent_id WHERE br.repo_id = $1 AND br.matched = false",
    )
    .bind(repo_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let mut matched_count = 0u64;

    for row in &unmatched_rows {
        let result = db::resolve_agent_for_hostname(&state.pool, &row.hostname).await?;
        let new_agent_id = match result {
            db::ResolveResult::ExactMatch(c) => Some(c.id),
            db::ResolveResult::PatternMatch(c) => Some(c.id),
            db::ResolveResult::Unmatched => None,
        };

        if let Some(agent_id) = new_agent_id {
            sqlx::query("UPDATE backup_reports SET agent_id = $1, matched = true WHERE id = $2")
                .bind(agent_id)
                .bind(row.report_id)
                .execute(&state.pool)
                .await
                .map_err(ApiError::Database)?;
            matched_count += 1;
        }
    }

    sqlx::query(
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SyncResponse {
    pub imported: u64,
    pub removed: u64,
    pub duration_secs: u64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SyncQuery {
    #[serde(default)]
    pub build_index: bool,
}

const SYNC_WARN_DURATION: Duration = Duration::from_secs(300);

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/sync",
    tag = "Repositories",
    operation_id = "syncRepo",
    summary = "Full repository sync - re-reads all archives from borg",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("build_index" = bool, Query, description = "Also build archive indexes while syncing"),
    ),
    responses(
        (status = 200, description = "Sync results", body = SyncResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Sync already in progress"),
    )
)]
pub async fn sync_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<SyncQuery>,
    Path(repo_id): Path<i64>,
) -> Result<Json<SyncResponse>, ApiError> {
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;
    if repo.importing {
        return Err(ApiError::Conflict("sync already in progress".to_string()));
    }

    db::set_repo_importing(&state.pool, repo_id, true).await?;

    let start = std::time::Instant::now();
    let result = sync_existing_archives(
        &state.pool,
        &state.encryption_key,
        repo_id,
        &state.ui_broadcast,
    )
    .await;
    let elapsed = start.elapsed();

    db::update_repo_last_synced(&state.pool, repo_id).await?;

    let (imported, removed) = match result {
        Ok(counts) => counts,
        Err(e) => {
            db::set_repo_importing(&state.pool, repo_id, false).await?;
            clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id).await;
            let msg = format!(
                "repo sync failed for '{}' after {:.1}s: {e}",
                repo.name,
                elapsed.as_secs_f64()
            );
            error!("{msg}");
            if let Err(log_err) =
                db::insert_system_event(&state.pool, "repo_sync_failed", None, &msg).await
            {
                error!(error = %log_err, "failed to log sync event");
            }
            return Err(e);
        }
    };

    let duration_secs = elapsed.as_secs();
    let msg = format!(
        "repo sync completed for '{}': imported {imported}, removed {removed} archives in \
         {duration_secs}s",
        repo.name
    );

    if elapsed > SYNC_WARN_DURATION {
        error!(
            repo_id,
            duration_secs,
            "repo sync exceeded {}s threshold",
            SYNC_WARN_DURATION.as_secs()
        );
        let warn_msg = format!(
            "repo sync for '{}' took {duration_secs}s (exceeds {}s threshold)",
            repo.name,
            SYNC_WARN_DURATION.as_secs()
        );
        if let Err(log_err) =
            db::insert_system_event(&state.pool, "repo_sync_slow", None, &warn_msg).await
        {
            error!(error = %log_err, "failed to log slow sync event");
        }
    }

    info!("{msg}");
    if let Err(log_err) = db::insert_system_event(&state.pool, "repo_sync", None, &msg).await {
        error!(error = %log_err, "failed to log sync event");
    }

    if query.build_index {
        // Index the archive contents in the background, keeping the repository
        // flagged as importing so the UI shows live per-archive progress. The
        // importing flag and progress state are cleared once indexing finishes.
        let pool = state.pool.clone();
        let key = state.encryption_key;
        let ui_broadcast = state.ui_broadcast.clone();
        let repo_lock = state.repo_lock.clone();
        tokio::spawn(async move {
            index_archives_with_progress(
                pool.clone(),
                key,
                repo_id,
                ui_broadcast.clone(),
                repo_lock,
            )
            .await;
            if let Err(e) = db::set_repo_importing(&pool, repo_id, false).await {
                error!(repo_id, error = %e, "failed to clear importing flag after indexing");
            }
            clear_import_progress_state(&pool, &ui_broadcast, repo_id).await;
            ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
        });
    } else {
        db::set_repo_importing(&state.pool, repo_id, false).await?;
        clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id).await;
        state
            .ui_broadcast
            .send(shared::protocol::ServerToUi::DataChanged);
    }

    Ok(Json(SyncResponse {
        imported,
        removed,
        duration_secs,
    }))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/reset-import",
    tag = "Repositories",
    operation_id = "resetImport",
    summary = "Reset a stuck importing state (admin only)",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 204, description = "Import state reset"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn reset_import(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::get_repo_with_stats(&state.pool, repo_id).await?;
    db::set_repo_importing(&state.pool, repo_id, false).await?;
    db::set_repo_import_error(&state.pool, repo_id, None).await?;
    clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    Ok(StatusCode::NO_CONTENT)
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
    fn parse_lock_holder_extracts_hostname_and_pid() {
        let stderr = concat!(
            "borgbackup.locking.LockTimeout: Failed to create/acquire the lock\n",
            "Holder: {'exclusive': True, 'hostname': 'web-01', 'pid': 4567, 'time': 0.0}",
        );
        let result = parse_lock_holder(stderr);
        assert_eq!(result.as_deref(), Some("web-01 (PID 4567)"));
    }

    #[test]
    fn parse_lock_holder_hostname_only_when_no_pid() {
        let stderr = "Holder: {'hostname': 'db-server', 'description': 'borg list'}";
        let result = parse_lock_holder(stderr);
        assert_eq!(result.as_deref(), Some("db-server"));
    }

    #[test]
    fn parse_lock_holder_returns_none_without_holder_line() {
        let stderr = "Failed to create/acquire the lock /repo/lock.exclusive (timeout).";
        assert!(parse_lock_holder(stderr).is_none());
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
}
