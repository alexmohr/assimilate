// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `BorgBase` endpoints against a local stand-in for the `BorgBase` API.

use axum::http::StatusCode;
use serde_json::{Value, json};
use shared::borgbase::{BorgbaseKeyAccess, BorgbaseRegion};
use sqlx::PgPool;

use super::*;
use crate::{
    api::auth::AuthUser,
    test_support::{audit_entries, build_test_state, start_graphql_stand_in},
};

const PUBLIC_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIserver server@assimilate";

fn admin() -> RequireAdmin {
    RequireAdmin(AuthUser {
        user_id: 1,
        username: "admin".to_owned(),
        session_id: None,
    })
}

fn url_of(url: &str) -> Extension<BorgbaseApiUrl> {
    Extension(BorgbaseApiUrl(url.to_owned()))
}

fn query_of(body: &Value) -> &str {
    body.get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// A stand-in that knows one repository and the account's keys, and accepts new ones.
fn account_with_one_repo(body: &Value) -> (StatusCode, Value) {
    let query = query_of(body);
    let data = if query.contains("sshAdd") {
        json!({"sshAdd": {"keyAdded": {"id": "9", "name": "assimilate", "keyData": PUBLIC_KEY}}})
    } else if query.contains("sshList") {
        json!({"sshList": []})
    } else if query.contains("repoAdd") {
        json!({"repoAdd": {"repoAdded": {"id": "ef56", "name": "offsite"}}})
    } else {
        json!({"repoList": [{"id": "ab12", "name": "laptop", "quota": 500, "quotaEnabled": true,
            "currentUsage": 12.5, "lastModified": null}]})
    };
    (StatusCode::OK, json!({ "data": data }))
}

async fn state_with_token(pool: PgPool, url: &str) -> AppState {
    let state = build_test_state(pool, b"borgbase-api-test");
    let _ = set_token(
        State(state.clone()),
        admin(),
        ClientIp(None),
        url_of(url),
        ApiJson(SetBorgbaseTokenRequest {
            token: "secret-token".to_owned(),
        }),
    )
    .await
    .unwrap();
    state
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_token_is_checked_then_stored_encrypted_and_audited(pool: PgPool) {
    let (url, seen) = start_graphql_stand_in(account_with_one_repo).await;
    let state = build_test_state(pool.clone(), b"borgbase-api-test");

    let Json(before) = get_status(State(state.clone()), admin()).await.unwrap();
    assert!(!before.token_set);

    let Json(after) = set_token(
        State(state.clone()),
        admin(),
        ClientIp(None),
        url_of(&url),
        ApiJson(SetBorgbaseTokenRequest {
            token: "  secret-token  ".to_owned(),
        }),
    )
    .await
    .unwrap();

    assert!(after.token_set);
    assert_eq!(
        seen.lock().unwrap().first().unwrap().0.as_deref(),
        Some("Bearer secret-token")
    );
    let stored = db::borgbase_account::get_api_token(&pool)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.windows(12).any(|w| w == b"secret-token"));
    assert_eq!(
        decrypt_passphrase(&stored, &state.encryption_key).unwrap(),
        "secret-token"
    );
    let entries = audit_entries(&pool).await;
    assert!(
        entries
            .iter()
            .any(|e| e.event == AuditEvent::SetBorgbaseToken {})
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_token_borgbase_refuses_is_not_stored(pool: PgPool) {
    let (url, _) = start_graphql_stand_in(|_| (StatusCode::UNAUTHORIZED, json!({}))).await;
    let state = build_test_state(pool.clone(), b"borgbase-api-test");

    let error = set_token(
        State(state),
        admin(),
        ClientIp(None),
        url_of(&url),
        ApiJson(SetBorgbaseTokenRequest {
            token: "wrong".to_owned(),
        }),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, ApiError::BadGateway(_)));
    assert!(
        db::borgbase_account::get_api_token(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_malformed_token_is_a_bad_request(pool: PgPool) {
    let state = build_test_state(pool, b"borgbase-api-test");

    let error = set_token(
        State(state),
        admin(),
        ClientIp(None),
        url_of("http://127.0.0.1:1/graphql"),
        ApiJson(SetBorgbaseTokenRequest {
            token: "two words".to_owned(),
        }),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, ApiError::BadRequest(_)));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn without_a_token_the_account_endpoints_conflict(pool: PgPool) {
    let state = build_test_state(pool, b"borgbase-api-test");

    let listed = list_repos(
        State(state.clone()),
        admin(),
        url_of("http://127.0.0.1:1/graphql"),
    )
    .await
    .unwrap_err();
    let created = create_repo(
        State(state),
        admin(),
        ClientIp(None),
        url_of("http://127.0.0.1:1/graphql"),
        ApiJson(CreateBorgbaseRepoRequest {
            name: "offsite".to_owned(),
            region: BorgbaseRegion::Eu,
            access: BorgbaseKeyAccess::AppendOnly,
        }),
    )
    .await
    .unwrap_err();

    assert!(matches!(listed, ApiError::Conflict(_)));
    assert!(matches!(created, ApiError::Conflict(_)));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn repositories_are_listed_with_their_host_and_usage(pool: PgPool) {
    let (url, _) = start_graphql_stand_in(account_with_one_repo).await;
    let state = state_with_token(pool, &url).await;

    let Json(repos) = list_repos(State(state), admin(), url_of(&url))
        .await
        .unwrap();

    let repo = repos.first().unwrap();
    assert_eq!(repo.id, "ab12");
    assert_eq!(repo.ssh_host, "ab12.repo.borgbase.com");
    assert_eq!(repo.usage, Some(12.5));
    assert_eq!(repo.quota, Some(500.0));
}

#[tokio::test]
async fn a_repository_is_created_for_the_servers_key() {
    let (url, seen) = start_graphql_stand_in(account_with_one_repo).await;
    let client = BorgbaseClient::new(url, ApiToken::parse("secret-token").unwrap()).unwrap();

    let created = create_repository(
        &client,
        PUBLIC_KEY,
        &CreateBorgbaseRepoRequest {
            name: "  offsite  ".to_owned(),
            region: BorgbaseRegion::Eu,
            access: BorgbaseKeyAccess::AppendOnly,
        },
    )
    .await
    .unwrap();

    assert_eq!(created.id, "ef56");
    assert_eq!(created.ssh_user, "ef56");
    assert_eq!(created.ssh_host, "ef56.repo.borgbase.com");
    assert_eq!(created.repo_path, "./repo");
    let log = seen.lock().unwrap();
    let add = log
        .iter()
        .find(|(_, body)| query_of(body).contains("repoAdd"))
        .unwrap();
    assert_eq!(add.1.pointer("/variables/name"), Some(&json!("offsite")));
    assert_eq!(
        add.1.pointer("/variables/appendOnlyKeys"),
        Some(&json!(["9"]))
    );
}

#[tokio::test]
async fn a_repository_needs_a_clean_name() {
    let client = BorgbaseClient::new(
        "http://127.0.0.1:1/graphql".to_owned(),
        ApiToken::parse("secret-token").unwrap(),
    )
    .unwrap();

    for name in ["", "   ", "bad\nname"] {
        let error = create_repository(
            &client,
            PUBLIC_KEY,
            &CreateBorgbaseRepoRequest {
                name: name.to_owned(),
                region: BorgbaseRegion::Us,
                access: BorgbaseKeyAccess::Full,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ApiError::BadRequest(_)), "name {name:?}");
    }
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn removing_the_token_forgets_it_and_is_audited(pool: PgPool) {
    let (url, _) = start_graphql_stand_in(account_with_one_repo).await;
    let state = state_with_token(pool.clone(), &url).await;

    let status = delete_token(State(state.clone()), admin(), ClientIp(None))
        .await
        .unwrap();
    let again = delete_token(State(state.clone()), admin(), ClientIp(None))
        .await
        .unwrap_err();

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(matches!(again, ApiError::NotFound(_)));
    let Json(after) = get_status(State(state), admin()).await.unwrap();
    assert!(!after.token_set);
    assert!(
        audit_entries(&pool)
            .await
            .iter()
            .any(|e| e.event == AuditEvent::DeleteBorgbaseToken {})
    );
}

#[test]
fn borgbase_failures_map_to_the_right_http_errors() {
    assert!(matches!(
        ApiError::from(BorgbaseError::InvalidToken),
        ApiError::BadRequest(_)
    ));
    for error in [
        BorgbaseError::Unreachable("down".to_owned()),
        BorgbaseError::Status(500),
        BorgbaseError::Api("no".to_owned()),
        BorgbaseError::Malformed,
    ] {
        assert!(matches!(ApiError::from(error), ApiError::BadGateway(_)));
    }
}
