// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use shared::{audit::AuditEvent, restore::RestoreDestination, types::RestoreRun};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{
    archives::{get_repo_env, stream_export_tar_lz4, validate_path},
    auth::{AuthUser, RequireAdmin},
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, db, error::ApiError, restore_runs};

/// Request payload for downloading files from an archive.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadFilesRequest {
    /// Paths within the archive to include in the download
    pub paths: Vec<String>,
}

/// Most archive paths one download or restore may name. A ticked directory
/// is one path however much it holds, so even a large selection stays far
/// below this, and borg's command line stays well within the kernel's
/// argument limit.
pub const MAX_REQUEST_PATHS: usize = 1000;

fn check_path_count(paths: &[String]) -> Result<(), ApiError> {
    if paths.len() > MAX_REQUEST_PATHS {
        return Err(ApiError::BadRequest(format!(
            "at most {MAX_REQUEST_PATHS} paths per request"
        )));
    }
    Ok(())
}

/// `paths` with exact repeats dropped, first occurrence kept.
fn dedup_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
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
        (status = 400, description = "Invalid, empty or too many paths"),
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
    check_path_count(&body.paths)?;

    for path in &body.paths {
        validate_path(path)?;
    }
    let paths = dedup_paths(body.paths);

    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_extract).await?;

    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    let repo_archive = format!("{borg_repo}::{archive_name}");

    let export = stream_export_tar_lz4(
        &Borg::new().with_registry(state.task_registry.clone()),
        &repo_archive,
        &paths,
        &env,
        &state.task_registry,
    )?;
    let filename = format!("{archive_name}.tar.lz4");

    // Record the attempt before waiting on borg, so a download that fails early (wrong
    // passphrase, missing archive or path) is still audited.
    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::DownloadFiles {
                archive: archive_name.clone(),
                path_count: Some(paths.len()),
                paths,
            },
            target_type: Some("archive"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    let body_stream = export.into_body().await?;
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
    /// Paths within the archive, restored by one `borg extract` run. An
    /// empty list restores the whole archive.
    pub paths: Vec<String>,
    /// Absolute directory on the agent that every path is restored under,
    /// keeping its archive structure. Omitted, or `/`, restores each path to
    /// its original location.
    #[serde(default = "original_location")]
    pub target_path: String,
    /// Hostname of the agent to restore to.
    pub hostname: String,
    /// Domain of the target agent, required if `hostname` is shared by
    /// multiple agents.
    pub domain: Option<String>,
}

fn original_location() -> String {
    RestoreDestination::original().to_string()
}

/// The archive paths a restore asks for, as borg stores them, each checked
/// like a download path. One leading `/` is dropped first: the restore
/// wizard takes paths as they were on the host, and borg drops it itself.
fn archive_paths(paths: &[String]) -> Result<Vec<String>, ApiError> {
    check_path_count(paths)?;
    paths
        .iter()
        .map(|path| {
            let path = path.strip_prefix('/').unwrap_or(path);
            validate_path(path)?;
            Ok(path.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(dedup_paths)
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
        (status = 202, description = "Restore recorded; runs in background", body = RestoreRun),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Agent or repository not found"),
    )
)]
/// Restore selected files from an archive to the agent filesystem.
///
/// Returns as soon as the restore is recorded. It is handed to the agent at
/// once if the agent is connected, otherwise when it next connects; follow
/// it with `GET /api/restores/{id}` or the UI WebSocket's
/// `RestoreRunChanged` events.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the agent or repository does not exist
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn restore_files(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<RestoreFilesRequest>,
) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
    let destination = RestoreDestination::try_from(body.target_path.as_str())
        .map_err(|e| ApiError::BadRequest(format!("invalid target_path: {e}")))?;
    let paths = archive_paths(&body.paths)?;

    let agent =
        db::get_agent_by_hostname(&state.pool, &body.hostname, body.domain.as_deref()).await?;
    db::get_repo_name(&state.pool, repo_id).await?;

    let id = db::restore_runs::insert_restore_run(
        &state.pool,
        &db::restore_runs::NewRestoreRun {
            agent_id: agent.id,
            repo_id,
            archive_name: &archive_name,
            paths: &paths,
            target_path: destination.as_str(),
            requested_by: &admin.username,
        },
    )
    .await?;

    let event = AuditEvent::RestoreFiles {
        archive: archive_name.clone(),
        path_count: Some(paths.len()),
        paths: paths.clone(),
        target_path: destination.to_string(),
        hostname: body.hostname.clone(),
    };
    audit_and_dispatch(&state, &admin, event, repo_id, id, agent.id).await
}

/// Audits restore `id`, just recorded for archive in `repo_id`, then hands
/// it to its agent and answers with the run. A failed audit write is only
/// logged: the restore was asked for and recorded either way.
pub(super) async fn audit_and_dispatch(
    state: &AppState,
    admin: &AuthUser,
    event: AuditEvent,
    repo_id: i64,
    id: Uuid,
    agent_id: i64,
) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            event,
            target_type: Some("archive"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    let run = restore_runs::dispatch(state, id, agent_id)
        .await?
        .ok_or_else(|| not_found(id))?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

/// Query parameters for listing restores.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListRestoreRunsQuery {
    /// Maximum number of restores to return, newest first (default 100,
    /// at most 500).
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/restores",
    tag = "Archives",
    operation_id = "listRestoreRuns",
    params(ListRestoreRunsQuery),
    responses(
        (status = 200, description = "Recent restores, newest first", body = Vec<RestoreRun>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
    )
)]
/// List recent restores onto agents, newest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_restore_runs(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Query(query): Query<ListRestoreRunsQuery>,
) -> Result<Json<Vec<RestoreRun>>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(
        db::restore_runs::list_restore_runs(&state.pool, limit).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/restores/{id}",
    tag = "Archives",
    operation_id = "getRestoreRun",
    params(("id" = String, Path, description = "Restore ID")),
    responses(
        (status = 200, description = "The restore", body = RestoreRun),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "No such restore"),
    )
)]
/// Get one restore onto an agent.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such restore.
pub async fn get_restore_run(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<RestoreRun>, ApiError> {
    let id = parse_restore_id(&id)?;
    Ok(Json(restore_run_or_not_found(&state, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/restores/{id}/cancel",
    tag = "Archives",
    operation_id = "cancelRestoreRun",
    params(("id" = String, Path, description = "Restore ID")),
    responses(
        (status = 200, description = "The cancelled restore", body = RestoreRun),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "No such restore"),
        (status = 409, description = "The restore was already handed to the agent or is over"),
    )
)]
/// Cancel a restore still waiting for its agent to connect.
///
/// A restore already handed to the agent runs to its end.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: there is no such restore
/// - [`ApiError::Conflict`]: the restore is no longer waiting
pub async fn cancel_restore_run(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<RestoreRun>, ApiError> {
    let id = parse_restore_id(&id)?;
    if !db::restore_runs::cancel_restore_run(&state.pool, id).await? {
        let run = restore_run_or_not_found(&state, id).await?;
        return Err(ApiError::Conflict(format!(
            "the restore is {} and can no longer be cancelled",
            run.status
        )));
    }
    let run = restore_runs::broadcast(&state, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    Ok(Json(run))
}

/// An id that is not a UUID names no restore.
fn parse_restore_id(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::NotFound(format!("restore {id} not found")))
}

async fn restore_run_or_not_found(state: &AppState, id: Uuid) -> Result<RestoreRun, ApiError> {
    db::restore_runs::get_restore_run(&state.pool, id)
        .await?
        .ok_or_else(|| not_found(id))
}

pub(super) fn not_found(id: Uuid) -> ApiError {
    ApiError::NotFound(format!("restore {id} not found"))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    /// A test state, a user named `username` and a repository that user may
    /// extract from.
    async fn extractor_on_a_repo(
        pool: &PgPool,
        username: &str,
    ) -> (AppState, db::UserRow, db::RepoRow) {
        let state = crate::test_support::build_test_state(pool.clone(), b"restore-test-secret-key");
        let user = db::insert_user(pool, username, "hash").await.unwrap();
        let passphrase =
            shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
        let repo = db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: "repo",
                repo_path: "/backups/repo",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::upsert_repo_permission(
            pool,
            &db::UpsertRepoPermissionParams {
                user_id: user.id,
                repo_id: repo.id,
                can_view: true,
                can_backup: false,
                can_modify_schedules: false,
                can_extract: true,
                can_delete: false,
            },
        )
        .await
        .unwrap();
        (state, user, repo)
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_download_that_borg_rejects_is_still_audited(pool: PgPool) {
        let (state, user, repo) = extractor_on_a_repo(&pool, "downloader").await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = crate::test_support::install_fake_borg(
            "#!/bin/sh\necho 'Archive repo::nightly does not exist' >&2\nexit 2\n",
        )
        .await;

        let result = download_files(
            State(state),
            AuthUser {
                user_id: user.id,
                username: "downloader".to_string(),
                session_id: None,
            },
            AxumPath((repo.id, "nightly".to_string())),
            Json(DownloadFilesRequest {
                paths: vec!["etc/hosts".to_string()],
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::NotFound(_))));
        let (entries, _) = db::audit::list_audit_entries(
            &pool,
            &db::audit::AuditEntryFilters {
                page: 1,
                per_page: 10,
                filter_user_id: Some(user.id),
                filter_action: None,
                filter_target_type: None,
                filter_from: None,
                filter_to: None,
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(
                entries.as_slice(),
                [entry] if matches!(
                    &entry.event,
                    AuditEvent::DownloadFiles { archive, paths, .. }
                        if archive == "nightly" && paths == &["etc/hosts".to_string()]
                )
            ),
            "the failed download attempt must be audited, got {entries:?}"
        );
    }

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

        assert_eq!(request.paths.len(), 0);
    }

    struct RestoreFixture {
        state: AppState,
        admin: AuthUser,
        repo_id: i64,
    }

    async fn restore_fixture(pool: &PgPool) -> RestoreFixture {
        let state = crate::test_support::build_test_state(pool.clone(), b"restore-test-secret-key");
        let user = db::insert_user(pool, "restorer", "hash").await.unwrap();
        db::insert_agent(pool, "web-01", None, "hash", None, None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: "repo",
                repo_path: "/backups/repo",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"encrypted",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        RestoreFixture {
            state,
            admin: AuthUser {
                user_id: user.id,
                username: "restorer".to_string(),
                session_id: None,
            },
            repo_id: repo.id,
        }
    }

    impl RestoreFixture {
        async fn restore(
            &self,
            repo_id: i64,
            target_path: &str,
        ) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
            self.restore_paths(repo_id, target_path, &["etc/hosts"])
                .await
        }

        async fn restore_paths(
            &self,
            repo_id: i64,
            target_path: &str,
            paths: &[&str],
        ) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
            restore_files(
                State(self.state.clone()),
                RequireAdmin(self.admin.clone()),
                AxumPath((repo_id, "nightly".to_string())),
                Json(RestoreFilesRequest {
                    paths: paths.iter().map(|p| (*p).to_owned()).collect(),
                    target_path: target_path.to_string(),
                    hostname: "web-01".to_string(),
                    domain: None,
                }),
            )
            .await
        }
    }

    /// The request returns once the restore is recorded, whether or not
    /// the agent is there to take it: an offline agent gets it when it
    /// reconnects.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_for_an_offline_agent_is_accepted_and_waits(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let (status, Json(run)) = fx.restore(fx.repo_id, "/restore").await.unwrap();

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(run.status, shared::types::RestoreRunStatus::Pending);
        assert_eq!(run.hostname, "web-01");
        assert_eq!(run.requested_by, "restorer");
        assert!(matches!(
            crate::test_support::audit_events(&pool).await.as_slice(),
            [AuditEvent::RestoreFiles { archive, target_path, .. }]
                if archive == "nightly" && target_path == "/restore"
        ));
    }

    /// Paths reach `borg extract` on the agent, so they are checked like
    /// download paths. A leading `/`, as the restore wizard sends it, is
    /// dropped the way borg drops it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn restore_paths_are_checked_like_download_paths(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let traversal = fx
            .restore_paths(fx.repo_id, "/restore", &["etc/../../root"])
            .await;
        let option = fx.restore_paths(fx.repo_id, "/restore", &["--help"]).await;
        let root = fx.restore_paths(fx.repo_id, "/restore", &["/"]).await;
        let (_, Json(absolute)) = fx
            .restore_paths(fx.repo_id, "/restore", &["/etc/hosts", "var/www"])
            .await
            .unwrap();

        assert!(matches!(traversal, Err(ApiError::BadRequest(_))));
        assert!(matches!(option, Err(ApiError::BadRequest(_))));
        assert!(matches!(root, Err(ApiError::BadRequest(_))));
        assert_eq!(absolute.paths, vec!["etc/hosts", "var/www"]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_without_a_target_is_rejected(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let result = fx.restore(fx.repo_id, "").await;

        assert!(matches!(result, Err(ApiError::BadRequest(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_from_an_unknown_repository_is_not_found(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let result = fx.restore(987_654, "/restore").await;

        assert!(matches!(result, Err(ApiError::NotFound(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_recorded_restore_can_be_listed_and_fetched(pool: PgPool) {
        let fx = restore_fixture(&pool).await;
        let (_, Json(run)) = fx.restore(fx.repo_id, "/restore").await.unwrap();

        let Json(listed) = list_restore_runs(
            State(fx.state.clone()),
            RequireAdmin(fx.admin.clone()),
            Query(ListRestoreRunsQuery { limit: None }),
        )
        .await
        .unwrap();
        let Json(fetched) = get_restore_run(
            State(fx.state.clone()),
            RequireAdmin(fx.admin.clone()),
            AxumPath(run.id.clone()),
        )
        .await
        .unwrap();
        let not_an_id = get_restore_run(
            State(fx.state.clone()),
            RequireAdmin(fx.admin.clone()),
            AxumPath("17".to_string()),
        )
        .await;
        let unknown = get_restore_run(
            State(fx.state),
            RequireAdmin(fx.admin),
            AxumPath(Uuid::new_v4().to_string()),
        )
        .await;

        assert_eq!(listed, vec![run.clone()]);
        assert_eq!(fetched, run);
        assert!(matches!(not_an_id, Err(ApiError::NotFound(_))));
        assert!(matches!(unknown, Err(ApiError::NotFound(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_waiting_restore_can_be_cancelled_once(pool: PgPool) {
        let fx = restore_fixture(&pool).await;
        let (_, Json(run)) = fx.restore(fx.repo_id, "/restore").await.unwrap();

        let Json(cancelled) = cancel_restore_run(
            State(fx.state.clone()),
            RequireAdmin(fx.admin.clone()),
            AxumPath(run.id.clone()),
        )
        .await
        .unwrap();
        let again =
            cancel_restore_run(State(fx.state), RequireAdmin(fx.admin), AxumPath(run.id)).await;

        assert_eq!(cancelled.status, shared::types::RestoreRunStatus::Cancelled);
        assert!(matches!(again, Err(ApiError::Conflict(_))));
    }

    #[test]
    fn a_restore_request_without_a_target_restores_to_the_original_locations() {
        let request: RestoreFilesRequest = serde_json::from_value(serde_json::json!({
            "paths": ["etc/hosts"],
            "hostname": "web-server-01",
        }))
        .unwrap();

        assert_eq!(request.target_path, "/");
    }

    /// Several paths restore under one destination in one run: one restore,
    /// one audit entry naming how many paths and where.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn several_paths_restore_under_one_destination(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let (status, Json(run)) = fx
            .restore_paths(
                fx.repo_id,
                "/srv//restore/",
                &["etc/nginx/nginx.conf", "var/www", "etc/nginx/nginx.conf"],
            )
            .await
            .unwrap();

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(run.paths, vec!["etc/nginx/nginx.conf", "var/www"]);
        assert_eq!(run.target_path.as_deref(), Some("/srv/restore"));
        assert!(matches!(
            crate::test_support::audit_events(&pool).await.as_slice(),
            [AuditEvent::RestoreFiles { paths, path_count: Some(2), target_path, .. }]
                if paths.len() == 2 && target_path == "/srv/restore"
        ));
    }

    /// The destination names a directory on the host's own filesystem, so
    /// anything but a plain absolute path is refused before it is recorded.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unsafe_destination_is_refused(pool: PgPool) {
        let fx = restore_fixture(&pool).await;

        let relative = fx.restore(fx.repo_id, "srv/restore").await;
        let traversal = fx.restore(fx.repo_id, "/srv/../etc").await;
        let control = fx.restore(fx.repo_id, "/srv/\nrestore").await;

        assert!(matches!(relative, Err(ApiError::BadRequest(_))));
        assert!(matches!(traversal, Err(ApiError::BadRequest(_))));
        assert!(matches!(control, Err(ApiError::BadRequest(_))));
        assert_eq!(crate::test_support::audit_events(&pool).await, []);
        assert_eq!(
            db::restore_runs::list_restore_runs(&pool, 10)
                .await
                .unwrap(),
            []
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_of_too_many_paths_is_refused(pool: PgPool) {
        let fx = restore_fixture(&pool).await;
        let paths: Vec<String> = (0..=MAX_REQUEST_PATHS).map(|i| format!("f{i}")).collect();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();

        let result = fx.restore_paths(fx.repo_id, "/restore", &paths).await;

        assert!(matches!(result, Err(ApiError::BadRequest(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_download_of_too_many_paths_is_refused(pool: PgPool) {
        let (state, user, repo) = extractor_on_a_repo(&pool, "bulk-downloader").await;

        let result = download_files(
            State(state),
            AuthUser {
                user_id: user.id,
                username: "bulk-downloader".to_string(),
                session_id: None,
            },
            AxumPath((repo.id, "nightly".to_string())),
            Json(DownloadFilesRequest {
                paths: (0..=MAX_REQUEST_PATHS).map(|i| format!("f{i}")).collect(),
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest(_))));
        assert_eq!(crate::test_support::audit_events(&pool).await, []);
    }

    /// A download of several paths is one borg export of each distinct
    /// path, audited with how many there were.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_download_drops_repeated_paths_and_records_the_count(pool: PgPool) {
        let (state, user, repo) = extractor_on_a_repo(&pool, "basket-downloader").await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = crate::test_support::install_fake_borg(
            "#!/bin/sh\necho 'Archive repo::nightly does not exist' >&2\nexit 2\n",
        )
        .await;

        let result = download_files(
            State(state),
            AuthUser {
                user_id: user.id,
                username: "basket-downloader".to_string(),
                session_id: None,
            },
            AxumPath((repo.id, "nightly".to_string())),
            Json(DownloadFilesRequest {
                paths: ["etc/hosts", "var/www", "etc/hosts"]
                    .map(str::to_owned)
                    .to_vec(),
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::NotFound(_))));
        assert!(matches!(
            crate::test_support::audit_events(&pool).await.as_slice(),
            [AuditEvent::DownloadFiles { archive, paths, path_count: Some(2) }]
                if archive == "nightly" && paths == &["etc/hosts", "var/www"]
        ));
    }
}
