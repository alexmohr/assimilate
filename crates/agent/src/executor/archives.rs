// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use shared::protocol::AgentToServer;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::{FreeTaskContext, RestoreTaskParams, transport::setup_ssh_forward};
use crate::{backup::BackupTarget, borg::Borg};

pub(super) async fn run_restore_task(
    params: RestoreTaskParams,
    ctx: FreeTaskContext<'_>,
    borg: &Borg,
) {
    let RestoreTaskParams {
        repo_id,
        mut target,
        archive_name,
        paths,
        target_path,
        request_id,
    } = params;

    let FreeTaskContext {
        hostname,
        server_url,
        token,
        outbound_tx,
    } = ctx;

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let env_vars = crate::backup::borg_env(&target);

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
        warn!(
            repo_id = ?repo_id,
            "{}",
            crate::backup::warning_status_log("extract", exit_code, &stderr)
        );
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

pub(super) fn restore_args(archive_name: &str, paths: &[String]) -> Vec<String> {
    let archive_spec = format!("::{archive_name}");
    Borg::args_with_positional(&["extract", "--log-json", &archive_spec], paths)
}

pub(super) async fn run_delete_archives_task(
    mut target: BackupTarget,
    archive_names: Vec<String>,
    ssh_params: (&str, &str, &str),
    request_id: String,
    borg: &Borg,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let (hostname, server_url, token) = ssh_params;
    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let env_vars = crate::backup::borg_env(&target);
    let mut deleted_count: u32 = 0;

    for archive_name in &archive_names {
        let args = vec![
            "delete".to_owned(),
            "--lock-wait".to_owned(),
            "600".to_owned(),
            "--".to_owned(),
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
