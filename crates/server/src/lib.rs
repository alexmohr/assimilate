// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Server crate for the assimilate backup system.

/// REST API handlers, grouped by domain.
pub mod api;
/// Borg archive content indexing and querying.
pub mod archive_index;
/// Tracks fire-and-forget background tasks not covered by other trackers.
pub mod background_tasks;
/// Borg subprocess management.
pub mod borg;
/// Client IP resolution from request headers.
pub mod client_ip;
/// Assembles per-agent configuration from the database.
pub mod config_assembler;
/// Session cookie parsing and construction helpers.
pub mod cookies;
/// Database query helpers.
pub mod db;
/// Error types for the API.
pub mod error;
/// In-memory ring buffer for log entries.
pub mod log_buffer;
/// Axum middleware (CSP headers).
pub mod middleware;
/// Notification channels (webhook, email, push).
pub mod notifications;
/// `OpenAPI` (utoipa) documentation struct.
pub mod openapi;
/// Quota enforcement logic run after each backup.
pub mod quota_enforcement;
/// IP-based rate limiter middleware.
pub mod rate_limit;
/// Tracks active/queued repository operations for the UI.
pub mod repo_op_tracker;
/// Scheduler that ticks schedules, syncs, retention, and session cleanup.
pub mod scheduler;
/// SSH key management, host key scanning, and key deployment.
pub mod ssh;
/// SSH reverse-tunnel management for agent connectivity.
pub mod tunnel;
/// WebSocket handlers (agent, UI, SSH relay).
pub mod ws;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use shared::types::DryRunFile;
use sqlx::PgPool;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    background_tasks::BackgroundTaskTracker,
    client_ip::ClientIpResolver,
    log_buffer::LogBuffer,
    notifications::NotificationService,
    repo_op_tracker::RepoOpTracker,
    tunnel::TunnelManager,
    ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

/// Per-repository mutex that serialises borg operations on the same repo.
#[derive(Clone, Default)]
pub struct RepoLock {
    locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
}

impl RepoLock {
    /// Acquire the per-repo lock, blocking until the current holder releases it.
    pub async fn acquire(&self, repo_id: i64) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut map = self.locks.lock().await;
            Arc::clone(
                map.entry(repo_id)
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        mutex.lock_owned().await
    }

    /// Drop all per-repo mutex entries so subsequent `acquire` calls get fresh,
    /// unlocked mutexes. Stuck tasks that hold an `OwnedMutexGuard` from before
    /// this call continue to own their (now orphaned) guard; they cannot block
    /// any new operations because new callers will receive a different `Arc`.
    pub async fn force_reset(&self) {
        self.locks.lock().await.clear();
    }
}

/// (`run_id`, `files`, `total_size`, `error_message`)
pub type PendingDryRuns =
    Arc<Mutex<HashMap<String, oneshot::Sender<(Vec<DryRunFile>, i64, Option<String>)>>>>;

/// (`success`, `files_restored`, `error_message`)
pub type PendingRestores =
    Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u64, Option<String>)>>>>;

/// (`success`, `error_message`)
pub type PendingMigrations = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, Option<String>)>>>>;

/// (`success`, `deleted_count`, `error_message`)
pub type PendingDeletes = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u32, Option<String>)>>>>;

/// Empty backing map for a `Pending*` one-shot-channel registry
/// (`PendingDryRuns`, `PendingRestores`, `PendingMigrations`, `PendingDeletes`).
#[must_use]
pub fn new_pending_map<T>() -> Arc<Mutex<HashMap<String, T>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

/// An in-flight repository import task with cancellation support.
#[derive(Clone)]
struct ImportTaskEntry {
    id: u64,
    cancel: CancellationToken,
}

/// Tracks running repository import tasks so they can be cancelled or checked
/// for staleness.
#[derive(Clone, Default)]
pub struct ImportTaskRegistry {
    tasks: Arc<Mutex<HashMap<i64, ImportTaskEntry>>>,
    next_id: Arc<AtomicU64>,
}

impl ImportTaskRegistry {
    /// Register a new import task for the given repo and return its ID and cancellation token.
    pub async fn start(&self, repo_id: i64) -> (u64, CancellationToken) {
        let id = self
            .next_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let cancel = CancellationToken::new();
        self.tasks.lock().await.insert(
            repo_id,
            ImportTaskEntry {
                id,
                cancel: cancel.clone(),
            },
        );
        (id, cancel)
    }

    /// Cancel the running import for `repo_id`, if any, and remove it.
    pub async fn cancel(&self, repo_id: i64) -> bool {
        let entry = self.tasks.lock().await.remove(&repo_id);
        if let Some(entry) = entry {
            entry.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Check whether the given `task_id` is still the current task for `repo_id`.
    pub async fn is_current(&self, repo_id: i64, task_id: u64) -> bool {
        self.tasks
            .lock()
            .await
            .get(&repo_id)
            .is_some_and(|entry| entry.id == task_id)
    }

    /// Remove the task if it is still the current one for this repo.
    pub async fn finish(&self, repo_id: i64, task_id: u64) {
        let mut tasks = self.tasks.lock().await;
        if tasks.get(&repo_id).is_some_and(|entry| entry.id == task_id) {
            tasks.remove(&repo_id);
        }
    }
}

/// Shared application state accessible from every handler.
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool.
    pub pool: PgPool,
    /// Derived AES-256-GCM encryption key for passphrases.
    pub encryption_key: [u8; 32],
    /// Registry of connected WebSocket agents.
    pub registry: AgentRegistry,
    /// Broadcast channel for UI-facing WebSocket messages.
    pub ui_broadcast: UiBroadcast,
    /// Manages SSH reverse tunnels to agents.
    pub tunnel_manager: TunnelManager,
    /// In-memory ring buffer of recent log entries.
    pub log_buffer: LogBuffer,
    /// Notification dispatch service.
    pub notification_service: NotificationService,
    /// Broadcast channel for backup completion events.
    pub completion_bus: CompletionBus,
    /// Tracks active/queued repository operations.
    pub repo_op_tracker: RepoOpTracker,
    /// Tracks fire-and-forget background tasks not covered by other trackers
    /// (e.g. archive stat enrichment, post-backup sync/indexing).
    pub background_task_tracker: BackgroundTaskTracker,
    /// Per-repository mutex for serialising borg operations.
    pub repo_lock: RepoLock,
    /// Tracks running repository import tasks.
    pub import_tasks: ImportTaskRegistry,
    /// One-shot channels for pending dry-run operations.
    pub pending_dryruns: PendingDryRuns,
    /// One-shot channels for pending restore operations.
    pub pending_restores: PendingRestores,
    /// One-shot channels for pending migration operations.
    pub pending_migrations: PendingMigrations,
    /// One-shot channels for pending delete operations.
    pub pending_deletes: PendingDeletes,
    /// Token that signals server shutdown.
    pub shutdown_token: CancellationToken,
    /// Resolves the real client IP behind proxies.
    pub client_ip_resolver: ClientIpResolver,
    /// Collects fire-and-forget work spawned outside the request/response cycle -
    /// every `Borg` invocation's `GracefulChild` reaper (SIGKILL-escalation +
    /// break-lock), a `RepoOpGuard`'s deferred clear, and the server's long-lived
    /// background loops (scheduler, tunnel manager, interrupted-import resume) -
    /// so `main`'s shutdown can join them instead of abandoning them mid-work when
    /// the process exits.
    pub task_registry: shared::task_registry::TaskRegistry,
    /// Cached session idle timeout in minutes (default 480/8h).
    /// Read from `system_settings` on startup and refreshed when the admin updates it.
    pub session_idle_timeout_minutes: Arc<AtomicI64>,
}

impl AppState {
    /// Reload the session idle timeout from the database.
    ///
    /// Called at startup and whenever the admin setting is updated.
    pub async fn reload_session_idle_timeout(&self) {
        match db::get_setting(&self.pool, "session_idle_timeout_minutes").await {
            Ok(Some(value)) => {
                if let Ok(minutes) = value.parse::<i64>() {
                    self.session_idle_timeout_minutes
                        .store(minutes, Ordering::Relaxed);
                }
            }
            Ok(None) => {
                // Setting not in DB yet - keep the default.
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load session_idle_timeout_minutes");
            }
        }
    }
}
