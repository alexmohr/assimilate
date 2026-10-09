// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::types::{Compression, FileChangePattern};

use super::{
    parse::{
        describe_borg_failure, filter_file_change_warnings, parse_json_stats,
        parse_source_not_found_errors, parse_warnings, stderr_has_warnings, warning_kind,
    },
    *,
};

fn mock_borg_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/mock-borg/borg")
}

fn test_target() -> BackupTarget {
    BackupTarget {
        target_name: "test-target".to_owned(),
        schedule_id: None,
        repo_path: "backup/test".to_owned(),
        ssh_user: "borg".to_owned(),
        ssh_host: "backup-server".to_owned(),
        ssh_port: 22,
        ssh_host_key: "ssh-ed25519 AAAATEST".to_owned(),
        known_hosts_path: None,
        passphrase: "test-passphrase".to_owned(),
        hostname: "test-host".to_owned(),
        compression: Compression::Lz4,
        backup_sources: vec!["/tmp".to_owned()],
        keep_hourly: 24,
        keep_daily: 7,
        keep_weekly: 4,
        keep_monthly: 6,
        keep_yearly: 0,
        compact_enabled: true,
        vm_snapshot: None,
        pre_backup_commands: Vec::new(),
        post_backup_commands: Vec::new(),
        hook_timeout_seconds: 60,
        skip_targets: Vec::new(),
        exclude_patterns: vec!["*.tmp".to_owned(), "/proc/*".to_owned()],
        include_patterns: Vec::new(),
        rate_limit_kbps: None,
        ssh_auth_sock: None,
        canary_enabled: false,
        accept_relocation: false,
        file_change_patterns: Vec::new(),
    }
}

#[test]
fn test_rate_limit_flag_included() {
    let mut target = test_target();
    target.rate_limit_kbps = Some(5000);

    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );

    assert!(args.iter().any(|arg| arg == "--upload-ratelimit"));
    assert!(args.iter().any(|arg| arg == "5000"));
}

#[test]
fn test_rate_limit_flag_absent_when_zero() {
    let mut target = test_target();
    target.rate_limit_kbps = Some(0);

    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );

    assert!(!args.iter().any(|arg| arg == "--upload-ratelimit"));
}

#[test]
fn borg_create_args_includes_separator_before_sources() {
    let target = test_target();
    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );
    let archive_spec_pos = args.iter().position(|a| a.starts_with("::"));
    let separator_pos = args.iter().position(|a| a == "--");
    let sources_start = args
        .iter()
        .position(|a| a.as_str() == target.backup_sources[0]);

    assert_eq!(separator_pos, Some(archive_spec_pos.unwrap() + 1));
    assert_eq!(sources_start, Some(separator_pos.unwrap() + 1));
}

#[test]
fn borg_create_args_no_separator_when_no_sources() {
    let target = BackupTarget {
        backup_sources: vec![],
        ..test_target()
    };
    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );
    assert!(!args.iter().any(|a| a == "--"));
}

#[test]
fn test_rate_limit_flag_absent() {
    let target = test_target();

    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );

    assert!(!args.iter().any(|arg| arg == "--upload-ratelimit"));
}

#[test]
fn borg_create_args_omits_patterns_from_without_include_patterns() {
    let target = test_target();

    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        None,
        "archive-name",
    );

    assert!(!args.iter().any(|a| a == "--patterns-from"));
}

#[test]
fn borg_create_args_includes_patterns_from_before_exclude_from() {
    let target = test_target();

    let args = BackupEngine::borg_create_args(
        &target,
        &target.backup_sources,
        Path::new("/tmp/excludes"),
        Some(Path::new("/tmp/includes")),
        "archive-name",
    );

    let patterns_from_pos = args.iter().position(|a| a == "--patterns-from").unwrap();
    assert_eq!(args[patterns_from_pos + 1], "/tmp/includes");
    let exclude_from_pos = args.iter().position(|a| a == "--exclude-from").unwrap();
    assert_eq!(args[exclude_from_pos + 1], "/tmp/excludes");
    assert!(
        patterns_from_pos < exclude_from_pos,
        "include patterns must be checked before excludes so they can rescue a path"
    );
}

#[test]
fn write_include_patterns_file_returns_none_when_empty() {
    assert!(
        BackupEngine::write_include_patterns_file(&[])
            .unwrap()
            .is_none()
    );
}

#[test]
fn write_include_patterns_file_prefixes_each_pattern_with_plus() {
    let file = BackupEngine::write_include_patterns_file(&[
        "/home/keep".to_owned(),
        "pp:/var/keep".to_owned(),
    ])
    .unwrap()
    .unwrap();
    let content = std::fs::read_to_string(file.path()).unwrap();
    assert_eq!(content, "+ /home/keep\n+ pp:/var/keep\n");
}

#[test]
fn preview_create_command_includes_patterns_from_when_target_has_include_patterns() {
    let target = BackupTarget {
        include_patterns: vec!["/home/keep".to_owned()],
        ..test_target()
    };
    let command = BackupEngine::preview_create_command(&target);
    assert!(command.contains("--patterns-from <include-file>"));
}

#[test]
fn preview_create_command_omits_patterns_from_without_include_patterns() {
    let target = test_target();
    let command = BackupEngine::preview_create_command(&target);
    assert!(!command.contains("--patterns-from"));
}

#[test]
fn borg_env_uses_pinned_known_hosts_file() {
    let known_hosts = tempfile::NamedTempFile::new().unwrap();
    let mut target = test_target();
    target.known_hosts_path = Some(known_hosts.path().to_path_buf());
    let env = borg_env(&target);
    let borg_rsh = env
        .iter()
        .find(|(key, _value)| key == "BORG_RSH")
        .map(|(_key, value)| value.as_str());
    let expected = shared::ssh::borg_rsh_with_known_hosts(known_hosts.path());

    assert_eq!(borg_rsh, Some(expected.as_str()));
}

#[tokio::test]
async fn test_successful_backup() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Success);
    assert_eq!(result.original_size, 1_073_741_824);
    assert_eq!(result.compressed_size, 536_870_912);
    assert_eq!(result.deduplicated_size, 268_435_456);
    assert_eq!(result.repo_unique_csize, 402_653_184);
    assert_eq!(result.files_processed, 1234);
    assert!(result.error_message.is_none());
    assert_eq!(result.warnings.len(), 0);
}

#[tokio::test]
async fn a_run_that_only_saw_files_change_is_a_file_changed_warning() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
    );

    let result = engine.run_backup(&test_target(), None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert_eq!(result.warning_kind, BackupWarningKind::FileChanged);
}

#[tokio::test]
async fn an_unexplained_warning_exit_is_a_general_warning() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_SIMULATE_UNEXPLAINED_WARNING".to_owned(),
            "1".to_owned(),
        )],
    );

    let result = engine.run_backup(&test_target(), None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert_eq!(result.warning_kind, BackupWarningKind::General);
}

#[test]
fn warning_kind_is_file_changed_only_when_every_warning_is_one() {
    let changed = vec!["/a: file changed while we backed it up".to_owned()];
    let other = "/b: [Errno 13] Permission denied".to_owned();

    assert_eq!(
        warning_kind(&changed, &changed),
        BackupWarningKind::FileChanged
    );
    assert_eq!(
        warning_kind(&[changed[0].clone(), other], &changed),
        BackupWarningKind::General
    );
    assert_eq!(warning_kind(&[], &changed), BackupWarningKind::General);
}

#[tokio::test]
async fn test_file_changed_warning() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
    );
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert!(result.error_message.is_some());
    assert!(
        result
            .error_message
            .as_ref()
            .unwrap()
            .contains("file changed")
    );
    assert_eq!(result.warnings.len(), 2);
    assert!(result.warnings[0].contains("file changed while we backed it up"));
    assert!(result.warnings[1].contains("file changed while we backed it up"));
}

#[tokio::test]
async fn test_borg_failure() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![("MOCK_BORG_FAIL".to_owned(), "1".to_owned())],
    );
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(
        matches!(err, BackupError::BorgFailed(_)),
        "Expected BorgFailed, got: {err:?}"
    );
}

#[tokio::test]
async fn test_unrelated_fatal_exit_is_not_masked_by_fatal_pattern() {
    // Regression test: an unrelated hard failure (e.g. repository error,
    // exit code 2) must surface its own message even if the stderr also
    // happens to contain a warning that matches a `fatal` file-change
    // pattern. The fatal-pattern check must only apply on the exit-code
    // paths that actually determine success/warning status.
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![("MOCK_BORG_FATAL_UNRELATED".to_owned(), "1".to_owned())],
    );
    let mut target = test_target();
    target.file_change_patterns = vec![FileChangePattern {
        path: "*/etc/config*".to_owned(),
        action: shared::types::FileChangeAction::Fatal,
    }];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = &err else {
        panic!("Expected BorgFailed, got: {err:?}");
    };
    assert!(
        msg.contains("Repository ID mismatch"),
        "Expected the original borg failure message, got: {err:?}"
    );
}

#[tokio::test]
async fn test_pre_backup_command_success() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.pre_backup_commands = vec![HookCommand::new("true")];

    let result = engine.run_backup(&target, None, None).await.unwrap();
    assert_eq!(result.status, BackupStatus::Success);
}

#[tokio::test]
async fn test_pre_backup_command_failure() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.pre_backup_commands = vec![HookCommand::new("false")];

    let result = engine.run_backup(&target, None, None).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), BackupError::BorgFailed(_)));
}

#[tokio::test]
async fn test_pre_backup_command_failure_includes_stderr() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.pre_backup_commands = vec![HookCommand::new("echo 'connection refused' >&2 && exit 1")];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("connection refused"),
        "error message should include stderr: {msg}"
    );
}

#[tokio::test]
async fn test_pre_backup_command_failure_includes_stdout() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.pre_backup_commands = vec![HookCommand::new(
        "echo 'retrying connection' && echo 'connection refused' >&2 && exit 1",
    )];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("retrying connection"),
        "error message should include stdout: {msg}"
    );
    assert!(
        msg.contains("connection refused"),
        "error message should include stderr: {msg}"
    );
}

#[tokio::test]
async fn test_post_backup_command_failure_includes_stdout() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.post_backup_commands = vec![HookCommand::new(
        "echo 'cleanup starting' && echo 'cleanup failed' >&2 && exit 1",
    )];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("cleanup starting"),
        "error message should include stdout: {msg}"
    );
    assert!(
        msg.contains("cleanup failed"),
        "error message should include stderr: {msg}"
    );
}

#[tokio::test]
async fn test_hook_command_failure_keeps_stderr_when_stdout_is_huge() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    // Stdout alone exceeds MAX_FAILURE_CHARS (4000): truncating the
    // combined message from the front would cut the string off inside
    // or before "stderr:", silently dropping the actual error.
    target.pre_backup_commands = vec![HookCommand::new(
        "head -c 5000 /dev/zero | tr '\\0' 'a'; echo 'db connection refused' >&2; exit 1",
    )];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("stderr:"),
        "message should still have a stderr section: {msg}"
    );
    assert!(
        msg.contains("db connection refused"),
        "a large stdout must not push stderr out of the message: {msg}"
    );
}

#[tokio::test]
async fn test_pre_backup_command_respects_configured_timeout() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.hook_timeout_seconds = 1;
    target.pre_backup_commands = vec![HookCommand::new("sleep 5")];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("timed out after 1 seconds"),
        "error message should reflect the configured timeout: {msg}"
    );
}

#[tokio::test]
async fn test_pre_backup_command_allows_longer_configured_timeout() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.hook_timeout_seconds = 5;
    target.pre_backup_commands = vec![HookCommand::new("sleep 1")];

    let result = engine.run_backup(&target, None, None).await.unwrap();
    assert_eq!(result.status, BackupStatus::Success);
}

#[tokio::test]
async fn test_pre_backup_command_timeout_overrides_schedule_default() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.hook_timeout_seconds = 60;
    target.pre_backup_commands = vec![HookCommand {
        command: "sleep 5".to_owned(),
        timeout_seconds: Some(1),
    }];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("timed out after 1 seconds"),
        "the command's own timeout should win over the schedule default: {msg}"
    );
}

#[tokio::test]
async fn test_pre_backup_command_timeout_can_exceed_schedule_default() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.hook_timeout_seconds = 1;
    target.pre_backup_commands = vec![HookCommand {
        command: "sleep 2".to_owned(),
        timeout_seconds: Some(30),
    }];

    let result = engine.run_backup(&target, None, None).await.unwrap();
    assert_eq!(result.status, BackupStatus::Success);
}

#[tokio::test]
async fn test_post_backup_command_uses_its_own_timeout() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.hook_timeout_seconds = 60;
    target.post_backup_commands = vec![HookCommand {
        command: "sleep 5".to_owned(),
        timeout_seconds: Some(1),
    }];

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("post-backup hook command timed out after 1 seconds"),
        "post-backup hooks honour their own timeout: {msg}"
    );
}

#[tokio::test]
async fn test_skip_targets() {
    let engine = BackupEngine::with_config(mock_borg_path(), vec![]);
    let mut target = test_target();
    target.skip_targets = vec![target.target_name.clone()];

    let result = engine.run_backup(&target, None, None).await;
    assert!(matches!(result.unwrap_err(), BackupError::Skipped(_)));
}

#[tokio::test]
async fn test_compression_arg() {
    assert_eq!(BackupEngine::compression_arg(&Compression::None), "none");
    assert_eq!(BackupEngine::compression_arg(&Compression::Lz4), "lz4");
    assert_eq!(
        BackupEngine::compression_arg(&Compression::Zstd { level: 3 }),
        "zstd,3"
    );
    assert_eq!(
        BackupEngine::compression_arg(&Compression::Zlib { level: 6 }),
        "zlib,6"
    );
}

#[tokio::test]
async fn test_parse_json_stats() {
    let json = r#"{
        "archive": {
            "name": "test",
            "stats": {
                "original_size": 100,
                "compressed_size": 80,
                "deduplicated_size": 50,
                "nfiles": 42
            }
        }
    }"#;

    let stats = parse_json_stats(json.as_bytes()).unwrap();
    assert_eq!(stats.original_size, 100);
    assert_eq!(stats.compressed_size, 80);
    assert_eq!(stats.deduplicated_size, 50);
    assert_eq!(stats.repo_unique_csize, 0);
    assert_eq!(stats.files_processed, 42);
}

#[tokio::test]
async fn test_parse_json_stats_with_cache() {
    let json = r#"{
        "archive": {
            "name": "test",
            "stats": {
                "original_size": 100,
                "compressed_size": 80,
                "deduplicated_size": 50,
                "nfiles": 42
            }
        },
        "cache": {
            "stats": {
                "total_size": 1000,
                "total_csize": 800,
                "unique_size": 400,
                "unique_csize": 300,
                "total_unique_chunks": 10,
                "total_chunks": 40
            }
        }
    }"#;

    let stats = parse_json_stats(json.as_bytes()).unwrap();
    assert_eq!(stats.original_size, 100);
    assert_eq!(stats.compressed_size, 80);
    assert_eq!(stats.deduplicated_size, 50);
    assert_eq!(stats.repo_unique_csize, 300);
    assert_eq!(stats.files_processed, 42);
}

#[test]
fn test_parse_warnings_json() {
    let stderr = [
        concat!(
            r#"{"type": "log_message", "time": 1704067200, "#,
            r#""levelname": "WARNING", "name": "borg.archive", "#,
            r#""msgid": "FileChangedWarning", "#,
            r#""message": "/tmp/test.log: file changed"}"#,
        ),
        concat!(
            r#"{"type": "log_message", "time": 1704067200, "#,
            r#""levelname": "INFO", "name": "borg.archive", "#,
            r#""message": "some info"}"#,
        ),
        concat!(
            r#"{"type": "log_message", "time": 1704067200, "#,
            r#""levelname": "ERROR", "name": "borg.archive", "#,
            r#""msgid": "BackupError", "#,
            r#""message": "some error"}"#,
        ),
    ]
    .join("\n");

    let warnings = parse_warnings(&stderr);
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("file changed"));
    assert_eq!(warnings[1], "some error");
}

#[test]
fn test_parse_warnings_ignores_non_log_types() {
    let stderr = [
        r#"{"type": "archive_progress", "original_size": 100}"#,
        r#"{"type": "file_status", "status": "A", "path": "/a"}"#,
    ]
    .join("\n");

    let warnings = parse_warnings(&stderr);
    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_stderr_has_warnings_json() {
    let with_warning = r#"{"type": "log_message", "levelname": "WARNING", "message": "oops"}"#;
    assert!(stderr_has_warnings(with_warning));

    let without_warning = r#"{"type": "log_message", "levelname": "INFO", "message": "ok"}"#;
    assert!(!stderr_has_warnings(without_warning));

    assert!(!stderr_has_warnings("plain text warning"));
}

#[test]
fn test_parse_source_not_found_errors_detects_msgid() {
    let stderr = concat!(
        r#"{"type": "log_message", "time": 1704067200, "levelname": "WARNING", "#,
        r#""name": "borg.archiver", "msgid": "BackupFileNotFoundError", "#,
        r#""message": "/mnt/photos: stat: [Errno 2] No such file or directory"}"#,
    );
    let errors = parse_source_not_found_errors(stderr);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("/mnt/photos"));
}

#[test]
fn test_parse_source_not_found_errors_ignores_other_warnings() {
    let stderr = concat!(
        r#"{"type": "log_message", "levelname": "WARNING", "#,
        r#""msgid": "FileChangedWarning", "message": "/tmp/test.log: file changed"}"#,
    );
    let errors = parse_source_not_found_errors(stderr);
    assert_eq!(errors.len(), 0);
}

#[tokio::test]
async fn test_missing_backup_source_is_error_not_warning() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_SIMULATE_SOURCE_NOT_FOUND".to_owned(),
            "1".to_owned(),
        )],
    );
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    let BackupError::BorgFailed(msg) = err else {
        panic!("expected BorgFailed, got {err:?}");
    };
    assert!(
        msg.contains("backup source(s) not found"),
        "error should mention missing source: {msg}"
    );
    assert!(
        msg.contains("/mnt/missing"),
        "error should include the missing path: {msg}"
    );
}

#[tokio::test]
async fn test_all_zero_retention_skips_prune() {
    let log_file = tempfile::NamedTempFile::new().unwrap();
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_LOG".to_owned(),
            log_file.path().to_string_lossy().into_owned(),
        )],
    );
    let mut target = test_target();
    target.keep_hourly = 0;
    target.keep_daily = 0;
    target.keep_weekly = 0;
    target.keep_monthly = 0;
    target.keep_yearly = 0;

    let result = engine.run_backup(&target, None, None).await.unwrap();
    assert_eq!(result.status, BackupStatus::Success);

    let log = std::fs::read_to_string(log_file.path()).unwrap();
    assert!(
        !log.contains("prune"),
        "prune should not be called when all retention values are zero, but log contains: {log}"
    );
}

#[tokio::test]
async fn test_partial_retention_only_includes_nonzero_keep_flags() {
    let log_file = tempfile::NamedTempFile::new().unwrap();
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_LOG".to_owned(),
            log_file.path().to_string_lossy().into_owned(),
        )],
    );
    let mut target = test_target();
    target.keep_hourly = 0;
    target.keep_daily = 7;
    target.keep_weekly = 0;
    target.keep_monthly = 3;
    target.keep_yearly = 0;

    let result = engine.run_backup(&target, None, None).await.unwrap();
    assert_eq!(result.status, BackupStatus::Success);

    let log = std::fs::read_to_string(log_file.path()).unwrap();
    assert!(log.contains("prune"), "prune should be called");
    assert!(
        log.contains("--keep-daily"),
        "keep-daily flag expected in: {log}"
    );
    assert!(
        log.contains("--keep-monthly"),
        "keep-monthly flag expected in: {log}"
    );
    assert!(
        !log.contains("--keep-hourly"),
        "keep-hourly should not appear when zero, but found in: {log}"
    );
    assert!(
        !log.contains("--keep-weekly"),
        "keep-weekly should not appear when zero, but found in: {log}"
    );
    assert!(
        !log.contains("--keep-yearly"),
        "keep-yearly should not appear when zero, but found in: {log}"
    );
}

#[test]
fn test_filter_file_change_warnings_passthrough() {
    let warnings = vec!["file changed".to_owned(), "other warning".to_owned()];
    let result = filter_file_change_warnings(warnings, &[]).unwrap();
    assert_eq!(result.len(), 2);
}

#[test]
fn test_filter_file_change_warnings_ignore() {
    // Note: glob-match * does not match /, so patterns match warning message suffixes
    let patterns = vec![FileChangePattern {
        path: "*test.log: file changed".to_owned(),
        action: shared::types::FileChangeAction::Ignore,
    }];
    let warnings = vec![
        "test.log: file changed".to_owned(),
        "other warning".to_owned(),
    ];
    let result = filter_file_change_warnings(warnings, &patterns).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].contains("other warning"));
}

#[test]
fn describe_borg_failure_prefers_diagnostics_over_raw_json() {
    let stderr = [
        r#"{"type": "archive_progress", "original_size": 100}"#,
        r#"{"type": "log_message", "levelname": "ERROR", "message": "Connection closed"}"#,
        "borg: Fatal: Repository ID mismatch.",
    ]
    .join("\n");

    let described = describe_borg_failure(&stderr);

    assert_eq!(
        described,
        "Connection closed; borg: Fatal: Repository ID mismatch."
    );
}

#[tokio::test]
async fn unexplained_warning_exit_reports_why_it_is_flagged() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_SIMULATE_UNEXPLAINED_WARNING".to_owned(),
            "1".to_owned(),
        )],
    );
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert_eq!(result.warnings.len(), 1);
    let warning = &result.warnings[0];
    assert!(
        !warning.contains("terminating with warning status"),
        "the --show-rc footer explains nothing: {warning}"
    );
    assert!(
        warning.contains("borg exited with code 1"),
        "the warning should name the exit code: {warning}"
    );
    assert!(
        warning.contains("Creating archive at"),
        "the warning should carry borg's last output: {warning}"
    );
    assert_eq!(result.error_message.as_ref(), Some(warning));
}

#[test]
fn warning_status_log_speaks_up_when_the_footer_was_the_only_output() {
    let stderr = concat!(
        r#"{"type": "log_message", "levelname": "WARNING", "#,
        r#""message": "terminating with warning status, rc 1"}"#,
    );

    let logged = warning_status_log("prune", 1, stderr);

    assert!(
        logged.contains("borg prune exited with code 1"),
        "a prune that flagged something must not go unlogged: {logged}"
    );
}

#[test]
fn warning_status_log_lists_the_diagnostics_when_there_are_any() {
    let stderr = [
        r#"{"type": "log_message", "levelname": "WARNING", "message": "stale lock removed"}"#,
        concat!(
            r#"{"type": "log_message", "levelname": "WARNING", "#,
            r#""message": "terminating with warning status, rc 1"}"#,
        ),
    ]
    .join("\n");

    let logged = warning_status_log("check", 1, &stderr);

    assert_eq!(logged, "borg check warnings: stale lock removed");
}

#[test]
fn describe_borg_failure_says_so_when_only_the_footer_was_printed() {
    let stderr = concat!(
        r#"{"type": "log_message", "levelname": "WARNING", "#,
        r#""message": "terminating with error status, rc 2"}"#,
    );

    let described = describe_borg_failure(stderr);

    assert_eq!(
        described,
        "borg reported no diagnostic beyond its exit status"
    );
}

#[tokio::test]
async fn an_ignore_pattern_cannot_turn_an_error_level_run_into_a_success() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![(
            "MOCK_BORG_SIMULATE_IGNORED_ERROR".to_owned(),
            "1".to_owned(),
        )],
    );
    let mut target = test_target();
    target.file_change_patterns = vec![FileChangePattern {
        path: "**/*: file changed while we backed it up".to_owned(),
        action: shared::types::FileChangeAction::Ignore,
    }];

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert_eq!(result.warnings.len(), 1);
    assert!(
        result.warnings[0].contains("error-level message"),
        "the run should say an error-level diagnostic was suppressed: {}",
        result.warnings[0]
    );
}

#[tokio::test]
async fn a_suppressed_error_is_reported_even_when_other_warnings_survive() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![
            (
                "MOCK_BORG_SIMULATE_IGNORED_ERROR".to_owned(),
                "1".to_owned(),
            ),
            ("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned()),
        ],
    );
    let mut target = test_target();
    // Matches the ERROR record's path only, leaving the two file-change
    // warnings to survive filtering alongside it.
    target.file_change_patterns = vec![FileChangePattern {
        path: "**/db.sqlite: *".to_owned(),
        action: shared::types::FileChangeAction::Ignore,
    }];

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Warning);
    assert!(
        result.warnings.iter().any(|w| w.contains("error-level")),
        "a suppressed error must not vanish behind surviving warnings: {:?}",
        result.warnings
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("/tmp/test.log: file changed")),
        "the surviving warnings are still reported: {:?}",
        result.warnings
    );
    assert!(
        !result.warnings.iter().any(|w| w.contains("db.sqlite")),
        "the ignored message itself stays suppressed: {:?}",
        result.warnings
    );
}

#[tokio::test]
async fn warnings_suppressed_by_ignore_patterns_leave_a_successful_run() {
    let engine = BackupEngine::with_config(
        mock_borg_path(),
        vec![("MOCK_BORG_SIMULATE_WARNING".to_owned(), "1".to_owned())],
    );
    let mut target = test_target();
    target.file_change_patterns = vec![FileChangePattern {
        path: "**/*: file changed while we backed it up".to_owned(),
        action: shared::types::FileChangeAction::Ignore,
    }];

    let result = engine.run_backup(&target, None, None).await.unwrap();

    assert_eq!(result.status, BackupStatus::Success);
    assert_eq!(result.warnings, [] as [String; 0]);
    assert!(result.error_message.is_none());
}

/// The patterns in `docs/file-change-patterns.md` and in the file change
/// pattern editor's hint have to match the messages they claim to: a
/// leading `*` cannot cross `/`, so a documented pattern that opens with
/// one silently never fires.
#[test]
fn documented_patterns_match_the_messages_they_document() {
    let matches = |pattern: &str, message: &str| {
        let patterns = vec![FileChangePattern {
            path: pattern.to_owned(),
            action: shared::types::FileChangeAction::Ignore,
        }];
        filter_file_change_warnings(vec![message.to_owned()], &patterns)
            .unwrap()
            .is_empty()
    };
    let access_log = "/var/log/nginx/access.log: file changed while we backed it up";
    let nested = "/tmp/logs/nested/deep.log: file changed while we backed it up";

    assert!(matches("/var/log/nginx/access.log*", access_log));
    assert!(matches("/var/log/nginx/**", access_log));
    assert!(matches("**/access.log*", access_log));
    assert!(matches(
        "/etc/config: *",
        "/etc/config: file changed while we backed it up"
    ));
    assert!(matches("/tmp/logs/**", nested));
    assert!(matches(
        "/var/www/cache/**",
        "/var/www/cache/session/abc: file changed while we backed it up"
    ));

    // The forms these replaced, kept as the reason the docs changed.
    assert!(!matches("*access.log*", access_log));
    assert!(!matches("*/tmp/logs*", nested));
}

#[test]
fn test_filter_file_change_warnings_fatal() {
    let patterns = vec![FileChangePattern {
        path: "*test.conf: file changed".to_owned(),
        action: shared::types::FileChangeAction::Fatal,
    }];
    let warnings = vec![
        "test.conf: file changed".to_owned(),
        "other warning".to_owned(),
    ];
    let result = filter_file_change_warnings(warnings, &patterns);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_borg_timeout_is_only_applied_when_configured() {
    let engine = BackupEngine::with_config_and_timeout(
        mock_borg_path(),
        vec![
            ("MOCK_BORG_SLEEP_SUBCOMMAND".to_owned(), "prune".to_owned()),
            ("MOCK_BORG_SLEEP_SECS".to_owned(), "0.1".to_owned()),
        ],
        Some(Duration::from_millis(10)),
    );
    let target = test_target();

    let result = engine.run_backup(&target, None, None).await;
    let err = result.unwrap_err();
    assert!(err.to_string().contains("borg prune"));
    let BackupError::Timeout { seconds, command } = err else {
        panic!("expected timeout error");
    };
    assert_eq!(seconds, 1);
    assert!(command.contains(" borg prune "));
}

#[tokio::test]
async fn write_canary_creates_file_in_first_directory_source() {
    let dir = tempfile::tempdir().unwrap();
    let sources = vec![
        "!/excluded".to_owned(),
        "/definitely-not-a-real-dir-assimilate-test".to_owned(),
        dir.path().to_string_lossy().into_owned(),
    ];

    let canary = BackupEngine::write_canary(&sources).await.unwrap();

    assert_eq!(canary.canary_path, dir.path().join(".assimilate-canary"));
    let content = tokio::fs::read_to_string(&canary.canary_path)
        .await
        .unwrap();
    assert_eq!(content, canary.expected_content);
    assert!(content.contains(&canary.nonce));
}

#[tokio::test]
async fn write_canary_errors_without_usable_directory() {
    let sources = vec![
        "!/excluded".to_owned(),
        "/definitely-not-a-real-dir-assimilate-test".to_owned(),
    ];

    let err = BackupEngine::write_canary(&sources).await.unwrap_err();
    assert!(matches!(err, BackupError::BorgFailed(_)));
}

#[tokio::test]
async fn cleanup_canary_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let canary_path = dir.path().join(".assimilate-canary");
    tokio::fs::write(&canary_path, "content").await.unwrap();
    let token = CanaryToken {
        nonce: "nonce".to_owned(),
        canary_path: canary_path.clone(),
        expected_content: "content".to_owned(),
    };

    BackupEngine::cleanup_canary(&token).await;

    assert!(!canary_path.exists());
}

#[tokio::test]
async fn cleanup_canary_tolerates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let token = CanaryToken {
        nonce: "nonce".to_owned(),
        canary_path: dir.path().join("missing-canary"),
        expected_content: "content".to_owned(),
    };

    BackupEngine::cleanup_canary(&token).await;
}
