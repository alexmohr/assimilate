// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::process::Stdio;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::process::Command;
use tokio_util::io::ReaderStream;

use super::{
    archives::{LOCK_WAIT_SECS, borg_binary, get_repo_env},
    auth::AuthUser,
    permissions::check_repo_permission,
};
use crate::{AppState, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub path: Option<String>,
}

fn validate_export_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() {
        return Err(ApiError::BadRequest("path must not be empty".to_string()));
    }
    if path.starts_with('/') {
        return Err(ApiError::BadRequest(
            "absolute paths not allowed".to_string(),
        ));
    }
    if path.contains("..") {
        return Err(ApiError::BadRequest(
            "path traversal not allowed".to_string(),
        ));
    }
    if path.contains('\0') {
        return Err(ApiError::BadRequest(
            "null bytes not allowed in path".to_string(),
        ));
    }
    let has_invalid_component = std::path::Path::new(path).components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    });
    if has_invalid_component {
        return Err(ApiError::BadRequest(
            "path traversal not allowed".to_string(),
        ));
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/export",
    tag = "Archives",
    operation_id = "exportArchive",
    summary = "Export an archive as a streaming tar.lz4 download",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        ("path" = Option<String>, Query,
            description = "Optional subdirectory to limit the export to"),
    ),
    responses(
        (status = 200, description = "Archive tar.lz4 stream",
            content_type = "application/octet-stream"),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
pub async fn export_archive(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;

    if let Some(ref p) = query.path {
        validate_export_path(p)?;
    }

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let mut borg_cmd = Command::new(borg_binary());
    borg_cmd
        .arg("export-tar")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&repo_archive)
        .arg("-")
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if let Some(ref p) = query.path {
        borg_cmd.arg(p);
    }

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

    let (body, filename) = if lz4_available {
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

    let disposition = format!("attachment; filename=\"{filename}\"");

    Ok((
        [
            (
                header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}
