// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Run with: `DATABASE_URL=postgres://... cargo test -p server --test integration -- --ignored`

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{delete, get, post, put},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::{Service, ServiceExt};

const TEST_SESSION_ID: &str = "test-integration-session-id-00000000";

async fn oneshot(app: &mut Router, req: Request<Body>) -> axum::response::Response {
    ServiceExt::<Request<Body>>::ready(app)
        .await
        .unwrap()
        .call(req)
        .await
        .unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

async fn build_test_app(pool: PgPool) -> Router {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration");
    let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
    let server_addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let tunnel_manager =
        server::tunnel::TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr);
    let state = server::AppState {
        pool: pool.clone(),
        encryption_key,
        registry: server::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer: server::log_buffer::LogBuffer::default(),
        notification_service: server::notifications::NotificationService::new(
            pool,
            reqwest::Client::new(),
        ),
        pending_dryruns: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        pending_restores: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        pending_migrations: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        pending_deletes: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
    };

    Router::new()
        .route("/api/health", get(server::api::health::health))
        .route("/api/auth/login", post(server::api::auth::login))
        .route("/api/auth/logout", post(server::api::auth::logout))
        .route("/api/auth/me", get(server::api::auth::me))
        .route(
            "/api/users",
            get(server::api::users::list_users).post(server::api::users::create_user),
        )
        .route("/api/users/{id}/role", put(server::api::users::update_role))
        .route("/api/users/{id}", delete(server::api::users::delete_user))
        .route(
            "/api/clients",
            get(server::api::clients::list_clients).post(server::api::clients::create_client),
        )
        .route(
            "/api/clients/{hostname}",
            get(server::api::clients::get_client)
                .put(server::api::clients::update_client)
                .delete(server::api::clients::delete_client),
        )
        .route("/api/repos", get(server::api::repos::list_repos))
        .route(
            "/api/repos/{repo_id}",
            get(server::api::repos::get_repo)
                .put(server::api::repos::update_repo)
                .delete(server::api::repos::delete_repo),
        )
        .route(
            "/api/excludes",
            get(server::api::excludes::list_excludes).post(server::api::excludes::create_exclude),
        )
        .route(
            "/api/excludes/{id}",
            put(server::api::excludes::update_exclude)
                .delete(server::api::excludes::delete_exclude),
        )
        .route(
            "/api/schedules",
            get(server::api::schedules::list_schedules),
        )
        .route(
            "/api/clients/{hostname}/reports",
            get(server::api::reports::list_reports),
        )
        .route("/api/stats/storage", get(server::api::stats::storage))
        .route("/api/stats/activity", get(server::api::stats::activity))
        .route("/api/stats/health", get(server::api::stats::health))
        .route("/api/audit-log", get(server::api::audit::list_audit_log))
        .route(
            "/api/notifications/channels",
            get(server::api::notifications::list_channels)
                .post(server::api::notifications::create_channel),
        )
        .route(
            "/api/notifications/channels/{id}",
            put(server::api::notifications::update_channel)
                .delete(server::api::notifications::delete_channel),
        )
        .route(
            "/api/notifications/rules",
            get(server::api::notifications::list_rules)
                .post(server::api::notifications::create_rule),
        )
        .route(
            "/api/notifications/rules/{id}",
            delete(server::api::notifications::delete_rule),
        )
        .route(
            "/api/tunnels",
            get(server::api::tunnels::list_tunnels).post(server::api::tunnels::create_tunnel),
        )
        .route(
            "/api/tunnels/{id}",
            get(server::api::tunnels::get_tunnel)
                .put(server::api::tunnels::update_tunnel)
                .delete(server::api::tunnels::delete_tunnel),
        )
        .with_state(state)
}

async fn setup_pool() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let pool = PgPool::connect(&database_url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn create_test_user_and_session(pool: &PgPool) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, role) VALUES ('integration-admin', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', 'admin') ON CONFLICT \
         (username) DO UPDATE SET username = EXCLUDED.username RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let expires = chrono::Utc::now() + chrono::Duration::hours(24);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO \
         UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(TEST_SESSION_ID)
    .bind(user_id)
    .bind(expires)
    .execute(pool)
    .await
    .unwrap();
}

async fn clean_tables(pool: &PgPool) {
    sqlx::query("DELETE FROM backup_reports")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM schedules")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM excludes_global")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM repos")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM clients")
        .execute(pool)
        .await
        .unwrap();
}

/// Inserts a repo directly into DB, bypassing the API (which requires SSH connectivity).
async fn insert_test_repo(pool: &PgPool, name: &str) -> i64 {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration");
    let passphrase_encrypted = shared::crypto::encrypt_passphrase("test-pass", &encryption_key)
        .expect("encryption should not fail");
    sqlx::query_scalar(
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(name)
    .bind("/backups/test")
    .bind("backup")
    .bind("storage.local")
    .bind(22_i32)
    .bind(&passphrase_encrypted)
    .bind("lz4")
    .bind("repokey")
    .fetch_one(pool)
    .await
    .unwrap()
}

fn session_cookie() -> String {
    format!("session={TEST_SESSION_ID}")
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("content-type", "application/json")
        .header("cookie", session_cookie());
    match body {
        Some(val) => builder
            .body(Body::from(serde_json::to_vec(&val).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .header("cookie", session_cookie())
        .body(Body::empty())
        .unwrap()
}

fn delete_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("cookie", session_cookie())
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn test_client_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({
            "hostname": "test-host-1",
            "display_name": "Test Host 1"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["client"]["hostname"], "test-host-1");
    assert_eq!(body["client"]["display_name"], "Test Host 1");
    assert!(body["token"].as_str().is_some_and(|t| t.len() == 64));

    let req = get_request("/api/clients/test-host-1");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_object());
    assert_eq!(body["hostname"], "test-host-1");
}

#[tokio::test]
#[ignore]
async fn test_notification_channels_list() {
    let pool = setup_pool().await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/notifications/channels");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array());
}

#[tokio::test]
#[ignore]
async fn test_notification_channel_create_webhook() {
    let pool = setup_pool().await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/notifications/channels",
        Some(json!({
            "name": "test-webhook",
            "channel_type": "webhook",
            "config": {
                "url": "https://hooks.example.com/notify"
            }
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "test-webhook");
    assert_eq!(body["channel_type"], "webhook");
}

#[tokio::test]
#[ignore]
async fn test_tunnels_list() {
    let pool = setup_pool().await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/tunnels");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array());
}

#[tokio::test]
#[ignore]
async fn test_tunnel_create() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, display_name, agent_token_hash) VALUES ('tunnel-host', \
         'Tunnel Host', 'fakehash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let req = json_request(
        "POST",
        "/api/tunnels",
        Some(json!({
            "client_id": client_id,
            "ssh_host": "remote.example.com",
            "ssh_user": "backup",
            "ssh_port": 22,
            "tunnel_port": 2222,
            "enabled": false
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["ssh_host"], "remote.example.com");
    assert_eq!(body["tunnel_port"], 2222);
}

#[tokio::test]
#[ignore]
async fn test_delete_client() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "to-delete" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = delete_request("/api/clients/to-delete");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request("/api/clients/to-delete");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore]
async fn test_repo_update() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "update-repo").await;

    let req = json_request(
        "PUT",
        &format!("/api/repos/{repo_id}"),
        Some(json!({
            "repo_path": "/backups/test",
            "ssh_host": "storage.local",
            "compression": "zstd"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["compression"], "zstd");
}

#[tokio::test]
#[ignore]
async fn test_repo_delete() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "delete-repo").await;

    let req = delete_request(&format!("/api/repos/{repo_id}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request(&format!("/api/repos/{repo_id}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore]
async fn test_auth_me_without_session() {
    let pool = setup_pool().await;
    let mut app = build_test_app(pool.clone()).await;

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
