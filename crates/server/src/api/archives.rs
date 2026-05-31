// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, path::Path, process::Stdio, time::Duration};

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::process::Command;
use tokio_util::io::ReaderStream;

use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{AppState, db, error::ApiError};

const EXTRACT_TIMEOUT: Duration = Duration::from_secs(300);
pub const LOCK_WAIT_SECS: &str = "60";

pub fn borg_binary() -> String {
    std::env::var("BORG_BINARY").unwrap_or_else(|_| "borg".to_string())
}

pub fn validate_path(path: &str) -> Result<(), ApiError> {
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

fn validate_extract_path(path: &str) -> Result<(), ApiError> {
    validate_path(path)?;
    if path.ends_with('/') {
        return Err(ApiError::BadRequest(
            "cannot extract directories".to_string(),
        ));
    }
    Ok(())
}

pub async fn get_repo_env(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
) -> Result<(String, HashMap<String, String>), ApiError> {
    let repo = db::get_repo_with_passphrase(pool, repo_id).await?;

    let passphrase =
        shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, encryption_key)?;

    let borg_repo = format!(
        "ssh://{}@{}:{}/{}",
        repo.ssh_user, repo.ssh_host, repo.ssh_port, repo.repo_path
    );

    let mut env = HashMap::new();
    env.insert("BORG_PASSPHRASE".to_string(), passphrase);
    env.insert(
        "BORG_RSH".to_string(),
        "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string(),
    );

    if repo.relocation_pending {
        env.insert(
            "BORG_RELOCATED_REPO_ACCESS_IS_OK".to_string(),
            "yes".to_string(),
        );
    }

    Ok((borg_repo, env))
}

pub fn classify_borg_error(exit_code: i32, stderr: &str) -> ApiError {
    if exit_code == 1 && stderr.to_lowercase().contains("lock") {
        return ApiError::Conflict("repository is locked by another operation".to_string());
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

fn content_type_for_extension(filename: &str) -> &'static str {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "txt" | "log" | "conf" | "cfg" | "ini" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ArchiveEntry {
    pub name: String,
    pub start: String,
    pub hostname: String,
    pub comment: String,
    pub original_size: i64,
    pub deduplicated_size: i64,
    pub matched: Option<bool>,
    pub client_hostname: Option<String>,
}

struct ReportRow {
    started_at: DateTime<Utc>,
    matched: bool,
    client_hostname: String,
}

async fn fetch_report_rows(pool: &PgPool, repo_id: i64) -> Result<Vec<ReportRow>, ApiError> {
    let rows = sqlx::query_as::<_, (DateTime<Utc>, bool, String)>(
        r"
        SELECT br.started_at, br.matched, c.hostname AS client_hostname
        FROM backup_reports br
        JOIN clients c ON c.id = br.client_id
        WHERE br.repo_id = $1
        ",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|(started_at, matched, client_hostname)| ReportRow {
            started_at,
            matched,
            client_hostname,
        })
        .collect())
}

fn parse_borg_timestamp(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
        .map(|dt| dt.and_utc())
}

fn find_matching_report<'a>(
    archive_start: &str,
    reports: &'a [ReportRow],
) -> Option<&'a ReportRow> {
    let archive_ts = parse_borg_timestamp(archive_start)?;
    reports
        .iter()
        .filter(|r| {
            let diff = (archive_ts - r.started_at).num_seconds().abs();
            diff <= 5
        })
        .min_by_key(|r| (archive_ts - r.started_at).num_seconds().abs())
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ArchiveInfo {
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub nfiles: i64,
    pub duration: f64,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ContentEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub path: String,
    pub size: i64,
    pub mtime: String,
    pub mode: String,
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
        (status = 200, description = "List of archives", body = Vec<ArchiveEntry>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
pub async fn list_archives(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(repo_id): AxumPath<i64>,
) -> Result<Json<Vec<ArchiveEntry>>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let output = Command::new(borg_binary())
        .arg("info")
        .arg("--json")
        .arg("--glob-archives")
        .arg("*")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&borg_repo)
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg output: {e}")))?;

    let archives = json_output["archives"]
        .as_array()
        .map_or_else(Vec::new, |arr| {
            arr.iter()
                .map(|a| {
                    let stats = &a["stats"];
                    ArchiveEntry {
                        name: a["name"].as_str().unwrap_or("").to_string(),
                        start: ensure_utc_suffix(a["start"].as_str().unwrap_or("")),
                        hostname: a["hostname"].as_str().unwrap_or("").to_string(),
                        comment: a["comment"].as_str().unwrap_or("").to_string(),
                        original_size: stats["original_size"].as_i64().unwrap_or(0),
                        deduplicated_size: stats["deduplicated_size"].as_i64().unwrap_or(0),
                        matched: None,
                        client_hostname: None,
                    }
                })
                .collect()
        });

    let reports = fetch_report_rows(&state.pool, repo_id).await?;

    let archives = archives
        .into_iter()
        .map(|mut entry| {
            if let Some(report) = find_matching_report(&entry.start, &reports) {
                entry.matched = Some(report.matched);
                entry.client_hostname = Some(report.client_hostname.clone());
            }
            entry
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
        (status = 200, description = "Archive info", body = ArchiveInfo),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
pub async fn archive_info(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<ArchiveInfo>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let output = Command::new(borg_binary())
        .arg("info")
        .arg("--json")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&repo_archive)
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let json_output: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg output: {e}")))?;

    let archives = &json_output["archives"];
    let archive = archives
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| ApiError::NotFound(format!("archive '{archive_name}' not found")))?;

    let stats = &archive["stats"];

    let info = ArchiveInfo {
        original_size: stats["original_size"].as_i64().unwrap_or(0),
        compressed_size: stats["compressed_size"].as_i64().unwrap_or(0),
        deduplicated_size: stats["deduplicated_size"].as_i64().unwrap_or(0),
        nfiles: stats["nfiles"].as_i64().unwrap_or(0),
        duration: archive["duration"].as_f64().unwrap_or(0.0),
        start: ensure_utc_suffix(archive["start"].as_str().unwrap_or("")),
        end: ensure_utc_suffix(archive["end"].as_str().unwrap_or("")),
    };

    Ok(Json(info))
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
        (status = 200, description = "Directory contents", body = Vec<ContentEntry>),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 502, description = "Borg command failed"),
    )
)]
pub async fn list_contents(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<ContentsQuery>,
) -> Result<Json<Vec<ContentEntry>>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let path = query.path.as_deref();
    let limit = query.limit.unwrap_or(100);

    if let Some(p) = path {
        validate_path(p)?;
    }

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let repo_archive = format!("{borg_repo}::{archive_name}");

    let mut cmd = Command::new(borg_binary());
    cmd.arg("list")
        .arg("--json-lines")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&repo_archive);
    if let Some(p) = path {
        cmd.arg(p);
    }
    let output = cmd
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(1);
        return Err(classify_borg_error(code, &stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<ContentEntry> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .take(limit)
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .inspect_err(|e| {
                    tracing::trace!(error = %e, "skipping unparseable borg output line");
                })
                .ok()?;
            Some(ContentEntry {
                entry_type: v["type"].as_str().unwrap_or("").to_string(),
                path: v["path"].as_str().unwrap_or("").to_string(),
                size: v["size"].as_i64().unwrap_or(0),
                mtime: v["mtime"].as_str().unwrap_or("").to_string(),
                mode: v["mode"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();

    Ok(Json(entries))
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

    let mut child = Command::new(borg_binary())
        .arg("extract")
        .arg("--stdout")
        .arg("--lock-wait")
        .arg(LOCK_WAIT_SECS)
        .arg(&repo_archive)
        .arg(&query.path)
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdout".to_string()))?;

    let basename = Path::new(&query.path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    let content_type = content_type_for_extension(basename);
    let disposition = format!("attachment; filename=\"{basename}\"");

    let stream = ReaderStream::new(stdout);
    let body = Body::from_stream(stream);

    tokio::spawn(async move {
        tokio::time::sleep(EXTRACT_TIMEOUT).await;
        let _kill_result = child.kill().await;
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
}
