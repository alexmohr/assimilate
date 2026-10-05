// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

use tracing::{error, info, warn};
use uuid::Uuid;

use super::{
    BackupEngine, BackupError, BackupTarget, CanaryResult, CanaryToken, borg_env,
    warning_status_log,
};

impl BackupEngine {
    pub(super) async fn run_borg_prune(&self, target: &BackupTarget) -> Result<(), BackupError> {
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

        let env_vars = borg_env(target);

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
            warn!("{}", warning_status_log("prune", exit_code, &stderr));
        }

        Ok(())
    }

    pub(super) async fn run_borg_compact(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = borg_env(target);

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
            warn!("{}", warning_status_log("compact", exit_code, &stderr));
        }

        Ok(())
    }

    pub async fn run_check(&self, target: &BackupTarget) -> Result<(), BackupError> {
        let env_vars = borg_env(target);

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
            warn!("{}", warning_status_log("check", exit_code, &stderr));
        }

        info!(target = %target.target_name, "borg check completed");
        Ok(())
    }

    pub async fn run_verify(&self, target: &BackupTarget) -> Result<i64, BackupError> {
        let env_vars = borg_env(target);
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
        let env_vars = borg_env(target);
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
