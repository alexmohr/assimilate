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
    use k8s_openapi::api::core::v1::Pod;
    use shared::{
        database::{
            ANNOTATION_DATABASE, ANNOTATION_DATABASES, DatabaseSelectionMode, DatabaseSourceConfig,
            DatabaseSourceId,
        },
        task_registry::TaskRegistry,
    };

    use super::*;
    use crate::database::fake_kube::{FakeKube, Reply};

    /// A running pod of the `shop/db` `StatefulSet` that names its databases,
    /// so planning needs no exec of its own.
    fn annotated_pod(databases: Option<&str>) -> Pod {
        let mut annotations = serde_json::json!({ ANNOTATION_DATABASE: "postgresql" });
        if let Some(list) = databases {
            annotations
                .as_object_mut()
                .unwrap()
                .insert(ANNOTATION_DATABASES.to_owned(), list.into());
        }
        serde_json::from_value(serde_json::json!({
            "metadata": {
                "name": "db-0",
                "namespace": "shop",
                "annotations": annotations,
                "ownerReferences": [{
                    "apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db",
                    "uid": "1", "controller": true,
                }],
            },
            "spec": { "containers": [{ "name": "postgres" }] },
            "status": { "phase": "Running" },
        }))
        .unwrap()
    }

    async fn target_for(kube: &FakeKube, dir: &Path, config: DatabaseBackupConfig) -> BackupTarget {
        BackupTarget {
            schedule_id: Some(4),
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                kubeconfig_path: Some(kube.kubeconfig(dir).await.to_string_lossy().into_owned()),
                dump_dir: dir.join("dumps").to_string_lossy().into_owned(),
                ..config
            }),
            ..BackupTarget::default()
        }
    }

    async fn prepare_error(target: &BackupTarget) -> String {
        match prepare_dumps(target).await {
            Err(reason) => reason,
            Ok(_) => panic!("preparing must fail"),
        }
    }

    #[tokio::test]
    async fn dumps_are_prepared_for_every_selected_database() {
        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(vec![annotated_pod(Some("orders,users"))]).await;
        let target = target_for(&kube, dir.path(), DatabaseBackupConfig::default()).await;

        let dumps = prepare_dumps(&target).await.unwrap().unwrap();

        assert_eq!(dumps.dir(), dir.path().join("dumps/schedule-4"));
        for database in ["orders", "users"] {
            let pipe = dumps.dir().join(format!("shop/db/{database}.pgdump"));
            assert!(
                tokio::fs::metadata(pipe).await.is_ok(),
                "{database} has a pipe"
            );
        }
        dumps.remove().await;
    }

    #[tokio::test]
    async fn a_cluster_with_nothing_selected_backs_up_without_dumps() {
        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(vec![annotated_pod(Some("orders"))]).await;
        let target = target_for(
            &kube,
            dir.path(),
            DatabaseBackupConfig {
                selection: DatabaseSelectionMode::Selected,
                sources: vec![DatabaseSourceConfig {
                    id: DatabaseSourceId {
                        namespace: "other".to_owned(),
                        name: "db".to_owned(),
                    },
                    included: Some(true),
                }],
                ..DatabaseBackupConfig::default()
            },
        )
        .await;
        assert!(prepare_dumps(&target).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn every_step_that_fails_names_itself() {
        let dir = tempfile::tempdir().unwrap();

        let refused = FakeKube::start(Vec::new()).await;
        refused.fail_list(403);
        let reason =
            prepare_error(&target_for(&refused, dir.path(), DatabaseBackupConfig::default()).await)
                .await;
        assert!(
            reason.starts_with("could not list the cluster's pods"),
            "{reason}"
        );

        // No database list in the annotation, and the listing exec fails.
        let unlisted = FakeKube::start(vec![annotated_pod(None)]).await;
        let reason = prepare_error(
            &target_for(&unlisted, dir.path(), DatabaseBackupConfig::default()).await,
        )
        .await;
        assert!(
            reason.starts_with("could not list the databases to dump"),
            "{reason}"
        );

        // The dump directory sits below a regular file.
        let blocked = FakeKube::start(vec![annotated_pod(Some("orders"))]).await;
        tokio::fs::write(dir.path().join("dumps"), b"not a directory")
            .await
            .unwrap();
        let reason =
            prepare_error(&target_for(&blocked, dir.path(), DatabaseBackupConfig::default()).await)
                .await;
        assert!(
            reason.starts_with("could not create the dump pipes"),
            "{reason}"
        );
    }

    async fn scan(executor: &Executor, kubeconfig_path: Option<String>) -> AgentToServer {
        let (tx, mut rx) = mpsc::channel(4);
        executor
            .handle_scan_databases(
                ScanDatabasesRequest {
                    request_id: "req-1".to_owned(),
                    kubeconfig_path,
                },
                &tx,
            )
            .await;
        rx.recv().await.unwrap()
    }

    #[tokio::test]
    async fn a_scan_reports_each_source_with_its_databases() {
        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(vec![annotated_pod(None)]).await;
        kube.reply(
            "shop",
            "db-0",
            "postgres",
            "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY \
             datname",
            Reply::ok(b"orders\nusers\n"),
        );
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());

        let reply = scan(
            &executor,
            Some(
                kube.kubeconfig(dir.path())
                    .await
                    .to_string_lossy()
                    .into_owned(),
            ),
        )
        .await;

        let AgentToServer::DatabaseScanResult {
            request_id,
            sources,
            error,
        } = reply
        else {
            panic!("a scan answers with its result");
        };
        assert_eq!(request_id, "req-1");
        assert_eq!(error, None);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "shop/db");
        assert_eq!(source.databases, ["orders", "users"]);
    }

    #[tokio::test]
    async fn a_scan_that_cannot_reach_the_cluster_says_why() {
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
        let AgentToServer::DatabaseScanResult { sources, error, .. } =
            scan(&executor, Some("/nonexistent/kubeconfig".to_owned())).await
        else {
            panic!("a scan answers with its result");
        };
        assert_eq!(sources.len(), 0);
        assert!(
            error
                .unwrap_or_default()
                .contains("/nonexistent/kubeconfig")
        );
    }

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
