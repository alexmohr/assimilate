// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

use shared::{database::DatabaseBackupConfig, protocol::AgentToServer};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{Executor, command::ScanDatabasesRequest, send_outbound};
use crate::{
    backup::BackupTarget,
    database::{
        ExecError, KubeExec, PreparedDumps, describe_sources, find_sources, plan_dumps,
        schedule_dir,
    },
};

/// The dumps a backup streams into its archive, when its schedule asked for
/// databases. `Ok(None)` is a backup without dumps; an error is the reason the
/// backup must not run, since an archive missing a database the operator
/// selected is worse than a failed run they are told about.
pub(super) async fn prepare_dumps(
    target: &BackupTarget,
) -> Result<Option<PreparedDumps<KubeExec>>, String> {
    let Some(config) = &target.database_backup else {
        return Ok(None);
    };
    let exec = connect(config.kubeconfig_path.as_deref())
        .await
        .map_err(|e| format!("could not reach the Kubernetes cluster to dump databases: {e}"))?;
    let pods = exec
        .list_pods()
        .await
        .map_err(|e| format!("could not list the cluster's pods to dump databases: {e}"))?;
    let jobs = plan_dumps(&exec, config, &find_sources(&pods))
        .await
        .map_err(|e| format!("could not list the databases to dump: {e}"))?;
    if jobs.is_empty() {
        warn!(
            "the schedule asks for databases, but no database source in the cluster is selected; \
             backing up without dumps"
        );
        return Ok(None);
    }
    let dir = schedule_dir(&config.dump_dir, target.schedule_id);
    info!(
        "Streaming {} database dump(s) from {}",
        jobs.len(),
        dir.display()
    );
    PreparedDumps::create(exec, dir.clone(), jobs)
        .await
        .map(Some)
        .map_err(|e| format!("could not create the dump pipes in {}: {e}", dir.display()))
}

async fn connect(kubeconfig_path: Option<&str>) -> Result<KubeExec, ExecError> {
    KubeExec::connect(kubeconfig_path.map(Path::new)).await
}

impl Executor {
    /// Lists the database workloads in the host's cluster and their databases,
    /// with `kubeconfig_path` when the request names one and the host's own
    /// setting otherwise. Runs on its own task: asking every pod for its
    /// databases takes a while, and backups must not wait behind it.
    pub(super) async fn handle_scan_databases(
        &self,
        request: ScanDatabasesRequest,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let ScanDatabasesRequest {
            request_id,
            kubeconfig_path,
        } = request;
        let configured = self
            .current_config
            .lock()
            .await
            .as_ref()
            .map_or_else(DatabaseBackupConfig::default, |config| {
                config.database_backup.clone()
            });
        let path = kubeconfig_path.or(configured.kubeconfig_path);
        let outbound_tx = outbound_tx.clone();
        let handle = tokio::spawn(async move {
            let scanned = async {
                let exec = connect(path.as_deref()).await?;
                let pods = exec.list_pods().await?;
                Ok::<_, ExecError>(describe_sources(&exec, &find_sources(&pods)).await)
            }
            .await;
            let (sources, error) = match scanned {
                Ok(sources) => (sources, None),
                Err(e) => {
                    warn!("Database scan failed: {e}");
                    (Vec::new(), Some(e.to_string()))
                }
            };
            send_outbound(
                &outbound_tx,
                AgentToServer::DatabaseScanResult {
                    request_id,
                    sources,
                    error,
                },
            )
            .await;
        });
        self.task_registry.register(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_backup_without_database_settings_prepares_nothing() {
        let target = BackupTarget::default();
        assert!(prepare_dumps(&target).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_missing_kubeconfig_fails_the_backup_with_the_reason() {
        let target = BackupTarget {
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };
        let Err(reason) = prepare_dumps(&target).await else {
            panic!("a missing kubeconfig must fail the backup");
        };
        assert!(reason.contains("could not reach the Kubernetes cluster"));
        assert!(reason.contains("/nonexistent/kubeconfig"));
    }
}
