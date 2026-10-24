// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;
use crate::api::repos::test_helpers::{
    LOCKED, REPO_URL, closed_pool, install_fake_script, locked_once_then, missing_borg,
};

#[test]
fn is_lock_error_detects_lock_create_message() {
    assert!(is_lock_error(
        "Failed to create/acquire the lock /repo/lock.exclusive"
    ));
}

#[test]
fn is_lock_error_detects_lock_exclusive() {
    assert!(is_lock_error("waiting for lock.exclusive to be released"));
}

#[test]
fn is_lock_error_detects_lockroster() {
    assert!(is_lock_error("LockRoster: another process holds the lock"));
}

#[test]
fn is_lock_error_is_case_insensitive() {
    assert!(is_lock_error("FAILED TO CREATE/ACQUIRE THE LOCK"));
    assert!(is_lock_error("LOCK.EXCLUSIVE"));
    assert!(is_lock_error("LOCKROSTER"));
}

#[test]
fn is_lock_error_returns_false_for_unrelated_errors() {
    assert!(!is_lock_error("Repository does not exist"));
    assert!(!is_lock_error("passphrase is incorrect"));
    assert!(!is_lock_error("Connection refused"));
    assert!(!is_lock_error(""));
}

#[test]
fn is_lock_error_returns_false_for_partial_matches() {
    assert!(!is_lock_error("lock timeout after 60 seconds"));
    assert!(!is_lock_error("unlock successful"));
}

#[test]
fn borg_list_args_do_not_request_archive_metadata_format() {
    let args = borg_list_args("ssh://borg@example.test/repo");
    assert_eq!(
        args,
        [
            "list",
            "--json",
            "--lock-wait",
            LOCK_WAIT_SECS,
            "--",
            "ssh://borg@example.test/repo"
        ]
    );
}

#[test]
fn extract_borg_error_returns_first_line_when_no_traceback() {
    let stderr = "Repository does not exist.\nsome extra context\n";
    assert_eq!(extract_borg_error(stderr), "Repository does not exist.");
}

#[test]
fn extract_borg_error_returns_whole_input_when_single_line() {
    assert_eq!(extract_borg_error("boom"), "boom");
}

#[test]
fn extract_borg_error_picks_exception_line_from_traceback() {
    let stderr = "Traceback (most recent call last):\n  File \"borg/archiver.py\", line 1, in \
                  main\n    do_thing()\nValueError: invalid passphrase\nPlatform: Linux x86_64";
    assert_eq!(extract_borg_error(stderr), "ValueError: invalid passphrase");
}

#[test]
fn extract_borg_error_stops_at_platform_marker() {
    let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 2\nRuntimeError: lock \
                  timeout\nPlatform: Linux\nLater: noise";
    assert_eq!(extract_borg_error(stderr), "RuntimeError: lock timeout");
}

#[tokio::test]
async fn clear_stale_cache_lock_if_present_clears_a_detected_cache_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("a".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    // lock.exclusive is a directory - borg's ExclusiveLock.acquire() creates it via
    // os.mkdir(), with a per-holder marker file inside it.
    let lock_exclusive = lock_dir.join("lock.exclusive");
    tokio::fs::create_dir(&lock_exclusive).await.unwrap();
    tokio::fs::write(lock_exclusive.join("host-1234-1"), b"")
        .await
        .unwrap();
    tokio::fs::write(lock_dir.join("lock.roster"), b"{}")
        .await
        .unwrap();

    let script = format!(
        "#!/bin/sh\nset -eu\necho 'Failed to create/acquire the lock {}/lock.exclusive \
         (timeout).' >&2\nexit 1\n",
        lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    let note = clear_stale_cache_lock_if_present(
        "ssh://user@host/repo",
        &HashMap::new(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    assert!(note.is_some_and(|n| n.contains(&lock_dir.display().to_string())));
    assert!(
        !tokio::fs::try_exists(lock_dir.join("lock.exclusive"))
            .await
            .unwrap()
    );
    assert!(
        !tokio::fs::try_exists(lock_dir.join("lock.roster"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn run_borg_info_with_retry_reads_the_encryption_mode() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) =
        install_fake_script("#!/bin/sh\nprintf '{\"encryption\":{\"mode\":\"repokey\"}}'\n").await;

    let info = run_borg_info_with_retry(
        "ssh://user@host/repo",
        &HashMap::new(),
        Duration::from_secs(30),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await
    .unwrap();

    assert_eq!(info.encryption, BorgEncryption::Repokey);
}

#[tokio::test]
async fn run_borg_info_with_retry_reports_a_hung_borg_as_an_internal_error() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nsleep 5\n").await;

    let result = run_borg_info_with_retry(
        "ssh://user@host/repo",
        &HashMap::new(),
        Duration::from_secs(1),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    let err = result
        .err()
        .expect("a hung borg must not be reported as success");
    assert!(
        matches!(err, ApiError::Internal(ref msg) if msg.contains("timed out after 1s")),
        "a timeout is not lock contention, so it fails at once instead of retrying: {err:?}"
    );
}

#[tokio::test]
async fn clear_stale_cache_lock_if_present_ignores_a_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let script = "#!/bin/sh\nset -eu\necho 'Failed to create/acquire the lock \
                  /home/backups/gremlin/lock.exclusive (timeout).' >&2\nexit 1\n";
    let (_borg_dir, _guard) = install_fake_script(script).await;

    let note = clear_stale_cache_lock_if_present(
        "ssh://user@host/repo",
        &HashMap::new(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    assert!(note.is_none());
}

#[tokio::test]
async fn clear_stale_cache_lock_if_present_returns_none_when_info_succeeds() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let script = "#!/bin/sh\nset -eu\necho '{}'\n";
    let (_borg_dir, _guard) = install_fake_script(script).await;

    let note = clear_stale_cache_lock_if_present(
        "ssh://user@host/repo",
        &HashMap::new(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    assert!(note.is_none());
}

#[tokio::test]
async fn run_borg_break_lock_clears_stale_cache_lock_and_breaks_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("b".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    tokio::fs::create_dir(lock_dir.join("lock.exclusive"))
        .await
        .unwrap();

    let script = format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    echo 'Failed to create/acquire the lock \
         {}/lock.exclusive (timeout).' >&2\n    exit 1\n    ;;\n  break-lock)\n    echo \
         'successfully broke lock'\n    ;;\n  *)\n    exit 1\n    ;;\nesac\n",
        lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    let result = run_borg_break_lock(
        1,
        "ssh://user@host/repo",
        "hunter2",
        &shared::task_registry::TaskRegistry::default(),
        &RepoLock::default(),
    )
    .await
    .unwrap();

    assert!(result.contains("cleared stale local cache lock"));
    assert!(result.contains("successfully broke lock"));
    assert!(
        !tokio::fs::try_exists(lock_dir.join("lock.exclusive"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn run_borg_break_lock_skips_cache_lock_clearing_when_repo_lock_is_held() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("f".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    tokio::fs::create_dir(lock_dir.join("lock.exclusive"))
        .await
        .unwrap();

    let script = format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    echo 'Failed to create/acquire the lock \
         {}/lock.exclusive (timeout).' >&2\n    exit 1\n    ;;\n  break-lock)\n    echo \
         'successfully broke lock'\n    ;;\n  *)\n    exit 1\n    ;;\nesac\n",
        lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    // Simulate another operation (a running backup, delete, import, ...) already
    // holding this repo's lock for the whole call, exactly as every genuine
    // cache-touching operation does.
    let repo_lock = RepoLock::default();
    let _held = repo_lock.acquire(1).await;

    let result = run_borg_break_lock(
        1,
        "ssh://user@host/repo",
        "hunter2",
        &shared::task_registry::TaskRegistry::default(),
        &repo_lock,
    )
    .await
    .unwrap();

    // The repository lock still gets broken - unchanged, pre-existing behavior...
    assert!(result.contains("successfully broke lock"));
    // ...but the cache lock is left untouched, since it can't be proven stale while
    // something else might genuinely be using it right now.
    assert!(result.contains("skipped local cache-lock check"));
    assert!(
        tokio::fs::try_exists(lock_dir.join("lock.exclusive"))
            .await
            .unwrap(),
        "a cache lock must never be cleared while repo_lock is held by another operation"
    );
}

#[tokio::test]
async fn clear_stale_cache_lock_if_present_surfaces_a_failed_clear() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("c".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    // `lock.exclusive` should always be a directory; a plain file there is a real,
    // if unusual, on-disk anomaly that makes `remove_dir_all` hit a genuine I/O error
    // (ENOTDIR) instead of "not found" - mirroring e.g. a permission error.
    tokio::fs::write(lock_dir.join("lock.exclusive"), b"not a directory")
        .await
        .unwrap();

    let script = format!(
        "#!/bin/sh\nset -eu\necho 'Failed to create/acquire the lock {}/lock.exclusive \
         (timeout).' >&2\nexit 1\n",
        lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    let note = clear_stale_cache_lock_if_present(
        "ssh://user@host/repo",
        &HashMap::new(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    let note = note.expect("a detected-but-unclearable cache lock must still produce a note");
    assert!(note.contains(&lock_dir.display().to_string()));
    assert!(note.contains("failed to clear"));
}

#[tokio::test]
async fn run_borg_break_lock_recovers_a_cache_lock_masked_by_a_stale_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("d".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    tokio::fs::create_dir(lock_dir.join("lock.exclusive"))
        .await
        .unwrap();

    // Borg acquires the repository lock before the cache lock, so while the repository
    // lock is also stale, the pre-`break-lock` probe's `info` call times out on the
    // repository lock (not the cache lock) and cannot see the cache lock at all. Only
    // once `break-lock` has cleared the repository lock does a follow-up `info` call
    // reach the cache lock and report its path.
    let probed_repo_lock_marker = tempdir.path().join("probed_repo_lock");
    let script = format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    if [ ! -f {marker} ]; then\n      \
         touch {marker}\n      echo 'Failed to create/acquire the lock \
         /home/backups/gremlin/lock.exclusive (timeout).' >&2\n      exit 1\n    else\n      \
         echo 'Failed to create/acquire the lock {lock}/lock.exclusive (timeout).' >&2\n      \
         exit 1\n    fi\n    ;;\n  break-lock)\n    echo 'successfully broke lock'\n    ;;\n  \
         *)\n    exit 1\n    ;;\nesac\n",
        marker = probed_repo_lock_marker.display(),
        lock = lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    let result = run_borg_break_lock(
        1,
        "ssh://user@host/repo",
        "hunter2",
        &shared::task_registry::TaskRegistry::default(),
        &RepoLock::default(),
    )
    .await
    .unwrap();

    assert!(result.contains("cleared stale local cache lock"));
    assert!(result.contains(&lock_dir.display().to_string()));
    assert!(result.contains("successfully broke lock"));
    assert!(
        !tokio::fs::try_exists(lock_dir.join("lock.exclusive"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn run_borg_break_lock_preserves_the_cache_lock_note_when_break_lock_itself_fails() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let tempdir = tempfile::tempdir().unwrap();
    let lock_dir = tempdir.path().join("borg").join("e".repeat(64));
    tokio::fs::create_dir_all(&lock_dir).await.unwrap();
    tokio::fs::create_dir(lock_dir.join("lock.exclusive"))
        .await
        .unwrap();

    // The pre-probe finds and clears a stale cache lock, but the repository lock itself
    // is held by a genuinely active backup, so `break-lock` fails. That filesystem
    // mutation already happened by the time the error is built - the note must not be
    // silently dropped just because the overall operation goes on to fail.
    let script = format!(
        "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    echo 'Failed to create/acquire the lock \
         {}/lock.exclusive (timeout).' >&2\n    exit 1\n    ;;\n  break-lock)\n    echo 'lock \
         held by another process' >&2\n    exit 1\n    ;;\n  *)\n    exit 1\n    ;;\nesac\n",
        lock_dir.display()
    );
    let (_borg_dir, _guard) = install_fake_script(&script).await;

    let err = run_borg_break_lock(
        1,
        "ssh://user@host/repo",
        "hunter2",
        &shared::task_registry::TaskRegistry::default(),
        &RepoLock::default(),
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("cleared stale local cache lock"));
    assert!(message.contains(&lock_dir.display().to_string()));
    assert!(message.contains("borg break-lock failed"));
    assert!(
        !tokio::fs::try_exists(lock_dir.join("lock.exclusive"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn run_borg_info_opens_the_repository_with_the_given_passphrase() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_script(
        "#!/bin/sh\nif [ \"$BORG_PASSPHRASE\" = right ]; then\n  printf \
         '{\"encryption\":{\"mode\":\"repokey\"}}'\nelse\n  echo 'passphrase supplied in \
         BORG_PASSPHRASE is incorrect' >&2\n  exit 52\nfi\n",
    )
    .await;
    let registry = shared::task_registry::TaskRegistry::default();

    let info = run_borg_info(REPO_URL, "right", &registry).await.unwrap();
    assert_eq!(info.encryption, BorgEncryption::Repokey);

    let wrong = run_borg_info(REPO_URL, "wrong", &registry).await.err();
    assert!(
        matches!(wrong, Some(ApiError::BadRequest(ref msg)) if msg.contains("passphrase is incorrect")),
        "{wrong:?}"
    );
}

async fn borg_info_error(script: &str) -> Option<ApiError> {
    let (_borg_dir, _guard) = install_fake_script(script).await;
    run_borg_info_once(
        REPO_URL,
        &HashMap::new(),
        &shared::task_registry::TaskRegistry::default(),
    )
    .await
    .err()
}

#[tokio::test]
async fn run_borg_info_reports_a_path_without_a_repository_as_a_bad_request() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let err =
        borg_info_error("#!/bin/sh\necho 'Repository /repo does not exist.' >&2\nexit 2\n").await;

    assert!(
        matches!(err, Some(ApiError::BadRequest(ref msg)) if msg == "path does not contain a valid borg repository"),
        "{err:?}"
    );
}

#[tokio::test]
async fn run_borg_info_rejects_output_without_an_encryption_mode() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let err = borg_info_error("#!/bin/sh\nprintf '{\"encryption\":{}}'\n").await;

    assert!(
        matches!(err, Some(ApiError::Internal(ref msg)) if msg.contains("missing encryption.mode")),
        "{err:?}"
    );
}

#[tokio::test]
async fn run_borg_info_rejects_an_encryption_mode_it_does_not_know() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let err = borg_info_error("#!/bin/sh\nprintf '{\"encryption\":{\"mode\":\"rot13\"}}'\n").await;

    assert!(
        matches!(err, Some(ApiError::Internal(ref msg)) if msg.contains("unsupported encryption mode")),
        "{err:?}"
    );
}

#[tokio::test]
async fn run_borg_info_retrying_every_waits_out_a_held_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) =
        install_fake_script(&locked_once_then("{\"encryption\":{\"mode\":\"repokey\"}}")).await;

    let mut result = None;
    let warnings = crate::test_support::warnings_while(async {
        result = Some(
            run_borg_info_retrying_every(
                Duration::from_millis(10),
                REPO_URL,
                &HashMap::new(),
                Duration::from_secs(30),
                &shared::task_registry::TaskRegistry::default(),
            )
            .await,
        );
    })
    .await;

    let info = result
        .expect("ran")
        .expect("the second attempt finds the lock released");
    assert_eq!(info.encryption, BorgEncryption::Repokey);
    assert!(crate::test_support::logged(
        &warnings,
        "borg info lock contention"
    ));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn the_borg_list_stage_timeout_ignores_an_unusable_setting(pool: PgPool) {
    db::set_setting(&pool, "borg_list_stage_timeout_secs", "45")
        .await
        .unwrap();
    assert_eq!(
        get_borg_list_stage_timeout(&pool).await,
        Duration::from_secs(45)
    );

    // CI leaves ASSIMILATE_BORG_LIST_STAGE_TIMEOUT_SECS unset, so a zero
    // setting falls through to the default.
    db::set_setting(&pool, "borg_list_stage_timeout_secs", "0")
        .await
        .unwrap();
    assert_eq!(
        get_borg_list_stage_timeout(&pool).await,
        Duration::from_secs(DEFAULT_BORG_LIST_STAGE_TIMEOUT_SECS)
    );
}

async fn borg_init(script: &str) -> Result<String, ApiError> {
    let (_borg_dir, _guard) = install_fake_script(script).await;
    run_borg_init(
        REPO_URL,
        &HashMap::new(),
        BorgEncryption::Repokey,
        &shared::task_registry::TaskRegistry::default(),
    )
    .await
}

#[tokio::test]
async fn run_borg_init_returns_what_borg_printed() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let both = borg_init("#!/bin/sh\nprintf '%s' \"$*\"\necho 'keep your key safe' >&2\n").await;
    assert_eq!(
        both.unwrap(),
        format!(
            "init --encryption {} {REPO_URL}\nkeep your key safe",
            BorgEncryption::Repokey.as_borg_arg()
        )
    );

    let stderr_only = borg_init("#!/bin/sh\necho 'keep your key safe' >&2\n").await;
    assert_eq!(stderr_only.unwrap(), "keep your key safe");
}

#[tokio::test]
async fn run_borg_init_reports_an_existing_repository_as_a_conflict() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let result =
        borg_init("#!/bin/sh\necho 'A repository already exists at /repo.' >&2\nexit 2\n").await;

    assert!(
        matches!(result, Err(ApiError::Conflict(ref msg)) if msg == "repository already exists at this path"),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_borg_init_reports_any_other_failure_as_a_bad_gateway() {
    let _gate = crate::borg::acquire_test_binary_gate().await;

    let result = borg_init("#!/bin/sh\necho 'Remote: connection closed' >&2\nexit 2\n").await;

    assert!(
        matches!(result, Err(ApiError::BadGateway(ref msg)) if msg == "borg init failed (exit 2): Remote: connection closed"),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_borg_init_reports_a_borg_that_cannot_start() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let _guard = missing_borg();

    let result = run_borg_init(
        REPO_URL,
        &HashMap::new(),
        BorgEncryption::Repokey,
        &shared::task_registry::TaskRegistry::default(),
    )
    .await;

    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.starts_with("failed to execute borg")),
        "{result:?}"
    );
}

async fn list_with_retry(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    stage_timeout: Duration,
    retry_interval: Duration,
) -> Result<Vec<serde_json::Value>, ApiError> {
    run_borg_list_with_retry(BorgListRetryArgs {
        borg_repo: REPO_URL,
        env: &HashMap::new(),
        timeout: Duration::from_secs(30),
        stage_timeout,
        retry_interval,
        pool,
        ui_broadcast,
        repo_id: 1,
        task_registry: &shared::task_registry::TaskRegistry::default(),
    })
    .await
}

#[tokio::test]
async fn run_borg_list_with_retry_gives_up_once_the_stage_has_no_time_left() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nprintf '{\"archives\":[]}'\n").await;

    let result = list_with_retry(
        &closed_pool().await,
        &UiBroadcast::new(),
        Duration::ZERO,
        Duration::from_millis(10),
    )
    .await;

    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.starts_with("borg list stage timed out after 0s")),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_borg_list_with_retry_does_not_wait_for_a_lock_past_the_stage_deadline() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_script(LOCKED).await;

    let result = list_with_retry(
        &closed_pool().await,
        &UiBroadcast::new(),
        Duration::from_secs(30),
        Duration::from_hours(1),
    )
    .await;

    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.starts_with("borg list stage timed out after 30s")),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_borg_list_with_retry_waits_out_a_held_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) =
        install_fake_script(&locked_once_then("{\"archives\":[{\"name\":\"a1\"}]}")).await;
    let ui_broadcast = UiBroadcast::new();
    let mut events = ui_broadcast.subscribe();
    let pool = closed_pool().await;

    let mut result = None;
    let warnings = crate::test_support::warnings_while(async {
        result = Some(
            list_with_retry(
                &pool,
                &ui_broadcast,
                Duration::from_secs(30),
                Duration::from_millis(10),
            )
            .await,
        );
    })
    .await;

    let archives = result.expect("ran").expect("the second listing succeeds");
    assert_eq!(archives, vec![serde_json::json!({"name": "a1"})]);
    assert!(crate::test_support::logged(
        &warnings,
        "borg list lock contention"
    ));
    let event = events.try_recv().expect("the wait is shown");
    assert!(
        matches!(event, shared::protocol::ServerToUi::ImportProgress { message: Some(ref msg), .. } if msg == "Waiting for lock\u{2026}"),
        "{event:?}"
    );
}
