// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{process::Stdio, time::Duration};

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use shared::{protocol::ServerToAgent, types::RepoId};
use tokio::{process::Command, sync::oneshot};
use tokio_util::io::ReaderStream;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    archives::{LOCK_WAIT_SECS, borg_binary, get_repo_env, validate_path},
    auth::{AuthUser, RequireAdmin},
    permissions::check_repo_permission,
};
use crate::{AppState, db, error::ApiError};

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
    summary = "Download selected files/directories from an archive as a streaming tar.lz4",
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

    let mut borg_cmd = Command::new(borg_binary());
    borg_cmd
        .arg("export-tar")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&repo_archive)
        .arg("-")
        .args(&body.paths)
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut borg = borg_cmd
        .spawn()
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let borg_stdout = borg
        .stdout
        .take()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdout".to_string()))?;

    let lz4_available = Command::new("lz4")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    let (body_stream, filename) = if lz4_available {
        let mut lz4 = Command::new("lz4")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ApiError::Internal(format!("failed to spawn lz4: {e}")))?;

        let lz4_stdin = lz4
            .stdin
            .take()
            .ok_or_else(|| ApiError::Internal("failed to capture lz4 stdin".to_string()))?;

        let lz4_stdout = lz4
            .stdout
            .take()
            .ok_or_else(|| ApiError::Internal("failed to capture lz4 stdout".to_string()))?;

        tokio::spawn(async move {
            let mut stdin = lz4_stdin;
            let mut stdout = borg_stdout;
            tokio::io::copy(&mut stdout, &mut stdin).await.ok();
            drop(stdin);
            let _r = borg.wait().await;
            let _r = lz4.wait().await;
        });

        let stream = ReaderStream::new(lz4_stdout);
        (
            Body::from_stream(stream),
            format!("{archive_name}.tar.lz4"),
        )
    } else {
        tokio::spawn(async move {
            let _r = borg.wait().await;
        });

        let stream = ReaderStream::new(borg_stdout);
        (Body::from_stream(stream), format!("{archive_name}.tar"))
    };

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
            (
                header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body_stream,
    )
        .into_response())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreFilesRequest {
    pub paths: Vec<String>,
    pub target_path: String,
    pub hostname: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RestoreFilesResponse {
    pub success: bool,
    pub files_restored: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/restore",
    tag = "Archives",
    operation_id = "restoreFiles",
    summary = "Restore selected files from an archive to the agent filesystem",
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
pub async fn restore_files(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<RestoreFilesRequest>,
) -> Result<Json<RestoreFilesResponse>, ApiError> {
    if body.paths.is_empty() {
        return Err(ApiError::BadRequest(
            "paths array must not be empty".to_owned(),
        ));
    }

    if body.target_path.is_empty() {
        return Err(ApiError::BadRequest(
            "target_path must not be empty".to_owned(),
        ));
    }

    if !state.registry.is_connected(&body.hostname).await {
        return Err(ApiError::ServiceUnavailable(
            "agent is offline".to_owned(),
        ));
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
        return Err(ApiError::ServiceUnavailable(
            "agent is offline".to_owned(),
        ));
    }

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(_admin.user_id),
            username: &_admin.username,
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
        Ok(Ok((success, files_restored, error_message))) => {
            Ok(Json(RestoreFilesResponse {
                success,
                files_restored,
                error_message,
            }))
        }
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
