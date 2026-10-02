// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    protocol::AgentToServer,
    types::{RepoId, build_repo_url},
};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::{
    DryRunTaskParams, Executor, FreeTaskContext, InitRepoParams, RepoOperationKey,
    RestoreFilesParams, RestoreTaskParams,
    archives::{run_delete_archives_task, run_restore_task},
    dry_run::run_dry_run_task,
    maintenance::run_init_repo_task,
    transport::backup_target_from_repo,
};
use crate::borg::Borg;

impl Executor {
    pub(super) async fn handle_init_repo(
        &self,
        params: InitRepoParams<'_>,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let InitRepoParams {
            repo_path,
            ssh_user,
            ssh_host,
            ssh_port,
            passphrase,
            encryption,
        } = params;

        let hostname = {
            let config_guard = self.current_config.lock().await;
            config_guard
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), |c| c.agent_hostname.clone())
        };

        let repo_url = build_repo_url(ssh_user, ssh_host, ssh_port, repo_path);
        info!(repo_url = %repo_url, "initializing repository");

        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let outbound = outbound_tx.clone();
        let repo_path_owned = repo_path.to_owned();
        let passphrase_owned = passphrase.to_owned();
        let task_registry = self.task_registry.clone();

        let handle = tokio::spawn(async move {
            let result = run_init_repo_task(
                &repo_url,
                &passphrase_owned,
                encryption,
                &hostname,
                &server_url,
                &token,
                task_registry,
            )
            .await;

            let (success, error_message) = match result {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e)),
            };

            let msg = AgentToServer::InitRepoCompleted {
                repo_path: repo_path_owned,
                success,
                error_message,
            };
            if let Err(e) = outbound.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
        });
        self.task_registry.register(handle);
    }

    pub(super) async fn handle_dry_run(
        &self,
        repo_id: RepoId,
        schedule_id: i64,
        request_id: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available for dry-run");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "agent has no config yet".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config for dry-run");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "repo not found in agent config".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let schedule = repo
            .schedules
            .iter()
            .find(|s| s.id == schedule_id)
            .or_else(|| repo.schedules.first());

        let Some(schedule) = schedule else {
            warn!(repo_id = ?repo_id, "no schedule found for dry-run");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "no schedule configured for this repo".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let backup_sources = schedule.backup_sources.clone();
        let exclude_patterns = schedule.exclude_patterns.clone();
        let include_patterns = schedule.include_patterns.clone();
        let target = backup_target_from_repo(
            repo,
            &config.agent_hostname,
            Some(schedule_id),
            &config.vm_snapshot,
        );
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new(self.task_registry.clone());

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg dry-run");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_dry_run_task(
                DryRunTaskParams {
                    repo_id,
                    target,
                    backup_sources,
                    exclude_patterns,
                    include_patterns,
                    request_id,
                },
                FreeTaskContext {
                    hostname: &hostname,
                    server_url: &server_url,
                    token: &token,
                    outbound_tx: &outbound,
                },
                &borg,
            )
            .await;
        });
        self.task_registry.register(handle);
    }

    pub(super) async fn handle_restore_files(
        &self,
        params: RestoreFilesParams,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let RestoreFilesParams {
            repo_id,
            archive_name,
            paths,
            target_path,
            request_id,
        } = params;

        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available for restore");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "agent has no config yet".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config for restore");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "repo not found in agent config".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let target =
            backup_target_from_repo(repo, &config.agent_hostname, None, &config.vm_snapshot);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new(self.task_registry.clone());

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg restore");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_restore_task(
                RestoreTaskParams {
                    repo_id,
                    target,
                    archive_name,
                    paths,
                    target_path,
                    request_id,
                },
                FreeTaskContext {
                    hostname: &hostname,
                    server_url: &server_url,
                    token: &token,
                    outbound_tx: &outbound,
                },
                &borg,
            )
            .await;
        });
        self.task_registry.register(handle);
    }

    pub(super) async fn handle_delete_archives(
        &self,
        repo_id: RepoId,
        archive_names: Vec<String>,
        request_id: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available for delete archives");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "agent has no config yet".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config for delete archives");
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "repo not found in agent config".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let target =
            backup_target_from_repo(repo, &config.agent_hostname, None, &config.vm_snapshot);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.agent_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new(self.task_registry.clone());

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg delete");

        let handle = tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_delete_archives_task(
                target,
                archive_names,
                (&hostname, &server_url, &token),
                request_id,
                &borg,
                &outbound,
            )
            .await;
        });
        self.task_registry.register(handle);
    }
}
