// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Assimilate server binary.

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
    client_ip::ClientIpResolver,
    db,
    log_buffer::{LogBuffer, LogBufferLayer},
    middleware::security_headers,
    notifications::NotificationService,
    openapi::ApiDoc,
    rate_limit::{
        IpRateLimitMiddlewareState, IpRateLimiter, UserRateLimiter, auth_tracking_middleware,
        ip_rate_limit_middleware,
    },
    tunnel::TunnelManager,
    ws,
};
use shared::protocol::ServerToAgent;
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
    #[error("crypto error: {0}")]
    Crypto(#[from] shared::crypto::CryptoError),
    #[error("failed to encrypt legacy notification channel secrets: {0}")]
    ChannelSecretMigration(#[from] server::notifications::NotificationError),
    #[error("failed to install rustls crypto provider")]
    RustlsProvider,
}

/// How startup keeps trying to reach a database that may still be booting.
#[derive(Debug, Clone, Copy)]
struct DbConnectRetry {
    /// Attempts before giving up; the first attempt always runs.
    max_attempts: u32,
    /// Delay between two attempts.
    retry_interval: Duration,
    /// How long a single attempt may take to acquire a connection.
    acquire_timeout: Duration,
}

const DB_CONNECT_RETRY: DbConnectRetry = DbConnectRetry {
    max_attempts: 30,
    retry_interval: Duration::from_secs(2),
    acquire_timeout: Duration::from_secs(10),
};

/// Extra time beyond borg's own SIGKILL-escalation delay ([`shared::borg::kill_escalation_delay`])
/// that shutdown waits for `AppState::task_registry` (every in-flight `Borg`
/// invocation's `GracefulChild` reaper) to drain, before giving up and letting the
/// process exit anyway.
const SHUTDOWN_GRACE_BUFFER: Duration = Duration::from_secs(10);

/// How long shutdown waits for `AppState::background_task_tracker` to go idle before giving up
/// and draining `task_registry` anyway. It covers every task that claims a tracker guard. That
/// is deliberately stated as a rule rather than a list: the list version of this comment went
/// stale three times, so read it as every `BackgroundTaskTracker::spawn_tracked` call site, plus
/// the three that thread a guard through by value instead (`resume_single_import`'s parameter,
/// and `scheduler.rs`'s `ScheduledRepoSync` and `SequentialExecution` fields). All of them claim
/// the guard synchronously, before the spawn, so `any_active()` reads true the instant the
/// spawning call returns rather than only once the task is first polled. These tasks
/// aren't themselves registered with `task_registry` - only the `GracefulChild` reapers a
/// *cancelled* borg call inside them would spawn are - so without this wait, a task still
/// normally running a borg call when shutdown lands would have that call force-dropped when
/// the runtime tears down, with nothing having ever tried to let it finish first.
const BACKGROUND_TASK_SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

/// How long the HTTP server may keep draining open connections after the
/// shutdown signal before the process stops waiting for it.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// How [`serve_until_shutdown`] ended.
#[derive(Debug, PartialEq, Eq)]
enum ShutdownOutcome {
    /// The server finished draining its connections on its own.
    Graceful,
    /// The server was still draining `timeout` after the shutdown signal.
    TimedOut,
}

/// Runs `server` until it finishes, but gives up on it once `timeout` has
/// passed after `shutdown_rx` fires, so one stuck connection can't hold the
/// process open forever.
async fn serve_until_shutdown<F>(
    server: F,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    timeout: Duration,
) -> std::io::Result<ShutdownOutcome>
where
    F: std::future::IntoFuture<Output = std::io::Result<()>>,
{
    tokio::select! {
        result = server => {
            result?;
            Ok(ShutdownOutcome::Graceful)
        }
        () = async {
            let _ = shutdown_rx.await;
            tokio::time::sleep(timeout).await;
        } => {
            // Tracing skips its arguments when no subscriber listens, so compute
            // the value up front rather than inside the macro.
            let secs = timeout.as_secs();
            tracing::warn!("graceful shutdown timed out after {secs}s, exiting");
            Ok(ShutdownOutcome::TimedOut)
        }
    }
}

/// Moves notification channel secrets older versions stored in plaintext (SMTP passwords,
/// webhook header values) into their encrypted columns. Idempotent; see the two migrations.
async fn encrypt_legacy_channel_secrets(
    pool: &PgPool,
    encryption_key: &[u8; 32],
) -> Result<(), StartupError> {
    let encrypted_smtp_passwords =
        server::notifications::smtp_migration::encrypt_plaintext_smtp_passwords(
            pool,
            encryption_key,
        )
        .await?;
    if encrypted_smtp_passwords > 0 {
        tracing::info!(
            channels = encrypted_smtp_passwords,
            "encrypted legacy plaintext SMTP passwords"
        );
    }
    let encrypted_webhook_headers =
        server::notifications::webhook_header_migration::encrypt_plaintext_webhook_headers(
            pool,
            encryption_key,
        )
        .await?;
    if encrypted_webhook_headers > 0 {
        tracing::info!(
            channels = encrypted_webhook_headers,
            "encrypted legacy plaintext webhook headers"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| StartupError::RustlsProvider)?;

    let log_buffer = init_logging();

    let database_url = std::env::var("DATABASE_URL")?;
    let secret_key = std::env::var("ASSIMILATE_SECRET_KEY")?;

    let max_connections: u32 = std::env::var("ASSIMILATE_DB_MAX_CONN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let pool = connect_with_retry(&database_url, max_connections, DB_CONNECT_RETRY).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    bootstrap_admin(&pool).await?;

    let encryption_key = shared::crypto::derive_key(secret_key.as_bytes())?;
    encrypt_legacy_channel_secrets(&pool, &encryption_key).await?;
    let addr = resolve_bind_addr()?;
    let server_addr = server::tunnel::tunnel_target_addr(addr);
    let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
    let tunnel_manager = TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr);

    let notification_service = NotificationService::new(pool.clone(), encryption_key);
    if let Err(e) = notification_service.ensure_vapid_keys().await {
        tracing::warn!("failed to ensure VAPID keys: {e}");
    }

    let client_ip_resolver =
        ClientIpResolver::from_env(std::env::var("ASSIMILATE_TRUSTED_PROXIES").ok());
    let shutdown_token = tokio_util::sync::CancellationToken::new();

    let state = build_app_state(BuildAppStateArgs {
        pool,
        encryption_key,
        ui_broadcast,
        tunnel_manager: tunnel_manager.clone(),
        log_buffer,
        notification_service,
        client_ip_resolver: client_ip_resolver.clone(),
        shutdown_token: shutdown_token.clone(),
    });

    // Load the cached session idle timeout from the database
    state.reload_session_idle_timeout().await;

    // Before anything can start indexing again: jobs this process never ran would
    // otherwise block every future claim on their archive.
    match server::archive_index::release_interrupted_jobs(&state.pool).await {
        Ok(0) => {}
        Ok(released) => tracing::info!(released, "released interrupted archive index jobs"),
        Err(e) => tracing::warn!("failed to release interrupted archive index jobs: {e}"),
    }

    spawn_background_tasks(&state, &tunnel_manager);

    let login_router = build_login_router(&state, &client_ip_resolver);
    let registry = state.registry.clone();
    let task_registry = state.task_registry.clone();
    let background_task_tracker = state.background_task_tracker.clone();
    let app = build_router(&state, login_router)
        .with_state(state)
        .layer(axum_middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024));
    let app = configure_docs_and_static(app).await;

    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal(registry, shutdown_token.clone()).await;
        let _ = shutdown_tx.send(());
    });

    serve_until_shutdown(server, shutdown_rx, GRACEFUL_SHUTDOWN_TIMEOUT).await?;

    // Give outer background tasks (scheduled sync, post-backup sync/indexing, initial
    // import) a chance to finish - including whatever borg call they're in the middle of
    // running - before task_registry.shutdown() below tries to join reaper tasks that
    // wouldn't exist yet if one of these got force-dropped by the runtime instead.
    if !background_task_tracker
        .wait_until_idle(BACKGROUND_TASK_SHUTDOWN_GRACE)
        .await
    {
        tracing::warn!("background tasks still running at shutdown deadline");
    }

    // Let every `Borg` invocation's GracefulChild reaper (SIGKILL-escalation +
    // break-lock, see shared::borg::kill_escalation_delay) finish before the process
    // exits out from under it, instead of abandoning whatever cleanup it promised.
    let outstanding = task_registry
        .shutdown(shared::borg::kill_escalation_delay().saturating_add(SHUTDOWN_GRACE_BUFFER))
        .await;
    if outstanding > 0 {
        tracing::warn!(
            outstanding,
            "background borg tasks still running at shutdown deadline"
        );
    }

    tunnel_manager.shutdown().await;
    Ok(())
}

fn init_logging() -> LogBuffer {
    let log_buffer = LogBuffer::default();

    let default_filter = "info,sqlx=info,russh=info";
    let noise_clamp = ",sqlx=info,russh=info";

    let env_filter = std::env::var("RUST_LOG").map_or_else(
        |_| EnvFilter::new(default_filter),
        |val| EnvFilter::new(format!("{val}{noise_clamp}")),
    );

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
        .with(LogBufferLayer::new(log_buffer.clone()).with_filter(EnvFilter::new(default_filter)))
        .init();

    log_buffer
}

fn resolve_bind_addr() -> Result<SocketAddr, StartupError> {
    let bind_addr =
        std::env::var("ASSIMILATE_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    bind_addr.parse().map_err(|e| {
        StartupError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid bind address: {e}"),
        ))
    })
}

struct BuildAppStateArgs {
    pool: PgPool,
    encryption_key: [u8; 32],
    ui_broadcast: server::ws::ui_broadcast::UiBroadcast,
    tunnel_manager: TunnelManager,
    log_buffer: LogBuffer,
    notification_service: NotificationService,
    client_ip_resolver: ClientIpResolver,
    shutdown_token: tokio_util::sync::CancellationToken,
}

fn build_app_state(args: BuildAppStateArgs) -> AppState {
    let BuildAppStateArgs {
        pool,
        encryption_key,
        ui_broadcast,
        tunnel_manager,
        log_buffer,
        notification_service,
        client_ip_resolver,
        shutdown_token,
    } = args;
    let task_registry = shared::task_registry::TaskRegistry::default();

    let user_rate_limiter = UserRateLimiter::new(60, Duration::from_mins(1));

    AppState {
        pool,
        encryption_key,
        registry: server::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer,
        notification_service,
        completion_bus: server::ws::completion_bus::CompletionBus::new(),
        repo_op_tracker: server::repo_op_tracker::RepoOpTracker::default(),
        background_task_tracker: server::background_tasks::BackgroundTaskTracker::default(),
        repo_lock: server::RepoLock::default(),
        import_tasks: server::ImportTaskRegistry::default(),
        pending_dryruns: server::new_pending_map(),
        pending_restores: server::new_pending_map(),
        pending_vm_scans: server::new_pending_map(),
        pending_vm_builds: server::new_pending_map(),
        pending_vm_stages: server::new_pending_map(),
        pending_migrations: server::new_pending_map(),
        pending_deletes: server::new_pending_map(),
        shutdown_token,
        client_ip_resolver,
        task_registry,
        user_rate_limiter,
        session_idle_timeout_minutes: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(480)),
        power_sessions: server::power::PowerSessionTracker::default(),
    }
}

/// Runs `work` under `repo_id`'s repository lock unless `cancel` fires first,
/// returning whether it ran to completion.
///
/// The guard is taken here, outside the race against `cancel`, and dropped
/// only once that race has resolved: on a cancellation, `work`'s future has
/// already been torn down by then, so the lock never passes to a queued
/// indexing run or eviction while the cancelled work could still touch the
/// repository. A cancellation that lands while still queued for the lock
/// leaves without running `work` at all.
async fn run_locked_until_cancelled(
    repo_lock: &server::RepoLock,
    repo_id: i64,
    cancel: &tokio_util::sync::CancellationToken,
    work: impl std::future::Future<Output = ()>,
) -> bool {
    let repo_guard = tokio::select! {
        () = cancel.cancelled() => return false,
        guard = repo_lock.acquire(repo_id) => guard,
    };
    let completed = tokio::select! {
        () = cancel.cancelled() => false,
        () = work => true,
    };
    drop(repo_guard);
    completed
}

/// Resumes a single repository import that was interrupted (e.g. by a
/// server restart) while it was still marked as importing.
async fn resume_single_import(
    state: AppState,
    pool: PgPool,
    broadcast: server::ws::ui_broadcast::UiBroadcast,
    key: [u8; 32],
    repo_id: i64,
    _task_guard: server::background_tasks::BackgroundTaskGuard,
) {
    let (task_id, cancel) = state.import_tasks.start(repo_id).await;

    let op_clear_guard = server::api::repos::set_server_sync_op(&state, repo_id).await;
    // Held like the fresh import holds it: the full sync prunes vanished
    // archives' index rows and garbage-collects their directory paths, which
    // must not interleave with an indexing run or the content-index eviction
    // (both also start at boot) on the same repository.
    let sync = async {
        if let Err(e) = server::api::repos::sync_existing_archives(
            &pool,
            &key,
            repo_id,
            &broadcast,
            &state.background_task_tracker,
            &state.task_registry,
        )
        .await
        {
            tracing::warn!(repo_id, error = %e, "failed to resume import");
            if state.import_tasks.is_current(repo_id, task_id).await {
                let _ = db::set_repo_import_error(&pool, repo_id, Some(&format!("{e}"))).await;
            }
        }
        if state.import_tasks.is_current(repo_id, task_id).await {
            let _ = db::set_repo_importing(&pool, repo_id, false).await;
            server::api::repos::clear_import_progress_state(&pool, &broadcast, repo_id).await;
            broadcast.send(shared::protocol::ServerToUi::DataChanged);
        }
    };
    if !run_locked_until_cancelled(&state.repo_lock, repo_id, &cancel, sync).await {
        tracing::info!(repo_id, "resumed import cancelled");
    }

    server::api::repos::finish_server_sync_task(
        &state.import_tasks,
        &broadcast,
        repo_id,
        task_id,
        op_clear_guard,
    )
    .await;
}

/// Finds repositories still marked as importing from before this server
/// process started (e.g. after a crash or restart) and resumes their sync.
async fn resume_interrupted_imports(state: AppState) {
    let pool = state.pool.clone();
    let key = state.encryption_key;
    let broadcast = state.ui_broadcast.clone();

    let repo_ids = match db::list_resumable_import_repo_ids(&pool).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("failed to query importing repos: {e}");
            return;
        }
    };
    for repo_id in repo_ids {
        tracing::info!(repo_id, "resuming interrupted import");
        // Claimed synchronously (before spawning), same as run_repo_sync's scheduled
        // syncs, so background_task_tracker.any_active() - and thus /api/health's
        // background_ops_in_flight - reflects this task immediately instead of only
        // once it gets its first poll.
        let task_guard = state.background_task_tracker.begin();
        tokio::spawn(resume_single_import(
            state.clone(),
            pool.clone(),
            broadcast.clone(),
            key,
            repo_id,
            task_guard,
        ));
    }
}

/// Spawns the server's long-lived, detached background loops and registers each
/// handle with `task_registry` so shutdown can join them (bounded by
/// `task_registry.shutdown`'s timeout) instead of the runtime aborting them
/// mid-work when the process exits. `scheduler::run` reacts to
/// `state.shutdown_token` and returns promptly; `tunnel_manager.run()` and
/// `resume_interrupted_imports` finish on their own shortly after startup
/// regardless, so registering them just gives shutdown visibility into stragglers.
fn spawn_background_tasks(state: &AppState, tunnel_manager: &TunnelManager) {
    let scheduler_handle = tokio::spawn(server::scheduler::run(state.clone()));
    state.task_registry.register(scheduler_handle);

    let tm = tunnel_manager.clone();
    let tunnel_handle = tokio::spawn(async move { tm.run().await });
    state.task_registry.register(tunnel_handle);

    let resume_handle = tokio::spawn(resume_interrupted_imports(state.clone()));
    state.task_registry.register(resume_handle);
}

/// Per-IP cap for both the login and TOTP-step rate limiters. 10/min proved
/// too tight even for legitimate traffic: a single office NAT (or a
/// sequential e2e test suite hitting /api/auth/login from one IP) can
/// plausibly exceed 10 login attempts within a minute with zero malicious
/// intent. The primary brute-force defense for password guessing is the
/// per-username+IP DB-tracked lockout (`MAX_LOGIN_ATTEMPTS`/`LOGIN_WINDOW_MINUTES`
/// in `api::auth`); this IP-only limiter is a coarser backstop, so it can afford more
/// headroom without meaningfully weakening protection.
const AUTH_RATE_LIMIT_PER_MINUTE: u32 = 30;

fn build_login_router(state: &AppState, client_ip_resolver: &ClientIpResolver) -> Router<AppState> {
    let login_ip_limiter = IpRateLimiter::new(AUTH_RATE_LIMIT_PER_MINUTE, Duration::from_mins(1));
    let login_rate_limit_state = IpRateLimitMiddlewareState {
        limiter: login_ip_limiter,
        resolver: client_ip_resolver.clone(),
    };
    let login = Router::new()
        .route("/api/auth/login", post(api::auth::login))
        .layer(axum_middleware::from_fn_with_state(
            login_rate_limit_state,
            ip_rate_limit_middleware,
        ));

    // TOTP verify-login and recovery complete a login using only a
    // temp_token, with no username/password - rate limit them too, or an
    // attacker who already has a valid password can brute-force the 6-digit
    // code (or a recovery code) with unlimited attempts. This uses its own
    // bucket rather than sharing login_ip_limiter's, so a burst of
    // ordinary password logins from the same IP (e.g. a shared office NAT)
    // can't starve a legitimate user's TOTP step of its own budget.
    let totp_ip_limiter = IpRateLimiter::new(AUTH_RATE_LIMIT_PER_MINUTE, Duration::from_mins(1));
    let totp_rate_limit_state = IpRateLimitMiddlewareState {
        limiter: totp_ip_limiter,
        resolver: client_ip_resolver.clone(),
    };
    let totp_login = Router::new()
        .route(
            "/api/auth/totp/verify-login",
            post(api::totp::totp_verify_login),
        )
        .route("/api/auth/totp/recovery", post(api::totp::totp_recovery))
        .layer(axum_middleware::from_fn_with_state(
            totp_rate_limit_state,
            ip_rate_limit_middleware,
        ));

    login.merge(totp_login).with_state(state.clone())
}

fn core_routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/logout", post(api::auth::logout))
        .route("/api/auth/me", get(api::auth::me))
        .route("/api/auth/refresh", post(api::auth::refresh_session))
        .route(
            "/api/auth/change-password",
            post(api::auth::change_password),
        )
        .route(
            "/api/auth/preferences",
            get(api::auth::get_preferences).put(api::auth::update_preferences),
        )
        .route("/api/auth/totp/setup", post(api::totp::totp_setup))
        .route("/api/auth/totp/verify", post(api::totp::totp_verify))
        .route("/api/auth/totp/disable", post(api::totp::totp_disable))
        .route("/api/auth/sessions", get(api::auth::list_sessions))
        .route(
            "/api/auth/sessions/{session_id}",
            delete(api::auth::revoke_session),
        )
        .route(
            "/api/users",
            get(api::users::list_users).post(api::users::create_user),
        )
        .route("/api/users/{id}/password", put(api::users::update_password))
        .route("/api/users/{id}", delete(api::users::delete_user))
        .route("/ws/agent", get(ws::handler::ws_handler))
        .route("/ws/ui", get(ws::ui_handler::ui_ws_handler))
        .route(
            "/ws/ssh-agent/{hostname}",
            get(ws::ssh_relay::ssh_relay_handler),
        )
}

fn agent_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/agents",
            get(api::agents::list_agents).post(api::agents::create_agent),
        )
        .route(
            "/api/agents/{hostname}",
            get(api::agents::get_agent)
                .put(api::agents::update_agent)
                .delete(api::agents::delete_agent),
        )
        .route(
            "/api/agents/{hostname}/regenerate-token",
            post(api::agents::regenerate_token),
        )
        .route(
            "/api/agents/{hostname}/restart",
            post(api::agents::restart_agent),
        )
        .route(
            "/api/agents/{hostname}/power",
            put(api::agents::update_agent_power),
        )
        .route(
            "/api/agents/{hostname}/availability",
            get(api::availability::get_agent_availability)
                .put(api::availability::update_agent_availability),
        )
        .route(
            "/api/agents/{hostname}/hostname-patterns",
            get(api::agents::list_hostname_patterns).post(api::agents::add_hostname_pattern),
        )
        .route(
            "/api/agents/{hostname}/hostname-patterns/{pattern_id}",
            delete(api::agents::delete_hostname_pattern),
        )
        .route(
            "/api/agents/{hostname}/merge-from/{source_id}",
            post(api::agents::merge_agent),
        )
        .route("/api/agents/{hostname}/hide", put(api::agents::hide_agent))
        .route(
            "/api/agents/{hostname}/unhide",
            put(api::agents::unhide_agent),
        )
        .route(
            "/api/agents/{hostname}/delete-archives",
            post(api::agents::delete_agent_archives),
        )
        .route(
            "/api/agents/{hostname}/deploy",
            post(api::deploy::deploy_agent),
        )
        .route(
            "/api/agents/{hostname}/service-unit",
            post(api::deploy::fetch_service_unit),
        )
        .route(
            "/api/agents/{hostname}/tunnel",
            get(api::tunnels::get_agent_tunnel),
        )
        .route(
            "/api/agents/{hostname}/repos",
            get(api::repos::get_agent_repos),
        )
        .route(
            "/api/agents/{hostname}/repos/{repo_id}/cancel-backup",
            post(api::agents::cancel_agent_backup),
        )
        .route(
            "/api/agents/{hostname}/reports",
            get(api::reports::list_reports),
        )
        .route(
            "/api/agents/{hostname}/reports/failed",
            delete(api::reports::delete_failed_reports),
        )
        .route(
            "/api/agents/{hostname}/reports/failed/count",
            get(api::reports::count_failed_reports),
        )
        .route(
            "/api/agents/{hostname}/tags",
            get(api::tags::get_agent_tags).put(api::tags::set_agent_tags),
        )
}

fn agent_vm_routes() -> Router<AppState> {
    Router::new()
        .route("/api/agents/{hostname}/vms", get(api::vms::get_agent_vms))
        .route(
            "/api/agents/{hostname}/vms/scan",
            post(api::vms::scan_agent_vms),
        )
        .route(
            "/api/agents/{hostname}/vms/build",
            post(api::vms::build_agent_vm),
        )
        .route(
            "/api/agents/{hostname}/vms/{name}",
            put(api::vms::update_agent_vm),
        )
        .route(
            "/api/agents/{hostname}/vms/{name}/snapshot",
            post(api::vms::snapshot_agent_vm),
        )
        .route(
            "/api/agents/{hostname}/vm-snapshot",
            put(api::vms::update_agent_vm_snapshot),
        )
}

/// Dependency hosts, and the schedule and agent settings that require them.
fn dependency_host_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/dependency-hosts",
            get(api::dependency_hosts::list_dependency_hosts)
                .post(api::dependency_hosts::create_dependency_host),
        )
        .route(
            "/api/dependency-hosts/test",
            post(api::dependency_hosts::test_dependency_address),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}",
            get(api::dependency_hosts::get_dependency_host)
                .put(api::dependency_hosts::update_dependency_host)
                .delete(api::dependency_hosts::delete_dependency_host),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}/power",
            put(api::dependency_hosts::update_dependency_host_power),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}/availability",
            get(api::dependency_hosts::get_dependency_host_availability)
                .put(api::dependency_hosts::update_dependency_host_availability),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}/availability/check",
            post(api::dependency_hosts::check_dependency_host_now),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}/test",
            post(api::dependency_hosts::test_dependency_host),
        )
        .route(
            "/api/dependency-hosts/{dependency_host_id}/usage",
            get(api::dependency_hosts::list_dependency_host_usage),
        )
        .route(
            "/api/schedules/{id}/dependencies",
            get(api::dependency_hosts::get_schedule_dependencies)
                .put(api::dependency_hosts::update_schedule_dependencies),
        )
        .route(
            "/api/agents/{hostname}/dependencies",
            get(api::dependency_hosts::get_agent_dependencies)
                .put(api::dependency_hosts::update_agent_dependencies),
        )
}

fn repo_routes() -> Router<AppState> {
    Router::new()
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
            "/api/repos/{repo_id}/destroy",
            post(api::repos::destroy_repo),
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
            get(api::repos::get_passphrase).put(api::keys::set_passphrase),
        )
        .route(
            "/api/repos/{repo_id}/availability",
            get(api::availability::get_repo_availability),
        )
        .route("/api/repo-hosts", get(api::repo_hosts::list_repo_hosts))
        .route(
            "/api/repo-hosts/{repo_host_id}",
            get(api::repo_hosts::get_repo_host)
                .put(api::repo_hosts::update_repo_host)
                .delete(api::repo_hosts::delete_repo_host),
        )
        .route(
            "/api/repo-hosts/{repo_host_id}/power",
            put(api::repo_hosts::update_repo_host_power),
        )
        .route(
            "/api/repo-hosts/{repo_host_id}/availability",
            get(api::availability::get_repo_host_availability)
                .put(api::availability::update_repo_host_availability),
        )
        .route(
            "/api/repo-hosts/{repo_host_id}/availability/check",
            post(api::availability::check_repo_host_availability_now),
        )
        .route(
            "/api/repo-hosts/{repo_host_id}/ssh-host-key/scan",
            post(api::repo_hosts::scan_repo_host_key),
        )
        .route(
            "/api/repo-hosts/{repo_host_id}/ssh-host-key",
            post(api::repo_hosts::accept_repo_host_key),
        )
        .route(
            "/api/repos/{repo_id}/schedules",
            get(api::repos::list_schedules_for_repo),
        )
        .route(
            "/api/repos/{repo_id}/confirm-relocation",
            post(api::repos::confirm_relocation),
        )
        .route(
            "/api/repos/{repo_id}/break-lock",
            post(api::repos::break_lock),
        )
        .route("/api/repos/{repo_id}/exec", post(api::repos::exec_borg))
        .route("/api/repos/{repo_id}/rescan", post(api::repos::rescan_repo))
        .route("/api/repos/{repo_id}/sync", post(api::repos::sync_repo))
        .route(
            "/api/repos/{repo_id}/reset-and-sync",
            post(api::repos::reset_and_sync_repo),
        )
        .route(
            "/api/repos/{repo_id}/reset-import",
            post(api::repos::reset_import),
        )
        .route("/api/repos/{repo_id}/dry-run", post(api::dryrun::dry_run))
        .route(
            "/api/repos/{repo_id}/tags",
            get(api::tags::get_repo_tags).put(api::tags::set_repo_tags),
        )
        .merge(repo_permission_routes())
}

fn repo_permission_routes() -> Router<AppState> {
    Router::new()
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
}

fn schedule_and_config_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/excludes",
            get(api::excludes::get_excludes).put(api::excludes::set_excludes),
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
            "/api/schedules/{id}/run",
            post(api::schedules::run_schedule_now),
        )
        .route(
            "/api/schedules/{id}/cancel",
            post(api::schedules::cancel_running_backup),
        )
        .route(
            "/api/schedules/{id}/reports",
            get(api::schedules::list_schedule_reports),
        )
        .route(
            "/api/schedules/{id}/reports/failed",
            delete(api::schedules::delete_failed_schedule_reports),
        )
        .route(
            "/api/schedules/{id}/reports/failed/count",
            get(api::schedules::count_failed_schedule_reports),
        )
        .route(
            "/api/schedules/{id}/targets",
            get(api::schedules::list_schedule_targets),
        )
        .route(
            "/api/schedules/{id}/repos",
            get(api::schedules::list_schedule_repos),
        )
        .route(
            "/api/schedules/{id}/sources",
            get(api::schedules::list_schedule_backup_sources),
        )
        .route(
            "/api/schedules/{id}/catch-up",
            get(api::schedules::list_schedule_catch_up_sources),
        )
        .route("/api/config/export", get(api::config_io::export_config))
        .route("/api/config/import", post(api::config_io::import_config))
}

fn system_and_audit_routes() -> Router<AppState> {
    Router::new()
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
        .route(
            "/api/system/database-storage",
            get(api::system::get_database_storage),
        )
        .route("/api/system/version", get(api::system::get_version))
        .route("/api/system/reset", post(api::system::reset_system))
        .route("/api/ssh/test-connection", post(api::ssh::test_connection))
        .route("/api/ssh/deploy-key", post(api::ssh::deploy_key))
        .route("/api/ssh/list-dir", post(api::ssh::list_dir))
        .route("/api/ssh/mkdir", post(api::ssh::mkdir))
}

fn stats_routes() -> Router<AppState> {
    Router::new()
        .route("/api/stats/summary", get(api::stats::summary))
        .route(
            "/api/stats/dashboard-overview",
            get(api::stats::dashboard_overview),
        )
        .route("/api/stats/storage", get(api::stats::storage))
        .route(
            "/api/stats/storage-breakdown",
            get(api::stats::storage_breakdown),
        )
        .route("/api/stats/activity", get(api::stats::activity))
        .route(
            "/api/stats/activity/{id}/acknowledge",
            post(api::stats::acknowledge_activity_entry)
                .delete(api::stats::unacknowledge_activity_entry),
        )
        .route(
            "/api/stats/activity/acknowledge-all",
            post(api::stats::acknowledge_all_activity),
        )
        .route(
            "/api/stats/activity/outstanding",
            get(api::stats::outstanding_acknowledgements),
        )
        .route("/api/stats/system-events", get(api::stats::system_events))
        .route(
            "/api/stats/system-events/{id}/acknowledge",
            post(api::stats::acknowledge_system_event)
                .delete(api::stats::unacknowledge_system_event),
        )
        .route("/api/stats/health", get(api::stats::health))
        .route("/api/stats/trends", get(api::stats::trends))
        .route("/api/stats/storage-trends", get(api::stats::storage_trends))
        .route(
            "/api/stats/storage-trends/by-repo",
            get(api::stats::storage_trends_by_repo),
        )
        .route("/api/stats/calendar", get(api::stats::calendar))
        .route(
            "/api/stats/schedule-counts",
            get(api::stats::schedule_counts),
        )
        .route(
            "/api/stats/findings/{finding_id}/dismiss",
            axum::routing::post(api::stats::dismiss_finding).delete(api::stats::undismiss_finding),
        )
        .route("/api/logs", get(api::logs::get_logs))
        .route(
            "/api/runs/{run_id}/events",
            get(api::reports::list_run_events),
        )
}

fn archive_routes() -> Router<AppState> {
    Router::new()
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
            "/api/repos/{repo_id}/archives/{archive_name}",
            delete(api::archives::delete_archive),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/contents",
            get(api::archives::list_contents),
        )
        .route(
            "/api/repos/{repo_id}/archives/{archive_name}/index-status",
            get(api::archives::get_archive_index_status),
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
}

fn access_control_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/tokens",
            get(api::tokens::list_tokens).post(api::tokens::create_token),
        )
        .route("/api/tokens/{id}", delete(api::tokens::delete_token))
        .route(
            "/api/server-quotas",
            get(api::server_quotas::list_server_quotas),
        )
        .route(
            "/api/server-quotas/{ssh_host}",
            put(api::server_quotas::upsert_server_quota)
                .delete(api::server_quotas::delete_server_quota),
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
            "/api/agent-tags",
            get(api::tags::list_agent_tag_associations),
        )
        .route("/api/repo-tags", get(api::tags::list_repo_tag_associations))
        .merge(rbac_routes())
}

fn rbac_routes() -> Router<AppState> {
    Router::new()
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
}

fn tunnel_routes() -> Router<AppState> {
    Router::new()
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
}

fn notification_routes() -> Router<AppState> {
    Router::new()
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
}

fn misc_routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(api::health::health))
        .route(
            "/api/openapi.json",
            get(|| async { Json(ApiDoc::openapi()) }),
        )
        .merge(Scalar::with_url("/api/docs", ApiDoc::openapi()))
}

fn build_router(state: &AppState, login_router: Router<AppState>) -> Router<AppState> {
    // auth_tracking_middleware does a session/user DB lookup for every
    // request it sees, so it must only wrap routes that actually require
    // authentication -- login_router (login, TOTP verify/recovery) and
    // misc_routes() (health check, OpenAPI docs) are intentionally
    // unauthenticated and merged in outside this layer. Wrapping
    // login_router in particular would run this lookup before its own
    // ip_rate_limit_middleware gets a chance to reject the request.
    let authenticated_routes = core_routes()
        .merge(agent_routes())
        .merge(agent_vm_routes())
        .merge(repo_routes())
        .merge(dependency_host_routes())
        .merge(schedule_and_config_routes())
        .merge(system_and_audit_routes())
        .merge(stats_routes())
        .merge(archive_routes())
        .merge(access_control_routes())
        .merge(tunnel_routes())
        .merge(notification_routes())
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            auth_tracking_middleware,
        ));

    Router::new()
        .merge(login_router)
        .merge(authenticated_routes)
        .merge(misc_routes())
}

async fn configure_docs_and_static(app: Router) -> Router {
    let docs_dir = std::env::var("ASSIMILATE_DOCS_DIR")
        .map_or_else(|_| PathBuf::from("./docs_html"), PathBuf::from);
    let app = if tokio::fs::try_exists(&docs_dir).await.unwrap_or(false) {
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
    if tokio::fs::try_exists(&static_dir).await.unwrap_or(false) {
        let index = static_dir.join("index.html");
        app.fallback_service(ServeDir::new(&static_dir).fallback(ServeFile::new(index)))
    } else {
        app
    }
}

/// Opens the database pool, retrying while the database is still starting up
/// (e.g. a fresh `docker compose up`). Gives up after `retry.max_attempts`
/// attempts and returns the last connection error.
async fn connect_with_retry(
    url: &str,
    max_connections: u32,
    retry: DbConnectRetry,
) -> Result<PgPool, StartupError> {
    let DbConnectRetry {
        max_attempts,
        retry_interval,
        acquire_timeout,
    } = retry;
    let mut attempt: u32 = 1;
    loop {
        match sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(acquire_timeout)
            .connect(url)
            .await
        {
            Ok(pool) => {
                if attempt > 1 {
                    tracing::info!("database connection established after {attempt} attempts");
                }
                return Ok(pool);
            }
            Err(e) if attempt < max_attempts => {
                tracing::warn!(
                    "database connection attempt {attempt}/{max_attempts} failed: {e}, retrying \
                     in {retry_interval:?}"
                );
                tokio::time::sleep(retry_interval).await;
                attempt = attempt.saturating_add(1);
            }
            Err(e) => return Err(e.into()),
        }
    }
}

async fn shutdown_signal(
    registry: ws::registry::AgentRegistry,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(e) => {
                tracing::error!("failed to install SIGTERM handler, relying on Ctrl+C only: {e}");
                std::future::pending::<()>().await;
            }
        }
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

    tracing::info!("shutdown signal received, notifying agents");

    let agents = registry.connected_agents().await;
    for agent_id in &agents {
        if let Err(e) = registry
            .send_to(*agent_id, ServerToAgent::ShuttingDown)
            .await
        {
            tracing::debug!(
                agent_id = *agent_id,
                error = %e,
                "failed to send shutdown message to agent"
            );
        }
    }

    if !agents.is_empty() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    shutdown_token.cancel();

    tracing::info!("shutting down gracefully");
}

async fn bootstrap_admin(pool: &PgPool) -> Result<(), StartupError> {
    let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?
        .unwrap_or(0);

    if count > 0 {
        return Ok(());
    }

    let hash = bcrypt::hash("admin", 10)?;

    let user_id: i64 = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, must_change_password) VALUES ('admin', $1, \
         true) RETURNING id",
        &hash,
    )
    .fetch_one(pool)
    .await?;

    let admin_role_id: i64 = sqlx::query_scalar!("SELECT id FROM roles WHERE name = 'admin'",)
        .fetch_one(pool)
        .await?;

    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        user_id,
        admin_role_id,
    )
    .execute(pool)
    .await?;

    tracing::info!(
        "default admin user created (password: admin) -- password change required on first login"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn serve_until_shutdown_returns_graceful_when_server_finishes() {
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let outcome = serve_until_shutdown(
            std::future::ready(Ok(())),
            shutdown_rx,
            Duration::from_mins(1),
        )
        .await
        .unwrap();
        assert_eq!(outcome, ShutdownOutcome::Graceful);
    }

    #[tokio::test]
    async fn serve_until_shutdown_propagates_server_error() {
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let result = serve_until_shutdown(
            std::future::ready(Err(std::io::Error::other("accept failed"))),
            shutdown_rx,
            Duration::from_mins(1),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn serve_until_shutdown_times_out_when_server_never_drains() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        shutdown_tx.send(()).unwrap();
        let outcome = serve_until_shutdown(
            std::future::pending(),
            shutdown_rx,
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(outcome, ShutdownOutcome::TimedOut);
    }

    fn test_app_state(pool: PgPool) -> AppState {
        let ui_broadcast = server::ws::ui_broadcast::UiBroadcast::new();
        let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-main").unwrap();
        build_app_state(BuildAppStateArgs {
            encryption_key,
            tunnel_manager: TunnelManager::new(pool.clone(), ui_broadcast.clone(), server_addr),
            ui_broadcast,
            log_buffer: LogBuffer::default(),
            notification_service: NotificationService::new(pool.clone(), encryption_key),
            client_ip_resolver: ClientIpResolver::from_env(None),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            pool,
        })
    }

    /// Serialises the tests that point `BORG_BINARY` at a [`FakeBorg`]: the
    /// variable is process-wide.
    static BORG_BINARY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Exits non-zero for every subcommand, so the resumed sync fails.
    const FAILING_BORG: &str = "exit 2\n";

    /// Lists a repository without any archives, so the resumed sync succeeds.
    const EMPTY_REPO_BORG: &str =
        "case \"$1\" in\n  list) echo '{\"archives\":[]}' ;;\n  *) exit 1 ;;\nesac\n";

    /// A fake borg, installed as `BORG_BINARY` until dropped, that appends
    /// each invocation's subcommand to a log before running its body. Every
    /// resumed-import test installs one, so the outcome of its sync never
    /// depends on whether a real `borg` happens to be on `PATH`.
    struct FakeBorg {
        dir: tempfile::TempDir,
        _lock: tokio::sync::MutexGuard<'static, ()>,
    }

    impl FakeBorg {
        async fn install(body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt as _;

            let lock = BORG_BINARY_LOCK.lock().await;
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("calls.log");
            let borg = dir.path().join("borg");
            tokio::fs::write(
                &borg,
                format!("#!/bin/sh\necho \"$1\" >> '{}'\n{body}", log.display()),
            )
            .await
            .unwrap();
            tokio::fs::set_permissions(&borg, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
            // SAFETY: every test that touches BORG_BINARY holds BORG_BINARY_LOCK.
            unsafe { std::env::set_var("BORG_BINARY", &borg) };
            Self { dir, _lock: lock }
        }

        /// Whether anything ran this borg.
        async fn ran(&self) -> bool {
            tokio::fs::try_exists(self.dir.path().join("calls.log"))
                .await
                .unwrap()
        }
    }

    impl Drop for FakeBorg {
        fn drop(&mut self) {
            // SAFETY: BORG_BINARY_LOCK is still held; it is released after this.
            unsafe { std::env::remove_var("BORG_BINARY") };
        }
    }

    /// Inserts a repository left `importing = true`, as a server restart
    /// mid-import leaves it, and returns its id.
    async fn insert_importing_repo(pool: &PgPool, name: &str) -> i64 {
        let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-main").unwrap();
        let passphrase_encrypted =
            shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
        let repo = db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name,
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        db::set_repo_importing(pool, repo.id, true).await.unwrap();
        repo.id
    }

    async fn is_importing(pool: &PgPool, repo_id: i64) -> bool {
        db::list_importing_repo_ids(pool)
            .await
            .unwrap()
            .contains(&repo_id)
    }

    async fn import_error(pool: &PgPool, repo_id: i64) -> Option<String> {
        db::get_repo_with_stats(pool, repo_id)
            .await
            .unwrap()
            .import_error
    }

    /// Exercises `resume_interrupted_imports`/`resume_single_import` - the
    /// startup routine that resumes a repo left `importing = true` from before
    /// a server restart (e.g. after a crash). This was previously only ever
    /// incidentally covered when a demo container happened to restart
    /// mid-import during CI - the same non-deterministic-coverage class
    /// already fixed for `enrich_archive_stats_background` (#371) and
    /// `run_repo_sync` (`scheduler.rs`) - so it exercises the function
    /// directly and deterministically instead. A fake borg that fails every
    /// call drives the resume through its error-handling path.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resume_interrupted_imports_clears_importing_flag_on_failure(pool: sqlx::PgPool) {
        let borg = FakeBorg::install(FAILING_BORG).await;
        let repo_id = insert_importing_repo(&pool, "resume-test-repo").await;

        let state = test_app_state(pool.clone());
        resume_interrupted_imports(state.clone()).await;

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;

        assert!(borg.ran().await);
        assert!(!is_importing(&pool, repo_id).await);
        assert!(import_error(&pool, repo_id).await.is_some());
    }

    /// A config-imported repository held for its passphrase is importing but
    /// has nothing to resume: resuming it with the placeholder passphrase only
    /// ever failed, left an import error behind, and - while it ran - made
    /// setting the passphrase report a sync in progress.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resume_interrupted_imports_skips_a_repo_held_for_its_passphrase(pool: sqlx::PgPool) {
        let borg = FakeBorg::install(FAILING_BORG).await;
        let repo_id = insert_importing_repo(&pool, "held-for-passphrase-repo").await;
        db::hold_repo_for_passphrase(&pool, repo_id).await.unwrap();

        let state = test_app_state(pool.clone());
        resume_interrupted_imports(state.clone()).await;
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;

        assert!(!borg.ran().await);
        assert!(is_importing(&pool, repo_id).await);
        assert_eq!(import_error(&pool, repo_id).await, None);
    }

    /// A resumed import whose sync succeeds clears `importing` without
    /// recording an import error.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resume_interrupted_imports_clears_importing_flag_on_success(pool: sqlx::PgPool) {
        let borg = FakeBorg::install(EMPTY_REPO_BORG).await;
        let repo_id = insert_importing_repo(&pool, "resume-success-repo").await;

        let state = test_app_state(pool.clone());
        resume_interrupted_imports(state.clone()).await;
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;

        assert!(borg.ran().await);
        assert!(!is_importing(&pool, repo_id).await);
        assert!(import_error(&pool, repo_id).await.is_none());
    }

    /// The resumed import's full sync prunes index rows and garbage-collects
    /// directory paths, so it waits for the repository lock like the fresh
    /// import does instead of running alongside an indexing run or the
    /// content-index eviction.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resumed_import_waits_for_the_repo_lock(pool: sqlx::PgPool) {
        let _borg = FakeBorg::install(FAILING_BORG).await;
        let repo_id = insert_importing_repo(&pool, "resume-lock-repo").await;

        let state = test_app_state(pool.clone());
        let held = state.repo_lock.acquire(repo_id).await;
        resume_interrupted_imports(state.clone()).await;

        tokio::time::timeout(Duration::from_secs(30), async {
            while state.repo_lock.queued(repo_id).await == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the resumed import never queued for the repository lock");
        assert!(
            import_error(&pool, repo_id).await.is_none(),
            "the sync must not run while the lock is held"
        );
        assert!(is_importing(&pool, repo_id).await);

        drop(held);
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
        assert!(
            import_error(&pool, repo_id).await.is_some(),
            "the sync ran once released"
        );
        assert!(!is_importing(&pool, repo_id).await);
    }

    /// A resumed import superseded by a newer import of the same repository
    /// while it is still syncing leaves the repository's import state to that
    /// newer import: its own failure is neither recorded nor allowed to clear
    /// `importing`, and the newer import stays registered.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn superseded_resumed_import_leaves_the_import_state_alone(pool: sqlx::PgPool) {
        let borg = FakeBorg::install(FAILING_BORG).await;
        let repo_id = insert_importing_repo(&pool, "resume-superseded-repo").await;

        let state = test_app_state(pool.clone());
        let held = state.repo_lock.acquire(repo_id).await;
        resume_interrupted_imports(state.clone()).await;
        wait_until_queued(&state.repo_lock, repo_id).await;
        let (newer_task, newer_cancel) = state.import_tasks.start(repo_id).await;

        drop(held);
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;

        assert!(borg.ran().await, "the superseded sync never ran");
        assert!(import_error(&pool, repo_id).await.is_none());
        assert!(is_importing(&pool, repo_id).await);
        assert!(state.import_tasks.is_current(repo_id, newer_task).await);
        assert!(!newer_cancel.is_cancelled());
    }

    /// A resumed import cancelled while still queued for the repository lock
    /// never runs its sync, and leaves the repository's import state to
    /// whoever cancelled it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn resumed_import_cancelled_while_queued_never_syncs(pool: sqlx::PgPool) {
        let borg = FakeBorg::install(FAILING_BORG).await;
        let repo_id = insert_importing_repo(&pool, "resume-cancelled-repo").await;

        let state = test_app_state(pool.clone());
        let held = state.repo_lock.acquire(repo_id).await;
        resume_interrupted_imports(state.clone()).await;
        wait_until_queued(&state.repo_lock, repo_id).await;
        assert!(state.import_tasks.cancel(repo_id).await);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
        drop(held);

        assert!(!borg.ran().await, "the cancelled import ran its sync");
        assert!(import_error(&pool, repo_id).await.is_none());
        assert!(is_importing(&pool, repo_id).await);
    }

    /// Flags, when dropped, that the work future owning it has been torn down.
    struct TornDown(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for TornDown {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Stand-in for a resumed import's sync that never finishes on its own:
    /// reports through `polled` once it has been polled, and through its
    /// [`TornDown`] once it has been dropped.
    struct PendingWork {
        polled: tokio::sync::watch::Sender<bool>,
        _torn_down: TornDown,
    }

    impl std::future::Future for PendingWork {
        type Output = ();

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            self.polled.send_replace(true);
            std::task::Poll::Pending
        }
    }

    /// A [`PendingWork`], with the receiver reporting whether it has been
    /// polled and the flag reporting whether it has been torn down.
    fn pending_work() -> (
        PendingWork,
        tokio::sync::watch::Receiver<bool>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (polled, polled_rx) = tokio::sync::watch::channel(false);
        let torn_down = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let work = PendingWork {
            polled,
            _torn_down: TornDown(std::sync::Arc::clone(&torn_down)),
        };
        (work, polled_rx, torn_down)
    }

    async fn wait_until_queued(repo_lock: &server::RepoLock, repo_id: i64) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while repo_lock.queued(repo_id).await == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("nothing queued for the repository lock");
    }

    /// Cancelling a resumed import mid-sync must not hand the repository lock
    /// to an indexing run or eviction queued behind it while the sync is still
    /// in flight: the waiter only gets the lock once the sync future is gone.
    #[tokio::test]
    async fn cancelled_locked_work_holds_the_lock_until_torn_down() {
        let repo_lock = server::RepoLock::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (work, mut polled, torn_down) = pending_work();

        let runner = tokio::spawn({
            let repo_lock = repo_lock.clone();
            let cancel = cancel.clone();
            async move { run_locked_until_cancelled(&repo_lock, 1, &cancel, work).await }
        });
        polled.wait_for(|polled| *polled).await.unwrap();

        let waiter = tokio::spawn({
            let repo_lock = repo_lock.clone();
            let torn_down = std::sync::Arc::clone(&torn_down);
            async move {
                let _guard = repo_lock.acquire(1).await;
                torn_down.load(std::sync::atomic::Ordering::SeqCst)
            }
        });
        wait_until_queued(&repo_lock, 1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "the waiter ran alongside the work");

        cancel.cancel();
        assert!(!runner.await.unwrap(), "cancelled work reported completion");
        assert!(
            waiter.await.unwrap(),
            "the waiter got the lock before the cancelled work was torn down"
        );
    }

    /// A cancellation that lands while the resumed import is still queued for
    /// the lock leaves without ever running the sync.
    #[tokio::test]
    async fn cancelled_while_queued_never_runs_the_work() {
        let repo_lock = server::RepoLock::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (work, polled, torn_down) = pending_work();
        let held = repo_lock.acquire(1).await;

        let runner = tokio::spawn({
            let repo_lock = repo_lock.clone();
            let cancel = cancel.clone();
            async move { run_locked_until_cancelled(&repo_lock, 1, &cancel, work).await }
        });
        wait_until_queued(&repo_lock, 1).await;

        cancel.cancel();
        assert!(!runner.await.unwrap());
        assert!(!*polled.borrow(), "the work ran despite the cancellation");
        assert!(torn_down.load(std::sync::atomic::Ordering::SeqCst));
        drop(held);
    }

    /// Uncancelled work runs to completion with the lock held, and releases it
    /// afterwards.
    #[tokio::test]
    async fn uncancelled_locked_work_runs_to_completion() {
        let repo_lock = server::RepoLock::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let completed = run_locked_until_cancelled(&repo_lock, 1, &cancel, async {
            let contender =
                tokio::time::timeout(Duration::from_millis(20), repo_lock.acquire(1)).await;
            assert!(contender.is_err(), "the lock was not held during the work");
        })
        .await;
        assert!(completed);
        tokio::time::timeout(Duration::from_secs(5), repo_lock.acquire(1))
            .await
            .expect("the lock was not released after the work");
    }

    const UNREACHABLE_DB_URL: &str = "postgres://assimilate@127.0.0.1:1/assimilate";

    /// Nothing listens on port 1, so every attempt fails fast: the retry loop
    /// must give up after `max_attempts` and surface the connection error
    /// instead of panicking, having slept between attempts.
    #[tokio::test]
    async fn connect_with_retry_returns_error_after_max_attempts() {
        let retry = DbConnectRetry {
            max_attempts: 3,
            retry_interval: Duration::from_millis(50),
            acquire_timeout: Duration::from_millis(100),
        };
        let started = std::time::Instant::now();

        let result = connect_with_retry(UNREACHABLE_DB_URL, 1, retry).await;

        assert!(matches!(result, Err(StartupError::Database(_))));
        assert!(started.elapsed() >= retry.retry_interval.saturating_mul(2));
    }

    #[tokio::test]
    async fn connect_with_retry_with_zero_attempts_still_tries_once() {
        let retry = DbConnectRetry {
            max_attempts: 0,
            retry_interval: Duration::from_secs(30),
            acquire_timeout: Duration::from_millis(100),
        };

        let result = connect_with_retry(UNREACHABLE_DB_URL, 1, retry).await;

        assert!(matches!(result, Err(StartupError::Database(_))));
    }
}
