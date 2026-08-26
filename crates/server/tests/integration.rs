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
use server::{
    api::tokens::hash_token,
    archive_index::codec::{self, DirEntry},
};
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

/// Build a JSON POST request with a `ConnectInfo<SocketAddr>` extension
/// pre-inserted, needed by any handler (e.g. `totp_verify_login`,
/// `totp_recovery`) that resolves the caller's IP for rate limiting /
/// brute-force tracking. `oneshot()` calls the router directly, bypassing
/// the `into_make_service_with_connect_info` wrapper that normally
/// supplies this extension.
#[cfg(test)]
fn json_post_request_with_connect_info(uri: &str, body: &Value) -> Request<Body> {
    let mut req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
            "127.0.0.1:54321".parse().unwrap(),
        ));
    req
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
        session_idle_timeout_minutes: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(480)),
        shutdown_token: tokio_util::sync::CancellationToken::new(),
        client_ip_resolver: server::client_ip::ClientIpResolver::new(),
        task_registry: shared::task_registry::TaskRegistry::default(),

        user_rate_limiter: server::rate_limit::UserRateLimiter::new(
            60,
            std::time::Duration::from_mins(1),
        ),
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
            "/api/auth/totp/verify-login",
            post(server::api::totp::totp_verify_login),
        )
        .route(
            "/api/auth/totp/disable",
            post(server::api::totp::totp_disable),
        )
        .route(
            "/api/auth/totp/recovery",
            post(server::api::totp::totp_recovery),
        )
        .route("/api/auth/sessions", get(server::api::auth::list_sessions))
        .route(
            "/api/auth/sessions/{session_id}",
            delete(server::api::auth::revoke_session),
        )
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
        .route(
            "/api/agents/{hostname}/repos/{repo_id}/cancel-backup",
            post(server::api::agents::cancel_agent_backup),
        )
}

#[cfg(test)]
fn test_app_repo_routes() -> Router<server::AppState> {
    Router::new()
        .route("/api/repos", get(server::api::repos::list_repos))
        .route(
            "/api/repos/stats",
            get(server::api::repos::list_repos_with_stats),
        )
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
            "/api/schedules/{id}",
            get(server::api::schedules::get_schedule)
                .put(server::api::schedules::update_schedule)
                .delete(server::api::schedules::delete_schedule),
        )
        .route(
            "/api/schedules/{id}/sources",
            get(server::api::schedules::list_schedule_backup_sources),
        )
        .route(
            "/api/schedules/{id}/run",
            post(server::api::schedules::run_schedule_now),
        )
        .route(
            "/api/schedules/{id}/cancel",
            post(server::api::schedules::cancel_running_backup),
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
        .route("/api/stats/calendar", get(server::api::stats::calendar))
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
        .route(
            "/api/system/settings",
            get(server::api::system::get_settings).put(server::api::system::update_settings),
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
    build_test_app_with_idle_timeout(pool, 480)
}

/// Build a test app with a custom session idle timeout (in minutes).
#[cfg(test)]
fn build_test_app_with_idle_timeout(
    pool: PgPool,
    idle_timeout_minutes: i64,
) -> (Router, server::AppState) {
    let state = build_test_state(pool);
    state
        .session_idle_timeout_minutes
        .store(idle_timeout_minutes, std::sync::atomic::Ordering::Relaxed);

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
    // Every invoked subcommand is appended here so tests can assert which
    // borg operations actually ran (e.g. that compact follows a delete).
    let calls_log = tempdir.path().join("calls.log").display().to_string();
    let script = format!(
        r#"#!/bin/sh
set -eu
echo "$1" >> "{calls_log}"
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
  compact)
    if [ -n "${{FAKE_BORG_COMPACT_EXIT:-}}" ]; then
      echo "fake compact failure" >&2
      exit "$FAKE_BORG_COMPACT_EXIT"
    fi
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

/// Polls `install_fake_borg`'s call log until `subcommand` has been invoked
/// at least `expected` times, then returns the count actually observed.
#[cfg(test)]
async fn wait_for_calls_log_count(tempdir: &TempDir, subcommand: &str, expected: usize) -> usize {
    use tokio::time::{Duration, timeout};

    timeout(Duration::from_secs(10), async {
        loop {
            let content = tokio::fs::read_to_string(tempdir.path().join("calls.log"))
                .await
                .unwrap_or_default();
            let count = content.lines().filter(|l| *l == subcommand).count();
            if count >= expected {
                return count;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for {expected} '{subcommand}' call(s) in the fake borg log")
    })
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
         per_agent_file_change_patterns, archive_dirs, archive_tags, archive_index_jobs, \
         archive_paths, archives, backup_sources, backup_reports, canary_results, repo_tags, \
         repo_stats, repo_import_state, repo_last_op, repo_quotas, repo_relocation_pending_hosts, \
         schedules, dismissed_dashboard_findings, push_subscriptions, api_tokens, sessions, \
         user_roles, user_groups, repo_permissions, totp_attempts, users, groups, tags, repos, \
         agents, notification_channels CASCADE",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE excludes_global_config SET raw_text = ''")
        .execute(pool)
        .await
        .unwrap();
    reset_system_settings(pool).await;
}

/// Restores `system_settings` to the state a freshly migrated database has.
///
/// These tests share one database, and unlike the other tables `system_settings`
/// was never reset between them, so any test that writes a setting silently
/// changed the behaviour of every test that ran afterwards. That is not
/// hypothetical: the settings partial-update test persists
/// `borg_query_timeout_secs = 120`, and `get_borg_timeout` consults the database
/// setting *before* the `ASSIMILATE_BORG_QUERY_TIMEOUT_SECS` environment
/// variable, so the borg-hang tests silently got a 120 s timeout instead of the
/// 1 s they asked for and blew past their own wait. CI hid it by starting from an
/// empty database every run; running the suite twice against one database, as one
/// does locally, reproduced it every time.
///
/// Only `retention_days` and `session_idle_timeout_minutes` are seeded by
/// migrations (`0001_schema.sql` and `20260707163000_account_security.sql`), so
/// clearing the table and re-inserting those two reproduces a fresh database
/// exactly.
#[cfg(test)]
async fn reset_system_settings(pool: &PgPool) {
    sqlx::query("DELETE FROM system_settings")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO system_settings (key, value) VALUES ('retention_days', '7'), \
         ('session_idle_timeout_minutes', '480')",
    )
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

/// A POST with no body and no `Content-Type` header at all - unlike
/// `json_request(.., None)`, which still sends `Content-Type: application/json`
/// with an empty body (a JSON parse error, not "no body"). Used to simulate a
/// caller predating an endpoint's JSON body being introduced.
#[cfg(test)]
fn post_request_without_body(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
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
async fn test_delete_archive_runs_compact_afterwards() {
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
    let (borg_dir, _borg_guard) =
        install_fake_borg(empty_list, empty_list, info_repo_json, "", "").await;

    let mut app = build_test_app(pool.clone());
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('compact-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "delete-archive-compact-repo").await;

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

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // A single delete should trigger exactly one compact once it finishes.
    let count = wait_for_calls_log_count(&borg_dir, "compact", 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let settled = wait_for_calls_log_count(&borg_dir, "compact", count).await;
    assert_eq!(
        settled, 1,
        "exactly one compact should run after a single archive delete"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_archive_logs_system_event_when_compact_fails() {
    use tokio::time::{Duration, timeout};

    let _borg_lock = borg_binary_lock().await;
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    sqlx::query("DELETE FROM system_events WHERE event_type = 'archive_compact_failed'")
        .execute(&pool)
        .await
        .unwrap();

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

    // SAFETY: tests serialize BORG_BINARY (and this) changes with borg_binary_lock.
    unsafe { std::env::set_var("FAKE_BORG_COMPACT_EXIT", "2") };

    let mut app = build_test_app(pool.clone());
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('compact-fail-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "delete-archive-compact-fail-repo").await;

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

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The delete itself must still succeed even though the compact that
    // follows it fails.
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
    .expect("the delete should complete even though the follow-up compact fails");

    let event_rows: i64 = timeout(Duration::from_secs(10), async {
        loop {
            let rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM system_events WHERE event_type = 'archive_compact_failed'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if rows > 0 {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("a failed compact should log an archive_compact_failed system event");
    assert_eq!(event_rows, 1);

    // SAFETY: env var must remain set until the background task finishes -
    // cleared here, before dropping the borg binary lock, same as other
    // tests that mutate process-global borg-related env vars.
    unsafe { std::env::remove_var("FAKE_BORG_COMPACT_EXIT") };
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_archive_transitions_straight_to_compact_without_a_stale_drained_broadcast() {
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

    let (mut app, state) = build_test_app_with_state(pool.clone());
    let mut ws_rx = state.ui_broadcast.subscribe();

    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('broadcast-order-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "delete-archive-broadcast-order-repo").await;

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

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Collect every RepoOpChanged kind broadcast for this repo (as
    // Option<RepoOpKind>, None meaning "op cleared") until the compact
    // phase begins.
    let mut kinds: Vec<Option<shared::protocol::RepoOpKind>> = Vec::new();
    timeout(Duration::from_secs(10), async {
        loop {
            let msg = loop {
                match ws_rx.recv().await {
                    Ok(m) => break m,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("ui broadcast channel closed unexpectedly: {e}"),
                }
            };
            if let shared::protocol::ServerToUi::RepoOpChanged { repo_id: rid, op } = msg
                && rid == repo_id
            {
                let kind = op.map(|o| o.kind);
                let reached_compact = kind == Some(shared::protocol::RepoOpKind::CompactRepo);
                kinds.push(kind);
                if reached_compact {
                    return;
                }
            }
        }
    })
    .await
    .expect("should observe the delete phase transition into the compact phase");

    let delete_idx = kinds
        .iter()
        .position(|k| *k == Some(shared::protocol::RepoOpKind::DeleteArchive))
        .expect("should have broadcast a delete_archive op");
    let compact_idx = kinds
        .iter()
        .position(|k| *k == Some(shared::protocol::RepoOpKind::CompactRepo))
        .expect("should have broadcast a compact_repo op");

    // A `None` here would tell clients this repo's delete queue has fully
    // drained while the compact that follows this very delete hasn't even
    // started yet - wiping every archive's client-side "deleting" state
    // prematurely (see PR #410 review).
    assert!(
        !kinds
            .get(delete_idx..compact_idx)
            .expect("delete_idx and compact_idx should be valid bounds into kinds")
            .contains(&None),
        "no RepoOpChanged with a cleared op should be broadcast between the delete and compact \
         phases of the same archive deletion, got {kinds:?}"
    );
}

/// The exact race a follow-up review flagged: when a repo has no operation
/// already active, `enqueue()` alone never sets `active`, so a broadcast
/// fired right after it (before the spawned task's `begin()` runs) carries
/// `op: None` - indistinguishable to clients from "this repo's delete queue
/// has fully drained". The frontend clears its per-archive "deleting" state
/// on exactly that signal, so a client that had just set its own "deleting"
/// state for the archive it asked to delete could see it wiped out by this
/// stale broadcast a moment later. The fix is to skip the enqueue-time
/// broadcast entirely when nothing was already active, since the spawned
/// task's `begin()` broadcasts the real `delete_archive` state moments
/// later regardless.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_archive_does_not_broadcast_a_drained_op_before_it_ever_begins() {
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

    let (mut app, state) = build_test_app_with_state(pool.clone());
    let mut ws_rx = state.ui_broadcast.subscribe();

    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('enqueue-broadcast-host', \
         'hash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "delete-archive-enqueue-broadcast-repo").await;

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

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The first RepoOpChanged broadcast for this repo must already report
    // the delete as active - never a cleared (`None`) op sent while it was
    // merely queued.
    let first_kind = timeout(Duration::from_secs(10), async {
        loop {
            let msg = loop {
                match ws_rx.recv().await {
                    Ok(m) => break m,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("ui broadcast channel closed unexpectedly: {e}"),
                }
            };
            if let shared::protocol::ServerToUi::RepoOpChanged { repo_id: rid, op } = msg
                && rid == repo_id
            {
                return op.map(|o| o.kind);
            }
        }
    })
    .await
    .expect("should observe a RepoOpChanged broadcast for this repo");

    assert_eq!(
        first_kind,
        Some(shared::protocol::RepoOpKind::DeleteArchive),
        "the first RepoOpChanged broadcast for a freshly queued delete must not report a cleared \
         op - a client that just marked this archive as deleting client-side would see that state \
         wiped out by a stale 'nothing happening' signal"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_delete_archive_broadcasts_archive_deleted_before_data_changed() {
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

    let (mut app, state) = build_test_app_with_state(pool.clone());
    let mut ws_rx = state.ui_broadcast.subscribe();

    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('archive-deleted-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo_id = insert_test_repo(&pool, "archive-deleted-broadcast-repo").await;

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

    let req = delete_request(&format!("/api/repos/{repo_id}/archives/delete-me"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The precise ArchiveDeleted event must name the deleted archive and
    // arrive before the generic DataChanged refresh signal - a client
    // reacting to it directly shouldn't have to wait on (or race) the
    // broader refetch-and-diff path.
    let (archive_deleted_at, data_changed_at) = timeout(Duration::from_secs(10), async {
        let mut archive_deleted_at = None;
        let mut data_changed_at = None;
        let mut position = 0usize;
        loop {
            let msg = loop {
                match ws_rx.recv().await {
                    Ok(m) => break m,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("ui broadcast channel closed unexpectedly: {e}"),
                }
            };
            match msg {
                shared::protocol::ServerToUi::ArchiveDeleted {
                    repo_id: rid,
                    archive_name,
                } if rid == repo_id => {
                    assert_eq!(archive_name, "delete-me");
                    archive_deleted_at = Some(position);
                }
                shared::protocol::ServerToUi::DataChanged if archive_deleted_at.is_some() => {
                    data_changed_at = Some(position);
                }
                _ => {}
            }
            if let (Some(a), Some(d)) = (archive_deleted_at, data_changed_at) {
                return (a, d);
            }
            position += 1;
        }
    })
    .await
    .expect("should observe both ArchiveDeleted and a subsequent DataChanged");

    assert!(
        archive_deleted_at < data_changed_at,
        "ArchiveDeleted should broadcast before DataChanged"
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
    let (borg_dir, _borg_guard) =
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

    // Each queued delete runs its own compact once it completes.
    let count = wait_for_calls_log_count(&borg_dir, "compact", names.len()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let settled = wait_for_calls_log_count(&borg_dir, "compact", count).await;
    assert_eq!(
        settled,
        names.len(),
        "each successful delete in the batch should trigger its own compact"
    );
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
    // Only directories get an archive_paths row; file names live inside the blob.
    let root_path_id: i64 = sqlx::query_scalar(
        "INSERT INTO archive_paths (repo_id, path) VALUES ($1, $2) RETURNING id",
    )
    .bind(repo_id)
    .bind("")
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archive_dirs (archive_id, dir_path_id, chunk_no, entries) VALUES ($1, $2, 0, \
         $3)",
    )
    .bind(stale_archive_id)
    .bind(root_path_id)
    .bind(codec::encode(&[DirEntry {
        name: "stale.txt".to_owned(),
        entry_type: "-".to_owned(),
        size: 1,
        mtime: String::new(),
        mode: String::new(),
    }]))
    .execute(pool)
    .await
    .unwrap();
}

/// Total number of indexed entries for an archive, summed across its stored
/// directory blobs.
#[cfg(test)]
async fn count_indexed_entries(pool: &PgPool, repo_id: i64, archive_name: &str) -> i64 {
    let blobs: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT d.entries FROM archive_dirs d JOIN archives a ON a.id = d.archive_id WHERE \
         a.repo_id = $1 AND a.name = $2",
    )
    .bind(repo_id)
    .bind(archive_name)
    .fetch_all(pool)
    .await
    .unwrap();

    blobs
        .iter()
        .map(|blob| i64::try_from(codec::decode(blob).unwrap().len()).unwrap())
        .sum()
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
        "SELECT COUNT(*) FROM archive_dirs d JOIN archives a ON a.id = d.archive_id WHERE \
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

    let file_rows = count_indexed_entries(&pool, repo_id, "sync-archive-1").await;
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

/// Drives the full account-lockout flow through the real `login()` HTTP
/// handler: enough failed attempts to cross `MAX_ACCOUNT_FAILURES`, then
/// verifies a *correct* password is also rejected while the account is
/// locked. Locked/wrong-password both return 401 "invalid credentials" by
/// design (anti-enumeration), so a correct-password attempt is the only way
/// to observe that lockout, rather than just another wrong password, is
/// actually in effect.
///
/// Failed attempts are spread across several source IPs because
/// `MAX_LOGIN_ATTEMPTS` (5 failures per username+IP within
/// `LOGIN_WINDOW_MINUTES`) blocks a 6th attempt from the *same* IP with 429
/// before it ever reaches the account-wide counter -- one IP alone can never
/// reach `MAX_ACCOUNT_FAILURES` (10).
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn test_account_lockout_rejects_correct_password_while_locked(pool: sqlx::PgPool) {
    use std::net::SocketAddr;

    let mut app = build_test_app(pool.clone());

    let username = "lockout-integration-user";
    let correct_password = "correct-horse-battery-staple";
    let hash = tokio::task::spawn_blocking({
        let correct_password = correct_password.to_string();
        move || bcrypt::hash(correct_password, 4)
    })
    .await
    .unwrap()
    .unwrap();
    sqlx::query("INSERT INTO users (username, password_hash) VALUES ($1, $2)")
        .bind(username)
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();

    let login_request = |ip: &str, password: &str| {
        let mut req = Request::builder()
            .uri("/api/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({ "username": username, "password": password }))
                    .unwrap(),
            ))
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo::<SocketAddr>(
                ip.parse().unwrap(),
            ));
        req
    };

    // 10 wrong-password attempts, 5 per source IP so neither IP alone trips
    // the per-(username, IP) limiter before the account-wide threshold.
    for ip in ["10.0.1.1:1", "10.0.1.2:1"] {
        for _ in 0..5 {
            let resp = oneshot(&mut app, login_request(ip, "wrong-password")).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
    }

    let user: (Option<chrono::DateTime<chrono::Utc>>,) =
        sqlx::query_as("SELECT locked_until FROM users WHERE username = $1")
            .bind(username)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        user.0
            .is_some_and(|locked_until| locked_until > chrono::Utc::now()),
        "account should be locked after {} failed attempts",
        10
    );

    // A correct password from an unused (0 prior failures) IP would
    // normally succeed -- confirm it's rejected instead, proving the
    // lockout itself is blocking it and not a coincidental wrong password.
    let resp = oneshot(&mut app, login_request("10.0.1.3:1", correct_password)).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "correct password should still be rejected while the account is locked"
    );
}

/// Drives real HTTP requests through `auth_tracking_middleware` (rather than
/// unit-testing `UserRateLimiter` in isolation) to prove the per-user 60
/// req/min limit on mutating authenticated routes actually engages: an
/// authenticated caller gets 429 after its 60th mutating request within the
/// window, and reads are never throttled.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn test_user_rate_limiter_returns_429_after_60_mutating_requests(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;

    let state = build_test_state(pool);
    let mut app = Router::new()
        .route(
            "/api/excludes",
            get(server::api::excludes::get_excludes).put(server::api::excludes::set_excludes),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            server::rate_limit::auth_tracking_middleware,
        ))
        .with_state(state);

    let put_excludes =
        || json_request("PUT", "/api/excludes", Some(json!({ "raw_text": "*.tmp" })));

    for i in 0..60 {
        let resp = oneshot(&mut app, put_excludes()).await;
        assert_eq!(resp.status(), StatusCode::OK, "request {i} should succeed");
    }

    let resp = oneshot(&mut app, put_excludes()).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "61st mutating request within the window should be rate-limited"
    );

    // Reads are never throttled, even once the mutating-request budget is spent.
    let resp = oneshot(&mut app, get_request("/api/excludes")).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET requests must not count against the mutating-request limiter"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_session_idle_timeout_revokes_inactive_session(pool: sqlx::PgPool) {
    let mut app = build_test_app_with_idle_timeout(pool.clone(), 1).0;

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, must_change_password)
         VALUES ('idle-timeout-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', false)
         RETURNING id",
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
    let idle_since = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::minutes(2))
        .unwrap();
    let session_id = "idle-timeout-session-id-00000000000";
    let hashed_id = hash_token(session_id);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at, last_seen_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&hashed_id)
    .bind(user_id)
    .bind(expires)
    .bind(idle_since)
    .execute(&pool)
    .await
    .unwrap();

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "idle session older than timeout must be rejected"
    );

    // The rejected session should also be deleted from the database.
    let still_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(&hashed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!still_exists, "idle session must be deleted on rejection");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_session_idle_timeout_exempts_remember_me_session(pool: sqlx::PgPool) {
    let mut app = build_test_app_with_idle_timeout(pool.clone(), 1).0;

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, must_change_password)
         VALUES ('remember-me-idle-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', false)
         RETURNING id",
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

    // Idle far beyond the configured 1-minute idle timeout, but still well
    // within the session's own (remember-me) absolute expiry.
    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .unwrap();
    let idle_since = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::hours(9))
        .unwrap();
    let session_id = "remember-me-idle-session-id-000000000";
    let hashed_id = hash_token(session_id);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at, last_seen_at, remember_me) VALUES ($1, \
         $2, $3, $4, true)",
    )
    .bind(&hashed_id)
    .bind(user_id)
    .bind(expires)
    .bind(idle_since)
    .execute(&pool)
    .await
    .unwrap();

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a remember-me session must not be revoked by the idle timeout"
    );

    let still_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(&hashed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        still_exists,
        "remember-me session must survive despite being idle"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_session_idle_timeout_resets_on_activity(pool: sqlx::PgPool) {
    let mut app = build_test_app_with_idle_timeout(pool.clone(), 10).0;

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, must_change_password)
         VALUES ('idle-active-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', false)
         RETURNING id",
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
    let session_id = "idle-active-session-id-000000000000";
    let hashed_id = hash_token(session_id);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&hashed_id)
        .bind(user_id)
        .bind(expires)
        .execute(&pool)
        .await
        .unwrap();

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "active session must not be rejected"
    );
}

// Regression test: `me()`'s `can_upgrade_agent` field must fold in the
// codebase-wide admin-bypass (can_delete_repo - see
// docs/access-control.md), matching require_upgrade_agent's own bypass
// (deploy.rs). Without this, a custom "admin-like" role with
// can_delete_repo=true but can_upgrade_agent left false is correctly
// authorized by the backend but the frontend never renders the
// Deploy/Upgrade button, since it gates purely on this field.
#[sqlx::test(migrations = "./migrations")]
async fn test_me_can_upgrade_agent_reflects_admin_bypass(pool: sqlx::PgPool) {
    let mut app = build_test_app(pool.clone());

    let user = server::db::insert_user(&pool, "admin-like-me", "hash")
        .await
        .expect("insert user");
    let role = server::db::insert_role(
        &pool,
        &server::db::InsertRoleParams {
            name: "admin-like-me-role",
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: false,
            can_delete_repo: true,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        },
    )
    .await
    .expect("insert role");
    server::db::set_user_roles(&pool, user.id, &[role.id])
        .await
        .expect("assign role");

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let session_id = "admin-bypass-me-session-0000000000";
    let hashed_id = hash_token(session_id);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&hashed_id)
        .bind(user.id)
        .bind(expires)
        .execute(&pool)
        .await
        .unwrap();

    let req = Request::builder()
        .uri("/api/auth/me")
        .method("GET")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(
        body.get("can_upgrade_agent").unwrap(),
        true,
        "can_delete_repo should bypass into can_upgrade_agent, matching require_upgrade_agent"
    );
}

/// Regression test: for a TOTP-enabled account, entering the correct
/// password alone must NOT clear the account's password-lockout escalation
/// state or record a successful `login_attempts` row -- login isn't
/// actually complete until the TOTP step also succeeds. Only
/// `totp_verify_login` should record the real success.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn test_totp_login_defers_lockout_clear_until_totp_step_succeeds(pool: sqlx::PgPool) {
    use std::net::SocketAddr;

    use totp_rs::{Algorithm, Secret, TOTP};

    let (mut app, state) = build_test_app_with_state(pool.clone());

    let username = "totp-defer-lockout-user";
    let password = "correct-horse-battery-staple";
    let password_hash = server::api::helpers::hash_password(password.to_string())
        .await
        .unwrap();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(username)
    .bind(&password_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x55u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    // Simulate an account that has previously been through a lockout cycle
    // (escalation level 2) but whose lockout has since expired -- a
    // realistic pre-condition, since a *currently* locked account would be
    // rejected by login() before ever reaching the TOTP branch.
    sqlx::query(
        "UPDATE users SET lockout_escalation_level = 2, locked_until = NOW() - INTERVAL '1 \
         minute' WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    // Step 1: correct password. Should succeed with totp_required, but must
    // not touch the lockout state or record a success yet.
    let body = serde_json::json!({ "username": username, "password": password });
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
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "password step should succeed"
    );
    let json = body_json(resp).await;
    assert_eq!(json.get("totp_required").unwrap(), true);
    let temp_token = json
        .get("temp_token")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    let escalation_after_password: i32 =
        sqlx::query_scalar("SELECT lockout_escalation_level FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        escalation_after_password, 2,
        "the password step alone must not reset the lockout escalation level"
    );

    let success_count_after_password: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_attempts WHERE username = $1 AND success = true",
    )
    .bind(username)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        success_count_after_password, 0,
        "the password step alone must not record a successful login_attempts row"
    );

    // Step 2: complete login with the correct TOTP code.
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret).to_bytes().unwrap(),
        None,
        String::new(),
    )
    .unwrap();
    let code = totp.generate_current().unwrap();
    let body = serde_json::json!({ "code": code, "temp_token": temp_token });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK, "TOTP step should succeed");

    let escalation_after_totp: i32 =
        sqlx::query_scalar("SELECT lockout_escalation_level FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        escalation_after_totp, 0,
        "completing the TOTP step must reset the lockout escalation level"
    );

    let success_count_after_totp: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_attempts WHERE username = $1 AND success = true",
    )
    .bind(username)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        success_count_after_totp, 1,
        "completing the TOTP step must record exactly one successful login_attempts row"
    );
}

/// Same regression as `test_totp_login_defers_lockout_clear_until_totp_step_succeeds`,
/// but for the recovery-code completion path (`totp_recovery`) instead of
/// the TOTP-code one (`totp_verify_login`). Both are ways login can
/// actually finish for a TOTP-enabled account, and both must reset the
/// password-lockout state on success -- `totp_recovery` was initially
/// missed when `db::record_successful_login` was introduced.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn test_totp_recovery_resets_lockout_state_on_success(pool: sqlx::PgPool) {
    use std::net::SocketAddr;

    let (mut app, state) = build_test_app_with_state(pool.clone());

    let username = "totp-recovery-lockout-user";
    let password = "correct-horse-battery-staple";
    let password_hash = server::api::helpers::hash_password(password.to_string())
        .await
        .unwrap();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(username)
    .bind(&password_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x66u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    let recovery_code = "abcd-1234-ef56-7890";
    let normalized_code = recovery_code.replace('-', "").to_lowercase();
    let recovery_code_hash = tokio::task::spawn_blocking(move || {
        bcrypt::hash(normalized_code, bcrypt::DEFAULT_COST).unwrap()
    })
    .await
    .unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[recovery_code_hash])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    // Simulate an account that previously went through a lockout cycle
    // (escalation level 2), currently expired but not yet reset.
    sqlx::query(
        "UPDATE users SET lockout_escalation_level = 2, locked_until = NOW() - INTERVAL '1 \
         minute' WHERE id = $1",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    // Password step.
    let body = serde_json::json!({ "username": username, "password": password });
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
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "password step should succeed"
    );
    let json = body_json(resp).await;
    let temp_token = json
        .get("temp_token")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Recovery-code step.
    let body = serde_json::json!({ "code": recovery_code, "temp_token": temp_token });
    let req = json_post_request_with_connect_info("/api/auth/totp/recovery", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "recovery-code step should succeed"
    );

    let escalation_after_recovery: i32 =
        sqlx::query_scalar("SELECT lockout_escalation_level FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        escalation_after_recovery, 0,
        "a successful recovery-code login must reset the lockout escalation level"
    );

    let success_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM login_attempts WHERE username = $1 AND success = true",
    )
    .bind(username)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        success_count, 1,
        "a successful recovery-code login must record a successful login_attempts row"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_totp_login_rejects_replayed_code_within_window(pool: sqlx::PgPool) {
    use totp_rs::{Algorithm, Secret, TOTP};

    let (mut app, state) = build_test_app_with_state(pool.clone());

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('totp-replay-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x42u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret).to_bytes().unwrap(),
        None,
        String::new(),
    )
    .unwrap();
    let code = totp.generate_current().unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap();

    // First login: a fresh pending_totp session verified with a fresh code succeeds.
    let temp_token_a = "totp-replay-temp-token-aaaaaaaaaaaaaaaaaaaa";
    server::db::insert_session(
        &pool,
        &hash_token(temp_token_a),
        user_id,
        expires,
        false,
        true,
    )
    .await
    .unwrap();

    let body = serde_json::json!({ "code": code, "temp_token": temp_token_a });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "first login with a fresh code should succeed"
    );

    // Second login attempt: a brand new pending session (e.g. re-entering the
    // password), but replaying the SAME code that was just consumed. Even
    // though the code is still cryptographically within its valid TOTP
    // period, the replay-protection window must reject it.
    let temp_token_b = "totp-replay-temp-token-bbbbbbbbbbbbbbbbbbbb";
    server::db::insert_session(
        &pool,
        &hash_token(temp_token_b),
        user_id,
        expires,
        false,
        true,
    )
    .await
    .unwrap();

    let body = serde_json::json!({ "code": code, "temp_token": temp_token_b });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "replaying the same TOTP code within the dedup window must be rejected"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_totp_login_accepts_different_code_from_next_step(pool: sqlx::PgPool) {
    use totp_rs::{Algorithm, Secret, TOTP};

    let (mut app, state) = build_test_app_with_state(pool.clone());

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('totp-multidevice-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x77u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret).to_bytes().unwrap(),
        None,
        String::new(),
    )
    .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let current_step = now.checked_div(totp.step).unwrap();
    let code_a = totp.generate(current_step.checked_mul(totp.step).unwrap());
    // A different, still-valid code from the very next time-step - within
    // the same ~90s wall-clock window as `code_a`, but not the same code.
    let next_step = current_step.checked_add(1).unwrap();
    let code_b = totp.generate(next_step.checked_mul(totp.step).unwrap());
    assert_ne!(
        code_a, code_b,
        "test setup requires two distinct codes across adjacent steps"
    );

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap();

    // First device logs in with code_a.
    let temp_token_a = "totp-multidevice-temp-token-aaaaaaaaaaaaaaa";
    server::db::insert_session(
        &pool,
        &hash_token(temp_token_a),
        user_id,
        expires,
        false,
        true,
    )
    .await
    .unwrap();

    let body = serde_json::json!({ "code": code_a, "temp_token": temp_token_a });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "first device's login with a fresh code should succeed"
    );

    // Second device logs in shortly after, with a different, currently-valid
    // code from the next time-step. This must succeed - it is not a replay
    // of code_a, even though it falls within the same wall-clock window a
    // naive timestamp-only dedup window would have blocked.
    let temp_token_b = "totp-multidevice-temp-token-bbbbbbbbbbbbbbb";
    server::db::insert_session(
        &pool,
        &hash_token(temp_token_b),
        user_id,
        expires,
        false,
        true,
    )
    .await
    .unwrap();

    let body = serde_json::json!({ "code": code_b, "temp_token": temp_token_b });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a different, currently-valid code from a later time-step must not be rejected as a replay"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_totp_login_locks_account_after_repeated_failed_codes(pool: sqlx::PgPool) {
    use totp_rs::{Algorithm, Secret, TOTP};

    let (mut app, state) = build_test_app_with_state(pool.clone());

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('totp-lockout-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x11u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Raw(secret).to_bytes().unwrap(),
        None,
        String::new(),
    )
    .unwrap();
    let valid_code = totp.generate_current().unwrap();
    // Guaranteed-wrong code: differs from the valid one in the first digit
    // (wrapping so it's never identical even in the 9xxxxx case).
    let wrong_code: String = valid_code
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i == 0 {
                let d = c.to_digit(10).unwrap();
                char::from_digit(d.wrapping_add(1) % 10, 10).unwrap()
            } else {
                c
            }
        })
        .collect();
    assert_ne!(wrong_code, valid_code);

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap();
    let temp_token = "totp-lockout-temp-token-aaaaaaaaaaaaaaaaaaa";
    server::db::insert_session(
        &pool,
        &hash_token(temp_token),
        user_id,
        expires,
        false,
        true,
    )
    .await
    .unwrap();

    // A failed attempt doesn't consume the temp session, so the same
    // pending login can be retried with the wrong code repeatedly - exactly
    // the scenario the account-level lockout must catch.
    for attempt in 0..5 {
        let body = serde_json::json!({ "code": wrong_code, "temp_token": temp_token });
        let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
        let resp = oneshot(&mut app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} with a wrong code should be rejected as invalid"
        );
    }

    // The account is now locked out - even a correct, currently-valid code
    // must be rejected, since an attacker who already has the password
    // could otherwise keep guessing indefinitely (this is the account-level
    // backstop on top of the coarser per-IP rate limiter).
    let body = serde_json::json!({ "code": valid_code, "temp_token": temp_token });
    let req = json_post_request_with_connect_info("/api/auth/totp/verify-login", &body);
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the correct code must still be rejected once the account is locked out"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_totp_disable_locks_account_after_repeated_wrong_passwords(pool: sqlx::PgPool) {
    let (mut app, state) = build_test_app_with_state(pool.clone());

    let password_hash = server::api::helpers::hash_password("correct-horse-battery".to_string())
        .await
        .unwrap();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('totp-disable-lockout-user', $1) \
         RETURNING id",
    )
    .bind(&password_hash)
    .fetch_one(&pool)
    .await
    .unwrap();

    let secret = vec![0x33u8; 20];
    let encrypted =
        shared::crypto::encrypt_passphrase(&hex::encode(&secret), &state.encryption_key).unwrap();
    server::db::set_user_totp_secret(&pool, user_id, &encrypted, &[])
        .await
        .unwrap();
    server::db::enable_user_totp(&pool, user_id, 0)
        .await
        .unwrap();

    let session_id = "totp-disable-lockout-session-0000000";
    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    server::db::insert_session(
        &pool,
        &hash_token(session_id),
        user_id,
        expires,
        false,
        false,
    )
    .await
    .unwrap();

    // Hijacking an authenticated session must not grant unlimited password
    // guesses against this endpoint, the same way login/TOTP-verify are
    // account-locked after repeated failures.
    for attempt in 0..5 {
        let body = serde_json::json!({ "password": "wrong-password" });
        let mut req = Request::builder()
            .uri("/api/auth/totp/disable")
            .method("POST")
            .header("content-type", "application/json")
            .header("cookie", format!("session={session_id}"))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
                "127.0.0.1:54321".parse().unwrap(),
            ));
        let resp = oneshot(&mut app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "attempt {attempt} with a wrong password should be rejected"
        );
    }

    let body = serde_json::json!({ "password": "correct-horse-battery" });
    let mut req = Request::builder()
        .uri("/api/auth/totp/disable")
        .method("POST")
        .header("content-type", "application/json")
        .header("cookie", format!("session={session_id}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo::<std::net::SocketAddr>(
            "127.0.0.1:54321".parse().unwrap(),
        ));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the correct password must still be rejected once the account is locked out"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_revoke_session_rejects_own_current_session(pool: sqlx::PgPool) {
    let mut app = build_test_app(pool.clone());

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('revoke-self-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let session_id = "revoke-self-session-id-0000000000000";
    let hashed_id = hash_token(session_id);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&hashed_id)
        .bind(user_id)
        .bind(expires)
        .execute(&pool)
        .await
        .unwrap();

    let req = Request::builder()
        .uri(format!("/api/auth/sessions/{hashed_id}"))
        .method("DELETE")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "revoking one's own current session must be rejected"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_revoke_session_returns_404_for_unknown_session(pool: sqlx::PgPool) {
    let mut app = build_test_app(pool.clone());

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('revoke-unknown-user', \
         '$2b$12$xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap();
    let session_id = "revoke-unknown-caller-session-id-000";
    let hashed_id = hash_token(session_id);
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&hashed_id)
        .bind(user_id)
        .bind(expires)
        .execute(&pool)
        .await
        .unwrap();

    let req = Request::builder()
        .uri("/api/auth/sessions/this-session-id-does-not-exist")
        .method("DELETE")
        .header("cookie", format!("session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = oneshot(&mut app, req).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "revoking a nonexistent session must return 404"
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
         false, 7, 4, 6, 0, true, '[]', '[]', 'sequential', 'stop') RETURNING id",
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

/// Retargeting an auto-disabled schedule away from the agent that caused the disable,
/// through the actual `PUT /api/schedules/{id}` handler (not just the DB function it
/// delegates to), must clear the stale auto-disable bookkeeping so the dropped agent
/// can never incorrectly re-enable this schedule again on a later reconnect.
#[sqlx::test(migrations = "./migrations")]
async fn test_schedule_update_clears_auto_disable_bookkeeping_for_dropped_agent(
    pool: sqlx::PgPool,
) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let old_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('retarget-old-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let new_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('retarget-new-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "retarget-repo").await;
    let schedule_id = insert_test_schedule(&pool, old_agent_id, repo_id).await;

    // Simulate the scheduler having already auto-disabled this schedule after
    // repeated failures against the old (now permanently broken) agent.
    sqlx::query(
        "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = true, \
         auto_disabled_by_agent_id = $2, consecutive_failures = 3, \
         failure_streak_pure_connectivity = true WHERE id = $1",
    )
    .bind(schedule_id)
    .bind(old_agent_id)
    .execute(&pool)
    .await
    .unwrap();

    // The realistic remediation: retarget to a working agent, leaving `enabled`
    // unchanged (false) in the same request.
    let req = json_request(
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(json!({
            "cron_expression": "0 3 * * *",
            "enabled": false,
            "agent_ids": [new_agent_id],
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let row: (bool, bool, Option<i64>, i32) = sqlx::query_as(
        "SELECT enabled, auto_disabled_agent_unreachable, auto_disabled_by_agent_id, \
         consecutive_failures FROM schedules WHERE id = $1",
    )
    .bind(schedule_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!row.0, "retargeting alone must not re-enable the schedule");
    assert!(
        !row.1,
        "the stale auto-disable flag must be cleared once the causing agent is dropped"
    );
    assert_eq!(row.2, None);
    assert_eq!(row.3, 0);

    // The dropped agent is no longer a target, so its reconnect must never touch
    // this schedule again.
    let reenabled = server::db::reenable_system_disabled_schedules_for_agent(
        &pool,
        old_agent_id,
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(reenabled, Vec::<i64>::new());
}

/// For a multi-target schedule, retargeting through the actual `PUT
/// /api/schedules/{id}` handler must not forgive a still-targeted, still-broken
/// agent just because an unrelated sibling target was dropped in the same edit -
/// scheduler.rs's DB-level tests only covered this via
/// `db::reset_schedule_failure_tracking_if_target_dropped` directly; this exercises
/// the same scenario through the production entry point.
#[sqlx::test(migrations = "./migrations")]
async fn test_schedule_update_does_not_reset_bookkeeping_when_only_a_sibling_target_is_dropped(
    pool: sqlx::PgPool,
) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let causing_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('multi-http-causing', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let sibling_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('multi-http-sibling', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "multi-http-repo").await;
    let schedule_id = insert_test_schedule(&pool, causing_agent_id, repo_id).await;
    sqlx::query(
        "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, $2, 1)",
    )
    .bind(schedule_id)
    .bind(sibling_agent_id)
    .execute(&pool)
    .await
    .unwrap();

    // Simulate the scheduler having already auto-disabled this schedule because of
    // causing_agent_id specifically, with sibling_agent_id an unrelated, healthy target.
    sqlx::query(
        "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = true, \
         auto_disabled_by_agent_id = $2, consecutive_failures = 3, \
         failure_streak_pure_connectivity = true WHERE id = $1",
    )
    .bind(schedule_id)
    .bind(causing_agent_id)
    .execute(&pool)
    .await
    .unwrap();

    // Drop only the unrelated sibling; the causing agent stays a target.
    let req = json_request(
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(json!({
            "cron_expression": "0 3 * * *",
            "enabled": false,
            "agent_ids": [causing_agent_id],
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let row: (bool, bool, Option<i64>, i32) = sqlx::query_as(
        "SELECT enabled, auto_disabled_agent_unreachable, auto_disabled_by_agent_id, \
         consecutive_failures FROM schedules WHERE id = $1",
    )
    .bind(schedule_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!row.0);
    assert!(
        row.1,
        "dropping an unrelated sibling must not clear bookkeeping the causing agent is still \
         responsible for"
    );
    assert_eq!(row.2, Some(causing_agent_id));
    assert_eq!(row.3, 3);
}

/// The inverse of the above, through the same production entry point: dropping the
/// agent actually recorded as the cause must clear the bookkeeping, even though an
/// unrelated sibling target stays on the schedule.
#[sqlx::test(migrations = "./migrations")]
async fn test_schedule_update_clears_bookkeeping_when_causing_agent_dropped_multi_target(
    pool: sqlx::PgPool,
) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let causing_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('multi-http-causing-2', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let sibling_agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('multi-http-sibling-2', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "multi-http-repo-2").await;
    let schedule_id = insert_test_schedule(&pool, causing_agent_id, repo_id).await;
    sqlx::query(
        "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, $2, 1)",
    )
    .bind(schedule_id)
    .bind(sibling_agent_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = true, \
         auto_disabled_by_agent_id = $2, consecutive_failures = 3, \
         failure_streak_pure_connectivity = true WHERE id = $1",
    )
    .bind(schedule_id)
    .bind(causing_agent_id)
    .execute(&pool)
    .await
    .unwrap();

    // Drop the causing agent; the unrelated sibling stays a target.
    let req = json_request(
        "PUT",
        &format!("/api/schedules/{schedule_id}"),
        Some(json!({
            "cron_expression": "0 3 * * *",
            "enabled": false,
            "agent_ids": [sibling_agent_id],
        })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let row: (bool, bool, Option<i64>, i32) = sqlx::query_as(
        "SELECT enabled, auto_disabled_agent_unreachable, auto_disabled_by_agent_id, \
         consecutive_failures FROM schedules WHERE id = $1",
    )
    .bind(schedule_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!row.0, "retargeting alone must not re-enable the schedule");
    assert!(
        !row.1,
        "dropping the causing agent must clear the stale bookkeeping"
    );
    assert_eq!(row.2, None);
    assert_eq!(row.3, 0);
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

/// Regression test for `project_upcoming_schedule_events`'s batch-hostname rewrite: the
/// projected "Scheduled" calendar event for an enabled schedule must still carry its target
/// agent's hostname, now that the lookup is a single batched query instead of one query per
/// schedule inside the loop.
#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_upcoming_schedule_includes_target_hostname(pool: sqlx::PgPool) {
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('calendar-host', 'hash-cal') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo_id = insert_test_repo(&pool, "calendar-repo").await;
    insert_test_schedule(&pool, agent_id, repo_id).await;

    let now = chrono::Utc::now();
    let month = format!("{}-{:02}", now.format("%Y"), now.format("%m"));
    let req = get_request(&format!("/api/stats/calendar?month={month}"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_json(resp).await;
    let days = body.as_array().unwrap();
    let scheduled_event = days
        .iter()
        .flat_map(|d| d.get("events").and_then(|e| e.as_array()).unwrap())
        .find(|e| e.get("status").and_then(|s| s.as_str()) == Some("scheduled"));
    let event = scheduled_event.expect("an upcoming scheduled event for the new schedule");
    assert_eq!(
        event.get("hostname").and_then(|h| h.as_str()),
        Some("calendar-host")
    );
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
                "default_pre_backup_commands": [],
                "default_post_backup_commands": [],
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
                "default_pre_backup_commands": [],
                "default_post_backup_commands": [],
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
    assert_ne!(warnings.len(), 0);
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
async fn repo_list_hides_quota_config_from_viewer_but_shows_it_to_admin(pool: sqlx::PgPool) {
    let repo_id = insert_test_repo(&pool, "quota-visibility-repo").await;
    server::db::quota::upsert_quota(
        &pool,
        repo_id,
        Some(500),
        Some(1_000),
        shared::types::QuotaAction::NotifyOnly,
        shared::types::QuotaAction::BlockBackups,
        true,
    )
    .await
    .unwrap();

    create_non_admin_user_and_session(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool);

    let viewer_resp = oneshot(&mut app, non_admin_get_request("/api/repos/stats")).await;
    let viewer_status = viewer_resp.status();
    let viewer_repos = body_json(viewer_resp).await;
    assert_eq!(viewer_status, StatusCode::OK, "body: {viewer_repos:?}");
    let viewer_repo = viewer_repos
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r.get("name").is_some_and(|n| n == "quota-visibility-repo"))
        .expect("viewer should still see the repo itself");
    assert!(
        viewer_repo.get("quota").is_some_and(Value::is_null),
        "a viewer must not see quota configuration, which is otherwise gated to operators/admins"
    );

    let admin_resp = oneshot(&mut app, get_request("/api/repos/stats")).await;
    assert_eq!(admin_resp.status(), StatusCode::OK);
    let admin_repos = body_json(admin_resp).await;
    let admin_repo = admin_repos
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r.get("name").is_some_and(|n| n == "quota-visibility-repo"))
        .expect("admin should see the repo");
    let admin_quota = admin_repo.get("quota").unwrap();
    assert_eq!(admin_quota.get("warn_bytes").unwrap(), 500);
    assert_eq!(admin_quota.get("critical_bytes").unwrap(), 1_000);
    assert_eq!(admin_quota.get("critical_action").unwrap(), "block_backups");
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

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_run_schedule_now_restricted_to_agent_ids() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "run-now-repo").await;
    let agent_a: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('run-now-a', 'hash-a') RETURNING \
         id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let agent_b: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('run-now-b', 'hash-b') RETURNING \
         id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let schedule_id = insert_test_schedule(&pool, agent_a, repo_id).await;
    sqlx::query(
        "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, $2, 1)",
    )
    .bind(schedule_id)
    .bind(agent_b)
    .execute(&pool)
    .await
    .unwrap();

    // Restricting to agent_a only should insert a pending report for agent_a, not agent_b.
    let req = json_request(
        "POST",
        &format!("/api/schedules/{schedule_id}/run"),
        Some(json!({ "agent_ids": [agent_a] })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let pending_agents: Vec<i64> = sqlx::query_scalar(
        "SELECT agent_id FROM backup_reports WHERE schedule_id = $1 AND status = 'pending'",
    )
    .bind(schedule_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(pending_agents, vec![agent_a]);

    // An agent_id that isn't a target of this schedule is rejected.
    let req = json_request(
        "POST",
        &format!("/api/schedules/{schedule_id}/run"),
        Some(json!({ "agent_ids": [999_999_999] })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Omitting agent_ids runs every target.
    sqlx::query("DELETE FROM backup_reports WHERE schedule_id = $1")
        .bind(schedule_id)
        .execute(&pool)
        .await
        .unwrap();
    let req = json_request(
        "POST",
        &format!("/api/schedules/{schedule_id}/run"),
        Some(json!({})),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let mut pending_agents: Vec<i64> = sqlx::query_scalar(
        "SELECT agent_id FROM backup_reports WHERE schedule_id = $1 AND status = 'pending'",
    )
    .bind(schedule_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    pending_agents.sort_unstable();
    let mut expected = vec![agent_a, agent_b];
    expected.sort_unstable();
    assert_eq!(pending_agents, expected);
}

/// Regression test for: `run_schedule_now` used to require a JSON body
/// unconditionally, which would reject any pre-existing caller (a saved
/// script, an external integration) that sends no body and no `Content-Type`
/// at all - the contract before `agent_ids` filtering was added. The body is
/// now optional, and an absent one is treated the same as `{}`.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_run_schedule_now_without_a_body_runs_every_target() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "run-now-no-body-repo").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('run-now-no-body', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let schedule_id = insert_test_schedule(&pool, agent_id, repo_id).await;

    let req = post_request_without_body(&format!("/api/schedules/{schedule_id}/run"));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let pending_agents: Vec<i64> = sqlx::query_scalar(
        "SELECT agent_id FROM backup_reports WHERE schedule_id = $1 AND status = 'pending'",
    )
    .bind(schedule_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(pending_agents, vec![agent_id]);
}

/// Regression test for: duplicate `agent_ids` entries used to be compared
/// against the (deduplicated) filtered-targets count, so a request like
/// `{"agent_ids": [a, a]}` for a genuinely valid target `a` was wrongly
/// rejected as containing an id that isn't a target of this schedule.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_run_schedule_now_allows_duplicate_agent_ids() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "run-now-dup-repo").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('run-now-dup', 'hash') RETURNING \
         id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let schedule_id = insert_test_schedule(&pool, agent_id, repo_id).await;

    let req = json_request(
        "POST",
        &format!("/api/schedules/{schedule_id}/run"),
        Some(json!({ "agent_ids": [agent_id, agent_id] })),
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

/// Regression test for: `cancel_running_backup`'s offline-agent fallback (the
/// branch that cancels the backup report directly in the DB when
/// `registry.send_to` fails because no agent is connected) had no dedicated
/// test. The test registry is always empty in this suite, so `send_to`
/// deterministically fails for every hostname, which is exactly the
/// condition this branch exists for - asserts the in-progress backup report
/// is marked cancelled rather than merely checking the HTTP status.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_cancel_running_backup_marks_report_cancelled_when_agent_offline() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "cancel-offline-repo").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('cancel-offline-host', 'hash') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let schedule_id = insert_test_schedule(&pool, agent_id, repo_id).await;

    let backup_report_id: i64 = sqlx::query_scalar(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status) VALUES \
         ($1, $2, NOW(), NOW(), 'started') RETURNING id",
    )
    .bind(agent_id)
    .bind(repo_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let req = json_request(
        "POST",
        &format!("/api/schedules/{schedule_id}/cancel"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let status: String = sqlx::query_scalar("SELECT status FROM backup_reports WHERE id = $1")
        .bind(backup_report_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "cancelled",
        "offline-agent fallback must cancel the in-progress backup report"
    );
}

/// Regression test for: the agent-overview "cancel backup in progress" button
/// has no schedule to key off (a manually-triggered run may have
/// `schedule_id = NULL`), so it must be able to cancel by hostname + repo id
/// directly. Mirrors the schedule-scoped offline-agent fallback test above,
/// but through `cancel_agent_backup`.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_cancel_agent_backup_marks_report_cancelled_when_agent_offline() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "cancel-agent-offline-repo").await;
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ('cancel-agent-offline-host', \
         'hash') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let backup_report_id: i64 = sqlx::query_scalar(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status) VALUES \
         ($1, $2, NOW(), NOW(), 'started') RETURNING id",
    )
    .bind(agent_id)
    .bind(repo_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let req = json_request(
        "POST",
        &format!("/api/agents/cancel-agent-offline-host/repos/{repo_id}/cancel-backup"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let status: String = sqlx::query_scalar("SELECT status FROM backup_reports WHERE id = $1")
        .bind(backup_report_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status, "cancelled",
        "offline-agent fallback must cancel the in-progress backup report"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_cancel_agent_backup_unknown_hostname_returns_not_found() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "cancel-agent-404-repo").await;

    let req = json_request(
        "POST",
        &format!("/api/agents/no-such-host/repos/{repo_id}/cancel-backup"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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

/// Seeds a repo with one already-synced archive: an agent, a successful
/// backup report, an `archives` row, and matching `repo_stats` totals.
#[cfg(test)]
async fn seed_synced_archive(pool: &PgPool, repo_id: i64, agent_host: &str, archive_name: &str) {
    let agent_id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (hostname, agent_token_hash) VALUES ($1, 'hash') RETURNING id",
    )
    .bind(agent_host)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, matched, \
         archive_name) VALUES ($1, $2, NOW(), NOW(), 'success', true, $3)",
    )
    .bind(agent_id)
    .bind(repo_id)
    .bind(archive_name)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT (repo_id, name) DO \
         UPDATE SET name = EXCLUDED.name",
    )
    .bind(repo_id)
    .bind(archive_name)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO repo_stats (repo_id, archive_count, deduplicated_size) VALUES ($1, 1, \
         247700000000) ON CONFLICT (repo_id) DO UPDATE SET archive_count = \
         EXCLUDED.archive_count, deduplicated_size = EXCLUDED.deduplicated_size",
    )
    .bind(repo_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Regression test for: a full resync that got back an unexpectedly empty
/// `borg list` result (e.g. a transient SSH hiccup or a relocated/misconfigured
/// repo path) used to prune every known archive and backup report for the
/// repo, zeroing "Archives" and "Last Backup" in the UI while the repo's
/// deduplicated size (read separately from `borg info`'s local cache) stayed
/// unchanged. The sync must now refuse to prune when `borg list` reports zero
/// archives but the DB already knows about some, surfacing an import error
/// instead of silently wiping the archive history.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_sync_refuses_to_prune_all_archives_when_borg_list_returns_empty() {
    let _borg_lock = borg_binary_lock().await;

    let empty_list = r#"{"archives": []}"#;
    let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 100000,
      "total_csize": 50000,
      "unique_csize": 50000,
      "total_chunks": 10,
      "total_unique_chunks": 10
    }
  }
}"#;
    let (_borg_dir, _borg_guard) =
        install_fake_borg(empty_list, info_repo_json, info_repo_json, "", "").await;

    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;
    let mut app = build_test_app(pool.clone());

    let repo_id = insert_test_repo(&pool, "spurious-empty-list-repo").await;
    seed_synced_archive(&pool, repo_id, "spurious-host", "keep-me").await;

    let req = json_request(
        "POST",
        &format!("/api/repos/{repo_id}/sync?build_index=true"),
        None,
    );
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    wait_for_import_completion(&pool, repo_id).await;

    let import_error: Option<String> =
        sqlx::query_scalar("SELECT error FROM repo_import_state WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        import_error.is_some_and(|e| e.contains("refusing to prune")),
        "sync must surface an import error instead of silently succeeding"
    );

    let archive_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archives WHERE repo_id = $1")
        .bind(repo_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        archive_count, 1,
        "existing archive record must not be pruned"
    );

    let backup_report_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        backup_report_count, 1,
        "existing backup report must not be pruned"
    );

    let stats_archive_count: i32 =
        sqlx::query_scalar("SELECT archive_count FROM repo_stats WHERE repo_id = $1")
            .bind(repo_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stats_archive_count, 1,
        "repo_stats.archive_count must not be zeroed out by the aborted sync"
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

/// Regression test for the stale-echo bug: `PUT /api/system/settings` used to
/// build its response from the request body's fields (falling back to
/// request-derived defaults for omitted optional fields) instead of reading
/// back what was actually persisted. A partial update that omits an
/// already-configured field must still report the previously persisted
/// value, not a default.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn test_update_settings_partial_put_reflects_persisted_values_not_request_defaults() {
    let pool = setup_pool().await;
    clean_tables(&pool).await;
    create_test_user_and_session(&pool).await;

    let mut app = build_test_app(pool.clone());

    // First PUT sets every optional field to a non-default value.
    let full_body = json!({
        "retention_days": 7,
        "report_retention_days": 45,
        "failed_report_retention_days": 200,
        "system_event_retention_days": 30,
        "notification_delivery_retention_days": 15,
        "timezone": "UTC",
        "borg_query_timeout_secs": 120,
        "session_idle_timeout_minutes": 60,
    });
    let req = json_request("PUT", "/api/system/settings", Some(full_body));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("report_retention_days").unwrap(), 45);
    assert_eq!(body.get("failed_report_retention_days").unwrap(), 200);
    assert_eq!(body.get("system_event_retention_days").unwrap(), 30);
    assert_eq!(
        body.get("notification_delivery_retention_days").unwrap(),
        15
    );
    assert_eq!(body.get("session_idle_timeout_minutes").unwrap(), 60);
    assert_eq!(body.get("timezone").unwrap(), "UTC");
    assert_eq!(body.get("borg_query_timeout_secs").unwrap(), 120);

    // Second PUT omits all the optional fields except the required
    // retention_days. Their values must still reflect what was persisted
    // above, not request-derived defaults (0/365/90/absent/300/system-tz).
    let partial_body = json!({ "retention_days": 7 });
    let req = json_request("PUT", "/api/system/settings", Some(partial_body));
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(
        body.get("report_retention_days").unwrap(),
        45,
        "omitted field must echo the persisted value, not a request-derived default"
    );
    assert_eq!(
        body.get("failed_report_retention_days").unwrap(),
        200,
        "omitted field must echo the persisted value, not a request-derived default"
    );
    assert_eq!(
        body.get("system_event_retention_days").unwrap(),
        30,
        "omitted field must echo the persisted value, not a request-derived default"
    );
    assert_eq!(
        body.get("notification_delivery_retention_days").unwrap(),
        15,
        "omitted field must echo the persisted value, not a request-derived default"
    );
    assert_eq!(
        body.get("session_idle_timeout_minutes").unwrap(),
        60,
        "omitted field must echo the persisted value, not a request-derived default"
    );
    assert_eq!(
        body.get("timezone").unwrap(),
        "UTC",
        "omitted timezone must not be silently reset to the system default"
    );
    assert_eq!(
        body.get("borg_query_timeout_secs").unwrap(),
        120,
        "omitted borg_query_timeout_secs must not be silently reset to 300"
    );

    // GET must agree with what the PUT response reported.
    let req = get_request("/api/system/settings");
    let resp = oneshot(&mut app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.get("report_retention_days").unwrap(), 45);
    assert_eq!(body.get("failed_report_retention_days").unwrap(), 200);
    assert_eq!(body.get("system_event_retention_days").unwrap(), 30);
    assert_eq!(
        body.get("notification_delivery_retention_days").unwrap(),
        15
    );
    assert_eq!(body.get("session_idle_timeout_minutes").unwrap(), 60);
    assert_eq!(body.get("timezone").unwrap(), "UTC");
    assert_eq!(body.get("borg_query_timeout_secs").unwrap(), 120);
}
