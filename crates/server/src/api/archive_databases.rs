// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Database dumps inside an archive: listing them, as the archive's own
//! manifests describe them, and restoring one into the database server its
//! source runs on now.
//!
//! The agent writes a manifest (`databases.json`) next to the dumps of every
//! run, in `<dump dir>/schedule-<id>/` or `<dump dir>/manual/`. The server
//! reads those manifests from the archive rather than its own tables, so an
//! archive explains itself, and a restore request names only the dump's path:
//! what to restore is always taken from the manifest, never from the client.

use std::{path::Component, str::FromStr};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    audit::AuditEvent,
    database::{
        DatabaseManifest, DatabaseRestore, DatabaseRestoreTarget, DatabaseRuntime,
        DatabaseRuntimes, MANIFEST_FILE_NAME, MANIFEST_VERSION, MAX_DATABASE_NAME_BYTES,
        is_valid_new_database_name,
    },
    responses::{ArchiveDatabaseDump, ArchiveDatabasesResponse},
    types::RestoreRun,
};
use utoipa::ToSchema;

use super::{
    archives::{borg_list_raw_entries, get_repo_env, stream_extract, validate_path},
    auth::{AuthUser, RequireAdmin},
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, db, error::ApiError};

/// Largest manifest read from an archive. A real one lists a few dumps in a
/// few kilobytes; the cap keeps a hostile archive from making the server
/// buffer an arbitrary amount.
const MANIFEST_READ_LIMIT: usize = 1024 * 1024;

/// The directory a run's dumps and manifest sit in, below the dump directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DumpRunDir {
    /// `schedule-<id>`: a scheduled backup's run.
    Schedule,
    /// `manual`: a backup started by hand.
    Manual,
}

/// A directory name that is no run's.
#[derive(Debug)]
struct NotADumpRunDir;

impl FromStr for DumpRunDir {
    type Err = NotADumpRunDir;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "manual" {
            return Ok(Self::Manual);
        }
        s.strip_prefix("schedule-")
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .map(|_| Self::Schedule)
            .ok_or(NotADumpRunDir)
    }
}

/// The borg patterns that keep only manifests in a listing: a
/// `databases.json` directly inside a `schedule-<id>` or `manual` directory.
fn manifest_patterns() -> Vec<String> {
    let file_name = MANIFEST_FILE_NAME.replace('.', "\\.");
    vec![
        format!("+re:(^|/)(schedule-[0-9]+|manual)/{file_name}$"),
        "-sh:**".to_owned(),
    ]
}

/// The directory of the manifest at `path` in an archive, or `None` when
/// `path` is not a manifest's.
fn manifest_dir(path: &str) -> Option<&str> {
    let dir = path.strip_suffix(MANIFEST_FILE_NAME)?.strip_suffix('/')?;
    let run = dir.rsplit_once('/').map_or(dir, |(_, run)| run);
    run.parse::<DumpRunDir>().ok()?;
    validate_path(dir).ok()?;
    Some(dir)
}

/// Whether a manifest entry's `path` stays below the manifest's directory:
/// relative, of plain non-empty segments, and fit to hand to `borg extract`.
fn is_safe_entry_path(path: &str) -> bool {
    // Segment by segment, since `Path::components` would quietly drop the
    // empty and `.` ones.
    validate_path(path).is_ok()
        && path.split('/').all(|segment| {
            let mut components = std::path::Path::new(segment).components();
            matches!(
                (components.next(), components.next()),
                (Some(Component::Normal(_)), None)
            )
        })
}

/// A dump's path in the archive, from its manifest's directory and the
/// manifest entry's path relative to it.
fn dump_archive_path(manifest_dir: &str, entry_path: &str) -> String {
    format!("{manifest_dir}/{entry_path}")
}

/// The dumps the manifest at `manifest_path`, holding `contents`, lists.
/// A manifest that does not parse, or comes from a newer build, lists none;
/// an entry whose path would leave the manifest's directory is left out.
fn dumps_in_manifest(manifest_path: &str, contents: &[u8]) -> Vec<ArchiveDatabaseDump> {
    let Some(dir) = manifest_dir(manifest_path) else {
        return Vec::new();
    };
    let manifest = match serde_json::from_slice::<DatabaseManifest>(contents) {
        Ok(manifest) => manifest,
        Err(e) => {
            tracing::warn!(manifest_path, error = %e, "skipping a database manifest that does not parse");
            return Vec::new();
        }
    };
    if manifest.version > MANIFEST_VERSION {
        tracing::warn!(
            manifest_path,
            version = manifest.version,
            "skipping a database manifest written by a newer agent"
        );
        return Vec::new();
    }
    manifest
        .dumps
        .into_iter()
        .filter(|entry| {
            let safe = is_safe_entry_path(&entry.path);
            if !safe {
                tracing::warn!(
                    manifest_path,
                    path = %entry.path,
                    "skipping a database dump whose path leaves its manifest's directory"
                );
            }
            safe
        })
        .map(|entry| ArchiveDatabaseDump {
            manifest_path: manifest_path.to_owned(),
            archive_path: dump_archive_path(dir, &entry.path),
            entry,
        })
        .collect()
}

/// Every dump the archive's manifests list: manifests in path order, each
/// one's dumps in the order it lists them.
async fn archive_dumps(
    state: &AppState,
    repo_id: i64,
    archive_name: &str,
) -> Result<Vec<ArchiveDatabaseDump>, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let entries = borg_list_raw_entries(
        &env,
        &borg_repo,
        archive_name,
        &manifest_patterns(),
        &state.task_registry,
    )
    .await?;
    let mut manifest_paths: Vec<String> = entries
        .into_iter()
        .map(|entry| entry.path)
        .filter(|path| manifest_dir(path).is_some())
        .collect();
    manifest_paths.sort();
    manifest_paths.dedup();

    let repo_archive = format!("{borg_repo}::{archive_name}");
    let borg = Borg::new().with_registry(state.task_registry.clone());
    let mut dumps = Vec::new();
    for manifest_path in manifest_paths {
        let contents = stream_extract(
            &borg,
            &repo_archive,
            &manifest_path,
            &env,
            &state.task_registry,
        )?
        .read_capped(MANIFEST_READ_LIMIT)
        .await?;
        let Some(contents) = contents else {
            tracing::warn!(
                manifest_path,
                limit = MANIFEST_READ_LIMIT,
                "skipping a database manifest larger than the limit"
            );
            continue;
        };
        dumps.extend(dumps_in_manifest(&manifest_path, &contents));
    }
    Ok(dumps)
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/databases",
    tag = "Archives",
    operation_id = "listArchiveDatabases",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "The database dumps the archive holds",
            body = ArchiveDatabasesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Archive or repository not found"),
        (status = 409, description = "Repository is locked by another operation"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// List the database dumps an archive holds.
///
/// Read from the manifests the agent stores next to the dumps, so the list
/// is what the archive itself says it holds. An archive without database
/// dumps lists none.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Forbidden`]: the user may not view the repository
/// - [`ApiError::NotFound`]: the repository or archive does not exist
/// - a borg-derived error if listing or reading the archive fails
pub async fn list_archive_databases(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
) -> Result<Json<ArchiveDatabasesResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let dumps = archive_dumps(&state, repo_id, &archive_name).await?;
    Ok(Json(ArchiveDatabasesResponse { dumps }))
}

/// Request payload for restoring a database dump from an archive.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestoreDatabaseRequest {
    /// The dump's path in the archive, as the archive's database listing
    /// gives it. What it holds and where it came from are read from the
    /// archive's manifest.
    pub archive_path: String,
    /// Where the dump is restored to: a new database, or the one it came
    /// from.
    pub target: DatabaseRestoreTarget,
    /// Hostname of the agent that restores it, the one whose database
    /// servers the dump's source runs on.
    pub hostname: String,
    /// Domain of that agent, required if `hostname` is shared by multiple
    /// agents.
    pub domain: Option<String>,
}

/// Whether the agent with `runtimes` turned on looks for databases in
/// `runtime`, and so can reach a source there.
const fn runtime_enabled(runtimes: &DatabaseRuntimes, runtime: DatabaseRuntime) -> bool {
    match runtime {
        DatabaseRuntime::Kubernetes => runtimes.kubernetes,
        DatabaseRuntime::Docker => runtimes.docker,
        DatabaseRuntime::Host => runtimes.host,
    }
}

/// Refuses a new database name `PostgreSQL` or a client could misread.
fn validate_target(target: &DatabaseRestoreTarget) -> Result<(), ApiError> {
    match target {
        DatabaseRestoreTarget::NewDatabase { name } if !is_valid_new_database_name(name) => {
            Err(ApiError::BadRequest(format!(
                "the new database name must be 1 to {MAX_DATABASE_NAME_BYTES} ASCII letters, \
                 digits, '_' or '-', and must not start with '-' or 'pg_'"
            )))
        }
        DatabaseRestoreTarget::NewDatabase { .. } | DatabaseRestoreTarget::Original => Ok(()),
    }
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/databases/restore",
    tag = "Archives",
    operation_id = "restoreDatabase",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = RestoreDatabaseRequest,
    responses(
        (status = 202, description = "Restore recorded; runs in background", body = RestoreRun),
        (status = 400, description = "Invalid request, or the agent does not reach the \
            dump's runtime"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Agent, repository, archive or dump not found"),
        (status = 409, description = "Repository is locked by another operation"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Restore a database dump from an archive into the database server its
/// source runs on.
///
/// The dump is looked up in the archive's manifests by its path; the
/// source, database and format come from there. Returns as soon as the
/// restore is recorded. It is handed to the agent at once if the agent is
/// connected, otherwise when it next connects; follow it with
/// `GET /api/restores/{id}` or the UI WebSocket's `RestoreRunChanged`
/// events.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid, or the agent does not
///   look for databases in the runtime the dump's source runs in
/// - [`ApiError::NotFound`]: the agent, repository, archive or dump does not
///   exist
/// - a borg-derived error if reading the archive fails
pub async fn restore_database(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<RestoreDatabaseRequest>,
) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
    validate_path(&body.archive_path)?;
    validate_target(&body.target)?;

    let agent =
        db::get_agent_by_hostname(&state.pool, &body.hostname, body.domain.as_deref()).await?;
    db::get_repo_name(&state.pool, repo_id).await?;

    let dump = archive_dumps(&state, repo_id, &archive_name)
        .await?
        .into_iter()
        .find(|dump| dump.archive_path == body.archive_path)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "archive '{archive_name}' holds no database dump at '{}'",
                body.archive_path
            ))
        })?;

    let settings = db::databases::get_agent_database_backup(&state.pool, agent.id).await?;
    let runtime = dump.entry.source.runtime;
    if !runtime_enabled(&settings.runtimes, runtime) {
        return Err(ApiError::BadRequest(format!(
            "agent '{}' does not look for databases in {runtime}; turn {runtime} on in its \
             database settings to restore this dump",
            body.hostname
        )));
    }

    let restore = DatabaseRestore {
        source: dump.entry.source,
        database: dump.entry.database,
        archive_path: dump.archive_path,
        format: dump.entry.format,
        target: body.target,
    };
    let id = db::restore_runs::insert_database_restore_run(
        &state.pool,
        &db::restore_runs::NewDatabaseRestoreRun {
            agent_id: agent.id,
            repo_id,
            archive_name: &archive_name,
            restore: &restore,
            requested_by: &admin.username,
        },
    )
    .await?;

    let event = AuditEvent::RestoreDatabase {
        archive: archive_name.clone(),
        archive_path: restore.archive_path.clone(),
        source: restore.source.clone(),
        database: restore.database.clone(),
        target: restore.target.clone(),
        hostname: body.hostname.clone(),
    };
    super::restore::audit_and_dispatch(&state, &admin, event, repo_id, id, agent.id).await
}

#[cfg(test)]
mod tests;
