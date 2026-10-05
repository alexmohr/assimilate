// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
};

use shared::{
    protocol::AgentToServer,
    types::{RepoId, build_repo_url},
};
use tokio::{
    sync::{Mutex, Semaphore, mpsc},
    task::AbortHandle,
};
use tracing::error;

use super::{Executor, OwnedTaskContext, transport::backup_target_from_repo};
use crate::{backup::BackupTarget, borg::Borg};

/// Why a request naming a repo could not be resolved against the config.
pub(super) enum ConfigLookupError {
    /// The server has not sent a config yet.
    NoConfig,
    /// The config does not list the repo.
    UnknownRepo,
}

impl Executor {
    pub(super) async fn repo_operation_queue(&self, repo_key: &RepoOperationKey) -> Arc<Semaphore> {
        let mut repo_operation_queues = self.repo_operation_queues.lock().await;
        Arc::clone(
            repo_operation_queues
                .entry(repo_key.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    pub(super) fn next_task_id(&self) -> u64 {
        self.next_task_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) async fn push_backup_task(
        active_backup_tasks: &Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
        repo_id: RepoId,
        task: ActiveBackupTask,
    ) {
        let mut active_backup_tasks = active_backup_tasks.lock().await;
        active_backup_tasks.entry(repo_id).or_default().push(task);
    }

    pub(super) async fn take_backup_tasks(
        active_backup_tasks: &Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
        repo_id: RepoId,
    ) -> Option<Vec<ActiveBackupTask>> {
        active_backup_tasks.lock().await.remove(&repo_id)
    }

    pub(super) async fn remove_backup_task(
        active_backup_tasks: &Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
        repo_id: RepoId,
        task_id: u64,
    ) {
        let mut active_backup_tasks = active_backup_tasks.lock().await;
        let Some(tasks) = active_backup_tasks.get_mut(&repo_id) else {
            return;
        };

        tasks.retain(|task| task.task_id != task_id);
        if tasks.is_empty() {
            active_backup_tasks.remove(&repo_id);
        }
    }

    /// Runs `task` once `repo_key`'s queue lets it, so operations against the
    /// same physical repository never overlap.
    pub(super) async fn spawn_queued(
        &self,
        repo_id: RepoId,
        repo_key: RepoOperationKey,
        task: impl Future<Output = ()> + Send + 'static,
    ) {
        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            task.await;
        });
        self.task_registry.register(handle);
    }

    /// The context a queued request task for `hostname` runs with.
    pub(super) fn owned_task_context(
        &self,
        hostname: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) -> OwnedTaskContext {
        OwnedTaskContext {
            hostname,
            server_url: self.server_url.clone(),
            token: self.token.clone(),
            outbound_tx: outbound_tx.clone(),
            borg: Borg::new(self.task_registry.clone()),
        }
    }

    /// The target for configured repo `repo_id`, and the agent hostname to
    /// run it as.
    pub(super) async fn configured_target(
        &self,
        repo_id: RepoId,
    ) -> Result<(BackupTarget, String), ConfigLookupError> {
        let config_guard = self.current_config.lock().await;
        let config = config_guard.as_ref().ok_or(ConfigLookupError::NoConfig)?;
        let repo = config
            .repos
            .iter()
            .find(|r| r.repo_id == repo_id)
            .ok_or(ConfigLookupError::UnknownRepo)?;
        let target =
            backup_target_from_repo(repo, &config.agent_hostname, None, &config.vm_snapshot);
        Ok((target, config.agent_hostname.clone()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RepoOperationKey {
    ssh_user: String,
    ssh_host: String,
    ssh_port: u16,
    repo_path: String,
}

impl RepoOperationKey {
    pub(super) fn from_backup_target(target: &BackupTarget) -> Self {
        Self {
            ssh_user: target.ssh_user.clone(),
            ssh_host: target.ssh_host.clone(),
            ssh_port: target.ssh_port,
            repo_path: target.repo_path.clone(),
        }
    }

    pub(super) fn repo_url(&self) -> String {
        build_repo_url(
            &self.ssh_user,
            &self.ssh_host,
            self.ssh_port,
            &self.repo_path,
        )
    }
}

pub(super) struct ActiveBackupTask {
    pub(super) task_id: u64,
    pub(super) abort_handle: AbortHandle,
}
