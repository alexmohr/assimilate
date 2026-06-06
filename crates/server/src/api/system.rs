// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::PathBuf;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use ssh_key::{Algorithm, LineEnding, rand_core::OsRng};

use super::deploy::{agent_binary_dir, query_available_agent_version};
use crate::{AppState, api::auth::RequireAdmin, db, error::ApiError};

#[derive(Serialize, utoipa::ToSchema)]
pub struct SshPublicKeyResponse {
    pub public_key: String,
}

fn ssh_key_dir() -> PathBuf {
    PathBuf::from(std::env::var("SSH_KEY_DIR").unwrap_or_else(|_| "/app/ssh".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/system/ssh-public-key",
    tag = "System",
    operation_id = "getSshPublicKey",
    summary = "Get the server's SSH public key",
    responses(
        (status = 200, description = "SSH public key", body = SshPublicKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn ssh_public_key(_admin: RequireAdmin) -> Result<Json<SshPublicKeyResponse>, ApiError> {
    let pub_path = ssh_key_dir().join("id_ed25519.pub");

    let public_key = tokio::fs::read_to_string(&pub_path)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to read SSH public key: {e}")))?;

    Ok(Json(SshPublicKeyResponse {
        public_key: public_key.trim().to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/system/ssh-regenerate-key",
    tag = "System",
    operation_id = "regenerateSshKey",
    summary = "Regenerate the server's SSH key pair",
    responses(
        (status = 200, description = "New SSH public key", body = SshPublicKeyResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 500, description = "Key generation failed"),
    )
)]
pub async fn ssh_regenerate_key(
    _admin: RequireAdmin,
) -> Result<Json<SshPublicKeyResponse>, ApiError> {
    let key_dir = ssh_key_dir();

    tokio::fs::create_dir_all(&key_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to create key directory: {e}")))?;

    let key_path = key_dir.join("id_ed25519");

    if key_path.exists() {
        tokio::fs::remove_file(&key_path)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to remove old private key: {e}")))?;
    }

    let pub_path = key_dir.join("id_ed25519.pub");
    if pub_path.exists() {
        tokio::fs::remove_file(&pub_path)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to remove old public key: {e}")))?;
    }

    let key_path_clone = key_path.clone();
    let pub_path_clone = pub_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        let private_key = ssh_key::PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
            .map_err(|e| ApiError::Internal(format!("failed to generate key: {e}")))?;

        let private_pem = private_key
            .to_openssh(LineEnding::LF)
            .map_err(|e| ApiError::Internal(format!("failed to encode private key: {e}")))?;

        std::fs::write(&key_path_clone, private_pem.as_bytes())
            .map_err(|e| ApiError::Internal(format!("failed to write private key: {e}")))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path_clone, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| {
                    ApiError::Internal(format!("failed to set private key permissions: {e}"))
                })?;
        }

        let public_key = private_key.public_key();
        let pub_str = public_key
            .to_openssh()
            .map_err(|e| ApiError::Internal(format!("failed to encode public key: {e}")))?;

        let pub_with_comment = format!("{pub_str} assimilate-server\n");
        std::fs::write(&pub_path_clone, pub_with_comment.as_bytes())
            .map_err(|e| ApiError::Internal(format!("failed to write public key: {e}")))?;

        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("key generation task failed: {e}")))??;

    let public_key = tokio::fs::read_to_string(&pub_path)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to read new public key: {e}")))?;

    tracing::info!("SSH keypair regenerated");

    Ok(Json(SshPublicKeyResponse {
        public_key: public_key.trim().to_string(),
    }))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SettingsResponse {
    pub retention_days: i64,
    pub timezone: String,
}

#[utoipa::path(
    get,
    path = "/api/system/settings",
    tag = "System",
    operation_id = "getSettings",
    summary = "Get system settings",
    responses(
        (status = 200, description = "System settings", body = SettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn get_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let retention_days = db::get_setting(&state.pool, "retention_days")
        .await?
        .and_then(|v| {
            v.parse::<i64>().inspect_err(|e| {
                tracing::warn!(value = %v, error = %e, "failed to parse retention_days setting");
            }).ok()
        })
        .unwrap_or(7);

    let timezone = db::get_schedule_timezone(&state.pool).await?;

    Ok(Json(SettingsResponse {
        retention_days,
        timezone: timezone.name().to_owned(),
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateSettingsRequest {
    pub retention_days: i64,
    pub timezone: Option<String>,
}

#[utoipa::path(
    put,
    path = "/api/system/settings",
    tag = "System",
    operation_id = "updateSettings",
    summary = "Update system settings",
    request_body = UpdateSettingsRequest,
    responses(
        (status = 200, description = "Updated settings", body = SettingsResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn update_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<SettingsResponse>, ApiError> {
    if body.retention_days < 0 {
        return Err(ApiError::BadRequest(
            "retention_days must be non-negative".to_string(),
        ));
    }

    let timezone = body.timezone.unwrap_or_default();
    if !timezone.is_empty() {
        timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| ApiError::BadRequest(format!("invalid timezone: {timezone}")))?;
    }

    db::set_setting(
        &state.pool,
        "retention_days",
        &body.retention_days.to_string(),
    )
    .await?;

    db::set_setting(&state.pool, "timezone", &timezone).await?;

    let effective_tz = db::get_schedule_timezone(&state.pool).await?;

    Ok(Json(SettingsResponse {
        retention_days: body.retention_days,
        timezone: effective_tz.name().to_owned(),
    }))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct DatabaseStorageResponse {
    pub database_bytes: i64,
    pub other_bytes: i64,
    pub relations: Vec<db::DatabaseRelationSizeRow>,
}

#[utoipa::path(
    get,
    path = "/api/system/database-storage",
    tag = "System",
    operation_id = "getDatabaseStorage",
    summary = "Get PostgreSQL storage usage by application table",
    responses(
        (status = 200, description = "Database storage breakdown", body = DatabaseStorageResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn get_database_storage(
    _admin: RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<DatabaseStorageResponse>, ApiError> {
    let (database_bytes, relations) = db::get_database_storage(&state.pool).await?;
    let relation_bytes = relations
        .iter()
        .map(|relation| relation.total_bytes)
        .sum::<i64>();

    Ok(Json(DatabaseStorageResponse {
        database_bytes,
        other_bytes: database_bytes.saturating_sub(relation_bytes),
        relations,
    }))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VersionResponse {
    pub server_version: String,
    pub server_git_sha: String,
    pub build_timestamp: String,
    pub agent_version: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/system/version",
    tag = "System",
    operation_id = "getVersion",
    summary = "Get server and agent version information",
    responses(
        (status = 200, description = "Version information", body = VersionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn get_version(_admin: RequireAdmin) -> Result<Json<VersionResponse>, ApiError> {
    let binary_dir = agent_binary_dir();
    let agent_version = query_available_agent_version(&binary_dir).await;

    let git_sha = option_env!("GIT_SHA").unwrap_or_default();
    let server_version = if git_sha.is_empty() {
        env!("CARGO_PKG_VERSION").to_owned()
    } else {
        format!("{}+{}", env!("CARGO_PKG_VERSION"), git_sha)
    };
    let build_timestamp = option_env!("BUILD_TIMESTAMP").unwrap_or("unknown");

    Ok(Json(VersionResponse {
        server_version,
        server_git_sha: git_sha.to_owned(),
        build_timestamp: build_timestamp.to_owned(),
        agent_version,
    }))
}
