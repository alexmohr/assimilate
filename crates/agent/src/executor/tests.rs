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
        fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
        lvm_snapshot_percent: shared::snapshot::DEFAULT_LVM_SNAPSHOT_PERCENT,
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        TaskRegistry::default(),
    );
    let (tx, mut rx) = mpsc::channel(8);
    let repo_id = shared::types::RepoId(1);

    executor.handle_cancel_backup(repo_id, &tx).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn cancel_backup_aborts_queued_task_and_sends_cancelled() {
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        TaskRegistry::default(),
    );
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        task_registry.clone(),
    );
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        TaskRegistry::default(),
    );
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        TaskRegistry::default(),
    );
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        task_registry.clone(),
    );
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
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        task_registry.clone(),
    );
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

/// A repository no borg can reach: SSH to it is refused at once, and where
/// borg is not installed the command fails before that.
fn unreachable_repo() -> shared::types::RepoConfig {
    shared::types::RepoConfig {
        ssh_host: "127.0.0.1".to_owned(),
        ssh_port: 1,
        ..make_repo(vec![make_schedule(10, vec!["/var"])])
    }
}

async fn next_reply(rx: &mut mpsc::Receiver<AgentToServer>) -> AgentToServer {
    tokio::time::timeout(Duration::from_mins(1), rx.recv())
        .await
        .expect("the task answers")
        .expect("the outbound channel stays open")
}

/// Every borg task the server can request runs with the server endpoint it
/// forwards SSH through, and reports back when borg fails.
#[tokio::test]
async fn requested_borg_tasks_report_a_failure_back() {
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://127.0.0.1:1"),
        TaskRegistry::default(),
    );
    configure_single_repo(&executor, &unreachable_repo()).await;
    let (tx, mut rx) = mpsc::channel(8);

    executor
        .handle_maintenance(MaintenanceKind::Check, RepoId(1), &tx)
        .await;
    let check = next_reply(&mut rx).await;
    assert!(
        matches!(
            check,
            AgentToServer::CheckCompleted {
                repo_id: RepoId(1),
                success: false,
                ..
            }
        ),
        "{check:?}"
    );

    executor
        .handle_dry_run(RepoId(1), 10, "req-dry".to_owned(), &tx)
        .await;
    let dry_run = next_reply(&mut rx).await;
    assert!(
        matches!(&dry_run, AgentToServer::OperationFailed { request_id, .. } if request_id == "req-dry"),
        "{dry_run:?}"
    );

    executor
        .handle_delete_archives(RepoId(1), vec!["old".to_owned()], "req-del".to_owned(), &tx)
        .await;
    let delete = next_reply(&mut rx).await;
    assert!(
        matches!(
            &delete,
            AgentToServer::DeleteArchivesResult { request_id, success: false, deleted_count: 0, .. }
                if request_id == "req-del"
        ),
        "{delete:?}"
    );

    executor
        .handle_init_repo(
            InitRepoParams {
                repo_path: "/backup/new",
                ssh_user: "borg",
                ssh_host: "127.0.0.1",
                ssh_port: 1,
                passphrase: "secret",
                encryption: BorgEncryption::Repokey,
            },
            &tx,
        )
        .await;
    let init = next_reply(&mut rx).await;
    assert!(
        matches!(
            &init,
            AgentToServer::InitRepoCompleted { repo_path, success: false, error_message: Some(_) }
                if repo_path == "/backup/new"
        ),
        "{init:?}"
    );
}

#[test]
fn maintenance_kind_names_the_borg_operation() {
    assert_eq!(MaintenanceKind::Check.to_string(), "check");
    assert_eq!(MaintenanceKind::Verify.to_string(), "verify");
}

mod database_dumps {
    #[cfg(unix)]
    use std::path::Path;

    #[cfg(unix)]
    use k8s_openapi::api::core::v1::Pod;
    #[cfg(unix)]
    use shared::database::{ANNOTATION_DATABASE, ANNOTATION_DATABASES};
    use shared::{database::DatabaseRuntimes, protocol::AgentToServer, types::BackupStatus};

    use super::*;
    #[cfg(unix)]
    use crate::tests::{
        fake_docker::{ExecReply, FakeDocker},
        fake_kube::{FakeKube, Reply},
    };
    use crate::{
        backup::{BackupEngine, BackupTarget},
        executor::{BackupTaskContext, ExecutorCommand, ScanDatabasesRequest, backup_task},
    };

    fn mock_borg() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/mock-borg/borg")
    }

    #[cfg(unix)]
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
            server: crate::endpoint::test_support::endpoint("ws://127.0.0.1:9"),
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

    /// Runs a backup of schedule 4 that dumps databases with `runtimes`,
    /// while borg reads the dump at `pipe` below the schedule's dump
    /// directory. Returns how the run ended, and whether the pipes were
    /// cleared after it.
    #[cfg(unix)]
    async fn backup_dumping(
        dir: &Path,
        runtimes: DatabaseRuntimes,
        pipe: &str,
    ) -> (BackupStatus, bool) {
        let dump_dir = dir.join("dumps");
        let engine = BackupEngine::with_config(
            mock_borg(),
            vec![(
                "MOCK_BORG_READ_FILE".to_owned(),
                dump_dir
                    .join("schedule-4")
                    .join(pipe)
                    .to_string_lossy()
                    .into_owned(),
            )],
        );
        let target = BackupTarget {
            schedule_id: Some(4),
            hostname: "hostname".to_owned(),
            database_backup: Some(DatabaseBackupConfig {
                enabled: true,
                runtimes,
                dump_dir: dump_dir.to_string_lossy().into_owned(),
                ..DatabaseBackupConfig::default()
            }),
            ..BackupTarget::default()
        };
        let sent = run(target, &engine).await;
        let cleared = tokio::fs::metadata(dump_dir.join("schedule-4"))
            .await
            .is_err();
        (completed(&sent).status, cleared)
    }

    #[cfg(unix)]
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

        let (status, cleared) =
            backup_dumping(dir.path(), runtimes, "kubernetes/shop/db-0/orders.pgdump").await;

        assert_eq!(status, BackupStatus::Success);
        assert_eq!(kube.calls(), ["shop/db-0 postgres --dbname=orders"]);
        assert!(cleared, "the pipes are gone once the run is over");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_backup_streams_a_containers_dump() {
        let dir = tempfile::tempdir().unwrap();
        let container = bollard::models::ContainerSummary {
            id: Some("c1".to_owned()),
            names: Some(vec!["/wiki-db".to_owned()]),
            labels: Some(
                [
                    (ANNOTATION_DATABASE.to_owned(), "postgresql".to_owned()),
                    (ANNOTATION_DATABASES.to_owned(), "wiki".to_owned()),
                ]
                .into(),
            ),
            ..bollard::models::ContainerSummary::default()
        };
        let docker = FakeDocker::start(vec![container]);
        docker.reply("c1", "--dbname=wiki", ExecReply::exit(0, b"dump", ""));
        let runtimes = DatabaseRuntimes {
            docker: true,
            docker_socket: Some(docker.socket()),
            ..DatabaseRuntimes::default()
        };

        let (status, cleared) =
            backup_dumping(dir.path(), runtimes, "docker/wiki-db/wiki.pgdump").await;

        assert_eq!(status, BackupStatus::Success);
        assert_eq!(docker.calls().len(), 1);
        assert!(cleared);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_backup_streams_the_hosts_own_dump() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        tokio::fs::create_dir(&bin).await.unwrap();
        for (program, body) in [("psql", "echo app"), ("pg_dump", "printf dump")] {
            let script = bin.join(program);
            tokio::fs::write(&script, format!("#!/bin/sh\n{body}\n"))
                .await
                .unwrap();
            tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
        }
        let me = nix::unistd::User::from_uid(nix::unistd::getuid())
            .unwrap()
            .unwrap()
            .name;
        let runtimes = DatabaseRuntimes {
            host: true,
            host_os_user: Some(me),
            host_bin_dir: Some(bin.to_string_lossy().into_owned()),
            ..DatabaseRuntimes::default()
        };

        let (status, cleared) =
            backup_dumping(dir.path(), runtimes, "host/postgresql/app.pgdump").await;

        assert_eq!(status, BackupStatus::Success);
        assert!(cleared);
    }

    #[tokio::test]
    async fn a_backup_whose_dumps_cannot_be_prepared_fails_with_the_reason() {
        let engine = BackupEngine::with_config(mock_borg(), Vec::new());
        let target = BackupTarget {
            hostname: "hostname".to_owned(),
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
    async fn a_config_update_is_what_the_next_scan_searches() {
        let executor = Executor::new(
            crate::endpoint::test_support::endpoint("ws://localhost"),
            TaskRegistry::default(),
        );
        let (tx, _rx) = mpsc::channel(4);
        let config = shared::types::AgentConfig {
            agent_hostname: "hostname".to_owned(),
            vm_snapshot: VmSnapshotConfig::default(),
            database_backup: DatabaseBackupConfig {
                runtimes: DatabaseRuntimes {
                    docker: true,
                    ..DatabaseRuntimes::default()
                },
                ..DatabaseBackupConfig::default()
            },
            skip_targets: Vec::new(),
            repos: Vec::new(),
        };

        executor
            .dispatch(ExecutorCommand::UpdateConfig(Box::new(config)), &tx)
            .await;

        let stored = executor.current_config.lock().await;
        assert!(stored.as_ref().unwrap().database_backup.runtimes.docker);
    }

    #[tokio::test]
    async fn a_scan_command_is_answered_by_the_scan() {
        let executor = Executor::new(
            crate::endpoint::test_support::endpoint("ws://localhost"),
            TaskRegistry::default(),
        );
        let (tx, mut rx) = mpsc::channel(4);
        executor
            .dispatch(
                ExecutorCommand::ScanDatabases(ScanDatabasesRequest {
                    request_id: "req-scan".to_owned(),
                    runtimes: Some(Box::new(DatabaseRuntimes {
                        kubernetes: true,
                        kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
                        ..DatabaseRuntimes::default()
                    })),
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
        let executor = Executor::new(
            crate::endpoint::test_support::endpoint("ws://localhost"),
            task_registry.clone(),
        );
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

/// A live read answers with the file when it is inside a source of the
/// current configuration, and refuses it once the configuration no longer
/// covers it.
#[tokio::test]
async fn a_live_read_follows_the_current_sources() {
    use shared::live_file::{LiveFile, LiveReadRefusal};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    tokio::fs::write(root.join("app.conf"), "port = 80\n")
        .await
        .unwrap();
    let archive_path = root
        .join("app.conf")
        .to_str()
        .unwrap()
        .trim_start_matches('/')
        .to_owned();
    let task_registry = TaskRegistry::default();
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        task_registry.clone(),
    );
    let (tx, mut rx) = mpsc::channel(8);

    let repo = make_repo(vec![make_schedule(10, vec![root.to_str().unwrap()])]);
    configure_single_repo(&executor, &repo).await;
    executor
        .handle_read_live_file(
            LiveFileRequest {
                request_id: "req-in".to_owned(),
                path: archive_path.clone(),
            },
            &tx,
        )
        .await;
    let inside = rx.recv().await.unwrap();

    configure_single_repo(&executor, &make_repo(vec![make_schedule(10, vec!["/var"])])).await;
    executor
        .handle_read_live_file(
            LiveFileRequest {
                request_id: "req-out".to_owned(),
                path: archive_path,
            },
            &tx,
        )
        .await;
    let outside = rx.recv().await.unwrap();

    assert!(matches!(
        inside,
        AgentToServer::LiveFileRead { request_id, file: LiveFile::Text { text, .. } }
            if request_id == "req-in" && text.0 == "port = 80\n"
    ));
    assert!(matches!(
        outside,
        AgentToServer::LiveFileRead {
            request_id,
            file: LiveFile::Refused { reason: LiveReadRefusal::NotInSources },
        } if request_id == "req-out"
    ));
}

#[tokio::test]
async fn a_live_read_whose_answer_cannot_be_sent_is_logged() {
    let task_registry = TaskRegistry::default();
    let executor = Executor::new(
        crate::endpoint::test_support::endpoint("ws://localhost"),
        task_registry.clone(),
    );
    let (tx, rx) = mpsc::channel(1);
    drop(rx);

    let ((), logs) = crate::tests::logs::logs_while(async {
        executor
            .handle_read_live_file(
                LiveFileRequest {
                    request_id: "req-gone".to_owned(),
                    path: "etc/hosts".to_owned(),
                },
                &tx,
            )
            .await;
        task_registry.shutdown(Duration::from_secs(10)).await;
    })
    .await;

    assert!(
        logs.contains("Outbound channel closed; live file answer dropped"),
        "{logs}"
    );
}

#[cfg(unix)]
mod restore_to_destination {
    use shared::protocol::AgentToServer;

    use super::*;
    use crate::{
        backup::BackupTarget,
        executor::{FreeTaskContext, RestoreTaskParams, archives::run_restore_task},
    };

    fn mock_borg() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/mock-borg/borg")
    }

    async fn restore(borg: &Borg, paths: &[&str], target_path: &str) -> Vec<AgentToServer> {
        let (tx, mut rx) = mpsc::channel(8);
        run_restore_task(
            RestoreTaskParams {
                repo_id: RepoId(1),
                target: BackupTarget {
                    hostname: "hostname".to_owned(),
                    ..BackupTarget::default()
                },
                archive_name: "archive-1".to_owned(),
                paths: paths.iter().map(|p| (*p).to_owned()).collect(),
                target_path: target_path.to_owned(),
                request_id: "req-restore".to_owned(),
            },
            FreeTaskContext {
                hostname: "hostname",
                server: &crate::endpoint::test_support::endpoint("ws://127.0.0.1:9"),
                outbound_tx: &tx,
            },
            borg,
        )
        .await;
        drop(tx);
        let mut sent = Vec::new();
        while let Some(msg) = rx.recv().await {
            sent.push(msg);
        }
        sent
    }

    /// A basket is one `borg extract` of every selected path, run inside the
    /// destination, so each path lands under it with its archive structure.
    #[tokio::test]
    async fn every_path_lands_under_the_destination_in_one_borg_run() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("borg.log");
        let destination = dir.path().join("restore/target");
        let borg = Borg::with_extra_env(
            mock_borg(),
            vec![
                ("MOCK_BORG_EXTRACT_CREATE".to_owned(), "1".to_owned()),
                (
                    "MOCK_BORG_LOG".to_owned(),
                    log.to_string_lossy().into_owned(),
                ),
            ],
        );

        let sent = restore(
            &borg,
            &["etc/nginx/nginx.conf", "var/www/html/index.html"],
            &format!("{}/", destination.to_string_lossy()),
        )
        .await;

        assert!(matches!(
            sent.as_slice(),
            [AgentToServer::RestoreCompleted { request_id, success: true, files_restored: 2, .. }]
                if request_id == "req-restore"
        ));
        assert!(
            tokio::fs::metadata(destination.join("etc/nginx/nginx.conf"))
                .await
                .is_ok()
        );
        assert!(
            tokio::fs::metadata(destination.join("var/www/html/index.html"))
                .await
                .is_ok()
        );
        let calls = tokio::fs::read_to_string(&log).await.unwrap();
        let extracts: Vec<&str> = calls.lines().filter(|l| l.contains(" extract ")).collect();
        assert_eq!(
            extracts.len(),
            1,
            "one borg run for the whole basket: {calls}"
        );
        assert!(
            extracts
                .first()
                .is_some_and(|l| l.ends_with("-- etc/nginx/nginx.conf var/www/html/index.html"))
        );
    }

    /// The extract is logged with how many paths it restores, so a restore
    /// shows up in the agent's log before borg finishes.
    #[tokio::test]
    async fn the_extract_is_logged_with_its_path_count() {
        let dir = tempfile::tempdir().unwrap();
        let borg = Borg::with_extra_env(
            mock_borg(),
            vec![("MOCK_BORG_EXTRACT_CREATE".to_owned(), "1".to_owned())],
        );
        let destination = dir.path().join("restore").to_string_lossy().into_owned();

        let (sent, logs) = crate::tests::logs::logs_while(restore(
            &borg,
            &["etc/hosts", "etc/hostname", "etc/fstab"],
            &destination,
        ))
        .await;

        assert!(matches!(
            sent.as_slice(),
            [AgentToServer::RestoreCompleted { success: true, .. }]
        ));
        let extract_line = logs
            .lines()
            .find(|line| line.contains("running borg extract"))
            .unwrap_or_default();
        assert!(extract_line.contains("paths=3"), "{logs}");
        assert!(extract_line.contains("archive=archive-1"), "{logs}");
    }

    /// The agent does not take the server's word for the destination: one
    /// that is not a plain absolute path fails before borg runs.
    #[tokio::test]
    async fn an_unsafe_destination_is_refused_without_running_borg() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("borg.log");
        let borg = Borg::with_extra_env(
            mock_borg(),
            vec![(
                "MOCK_BORG_LOG".to_owned(),
                log.to_string_lossy().into_owned(),
            )],
        );

        for target in ["relative/restore", "/srv/../etc", "", "C:\\restore"] {
            let sent = restore(&borg, &["etc/hosts"], target).await;
            assert!(
                matches!(
                    sent.as_slice(),
                    [AgentToServer::OperationFailed { request_id, error }]
                        if request_id == "req-restore" && error.contains("refusing to restore")
                ),
                "{target:?} must be refused, got {sent:?}"
            );
        }
        assert!(tokio::fs::metadata(&log).await.is_err(), "borg never ran");
    }
}
