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
        pool,
        encryption_key,
        registry: server::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer: server::log_buffer::LogBuffer::default(),
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
        .route(
            "/api/clients/{hostname}/repos",
            post(server::api::repos::create_repo),
        )
        .route("/api/repos", get(server::api::repos::list_repos))
        .route(
            "/api/clients/{hostname}/repos/{target}",
            put(server::api::repos::update_repo),
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
            get(server::api::schedules::list_schedules)
                .post(server::api::schedules::create_schedule),
        )
        .route(
            "/api/schedules/{id}",
            put(server::api::schedules::update_schedule),
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

const TEST_SESSION_ID: &str = "test-integration-session-id-00000000";

async fn setup_pool() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let pool = PgPool::connect(&database_url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn create_test_user_and_session(pool: &PgPool) {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(TEST_SESSION_ID)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE username = 'integration-admin'")
        .execute(pool)
        .await
        .unwrap();

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, role) VALUES ('integration-admin', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', 'admin') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let expires = chrono::Utc::now() + chrono::Duration::hours(24);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
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
    sqlx::query("DELETE FROM backup_sources")
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

fn session_cookie() -> &'static str {
    const COOKIE_VAL: &str = const_format::concatcp!("session=", TEST_SESSION_ID);
    COOKIE_VAL
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    match body {
        Some(val) => Request::builder()
            .uri(uri)
            .method(method)
            .header("content-type", "application/json")
            .header("cookie", session_cookie())
            .body(Body::from(serde_json::to_vec(&val).unwrap()))
            .unwrap(),
        None => Request::builder()
            .uri(uri)
            .method(method)
            .header("content-type", "application/json")
            .header("cookie", session_cookie())
            .body(Body::empty())
            .unwrap(),
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
async fn test_repo_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "repo-test-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = json_request(
        "POST",
        "/api/clients/repo-test-host/repos",
        Some(json!({
            "target_name": "daily-backup",
            "repo_path": "/backups/daily",
            "ssh_user": "backup",
            "ssh_host": "storage.local",
            "ssh_port": 2222,
            "passphrase": "super-secret-passphrase",
            "compression": { "type": "Zstd", "value": { "level": 3 } }
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["target_name"], "daily-backup");
    assert_eq!(body["repo_path"], "/backups/daily");
    assert_eq!(body["ssh_user"], "backup");
    assert_eq!(body["ssh_host"], "storage.local");
    assert_eq!(body["ssh_port"], 2222);
    assert_eq!(body["compression"], "zstd,3");
    assert!(body["enabled"].as_bool().unwrap());

    let req = get_request("/api/repos");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let repos = body.as_array().unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0]["target_name"], "daily-backup");
    assert_eq!(repos[0]["hostname"], "repo-test-host");

    let req = json_request(
        "PUT",
        "/api/clients/repo-test-host/repos/daily-backup",
        Some(json!({
            "repo_path": "/backups/daily-v2",
            "ssh_user": "backup2",
            "ssh_host": "storage2.local",
            "ssh_port": 22,
            "compression": { "type": "Lz4" },
            "enabled": false
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["repo_path"], "/backups/daily-v2");
    assert_eq!(body["ssh_user"], "backup2");
    assert_eq!(body["ssh_host"], "storage2.local");
    assert_eq!(body["compression"], "lz4");
    assert!(!body["enabled"].as_bool().unwrap());
}

#[tokio::test]
#[ignore]
async fn test_excludes_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/excludes",
        Some(json!({
            "scope": { "type": "Global" },
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

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "exclude-test-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = json_request(
        "POST",
        "/api/excludes",
        Some(json!({
            "scope": { "type": "Machine", "hostname": "exclude-test-host" },
            "pattern": "/var/log/*",
            "sort_order": 2
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["pattern"], "/var/log/*");
    let client_exclude_id = body["id"].as_i64().unwrap();

    let req = get_request("/api/excludes");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["global"].as_array().unwrap().len(), 1);
    assert_eq!(body["client"].as_array().unwrap().len(), 1);

    let req = json_request(
        "PUT",
        &format!("/api/excludes/{exclude_id}"),
        Some(json!({
            "scope": { "type": "Global" },
            "pattern": "*.log",
            "sort_order": 5
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["pattern"], "*.log");
    assert_eq!(body["sort_order"], 5);

    let req = delete_request(&format!("/api/excludes/{exclude_id}?scope=global"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = delete_request(&format!("/api/excludes/{client_exclude_id}?scope=machine"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request("/api/excludes");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["global"].as_array().unwrap().is_empty());
    assert!(body["client"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore]
async fn test_schedule_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "schedule-test-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = json_request(
        "POST",
        "/api/clients/schedule-test-host/repos",
        Some(json!({
            "target_name": "sched-repo",
            "repo_path": "/backups/sched",
            "ssh_user": "user",
            "ssh_host": "host.local",
            "passphrase": "passphrase123"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let repo_id = body["id"].as_i64().unwrap();

    let req = json_request(
        "POST",
        "/api/schedules",
        Some(json!({
            "repo_id": repo_id,
            "interval": { "type": "Hourly", "value": { "every_n_hours": 6 } },
            "time_of_day": "03:00:00",
            "enabled": true
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["repo_id"], repo_id);
    assert_eq!(body["interval_type"], "hourly");
    assert_eq!(body["every_n_hours"], 6);
    assert_eq!(body["time_of_day"], "03:00:00");
    assert!(body["enabled"].as_bool().unwrap());
    let schedule_id = body["id"].as_i64().unwrap();

    let req = get_request("/api/schedules");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let schedules = body.as_array().unwrap();
    assert_eq!(schedules.len(), 1);

    let req = json_request(
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(json!({
            "interval": { "type": "Daily" },
            "time_of_day": "22:00:00",
            "day_of_week": "Monday",
            "enabled": false
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["interval_type"], "daily");
    assert_eq!(body["time_of_day"], "22:00:00");
    assert_eq!(body["day_of_week"], "monday");
    assert!(!body["enabled"].as_bool().unwrap());
}

#[tokio::test]
#[ignore]
async fn test_reports() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "report-test-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let client_id = body["client"]["id"].as_i64().unwrap();

    let req = json_request(
        "POST",
        "/api/clients/report-test-host/repos",
        Some(json!({
            "target_name": "report-repo",
            "repo_path": "/backups/report",
            "ssh_user": "user",
            "ssh_host": "host.local",
            "passphrase": "pass"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let repo_id = body["id"].as_i64().unwrap();

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

    let req = get_request("/api/clients/report-test-host/reports");
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

    let req = get_request("/api/clients/report-test-host/reports?target=report-repo");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 1);

    let req = get_request("/api/clients/report-test-host/reports?target=no-such-repo");
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
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request(
        "POST",
        "/api/clients",
        Some(json!({ "hostname": "stats-host" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let client_id = body["client"]["id"].as_i64().unwrap();

    let req = json_request(
        "POST",
        "/api/clients/stats-host/repos",
        Some(json!({
            "target_name": "stats-repo",
            "repo_path": "/backups/stats",
            "ssh_user": "user",
            "ssh_host": "host.local",
            "passphrase": "pass"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    let repo_id = body["id"].as_i64().unwrap();

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
async fn test_client_not_found() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
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
    let mut app = build_test_app(pool.clone()).await;

    let req = json_request("POST", "/api/clients", Some(json!({ "hostname": "" })));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore]
async fn test_health_endpoint() {
    let pool = setup_pool().await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/health");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
