// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{process::Stdio, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::DateTime;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use shared::{crypto::encrypt_passphrase, types::BorgEncryption};
use sqlx::PgPool;
use tokio::process::Command;
use tracing::{error, info, warn};

use super::{
    archives::{LOCK_WAIT_SECS, borg_binary},
    auth::{AuthUser, RequireAdmin, Role},
    helpers,
    permissions::is_visible_to_user,
};
use crate::{
    AppState,
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

    let repo_url = format!(
        "ssh://{user}@{host}:{port}/{path}",
        user = req.ssh_user,
        host = req.ssh_host,
        port = ssh_port,
        path = req.repo_path,
    );

    let info_timeout = Duration::from_secs(30);
    let info_result = tokio::time::timeout(info_timeout, run_borg_info(&repo_url, &req.passphrase))
        .await
        .ok()
        .transpose()?;

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
                    let msg = format!("{e}");
                    warn!(repo_id, error = %msg, "deferred borg info failed");
                    let _ = db::set_repo_import_error(&pool, repo_id, Some(&msg)).await;
                    let _ = db::set_repo_importing(&pool, repo_id, false).await;
                    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
                    return;
                }
            }
        }

        if let Err(e) = sync_existing_archives(&pool, &encryption_key, repo_id, &ui_broadcast).await
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

    let compression = helpers::validate_compression(req.compression.as_deref())?;

    let existing = db::get_repo_with_passphrase(&state.pool, repo_id).await?;
    let location_changed = existing.repo_path != req.repo_path
        || existing.ssh_host != req.ssh_host
        || existing.ssh_port != req.ssh_port.unwrap_or(22);

    let encryption = req
        .encryption
        .map_or_else(|| "repokey-blake2".to_string(), |e| e.to_string());

    let sync_schedule = req.sync_schedule.unwrap_or(existing.sync_schedule);

    let repo = db::update_repo(
        &state.pool,
        &UpdateRepoParams {
            repo_id,
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
    let repo_url = format!(
        "ssh://{user}@{host}:{port}/{path}",
        user = req.ssh_user,
        host = req.ssh_host,
        port = ssh_port,
        path = req.repo_path,
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

    let repo_url = format!(
        "ssh://{}@{}:{}/{}",
        repo.ssh_user, repo.ssh_host, repo.ssh_port, repo.repo_path
    );

    let borg_output = run_borg_break_lock(&repo_url, &passphrase).await?;

    info!(repo_id, name = %repo.name, "repository lock broken");

    Ok(Json(BreakLockResponse {
        message: format!("lock broken on repository '{}'", repo.name),
        borg_output,
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

    let repo_url = format!(
        "ssh://{}@{}:{}/{}",
        repo.ssh_user, repo.ssh_host, repo.ssh_port, repo.repo_path
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
    let borg_binary = std::env::var("BORG_BINARY").unwrap_or_else(|_| "borg".to_string());
    let ssh_auth_sock = std::env::var("SSH_AUTH_SOCK").ok();

    let mut cmd = Command::new(&borg_binary);
    cmd.args(["info", "--json", repo_url])
        .env("BORG_PASSPHRASE", passphrase)
        .env(
            "BORG_RSH",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        );

    if let Some(sock) = &ssh_auth_sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }

    let output = cmd
        .output()
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
    let borg_binary = std::env::var("BORG_BINARY").unwrap_or_else(|_| "borg".to_string());
    let ssh_auth_sock = std::env::var("SSH_AUTH_SOCK").ok();

    let mut cmd = Command::new(&borg_binary);
    cmd.args(["break-lock", repo_url])
        .env("BORG_PASSPHRASE", passphrase)
        .env(
            "BORG_RSH",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        );

    if let Some(sock) = &ssh_auth_sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }

    let output = cmd
        .output()
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
    let borg_binary = std::env::var("BORG_BINARY").unwrap_or_else(|_| "borg".to_string());

    let ssh_auth_sock = std::env::var("SSH_AUTH_SOCK").ok();

    let mut cmd = Command::new(&borg_binary);
    cmd.args(["init", "--encryption", encryption.as_borg_arg(), repo_url])
        .env("BORG_PASSPHRASE", passphrase)
        .env(
            "BORG_RSH",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        );

    if let Some(sock) = &ssh_auth_sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }

    let output = cmd
        .output()
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
) -> Result<std::process::Output, ApiError> {
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let output = Command::new(borg_binary())
            .arg("list")
            .arg("--json")
            .arg("--lock-wait")
            .arg(LOCK_WAIT_SECS)
            .arg(borg_repo)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
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

async fn run_borg_archive_info_with_retry(
    repo_archive: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<Option<std::process::Output>, ApiError> {
    for attempt in 1..=LOCK_RETRY_MAX_ATTEMPTS {
        let output = Command::new(borg_binary())
            .arg("info")
            .arg("--json")
            .arg("--lock-wait")
            .arg(LOCK_WAIT_SECS)
            .arg(repo_archive)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| ApiError::Internal(format!("failed to execute borg info: {e}")))?;

        if output.status.success() {
            return Ok(Some(output));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_lock_error(&stderr) || is_ssh_connection_error(&stderr) {
            if attempt == LOCK_RETRY_MAX_ATTEMPTS {
                warn!(
                    repo_archive,
                    stderr = %stderr,
                    "borg info exhausted retries, skipping archive"
                );
                return Ok(None);
            }
            warn!(
                attempt,
                max = LOCK_RETRY_MAX_ATTEMPTS,
                repo_archive,
                "borg info retryable error, retrying in {}s",
                LOCK_RETRY_INTERVAL.as_secs()
            );
            tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
        } else {
            warn!(
                repo_archive,
                stderr = %stderr,
                "borg info non-retryable error, skipping archive"
            );
            return Ok(None);
        }
    }
    unreachable!()
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
) -> Result<u64, ApiError> {
    let (borg_repo, env) = super::archives::get_repo_env(pool, encryption_key, repo_id).await?;

    let output = run_borg_list_with_retry(&borg_repo, &env).await?;

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg list output: {e}")))?;

    let archives = json_output["archives"]
        .as_array()
        .map_or_else(Vec::new, Clone::clone);

    if archives.is_empty() {
        return Ok(0);
    }

    const CONCURRENCY: usize = 8;

    let mut archive_clients: Vec<(String, String, i64, bool)> = Vec::with_capacity(archives.len());
    for archive in &archives {
        let name = archive["name"].as_str().unwrap_or_default();
        let hostname = archive["hostname"].as_str().unwrap_or("unknown");
        if name.is_empty() {
            continue;
        }
        let (client, matched) = match db::resolve_client_for_hostname(pool, hostname).await? {
            db::ResolveResult::ExactMatch(c) => (c, true),
            db::ResolveResult::PatternMatch(c) => (c, true),
            db::ResolveResult::Unmatched => {
                let c = db::get_or_create_client_by_hostname(pool, hostname).await?;
                (c, false)
            }
        };
        archive_clients.push((name.to_string(), hostname.to_string(), client.id, matched));
    }

    let total = archive_clients.len();
    let _ = db::update_repo_import_progress(pool, repo_id, 0, total as i64).await;
    ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);

    let (imported, _skipped): (u64, u64) = stream::iter(archive_clients)
        .map(|(name, _hostname, client_id, matched)| {
            let borg_repo = &borg_repo;
            let env = &env;
            async move {
                let repo_archive = format!("{borg_repo}::{name}");
                let info_output = match run_borg_archive_info_with_retry(&repo_archive, env).await {
                    Ok(Some(output)) => output,
                    Ok(None) => {
                        return (None, true);
                    }
                    Err(e) => {
                        warn!(archive = %name, error = %e, "borg info error, skipping");
                        return (None, true);
                    }
                };

                let info_json: serde_json::Value = match serde_json::from_slice(&info_output.stdout)
                {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(archive = %name, error = %e, "failed to parse borg info output");
                        return (None, true);
                    }
                };

                let Some(archive_info) = info_json["archives"].as_array().and_then(|a| a.first())
                else {
                    return (None, true);
                };

                let stats = &archive_info["stats"];
                let start_str = archive_info["start"].as_str().unwrap_or_default();
                let end_str = archive_info["end"].as_str().unwrap_or_default();
                let duration = archive_info["duration"].as_f64().unwrap_or(0.0);

                let Some(started_at) = parse_borg_timestamp(start_str) else {
                    return (None, true);
                };
                let Some(finished_at) = parse_borg_timestamp(end_str) else {
                    return (None, true);
                };

                #[allow(clippy::cast_possible_truncation)]
                let params = db::InsertReportParams {
                    client_id,
                    repo_id,
                    started_at,
                    finished_at,
                    status: "success".to_string(),
                    original_size: stats["original_size"].as_i64().unwrap_or(0),
                    compressed_size: stats["compressed_size"].as_i64().unwrap_or(0),
                    deduplicated_size: stats["deduplicated_size"].as_i64().unwrap_or(0),
                    files_processed: stats["nfiles"].as_i64().unwrap_or(0),
                    duration_secs: duration as i64,
                    error_message: None,
                    warnings: vec![],
                    borg_version: None,
                    matched,
                    archive_name: Some(name.to_string()),
                };

                (Some(params), false)
            }
        })
        .buffer_unordered(CONCURRENCY)
        .fold((0u64, 0u64), |(imported, skipped), (params, is_skipped)| async move {
            let (new_imported, new_skipped) = if is_skipped {
                (imported, skipped + 1)
            } else {
                let Some(params) = params else {
                    return (imported, skipped + 1);
                };
                let name = params.archive_name.as_deref().unwrap_or("?");
                if let Err(e) = db::insert_backup_report(pool, &params).await {
                    warn!(archive = name, error = %e, "failed to insert imported archive report");
                    (imported, skipped + 1)
                } else {
                    (imported + 1, skipped)
                }
            };
            let processed = new_imported + new_skipped;
            if processed % 10 == 0 || processed as usize == total {
                let _ = db::update_repo_import_progress(
                    pool,
                    repo_id,
                    processed as i64,
                    total as i64,
                )
                .await;
                ui_broadcast.send(shared::protocol::ServerToUi::DataChanged);
            }
            (new_imported, new_skipped)
        })
        .await;

    info!(
        repo_id,
        imported,
        skipped = total as u64 - imported,
        total,
        "synced existing archives"
    );
    Ok(imported)
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
    pub duration_secs: u64,
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

    db::set_repo_importing(&state.pool, repo_id, false).await?;
    db::update_repo_last_synced(&state.pool, repo_id).await?;

    let imported = match result {
        Ok(n) => n,
        Err(e) => {
            let msg = format!(
                "repo sync failed for '{}' after {:.1}s: {e}",
                repo.name,
                elapsed.as_secs_f64()
            );
            error!("{msg}");
            if let Err(log_err) =
                db::insert_system_event(&state.pool, "repo_sync", None, &msg).await
            {
                error!(error = %log_err, "failed to log sync event");
            }
            return Err(e);
        }
    };

    let duration_secs = elapsed.as_secs();
    let msg = format!(
        "repo sync completed for '{}': imported {imported} archives in {duration_secs}s",
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
}
