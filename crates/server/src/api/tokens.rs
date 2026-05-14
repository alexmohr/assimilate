// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::helpers;
use crate::{
    AppState,
    api::auth::{AuthUser, Role},
    db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTokenRequest {
    pub name: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreateTokenResponse {
    pub token: db::ApiTokenRow,
    pub plaintext: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ListTokensResponse {
    pub tokens: Vec<db::ApiTokenRow>,
}

fn generate_token() -> String {
    helpers::generate_random_hex(32)
}

pub fn hash_token(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

#[utoipa::path(
    post,
    path = "/api/tokens",
    tag = "API Tokens",
    operation_id = "create_token",
    summary = "Create a new API token for the current user",
    request_body = CreateTokenRequest,
    responses(
        (status = 200, description = "Token created", body = CreateTokenResponse),
        (status = 400, description = "Invalid token name"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn create_token(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<CreateTokenRequest>,
) -> Result<Json<CreateTokenResponse>, ApiError> {
    helpers::validate_non_empty(req.name.trim(), "token name")?;

    let plaintext = generate_token();
    let token_hash = hash_token(&plaintext);

    let token =
        db::insert_api_token(&state.pool, auth.user_id, req.name.trim(), &token_hash).await?;

    Ok(Json(CreateTokenResponse { token, plaintext }))
}

#[utoipa::path(
    get,
    path = "/api/tokens",
    tag = "API Tokens",
    operation_id = "list_tokens",
    summary = "List API tokens (all tokens for admin, own tokens for regular users)",
    responses(
        (status = 200, description = "List of tokens", body = ListTokensResponse),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn list_tokens(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ListTokensResponse>, ApiError> {
    let tokens = if auth.role == Role::Admin {
        db::list_all_api_tokens(&state.pool).await?
    } else {
        db::list_api_tokens_for_user(&state.pool, auth.user_id).await?
    };

    Ok(Json(ListTokensResponse { tokens }))
}

#[utoipa::path(
    delete,
    path = "/api/tokens/{id}",
    tag = "API Tokens",
    operation_id = "delete_token",
    summary = "Delete an API token",
    params(
        ("id" = i64, Path, description = "Token ID"),
    ),
    responses(
        (status = 200, description = "Token deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Cannot delete another user's token"),
        (status = 404, description = "Token not found"),
        (status = 500, description = "Internal server error"),
    )
)]
pub async fn delete_token(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if auth.role != Role::Admin {
        let owner_id = db::get_api_token_owner(&state.pool, id).await?;
        if owner_id != auth.user_id {
            return Err(ApiError::Forbidden(
                "cannot delete another user's token".to_string(),
            ));
        }
    }

    db::delete_api_token(&state.pool, id).await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
