// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use shared::{
    borg::{
        env::{EnvParams, rsh_for_target},
        log_json::truncate_chars,
    },
    hooks::HookCommand,
    task_registry::TaskRegistry,
    types::{BackupStatus, BackupWarningKind},
};
use tokio::{process::Command, sync::mpsc};
use tracing::{error, info, warn};

use crate::borg::Borg;

mod create;
mod maintenance;
mod parse;
mod target;

use self::parse::MAX_FAILURE_CHARS;
pub(crate) use self::parse::warning_status_log;
pub use self::target::BackupTarget;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup skipped: {0}")]
    Skipped(String),
    #[error("borg command failed: {0}")]
    BorgFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stats parse error: {0}")]
    StatsParse(String),
    #[error("borg command timed out after {seconds} seconds: {command}")]
    Timeout { seconds: u64, command: String },
}

impl BackupError {
    pub fn borg_command(&self) -> Option<&str> {
        match self {
            Self::Timeout { command, .. } => Some(command.as_str()),
            Self::Skipped(_) | Self::BorgFailed(_) | Self::Io(_) | Self::StatsParse(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct BackupResult {
    pub status: BackupStatus,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub repo_unique_csize: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub warnings: Vec<String>,
    pub warning_kind: BackupWarningKind,
    pub archive_name: Option<String>,
    pub borg_command: Option<String>,
    pub canary_result: Option<CanaryResult>,
}

#[derive(Debug)]
pub struct CanaryResult {
    pub success: bool,
    pub archive_name: String,
    pub error_message: Option<String>,
}

pub struct BackupEngine {
    borg: Borg,
    borg_timeout: Option<Duration>,
    /// Kept alongside `borg` so the virtual-machine staging that runs before
    /// a backup registers its children with the same registry shutdown
    /// drains, rather than leaving `virsh` and `qemu-img` unreachable.
    task_registry: TaskRegistry,
}

impl BackupEngine {
    pub fn new(task_registry: TaskRegistry) -> Self {
        Self {
            borg: Borg::new(task_registry.clone()),
            borg_timeout: None,
            task_registry,
        }
    }

    /// The registry every child this engine starts registers with.
    #[must_use]
    pub fn task_registry(&self) -> &TaskRegistry {
        &self.task_registry
    }

    #[cfg(test)]
    fn with_config(borg_binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            borg: Borg::with_extra_env(borg_binary, extra_env),
            borg_timeout: None,
            task_registry: TaskRegistry::default(),
        }
    }

    #[cfg(test)]
    fn with_config_and_timeout(
        borg_binary: PathBuf,
        extra_env: Vec<(String, String)>,
        borg_timeout: Option<Duration>,
    ) -> Self {
        Self {
            borg: Borg::with_extra_env(borg_binary, extra_env),
            borg_timeout,
            task_registry: TaskRegistry::default(),
        }
    }

    pub async fn run_backup(
        &self,
        target: &BackupTarget,
        canary: Option<&CanaryToken>,
        log_tx: Option<mpsc::Sender<String>>,
    ) -> Result<BackupResult, BackupError> {
        let start = Instant::now();
        let target_name = &target.target_name;

        if target
            .skip_targets
            .iter()
            .any(|skip| skip == &target.target_name)
        {
            warn!(target_name = %target_name, "Skipping target listed in skip_targets");
            return Err(BackupError::Skipped(format!(
                "target {target_name} is listed in skip_targets"
            )));
        }

        for cmd in &target.pre_backup_commands {
            self.run_hook_command(cmd, "pre-backup", target.hook_timeout_seconds)
                .await?;
        }

        let exclude_file = Self::write_exclude_file(&target.exclude_patterns)?;
        let include_file = Self::write_include_patterns_file(&target.include_patterns)?;

        let create_result = self
            .run_borg_create(
                target,
                &target.backup_sources,
                exclude_file.path(),
                include_file.as_ref().map(tempfile::NamedTempFile::path),
                log_tx,
            )
            .await?;

        let canary_result = if let Some(canary) = canary {
            Some(
                self.verify_canary(target, canary, &create_result.archive_name)
                    .await,
            )
        } else {
            None
        };

        self.run_borg_prune(target).await?;

        if target.compact_enabled {
            self.run_borg_compact(target).await?;
        }

        for cmd in &target.post_backup_commands {
            self.run_hook_command(cmd, "post-backup", target.hook_timeout_seconds)
                .await?;
        }

        let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

        Ok(BackupResult {
            status: create_result.status,
            original_size: create_result.stats.original_size,
            compressed_size: create_result.stats.compressed_size,
            deduplicated_size: create_result.stats.deduplicated_size,
            repo_unique_csize: create_result.stats.repo_unique_csize,
            files_processed: create_result.stats.files_processed,
            duration_secs,
            error_message: create_result.error_message,
            warnings: create_result.warnings,
            warning_kind: create_result.warning_kind,
            archive_name: Some(create_result.archive_name),
            borg_command: Some(create_result.borg_command),
            canary_result,
        })
    }

    async fn run_hook_command(
        &self,
        cmd: &HookCommand,
        label: &str,
        default_timeout_seconds: u32,
    ) -> Result<(), BackupError> {
        let timeout_seconds = cmd.timeout_or(default_timeout_seconds);
        info!("Running {label} hook command (timeout {timeout_seconds}s)");

        let output = tokio::time::timeout(
            Duration::from_secs(timeout_seconds.into()),
            Command::new("sh").arg("-c").arg(&cmd.command).output(),
        )
        .await
        .map_err(|_| {
            BackupError::BorgFailed(format!(
                "{label} hook command timed out after {timeout_seconds} seconds"
            ))
        })??;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);
            error!("{label} hook command failed (exit {exit_code})");
            let mut message = format!("{label} hook command exited with code {exit_code}");
            let stdout_trimmed = stdout.trim();
            let stderr_trimmed = stderr.trim();
            // Truncate each stream independently rather than the combined
            // message: stderr usually carries the actual error, and a large
            // stdout must not be able to push it past the cutoff entirely.
            if !stdout_trimmed.is_empty() {
                let _ = write!(
                    message,
                    "\nstdout:\n{}",
                    truncate_chars(stdout_trimmed.to_owned(), MAX_FAILURE_CHARS)
                );
            }
            if !stderr_trimmed.is_empty() {
                let _ = write!(
                    message,
                    "\nstderr:\n{}",
                    truncate_chars(stderr_trimmed.to_owned(), MAX_FAILURE_CHARS)
                );
            }
            return Err(BackupError::BorgFailed(message));
        }

        Ok(())
    }

    async fn run_borg_command(
        &self,
        target: &BackupTarget,
        args: &[&str],
        env_vars: &[(String, String)],
    ) -> Result<std::process::Output, BackupError> {
        let command = Self::format_command_slice(target, args);
        match self.borg_timeout {
            Some(timeout) => Ok(tokio::time::timeout(timeout, self.borg.run(args, env_vars))
                .await
                .map_err(|_| BackupError::Timeout {
                    seconds: timeout_secs(timeout),
                    command: command.clone(),
                })??),
            None => Ok(self.borg.run(args, env_vars).await?),
        }
    }

    async fn run_borg_command_in_dir(
        &self,
        target: &BackupTarget,
        args: &[&str],
        env_vars: &[(String, String)],
        dir: &Path,
    ) -> Result<std::process::Output, BackupError> {
        let command = Self::format_command_slice(target, args);
        match self.borg_timeout {
            Some(timeout) => Ok(tokio::time::timeout(
                timeout,
                self.borg.run_in_dir(args, env_vars, dir),
            )
            .await
            .map_err(|_| BackupError::Timeout {
                seconds: timeout_secs(timeout),
                command: command.clone(),
            })??),
            None => Ok(self.borg.run_in_dir(args, env_vars, dir).await?),
        }
    }
}

/// The `--rsh` command for `target`: pinned to its `known_hosts_path` when
/// set, otherwise auto-accepting a new host key unless one was already
/// recorded out of band. Shared by [`BackupEngine`] and the task executor,
/// which both run borg against the same kind of target.
pub(crate) fn borg_rsh_for_target(target: &BackupTarget) -> String {
    rsh_for_target(&target.ssh_host_key, target.known_hosts_path.as_deref())
}

/// The environment borg needs to run a backup/maintenance command against
/// `target`'s repository. Shared by [`BackupEngine`] and the task executor,
/// which both run borg against the same kind of target.
pub(crate) fn borg_env(target: &BackupTarget) -> Vec<(String, String)> {
    shared::borg::env::build_env(&EnvParams {
        ssh_user: &target.ssh_user,
        ssh_host: &target.ssh_host,
        ssh_port: target.ssh_port,
        repo_path: &target.repo_path,
        passphrase: &target.passphrase,
        hostname: &target.hostname,
        ssh_host_key: &target.ssh_host_key,
        known_hosts_path: target.known_hosts_path.as_deref(),
        accept_relocation: target.accept_relocation,
        ssh_auth_sock: target.ssh_auth_sock.as_deref(),
    })
}

fn timeout_secs(timeout: Duration) -> u64 {
    timeout.as_secs().max(1)
}

#[derive(Debug)]
pub struct CanaryToken {
    pub nonce: String,
    pub canary_path: PathBuf,
    pub expected_content: String,
}

struct CreateResult {
    status: BackupStatus,
    stats: ParsedStats,
    error_message: Option<String>,
    warnings: Vec<String>,
    warning_kind: BackupWarningKind,
    archive_name: String,
    borg_command: String,
}

struct ParsedStats {
    original_size: i64,
    compressed_size: i64,
    deduplicated_size: i64,
    repo_unique_csize: i64,
    files_processed: i64,
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
