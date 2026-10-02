// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use chrono::Utc;
use shared::{protocol::AgentToServer, types::DryRunFile};
use tokio::sync::mpsc;
use tracing::{error, info};

use super::{DryRunTaskParams, FreeTaskContext, transport::setup_ssh_forward};
use crate::borg::Borg;

/// Writes the exclude and (when present) include patterns files a dry-run
/// preview needs, reporting `OperationFailed` and returning `None` if either
/// write fails.
pub(super) async fn write_dry_run_pattern_files(
    exclude_patterns: &[String],
    include_patterns: &[String],
    request_id: &str,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) -> Option<(tempfile::NamedTempFile, Option<tempfile::NamedTempFile>)> {
    let exclude_file = match shared::borg::env::write_exclude_file(exclude_patterns) {
        Ok(f) => f,
        Err(e) => {
            let msg = AgentToServer::OperationFailed {
                request_id: request_id.to_owned(),
                error: format!("failed to write exclude file: {e}"),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return None;
        }
    };
    let include_file = match shared::borg::env::write_include_patterns_file(include_patterns) {
        Ok(f) => f,
        Err(e) => {
            let msg = AgentToServer::OperationFailed {
                request_id: request_id.to_owned(),
                error: format!("failed to write include patterns file: {e}"),
            };
            if let Err(send_err) = outbound_tx.send(msg).await {
                tracing::debug!(error = %send_err, "outbound send failed");
            }
            return None;
        }
    };
    Some((exclude_file, include_file))
}

/// Builds the `borg create --dry-run` flags, placing `--patterns-from`
/// (when include patterns exist) before `--exclude-from`: borg tests
/// patterns in the order given on the command line, first match wins, so an
/// include there rescues a path a later, broader exclude would otherwise
/// drop.
pub(super) fn dry_run_create_args<'a>(
    archive_spec: &'a str,
    include_file_path: Option<&'a str>,
    exclude_file_path: &'a str,
) -> Vec<&'a str> {
    let mut flags: Vec<&str> = vec!["create", "--dry-run", "--list", "--log-json"];
    if let Some(include_file_path) = include_file_path {
        flags.push("--patterns-from");
        flags.push(include_file_path);
    }
    flags.push("--exclude-from");
    flags.push(exclude_file_path);
    flags.push(archive_spec);
    flags
}

pub(super) async fn run_dry_run_task(
    params: DryRunTaskParams,
    ctx: FreeTaskContext<'_>,
    borg: &Borg,
) {
    let DryRunTaskParams {
        repo_id,
        mut target,
        backup_sources,
        exclude_patterns,
        include_patterns,
        request_id,
    } = params;

    let FreeTaskContext {
        hostname,
        server_url,
        token,
        outbound_tx,
    } = ctx;

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let Some((exclude_file, include_file)) = write_dry_run_pattern_files(
        &exclude_patterns,
        &include_patterns,
        &request_id,
        outbound_tx,
    )
    .await
    else {
        return;
    };

    let timestamp = Utc::now().timestamp();
    let archive_spec = format!("::{hostname}-dryrun-{timestamp}");

    let env_vars = crate::backup::borg_env(&target);

    let include_file_path = include_file
        .as_ref()
        .map(|f| f.path().to_string_lossy().into_owned());
    let exclude_file_path = exclude_file.path().to_string_lossy().into_owned();

    let flags = dry_run_create_args(
        &archive_spec,
        include_file_path.as_deref(),
        &exclude_file_path,
    );
    let args = Borg::args_with_positional(&flags, &backup_sources);

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

pub(super) fn parse_dry_run_output(stderr: &str) -> (Vec<DryRunFile>, i64) {
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
