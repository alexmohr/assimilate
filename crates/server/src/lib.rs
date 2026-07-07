// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod api;
pub mod archive_index;
pub mod borg;
pub mod client_ip;
pub mod config_assembler;
pub mod cookies;
pub mod db;
pub mod error;
pub mod log_buffer;
pub mod middleware;
pub mod notifications;
pub mod openapi;
pub mod quota_enforcement;
pub mod rate_limit;
pub mod repo_op_tracker;
pub mod scheduler;
pub mod ssh;
pub mod tunnel;
pub mod ws;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use shared::types::DryRunFile;
use sqlx::PgPool;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    client_ip::ClientIpResolver,
    log_buffer::LogBuffer,
    notifications::NotificationService,
    repo_op_tracker::RepoOpTracker,
    tunnel::TunnelManager,
    ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

#[derive(Clone, Default)]
pub struct RepoLock {
    locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
}

impl RepoLock {
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

pub type PendingDryRuns =
    Arc<Mutex<HashMap<String, oneshot::Sender<(Vec<DryRunFile>, i64, Option<String>)>>>>;

/// (success, files_restored, error_message)
pub type PendingRestores =
    Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u64, Option<String>)>>>>;

pub type PendingMigrations = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, Option<String>)>>>>;

/// (success, deleted_count, error_message)
pub type PendingDeletes = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u32, Option<String>)>>>>;

#[derive(Clone)]
struct ImportTaskEntry {
    id: u64,
    cancel: CancellationToken,
}

#[derive(Clone, Default)]
pub struct ImportTaskRegistry {
    tasks: Arc<Mutex<HashMap<i64, ImportTaskEntry>>>,
    next_id: Arc<AtomicU64>,
}

impl ImportTaskRegistry {
    pub async fn start(&self, repo_id: i64) -> (u64, CancellationToken) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
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

    pub async fn cancel(&self, repo_id: i64) -> bool {
        let entry = self.tasks.lock().await.remove(&repo_id);
        if let Some(entry) = entry {
            entry.cancel.cancel();
            true
        } else {
            false
        }
    }

    pub async fn is_current(&self, repo_id: i64, task_id: u64) -> bool {
        self.tasks
            .lock()
            .await
            .get(&repo_id)
            .is_some_and(|entry| entry.id == task_id)
    }

    pub async fn finish(&self, repo_id: i64, task_id: u64) {
        let mut tasks = self.tasks.lock().await;
        if tasks.get(&repo_id).is_some_and(|entry| entry.id == task_id) {
            tasks.remove(&repo_id);
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub encryption_key: [u8; 32],
    pub registry: AgentRegistry,
    pub ui_broadcast: UiBroadcast,
    pub tunnel_manager: TunnelManager,
    pub log_buffer: LogBuffer,
    pub notification_service: NotificationService,
    pub completion_bus: CompletionBus,
    pub repo_op_tracker: RepoOpTracker,
    pub repo_lock: RepoLock,
    pub import_tasks: ImportTaskRegistry,
    pub pending_dryruns: PendingDryRuns,
    pub pending_restores: PendingRestores,
    pub pending_migrations: PendingMigrations,
    pub pending_deletes: PendingDeletes,
    pub shutdown_token: CancellationToken,
    pub client_ip_resolver: ClientIpResolver,
}
