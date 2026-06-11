// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use shared::{
    protocol::AgentToServer,
    ssh::{borg_rsh, borg_rsh_with_known_hosts, known_hosts_host},
    types::{
        AgentConfig, BorgEncryption, Compression, DryRunFile, RepoConfig, RepoId, build_repo_url,
    },
};
use tokio::{
    sync::{Mutex, Semaphore, mpsc},
    task::JoinHandle,
};
use tracing::{error, info, warn};

use crate::{
    backup::{BackupEngine, BackupError, BackupTarget, CanaryToken},
    borg::Borg,
    ssh_forward::{SshForwardError, SshForwardSocket, run_ssh_forward},
};

pub enum ExecutorCommand {
    UpdateConfig(AgentConfig),
    RunNow {
        repo_id: RepoId,
        schedule_id: Option<i64>,
        run_id: Option<String>,
    },
    CancelBackup {
        repo_id: RepoId,
    },
    RunCheckNow {
        repo_id: RepoId,
    },
    RunVerifyNow {
        repo_id: RepoId,
    },
    InitRepo {
        repo_path: String,
        ssh_user: String,
        ssh_host: String,
        ssh_port: u16,
        passphrase: String,
        encryption: BorgEncryption,
    },
    DryRun {
        repo_id: RepoId,
        schedule_id: i64,
        request_id: String,
    },
    RestoreFiles {
        repo_id: RepoId,
        archive_name: String,
        paths: Vec<String>,
        target_path: String,
        request_id: String,
    },
    DeleteArchives {
        repo_id: RepoId,
        archive_names: Vec<String>,
        request_id: String,
    },
}

pub struct Executor {
    server_url: String,
    token: String,
    repo_operation_queues: Arc<Mutex<HashMap<RepoOperationKey, Arc<Semaphore>>>>,
    active_backup_tasks: Arc<Mutex<HashMap<RepoId, Vec<ActiveBackupTask>>>>,
    next_task_id: AtomicU64,
    current_config: Arc<Mutex<Option<AgentConfig>>>,
    engine: Arc<BackupEngine>,
}

impl Executor {
    pub fn new(server_url: &str, token: &str) -> Self {
        Self {
            server_url: server_url.to_owned(),
            token: token.to_owned(),
            repo_operation_queues: Arc::new(Mutex::new(HashMap::new())),
            active_backup_tasks: Arc::new(Mutex::new(HashMap::new())),
            next_task_id: AtomicU64::new(0),
            current_config: Arc::new(Mutex::new(None)),
            engine: Arc::new(BackupEngine::new()),
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
                        &repo_path,
                        &ssh_user,
                        &ssh_host,
                        ssh_port,
                        &passphrase,
                        encryption,
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
                        repo_id,
                        archive_name,
                        paths,
                        target_path,
                        request_id,
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

    async fn handle_run_now(
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

        let target = backup_target_from_repo(repo, &config.client_hostname, schedule_id);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
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

        Self::push_backup_task(
            &active_backup_tasks,
            repo_id,
            ActiveBackupTask { task_id, handle },
        )
        .await;
    }

    async fn handle_cancel_backup(
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
            task.handle.abort();
        }

        info!(repo_id = ?repo_id, task_count, "backup cancelled");
        let msg = AgentToServer::BackupCancelled { repo_id };
        if let Err(e) = outbound_tx.send(msg).await {
            tracing::debug!(error = %e, "outbound send failed");
        }
    }

    async fn handle_run_check(&self, repo_id: RepoId, outbound_tx: &mpsc::Sender<AgentToServer>) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting check");
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config, rejecting check");
            return;
        };

        let target = backup_target_from_repo(repo, &config.client_hostname, None);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg check");

        tokio::spawn(async move {
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
    }

    async fn handle_run_verify(&self, repo_id: RepoId, outbound_tx: &mpsc::Sender<AgentToServer>) {
        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting verify");
            return;
        };

        let Some(repo) = config.repos.iter().find(|r| r.repo_id == repo_id) else {
            warn!(repo_id = ?repo_id, "repo not found in config, rejecting verify");
            return;
        };

        let target = backup_target_from_repo(repo, &config.client_hostname, None);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg verify");

        tokio::spawn(async move {
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
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_init_repo(
        &self,
        repo_path: &str,
        ssh_user: &str,
        ssh_host: &str,
        ssh_port: u16,
        passphrase: &str,
        encryption: BorgEncryption,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let hostname = {
            let config_guard = self.current_config.lock().await;
            config_guard
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), |c| c.client_hostname.clone())
        };

        let repo_url = build_repo_url(ssh_user, ssh_host, ssh_port, repo_path);
        info!(repo_url = %repo_url, "initializing repository");

        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let outbound = outbound_tx.clone();
        let repo_path_owned = repo_path.to_owned();
        let passphrase_owned = passphrase.to_owned();

        tokio::spawn(async move {
            let result = run_init_repo_task(
                &repo_url,
                &passphrase_owned,
                encryption,
                &hostname,
                &server_url,
                &token,
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
    }

    async fn handle_dry_run(
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
        let target = backup_target_from_repo(repo, &config.client_hostname, Some(schedule_id));
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg dry-run");

        tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_dry_run_task(
                repo_id,
                target,
                backup_sources,
                exclude_patterns,
                &hostname,
                &server_url,
                &token,
                request_id,
                &borg,
                &outbound,
            )
            .await;
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_restore_files(
        &self,
        repo_id: RepoId,
        archive_name: String,
        paths: Vec<String>,
        target_path: String,
        request_id: String,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
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

        let target = backup_target_from_repo(repo, &config.client_hostname, None);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg restore");

        tokio::spawn(async move {
            let Ok(_permit) = repo_queue.acquire_owned().await else {
                error!(
                    repo_id = ?repo_id,
                    repo = %repo_key.repo_url(),
                    "failed to acquire repo queue"
                );
                return;
            };

            run_restore_task(
                repo_id,
                target,
                archive_name,
                paths,
                target_path,
                &hostname,
                &server_url,
                &token,
                request_id,
                &borg,
                &outbound,
            )
            .await;
        });
    }

    async fn handle_delete_archives(
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

        let target = backup_target_from_repo(repo, &config.client_hostname, None);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let repo_queue = self.repo_operation_queue(&repo_key).await;
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let borg = Borg::new();

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued borg delete");

        tokio::spawn(async move {
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
    handle: JoinHandle<()>,
}

async fn run_init_repo_task(
    repo_url: &str,
    passphrase: &str,
    encryption: BorgEncryption,
    hostname: &str,
    server_url: &str,
    token: &str,
) -> Result<(), String> {
    let mut ssh_forward_target = BackupTarget {
        target_name: String::new(),
        schedule_id: None,
        repo_path: String::new(),
        ssh_user: String::new(),
        ssh_host: String::new(),
        ssh_port: 22,
        ssh_host_key: String::new(),
        known_hosts_path: None,
        passphrase: String::new(),
        hostname: hostname.to_owned(),
        compression: Compression::Lz4,
        backup_sources: Vec::new(),
        rate_limit_kbps: None,
        keep_hourly: 0,
        keep_daily: 0,
        keep_weekly: 0,
        keep_monthly: 0,
        keep_yearly: 0,
        compact_enabled: false,
        pre_backup_commands: Vec::new(),
        post_backup_commands: Vec::new(),
        skip_targets: Vec::new(),
        exclude_patterns: Vec::new(),
        ssh_auth_sock: None,
        canary_enabled: false,
        accept_relocation: false,
    };

    let _ssh_forward =
        setup_ssh_forward(&mut ssh_forward_target, hostname, server_url, token).await;

    let mut env = vec![
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        ("BORG_DISPLAY_PASSPHRASE".to_owned(), "no".to_owned()),
        (
            "BORG_RSH".to_owned(),
            borg_rsh_for_target(&ssh_forward_target),
        ),
        ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
        ("LC_CTYPE".to_owned(), "en_US.UTF-8".to_owned()),
    ];
    if let Some(sock) = &ssh_forward_target.ssh_auth_sock {
        env.push((
            "SSH_AUTH_SOCK".to_owned(),
            sock.to_string_lossy().into_owned(),
        ));
    }

    let borg = Borg::new();
    let init_args = ["init", "--encryption", encryption.as_borg_arg(), repo_url];
    let output = tokio::time::timeout(Duration::from_mins(2), borg.run(&init_args, &env))
        .await
        .map_err(|_| "borg init timed out after 120 seconds".to_owned())?
        .map_err(|e| format!("failed to execute borg: {e}"))?;

    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(exit_code, stderr = %stderr, "borg init failed on agent");
        return Err(format!("borg init failed (exit {exit_code}): {stderr}"));
    }

    info!(repo_url = %repo_url, "repository initialized successfully");
    Ok(())
}

fn borg_rsh_for_target(target: &BackupTarget) -> String {
    target.known_hosts_path.as_ref().map_or_else(
        || {
            if target.ssh_host_key.is_empty() {
                borg_rsh()
            } else {
                "false".to_owned()
            }
        },
        |path| borg_rsh_with_known_hosts(path),
    )
}

fn make_failed_report(
    repo_id: RepoId,
    schedule_id: Option<i64>,
    started_at: chrono::DateTime<Utc>,
    error_message: String,
    run_id: Option<String>,
) -> shared::types::BackupReport {
    let finished_at = Utc::now();
    shared::types::BackupReport {
        id: shared::types::ReportId(0),
        client_id: shared::types::ClientId(0),
        repo_id,
        schedule_id,
        started_at,
        finished_at,
        status: shared::types::BackupStatus::Failed,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: finished_at.signed_duration_since(started_at).num_seconds(),
        error_message: Some(error_message),
        warnings: Vec::new(),
        borg_version: None,
        archive_name: None,
        borg_command: None,
        run_id,
        detected_relocation: false,
    }
}

struct BackupTaskContext {
    hostname: String,
    server_url: String,
    token: String,
    run_id: Option<String>,
}

async fn run_backup_task(
    repo_id: RepoId,
    mut target: BackupTarget,
    ctx: BackupTaskContext,
    engine: &BackupEngine,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let BackupTaskContext {
        hostname,
        server_url,
        token,
        run_id,
    } = ctx;
    let started_at = Utc::now();
    let schedule_id = target.schedule_id;
    let borg_command = BackupEngine::preview_create_command(&target);
    let started_msg = AgentToServer::BackupStarted {
        repo_id,
        schedule_id,
        started_at,
        borg_command: Some(borg_command),
        run_id: run_id.clone(),
    };
    if let Err(e) = outbound_tx.send(started_msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }

    let _ssh_forward = setup_ssh_forward(&mut target, &hostname, &server_url, &token).await;

    let canary = if target.canary_enabled {
        match BackupEngine::write_canary(&target.backup_sources) {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(repo_id = ?repo_id, error = %e, "canary write failed, proceeding without");
                None
            }
        }
    } else {
        None
    };

    let (report, run_canary) = match engine.run_backup(&target).await {
        Ok(result) => {
            let finished_at = Utc::now();
            let report = shared::types::BackupReport {
                id: shared::types::ReportId(0),
                client_id: shared::types::ClientId(0),
                repo_id,
                schedule_id,
                started_at,
                finished_at,
                status: result.status,
                original_size: result.original_size,
                compressed_size: result.compressed_size,
                deduplicated_size: result.deduplicated_size,
                repo_unique_csize: result.repo_unique_csize,
                files_processed: result.files_processed,
                duration_secs: result.duration_secs,
                error_message: result.error_message,
                warnings: result.warnings,
                borg_version: None,
                archive_name: result.archive_name,
                borg_command: result.borg_command,
                run_id: run_id.clone(),
                detected_relocation: false,
            };
            (report, true)
        }
        Err(BackupError::Skipped(reason)) => {
            error!(repo_id = ?repo_id, reason = %reason, "backup skipped, treating as failure");
            (
                make_failed_report(repo_id, schedule_id, started_at, reason, run_id.clone()),
                false,
            )
        }
        Err(BackupError::BorgRelocated(msg)) => {
            error!(repo_id = ?repo_id, error = %msg, "backup failed: repository relocation detected, will auto-accept on next run");
            let mut report =
                make_failed_report(repo_id, schedule_id, started_at, msg, run_id.clone());
            report.detected_relocation = true;
            (report, false)
        }
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "backup failed");
            (
                make_failed_report(
                    repo_id,
                    schedule_id,
                    started_at,
                    e.to_string(),
                    run_id.clone(),
                ),
                false,
            )
        }
    };

    let msg = AgentToServer::BackupCompleted { report };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }

    if let Some(canary) = &canary {
        if run_canary {
            run_canary_verification(repo_id, &target, engine, canary, outbound_tx).await;
        }
        BackupEngine::cleanup_canary(canary);
    }
}

struct RepoTransport {
    _ssh_forward: Option<SshForwardSocket>,
    _known_hosts: Option<tempfile::NamedTempFile>,
}

async fn setup_ssh_forward(
    target: &mut BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
) -> RepoTransport {
    let known_hosts = match write_known_hosts(target) {
        Ok(known_hosts) => known_hosts,
        Err(e) => {
            error!(error = %e, "failed to create pinned SSH known_hosts file");
            None
        }
    };

    let socket = match SshForwardSocket::create() {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "ssh forward: failed to create socket, proceeding without");
            return RepoTransport {
                _ssh_forward: None,
                _known_hosts: known_hosts,
            };
        }
    };

    if let Err(e) = run_ssh_forward(&socket, server_url, hostname, token).await {
        match e {
            SshForwardError::Url(msg) => warn!(error = %msg, "ssh forward: bad relay url"),
            other => warn!(error = %other, "ssh forward: setup failed, proceeding without"),
        }
        return RepoTransport {
            _ssh_forward: None,
            _known_hosts: known_hosts,
        };
    }

    target.ssh_auth_sock = Some(socket.socket_path.clone());
    RepoTransport {
        _ssh_forward: Some(socket),
        _known_hosts: known_hosts,
    }
}

fn write_known_hosts(
    target: &mut BackupTarget,
) -> Result<Option<tempfile::NamedTempFile>, std::io::Error> {
    if target.ssh_host_key.is_empty() {
        return Ok(None);
    }

    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(
        file,
        "{} {}",
        known_hosts_host(&target.ssh_host, target.ssh_port),
        target.ssh_host_key
    )?;
    file.flush()?;
    target.known_hosts_path = Some(file.path().to_path_buf());
    Ok(Some(file))
}

async fn run_canary_verification(
    repo_id: RepoId,
    target: &BackupTarget,
    engine: &BackupEngine,
    canary: &CanaryToken,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let (success, archive_name, error_message) = match engine.verify_canary(target, canary).await {
        Ok(archive) => (true, archive, None),
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "canary verification failed");
            (false, String::new(), Some(e.to_string()))
        }
    };

    let msg = AgentToServer::CanaryVerified {
        repo_id,
        success,
        nonce: canary.nonce.clone(),
        archive_name,
        error_message,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

pub fn backup_target_from_repo(
    repo: &RepoConfig,
    hostname: &str,
    schedule_id: Option<i64>,
) -> BackupTarget {
    let schedule = schedule_id
        .and_then(|id| repo.schedules.iter().find(|s| s.id == id))
        .or_else(|| repo.schedules.first());
    BackupTarget {
        target_name: repo.name.clone(),
        schedule_id: schedule.map(|s| s.id),
        repo_path: repo.repo_path.clone(),
        ssh_user: repo.ssh_user.clone(),
        ssh_host: repo.ssh_host.clone(),
        ssh_port: repo.ssh_port,
        ssh_host_key: repo.ssh_host_key.clone(),
        known_hosts_path: None,
        passphrase: repo.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: repo.compression.clone(),
        backup_sources: schedule.map_or_else(Vec::new, |s| s.backup_sources.clone()),
        rate_limit_kbps: schedule.and_then(|s| s.rate_limit_kbps),
        keep_hourly: schedule.map_or(24, |s| s.keep_hourly),
        keep_daily: schedule.map_or(7, |s| s.keep_daily),
        keep_weekly: schedule.map_or(4, |s| s.keep_weekly),
        keep_monthly: schedule.map_or(6, |s| s.keep_monthly),
        keep_yearly: schedule.map_or(0, |s| s.keep_yearly),
        compact_enabled: schedule.is_none_or(|s| s.compact_enabled),
        pre_backup_commands: schedule.map_or_else(Vec::new, |s| s.pre_backup_commands.clone()),
        post_backup_commands: schedule.map_or_else(Vec::new, |s| s.post_backup_commands.clone()),
        skip_targets: Vec::new(),
        exclude_patterns: schedule.map_or_else(Vec::new, |s| s.exclude_patterns.clone()),
        ssh_auth_sock: None,
        canary_enabled: schedule.is_some_and(|s| s.canary_enabled),
        accept_relocation: repo.accept_relocation,
    }
}

async fn run_check_task(
    repo_id: RepoId,
    target: &BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
    engine: &BackupEngine,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let start = std::time::Instant::now();
    let mut target = BackupTarget {
        target_name: target.target_name.clone(),
        schedule_id: target.schedule_id,
        repo_path: target.repo_path.clone(),
        ssh_user: target.ssh_user.clone(),
        ssh_host: target.ssh_host.clone(),
        ssh_port: target.ssh_port,
        ssh_host_key: target.ssh_host_key.clone(),
        known_hosts_path: None,
        passphrase: target.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: target.compression.clone(),
        backup_sources: Vec::new(),
        rate_limit_kbps: target.rate_limit_kbps,
        keep_hourly: 0,
        keep_daily: 0,
        keep_weekly: 0,
        keep_monthly: 0,
        keep_yearly: 0,
        compact_enabled: false,
        pre_backup_commands: Vec::new(),
        post_backup_commands: Vec::new(),
        skip_targets: Vec::new(),
        exclude_patterns: Vec::new(),
        ssh_auth_sock: None,
        canary_enabled: false,
        accept_relocation: target.accept_relocation,
    };

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let result = engine.run_check(&target).await;
    let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

    let (success, error_message) = match result {
        Ok(()) => (true, None),
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "check failed");
            (false, Some(e.to_string()))
        }
    };

    let msg = AgentToServer::CheckCompleted {
        repo_id,
        success,
        duration_secs,
        error_message,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

async fn run_verify_task(
    repo_id: RepoId,
    target: &BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
    engine: &BackupEngine,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let start = std::time::Instant::now();
    let mut target = BackupTarget {
        target_name: target.target_name.clone(),
        schedule_id: target.schedule_id,
        repo_path: target.repo_path.clone(),
        ssh_user: target.ssh_user.clone(),
        ssh_host: target.ssh_host.clone(),
        ssh_port: target.ssh_port,
        ssh_host_key: target.ssh_host_key.clone(),
        known_hosts_path: None,
        passphrase: target.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: target.compression.clone(),
        backup_sources: Vec::new(),
        rate_limit_kbps: target.rate_limit_kbps,
        keep_hourly: 0,
        keep_daily: 0,
        keep_weekly: 0,
        keep_monthly: 0,
        keep_yearly: 0,
        compact_enabled: false,
        pre_backup_commands: Vec::new(),
        post_backup_commands: Vec::new(),
        skip_targets: Vec::new(),
        exclude_patterns: Vec::new(),
        ssh_auth_sock: None,
        canary_enabled: false,
        accept_relocation: target.accept_relocation,
    };

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let result = engine.run_verify(&target).await;
    let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

    let (success, error_message, files_verified) = match result {
        Ok(count) => (true, None, count),
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "verify failed");
            (false, Some(e.to_string()), 0)
        }
    };

    let msg = AgentToServer::VerifyCompleted {
        repo_id,
        success,
        duration_secs,
        error_message,
        files_verified,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_dry_run_task(
    repo_id: RepoId,
    mut target: BackupTarget,
    backup_sources: Vec<String>,
    exclude_patterns: Vec<String>,
    hostname: &str,
    server_url: &str,
    token: &str,
    request_id: String,
    borg: &Borg,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let exclude_file = match write_temp_excludes(&exclude_patterns) {
        Ok(f) => f,
        Err(e) => {
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: format!("failed to write exclude file: {e}"),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return;
        }
    };

    let timestamp = Utc::now().timestamp();
    let archive_spec = format!("::{hostname}-dryrun-{timestamp}");

    let env_vars = build_borg_env(&target);

    let mut args = vec![
        "create".to_owned(),
        "--dry-run".to_owned(),
        "--list".to_owned(),
        "--log-json".to_owned(),
        "--exclude-from".to_owned(),
        exclude_file.path().to_string_lossy().into_owned(),
        archive_spec,
    ];
    args.extend(backup_sources.iter().cloned());

    info!(repo_id = ?repo_id, "running borg create --dry-run");

    let output =
        match tokio::time::timeout(Duration::from_mins(10), borg.run(&args, &env_vars)).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let msg = AgentToServer::OperationFailed {
                    request_id,
                    error: format!("failed to execute borg: {e}"),
                };
                if let Err(send_err) = outbound_tx.send(msg).await {
                    tracing::debug!(error = %send_err, "outbound send failed");
                }
                return;
            }
            Err(_) => {
                let msg = AgentToServer::OperationFailed {
                    request_id,
                    error: "borg dry-run timed out".to_owned(),
                };
                if let Err(send_err) = outbound_tx.send(msg).await {
                    tracing::debug!(error = %send_err, "outbound send failed");
                }
                return;
            }
        };

    let exit_code = output.status.code().unwrap_or(-1);
    if exit_code != 0 && exit_code != 1 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(repo_id = ?repo_id, exit_code, stderr = %stderr, "borg dry-run failed");
        let msg = AgentToServer::OperationFailed {
            request_id,
            error: format!("borg dry-run failed (exit {exit_code}): {stderr}"),
        };
        if let Err(send_err) = outbound_tx.send(msg).await {
            tracing::debug!(error = %send_err, "outbound send failed");
        }
        return;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let (files, total_size) = parse_dry_run_output(&stderr);

    info!(repo_id = ?repo_id, file_count = files.len(), total_size, "borg dry-run completed");

    let msg = AgentToServer::DryRunResult {
        request_id,
        files,
        total_size,
        error_message: None,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_restore_task(
    repo_id: RepoId,
    mut target: BackupTarget,
    archive_name: String,
    paths: Vec<String>,
    target_path: String,
    hostname: &str,
    server_url: &str,
    token: &str,
    request_id: String,
    borg: &Borg,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let env_vars = build_borg_env(&target);

    let args = restore_args(&archive_name, &paths);
    let target_path = std::path::PathBuf::from(target_path);
    if let Err(e) = tokio::fs::create_dir_all(&target_path).await {
        let msg = AgentToServer::OperationFailed {
            request_id,
            error: format!("failed to create restore directory: {e}"),
        };
        if let Err(send_err) = outbound_tx.send(msg).await {
            tracing::debug!(error = %send_err, "outbound send failed");
        }
        return;
    }

    info!(repo_id = ?repo_id, archive = %archive_name, "running borg extract");

    let output = match tokio::time::timeout(
        Duration::from_mins(30),
        borg.run_in_dir(&args, &env_vars, &target_path),
    )
    .await
    {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: format!("failed to execute borg: {e}"),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return;
        }
        Err(_) => {
            let msg = AgentToServer::OperationFailed {
                request_id,
                error: "borg extract timed out".to_owned(),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return;
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    if exit_code != 0 && exit_code != 1 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(repo_id = ?repo_id, exit_code, stderr = %stderr, "borg extract failed");
        let msg = AgentToServer::RestoreCompleted {
            request_id,
            success: false,
            files_restored: 0,
            error_message: Some(format!("borg extract failed (exit {exit_code}): {stderr}")),
        };
        if let Err(send_err) = outbound_tx.send(msg).await {
            tracing::debug!(error = %send_err, "outbound send failed");
        }
        return;
    }

    if exit_code == 1 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let warnings = crate::backup::parse_warnings(&stderr);
        if !warnings.is_empty() {
            warn!(repo_id = ?repo_id, "borg extract warnings: {}", warnings.join("; "));
        }
    }

    let files_restored = u64::try_from(paths.len()).unwrap_or(0);

    info!(repo_id = ?repo_id, files_restored, "borg extract completed");

    let msg = AgentToServer::RestoreCompleted {
        request_id,
        success: true,
        files_restored,
        error_message: None,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

fn restore_args(archive_name: &str, paths: &[String]) -> Vec<String> {
    let mut args = vec![
        "extract".to_owned(),
        "--log-json".to_owned(),
        format!("::{archive_name}"),
    ];
    args.extend(paths.iter().cloned());
    args
}

async fn run_delete_archives_task(
    mut target: BackupTarget,
    archive_names: Vec<String>,
    ssh_params: (&str, &str, &str),
    request_id: String,
    borg: &Borg,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let (hostname, server_url, token) = ssh_params;
    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let env_vars = build_borg_env(&target);
    let mut deleted_count: u32 = 0;

    for archive_name in &archive_names {
        let args = vec![
            "delete".to_owned(),
            "--lock-wait".to_owned(),
            "600".to_owned(),
            format!("::{archive_name}"),
        ];

        info!(archive = %archive_name, "running borg delete");

        let output =
            match tokio::time::timeout(Duration::from_mins(10), borg.run(&args, &env_vars)).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    let msg = AgentToServer::DeleteArchivesResult {
                        request_id,
                        success: false,
                        deleted_count,
                        error_message: Some(format!(
                            "failed to execute borg delete for {archive_name}: {e}"
                        )),
                    };
                    if let Err(send_err) = outbound_tx.send(msg).await {
                        tracing::debug!(error = %send_err, "outbound send failed");
                    }
                    return;
                }
                Err(_) => {
                    let msg = AgentToServer::DeleteArchivesResult {
                        request_id,
                        success: false,
                        deleted_count,
                        error_message: Some(format!("borg delete timed out for {archive_name}")),
                    };
                    if let Err(send_err) = outbound_tx.send(msg).await {
                        tracing::debug!(error = %send_err, "outbound send failed");
                    }
                    return;
                }
            };

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code != 0 && exit_code != 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(archive = %archive_name, exit_code, stderr = %stderr, "borg delete failed");
            let msg = AgentToServer::DeleteArchivesResult {
                request_id,
                success: false,
                deleted_count,
                error_message: Some(format!(
                    "borg delete failed for {archive_name} (exit {exit_code}): {stderr}"
                )),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return;
        }

        deleted_count = deleted_count.saturating_add(1);
    }

    info!(deleted_count, "borg delete archives completed");

    let msg = AgentToServer::DeleteArchivesResult {
        request_id,
        success: true,
        deleted_count,
        error_message: None,
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }
}

fn parse_dry_run_output(stderr: &str) -> (Vec<DryRunFile>, i64) {
    let mut files = Vec::new();
    let mut total_size: i64 = 0;

    for line in stderr.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        let event_type = value.get("type").and_then(serde_json::Value::as_str);

        match event_type {
            Some("file_status") => {
                let Some(path) = value.get("path").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                files.push(DryRunFile {
                    path: path.to_owned(),
                    size: 0,
                });
            }
            Some("archive_progress") => {
                if let Some(size) = value
                    .get("original_size")
                    .and_then(serde_json::Value::as_i64)
                {
                    total_size = size;
                }
            }
            Some(_) | None => {}
        }
    }

    (files, total_size)
}

fn write_temp_excludes(patterns: &[String]) -> Result<tempfile::NamedTempFile, std::io::Error> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new()?;
    for pattern in patterns {
        writeln!(file, "{pattern}")?;
    }
    file.flush()?;
    Ok(file)
}

fn build_borg_env(target: &BackupTarget) -> Vec<(String, String)> {
    let repo_url = build_repo_url(
        &target.ssh_user,
        &target.ssh_host,
        target.ssh_port,
        &target.repo_path,
    );

    let mut env = vec![
        ("BORG_REPO".to_owned(), repo_url),
        ("BORG_PASSPHRASE".to_owned(), target.passphrase.clone()),
        ("BORG_HOST_ID".to_owned(), target.hostname.clone()),
        ("BORG_RSH".to_owned(), borg_rsh_for_target(target)),
        ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
        ("LC_CTYPE".to_owned(), "en_US.UTF-8".to_owned()),
    ];

    if target.accept_relocation {
        env.push((
            "BORG_RELOCATED_REPO_ACCESS_IS_OK".to_owned(),
            "yes".to_owned(),
        ));
    }

    if let Some(sock) = &target.ssh_auth_sock {
        env.push((
            "SSH_AUTH_SOCK".to_owned(),
            sock.to_string_lossy().into_owned(),
        ));
    }

    env
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn restore_args_select_the_whole_archive_when_paths_are_empty() {
        assert_eq!(
            restore_args("archive-1", &[]),
            vec!["extract", "--log-json", "::archive-1"]
        );
    }

    #[test]
    fn restore_args_append_selected_paths() {
        assert_eq!(
            restore_args(
                "archive-1",
                &["etc/hosts".to_owned(), "var/lib/app".to_owned()]
            ),
            vec![
                "extract",
                "--log-json",
                "::archive-1",
                "etc/hosts",
                "var/lib/app"
            ]
        );
    }

    #[test]
    fn parse_dry_run_file_status_and_archive_progress() {
        let stderr = [
            r#"{"type": "file_status", "status": "A", "path": "/home/user/doc.txt"}"#,
            concat!(
                r#"{"type": "archive_progress", "#,
                r#""original_size": 500, "compressed_size": 300, "#,
                r#""deduplicated_size": 200, "nfiles": 1, "#,
                r#""path": "/home/user/doc.txt"}"#,
            ),
            r#"{"type": "file_status", "status": "U", "path": "/home/user/photo.jpg"}"#,
            concat!(
                r#"{"type": "archive_progress", "#,
                r#""original_size": 10240, "compressed_size": 8000, "#,
                r#""deduplicated_size": 5000, "nfiles": 2, "#,
                r#""path": "/home/user/photo.jpg", "finished": true}"#,
            ),
        ]
        .join("\n");

        let (files, total_size) = parse_dry_run_output(&stderr);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "/home/user/doc.txt");
        assert_eq!(files[0].size, 0);
        assert_eq!(files[1].path, "/home/user/photo.jpg");
        assert_eq!(files[1].size, 0);
        assert_eq!(total_size, 10240);
    }

    #[test]
    fn parse_dry_run_file_status_only() {
        let stderr = [
            r#"{"type": "file_status", "status": "A", "path": "/etc/hostname"}"#,
            r#"{"type": "file_status", "status": "A", "path": "/etc/passwd"}"#,
        ]
        .join("\n");

        let (files, total_size) = parse_dry_run_output(&stderr);

        assert_eq!(files.len(), 2);
        assert_eq!(total_size, 0);
    }

    #[test]
    fn parse_dry_run_ignores_non_json_lines() {
        let stderr = [
            "not json",
            r#"{"type": "file_status", "status": "A", "path": "/a"}"#,
            "some garbage",
        ]
        .join("\n");

        let (files, total_size) = parse_dry_run_output(&stderr);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/a");
        assert_eq!(total_size, 0);
    }

    #[test]
    fn parse_dry_run_ignores_log_messages() {
        let stderr = [
            r#"{"type": "log_message", "levelname": "WARNING", "message": "something"}"#,
            r#"{"type": "file_status", "status": "A", "path": "/b"}"#,
        ]
        .join("\n");

        let (files, total_size) = parse_dry_run_output(&stderr);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/b");
        assert_eq!(total_size, 0);
    }

    #[test]
    fn parse_dry_run_empty_input() {
        let (files, total_size) = parse_dry_run_output("");

        assert!(files.is_empty());
        assert_eq!(total_size, 0);
    }

    fn make_schedule(id: i64, sources: Vec<&str>) -> shared::types::ScheduleConfig {
        shared::types::ScheduleConfig {
            id,
            schedule_type: shared::types::ScheduleType::Backup,
            cron_expression: "0 3 * * *".to_owned(),
            enabled: true,
            backup_sources: sources.into_iter().map(str::to_owned).collect(),
            rate_limit_kbps: None,
            canary_enabled: false,
            exclude_patterns: Vec::new(),
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 0,
            compact_enabled: true,
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
        }
    }

    fn make_repo(schedules: Vec<shared::types::ScheduleConfig>) -> shared::types::RepoConfig {
        shared::types::RepoConfig {
            repo_id: shared::types::RepoId(1),
            name: "test-repo".to_owned(),
            repo_path: "/backup/test".to_owned(),
            ssh_user: "borg".to_owned(),
            ssh_host: "backup.example.com".to_owned(),
            ssh_port: 22,
            ssh_host_key: "ssh-ed25519 AAAATEST".to_owned(),
            passphrase: "secret".to_owned(),
            compression: shared::types::Compression::Lz4,
            enabled: true,
            accept_relocation: false,
            schedules,
        }
    }

    #[test]
    fn backup_target_uses_first_schedule_when_no_id_given() {
        let repo = make_repo(vec![
            make_schedule(10, vec!["/var"]),
            make_schedule(20, vec!["/home"]),
        ]);
        let target = backup_target_from_repo(&repo, "hostname", None);
        assert_eq!(target.backup_sources, vec!["/var"]);
    }

    #[test]
    fn backup_target_uses_specified_schedule_id() {
        let repo = make_repo(vec![
            make_schedule(10, vec!["/var"]),
            make_schedule(20, vec!["/home"]),
        ]);
        let target = backup_target_from_repo(&repo, "hostname", Some(20));
        assert_eq!(target.backup_sources, vec!["/home"]);
    }

    #[test]
    fn backup_target_falls_back_to_first_when_id_not_found() {
        let repo = make_repo(vec![
            make_schedule(10, vec!["/var"]),
            make_schedule(20, vec!["/home"]),
        ]);
        let target = backup_target_from_repo(&repo, "hostname", Some(99));
        assert_eq!(target.backup_sources, vec!["/var"]);
    }

    #[test]
    fn build_borg_env_uses_pinned_known_hosts_file() {
        let mut target = backup_target_from_repo(
            &make_repo(vec![make_schedule(10, vec!["/var"])]),
            "hostname",
            None,
        );
        let known_hosts = write_known_hosts(&mut target).unwrap().unwrap();
        let env = build_borg_env(&target);
        let borg_rsh = env
            .iter()
            .find(|(key, _value)| key == "BORG_RSH")
            .map(|(_key, value)| value.as_str());
        let expected = shared::ssh::borg_rsh_with_known_hosts(known_hosts.path());

        assert_eq!(borg_rsh, Some(expected.as_str()));
    }

    #[test]
    fn write_known_hosts_pins_the_repository_endpoint() {
        let mut target = backup_target_from_repo(
            &make_repo(vec![make_schedule(10, vec!["/var"])]),
            "hostname",
            None,
        );
        target.ssh_port = 2222;

        let known_hosts = write_known_hosts(&mut target).unwrap().unwrap();
        let contents = std::fs::read_to_string(known_hosts.path()).unwrap();

        assert_eq!(contents, "[backup.example.com]:2222 ssh-ed25519 AAAATEST\n");
    }

    #[test]
    fn repo_operation_key_is_based_on_physical_repo_location() {
        let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
        let first = backup_target_from_repo(&repo, "hostname", None);
        let second = backup_target_from_repo(&repo, "hostname", Some(10));

        assert_eq!(
            RepoOperationKey::from_backup_target(&first),
            RepoOperationKey::from_backup_target(&second)
        );
    }

    #[tokio::test]
    async fn cancel_backup_with_no_active_task_sends_nothing() {
        let executor = Executor::new("ws://localhost", "token");
        let (tx, mut rx) = mpsc::channel(8);
        let repo_id = shared::types::RepoId(1);

        executor.handle_cancel_backup(repo_id, &tx).await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn cancel_backup_aborts_queued_task_and_sends_cancelled() {
        let executor = Executor::new("ws://localhost", "token");
        let (tx, mut rx) = mpsc::channel(8);
        let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
        let config = shared::types::AgentConfig {
            client_hostname: "hostname".to_owned(),
            skip_targets: Vec::new(),
            repos: vec![repo.clone()],
        };
        *executor.current_config.lock().await = Some(config);

        let repo_key =
            RepoOperationKey::from_backup_target(&backup_target_from_repo(&repo, "hostname", None));
        let repo_queue = executor.repo_operation_queue(&repo_key).await;
        let permit = Arc::clone(&repo_queue).acquire_owned().await.unwrap();

        executor.handle_run_now(repo.repo_id, None, None, &tx).await;

        assert!(rx.try_recv().is_err());
        assert_eq!(executor.active_backup_tasks.lock().await.len(), 1);

        executor.handle_cancel_backup(repo.repo_id, &tx).await;

        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, AgentToServer::BackupCancelled { repo_id: r } if r == repo.repo_id));
        assert!(executor.active_backup_tasks.lock().await.is_empty());
        drop(permit);
    }

    #[tokio::test]
    async fn repo_operation_queue_serializes_tasks() {
        let executor = Executor::new("ws://localhost", "token");
        let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
        let repo_key =
            RepoOperationKey::from_backup_target(&backup_target_from_repo(&repo, "hostname", None));
        let repo_queue = executor.repo_operation_queue(&repo_key).await;
        let permit = Arc::clone(&repo_queue).acquire_owned().await.unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        let queue = executor.repo_operation_queue(&repo_key).await;

        let handle = tokio::spawn(async move {
            let Ok(_permit) = queue.acquire_owned().await else {
                return;
            };

            if let Err(e) = tx.send(()).await {
                tracing::debug!(error = %e, "test send failed");
            }
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );

        drop(permit);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .is_ok_and(|msg| msg.is_some())
        );
        handle.await.unwrap();
    }
}
