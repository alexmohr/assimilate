// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use lz4_flex::frame::FrameEncoder;
use serde::Deserialize;
use tokio_util::io::{ReaderStream, SyncIoBridge};

use super::{
    archives::{LOCK_WAIT_SECS, get_repo_env},
    auth::AuthUser,
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, error::ApiError};

/// Query parameters for archive export.
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// Optional subdirectory path to limit the export to.
    pub path: Option<String>,
}

fn validate_export_path(path: &str) -> Result<(), ApiError> {
    if path.starts_with('-') {
        return Err(ApiError::BadRequest(
            "paths must not start with '-'".to_string(),
        ));
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
/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn export_archive(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;

    let export_path = query.path.filter(|path| !path.is_empty());
    if let Some(ref p) = export_path {
        validate_export_path(p)?;
    }

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let positional: &[String] = match export_path {
        Some(ref p) => std::slice::from_ref(p),
        None => &[],
    };
    let args = Borg::args_with_positional(
        &[
            "export-tar",
            "--lock-wait",
            LOCK_WAIT_SECS,
            repo_archive.as_str(),
            "-",
        ],
        positional,
    );

    let mut child = Borg::new()
        .spawn(&args, &env)
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let borg_stdout = child
        .take_stdout()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdout".to_string()))?;

    let (reader, writer) = tokio::io::duplex(64 * 1024);

    // Pipe borg's stdout through the lz4 encoder as it arrives, rather than
    // buffering the whole tar in memory first. Aborting the download (browser
    // cancel, client disconnect) drops `reader`, which makes writes into
    // `writer` fail, so the copy below returns early and `child` is dropped
    // (SIGTERM, escalating to SIGKILL + break-lock) instead of running to
    // completion regardless of the client.
    tokio::spawn(async move {
        let mut sync_stdout = SyncIoBridge::new(borg_stdout);
        let sync_writer = SyncIoBridge::new(writer);
        tokio::task::spawn_blocking(move || {
            let mut encoder = FrameEncoder::new(sync_writer);
            std::io::copy(&mut sync_stdout, &mut encoder).ok();
            encoder.finish().ok();
        })
        .await
        .ok();

        let _r = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
    });

    let stream = ReaderStream::new(reader);
    let body = Body::from_stream(stream);
    let filename = format!("{archive_name}.tar.lz4");

    let disposition = format!("attachment; filename=\"{filename}\"");

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_selects_the_whole_archive() {
        assert!(validate_export_path("").is_ok());
    }

    #[test]
    fn validate_export_path_rejects_leading_dash() {
        assert!(validate_export_path("-").is_err());
        assert!(validate_export_path("-something").is_err());
        assert!(validate_export_path("--flag").is_err());
    }

    #[test]
    fn path_validation_rejects_unsafe_paths() {
        assert!(validate_export_path("/etc/passwd").is_err());
        assert!(validate_export_path("../etc/passwd").is_err());
        assert!(validate_export_path("etc/../passwd").is_err());
        assert!(validate_export_path("etc\0passwd").is_err());
    }
}
