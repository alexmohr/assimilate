// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::Json;

use super::{auth::RequireAdmin, helpers};
use crate::{
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
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<TestConnectionRequest>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    helpers::validate_non_empty(&req.ssh_host, "ssh_host")?;
    helpers::validate_non_empty(&req.ssh_user, "ssh_user")?;

    let response = ssh::test_connection(&req).await;
    Ok(Json(response))
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
