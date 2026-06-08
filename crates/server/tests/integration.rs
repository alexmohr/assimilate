// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Run with: `DATABASE_URL=postgres://... cargo test -p server --test integration -- --ignored`

use std::{os::unix::fs::PermissionsExt, sync::OnceLock};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{delete, get, post, put},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tower::{Service, ServiceExt};

const TEST_SESSION_ID: &str = "test-integration-session-id-00000000";
static BORG_BINARY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct BorgBinaryGuard {
    previous: Option<String>,
}

impl Drop for BorgBinaryGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.clone() {
            // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
            unsafe { std::env::set_var("BORG_BINARY", previous) };
        } else {
            // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
            unsafe { std::env::remove_var("BORG_BINARY") };
        }
    }
}

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
        completion_bus: server::ws::completion_bus::CompletionBus::new(),
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
            "/api/repos/{repo_id}/ssh-host-key/scan",
            post(server::api::repos::scan_repo_host_key),
        )
        .route(
            "/api/repos/{repo_id}/ssh-host-key",
            post(server::api::repos::accept_repo_host_key),
        )
        .route(
            "/api/repos/{repo_id}/sync",
            post(server::api::repos::sync_repo),
        )
        .route(
            "/api/repos/{repo_id}/reset-import",
            post(server::api::repos::reset_import),
        )
        .route(
            "/api/excludes",
            get(server::api::excludes::get_excludes).put(server::api::excludes::set_excludes),
        )
        .route(
            "/api/schedules",
            get(server::api::schedules::list_schedules),
        )
        .route(
            "/api/schedules/{id}/sources",
            get(server::api::schedules::list_schedule_backup_sources),
        )
        .route(
            "/api/clients/{hostname}/reports",
            get(server::api::reports::list_reports),
        )
        .route("/api/stats/storage", get(server::api::stats::storage))
        .route("/api/stats/activity", get(server::api::stats::activity))
        .route("/api/stats/health", get(server::api::stats::health))
        .route("/api/stats/summary", get(server::api::stats::summary))
        .route(
            "/api/stats/storage-breakdown",
            get(server::api::stats::storage_breakdown),
        )
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
        .route(
            "/api/config/export",
            get(server::api::config_io::export_config),
        )
        .route(
            "/api/config/import",
            post(server::api::config_io::import_config),
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

async fn borg_binary_lock() -> tokio::sync::MutexGuard<'static, ()> {
    BORG_BINARY_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

async fn install_fake_borg(
    list_json: &str,
    info_all_json: &str,
    info_repo_json: &str,
    json_lines: &str,
) -> (TempDir, BorgBinaryGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let script = format!(
        r#"#!/bin/sh
set -eu
case "$1" in
  list)
    case " $* " in
      *" --json-lines "*) cat <<'EOF'
{json_lines}
EOF
        ;;
      *) cat <<'EOF'
{list_json}
EOF
        ;;
    esac
    ;;
  info)
    case " $* " in
      *" --glob-archives "*) cat <<'EOF'
{info_all_json}
EOF
        ;;
      *) cat <<'EOF'
{info_repo_json}
EOF
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
"#
    );

    let borg_path = tempdir.path().join("borg");
    tokio::fs::write(&borg_path, script).await.unwrap();
    let mut permissions = tokio::fs::metadata(&borg_path).await.unwrap().permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&borg_path, permissions)
        .await
        .unwrap();

    let previous = std::env::var("BORG_BINARY").ok();
    // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
    unsafe { std::env::set_var("BORG_BINARY", &borg_path) };

    (tempdir, BorgBinaryGuard { previous })
}

async fn wait_for_archive_index(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> (String, Option<i64>) {
    use tokio::time::{Duration, timeout};

    timeout(Duration::from_secs(10), async move {
        loop {
            let row = sqlx::query_as::<_, (String, Option<i64>)>(
                "SELECT status, file_count FROM archive_index_jobs WHERE repo_id = $1 AND \
                 archive_name = $2",
            )
            .bind(repo_id)
            .bind(archive_name)
            .fetch_optional(pool)
            .await
            .unwrap();

            if let Some(row) = row
                && row.0 == "done"
            {
                return row;
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap()
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
    sqlx::query("UPDATE excludes_global_config SET raw_text = ''")
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
    assert_eq!(body["compression"], "zstd,3");
}

#[tokio::test]
#[ignore]
async fn test_repo_accept_ssh_host_key() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "accept-host-key-repo").await;
    let ssh_host_key = "ssh-ed25519 AAAAACCEPTED";

    let req = json_request(
        "POST",
        &format!("/api/repos/{repo_id}/ssh-host-key"),
        Some(json!({ "ssh_host_key": ssh_host_key })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["ssh_host_key"], ssh_host_key);

    let stored: Option<String> = sqlx::query_scalar("SELECT ssh_host_key FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some(ssh_host_key));
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
async fn test_sync_repo_unreachable_returns_error_and_clears_importing() {
    // sync_repo runs synchronously. The test repo points at an unreachable host
    // ("storage.local"), so the borg list fails and the endpoint returns an
    // error -- but the importing flag must be cleared again before it returns.
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "sync-accepted-repo").await;

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert!(
        resp.status().is_server_error(),
        "expected a server error for an unreachable repo, got {}",
        resp.status()
    );

    let importing: bool = sqlx::query_scalar("SELECT importing FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !importing,
        "importing should be cleared after the synchronous sync returns"
    );
}

#[tokio::test]
#[ignore]
async fn test_sync_repo_returns_409_when_already_importing() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "sync-conflict-repo").await;

    // pre-set importing = true to simulate in-progress sync
    server::db::set_repo_importing(&pool, repo_id, true)
        .await
        .unwrap();

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // flag must still be true (we didn't touch it)
    let importing: bool = sqlx::query_scalar("SELECT importing FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        importing,
        "importing should remain true after rejected sync"
    );
}

#[tokio::test]
#[ignore]
async fn test_sync_repo_indexes_new_archive_after_success() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let list_json = r#"{
  "archives": [
    {
      "name": "sync-archive-1",
      "hostname": "web-server-01",
      "start": "2026-06-05T10:00:00Z",
      "end": "2026-06-05T10:05:00Z",
      "duration": 300.0,
      "stats": {
        "original_size": 1000,
        "compressed_size": 500,
        "deduplicated_size": 250,
        "nfiles": 2
      }
    }
  ]
}"#;
    let info_all_json = list_json;
    let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 1000,
      "total_csize": 600,
      "unique_csize": 500,
      "total_chunks": 10,
      "unique_chunks": 8
    }
  }
}"#;
    let json_lines = concat!(
        r#"{"type":"d","path":"docs","size":0,"mtime":"2026-06-05T10:00:00Z","#,
        r#""mode":"drwxr-xr-x"}"#,
        "\n",
        r#"{"type":"f","path":"docs/manual.txt","size":12,"mtime":"2026-06-05T10:00:00Z","#,
        r#""mode":"-rw-r--r--"}"#,
    );

    let (_borg_dir, _borg_guard) =
        install_fake_borg(list_json, info_all_json, info_repo_json, json_lines).await;

    let mut app = build_test_app(pool.clone()).await;
    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, display_name, agent_token_hash) VALUES ($1, $2, $3) \
         RETURNING id",
    )
    .bind("stale-host")
    .bind("Stale Host")
    .bind("token-hash")
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "sync-success-repo").await;

    let stale_started_at = chrono::Utc::now() - chrono::Duration::days(1);
    let stale_finished_at = stale_started_at + chrono::Duration::minutes(5);
    sqlx::query(
        "INSERT INTO backup_reports (client_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, NULL, $3, $4, 'success', 10, 5, 5, 5, 1, \
         300, NULL, '{}'::text[], NULL, true, $5, NULL)",
    )
    .bind(client_id)
    .bind(repo_id)
    .bind(stale_started_at)
    .bind(stale_finished_at)
    .bind("stale-archive")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archive_index_jobs (repo_id, archive_name, status, file_count) VALUES ($1, \
         $2, 'done', 1)",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archive_paths (repo_id, archive_name, path) VALUES ($1, $2, $3), ($1, $2, $4)",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .bind("")
    .bind("stale.txt")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archive_files (repo_id, archive_name, path_id, parent_path_id, entry_type, \
         size, mtime, mode) SELECT $1, $2, child.id, parent.id, 'f', 1, '', '' FROM archive_paths \
         child JOIN archive_paths parent ON parent.repo_id = child.repo_id AND \
         parent.archive_name = child.archive_name AND parent.path = $4 WHERE child.repo_id = $1 \
         AND child.archive_name = $2 AND child.path = $3",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .bind("stale.txt")
    .bind("")
    .execute(&pool)
    .await
    .unwrap();

    let req = json_request(
        "POST",
        &format!("/api/repos/{repo_id}/sync?build_index=true"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["imported"], 1);
    assert_eq!(body["removed"], 1);

    let (status, file_count) = wait_for_archive_index(&pool, repo_id, "sync-archive-1").await;
    assert_eq!(status, "done");
    assert_eq!(file_count, Some(2));

    let stale_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stale_count, 0);
    let stale_index_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_index_jobs WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stale_index_rows, 0);
    let stale_file_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_files WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stale_file_rows, 0);

    let file_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_files WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("sync-archive-1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(file_rows, 2);
}

#[tokio::test]
#[ignore]
async fn test_stats_summary_returns_200() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/stats/summary");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_object(), "summary should be a JSON object");
    assert!(body["total_clients"].is_number());
    assert!(body["total_repos"].is_number());
    assert!(body["total_storage_bytes"].is_number());
}

#[tokio::test]
#[ignore]
async fn test_storage_breakdown_empty() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let req = get_request("/api/stats/storage-breakdown");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array(), "storage breakdown should be a JSON array");
}

#[tokio::test]
#[ignore]
async fn test_storage_breakdown_with_data() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "breakdown-repo").await;
    server::db::update_repo_info_stats(
        &pool,
        repo_id,
        &server::db::RepoInfoStats {
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let req = get_request("/api/stats/storage-breakdown");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let entries = body.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "breakdown-repo");
    assert_eq!(entries[0]["compressed_size"], 500_000);
    assert_eq!(entries[0]["deduplicated_size"], 250_000);
    // sole repo owns 100 % of storage
    let pct = entries[0]["percentage"].as_f64().unwrap();
    assert!(
        (pct - 100.0).abs() < 0.01,
        "single repo should be 100%, got {pct}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reset_import_clears_state() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "reset-import-repo").await;

    // Simulate a stuck import
    server::db::set_repo_importing(&pool, repo_id, true)
        .await
        .unwrap();
    server::db::set_repo_import_error(&pool, repo_id, Some("stuck error"))
        .await
        .unwrap();

    let req = json_request("POST", &format!("/api/repos/{repo_id}/reset-import"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(!stats.importing, "importing should be cleared after reset");
    assert!(
        stats.import_error.is_none(),
        "import_error should be cleared after reset"
    );
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

// -- Excludes API tests --

/// Helper: insert a schedule directly into the DB (bypasses SSH check in the API).
async fn insert_test_schedule(pool: &sqlx::PgPool, client_id: i64, repo_id: i64) -> i64 {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration");
    let passphrase_encrypted = shared::crypto::encrypt_passphrase("pass", &encryption_key).unwrap();
    sqlx::query_scalar("UPDATE repos SET passphrase_encrypted = $2 WHERE id = $1 RETURNING id")
        .bind(repo_id)
        .bind(&passphrase_encrypted)
        .fetch_one(pool)
        .await
        .unwrap_or(repo_id);

    let schedule_id: i64 = sqlx::query_scalar(
        "INSERT INTO schedules (repo_id, name, schedule_type, cron_expression, enabled, \
         canary_enabled, exclude_patterns_raw, ignore_global_excludes, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure) VALUES ($1, 'test', 'backup', '0 3 * * *', true, false, $2, \
         false, 7, 4, 6, 0, true, '[]', '[]', 'parallel', 'stop') RETURNING id",
    )
    .bind(repo_id)
    .bind("")
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO schedule_targets (schedule_id, client_id, execution_order) VALUES ($1, $2, 0)",
    )
    .bind(schedule_id)
    .bind(client_id)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO backup_sources (schedule_id, path, sort_order) VALUES ($1, '/home', 0)",
    )
    .bind(schedule_id)
    .execute(pool)
    .await
    .unwrap();

    schedule_id
}

#[sqlx::test(migrations = "./migrations")]
async fn test_global_excludes_get_initially_empty(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let resp = oneshot(&mut app, get_request("/api/excludes")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["raw_text"], "");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_global_excludes_roundtrip_preserves_blank_lines_and_comments(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let raw = "# System paths\n/proc\n/sys\n\n# Cache\n*.cache\npp:__pycache__";

    let resp = oneshot(
        &mut app,
        json_request(
            "PUT",
            "/api/excludes",
            Some(serde_json::json!({"raw_text": raw})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = oneshot(&mut app, get_request("/api/excludes")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["raw_text"], raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_global_excludes_overwrite_replaces_fully(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    for text in &["first\nsecond\nthird", "only-this-one"] {
        let resp = oneshot(
            &mut app,
            json_request(
                "PUT",
                "/api/excludes",
                Some(serde_json::json!({"raw_text": text})),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let resp = oneshot(&mut app, get_request("/api/excludes")).await;
    let body = body_json(resp).await;
    assert_eq!(body["raw_text"], "only-this-one");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_per_host_excludes_roundtrip_preserves_raw_text(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    // Set up client and repo directly
    let client_id: i64 = sqlx::query_scalar(
        "INSERT INTO clients (hostname, agent_token_hash) VALUES ('exc-host', 'hash-exc') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "exc-repo").await;
    let schedule_id = insert_test_schedule(&pool, client_id, repo_id).await;

    let raw = "# Cache dirs\n*.cache\npp:__pycache__\n\n# Runtime\n/proc\n/sys";

    sqlx::query(
        "INSERT INTO per_host_excludes (schedule_id, client_id, raw_text) VALUES ($1, $2, $3)",
    )
    .bind(schedule_id)
    .bind(client_id)
    .bind(raw)
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(
        &mut app,
        get_request(&format!("/api/schedules/{schedule_id}/sources")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;

    let per_host = body["exclude_patterns_per_host"].as_array().unwrap();
    assert_eq!(per_host.len(), 1);
    assert_eq!(per_host[0]["client_id"], client_id);
    assert_eq!(per_host[0]["raw_text"], raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_empty(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["version"], 1);
    assert!(body["exported_at"].is_string());
    assert_eq!(body["hosts"].as_array().unwrap().len(), 0);
    assert_eq!(body["schedules"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_with_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    sqlx::query(
        "INSERT INTO clients (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns) VALUES ('export-host', 'Export Host', 'real-token', \
         ARRAY['/etc','/home'], ARRAY['*.log'])",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hosts = body["hosts"].as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["hostname"], "export-host");
    assert_eq!(hosts[0]["display_name"], "Export Host");
    assert_eq!(hosts[0]["default_backup_paths"][0], "/etc");
    assert_eq!(hosts[0]["default_backup_paths"][1], "/home");
    assert_eq!(hosts[0]["default_exclude_patterns"][0], "*.log");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_skips_imported_token_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    sqlx::query(
        "INSERT INTO clients (hostname, agent_token_hash) VALUES ('real-host', 'real-token'), \
         ('imported-host', 'imported:no-auth')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hosts = body["hosts"].as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["hostname"], "real-host");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_creates_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let payload = json!({
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [
            {
                "hostname": "new-host-1",
                "display_name": "New Host 1",
                "default_backup_paths": ["/etc", "/home"],
                "default_exclude_patterns": ["*.log"],
                "hostname_patterns": []
            }
        ],
        "schedules": []
    });

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hosts_created"], 1);
    assert_eq!(body["hosts_updated"], 0);
    assert_eq!(body["schedules_created"], 0);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM clients WHERE hostname = 'new-host-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_updates_existing_host(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    sqlx::query(
        "INSERT INTO clients (hostname, agent_token_hash) VALUES ('existing-host', 'real-token')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let payload = json!({
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [
            {
                "hostname": "existing-host",
                "display_name": "Updated Name",
                "default_backup_paths": ["/var"],
                "default_exclude_patterns": [],
                "hostname_patterns": []
            }
        ],
        "schedules": []
    });

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hosts_created"], 0);
    assert_eq!(body["hosts_updated"], 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_rejects_wrong_version(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let payload = json!({
        "version": 999,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [],
        "schedules": []
    });

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_warns_on_missing_repo(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    sqlx::query(
        "INSERT INTO clients (hostname, agent_token_hash) VALUES ('sched-host', 'real-token')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let payload = json!({
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [],
        "schedules": [
            {
                "name": "orphan-schedule",
                "schedule_type": "backup",
                "cron_expression": "0 3 * * *",
                "enabled": true,
                "canary_enabled": false,
                "execution_mode": "parallel",
                "on_failure": "stop",
                "exclude_patterns_raw": "",
                "ignore_global_excludes": false,
                "keep_hourly": 0,
                "keep_daily": 7,
                "keep_weekly": 4,
                "keep_monthly": 6,
                "keep_yearly": 0,
                "compact_enabled": true,
                "rate_limit_kbps": null,
                "pre_backup_commands": [],
                "post_backup_commands": [],
                "repo_name": "nonexistent-repo",
                "backup_sources": [],
                "targets": [
                    {
                        "hostname": "sched-host",
                        "execution_order": 0,
                        "backup_sources": [],
                        "exclude_patterns": ""
                    }
                ]
            }
        ]
    });

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["schedules_created"], 0);
    let warnings = body["warnings"].as_array().unwrap();
    assert!(!warnings.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_creates_schedule_with_matching_repo(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    let repo_id = insert_test_repo(&pool, "import-repo").await;
    let _ = repo_id;

    sqlx::query(
        "INSERT INTO clients (hostname, agent_token_hash) VALUES ('import-target', 'real-token')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let payload = json!({
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [],
        "schedules": [
            {
                "name": "import-schedule",
                "schedule_type": "backup",
                "cron_expression": "0 3 * * *",
                "enabled": true,
                "canary_enabled": false,
                "execution_mode": "parallel",
                "on_failure": "stop",
                "exclude_patterns_raw": "",
                "ignore_global_excludes": false,
                "keep_hourly": 0,
                "keep_daily": 7,
                "keep_weekly": 4,
                "keep_monthly": 6,
                "keep_yearly": 0,
                "compact_enabled": true,
                "rate_limit_kbps": null,
                "pre_backup_commands": ["/usr/bin/pre.sh"],
                "post_backup_commands": [],
                "repo_name": "import-repo",
                "backup_sources": ["/home"],
                "targets": [
                    {
                        "hostname": "import-target",
                        "execution_order": 0,
                        "backup_sources": ["/etc"],
                        "exclude_patterns": "*.tmp"
                    }
                ]
            }
        ]
    });

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["schedules_created"], 1);
    assert_eq!(body["warnings"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_then_import_roundtrip(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone()).await;

    sqlx::query(
        "INSERT INTO clients (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns) VALUES ('roundtrip-host', 'RT Host', 'real-token', \
         ARRAY['/etc'], ARRAY['*.swp'])",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let export = body_json(resp).await;

    sqlx::query("DELETE FROM clients WHERE hostname = 'roundtrip-host'")
        .execute(&pool)
        .await
        .unwrap();

    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(export)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hosts_created"], 1);

    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT default_backup_paths FROM clients WHERE hostname = 'roundtrip-host'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(paths, vec!["/etc"]);
}
