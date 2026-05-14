// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::Utc;
use shared::{
    protocol::AgentToServer,
    types::{AgentConfig, BorgEncryption, Compression, RepoConfig, RepoId},
};
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info, warn};

use crate::{
    backup::{BackupEngine, BackupError, BackupTarget, CanaryToken},
    ssh_forward::{SshForwardError, SshForwardSocket, run_ssh_forward},
};

pub enum ExecutorCommand {
    UpdateConfig(AgentConfig),
    RunNow {
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
}

pub struct Executor {
    server_url: String,
    token: String,
    active_repos: Arc<Mutex<HashSet<RepoId>>>,
    current_config: Arc<Mutex<Option<AgentConfig>>>,
    engine: Arc<BackupEngine>,
}

impl Executor {
    pub fn new(server_url: &str, token: &str) -> Self {
        Self {
            server_url: server_url.to_owned(),
            token: token.to_owned(),
            active_repos: Arc::new(Mutex::new(HashSet::new())),
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
                ExecutorCommand::RunNow { repo_id } => {
                    self.handle_run_now(repo_id, &outbound_tx).await;
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
            }
        }
    }

    async fn handle_run_now(&self, repo_id: RepoId, outbound_tx: &mpsc::Sender<AgentToServer>) {
        {
            let mut active = self.active_repos.lock().await;
            if !active.insert(repo_id) {
                warn!(repo_id = ?repo_id, "backup already in progress, rejecting");
                let msg = AgentToServer::BackupRejected {
                    repo_id,
                    reason: "backup already in progress for this repo".to_owned(),
                };
                if let Err(e) = outbound_tx.send(msg).await {
                    tracing::debug!(error = %e, "outbound send failed");
                }
                return;
            }
        }

        let config_guard = self.current_config.lock().await;
        let Some(config) = config_guard.as_ref() else {
            warn!(repo_id = ?repo_id, "no config available, rejecting backup");
            self.active_repos.lock().await.remove(&repo_id);
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
            self.active_repos.lock().await.remove(&repo_id);
            let msg = AgentToServer::BackupRejected {
                repo_id,
                reason: "repo not found in agent config".to_owned(),
            };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            return;
        };

        let target = backup_target_from_repo(repo, &config.client_hostname);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let active_repos = Arc::clone(&self.active_repos);
        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        tokio::spawn(async move {
            run_backup_task(
                repo_id,
                target,
                &hostname,
                &server_url,
                &token,
                &engine,
                &outbound,
            )
            .await;
            active_repos.lock().await.remove(&repo_id);
        });
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

        let target = backup_target_from_repo(repo, &config.client_hostname);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        tokio::spawn(async move {
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

        let target = backup_target_from_repo(repo, &config.client_hostname);
        let hostname = config.client_hostname.clone();
        drop(config_guard);

        let engine = Arc::clone(&self.engine);
        let outbound = outbound_tx.clone();
        let server_url = self.server_url.clone();
        let token = self.token.clone();

        tokio::spawn(async move {
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

        let repo_url = format!("ssh://{ssh_user}@{ssh_host}:{ssh_port}/{repo_path}");
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
        repo_path: String::new(),
        ssh_user: String::new(),
        ssh_host: String::new(),
        ssh_port: 22,
        passphrase: String::new(),
        hostname: hostname.to_owned(),
        compression: Compression::Lz4,
        backup_sources: Vec::new(),
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
    };

    let _ssh_forward =
        setup_ssh_forward(&mut ssh_forward_target, hostname, server_url, token).await;

    let borg_binary = std::env::var("BORG_BINARY").unwrap_or_else(|_| "borg".to_string());

    let mut cmd = tokio::process::Command::new(&borg_binary);
    cmd.args(["init", "--encryption", encryption.as_borg_arg(), repo_url])
        .env("BORG_PASSPHRASE", passphrase)
        .env(
            "BORG_RSH",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new",
        );

    if let Some(sock) = &ssh_forward_target.ssh_auth_sock {
        cmd.env("SSH_AUTH_SOCK", sock);
    }

    let output = tokio::time::timeout(Duration::from_mins(2), cmd.output())
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

async fn run_backup_task(
    repo_id: RepoId,
    mut target: BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
    engine: &BackupEngine,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let started_at = Utc::now();
    let started_msg = AgentToServer::BackupStarted {
        repo_id,
        started_at,
    };
    if let Err(e) = outbound_tx.send(started_msg).await {
        tracing::debug!(error = %e, "outbound send failed");
    }

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

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

    match engine.run_backup(&target).await {
        Ok(result) => {
            let finished_at = Utc::now();
            let report = shared::types::BackupReport {
                id: shared::types::ReportId(0),
                client_id: shared::types::ClientId(0),
                repo_id,
                started_at,
                finished_at,
                status: result.status,
                original_size: result.original_size,
                compressed_size: result.compressed_size,
                deduplicated_size: result.deduplicated_size,
                files_processed: result.files_processed,
                duration_secs: result.duration_secs,
                error_message: result.error_message,
                warnings: result.warnings,
                borg_version: None,
            };
            let msg = AgentToServer::BackupCompleted { report };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }

            if let Some(canary) = &canary {
                run_canary_verification(repo_id, &target, engine, canary, outbound_tx).await;
                BackupEngine::cleanup_canary(canary);
            }
        }
        Err(BackupError::Skipped(reason)) => {
            warn!(repo_id = ?repo_id, reason = %reason, "backup skipped");
            let msg = AgentToServer::BackupRejected { repo_id, reason };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            if let Some(canary) = &canary {
                BackupEngine::cleanup_canary(canary);
            }
        }
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "backup failed");
            let finished_at = Utc::now();
            let report = shared::types::BackupReport {
                id: shared::types::ReportId(0),
                client_id: shared::types::ClientId(0),
                repo_id,
                started_at,
                finished_at,
                status: shared::types::BackupStatus::Failed,
                original_size: 0,
                compressed_size: 0,
                deduplicated_size: 0,
                files_processed: 0,
                duration_secs: finished_at.signed_duration_since(started_at).num_seconds(),
                error_message: Some(e.to_string()),
                warnings: Vec::new(),
                borg_version: None,
            };
            let msg = AgentToServer::BackupCompleted { report };
            if let Err(e) = outbound_tx.send(msg).await {
                tracing::debug!(error = %e, "outbound send failed");
            }
            if let Some(canary) = &canary {
                BackupEngine::cleanup_canary(canary);
            }
        }
    }
}

async fn setup_ssh_forward(
    target: &mut BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
) -> Option<SshForwardSocket> {
    let socket = match SshForwardSocket::create() {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "ssh forward: failed to create socket, proceeding without");
            return None;
        }
    };

    if let Err(e) = run_ssh_forward(&socket, server_url, hostname, token).await {
        match e {
            SshForwardError::Url(msg) => warn!(error = %msg, "ssh forward: bad relay url"),
            other => warn!(error = %other, "ssh forward: setup failed, proceeding without"),
        }
        return None;
    }

    target.ssh_auth_sock = Some(socket.socket_path.clone());
    Some(socket)
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

pub fn backup_target_from_repo(repo: &RepoConfig, hostname: &str) -> BackupTarget {
    let schedule = repo.schedules.first();
    BackupTarget {
        target_name: repo.name.clone(),
        repo_path: repo.repo_path.clone(),
        ssh_user: repo.ssh_user.clone(),
        ssh_host: repo.ssh_host.clone(),
        ssh_port: repo.ssh_port,
        passphrase: repo.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: repo.compression.clone(),
        backup_sources: schedule.map_or_else(Vec::new, |s| s.backup_sources.clone()),
        keep_daily: schedule.map_or(7, |s| s.keep_daily),
        keep_weekly: schedule.map_or(4, |s| s.keep_weekly),
        keep_monthly: schedule.map_or(6, |s| s.keep_monthly),
        keep_yearly: schedule.map_or(0, |s| s.keep_yearly),
        compact_enabled: schedule.is_none_or(|s| s.compact_enabled),
        pre_backup_commands: schedule.map_or_else(Vec::new, |s| s.pre_backup_commands.clone()),
        post_backup_commands: schedule.map_or_else(Vec::new, |s| s.post_backup_commands.clone()),
        skip_targets: Vec::new(),
        exclude_patterns: Vec::new(),
        ssh_auth_sock: None,
        canary_enabled: schedule.is_some_and(|s| s.canary_enabled),
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
        repo_path: target.repo_path.clone(),
        ssh_user: target.ssh_user.clone(),
        ssh_host: target.ssh_host.clone(),
        ssh_port: target.ssh_port,
        passphrase: target.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: target.compression.clone(),
        backup_sources: Vec::new(),
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
        repo_path: target.repo_path.clone(),
        ssh_user: target.ssh_user.clone(),
        ssh_host: target.ssh_host.clone(),
        ssh_port: target.ssh_port,
        passphrase: target.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: target.compression.clone(),
        backup_sources: Vec::new(),
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
