// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use shared::audit::AuditEvent;
use tokio::io::AsyncWriteExt;

use super::{
    archives::{ensure_borg_success, get_repo_env},
    auth::RequireAdmin,
    helpers,
    repos::verify_repo_access,
};
use crate::{
    AppState,
    borg::Borg,
    db::{
        self,
        audit::{NewAuditEntry, insert_audit_entry},
    },
    error::{ApiError, ApiJson},
};

/// Request payload for importing a borg repository key.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportKeyRequest {
    /// The key data to import.
    pub key_data: String,
}

/// Request payload for changing a repository passphrase.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct ChangePassphraseRequest {
    /// The new passphrase.
    pub new_passphrase: String,
}

impl std::fmt::Debug for ChangePassphraseRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        redacted(f, "ChangePassphraseRequest", "new_passphrase")
    }
}

/// Request payload for setting the passphrase Assimilate uses for a repository.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetPassphraseRequest {
    /// The repository's current borg passphrase.
    pub passphrase: String,
}

impl std::fmt::Debug for SetPassphraseRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        redacted(f, "SetPassphraseRequest", "passphrase")
    }
}

/// Formats a request whose only field is a secret, without the secret.
fn redacted(f: &mut std::fmt::Formatter<'_>, name: &str, field: &str) -> std::fmt::Result {
    f.debug_struct(name).field(field, &"[REDACTED]").finish()
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/export",
    tag = "Keys",
    operation_id = "exportKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    responses(
        (status = 200, description = "Key exported as text/plain", content_type = "text/plain"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Export the borg repository key.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn export_key(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
) -> Result<Response, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let output = Borg::new()
        .with_registry(state.task_registry.clone())
        .run(&["key", "export", "--stdout", borg_repo.as_str()], &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;
    let stdout = ensure_borg_success(output)?;

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::KeyExport {},
            target_type: Some("repo"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await?;

    let key_text = String::from_utf8(stdout)
        .map_err(|e| ApiError::Internal(format!("borg key output is not valid UTF-8: {e}")))?;

    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        key_text,
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/import",
    tag = "Keys",
    operation_id = "importKey",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = ImportKeyRequest,
    responses(
        (status = 204, description = "Key imported successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Import a borg repository key.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn import_key(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
    Json(req): Json<ImportKeyRequest>,
) -> Result<StatusCode, ApiError> {
    let (borg_repo, env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    let mut child = Borg::new()
        .with_registry(state.task_registry.clone())
        .spawn_with_stdin(&["key", "import", borg_repo.as_str(), "-"], &env)
        .map_err(|e| ApiError::Internal(format!("failed to spawn borg: {e}")))?;

    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| ApiError::Internal("failed to capture borg stdin".to_string()))?;

    stdin
        .write_all(req.key_data.as_bytes())
        .await
        .map_err(|e| ApiError::Internal(format!("failed to write key to borg stdin: {e}")))?;

    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to wait for borg: {e}")))?;
    ensure_borg_success(output)?;

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::KeyImport {},
            target_type: Some("repo"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/key/change-passphrase",
    tag = "Keys",
    operation_id = "changePassphrase",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = ChangePassphraseRequest,
    responses(
        (status = 204, description = "Passphrase changed successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 502, description = "Borg command failed"),
    )
)]
/// Change the borg repository passphrase.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Database`]: the database query fails
pub async fn change_passphrase(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
    Json(req): Json<ChangePassphraseRequest>,
) -> Result<StatusCode, ApiError> {
    let (borg_repo, mut env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;

    env.insert(
        "BORG_NEW_PASSPHRASE".to_string(),
        req.new_passphrase.clone(),
    );

    let output = Borg::new()
        .with_registry(state.task_registry.clone())
        .run(&["key", "change-passphrase", borg_repo.as_str()], &env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;
    ensure_borg_success(output)?;

    let encrypted = shared::crypto::encrypt_passphrase(&req.new_passphrase, &state.encryption_key)
        .map_err(|e| ApiError::Internal(format!("failed to encrypt passphrase: {e}")))?;

    db::update_repo_passphrase(&state.pool, repo_id, &encrypted).await?;

    // A real passphrase means the repo is ready for scheduled sync - clear the
    // importing guard that was set (e.g. by config import) to prevent sync with
    // a placeholder passphrase.
    if let Err(e) = db::set_repo_importing(&state.pool, repo_id, false).await {
        tracing::error!(repo_id, error = %e, "failed to clear importing flag after passphrase change");
    }

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::KeyChangePassphrase {},
            target_type: Some("repo"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await?;

    crate::api::helpers::push_config_to_all_agents(&state).await;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/passphrase",
    tag = "Keys",
    operation_id = "setRepoPassphrase",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
    ),
    request_body = SetPassphraseRequest,
    responses(
        (status = 204, description = "Passphrase verified and stored"),
        (status = 400, description = "borg rejected the passphrase"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "A sync is running for the repository"),
        (status = 502, description = "borg could not reach the repository"),
    )
)]
/// Set the passphrase Assimilate uses for a repository (admin only).
///
/// Unlike [`change_passphrase`], this leaves the repository's key alone: it
/// records the passphrase the key already has, e.g. for a repository created
/// by a config import, which arrives without one. borg must accept the
/// passphrase before it is stored, and storing it clears the importing guard
/// that kept the scheduler away from the repository until then.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: borg rejects the passphrase
/// - [`ApiError::Conflict`]: a sync is running for the repository
/// - [`ApiError::BadGateway`]: borg cannot reach the repository
/// - [`ApiError::Database`]: the database query fails
pub async fn set_passphrase(
    State(state): State<AppState>,
    RequireAdmin(auth): RequireAdmin,
    AxumPath(repo_id): AxumPath<i64>,
    ApiJson(req): ApiJson<SetPassphraseRequest>,
) -> Result<StatusCode, ApiError> {
    ensure_no_sync_running(&state, repo_id).await?;
    // With no sync running, a set importing flag is the hold a config import
    // left until the passphrase is known - and while it is set, no sync can
    // start. Only that hold is ours to release: on any other repository the
    // flag belongs to whichever sync sets it, so it is never written here.
    let held_for_passphrase = db::get_repo_with_stats(&state.pool, repo_id)
        .await?
        .importing;

    let (borg_repo, mut env) = get_repo_env(&state.pool, &state.encryption_key, repo_id).await?;
    env.insert("BORG_PASSPHRASE".to_string(), req.passphrase.clone());
    verify_repo_access(&state.pool, &borg_repo, &env, &state.task_registry).await?;
    // borg can take as long as the query timeout, and a sync started meanwhile
    // (after a reset of the import state) would now own the importing flag.
    ensure_no_sync_running(&state, repo_id).await?;

    let encrypted = shared::crypto::encrypt_passphrase(&req.passphrase, &state.encryption_key)
        .map_err(|e| ApiError::Internal(format!("failed to encrypt passphrase: {e}")))?;
    db::update_repo_passphrase(&state.pool, repo_id, &encrypted).await?;
    if held_for_passphrase {
        db::set_repo_importing(&state.pool, repo_id, false).await?;
    }
    // An import or sync that ran with the old passphrase left its failure
    // behind; borg has just accepted the new one, so that error is stale.
    db::set_repo_import_error(&state.pool, repo_id, None).await?;

    insert_audit_entry(
        &state.pool,
        &NewAuditEntry {
            user_id: Some(auth.user_id),
            username: &auth.username,
            event: AuditEvent::SetRepoPassphrase {},
            target_type: Some("repo"),
            target_id: Some(repo_id),
            ip_address: None,
        },
    )
    .await?;

    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
    helpers::push_config_to_all_agents(&state).await;

    Ok(StatusCode::NO_CONTENT)
}

/// Refuses with [`ApiError::Conflict`] while an import or sync of `repo_id` runs.
async fn ensure_no_sync_running(state: &AppState, repo_id: i64) -> Result<(), ApiError> {
    if state.import_tasks.is_running(repo_id).await {
        return Err(ApiError::Conflict(
            "a sync is running for this repository; wait for it to finish before setting the \
             passphrase"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{
        audit_events, build_test_state, insert_auth_user, install_fake_borg,
    };

    const KEY_MATERIAL: &[u8] = b"keys-test-secret-key";

    /// A borg that only opens the repository with the passphrase `right`.
    const FAKE_BORG: &str = "#!/bin/sh\nif [ \"$BORG_PASSPHRASE\" = right ]; then\n  printf \
                             '{\"encryption\":{\"mode\":\"repokey\"}}'\n  exit 0\nfi\necho \
                             'passphrase supplied in BORG_PASSPHRASE is incorrect' >&2\nexit 2\n";

    /// A repository as a config import leaves it: placeholder passphrase, importing.
    async fn insert_imported_repo(state: &AppState) -> i64 {
        let placeholder = shared::crypto::encrypt_passphrase("", &state.encryption_key).unwrap();
        let repo = db::insert_repo(
            &state.pool,
            &db::InsertRepoParams {
                name: "imported",
                repo_path: "/backups/imported",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &placeholder,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::set_repo_importing(&state.pool, repo.id, true)
            .await
            .unwrap();
        repo.id
    }

    async fn set(state: &AppState, repo_id: i64, passphrase: &str) -> Result<StatusCode, ApiError> {
        set_passphrase(
            State(state.clone()),
            RequireAdmin(insert_auth_user(&state.pool, &format!("admin-{passphrase}")).await),
            AxumPath(repo_id),
            ApiJson(SetPassphraseRequest {
                passphrase: passphrase.to_string(),
            }),
        )
        .await
    }

    async fn stored_passphrase(state: &AppState, repo_id: i64) -> String {
        let encrypted = db::get_repo_passphrase(&state.pool, repo_id).await.unwrap();
        shared::crypto::decrypt_passphrase(&encrypted, &state.encryption_key).unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_passphrase_borg_accepts_is_stored_and_releases_the_repository(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        db::set_repo_import_error(&state.pool, repo_id, Some("passphrase is incorrect"))
            .await
            .unwrap();
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

        assert_eq!(
            set(&state, repo_id, "right").await.unwrap(),
            StatusCode::NO_CONTENT
        );

        assert_eq!(stored_passphrase(&state, repo_id).await, "right");
        let repo = db::get_repo_with_stats(&state.pool, repo_id).await.unwrap();
        assert!(
            !repo.importing,
            "the scheduler must no longer skip the repository"
        );
        assert_eq!(
            repo.import_error, None,
            "a failure from the old passphrase must not outlive the new one"
        );
        assert_eq!(
            audit_events(&state.pool).await,
            vec![AuditEvent::SetRepoPassphrase {}]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_passphrase_borg_rejects_is_not_stored(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

        let result = set(&state, repo_id, "wrong").await;

        assert!(
            matches!(result, Err(ApiError::BadRequest(ref msg)) if msg.contains("incorrect")),
            "expected borg's rejection, got {result:?}"
        );
        assert_eq!(stored_passphrase(&state, repo_id).await, "");
        let repo = db::get_repo_with_stats(&state.pool, repo_id).await.unwrap();
        assert!(
            repo.importing,
            "a rejected passphrase must keep the scheduler away"
        );
        assert_eq!(audit_events(&state.pool).await, Vec::<AuditEvent>::new());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_passphrase_is_not_set_while_a_sync_runs(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        let _task = state.import_tasks.start(repo_id).await;

        let result = set(&state, repo_id, "right").await;

        assert!(
            matches!(result, Err(ApiError::Conflict(_))),
            "expected a conflict, got {result:?}"
        );
        assert_eq!(stored_passphrase(&state, repo_id).await, "");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_sync_started_while_borg_checks_the_passphrase_keeps_it_unset(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        let flags = tempfile::tempdir().unwrap();
        let started = flags.path().join("started");
        let release = flags.path().join("release");
        // Accepts the passphrase, but only once the test has started a sync.
        let script = format!(
            "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nprintf \
             '{{\"encryption\":{{\"mode\":\"repokey\"}}}}'\n",
            started.display(),
            release.display()
        );
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(&script).await;

        let request = tokio::spawn({
            let state = state.clone();
            async move { set(&state, repo_id, "right").await }
        });
        while !tokio::fs::try_exists(&started).await.unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let _task = state.import_tasks.start(repo_id).await;
        tokio::fs::write(&release, b"").await.unwrap();
        let result = request.await.unwrap();

        assert!(
            matches!(result, Err(ApiError::Conflict(_))),
            "expected a conflict, got {result:?}"
        );
        assert_eq!(stored_passphrase(&state, repo_id).await, "");
        let repo = db::get_repo_with_stats(&state.pool, repo_id).await.unwrap();
        assert!(
            repo.importing,
            "the running sync must keep its importing flag"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_repository_not_held_for_its_passphrase_keeps_a_sync_s_importing_flag(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        db::set_repo_importing(&state.pool, repo_id, false)
            .await
            .unwrap();
        let flags = tempfile::tempdir().unwrap();
        let started = flags.path().join("started");
        let release = flags.path().join("release");
        let script = format!(
            "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nprintf \
             '{{\"encryption\":{{\"mode\":\"repokey\"}}}}'\n",
            started.display(),
            release.display()
        );
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(&script).await;

        let request = tokio::spawn({
            let state = state.clone();
            async move { set(&state, repo_id, "right").await }
        });
        while !tokio::fs::try_exists(&started).await.unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        // "Sync now" sets the flag before it registers its task, so the
        // second running-sync check cannot see it yet.
        db::set_repo_importing(&state.pool, repo_id, true)
            .await
            .unwrap();
        tokio::fs::write(&release, b"").await.unwrap();

        assert_eq!(request.await.unwrap().unwrap(), StatusCode::NO_CONTENT);
        assert_eq!(stored_passphrase(&state, repo_id).await, "right");
        let repo = db::get_repo_with_stats(&state.pool, repo_id).await.unwrap();
        assert!(repo.importing, "the sync's importing flag must survive");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn an_unreachable_repository_times_out_without_storing_anything(pool: PgPool) {
        let state = build_test_state(pool, KEY_MATERIAL);
        let repo_id = insert_imported_repo(&state).await;
        db::set_setting(&state.pool, "borg_query_timeout_secs", "1")
            .await
            .unwrap();
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg("#!/bin/sh\nsleep 5\n").await;

        let result = set(&state, repo_id, "right").await;

        assert!(
            matches!(result, Err(ApiError::BadGateway(ref msg)) if msg.contains("timed out after 1s")),
            "expected the borg timeout, got {result:?}"
        );
        assert_eq!(stored_passphrase(&state, repo_id).await, "");
    }

    #[test]
    fn the_request_never_debug_prints_the_passphrase() {
        let req = SetPassphraseRequest {
            passphrase: "hunter2".to_string(),
        };

        let printed = format!("{req:?}");

        assert!(!printed.contains("hunter2"));
        assert!(printed.contains("[REDACTED]"));
    }

    #[test]
    fn a_passphrase_change_never_debug_prints_the_new_passphrase() {
        let req = ChangePassphraseRequest {
            new_passphrase: "hunter2".to_string(),
        };

        let printed = format!("{req:?}");

        assert!(!printed.contains("hunter2"));
        assert!(printed.contains("new_passphrase: \"[REDACTED]\""));
    }
}
