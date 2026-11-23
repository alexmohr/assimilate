// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `BorgBase` account the server manages repositories on.
//!
//! An admin stores one API token. With it the server lists the account's repositories with
//! their quota and usage, and creates a repository whose only key is the server's own, so the
//! new repository can be added as a repository host without leaving Assimilate.

use axum::{Extension, Json, extract::State, http::StatusCode};
use shared::{
    audit::AuditEvent,
    borgbase::{
        BorgbaseRepo, BorgbaseStatus, CreateBorgbaseRepoRequest, CreatedBorgbaseRepo,
        SetBorgbaseTokenRequest,
    },
    crypto::{decrypt_passphrase, encrypt_passphrase},
};
use sqlx::PgPool;

use super::{
    audit_trail::{self, Actor, ClientIp},
    auth::RequireAdmin,
};
use crate::{
    AppState,
    borgbase::{ApiToken, BorgbaseClient, BorgbaseError},
    db,
    error::{ApiError, ApiJson},
    ssh,
};

/// The GraphQL endpoint the `BorgBase` handlers send requests to, added to the router as an
/// extension so a test can point them at a stand-in.
#[derive(Debug, Clone)]
pub struct BorgbaseApiUrl(pub String);

/// The name the server's key is stored under on `BorgBase`.
const KEY_NAME: &str = "assimilate";

impl From<BorgbaseError> for ApiError {
    fn from(e: BorgbaseError) -> Self {
        match e {
            BorgbaseError::InvalidToken => Self::BadRequest(e.to_string()),
            BorgbaseError::Unreachable(_)
            | BorgbaseError::Status(_)
            | BorgbaseError::Api(_)
            | BorgbaseError::Malformed => Self::BadGateway(e.to_string()),
        }
    }
}

/// The client for the stored token, sending requests to `url`.
async fn account_client(
    pool: &PgPool,
    key: &[u8; 32],
    url: String,
) -> Result<BorgbaseClient, ApiError> {
    let stored = db::borgbase_account::get_api_token(pool)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(
                "no BorgBase API token is stored; enter one on the BorgBase page first".to_owned(),
            )
        })?;
    let token = ApiToken::parse(&decrypt_passphrase(&stored, key)?)?;
    Ok(BorgbaseClient::new(url, token)?)
}

/// Checks `entered` against `BorgBase`, then stores it encrypted.
async fn store_token(
    pool: &PgPool,
    key: &[u8; 32],
    url: String,
    entered: &str,
) -> Result<(), ApiError> {
    let token = ApiToken::parse(entered)?;
    // Listing repositories needs the least permission any token has, so a token that cannot do
    // it is not one this server can use.
    BorgbaseClient::new(url, token.clone())?
        .list_repos()
        .await?;
    let encrypted = encrypt_passphrase(token.expose(), key)?;
    db::borgbase_account::set_api_token(pool, &encrypted).await?;
    Ok(())
}

/// Creates the repository, adding the server's key to the account first if it is not there.
async fn create_repository(
    client: &BorgbaseClient,
    server_public_key: &str,
    req: &CreateBorgbaseRepoRequest,
) -> Result<CreatedBorgbaseRepo, ApiError> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "the repository needs a name without control characters".to_owned(),
        ));
    }
    let key = client.ensure_key(KEY_NAME, server_public_key).await?;
    let created = client
        .create_repo(name, req.region, &key.id, req.access)
        .await?;
    Ok(CreatedBorgbaseRepo {
        ssh_host: shared::borgbase::ssh_host(&created.id),
        ssh_user: created.id.clone(),
        repo_path: shared::borgbase::REPO_PATH.to_owned(),
        id: created.id,
        name: created.name,
    })
}

#[utoipa::path(
    get,
    path = "/api/borgbase",
    tag = "BorgBase",
    operation_id = "getBorgbaseStatus",
    responses(
        (status = 200, description = "Whether a token is stored", body = BorgbaseStatus),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Whether a `BorgBase` API token is stored.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_status(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<BorgbaseStatus>, ApiError> {
    let token_set = db::borgbase_account::get_api_token(&state.pool)
        .await?
        .is_some();
    Ok(Json(BorgbaseStatus { token_set }))
}

#[utoipa::path(
    put,
    path = "/api/borgbase/token",
    tag = "BorgBase",
    operation_id = "setBorgbaseToken",
    request_body = SetBorgbaseTokenRequest,
    responses(
        (status = 200, description = "The token works and is stored", body = BorgbaseStatus),
        (status = 400, description = "The token is malformed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 502, description = "BorgBase refused the token or could not be reached"),
    )
)]
/// Stores the `BorgBase` API token, encrypted, after checking that `BorgBase` accepts it.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the token is malformed, [`ApiError::BadGateway`] if
/// `BorgBase` refuses it or cannot be reached, or [`ApiError::Database`] if storing it fails.
pub async fn set_token(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Extension(BorgbaseApiUrl(url)): Extension<BorgbaseApiUrl>,
    ApiJson(req): ApiJson<SetBorgbaseTokenRequest>,
) -> Result<Json<BorgbaseStatus>, ApiError> {
    store_token(&state.pool, &state.encryption_key, url, &req.token).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        None,
        AuditEvent::SetBorgbaseToken {},
    )
    .await;
    Ok(Json(BorgbaseStatus { token_set: true }))
}

#[utoipa::path(
    delete,
    path = "/api/borgbase/token",
    tag = "BorgBase",
    operation_id = "deleteBorgbaseToken",
    responses(
        (status = 204, description = "The token is removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "No token is stored"),
    )
)]
/// Removes the stored `BorgBase` API token.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no token is stored, or [`ApiError::Database`] if the query
/// fails.
pub async fn delete_token(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
) -> Result<StatusCode, ApiError> {
    if !db::borgbase_account::clear_api_token(&state.pool).await? {
        return Err(ApiError::NotFound(
            "no BorgBase API token is stored".to_owned(),
        ));
    }
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        None,
        AuditEvent::DeleteBorgbaseToken {},
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/borgbase/repos",
    tag = "BorgBase",
    operation_id = "listBorgbaseRepos",
    responses(
        (status = 200, description = "The account's repositories", body = Vec<BorgbaseRepo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 409, description = "No API token is stored"),
        (status = 502, description = "BorgBase refused the request or could not be reached"),
    )
)]
/// The repositories on the `BorgBase` account, with their quota and usage.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if no token is stored, or [`ApiError::BadGateway`] if
/// `BorgBase` refuses the request or cannot be reached.
pub async fn list_repos(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Extension(BorgbaseApiUrl(url)): Extension<BorgbaseApiUrl>,
) -> Result<Json<Vec<BorgbaseRepo>>, ApiError> {
    let client = account_client(&state.pool, &state.encryption_key, url).await?;
    Ok(Json(client.list_repos().await?))
}

#[utoipa::path(
    post,
    path = "/api/borgbase/repos",
    tag = "BorgBase",
    operation_id = "createBorgbaseRepo",
    request_body = CreateBorgbaseRepoRequest,
    responses(
        (status = 200, description = "The new repository", body = CreatedBorgbaseRepo),
        (status = 400, description = "The name is invalid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 409, description = "No API token is stored"),
        (status = 502, description = "BorgBase refused the request or could not be reached"),
    )
)]
/// Creates a repository on `BorgBase` that the server's SSH key may use, adding the key to the
/// account first if it is not there.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the name is invalid, [`ApiError::Conflict`] if no token
/// is stored, or [`ApiError::BadGateway`] if `BorgBase` refuses the request or cannot be
/// reached.
pub async fn create_repo(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Extension(BorgbaseApiUrl(url)): Extension<BorgbaseApiUrl>,
    ApiJson(req): ApiJson<CreateBorgbaseRepoRequest>,
) -> Result<Json<CreatedBorgbaseRepo>, ApiError> {
    let client = account_client(&state.pool, &state.encryption_key, url).await?;
    let public_key = ssh::read_server_public_key()
        .await
        .map_err(|e| ApiError::Internal(format!("failed to read the SSH public key: {e}")))?;
    let created = create_repository(&client, &public_key, &req).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        None,
        AuditEvent::CreateBorgbaseRepo {
            name: created.name.clone(),
        },
    )
    .await;
    Ok(Json(created))
}

#[cfg(test)]
mod tests;
