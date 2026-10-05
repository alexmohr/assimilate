// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    protocol::AgentToServer,
    types::{RepoId, build_repo_url},
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{
    DryRunTaskParams, Executor, InitRepoParams, RepoOperationKey, RestoreFilesParams,
    RestoreTaskParams,
    archives::{run_delete_archives_task, run_restore_task},
    dry_run::run_dry_run_task,
    maintenance::run_init_repo_task,
    queue::ConfigLookupError,
    send_operation_failed, send_outbound,
    transport::backup_target_from_repo,
};
use crate::backup::BackupTarget;

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
            send_outbound(&outbound, msg).await;
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
            send_operation_failed(
                outbound_tx,
                request_id,
                "agent has no config yet".to_owned(),
            )
            .await;
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config for dry-run");
            send_operation_failed(
                outbound_tx,
                request_id,
                "repo not found in agent config".to_owned(),
            )
            .await;
            return;
        };

        let schedule = repo
            .schedules
            .iter()
            .find(|s| s.id == schedule_id)
            .or_else(|| repo.schedules.first());

        let Some(schedule) = schedule else {
            warn!(repo_id = ?repo_id, "no schedule found for dry-run");
            send_operation_failed(
                outbound_tx,
                request_id,
                "no schedule configured for this repo".to_owned(),
            )
            .await;
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

        let ctx = self.owned_task_context(hostname, outbound_tx);

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg dry-run");

        self.spawn_queued(repo_id, repo_key, async move {
            run_dry_run_task(
                DryRunTaskParams {
                    repo_id,
                    target,
                    backup_sources,
                    exclude_patterns,
                    include_patterns,
                    request_id,
                },
                ctx.borrowed(),
                &ctx.borg,
            )
            .await;
        })
        .await;
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

        let Some((target, hostname)) = self
            .request_target(repo_id, &request_id, "restore", outbound_tx)
            .await
        else {
            return;
        };
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let ctx = self.owned_task_context(hostname, outbound_tx);

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg restore");

        self.spawn_queued(repo_id, repo_key, async move {
            run_restore_task(
                RestoreTaskParams {
                    repo_id,
                    target,
                    archive_name,
                    paths,
                    target_path,
                    request_id,
                },
                ctx.borrowed(),
                &ctx.borg,
            )
            .await;
        })
        .await;
    }

    pub(super) async fn handle_delete_archives(
        &self,
        repo_id: RepoId,
        archive_names: Vec<String>,
        request_id: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let Some((target, hostname)) = self
            .request_target(repo_id, &request_id, "delete archives", outbound_tx)
            .await
        else {
            return;
        };
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let ctx = self.owned_task_context(hostname, outbound_tx);

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg delete");

        self.spawn_queued(repo_id, repo_key, async move {
            run_delete_archives_task(
                target,
                archive_names,
                (&ctx.hostname, &ctx.server_url, &ctx.token),
                request_id,
                &ctx.borg,
                &ctx.outbound_tx,
            )
            .await;
        })
        .await;
    }

    /// [`Self::configured_target`] for a request the server is waiting on: a
    /// lookup failure is reported back as `OperationFailed` for `request_id`.
    async fn request_target(
        &self,
        repo_id: RepoId,
        request_id: &str,
        operation: &str,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) -> Option<(BackupTarget, String)> {
        let error = match self.configured_target(repo_id).await {
            Ok(found) => return Some(found),
            Err(ConfigLookupError::NoConfig) => {
                warn!(repo_id = ?repo_id, "no config available for {operation}");
                "agent has no config yet"
            }
            Err(ConfigLookupError::UnknownRepo) => {
                warn!(repo_id = ?repo_id, "repo not found in config for {operation}");
                "repo not found in agent config"
            }
        };
        send_operation_failed(outbound_tx, request_id.to_owned(), error.to_owned()).await;
        None
    }
}
