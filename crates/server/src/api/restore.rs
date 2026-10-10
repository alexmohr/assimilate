// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use shared::{audit::AuditEvent, types::RestoreRun};
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

    let export = stream_export_tar_lz4(
        &Borg::new().with_registry(state.task_registry.clone()),
        &repo_archive,
        &body.paths,
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
                paths: body.paths.clone(),
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
    /// Paths within the archive. An empty list restores the whole archive.
    pub paths: Vec<String>,
    /// Target directory on the agent filesystem.
    pub target_path: String,
    /// Hostname of the agent to restore to.
    pub hostname: String,
    /// Domain of the target agent, required if `hostname` is shared by
    /// multiple agents.
    pub domain: Option<String>,
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
    if body.target_path.is_empty() {
        return Err(ApiError::BadRequest(
            "target_path must not be empty".to_owned(),
        ));
    }

    let agent =
        db::get_agent_by_hostname(&state.pool, &body.hostname, body.domain.as_deref()).await?;
    db::get_repo_name(&state.pool, repo_id).await?;

    let id = db::restore_runs::insert_restore_run(
        &state.pool,
        &db::restore_runs::NewRestoreRun {
            agent_id: agent.id,
            repo_id,
            archive_name: &archive_name,
            paths: &body.paths,
            target_path: &body.target_path,
            requested_by: &admin.username,
        },
    )
    .await?;

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            event: AuditEvent::RestoreFiles {
                archive: archive_name.clone(),
                paths: body.paths.clone(),
                target_path: body.target_path.clone(),
                hostname: body.hostname.clone(),
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

    let run = restore_runs::dispatch(&state, id, agent.id)
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

fn not_found(id: Uuid) -> ApiError {
    ApiError::NotFound(format!("restore {id} not found"))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_download_that_borg_rejects_is_still_audited(pool: PgPool) {
        let state = crate::test_support::build_test_state(pool.clone(), b"restore-test-secret-key");
        let user = db::insert_user(&pool, "downloader", "hash").await.unwrap();
        let passphrase =
            shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
        let repo = db::insert_repo(
            &pool,
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
            &pool,
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
                    AuditEvent::DownloadFiles { archive, paths }
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
            restore_files(
                State(self.state.clone()),
                RequireAdmin(self.admin.clone()),
                AxumPath((repo_id, "nightly".to_string())),
                Json(RestoreFilesRequest {
                    paths: vec!["etc/hosts".to_string()],
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
}
