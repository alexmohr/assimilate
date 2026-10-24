// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, time::Duration};

use shared::types::BorgEncryption;
use sqlx::PgPool;
use tracing::{error, warn};

use super::sync::publish_import_progress;
use crate::{
    RepoLock,
    api::{archives::LOCK_WAIT_SECS, helpers},
    borg::Borg,
    db,
    error::ApiError,
    ws::ui_broadcast::UiBroadcast,
};

/// Extracts a concise, user-facing error message from borg stderr.
///
/// Borg sometimes outputs a full Python traceback; in that case the actual
/// exception is on a line matching `SomeError: description`. When no traceback
/// is present the first line is usually the useful message.
pub(super) fn extract_borg_error(stderr: &str) -> &str {
    let first = stderr.lines().next().unwrap_or(stderr);
    if !first.starts_with("Traceback") {
        return first;
    }
    stderr
        .lines()
        .take_while(|l| !l.starts_with("Platform:") && !l.starts_with("Borg server: Platform:"))
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with(' ')
                && !l.starts_with("Traceback")
                && !l.starts_with("Borg server:")
        })
        .last()
        .unwrap_or(first)
}

pub(super) struct BorgInfoResult {
    pub(super) encryption: BorgEncryption,
}

pub(super) fn is_lock_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("failed to create/acquire the lock")
        || lower.contains("lock.exclusive")
        || lower.contains("lockroster")
}

pub(super) async fn run_borg_info(
    repo_url: &str,
    passphrase: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    let env = helpers::borg_base_env(passphrase);
    run_borg_info_once(repo_url, &env, task_registry).await
}

pub(super) const LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const LOCK_RETRY_MAX_ATTEMPTS: u32 = 60;
pub(super) const DEFAULT_BORG_LIST_STAGE_TIMEOUT_SECS: u64 = 1800;

/// Upper bound for a single `borg list`/`borg info` invocation. `--lock-wait`
/// only bounds lock contention; a hung SSH connection would otherwise keep the
/// process (and the import) waiting forever, leaving the repo stuck at "Listing
/// archives". On timeout the process is killed and the operation fails so the
/// importing state is cleared.
///
/// Read per call from the `borg_query_timeout_secs` system setting, so a change
/// on the System page applies to the next query. A failed lookup falls back to
/// the default rather than leaving the query unbounded.
pub(super) async fn get_borg_timeout(pool: &PgPool) -> Duration {
    crate::settings::BorgQueryTimeout::load(pool)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, "failed to read the borg query timeout"))
        .unwrap_or_default()
        .duration()
}

/// Checks that the passphrase in `env` opens the repository at `repo_url`,
/// bounded by the configured borg query timeout so an unreachable repository
/// can't hold the request open.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if borg rejects the passphrase or finds no
/// repository there, [`ApiError::Conflict`] if another borg process holds the
/// repository's lock, and [`ApiError::BadGateway`] if borg fails otherwise or
/// times out.
pub(in crate::api) async fn verify_repo_access(
    pool: &PgPool,
    repo_url: &str,
    env: &HashMap<String, String>,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(), ApiError> {
    let timeout = get_borg_timeout(pool).await;
    // A request can't sit out lock contention the way an import does, so a
    // locked repository is reported as such rather than as a borg failure.
    match run_borg_info_within(repo_url, env, timeout, task_registry, ApiError::BadGateway).await {
        Ok(_) => Ok(()),
        Err(ApiError::BadGateway(msg)) if is_lock_error(&msg) => Err(ApiError::Conflict(
            "the repository is locked by another borg process; try again once it finishes, or \
             break the lock if it is stale"
                .to_string(),
        )),
        Err(e) => Err(e),
    }
}

/// Upper bound on the *total* duration of all `borg list` attempts in a single
/// listing stage, including per-attempt timeouts and lock-retry sleeps. Without
/// this, a repository locked by a long-running backup can keep the import stuck
/// at "Listing archives" for up to 90 minutes (60 retries, roughly 90s each).
///
/// Resolution order: DB setting `borg_list_stage_timeout_secs`, then env var
/// `ASSIMILATE_BORG_LIST_STAGE_TIMEOUT_SECS`, then 1800 s default.
pub(super) async fn get_borg_list_stage_timeout(pool: &PgPool) -> Duration {
    if let Ok(Some(v)) = db::get_setting(pool, "borg_list_stage_timeout_secs").await
        && let Ok(secs) = v.parse::<u64>()
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    std::env::var("ASSIMILATE_BORG_LIST_STAGE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or_else(
            || Duration::from_secs(DEFAULT_BORG_LIST_STAGE_TIMEOUT_SECS),
            Duration::from_secs,
        )
}

pub(super) async fn run_borg_info_with_retry(
    repo_url: &str,
    env: &HashMap<String, String>,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    run_borg_info_retrying_every(LOCK_RETRY_INTERVAL, repo_url, env, timeout, task_registry).await
}

/// Runs `borg info`, waiting `retry_interval` and trying again while another
/// borg process holds the repository lock.
pub(super) async fn run_borg_info_retrying_every(
    retry_interval: Duration,
    repo_url: &str,
    env: &HashMap<String, String>,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    let mut attempt = 1;
    loop {
        let result =
            run_borg_info_within(repo_url, env, timeout, task_registry, ApiError::Internal).await;
        let is_lock = matches!(&result, Err(ApiError::BadGateway(msg)) if is_lock_error(msg));
        if !is_lock || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            return result;
        }
        let retry_secs = retry_interval.as_secs();
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            "borg info lock contention, retrying in {retry_secs}s"
        );
        tokio::time::sleep(retry_interval).await;
        attempt = attempt.saturating_add(1);
    }
}

/// [`run_borg_info_once`], killed once `timeout` passes, with the timeout
/// reported through `timed_out` - callers differ in which kind of failure an
/// unreachable repository is to them.
pub(super) async fn run_borg_info_within(
    repo_url: &str,
    env: &HashMap<String, String>,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
    timed_out: fn(String) -> ApiError,
) -> Result<BorgInfoResult, ApiError> {
    tokio::time::timeout(timeout, run_borg_info_once(repo_url, env, task_registry))
        .await
        .unwrap_or_else(|_| {
            Err(timed_out(format!(
                "borg info timed out after {}s; the repository may be unreachable",
                timeout.as_secs()
            )))
        })
}

pub(super) async fn run_borg_info_once(
    repo_url: &str,
    env: &HashMap<String, String>,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgInfoResult, ApiError> {
    let BorgOutput {
        exit_code,
        stdout,
        stderr,
    } = run_borg_command(&["info", "--json", repo_url], env, task_registry).await?;

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg info failed");

        let lower = stderr.to_lowercase();
        if lower.contains("passphrase") || lower.contains("decrypt") {
            return Err(ApiError::BadRequest(
                "passphrase is incorrect for this repository".to_string(),
            ));
        }
        if lower.contains("not a valid repository")
            || lower.contains("does not exist")
            || lower.contains("failed to open repository")
        {
            return Err(ApiError::BadRequest(
                "path does not contain a valid borg repository".to_string(),
            ));
        }

        let summary = extract_borg_error(&stderr);
        return Err(ApiError::BadGateway(format!(
            "borg info failed (exit {exit_code}): {summary}"
        )));
    }

    let json: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| ApiError::Internal(format!("failed to parse borg info JSON: {e}")))?;

    let mode_str = json
        .get("encryption")
        .and_then(|e| e.get("mode"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ApiError::Internal("borg info JSON missing encryption.mode field".to_string())
        })?;

    let encryption = mode_str.parse::<BorgEncryption>().map_err(|e| {
        ApiError::Internal(format!("unsupported encryption mode from borg info: {e}"))
    })?;

    Ok(BorgInfoResult { encryption })
}

/// Upper bound for the lock probe [`clear_stale_cache_lock_if_present`] uses to surface a
/// stale local cache lock's path via borg's own error message. Deliberately short - this call
/// exists only to read the path out of borg's error, not to wait out real contention. `borg
/// break-lock` below still targets the repository lock with the normal [`LOCK_WAIT_SECS`]
/// regardless of what this probe finds.
pub(super) const CACHE_LOCK_PROBE_WAIT_SECS: &str = "1";
/// How long `run_borg_break_lock` waits to acquire `RepoLock` before it will probe for or
/// clear a stale local cache lock. Deliberately short: this only needs to prove no other
/// operation from this server is *currently* using the cache, not wait out a hung one (that's
/// what Break Lock exists to recover from).
pub(super) const CACHE_LOCK_SERIALIZATION_WAIT: Duration = Duration::from_secs(2);

/// Probes the repository for a stale local *cache* lock (as opposed to the repository lock
/// `borg break-lock` already clears) and, if found, attempts to remove it. Returns `None` when
/// no stale cache lock was found; otherwise a human-readable note describing either what was
/// cleared or - if removal itself failed - what was found but left in place, so callers don't
/// mistake a detected-but-unclearable lock for a clean bill of health.
///
/// `borg break-lock` only ever clears the repository-side lock - never borg's own local
/// on-disk cache lock - so a borg process killed (crash, OOM, forced container restart) while
/// holding the cache lock previously left it stuck until an operator connected via SSH and
/// deleted `lock.exclusive`/`lock.roster` by hand. This is invoked from the same admin-gated,
/// explicit "Break Lock" action as the repository-lock break below, and carries the same
/// safety caveat: only use it when certain nothing else is legitimately using the repository.
pub(super) async fn clear_stale_cache_lock_if_present(
    repo_url: &str,
    env: &HashMap<String, String>,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Option<String> {
    let output = Borg::new()
        .with_registry(task_registry.clone())
        .run(
            &[
                "info",
                "--json",
                "--lock-wait",
                CACHE_LOCK_PROBE_WAIT_SECS,
                repo_url,
            ],
            env,
        )
        .await
        .ok()?;

    if output.status.success() {
        return None;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let dir = shared::borg::cache_lock_dir_from_timeout_error(&stderr)?;

    match shared::borg::clear_stale_cache_lock(&dir).await {
        Ok(()) => {
            warn!(dir = %dir.display(), "break-lock: cleared stale local borg cache lock");
            Some(format!(
                "cleared stale local cache lock at {}",
                dir.display()
            ))
        }
        Err(e) => {
            let dir = dir.display();
            warn!(
                %dir,
                error = %e,
                "break-lock: failed to clear stale local borg cache lock"
            );
            Some(format!(
                "detected a stale local cache lock at {dir} but failed to clear it: {e}"
            ))
        }
    }
}

pub(super) async fn run_borg_break_lock(
    repo_id: i64,
    repo_url: &str,
    passphrase: &str,
    task_registry: &shared::task_registry::TaskRegistry,
    repo_lock: &RepoLock,
) -> Result<String, ApiError> {
    let env = helpers::borg_base_env(passphrase);

    // Unlike the repository lock (which the unconditional `break-lock` call below has
    // always broken regardless of whether anything else is using it - a pre-existing,
    // documented risk this feature doesn't change), clearing the *local cache* lock
    // deletes files out from under whatever currently holds them, with no equivalent
    // "borg will just re-acquire it" recovery path. Every other operation that touches
    // this repo's local cache (backup, delete, import, rescan, reset & sync, an ad-hoc
    // admin command via `exec_borg`, encryption migration) serializes through `repo_lock`
    // first, so successfully acquiring it here proves none of them is concurrently using
    // the cache - safe to probe and clear. The wait is short and bounded rather than a
    // blocking `.acquire()`: Break Lock exists specifically to
    // recover from an operation that's hung and will never release this lock on its
    // own, so waiting on it indefinitely here would defeat the action's entire purpose.
    // If the wait times out, skip cache-lock clearing for this call (surfaced as a note
    // below) - the repository-lock break still proceeds exactly as it always has.
    let cache_lock_serialization =
        tokio::time::timeout(CACHE_LOCK_SERIALIZATION_WAIT, repo_lock.acquire(repo_id))
            .await
            .ok();
    let cache_lock_checked = cache_lock_serialization.is_some();
    let skipped_note = (!cache_lock_checked).then(|| {
        "skipped local cache-lock check: another operation currently holds this repository's lock \
         (safe to ignore if it's genuinely running; retry Break Lock once it's confirmed stuck)"
            .to_string()
    });

    let pre_note = if cache_lock_checked {
        clear_stale_cache_lock_if_present(repo_url, &env, task_registry).await
    } else {
        None
    };

    let BorgOutput {
        exit_code,
        stdout,
        stderr,
    } = run_borg_command(
        &["break-lock", "--lock-wait", LOCK_WAIT_SECS, repo_url],
        &env,
        task_registry,
    )
    .await?;

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg break-lock failed");
        let summary = extract_borg_error(&stderr);
        // The cache-lock probe above already ran (and, if it found something, already
        // mutated the filesystem) regardless of whether break-lock itself goes on to
        // succeed or fail here - surface that in the error too, so a failure doesn't
        // read as a clean no-op when a stale cache lock was in fact found or cleared.
        let note_prefix = skipped_note
            .or(pre_note)
            .map_or_else(String::new, |note| format!("{note}\n"));
        return Err(ApiError::BadGateway(format!(
            "{note_prefix}borg break-lock failed (exit {exit_code}): {summary}"
        )));
    }

    // Borg acquires the repository lock before it can even attempt the local cache lock, so
    // if the repository lock was itself stale, the pre-probe above could not see the cache
    // lock at all: its `borg info` call timed out on the repository lock first, and the
    // reported path belongs to the repository lock, not the cache lock. Now that break-lock
    // has cleared the repository lock, probe again so a single "Break Lock" invocation still
    // catches a cache lock left stale by the same crash/OOM/forced-restart. Only worth
    // retrying if the pre-probe actually ran and genuinely found nothing (not if it was
    // skipped above, in which case `repo_lock` is still held by whatever's using it).
    let post_note = if cache_lock_checked && pre_note.is_none() {
        clear_stale_cache_lock_if_present(repo_url, &env, task_registry).await
    } else {
        None
    };

    let combined = combined_output(&stdout, &stderr);

    // skipped_note, pre_note, and post_note are mutually exclusive by construction (at
    // most one of the three is ever `Some`), so picking the first present one is
    // equivalent to collecting and joining them all.
    Ok(match skipped_note.or(pre_note).or(post_note) {
        Some(note) => format!("{note}\n{combined}"),
        None => combined,
    })
}

pub(super) async fn run_borg_init(
    repo_url: &str,
    env: &HashMap<String, String>,
    encryption: BorgEncryption,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<String, ApiError> {
    let BorgOutput {
        exit_code,
        stdout,
        stderr,
    } = run_borg_command(
        &["init", "--encryption", encryption.as_borg_arg(), repo_url],
        env,
        task_registry,
    )
    .await?;

    if exit_code != 0 {
        error!(exit_code, stderr = %stderr, "borg init failed");

        if stderr.contains("repository already exists") {
            return Err(ApiError::Conflict(
                "repository already exists at this path".to_string(),
            ));
        }

        let summary = extract_borg_error(&stderr);
        return Err(ApiError::BadGateway(format!(
            "borg init failed (exit {exit_code}): {summary}"
        )));
    }

    Ok(combined_output(&stdout, &stderr))
}

/// What a finished borg command printed, and how it exited.
struct BorgOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

/// Runs borg with `args` to completion.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if borg cannot be started.
async fn run_borg_command(
    args: &[&str],
    env: &HashMap<String, String>,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<BorgOutput, ApiError> {
    let output = Borg::new()
        .with_registry(task_registry.clone())
        .run(args, env)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to execute borg: {e}")))?;
    Ok(BorgOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// A successful borg command's output to show the user: stdout followed by
/// stderr, where borg prints its notices.
fn combined_output(stdout: &str, stderr: &str) -> String {
    let combined = if stdout.is_empty() {
        stderr.to_owned()
    } else {
        format!("{stdout}\n{stderr}")
    };
    combined.trim().to_string()
}

pub(super) struct BorgListRetryArgs<'a> {
    pub(super) borg_repo: &'a str,
    pub(super) env: &'a std::collections::HashMap<String, String>,
    pub(super) timeout: Duration,
    pub(super) stage_timeout: Duration,
    /// How long to wait before listing again while the repository is locked.
    pub(super) retry_interval: Duration,
    pub(super) pool: &'a PgPool,
    pub(super) ui_broadcast: &'a UiBroadcast,
    pub(super) repo_id: i64,
    pub(super) task_registry: &'a shared::task_registry::TaskRegistry,
}

pub(super) async fn run_borg_list_with_retry(
    args: BorgListRetryArgs<'_>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let BorgListRetryArgs {
        borg_repo,
        env,
        timeout,
        stage_timeout,
        retry_interval,
        pool,
        ui_broadcast,
        repo_id,
        task_registry,
    } = args;
    let borg = Borg::new().with_registry(task_registry.clone());
    // Plain `borg list --json` reads the repository manifest and returns
    // immediately; adding `--format` forces per-archive metadata loads and
    // makes full resyncs crawl on large repositories.
    let args = borg_list_args(borg_repo);
    let stage_deadline = tokio::time::Instant::now()
        .checked_add(stage_timeout)
        .unwrap_or_else(tokio::time::Instant::now);

    let mut attempt = 1;
    loop {
        let remaining = stage_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(ApiError::Internal(format!(
                "borg list stage timed out after {}s; repository may be locked by a long-running \
                 backup",
                stage_timeout.as_secs()
            )));
        }
        let attempt_timeout = timeout.min(remaining);

        let output = match tokio::time::timeout(attempt_timeout, borg.run(&args, env)).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ApiError::Internal(format!(
                    "failed to execute borg list: {e}"
                )));
            }
            Err(_) => {
                return Err(ApiError::Internal(format!(
                    "borg list timed out after {}s; the repository may be unreachable",
                    attempt_timeout.as_secs()
                )));
            }
        };

        if output.status.success() {
            // A successful exit with unparseable output is a hard error: silently
            // treating it as empty would prune every existing archive record.
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
                ApiError::Internal(format!("borg list returned malformed JSON: {e}"))
            })?;
            let archives = json
                .get("archives")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    ApiError::Internal("borg list JSON missing 'archives' array".to_string())
                })?;
            return Ok(archives);
        }

        let stderr_str = String::from_utf8_lossy(&output.stderr);
        if !is_lock_error(&stderr_str) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            return Err(ApiError::Internal(format!(
                "borg list failed: {stderr_str}"
            )));
        }

        let now = tokio::time::Instant::now();
        if now
            .checked_add(retry_interval)
            .is_none_or(|next| next >= stage_deadline)
        {
            return Err(ApiError::Internal(format!(
                "borg list stage timed out after {}s; repository may be locked by a long-running \
                 backup",
                stage_timeout.as_secs()
            )));
        }

        let retry_secs = retry_interval.as_secs();
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            "borg list lock contention, retrying in {retry_secs}s"
        );
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            0,
            0,
            Some("Waiting for lock\u{2026}"),
        )
        .await;
        tokio::time::sleep(retry_interval).await;
        attempt = attempt.saturating_add(1);
    }
}

pub(super) fn borg_list_args(borg_repo: &str) -> [&str; 6] {
    [
        "list",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        borg_repo,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::repos::test_helpers::{
        LOCKED, REPO_URL, closed_pool, install_fake_script, locked_once_then, missing_borg,
    };

    /// A stale local cache lock: a cache directory under `tempdir`, named by
    /// repeating `name`, whose `lock.exclusive` is still there.
    async fn stale_cache_lock(tempdir: &tempfile::TempDir, name: char) -> std::path::PathBuf {
        let lock_dir = tempdir
            .path()
            .join("borg")
            .join(name.to_string().repeat(64));
        tokio::fs::create_dir_all(&lock_dir).await.unwrap();
        tokio::fs::create_dir(lock_dir.join("lock.exclusive"))
            .await
            .unwrap();
        lock_dir
    }

    /// A fake borg that times out on the lock in `lock_dir`.
    fn lock_timeout_borg(lock_dir: &std::path::Path) -> String {
        format!(
            "#!/bin/sh\nset -eu\necho 'Failed to create/acquire the lock {}/lock.exclusive \
             (timeout).' >&2\nexit 1\n",
            lock_dir.display()
        )
    }

    /// A fake borg whose `info` times out on the lock in `lock_dir` and whose
    /// `break-lock` succeeds.
    fn cache_locked_borg(lock_dir: &std::path::Path) -> String {
        format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    echo 'Failed to create/acquire the \
             lock {}/lock.exclusive (timeout).' >&2\n    exit 1\n    ;;\n  break-lock)\n    echo \
             'successfully broke lock'\n    ;;\n  *)\n    exit 1\n    ;;\nesac\n",
            lock_dir.display()
        )
    }

    async fn probe_cache_lock() -> Option<String> {
        clear_stale_cache_lock_if_present(
            "ssh://user@host/repo",
            &HashMap::new(),
            &shared::task_registry::TaskRegistry::default(),
        )
        .await
    }

    async fn break_lock(repo_lock: &RepoLock) -> Result<String, ApiError> {
        run_borg_break_lock(
            1,
            "ssh://user@host/repo",
            "hunter2",
            &shared::task_registry::TaskRegistry::default(),
            repo_lock,
        )
        .await
    }

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
        let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 2\nRuntimeError: \
                      lock timeout\nPlatform: Linux\nLater: noise";
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

        let script = lock_timeout_borg(&lock_dir);
        let (_borg_dir, _guard) = install_fake_script(&script).await;

        let note = probe_cache_lock().await;

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
            install_fake_script("#!/bin/sh\nprintf '{\"encryption\":{\"mode\":\"repokey\"}}'\n")
                .await;

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

        let note = probe_cache_lock().await;

        assert!(note.is_none());
    }

    #[tokio::test]
    async fn clear_stale_cache_lock_if_present_returns_none_when_info_succeeds() {
        let _gate = crate::borg::acquire_test_binary_gate().await;

        let script = "#!/bin/sh\nset -eu\necho '{}'\n";
        let (_borg_dir, _guard) = install_fake_script(script).await;

        let note = probe_cache_lock().await;

        assert!(note.is_none());
    }

    #[tokio::test]
    async fn run_borg_break_lock_clears_stale_cache_lock_and_breaks_repository_lock() {
        let _gate = crate::borg::acquire_test_binary_gate().await;

        let tempdir = tempfile::tempdir().unwrap();
        let lock_dir = stale_cache_lock(&tempdir, 'b').await;

        let script = cache_locked_borg(&lock_dir);
        let (_borg_dir, _guard) = install_fake_script(&script).await;

        let result = break_lock(&RepoLock::default()).await.unwrap();

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
        let lock_dir = stale_cache_lock(&tempdir, 'f').await;

        let script = cache_locked_borg(&lock_dir);
        let (_borg_dir, _guard) = install_fake_script(&script).await;

        // Simulate another operation (a running backup, delete, import, ...) already
        // holding this repo's lock for the whole call, exactly as every genuine
        // cache-touching operation does.
        let repo_lock = RepoLock::default();
        let _held = repo_lock.acquire(1).await;

        let result = break_lock(&repo_lock).await.unwrap();

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

        let script = lock_timeout_borg(&lock_dir);
        let (_borg_dir, _guard) = install_fake_script(&script).await;

        let note = probe_cache_lock().await;

        let note = note.expect("a detected-but-unclearable cache lock must still produce a note");
        assert!(note.contains(&lock_dir.display().to_string()));
        assert!(note.contains("failed to clear"));
    }

    #[tokio::test]
    async fn run_borg_break_lock_recovers_a_cache_lock_masked_by_a_stale_repository_lock() {
        let _gate = crate::borg::acquire_test_binary_gate().await;

        let tempdir = tempfile::tempdir().unwrap();
        let lock_dir = stale_cache_lock(&tempdir, 'd').await;

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

        let result = break_lock(&RepoLock::default()).await.unwrap();

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
        let lock_dir = stale_cache_lock(&tempdir, 'e').await;

        // The pre-probe finds and clears a stale cache lock, but the repository lock itself
        // is held by a genuinely active backup, so `break-lock` fails. That filesystem
        // mutation already happened by the time the error is built - the note must not be
        // silently dropped just because the overall operation goes on to fail.
        let script = format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\n  info)\n    echo 'Failed to create/acquire the \
             lock {}/lock.exclusive (timeout).' >&2\n    exit 1\n    ;;\n  break-lock)\n    echo \
             'lock held by another process' >&2\n    exit 1\n    ;;\n  *)\n    exit 1\n    \
             ;;\nesac\n",
            lock_dir.display()
        );
        let (_borg_dir, _guard) = install_fake_script(&script).await;

        let err = break_lock(&RepoLock::default()).await.unwrap_err();

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
            borg_info_error("#!/bin/sh\necho 'Repository /repo does not exist.' >&2\nexit 2\n")
                .await;

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

        let err =
            borg_info_error("#!/bin/sh\nprintf '{\"encryption\":{\"mode\":\"rot13\"}}'\n").await;

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

        let both =
            borg_init("#!/bin/sh\nprintf '%s' \"$*\"\necho 'keep your key safe' >&2\n").await;
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
            borg_init("#!/bin/sh\necho 'A repository already exists at /repo.' >&2\nexit 2\n")
                .await;

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
        let (_borg_dir, _guard) =
            install_fake_script("#!/bin/sh\nprintf '{\"archives\":[]}'\n").await;

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
}
