// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{ffi::OsStr, path::Path};

use chrono::Utc;
use shared::{
    borg::log_json::{BorgDiagnostics, parse_diagnostics},
    types::{BackupStatus, Compression, build_repo_url},
};
use tokio::sync::mpsc;
use tracing::{error, info};

use super::{
    BackupEngine, BackupError, BackupTarget, CreateResult, borg_env,
    parse::{
        describe_borg_failure, filter_file_change_warnings, parse_json_stats,
        parse_source_not_found_errors, stderr_has_warnings, summarize_warnings,
        suppressed_error_exit, unexplained_exit, warning_kind,
    },
};
use crate::borg::Borg;

impl BackupEngine {
    pub(super) fn write_exclude_file(
        patterns: &[String],
    ) -> Result<tempfile::NamedTempFile, BackupError> {
        Ok(shared::borg::env::write_exclude_file(patterns)?)
    }

    /// Writes a borg patterns file rescuing `patterns` from the exclude list -
    /// each line is prefixed `+` (include), so it is checked, in order,
    /// before `--exclude-from`'s patterns and wins first-match-wins. Returns
    /// `None` when there is nothing to rescue, so a schedule with no include
    /// patterns runs the exact command it always has.
    pub(super) fn write_include_patterns_file(
        patterns: &[String],
    ) -> Result<Option<tempfile::NamedTempFile>, BackupError> {
        Ok(shared::borg::env::write_include_patterns_file(patterns)?)
    }

    pub(super) fn compression_arg(compression: &Compression) -> String {
        compression.to_string()
    }

    pub(super) async fn run_borg_create(
        &self,
        target: &BackupTarget,
        backup_sources: &[String],
        exclude_file: &Path,
        include_file: Option<&Path>,
        log_tx: Option<mpsc::Sender<String>>,
    ) -> Result<CreateResult, BackupError> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let archive_name = format!("{hostname}-{now}", hostname = target.hostname);

        let args = Self::borg_create_args(
            target,
            backup_sources,
            exclude_file,
            include_file,
            &archive_name,
        );
        let borg_command = Self::format_command_string(target, &args);

        let env_vars = borg_env(target);

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
                let BorgDiagnostics {
                    warnings,
                    file_changed,
                    ..
                } = parse_diagnostics(&stderr);
                let warnings = filter_file_change_warnings(warnings, &target.file_change_patterns)?;
                let status = if warnings.is_empty() {
                    BackupStatus::Success
                } else {
                    BackupStatus::Warning
                };
                let warning_kind = warning_kind(&warnings, &file_changed);
                Ok(CreateResult {
                    status,
                    stats,
                    error_message: None,
                    warnings,
                    warning_kind,
                    archive_name,
                    borg_command,
                })
            }
            1 if stderr_has_warnings(&stderr) => {
                let BorgDiagnostics {
                    warnings: reported_warnings,
                    context,
                    error_level,
                    file_changed,
                } = parse_diagnostics(&stderr);
                let reported = reported_warnings.len();
                let mut warnings =
                    filter_file_change_warnings(reported_warnings, &target.file_change_patterns)?;
                // An `ignore` pattern matches message text alone, so a broad one
                // can swallow an error-level record as easily as the file-change
                // warning it was written for. That may never pass silently -
                // neither as a success nor as a report of the warnings that
                // happened to survive alongside it.
                if error_level.iter().any(|m| !warnings.contains(m)) {
                    warnings.push(suppressed_error_exit(exit_code));
                }
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
                let warning_kind = warning_kind(&warnings, &file_changed);
                let error_message = summarize_warnings(&warnings, reported);
                let stats = parse_json_stats(&output.stdout)?;
                Ok(CreateResult {
                    status,
                    stats,
                    error_message,
                    warnings,
                    warning_kind,
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
    /// placeholder for the transient exclude-from (and, when the target has
    /// include patterns, patterns-from) temp file path.
    pub fn preview_create_command(target: &BackupTarget) -> String {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let archive_name = format!("{hostname}-{now}", hostname = target.hostname);
        let exclude_placeholder = std::path::Path::new("<exclude-file>");
        let include_placeholder = std::path::Path::new("<include-file>");
        let include_file = (!target.include_patterns.is_empty()).then_some(include_placeholder);
        let args = Self::borg_create_args(
            target,
            &target.backup_sources,
            exclude_placeholder,
            include_file,
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

    pub(super) fn format_command_slice(target: &BackupTarget, args: &[&str]) -> String {
        let repo_url = build_repo_url(
            &target.ssh_user,
            &target.ssh_host,
            target.ssh_port,
            &target.repo_path,
        );
        let args_str = args.join(" ");
        format!("BORG_REPO={repo_url} borg {args_str}")
    }

    pub(super) fn borg_create_args(
        target: &BackupTarget,
        backup_sources: &[String],
        exclude_file: &Path,
        include_file: Option<&Path>,
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
        ];

        // Listed before --exclude-from: borg tests patterns in the order they
        // are given on the command line, first match wins, so an include here
        // rescues a path a later, broader exclude would otherwise drop.
        if let Some(include_file) = include_file {
            flags.push("--patterns-from".to_owned());
            flags.push(include_file.to_string_lossy().into_owned());
        }

        flags.push("--exclude-from".to_owned());
        flags.push(exclude_file.to_string_lossy().into_owned());

        if let Some(rate_limit_kbps) = target.rate_limit_kbps.filter(|&kbps| kbps > 0) {
            flags.push("--upload-ratelimit".to_owned());
            flags.push(rate_limit_kbps.to_string());
        }

        flags.push(format!("::{archive_name}"));

        Borg::args_with_positional(&flags, backup_sources)
    }
}
