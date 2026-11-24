// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Enrollment tokens: create, list and revoke the single-use tokens a new host
//! trades for an agent token. The trade itself happens on the agent channel
//! (see `ws::handler::enroll`).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use shared::{
    audit::AuditEvent,
    enrollment::{
        CreateEnrollmentTokenRequest, CreateEnrollmentTokenResponse,
        DEFAULT_ENROLLMENT_LIFETIME_SECS, ENROLLMENT_TOKEN_BYTES, ENROLLMENT_TOKEN_HINT_LEN,
        ENROLLMENT_TOKEN_PREFIX, ListEnrollmentTokensResponse, MAX_ENROLLMENT_LIFETIME_SECS,
        RevokeEnrollmentTokenResponse,
    },
    time::{SignedDuration, Timestamp},
};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    auth::RequireAdmin,
    helpers::{self, MaxLen},
    tokens::hash_token,
};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

/// The shortest lifetime a token may be given. A token that expires before it
/// can be pasted into a shell is a mistake, not a choice.
const MIN_ENROLLMENT_LIFETIME_SECS: i64 = 60;

/// The scope under which agent tags are catalogued.
const AGENT_TAG_SCOPE: &str = "agent";

/// A fresh enrollment token: the prefix, then 32 random bytes in hex.
fn generate_enrollment_token() -> String {
    format!(
        "{ENROLLMENT_TOKEN_PREFIX}{}",
        helpers::generate_random_hex(ENROLLMENT_TOKEN_BYTES)
    )
}

/// The first of `requested` that is not among the agent tags in `known`.
fn first_unknown_tag(requested: &[i64], known: &[db::TagRow]) -> Option<i64> {
    requested
        .iter()
        .copied()
        .find(|id| !known.iter().any(|tag| tag.id == *id))
}

/// Checks a create request, returning the lifetime to give the token.
async fn validate_request(
    state: &AppState,
    req: &CreateEnrollmentTokenRequest,
) -> Result<SignedDuration, ApiError> {
    let lifetime_secs = req
        .lifetime_secs
        .unwrap_or(DEFAULT_ENROLLMENT_LIFETIME_SECS);
    if !(MIN_ENROLLMENT_LIFETIME_SECS..=MAX_ENROLLMENT_LIFETIME_SECS).contains(&lifetime_secs) {
        return Err(ApiError::BadRequest(format!(
            "a token's lifetime must be between {MIN_ENROLLMENT_LIFETIME_SECS} seconds and \
             {MAX_ENROLLMENT_LIFETIME_SECS} seconds"
        )));
    }
    if let Some(hostname) = req.hostname.as_deref().map(str::trim) {
        helpers::validate_non_empty(hostname, "hostname")?;
        helpers::validate_max_len(hostname, "hostname", MaxLen::Hostname)?;
    }
    if !req.tag_ids.is_empty() {
        let known = db::list_tags(&state.pool, AGENT_TAG_SCOPE).await?;
        if let Some(unknown) = first_unknown_tag(&req.tag_ids, &known) {
            return Err(ApiError::BadRequest(format!(
                "tag {unknown} is not an agent tag"
            )));
        }
    }
    Ok(SignedDuration::from_secs(lifetime_secs))
}

#[utoipa::path(
    post,
    path = "/api/enrollment-tokens",
    tag = "Enrollment",
    operation_id = "createEnrollmentToken",
    request_body = CreateEnrollmentTokenRequest,
    responses(
        (status = 201, description = "Token created", body = CreateEnrollmentTokenResponse),
        (status = 400, description = "Invalid lifetime, hostname or tags"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
    )
)]
/// Create a single-use enrollment token. The token is shown once.
///
/// # Errors
///
/// Returns an error if the request is invalid or the underlying operation fails.
pub async fn create_enrollment_token(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    ApiJson(req): ApiJson<CreateEnrollmentTokenRequest>,
) -> Result<(StatusCode, Json<CreateEnrollmentTokenResponse>), ApiError> {
    let lifetime = validate_request(&state, &req).await?;
    let hostname = req.hostname.as_deref().map(str::trim);

    let plaintext = generate_enrollment_token();
    let hint: String = plaintext
        .chars()
        .take(
            ENROLLMENT_TOKEN_PREFIX
                .len()
                .saturating_add(ENROLLMENT_TOKEN_HINT_LEN),
        )
        .collect();
    let now = Timestamp::now();
    let expires_at = now.saturating_add(lifetime);

    let row = db::enrollment_tokens::insert_enrollment_token(
        &state.pool,
        &db::enrollment_tokens::NewEnrollmentToken {
            token_hash: &hash_token(&plaintext),
            hint: &hint,
            created_by: admin.user_id,
            expires_at,
            hostname,
            tag_ids: &req.tag_ids,
        },
    )
    .await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::EnrollmentToken(row.id)),
        AuditEvent::CreateEnrollmentToken {
            hostname: row.hostname.clone(),
            expires_at,
        },
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateEnrollmentTokenResponse {
            token: row.into_response(now),
            plaintext,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/enrollment-tokens",
    tag = "Enrollment",
    operation_id = "listEnrollmentTokens",
    responses(
        (status = 200, description = "Tokens, newest first", body = ListEnrollmentTokensResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
    )
)]
/// List enrollment tokens with their status. Secrets are never included.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_enrollment_tokens(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<ListEnrollmentTokensResponse>, ApiError> {
    let now = Timestamp::now();
    let tokens = db::enrollment_tokens::list_enrollment_tokens(&state.pool)
        .await?
        .into_iter()
        .map(|row| row.into_response(now))
        .collect();
    Ok(Json(ListEnrollmentTokensResponse { tokens }))
}

#[utoipa::path(
    delete,
    path = "/api/enrollment-tokens/{id}",
    tag = "Enrollment",
    operation_id = "revokeEnrollmentToken",
    params(("id" = i64, Path, description = "Enrollment token ID")),
    responses(
        (status = 200, description = "Token revoked", body = RevokeEnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Token not found"),
        (status = 409, description = "The token is already used, revoked or expired"),
    )
)]
/// Revoke a token that no host has used yet.
///
/// # Errors
///
/// Returns an error if the token does not exist, is no longer pending, or the
/// underlying operation fails.
pub async fn revoke_enrollment_token(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
) -> Result<Json<RevokeEnrollmentTokenResponse>, ApiError> {
    let existing = db::enrollment_tokens::get_enrollment_token(&state.pool, id).await?;
    if !db::enrollment_tokens::revoke_enrollment_token(&state.pool, id).await? {
        return Err(ApiError::Conflict(
            "the token is already used, revoked or expired".to_owned(),
        ));
    }
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::EnrollmentToken(id)),
        AuditEvent::RevokeEnrollmentToken {
            hostname: existing.hostname,
        },
    )
    .await;
    Ok(Json(RevokeEnrollmentTokenResponse { revoked: true }))
}

#[cfg(test)]
mod tests {
    use shared::enrollment::EnrollmentTokenStatus;
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{audit_events, build_test_state, insert_auth_user};

    fn request(lifetime_secs: Option<i64>, hostname: Option<&str>) -> CreateEnrollmentTokenRequest {
        CreateEnrollmentTokenRequest {
            lifetime_secs,
            hostname: hostname.map(str::to_owned),
            tag_ids: Vec::new(),
        }
    }

    async fn create(
        state: &AppState,
        admin: &crate::api::auth::AuthUser,
        req: CreateEnrollmentTokenRequest,
    ) -> Result<CreateEnrollmentTokenResponse, ApiError> {
        create_enrollment_token(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            ApiJson(req),
        )
        .await
        .map(|(_, Json(created))| created)
    }

    fn tag(id: i64) -> db::TagRow {
        db::TagRow {
            id,
            name: format!("tag-{id}"),
            color: "#336699".to_owned(),
            scope: AGENT_TAG_SCOPE.to_owned(),
        }
    }

    #[test]
    fn a_requested_tag_that_is_not_an_agent_tag_is_found() {
        let known = [tag(1), tag(2)];

        assert_eq!(first_unknown_tag(&[2, 1], &known), None);
        assert_eq!(first_unknown_tag(&[], &known), None);
        assert_eq!(first_unknown_tag(&[2, 9, 7], &known), Some(9));
    }

    #[test]
    fn a_token_carries_the_prefix_and_32_random_bytes() {
        let token = generate_enrollment_token();

        assert!(token.starts_with(ENROLLMENT_TOKEN_PREFIX));
        assert_eq!(
            token.len(),
            ENROLLMENT_TOKEN_PREFIX.len() + ENROLLMENT_TOKEN_BYTES * 2
        );
        assert_ne!(token, generate_enrollment_token());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_created_token_is_pending_for_an_hour_and_never_listed_in_full(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;

        let created = create(&state, &admin, request(None, Some(" db-02 ")))
            .await
            .unwrap();
        let Json(listed) = list_enrollment_tokens(State(state), RequireAdmin(admin))
            .await
            .unwrap();

        assert_eq!(created.token.status, EnrollmentTokenStatus::Pending);
        assert_eq!(created.token.hostname.as_deref(), Some("db-02"));
        assert_eq!(created.token.created_by.as_deref(), Some("enroller"));
        assert!(created.plaintext.starts_with(ENROLLMENT_TOKEN_PREFIX));
        let lifetime = created.token.expires_at.since(created.token.created_at);
        assert!((3590..=3610).contains(&lifetime.as_secs()), "{lifetime:?}");
        let [entry] = listed.tokens.as_slice() else {
            panic!("expected one token, got {listed:?}");
        };
        assert_eq!(entry.id, created.token.id);
        assert!(!format!("{listed:?}").contains(&created.plaintext));
        assert!(created.plaintext.starts_with(&entry.hint));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_token_can_carry_agent_tags(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;
        let tag = db::insert_tag(&pool, "enrolled-tag", "#336699", AGENT_TAG_SCOPE)
            .await
            .unwrap();

        let created = create(
            &state,
            &admin,
            CreateEnrollmentTokenRequest {
                tag_ids: vec![tag.id],
                ..request(None, None)
            },
        )
        .await
        .unwrap();

        assert_eq!(created.token.tag_ids, vec![tag.id]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_lifetime_outside_the_allowed_range_is_rejected(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;

        for lifetime in [
            0,
            MIN_ENROLLMENT_LIFETIME_SECS - 1,
            MAX_ENROLLMENT_LIFETIME_SECS + 1,
        ] {
            let result = create(&state, &admin, request(Some(lifetime), None)).await;
            assert!(
                matches!(result, Err(ApiError::BadRequest(_))),
                "{lifetime}: {result:?}"
            );
        }
        for lifetime in [MIN_ENROLLMENT_LIFETIME_SECS, MAX_ENROLLMENT_LIFETIME_SECS] {
            create(&state, &admin, request(Some(lifetime), None))
                .await
                .unwrap();
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_blank_hostname_or_an_unknown_tag_is_rejected(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;

        let blank = create(&state, &admin, request(None, Some("  "))).await;
        let unknown_tag = create(
            &state,
            &admin,
            CreateEnrollmentTokenRequest {
                tag_ids: vec![987_654],
                ..request(None, None)
            },
        )
        .await;

        assert!(matches!(blank, Err(ApiError::BadRequest(_))), "{blank:?}");
        assert!(
            matches!(unknown_tag, Err(ApiError::BadRequest(_))),
            "{unknown_tag:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn creating_and_revoking_a_token_is_audited_without_the_token(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;

        let created = create(&state, &admin, request(None, None)).await.unwrap();
        let Json(revoked) = revoke_enrollment_token(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            Path(created.token.id),
        )
        .await
        .unwrap();
        let again = revoke_enrollment_token(
            State(state.clone()),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(created.token.id),
        )
        .await;

        assert!(revoked.revoked);
        assert!(matches!(again, Err(ApiError::Conflict(_))), "{again:?}");
        let events = audit_events(&pool).await;
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditEvent::CreateEnrollmentToken { hostname: None, .. })),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditEvent::RevokeEnrollmentToken { hostname: None })),
            "{events:?}"
        );
        assert!(!format!("{events:?}").contains(&created.plaintext));
        let Json(listed) = list_enrollment_tokens(
            State(state),
            RequireAdmin(insert_auth_user(&pool, "viewer-of-list").await),
        )
        .await
        .unwrap();
        assert_eq!(
            listed.tokens.first().map(|t| t.status),
            Some(EnrollmentTokenStatus::Revoked)
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_token_past_its_lifetime_lists_as_expired_and_cannot_be_revoked(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"enrollment-api-test-key");
        let admin = insert_auth_user(&pool, "enroller").await;
        let created = create(&state, &admin, request(None, None)).await.unwrap();
        sqlx::query!(
            "UPDATE enrollment_tokens SET expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1",
            created.token.id
        )
        .execute(&pool)
        .await
        .unwrap();

        let Json(listed) =
            list_enrollment_tokens(State(state.clone()), RequireAdmin(admin.clone()))
                .await
                .unwrap();
        let revoke = revoke_enrollment_token(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(created.token.id),
        )
        .await;

        assert_eq!(
            listed.tokens.first().map(|t| t.status),
            Some(EnrollmentTokenStatus::Expired)
        );
        assert!(matches!(revoke, Err(ApiError::Conflict(_))), "{revoke:?}");
    }
}
