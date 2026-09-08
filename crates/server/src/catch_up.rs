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

use chrono::{DateTime, Utc};
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
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NextRunTooClose => write!(f, "the next scheduled run is too close"),
            Self::UnknownScheduleType => write!(f, "the schedule type is not recognised"),
        }
    }
}

/// Whether `candidate` may still run, given how much time is left before its
/// schedule's next regular run.
///
/// A schedule with no computed next run has nothing for the catch-up to collide
/// with, so the floor doesn't apply to it.
fn has_room_before_next_run(candidate: &CatchUpCandidate, now: DateTime<Utc>) -> bool {
    let Some(next_run_at) = candidate.next_run_at else {
        return true;
    };
    let lead = i64::from(candidate.min_lead_minutes);
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
    let Ok(schedule_type) = candidate.schedule_type.parse::<ScheduleType>() else {
        skip(candidate, SkipReason::UnknownScheduleType);
        return;
    };
    if !has_room_before_next_run(candidate, now) {
        skip(candidate, SkipReason::NextRunTooClose);
        return;
    }

    let run_id = Uuid::new_v4().to_string();
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

    if matches!(schedule_type, ScheduleType::Backup) {
        for repo_id in &repo_ids {
            if let Err(e) = db::insert_backup_pending(
                &state.pool,
                candidate.agent_id,
                *repo_id,
                Some(candidate.schedule_id),
                &run_id,
                now,
            )
            .await
            {
                tracing::warn!(
                    hostname = %candidate.hostname,
                    schedule_id = candidate.schedule_id,
                    repo_id = *repo_id,
                    error = %e,
                    "catch-up run: failed to insert pending record"
                );
            }
        }
    }

    tracing::info!(
        hostname = %candidate.hostname,
        schedule_id = candidate.schedule_id,
        missed_occurrence = %candidate.pending_for,
        "catch-up run: host reconnected, running the occurrence it missed"
    );
    record_catch_up_event(state, candidate).await;

    let request = RunRequest {
        repo_ids: repo_ids.into_iter().map(RepoId).collect(),
        schedule_type,
        schedule_id: candidate.schedule_id,
        run_id,
        origin: RunOrigin::CatchUp,
    };
    let state = state.clone();
    let schedule_id = candidate.schedule_id;
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

    fn candidate(next_run_at: Option<DateTime<Utc>>, min_lead_minutes: i32) -> CatchUpCandidate {
        CatchUpCandidate {
            schedule_id: 1,
            schedule_name: "Nightly workstations".to_owned(),
            agent_id: 1,
            hostname: "lab-ws-02".to_owned(),
            schedule_type: "backup".to_owned(),
            pending_for: Utc.with_ymd_and_hms(2026, 9, 1, 2, 0, 0).unwrap(),
            next_run_at,
            min_lead_minutes,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 6, 9, 0, 0).unwrap()
    }

    #[test]
    fn a_next_run_further_out_than_the_floor_leaves_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        assert!(has_room_before_next_run(&candidate(Some(next), 120), now()));
    }

    /// The reconnect-just-before-the-nightly-run case: the regular run is about to
    /// happen, so the catch-up would only duplicate it.
    #[test]
    fn a_next_run_closer_than_the_floor_leaves_no_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::minutes(90))
            .unwrap();
        assert!(!has_room_before_next_run(
            &candidate(Some(next), 120),
            now()
        ));
    }

    /// Exactly the floor still runs - the field reads "at least this much time".
    #[test]
    fn a_next_run_exactly_at_the_floor_leaves_room() {
        let next = now()
            .checked_add_signed(chrono::Duration::minutes(120))
            .unwrap();
        assert!(has_room_before_next_run(&candidate(Some(next), 120), now()));
    }

    /// A schedule the reconnect handler just re-enabled has `next_run_at` set to
    /// the reconnect moment, so it is always inside the floor: the scheduler's own
    /// tick runs it seconds later, and the catch-up must not double it up.
    #[test]
    fn a_schedule_that_is_already_due_leaves_no_room() {
        assert!(!has_room_before_next_run(&candidate(Some(now()), 1), now()));
    }

    /// An overdue next run is even further inside the floor than a due one.
    #[test]
    fn an_overdue_next_run_leaves_no_room() {
        let next = now()
            .checked_sub_signed(chrono::Duration::hours(3))
            .unwrap();
        assert!(!has_room_before_next_run(&candidate(Some(next), 1), now()));
    }

    /// Nothing to collide with, so the floor has nothing to protect.
    #[test]
    fn no_next_run_at_all_leaves_room() {
        assert!(has_room_before_next_run(&candidate(None, 10_080), now()));
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
    }
}
