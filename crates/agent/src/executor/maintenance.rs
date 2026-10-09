// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{fmt, time::Duration};

use shared::{
    protocol::AgentToServer,
    task_registry::TaskRegistry,
    types::{BorgEncryption, RepoId},
};
use tracing::{error, info};

use super::{FreeTaskContext, send_outbound, transport::setup_ssh_forward};
use crate::{
    backup::{BackupEngine, BackupTarget},
    borg::Borg,
};

pub(super) async fn run_init_repo_task(
    repo_url: &str,
    passphrase: &str,
    encryption: BorgEncryption,
    hostname: &str,
    server_url: &str,
    token: &str,
    task_registry: TaskRegistry,
) -> Result<(), String> {
    let mut ssh_forward_target = BackupTarget {
        hostname: hostname.to_owned(),
        ..BackupTarget::default()
    };

    let _ssh_forward =
        setup_ssh_forward(&mut ssh_forward_target, hostname, server_url, token).await;

    let mut env = vec![
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        ("BORG_DISPLAY_PASSPHRASE".to_owned(), "no".to_owned()),
        (
            "BORG_RSH".to_owned(),
            crate::backup::borg_rsh_for_target(&ssh_forward_target),
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

    let borg = Borg::new(task_registry);
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

/// The repository maintenance operations the server can ask for on demand.
#[derive(Debug, Clone, Copy)]
pub(super) enum MaintenanceKind {
    Check,
    Verify,
}

impl fmt::Display for MaintenanceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Check => write!(f, "check"),
            Self::Verify => write!(f, "verify"),
        }
    }
}

pub(super) async fn run_maintenance_task(
    kind: MaintenanceKind,
    repo_id: RepoId,
    target: &BackupTarget,
    engine: &BackupEngine,
    ctx: FreeTaskContext<'_>,
) {
    let FreeTaskContext {
        hostname,
        server_url,
        token,
        outbound_tx,
    } = ctx;
    let start = std::time::Instant::now();
    let mut target = BackupTarget::for_maintenance(target, hostname.to_owned());

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;

    let msg = match kind {
        MaintenanceKind::Check => {
            let result = engine.run_check(&target).await;
            let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

            let (success, error_message) = match result {
                Ok(()) => (true, None),
                Err(e) => {
                    error!(repo_id = ?repo_id, error = %e, "check failed");
                    (false, Some(e.to_string()))
                }
            };

            AgentToServer::CheckCompleted {
                repo_id,
                success,
                duration_secs,
                error_message,
            }
        }
        MaintenanceKind::Verify => {
            let result = engine.run_verify(&target).await;
            let duration_secs = i64::try_from(start.elapsed().as_secs()).unwrap_or(i64::MAX);

            let (success, error_message, files_verified) = match result {
                Ok(count) => (true, None, count),
                Err(e) => {
                    error!(repo_id = ?repo_id, error = %e, "verify failed");
                    (false, Some(e.to_string()), 0)
                }
            };

            AgentToServer::VerifyCompleted {
                repo_id,
                success,
                duration_secs,
                error_message,
                files_verified,
            }
        }
    };
    send_outbound(outbound_tx, msg).await;
}
