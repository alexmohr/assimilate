// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Server-side maintenance of append-only repositories.
//!
//! The agents' key cannot delete from an append-only repository, so the server
//! prunes and compacts it itself, on the repository's maintenance schedule,
//! with the maintenance key.
//!
//! Borg makes the deletions an append-only connection only pretends to do
//! permanent the first time something writes with full access. So a run looks
//! before it writes: if archives are gone that Assimilate did not delete, it
//! stops without touching the repository, which leaves the host's transaction
//! log usable to roll the repository back.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use shared::{append_only::MaintenanceOutcome, protocol::RepoOpKind};

use crate::{
    AppState,
    api::archives::{classify_borg_error, get_repo_env_privileged},
    borg::Borg,
    db::{self, append_only::RetentionRow},
    error::ApiError,
};

/// How long borg waits for the repository lock before giving up.
const LOCK_WAIT_SECS: &str = "600";

/// What a finished run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// How the run ended.
    pub outcome: MaintenanceOutcome,
    /// Archives the run removed.
    pub archives_removed: i64,
    /// Repository space the run freed.
    pub bytes_freed: i64,
    /// What happened, when the outcome is not `ok`.
    pub detail: Option<String>,
}

impl Finished {
    fn of(outcome: MaintenanceOutcome, detail: impl Into<String>) -> Self {
        Self {
            outcome,
            archives_removed: 0,
            bytes_freed: 0,
            detail: Some(detail.into()),
        }
    }
}

/// The archive names in `borg list --json` output.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if the output is not what borg prints.
pub fn parse_archive_names(json: &[u8]) -> Result<HashSet<String>, ApiError> {
    let value: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| ApiError::Internal(format!("borg list printed unreadable JSON: {e}")))?;
    let archives = value
        .get("archives")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ApiError::Internal("borg list printed no archive list".to_owned()))?;
    Ok(archives
        .iter()
        .filter_map(|archive| archive.get("name").and_then(serde_json::Value::as_str))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

/// The archives Assimilate has records of that borg no longer lists.
#[must_use]
pub fn vanished_names<S: std::hash::BuildHasher>(
    known: &HashSet<String, S>,
    listed: &HashSet<String, S>,
) -> Vec<String> {
    let mut gone: Vec<String> = known.difference(listed).cloned().collect();
    gone.sort();
    gone
}

/// The archives a schedule wrote under an agent's hostname, which only its
/// own retention may prune: they are named `<host>-s<schedule id>-<time>`.
#[must_use]
pub fn schedule_glob(hostname: &str, schedule_id: i64) -> String {
    format!("*{hostname}-s{schedule_id}-*")
}

/// The archives named before they carried a schedule id
/// (`<host>-<timestamp>`). Spelling out the date keeps the pattern from
/// matching the newer names.
#[must_use]
pub fn legacy_glob(hostname: &str) -> String {
    format!("*{hostname}-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*")
}

/// The most each age bucket keeps over `rows`, so archives whose schedule is
/// unknown are never kept for less than any schedule would keep them.
#[must_use]
pub fn legacy_retention<'a>(
    rows: impl IntoIterator<Item = &'a RetentionRow>,
) -> Option<RetentionRow> {
    rows.into_iter().cloned().reduce(|a, b| RetentionRow {
        schedule_id: a.schedule_id,
        hostname: a.hostname,
        keep_hourly: a.keep_hourly.max(b.keep_hourly),
        keep_daily: a.keep_daily.max(b.keep_daily),
        keep_weekly: a.keep_weekly.max(b.keep_weekly),
        keep_monthly: a.keep_monthly.max(b.keep_monthly),
        keep_yearly: a.keep_yearly.max(b.keep_yearly),
    })
}

/// The `borg prune` arguments for the archives matching `glob`, or `None`
/// when `retention` keeps nothing - which would prune every one of them, so
/// the agent skips it too.
#[must_use]
pub fn prune_args(retention: &RetentionRow, glob: &str, borg_repo: &str) -> Option<Vec<String>> {
    let keeps = [
        ("--keep-hourly", retention.keep_hourly),
        ("--keep-daily", retention.keep_daily),
        ("--keep-weekly", retention.keep_weekly),
        ("--keep-monthly", retention.keep_monthly),
        ("--keep-yearly", retention.keep_yearly),
    ];
    if keeps.iter().all(|(_, count)| *count <= 0) {
        return None;
    }
    let mut args = vec![
        "prune".to_owned(),
        "--lock-wait".to_owned(),
        LOCK_WAIT_SECS.to_owned(),
        "-a".to_owned(),
        glob.to_owned(),
    ];
    args.extend(
        keeps
            .iter()
            .filter(|(_, count)| *count > 0)
            .flat_map(|(flag, count)| [(*flag).to_owned(), count.to_string()]),
    );
    args.push(borg_repo.to_owned());
    Some(args)
}

/// `amount` ("1.58") times `multiplier`, in whole units, without going through
/// floating point.
fn scaled(amount: &str, multiplier: u128) -> Option<u128> {
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: u128 = whole.parse().ok()?;
    let digits = u32::try_from(fraction.len()).ok()?;
    let fraction: u128 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    whole.checked_mul(multiplier)?.checked_add(
        fraction
            .checked_mul(multiplier)?
            .checked_div(10u128.checked_pow(digits)?)?,
    )
}

/// The unit of a size borg prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeUnit {
    Byte,
    Kilo,
    Mega,
    Giga,
    Tera,
}

impl SizeUnit {
    const fn bytes(self) -> u128 {
        match self {
            Self::Byte => 1,
            Self::Kilo => 1_000,
            Self::Mega => 1_000_000,
            Self::Giga => 1_000_000_000,
            Self::Tera => 1_000_000_000_000,
        }
    }
}

impl std::str::FromStr for SizeUnit {
    type Err = ();

    fn from_str(unit: &str) -> Result<Self, Self::Err> {
        match unit {
            "B" => Ok(Self::Byte),
            "kB" => Ok(Self::Kilo),
            "MB" => Ok(Self::Mega),
            "GB" => Ok(Self::Giga),
            "TB" => Ok(Self::Tera),
            _ => Err(()),
        }
    }
}

/// The space `borg compact` says it freed, from its `--verbose` output
/// ("compaction freed about 1.58 GB repository space."). Zero when it says
/// nothing.
#[must_use]
pub fn parse_compact_freed(output: &str) -> i64 {
    output
        .lines()
        .find_map(|line| {
            let rest = line.split("compaction freed about ").nth(1)?;
            let mut parts = rest.split_whitespace();
            let amount = parts.next()?;
            let unit: SizeUnit = parts.next()?.parse().ok()?;
            scaled(amount, unit.bytes())
        })
        .and_then(|bytes| i64::try_from(bytes).ok())
        .unwrap_or(0)
}

fn borg_failure(what: &str, output: &std::process::Output) -> ApiError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or(-1);
    match classify_borg_error(code, &stderr) {
        ApiError::Internal(msg) => ApiError::Internal(format!("{what}: {msg}")),
        other => other,
    }
}

struct Repo<'a> {
    state: &'a AppState,
    url: String,
    env: HashMap<String, String>,
}

impl Repo<'_> {
    async fn run(&self, args: &[String]) -> Result<std::process::Output, ApiError> {
        Borg::new()
            .with_registry(self.state.task_registry.clone())
            .run(args, &self.env)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to run borg: {e}")))
    }

    /// Runs borg and returns it unless it failed. Exit code 1 is a warning.
    async fn run_ok(&self, what: &str, args: &[String]) -> Result<std::process::Output, ApiError> {
        let output = self.run(args).await?;
        match output.status.code() {
            Some(0 | 1) => Ok(output),
            _ => Err(borg_failure(what, &output)),
        }
    }

    async fn list(&self) -> Result<HashSet<String>, ApiError> {
        let args = [
            "list".to_owned(),
            "--json".to_owned(),
            "--lock-wait".to_owned(),
            LOCK_WAIT_SECS.to_owned(),
            self.url.clone(),
        ];
        let output = self.run_ok("borg list failed", &args).await?;
        parse_archive_names(&output.stdout)
    }
}

/// Runs maintenance on a repository: the checks, the queued deletions, the
/// schedules' retention and a compact. Holds the repository's lock throughout.
///
/// The outcome is recorded in the repository's run history and moves its next
/// run along; nothing here returns an error, because there is nobody to
/// return one to.
pub async fn run(state: &AppState, repo_id: i64, actor: &str) {
    let _repo_guard = state.repo_lock.acquire(repo_id).await;
    state
        .repo_op_tracker
        .set(repo_id, RepoOpKind::Maintenance, actor.to_owned(), None)
        .await;
    broadcast_op(state, repo_id).await;

    let started_at = Utc::now();
    let run_id = match db::append_only::start_run(&state.pool, repo_id).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(repo_id, error = %e, "failed to record the start of maintenance");
            None
        }
    };

    let finished = match execute(state, repo_id).await {
        Ok(finished) => finished,
        Err(e) => {
            tracing::error!(repo_id, error = %e, "repository maintenance failed");
            Finished::of(MaintenanceOutcome::Failed, e.to_string())
        }
    };
    tracing::info!(repo_id, outcome = %finished.outcome, "repository maintenance finished");

    record(state, repo_id, run_id, started_at, &finished).await;

    state.repo_op_tracker.clear(repo_id).await;
    broadcast_op(state, repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::DataChanged);
}

async fn broadcast_op(state: &AppState, repo_id: i64) {
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });
}

async fn record(
    state: &AppState,
    repo_id: i64,
    run_id: Option<i64>,
    ran_at: DateTime<Utc>,
    finished: &Finished,
) {
    if let Some(run_id) = run_id
        && let Err(e) = db::append_only::finish_run(
            &state.pool,
            run_id,
            finished.outcome,
            finished.archives_removed,
            finished.bytes_freed,
            finished.detail.as_deref(),
        )
        .await
    {
        tracing::warn!(repo_id, error = %e, "failed to record the end of maintenance");
    }

    let stopped = finished.outcome == MaintenanceOutcome::Stopped;
    let next_run_at = if stopped {
        None
    } else {
        next_run(state, repo_id, ran_at).await
    };
    if let Err(e) =
        db::append_only::finish_maintenance(&state.pool, repo_id, ran_at, next_run_at, stopped)
            .await
    {
        tracing::warn!(repo_id, error = %e, "failed to schedule the next maintenance");
    }
}

/// When maintenance should run after `after`, from the repository's cron
/// expression.
pub async fn next_run(
    state: &AppState,
    repo_id: i64,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let settings = db::append_only::get_maintenance(&state.pool, repo_id)
        .await
        .ok()
        .flatten()?;
    next_cron_run_after(state, &settings.cron_expression, after).await
}

/// The next time `cron_expression` comes due, in the server's timezone.
pub async fn next_cron_run(state: &AppState, cron_expression: &str) -> Option<DateTime<Utc>> {
    next_cron_run_after(state, cron_expression, Utc::now()).await
}

async fn next_cron_run_after(
    state: &AppState,
    cron_expression: &str,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let tz = db::get_schedule_timezone(&state.pool)
        .await
        .unwrap_or(chrono_tz::Tz::UTC);
    shared::schedule::calculate_next_run(cron_expression, after, tz)
        .inspect_err(|e| tracing::warn!(error = %e, "maintenance has no next run"))
        .ok()
}

/// What looking at the repository found.
enum Looked {
    /// Archives vanished: the run ends here, having written nothing.
    Stopped(Finished),
    /// Nothing vanished. The archives the repository lists.
    Clear(HashSet<String>),
}

/// Compares the repository's archives with Assimilate's records, and notes
/// those that are gone without being deleted.
async fn look(state: &AppState, repo: &Repo<'_>, repo_id: i64) -> Result<Looked, ApiError> {
    let listed = repo.list().await?;
    let known = db::list_archive_names_for_repo(&state.pool, repo_id).await?;
    if listed.is_empty() && !known.is_empty() {
        return Err(ApiError::Internal(format!(
            "borg list returned 0 archives but {} were previously known; refusing to go on",
            known.len()
        )));
    }
    db::append_only::mark_vanished(&state.pool, repo_id, &vanished_names(&known, &listed)).await?;
    let present: Vec<String> = listed.iter().cloned().collect();
    db::append_only::clear_vanished_present(&state.pool, repo_id, &present).await?;
    let vanished = db::append_only::list_vanished(&state.pool, repo_id).await?;
    if vanished.is_empty() {
        return Ok(Looked::Clear(listed));
    }
    Ok(Looked::Stopped(Finished::of(
        MaintenanceOutcome::Stopped,
        format!(
            "{} archive(s) are gone from the repository but Assimilate did not delete them, so \
             nothing was written: {}",
            vanished.len(),
            vanished.join(", ")
        ),
    )))
}

async fn execute(state: &AppState, repo_id: i64) -> Result<Finished, ApiError> {
    let settings = db::append_only::get_maintenance(&state.pool, repo_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("repo {repo_id} has no maintenance schedule")))?;
    let (borg_repo, env) =
        get_repo_env_privileged(&state.pool, &state.encryption_key, repo_id).await?;
    let repo = Repo {
        state,
        url: borg_repo,
        env,
    };

    // 1. Look before writing: borg makes deletions made over an append-only
    // connection permanent on the first full-access write.
    let listed = match look(state, &repo, repo_id).await? {
        Looked::Stopped(finished) => return Ok(finished),
        Looked::Clear(listed) => listed,
    };

    // 2. Check the repository, so tampering is reported instead of pruned.
    if settings.check_before_prune {
        let args = [
            "check".to_owned(),
            "--repository-only".to_owned(),
            "--lock-wait".to_owned(),
            LOCK_WAIT_SECS.to_owned(),
            repo.url.clone(),
        ];
        let output = repo.run(&args).await?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            return Ok(Finished::of(
                MaintenanceOutcome::CheckFailed,
                format!(
                    "borg check failed, so nothing was pruned: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
    }

    // 3. The deletions users asked for.
    let mut problems = delete_queued(state, &repo, repo_id, &listed).await?;

    // 4. The retention every schedule writing here asks for. A schedule only
    // prunes its own archives, and archives named before schedule ids existed
    // are pruned with the most any of the agent's schedules keeps.
    let retention = db::append_only::list_retention(&state.pool, repo_id).await?;
    let mut prunes: Vec<(String, Vec<String>)> = retention
        .iter()
        .filter_map(|row| {
            let glob = schedule_glob(&row.hostname, row.schedule_id);
            prune_args(row, &glob, &repo.url).map(|args| (row.hostname.clone(), args))
        })
        .collect();
    let hostnames: std::collections::BTreeSet<&str> =
        retention.iter().map(|row| row.hostname.as_str()).collect();
    for hostname in hostnames {
        let union = legacy_retention(retention.iter().filter(|row| row.hostname == hostname));
        if let Some(args) =
            union.and_then(|row| prune_args(&row, &legacy_glob(hostname), &repo.url))
        {
            prunes.push((hostname.to_owned(), args));
        }
    }
    for (hostname, args) in prunes {
        if let Err(e) = repo.run_ok("borg prune failed", &args).await {
            problems.push(format!("{hostname}: {e}"));
        }
    }

    // 5. Reclaim the space.
    let compact = [
        "compact".to_owned(),
        "--verbose".to_owned(),
        "--lock-wait".to_owned(),
        LOCK_WAIT_SECS.to_owned(),
        repo.url.clone(),
    ];
    let bytes_freed = match repo.run_ok("borg compact failed", &compact).await {
        Ok(output) => parse_compact_freed(&String::from_utf8_lossy(&output.stderr)),
        Err(e) => {
            problems.push(e.to_string());
            0
        }
    };

    // 6. Forget the records of what is gone.
    let remaining = repo.list().await?;
    let removed: Vec<String> = listed.difference(&remaining).cloned().collect();
    db::delete_archive_records_by_names(&state.pool, repo_id, &removed).await?;

    if let Err(e) = crate::api::repos::sync_existing_archives(
        &state.pool,
        &state.encryption_key,
        repo_id,
        &state.ui_broadcast,
        &state.background_task_tracker,
        &state.task_registry,
    )
    .await
    {
        tracing::warn!(repo_id, error = %e, "archive list refresh after maintenance failed");
    }
    crate::api::repos::clear_import_progress_state(&state.pool, &state.ui_broadcast, repo_id).await;

    Ok(Finished {
        outcome: if problems.is_empty() {
            MaintenanceOutcome::Ok
        } else {
            MaintenanceOutcome::Failed
        },
        archives_removed: i64::try_from(removed.len()).unwrap_or(i64::MAX),
        bytes_freed,
        detail: (!problems.is_empty()).then(|| problems.join("; ")),
    })
}

/// Deletes the archives users queued. Returns what could not be deleted.
async fn delete_queued(
    state: &AppState,
    repo: &Repo<'_>,
    repo_id: i64,
    listed: &HashSet<String>,
) -> Result<Vec<String>, ApiError> {
    let mut problems = Vec::new();
    for pending in db::append_only::list_pending_deletions(&state.pool, repo_id).await? {
        let name = &pending.archive_name;
        if listed.contains(name) {
            let args = [
                "delete".to_owned(),
                "--lock-wait".to_owned(),
                LOCK_WAIT_SECS.to_owned(),
                "--".to_owned(),
                format!("{}::{name}", repo.url),
            ];
            if let Err(e) = repo.run_ok("borg delete failed", &args).await {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        }
        db::append_only::cancel_deletion(&state.pool, repo_id, name).await?;
    }
    Ok(problems)
}

/// Runs maintenance for every repository whose schedule has come due.
pub async fn run_due(state: &AppState) {
    let due = match db::append_only::list_due_maintenance(&state.pool, Utc::now()).await {
        Ok(due) => due,
        Err(e) => {
            tracing::error!(error = %e, "failed to list repositories due for maintenance");
            return;
        }
    };
    for row in due {
        let repo_id = row.repo_id;
        // Claim the run by moving the next one on, so the next tick does not
        // start a second run while this one is still waiting for the lock.
        let next = next_run(state, repo_id, Utc::now()).await;
        if let Err(e) = db::append_only::set_next_run(&state.pool, repo_id, next).await {
            tracing::warn!(repo_id, error = %e, "failed to claim a maintenance run");
            continue;
        }
        let state = state.clone();
        state
            .background_task_tracker
            .clone()
            .spawn_tracked(async move { run(&state, repo_id, "scheduler").await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retention(hostname: &str, counts: [i32; 5]) -> RetentionRow {
        RetentionRow {
            schedule_id: 3,
            hostname: hostname.to_owned(),
            keep_hourly: counts[0],
            keep_daily: counts[1],
            keep_weekly: counts[2],
            keep_monthly: counts[3],
            keep_yearly: counts[4],
        }
    }

    fn names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn archive_names_are_read_from_borg_list_json() {
        let json =
            br#"{"archives":[{"name":"a-1","id":"x"},{"name":"a-2"},{"name":""}],"repository":{}}"#;
        assert_eq!(parse_archive_names(json).unwrap(), names(&["a-1", "a-2"]));
    }

    #[test]
    fn archive_names_of_an_empty_repository_are_empty() {
        assert!(
            parse_archive_names(br#"{"archives":[]}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn output_that_is_not_a_borg_listing_is_an_error() {
        assert!(parse_archive_names(b"not json").is_err());
        assert!(parse_archive_names(br#"{"repository":{}}"#).is_err());
    }

    #[test]
    fn vanished_archives_are_the_known_ones_borg_no_longer_lists() {
        let gone = vanished_names(&names(&["b", "a", "c"]), &names(&["c", "new"]));
        assert_eq!(gone, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(
            vanished_names(&names(&["a"]), &names(&["a", "b"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn prune_args_name_the_agents_archives_and_each_nonzero_keep() {
        let row = retention("web-1", [0, 7, 0, 3, 0]);
        let args = prune_args(&row, &schedule_glob("web-1", 3), "ssh://b@h:22/srv/r").unwrap();
        assert_eq!(
            args,
            [
                "prune",
                "--lock-wait",
                "600",
                "-a",
                "*web-1-s3-*",
                "--keep-daily",
                "7",
                "--keep-monthly",
                "3",
                "ssh://b@h:22/srv/r"
            ]
        );
    }

    #[test]
    fn a_schedule_prunes_only_the_archives_it_wrote() {
        assert_eq!(schedule_glob("web-1", 12), "*web-1-s12-*");
        // Not another schedule's, even one whose id starts with the same digits.
        assert!(!schedule_glob("web-1", 1).contains("s12"));
    }

    #[test]
    fn archives_without_a_schedule_id_are_matched_by_their_date() {
        assert_eq!(
            legacy_glob("web-1"),
            "*web-1-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*"
        );
    }

    #[test]
    fn legacy_archives_keep_the_most_any_schedule_keeps() {
        let a = retention("h", [0, 7, 0, 3, 0]);
        let b = retention("h", [24, 2, 4, 0, 1]);
        let union = legacy_retention([&a, &b]).unwrap();
        assert_eq!(
            [
                union.keep_hourly,
                union.keep_daily,
                union.keep_weekly,
                union.keep_monthly,
                union.keep_yearly
            ],
            [24, 7, 4, 3, 1]
        );
        assert!(legacy_retention(std::iter::empty::<&RetentionRow>()).is_none());
    }

    #[test]
    fn prune_args_keep_every_period_that_is_set() {
        let args = prune_args(&retention("h", [24, 7, 4, 6, 1]), "*h-s3-*", "r").unwrap();
        for flag in [
            "--keep-hourly",
            "--keep-daily",
            "--keep-weekly",
            "--keep-monthly",
            "--keep-yearly",
        ] {
            assert!(
                args.iter().any(|a| a == flag),
                "{flag} missing from {args:?}"
            );
        }
    }

    #[test]
    fn no_retention_means_no_prune_rather_than_pruning_everything() {
        assert!(prune_args(&retention("h", [0, 0, 0, 0, 0]), "*h-s3-*", "r").is_none());
        assert!(prune_args(&retention("h", [-1, 0, 0, 0, 0]), "*h-s3-*", "r").is_none());
    }

    #[test]
    fn compact_output_says_how_much_it_freed() {
        let out =
            "Starting repository compaction\ncompaction freed about 1.58 GB repository space.\n";
        assert_eq!(parse_compact_freed(out), 1_580_000_000);
        assert_eq!(
            parse_compact_freed("compaction freed about 512 B repository space."),
            512
        );
        assert_eq!(
            parse_compact_freed("compaction freed about 3.5 kB repository space."),
            3500
        );
        assert_eq!(
            parse_compact_freed("compaction freed about 2 TB repository space."),
            2_000_000_000_000
        );
    }

    #[test]
    fn compact_output_without_a_figure_freed_nothing() {
        assert_eq!(parse_compact_freed(""), 0);
        assert_eq!(
            parse_compact_freed("compaction freed about lots repository space."),
            0
        );
        assert_eq!(parse_compact_freed("compaction freed about 3 parsecs"), 0);
    }
}
