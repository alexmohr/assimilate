// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    middleware as axum_middleware,
    response::Redirect,
    routing::{delete, get, post, put},
};
use server::{
    AppState, api,
    log_buffer::{LogBuffer, LogBufferLayer},
    middleware::csp_headers,
    notifications::NotificationService,
    openapi::ApiDoc,
    rate_limit::{RateLimiter, rate_limit_middleware},
    tunnel::TunnelManager,
    ws,
};
use sqlx::PgPool;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::{EnvFilter, Layer as _, layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi as _;
use utoipa_scalar::{Scalar, Servable as _};

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing env var: {0}")]
    EnvVar(#[from] std::env::VarError),
    #[error("bcrypt error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    let log_buffer = LogBuffer::default();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        ))
        .with(LogBufferLayer::new(log_buffer.clone()))
        .init();

    let database_url = std::env::var("DATABASE_URL")?;
    let secret_key = std::env::var("ASSIMILATE_SECRET_KEY")?;

    let max_connections: u32 = std::env::var("ASSIMILATE_DB_MAX_CONN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let pool = connect_with_retry(&database_url, max_connections).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    bootstrap_admin(&pool).await?;

    let encryption_key = shared::crypto::derive_key(secret_key.as_bytes());

    let bind_addr =
        std::env::var("ASSIMILATE_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let addr: SocketAddr = bind_addr.parse().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid bind address: {e}"),
        )
    })?;

    let server_addr = SocketAddr::from(([127, 0, 0, 1], addr.port()));
    let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
    let tunnel_manager = TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr);

    let notification_service = NotificationService::new(pool.clone(), reqwest::Client::new());
    if let Err(e) = notification_service.ensure_vapid_keys().await {
        tracing::warn!("failed to ensure VAPID keys: {e}");
    }

    let state = AppState {
        pool,
        encryption_key,
        registry: server::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager: tunnel_manager.clone(),
        log_buffer,
        notification_service,
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

    tokio::spawn(server::scheduler::run(
        state.pool.clone(),
        state.registry.clone(),
        state.encryption_key,
        state.ui_broadcast.clone(),
    ));

    let tm = tunnel_manager.clone();
    tokio::spawn(async move { tm.run().await });

    let login_rate_limiter = RateLimiter::new(10, Duration::from_secs(60));

    let login_router = Router::new()
        .route("/api/auth/login", post(api::auth::login))
        .layer(axum_middleware::from_fn_with_state(
            login_rate_limiter,
            rate_limit_middleware,
        ))
        .with_state(state.clone());

    let app = Router::new()
        .merge(login_router)
        .route("/api/health", get(api::health::health))
        .route("/api/auth/logout", post(api::auth::logout))
        .route("/api/auth/me", get(api::auth::me))
        .route(
            "/api/auth/change-password",
            post(api::auth::change_password),
        )
        .route(
            "/api/auth/preferences",
            get(api::auth::get_preferences).put(api::auth::update_preferences),
        )
        .route(
            "/api/users",
            get(api::users::list_users).post(api::users::create_user),
        )
        .route("/api/users/{id}/role", put(api::users::update_role))
        .route("/api/users/{id}/password", put(api::users::update_password))
        .route("/api/users/{id}", delete(api::users::delete_user))
        .route("/ws/agent", get(ws::handler::ws_handler))
        .route("/ws/ui", get(ws::ui_handler::ui_ws_handler))
        .route(
            "/ws/ssh-agent/{hostname}",
            get(ws::ssh_relay::ssh_relay_handler),
        )
        .route(
            "/api/clients",
            get(api::clients::list_clients).post(api::clients::create_client),
        )
        .route(
            "/api/clients/{hostname}",
            get(api::clients::get_client)
                .put(api::clients::update_client)
                .delete(api::clients::delete_client),
        )
        .route(
            "/api/clients/{hostname}/regenerate-token",
            post(api::clients::regenerate_token),
        )
        .route(
            "/api/clients/{hostname}/restart",
            post(api::clients::restart_agent),
        )
        .route(
            "/api/clients/{hostname}/hostname-patterns",
            get(api::clients::list_hostname_patterns).post(api::clients::add_hostname_pattern),
        )
        .route(
            "/api/clients/{hostname}/hostname-patterns/{pattern_id}",
            delete(api::clients::delete_hostname_pattern),
        )
        .route(
            "/api/clients/{hostname}/merge-from/{source_id}",
            post(api::clients::merge_client),
        )
        .route(
            "/api/clients/{hostname}/deploy",
            post(api::deploy::deploy_agent),
        )
        .route(
            "/api/clients/{hostname}/repos",
            get(api::repos::get_client_repos),
        )
        .route(
            "/api/repos",
            get(api::repos::list_repos).post(api::repos::create_repo),
        )
        .route("/api/repos/init", post(api::repos::init_repo))
        .route("/api/repos/stats", get(api::repos::list_repos_with_stats))
        .route(
            "/api/repos/{repo_id}",
            get(api::repos::get_repo)
                .put(api::repos::update_repo)
                .delete(api::repos::delete_repo),
        )
        .route(
            "/api/repos/{repo_id}/key/export",
            post(api::keys::export_key),
        )
        .route(
            "/api/repos/{repo_id}/key/import",
            post(api::keys::import_key),
        )
        .route(
            "/api/repos/{repo_id}/key/change-passphrase",
            post(api::keys::change_passphrase),
        )
        .route(
            "/api/repos/{repo_id}/passphrase",
            get(api::repos::get_passphrase),
        )
        .route(
            "/api/repos/{repo_id}/break-lock",
            post(api::repos::break_lock),
        )
        .route("/api/repos/{repo_id}/rescan", post(api::repos::rescan_repo))
        .route("/api/repos/{repo_id}/sync", post(api::repos::sync_repo))
        .route("/api/repos/{repo_id}/dry-run", post(api::dryrun::dry_run))
        .route(
            "/api/repos/{repo_id}/tags",
            get(api::tags::get_repo_tags).put(api::tags::set_repo_tags),
        )
        .route(
            "/api/excludes",
            get(api::excludes::list_excludes).post(api::excludes::create_exclude),
        )
        .route(
            "/api/excludes/{id}",
            put(api::excludes::update_exclude).delete(api::excludes::delete_exclude),
        )
        .route(
            "/api/schedules",
            get(api::schedules::list_schedules).post(api::schedules::create_schedule),
        )
        .route(
            "/api/schedules/{id}",
            get(api::schedules::get_schedule)
                .put(api::schedules::update_schedule)
                .delete(api::schedules::delete_schedule),
        )
        .route(
            "/api/schedules/{id}/clone",
            post(api::schedules::clone_schedule),
        )
        .route(
            "/api/schedules/{id}/run",
            post(api::schedules::run_schedule_now),
        )
        .route(
            "/api/schedules/{id}/reports",
            get(api::schedules::list_schedule_reports),
        )
        .route(
            "/api/clients/{hostname}/reports",
            get(api::reports::list_reports),
        )
        .route("/api/audit-log", get(api::audit::list_audit_log))
        .route(
            "/api/system/ssh-public-key",
            get(api::system::ssh_public_key),
        )
        .route(
            "/api/system/ssh-regenerate-key",
            post(api::system::ssh_regenerate_key),
        )
        .route(
            "/api/system/settings",
            get(api::system::get_settings).put(api::system::update_settings),
        )
        .route("/api/system/version", get(api::system::get_version))
        .route("/api/ssh/test-connection", post(api::ssh::test_connection))
        .route("/api/ssh/deploy-key", post(api::ssh::deploy_key))
        .route("/api/ssh/list-dir", post(api::ssh::list_dir))
        .route("/api/ssh/mkdir", post(api::ssh::mkdir))
        .route("/api/stats/summary", get(api::stats::summary))
        .route("/api/stats/storage", get(api::stats::storage))
        .route(
            "/api/stats/storage-breakdown",
            get(api::stats::storage_breakdown),
        )
        .route("/api/stats/activity", get(api::stats::activity))
        .route("/api/stats/system-events", get(api::stats::system_events))
        .route("/api/stats/health", get(api::stats::health))
        .route("/api/stats/trends", get(api::stats::trends))
        .route("/api/stats/calendar", get(api::stats::calendar))
        .route("/api/logs", get(api::logs::get_logs))
        .route(
            "/api/repos/{repo_id}/archives/diff",
            get(api::diff::diff_archives),
        )
        .route(
            "/api/repos/{repo_id}/archives",
            get(api::archives::list_archives),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}",
            get(api::archives::archive_info),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/contents",
            get(api::archives::list_contents),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/extract",
            get(api::archives::extract_file),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/export",
            get(api::export::export_archive),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/download",
            post(api::restore::download_files),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/restore",
            post(api::restore::restore_files),
        )
        .route(
            "/api/repos/{repo_id}/search",
            get(api::search::cross_archive_search),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/search",
            get(api::search::search_archive),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/tags",
            get(api::tags::list_archive_tags).post(api::tags::add_archive_tag),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/tags/{tag}",
            delete(api::tags::remove_archive_tag),
        )
        .route(
            "/api/tokens",
            get(api::tokens::list_tokens).post(api::tokens::create_token),
        )
        .route("/api/tokens/{id}", delete(api::tokens::delete_token))
        .route(
            "/api/repos/{repo_id}/permissions",
            get(api::permissions::list_for_repo),
        )
        .route(
            "/api/repos/{repo_id}/permissions/{user_id}",
            put(api::permissions::upsert),
        )
        .route(
            "/api/repos/{id}/quota",
            get(api::quota::get_quota).put(api::quota::upsert_quota),
        )
        .route(
            "/api/users/{id}/permissions",
            get(api::permissions::list_for_user),
        )
        .route(
            "/api/tags",
            get(api::tags::list_tags).post(api::tags::create_tag),
        )
        .route("/api/tags/{id}", delete(api::tags::delete_tag))
        .route(
            "/api/clients/{hostname}/tags",
            get(api::tags::get_host_tags).put(api::tags::set_host_tags),
        )
        .route("/api/host-tags", get(api::tags::list_host_tag_associations))
        .route("/api/repo-tags", get(api::tags::list_repo_tag_associations))
        .route(
            "/api/groups",
            get(api::rbac::list_groups).post(api::rbac::create_group),
        )
        .route(
            "/api/groups/{id}",
            put(api::rbac::update_group).delete(api::rbac::delete_group),
        )
        .route(
            "/api/groups/{id}/members",
            get(api::rbac::list_group_members).put(api::rbac::set_group_members),
        )
        .route(
            "/api/roles",
            get(api::rbac::list_roles).post(api::rbac::create_role),
        )
        .route(
            "/api/roles/{id}",
            put(api::rbac::update_role).delete(api::rbac::delete_role),
        )
        .route(
            "/api/users/{id}/roles",
            get(api::rbac::list_user_roles).put(api::rbac::set_user_roles),
        )
        .route("/api/users/{id}/groups", get(api::rbac::list_user_groups))
        .route(
            "/api/users/{id}/effective-permissions",
            get(api::rbac::get_effective_permissions),
        )
        .route(
            "/api/tunnels",
            get(api::tunnels::list_tunnels).post(api::tunnels::create_tunnel),
        )
        .route(
            "/api/tunnels/{id}",
            get(api::tunnels::get_tunnel)
                .put(api::tunnels::update_tunnel)
                .delete(api::tunnels::delete_tunnel),
        )
        .route(
            "/api/tunnels/{id}/enable",
            post(api::tunnels::enable_tunnel),
        )
        .route(
            "/api/tunnels/{id}/reconnect",
            post(api::tunnels::reconnect_tunnel),
        )
        .route(
            "/api/tunnels/{id}/disable",
            post(api::tunnels::disable_tunnel),
        )
        .route(
            "/api/notifications/channels",
            get(api::notifications::list_channels).post(api::notifications::create_channel),
        )
        .route(
            "/api/notifications/channels/{id}",
            put(api::notifications::update_channel).delete(api::notifications::delete_channel),
        )
        .route(
            "/api/notifications/channels/{id}/test",
            post(api::notifications::test_channel),
        )
        .route(
            "/api/notifications/rules",
            get(api::notifications::list_rules).post(api::notifications::create_rule),
        )
        .route(
            "/api/notifications/rules/{id}",
            delete(api::notifications::delete_rule),
        )
        .route(
            "/api/notifications/push/vapid-key",
            get(api::notifications::get_vapid_key).put(api::notifications::set_vapid_keys),
        )
        .route(
            "/api/notifications/push/subscribe",
            post(api::notifications::subscribe_push),
        )
        .route(
            "/api/notifications/push/unsubscribe",
            post(api::notifications::unsubscribe_push),
        )
        .route(
            "/api/notifications/push/subscriptions",
            get(api::notifications::list_push_subscriptions),
        )
        .route(
            "/api/notifications/deliveries",
            get(api::notifications::list_deliveries),
        )
        .route(
            "/api/notifications/validate-smtp",
            post(api::notifications::validate_smtp),
        )
        .route(
            "/api/openapi.json",
            get(|| async { Json(ApiDoc::openapi()) }),
        )
        .merge(Scalar::with_url("/api/docs", ApiDoc::openapi()))
        .with_state(state)
        .layer(axum_middleware::from_fn(csp_headers))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024));

    let docs_dir = std::env::var("ASSIMILATE_DOCS_DIR")
        .map_or_else(|_| PathBuf::from("./docs_html"), PathBuf::from);
    let app = if docs_dir.exists() {
        app.route("/docs", get(|| async { Redirect::permanent("/docs/") }))
            .nest_service("/docs/", ServeDir::new(&docs_dir))
    } else {
        tracing::warn!(
            "docs directory not found at {:?}, /docs route disabled",
            docs_dir
        );
        app
    };

    let static_dir = std::env::var("ASSIMILATE_STATIC_DIR")
        .map_or_else(|_| PathBuf::from("./static"), PathBuf::from);
    let app = if static_dir.exists() {
        let index = static_dir.join("index.html");
        app.fallback_service(ServeDir::new(&static_dir).fallback(ServeFile::new(index)))
    } else {
        app
    };

    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tunnel_manager.shutdown().await;
    Ok(())
}

async fn connect_with_retry(url: &str, max_connections: u32) -> Result<PgPool, StartupError> {
    let max_retries = 30;
    let retry_interval = Duration::from_secs(2);

    for attempt in 1..=max_retries {
        match sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await
        {
            Ok(pool) => {
                if attempt > 1 {
                    tracing::info!("database connection established after {attempt} attempts");
                }
                return Ok(pool);
            }
            Err(e) if attempt < max_retries => {
                tracing::warn!(
                    "database connection attempt {attempt}/{max_retries} failed: {e}, retrying in \
                     {}s",
                    retry_interval.as_secs()
                );
                tokio::time::sleep(retry_interval).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        result = ctrl_c => {
            if let Err(e) = result {
                tracing::error!("failed to listen for Ctrl+C: {e}");
            }
        }
        () = terminate => {}
    }

    tracing::info!("shutdown signal received, shutting down gracefully");
}

async fn bootstrap_admin(pool: &PgPool) -> Result<(), StartupError> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;

    if count.0 > 0 {
        return Ok(());
    }

    let hash = bcrypt::hash("admin", 10)?;

    sqlx::query(
        "INSERT INTO users (username, password_hash, role, must_change_password) VALUES ('admin', \
         $1, 'admin', true)",
    )
    .bind(&hash)
    .execute(pool)
    .await?;

    tracing::info!(
        "default admin user created (password: admin) -- password change required on first login"
    );
    Ok(())
}
