// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{database::DatabaseBackupConfig, protocol::AgentToServer};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{Executor, command::ScanDatabasesRequest, send_outbound};
use crate::{
    backup::BackupTarget,
    database::{
        PreparedDumps, RuntimeClients, describe_sources, find_all, plan_dumps, schedule_dir,
    },
};

/// The dumps a backup streams into its archive, when its schedule asked for
/// databases. `Ok(None)` is a backup without dumps; an error is the reason the
/// backup must not run, since an archive missing a database the operator
/// selected is worse than a failed run they are told about.
pub(super) async fn prepare_dumps(
    target: &BackupTarget,
) -> Result<Option<PreparedDumps<RuntimeClients>>, String> {
    let Some(config) = &target.database_backup else {
        return Ok(None);
    };
    if !config.runtimes.any() {
        warn!(
            "the schedule asks for databases, but this host searches no runtime for them; backing \
             up without dumps"
        );
        return Ok(None);
    }
    let found = find_all(&config.runtimes).await;
    if let Some(failure) = found.failure() {
        return Err(format!("{failure}; cannot dump databases"));
    }
    let jobs = plan_dumps(&found.clients, config, &found.sources)
        .await
        .map_err(|e| format!("could not list the databases to dump: {e}"))?;
    if jobs.is_empty() {
        warn!(
            "the schedule asks for databases, but no database source on this host is selected; \
             backing up without dumps"
        );
        return Ok(None);
    }
    let dir = schedule_dir(&config.dump_dir, target.schedule_id);
    let (count, shown) = (jobs.len(), dir.display());
    info!("Streaming {count} database dump(s) from {shown}");
    PreparedDumps::create(found.clients, dir.clone(), jobs)
        .await
        .map(Some)
        .map_err(|e| format!("could not create the dump pipes in {}: {e}", dir.display()))
}

impl Executor {
    /// Lists the database workloads on the host and their databases, in the
    /// runtimes the request names, or else the ones the host's settings turn
    /// on. Runs on its own task: asking every server for its databases takes
    /// a while, and backups must not wait behind it.
    pub(super) async fn handle_scan_databases(
        &self,
        request: ScanDatabasesRequest,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let ScanDatabasesRequest {
            request_id,
            runtimes,
        } = request;
        let configured = self
            .current_config
            .lock()
            .await
            .as_ref()
            .map_or_else(DatabaseBackupConfig::default, |config| {
                config.database_backup.clone()
            });
        let runtimes = runtimes.map_or(configured.runtimes, |runtimes| *runtimes);
        let outbound_tx = outbound_tx.clone();
        let handle = tokio::spawn(async move {
            let (sources, error, failed_runtimes) = if runtimes.any() {
                let found = find_all(&runtimes).await;
                let sources = describe_sources(&found.clients, &found.sources).await;
                (sources, found.failure(), found.failed_runtimes())
            } else {
                let none = "no runtime is turned on to search for databases".to_owned();
                (Vec::new(), Some(none), Vec::new())
            };
            if let Some(error) = &error {
                warn!("Database scan failed: {error}");
            }
            send_outbound(
                &outbound_tx,
                AgentToServer::DatabaseScanResult {
                    request_id,
                    sources,
                    error,
                    failed_runtimes,
                },
            )
            .await;
        });
        self.task_registry.register(handle);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use k8s_openapi::api::core::v1::Pod;
    use shared::{
        database::{
            ANNOTATION_DATABASE, ANNOTATION_DATABASES, DatabaseRuntime, DatabaseRuntimes,
            DatabaseSelectionMode, DatabaseSourceConfig, DatabaseSourceId,
        },
        task_registry::TaskRegistry,
    };

    use super::*;
    use crate::tests::{
        fake_kube::{FakeKube, Reply},
        messages::database_scan_result,
    };

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

    /// The settings that search only `kube`'s cluster.
    async fn cluster(kube: &FakeKube, dir: &Path) -> DatabaseRuntimes {
        DatabaseRuntimes {
            kubernetes: true,
            kubeconfig_path: Some(kube.kubeconfig(dir).await.to_string_lossy().into_owned()),
            ..DatabaseRuntimes::default()
        }
    }

    async fn target_for(kube: &FakeKube, dir: &Path, config: DatabaseBackupConfig) -> BackupTarget {
        BackupTarget {
            schedule_id: Some(4),
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                runtimes: cluster(kube, dir).await,
                dump_dir: dir.join("dumps").to_string_lossy().into_owned(),
                ..config
            }),
            ..BackupTarget::default()
        }
    }

    async fn prepare_error(target: &BackupTarget) -> String {
        prepare_dumps(target)
            .await
            .err()
            .expect("preparing must fail")
    }

    #[tokio::test]
    async fn dumps_are_prepared_for_every_selected_database() {
        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(vec![annotated_pod(Some("orders,users"))]).await;
        let target = target_for(&kube, dir.path(), DatabaseBackupConfig::default()).await;

        let dumps = prepare_dumps(&target).await.unwrap().unwrap();

        assert_eq!(dumps.dir(), dir.path().join("dumps/schedule-4"));
        for database in ["orders", "users"] {
            let pipe = dumps
                .dir()
                .join(format!("kubernetes/shop/db/{database}.pgdump"));
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
                        runtime: DatabaseRuntime::Kubernetes,
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

    async fn scan(executor: &Executor, runtimes: Option<DatabaseRuntimes>) -> AgentToServer {
        let runtimes = runtimes.map(Box::new);
        let (tx, mut rx) = mpsc::channel(4);
        executor
            .handle_scan_databases(
                ScanDatabasesRequest {
                    request_id: "req-1".to_owned(),
                    runtimes,
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

        let reply = scan(&executor, Some(cluster(&kube, dir.path()).await)).await;

        let (request_id, sources, error) = database_scan_result(Some(reply));
        assert_eq!(request_id, "req-1");
        assert_eq!(error, None);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "shop/db");
        assert_eq!(source.databases, ["orders", "users"]);
    }

    #[tokio::test]
    async fn a_scan_without_runtimes_of_its_own_uses_the_hosts() {
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
        *executor.current_config.lock().await = Some(shared::types::AgentConfig {
            agent_hostname: "hostname".to_owned(),
            vm_snapshot: shared::vm::VmSnapshotConfig::default(),
            database_backup: DatabaseBackupConfig {
                runtimes: DatabaseRuntimes {
                    kubernetes: true,
                    kubeconfig_path: Some("/nonexistent/host-kubeconfig".to_owned()),
                    ..DatabaseRuntimes::default()
                },
                ..DatabaseBackupConfig::default()
            },
            skip_targets: Vec::new(),
            repos: Vec::new(),
        });

        let (_, _, error) = database_scan_result(Some(scan(&executor, None).await));

        assert!(
            error
                .unwrap_or_default()
                .contains("/nonexistent/host-kubeconfig")
        );
    }

    #[tokio::test]
    async fn a_scan_that_cannot_reach_the_cluster_says_why() {
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
        let runtimes = DatabaseRuntimes {
            kubernetes: true,
            kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
            ..DatabaseRuntimes::default()
        };
        let (_, sources, error) = database_scan_result(Some(scan(&executor, Some(runtimes)).await));
        assert_eq!(sources.len(), 0);
        assert!(
            error
                .unwrap_or_default()
                .contains("/nonexistent/kubeconfig")
        );
    }

    #[tokio::test]
    async fn a_scan_with_no_runtime_turned_on_says_so() {
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
        let (_, sources, error) = database_scan_result(Some(scan(&executor, None).await));
        assert_eq!(sources.len(), 0);
        assert!(
            error
                .unwrap_or_default()
                .contains("no runtime is turned on")
        );
    }

    #[tokio::test]
    async fn a_host_with_no_runtime_turned_on_backs_up_without_dumps() {
        let target = BackupTarget {
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };
        let (prepared, logs) = crate::tests::logs::logs_while(prepare_dumps(&target)).await;
        assert!(prepared.unwrap().is_none());
        assert!(logs.contains("searches no runtime"), "{logs}");
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
                runtimes: DatabaseRuntimes {
                    kubernetes: true,
                    kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
                    ..DatabaseRuntimes::default()
                },
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };
        let reason = prepare_dumps(&target)
            .await
            .err()
            .expect("a missing kubeconfig must fail the backup");
        assert!(reason.contains("could not reach the Kubernetes cluster"));
        assert!(reason.contains("/nonexistent/kubeconfig"));
    }
}
