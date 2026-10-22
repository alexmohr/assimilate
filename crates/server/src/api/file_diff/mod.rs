// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Line-by-line comparison of one file between two archives, or between
//! an archive and the file as it is on its host now.

mod compute;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use serde::Deserialize;
use shared::{
    audit::AuditEvent,
    live_file::{LiveFile, LiveReadRefusal},
    protocol::ServerToAgent,
    responses::{FileDiffResponse, FileDiffSide},
};
use tokio::sync::oneshot;
use uuid::Uuid;

use self::compute::{DiffScope, diff_text};
use super::{auth::AuthUser, permissions::check_repo_permission};
use crate::{
    AppState,
    api::{
        archives::{get_repo_env, stream_extract, validate_extract_path},
        file_history::archive_owner,
    },
    borg::Borg,
    db,
    error::ApiError,
};

/// The largest version of a file, in bytes, the server reads to compare. Both
/// versions are held in memory and their diff is returned as JSON, so this is
/// sized for configuration files and scripts, not data.
const FILE_DIFF_MAX_BYTES: usize = 1024 * 1024;

/// Query parameters for comparing one file between two archives.
#[derive(Debug, Deserialize)]
pub struct FileDiffQuery {
    /// Path of the file within both archives.
    pub path: String,
    /// The archive to compare against: the base, whose lines the diff removes.
    pub against: String,
    /// Repository `against` is in, when it is not the request path's.
    pub against_repo_id: Option<i64>,
    /// Return every line instead of only the changes and their context.
    #[serde(default)]
    pub whole_file: bool,
}

/// How long the server waits for an agent to read a file.
const LIVE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Query parameters for comparing one file in an archive with the file as it
/// is on its host now.
#[derive(Debug, Deserialize)]
pub struct LiveFileDiffQuery {
    /// Path of the file within the archive.
    pub path: String,
    /// Return every line instead of only the changes and their context.
    #[serde(default)]
    pub whole_file: bool,
}

/// What reading one version of the file produced.
enum Version {
    Text(String),
    Binary,
    TooLarge,
}

impl From<Vec<u8>> for Version {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.contains(&0) {
            return Self::Binary;
        }
        String::from_utf8(bytes).map_or(Self::Binary, Self::Text)
    }
}

impl Version {
    /// The text of this version, or the response that explains why there is none.
    fn into_text(self, side: FileDiffSide) -> Result<String, FileDiffResponse> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Binary => Err(FileDiffResponse::Binary { side }),
            Self::TooLarge => Err(FileDiffResponse::TooLarge {
                side,
                limit_bytes: FILE_DIFF_MAX_BYTES,
            }),
        }
    }
}

/// Diffs two versions line by line off the async executor, or explains
/// why one of them cannot be compared.
async fn compare(
    base: Version,
    target: Version,
    whole_file: bool,
) -> Result<FileDiffResponse, ApiError> {
    let texts = base.into_text(FileDiffSide::Base).and_then(|base| {
        target
            .into_text(FileDiffSide::Target)
            .map(|target| (base, target))
    });
    match texts {
        Ok((base, target)) => {
            let scope = DiffScope::from(whole_file);
            tokio::task::spawn_blocking(move || diff_text(&base, &target, scope))
                .await
                .map_err(|e| ApiError::Internal(format!("file diff task failed: {e}")))
        }
        Err(response) => Ok(response),
    }
}

async fn read_version(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
    path: &str,
) -> Result<Version, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let contents = stream_extract(
        &Borg::new().with_registry(state.task_registry.clone()),
        &repo_archive,
        path,
        &env,
        &state.task_registry,
    )?
    .read_capped(FILE_DIFF_MAX_BYTES)
    .await?;
    Ok(contents.map_or(Version::TooLarge, Version::from))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/file-diff",
    tag = "Archives",
    operation_id = "diffArchiveFile",
    params(
        ("repo_id" = i64, Path, description = "Repository of the target archive"),
        ("archive_name" = String, Path,
            description = "Target archive: the version the diff adds lines to"),
        ("path" = String, Query, description = "File path within both archives"),
        ("against" = String, Query,
            description = "Base archive: the version the diff removes lines from"),
        ("against_repo_id" = Option<i64>, Query,
            description = "Repository of the base archive (default: the target's)"),
        ("whole_file" = Option<bool>, Query,
            description = "Return every line, not only the changes (default: false)"),
    ),
    responses(
        (status = 200, description = "The comparison, or why the file cannot be compared",
            body = FileDiffResponse),
        (status = 400, description = "Invalid path, or the file compared with itself"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive or file not found"),
        (status = 409, description = "Repository is locked by another operation"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Compare one file between two archives, line by line.
///
/// Both versions are read with `borg extract`, so the caller needs extract
/// permission on both repositories, the same as downloading the file.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid path or a file compared
/// with itself, the permission errors of either repository, and the
/// classified borg error if reading a version failed.
pub async fn diff_file(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<FileDiffQuery>,
) -> Result<Json<FileDiffResponse>, ApiError> {
    let against_repo_id = query.against_repo_id.unwrap_or(repo_id);
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;
    if against_repo_id != repo_id {
        check_repo_permission(&state.pool, &auth, against_repo_id, |p| p.can_extract).await?;
    }
    validate_extract_path(&query.path)?;
    if against_repo_id == repo_id && query.against == archive_name {
        return Err(ApiError::BadRequest(
            "a file cannot be compared with itself; pick another archive".to_string(),
        ));
    }

    let (base, target) = tokio::try_join!(
        read_version(&state, against_repo_id, &query.against, &query.path),
        read_version(&state, repo_id, &archive_name, &query.path),
    )?;

    compare(base, target, query.whole_file).await.map(Json)
}

/// Reads `path` on `agent_id`'s host through its agent, or says why it
/// could not be read.
async fn read_live(
    state: &AppState,
    agent_id: i64,
    path: &str,
) -> Result<Version, LiveReadRefusal> {
    if !state.registry.is_connected(agent_id).await {
        return Err(LiveReadRefusal::AgentOffline);
    }
    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    state
        .pending_live_reads
        .insert(request_id.clone(), agent_id, tx)
        .await;
    let request = ServerToAgent::ReadLiveFile {
        request_id: request_id.clone(),
        path: path.to_owned(),
    };
    if state.registry.send_to(agent_id, request).await.is_err() {
        state.pending_live_reads.remove(&request_id).await;
        return Err(LiveReadRefusal::AgentOffline);
    }
    let answer = tokio::time::timeout(LIVE_READ_TIMEOUT, rx).await;
    state.pending_live_reads.remove(&request_id).await;
    match answer {
        Ok(Ok(LiveFile::Text { text, .. })) => Ok(Version::Text(text.0)),
        Ok(Ok(LiveFile::Binary)) => Ok(Version::Binary),
        Ok(Ok(LiveFile::TooLarge { .. })) => Ok(Version::TooLarge),
        Ok(Ok(LiveFile::Refused { reason })) => Err(reason),
        Ok(Err(_)) => Err(LiveReadRefusal::AgentOffline),
        Err(_) => Err(LiveReadRefusal::TimedOut),
    }
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/file-diff-live",
    tag = "Archives",
    operation_id = "diffArchiveFileWithLive",
    params(
        ("repo_id" = i64, Path, description = "Repository of the archive"),
        ("archive_name" = String, Path,
            description = "Base archive: the version the diff removes lines from"),
        ("path" = String, Query, description = "File path within the archive"),
        ("whole_file" = Option<bool>, Query,
            description = "Return every line, not only the changes (default: false)"),
    ),
    responses(
        (status = 200,
            description = "The comparison, or why the file cannot be compared or read now",
            body = FileDiffResponse),
        (status = 400, description = "Invalid path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive or file not found"),
        (status = 409, description = "Repository is locked by another operation"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Compare one file in an archive with the file as it is now on the host
/// that wrote the archive. The host's agent reads it over the encrypted
/// agent channel, only from inside its configured backup sources; the
/// server compares it in memory and keeps nothing.
///
/// Needs extract permission on the repository, the same as downloading the
/// file. Every read is recorded in the audit log.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an invalid path, the repository's
/// permission errors, [`ApiError::NotFound`] for an archive no backup
/// recorded, and the classified borg error if reading the archived version
/// failed.
pub async fn diff_file_live(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<LiveFileDiffQuery>,
) -> Result<Json<FileDiffResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;
    validate_extract_path(&query.path)?;
    let owner = archive_owner(&state.pool, repo_id, &archive_name).await?;

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::ReadLiveFile {
                hostname: owner.hostname.clone(),
                path: query.path.clone(),
            },
            target_type: Some("agent"),
            target_id: Some(owner.agent_id),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    let (base, live) = tokio::join!(
        read_version(&state, repo_id, &archive_name, &query.path),
        read_live(&state, owner.agent_id, &query.path),
    );
    let base = base?;
    let live = match live {
        Ok(live) => live,
        Err(reason) => return Ok(Json(FileDiffResponse::LiveUnavailable { reason })),
    };

    compare(base, live, query.whole_file).await.map(Json)
}

#[cfg(test)]
mod tests;
