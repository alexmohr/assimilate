// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{path::PathBuf, time::Duration};

use shared::{
    database::{ANNOTATION_DATABASE, ANNOTATION_DATABASES, DatabaseBackupConfig, DumpFormat},
    task_registry::TaskRegistry,
    types::AgentConfig,
};
use tokio::io::AsyncReadExt as _;

use super::*;
use crate::{
    executor::{ExecutorCommand, OwnedTaskContext},
    tests::{
        fake_kube::{FakeKube, Reply},
        host_postgres::HostPostgres,
    },
};

/// What the mock borg's `extract --stdout` writes.
const PAYLOAD: &[u8] = b"mock-file-content-from-borg-extract\n";

fn mock_borg() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/mock-borg/borg")
}

fn host_source_id() -> DatabaseSourceId {
    DatabaseSourceId {
        runtime: DatabaseRuntime::Host,
        namespace: String::new(),
        name: "postgresql".to_owned(),
    }
}

fn restoring(source: DatabaseSourceId, target: DatabaseRestoreTarget) -> DatabaseRestore {
    DatabaseRestore {
        source,
        database: "orders".to_owned(),
        archive_path: "var/lib/assimilate/database-dumps/schedule-1/orders.pgdump".to_owned(),
        format: DumpFormat::PgCustom,
        target,
    }
}

fn into_new() -> DatabaseRestore {
    restoring(
        host_source_id(),
        DatabaseRestoreTarget::NewDatabase {
            name: "orders_restored".to_owned(),
        },
    )
}

/// Runs the restore task with the mock borg, whose environment `borg_env`
/// adds to, and returns what it reported, as JSON.
async fn run_task(
    restore: DatabaseRestore,
    runtimes: DatabaseRuntimes,
    borg: Borg,
) -> serde_json::Value {
    let (tx, mut rx) = mpsc::channel(8);
    let ctx = OwnedTaskContext {
        hostname: "hostname".to_owned(),
        server_url: "ws://127.0.0.1:9".to_owned(),
        token: "token".to_owned(),
        outbound_tx: tx,
        borg,
    };
    run_database_restore_task(
        DatabaseRestoreTaskParams {
            repo_id: RepoId(1),
            target: BackupTarget::default(),
            archive_name: "host-2026-10-16".to_owned(),
            restore,
            runtimes,
            request_id: "req-db".to_owned(),
        },
        ctx.borrowed(),
        &ctx.borg,
    )
    .await;
    serde_json::to_value(rx.recv().await.unwrap()).unwrap()
}

fn with_mock_borg(env: &[(&str, &str)]) -> Borg {
    let env = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    Borg::with_extra_env(mock_borg(), env)
}

fn kind(reply: &serde_json::Value) -> &str {
    reply
        .pointer("/type")
        .and_then(serde_json::Value::as_str)
        .unwrap()
}

/// The error a failed restore reported, either way it can be reported.
fn error(reply: &serde_json::Value) -> String {
    ["/payload/error", "/payload/error_message"]
        .into_iter()
        .find_map(|pointer| reply.pointer(pointer).and_then(serde_json::Value::as_str))
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_dump_is_streamed_from_borg_into_a_new_database() {
    let pg = HostPostgres::install(&[]).await;

    let reply = run_task(into_new(), pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(kind(&reply), "RestoreCompleted");
    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        reply.pointer("/payload/files_restored"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        reply.pointer("/payload/request_id"),
        Some(&serde_json::json!("req-db"))
    );
    assert_eq!(pg.programs().await, ["createdb", "pg_restore"]);
    assert_eq!(pg.restored().await, PAYLOAD);
}

#[tokio::test]
async fn a_dump_restored_over_the_original_cleans_it_first() {
    let pg = HostPostgres::install(&[]).await;

    let restore = restoring(host_source_id(), DatabaseRestoreTarget::Original);
    let reply = run_task(restore, pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(pg.programs().await, ["pg_restore"]);
    let log = pg.log().await;
    assert!(log.contains("--clean --if-exists"), "{log}");
    assert!(log.contains("--dbname=orders"), "{log}");
}

#[tokio::test]
async fn a_dump_is_restored_into_the_pod_its_source_runs_in_now() {
    let dir = tempfile::tempdir().unwrap();
    let pod = serde_json::from_value(serde_json::json!({
        "metadata": {
            "name": "db-1",
            "namespace": "shop",
            "annotations": {
                ANNOTATION_DATABASE: "postgresql",
                ANNOTATION_DATABASES: "orders",
            },
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db",
                "uid": "1", "controller": true,
            }],
        },
        "spec": { "containers": [{ "name": "postgres" }] },
        "status": { "phase": "Running" },
    }))
    .unwrap();
    let kube = FakeKube::start(vec![pod]).await;
    kube.reply(
        "shop",
        "db-1",
        "postgres",
        "--dbname=orders",
        Reply::ok(b""),
    );
    let runtimes = DatabaseRuntimes {
        kubernetes: true,
        kubeconfig_path: Some(
            kube.kubeconfig(dir.path())
                .await
                .to_string_lossy()
                .into_owned(),
        ),
        ..DatabaseRuntimes::default()
    };
    let source = DatabaseSourceId {
        runtime: DatabaseRuntime::Kubernetes,
        namespace: "shop".to_owned(),
        name: "db".to_owned(),
    };

    let restore = restoring(source, DatabaseRestoreTarget::Original);
    let reply = run_task(restore, runtimes, with_mock_borg(&[])).await;

    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(true)),
        "{reply}"
    );
    assert_eq!(kube.calls(), ["shop/db-1 postgres --dbname=orders"]);
    assert_eq!(kube.inputs(), [PAYLOAD.to_vec()]);
}

#[tokio::test]
async fn a_source_that_is_not_running_here_is_refused() {
    let pg = HostPostgres::install(&[]).await;
    let mut restore = into_new();
    restore.source.name = "elsewhere".to_owned();

    let reply = run_task(restore, pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(kind(&reply), "OperationFailed");
    assert_eq!(error(&reply), "host:elsewhere is not running on this host");
    assert_eq!(pg.programs().await.len(), 0);
}

#[tokio::test]
async fn a_source_whose_runtime_is_off_is_refused() {
    let pg = HostPostgres::install(&[]).await;
    let mut restore = into_new();
    restore.source.runtime = DatabaseRuntime::Docker;

    let reply = run_task(restore, pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(kind(&reply), "OperationFailed");
    assert!(
        error(&reply).ends_with("this host does not search docker for databases"),
        "{reply}"
    );
}

#[tokio::test]
async fn a_runtime_that_cannot_be_reached_is_reported() {
    let mut restore = into_new();
    restore.source = DatabaseSourceId {
        runtime: DatabaseRuntime::Kubernetes,
        namespace: "shop".to_owned(),
        name: "db".to_owned(),
    };
    let runtimes = DatabaseRuntimes {
        kubernetes: true,
        kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
        // Turned on, but not searched for a source in the cluster.
        docker: true,
        docker_socket: Some("/nonexistent/docker.sock".to_owned()),
        ..DatabaseRuntimes::default()
    };

    let reply = run_task(restore, runtimes, with_mock_borg(&[])).await;

    assert_eq!(kind(&reply), "OperationFailed");
    let error = error(&reply);
    assert!(
        error.starts_with("could not reach the Kubernetes cluster"),
        "{error}"
    );
    assert!(!error.contains("container engine"), "{error}");
    assert!(error.ends_with("cannot restore into shop/db"), "{error}");
}

#[tokio::test]
async fn a_new_database_that_cannot_be_created_fails_the_restore() {
    let pg = HostPostgres::install(&["createdb"]).await;

    let reply = run_task(into_new(), pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(kind(&reply), "RestoreCompleted");
    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        reply.pointer("/payload/files_restored"),
        Some(&serde_json::json!(0))
    );
    let error = error(&reply);
    assert!(
        error.starts_with("could not create the database orders_restored"),
        "{error}"
    );
    assert!(error.contains("createdb: error: it broke"), "{error}");
    assert_eq!(pg.programs().await, ["createdb"]);
}

#[tokio::test]
async fn a_failed_restore_drops_the_new_database_again() {
    let pg = HostPostgres::install(&["pg_restore"]).await;

    let reply = run_task(into_new(), pg.runtimes(), with_mock_borg(&[])).await;

    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(false))
    );
    let error = error(&reply);
    assert!(error.contains("pg_restore: error: it broke"), "{error}");
    assert_eq!(pg.programs().await, ["createdb", "pg_restore", "dropdb"]);
}

#[tokio::test]
async fn a_failing_borg_is_what_the_restore_reports() {
    let pg = HostPostgres::install(&[]).await;

    let reply = run_task(
        into_new(),
        pg.runtimes(),
        with_mock_borg(&[("MOCK_BORG_FAIL", "1")]),
    )
    .await;

    assert_eq!(
        reply.pointer("/payload/success"),
        Some(&serde_json::json!(false))
    );
    let error = error(&reply);
    assert!(
        error.starts_with("borg extract failed (exit 2): borg: Remote Exception"),
        "{error}"
    );
    let programs = pg.programs().await;
    assert_eq!(programs.first().map(String::as_str), Some("createdb"));
    assert_eq!(programs.last().map(String::as_str), Some("dropdb"));
    assert!(
        !pg.log().await.contains("pg_restore"),
        "pg_restore was stopped before it saw the end of its input"
    );
}

#[tokio::test]
async fn a_borg_that_cannot_start_is_an_operation_failure() {
    let pg = HostPostgres::install(&[]).await;
    let borg = Borg::with_extra_env(PathBuf::from("/nonexistent/borg"), Vec::new());

    let reply = run_task(into_new(), pg.runtimes(), borg).await;

    assert_eq!(kind(&reply), "OperationFailed");
    assert!(
        error(&reply).starts_with("failed to execute borg"),
        "{reply}"
    );
    assert_eq!(pg.programs().await.len(), 0);
}

#[test]
fn the_dump_is_extracted_to_stdout_by_its_path() {
    assert_eq!(
        extract_dump_args("host-1", "-odd/orders.pgdump"),
        [
            "extract",
            "--stdout",
            "--lock-wait",
            "600",
            "::host-1",
            "--",
            "-odd/orders.pgdump"
        ]
    );
}

#[test]
fn only_the_sources_own_runtime_is_searched() {
    let all = DatabaseRuntimes {
        kubernetes: true,
        docker: true,
        host: true,
        host_port: Some(5433),
        ..DatabaseRuntimes::default()
    };
    for runtime in [
        DatabaseRuntime::Kubernetes,
        DatabaseRuntime::Docker,
        DatabaseRuntime::Host,
    ] {
        let only = only_runtime(&all, runtime).unwrap();
        let on = [only.kubernetes, only.docker, only.host];
        assert_eq!(on.iter().filter(|on| **on).count(), 1, "{runtime}");
        assert_eq!(only.host_port, Some(5433));
        assert_eq!(only_runtime(&DatabaseRuntimes::default(), runtime), None);
    }
    assert!(only_runtime(&all, DatabaseRuntime::Docker).unwrap().docker);
}

#[test]
fn a_new_database_needs_a_plain_name() {
    let mut restore = into_new();
    assert_eq!(rejection(&restore), None);
    restore.target = DatabaseRestoreTarget::NewDatabase {
        name: "-orders".to_owned(),
    };
    let reason = rejection(&restore).unwrap();
    assert!(reason.contains("\"-orders\""), "{reason}");
}

#[tokio::test]
async fn the_dump_stays_ended_once_borg_has() {
    let StreamingBorg { child, stdout, .. } = Borg::with_extra_env(PathBuf::from("sh"), Vec::new())
        .spawn(&["-c", "printf dump"], &[])
        .unwrap();
    let (exit_tx, exit_rx) = oneshot::channel();
    let mut dump = DumpStream::new(stdout, child, exit_tx);
    let mut read = Vec::new();
    dump.read_to_end(&mut read).await.unwrap();
    assert_eq!(read, b"dump");
    assert_eq!(dump.read(&mut [0u8; 8]).await.unwrap(), 0);
    assert_eq!(exit_rx.await.unwrap(), 0);
}

mod requests {
    use super::*;

    fn request(restore: DatabaseRestore) -> RestoreDatabaseRequest {
        RestoreDatabaseRequest {
            request_id: "req-db".to_owned(),
            repo_id: RepoId(1),
            archive_name: "host-2026-10-16".to_owned(),
            restore,
        }
    }

    async fn handle(executor: &Executor, restore: DatabaseRestore) -> serde_json::Value {
        let (tx, mut rx) = mpsc::channel(4);
        executor
            .dispatch(ExecutorCommand::RestoreDatabase(request(restore)), &tx)
            .await;
        executor
            .task_registry
            .shutdown(Duration::from_secs(10))
            .await;
        serde_json::to_value(rx.try_recv().unwrap()).unwrap()
    }

    fn configured() -> Executor {
        Executor::new("ws://127.0.0.1:9", "token", TaskRegistry::default())
    }

    async fn configure(executor: &Executor) {
        let repo = shared::types::RepoConfig {
            repo_id: RepoId(1),
            name: "daily".to_owned(),
            repo_path: "/backups/daily".to_owned(),
            ssh_user: "borg".to_owned(),
            ssh_host: "storage.example.com".to_owned(),
            ssh_port: 22,
            ssh_host_key: String::new(),
            passphrase: "secret".to_owned(),
            compression: shared::types::Compression::Lz4,
            enabled: true,
            accept_relocation: false,
            schedules: Vec::new(),
        };
        *executor.current_config.lock().await = Some(AgentConfig {
            agent_hostname: "hostname".to_owned(),
            vm_snapshot: shared::vm::VmSnapshotConfig::default(),
            database_backup: DatabaseBackupConfig::default(),
            skip_targets: Vec::new(),
            repos: vec![repo],
        });
    }

    #[tokio::test]
    async fn an_invalid_new_database_name_is_refused_before_anything_queues() {
        let executor = configured();
        configure(&executor).await;
        let mut restore = into_new();
        restore.target = DatabaseRestoreTarget::NewDatabase {
            name: "pg_catalog".to_owned(),
        };

        let reply = handle(&executor, restore).await;

        assert_eq!(kind(&reply), "OperationFailed");
        assert!(error(&reply).contains("\"pg_catalog\""), "{reply}");
        assert_eq!(executor.task_registry.pending_count(), 0);
    }

    #[tokio::test]
    async fn a_restore_without_a_config_is_refused() {
        let reply = handle(&configured(), into_new()).await;

        assert_eq!(kind(&reply), "OperationFailed");
        assert_eq!(error(&reply), "agent has no config yet");
    }

    #[tokio::test]
    async fn a_restore_is_queued_on_its_repository_and_searches_the_hosts_runtimes() {
        let executor = configured();
        configure(&executor).await;

        let reply = handle(&executor, into_new()).await;

        // The host's settings turn no runtime on.
        assert_eq!(kind(&reply), "OperationFailed");
        assert!(
            error(&reply).ends_with("this host does not search host for databases"),
            "{reply}"
        );
    }
}
