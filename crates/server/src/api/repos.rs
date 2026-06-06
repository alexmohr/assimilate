// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, process::Output, time::Duration};

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
    AppState, archive_index,
    borg::Borg,
    config_assembler,
    db::{self, InsertRepoParams, RepoRow, RepoWithStatsRow, UpdateRepoParams},
    error::{ApiError, ApiJson},
    ws::ui_broadcast::UiBroadcast,
};

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
    path = "/api/clients/{hostname}/repos",
    tag = "Repositories",
    operation_id = "getClientRepos",
    summary = "List repositories for a specific host",
    params(
        ("hostname" = String, Path, description = "Client hostname"),
    ),
    responses(
        (status = 200, description = "List of repositories", body = Vec<RepoRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_client_repos(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<RepoRow>>, ApiError> {
    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;
    let repos = db::list_repos_for_client_public(&state.pool, client.id).await?;
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
    _auth: AuthUser,
    ApiJson(req): ApiJson<CreateRepoRequest>,
) -> Result<(StatusCode, Json<RepoRow>), ApiError> {
    helpers::validate_non_empty(&req.name, "name")?;
    helpers::validate_non_empty(&req.repo_path, "repo_path")?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;

    let ssh_port = req.ssh_port.unwrap_or(22);
    helpers::validate_path_exists(
        &req.ssh_host,
        &req.ssh_user,
        u16::try_from(ssh_port).unwrap_or(22),
        &req.repo_path,
    )
    .await?;

    let repo_url = build_repo_url(
        &req.ssh_user,
        &req.ssh_host,
        u16::try_from(ssh_port).unwrap_or(22),
        &req.repo_path,
    );

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

    let repo_id = repo.id;
    let pool = state.pool.clone();
    let encryption_key = state.encryption_key;
    let ui_broadcast = state.ui_broadcast.clone();
    let need_borg_info = info_result.is_none();
    let bg_repo_url = repo_url.clone();
    let bg_passphrase = req.passphrase.clone();

    db::set_repo_importing(&state.pool, repo_id, true).await?;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);

    tokio::spawn(async move {
        if need_borg_info {
            match run_borg_info_with_retry(&bg_repo_url, &bg_passphrase).await {
                Ok(info) => {
                    if let Err(e) =
                        db::update_repo_encryption(&pool, repo_id, &info.encryption.to_string())
                            .await
                    {
                        warn!(repo_id, error = %e, "failed to update encryption after deferred borg info");
                    }
                }
                Err(e) => {
                    warn!(repo_id, error = %e, "deferred borg info failed, continuing to archive sync");
                }
            }
        }

        if let Err(e) =
            sync_existing_archives(&pool, &encryption_key, repo_id, &ui_broadcast, false).await
        {
            warn!(repo_id, error = %e, "failed to sync existing archives on import");
            let _ = db::set_repo_import_error(&pool, repo_id, Some(&format!("{e}"))).await;
        }
        if let Err(e) = db::set_repo_importing(&pool, repo_id, false).await {
            warn!(repo_id, error = %e, "failed to clear importing flag");
        }
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

    let repo = db::update_repo(
        &state.pool,
        &UpdateRepoParams {
            repo_id,
            name: &name,
            repo_path: &req.repo_path,
            ssh_user: &req.ssh_user,
            ssh_host: &req.ssh_host,
            ssh_port: req.ssh_port.unwrap_or(22),
            compression: &compression,
            encryption: &encryption,
            enabled: req.enabled.unwrap_or(true),
            sync_schedule: sync_schedule.as_deref(),
        },
    )
    .await?;

    if location_changed {
        db::set_relocation_pending(&state.pool, repo_id).await?;
    }

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

    run_ssh_command(&conn.ssh_user, &conn.ssh_host, conn.ssh_port, &command)
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
        (status = 200, description = "Repository with stats", body = RepoWithStatsRow),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_repo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(repo_id): Path<i64>,
) -> Result<Json<RepoWithStatsRow>, ApiError> {
    let repo = db::get_repo_with_stats(&state.pool, repo_id).await?;
    Ok(Json(repo))
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
    _auth: AuthUser,
    ApiJson(req): ApiJson<InitRepoRequest>,
) -> Result<(StatusCode, Json<InitRepoResponse>), ApiError> {
    helpers::validate_non_empty(&req.name, "name")?;
    helpers::validate_non_empty(&req.repo_path, "repo_path")?;
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;

    let ssh_port = req.ssh_port.unwrap_or(22);
    let repo_url = build_repo_url(
        &req.ssh_user,
        &req.ssh_host,
        u16::try_from(ssh_port).unwrap_or(22),
        &req.repo_path,
    );

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

    let borg_output = run_borg_break_lock(&repo_url, &passphrase).await?;

    info!(repo_id, name = %repo.name, "repository lock broken");

    Ok(Json(BreakLockResponse {
        message: format!("lock broken on repository '{}'", repo.name),
        borg_output,
    }))
}

const ALLOWED_BORG_SUBCOMMANDS: &[&str] = &[
    "info", "list", "check", "compact", "prune", "delete", "diff", "rename", "recreate",
];

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
    let subcommand = req
        .args
        .first()
        .ok_or_else(|| ApiError::BadRequest("args must not be empty".to_owned()))?;

    if !ALLOWED_BORG_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "subcommand '{}' is not allowed; permitted: {}",
            subcommand,
            ALLOWED_BORG_SUBCOMMANDS.join(", ")
        )));
    }

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

    let mut env = HashMap::from([
        ("BORG_PASSPHRASE".to_owned(), passphrase),
        ("BORG_REPO".to_owned(), repo_url),
        (
            "BORG_RSH".to_owned(),
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_owned(),
        ),
    ]);
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        env.insert("SSH_AUTH_SOCK".to_owned(), sock);
    }

    info!(repo_id, name = %repo.name, subcommand, "admin executing borg command");

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
        &format!("mv {} {migrated_path}", repo.repo_path),
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
            &format!("mv {migrated_path} {}", repo.repo_path),
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
) -> Result<(), String> {
    let port = u16::try_from(ssh_port).map_err(|e| format!("invalid SSH port: {e}"))?;

    let session = crate::ssh::connect_with_key(ssh_host, ssh_user, port)
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

async fn run_borg_info_with_retry(
    repo_url: &str,
    passphrase: &str,
) -> Result<BorgInfoResult, ApiError> {
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        match run_borg_info_once(repo_url, passphrase).await {
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
    unreachable!()
}

async fn run_borg_info_once(repo_url: &str, passphrase: &str) -> Result<BorgInfoResult, ApiError> {
    let mut env = HashMap::from([
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        (
            "BORG_RSH".to_owned(),
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_owned(),
        ),
    ]);
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        env.insert("SSH_AUTH_SOCK".to_owned(), sock);
    }

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
    let mut env = HashMap::from([
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        (
            "BORG_RSH".to_owned(),
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_owned(),
        ),
    ]);
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        env.insert("SSH_AUTH_SOCK".to_owned(), sock);
    }

    let output = Borg::new()
        .run(&["break-lock", repo_url], &env)
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
    let mut env = HashMap::from([
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        (
            "BORG_RSH".to_owned(),
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_owned(),
        ),
    ]);
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        env.insert("SSH_AUTH_SOCK".to_owned(), sock);
    }

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

fn human_bytes(bytes: i64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}

async fn run_borg_list_with_retry(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<Output, ApiError> {
    let borg = Borg::new();
    let args = [
        "list",
        "--json",
        "--format",
        "{hostname}{end}",
        "--lock-wait",
        LOCK_WAIT_SECS,
        borg_repo,
    ];
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let output = borg
            .run(&args, env)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to execute borg list: {e}")))?;

        if output.status.success() {
            return Ok(output);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !is_lock_error(&stderr) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            return Err(ApiError::Internal(format!("borg list failed: {stderr}")));
        }
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            "borg list lock contention, retrying in {}s",
            LOCK_RETRY_INTERVAL.as_secs()
        );
        tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
    }
    unreachable!()
}

fn is_ssh_connection_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("connection refused")
        || lower.contains("connection timed out")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("ssh: connect to host")
}

/// Runs `borg info --glob '*' --json` once to fetch stats for all archives in
/// the repository. Returns a map of archive name -> JSON value (the archive
/// object from the `archives` array).
///
/// Uses a single SSH connection for all archives, which is dramatically faster
/// than one `borg info <repo>::<name>` call per archive.
async fn run_borg_info_all_archives(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<HashMap<String, serde_json::Value>, ApiError> {
    let borg = Borg::new();
    let args = [
        "info",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--glob-archives",
        "*",
        borg_repo,
    ];
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let output = borg
            .run(&args, env)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to execute borg info: {e}")))?;

        if output.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
                ApiError::Internal(format!("failed to parse borg info output: {e}"))
            })?;
            let map = json["archives"]
                .as_array()
                .map_or_else(Vec::new, Clone::clone)
                .into_iter()
                .filter_map(|a| a["name"].as_str().map(|n| (n.to_string(), a.clone())))
                .collect();
            return Ok(map);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if (!is_lock_error(&stderr) && !is_ssh_connection_error(&stderr))
            || attempt == LOCK_RETRY_MAX_ATTEMPTS
        {
            warn!(
                borg_repo,
                stderr = %stderr,
                "borg info --glob-archives failed, archive stats will be unavailable"
            );
            return Ok(HashMap::new());
        }
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            "borg info --glob-archives retryable error, retrying in {}s",
            LOCK_RETRY_INTERVAL.as_secs()
        );
        tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
    }
    unreachable!()
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
    let output = match Borg::new().run(&args, env).await {
        Ok(output) => output,
        Err(e) => {
            warn!(repo_id, error = %e, "failed to run borg info for repo stats");
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

pub async fn sync_existing_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    build_index: bool,
) -> Result<(u64, u64), ApiError> {
    let (borg_repo, env) = super::archives::get_repo_env(pool, encryption_key, repo_id).await?;

    let listing_msg = "Listing archives\u{2026}".to_string();
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: 0,
        message: Some(listing_msg.clone()),
    });
    let _ = db::update_repo_import_progress(pool, repo_id, 0, 0).await;
    let _ = db::set_import_status_message(pool, repo_id, Some(&listing_msg)).await;

    let output = run_borg_list_with_retry(&borg_repo, &env).await?;

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg list output: {e}")))?;

    let archives = json_output["archives"]
        .as_array()
        .map_or_else(Vec::new, Clone::clone);

    let borg_names: std::collections::HashSet<String> = archives
        .iter()
        .filter_map(|a| a["name"].as_str())
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect();

    let known_names = db::list_archive_names_for_repo(pool, repo_id).await?;
    let stale: Vec<String> = known_names.difference(&borg_names).cloned().collect();
    let removed = db::delete_archive_records_by_names(pool, repo_id, &stale).await?;
    if removed > 0 {
        info!(repo_id, removed, "removed stale archives during full sync");
    }

    if archives.is_empty() {
        refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, 0).await;
        return Ok((0, removed));
    }

    let total = archives.len();
    let fetching_msg = format!("Fetching stats for {total} archives\u{2026}");
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: total as i32,
        message: Some(fetching_msg.clone()),
    });
    let _ = db::update_repo_import_progress(pool, repo_id, 0, total as i64).await;
    let _ = db::set_import_status_message(pool, repo_id, Some(&fetching_msg)).await;

    let archive_stats = run_borg_info_all_archives(&borg_repo, &env).await?;

    let importing_msg = format!("Importing {total} archives\u{2026}");
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: total as i32,
        message: Some(importing_msg.clone()),
    });
    let _ = db::set_import_status_message(pool, repo_id, Some(&importing_msg)).await;

    let mut hostname_cache: HashMap<String, (i64, bool)> = HashMap::new();
    let mut report_params = Vec::with_capacity(archives.len());
    for (processed, archive) in archives.iter().enumerate() {
        let name = archive["name"].as_str().unwrap_or_default();
        let hostname = archive["hostname"].as_str().unwrap_or("unknown");
        if name.is_empty() {
            continue;
        }

        let (client_id, matched) = if let Some(&cached) = hostname_cache.get(hostname) {
            cached
        } else {
            let (client, matched) = match db::resolve_client_for_hostname(pool, hostname).await? {
                db::ResolveResult::ExactMatch(c) => (c, true),
                db::ResolveResult::PatternMatch(c) => (c, true),
                db::ResolveResult::Unmatched => {
                    let c = db::get_or_create_client_by_hostname(pool, hostname).await?;
                    (c, false)
                }
            };
            let entry = (client.id, matched);
            hostname_cache.insert(hostname.to_string(), entry);
            entry
        };

        let (
            started_at,
            finished_at,
            original_size,
            compressed_size,
            deduplicated_size,
            files_processed,
            duration_secs,
        ) = if let Some(info) = archive_stats.get(name) {
            let stats = &info["stats"];
            #[allow(clippy::cast_possible_truncation)]
            let dur = info["duration"].as_f64().unwrap_or(0.0) as i64;
            let start = parse_borg_timestamp(info["start"].as_str().unwrap_or_default());
            let end = parse_borg_timestamp(info["end"].as_str().unwrap_or_default());
            (
                start,
                end,
                stats["original_size"].as_i64().unwrap_or(0),
                stats["compressed_size"].as_i64().unwrap_or(0),
                stats["deduplicated_size"].as_i64().unwrap_or(0),
                stats["nfiles"].as_i64().unwrap_or(0),
                dur,
            )
        } else {
            let start = parse_borg_timestamp(archive["start"].as_str().unwrap_or_default());
            let end = parse_borg_timestamp(archive["end"].as_str().unwrap_or_default());
            let dur = end
                .zip(start)
                .map_or(0, |(e, s)| e.signed_duration_since(s).num_seconds().max(0));
            (start, end, 0, 0, 0, 0, dur)
        };

        let Some(started_at) = started_at else {
            warn!(repo_id, archive = %name, "skipping archive with unparseable start timestamp");
            continue;
        };
        let finished_at = finished_at.unwrap_or(started_at);

        let processed_count = (processed + 1) as i32;
        info!(repo_id, archive = %name, processed = processed_count, total, original_size, "archive imported");
        let progress_msg = format!(
            "Imported \u{2018}{name}\u{2019} ({processed_count}/{total}) \u{00b7} {}",
            human_bytes(original_size)
        );
        ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
            repo_id,
            progress: processed_count,
            total: total as i32,
            message: Some(progress_msg.clone()),
        });
        let _ =
            db::update_repo_import_progress(pool, repo_id, processed_count as i64, total as i64)
                .await;
        let _ = db::set_import_status_message(pool, repo_id, Some(&progress_msg)).await;

        report_params.push(db::InsertReportParams {
            client_id,
            repo_id,
            schedule_id: None,
            started_at,
            finished_at,
            status: "success".to_string(),
            original_size,
            compressed_size,
            deduplicated_size,
            repo_unique_csize: 0,
            files_processed,
            duration_secs,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched,
            archive_name: Some(name.to_string()),
            borg_command: None,
        });
    }

    let imported = report_params.len() as u64;
    db::bulk_insert_backup_reports(pool, &report_params).await?;
    if build_index {
        let archive_names: Vec<String> = report_params
            .iter()
            .filter_map(|params| params.archive_name.clone())
            .collect();
        queue_archive_indexing(pool, encryption_key, repo_id, &archive_names, "full sync").await;
    }
    refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, borg_names.len() as i64).await;
    info!(repo_id, imported, total, "synced existing archives");
    Ok((imported, removed))
}

pub async fn sync_new_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
) -> Result<(u64, u64), ApiError> {
    let (borg_repo, env) = super::archives::get_repo_env(pool, encryption_key, repo_id).await?;

    let listing_msg = "Listing archives\u{2026}".to_string();
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: 0,
        message: Some(listing_msg.clone()),
    });
    let _ = db::update_repo_import_progress(pool, repo_id, 0, 0).await;
    let _ = db::set_import_status_message(pool, repo_id, Some(&listing_msg)).await;

    let output = run_borg_list_with_retry(&borg_repo, &env).await?;

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg list output: {e}")))?;

    let archives = json_output["archives"]
        .as_array()
        .map_or_else(Vec::new, Clone::clone);

    let borg_names: std::collections::HashSet<String> = archives
        .iter()
        .filter_map(|a| a["name"].as_str())
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect();

    let known_names = db::list_archive_names_for_repo(pool, repo_id).await?;

    let new_archives: Vec<&serde_json::Value> = archives
        .iter()
        .filter(|a| {
            a["name"]
                .as_str()
                .is_some_and(|n| !n.is_empty() && !known_names.contains(n))
        })
        .collect();

    if new_archives.is_empty() {
        refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, borg_names.len() as i64).await;
        return Ok((0, 0));
    }

    let total = new_archives.len();
    let fetching_msg = format!("Fetching stats for {total} new archives\u{2026}");
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: total as i32,
        message: Some(fetching_msg.clone()),
    });
    let _ = db::update_repo_import_progress(pool, repo_id, 0, total as i64).await;
    let _ = db::set_import_status_message(pool, repo_id, Some(&fetching_msg)).await;

    let archive_stats = run_borg_info_all_archives(&borg_repo, &env).await?;

    let importing_msg = format!("Importing {total} archives\u{2026}");
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress: 0,
        total: total as i32,
        message: Some(importing_msg.clone()),
    });
    let _ = db::set_import_status_message(pool, repo_id, Some(&importing_msg)).await;

    let mut hostname_cache: HashMap<String, (i64, bool)> = HashMap::new();
    let mut report_params = Vec::with_capacity(new_archives.len());
    for (processed, archive) in new_archives.iter().enumerate() {
        let name = archive["name"].as_str().unwrap_or_default();
        let hostname = archive["hostname"].as_str().unwrap_or("unknown");
        if name.is_empty() {
            continue;
        }

        let (client_id, matched) = if let Some(&cached) = hostname_cache.get(hostname) {
            cached
        } else {
            let (client, matched) = match db::resolve_client_for_hostname(pool, hostname).await? {
                db::ResolveResult::ExactMatch(c) => (c, true),
                db::ResolveResult::PatternMatch(c) => (c, true),
                db::ResolveResult::Unmatched => {
                    let c = db::get_or_create_client_by_hostname(pool, hostname).await?;
                    (c, false)
                }
            };
            let entry = (client.id, matched);
            hostname_cache.insert(hostname.to_string(), entry);
            entry
        };

        let (
            started_at,
            finished_at,
            original_size,
            compressed_size,
            deduplicated_size,
            files_processed,
            duration_secs,
        ) = if let Some(info) = archive_stats.get(name) {
            let stats = &info["stats"];
            #[allow(clippy::cast_possible_truncation)]
            let dur = info["duration"].as_f64().unwrap_or(0.0) as i64;
            let start = parse_borg_timestamp(info["start"].as_str().unwrap_or_default());
            let end = parse_borg_timestamp(info["end"].as_str().unwrap_or_default());
            (
                start,
                end,
                stats["original_size"].as_i64().unwrap_or(0),
                stats["compressed_size"].as_i64().unwrap_or(0),
                stats["deduplicated_size"].as_i64().unwrap_or(0),
                stats["nfiles"].as_i64().unwrap_or(0),
                dur,
            )
        } else {
            let start = parse_borg_timestamp(archive["start"].as_str().unwrap_or_default());
            let end = parse_borg_timestamp(archive["end"].as_str().unwrap_or_default());
            let dur = end
                .zip(start)
                .map_or(0, |(e, s)| e.signed_duration_since(s).num_seconds().max(0));
            (start, end, 0, 0, 0, 0, dur)
        };

        let Some(started_at) = started_at else {
            warn!(repo_id, archive = %name, "skipping archive with unparseable start timestamp");
            continue;
        };
        let finished_at = finished_at.unwrap_or(started_at);

        let processed_count = (processed + 1) as i32;
        info!(repo_id, archive = %name, processed = processed_count, total, original_size, "archive imported");
        let progress_msg = format!(
            "Imported \u{2018}{name}\u{2019} ({processed_count}/{total}) \u{00b7} {}",
            human_bytes(original_size)
        );
        ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
            repo_id,
            progress: processed_count,
            total: total as i32,
            message: Some(progress_msg.clone()),
        });
        let _ =
            db::update_repo_import_progress(pool, repo_id, processed_count as i64, total as i64)
                .await;
        let _ = db::set_import_status_message(pool, repo_id, Some(&progress_msg)).await;

        report_params.push(db::InsertReportParams {
            client_id,
            repo_id,
            schedule_id: None,
            started_at,
            finished_at,
            status: "success".to_string(),
            original_size,
            compressed_size,
            deduplicated_size,
            repo_unique_csize: 0,
            files_processed,
            duration_secs,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched,
            archive_name: Some(name.to_string()),
            borg_command: None,
        });
    }

    let added = report_params.len() as u64;
    let archive_names: Vec<String> = report_params
        .iter()
        .filter_map(|params| params.archive_name.clone())
        .collect();
    db::bulk_insert_backup_reports(pool, &report_params).await?;
    queue_archive_indexing(
        pool,
        encryption_key,
        repo_id,
        &archive_names,
        "incremental sync",
    )
    .await;
    refresh_repo_info_stats(pool, &borg_repo, &env, repo_id, borg_names.len() as i64).await;
    info!(repo_id, added, total, "incremental sync complete");
    Ok((added, 0))
}

async fn queue_archive_indexing(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    archive_names: &[String],
    sync_kind: &str,
) {
    join_all(archive_names.iter().map(|archive_name| {
        let pool = pool.clone();
        async move {
            if let Err(e) =
                archive_index::ensure_indexed(pool, *encryption_key, repo_id, archive_name.clone())
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
        "SELECT br.id AS report_id, c.hostname FROM backup_reports br JOIN clients c ON c.id = \
         br.client_id WHERE br.repo_id = $1 AND br.matched = false",
    )
    .bind(repo_id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let mut matched_count = 0u64;

    for row in &unmatched_rows {
        let result = db::resolve_client_for_hostname(&state.pool, &row.hostname).await?;
        let new_client_id = match result {
            db::ResolveResult::ExactMatch(c) => Some(c.id),
            db::ResolveResult::PatternMatch(c) => Some(c.id),
            db::ResolveResult::Unmatched => None,
        };

        if let Some(client_id) = new_client_id {
            sqlx::query("UPDATE backup_reports SET client_id = $1, matched = true WHERE id = $2")
                .bind(client_id)
                .bind(row.report_id)
                .execute(&state.pool)
                .await
                .map_err(ApiError::Database)?;
            matched_count += 1;
        }
    }

    sqlx::query(
        "DELETE FROM clients WHERE agent_token_hash = 'imported:no-auth' AND NOT EXISTS (SELECT 1 \
         FROM backup_reports WHERE client_id = clients.id)",
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
        query.build_index,
    )
    .await;
    let elapsed = start.elapsed();

    db::set_repo_importing(&state.pool, repo_id, false).await?;
    db::update_repo_last_synced(&state.pool, repo_id).await?;

    let (imported, removed) = match result {
        Ok(counts) => counts,
        Err(e) => {
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

    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);

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
    fn human_bytes_formats_bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1023), "1023 B");
    }

    #[test]
    fn human_bytes_formats_kilobytes() {
        assert_eq!(human_bytes(1_024), "1.0 KB");
        assert_eq!(human_bytes(2_048), "2.0 KB");
        assert_eq!(human_bytes(1_048_575), "1024.0 KB");
    }

    #[test]
    fn human_bytes_formats_megabytes() {
        assert_eq!(human_bytes(1_048_576), "1.0 MB");
        assert_eq!(human_bytes(5_242_880), "5.0 MB");
        assert_eq!(human_bytes(1_073_741_823), "1024.0 MB");
    }

    #[test]
    fn human_bytes_formats_gigabytes() {
        assert_eq!(human_bytes(1_073_741_824), "1.0 GB");
        assert_eq!(human_bytes(10_737_418_240), "10.0 GB");
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
}
