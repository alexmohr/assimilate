// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use shared::{
    audit::AuditEvent, protocol::ServerToUi, responses::RestoreResponse, types::RestoreStatus,
};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    archives::{get_repo_env, stream_export_tar_lz4, validate_path},
    auth::{AuthUser, RequireAdmin},
    permissions::check_repo_permission,
};
use crate::{AppState, borg::Borg, db, error::ApiError, restores};

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
        (status = 202, description = "Restore accepted: sent to the agent, or queued until it \
            reconnects. Follow it with GET /api/restores/{id}.", body = RestoreResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Repository or agent not found"),
    )
)]
/// Restore selected files from an archive to the agent filesystem.
///
/// Returns as soon as the restore is recorded. It is sent to the agent right
/// away when the agent is connected, and once it reconnects otherwise.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the repository or the agent does not exist
/// - [`ApiError::Database`]: the restore could not be recorded
pub async fn restore_files(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<RestoreFilesRequest>,
) -> Result<(StatusCode, Json<RestoreResponse>), ApiError> {
    if body.target_path.is_empty() {
        return Err(ApiError::BadRequest(
            "target_path must not be empty".to_owned(),
        ));
    }
    // borg stores paths without their leading slash and matches them the
    // same way with or without one, so "/etc/hosts" is taken as "etc/hosts".
    let paths: Vec<String> = body
        .paths
        .iter()
        .map(|path| path.trim_start_matches('/').to_owned())
        .collect();
    for path in &paths {
        validate_path(path)?;
    }

    db::get_repo_name(&state.pool, repo_id).await?;
    let agent =
        db::get_agent_by_hostname(&state.pool, &body.hostname, body.domain.as_deref()).await?;

    let request_id = Uuid::new_v4().to_string();
    let restore_id = db::restores::insert_restore(
        &state.pool,
        &db::restores::NewRestore {
            request_id: &request_id,
            repo_id,
            agent_id: agent.id,
            archive_name: &archive_name,
            paths: &paths,
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
                paths: paths.clone(),
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

    let restore = db::restores::get_restore(&state.pool, restore_id).await?;
    restores::dispatch(&state, &restore).await?;
    let restore = db::restores::get_restore(&state.pool, restore_id).await?;
    Ok((StatusCode::ACCEPTED, Json(restore.into())))
}

#[utoipa::path(
    get,
    path = "/api/restores/{id}",
    tag = "Archives",
    operation_id = "getRestore",
    params(("id" = i64, Path, description = "Restore ID")),
    responses(
        (status = 200, description = "The restore and how far it has got", body = RestoreResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Restore not found"),
    )
)]
/// Get a restore and how far it has got.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such restore.
pub async fn get_restore(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<RestoreResponse>, ApiError> {
    Ok(Json(
        db::restores::get_restore(&state.pool, id).await?.into(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/restores/{id}/cancel",
    tag = "Archives",
    operation_id = "cancelRestore",
    params(("id" = i64, Path, description = "Restore ID")),
    responses(
        (status = 200, description = "Restore cancelled", body = RestoreResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin only"),
        (status = 404, description = "Restore not found"),
        (status = 409, description = "The restore was already sent to its agent"),
    )
)]
/// Cancel a restore that is still waiting for its agent to reconnect. Once
/// the agent has it, a restore runs to the end.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such restore, or
/// [`ApiError::Conflict`] if it is no longer queued.
pub async fn cancel_restore(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<RestoreResponse>, ApiError> {
    let restore = db::restores::get_restore(&state.pool, id).await?;
    if !db::restores::cancel_queued_restore(&state.pool, id).await? {
        return Err(ApiError::Conflict(
            "the restore was already sent to its agent and can no longer be cancelled".to_owned(),
        ));
    }

    if let Err(e) = db::audit::insert_audit_entry(
        &state.pool,
        &db::audit::NewAuditEntry {
            user_id: Some(admin.user_id),
            username: &admin.username,
            event: AuditEvent::RestoreCancelled {
                archive: restore.archive_name.clone(),
                paths: restore.paths.clone(),
                target_path: restore.target_path.clone(),
                hostname: restore.hostname.clone(),
            },
            target_type: Some("archive"),
            target_id: Some(restore.repo_id),
            ip_address: None,
        },
    )
    .await
    {
        tracing::warn!("failed to write audit log: {e}");
    }

    state.ui_broadcast.send(ServerToUi::RestoreUpdated {
        restore_id: id,
        status: RestoreStatus::Cancelled,
    });
    Ok(Json(
        db::restores::get_restore(&state.pool, id).await?.into(),
    ))
}

#[cfg(test)]
mod tests {
    use shared::{protocol::ServerToAgent, types::SystemEventType};
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

    struct Fixture {
        state: AppState,
        admin: AuthUser,
        repo_id: i64,
        agent: db::AgentRow,
    }

    async fn fixture(pool: &PgPool, hostname: &str) -> Fixture {
        let state = crate::test_support::build_test_state(pool.clone(), b"restore-test-secret-key");
        let user = db::insert_user(pool, &format!("{hostname}-admin"), "hash")
            .await
            .unwrap();
        let passphrase =
            shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
        let repo = db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: &format!("{hostname}-repo"),
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
        let agent = db::insert_agent(pool, hostname, None, "hash", None, None)
            .await
            .unwrap();
        Fixture {
            state,
            admin: AuthUser {
                user_id: user.id,
                username: format!("{hostname}-admin"),
                session_id: None,
            },
            repo_id: repo.id,
            agent,
        }
    }

    fn request(hostname: &str, paths: &[&str]) -> RestoreFilesRequest {
        RestoreFilesRequest {
            paths: paths.iter().map(ToString::to_string).collect(),
            target_path: "/tmp/restore".to_owned(),
            hostname: hostname.to_owned(),
            domain: None,
        }
    }

    async fn start(fixture: &Fixture, paths: &[&str]) -> RestoreResponse {
        start_with_request(fixture, request(&fixture.agent.hostname, paths)).await
    }

    async fn start_with_request(fixture: &Fixture, body: RestoreFilesRequest) -> RestoreResponse {
        let (status, Json(restore)) = restore_files(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath((fixture.repo_id, "nightly".to_owned())),
            Json(body),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::ACCEPTED);
        restore
    }

    async fn connect(fixture: &Fixture) -> tokio::sync::mpsc::Receiver<ServerToAgent> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        fixture
            .state
            .registry
            .register(fixture.agent.id, tx, false, None)
            .await;
        rx
    }

    fn sent_request_id(rx: &mut tokio::sync::mpsc::Receiver<ServerToAgent>) -> String {
        let sent = rx
            .try_recv()
            .map(|msg| serde_json::to_value(msg).unwrap())
            .unwrap_or_default();
        let field = |path: &str| sent.pointer(path).cloned();
        assert_eq!(
            field("/type"),
            Some(serde_json::json!("RestoreFiles")),
            "{sent}"
        );
        assert_eq!(
            field("/payload/archive_name"),
            Some(serde_json::json!("nightly"))
        );
        assert_eq!(
            field("/payload/paths"),
            Some(serde_json::json!(["etc/hosts"]))
        );
        assert_eq!(
            field("/payload/target_path"),
            Some(serde_json::json!("/tmp/restore"))
        );
        sent.pointer("/payload/request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    /// Registers the fixture's agent with a connection whose other end is
    /// already gone, so it counts as connected but nothing reaches it.
    async fn connect_dead(fixture: &Fixture) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        drop(rx);
        fixture
            .state
            .registry
            .register(fixture.agent.id, tx, false, None)
            .await;
    }

    /// Makes every write the named trigger guards fail, as a database that
    /// refuses the statement would.
    async fn create_failing_write_function(pool: &PgPool) {
        sqlx::query!(
            "CREATE FUNCTION test_refuse_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
             RAISE EXCEPTION 'write refused by test'; END $$"
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn system_events(pool: &PgPool) -> Vec<(SystemEventType, String)> {
        db::get_system_events(pool, 50, shared::types::AcknowledgedFilter::All)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.event_type, e.message))
            .collect()
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_to_an_offline_agent_is_queued_and_audited(pool: PgPool) {
        let fixture = fixture(&pool, "restore-offline").await;

        let restore = start(&fixture, &["etc/hosts"]).await;

        assert_eq!(restore.status, RestoreStatus::Queued);
        assert_eq!(restore.hostname, "restore-offline");
        assert_eq!(restore.requested_by, "restore-offline-admin");
        assert_eq!(restore.paths, ["etc/hosts"]);
        let (entries, _) = db::audit::list_audit_entries(
            &pool,
            &db::audit::AuditEntryFilters {
                page: 1,
                per_page: 10,
                filter_user_id: Some(fixture.admin.user_id),
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
                [entry] if matches!(&entry.event, AuditEvent::RestoreFiles { archive, .. }
                    if archive == "nightly")
            ),
            "the restore request must be audited, got {entries:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_returns_at_once_and_follows_the_agent_to_success(pool: PgPool) {
        let fixture = fixture(&pool, "restore-online").await;
        let mut rx = connect(&fixture).await;

        let restore = start(&fixture, &["etc/hosts"]).await;
        assert_eq!(restore.status, RestoreStatus::Dispatched);
        let request_id = sent_request_id(&mut rx);

        assert!(restores::record_started(&fixture.state, fixture.agent.id, &request_id).await);
        let running = db::restores::get_restore(&pool, restore.id).await.unwrap();
        assert_eq!(running.status, RestoreStatus::Running);
        assert!(running.started_at.is_some());

        let outcome = db::restores::RestoreOutcome::Succeeded { files_restored: 1 };
        assert!(
            restores::record_finished(&fixture.state, fixture.agent.id, &request_id, &outcome)
                .await
        );
        let done = get_restore(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath(restore.id),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(done.status, RestoreStatus::Succeeded);
        assert_eq!(done.files_restored, Some(1));
        assert!(done.finished_at.is_some());
        assert_eq!(
            system_events(&pool).await,
            [(
                SystemEventType::RestoreCompleted,
                "Restored etc/hosts from nightly to /tmp/restore on restore-online".to_owned()
            )]
        );

        // A late or repeated answer must not rewrite a restore that ended.
        let late = db::restores::RestoreOutcome::Failed {
            error_message: "late".to_owned(),
        };
        assert!(
            !restores::record_finished(&fixture.state, fixture.agent.id, &request_id, &late).await
        );
        assert_eq!(
            db::restores::get_restore(&pool, restore.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Succeeded
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_restore_records_why_in_the_activity_log(pool: PgPool) {
        let fixture = fixture(&pool, "restore-fails").await;
        let mut rx = connect(&fixture).await;
        let restore = start(&fixture, &["etc/hosts"]).await;
        let request_id = sent_request_id(&mut rx);

        let outcome = db::restores::RestoreOutcome::Failed {
            error_message: "borg extract failed (exit 2): no such archive".to_owned(),
        };
        assert!(
            restores::record_finished(&fixture.state, fixture.agent.id, &request_id, &outcome)
                .await
        );

        let failed = db::restores::get_restore(&pool, restore.id).await.unwrap();
        assert_eq!(failed.status, RestoreStatus::Failed);
        assert_eq!(
            failed.error_message.as_deref(),
            Some("borg extract failed (exit 2): no such archive")
        );
        assert_eq!(
            system_events(&pool).await,
            [(
                SystemEventType::RestoreFailed,
                "Restore of etc/hosts from nightly to /tmp/restore on restore-fails failed: borg \
                 extract failed (exit 2): no such archive"
                    .to_owned()
            )]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn another_agent_cannot_answer_a_restore(pool: PgPool) {
        let fixture = fixture(&pool, "restore-target").await;
        let other = db::insert_agent(&pool, "restore-other", None, "hash", None, None)
            .await
            .unwrap();
        let mut rx = connect(&fixture).await;
        let restore = start(&fixture, &["etc/hosts"]).await;
        let request_id = sent_request_id(&mut rx);

        let forged = db::restores::RestoreOutcome::Succeeded { files_restored: 1 };
        assert!(!restores::record_started(&fixture.state, other.id, &request_id).await);
        assert!(!restores::record_finished(&fixture.state, other.id, &request_id, &forged).await);

        assert_eq!(
            db::restores::get_restore(&pool, restore.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Dispatched
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_reconnecting_agent_gets_its_queued_and_unfinished_restores(pool: PgPool) {
        let fixture = fixture(&pool, "restore-reconnect").await;
        let queued = start(&fixture, &["etc/hosts"]).await;
        assert_eq!(queued.status, RestoreStatus::Queued);

        let mut rx = connect(&fixture).await;
        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-reconnect").await;
        let request_id = sent_request_id(&mut rx);
        assert_eq!(
            db::restores::get_restore(&pool, queued.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Dispatched
        );

        // The agent may have restarted and lost it: a later connect sends it
        // again, under the same request id so the agent can tell.
        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-reconnect").await;
        assert_eq!(sent_request_id(&mut rx), request_id);
        assert!(
            rx.try_recv().is_err(),
            "each restore is sent once per connect"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn only_a_restore_still_queued_can_be_cancelled(pool: PgPool) {
        let fixture = fixture(&pool, "restore-cancel").await;
        let queued = start(&fixture, &["etc/hosts"]).await;

        let cancelled = cancel_restore(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath(queued.id),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(cancelled.status, RestoreStatus::Cancelled);
        assert!(cancelled.finished_at.is_some());

        // A cancelled restore is never sent once the agent comes back.
        let mut rx = connect(&fixture).await;
        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-cancel").await;
        assert!(rx.try_recv().is_err());

        let dispatched = start(&fixture, &["etc/hosts"]).await;
        assert_eq!(dispatched.status, RestoreStatus::Dispatched);
        let result = cancel_restore(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath(dispatched.id),
        )
        .await;
        assert!(matches!(result, Err(ApiError::Conflict(_))));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_checks_its_paths_and_repository(pool: PgPool) {
        let fixture = fixture(&pool, "restore-invalid").await;

        let traversal = restore_files(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath((fixture.repo_id, "nightly".to_owned())),
            Json(request("restore-invalid", &["../etc/shadow"])),
        )
        .await;
        assert!(matches!(traversal, Err(ApiError::BadRequest(_))));

        let absolute =
            start_with_request(&fixture, request("restore-invalid", &["/etc/hosts"])).await;
        assert_eq!(
            absolute.paths,
            ["etc/hosts"],
            "a leading slash is dropped, as borg itself does"
        );

        let unknown_repo = restore_files(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath((987_654, "nightly".to_owned())),
            Json(request("restore-invalid", &["etc/hosts"])),
        )
        .await;
        assert!(matches!(unknown_repo, Err(ApiError::NotFound(_))));

        let unknown = get_restore(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath(424_242),
        )
        .await;
        assert!(matches!(unknown, Err(ApiError::NotFound(_))));
    }

    #[test]
    fn restore_response_serializes_its_state_for_the_ui() {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let response: RestoreResponse = db::restores::RestoreRow {
            id: 7,
            request_id: "req-7".to_owned(),
            repo_id: 2,
            agent_id: 3,
            hostname: "web-server-01".to_owned(),
            archive_name: "nightly".to_owned(),
            paths: vec![],
            target_path: "/tmp/restore".to_owned(),
            requested_by: "admin".to_owned(),
            status: RestoreStatus::Queued,
            files_restored: None,
            error_message: None,
            created_at,
            started_at: None,
            finished_at: None,
        }
        .into();

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "id": 7,
                "repo_id": 2,
                "archive_name": "nightly",
                "paths": [],
                "target_path": "/tmp/restore",
                "agent_id": 3,
                "hostname": "web-server-01",
                "status": "queued",
                "files_restored": null,
                "error_message": null,
                "requested_by": "admin",
                "created_at": "2026-10-09T12:00:00Z",
                "started_at": null,
                "finished_at": null,
            })
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_already_sent_is_not_sent_twice(pool: PgPool) {
        let fixture = fixture(&pool, "restore-once").await;
        let mut rx = connect(&fixture).await;
        let restore = start(&fixture, &["etc/hosts"]).await;
        sent_request_id(&mut rx);

        let row = db::restores::get_restore(&pool, restore.id).await.unwrap();
        restores::dispatch(&fixture.state, &row).await.unwrap();

        assert!(
            rx.try_recv().is_err(),
            "a dispatched restore is not sent again"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_that_cannot_reach_its_agent_goes_back_in_the_queue(pool: PgPool) {
        let fixture = fixture(&pool, "restore-dead-link").await;
        connect_dead(&fixture).await;

        let restore = start(&fixture, &["etc/hosts"]).await;

        assert_eq!(restore.status, RestoreStatus::Queued);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_resend_to_an_agent_that_went_away_leaves_the_restore_unfinished(pool: PgPool) {
        let fixture = fixture(&pool, "restore-resend-gone").await;
        let mut rx = connect(&fixture).await;
        let restore = start(&fixture, &["etc/hosts"]).await;
        sent_request_id(&mut rx);
        drop(rx);

        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-resend-gone").await;

        assert_eq!(
            db::restores::get_restore(&pool, restore.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Dispatched
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_queued_restore_whose_hand_over_cannot_be_recorded_stays_queued(pool: PgPool) {
        let fixture = fixture(&pool, "restore-locked").await;
        let queued = start(&fixture, &["etc/hosts"]).await;
        create_failing_write_function(&pool).await;
        sqlx::query!(
            "CREATE TRIGGER refuse_restore_updates BEFORE UPDATE ON restores FOR EACH ROW EXECUTE \
             FUNCTION test_refuse_write()"
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut rx = connect(&fixture).await;

        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-locked").await;

        assert!(
            rx.try_recv().is_err(),
            "nothing is sent without the hand-over"
        );
        assert_eq!(
            db::restores::get_restore(&pool, queued.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Queued
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_still_finishes_when_its_activity_entry_cannot_be_written(pool: PgPool) {
        let fixture = fixture(&pool, "restore-no-event").await;
        let mut rx = connect(&fixture).await;
        let restore = start(&fixture, &["etc/hosts"]).await;
        let request_id = sent_request_id(&mut rx);
        create_failing_write_function(&pool).await;
        sqlx::query!(
            "CREATE TRIGGER refuse_system_events BEFORE INSERT ON system_events FOR EACH ROW \
             EXECUTE FUNCTION test_refuse_write()"
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = db::restores::RestoreOutcome::Succeeded { files_restored: 1 };
        assert!(
            restores::record_finished(&fixture.state, fixture.agent.id, &request_id, &outcome)
                .await
        );

        assert_eq!(
            db::restores::get_restore(&pool, restore.id)
                .await
                .unwrap()
                .status,
            RestoreStatus::Succeeded
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_is_accepted_and_cancelled_even_when_it_cannot_be_audited(pool: PgPool) {
        let fixture = fixture(&pool, "restore-no-audit").await;
        create_failing_write_function(&pool).await;
        sqlx::query!(
            "CREATE TRIGGER refuse_audit_entries BEFORE INSERT ON audit_log FOR EACH ROW EXECUTE \
             FUNCTION test_refuse_write()"
        )
        .execute(&pool)
        .await
        .unwrap();

        let restore = start(&fixture, &["etc/hosts"]).await;
        assert_eq!(restore.status, RestoreStatus::Queued);

        let cancelled = cancel_restore(
            State(fixture.state.clone()),
            RequireAdmin(fixture.admin.clone()),
            AxumPath(restore.id),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(cancelled.status, RestoreStatus::Cancelled);
    }

    /// An agent answer that arrives while the database is unreachable is
    /// claimed (so it is not reported as unexpected) and logged, never
    /// half-applied.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn agent_answers_survive_a_database_that_is_gone(pool: PgPool) {
        let fixture = fixture(&pool, "restore-db-gone").await;
        fixture.state.pool.close().await;

        restores::resume_for_agent(&fixture.state, fixture.agent.id, "restore-db-gone").await;
        assert!(restores::record_started(&fixture.state, fixture.agent.id, "req-gone").await);
        let outcome = db::restores::RestoreOutcome::Failed {
            error_message: "gone".to_owned(),
        };
        assert!(
            restores::record_finished(&fixture.state, fixture.agent.id, "req-gone", &outcome).await
        );
    }
}
