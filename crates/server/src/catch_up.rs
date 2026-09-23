// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Catching up a run a host missed while it was unreachable.
//!
//! The scheduler records one pending occurrence per target when it can't reach
//! that target's agent (see `scheduler::mark_catch_up_pending`). A later miss
//! overwrites the marker rather than queueing behind it, so however long a host
//! stays away - thirty-five nightly occurrences or one - exactly one catch-up run
//! follows when it comes back.
//!
//! This module is that "when it comes back": the agent websocket handler calls
//! [`run_catch_ups_on_reconnect`] once the agent has registered, after the
//! re-enable pass, so a schedule auto-disabled while the host was down is enabled
//! again before its pending run is considered.

use chrono::{DateTime, TimeDelta, Utc};
use shared::types::{RepoId, ScheduleType, SystemEventType};
use uuid::Uuid;

use crate::{
    AppState,
    db::{self, catch_up::CatchUpCandidate},
    run_dispatch::{self, RunOrigin, RunRequest},
};

/// Why a pending miss did not turn into a run. Every one of these still clears the
/// marker: the decision is made once, at reconnect, and the next scheduled run
/// supersedes anything skipped here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// The next scheduled run is closer than the schedule's configured floor, so a
    /// catch-up now would collide with the run that is about to happen anyway.
    NextRunTooClose,
    /// The schedule's type column doesn't parse - a catch-up can't guess what the
    /// host was meant to do.
    UnknownScheduleType,
    /// The host came back, but past the point the schedule said to stop waiting
    /// for it. Unlike the others this one is reported: the backup never
    /// happened, and nothing else is going to say so.
    GaveUpWaiting,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NextRunTooClose => write!(f, "the next scheduled run is too close"),
            Self::UnknownScheduleType => write!(f, "the schedule type is not recognised"),
            Self::GaveUpWaiting => write!(f, "the give-up window had already passed"),
        }
    }
}

/// Whether a pending catch-up may still run, given how much time is left before
/// its schedule's next regular run.
///
/// A schedule with no computed next run has nothing for the catch-up to collide
/// with, so the floor doesn't apply to it.
///
/// Shared with [`crate::repo_catch_up`]: the floor is a property of the
/// schedule, so it means the same thing whether the thing that was away was the
/// host running the backup or the host receiving it.
pub(crate) fn has_room_before_next_run(
    next_run_at: Option<DateTime<Utc>>,
    min_lead_minutes: i32,
    now: DateTime<Utc>,
) -> bool {
    let Some(next_run_at) = next_run_at else {
        return true;
    };
    let lead = i64::from(min_lead_minutes);
    next_run_at
        .signed_duration_since(now)
        .num_minutes()
        .ge(&lead)
}

/// Runs whatever `agent_id` missed while it was unreachable, at most one run per
/// schedule it is a target of.
///
/// Every pending marker this agent carries is cleared first, whether or not it
/// leads to a run: a miss that is skipped here (the next run is minutes away, the
/// setting was switched off in the meantime, the schedule is disabled) is dropped
/// rather than carried forward, so it can never surface days later.
pub async fn run_catch_ups_on_reconnect(state: &AppState, agent_id: i64, hostname: &str) {
    let candidates =
        match db::catch_up::list_catch_up_candidates_for_agent(&state.pool, agent_id).await {
            Ok(candidates) => candidates,
            Err(e) => {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to look up pending catch-up runs on agent reconnect"
                );
                return;
            }
        };

    match db::catch_up::clear_catch_up_pending(&state.pool, agent_id).await {
        Ok(_) => {}
        Err(e) => {
            // Without a cleared marker the same miss would be reconsidered on every
            // future reconnect, so don't run anything off a marker that is still set.
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to clear pending catch-up markers on agent reconnect; not running them"
            );
            return;
        }
    }

    let now = Utc::now();
    for candidate in candidates {
        dispatch_catch_up(state, &candidate, now).await;
    }
}

async fn dispatch_catch_up(state: &AppState, candidate: &CatchUpCandidate, now: DateTime<Utc>) {
    // Checked before anything else: a host that reconnects after the window has
    // closed is not caught up, and the run it missed is reported as the failure
    // it became. The poller reports most of these long before the host is back;
    // this covers the one that reconnects between two passes.
    if gave_up_by(candidate, now) {
        skip(candidate, SkipReason::GaveUpWaiting);
        report_abandoned_agent_catch_up(state, candidate, now).await;
        return;
    }
    let Ok(schedule_type) = candidate.schedule_type.parse::<ScheduleType>() else {
        skip(candidate, SkipReason::UnknownScheduleType);
        return;
    };
    if !has_room_before_next_run(candidate.next_run_at, candidate.min_lead_minutes, now) {
        skip(candidate, SkipReason::NextRunTooClose);
        return;
    }

    let targets = vec![db::ScheduleRunTarget {
        agent_id: candidate.agent_id,
        hostname: candidate.hostname.clone(),
    }];

    // Every repository the missed tick would have written, not just the
    // schedule's denormalised primary - otherwise each miss leaves the
    // secondary copies further behind with nothing reported.
    let repo_ids =
        match db::catch_up::list_enabled_catch_up_repos(&state.pool, candidate.schedule_id).await {
            Ok(repo_ids) if !repo_ids.is_empty() => repo_ids,
            Ok(_) => {
                tracing::warn!(
                    hostname = %candidate.hostname,
                    schedule_id = candidate.schedule_id,
                    "catch-up run: no enabled target repository left, skipping"
                );
                return;
            }
            Err(e) => {
                tracing::error!(
                    hostname = %candidate.hostname,
                    schedule_id = candidate.schedule_id,
                    error = %e,
                    "catch-up run: failed to resolve target repositories"
                );
                return;
            }
        };

    tracing::info!(
        hostname = %candidate.hostname,
        schedule_id = candidate.schedule_id,
        missed_occurrence = %candidate.pending_for,
        "catch-up run: host reconnected, running the occurrence it missed"
    );
    record_catch_up_event(state, candidate).await;

    spawn_catch_up_run(
        state,
        CatchUpRun {
            schedule_id: candidate.schedule_id,
            schedule_type,
            cron_expression: candidate.cron_expression.clone(),
            targets,
            repo_ids,
            now,
        },
    )
    .await;
}

/// One catch-up run, whichever half of the feature asked for it.
///
/// The two halves differ only in what they fill in here: a reconnecting agent
/// is one target writing every repository the missed tick would have, and a
/// returning repository is every target of the schedule writing that one
/// repository. Everything downstream - the pending records, the run id, the
/// failure-streak reset - is the same work, and was worth having in one place
/// rather than two that drift.
pub(crate) struct CatchUpRun {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// What the run does, already parsed.
    pub schedule_type: ScheduleType,
    /// Needed to advance `next_run_at` past this run.
    pub cron_expression: String,
    /// Agents to run it on, in order.
    pub targets: Vec<db::ScheduleRunTarget>,
    /// Repositories to write, in order.
    pub repo_ids: Vec<i64>,
    /// The moment the run was decided on.
    pub now: DateTime<Utc>,
}

/// Records a pending row per (target, repository) and dispatches the run in the
/// background, resetting the schedule's connectivity failure streak if it got
/// anywhere.
pub(crate) async fn spawn_catch_up_run(state: &AppState, run: CatchUpRun) {
    let run_id = Uuid::new_v4().to_string();
    if matches!(run.schedule_type, ScheduleType::Backup) {
        for target in &run.targets {
            for repo_id in &run.repo_ids {
                if let Err(e) = db::insert_backup_pending(
                    &state.pool,
                    target.agent_id,
                    *repo_id,
                    Some(run.schedule_id),
                    &run_id,
                    run.now,
                )
                .await
                {
                    tracing::warn!(
                        hostname = %target.hostname,
                        schedule_id = run.schedule_id,
                        repo_id = *repo_id,
                        error = %e,
                        "catch-up run: failed to insert pending record"
                    );
                }
            }
        }
    }

    let request = RunRequest {
        repo_ids: run.repo_ids.into_iter().map(RepoId).collect(),
        schedule_type: run.schedule_type,
        schedule_id: run.schedule_id,
        cron_expression: run.cron_expression,
        now: run.now,
        run_id,
        origin: RunOrigin::CatchUp,
    };
    let state = state.clone();
    let schedule_id = run.schedule_id;
    let targets = run.targets;
    tokio::spawn(async move {
        if run_dispatch::run_targets_sequential(state.clone(), targets, request).await > 0 {
            reset_failures_if_every_target_is_back(&state, schedule_id).await;
        }
    });
}

/// Clears the schedule's connectivity failure streak once a catch-up has actually
/// reached its agent, on the same terms a regular tick does: only a schedule with
/// no *other* unreachable target counts as healthy again, so a multi-target
/// schedule whose second host is still down keeps counting toward its
/// auto-disable threshold instead of being propped up by this one host's return.
async fn reset_failures_if_every_target_is_back(state: &AppState, schedule_id: i64) {
    let targets_by_schedule =
        match db::get_schedule_target_agent_ids_by_schedule(&state.pool, &[schedule_id]).await {
            Ok(targets) => targets,
            Err(e) => {
                tracing::error!(
                    schedule_id,
                    error = %e,
                    "catch-up run: failed to look up targets to reset the failure count"
                );
                return;
            }
        };
    let targets = targets_by_schedule
        .get(&schedule_id)
        .map_or(&[][..], |ids| ids.as_slice());
    for agent_id in targets {
        if !state.registry.is_connected(*agent_id).await {
            return;
        }
    }
    if let Err(e) = db::reset_schedule_consecutive_failures(&state.pool, schedule_id).await {
        tracing::error!(
            schedule_id,
            error = %e,
            "catch-up run: failed to reset the schedule failure count"
        );
    }
}

/// When a wait that started at `pending_for` runs out, or `None` when the host
/// is waited for indefinitely.
///
/// Shared by both halves and by the Power pane's countdown, so all three agree
/// to the minute. A window that runs off the end of the calendar is one nobody
/// outlives, so it reads as the "indefinitely" it effectively is.
pub(crate) fn give_up_deadline(
    pending_for: DateTime<Utc>,
    give_up_minutes: i32,
) -> Option<DateTime<Utc>> {
    (give_up_minutes > 0)
        .then(|| pending_for.checked_add_signed(TimeDelta::minutes(i64::from(give_up_minutes))))
        .flatten()
}

/// Whether this wait is already past the window its agent set.
fn gave_up_by(candidate: &CatchUpCandidate, now: DateTime<Utc>) -> bool {
    give_up_deadline(candidate.pending_for, candidate.give_up_minutes)
        .is_some_and(|deadline| now >= deadline)
}

/// Ends every agent wait whose window has closed, whether or not that host has
/// any intention of coming back.
///
/// Without this the window would only ever be noticed at reconnect, so a host
/// that never returns would leave its schedule reporting nothing at all - which
/// is the state the window exists to end.
pub async fn expire_agent_catch_ups(state: &AppState) {
    let now = Utc::now();
    let expired = match db::catch_up::list_expired_agent_catch_ups(&state.pool, now).await {
        Ok(expired) => expired,
        Err(e) => {
            tracing::error!(error = %e, "failed to look up expired catch-up waits");
            return;
        }
    };
    for candidate in expired {
        if let Err(e) = db::catch_up::clear_catch_up_pending_for_target(
            &state.pool,
            candidate.schedule_id,
            candidate.agent_id,
        )
        .await
        {
            // Still set means it comes back next pass; reporting now would
            // report it again then.
            tracing::error!(
                schedule_id = candidate.schedule_id,
                error = %e,
                "failed to clear an expired catch-up marker; leaving it for the next pass"
            );
            continue;
        }
        report_abandoned_agent_catch_up(state, &candidate, now).await;
    }
}

/// Reports a host wait that ran out, as one failed backup per repository the
/// run would have written.
async fn report_abandoned_agent_catch_up(
    state: &AppState,
    candidate: &CatchUpCandidate,
    now: DateTime<Utc>,
) {
    tracing::warn!(
        hostname = %candidate.hostname,
        schedule_id = candidate.schedule_id,
        missed_occurrence = %candidate.pending_for,
        waited_minutes = candidate.give_up_minutes,
        "catch-up run: abandoned, the host did not come back in time"
    );
    let repo_ids = db::catch_up::list_enabled_catch_up_repos(&state.pool, candidate.schedule_id)
        .await
        .unwrap_or_default();
    let mut pairs = Vec::with_capacity(repo_ids.len());
    for repo_id in repo_ids {
        pairs.push(AbandonedPair {
            agent_id: candidate.agent_id,
            hostname: candidate.hostname.clone(),
            repo_id,
            repo_name: db::get_repo_name(&state.pool, repo_id)
                .await
                .unwrap_or_default(),
        });
    }
    report_abandoned_catch_up(
        state,
        &AbandonedCatchUp {
            schedule_id: candidate.schedule_id,
            schedule_name: &candidate.schedule_name,
            pending_for: candidate.pending_for,
            give_up_minutes: candidate.give_up_minutes,
            absent: format!("host '{}'", candidate.hostname),
            event_host: &candidate.hostname,
            next_run_at: candidate.next_run_at,
            pairs,
            now,
        },
    )
    .await;
}

/// One (host, repository) a run would have covered. An abandoned catch-up sends
/// one alert per pair rather than one for the schedule, so a notification rule
/// scoped to either still matches - the same shape an ordinary backup failure
/// would have had.
pub(crate) struct AbandonedPair {
    /// Host the backup would have run on.
    pub agent_id: i64,
    /// That host's name.
    pub hostname: String,
    /// Repository it would have written.
    pub repo_id: i64,
    /// That repository's name.
    pub repo_name: String,
}

/// A catch-up that waited out its schedule's give-up window, and everything
/// needed to say so.
pub(crate) struct AbandonedCatchUp<'a> {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// Its display name.
    pub schedule_name: &'a str,
    /// The occurrence that was missed.
    pub pending_for: DateTime<Utc>,
    /// The window that ran out, for the message.
    pub give_up_minutes: i32,
    /// What never came back, phrased for a sentence: `host 'lab-ws-02'` or
    /// `repository 'borg-nas'`.
    pub absent: String,
    /// Host the activity-log entry is filed under.
    pub event_host: &'a str,
    /// When the schedule next runs, carried into the alert.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Every (host, repository) the run would have covered.
    pub pairs: Vec<AbandonedPair>,
    /// When the wait was ended.
    pub now: DateTime<Utc>,
}

/// Records an abandoned catch-up and alerts on it.
///
/// The alert is a plain `backup_failed`, not an event type of its own: a rule
/// that already alerts on failed backups then covers this without anyone having
/// to add a rule for an event they have never seen. Because it arrives long
/// after the run it is about, the message says how long was waited.
pub(crate) async fn report_abandoned_catch_up(state: &AppState, run: &AbandonedCatchUp<'_>) {
    let reason = format!(
        "{} did not come back within {}",
        run.absent,
        humanize_minutes(run.give_up_minutes)
    );
    record_system_event(
        state,
        SystemEventType::ScheduleCatchUpAbandoned,
        run.event_host,
        &format!(
            "Backup for schedule '{}' missed at {} was abandoned: {reason}",
            run.schedule_name, run.pending_for
        ),
    )
    .await;

    for pair in &run.pairs {
        let event = crate::notifications::NotificationEvent {
            event_type: crate::notifications::EventType::BackupFailed,
            hostname: pair.hostname.clone(),
            repo_name: pair.repo_name.clone(),
            status: "failed".to_owned(),
            error_message: Some(reason.clone()),
            timestamp: run.now,
            repo_id: Some(pair.repo_id),
            agent_id: Some(pair.agent_id),
            schedule_id: Some(run.schedule_id),
            schedule_name: Some(run.schedule_name.to_owned()),
            archive_name: None,
            run_id: None,
            duration_secs: None,
            original_size: None,
            compressed_size: None,
            deduplicated_size: None,
            files_processed: None,
            warnings: Vec::new(),
            next_run_at: run.next_run_at,
            activity_url: None,
        };
        if let Err(e) =
            crate::notifications::dispatch(&state.notification_service, event, &state.task_registry)
                .await
        {
            tracing::error!(
                schedule_id = run.schedule_id,
                error = %e,
                "failed to dispatch the abandoned-catch-up notification"
            );
        }
    }
}

/// Records one catch-up system event, shared with [`crate::repo_catch_up`].
pub(crate) async fn record_system_event(
    state: &AppState,
    event: SystemEventType,
    host: &str,
    msg: &str,
) {
    if let Err(e) = db::insert_system_event(&state.pool, event, Some(host), msg).await {
        tracing::error!(error = %e, "failed to record a catch-up system event");
    }
}

/// "3 days" rather than "4320 minutes", for a message a human reads once and
/// has to believe.
fn humanize_minutes(minutes: i32) -> String {
    const MINUTES_PER_HOUR: i32 = 60;
    const MINUTES_PER_DAY: i32 = 1_440;
    let (value, unit) = if minutes % MINUTES_PER_DAY == 0 {
        (minutes / MINUTES_PER_DAY, "day")
    } else if minutes % MINUTES_PER_HOUR == 0 {
        (minutes / MINUTES_PER_HOUR, "hour")
    } else {
        (minutes, "minute")
    };
    if value == 1 {
        format!("1 {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

fn skip(candidate: &CatchUpCandidate, reason: SkipReason) {
    tracing::info!(
        hostname = %candidate.hostname,
        schedule_id = candidate.schedule_id,
        missed_occurrence = %candidate.pending_for,
        %reason,
        "catch-up run: skipped"
    );
}

async fn record_catch_up_event(state: &AppState, candidate: &CatchUpCandidate) {
    let msg = format!(
        "Catching up the run schedule '{}' missed at {} while agent '{}' was unreachable",
        candidate.schedule_name, candidate.pending_for, candidate.hostname
    );
    if let Err(e) = db::insert_system_event(
        &state.pool,
        SystemEventType::ScheduleCatchUp,
        Some(&candidate.hostname),
        &msg,
    )
    .await
    {
        tracing::error!(
            schedule_id = candidate.schedule_id,
            error = %e,
            "failed to record schedule-catch-up system event"
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 6, 9, 0, 0).unwrap()
    }

    #[test]
    fn a_next_run_further_out_than_the_floor_leaves_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        assert!(has_room_before_next_run(Some(next), 120, now()));
    }

    /// The reconnect-just-before-the-nightly-run case: the regular run is about to
    /// happen, so the catch-up would only duplicate it.
    #[test]
    fn a_next_run_closer_than_the_floor_leaves_no_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::minutes(90))
            .unwrap();
        assert!(!has_room_before_next_run(Some(next), 120, now()));
    }

    /// Exactly the floor still runs - the field reads "at least this much time".
    #[test]
    fn a_next_run_exactly_at_the_floor_leaves_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::minutes(120))
            .unwrap();
        assert!(has_room_before_next_run(Some(next), 120, now()));
    }

    /// A schedule the reconnect handler just re-enabled has `next_run_at` set to
    /// the reconnect moment, so it is always inside the floor: the scheduler's own
    /// tick runs it seconds later, and the catch-up must not double it up.
    #[test]
    fn a_schedule_that_is_already_due_leaves_no_room() {
        assert!(!has_room_before_next_run(Some(now()), 1, now()));
    }

    /// An overdue next run is even further inside the floor than a due one.
    #[test]
    fn an_overdue_next_run_leaves_no_room() {
        let next = now()
            .checked_sub_signed(chrono::Duration::hours(3))
            .unwrap();
        assert!(!has_room_before_next_run(Some(next), 1, now()));
    }

    /// Nothing to collide with, so the floor has nothing to protect.
    #[test]
    fn no_next_run_at_all_leaves_room() {
        assert!(has_room_before_next_run(None, 10_080, now()));
    }

    #[test]
    fn skip_reasons_read_as_sentences() {
        assert_eq!(
            SkipReason::NextRunTooClose.to_string(),
            "the next scheduled run is too close"
        );
        assert_eq!(
            SkipReason::UnknownScheduleType.to_string(),
            "the schedule type is not recognised"
        );
        assert_eq!(
            SkipReason::GaveUpWaiting.to_string(),
            "the give-up window had already passed"
        );
    }

    fn waiting_since(pending_for: DateTime<Utc>, give_up_minutes: i32) -> CatchUpCandidate {
        CatchUpCandidate {
            schedule_id: 1,
            schedule_name: "Nightly workstations".to_owned(),
            agent_id: 1,
            hostname: "lab-ws-02".to_owned(),
            schedule_type: "backup".to_owned(),
            cron_expression: "0 2 * * *".to_owned(),
            pending_for,
            next_run_at: None,
            min_lead_minutes: 120,
            give_up_minutes,
        }
    }

    /// Zero is how every schedule behaved before the window existed, so it has
    /// to keep meaning "wait for as long as it takes" however long that is.
    #[test]
    fn a_zero_window_never_gives_up() {
        let candidate = waiting_since(now() - chrono::Duration::days(365), 0);
        assert!(!gave_up_by(&candidate, now()));
    }

    #[test]
    fn a_window_ends_exactly_when_it_says() {
        let pending_for = now() - chrono::Duration::days(3);
        let candidate = waiting_since(pending_for, 3 * 24 * 60);
        assert!(gave_up_by(&candidate, now()));
        assert!(
            !gave_up_by(&candidate, now() - chrono::Duration::minutes(1)),
            "a minute short of the window is still waiting"
        );
    }

    #[test]
    fn a_deadline_is_the_window_after_the_miss_or_none() {
        let missed = Utc.with_ymd_and_hms(2026, 9, 1, 2, 0, 0).unwrap();
        assert_eq!(give_up_deadline(missed, 0), None);
        assert_eq!(
            give_up_deadline(missed, 3 * 24 * 60),
            Some(Utc.with_ymd_and_hms(2026, 9, 4, 2, 0, 0).unwrap())
        );
    }

    #[test]
    fn minutes_read_as_the_largest_whole_unit_they_fit() {
        assert_eq!(humanize_minutes(1), "1 minute");
        assert_eq!(humanize_minutes(45), "45 minutes");
        assert_eq!(humanize_minutes(60), "1 hour");
        assert_eq!(humanize_minutes(90), "90 minutes");
        assert_eq!(humanize_minutes(4 * 60), "4 hours");
        assert_eq!(humanize_minutes(24 * 60), "1 day");
        assert_eq!(humanize_minutes(3 * 24 * 60), "3 days");
    }
}
