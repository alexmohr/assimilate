// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The client against a stand-in `BorgBase`: a local HTTP server that records what it is sent and
//! answers from a script.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde_json::{Value, json};

use super::*;

/// The requests the stand-in has received: the `Authorization` header and the JSON body.
type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

type Responder = Arc<dyn Fn(&Value) -> (StatusCode, Value) + Send + Sync>;

#[derive(Clone)]
struct Mock {
    seen: Seen,
    respond: Responder,
}

async fn handle(
    State(mock): State<Mock>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let answer = (mock.respond)(&body);
    mock.seen.lock().unwrap().push((auth, body));
    (answer.0, Json(answer.1))
}

/// Starts the stand-in and returns a client for it and the log of requests it receives.
async fn start(
    respond: impl Fn(&Value) -> (StatusCode, Value) + Send + Sync + 'static,
) -> (BorgbaseClient, Seen) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = Mock {
        seen: Arc::clone(&seen),
        respond: Arc::new(respond),
    };
    let app = Router::new()
        .route("/graphql", post(handle))
        .with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = BorgbaseClient::new(
        format!("http://{addr}/graphql"),
        ApiToken::parse("secret-token").unwrap(),
    )
    .unwrap();
    (client, seen)
}

fn query_of(body: &Value) -> &str {
    body.get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

#[test]
fn a_token_is_a_single_word() {
    assert_eq!(ApiToken::parse("  abc123  ").unwrap().expose(), "abc123");
    assert!(matches!(
        ApiToken::parse("   "),
        Err(BorgbaseError::InvalidToken)
    ));
    assert!(matches!(
        ApiToken::parse("two words"),
        Err(BorgbaseError::InvalidToken)
    ));
    assert!(matches!(
        ApiToken::parse("line\nbreak"),
        Err(BorgbaseError::InvalidToken)
    ));
}

#[test]
fn a_token_never_shows_in_debug_output() {
    let token = ApiToken::parse("secret-token").unwrap();
    assert_eq!(format!("{token:?}"), "ApiToken([REDACTED])");
    let client = BorgbaseClient::new(DEFAULT_API_URL.to_owned(), token).unwrap();
    assert!(!format!("{client:?}").contains("secret-token"));
}

#[test]
fn the_endpoint_defaults_to_borgbase() {
    assert_eq!(DEFAULT_API_URL, "https://api.borgbase.com/graphql");
}

#[test]
fn a_key_is_identified_by_its_material_not_its_comment() {
    assert_eq!(
        key_material("ssh-ed25519 AAAAC3 server@assimilate\n"),
        "ssh-ed25519 AAAAC3"
    );
    assert_eq!(key_material("ssh-ed25519 AAAAC3"), "ssh-ed25519 AAAAC3");
}

#[tokio::test]
async fn repositories_are_listed_with_quota_and_usage() {
    let (client, seen) = start(|_| {
        (
            StatusCode::OK,
            json!({"data": {"repoList": [
                {"id": "ab12", "name": "laptop", "quota": 1000, "quotaEnabled": true,
                 "currentUsage": 12.5, "lastModified": "2026-11-20T10:00:00Z"},
                {"id": "cd34", "name": "nas", "quota": null, "quotaEnabled": false,
                 "currentUsage": null, "lastModified": null}
            ]}}),
        )
    })
    .await;

    let repos = client.list_repos().await.unwrap();

    assert_eq!(repos.len(), 2);
    let laptop = repos.first().unwrap();
    assert_eq!(laptop.id, "ab12");
    assert!(laptop.quota_enabled);
    assert_eq!(laptop.quota, Some(1000.0));
    assert_eq!(laptop.usage, Some(12.5));
    assert_eq!(laptop.ssh_host, "ab12.repo.borgbase.com");
    let nas = repos.get(1).unwrap();
    assert_eq!(nas.quota, None);
    assert_eq!(nas.usage, None);
    let log = seen.lock().unwrap();
    assert_eq!(
        log.first().unwrap().0.as_deref(),
        Some("Bearer secret-token")
    );
}

#[tokio::test]
async fn an_existing_key_is_reused_not_added_twice() {
    let (client, seen) = start(|body| {
        assert!(query_of(body).contains("sshList"));
        (
            StatusCode::OK,
            json!({"data": {"sshList": [
                {"id": "7", "name": "old name", "keyData": "ssh-ed25519 AAAAC3 someone-else"}
            ]}}),
        )
    })
    .await;

    let key = client
        .ensure_key("assimilate", "ssh-ed25519 AAAAC3 server@assimilate")
        .await
        .unwrap();

    assert_eq!(key.id, "7");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_missing_key_is_added() {
    let (client, seen) = start(|body| {
        if query_of(body).contains("sshAdd") {
            (
                StatusCode::OK,
                json!({"data": {"sshAdd": {"keyAdded":
                    {"id": "9", "name": "assimilate", "keyData": "ssh-ed25519 AAAAC3"}}}}),
            )
        } else {
            (StatusCode::OK, json!({"data": {"sshList": []}}))
        }
    })
    .await;

    let key = client
        .ensure_key("assimilate", "ssh-ed25519 AAAAC3 server@assimilate\n")
        .await
        .unwrap();

    assert_eq!(key.id, "9");
    let log = seen.lock().unwrap();
    let add = log.get(1).unwrap();
    assert_eq!(
        add.1.pointer("/variables/keyData"),
        Some(&json!("ssh-ed25519 AAAAC3 server@assimilate"))
    );
    assert_eq!(add.1.pointer("/variables/name"), Some(&json!("assimilate")));
}

#[tokio::test]
async fn an_append_only_repository_gives_the_key_append_only_access() {
    let (client, seen) = start(|_| {
        (
            StatusCode::OK,
            json!({"data": {"repoAdd": {"repoAdded": {"id": "ef56", "name": "offsite"}}}}),
        )
    })
    .await;

    let repo = client
        .create_repo(
            "offsite",
            BorgbaseRegion::Eu,
            "9",
            BorgbaseKeyAccess::AppendOnly,
        )
        .await
        .unwrap();

    assert_eq!(repo.id, "ef56");
    let log = seen.lock().unwrap();
    let vars = log.first().unwrap().1.get("variables").unwrap();
    assert_eq!(vars.get("region"), Some(&json!("eu")));
    assert_eq!(vars.get("appendOnlyKeys"), Some(&json!(["9"])));
    assert_eq!(vars.get("fullAccessKeys"), Some(&json!([])));
}

#[tokio::test]
async fn a_full_access_repository_gives_the_key_full_access() {
    let (client, seen) = start(|_| {
        (
            StatusCode::OK,
            json!({"data": {"repoAdd": {"repoAdded": {"id": "ef56", "name": "offsite"}}}}),
        )
    })
    .await;

    client
        .create_repo("offsite", BorgbaseRegion::Us, "9", BorgbaseKeyAccess::Full)
        .await
        .unwrap();

    let log = seen.lock().unwrap();
    let vars = log.first().unwrap().1.get("variables").unwrap();
    assert_eq!(vars.get("region"), Some(&json!("us")));
    assert_eq!(vars.get("appendOnlyKeys"), Some(&json!([])));
    assert_eq!(vars.get("fullAccessKeys"), Some(&json!(["9"])));
}

#[tokio::test]
async fn a_failure_status_names_the_status_but_not_the_token() {
    let (client, _) = start(|_| (StatusCode::UNAUTHORIZED, json!({"detail": "no"}))).await;

    let error = client.list_repos().await.unwrap_err();

    assert!(matches!(error, BorgbaseError::Status(401)));
    assert!(!error.to_string().contains("secret-token"));
}

#[tokio::test]
async fn graphql_errors_are_reported_with_their_messages() {
    let (client, _) = start(|_| {
        (
            StatusCode::OK,
            json!({"errors": [{"message": "name taken"}, {"message": "bad region"}]}),
        )
    })
    .await;

    let error = client
        .create_repo("x", BorgbaseRegion::Eu, "1", BorgbaseKeyAccess::Full)
        .await
        .unwrap_err();

    assert!(matches!(&error, BorgbaseError::Api(m) if m == "name taken; bad region"));
}

#[tokio::test]
async fn an_answer_in_the_wrong_shape_is_malformed() {
    let (client, _) = start(|_| (StatusCode::OK, json!({"data": {"unexpected": 1}}))).await;

    assert!(matches!(
        client.list_repos().await,
        Err(BorgbaseError::Malformed)
    ));
}

#[tokio::test]
async fn an_unreachable_api_is_reported_without_the_token() {
    let client = BorgbaseClient::new(
        "http://127.0.0.1:1/graphql".to_owned(),
        ApiToken::parse("secret-token").unwrap(),
    )
    .unwrap();

    let error = client.list_repos().await.unwrap_err();

    assert!(matches!(error, BorgbaseError::Unreachable(_)));
    assert!(!error.to_string().contains("secret-token"));
}
