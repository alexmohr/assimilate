// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, path::Path, time::Duration};

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use shared::{
    responses::{
        ArchiveEntryResponse, ArchiveIndexStatusResponse, ArchiveInfoResponse,
        DeleteArchiveResponse as SharedDeleteArchiveResponse,
    },
    types::build_repo_url,
};
use sqlx::PgPool;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    sync::oneshot,
};
use tokio_util::io::ReaderStream;

use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{AppState, borg::Borg, db, error::ApiError};

fn index_status_to_string(s: &crate::archive_index::IndexStatus) -> String {
    match s {
        crate::archive_index::IndexStatus::Pending => "pending",
        crate::archive_index::IndexStatus::Indexing => "indexing",
        crate::archive_index::IndexStatus::Done => "done",
        crate::archive_index::IndexStatus::Failed => "failed",
    }
    .to_string()
}

pub const LOCK_WAIT_SECS: &str = "60";

/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub fn validate_path(path: &str) -> Result<(), ApiError> {
    if path.is_empty() {
        return Err(ApiError::BadRequest("path must not be empty".to_string()));
    }
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

fn validate_extract_path(path: &str) -> Result<(), ApiError> {
    validate_path(path)?;
    if path.ends_with('/') {
        return Err(ApiError::BadRequest(
            "cannot extract directories".to_string(),
        ));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_repo_env(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
) -> Result<(String, HashMap<String, String>), ApiError> {
    let repo = db::get_repo_with_passphrase(pool, repo_id).await?;

    let passphrase =
        shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, encryption_key)?;

    let borg_repo = build_repo_url(
        &repo.ssh_user,
        &repo.ssh_host,
        u16::try_from(repo.ssh_port).unwrap_or(22),
        &repo.repo_path,
    );

    let mut env = super::helpers::borg_base_env(&passphrase);

    if repo.relocation_pending {
        env.insert(
            "BORG_RELOCATED_REPO_ACCESS_IS_OK".to_string(),
            "yes".to_string(),
        );
    }

    Ok((borg_repo, env))
}

#[must_use]
pub fn classify_borg_error(exit_code: i32, stderr: &str) -> ApiError {
    if exit_code == 1 && stderr.to_lowercase().contains("lock") {
        return ApiError::Conflict("repository is locked by another operation".to_string());
    }
    if stderr.contains("Archive") && stderr.contains("does not exist") {
        return ApiError::NotFound(format!("archive not found: {stderr}"));
    }
    if stderr.contains("Connection refused")
        || stderr.contains("Connection timed out")
        || stderr.contains("ssh: connect to host")
        || stderr.contains("Could not resolve hostname")
    {
        return ApiError::BadGateway(format!("SSH connection failed: {stderr}"));
    }
    ApiError::Internal(format!("borg command failed (exit {exit_code}): {stderr}"))
}

enum ContentType {
    TextPlain,
    TextHtml,
    TextCss,
    Javascript,
    Json,
    Xml,
    Pdf,
    Zip,
    Gzip,
    Tar,
    Png,
    Jpeg,
    Gif,
    Svg,
    OctetStream,
}

impl From<&str> for ContentType {
    fn from(ext: &str) -> Self {
        match ext {
            "txt" | "log" | "conf" | "cfg" | "ini" => Self::TextPlain,
            "html" | "htm" => Self::TextHtml,
            "css" => Self::TextCss,
            "js" => Self::Javascript,
            "json" => Self::Json,
            "xml" => Self::Xml,
            "pdf" => Self::Pdf,
            "zip" => Self::Zip,
            "gz" | "gzip" => Self::Gzip,
            "tar" => Self::Tar,
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "gif" => Self::Gif,
            "svg" => Self::Svg,
            _ => Self::OctetStream,
        }
    }
}

impl ContentType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TextPlain => "text/plain",
            Self::TextHtml => "text/html",
            Self::TextCss => "text/css",
            Self::Javascript => "application/javascript",
            Self::Json => "application/json",
            Self::Xml => "application/xml",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
            Self::Gzip => "application/gzip",
            Self::Tar => "application/x-tar",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Svg => "image/svg+xml",
            Self::OctetStream => "application/octet-stream",
        }
    }
}

fn content_type_for_extension(filename: &str) -> &'static str {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    ContentType::from(ext).as_str()
}

fn ensure_utc_suffix(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    if ts.ends_with('Z') || ts.contains('+') {
        ts.to_string()
    } else {
        format!("{ts}Z")
    }
}

pub use shared::responses::ContentEntryResponse as ContentEntry;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContentsResponse {
    pub index_status: String,
    pub entries: Vec<ContentEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ArchiveIndexStatus {
    pub status: crate::archive_index::IndexStatus,
    pub file_count: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContentsQuery {
    pub path: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractQuery {
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeleteArchiveResponse {
    pub success: bool,
    pub archive_name: String,
}

#[derive(sqlx::FromRow)]
struct ListArchivesRow {
    archive_name: Option<String>,
    started_at: DateTime<Utc>,
    original_size: i64,
    deduplicated_size: i64,
    matched: bool,
    agent_hostname: String,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives",
    tag = "Archives",
    operation_id = "listArchives",
    summary = "List all archives in a repository",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "List of archives", body = Vec<ArchiveEntryResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_archives(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(repo_id): AxumPath<i64>,
) -> Result<Json<Vec<ArchiveEntryResponse>>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;

    let rows = sqlx::query_as!(
        ListArchivesRow,
        "WITH latest_archives AS (SELECT DISTINCT ON (br.archive_name) br.archive_name, \
         br.started_at, br.original_size, br.deduplicated_size, br.matched, c.hostname AS \
         agent_hostname FROM backup_reports br JOIN agents c ON c.id = br.agent_id WHERE \
         br.repo_id = $1 AND br.archive_name IS NOT NULL AND br.status IN ('success', 'warning') \
         ORDER BY br.archive_name, br.started_at DESC, br.id DESC) SELECT archive_name, \
         started_at, original_size, deduplicated_size, matched, agent_hostname FROM \
         latest_archives ORDER BY started_at DESC",
        repo_id,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let archives = rows
        .into_iter()
        .map(|row| {
            let name = row.archive_name.unwrap_or_default();
            let start = row.started_at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
            ArchiveEntryResponse {
                name,
                start,
                hostname: row.agent_hostname.clone(),
                comment: String::new(),
                original_size: row.original_size,
                deduplicated_size: row.deduplicated_size,
                matched: Some(row.matched),
                agent_hostname: Some(row.agent_hostname),
            }
        })
        .collect();

    Ok(Json(archives))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}",
    tag = "Archives",
    operation_id = "archiveInfo",
    summary = "Get statistics for a specific archive",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "Archive info", body = ArchiveInfoResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn archive_info(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<ArchiveInfoResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let output = Borg::new()
        .run(
            &[
                "info",
                "--json",
                "--lock-wait",
                LOCK_WAIT_SECS,
                repo_archive.as_str(),
            ],
            &env,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg output: {e}")))?;

    let archive = json_output
        .get("archives")
        .and_then(serde_json::Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| ApiError::NotFound(format!("archive '{archive_name}' not found")))?;

    let stats = archive.get("stats");

    let info = ArchiveInfoResponse {
        original_size: stats
            .and_then(|s| s.get("original_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        compressed_size: stats
            .and_then(|s| s.get("compressed_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        deduplicated_size: stats
            .and_then(|s| s.get("deduplicated_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        nfiles: stats
            .and_then(|s| s.get("nfiles"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        duration: archive
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        start: ensure_utc_suffix(
            archive
                .get("start")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        ),
        end: ensure_utc_suffix(
            archive
                .get("end")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        ),
    };

    Ok(Json(info))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}/archives/{archive_name}",
    tag = "Archives",
    operation_id = "deleteArchive",
    summary = "Delete a single archive from a repository",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 202, description = "Archive deletion started",
            body = SharedDeleteArchiveResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive not found"),
        (status = 409, description = "Another repository operation is in progress"),
    )
)]
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_archive(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<(StatusCode, Json<SharedDeleteArchiveResponse>), ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_delete).await?;
    // Resolve credentials up front so authorisation/borg-config errors surface
    // synchronously; the slow `borg delete` itself runs in the background so the
    // UI is never blocked waiting for it.
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    // Queue the deletion rather than rejecting concurrent requests: every
    // server-side borg operation for a repository runs sequentially via the
    // per-repo lock, so deleting many archives at once just lines them up.
    state.repo_op_tracker.enqueue(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });

    tokio::spawn(run_archive_deletion(
        state,
        repo_id,
        archive_name.clone(),
        borg_repo,
        env,
        auth.user_id,
        auth.username,
    ));

    Ok((
        StatusCode::ACCEPTED,
        Json(SharedDeleteArchiveResponse {
            success: true,
            archive_name,
        }),
    ))
}

async fn run_archive_deletion(
    state: AppState,
    repo_id: i64,
    archive_name: String,
    borg_repo: String,
    env: HashMap<String, String>,
    user_id: i64,
    username: String,
) {
    // Serialise with every other borg operation on this repository. While we
    // wait here the deletion stays counted as queued in the op tracker.
    let _repo_guard = state.repo_lock.acquire(repo_id).await;
    state
        .repo_op_tracker
        .begin(
            repo_id,
            shared::protocol::RepoOpKind::DeleteArchive,
            username.clone(),
        )
        .await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });

    let deleted = execute_borg_delete(&state, repo_id, &borg_repo, &archive_name, &env).await;

    state.repo_op_tracker.clear(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });

    if !deleted {
        state
            .ui_broadcast
            .send(shared::protocol::ServerToUi::DataChanged);
        return;
    }

    finalize_archive_deletion(&state, repo_id, &archive_name, user_id, &username).await;

    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
}

/// Runs `borg delete` for the given archive, logging and recording a system
/// event on failure. Returns `true` if the archive was actually deleted (or
/// borg reported it as already gone, exit code 1) and the caller should
/// proceed with local bookkeeping.
async fn execute_borg_delete(
    state: &AppState,
    repo_id: i64,
    borg_repo: &str,
    archive_name: &str,
    env: &HashMap<String, String>,
) -> bool {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let result = Borg::new()
        .run(
            &[
                "delete",
                "--lock-wait",
                LOCK_WAIT_SECS,
                "--",
                repo_archive.as_str(),
            ],
            env,
        )
        .await;

    match result {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            if exit_code != 0 && exit_code != 1 {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let err = classify_borg_error(exit_code, &stderr);
                tracing::error!(repo_id, archive = %archive_name, error = %err, "archive deletion failed");
                let msg =
                    format!("failed to delete archive '{archive_name}' (repo {repo_id}): {err}");
                if let Err(e) =
                    db::insert_system_event(&state.pool, "archive_delete_failed", None, &msg).await
                {
                    tracing::warn!(error = %e, "failed to log archive delete failure");
                }
                return false;
            }
            true
        }
        Err(e) => {
            tracing::error!(repo_id, archive = %archive_name, error = %e, "failed to execute borg delete");
            let msg = format!("failed to delete archive '{archive_name}' (repo {repo_id}): {e}");
            if let Err(log_err) =
                db::insert_system_event(&state.pool, "archive_delete_failed", None, &msg).await
            {
                tracing::warn!(error = %log_err, "failed to log archive delete failure");
            }
            false
        }
    }
}

/// Deletes the local archive records, writes an audit log entry, and (once
/// the deletion queue for this repo has drained) reconciles the archive list
/// and repo stats by reusing the metadata import path. Content indexing is
/// deliberately not run here.
async fn finalize_archive_deletion(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
    user_id: i64,
    username: &str,
) {
    if let Err(e) =
        db::delete_archive_records_by_names(&state.pool, repo_id, &[archive_name.to_owned()]).await
    {
        tracing::error!(repo_id, archive = %archive_name, error = %e, "failed to delete archive record");
    }

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(user_id),
            username,
            action: "delete_archive",
            target_type: Some("archive"),
            target_id: Some(repo_id),
            details: Some(serde_json::json!({ "archive": archive_name })),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    if state.repo_op_tracker.queued_count(repo_id).await == 0 {
        if let Err(e) = crate::api::repos::sync_existing_archives(
            &state.pool,
            &state.encryption_key,
            repo_id,
            &state.ui_broadcast,
        )
        .await
        {
            tracing::warn!(repo_id, error = %e, "post-delete archive list refresh failed");
        }
        crate::api::repos::clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id)
            .await;
    }
}

// Strip the leading "./" or bare "." that borg emits when archives are
// created with "borg create repo::name ." so the API always returns
// clean relative paths (e.g. "var/www" instead of "./var/www").
#[allow(
    unknown_lints,
    reason = "no_string_control_flow is a workspace-local dylint lint, unknown to plain \
              rustc/clippy"
)]
#[allow(
    no_string_control_flow,
    reason = "\".\" is borg's literal path-syntax token for the archive root, not domain state \
              with an enum representation"
)]
pub(crate) fn normalize_path(p: &str) -> String {
    if p == "." {
        String::new()
    } else {
        p.strip_prefix("./").unwrap_or(p).to_string()
    }
}

// Build subtree borg patterns for a directory listing request.
//
// We request the whole subtree (using `**` which crosses separators in borg
// 1.4+) and then fold the raw entries into immediate children ourselves via
// `fold_immediate_children`. This is more robust than depth-limited patterns
// because it synthesises directory entries for intermediate directories that
// borg may not have emitted (e.g. archives created with borg 1.2 or with
// unusual path styles).
fn list_patterns(path: Option<&str>) -> Vec<String> {
    match path {
        None => vec!["+sh:**".to_string()],
        Some(p) => {
            let p = p.trim_end_matches('/');
            vec![
                format!("+sh:{p}"),
                format!("+sh:{p}/**"),
                format!("+sh:./{p}"),
                format!("+sh:./{p}/**"),
                "-sh:**".to_string(),
            ]
        }
    }
}

// Given a flat stream of borg entries for a subtree rooted at `prefix`
// (empty string = archive root), return only the immediate children.
//
// For each normalised entry path:
// - If the path is exactly `prefix` (i.e. the directory entry itself) -> skip.
// - Strip `prefix/` to get the remainder.
// - Take the first path segment of the remainder as the child name.
// - If there are more segments (i.e. the entry is deeper than one level),
//   synthesise a directory `ContentEntry` for the child directory name.
// - Otherwise use the real entry.
//
// Directories are deduplicated by name; the first occurrence wins.
fn fold_immediate_children(prefix: &str, entries: Vec<ContentEntry>) -> Vec<ContentEntry> {
    // Tracks the child paths already emitted (both real and synthesised),
    // preventing both duplicate synthetic dirs and synthetic dirs clobbering
    // a real immediate-child entry that was emitted first.
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result: Vec<ContentEntry> = Vec::new();

    for entry in entries {
        let path = &entry.path;

        // Determine the remainder relative to prefix.
        let remainder = if prefix.is_empty() {
            path.as_str()
        } else if path == prefix {
            // This is the directory entry for the requested path itself - skip.
            continue;
        } else if let Some(rel) = path.strip_prefix(&format!("{prefix}/")) {
            rel
        } else {
            // Entry is outside the requested subtree (shouldn't happen with correct patterns).
            continue;
        };

        if remainder.is_empty() {
            continue;
        }

        // Split on the first '/' to get the immediate child name.
        if let Some(slash) = remainder.find('/') {
            // Deeper entry - synthesise a directory for the first segment.
            let dir_name = &remainder[..slash];
            let child_path = if prefix.is_empty() {
                dir_name.to_string()
            } else {
                format!("{prefix}/{dir_name}")
            };
            if emitted.insert(child_path.clone()) {
                result.push(ContentEntry {
                    entry_type: "d".to_string(),
                    path: child_path,
                    size: 0,
                    mtime: entry.mtime.clone(),
                    mode: String::new(),
                });
            }
        } else {
            // Immediate child - use the real entry unless we already emitted
            // something with this path (e.g. a synthesised directory first).
            if emitted.insert(path.clone()) {
                result.push(entry);
            }
        }
    }

    result
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/contents",
    tag = "Archives",
    operation_id = "listContents",
    summary = "List files in an archive at a given path",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        ("path" = Option<String>, Query, description = "Directory path to list (default: /)"),
        ("limit" = Option<usize>, Query,
            description = "Max entries to return (default: 100)"),
    ),
    responses(
        (status = 200, description = "Directory contents", body = ContentsResponse),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn list_contents(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<ContentsQuery>,
) -> Result<Json<ContentsResponse>, ApiError> {
    use crate::archive_index::{self, IndexStatus};

    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let path = query.path.as_deref();
    let limit = query.limit.unwrap_or(100);

    if let Some(p) = path {
        validate_path(p)?;
    }

    let status = archive_index::get_index_status(&state.pool, repo_id, &archive_name).await?;

    match status {
        Some(IndexStatus::Done) => {
            let parent_path = path.map_or("", |p| p.trim_end_matches('/')).to_string();
            let entries = archive_index::query_dir(
                &state.pool,
                repo_id,
                &archive_name,
                &parent_path,
                i64::try_from(limit).unwrap_or(100),
            )
            .await?;
            return Ok(Json(ContentsResponse {
                index_status: index_status_to_string(&IndexStatus::Done),
                entries,
            }));
        }
        Some(IndexStatus::Failed) => {
            // Fall through to the borg-based path below so browsing still works.
        }
        Some(ref pending @ (IndexStatus::Pending | IndexStatus::Indexing)) => {
            return Ok(Json(ContentsResponse {
                index_status: index_status_to_string(pending),
                entries: vec![],
            }));
        }
        None => {
            // Not yet started - claim and launch background indexing.
            let triggered = archive_index::ensure_indexed(
                state.pool.clone(),
                state.encryption_key,
                repo_id,
                archive_name.clone(),
                state.repo_lock.clone(),
            )
            .await?;
            return Ok(Json(ContentsResponse {
                index_status: index_status_to_string(&triggered),
                entries: vec![],
            }));
        }
    }

    // Fallback: borg-based listing (used when index is in 'failed' state).
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let raw_entries = borg_list_raw_entries(&env, &borg_repo, &archive_name, path).await?;

    let prefix = path
        .map(|p| p.trim_end_matches('/').to_string())
        .unwrap_or_default();
    let children = fold_immediate_children(&prefix, raw_entries);
    let limited: Vec<ContentEntry> = children.into_iter().take(limit).collect();

    Ok(Json(ContentsResponse {
        index_status: index_status_to_string(&IndexStatus::Failed),
        entries: limited,
    }))
}

/// Runs `borg list --json-lines` for the archive and parses each line into a
/// [`ContentEntry`], tolerating and skipping unparseable lines. Used as the
/// listing fallback when an archive's content index is in the `failed`
/// state, since browsing should still work even if indexing didn't.
async fn borg_list_raw_entries(
    env: &HashMap<String, String>,
    borg_repo: &str,
    archive_name: &str,
    path: Option<&str>,
) -> Result<Vec<ContentEntry>, ApiError> {
    const LINE_READ_TIMEOUT: Duration = Duration::from_secs(30);

    let repo_archive = format!("{borg_repo}::{archive_name}");
    let patterns = list_patterns(path);

    let mut args: Vec<&str> = vec!["list", "--json-lines", "--lock-wait", LOCK_WAIT_SECS];
    for p in &patterns {
        args.extend_from_slice(&["--pattern", p.as_str()]);
    }
    args.push(repo_archive.as_str());

    let mut child = Borg::new()
        .spawn(&args, env)
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let Some(stdout) = child.take_stdout() else {
        return Err(ApiError::Internal("no stdout from borg".to_string()));
    };

    let mut lines = BufReader::new(stdout).lines();
    let mut raw_entries: Vec<ContentEntry> = Vec::new();

    loop {
        let line = tokio::time::timeout(LINE_READ_TIMEOUT, lines.next_line())
            .await
            .map_err(|_| ApiError::Internal("timed out reading borg output".to_string()))?
            .map_err(|e| ApiError::Internal(format!("reading borg output: {e}")))?;

        let Some(line) = line else { break };

        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line).inspect_err(|e| {
            tracing::trace!(error = %e, "skipping unparseable borg output line");
        }) else {
            continue;
        };
        raw_entries.push(ContentEntry {
            entry_type: v
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            path: v
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map_or_else(String::new, normalize_path),
            size: v
                .get("size")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            mtime: v
                .get("mtime")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            mode: v
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .map_err(|_| ApiError::Internal("borg wait timed out".to_string()))?
        .map_err(|e| ApiError::Internal(format!("borg wait failed: {e}")))?;
    if !status.success() {
        let mut stderr_str = String::new();
        if let Some(mut se) = child.take_stderr() {
            let _ = se.read_to_string(&mut stderr_str).await;
        }
        let code = status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr_str));
    }

    Ok(raw_entries)
}

#[derive(sqlx::FromRow)]
struct ArchiveIndexStatusRow {
    status: String,
    file_count: Option<i64>,
    error_message: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/index-status",
    tag = "Archives",
    operation_id = "getArchiveIndexStatus",
    summary = "Get the index status for an archive",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "Index status", body = ArchiveIndexStatusResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_archive_index_status(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<ArchiveIndexStatusResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;

    let row = sqlx::query_as!(
        ArchiveIndexStatusRow,
        "SELECT j.status, j.file_count, j.error_message FROM archive_index_jobs j JOIN archives a \
         ON a.id = j.archive_id WHERE a.repo_id = $1 AND a.name = $2",
        repo_id,
        archive_name,
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::Database)?;

    let response = row.map_or(
        ArchiveIndexStatusResponse {
            status: "pending".to_string(),
            file_count: None,
            error: None,
        },
        |r| ArchiveIndexStatusResponse {
            status: r.status,
            file_count: r.file_count,
            error: r.error_message,
        },
    );

    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/extract",
    tag = "Archives",
    operation_id = "extractFile",
    summary = "Stream a file from an archive as a binary download",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        ("path" = String, Query, description = "File path within the archive to extract"),
    ),
    responses(
        (status = 200, description = "File content stream",
            content_type = "application/octet-stream"),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn extract_file(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<ExtractQuery>,
) -> Result<Response, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;
    validate_extract_path(&query.path)?;

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let mut child = Borg::new()
        .spawn(
            &[
                "extract",
                "--stdout",
                "--lock-wait",
                LOCK_WAIT_SECS,
                repo_archive.as_str(),
                "--",
                query.path.as_str(),
            ],
            &env,
        )
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let stdout = child
        .take_stdout()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdout".to_string()))?;

    let basename = Path::new(&query.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    let content_type = content_type_for_extension(basename);
    let disposition = format!("attachment; filename=\"{basename}\"");

    let (done_tx, done_rx) = oneshot::channel::<()>();

    // Wrap the stream so the sender is dropped (signalling completion) when the
    // stream is exhausted or the connection is closed.
    let stream = ReaderStream::new(stdout).inspect(move |_| {
        // kept alive until the closure is dropped; no-op on each chunk
        let _ = &done_tx;
    });
    let body = Body::from_stream(stream);

    // Hold the child alive until the stream finishes or the connection is closed,
    // then drop it. Dropping ServerChild sends SIGTERM first (graceful lock
    // release), escalating to SIGKILL + break-lock after 30 seconds if the
    // process has not already exited on its own.
    tokio::spawn(async move {
        let _ = done_rx.await;
        drop(child);
    });

    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_string()),
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
    fn validate_path_rejects_leading_dash() {
        let err = validate_path("-").unwrap_err();
        assert!(err.to_string().contains("must not start with '-'"));
    }

    #[test]
    fn validate_path_rejects_empty() {
        let err = validate_path("").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn validate_path_rejects_absolute() {
        let err = validate_path("/etc/passwd").unwrap_err();
        assert!(err.to_string().contains("absolute paths not allowed"));
    }

    #[test]
    fn validate_path_rejects_traversal() {
        let err = validate_path("foo/../bar").unwrap_err();
        assert!(err.to_string().contains("path traversal not allowed"));
    }

    #[test]
    fn validate_path_rejects_null_bytes() {
        let err = validate_path("foo\0bar").unwrap_err();
        assert!(err.to_string().contains("null bytes not allowed"));
    }

    #[test]
    fn validate_path_accepts_relative() {
        assert!(validate_path("home/user/documents").is_ok());
    }

    #[test]
    fn validate_path_accepts_single_segment() {
        assert!(validate_path("file.txt").is_ok());
    }

    #[test]
    fn validate_path_accepts_nested_relative() {
        assert!(validate_path("a/b/c/d.txt").is_ok());
    }

    #[test]
    fn list_patterns_root_is_wildcard_subtree() {
        let patterns = list_patterns(None);
        assert_eq!(patterns, vec!["+sh:**"]);
    }

    #[test]
    fn list_patterns_simple_dir() {
        let patterns = list_patterns(Some("home"));
        assert_eq!(
            patterns,
            vec![
                "+sh:home",
                "+sh:home/**",
                "+sh:./home",
                "+sh:./home/**",
                "-sh:**"
            ]
        );
    }

    #[test]
    fn list_patterns_nested_dir() {
        let patterns = list_patterns(Some("home/user/docs"));
        assert_eq!(
            patterns,
            vec![
                "+sh:home/user/docs",
                "+sh:home/user/docs/**",
                "+sh:./home/user/docs",
                "+sh:./home/user/docs/**",
                "-sh:**",
            ]
        );
    }

    #[test]
    fn list_patterns_trailing_slash_stripped() {
        let patterns = list_patterns(Some("home/user/"));
        assert_eq!(
            patterns,
            vec![
                "+sh:home/user",
                "+sh:home/user/**",
                "+sh:./home/user",
                "+sh:./home/user/**",
                "-sh:**",
            ]
        );
    }

    fn make_entry(entry_type: &str, path: &str) -> ContentEntry {
        ContentEntry {
            entry_type: entry_type.to_string(),
            path: path.to_string(),
            size: 0,
            mtime: "2024-01-01T00:00:00".to_string(),
            mode: String::new(),
        }
    }

    #[test]
    fn fold_synthesises_dir_from_nested_file() {
        // Archive has etc/passwd but no etc/ entry.
        let entries = vec![make_entry("-", "etc/passwd")];
        let result = fold_immediate_children("", entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_type, "d");
        assert_eq!(result[0].path, "etc");
    }

    #[test]
    fn fold_uses_real_entry_for_immediate_file() {
        let entries = vec![make_entry("-", "file.txt")];
        let result = fold_immediate_children("", entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_type, "-");
        assert_eq!(result[0].path, "file.txt");
    }

    #[test]
    fn fold_deduplicates_synthesised_dirs() {
        let entries = vec![
            make_entry("-", "etc/passwd"),
            make_entry("-", "etc/hostname"),
        ];
        let result = fold_immediate_children("", entries);
        assert_eq!(result.len(), 1, "etc should appear only once");
        assert_eq!(result[0].entry_type, "d");
        assert_eq!(result[0].path, "etc");
    }

    #[test]
    fn fold_excludes_deeper_levels() {
        let entries = vec![
            make_entry("d", "home"),
            make_entry("d", "home/user"),
            make_entry("-", "home/user/notes.txt"),
        ];
        let result = fold_immediate_children("", entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "home");
        assert_eq!(result[0].entry_type, "d");
    }

    #[test]
    fn fold_with_prefix_strips_prefix() {
        let entries = vec![
            make_entry("d", "etc"),
            make_entry("-", "etc/passwd"),
            make_entry("-", "etc/hostname"),
        ];
        let result = fold_immediate_children("etc", entries);
        assert_eq!(result.len(), 2);
        let paths: Vec<&str> = result.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"etc/passwd"));
        assert!(paths.contains(&"etc/hostname"));
    }

    #[test]
    fn fold_with_prefix_synthesises_subdir() {
        let entries = vec![make_entry("-", "usr/local/bin/tool")];
        let result = fold_immediate_children("usr", entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_type, "d");
        assert_eq!(result[0].path, "usr/local");
    }

    #[test]
    fn fold_handles_dot_slash_prefix_after_normalize() {
        // After normalize_path, "./etc/passwd" becomes "etc/passwd".
        let entries = vec![make_entry("-", "etc/passwd")];
        let result = fold_immediate_children("", entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "etc");
    }

    #[test]
    fn normalize_path_strips_dot_slash_prefix() {
        assert_eq!(normalize_path("./var/www"), "var/www");
    }

    #[test]
    fn normalize_path_bare_dot_becomes_empty() {
        assert_eq!(normalize_path("."), "");
    }

    #[test]
    fn normalize_path_bare_path_unchanged() {
        assert_eq!(normalize_path("home/user"), "home/user");
    }

    #[test]
    fn normalize_path_empty_unchanged() {
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn delete_archive_response_serializes_api_contract() {
        let response = DeleteArchiveResponse {
            success: true,
            archive_name: "daily-2026-06-06".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "success": true,
                "archive_name": "daily-2026-06-06",
            })
        );
    }

    #[test]
    fn content_type_matches_supported_download_extensions() {
        let cases = [
            ("notes.txt", "text/plain"),
            ("index.html", "text/html"),
            ("styles.css", "text/css"),
            ("app.js", "application/javascript"),
            ("data.json", "application/json"),
            ("feed.xml", "application/xml"),
            ("manual.pdf", "application/pdf"),
            ("files.zip", "application/zip"),
            ("backup.gz", "application/gzip"),
            ("archive.tar", "application/x-tar"),
            ("image.png", "image/png"),
            ("photo.jpeg", "image/jpeg"),
            ("animation.gif", "image/gif"),
            ("diagram.svg", "image/svg+xml"),
            ("binary", "application/octet-stream"),
        ];

        cases.iter().for_each(|(filename, expected)| {
            assert_eq!(content_type_for_extension(filename), *expected);
        });
    }
}
