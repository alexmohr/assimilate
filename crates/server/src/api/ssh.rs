// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, extract::State};
use shared::provider::HostProvider;
use sqlx::PgPool;

use super::{auth::RequireAdmin, helpers};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
    ssh::{
        self, DeployKeyRequest, DeployKeyResponse, ListDirRequest, ListDirResponse, MkdirRequest,
        MkdirResponse, TestConnectionRequest, TestConnectionResponse,
    },
};

#[utoipa::path(
    post,
    path = "/api/ssh/test-connection",
    tag = "SSH",
    operation_id = "testSshConnection",
    request_body = TestConnectionRequest,
    responses(
        (status = 200, description = "Connection test result", body = TestConnectionResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Test SSH connectivity and check if borg is installed.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn test_connection(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(mut req): ApiJson<TestConnectionRequest>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;

    req.provider = Some(probe_provider(&state.pool, &req.ssh_host, req.provider).await?);
    let response = ssh::test_connection(&req).await;
    Ok(Json(response))
}

/// The provider whose borg the probe asks for: the one named, else the one
/// the host is registered with, else generic. A host that does not exist yet
/// is not created by testing it.
async fn probe_provider(
    pool: &PgPool,
    ssh_host: &str,
    requested: Option<HostProvider>,
) -> Result<HostProvider, ApiError> {
    if let Some(requested) = requested {
        return Ok(requested);
    }
    Ok(db::repo_hosts::find_repo_host_by_name(pool, ssh_host)
        .await?
        .map_or_else(HostProvider::default, |host| host.provider))
}

#[utoipa::path(
    post,
    path = "/api/ssh/deploy-key",
    tag = "SSH",
    operation_id = "deploySshKey",
    request_body = DeployKeyRequest,
    responses(
        (status = 200, description = "Deploy result", body = DeployKeyResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Deploy the server's SSH public key to a remote host.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn deploy_key(
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<DeployKeyRequest>,
) -> Result<Json<DeployKeyResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;
    helpers::validate_non_empty(&req.password, "password")?;

    let response = ssh::deploy_key(&req).await;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/ssh/list-dir",
    tag = "SSH",
    operation_id = "listSshDir",
    request_body = ListDirRequest,
    responses(
        (status = 200, description = "Directory listing", body = ListDirResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List directory contents on a remote host via SSH.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_dir(
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<ListDirRequest>,
) -> Result<Json<ListDirResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;

    let response = ssh::list_dir(&req).await;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/ssh/mkdir",
    tag = "SSH",
    operation_id = "mkdirSsh",
    request_body = MkdirRequest,
    responses(
        (status = 200, description = "Mkdir result", body = MkdirResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Create a directory on a remote host via SSH.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn mkdir(
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<MkdirRequest>,
) -> Result<Json<MkdirResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;
    helpers::validate_non_empty(&req.path, "path")?;

    let response = ssh::mkdir(&req).await;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_probe_uses_the_named_provider_else_the_hosts_else_generic(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        db::repo_hosts::resolve_repo_host_for_provider(
            &mut tx,
            "ab1.rsync.net",
            22,
            Some(HostProvider::RsyncNet),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        // The host's own provider, when none is named.
        assert_eq!(
            probe_provider(&pool, "ab1.rsync.net", None).await.unwrap(),
            HostProvider::RsyncNet
        );
        // A named provider wins, as for a host that is not registered yet.
        assert_eq!(
            probe_provider(&pool, "new.example", Some(HostProvider::Hetzner))
                .await
                .unwrap(),
            HostProvider::Hetzner
        );
        // An unknown host is generic, and testing it does not register it.
        assert_eq!(
            probe_provider(&pool, "unknown.example", None)
                .await
                .unwrap(),
            HostProvider::Generic
        );
        assert!(
            db::repo_hosts::find_repo_host_by_name(&pool, "unknown.example")
                .await
                .unwrap()
                .is_none()
        );
    }
}
