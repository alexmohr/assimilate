// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::Utc;
use serde::Deserialize;
use shared::{
    hooks::HookCommand,
    ssh::{borg_rsh, borg_rsh_with_known_hosts},
    task_registry::TaskRegistry,
    types::{BORG_REPO_ENV_KEY, BackupStatus, Compression, FileChangePattern, build_repo_url},
};
use tokio::{process::Command, sync::mpsc};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::borg::Borg;

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

#[derive(Clone)]
pub struct BackupTarget {
    pub target_name: String,
    pub schedule_id: Option<i64>,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_host_key: String,
    pub known_hosts_path: Option<PathBuf>,
    pub passphrase: String,
    pub hostname: String,
    pub compression: Compression,
    pub backup_sources: Vec<String>,
    pub keep_hourly: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub keep_yearly: u32,
    pub compact_enabled: bool,
    pub pre_backup_commands: Vec<HookCommand>,
    pub post_backup_commands: Vec<HookCommand>,
    pub hook_timeout_seconds: u32,
    pub skip_targets: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub rate_limit_kbps: Option<u32>,
    pub ssh_auth_sock: Option<PathBuf>,
    pub canary_enabled: bool,
    pub accept_relocation: bool,
    pub file_change_patterns: Vec<FileChangePattern>,
}

impl Default for BackupTarget {
    fn default() -> Self {
        Self {
            target_name: String::new(),
            schedule_id: None,
            repo_path: String::new(),
            ssh_user: String::new(),
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_host_key: String::new(),
            known_hosts_path: None,
            passphrase: String::new(),
            hostname: String::new(),
            compression: Compression::Lz4,
            backup_sources: Vec::new(),
            keep_hourly: 0,
            keep_daily: 0,
            keep_weekly: 0,
            keep_monthly: 0,
            keep_yearly: 0,
            compact_enabled: false,
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
            hook_timeout_seconds: 60,
            skip_targets: Vec::new(),
            exclude_patterns: Vec::new(),
            rate_limit_kbps: None,
            ssh_auth_sock: None,
            canary_enabled: false,
            accept_relocation: false,
            file_change_patterns: Vec::new(),
        }
    }
}

impl BackupTarget {
    /// Construct a maintenance target (check/verify) from an existing backup target,
    /// overriding the hostname and clearing backup-specific fields irrelevant to
    /// maintenance operations.
    pub fn for_maintenance(base: &Self, hostname: String) -> Self {
        Self {
            target_name: base.target_name.clone(),
            schedule_id: base.schedule_id,
            repo_path: base.repo_path.clone(),
            ssh_user: base.ssh_user.clone(),
            ssh_host: base.ssh_host.clone(),
            ssh_port: base.ssh_port,
            ssh_host_key: base.ssh_host_key.clone(),
            passphrase: base.passphrase.clone(),
            compression: base.compression.clone(),
            rate_limit_kbps: base.rate_limit_kbps,
            accept_relocation: base.accept_relocation,
            hostname,
            ..Self::default()
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
}

impl BackupEngine {
    pub fn new(task_registry: TaskRegistry) -> Self {
        Self {
            borg: Borg::new(task_registry),
            borg_timeout: None,
        }
    }

    #[cfg(test)]
    fn with_config(borg_binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            borg: Borg::with_extra_env(borg_binary, extra_env),
            borg_timeout: None,
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

        let create_result = self
            .run_borg_create(target, &target.backup_sources, exclude_file.path(), log_tx)
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
            original_size: create_result.original_size,
            compressed_size: create_result.compressed_size,
            deduplicated_size: create_result.deduplicated_size,
            repo_unique_csize: create_result.repo_unique_csize,
            files_processed: create_result.files_processed,
            duration_secs,
            error_message: create_result.error_message,
            warnings: create_result.warnings,
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
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);
            error!("{label} hook command failed (exit {exit_code})");
            let stderr_trimmed = stderr.trim();
            let detail = if stderr_trimmed.is_empty() {
                String::new()
            } else {
                format!(": {stderr_trimmed}")
            };
            return Err(BackupError::BorgFailed(format!(
                "{label} hook command exited with code {exit_code}{detail}"
            )));
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

    fn write_exclude_file(patterns: &[String]) -> Result<tempfile::NamedTempFile, BackupError> {
        let mut file = tempfile::NamedTempFile::new()?;
        for pattern in patterns {
            writeln!(file, "{pattern}")?;
        }
        file.flush()?;
        Ok(file)
    }

    fn borg_env(target: &BackupTarget) -> Vec<(String, String)> {
        let repo_url = build_repo_url(
            &target.ssh_user,
            &target.ssh_host,
            target.ssh_port,
            &target.repo_path,
        );

        let mut env = vec![
            (BORG_REPO_ENV_KEY.to_owned(), repo_url),
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

    fn compression_arg(compression: &Compression) -> String {
        compression.to_string()
    }

    async fn run_borg_create(
        &self,
        target: &BackupTarget,
        backup_sources: &[String],
        exclude_file: &Path,
        log_tx: Option<mpsc::Sender<String>>,
    ) -> Result<CreateResult, BackupError> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let archive_name = format!("{hostname}-{now}", hostname = target.hostname);

        let args = Self::borg_create_args(target, backup_sources, exclude_file, &archive_name);
        let borg_command = Self::format_command_string(target, &args);

        let env_vars = Self::borg_env(target);

        info!("Running borg create for archive {archive_name}");

        let output = if let Some(tx) = log_tx {
            self.borg.run_with_log_channel(&args, &env_vars, tx).await?
        } else {
            self.borg.run(&args, &env_vars).await?
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let source_not_found = parse_source_not_found_errors(&stderr);
        if !source_not_found.is_empty() {
            let summary = source_not_found.join("; ");
            error!("Borg backup source(s) not found: {summary}");
            return Err(BackupError::BorgFailed(format!(
                "backup source(s) not found: {summary}"
            )));
        }

        match exit_code {
            0 => {
                let stats = parse_json_stats(&output.stdout)?;
                let warnings = parse_warnings(&stderr);
                let warnings = filter_file_change_warnings(warnings, &target.file_change_patterns)?;
                let status = if warnings.is_empty() {
                    BackupStatus::Success
                } else {
                    BackupStatus::Warning
                };
                Ok(CreateResult {
                    status,
                    original_size: stats.original_size,
                    compressed_size: stats.compressed_size,
                    deduplicated_size: stats.deduplicated_size,
                    repo_unique_csize: stats.repo_unique_csize,
                    files_processed: stats.files_processed,
                    error_message: None,
                    warnings,
                    archive_name,
                    borg_command,
                })
            }
            1 if stderr_has_warnings(&stderr) => {
                let BorgDiagnostics {
                    warnings: reported_warnings,
                    context,
                } = parse_diagnostics(&stderr);
                let reported = reported_warnings.len();
                let warnings =
                    filter_file_change_warnings(reported_warnings, &target.file_change_patterns)?;
                // rc 1 is borg's "finished, with warnings". Every warning it
                // reported may have matched an `ignore` pattern, which the user
                // asked to be silent about; when it reported none at all the run
                // still has to say something the bare exit code does not.
                let (status, warnings) = if !warnings.is_empty() {
                    (BackupStatus::Warning, warnings)
                } else if reported > 0 {
                    (BackupStatus::Success, Vec::new())
                } else {
                    (
                        BackupStatus::Warning,
                        vec![unexplained_exit(exit_code, &context)],
                    )
                };
                let summary = warnings.join("; ");
                if warnings.is_empty() {
                    info!("Borg reported {reported} warning(s), all suppressed by ignore patterns");
                } else {
                    warn!("Borg reported warnings: {summary}");
                }
                // Kept populated (not None) despite duplicating `warnings`:
                // dispatch_backup_completion_notification's backup_warning path
                // reads only this field for the email/push body, so clearing it
                // silently drops warning text from notifications. The
                // duplicate-display bug this was meant to fix is handled at the
                // UI layer instead (report detail views hide the Error box when
                // status is Warning).
                let error_message = (!summary.is_empty()).then_some(summary);
                let stats = parse_json_stats(&output.stdout)?;
                Ok(CreateResult {
                    status,
                    original_size: stats.original_size,
                    compressed_size: stats.compressed_size,
                    deduplicated_size: stats.deduplicated_size,
                    repo_unique_csize: stats.repo_unique_csize,
                    files_processed: stats.files_processed,
                    error_message,
                    warnings,
                    archive_name,
                    borg_command,
                })
            }
            _ => Err(BackupError::BorgFailed(format!(
                "borg create exited with code {exit_code}: {}",
                describe_borg_failure(&stderr)
            ))),
        }
    }

    /// Build a preview of the borg create command that will be run, using a
    /// placeholder for the transient exclude-from temp file path.
    pub fn preview_create_command(target: &BackupTarget) -> String {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let archive_name = format!("{hostname}-{now}", hostname = target.hostname);
        let exclude_placeholder = std::path::Path::new("<exclude-file>");
        let args = Self::borg_create_args(
            target,
            &target.backup_sources,
            exclude_placeholder,
            &archive_name,
        );
        Self::format_command_string(target, &args)
    }

    /// Build a human-readable borg command string with `BORG_REPO` expanded but
    /// the passphrase omitted (it is always passed via the environment).
    fn format_command_string(target: &BackupTarget, args: &[impl AsRef<OsStr>]) -> String {
        let repo_url = build_repo_url(
            &target.ssh_user,
            &target.ssh_host,
            target.ssh_port,
            &target.repo_path,
        );
        let args_str = args
            .iter()
            .map(|a| a.as_ref().to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        format!("BORG_REPO={repo_url} borg {args_str}")
    }

    fn format_command_slice(target: &BackupTarget, args: &[&str]) -> String {
        let repo_url = build_repo_url(
            &target.ssh_user,
            &target.ssh_host,
            target.ssh_port,
            &target.repo_path,
        );
        let args_str = args.join(" ");
        format!("BORG_REPO={repo_url} borg {args_str}")
    }

    fn borg_create_args(
        target: &BackupTarget,
        backup_sources: &[String],
        exclude_file: &Path,
        archive_name: &str,
    ) -> Vec<String> {
        let mut flags: Vec<String> = vec![
            "create".to_owned(),
            // do not make inodes part of the cache, to prevent issues on nfs volumes
            "--files-cache=ctime,size".to_owned(),
            "--lock-wait".to_owned(),
            "600".to_owned(),
            "--show-rc".to_owned(),
            "--json".to_owned(),
            "--log-json".to_owned(),
            "--progress".to_owned(),
            "--compression".to_owned(),
            Self::compression_arg(&target.compression),
            "--exclude-caches".to_owned(),
            "--exclude-if-present".to_owned(),
            ".nobackup".to_owned(),
            "--exclude-from".to_owned(),
            exclude_file.to_string_lossy().into_owned(),
        ];

        if let Some(rate_limit_kbps) = target.rate_limit_kbps.filter(|&kbps| kbps > 0) {
            flags.push("--upload-ratelimit".to_owned());
            flags.push(rate_limit_kbps.to_string());
        }

        flags.push(format!("::{archive_name}"));

        Borg::args_with_positional(&flags, backup_sources)
    }

    async fn run_borg_prune(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let all_zero = target.keep_hourly == 0
            && target.keep_daily == 0
            && target.keep_weekly == 0
            && target.keep_monthly == 0
            && target.keep_yearly == 0;

        if all_zero {
            warn!("no retention configured; skipping prune to avoid deleting all archives");
            return Ok(());
        }

        let glob_pattern = format!("*{hostname}-*", hostname = target.hostname);
        let keep_hourly = target.keep_hourly.to_string();
        let keep_daily = target.keep_daily.to_string();
        let keep_weekly = target.keep_weekly.to_string();
        let keep_monthly = target.keep_monthly.to_string();
        let keep_yearly = target.keep_yearly.to_string();

        let mut args = vec![
            "prune",
            "--lock-wait",
            "600",
            "--list",
            "--show-rc",
            "--log-json",
            "-a",
            &glob_pattern,
        ];

        if target.keep_hourly > 0 {
            args.push("--keep-hourly");
            args.push(&keep_hourly);
        }
        if target.keep_daily > 0 {
            args.push("--keep-daily");
            args.push(&keep_daily);
        }
        if target.keep_weekly > 0 {
            args.push("--keep-weekly");
            args.push(&keep_weekly);
        }
        if target.keep_monthly > 0 {
            args.push("--keep-monthly");
            args.push(&keep_monthly);
        }
        if target.keep_yearly > 0 {
            args.push("--keep-yearly");
            args.push(&keep_yearly);
        }

        let env_vars = Self::borg_env(target);

        info!("Running borg prune");

        let output = self.run_borg_command(target, &args, &env_vars).await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg prune exited with code {exit_code}: {stderr}"
            )));
        }

        if exit_code == 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let warnings = parse_warnings(&stderr);
            if !warnings.is_empty() {
                warn!("borg prune warnings: {}", warnings.join("; "));
            }
        }

        Ok(())
    }

    async fn run_borg_compact(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = Self::borg_env(target);

        info!("Running borg compact");

        let compact_args = ["compact", "--lock-wait", "600", "--log-json"];
        let output = self
            .run_borg_command(target, &compact_args, &env_vars)
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg compact exited with code {exit_code}: {stderr}"
            )));
        }

        if exit_code == 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let warnings = parse_warnings(&stderr);
            if !warnings.is_empty() {
                warn!("borg compact warnings: {}", warnings.join("; "));
            }
        }

        Ok(())
    }

    pub async fn run_check(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = Self::borg_env(target);

        info!(target = %target.target_name, "Running borg check");

        let check_args = ["check", "--lock-wait", "600", "--show-rc", "--log-json"];
        let output = self
            .run_borg_command(target, &check_args, &env_vars)
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg check exited with code {exit_code}: {stderr}"
            )));
        }

        if exit_code == 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let warnings = parse_warnings(&stderr);
            if !warnings.is_empty() {
                warn!("borg check warnings: {}", warnings.join("; "));
            }
        }

        info!(target = %target.target_name, "borg check completed");
        Ok(())
    }

    pub async fn run_verify(&self, target: &BackupTarget) -> Result<i64, BackupError> {
        let env_vars = Self::borg_env(target);
        let hostname = &target.hostname;

        info!(target = %target.target_name, "Running borg extract --dry-run (verify)");

        let glob_pattern = format!("*{hostname}-*");
        let list_args = [
            "list",
            "--lock-wait",
            "600",
            "--short",
            "--last",
            "1",
            "-a",
            glob_pattern.as_str(),
        ];
        let output = self.run_borg_command(target, &list_args, &env_vars).await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg list exited with code {exit_code}: {stderr}"
            )));
        }

        let archive_name = String::from_utf8_lossy(&output.stdout).trim().to_owned();

        if archive_name.is_empty() {
            return Err(BackupError::BorgFailed(
                "no archives found for verification".to_owned(),
            ));
        }

        let archive_ref = format!("::{archive_name}");
        let extract_args = [
            "extract",
            "--dry-run",
            "--lock-wait",
            "600",
            "--",
            archive_ref.as_str(),
        ];
        let output = self
            .run_borg_command(target, &extract_args, &env_vars)
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg extract --dry-run exited with code {exit_code}: {stderr}"
            )));
        }

        info!(target = %target.target_name, archive = %archive_name, "verify completed");
        Ok(1)
    }

    pub async fn write_canary(backup_sources: &[String]) -> Result<CanaryToken, BackupError> {
        let mut source_dir = None;
        for s in backup_sources {
            if !s.starts_with('!') && tokio::fs::metadata(s).await.is_ok_and(|m| m.is_dir()) {
                source_dir = Some(s);
                break;
            }
        }
        let source_dir = source_dir.ok_or_else(|| {
            BackupError::BorgFailed("no usable backup source directory for canary file".to_owned())
        })?;

        let nonce = Uuid::new_v4().to_string();
        let canary_path = Path::new(source_dir).join(".assimilate-canary");
        let content = format!("{{\"nonce\":\"{nonce}\"}}");

        tokio::fs::write(&canary_path, &content).await?;
        info!(path = %canary_path.display(), "canary file written");

        Ok(CanaryToken {
            nonce,
            canary_path,
            expected_content: content,
        })
    }

    pub async fn verify_canary(
        &self,
        target: &BackupTarget,
        canary: &CanaryToken,
        archive_name: &str,
    ) -> CanaryResult {
        match self
            .extract_and_verify_canary(target, canary, archive_name)
            .await
        {
            Ok(()) => {
                info!(archive = %archive_name, "canary verification passed");
                CanaryResult {
                    success: true,
                    archive_name: archive_name.to_owned(),
                    error_message: None,
                }
            }
            Err(e) => {
                error!(archive = %archive_name, error = %e, "canary verification failed");
                CanaryResult {
                    success: false,
                    archive_name: archive_name.to_owned(),
                    error_message: Some(e.to_string()),
                }
            }
        }
    }

    async fn extract_and_verify_canary(
        &self,
        target: &BackupTarget,
        canary: &CanaryToken,
        archive_name: &str,
    ) -> Result<(), BackupError> {
        let env_vars = Self::borg_env(target);
        let extract_dir = tempfile::tempdir()?;
        let archive_ref = format!("::{archive_name}");

        let canary_relative = canary
            .canary_path
            .strip_prefix("/")
            .unwrap_or(&canary.canary_path)
            .to_string_lossy()
            .into_owned();

        let extract_args = [
            "extract",
            "--lock-wait",
            "600",
            "--",
            archive_ref.as_str(),
            canary_relative.as_str(),
        ];
        let output = self
            .run_borg_command_in_dir(target, &extract_args, &env_vars, extract_dir.path())
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg extract canary exited with code {exit_code}: {stderr}"
            )));
        }

        let extracted_path = extract_dir.path().join(&canary_relative);
        let extracted_content = tokio::fs::read_to_string(&extracted_path)
            .await
            .map_err(|e| {
                BackupError::BorgFailed(format!("failed to read extracted canary: {e}"))
            })?;

        if extracted_content.trim() != canary.expected_content.trim() {
            return Err(BackupError::BorgFailed(format!(
                "canary mismatch: expected nonce '{}', got content '{extracted_content}'",
                canary.nonce
            )));
        }

        Ok(())
    }

    pub async fn cleanup_canary(canary: &CanaryToken) {
        if let Err(e) = tokio::fs::remove_file(&canary.canary_path).await {
            warn!(path = %canary.canary_path.display(), error = %e, "failed to remove canary file");
        }
    }
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
    original_size: i64,
    compressed_size: i64,
    deduplicated_size: i64,
    repo_unique_csize: i64,
    files_processed: i64,
    error_message: Option<String>,
    warnings: Vec<String>,
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

fn parse_json_stats(stdout: &[u8]) -> Result<ParsedStats, BackupError> {
    let output = String::from_utf8_lossy(stdout);
    let json: serde_json::Value = serde_json::from_str(output.trim())
        .map_err(|e| BackupError::StatsParse(format!("invalid JSON: {e}")))?;

    let stats = json
        .get("archive")
        .and_then(|a| a.get("stats"))
        .ok_or_else(|| BackupError::StatsParse("missing archive.stats".to_owned()))?;

    let original_size = stats
        .get("original_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing original_size".to_owned()))?;

    let compressed_size = stats
        .get("compressed_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing compressed_size".to_owned()))?;

    let deduplicated_size = stats
        .get("deduplicated_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing deduplicated_size".to_owned()))?;

    let repo_unique_csize = json
        .get("cache")
        .and_then(|c| c.get("stats"))
        .and_then(|s| s.get("unique_csize"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    let files_processed = stats
        .get("nfiles")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing nfiles".to_owned()))?;

    Ok(ParsedStats {
        original_size,
        compressed_size,
        deduplicated_size,
        repo_unique_csize,
        files_processed,
    })
}

/// The `type` field of a borg `--log-json` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BorgLogRecordType {
    LogMessage,
    #[serde(other)]
    Other,
}

/// The `levelname` field of a borg `--log-json` log message line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum BorgLogLevel {
    #[serde(rename = "DEBUG")]
    Debug,
    #[serde(rename = "INFO")]
    Info,
    #[serde(rename = "WARNING")]
    Warning,
    #[serde(rename = "ERROR")]
    Error,
    #[serde(rename = "CRITICAL")]
    Critical,
    #[serde(other)]
    Other,
}

impl BorgLogLevel {
    /// Whether a record at this level says something went wrong. `CRITICAL`
    /// counts: borg logs hard failures at that level, and dropping them left
    /// a failed run with nothing to show.
    fn is_diagnostic(self) -> bool {
        matches!(self, Self::Warning | Self::Error | Self::Critical)
    }
}

/// The `msgid` field of a borg `--log-json` log message line. Borg emits many
/// message ids; only `BackupFileNotFoundError` is meaningful to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum BorgMsgId {
    BackupFileNotFoundError,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct BorgLogLine {
    #[serde(rename = "type")]
    record_type: BorgLogRecordType,
    #[serde(default)]
    levelname: Option<BorgLogLevel>,
    #[serde(default)]
    msgid: Option<BorgMsgId>,
    #[serde(default)]
    message: Option<String>,
}

/// borg's `--show-rc` footer, e.g. `terminating with warning status, rc 1`.
/// It restates the exit code the agent already has and never says what caused
/// it, so it is kept out of the diagnostics a report shows.
fn is_exit_status_footer(message: &str) -> bool {
    message.starts_with("terminating with ") && message.contains(" status, rc ")
}

/// How many of borg's non-diagnostic stderr lines are kept as context for a run
/// that ends non-zero without saying why.
const MAX_CONTEXT_LINES: usize = 10;

/// How much of a single context line is kept - borg prints very long paths.
const MAX_CONTEXT_LINE_CHARS: usize = 300;

/// What borg's stderr said about a run.
#[derive(Debug, Default)]
pub(crate) struct BorgDiagnostics {
    /// The `WARNING`, `ERROR` and `CRITICAL` log records, minus the `--show-rc`
    /// footer: the messages that say what actually happened.
    pub(crate) warnings: Vec<String>,
    /// The tail of everything else borg printed - its `INFO` records and any
    /// line it did not emit as JSON (ssh notices, tracebacks). Noise while
    /// there are real diagnostics, and the only clue when there are none.
    pub(crate) context: Vec<String>,
}

impl BorgDiagnostics {
    fn push_line(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<BorgLogLine>(line) else {
            self.push_context(line.to_owned());
            return;
        };
        if record.record_type != BorgLogRecordType::LogMessage {
            return;
        }
        let Some(message) = record.message else {
            return;
        };
        match record.levelname {
            Some(BorgLogLevel::Warning | BorgLogLevel::Error | BorgLogLevel::Critical) => {
                if !is_exit_status_footer(&message) {
                    self.warnings.push(message);
                }
            }
            Some(BorgLogLevel::Info) => self.push_context(message),
            Some(BorgLogLevel::Debug | BorgLogLevel::Other) | None => {}
        }
    }

    fn push_context(&mut self, line: String) {
        if self.context.len() >= MAX_CONTEXT_LINES {
            self.context.remove(0);
        }
        self.context
            .push(truncate_chars(line, MAX_CONTEXT_LINE_CHARS));
    }
}

/// The message a report carries when borg ended with `exit_code` and no
/// diagnostic to explain it: the bare exit code tells a user nothing, so
/// whatever else borg printed is attached.
fn unexplained_exit(exit_code: i32, context: &[String]) -> String {
    let base = format!(
        "borg exited with code {exit_code} but reported no warning or error explaining why"
    );
    if context.is_empty() {
        base
    } else {
        format!("{base}; last borg output: {}", context.join(" | "))
    }
}

fn truncate_chars(mut line: String, max_chars: usize) -> String {
    if let Some((idx, _)) = line.char_indices().nth(max_chars) {
        line.truncate(idx);
        line.push_str("...");
    }
    line
}

/// Borg's stderr, one record per line. `\r` separates lines as well as `\n`:
/// borg's progress output ends its updates with a carriage return, which would
/// otherwise glue a whole run's progress and the log records printed between
/// them into a single unparsable line.
fn stderr_lines(stderr: &str) -> impl Iterator<Item = &str> {
    stderr
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Split borg's stderr into the diagnostics that explain a run and the context
/// that is left when it explains nothing.
pub(crate) fn parse_diagnostics(stderr: &str) -> BorgDiagnostics {
    stderr_lines(stderr).fold(BorgDiagnostics::default(), |mut diagnostics, line| {
        diagnostics.push_line(line);
        diagnostics
    })
}

pub(crate) fn parse_warnings(stderr: &str) -> Vec<String> {
    parse_diagnostics(stderr).warnings
}

/// How much of a failure description is kept; it is stored on the report and
/// shown in full in its Error box.
const MAX_FAILURE_CHARS: usize = 4000;

/// Renders borg's stderr into an error message. The raw buffer is a wall of
/// JSON progress records that ends up in the report's Error box, so prefer the
/// diagnostics and fall back to the tail of whatever else borg printed.
pub(crate) fn describe_borg_failure(stderr: &str) -> String {
    let diagnostics = parse_diagnostics(stderr);
    let described = diagnostics
        .warnings
        .iter()
        .chain(diagnostics.context.iter())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("; ");
    if described.is_empty() {
        "borg produced no output".to_owned()
    } else {
        truncate_chars(described, MAX_FAILURE_CHARS)
    }
}

/// Whether borg signalled a warning or worse on stderr. Unlike
/// [`parse_diagnostics`] this counts the `--show-rc` footer: an exit code of 1
/// with nothing but the footer is still borg reporting a warning, it just is
/// not saying why.
pub(crate) fn stderr_has_warnings(stderr: &str) -> bool {
    stderr_lines(stderr).any(|line| {
        let Ok(record) = serde_json::from_str::<BorgLogLine>(line) else {
            return false;
        };
        record.record_type == BorgLogRecordType::LogMessage
            && record.levelname.is_some_and(BorgLogLevel::is_diagnostic)
    })
}

/// Returns the messages for any log entries whose `msgid` indicates a backup
/// source path was not found.  These are emitted as `WARNING` by borg (rc=1)
/// but represent a configuration error - a configured source directory did
/// not exist at backup time - and must be surfaced as a hard failure rather
/// than a silent warning.
pub(crate) fn filter_file_change_warnings(
    warnings: Vec<String>,
    patterns: &[FileChangePattern],
) -> Result<Vec<String>, BackupError> {
    let mut filtered = Vec::new();
    for warning in warnings {
        let found = patterns
            .iter()
            .find(|p| glob_match::glob_match(&p.path, &warning));
        if let Some(pattern) = found {
            match &pattern.action {
                shared::types::FileChangeAction::Ignore => {}
                shared::types::FileChangeAction::Fatal => {
                    return Err(BackupError::BorgFailed(format!(
                        "file change pattern matched fatal: {warning}"
                    )));
                }
                shared::types::FileChangeAction::Warn => {
                    filtered.push(warning);
                }
            }
        } else {
            filtered.push(warning);
        }
    }
    Ok(filtered)
}

pub(crate) fn parse_source_not_found_errors(stderr: &str) -> Vec<String> {
    stderr_lines(stderr)
        .filter_map(|line| {
            let record: BorgLogLine = serde_json::from_str(line).ok()?;
            if record.record_type != BorgLogRecordType::LogMessage {
                return None;
            }
            if record.msgid == Some(BorgMsgId::BackupFileNotFoundError) {
                record.message
            } else {
                None
            }
        })
        .collect()
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
mod tests {
    use super::*;

    fn mock_borg_path() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/mock-borg/borg")
    }

    fn test_target() -> BackupTarget {
        BackupTarget {
            target_name: "test-target".to_owned(),
            schedule_id: None,
            repo_path: "backup/test".to_owned(),
            ssh_user: "borg".to_owned(),
            ssh_host: "backup-server".to_owned(),
            ssh_port: 22,
            ssh_host_key: "ssh-ed25519 AAAATEST".to_owned(),
            known_hosts_path: None,
            passphrase: "test-passphrase".to_owned(),
            hostname: "test-host".to_owned(),
            compression: Compression::Lz4,
            backup_sources: vec!["/tmp".to_owned()],
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 0,
            compact_enabled: true,
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
            hook_timeout_seconds: 60,
            skip_targets: Vec::new(),
            exclude_patterns: vec!["*.tmp".to_owned(), "/proc/*".to_owned()],
            rate_limit_kbps: None,
            ssh_auth_sock: None,
            canary_enabled: false,
            accept_relocation: false,
            file_change_patterns: Vec::new(),
        }
    }

    #[test]
    fn test_rate_limit_flag_included() {
        let mut target = test_target();
        target.rate_limit_kbps = Some(5000);

        let args = BackupEngine::borg_create_args(
            &target,
            &target.backup_sources,
            Path::new("/tmp/excludes"),
            "archive-name",
        );

        assert!(args.iter().any(|arg| arg == "--upload-ratelimit"));
        assert!(args.iter().any(|arg| arg == "5000"));
    }

    #[test]
    fn test_rate_limit_flag_absent_when_zero() {
        let mut target = test_target();
        target.rate_limit_kbps = Some(0);

        let args = BackupEngine::borg_create_args(
            &target,
            &target.backup_sources,
            Path::new("/tmp/excludes"),
            "archive-name",
        );

        assert!(!args.iter().any(|arg| arg == "--upload-ratelimit"));
    }

    #[test]
    fn borg_create_args_includes_separator_before_sources() {
        let target = test_target();
        let args = BackupEngine::borg_create_args(
            &target,
            &target.backup_sources,
            Path::new("/tmp/excludes"),
            "archive-name",
        );
        let archive_spec_pos = args.iter().position(|a| a.starts_with("::"));
        let separator_pos = args.iter().position(|a| a == "--");
        let sources_start = args
            .iter()
            .position(|a| a.as_str() == target.backup_sources[0]);

        assert_eq!(separator_pos, Some(archive_spec_pos.unwrap() + 1));
        assert_eq!(sources_start, Some(separator_pos.unwrap() + 1));
    }

    #[test]
    fn borg_create_args_no_separator_when_no_sources() {
        let target = BackupTarget {
            backup_sources: vec![],
            ..test_target()
        };
        let args = BackupEngine::borg_create_args(
            &target,
            &target.backup_sources,
            Path::new("/tmp/excludes"),
            "archive-name",
        );
        assert!(!args.iter().any(|a| a == "--"));
    }

    #[test]
    fn test_rate_limit_flag_absent() {
        let target = test_target();

        let args = BackupEngine::borg_create_args(
            &target,
            &target.backup_sources,
            Path::new("/tmp/excludes"),
            "archive-name",
        );

        assert!(!args.iter().any(|arg| arg == "--upload-ratelimit"));
    }

    #[test]
    fn borg_env_uses_pinned_known_hosts_file() {
        let known_hosts = tempfile::NamedTempFile::new().unwrap();
        let mut target = test_target();
        target.known_hosts_path = Some(known_hosts.path().to_path_buf());
        let env = BackupEngine::borg_env(&target);
        let borg_rsh = env
            .iter()
            .find(|(key, _value)| key == "BORG_RSH")
            .map(|(_key, value)| value.as_str());
        let expected = shared::ssh::borg_rsh_with_known_hosts(known_hosts.path());

        assert_eq!(borg_rsh, Some(expected.as_str()));
    }

    #[tokio::test]
    async fn test_successful_backup() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await.unwrap();

        assert_eq!(result.status, BackupStatus::Success);
        assert_eq!(result.original_size, 1_073_741_824);
        assert_eq!(result.compressed_size, 536_870_912);
        assert_eq!(result.deduplicated_size, 268_435_456);
        assert_eq!(result.repo_unique_csize, 402_653_184);
        assert_eq!(result.files_processed, 1234);
        assert!(result.error_message.is_none());
        assert_eq!(result.warnings.len(), 0);
    }

    #[tokio::test]
    async fn test_file_changed_warning() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
        );
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await.unwrap();

        assert_eq!(result.status, BackupStatus::Warning);
        assert!(result.error_message.is_some());
        assert!(
            result
                .error_message
                .as_ref()
                .unwrap()
                .contains("file changed")
        );
        assert_eq!(result.warnings.len(), 2);
        assert!(result.warnings[0].contains("file changed while we backed it up"));
        assert!(result.warnings[1].contains("file changed while we backed it up"));
    }

    #[tokio::test]
    async fn test_borg_failure() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_FAIL".to_owned(), "1".to_owned())],
        );
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            matches!(err, BackupError::BorgFailed(_)),
            "Expected BorgFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_unrelated_fatal_exit_is_not_masked_by_fatal_pattern() {
        // Regression test: an unrelated hard failure (e.g. repository error,
        // exit code 2) must surface its own message even if the stderr also
        // happens to contain a warning that matches a `fatal` file-change
        // pattern. The fatal-pattern check must only apply on the exit-code
        // paths that actually determine success/warning status.
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_FATAL_UNRELATED".to_owned(), "1".to_owned())],
        );
        let mut target = test_target();
        target.file_change_patterns = vec![FileChangePattern {
            path: "*/etc/config*".to_owned(),
            action: shared::types::FileChangeAction::Fatal,
        }];

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = &err else {
            panic!("Expected BorgFailed, got: {err:?}");
        };
        assert!(
            msg.contains("Repository ID mismatch"),
            "Expected the original borg failure message, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_pre_backup_command_success() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.pre_backup_commands = vec![HookCommand::new("true")];

        let result = engine.run_backup(&target, None, None).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);
    }

    #[tokio::test]
    async fn test_pre_backup_command_failure() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.pre_backup_commands = vec![HookCommand::new("false")];

        let result = engine.run_backup(&target, None, None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackupError::BorgFailed(_)));
    }

    #[tokio::test]
    async fn test_pre_backup_command_failure_includes_stderr() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.pre_backup_commands =
            vec![HookCommand::new("echo 'connection refused' >&2 && exit 1")];

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = err else {
            panic!("expected BorgFailed, got {err:?}");
        };
        assert!(
            msg.contains("connection refused"),
            "error message should include stderr: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pre_backup_command_respects_configured_timeout() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.hook_timeout_seconds = 1;
        target.pre_backup_commands = vec![HookCommand::new("sleep 5")];

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = err else {
            panic!("expected BorgFailed, got {err:?}");
        };
        assert!(
            msg.contains("timed out after 1 seconds"),
            "error message should reflect the configured timeout: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pre_backup_command_allows_longer_configured_timeout() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.hook_timeout_seconds = 5;
        target.pre_backup_commands = vec![HookCommand::new("sleep 1")];

        let result = engine.run_backup(&target, None, None).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);
    }

    #[tokio::test]
    async fn test_pre_backup_command_timeout_overrides_schedule_default() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.hook_timeout_seconds = 60;
        target.pre_backup_commands = vec![HookCommand {
            command: "sleep 5".to_owned(),
            timeout_seconds: Some(1),
        }];

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = err else {
            panic!("expected BorgFailed, got {err:?}");
        };
        assert!(
            msg.contains("timed out after 1 seconds"),
            "the command's own timeout should win over the schedule default: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pre_backup_command_timeout_can_exceed_schedule_default() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.hook_timeout_seconds = 1;
        target.pre_backup_commands = vec![HookCommand {
            command: "sleep 2".to_owned(),
            timeout_seconds: Some(30),
        }];

        let result = engine.run_backup(&target, None, None).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);
    }

    #[tokio::test]
    async fn test_post_backup_command_uses_its_own_timeout() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.hook_timeout_seconds = 60;
        target.post_backup_commands = vec![HookCommand {
            command: "sleep 5".to_owned(),
            timeout_seconds: Some(1),
        }];

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = err else {
            panic!("expected BorgFailed, got {err:?}");
        };
        assert!(
            msg.contains("post-backup hook command timed out after 1 seconds"),
            "post-backup hooks honour their own timeout: {msg}"
        );
    }

    #[tokio::test]
    async fn test_skip_targets() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.skip_targets = vec![target.target_name.clone()];

        let result = engine.run_backup(&target, None, None).await;
        assert!(matches!(result.unwrap_err(), BackupError::Skipped(_)));
    }

    #[tokio::test]
    async fn test_compression_arg() {
        assert_eq!(BackupEngine::compression_arg(&Compression::None), "none");
        assert_eq!(BackupEngine::compression_arg(&Compression::Lz4), "lz4");
        assert_eq!(
            BackupEngine::compression_arg(&Compression::Zstd { level: 3 }),
            "zstd,3"
        );
        assert_eq!(
            BackupEngine::compression_arg(&Compression::Zlib { level: 6 }),
            "zlib,6"
        );
    }

    #[tokio::test]
    async fn test_parse_json_stats() {
        let json = r#"{
            "archive": {
                "name": "test",
                "stats": {
                    "original_size": 100,
                    "compressed_size": 80,
                    "deduplicated_size": 50,
                    "nfiles": 42
                }
            }
        }"#;

        let stats = parse_json_stats(json.as_bytes()).unwrap();
        assert_eq!(stats.original_size, 100);
        assert_eq!(stats.compressed_size, 80);
        assert_eq!(stats.deduplicated_size, 50);
        assert_eq!(stats.repo_unique_csize, 0);
        assert_eq!(stats.files_processed, 42);
    }

    #[tokio::test]
    async fn test_parse_json_stats_with_cache() {
        let json = r#"{
            "archive": {
                "name": "test",
                "stats": {
                    "original_size": 100,
                    "compressed_size": 80,
                    "deduplicated_size": 50,
                    "nfiles": 42
                }
            },
            "cache": {
                "stats": {
                    "total_size": 1000,
                    "total_csize": 800,
                    "unique_size": 400,
                    "unique_csize": 300,
                    "total_unique_chunks": 10,
                    "total_chunks": 40
                }
            }
        }"#;

        let stats = parse_json_stats(json.as_bytes()).unwrap();
        assert_eq!(stats.original_size, 100);
        assert_eq!(stats.compressed_size, 80);
        assert_eq!(stats.deduplicated_size, 50);
        assert_eq!(stats.repo_unique_csize, 300);
        assert_eq!(stats.files_processed, 42);
    }

    #[test]
    fn test_parse_warnings_json() {
        let stderr = [
            concat!(
                r#"{"type": "log_message", "time": 1704067200, "#,
                r#""levelname": "WARNING", "name": "borg.archive", "#,
                r#""msgid": "FileChangedWarning", "#,
                r#""message": "/tmp/test.log: file changed"}"#,
            ),
            concat!(
                r#"{"type": "log_message", "time": 1704067200, "#,
                r#""levelname": "INFO", "name": "borg.archive", "#,
                r#""message": "some info"}"#,
            ),
            concat!(
                r#"{"type": "log_message", "time": 1704067200, "#,
                r#""levelname": "ERROR", "name": "borg.archive", "#,
                r#""msgid": "BackupError", "#,
                r#""message": "some error"}"#,
            ),
        ]
        .join("\n");

        let warnings = parse_warnings(&stderr);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("file changed"));
        assert_eq!(warnings[1], "some error");
    }

    #[test]
    fn test_parse_warnings_ignores_non_log_types() {
        let stderr = [
            r#"{"type": "archive_progress", "original_size": 100}"#,
            r#"{"type": "file_status", "status": "A", "path": "/a"}"#,
        ]
        .join("\n");

        let warnings = parse_warnings(&stderr);
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_stderr_has_warnings_json() {
        let with_warning = r#"{"type": "log_message", "levelname": "WARNING", "message": "oops"}"#;
        assert!(stderr_has_warnings(with_warning));

        let without_warning = r#"{"type": "log_message", "levelname": "INFO", "message": "ok"}"#;
        assert!(!stderr_has_warnings(without_warning));

        assert!(!stderr_has_warnings("plain text warning"));
    }

    #[test]
    fn test_parse_source_not_found_errors_detects_msgid() {
        let stderr = concat!(
            r#"{"type": "log_message", "time": 1704067200, "levelname": "WARNING", "#,
            r#""name": "borg.archiver", "msgid": "BackupFileNotFoundError", "#,
            r#""message": "/mnt/photos: stat: [Errno 2] No such file or directory"}"#,
        );
        let errors = parse_source_not_found_errors(stderr);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("/mnt/photos"));
    }

    #[test]
    fn test_parse_source_not_found_errors_ignores_other_warnings() {
        let stderr = concat!(
            r#"{"type": "log_message", "levelname": "WARNING", "#,
            r#""msgid": "FileChangedWarning", "message": "/tmp/test.log: file changed"}"#,
        );
        let errors = parse_source_not_found_errors(stderr);
        assert_eq!(errors.len(), 0);
    }

    #[tokio::test]
    async fn test_missing_backup_source_is_error_not_warning() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![(
                "MOCK_BORG_SIMULATE_SOURCE_NOT_FOUND".to_owned(),
                "1".to_owned(),
            )],
        );
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        let BackupError::BorgFailed(msg) = err else {
            panic!("expected BorgFailed, got {err:?}");
        };
        assert!(
            msg.contains("backup source(s) not found"),
            "error should mention missing source: {msg}"
        );
        assert!(
            msg.contains("/mnt/missing"),
            "error should include the missing path: {msg}"
        );
    }

    #[tokio::test]
    async fn test_all_zero_retention_skips_prune() {
        let log_file = tempfile::NamedTempFile::new().unwrap();
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![(
                "MOCK_BORG_LOG".to_owned(),
                log_file.path().to_string_lossy().into_owned(),
            )],
        );
        let mut target = test_target();
        target.keep_hourly = 0;
        target.keep_daily = 0;
        target.keep_weekly = 0;
        target.keep_monthly = 0;
        target.keep_yearly = 0;

        let result = engine.run_backup(&target, None, None).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);

        let log = std::fs::read_to_string(log_file.path()).unwrap();
        assert!(
            !log.contains("prune"),
            "prune should not be called when all retention values are zero, but log contains: \
             {log}"
        );
    }

    #[tokio::test]
    async fn test_partial_retention_only_includes_nonzero_keep_flags() {
        let log_file = tempfile::NamedTempFile::new().unwrap();
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![(
                "MOCK_BORG_LOG".to_owned(),
                log_file.path().to_string_lossy().into_owned(),
            )],
        );
        let mut target = test_target();
        target.keep_hourly = 0;
        target.keep_daily = 7;
        target.keep_weekly = 0;
        target.keep_monthly = 3;
        target.keep_yearly = 0;

        let result = engine.run_backup(&target, None, None).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);

        let log = std::fs::read_to_string(log_file.path()).unwrap();
        assert!(log.contains("prune"), "prune should be called");
        assert!(
            log.contains("--keep-daily"),
            "keep-daily flag expected in: {log}"
        );
        assert!(
            log.contains("--keep-monthly"),
            "keep-monthly flag expected in: {log}"
        );
        assert!(
            !log.contains("--keep-hourly"),
            "keep-hourly should not appear when zero, but found in: {log}"
        );
        assert!(
            !log.contains("--keep-weekly"),
            "keep-weekly should not appear when zero, but found in: {log}"
        );
        assert!(
            !log.contains("--keep-yearly"),
            "keep-yearly should not appear when zero, but found in: {log}"
        );
    }

    #[test]
    fn test_filter_file_change_warnings_passthrough() {
        let warnings = vec!["file changed".to_owned(), "other warning".to_owned()];
        let result = filter_file_change_warnings(warnings, &[]).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_file_change_warnings_ignore() {
        // Note: glob-match * does not match /, so patterns match warning message suffixes
        let patterns = vec![FileChangePattern {
            path: "*test.log: file changed".to_owned(),
            action: shared::types::FileChangeAction::Ignore,
        }];
        let warnings = vec![
            "test.log: file changed".to_owned(),
            "other warning".to_owned(),
        ];
        let result = filter_file_change_warnings(warnings, &patterns).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("other warning"));
    }

    #[test]
    fn parse_diagnostics_drops_the_show_rc_footer() {
        let stderr = [
            concat!(
                r#"{"type": "log_message", "levelname": "WARNING", "#,
                r#""message": "/tmp/test.log: file changed"}"#,
            ),
            concat!(
                r#"{"type": "log_message", "levelname": "WARNING", "#,
                r#""name": "borg.archiver", "#,
                r#""message": "terminating with warning status, rc 1"}"#,
            ),
        ]
        .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.warnings, vec!["/tmp/test.log: file changed"]);
    }

    #[test]
    fn parse_diagnostics_keeps_critical_records() {
        let stderr = concat!(
            r#"{"type": "log_message", "levelname": "CRITICAL", "#,
            r#""message": "Repository /repo does not exist."}"#,
        );

        let diagnostics = parse_diagnostics(stderr);

        assert_eq!(
            diagnostics.warnings,
            vec!["Repository /repo does not exist."]
        );
    }

    #[test]
    fn parse_diagnostics_collects_context_from_non_json_and_info_lines() {
        let stderr = [
            r#"{"type": "archive_progress", "original_size": 100}"#,
            r#"{"type": "log_message", "levelname": "INFO", "message": "Creating archive"}"#,
            "Warning: Permanently added 'storage' to the list of known hosts.",
        ]
        .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.warnings, [] as [String; 0]);
        assert_eq!(
            diagnostics.context,
            vec![
                "Creating archive",
                "Warning: Permanently added 'storage' to the list of known hosts."
            ]
        );
    }

    #[test]
    fn parse_diagnostics_keeps_only_the_tail_of_the_context() {
        let stderr = (0..MAX_CONTEXT_LINES + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.context.len(), MAX_CONTEXT_LINES);
        assert_eq!(diagnostics.context[0], "line 5");
        assert_eq!(
            diagnostics.context[MAX_CONTEXT_LINES - 1],
            format!("line {}", MAX_CONTEXT_LINES + 4)
        );
    }

    #[test]
    fn parse_diagnostics_truncates_an_overlong_context_line() {
        let stderr = "x".repeat(MAX_CONTEXT_LINE_CHARS + 50);

        let diagnostics = parse_diagnostics(&stderr);

        let line = &diagnostics.context[0];
        assert_eq!(line.chars().count(), MAX_CONTEXT_LINE_CHARS + 3);
        assert!(line.ends_with("..."));
    }

    #[test]
    fn describe_borg_failure_prefers_diagnostics_over_raw_json() {
        let stderr = [
            r#"{"type": "archive_progress", "original_size": 100}"#,
            r#"{"type": "log_message", "levelname": "ERROR", "message": "Connection closed"}"#,
            "borg: Fatal: Repository ID mismatch.",
        ]
        .join("\n");

        let described = describe_borg_failure(&stderr);

        assert_eq!(
            described,
            "Connection closed; borg: Fatal: Repository ID mismatch."
        );
    }

    #[tokio::test]
    async fn unexplained_warning_exit_reports_why_it_is_flagged() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![(
                "MOCK_BORG_SIMULATE_UNEXPLAINED_WARNING".to_owned(),
                "1".to_owned(),
            )],
        );
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await.unwrap();

        assert_eq!(result.status, BackupStatus::Warning);
        assert_eq!(result.warnings.len(), 1);
        let warning = &result.warnings[0];
        assert!(
            !warning.contains("terminating with warning status"),
            "the --show-rc footer explains nothing: {warning}"
        );
        assert!(
            warning.contains("borg exited with code 1"),
            "the warning should name the exit code: {warning}"
        );
        assert!(
            warning.contains("Creating archive at"),
            "the warning should carry borg's last output: {warning}"
        );
        assert_eq!(result.error_message.as_ref(), Some(warning));
    }

    #[tokio::test]
    async fn warnings_suppressed_by_ignore_patterns_leave_a_successful_run() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
        );
        let mut target = test_target();
        target.file_change_patterns = vec![FileChangePattern {
            path: "**/*: file changed while we backed it up".to_owned(),
            action: shared::types::FileChangeAction::Ignore,
        }];

        let result = engine.run_backup(&target, None, None).await.unwrap();

        assert_eq!(result.status, BackupStatus::Success);
        assert_eq!(result.warnings, [] as [String; 0]);
        assert!(result.error_message.is_none());
    }

    #[test]
    fn test_filter_file_change_warnings_fatal() {
        let patterns = vec![FileChangePattern {
            path: "*test.conf: file changed".to_owned(),
            action: shared::types::FileChangeAction::Fatal,
        }];
        let warnings = vec![
            "test.conf: file changed".to_owned(),
            "other warning".to_owned(),
        ];
        let result = filter_file_change_warnings(warnings, &patterns);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_borg_timeout_is_only_applied_when_configured() {
        let engine = BackupEngine::with_config_and_timeout(
            mock_borg_path(),
            vec![
                ("MOCK_BORG_SLEEP_SUBCOMMAND".to_owned(), "prune".to_owned()),
                ("MOCK_BORG_SLEEP_SECS".to_owned(), "0.1".to_owned()),
            ],
            Some(Duration::from_millis(10)),
        );
        let target = test_target();

        let result = engine.run_backup(&target, None, None).await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("borg prune"));
        let BackupError::Timeout { seconds, command } = err else {
            panic!("expected timeout error");
        };
        assert_eq!(seconds, 1);
        assert!(command.contains(" borg prune "));
    }

    #[tokio::test]
    async fn write_canary_creates_file_in_first_directory_source() {
        let dir = tempfile::tempdir().unwrap();
        let sources = vec![
            "!/excluded".to_owned(),
            "/definitely-not-a-real-dir-assimilate-test".to_owned(),
            dir.path().to_string_lossy().into_owned(),
        ];

        let canary = BackupEngine::write_canary(&sources).await.unwrap();

        assert_eq!(canary.canary_path, dir.path().join(".assimilate-canary"));
        let content = tokio::fs::read_to_string(&canary.canary_path)
            .await
            .unwrap();
        assert_eq!(content, canary.expected_content);
        assert!(content.contains(&canary.nonce));
    }

    #[tokio::test]
    async fn write_canary_errors_without_usable_directory() {
        let sources = vec![
            "!/excluded".to_owned(),
            "/definitely-not-a-real-dir-assimilate-test".to_owned(),
        ];

        let err = BackupEngine::write_canary(&sources).await.unwrap_err();
        assert!(matches!(err, BackupError::BorgFailed(_)));
    }

    #[tokio::test]
    async fn cleanup_canary_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let canary_path = dir.path().join(".assimilate-canary");
        tokio::fs::write(&canary_path, "content").await.unwrap();
        let token = CanaryToken {
            nonce: "nonce".to_owned(),
            canary_path: canary_path.clone(),
            expected_content: "content".to_owned(),
        };

        BackupEngine::cleanup_canary(&token).await;

        assert!(!canary_path.exists());
    }

    #[tokio::test]
    async fn cleanup_canary_tolerates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let token = CanaryToken {
            nonce: "nonce".to_owned(),
            canary_path: dir.path().join("missing-canary"),
            expected_content: "content".to_owned(),
        };

        BackupEngine::cleanup_canary(&token).await;
    }
}
