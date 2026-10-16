// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use shared::{database::DatabaseBackupConfig, vm::VmSnapshotConfig};

use super::{
    archives::restore_args,
    dry_run::{dry_run_create_args, parse_dry_run_output, write_dry_run_pattern_files},
    transport::{backup_target_from_repo, write_known_hosts},
    *,
};

#[test]
fn restore_args_select_the_whole_archive_when_paths_are_empty() {
    assert_eq!(
        restore_args("archive-1", &[]),
        vec!["extract", "--log-json", "::archive-1"]
    );
}

#[test]
fn restore_args_append_selected_paths() {
    assert_eq!(
        restore_args(
            "archive-1",
            &["etc/hosts".to_owned(), "var/lib/app".to_owned()]
        ),
        vec![
            "extract",
            "--log-json",
            "::archive-1",
            "--",
            "etc/hosts",
            "var/lib/app"
        ]
    );
}

#[test]
fn parse_dry_run_file_status_and_archive_progress() {
    let stderr = [
        r#"{"type": "file_status", "status": "A", "path": "/home/user/doc.txt"}"#,
        concat!(
            r#"{"type": "archive_progress", "#,
            r#""original_size": 500, "compressed_size": 300, "#,
            r#""deduplicated_size": 200, "nfiles": 1, "#,
            r#""path": "/home/user/doc.txt"}"#,
        ),
        r#"{"type": "file_status", "status": "U", "path": "/home/user/photo.jpg"}"#,
        concat!(
            r#"{"type": "archive_progress", "#,
            r#""original_size": 10240, "compressed_size": 8000, "#,
            r#""deduplicated_size": 5000, "nfiles": 2, "#,
            r#""path": "/home/user/photo.jpg", "finished": true}"#,
        ),
    ]
    .join("\n");

    let (files, total_size) = parse_dry_run_output(&stderr);

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "/home/user/doc.txt");
    assert_eq!(files[0].size, 0);
    assert_eq!(files[1].path, "/home/user/photo.jpg");
    assert_eq!(files[1].size, 0);
    assert_eq!(total_size, 10240);
}

#[test]
fn parse_dry_run_file_status_only() {
    let stderr = [
        r#"{"type": "file_status", "status": "A", "path": "/etc/hostname"}"#,
        r#"{"type": "file_status", "status": "A", "path": "/etc/passwd"}"#,
    ]
    .join("\n");

    let (files, total_size) = parse_dry_run_output(&stderr);

    assert_eq!(files.len(), 2);
    assert_eq!(total_size, 0);
}

#[test]
fn parse_dry_run_ignores_non_json_lines() {
    let stderr = [
        "not json",
        r#"{"type": "file_status", "status": "A", "path": "/a"}"#,
        "some garbage",
    ]
    .join("\n");

    let (files, total_size) = parse_dry_run_output(&stderr);

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/a");
    assert_eq!(total_size, 0);
}

#[test]
fn parse_dry_run_ignores_log_messages() {
    let stderr = [
        r#"{"type": "log_message", "levelname": "WARNING", "message": "something"}"#,
        r#"{"type": "file_status", "status": "A", "path": "/b"}"#,
    ]
    .join("\n");

    let (files, total_size) = parse_dry_run_output(&stderr);

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/b");
    assert_eq!(total_size, 0);
}

#[test]
fn parse_dry_run_empty_input() {
    let (files, total_size) = parse_dry_run_output("");

    assert_eq!(files.len(), 0);
    assert_eq!(total_size, 0);
}

#[test]
fn dry_run_create_args_without_include_patterns() {
    let flags = dry_run_create_args("::host-dryrun-1", None, "/tmp/exclude");
    assert_eq!(
        flags,
        vec![
            "create",
            "--dry-run",
            "--list",
            "--log-json",
            "--exclude-from",
            "/tmp/exclude",
            "::host-dryrun-1",
        ]
    );
}

#[test]
fn dry_run_create_args_places_include_before_exclude() {
    let flags = dry_run_create_args("::host-dryrun-1", Some("/tmp/include"), "/tmp/exclude");
    assert_eq!(
        flags,
        vec![
            "create",
            "--dry-run",
            "--list",
            "--log-json",
            "--patterns-from",
            "/tmp/include",
            "--exclude-from",
            "/tmp/exclude",
            "::host-dryrun-1",
        ]
    );
}

#[tokio::test]
async fn write_dry_run_pattern_files_writes_both_when_include_patterns_present() {
    let (outbound_tx, _outbound_rx) = mpsc::channel(4);
    let result = write_dry_run_pattern_files(
        &["*.log".to_owned()],
        &["/home/keep".to_owned()],
        "req-1",
        &outbound_tx,
    )
    .await;

    let (exclude_file, include_file) = result.expect("both pattern files should be written");
    let include_file = include_file.expect("include patterns were provided");
    assert_eq!(
        std::fs::read_to_string(exclude_file.path()).unwrap(),
        "*.log\n"
    );
    assert_eq!(
        std::fs::read_to_string(include_file.path()).unwrap(),
        "+ /home/keep\n"
    );
}

#[tokio::test]
async fn write_dry_run_pattern_files_omits_include_file_when_no_include_patterns() {
    let (outbound_tx, _outbound_rx) = mpsc::channel(4);
    let result =
        write_dry_run_pattern_files(&["*.log".to_owned()], &[], "req-1", &outbound_tx).await;

    let (_exclude_file, include_file) = result.expect("exclude file should still be written");
    assert!(include_file.is_none());
}

fn make_schedule(id: i64, sources: Vec<&str>) -> shared::types::ScheduleConfig {
    shared::types::ScheduleConfig {
        id,
        schedule_type: shared::types::ScheduleType::Backup,
        cron_expression: "0 3 * * *".to_owned(),
        enabled: true,
        backup_sources: sources.into_iter().map(str::to_owned).collect(),
        rate_limit_kbps: None,
        canary_enabled: false,
        vm_snapshot_enabled: false,
        database_backup_enabled: false,
        exclude_patterns: Vec::new(),
        ignore_global_excludes: false,
        include_patterns: Vec::new(),
        keep_hourly: 24,
        keep_daily: 7,
        keep_weekly: 4,
        keep_monthly: 6,
        keep_yearly: 0,
        compact_enabled: true,
        pre_backup_commands: Vec::new(),
        post_backup_commands: Vec::new(),
        hook_timeout_seconds: 60,
        file_change_patterns: Vec::new(),
    }
}

fn make_repo(schedules: Vec<shared::types::ScheduleConfig>) -> shared::types::RepoConfig {
    shared::types::RepoConfig {
        repo_id: shared::types::RepoId(1),
        name: "test-repo".to_owned(),
        repo_path: "/backup/test".to_owned(),
        ssh_user: "borg".to_owned(),
        ssh_host: "backup.example.com".to_owned(),
        ssh_port: 22,
        ssh_host_key: "ssh-ed25519 AAAATEST".to_owned(),
        passphrase: "secret".to_owned(),
        compression: shared::types::Compression::Lz4,
        enabled: true,
        accept_relocation: false,
        schedules,
    }
}

#[test]
fn backup_target_uses_first_schedule_when_no_id_given() {
    let repo = make_repo(vec![
        make_schedule(10, vec!["/var"]),
        make_schedule(20, vec!["/home"]),
    ]);
    let target = backup_target_from_repo(
        &repo,
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    assert_eq!(target.backup_sources, vec!["/var"]);
}

#[test]
fn backup_target_uses_specified_schedule_id() {
    let repo = make_repo(vec![
        make_schedule(10, vec!["/var"]),
        make_schedule(20, vec!["/home"]),
    ]);
    let target = backup_target_from_repo(
        &repo,
        "hostname",
        Some(20),
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    assert_eq!(target.backup_sources, vec!["/home"]);
}

#[test]
fn backup_target_falls_back_to_first_when_id_not_found() {
    let repo = make_repo(vec![
        make_schedule(10, vec!["/var"]),
        make_schedule(20, vec!["/home"]),
    ]);
    let target = backup_target_from_repo(
        &repo,
        "hostname",
        Some(99),
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    assert_eq!(target.backup_sources, vec!["/var"]);
}

#[test]
fn backup_target_maps_include_patterns_from_schedule() {
    let repo = make_repo(vec![shared::types::ScheduleConfig {
        include_patterns: vec!["/home/keep".to_owned()],
        ..make_schedule(10, vec!["/var"])
    }]);
    let target = backup_target_from_repo(
        &repo,
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    assert_eq!(target.include_patterns, vec!["/home/keep".to_owned()]);
}

#[test]
fn build_borg_env_uses_pinned_known_hosts_file() {
    let mut target = backup_target_from_repo(
        &make_repo(vec![make_schedule(10, vec!["/var"])]),
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    let known_hosts = write_known_hosts(&mut target).unwrap().unwrap();
    let env = crate::backup::borg_env(&target);
    let borg_rsh = env
        .iter()
        .find(|(key, _value)| key == "BORG_RSH")
        .map(|(_key, value)| value.as_str());
    let expected = shared::ssh::borg_rsh_with_known_hosts(known_hosts.path());

    assert_eq!(borg_rsh, Some(expected.as_str()));
}

#[test]
fn write_known_hosts_pins_the_repository_endpoint() {
    let mut target = backup_target_from_repo(
        &make_repo(vec![make_schedule(10, vec!["/var"])]),
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    target.ssh_port = 2222;

    let known_hosts = write_known_hosts(&mut target).unwrap().unwrap();
    let contents = std::fs::read_to_string(known_hosts.path()).unwrap();

    assert_eq!(contents, "[backup.example.com]:2222 ssh-ed25519 AAAATEST\n");
}

#[test]
fn repo_operation_key_is_based_on_physical_repo_location() {
    let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
    let first = backup_target_from_repo(
        &repo,
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );
    let second = backup_target_from_repo(
        &repo,
        "hostname",
        Some(10),
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    );

    assert_eq!(
        RepoOperationKey::from_backup_target(&first),
        RepoOperationKey::from_backup_target(&second)
    );
}

/// The queue key a backup of `repo` runs under, with no schedule.
fn repo_key_for(repo: &shared::types::RepoConfig) -> RepoOperationKey {
    RepoOperationKey::from_backup_target(&backup_target_from_repo(
        repo,
        "hostname",
        None,
        &VmSnapshotConfig::default(),
        &DatabaseBackupConfig::default(),
    ))
}

/// Hands `executor` a config naming only `repo`, as the server would.
async fn configure_single_repo(executor: &Executor, repo: &shared::types::RepoConfig) {
    let config = shared::types::AgentConfig {
        agent_hostname: "hostname".to_owned(),
        vm_snapshot: VmSnapshotConfig::default(),
        database_backup: DatabaseBackupConfig::default(),
        skip_targets: Vec::new(),
        repos: vec![repo.clone()],
    };
    *executor.current_config.lock().await = Some(config);
}

#[tokio::test]
async fn cancel_backup_with_no_active_task_sends_nothing() {
    let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
    let (tx, mut rx) = mpsc::channel(8);
    let repo_id = shared::types::RepoId(1);

    executor.handle_cancel_backup(repo_id, &tx).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn cancel_backup_aborts_queued_task_and_sends_cancelled() {
    let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
    let (tx, mut rx) = mpsc::channel(8);
    let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
    configure_single_repo(&executor, &repo).await;

    let repo_key = repo_key_for(&repo);
    let repo_queue = executor.repo_operation_queue(&repo_key).await;
    let permit = Arc::clone(&repo_queue).acquire_owned().await.unwrap();

    executor.handle_run_now(repo.repo_id, None, None, &tx).await;

    assert!(rx.try_recv().is_err());
    assert_eq!(executor.active_backup_tasks.lock().await.len(), 1);

    executor.handle_cancel_backup(repo.repo_id, &tx).await;

    let msg = rx.try_recv().unwrap();
    assert!(matches!(msg, AgentToServer::BackupCancelled { repo_id: r } if r == repo.repo_id));
    assert_eq!(executor.active_backup_tasks.lock().await.len(), 0);
    drop(permit);
}

#[tokio::test]
async fn handle_run_now_registers_its_spawned_task_in_the_registry() {
    let task_registry = TaskRegistry::default();
    let executor = Executor::new("ws://localhost", "token", task_registry.clone());
    let (tx, _rx) = mpsc::channel(8);
    let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
    configure_single_repo(&executor, &repo).await;

    assert_eq!(task_registry.pending_count(), 0);

    executor.handle_run_now(repo.repo_id, None, None, &tx).await;

    assert_eq!(
        task_registry.pending_count(),
        1,
        "the spawned backup task must be registered before handle_run_now returns"
    );

    let outstanding = task_registry.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        outstanding, 0,
        "shutdown should be able to join the registered task"
    );
}

#[tokio::test]
async fn repo_operation_queue_serializes_tasks() {
    let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
    let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
    let repo_key = repo_key_for(&repo);
    let repo_queue = executor.repo_operation_queue(&repo_key).await;
    let permit = Arc::clone(&repo_queue).acquire_owned().await.unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    let queue = executor.repo_operation_queue(&repo_key).await;

    let handle = tokio::spawn(async move {
        let Ok(_permit) = queue.acquire_owned().await else {
            return;
        };

        if let Err(e) = tx.send(()).await {
            tracing::debug!(error = %e, "test send failed");
        }
    });

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    drop(permit);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .is_ok_and(|msg| msg.is_some())
    );
    handle.await.unwrap();
}

#[tokio::test]
async fn a_request_without_a_config_is_reported_as_failed() {
    let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
    let (tx, mut rx) = mpsc::channel(8);

    executor
        .handle_delete_archives(RepoId(1), Vec::new(), "req-1".to_owned(), &tx)
        .await;

    let msg = rx.try_recv().unwrap();
    assert!(matches!(
        msg,
        AgentToServer::OperationFailed { request_id, error }
            if request_id == "req-1" && error == "agent has no config yet"
    ));
}

#[tokio::test]
async fn a_request_for_an_unknown_repo_is_reported_as_failed() {
    let task_registry = TaskRegistry::default();
    let executor = Executor::new("ws://localhost", "token", task_registry.clone());
    let (tx, mut rx) = mpsc::channel(8);
    let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
    configure_single_repo(&executor, &repo).await;

    executor
        .handle_restore_files(
            RestoreFilesParams {
                repo_id: RepoId(99),
                archive_name: "archive-1".to_owned(),
                paths: Vec::new(),
                target_path: "/tmp/restore".to_owned(),
                request_id: "req-2".to_owned(),
            },
            &tx,
        )
        .await;

    let msg = rx.try_recv().unwrap();
    assert!(matches!(
        msg,
        AgentToServer::OperationFailed { request_id, error }
            if request_id == "req-2" && error == "repo not found in agent config"
    ));
    assert_eq!(task_registry.pending_count(), 0);
}

#[tokio::test]
async fn maintenance_without_a_config_queues_nothing() {
    let task_registry = TaskRegistry::default();
    let executor = Executor::new("ws://localhost", "token", task_registry.clone());
    let (tx, mut rx) = mpsc::channel(8);

    executor
        .handle_maintenance(MaintenanceKind::Check, RepoId(1), &tx)
        .await;
    executor
        .handle_maintenance(MaintenanceKind::Verify, RepoId(1), &tx)
        .await;

    assert!(rx.try_recv().is_err());
    assert_eq!(task_registry.pending_count(), 0);
}

#[test]
fn maintenance_kind_names_the_borg_operation() {
    assert_eq!(MaintenanceKind::Check.to_string(), "check");
    assert_eq!(MaintenanceKind::Verify.to_string(), "verify");
}

mod database_dumps {
    use k8s_openapi::api::core::v1::Pod;
    use shared::{database::ANNOTATION_DATABASE, protocol::AgentToServer, types::BackupStatus};

    use super::*;
    use crate::{
        backup::{BackupEngine, BackupTarget},
        executor::{BackupTaskContext, ExecutorCommand, ScanDatabasesRequest, backup_task},
        tests::fake_kube::{FakeKube, Reply},
    };

    fn mock_borg() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/mock-borg/borg")
    }

    fn orders_pod() -> Pod {
        serde_json::from_value(serde_json::json!({
            "metadata": {
                "name": "db-0",
                "namespace": "shop",
                "annotations": {
                    ANNOTATION_DATABASE: "postgresql",
                    "assimilate.alexmohr.github.io/databases": "orders",
                },
            },
            "spec": { "containers": [{ "name": "postgres" }] },
            "status": { "phase": "Running" },
        }))
        .unwrap()
    }

    fn context() -> BackupTaskContext {
        BackupTaskContext {
            hostname: "hostname".to_owned(),
            server_url: "ws://127.0.0.1:9".to_owned(),
            token: "token".to_owned(),
            run_id: Some("run-1".to_owned()),
        }
    }

    async fn run(target: BackupTarget, engine: &BackupEngine) -> Vec<AgentToServer> {
        let (tx, mut rx) = mpsc::channel(64);
        backup_task::run_backup_task(shared::types::RepoId(1), target, context(), engine, &tx)
            .await;
        drop(tx);
        let mut sent = Vec::new();
        while let Some(msg) = rx.recv().await {
            sent.push(msg);
        }
        sent
    }

    fn completed(sent: &[AgentToServer]) -> &shared::types::BackupReport {
        sent.iter()
            .find_map(|msg| match msg {
                AgentToServer::BackupCompleted { report, .. } => Some(report),
                _ => None,
            })
            .expect("the run reports how it ended")
    }

    #[tokio::test]
    async fn a_backup_streams_its_dumps_and_clears_the_pipes_after() {
        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(vec![orders_pod()]).await;
        kube.reply(
            "shop",
            "db-0",
            "postgres",
            "--dbname=orders",
            Reply::ok(b"dump"),
        );
        let dump_dir = dir.path().join("dumps");
        let pipe = dump_dir.join("schedule-4/shop/db-0/orders.pgdump");
        let engine = BackupEngine::with_config(
            mock_borg(),
            vec![(
                "MOCK_BORG_READ_FILE".to_owned(),
                pipe.to_string_lossy().into_owned(),
            )],
        );
        let target = BackupTarget {
            schedule_id: Some(4),
            hostname: "hostname".to_owned(),
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                kubeconfig_path: Some(
                    kube.kubeconfig(dir.path())
                        .await
                        .to_string_lossy()
                        .into_owned(),
                ),
                dump_dir: dump_dir.to_string_lossy().into_owned(),
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };

        let sent = run(target, &engine).await;

        assert_eq!(completed(&sent).status, BackupStatus::Success);
        assert_eq!(kube.calls(), ["shop/db-0 postgres --dbname=orders"]);
        assert!(
            tokio::fs::metadata(dump_dir.join("schedule-4"))
                .await
                .is_err(),
            "the pipes are gone once the run is over"
        );
    }

    #[tokio::test]
    async fn a_backup_whose_dumps_cannot_be_prepared_fails_with_the_reason() {
        let engine = BackupEngine::with_config(mock_borg(), Vec::new());
        let target = BackupTarget {
            hostname: "hostname".to_owned(),
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };

        let sent = run(target, &engine).await;

        let report = completed(&sent);
        assert_eq!(report.status, BackupStatus::Failed);
        assert!(
            report
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("could not reach the Kubernetes cluster")
        );
    }

    #[tokio::test]
    async fn a_scan_command_is_answered_by_the_scan() {
        let executor = Executor::new("ws://localhost", "token", TaskRegistry::default());
        let (tx, mut rx) = mpsc::channel(4);
        executor
            .dispatch(
                ExecutorCommand::ScanDatabases(ScanDatabasesRequest {
                    request_id: "req-scan".to_owned(),
                    kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
                }),
                &tx,
            )
            .await;
        let Some(AgentToServer::DatabaseScanResult {
            request_id, error, ..
        }) = rx.recv().await
        else {
            panic!("a scan answers with its result");
        };
        assert_eq!(request_id, "req-scan");
        assert!(error.is_some());
    }

    #[tokio::test]
    async fn a_schedule_asking_for_databases_on_a_host_without_them_gets_none() {
        let mut schedule = make_schedule(10, vec!["/var"]);
        schedule.database_backup_enabled = true;
        let repo = make_repo(vec![schedule]);

        let (target, logs) = crate::tests::logs::logs_while(async {
            backup_target_from_repo(
                &repo,
                "hostname",
                Some(10),
                &VmSnapshotConfig::default(),
                &DatabaseBackupConfig::default(),
            )
        })
        .await;
        assert_eq!(target.database_backup, None);
        assert!(logs.contains("database dumps turned off"), "{logs}");

        let allowed = DatabaseBackupConfig {
            enabled: true,
            ..DatabaseBackupConfig::default()
        };
        let target = backup_target_from_repo(
            &repo,
            "hostname",
            Some(10),
            &VmSnapshotConfig::default(),
            &allowed,
        );
        assert_eq!(target.database_backup, Some(allowed));
    }

    #[tokio::test]
    async fn a_dry_run_for_a_configured_schedule_waits_its_turn_on_the_repository() {
        let task_registry = TaskRegistry::default();
        let executor = Executor::new("ws://localhost", "token", task_registry.clone());
        let repo = make_repo(vec![make_schedule(10, vec!["/var"])]);
        configure_single_repo(&executor, &repo).await;
        let queue = executor.repo_operation_queue(&repo_key_for(&repo)).await;
        let _permit = Arc::clone(&queue).acquire_owned().await.unwrap();
        let (tx, mut rx) = mpsc::channel(4);

        executor
            .handle_dry_run(repo.repo_id, 10, "req-dry".to_owned(), &tx)
            .await;

        assert_eq!(task_registry.pending_count(), 1);
        assert!(
            rx.try_recv().is_err(),
            "nothing runs while the repository is busy"
        );
    }
}
