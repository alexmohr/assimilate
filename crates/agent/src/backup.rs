// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::Utc;
use shared::types::{BackupStatus, Compression};
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

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
    #[error("borg command timed out after {0} seconds")]
    Timeout(u64),
}

pub struct BackupTarget {
    pub target_name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub passphrase: String,
    pub hostname: String,
    pub compression: Compression,
    pub backup_sources: Vec<String>,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub keep_yearly: u32,
    pub compact_enabled: bool,
    pub pre_backup_commands: Vec<String>,
    pub post_backup_commands: Vec<String>,
    pub skip_targets: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub rate_limit_kbps: Option<u32>,
    pub ssh_auth_sock: Option<PathBuf>,
    pub canary_enabled: bool,
    pub accept_relocation: bool,
}

#[derive(Debug)]
pub struct BackupResult {
    pub status: BackupStatus,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub warnings: Vec<String>,
    pub archive_name: Option<String>,
}

pub struct BackupEngine {
    borg_binary: PathBuf,
    extra_env: Vec<(String, String)>,
}

impl BackupEngine {
    pub fn new() -> Self {
        let borg_binary =
            std::env::var("BORG_BINARY").map_or_else(|_| PathBuf::from("borg"), PathBuf::from);
        Self {
            borg_binary,
            extra_env: Vec::new(),
        }
    }

    #[cfg(test)]
    fn with_config(borg_binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            borg_binary,
            extra_env,
        }
    }

    pub async fn run_backup(&self, target: &BackupTarget) -> Result<BackupResult, BackupError> {
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
            self.run_hook_command(cmd, "pre-backup").await?;
        }

        let exclude_file = Self::write_exclude_file(&target.exclude_patterns)?;

        let create_result = self
            .run_borg_create(target, &target.backup_sources, exclude_file.path())
            .await?;

        self.run_borg_prune(target).await?;

        if target.compact_enabled {
            self.run_borg_compact(target).await?;
        }

        for cmd in &target.post_backup_commands {
            self.run_hook_command(cmd, "post-backup").await?;
        }

        let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

        Ok(BackupResult {
            status: create_result.status,
            original_size: create_result.original_size,
            compressed_size: create_result.compressed_size,
            deduplicated_size: create_result.deduplicated_size,
            files_processed: create_result.files_processed,
            duration_secs,
            error_message: create_result.error_message,
            warnings: create_result.warnings,
            archive_name: Some(create_result.archive_name),
        })
    }

    async fn run_hook_command(&self, cmd: &str, label: &str) -> Result<(), BackupError> {
        info!("Running {label} command: {cmd}");

        let output = tokio::time::timeout(
            Duration::from_mins(1),
            Command::new("sh").arg("-c").arg(cmd).output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(60))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);
            warn!("{label} command failed with exit code {exit_code}: {stderr}");
            return Err(BackupError::Skipped(format!(
                "{label} command exited with code {exit_code}"
            )));
        }

        Ok(())
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
        let repo_url = format!(
            "ssh://{user}@{host}:{port}/{path}",
            user = target.ssh_user,
            host = target.ssh_host,
            port = target.ssh_port,
            path = target.repo_path,
        );

        let mut env = vec![
            ("BORG_REPO".to_owned(), repo_url),
            ("BORG_PASSPHRASE".to_owned(), target.passphrase.clone()),
            ("BORG_HOST_ID".to_owned(), target.hostname.clone()),
            (
                "BORG_RSH".to_owned(),
                "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_owned(),
            ),
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
    ) -> Result<CreateResult, BackupError> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let archive_name = format!("{hostname}-{now}", hostname = target.hostname);

        let args = Self::borg_create_args(target, backup_sources, exclude_file, &archive_name);

        let env_vars = Self::borg_env(target);

        info!("Running borg create for archive {archive_name}");

        let output = Command::new(&self.borg_binary)
            .args(&args)
            .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .output()
            .await?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);

        match exit_code {
            0 => {
                let stats = parse_json_stats(&output.stdout)?;
                Ok(CreateResult {
                    status: BackupStatus::Success,
                    original_size: stats.original_size,
                    compressed_size: stats.compressed_size,
                    deduplicated_size: stats.deduplicated_size,
                    files_processed: stats.files_processed,
                    error_message: None,
                    warnings: Vec::new(),
                    archive_name,
                })
            }
            1 if stderr.contains("file changed while we") => {
                warn!("Borg reported file-changed warning: {stderr}");
                let stats = parse_json_stats(&output.stdout)?;
                let warnings = parse_warnings(&stderr);
                Ok(CreateResult {
                    status: BackupStatus::Warning,
                    original_size: stats.original_size,
                    compressed_size: stats.compressed_size,
                    deduplicated_size: stats.deduplicated_size,
                    files_processed: stats.files_processed,
                    error_message: Some(stderr.into_owned()),
                    warnings,
                    archive_name,
                })
            }
            _ => Err(BackupError::BorgFailed(format!(
                "borg create exited with code {exit_code}: {stderr}"
            ))),
        }
    }

    fn borg_create_args(
        target: &BackupTarget,
        backup_sources: &[String],
        exclude_file: &Path,
        archive_name: &str,
    ) -> Vec<String> {
        let mut args = vec![
            "create".to_owned(),
            "--lock-wait".to_owned(),
            "600".to_owned(),
            "--show-rc".to_owned(),
            "--json".to_owned(),
            "--compression".to_owned(),
            Self::compression_arg(&target.compression),
            "--exclude-caches".to_owned(),
            "--exclude-if-present".to_owned(),
            ".nobackup".to_owned(),
            "--exclude-from".to_owned(),
            exclude_file.to_string_lossy().into_owned(),
        ];

        if let Some(rate_limit_kbps) = target.rate_limit_kbps {
            args.push("--remote-ratelimit".to_owned());
            args.push(rate_limit_kbps.to_string());
        }

        args.push(format!("::{archive_name}"));

        for source in backup_sources {
            args.push(source.clone());
        }

        args
    }

    async fn run_borg_prune(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let glob_pattern = format!("*{hostname}-*", hostname = target.hostname);
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
            "-a",
            &glob_pattern,
            "--keep-daily",
            &keep_daily,
            "--keep-weekly",
            &keep_weekly,
            "--keep-monthly",
            &keep_monthly,
        ];

        if target.keep_yearly > 0 {
            args.push("--keep-yearly");
            args.push(&keep_yearly);
        }

        let env_vars = Self::borg_env(target);

        info!("Running borg prune");

        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args(&args)
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg prune exited with code {exit_code}: {stderr}"
            )));
        }

        Ok(())
    }

    async fn run_borg_compact(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = Self::borg_env(target);

        info!("Running borg compact");

        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args(["compact", "--lock-wait", "600"])
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg compact exited with code {exit_code}: {stderr}"
            )));
        }

        Ok(())
    }

    pub async fn run_check(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = Self::borg_env(target);

        info!(target = %target.target_name, "Running borg check");

        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args(["check", "--lock-wait", "600", "--show-rc"])
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code >= 2 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg check exited with code {exit_code}: {stderr}"
            )));
        }

        if exit_code == 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("borg check returned warnings: {stderr}");
        }

        info!(target = %target.target_name, "borg check completed");
        Ok(())
    }

    pub async fn run_verify(&self, target: &BackupTarget) -> Result<i64, BackupError> {
        let env_vars = Self::borg_env(target);
        let hostname = &target.hostname;

        info!(target = %target.target_name, "Running borg extract --dry-run (verify)");

        let glob_pattern = format!("*{hostname}-*");
        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args([
                    "list",
                    "--lock-wait",
                    "600",
                    "--short",
                    "--last",
                    "1",
                    "-a",
                    &glob_pattern,
                ])
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

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
        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args(["extract", "--dry-run", "--lock-wait", "600", &archive_ref])
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

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

    pub fn write_canary(backup_sources: &[String]) -> Result<CanaryToken, BackupError> {
        let source_dir = backup_sources
            .iter()
            .find(|s| !s.starts_with('!') && Path::new(s).is_dir())
            .ok_or_else(|| {
                BackupError::BorgFailed(
                    "no usable backup source directory for canary file".to_owned(),
                )
            })?;

        let nonce = Uuid::new_v4().to_string();
        let canary_path = Path::new(source_dir).join(".assimilate-canary");
        let content = format!("{{\"nonce\":\"{nonce}\"}}");

        std::fs::write(&canary_path, &content)?;
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
    ) -> Result<String, BackupError> {
        let env_vars = Self::borg_env(target);
        let hostname = &target.hostname;

        let glob_pattern = format!("*{hostname}-*");
        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args([
                    "list",
                    "--lock-wait",
                    "600",
                    "--short",
                    "--last",
                    "1",
                    "-a",
                    &glob_pattern,
                ])
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BackupError::BorgFailed(format!(
                "borg list for canary exited with code {exit_code}: {stderr}"
            )));
        }

        let archive_name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if archive_name.is_empty() {
            return Err(BackupError::BorgFailed(
                "no archives found for canary verification".to_owned(),
            ));
        }

        let extract_dir = tempfile::tempdir()?;
        let archive_ref = format!("::{archive_name}");

        let canary_relative = canary
            .canary_path
            .strip_prefix("/")
            .unwrap_or(&canary.canary_path)
            .to_string_lossy()
            .into_owned();

        let output = tokio::time::timeout(
            Duration::from_mins(5),
            Command::new(&self.borg_binary)
                .args([
                    "extract",
                    "--lock-wait",
                    "600",
                    &archive_ref,
                    &canary_relative,
                ])
                .current_dir(extract_dir.path())
                .envs(env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .output(),
        )
        .await
        .map_err(|_| BackupError::Timeout(300))??;

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

        info!(archive = %archive_name, "canary verification passed");
        Ok(archive_name)
    }

    pub fn cleanup_canary(canary: &CanaryToken) {
        if let Err(e) = std::fs::remove_file(&canary.canary_path) {
            warn!(path = %canary.canary_path.display(), error = %e, "failed to remove canary file");
        }
    }
}

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
    files_processed: i64,
    error_message: Option<String>,
    warnings: Vec<String>,
    archive_name: String,
}

struct ParsedStats {
    original_size: i64,
    compressed_size: i64,
    deduplicated_size: i64,
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

    let files_processed = stats
        .get("nfiles")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing nfiles".to_owned()))?;

    Ok(ParsedStats {
        original_size,
        compressed_size,
        deduplicated_size,
        files_processed,
    })
}

fn parse_warnings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|line| line.contains("file changed") || line.starts_with("WARNING"))
        .map(std::borrow::ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
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
            repo_path: "backup/test".to_owned(),
            ssh_user: "borg".to_owned(),
            ssh_host: "backup-server".to_owned(),
            ssh_port: 22,
            passphrase: "test-passphrase".to_owned(),
            hostname: "test-host".to_owned(),
            compression: Compression::Lz4,
            backup_sources: vec!["/tmp".to_owned()],
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 0,
            compact_enabled: true,
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
            skip_targets: Vec::new(),
            exclude_patterns: vec!["*.tmp".to_owned(), "/proc/*".to_owned()],
            rate_limit_kbps: None,
            ssh_auth_sock: None,
            canary_enabled: false,
            accept_relocation: false,
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

        assert!(args.iter().any(|arg| arg == "--remote-ratelimit"));
        assert!(args.iter().any(|arg| arg == "5000"));
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

        assert!(!args.iter().any(|arg| arg == "--remote-ratelimit"));
    }

    #[tokio::test]
    async fn test_successful_backup() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let target = test_target();

        let result = engine.run_backup(&target).await.unwrap();

        assert_eq!(result.status, BackupStatus::Success);
        assert_eq!(result.original_size, 1_073_741_824);
        assert_eq!(result.compressed_size, 536_870_912);
        assert_eq!(result.deduplicated_size, 268_435_456);
        assert_eq!(result.files_processed, 1234);
        assert!(result.error_message.is_none());
        assert!(result.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_file_changed_warning() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
        );
        let target = test_target();

        let result = engine.run_backup(&target).await.unwrap();

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
        assert!(result.warnings[0].contains("file changed"));
        assert!(result.warnings[1].starts_with("WARNING"));
    }

    #[tokio::test]
    async fn test_borg_failure() {
        let engine = BackupEngine::with_config(
            mock_borg_path(),
            vec![("MOCK_BORG_FAIL".to_owned(), "1".to_owned())],
        );
        let target = test_target();

        let result = engine.run_backup(&target).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            matches!(err, BackupError::BorgFailed(_)),
            "Expected BorgFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_pre_backup_command_success() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.pre_backup_commands = vec!["true".to_owned()];

        let result = engine.run_backup(&target).await.unwrap();
        assert_eq!(result.status, BackupStatus::Success);
    }

    #[tokio::test]
    async fn test_pre_backup_command_failure() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.pre_backup_commands = vec!["false".to_owned()];

        let result = engine.run_backup(&target).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackupError::Skipped(_)));
    }

    #[tokio::test]
    async fn test_skip_targets() {
        let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
        let mut target = test_target();
        target.skip_targets = vec![target.target_name.clone()];

        let result = engine.run_backup(&target).await;
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
        assert_eq!(stats.files_processed, 42);
    }
}
