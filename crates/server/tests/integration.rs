// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Run with: `DATABASE_URL=postgres://... cargo test -p server --test integration -- --ignored`

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post, put},
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
    };

    Router::new()
        .route("/api/health", get(server::api::health::health))
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
            "/api/schedules/{id}/clone",
            post(server::api::schedules::clone_schedule),
        )
        .route(
            "/api/clients/{hostname}/reports",
            get(server::api::reports::list_reports),
        )
        .route("/api/stats/storage", get(server::api::stats::storage))
        .route("/api/stats/activity", get(server::api::stats::activity))
        .route("/api/stats/health", get(server::api::stats::health))
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
    assert_eq!(body["hostname"], "test-host-1");
    assert_eq!(body["display_name"], "Test Host 1");

    let req = get_request("/api/clients");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let clients = body.as_array().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0]["hostname"], "test-host-1");

    let req = json_request(
        "PUT",
        "/api/clients/test-host-1",
        Some(json!({ "display_name": "Updated Host 1" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["display_name"], "Updated Host 1");

    let req = delete_request("/api/clients/test-host-1");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request("/api/clients/test-host-1");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore]
async fn test_repos_list() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    insert_test_repo(&pool, "daily-backup").await;

    let req = get_request("/api/repos");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let repos = body.as_array().unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0]["name"], "daily-backup");
}

#[tokio::test]
#[ignore]
async fn test_excludes_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/excludes",
        Some(json!({
            "pattern": "*.tmp",
            "sort_order": 1
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["pattern"], "*.tmp");
    assert_eq!(body["sort_order"], 1);
    let exclude_id = body["id"].as_i64().unwrap();

    let req = get_request("/api/excludes");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let excludes = body.as_array().unwrap();
    assert_eq!(excludes.len(), 1);
    assert_eq!(excludes[0]["pattern"], "*.tmp");

    let req = json_request(
        "PUT",
        &format!("/api/excludes/{exclude_id}"),
        Some(json!({
            "pattern": "*.log",
            "sort_order": 5
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["pattern"], "*.log");
    assert_eq!(body["sort_order"], 5);

    let req = delete_request(&format!("/api/excludes/{exclude_id}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request("/api/excludes");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore]
async fn test_reports() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, display_name, agent_token_hash) VALUES ('report-host', \
         'Report Host', 'fakehash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "report-repo").await;

    let now = chrono::Utc::now();
    let started = now - chrono::Duration::minutes(5);
    sqlx::query(
        "INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, \
         original_size, compressed_size, deduplicated_size, files_processed, duration_secs, \
         borg_version) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(client_id)
    .bind(repo_id)
    .bind(started)
    .bind(now)
    .bind("success")
    .bind(1_073_741_824_i64)
    .bind(536_870_912_i64)
    .bind(268_435_456_i64)
    .bind(42_000_i64)
    .bind(300_i64)
    .bind("1.4.0")
    .execute(&pool)
    .await
    .unwrap();

    let req = get_request("/api/clients/report-host/reports");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let reports = body.as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["status"], "success");
    assert_eq!(reports[0]["original_size"], 1_073_741_824);
    assert_eq!(reports[0]["compressed_size"], 536_870_912);
    assert_eq!(reports[0]["deduplicated_size"], 268_435_456);
    assert_eq!(reports[0]["files_processed"], 42_000);
    assert_eq!(reports[0]["duration_secs"], 300);
    assert_eq!(reports[0]["borg_version"], "1.4.0");

    let req = get_request("/api/clients/report-host/reports?target=report-repo");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 1);

    let req = get_request("/api/clients/report-host/reports?target=no-such-repo");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore]
async fn test_stats_endpoints() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, display_name, agent_token_hash) VALUES ('stats-host', \
         'Stats Host', 'fakehash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "stats-repo").await;

    sqlx::query(
        "INSERT INTO schedules (client_id, repo_id, cron_expression, enabled) VALUES ($1, $2, $3, \
         $4)",
    )
    .bind(client_id)
    .bind(repo_id)
    .bind("0 2 * * *")
    .bind(true)
    .execute(&pool)
    .await
    .unwrap();

    let now = chrono::Utc::now();
    for i in 0..2 {
        let started = now - chrono::Duration::hours(i64::from(i) + 1);
        let finished = started + chrono::Duration::minutes(5);
        sqlx::query(
            "INSERT INTO backup_reports (client_id, repo_id, started_at, finished_at, status, \
             original_size, compressed_size, deduplicated_size, files_processed, duration_secs) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(client_id)
        .bind(repo_id)
        .bind(started)
        .bind(finished)
        .bind("success")
        .bind(1000_i64 * i64::from(i + 1))
        .bind(500_i64 * i64::from(i + 1))
        .bind(250_i64 * i64::from(i + 1))
        .bind(100_i64 * i64::from(i + 1))
        .bind(60_i64)
        .execute(&pool)
        .await
        .unwrap();
    }

    let req = get_request("/api/stats/storage");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let storage = body.as_array().unwrap();
    assert_eq!(storage.len(), 1);
    assert_eq!(storage[0]["hostname"], "stats-host");
    assert_eq!(storage[0]["target_name"], "stats-repo");
    assert_eq!(storage[0]["total_original_size"], 3000);
    assert_eq!(storage[0]["total_compressed_size"], 1500);
    assert_eq!(storage[0]["total_deduplicated_size"], 750);
    assert_eq!(storage[0]["report_count"], 2);

    let req = get_request("/api/stats/activity");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let activity = body.as_array().unwrap();
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0]["hostname"], "stats-host");
    assert_eq!(activity[0]["target_name"], "stats-repo");
    assert_eq!(activity[0]["status"], "success");
    assert_eq!(activity[0]["duration_secs"], 60);

    let req = get_request("/api/stats/health");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let health = body.as_array().unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0]["hostname"], "stats-host");
    assert_eq!(health[0]["target_name"], "stats-repo");
    assert_eq!(health[0]["last_status"], "success");
    assert!(health[0]["last_backup_at"].is_string());
}

#[tokio::test]
#[ignore]
async fn test_schedule_clone() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, display_name, agent_token_hash) VALUES ('clone-host', \
         'Clone Host', 'fakehash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "clone-repo").await;

    let schedule_id: i64 = sqlx::query_scalar(
        "INSERT INTO schedules (client_id, repo_id, schedule_type, cron_expression, enabled, \
         canary_enabled, exclude_patterns, ignore_global_excludes, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, pre_backup_commands, post_backup_commands) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) RETURNING id",
    )
    .bind(client_id)
    .bind(repo_id)
    .bind("backup")
    .bind("0 3 * * *")
    .bind(true)
    .bind(true)
    .bind(vec!["*.tmp".to_string(), "*.cache".to_string()])
    .bind(true)
    .bind(10_i32)
    .bind(11_i32)
    .bind(12_i32)
    .bind(13_i32)
    .bind(false)
    .bind("[\"echo pre\"]")
    .bind("[\"echo post\"]")
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO backup_sources (schedule_id, path, sort_order) VALUES ($1, $2, $3), ($1, $4, \
         $5)",
    )
    .bind(schedule_id)
    .bind("/src/one")
    .bind(0_i32)
    .bind("/src/two")
    .bind(1_i32)
    .execute(&pool)
    .await
    .unwrap();

    let req = Request::builder()
        .uri(format!("/api/schedules/{schedule_id}/clone"))
        .method("POST")
        .header("cookie", session_cookie())
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = body_json(resp).await;
    let cloned_id = body["id"].as_i64().unwrap();
    assert_ne!(cloned_id, schedule_id);
    assert!(!body["enabled"].as_bool().unwrap());
    assert_eq!(body["cron_expression"], "0 3 * * *");
    assert_eq!(body["repo_id"], repo_id);
    assert_eq!(body["schedule_type"], "backup");
    assert_eq!(body["canary_enabled"], true);
    assert_eq!(body["exclude_patterns"], json!(["*.tmp", "*.cache"]));
    assert_eq!(body["ignore_global_excludes"], true);
    assert_eq!(body["keep_daily"], 10);
    assert_eq!(body["keep_weekly"], 11);
    assert_eq!(body["keep_monthly"], 12);
    assert_eq!(body["keep_yearly"], 13);
    assert_eq!(body["compact_enabled"], false);
    assert_eq!(body["pre_backup_commands"], "[\"echo pre\"]");
    assert_eq!(body["post_backup_commands"], "[\"echo post\"]");
    assert!(body["last_run_at"].is_null());
    assert!(body["next_run_at"].is_null());

    let cloned_sources = sqlx::query_scalar::<_, String>(
        "SELECT path FROM backup_sources WHERE schedule_id = $1 ORDER BY sort_order, id",
    )
    .bind(cloned_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(cloned_sources, vec!["/src/one", "/src/two"]);
}

#[tokio::test]
#[ignore]
async fn test_client_not_found() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/clients/nonexistent");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore]
async fn test_duplicate_client_hostname() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "dup-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "dup-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore]
async fn test_empty_hostname_rejected() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request("POST", "/api/clients", Some(json!({ "hostname": "" })));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore]
async fn test_health_endpoint() {
    let pool = setup_pool().await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/health");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
