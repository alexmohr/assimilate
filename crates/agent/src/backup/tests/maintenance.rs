// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Prune, compact, check, verify and the canary check against the mock borg,
//! one exit status at a time: how each step treats success, borg's warning
//! status and a fatal status is what decides whether a run is reported as
//! failed.

use super::*;

/// An engine on the mock borg with `env` set for every call.
fn engine(env: &[(&str, &str)]) -> BackupEngine {
    BackupEngine::with_config(
        mock_borg_path(),
        env.iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

/// An engine whose borg ends `subcommand` with `code`.
fn ending(subcommand: &str, code: &str) -> BackupEngine {
    engine(&[
        ("MOCK_BORG_EXIT_SUBCOMMAND", subcommand),
        ("MOCK_BORG_EXIT_CODE", code),
    ])
}

/// The message of a [`BackupError::BorgFailed`], failing the test on any
/// other outcome.
fn borg_failure<T: std::fmt::Debug>(result: Result<T, BackupError>) -> String {
    match result {
        Err(BackupError::BorgFailed(message)) => message,
        other => panic!("expected a borg failure, got {other:?}"),
    }
}

#[tokio::test]
async fn a_yearly_retention_is_passed_to_prune() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let log_path = log.path().to_string_lossy().into_owned();
    let mut target = test_target();
    target.keep_yearly = 3;

    engine(&[("MOCK_BORG_LOG", &log_path)])
        .run_borg_prune(&target)
        .await
        .unwrap();

    let calls = std::fs::read_to_string(log.path()).unwrap();
    assert!(calls.contains("--keep-yearly 3"), "{calls}");
}

#[tokio::test]
async fn a_fatal_prune_fails_the_run() {
    let message = borg_failure(ending("prune", "2").run_borg_prune(&test_target()).await);
    assert!(
        message.starts_with("borg prune exited with code 2"),
        "{message}"
    );
}

#[tokio::test]
async fn a_prune_warning_does_not_fail_the_run() {
    ending("prune", "1")
        .run_borg_prune(&test_target())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_fatal_compact_fails_the_run() {
    let message = borg_failure(
        ending("compact", "2")
            .run_borg_compact(&test_target())
            .await,
    );
    assert!(
        message.starts_with("borg compact exited with code 2"),
        "{message}"
    );
}

#[tokio::test]
async fn a_compact_warning_does_not_fail_the_run() {
    ending("compact", "1")
        .run_borg_compact(&test_target())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_clean_check_succeeds() {
    engine(&[]).run_check(&test_target()).await.unwrap();
}

#[tokio::test]
async fn a_check_warning_does_not_fail_the_check() {
    ending("check", "1")
        .run_check(&test_target())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_fatal_check_fails_the_check() {
    let message = borg_failure(ending("check", "2").run_check(&test_target()).await);
    assert!(
        message.starts_with("borg check exited with code 2"),
        "{message}"
    );
}

#[tokio::test]
async fn verify_dry_runs_an_extract_of_the_newest_archive() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let log_path = log.path().to_string_lossy().into_owned();

    let verified = engine(&[("MOCK_BORG_LOG", &log_path)])
        .run_verify(&test_target())
        .await
        .unwrap();

    assert_eq!(verified, 1);
    let calls = std::fs::read_to_string(log.path()).unwrap();
    assert!(
        calls.contains("extract --dry-run --lock-wait 600 -- ::test-host-2024-01-01T00:00:00"),
        "{calls}"
    );
}

#[tokio::test]
async fn verify_fails_when_the_archives_cannot_be_listed() {
    let message = borg_failure(ending("list", "2").run_verify(&test_target()).await);
    assert!(
        message.starts_with("borg list exited with code 2"),
        "{message}"
    );
}

#[tokio::test]
async fn verify_fails_without_an_archive_to_verify() {
    let message = borg_failure(
        engine(&[("MOCK_BORG_LIST_SHORT", "")])
            .run_verify(&test_target())
            .await,
    );
    assert_eq!(message, "no archives found for verification");
}

#[tokio::test]
async fn verify_fails_when_the_dry_run_extract_does() {
    let message = borg_failure(ending("extract", "2").run_verify(&test_target()).await);
    assert!(
        message.starts_with("borg extract --dry-run exited with code 2"),
        "{message}"
    );
}

/// A canary token for a file this test never writes, so only what the mock
/// extracts can satisfy it.
fn canary() -> CanaryToken {
    CanaryToken {
        nonce: "nonce-1".to_owned(),
        canary_path: PathBuf::from("/srv/data/.assimilate-canary"),
        expected_content: "{\"nonce\":\"nonce-1\"}".to_owned(),
    }
}

#[tokio::test]
async fn a_canary_extracted_with_its_content_passes() {
    let result = engine(&[
        ("MOCK_BORG_EXTRACT_CREATE", "1"),
        ("MOCK_BORG_EXTRACT_CONTENT", "{\"nonce\":\"nonce-1\"}"),
    ])
    .verify_canary(&test_target(), &canary(), "test-host-2024-01-01")
    .await;

    assert!(result.success, "{:?}", result.error_message);
    assert_eq!(result.archive_name, "test-host-2024-01-01");
}

#[tokio::test]
async fn a_canary_the_archive_does_not_hold_fails_the_check() {
    let result = engine(&[])
        .verify_canary(&test_target(), &canary(), "test-host-2024-01-01")
        .await;

    assert!(!result.success);
    let message = result.error_message.unwrap();
    assert!(
        message.contains("failed to read extracted canary"),
        "{message}"
    );
}

#[tokio::test]
async fn a_canary_with_other_content_is_a_mismatch() {
    let result = engine(&[("MOCK_BORG_EXTRACT_CREATE", "1")])
        .verify_canary(&test_target(), &canary(), "test-host-2024-01-01")
        .await;

    assert!(!result.success);
    let message = result.error_message.unwrap();
    assert!(message.contains("canary mismatch"), "{message}");
}

#[tokio::test]
async fn a_failed_canary_extract_fails_the_check() {
    let result = ending("extract", "2")
        .verify_canary(&test_target(), &canary(), "test-host-2024-01-01")
        .await;

    assert!(!result.success);
    let message = result.error_message.unwrap();
    assert!(
        message.contains("borg extract canary exited with code 2"),
        "{message}"
    );
}
