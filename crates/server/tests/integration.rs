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
use server::api::tokens::hash_token;
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

#[cfg(test)]
async fn oneshot(app: &mut Router, req: Request<Body>) -> axum::response::Response {
    ServiceExt::<Request<Body>>::ready(app)
        .await
        .unwrap()
        .call(req)
        .await
        .unwrap()
}

#[cfg(test)]
async fn body_json(response: axum::response::Response) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[cfg(test)]
fn build_test_state(pool: PgPool) -> server::AppState {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
    let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
    let server_addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let tunnel_manager =
        server::tunnel::TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr);
    server::AppState {
        pool: pool.clone(),
        encryption_key,
        registry: server::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer: server::log_buffer::LogBuffer::default(),
        notification_service: server::notifications::NotificationService::new(pool),
        pending_dryruns: server::new_pending_map(),
        pending_restores: server::new_pending_map(),
        pending_migrations: server::new_pending_map(),
        pending_deletes: server::new_pending_map(),
        completion_bus: server::ws::completion_bus::CompletionBus::new(),
        repo_op_tracker: server::repo_op_tracker::RepoOpTracker::default(),
        background_task_tracker: server::background_tasks::BackgroundTaskTracker::default(),
        repo_lock: server::RepoLock::default(),
        import_tasks: server::ImportTaskRegistry::default(),
        shutdown_token: tokio_util::sync::CancellationToken::new(),
        client_ip_resolver: server::client_ip::ClientIpResolver::new(),
        task_registry: shared::task_registry::TaskRegistry::default(),
    }
}

#[cfg(test)]
fn test_app_core_routes() -> Router<server::AppState> {
    Router::new()
        .route("/api/health", get(server::api::health::health))
        .route("/api/auth/login", post(server::api::auth::login))
        .route("/api/auth/logout", post(server::api::auth::logout))
        .route("/api/auth/me", get(server::api::auth::me))
        .route(
            "/api/users",
            get(server::api::users::list_users).post(server::api::users::create_user),
        )
        .route("/api/users/{id}", delete(server::api::users::delete_user))
        .route(
            "/api/agents",
            get(server::api::agents::list_agents).post(server::api::agents::create_agent),
        )
        .route(
            "/api/agents/{hostname}",
            get(server::api::agents::get_agent)
                .put(server::api::agents::update_agent)
                .delete(server::api::agents::delete_agent),
        )
        .route(
            "/api/agents/{hostname}/reports",
            get(server::api::reports::list_reports),
        )
}

#[cfg(test)]
fn test_app_repo_routes() -> Router<server::AppState> {
    Router::new()
        .route("/api/repos", get(server::api::repos::list_repos))
        .route(
            "/api/repos/{repo_id}",
            get(server::api::repos::get_repo)
                .put(server::api::repos::update_repo)
                .delete(server::api::repos::delete_repo),
        )
        .route(
            "/api/repos/{repo_id}/archives",
            get(server::api::archives::list_archives),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}",
            delete(server::api::archives::delete_archive),
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
            "/api/repos/{repo_id}/schedules",
            get(server::api::repos::list_schedules_for_repo),
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
            "/api/config/export",
            get(server::api::config_io::export_config),
        )
        .route(
            "/api/config/import",
            post(server::api::config_io::import_config),
        )
}

#[cfg(test)]
fn test_app_stats_and_notification_routes() -> Router<server::AppState> {
    Router::new()
        .route("/api/stats/storage", get(server::api::stats::storage))
        .route("/api/stats/activity", get(server::api::stats::activity))
        .route("/api/stats/health", get(server::api::stats::health))
        .route("/api/stats/summary", get(server::api::stats::summary))
        .route(
            "/api/stats/storage-breakdown",
            get(server::api::stats::storage_breakdown),
        )
        .route("/api/audit-log", get(server::api::audit::list_audit_log))
        .route("/api/logs", get(server::api::logs::get_logs))
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
}

#[cfg(test)]
fn build_test_app(pool: PgPool) -> Router {
    build_test_app_with_state(pool).0
}

/// Like [`build_test_app`], but also hands back the [`server::AppState`] so a test
/// can wait on `background_task_tracker.any_active()` after a request that fires a
/// fire-and-forget background task (e.g. archive-stat enrichment after a sync).
#[cfg(test)]
fn build_test_app_with_state(pool: PgPool) -> (Router, server::AppState) {
    let state = build_test_state(pool);

    let router = Router::new()
        .merge(test_app_core_routes())
        .merge(test_app_repo_routes())
        .merge(test_app_stats_and_notification_routes())
        .with_state(state.clone());
    (router, state)
}

#[cfg(test)]
async fn setup_pool() -> PgPool {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let pool = PgPool::connect(&database_url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[cfg(test)]
async fn create_test_user_and_session(pool: &PgPool) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('integration-admin', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') ON CONFLICT (username) DO \
         UPDATE SET username = EXCLUDED.username RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let admin_role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'admin'")
        .fetch_one(pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(admin_role_id)
        .execute(pool)
        .await
        .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let hashed_id = hash_token(TEST_SESSION_ID);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO \
         UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(&hashed_id)
    .bind(user_id)
    .bind(expires)
    .execute(pool)
    .await
    .unwrap();
}

#[cfg(test)]
async fn borg_binary_lock() -> tokio::sync::MutexGuard<'static, ()> {
    BORG_BINARY_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

#[cfg(test)]
async fn install_fake_borg(
    list_json: &str,
    info_all_json: &str,
    info_repo_json: &str,
    repo_list_lines: &str,
    json_lines: &str,
) -> (TempDir, BorgBinaryGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let script = format!(
        r#"#!/bin/sh
set -eu
case "$1" in
  list)
    case " $* " in
      *" --json-lines "*)
        for _a; do _last="$_a"; done
        case "$_last" in
          *::*) cat <<'EOF'
{json_lines}
EOF
            ;;
          *) cat <<'EOF'
{repo_list_lines}
EOF
            ;;
        esac
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
      *"::"*) cat <<'EOF'
{info_all_json}
EOF
        ;;
      *) cat <<'EOF'
{info_repo_json}
EOF
        ;;
    esac
    ;;
  delete)
    exit 0
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

/// Installs a fake borg where `list` returns an empty archive set immediately
/// but `info` sleeps indefinitely. Used to reproduce the bug where
/// `refresh_repo_info_stats` had no timeout, causing repos with no archives
/// to hang forever with `importing = true`.
#[cfg(test)]
async fn install_borg_empty_list_hanging_info() -> (TempDir, BorgBinaryGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let script = concat!(
        "#!/bin/sh\n",
        "case \"$1\" in\n",
        "  list)\n",
        "    case \" $* \" in\n",
        "      *\" --json-lines \"*) ;;\n",
        "      *) echo '{\"archives\":[]}'  ;;\n",
        "    esac;;\n",
        "  info) sleep 120;;\n",
        "  *) exit 1;;\n",
        "esac\n",
    );
    let borg_path = tempdir.path().join("borg");
    tokio::fs::write(&borg_path, script).await.unwrap();
    let mut permissions = tokio::fs::metadata(&borg_path).await.unwrap().permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&borg_path, permissions)
        .await
        .unwrap();
    let previous = std::env::var("BORG_BINARY").ok();
    // SAFETY: tests serialise BORG_BINARY changes with a process-local lock.
    unsafe { std::env::set_var("BORG_BINARY", &borg_path) };
    (tempdir, BorgBinaryGuard { previous })
}

/// Installs a fake borg whose `list` returns an empty archive list after
/// sleeping for `delay_secs`. Used to verify that the scheduler dispatches
/// repo syncs concurrently instead of sequentially.
#[cfg(test)]
async fn install_slow_borg_list(delay_secs: u64) -> (TempDir, BorgBinaryGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let info_json = concat!(
        r#"{"cache":{"stats":{"total_size":0,"total_csize":0,"#,
        r#""unique_csize":0,"total_chunks":0,"total_unique_chunks":0}}}"#
    );
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  list)
    case " $* " in
      *" --json-lines "*) sleep {delay_secs} ;;
      *) sleep {delay_secs}; echo '{{"archives":[]}}' ;;
    esac ;;
  info) echo '{info_json}' ;;
  *) exit 1 ;;
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
    // SAFETY: tests serialise BORG_BINARY changes with a process-local lock.
    unsafe { std::env::set_var("BORG_BINARY", &borg_path) };
    (tempdir, BorgBinaryGuard { previous })
}

/// Installs a fake borg whose `list` hangs, to exercise the query timeout.
#[cfg(test)]
async fn install_hanging_borg() -> (TempDir, BorgBinaryGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let script = "#!/bin/sh\ncase \"$1\" in\n  list) sleep 60 ;;\n  *) exit 1 ;;\nesac\n";
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

#[cfg(test)]
async fn wait_for_archive_index(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> (String, Option<i64>) {
    use tokio::time::{Duration, timeout};

    timeout(Duration::from_secs(10), async move {
        loop {
            let row = sqlx::query_as::<_, (String, Option<i64>)>(
                "SELECT j.status, j.file_count FROM archive_index_jobs j JOIN archives a ON a.id \
                 = j.archive_id WHERE a.repo_id = $1 AND a.name = $2",
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

/// Poll the `importing` flag until the background sync/reset task finishes.
#[cfg(test)]
async fn wait_for_import_completion(pool: &PgPool, repo_id: i64) {
    use tokio::time::{Duration, timeout};

    timeout(Duration::from_secs(30), async move {
        loop {
            let importing: bool =
                sqlx::query_scalar("SELECT importing FROM repo_import_state WHERE repo_id = $1")
                    .bind(repo_id)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if !importing {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("import did not complete within 30 seconds");
}

#[cfg(test)]
async fn clean_tables(pool: &PgPool) {
    sqlx::query(
        "TRUNCATE TABLE audit_log, login_attempts, system_events, system_settings, server_quotas, \
         notification_deliveries, notification_rules, ssh_tunnels, agent_hostname_patterns, \
         agent_tags, schedule_targets, per_agent_excludes, per_agent_commands, \
         per_agent_file_change_patterns, archive_tags, archive_files, archive_index_jobs, \
         archive_paths, archives, backup_sources, backup_reports, canary_results, repo_tags, \
         repo_stats, repo_import_state, repo_last_op, repo_quotas, repo_relocation_pending_hosts, \
         schedules, dismissed_dashboard_findings, push_subscriptions, api_tokens, sessions, \
         user_roles, user_groups, repo_permissions, users, groups, tags, repos, agents, \
         notification_channels CASCADE",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE excludes_global_config SET raw_text = ''")
        .execute(pool)
        .await
        .unwrap();
}

/// Inserts a repo directly into DB, bypassing the API (which requires SSH connectivity).
#[cfg(test)]
async fn insert_test_repo(pool: &PgPool, name: &str) -> i64 {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
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
    .bind(22i32)
    .bind(&passphrase_encrypted)
    .bind("lz4")
    .bind("repokey")
    .fetch_one(pool)
    .await
    .unwrap()
}

#[cfg(test)]
fn session_cookie() -> String {
    format!("session={TEST_SESSION_ID}")
}

#[cfg(test)]
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

#[cfg(test)]
fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .header("cookie", session_cookie())
        .body(Body::empty())
        .unwrap()
}

#[cfg(test)]
fn delete_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("cookie", session_cookie())
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_agent_crud() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = json_request(
        "POST",
        "/api/agents",
        Some(json!({
            "hostname": "test-host-1",
            "display_name": "Test Host 1"
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(
        body.get("agent").unwrap().get("hostname").unwrap(),
        "test-host-1"
    );
    assert_eq!(
        body.get("agent").unwrap().get("display_name").unwrap(),
        "Test Host 1"
    );
    assert!(
        body.get("token")
            .unwrap()
            .as_str()
            .is_some_and(|t| t.len() == 64)
    );

    let req = get_request("/api/agents/test-host-1");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_object());
    assert_eq!(body.get("hostname").unwrap(), "test-host-1");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_notification_channels_list() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = get_request("/api/notifications/channels");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_notification_channel_create_webhook() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(body.get("name").unwrap(), "test-webhook");
    assert_eq!(body.get("channel_type").unwrap(), "webhook");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_tunnels_list() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = get_request("/api/tunnels");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_tunnel_create() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, display_name, agent_token_hash) VALUES ('tunnel-host', \
         'Tunnel Host', 'fakehash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let req = json_request(
        "POST",
        "/api/tunnels",
        Some(json!({
            "agent_id": agent_id,
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
    assert_eq!(body.get("ssh_host").unwrap(), "remote.example.com");
    assert_eq!(body.get("tunnel_port").unwrap(), 2222);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_agent() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = json_request(
        "POST",
        "/api/agents",
        Some(json!({ "hostname": "to-delete" })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = delete_request("/api/agents/to-delete");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request("/api/agents/to-delete");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_repo_update() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(body.get("compression").unwrap(), "zstd,3");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_repo_accept_ssh_host_key() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(body.get("ssh_host_key").unwrap(), ssh_host_key);

    let stored: Option<String> = sqlx::query_scalar("SELECT ssh_host_key FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some(ssh_host_key));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_repo_delete() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "delete-repo").await;

    let req = delete_request(&format!("/api/repos/{repo_id}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req = get_request(&format!("/api/repos/{repo_id}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_list_archives_deduplicates_archive_names() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "archive-list-repo").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('archive-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    for (started_at, finished_at, original_size, archive_name) in [
        (
            "2026-06-01T10:00:00Z",
            "2026-06-01T10:05:00Z",
            100i64,
            "dup-archive",
        ),
        (
            "2026-06-02T10:00:00Z",
            "2026-06-02T10:05:00Z",
            200i64,
            "dup-archive",
        ),
        (
            "2026-06-03T10:00:00Z",
            "2026-06-03T10:05:00Z",
            300i64,
            "unique-archive",
        ),
    ] {
        sqlx::query(
            "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
             original_size, compressed_size, deduplicated_size, repo_unique_csize, \
             files_processed, duration_secs, error_message, warnings, borg_version, matched, \
             archive_name, borg_command) VALUES ($1, $2, $3, $4, 'success', $5, $6, $7, $8, $9, \
             $10, NULL, ARRAY[]::text[], NULL, true, $11, NULL)",
        )
        .bind(agent_id)
        .bind(repo_id)
        .bind(chrono::DateTime::parse_from_rfc3339(started_at).unwrap())
        .bind(chrono::DateTime::parse_from_rfc3339(finished_at).unwrap())
        .bind(original_size)
        .bind(original_size - 10)
        .bind(original_size - 20)
        .bind(original_size - 30)
        .bind(original_size - 40)
        .bind(original_size - 50)
        .bind(archive_name)
        .execute(&pool)
        .await
        .unwrap();
    }

    let req = get_request(&format!("/api/repos/{repo_id}/archives"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let archives = body.as_array().unwrap();
    assert_eq!(archives.len(), 2);
    assert_eq!(
        archives.first().unwrap().get("name").unwrap(),
        "unique-archive"
    );
    assert_eq!(
        archives.first().unwrap().get("start").unwrap(),
        "2026-06-03T10:00:00.000000Z"
    );
    assert_eq!(archives.get(1).unwrap().get("name").unwrap(), "dup-archive");
    assert_eq!(
        archives.get(1).unwrap().get("start").unwrap(),
        "2026-06-02T10:00:00.000000Z"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_repo_unreachable_returns_error_and_clears_importing() {
    // sync_repo now accepts the sync request immediately (202) and runs the
    // actual sync in a background task. The test verifies that the background
    // task clears importing and stores the error message.
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "sync-accepted-repo").await;

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "sync should be accepted immediately, got {}",
        resp.status()
    );

    wait_for_import_completion(&pool, repo_id).await;

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(
        !stats.importing,
        "importing should be cleared after sync fails"
    );
    assert!(
        stats.import_error.is_some(),
        "import_error should be set after sync fails"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_repo_times_out_on_hanging_borg_and_clears_importing() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    // A borg that never returns must not hang the import forever.
    let (_borg_dir, _borg_guard) = install_hanging_borg().await;
    // SAFETY: BORG_BINARY/env changes are serialised by borg_binary_lock.
    unsafe { std::env::set_var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS", "1") };

    let mut app = build_test_app(pool.clone());
    let repo_id = insert_test_repo(&pool, "hanging-borg-repo").await;

    let started = std::time::Instant::now();
    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    let elapsed = started.elapsed();

    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "sync should be accepted immediately, got {}",
        resp.status()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "sync should return quickly, took {elapsed:?}"
    );

    wait_for_import_completion(&pool, repo_id).await;

    // SAFETY: env var must remain set until the background task finishes.
    unsafe { std::env::remove_var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS") };

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(
        !stats.importing,
        "importing must be cleared after a timeout"
    );
    assert!(
        stats.import_error.is_some(),
        "import_error should be set after a timeout"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_archive_runs_in_background() {
    use tokio::time::{Duration, timeout};

    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let empty_list = r#"{"archives": []}"#;
    let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 0,
      "total_csize": 0,
      "unique_csize": 0,
      "total_chunks": 0,
      "total_unique_chunks": 0
    }
  }
}"#;
    let (_borg_dir, _borg_guard) =
        install_fake_borg(empty_list, empty_list, info_repo_json, "", "").await;

    let mut app = build_test_app(pool.clone());
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('del-host', 'hash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "delete-archive-repo").await;

    sqlx::query(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, matched, \
         archive_name) VALUES ($1, $2, NOW(), NOW(), 'success', true, $3)",
    )
    .bind(agent_id)
    .bind(repo_id)
    .bind("delete-me")
    .execute(&pool)
    .await
    .unwrap();
    let delete_archive_id: i64 = sqlx::query_scalar(
        "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT (repo_id, name) DO \
         UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(repo_id)
    .bind("delete-me")
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archive_index_jobs (archive_id, status) VALUES ($1, 'done') ON CONFLICT DO \
         NOTHING",
    )
    .bind(delete_archive_id)
    .execute(&pool)
    .await
    .unwrap();

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The audit entry is written last in the background task, so waiting for it
    // guarantees the borg delete and DB cleanup have already completed.
    timeout(Duration::from_secs(10), async {
        loop {
            let audit_rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'delete_archive' AND target_id = $1",
            )
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if audit_rows == 1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("background deletion should write an audit entry for this repo");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("delete-me")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 0, "the archive report should be removed");

    let index_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_index_jobs j JOIN archives a ON a.id = j.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
    )
    .bind(repo_id)
    .bind("delete-me")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        index_rows, 0,
        "index job rows should be removed with the archive"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_multiple_archives_queues_without_conflict() {
    use tokio::time::{Duration, timeout};

    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let empty_list = r#"{"archives": []}"#;
    let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 0,
      "total_csize": 0,
      "unique_csize": 0,
      "total_chunks": 0,
      "total_unique_chunks": 0
    }
  }
}"#;
    let (_borg_dir, _borg_guard) =
        install_fake_borg(empty_list, empty_list, info_repo_json, "", "").await;

    let mut app = build_test_app(pool.clone());
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('multi-del', 'hash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "multi-delete-repo").await;

    let names = ["arch-a", "arch-b", "arch-c"];
    for name in names {
        sqlx::query(
            "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
             matched, archive_name) VALUES ($1, $2, NOW(), NOW(), 'success', true, $3)",
        )
        .bind(agent_id)
        .bind(repo_id)
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Fire all deletions back to back; none should be rejected with a conflict.
    for name in names {
        let req = delete_request(&format!("/api/repos/{repo_id}/archives/{name}"));
        let resp = oneshot(&mut app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "concurrent deletes should be queued, not rejected"
        );
    }

    timeout(Duration::from_secs(15), async {
        loop {
            let done: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'delete_archive' AND target_id = $1",
            )
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if done == i64::try_from(names.len()).unwrap_or(i64::MAX) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("all queued deletions should eventually complete");

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0, "every queued archive should be deleted");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_repo_returns_409_when_already_importing() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "sync-conflict-repo").await;

    // pre-set importing = true to simulate in-progress sync
    server::db::set_repo_importing(&pool, repo_id, true)
        .await
        .unwrap();

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // flag must still be true (we didn't touch it)
    let importing: bool =
        sqlx::query_scalar("SELECT importing FROM repo_import_state WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        importing,
        "importing should remain true after rejected sync"
    );
}

/// Inserts a completed backup report, archive, and fully-indexed archive
/// files/paths for a stale archive that a sync should prune away.
#[cfg(test)]
async fn insert_stale_archive_with_index(pool: &PgPool, agent_id: i64, repo_id: i64) {
    let stale_started_at = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::days(1))
        .unwrap();
    let stale_finished_at = stale_started_at
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap();
    sqlx::query(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, NULL, $3, $4, 'success', 10, 5, 5, 5, 1, \
         300, NULL, '{}'::text[], NULL, true, $5, NULL)",
    )
    .bind(agent_id)
    .bind(repo_id)
    .bind(stale_started_at)
    .bind(stale_finished_at)
    .bind("stale-archive")
    .execute(pool)
    .await
    .unwrap();
    let stale_archive_id: i64 =
        sqlx::query_scalar("INSERT INTO archives (repo_id, name) VALUES ($1, $2) RETURNING id")
            .bind(repo_id)
            .bind("stale-archive")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO archive_index_jobs (archive_id, status, file_count) VALUES ($1, 'done', 1)",
    )
    .bind(stale_archive_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO archive_paths (repo_id, path) VALUES ($1, $2), ($1, $3)")
        .bind(repo_id)
        .bind("")
        .bind("stale.txt")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO archive_files (archive_id, path_id, parent_path_id, entry_type, size, mtime, \
         mode) SELECT $1, child.id, parent.id, 'f', 1, '', '' FROM archive_paths child JOIN \
         archive_paths parent ON parent.repo_id = $2 AND parent.path = $4 WHERE child.repo_id = \
         $2 AND child.path = $3",
    )
    .bind(stale_archive_id)
    .bind(repo_id)
    .bind("stale.txt")
    .bind("")
    .execute(pool)
    .await
    .unwrap();
}

#[cfg(test)]
async fn assert_stale_archive_purged(pool: &PgPool, repo_id: i64) {
    let stale_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(stale_count, 0);
    let stale_index_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_index_jobs j JOIN archives a ON a.id = j.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(stale_index_rows, 0);
    let stale_file_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_files f JOIN archives a ON a.id = f.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
    )
    .bind(repo_id)
    .bind("stale-archive")
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(stale_file_rows, 0);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
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
    let repo_list_lines = concat!(
        r#"{"name":"sync-archive-1","hostname":"web-server-01","#,
        r#""start":"2026-06-05T10:00:00Z","end":"2026-06-05T10:05:00Z","#,
        r#""duration":300.0,"stats":{"original_size":1000,"compressed_size":500,"#,
        r#""deduplicated_size":250,"nfiles":2}}"#,
    );

    let (_borg_dir, _borg_guard) = install_fake_borg(
        list_json,
        info_all_json,
        info_repo_json,
        repo_list_lines,
        json_lines,
    )
    .await;

    let (mut app, state) = build_test_app_with_state(pool.clone());
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, display_name, agent_token_hash) VALUES ($1, $2, $3) \
         RETURNING id",
    )
    .bind("stale-host")
    .bind("Stale Host")
    .bind("token-hash")
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "sync-success-repo").await;

    insert_stale_archive_with_index(&pool, agent_id, repo_id).await;

    let req = json_request(
        "POST",
        &format!("/api/repos/{repo_id}/sync?build_index=true"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    wait_for_import_completion(&pool, repo_id).await;

    let (status, file_count) = wait_for_archive_index(&pool, repo_id, "sync-archive-1").await;
    assert_eq!(status, "done");
    assert_eq!(file_count, Some(2));

    assert_stale_archive_purged(&pool, repo_id).await;

    let file_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_files f JOIN archives a ON a.id = f.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
    )
    .bind(repo_id)
    .bind("sync-archive-1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(file_rows, 2);

    state
        .background_task_tracker
        .assert_idle(std::time::Duration::from_secs(5))
        .await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_stats_summary_returns_200() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = get_request("/api/stats/summary");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_object(), "summary should be a JSON object");
    assert!(body.get("total_agents").unwrap().is_number());
    assert!(body.get("total_repos").unwrap().is_number());
    assert!(body.get("total_storage_bytes").unwrap().is_number());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_storage_breakdown_empty() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = get_request("/api/stats/storage-breakdown");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.is_array(), "storage breakdown should be a JSON array");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_storage_breakdown_with_data() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(
        entries.first().unwrap().get("name").unwrap(),
        "breakdown-repo"
    );
    assert_eq!(
        entries.first().unwrap().get("compressed_size").unwrap(),
        500_000
    );
    assert_eq!(
        entries.first().unwrap().get("deduplicated_size").unwrap(),
        250_000
    );
    // sole repo owns 100 % of storage
    let pct = entries
        .first()
        .unwrap()
        .get("percentage")
        .unwrap()
        .as_f64()
        .unwrap();
    assert!(
        (pct - 100.0).abs() < 0.01,
        "single repo should be 100%, got {pct}"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_reset_import_clears_state() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
#[ignore = "requires DATABASE_URL"]
async fn test_reset_import_cancels_active_sync() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let (_borg_dir, _borg_guard) = install_slow_borg_list(30).await;

    let mut app = build_test_app(pool.clone());
    let repo_id = insert_test_repo(&pool, "cancel-active-import-repo").await;

    let sync_req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let sync_resp = oneshot(&mut app, sync_req).await;
    assert_eq!(sync_resp.status(), StatusCode::ACCEPTED);

    let mut saw_importing = false;
    for _ in 0..20 {
        let stats = server::db::get_repo_with_stats(&pool, repo_id)
            .await
            .unwrap();
        if stats.importing {
            saw_importing = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(saw_importing, "sync should mark repo as importing");

    let reset_req = json_request("POST", &format!("/api/repos/{repo_id}/reset-import"), None);
    let reset_resp = oneshot(&mut app, reset_req).await;
    assert_eq!(reset_resp.status(), StatusCode::NO_CONTENT);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(
        !stats.importing,
        "reset-import should cancel the active sync and clear importing"
    );
    assert!(
        stats.import_error.is_none(),
        "reset-import should not leave an import error behind"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_auth_me_without_session() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    let mut app = build_test_app(pool.clone());

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_sessions_stored_as_hashes_not_plaintext(pool: sqlx::PgPool) {
    let plaintext_id = "verify-hash-storage-session-000000000000";

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('hash-verify-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let admin_role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(admin_role_id)
        .execute(&pool)
        .await
        .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let hashed_id = server::api::tokens::hash_token(plaintext_id);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&hashed_id)
        .bind(user_id)
        .bind(expires)
        .execute(&pool)
        .await
        .unwrap();

    // Verify the stored session id is NOT the plaintext value
    let stored_id: String = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(
        stored_id, plaintext_id,
        "session id must not be stored in plaintext"
    );

    // Verify the stored session id IS the SHA-256 hash of the plaintext
    let expected_hash = server::api::tokens::hash_token(plaintext_id);
    assert_eq!(
        stored_id, expected_hash,
        "stored session id must be SHA-256 hash of the original session id"
    );

    // Also verify that a lookup with the hashed value finds the session
    let found_session = server::db::get_session(&pool, &hashed_id).await.unwrap();
    assert_eq!(found_session.user_id, user_id);

    // Verify that a lookup with the plaintext does NOT find a session
    let plaintext_lookup = server::db::get_session(&pool, plaintext_id).await;
    assert!(
        plaintext_lookup.is_err(),
        "looking up session by plaintext should fail"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_login_response_includes_role(pool: sqlx::PgPool) {
    use std::net::SocketAddr;

    let mut app = build_test_app(pool.clone());

    // Create a test user with a known bcrypt hash and the 'viewer' role.
    let password = "viewer-password";
    let hash = tokio::task::spawn_blocking(move || bcrypt::hash(password, 4))
        .await
        .unwrap()
        .unwrap();

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, must_change_password)
         VALUES ('login-role-viewer', $1, false) RETURNING id",
    )
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let viewer_role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'viewer'")
        .fetch_one(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(viewer_role_id)
        .execute(&pool)
        .await
        .unwrap();

    // Login as this user and verify the role field is present.
    // The login handler extracts ConnectInfo<SocketAddr> from the request
    // extensions, so we must provide one.
    let body =
        serde_json::json!({ "username": "login-role-viewer", "password": "viewer-password" });
    let mut req = Request::builder()
        .uri("/api/auth/login")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo::<SocketAddr>(
            "127.0.0.1:54321".parse().unwrap(),
        ));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");

    let json: serde_json::Value = body_json(resp).await;
    assert!(
        json.get("user").and_then(|u| u.get("role")).is_some(),
        "login response must include user.role"
    );
    assert_eq!(
        json.get("user").and_then(|u| u.get("role")).unwrap(),
        "viewer",
        "viewer user should have 'viewer' role"
    );
}

// -- Excludes API tests --

/// Helper: insert a schedule directly into the DB (bypasses SSH check in the API).
#[cfg(test)]
async fn insert_test_schedule(pool: &sqlx::PgPool, agent_id: i64, repo_id: i64) -> i64 {
    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
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
        "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, $2, 0)",
    )
    .bind(schedule_id)
    .bind(agent_id)
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
    let mut app = build_test_app(pool.clone());

    let resp = oneshot(&mut app, get_request("/api/excludes")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("raw_text").unwrap(), "");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_global_excludes_roundtrip_preserves_blank_lines_and_comments(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(body.get("raw_text").unwrap(), raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_global_excludes_overwrite_replaces_fully(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    assert_eq!(body.get("raw_text").unwrap(), "only-this-one");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_per_agent_excludes_roundtrip_preserves_raw_text(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    // Set up agent and repo directly
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('exc-host', 'hash-exc') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "exc-repo").await;
    let schedule_id = insert_test_schedule(&pool, agent_id, repo_id).await;

    let raw = "# Cache dirs\n*.cache\npp:__pycache__\n\n# Runtime\n/proc\n/sys";

    sqlx::query(
        "INSERT INTO per_agent_excludes (schedule_id, agent_id, raw_text) VALUES ($1, $2, $3)",
    )
    .bind(schedule_id)
    .bind(agent_id)
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

    let per_agent = body
        .get("exclude_patterns_per_agent")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(per_agent.len(), 1);
    assert_eq!(
        per_agent.first().unwrap().get("agent_id").unwrap(),
        agent_id
    );
    assert_eq!(per_agent.first().unwrap().get("raw_text").unwrap(), raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_empty(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("version").unwrap(), 1);
    assert!(body.get("exported_at").unwrap().is_string());
    assert_eq!(body.get("hosts").unwrap().as_array().unwrap().len(), 0);
    assert_eq!(body.get("schedules").unwrap().as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_with_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns) VALUES ('export-host', 'Export Host', 'real-token', \
         ARRAY['/etc','/home'], ARRAY['*.log'])",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hosts = body.get("hosts").unwrap().as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(
        hosts.first().unwrap().get("hostname").unwrap(),
        "export-host"
    );
    assert_eq!(
        hosts.first().unwrap().get("display_name").unwrap(),
        "Export Host"
    );
    assert_eq!(
        hosts
            .first()
            .unwrap()
            .get("default_backup_paths")
            .unwrap()
            .get(0)
            .unwrap(),
        "/etc"
    );
    assert_eq!(
        hosts
            .first()
            .unwrap()
            .get("default_backup_paths")
            .unwrap()
            .get(1)
            .unwrap(),
        "/home"
    );
    assert_eq!(
        hosts
            .first()
            .unwrap()
            .get("default_exclude_patterns")
            .unwrap()
            .get(0)
            .unwrap(),
        "*.log"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_config_skips_imported_token_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('real-host', 'real-token'), \
         ('imported-host', 'imported:no-auth')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let hosts = body.get("hosts").unwrap().as_array().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts.first().unwrap().get("hostname").unwrap(), "real-host");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_creates_hosts(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let payload = json!({
        "version": 1,
        "exported_at": "2026-01-01T00:00:00Z",
        "hosts": [
            {
                "hostname": "new-host-1",
                "display_name": "New Host 1",
                "default_backup_paths": ["/etc", "/home"],
                "default_exclude_patterns": ["*.log"],
                "default_pre_backup_commands": "[]",
                "default_post_backup_commands": "[]",
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
    assert_eq!(body.get("hosts_created").unwrap(), 1);
    assert_eq!(body.get("hosts_updated").unwrap(), 0);
    assert_eq!(body.get("schedules_created").unwrap(), 0);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE hostname = 'new-host-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_updates_existing_host(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('existing-host', 'real-token')",
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
                "default_pre_backup_commands": "[]",
                "default_post_backup_commands": "[]",
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
    assert_eq!(body.get("hosts_created").unwrap(), 0);
    assert_eq!(body.get("hosts_updated").unwrap(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_rejects_wrong_version(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

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
    let mut app = build_test_app(pool.clone());

    sqlx::query(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('sched-host', 'real-token')",
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
    assert_eq!(body.get("schedules_created").unwrap(), 0);
    let warnings = body.get("warnings").unwrap().as_array().unwrap();
    assert!(!warnings.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_creates_schedule_with_matching_repo(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "import-repo").await;
    let _ = repo_id;

    sqlx::query(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('import-target', 'real-token')",
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
    assert_eq!(body.get("schedules_created").unwrap(), 1);
    assert_eq!(body.get("warnings").unwrap().as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_then_import_roundtrip(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns) VALUES ('roundtrip-host', 'RT Host', 'real-token', \
         ARRAY['/etc'], ARRAY['*.swp'])",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let export = body_json(resp).await;

    sqlx::query("DELETE FROM agents WHERE hostname = 'roundtrip-host'")
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
    assert_eq!(body.get("hosts_created").unwrap(), 1);

    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT default_backup_paths FROM agents WHERE hostname = 'roundtrip-host'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(paths, vec!["/etc"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_config_repo_with_tags(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let payload = json!({
        "version": 1,
        "exported_at": "2026-06-01T00:00:00Z",
        "hosts": [],
        "schedules": [],
        "repos": [
            {
                "name": "tagged-repo",
                "repo_path": "/backups/tagged",
                "ssh_user": "borg",
                "ssh_host": "remote",
                "ssh_port": 22,
                "compression": "lz4",
                "encryption": "repokey",
                "enabled": true,
                "sync_schedule": "0 0,12 * * *",
                "quota_warn_bytes": null,
                "quota_critical_bytes": null,
                "quota_warn_action": "",
                "quota_critical_action": "",
                "tags": ["critical", "production"]
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
    assert_eq!(body.get("repos_created").unwrap(), 1);

    let repo_id: i64 = sqlx::query_scalar("SELECT id FROM repos WHERE name = 'tagged-repo'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let tag_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.name, t.scope FROM tags t JOIN repo_tags rt ON rt.tag_id = t.id WHERE \
         rt.repo_id = $1 ORDER BY t.name",
    )
    .bind(repo_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(tag_rows.len(), 2);
    assert_eq!(
        tag_rows.first().unwrap(),
        &("critical".to_string(), "repo".to_string())
    );
    assert_eq!(
        tag_rows.get(1).unwrap(),
        &("production".to_string(), "repo".to_string())
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_export_then_import_repo_roundtrip(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;

    // Create a repo with all the trimmings: quota, SSH host key, tags.
    let encryption_key: [u8; 32] =
        shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
    let passphrase_encrypted =
        shared::crypto::encrypt_passphrase("borg-pass", &encryption_key).unwrap();
    let repo_id: i64 = sqlx::query_scalar(
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption, sync_schedule) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL) \
         RETURNING id",
    )
    .bind("roundtrip-repo")
    .bind("/backups/roundtrip")
    .bind("borg")
    .bind("remote-host")
    .bind(2222i32)
    .bind(&passphrase_encrypted)
    .bind("lz4")
    .bind("repokey")
    .fetch_one(&pool)
    .await
    .unwrap();

    // Insert SSH host key
    sqlx::query("UPDATE repos SET ssh_host_key = $2 WHERE name = $1")
        .bind("roundtrip-repo")
        .bind("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...")
        .execute(&pool)
        .await
        .unwrap();

    // Insert quota
    sqlx::query(
        "INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, warn_action, \
         critical_action, enabled, updated_at) VALUES ($1, $2, $3, $4, $5, true, NOW())",
    )
    .bind(repo_id)
    .bind(1_000_000_000i64)
    .bind(2_000_000_000i64)
    .bind("notify_only")
    .bind("block_backups")
    .execute(&pool)
    .await
    .unwrap();

    // Create tags and associate them with the repo
    let tag1_id: i64 = sqlx::query_scalar(
        "INSERT INTO tags (name, color, scope) VALUES ('critical', '#EF4444', 'repo') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let tag2_id: i64 = sqlx::query_scalar(
        "INSERT INTO tags (name, color, scope) VALUES ('production', '#3B82F6', 'repo') RETURNING \
         id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO repo_tags (repo_id, tag_id) VALUES ($1, $2)")
        .bind(repo_id)
        .bind(tag1_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO repo_tags (repo_id, tag_id) VALUES ($1, $2)")
        .bind(repo_id)
        .bind(tag2_id)
        .execute(&pool)
        .await
        .unwrap();

    // Export config
    let mut app = build_test_app(pool.clone());
    let resp = oneshot(&mut app, get_request("/api/config/export")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let export = body_json(resp).await;

    // The export must contain one repo
    let repos = export.get("repos").and_then(|v| v.as_array()).unwrap();
    assert_eq!(repos.len(), 1);
    let repo = repos.first().unwrap();
    assert_eq!(repo["name"], "roundtrip-repo");
    assert_eq!(
        repo["ssh_host_key"],
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI..."
    );
    assert_eq!(repo["quota_warn_bytes"], 1_000_000_000);
    assert_eq!(repo["quota_critical_bytes"], 2_000_000_000);
    assert_eq!(repo["quota_warn_action"], "notify_only");
    assert_eq!(repo["quota_critical_action"], "block_backups");
    assert!(
        repo["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("critical"))
    );
    assert!(
        repo["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("production"))
    );
    // Passphrase must never be exported
    assert!(repo.get("passphrase").is_none());

    // Repo was created without a sync schedule; the export must preserve that
    assert!(
        repo.get("sync_schedule").and_then(|v| v.as_str()).is_none(),
        "sync_schedule must be null in the export when the repo has none"
    );

    // Wipe the repo
    sqlx::query("DELETE FROM repo_tags WHERE repo_id = $1")
        .bind(repo_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM tags")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM repo_quotas WHERE repo_id = $1")
        .bind(repo_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE repos SET ssh_host_key = NULL WHERE name = 'roundtrip-repo'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM repos WHERE id = $1")
        .bind(repo_id)
        .execute(&pool)
        .await
        .unwrap();

    // Re-import the same export
    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(export)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("repos_created").unwrap(), 1);

    // Verify the repo was restored
    let new_repo_id: i64 = sqlx::query_scalar("SELECT id FROM repos WHERE name = 'roundtrip-repo'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Check SSH host key
    let host_key: String =
        sqlx::query_scalar("SELECT ssh_host_key FROM repos WHERE name = 'roundtrip-repo'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(host_key, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...");

    // Check quota
    let (warn_bytes, critical_bytes, warn_action, critical_action): (
        Option<i64>,
        Option<i64>,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT warn_bytes, critical_bytes, warn_action, critical_action FROM repo_quotas WHERE \
         repo_id = $1",
    )
    .bind(new_repo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(warn_bytes, Some(1_000_000_000));
    assert_eq!(critical_bytes, Some(2_000_000_000));
    assert_eq!(warn_action, "notify_only");
    assert_eq!(critical_action, "block_backups");

    // Check tags (scope must be 'repo', not 'global')
    let tag_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.name, t.scope FROM tags t JOIN repo_tags rt ON rt.tag_id = t.id WHERE \
         rt.repo_id = $1 ORDER BY t.name",
    )
    .bind(new_repo_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tag_rows.len(), 2);
    assert_eq!(
        tag_rows.first().unwrap(),
        &("critical".to_string(), "repo".to_string())
    );
    assert_eq!(
        tag_rows.get(1).unwrap(),
        &("production".to_string(), "repo".to_string())
    );

    // Repo was created without a sync schedule; re-import must preserve null
    let imported_sync_schedule: Option<String> =
        sqlx::query_scalar("SELECT sync_schedule FROM repos WHERE id = $1")
            .bind(new_repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        imported_sync_schedule.is_none(),
        "re-imported repo must have null sync_schedule (not DB default)"
    );

    // The imported repo should be marked as importing (placeholder passphrase)
    let importing: bool =
        sqlx::query_scalar("SELECT importing FROM repo_import_state WHERE repo_id = $1")
            .bind(new_repo_id)
            .fetch_optional(&pool)
            .await
            .unwrap()
            .unwrap_or(false);
    assert!(
        importing,
        "imported repo must be guarded against scheduler sync"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_repo_updates_existing(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;

    // Start with a repo
    let encryption_key: [u8; 32] =
        shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
    let passphrase_encrypted =
        shared::crypto::encrypt_passphrase("original-pass", &encryption_key).unwrap();
    let repo_id: i64 = sqlx::query_scalar(
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind("update-repo")
    .bind("/backups/original")
    .bind("borg")
    .bind("old-host")
    .bind(22i32)
    .bind(&passphrase_encrypted)
    .bind("lz4")
    .bind("repokey")
    .fetch_one(&pool)
    .await
    .unwrap();

    // Give it a tag so we can verify update-side tag sync
    let tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO tags (name, color, scope) VALUES ('legacy', '#888888', 'repo') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO repo_tags (repo_id, tag_id) VALUES ($1, $2)")
        .bind(repo_id)
        .bind(tag_id)
        .execute(&pool)
        .await
        .unwrap();

    // Give it an SSH host key
    sqlx::query("UPDATE repos SET ssh_host_key = $2 WHERE name = $1")
        .bind("update-repo")
        .bind("old-host-key")
        .execute(&pool)
        .await
        .unwrap();

    // Give it a quota
    sqlx::query(
        "INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, warn_action, \
         critical_action, enabled, updated_at) VALUES ($1, $2, $3, $4, $5, true, NOW())",
    )
    .bind(repo_id)
    .bind(500_000_000i64)
    .bind(1_000_000_000i64)
    .bind("notify_only")
    .bind("notify_only")
    .execute(&pool)
    .await
    .unwrap();

    // Import a config that matches the same repo name but with different settings
    let payload = json!({
        "version": 1,
        "exported_at": "2026-06-01T00:00:00Z",
        "hosts": [],
        "schedules": [],
        "repos": [
            {
                "name": "update-repo",
                "repo_path": "/backups/updated",
                "ssh_user": "borg",
                "ssh_host": "new-host",
                "ssh_port": 2222,
                "compression": "zstd",
                "encryption": "repokey",
                "enabled": true,
                "sync_schedule": "0 */6 * * *",
                "ssh_host_key": "new-host-key-ssh-ed25519",
                "quota_warn_bytes": 2_000_000_000,
                "quota_critical_bytes": 2_000_000_000,
                "quota_warn_action": "notify_only",
                "quota_critical_action": "block_backups",
                "tags": ["updated-tag"]
            }
        ]
    });

    let mut app = build_test_app(pool.clone());
    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("repos_updated").unwrap(), 1);
    assert_eq!(body.get("repos_created").unwrap(), 0);

    // Verify the repo was updated, not duplicated
    let repo_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM repos WHERE name = 'update-repo'")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(repo_ids.len(), 1);
    let updated_id = *repo_ids.first().unwrap();
    assert_eq!(updated_id, repo_id);

    // Verify updated fields
    let (repo_path, ssh_host, ssh_port, compression, sync_schedule): (
        String,
        String,
        i32,
        String,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT repo_path, ssh_host, ssh_port, compression, sync_schedule FROM repos WHERE id = $1",
    )
    .bind(updated_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(repo_path, "/backups/updated");
    assert_eq!(ssh_host, "new-host");
    assert_eq!(ssh_port, 2222);
    assert_eq!(compression, "zstd");
    assert_eq!(sync_schedule.as_deref(), Some("0 */6 * * *"));

    // Verify SSH host key was updated
    let host_key: String =
        sqlx::query_scalar("SELECT ssh_host_key FROM repos WHERE name = 'update-repo'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(host_key, "new-host-key-ssh-ed25519");

    // Verify quota was upserted
    let (warn_bytes, critical_bytes, warn_action, critical_action): (
        Option<i64>,
        Option<i64>,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT warn_bytes, critical_bytes, warn_action, critical_action FROM repo_quotas WHERE \
         repo_id = $1",
    )
    .bind(updated_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(warn_bytes, Some(2_000_000_000));
    assert_eq!(critical_bytes, Some(2_000_000_000));
    assert_eq!(warn_action, "notify_only");
    assert_eq!(critical_action, "block_backups");

    // Verify tags were synced (old tag replaced by new one)
    let tag_names: Vec<String> = sqlx::query_scalar(
        "SELECT t.name FROM tags t JOIN repo_tags rt ON rt.tag_id = t.id WHERE rt.repo_id = $1 \
         ORDER BY t.name",
    )
    .bind(updated_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tag_names, vec!["updated-tag"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_import_repo_clears_sync_schedule(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;

    // Create a repo WITH a sync schedule
    let encryption_key: [u8; 32] =
        shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
    let passphrase_encrypted =
        shared::crypto::encrypt_passphrase("borg-pass", &encryption_key).unwrap();
    let repo_id: i64 = sqlx::query_scalar(
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption, sync_schedule) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         RETURNING id",
    )
    .bind("clear-schedule-repo")
    .bind("/backups/scheduled")
    .bind("borg")
    .bind("old-host")
    .bind(22i32)
    .bind(&passphrase_encrypted)
    .bind("lz4")
    .bind("repokey")
    .bind("0 0,12 * * *")
    .fetch_one(&pool)
    .await
    .unwrap();

    // Import a config that matches the same repo name with sync_schedule: null
    let payload = json!({
        "version": 1,
        "exported_at": "2026-06-01T00:00:00Z",
        "hosts": [],
        "schedules": [],
        "repos": [
            {
                "name": "clear-schedule-repo",
                "repo_path": "/backups/scheduled",
                "ssh_user": "borg",
                "ssh_host": "old-host",
                "ssh_port": 22,
                "compression": "lz4",
                "encryption": "repokey",
                "enabled": true,
                "sync_schedule": null
            }
        ]
    });

    let mut app = build_test_app(pool.clone());
    let resp = oneshot(
        &mut app,
        json_request("POST", "/api/config/import", Some(payload)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("repos_updated").unwrap(), 1);

    // Verify the repo still exists (single row)
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM repos WHERE name = 'clear-schedule-repo'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);

    // Verify sync_schedule was cleared to NULL
    let sync_schedule: Option<String> =
        sqlx::query_scalar("SELECT sync_schedule FROM repos WHERE id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        sync_schedule.is_none(),
        "importing with null sync_schedule must clear the existing schedule"
    );
}

// -- admin-only enforcement on agent-mutating endpoints --

const NON_ADMIN_SESSION_ID: &str = "non-admin-session-id-000000000000000";

#[cfg(test)]
async fn create_non_admin_user_and_session(pool: &PgPool) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('integration-viewer', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') ON CONFLICT (username) DO \
         UPDATE SET username = EXCLUDED.username RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let viewer_role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'viewer'")
        .fetch_one(pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(viewer_role_id)
        .execute(pool)
        .await
        .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let hashed_id = server::api::tokens::hash_token(NON_ADMIN_SESSION_ID);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO \
         UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(&hashed_id)
    .bind(user_id)
    .bind(expires)
    .execute(pool)
    .await
    .unwrap();
}

#[cfg(test)]
fn non_admin_delete_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("cookie", format!("session={NON_ADMIN_SESSION_ID}"))
        .body(Body::empty())
        .unwrap()
}

#[cfg(test)]
fn non_admin_get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .header("cookie", format!("session={NON_ADMIN_SESSION_ID}"))
        .body(Body::empty())
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_agent_forbidden_for_non_admin(pool: sqlx::PgPool) {
    create_non_admin_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query("INSERT INTO agents (hostname, agent_token_hash) VALUES ('guarded-host', 'hash')")
        .execute(&pool)
        .await
        .unwrap();

    let resp = oneshot(
        &mut app,
        non_admin_delete_request("/api/agents/guarded-host"),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a non-admin user must not be able to delete an agent"
    );

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE hostname = 'guarded-host'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        remaining, 1,
        "the agent must still exist after a rejected delete"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_agent_allowed_for_admin(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    sqlx::query("INSERT INTO agents (hostname, agent_token_hash) VALUES ('admin-host', 'hash')")
        .execute(&pool)
        .await
        .unwrap();

    let resp = oneshot(&mut app, delete_request("/api/agents/admin-host")).await;
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "an admin user must be able to delete an agent"
    );

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE hostname = 'admin-host'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        remaining, 0,
        "the agent should be removed by an admin delete"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn get_logs_forbidden_for_non_admin(pool: sqlx::PgPool) {
    create_non_admin_user_and_session(&pool).await;
    let mut app = build_test_app(pool);

    let resp = oneshot(&mut app, non_admin_get_request("/api/logs")).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a non-admin user must not be able to read logs"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn get_logs_allowed_for_admin(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool);

    let resp = oneshot(&mut app, get_request("/api/logs")).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "an admin user must be able to read logs"
    );
}

// -- must_change_password enforcement --

const MCP_SESSION_ID: &str = "must-change-password-session-0000000";

#[cfg(test)]
async fn create_must_change_password_user_and_session(pool: &PgPool) {
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, must_change_password) VALUES ('mcp-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', true) ON CONFLICT \
         (username) DO UPDATE SET must_change_password = true RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let admin_role_id: i64 = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'admin'")
        .fetch_one(pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(admin_role_id)
        .execute(pool)
        .await
        .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let hashed_id = server::api::tokens::hash_token(MCP_SESSION_ID);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO \
         UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(&hashed_id)
    .bind(user_id)
    .bind(expires)
    .execute(pool)
    .await
    .unwrap();
}

#[cfg(test)]
fn mcp_session_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method(method)
        .header("cookie", format!("session={MCP_SESSION_ID}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn must_change_password_blocks_regular_endpoints() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_must_change_password_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let resp = oneshot(&mut app, mcp_session_request("GET", "/api/agents")).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "must_change_password should block access to /api/agents"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn must_change_password_allows_me_endpoint() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_must_change_password_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let resp = oneshot(&mut app, mcp_session_request("GET", "/api/auth/me")).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/api/auth/me should be accessible even with must_change_password"
    );
    let body = body_json(resp).await;
    assert_eq!(body.get("must_change_password").unwrap(), true);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_list_schedules_for_repo() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "sched-repo-endpoint").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('sched-endpoint-host', 'hash2') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let schedule_id = insert_test_schedule(&pool, agent_id, repo_id).await;

    // Returns schedules for the correct repo
    let req = get_request(&format!("/api/repos/{repo_id}/schedules"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let schedules = body.as_array().unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules.first().unwrap().get("id").unwrap(), schedule_id);
    assert_eq!(
        schedules
            .first()
            .unwrap()
            .get("target_hostnames")
            .unwrap()
            .as_array()
            .unwrap()
            .first()
            .unwrap(),
        "sched-endpoint-host"
    );

    // Returns empty list for a different repo
    let other_repo_id = insert_test_repo(&pool, "sched-repo-other").await;
    let req = get_request(&format!("/api/repos/{other_repo_id}/schedules"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

// -- archive resync reliability --

/// Regression test for: repos with no backups getting stuck in "Listing archives..." forever.
///
/// `refresh_repo_info_stats` was called after an empty borg-list result without any
/// timeout guard. If `borg info` hung (e.g. due to a stalled SSH connection), the
/// importing flag was never cleared and the repo appeared stuck indefinitely.
///
/// This test installs a fake borg that returns an empty archive list immediately but
/// hangs on `borg info`, sets the per-command timeout to 1 s, and verifies that the
/// sync endpoint returns quickly and always clears the importing flag.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_empty_repo_does_not_hang_when_borg_info_hangs() {
    let _borg_lock = borg_binary_lock().await;
    // SAFETY: serialised by borg_binary_lock.
    unsafe { std::env::set_var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS", "1") };

    let (_borg_dir, _borg_guard) = install_borg_empty_list_hanging_info().await;

    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());
    let repo_id = insert_test_repo(&pool, "empty-repo-hanging-info").await;

    let started = std::time::Instant::now();
    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);

    // Without the fix the handler blocks on `borg info` until killed; the outer
    // timeout catches that hang and fails the test.
    let resp = tokio::time::timeout(std::time::Duration::from_secs(15), oneshot(&mut app, req))
        .await
        .expect("sync must complete within 15 s even when borg info hangs");

    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "sync should return quickly once borg info times out, took {elapsed:?}"
    );
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "sync should be accepted immediately, got {}",
        resp.status()
    );

    wait_for_import_completion(&pool, repo_id).await;

    // SAFETY: env var must remain set until the background task finishes.
    unsafe { std::env::remove_var("ASSIMILATE_BORG_QUERY_TIMEOUT_SECS") };

    let importing: bool =
        sqlx::query_scalar("SELECT importing FROM repo_import_state WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !importing,
        "importing must be cleared even when borg info hangs"
    );
}

/// Regression test for: the scheduled-sync loop blocking on each repo sequentially.
///
/// `run_repo_sync` previously called `sync_existing_archives` inline in a `for`
/// loop, so a slow repo held up every subsequent repo. With two repos that each
/// take `BORG_DELAY_SECS` seconds, sequential processing would block for at least
/// `BORG_DELAY_SECS` * 2; concurrent dispatching should return almost immediately
/// and let both syncs run in parallel.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_scheduler_dispatches_repo_syncs_concurrently() {
    // Each borg call sleeps for this long, simulating a slow network / large repo.
    const BORG_DELAY_SECS: u64 = 2;

    let _borg_lock = borg_binary_lock().await;

    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let (_borg_dir, _borg_guard) = install_slow_borg_list(BORG_DELAY_SECS).await;

    let repo_a = insert_test_repo(&pool, "concurrent-sync-repo-a").await;
    let repo_b = insert_test_repo(&pool, "concurrent-sync-repo-b").await;

    // Both repos are enabled and have a sync schedule that is already due.
    for repo_id in [repo_a, repo_b] {
        sqlx::query("UPDATE repos SET enabled = true, sync_schedule = '* * * * *' WHERE id = $1")
            .bind(repo_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO repo_stats (repo_id, last_synced_at) VALUES ($1, '1970-01-01T00:00:00Z') \
             ON CONFLICT (repo_id) DO UPDATE SET last_synced_at = EXCLUDED.last_synced_at",
        )
        .bind(repo_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-integration").unwrap();
    let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
    let repo_op_tracker = server::repo_op_tracker::RepoOpTracker::default();

    let repo_lock = server::RepoLock::default();
    let started = std::time::Instant::now();
    server::scheduler::run_repo_sync(
        &pool,
        &encryption_key,
        &ui_broadcast,
        &repo_op_tracker,
        &repo_lock,
        &server::background_tasks::BackgroundTaskTracker::default(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;
    let dispatch_elapsed = started.elapsed();

    // Sequential (buggy): run_repo_sync blocks for >= BORG_DELAY_SECS per repo.
    // Concurrent (fixed): run_repo_sync dispatches tasks and returns immediately.
    assert!(
        dispatch_elapsed < std::time::Duration::from_secs(BORG_DELAY_SECS),
        "run_repo_sync should dispatch all syncs without blocking on each one; took \
         {dispatch_elapsed:?} (sequential would take >={}s)",
        BORG_DELAY_SECS * 2,
    );

    // Wait for both background tasks to finish and verify the importing flag is cleared.
    for repo_id in [repo_a, repo_b] {
        tokio::time::timeout(std::time::Duration::from_secs(BORG_DELAY_SECS + 5), async {
            loop {
                let importing: bool = sqlx::query_scalar(
                    "SELECT importing FROM repo_import_state WHERE repo_id = $1",
                )
                .bind(repo_id)
                .fetch_one(&pool)
                .await
                .unwrap();
                if !importing {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("repo {repo_id} sync did not complete within expected time"));
    }
}

/// Regression test: full sync must keep the fast manifest-only `borg list`
/// path, then fetch authoritative per-archive metadata only after discovery.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_fetches_missing_hostname_via_borg_info() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let list_json = r#"{
  "archives": [
    {
      "name": "web-server-01-backup-2026-06-05T02:00:00",
      "start": "2026-06-05T02:00:00Z",
      "duration": 300.0
    }
  ]
}"#;
    let info_all_json = r#"{
  "archives": [
    {
      "name": "web-server-01-backup-2026-06-05T02:00:00",
      "hostname": "web-server-01",
      "start": "2026-06-05T02:00:00Z",
      "end": "2026-06-05T02:05:00Z",
      "duration": 300.0
    }
  ]
}"#;
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

    let (_borg_dir, _borg_guard) =
        install_fake_borg(list_json, info_all_json, info_repo_json, "", "").await;

    let (mut app, state) = build_test_app_with_state(pool.clone());
    let repo_id = insert_test_repo(&pool, "hostname-format-repo").await;

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    wait_for_import_completion(&pool, repo_id).await;

    let imported_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name IS NOT NULL",
    )
    .bind(repo_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(imported_count, 1, "archive should have been imported");

    let unknown_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE hostname = 'unknown'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        unknown_count, 0,
        "no placeholder agent should be created with hostname 'unknown'"
    );

    let correct_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE hostname = 'web-server-01'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        correct_count, 1,
        "placeholder agent should be created with hostname from borg list --format output"
    );

    let token_hash: String =
        sqlx::query_scalar("SELECT agent_token_hash FROM agents WHERE hostname = 'web-server-01'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        token_hash, "imported:no-auth",
        "placeholder agent should carry the imported sentinel token"
    );

    state
        .background_task_tracker
        .assert_idle(std::time::Duration::from_secs(5))
        .await;
}

/// Regression test: borg list exits 0 but outputs unparseable text.
///
/// Previously, a parse failure was silently treated as an empty archive list,
/// which would prune all existing archive records. Now it must be a hard error
/// so no records are touched.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_returns_error_on_malformed_borg_list_json() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let info_repo_json = r#"{"cache": {"stats": {"total_size": 0, "total_csize": 0}}}"#;

    // borg list exits 0 but stdout is not valid JSON
    let (_borg_dir, _borg_guard) =
        install_fake_borg("this is not valid json", "{}", info_repo_json, "", "").await;

    let mut app = build_test_app(pool.clone());
    let repo_id = insert_test_repo(&pool, "malformed-json-repo").await;

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "sync should be accepted immediately, got {}",
        resp.status()
    );

    wait_for_import_completion(&pool, repo_id).await;

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(
        stats.import_error.is_some(),
        "import_error should be set after malformed JSON sync fails"
    );
}

/// Regression test: borg list exits 0 with valid JSON but no `archives` key.
///
/// The `archives` array is required; a missing key must be a hard error for the
/// same reason as malformed JSON - silently treating it as empty would prune
/// all existing archive records.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_returns_error_when_borg_list_json_has_no_archives_key() {
    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let info_repo_json = r#"{"cache": {"stats": {"total_size": 0, "total_csize": 0}}}"#;

    // borg list exits 0 with valid JSON but no `archives` field
    let (_borg_dir, _borg_guard) = install_fake_borg(
        r#"{"encryption": {"mode": "none"}}"#,
        "{}",
        info_repo_json,
        "",
        "",
    )
    .await;

    let mut app = build_test_app(pool.clone());
    let repo_id = insert_test_repo(&pool, "missing-archives-key-repo").await;

    let req = json_request("POST", &format!("/api/repos/{repo_id}/sync"), None);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "sync should be accepted immediately, got {}",
        resp.status()
    );

    wait_for_import_completion(&pool, repo_id).await;

    let stats = server::db::get_repo_with_stats(&pool, repo_id)
        .await
        .unwrap();
    assert!(
        stats.import_error.is_some(),
        "import_error should be set after no-archives-key sync fails"
    );
}
