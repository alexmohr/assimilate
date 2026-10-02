// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use shared::{
    audit::AuditEvent,
    responses::{
        ApiTokenResponse, CreateApiTokenResponse, DeleteApiTokenResponse, ListApiTokensResponse,
    },
};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    helpers,
};
use crate::{
    AppState,
    api::auth::AuthUser,
    db,
    error::{ApiError, ApiJson},
};

/// Request payload for creating a new API token.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTokenRequest {
    /// Human-readable name for the token.
    pub name: String,
}

impl From<db::ApiTokenRow> for ApiTokenResponse {
    fn from(row: db::ApiTokenRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            last_used_at: row.last_used_at,
            created_at: row.created_at,
        }
    }
}

fn generate_token() -> String {
    helpers::generate_random_hex(32)
}

/// Hash a plaintext token using SHA-256.
#[must_use]
pub fn hash_token(plaintext: &str) -> String {
    use std::fmt::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    let result = hasher.finalize();
    result.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

#[utoipa::path(
    post,
    path = "/api/tokens",
    tag = "API Tokens",
    operation_id = "create_token",
    request_body = CreateTokenRequest,
    responses(
        (status = 200, description = "Token created", body = CreateApiTokenResponse),
        (status = 400, description = "Invalid token name"),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
/// Create a new API token for the current user.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn create_token(
    State(state): State<AppState>,
    auth: AuthUser,
    ip: ClientIp,
    ApiJson(req): ApiJson<CreateTokenRequest>,
) -> Result<Json<CreateApiTokenResponse>, ApiError> {
    helpers::validate_non_empty(req.name.trim(), "token name")?;
    helpers::validate_max_len(req.name.trim(), "token name", helpers::MaxLen::Name)?;

    let plaintext = generate_token();
    let token_hash = hash_token(&plaintext);

    let token: ApiTokenResponse =
        db::insert_api_token(&state.pool, auth.user_id, req.name.trim(), &token_hash)
            .await?
            .into();
    audit_trail::record(
        &state.pool,
        Actor::new(&auth, ip),
        Some(AuditTarget::ApiToken(token.id)),
        AuditEvent::CreateApiToken {
            name: token.name.clone(),
        },
    )
    .await;

    Ok(Json(CreateApiTokenResponse { token, plaintext }))
}

#[utoipa::path(
    get,
    path = "/api/tokens",
    tag = "API Tokens",
    operation_id = "list_tokens",
    responses(
        (status = 200, description = "List of tokens", body = ListApiTokensResponse),
        (status = 401, description = "Not authenticated"),
        (status = 500, description = "Internal server error"),
    )
)]
/// List API tokens (all tokens for admin, own tokens for regular users).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_tokens(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ListApiTokensResponse>, ApiError> {
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let token_rows = if effective.can_delete_repo {
        db::list_all_api_tokens(&state.pool).await?
    } else {
        db::list_api_tokens_for_user(&state.pool, auth.user_id).await?
    };

    let tokens: Vec<ApiTokenResponse> = token_rows.into_iter().map(Into::into).collect();
    Ok(Json(ListApiTokensResponse { tokens }))
}

#[utoipa::path(
    delete,
    path = "/api/tokens/{id}",
    tag = "API Tokens",
    operation_id = "delete_token",
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
/// Delete an API token.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller lacks permission for this operation.
pub async fn delete_token(
    State(state): State<AppState>,
    auth: AuthUser,
    ip: ClientIp,
    Path(id): Path<i64>,
) -> Result<Json<DeleteApiTokenResponse>, ApiError> {
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if !effective.can_delete_repo {
        let owner_id = db::get_api_token_owner(&state.pool, id).await?;
        if owner_id != auth.user_id {
            return Err(ApiError::Forbidden(
                "cannot delete another user's token".to_string(),
            ));
        }
    }

    let token = db::delete_api_token(&state.pool, id).await?;
    let owner = db::get_user_by_id(&state.pool, token.user_id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&auth, ip),
        Some(AuditTarget::ApiToken(id)),
        AuditEvent::DeleteApiToken {
            name: token.name,
            owner: owner.username,
        },
    )
    .await;
    Ok(Json(DeleteApiTokenResponse { deleted: true }))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{audit_entries, audit_events, build_test_state, insert_auth_user};

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn creating_and_revoking_a_token_is_audited_without_the_token(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"tokens-audit-test-key");
        let user = insert_auth_user(&pool, "token-owner").await;

        let Json(created) = create_token(
            State(state.clone()),
            user.clone(),
            ClientIp::default(),
            ApiJson(CreateTokenRequest {
                name: " ci-pipeline ".to_owned(),
            }),
        )
        .await
        .unwrap();
        let Json(_) = delete_token(
            State(state),
            user.clone(),
            ClientIp::default(),
            Path(created.token.id),
        )
        .await
        .unwrap();

        assert_eq!(
            audit_events(&pool).await,
            [
                AuditEvent::DeleteApiToken {
                    name: "ci-pipeline".to_owned(),
                    owner: "token-owner".to_owned(),
                },
                AuditEvent::CreateApiToken {
                    name: "ci-pipeline".to_owned()
                },
            ]
        );
        let stored = serde_json::to_string(&audit_entries(&pool).await).unwrap();
        assert!(
            !stored.contains(&created.plaintext),
            "the token itself must never reach the audit log"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_forbidden_revocation_is_not_audited(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"tokens-audit-test-key");
        let owner = insert_auth_user(&pool, "owner").await;
        let other = insert_auth_user(&pool, "other").await;
        let token = db::insert_api_token(&pool, owner.user_id, "owned", "hash")
            .await
            .unwrap();

        let result = delete_token(State(state), other, ClientIp::default(), Path(token.id)).await;

        assert!(matches!(result, Err(ApiError::Forbidden(_))));
        assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
    }
}
