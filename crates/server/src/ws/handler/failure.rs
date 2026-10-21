// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Classifying a failed backup run and the catch-up bookkeeping it drives.

use sqlx::PgPool;

use crate::{db, notifications::EventType};

/// Which event a *failed* backup is reported as.
///
/// Only a repository whose host the schedule waits for - marked as not always
/// online, or one the schedule is set to wait for - can turn a failure into
/// anything else. For any other - a server that should always be there - an
/// unreachable host is exactly the failure borg reported, and is left as one
/// without so much as a probe. For one that is waited for, a failure whose
/// host does not answer SSH is a skipped backup rather than a failed one: its
/// absence was expected, and the run is caught up once it is back.
///
/// Decided here rather than alongside the failure, so exactly one event
/// describes the run. Reporting the skip separately would leave every such
/// failure firing twice - once as `BackupFailed` from this same dispatch, and
/// again as the skip - which is two alerts and two activity rows for one
/// backup.
///
/// A host waited for and never woken is asked before the run instead (see
/// [`crate::repo_wait`]), so this mostly catches one that went away during
/// the backup, or one that answered after its wake and then failed.
///
/// Falls back to [`EventType::BackupFailed`] whenever the repository's row
/// can't be read: a repository this server cannot even look up is not one it
/// can call absent, however it is marked.
///
/// Runs on a background task, never on the caller's thread of control: the
/// probe is a live SSH round-trip, and every path here is reached from
/// `handle_agent_message`, which the agent's websocket loop awaits inline.
pub(super) async fn classify_failed_backup(
    pool: &PgPool,
    repo_id: i64,
    repo_name: &str,
    hostname: &str,
    schedule: Option<(i64, &str)>,
    run_id: Option<&str>,
) -> FailedBackupReport {
    let schedule_name = schedule.map(|(_, name)| name);
    let Ok(repo) = db::get_repo_by_id(pool, repo_id).await else {
        return FailedBackupReport::plain_failure();
    };
    // The schedule's own choice for the host's machine counts as much as the
    // machine's switch: a schedule set to wait for it waits for it here too.
    let mode = match schedule {
        Some((schedule_id, _)) => db::machines::list_schedule_machine_wake(pool, schedule_id)
            .await
            .ok()
            .and_then(|choices| {
                choices
                    .into_iter()
                    .find(|c| c.machine_id == repo.machine_id)
                    .map(|c| c.mode)
            }),
        None => None,
    };
    let intermittent = db::machines::get_machine(pool, repo.machine_id)
        .await
        .is_ok_and(|machine| machine.intermittent);
    let Some(wait) = shared::types::MachinePolicy::resolve(
        shared::types::ScheduleWakeOverride::HostDefault,
        mode,
        repo.wake_enabled,
        intermittent,
    )
    .wait
    else {
        return FailedBackupReport::plain_failure();
    };
    if crate::power::repo_reachable(&repo).await {
        return FailedBackupReport::plain_failure();
    }
    let (system_event, event_type) = crate::repo_wait::events_for(wait);

    tracing::warn!(
        hostname = %hostname,
        repo_id,
        "backup failed and the repository's host is not answering SSH; reporting it as skipped"
    );
    // The bare reason is what every channel shows, matching what the
    // agent-offline sibling puts in its own `error_message`; the activity log
    // gets the same reason behind a sentence that names the run.
    let reason = format!("the host for repository '{repo_name}' did not answer SSH");
    let msg = schedule_name.map_or_else(
        || format!("Backup failed: {reason}"),
        |name| format!("Backup for schedule '{name}' failed: {reason}"),
    );
    if let Err(e) = db::insert_system_event(pool, system_event, Some(hostname), &msg).await {
        tracing::error!(
            repo_id,
            error = %e,
            "failed to record backup-skipped-repo-offline system event"
        );
    }
    if let Some((schedule_id, _)) = schedule {
        mark_repo_catch_up_pending(pool, schedule_id, repo_id, repo_name, run_id).await;
    }
    FailedBackupReport {
        event_type,
        reason: Some(reason),
    }
}

/// Records that this repository owes the schedule a run, so
/// [`crate::repo_catch_up`] picks it up once the host answers again.
///
/// This is the only place that knows a run failed *because* the repository was
/// away, which is why the marker is written here rather than in the scheduler:
/// by the time the tick dispatched, the host still looked fine.
///
/// Only reached for a repository marked as not always online - the caller has
/// already decided that. A run with no schedule behind it (Run now) has
/// nothing to catch up to, so it never gets here either.
pub(super) async fn mark_repo_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
    repo_name: &str,
    run_id: Option<&str>,
) {
    // The occurrence this run stood for, not the moment it gave up: the
    // give-up window is the wait an operator asked for, measured from when the
    // backup should have happened. Read off the run's own report rather than
    // the schedule's `last_run_at`, which by the time a slow failure comes back
    // may already name a later run of the same schedule.
    let run_started = match run_id {
        Some(run_id) => db::catch_up::run_started_at(pool, run_id, repo_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let due_at = if let Some(started) = run_started {
        started
    } else {
        // A run with no report of its own falls back to the schedule's last
        // run, and a schedule that has somehow never run has no occurrence to
        // name, so the failure itself becomes one.
        let Ok(schedule) = db::get_schedule_by_id(pool, schedule_id).await else {
            return;
        };
        schedule.last_run_at.unwrap_or_else(chrono::Utc::now)
    };
    if let Err(e) =
        db::catch_up::mark_repo_catch_up_pending(pool, schedule_id, repo_id, due_at, run_id).await
    {
        tracing::error!(
            schedule_id,
            repo_id,
            error = %e,
            "failed to record a pending repository catch-up"
        );
        return;
    }
    tracing::info!(
        schedule_id,
        repo_id,
        missed_occurrence = %due_at,
        "repository '{repo_name}' was away; the run will be caught up when it answers"
    );
}

/// A backup that wrote this schedule's archive into `repo_id` settles whatever
/// that repository was waiting to catch up: the host is plainly back, and the
/// occurrence it missed is superseded by the one that just ran. Left in place,
/// the marker would have the poller run a redundant catch-up days later, the
/// next time it happens to find the host answering.
pub(super) async fn clear_repo_catch_up_on_success(pool: &PgPool, schedule_id: i64, repo_id: i64) {
    match db::catch_up::settle_repo_catch_up(pool, schedule_id, repo_id).await {
        Ok(true) => tracing::info!(
            schedule_id,
            repo_id,
            "a successful backup settled the repository's pending catch-up"
        ),
        Ok(false) => {}
        Err(e) => tracing::error!(
            schedule_id,
            repo_id,
            error = %e,
            "failed to clear a repository catch-up after a successful backup"
        ),
    }
}

/// Ends a target's wait on a dependency once one of its runs has reported -
/// the scheduled run that found the dependency back, or the catch-up itself.
pub(super) async fn settle_dependency_catch_up(pool: &PgPool, schedule_id: i64, agent_id: i64) {
    match db::dependency_catch_ups::clear_dependency_catch_up(pool, schedule_id, agent_id).await {
        Ok(true) => tracing::info!(
            schedule_id,
            agent_id,
            "a reported backup settled the target's pending dependency catch-up"
        ),
        Ok(false) => {}
        Err(e) => tracing::error!(
            schedule_id,
            agent_id,
            error = %e,
            "failed to clear a dependency catch-up after a reported backup"
        ),
    }
}

/// How a failed backup is reported: the event it is raised as, plus the
/// explanation that stands in for borg's own error when the failure turns out
/// to be a host that was not there.
pub(super) struct FailedBackupReport {
    pub(super) event_type: EventType,
    /// `None` leaves the agent's reported error in place, which is the right
    /// thing to show when borg's own message is the explanation. `Some` is the
    /// reason the run is being reported as a skip instead, and replaces it on
    /// every channel - otherwise a "Backup skipped" alert would carry the same
    /// raw connection error a plain failure did, and say nothing the label
    /// didn't already.
    pub(super) reason: Option<String>,
}

impl FailedBackupReport {
    /// A failure borg itself is the best witness to.
    const fn plain_failure() -> Self {
        Self {
            event_type: EventType::BackupFailed,
            reason: None,
        }
    }
}
