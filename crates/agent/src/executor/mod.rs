// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use shared::{
    protocol::AgentToServer,
    task_registry::TaskRegistry,
    types::{AgentConfig, BorgEncryption, RepoId, build_repo_url},
};
use tokio::{
    sync::{Mutex, Semaphore, mpsc},
    task::AbortHandle,
};
use tracing::info;

use crate::backup::{BackupEngine, BackupTarget};

mod archives;
mod backup_handlers;
mod backup_task;
mod command;
mod dry_run;
mod maintenance;
mod repo_handlers;
mod transport;
mod vms;

pub use self::command::ExecutorCommand;

pub struct Executor {
    server_url: String,
    token: String,
    repo_operation_queues: Arc<Mutex<HashMap<RepoOperationKey, Arc<Semaphore>>>>,
    active_backup_tasks: Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
    next_task_id: AtomicU64,
    current_config: Arc<Mutex<Option<AgentConfig>>>,
    engine: Arc<BackupEngine>,
    /// Where every queued-operation task (and, transitively via `Borg`, each borg child's
    /// SIGKILL-escalation reaper) registers its `JoinHandle` so shutdown can join them
    /// instead of silently dropping whatever is still in flight when the process exits.
    task_registry: TaskRegistry,
}

impl Executor {
    pub fn new(server_url: &str, token: &str, task_registry: TaskRegistry) -> Self {
        Self {
            server_url: server_url.to_owned(),
            token: token.to_owned(),
            repo_operation_queues: Arc::new(Mutex::new(HashMap::new())),
            active_backup_tasks: Arc::new(Mutex::new(HashMap::new())),
            next_task_id: AtomicU64::new(0),
            current_config: Arc::new(Mutex::new(None)),
            engine: Arc::new(BackupEngine::new(task_registry.clone())),
            task_registry,
        }
    }

    pub async fn run(
        self,
        mut cmd_rx: mpsc::Receiver<ExecutorCommand>,
        outbound_tx: mpsc::Sender<AgentToServer>,
    ) {
        loop {
            let Some(cmd) = cmd_rx.recv().await else {
                break;
            };

            match cmd {
                ExecutorCommand::UpdateConfig(config) => {
                    info!("Config updated: {} repos configured", config.repos.len());
                    *self.current_config.lock().await = Some(config);
                }
                ExecutorCommand::RunNow {
                    repo_id,
                    schedule_id,
                    run_id,
                } => {
                    self.handle_run_now(repo_id, schedule_id, run_id, &outbound_tx)
                        .await;
                }
                ExecutorCommand::ScanVms { request_id } => {
                    self.handle_scan_vms(request_id, &outbound_tx).await;
                }
                ExecutorCommand::BuildVm {
                    request_id,
                    request,
                } => {
                    self.handle_build_vm(request_id, &request, &outbound_tx)
                        .await;
                }
                ExecutorCommand::StageVm { request_id, domain } => {
                    self.handle_stage_vm(request_id, domain, &outbound_tx).await;
                }
                ExecutorCommand::RunCheckNow { repo_id } => {
                    self.handle_run_check(repo_id, &outbound_tx).await;
                }
                ExecutorCommand::RunVerifyNow { repo_id } => {
                    self.handle_run_verify(repo_id, &outbound_tx).await;
                }
                ExecutorCommand::InitRepo {
                    repo_path,
                    ssh_user,
                    ssh_host,
                    ssh_port,
                    passphrase,
                    encryption,
                } => {
                    self.handle_init_repo(
                        InitRepoParams {
                            repo_path: &repo_path,
                            ssh_user: &ssh_user,
                            ssh_host: &ssh_host,
                            ssh_port,
                            passphrase: &passphrase,
                            encryption,
                        },
                        &outbound_tx,
                    )
                    .await;
                }
                ExecutorCommand::DryRun {
                    repo_id,
                    schedule_id,
                    request_id,
                } => {
                    self.handle_dry_run(repo_id, schedule_id, request_id, &outbound_tx)
                        .await;
                }
                ExecutorCommand::RestoreFiles {
                    repo_id,
                    archive_name,
                    paths,
                    target_path,
                    request_id,
                } => {
                    self.handle_restore_files(
                        RestoreFilesParams {
                            repo_id,
                            archive_name,
                            paths,
                            target_path,
                            request_id,
                        },
                        &outbound_tx,
                    )
                    .await;
                }
                ExecutorCommand::DeleteArchives {
                    repo_id,
                    archive_names,
                    request_id,
                } => {
                    self.handle_delete_archives(repo_id, archive_names, request_id, &outbound_tx)
                        .await;
                }
                ExecutorCommand::CancelBackup { repo_id } => {
                    self.handle_cancel_backup(repo_id, &outbound_tx).await;
                }
            }
        }
    }

    async fn repo_operation_queue(&self, repo_key: &RepoOperationKey) -> Arc<Semaphore> {
        let mut repo_operation_queues = self.repo_operation_queues.lock().await;
        Arc::clone(
            repo_operation_queues
                .entry(repo_key.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    fn next_task_id(&self) -> u64 {
        self.next_task_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn push_backup_task(
        active_backup_tasks: &Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
        repo_id: RepoId,
        task: ActiveBackupTask,
    ) {
        let mut active_backup_tasks = active_backup_tasks.lock().await;
        active_backup_tasks.entry(repo_id).or_default().push(task);
    }

    async fn take_backup_tasks(
        active_backup_tasks: &Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
        repo_id: RepoId,
    ) -> Option<Vec<ActiveBackupTask>> {
        active_backup_tasks.lock().await.remove(&repo_id)
    }

    async fn remove_backup_task(
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RepoOperationKey {
    ssh_user: String,
    ssh_host: String,
    ssh_port: u16,
    repo_path: String,
}

impl RepoOperationKey {
    fn from_backup_target(target: &BackupTarget) -> Self {
        Self {
            ssh_user: target.ssh_user.clone(),
            ssh_host: target.ssh_host.clone(),
            ssh_port: target.ssh_port,
            repo_path: target.repo_path.clone(),
        }
    }

    fn repo_url(&self) -> String {
        build_repo_url(
            &self.ssh_user,
            &self.ssh_host,
            self.ssh_port,
            &self.repo_path,
        )
    }
}

struct ActiveBackupTask {
    task_id: u64,
    abort_handle: AbortHandle,
}

struct BackupTaskContext {
    hostname: String,
    server_url: String,
    token: String,
    run_id: Option<String>,
}

struct InitRepoParams<'a> {
    repo_path: &'a str,
    ssh_user: &'a str,
    ssh_host: &'a str,
    ssh_port: u16,
    passphrase: &'a str,
    encryption: BorgEncryption,
}

struct RestoreFilesParams {
    repo_id: RepoId,
    archive_name: String,
    paths: Vec<String>,
    target_path: String,
    request_id: String,
}

struct FreeTaskContext<'a> {
    hostname: &'a str,
    server_url: &'a str,
    token: &'a str,
    outbound_tx: &'a mpsc::Sender<AgentToServer>,
}

struct DryRunTaskParams {
    repo_id: RepoId,
    target: BackupTarget,
    backup_sources: Vec<String>,
    exclude_patterns: Vec<String>,
    include_patterns: Vec<String>,
    request_id: String,
}

struct RestoreTaskParams {
    repo_id: RepoId,
    target: BackupTarget,
    archive_name: String,
    paths: Vec<String>,
    target_path: String,
    request_id: String,
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    reason = "test-only assertions on known fixtures"
)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup/assertions"
)]
mod tests;
