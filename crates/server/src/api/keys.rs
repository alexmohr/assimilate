// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use super::{
    archives::{ensure_borg_success, get_repo_env},
    auth::RequireAdmin,
};
use crate::{
    AppState,
    borg::Borg,
    db::{
        self,
        audit::{NewAuditEntry, insert_audit_entry},
    },
    error::ApiError,
};

/// Request payload for importing a borg repository key.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportKeyRequest {
    /// The key data to import.
    pub key_data: String,
}

/// Request payload for changing a repository passphrase.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangePassphraseRequest {
    /// The new passphrase.
    pub new_passphrase: String,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/export",
    tag = "Keys",
    operation_id = "exportKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Key exported as text/plain", content_type = "text/plain"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Export the borg repository key.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn export_key(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
) -> Result<Response, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let output = Borg::new()
        .with_registry(state.task_registry.clone())
        .run(&["key", "export", "--stdout", borg_repo.as_str()], &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;
    let stdout = ensure_borg_success(output)?;

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            action: "key_export",
            target_type: Some("repo"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({"action": "key_export", "repo_id": repo_id})),
            ip_address: None,
        },
    )
    .await
    .map_err(ApiError::Database)?;

    let key_text = String::from_utf8(stdout)
        .map_err(|e| ApiError::Internal(format!("borg key output is not valid UTF-8: {e}")))?;

    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        key_text,
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/import",
    tag = "Keys",
    operation_id = "importKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = ImportKeyRequest,
    responses(
        (status = 204, description = "Key imported successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Import a borg repository key.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn import_key(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
    Json(req): Json<ImportKeyRequest>,
) -> Result<StatusCode, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let mut child = Borg::new()
        .with_registry(state.task_registry.clone())
        .spawn_with_stdin(&["key", "import", borg_repo.as_str(), "-"], &env)
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdin".to_string()))?;

    stdin
        .write_all(req.key_data.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to write key to borg stdin: {e}")))?;

    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to wait for borg: {e}")))?;
    ensure_borg_success(output)?;

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            action: "key_import",
            target_type: Some("repo"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({"action": "key_import", "repo_id": repo_id})),
            ip_address: None,
        },
    )
    .await
    .map_err(ApiError::Database)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/change-passphrase",
    tag = "Keys",
    operation_id = "changePassphrase",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = ChangePassphraseRequest,
    responses(
        (status = 204, description = "Passphrase changed successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Change the borg repository passphrase.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn change_passphrase(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
    Json(req): Json<ChangePassphraseRequest>,
) -> Result<StatusCode, ApiError> {
    let (borg_repo, mut env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    env.insert(
        "BORG_NEW_PASSPHRASE".to_string(),
        req.new_passphrase.clone(),
    );

    let output = Borg::new()
        .with_registry(state.task_registry.clone())
        .run(&["key", "change-passphrase", borg_repo.as_str()], &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;
    ensure_borg_success(output)?;

    let encrypted = shared::crypto::encrypt_passphrase(&req.new_passphrase, &state.encryption_key)
        .map_err(|e| ApiError::Internal(format!("failed to encrypt passphrase: {e}")))?;

    db::update_repo_passphrase(&state.pool, repo_id, &encrypted).await?;

    // A real passphrase means the repo is ready for scheduled sync - clear the
    // importing guard that was set (e.g. by config import) to prevent sync with
    // a placeholder passphrase.
    if let Err(e) = db::set_repo_importing(&state.pool, repo_id, false).await {
        tracing::error!(repo_id, error = %e, "failed to clear importing flag after passphrase change");
    }

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            action: "key_change_passphrase",
            target_type: Some("repo"),
            target_id: Some(repo_id),
            details: Some(
                serde_json::json!({"action": "key_change_passphrase", "repo_id": repo_id}),
            ),
            ip_address: None,
        },
    )
    .await
    .map_err(ApiError::Database)?;

    crate::api::helpers::push_config_to_all_agents(&state).await;

    Ok(StatusCode::NO_CONTENT)
}

