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
mod tests;
