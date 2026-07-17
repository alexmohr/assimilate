// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use shared::{protocol::ServerToAgent, types::RepoId};
use tokio::sync::oneshot;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    archives::{get_repo_env, stream_export_tar_lz4, validate_path},
    auth::{AuthUser, RequireAdmin},
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, db, error::ApiError};

/// Request payload for downloading files from an archive.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadFilesRequest {
    /// Paths within the archive to include in the download
    pub paths: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/download",
    tag = "Archives",
    operation_id = "downloadFiles",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = DownloadFilesRequest,
    responses(
        (status = 200, description = "tar.lz4 stream of selected paths",
            content_type = "application/octet-stream"),
        (status = 400, description = "Invalid or empty paths"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Download selected files/directories from an archive as a streaming tar.lz4.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn download_files(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<DownloadFilesRequest>,
) -> Result<Response, ApiError> {
    if body.paths.is_empty() {
        return Err(ApiError::BadRequest(
            "paths array must not be empty".to_string(),
        ));
    }

    for path in &body.paths {
        validate_path(path)?;
    }

    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let body_stream = stream_export_tar_lz4(
        &Borg::new().with_registry(state.task_registry.clone()),
        &repo_archive,
        &body.paths,
        &env,
    )?;
    let filename = format!("{archive_name}.tar.lz4");

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            action: "download_files",
            target_type: Some("archive"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({
                "archive": archive_name,
                "paths": body.paths,
            })),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    let disposition = format!("attachment; filename=\"{filename}\"");

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body_stream,
    )
        .into_response())
}

/// Request payload for restoring files from an archive to an agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreFilesRequest {
    /// Paths within the archive. An empty list restores the whole archive.
    pub paths: Vec<String>,
    /// Target directory on the agent filesystem.
    pub target_path: String,
    /// Hostname of the agent to restore to.
    pub hostname: String,
}

/// Result of a remote restore operation.
#[derive(Debug, Serialize, ToSchema)]
pub struct RestoreFilesResponse {
    /// Whether the restore completed successfully.
    pub success: bool,
    /// Number of files restored.
    pub files_restored: u64,
    /// Error message if the restore failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/restore",
    tag = "Archives",
    operation_id = "restoreFiles",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = RestoreFilesRequest,
    responses(
        (status = 200, description = "Restore completed", body = RestoreFilesResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 500, description = "Restore failed"),
        (status = 503, description = "Agent offline or timed out"),
    )
)]
/// Restore selected files from an archive to the agent filesystem.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::ServiceUnavailable`]: a required dependency (e.g. the target agent) is unavailable
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn restore_files(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<RestoreFilesRequest>,
) -> Result<Json<RestoreFilesResponse>, ApiError> {
    if body.target_path.is_empty() {
        return Err(ApiError::BadRequest(
            "target_path must not be empty".to_owned(),
        ));
    }

    if !state.registry.is_connected(&body.hostname).await {
        return Err(ApiError::ServiceUnavailable("agent is offline".to_owned()));
    }

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();

    state
        .pending_restores
        .lock()
        .await
        .insert(request_id.clone(), tx);

    let msg = ServerToAgent::RestoreFiles {
        request_id: request_id.clone(),
        repo_id: RepoId(repo_id),
        archive_name: archive_name.clone(),
        paths: body.paths.clone(),
        target_path: body.target_path.clone(),
    };

    if state.registry.send_to(&body.hostname, msg).await.is_err() {
        state.pending_restores.lock().await.remove(&request_id);
        return Err(ApiError::ServiceUnavailable("agent is offline".to_owned()));
    }

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            action: "restore_files",
            target_type: Some("archive"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({
                "archive": archive_name,
                "paths": body.paths,
                "target_path": body.target_path,
                "hostname": body.hostname,
            })),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok((success, files_restored, error_message))) => Ok(Json(RestoreFilesResponse {
            success,
            files_restored,
            error_message,
        })),
        Ok(Err(_)) => Err(ApiError::Internal(
            "restore response channel closed unexpectedly".to_owned(),
        )),
        Err(_) => {
            state.pending_restores.lock().await.remove(&request_id);
            Err(ApiError::ServiceUnavailable(
                "restore timed out after 30 seconds".to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_request_deserializes_selected_paths() {
        let request: RestoreFilesRequest = serde_json::from_value(serde_json::json!({
            "paths": ["etc/hosts", "var/lib/app"],
            "target_path": "/tmp/restore",
            "hostname": "web-server-01",
        }))
        .unwrap();

        assert_eq!(request.paths, ["etc/hosts", "var/lib/app"]);
        assert_eq!(request.target_path, "/tmp/restore");
        assert_eq!(request.hostname, "web-server-01");
    }

    #[test]
    fn restore_request_deserializes_empty_paths_for_whole_archive() {
        let request: RestoreFilesRequest = serde_json::from_value(serde_json::json!({
            "paths": [],
            "target_path": "/tmp/restore",
            "hostname": "web-server-01",
        }))
        .unwrap();

        assert!(request.paths.is_empty());
    }

    #[test]
    fn restore_response_omits_missing_error_message() {
        let response = RestoreFilesResponse {
            success: true,
            files_restored: 2,
            error_message: None,
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "success": true,
                "files_restored": 2,
            })
        );
    }

    #[test]
    fn restore_response_includes_error_message() {
        let response = RestoreFilesResponse {
            success: false,
            files_restored: 0,
            error_message: Some("restore failed".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "success": false,
                "files_restored": 0,
                "error_message": "restore failed",
            })
        );
    }
}
