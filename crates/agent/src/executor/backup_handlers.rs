// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::sync::Arc;

use shared::{protocol::AgentToServer, types::RepoId};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::{
    ActiveBackupTask, BackupTaskContext, Executor, RepoOperationKey,
    backup_task::run_backup_task,
    maintenance::{run_check_task, run_verify_task},
    transport::backup_target_from_repo,
};

impl Executor {
    pub(super) async fn handle_run_now(
        &self,
        repo_id: RepoId,
        schedule_id: Option<i64>,
        run_id: Option<String>,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting backup");
            let msg = AgentToServer::BackupRejected {
                repo_id,
                reason: "agent has no config yet".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config, rejecting");
            let msg = AgentToServer::BackupRejected {
                repo_id,
                reason: "repo not found in agent config".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let target = backup_target_from_repo(
            repo,
            &config.agent_hostname,
            schedule_id,
            &config.vm_snapshot,
        );
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let task_id = self.next_task_id();
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let active_backup_tasks = Arc::clone(&self.active_backup_tasks);
        let active_backup_tasks_for_spawn = Arc::clone(&active_backup_tasks);

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg backup");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_backup_task(
                repo_id,
                target,
                BackupTaskContext {
                    hostname,
                    server_url,
                    token,
                    run_id,
                },
                &engine,
                &outbound,
            )
            .await;
            Self::remove_backup_task(&active_backup_tasks_for_spawn, repo_id, task_id).await;
        });
        let abort_handle = handle.abort_handle();
        self.task_registry.register(handle);

        Self::push_backup_task(
            &active_backup_tasks,
            repo_id,
            ActiveBackupTask {
                task_id,
                abort_handle,
            },
        )
        .await;
    }

    pub(super) async fn handle_cancel_backup(
        &self,
        repo_id: RepoId,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let tasks = Self::take_backup_tasks(&self.active_backup_tasks, repo_id).await;
        let Some(tasks) = tasks else {
            warn!(repo_id = ?repo_id, "cancel requested but no active backup task found");
            return;
        };

        let task_count = tasks.len();
        for task in tasks {
            task.abort_handle.abort();
        }

        info!(repo_id = ?repo_id, task_count, "backup cancelled");
        let msg = AgentToServer::BackupCancelled { repo_id };
        if let Err(e) = outbound_tx.send(msg).await {
            tracing::debug!(error = %e, "outbound send failed");
        }
    }

    pub(super) async fn handle_run_check(
        &self,
        repo_id: RepoId,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting check");
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config, rejecting check");
            return;
        };

        let target =
            backup_target_from_repo(repo, &config.agent_hostname, None, &config.vm_snapshot);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg check");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_check_task(
                repo_id,
                &target,
                &hostname,
                &server_url,
                &token,
                &engine,
                &outbound,
            )
            .await;
        });
        self.task_registry.register(handle);
    }

    pub(super) async fn handle_run_verify(
        &self,
        repo_id: RepoId,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting verify");
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config, rejecting verify");
            return;
        };

        let target =
            backup_target_from_repo(repo, &config.agent_hostname, None, &config.vm_snapshot);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg verify");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_verify_task(
                repo_id,
                &target,
                &hostname,
                &server_url,
                &token,
                &engine,
                &outbound,
            )
            .await;
        });
        self.task_registry.register(handle);
    }
}
