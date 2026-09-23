// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Catching up a run that failed because the *repository* was not there.
//!
//! The mirror of [`crate::catch_up`], and deliberately not symmetric with it.
//! An agent announces its own return by reconnecting its websocket, so the
//! agent half is event-driven: one handler, no waiting, no polling. A
//! repository has no connection to the server and no way to say anything, so
//! this half has to ask - on the repository's own `catch_up_recheck_minutes`,
//! until it answers or its give-up window runs out. Both are set on the
//! repository, beside the switch that says its host is not always online:
//! how often to ask a machine, and how long to wait for it, are facts about
//! that machine rather than about any schedule that writes to it.
//!
//! Three rules hold the whole thing together:
//!
//! * **One probe per repository per pass.** Several schedules can be waiting on
//!   the same host; one SSH connection answers all of them.
//! * **Decided once.** A marker is cleared as soon as it is acted on, whatever
//!   the outcome, exactly as the agent half does at reconnect. A catch-up that
//!   is dropped because the next regular run is imminent is not carried
//!   forward, because that run does the same work.
//! * **Bounded.** Without `catch_up_give_up_minutes` a weekly schedule can wait
//!   a full week on a host that is never coming back, reporting nothing worse
//!   than "skipped" the whole time. Past the window the marker is dropped and
//!   the run is reported as a plain backup failure, because that is what it is.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};
use shared::types::{ScheduleType, SystemEventType};
use uuid::Uuid;

use crate::{
    AppState,
    catch_up::{
        AbandonedCatchUp, AbandonedPair, CatchUpRun, give_up_deadline, has_room_before_next_run,
        record_system_event, report_abandoned_catch_up, spawn_catch_up_run,
    },
    db::{
        self,
        catch_up::{RepoCatchUpCandidate, RepoCatchUpFilter},
    },
    error::ApiError,
};

/// How often the poller wakes. This is not the re-check interval an operator
/// configures - that is per schedule, and enforced against
/// `catch_up_last_probe_at`. This is only how finely those intervals are
/// honoured, so a 15-minute setting fires within a minute of 15 minutes rather
/// than whenever the next backup happens to be due.
const DEFAULT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Overridable via `SCHEDULER_REPO_CATCH_UP_INTERVAL_SECS`, for the same reason
/// [`crate::scheduler`]'s intervals are: a coverage-instrumented e2e run sets it
/// past the length of the run so only the guaranteed-immediate first tick fires.
pub(crate) fn poll_interval() -> std::time::Duration {
    std::env::var("SCHEDULER_REPO_CATCH_UP_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_POLL_INTERVAL, |secs| {
            std::time::Duration::from_secs(secs.max(1))
        })
}

/// What should happen to one pending marker, decided without touching the
/// database or the network so the rules can be tested on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    /// The schedule or repository no longer qualifies - the repository no
    /// longer marked as not always online, or either one disabled. Drop the
    /// marker without running anything and without reporting a failure:
    /// nobody is waiting on this any more.
    Drop,
    /// The give-up window has passed. Drop the marker and report the run as
    /// failed, because it is never going to happen.
    GiveUp,
    /// Still waiting, but not due another probe yet.
    Wait,
    /// Due a probe: ask the repository whether it is back.
    Probe,
}

/// When this repository is next due to be asked whether it is back.
///
/// Measured from the last probe, or from the missed occurrence when it has not
/// been asked yet. The first probe of a wait is therefore due one interval
/// after the occurrence, which for a run that failed slowly is already in the
/// past - deliberately, because by then the news is worth having at once.
fn next_probe_at(candidate: &RepoCatchUpCandidate) -> DateTime<Utc> {
    let from = candidate.last_probe_at.unwrap_or(candidate.pending_for);
    // An interval that overflows the calendar leaves the probe due now, which
    // is the harmless direction: one extra SSH connection rather than a wait
    // that silently never ends.
    from.checked_add_signed(TimeDelta::minutes(i64::from(candidate.recheck_minutes)))
        .unwrap_or(from)
}

/// When this catch-up stops being worth waiting for, or `None` when the
/// schedule is set to wait indefinitely.
fn give_up_at(candidate: &RepoCatchUpCandidate) -> Option<DateTime<Utc>> {
    give_up_deadline(candidate.pending_for, candidate.give_up_minutes)
}

fn next_action(candidate: &RepoCatchUpCandidate, now: DateTime<Utc>) -> PendingAction {
    if !candidate.intermittent || !candidate.schedule_enabled || !candidate.repo_enabled {
        return PendingAction::Drop;
    }
    // Ordered ahead of the probe: a marker past its window is abandoned rather
    // than asked one more time, so the window means what it says.
    if give_up_at(candidate).is_some_and(|deadline| now >= deadline) {
        return PendingAction::GiveUp;
    }
    if now >= next_probe_at(candidate) {
        PendingAction::Probe
    } else {
        PendingAction::Wait
    }
}

/// Whether this pass may probe a repository that is not yet due one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbePolicy {
    /// Honour each schedule's re-check interval - the poller's own behaviour.
    Scheduled,
    /// Ask now regardless, because a human pressed the button rather than
    /// editing the interval down and back to force one.
    Forced,
}

/// What one pass did, so the "check now" endpoint can say something more useful
/// than "ok".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PassOutcome {
    /// Repositories actually asked this pass.
    pub probed: usize,
    /// Of those, how many answered.
    pub reachable: usize,
    /// Catch-up runs started.
    pub started: usize,
    /// Markers abandoned because their give-up window had passed.
    pub abandoned: usize,
    /// Markers dropped because they no longer qualified, or because the next
    /// regular run was too close to be worth pre-empting.
    pub dropped: usize,
}

/// One poller pass over every repository with a catch-up waiting on it.
pub async fn run_pending_repo_catch_ups(state: &AppState) {
    let candidates = match db::catch_up::list_repo_catch_up_candidates(
        &state.pool,
        RepoCatchUpFilter::All,
    )
    .await
    {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::error!(error = %e, "failed to look up pending repository catch-ups");
            return;
        }
    };
    run_pass(state, candidates, Utc::now(), ProbePolicy::Scheduled).await;
}

/// Every schedule waiting on one repository, with the clocks that say when it
/// is next asked and when the wait runs out - the list on its Power pane.
///
/// Filtered to the markers that are still live: one the poller would drop on
/// its next pass is not something to show as pending.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the candidate lookup fails.
pub(crate) async fn waiting_for_repo(
    state: &AppState,
    repo_id: i64,
) -> Result<Vec<shared::responses::CatchUpWaitResponse>, ApiError> {
    let now = Utc::now();
    Ok(
        db::catch_up::list_repo_catch_up_candidates(&state.pool, RepoCatchUpFilter::Repo(repo_id))
            .await?
            .into_iter()
            .filter(|c| next_action(c, now) != PendingAction::Drop)
            .map(|c| shared::responses::CatchUpWaitResponse {
                schedule_id: c.schedule_id,
                schedule_name: c.schedule_name.clone(),
                pending_for: c.pending_for,
                last_probe_at: c.last_probe_at,
                next_probe_at: Some(next_probe_at(&c)),
                give_up_at: give_up_at(&c),
            })
            .collect(),
    )
}

/// Asks one repository whether it is back, right now, and catches up every
/// schedule waiting on it if it is.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the candidate lookup fails.
pub(crate) async fn check_repo_now(
    state: &AppState,
    repo_id: i64,
) -> Result<PassOutcome, ApiError> {
    let candidates =
        db::catch_up::list_repo_catch_up_candidates(&state.pool, RepoCatchUpFilter::Repo(repo_id))
            .await?;
    Ok(run_pass(state, candidates, Utc::now(), ProbePolicy::Forced).await)
}

async fn run_pass(
    state: &AppState,
    candidates: Vec<RepoCatchUpCandidate>,
    now: DateTime<Utc>,
    policy: ProbePolicy,
) -> PassOutcome {
    let mut by_repo: BTreeMap<i64, Vec<RepoCatchUpCandidate>> = BTreeMap::new();
    for candidate in candidates {
        by_repo
            .entry(candidate.repo_id)
            .or_default()
            .push(candidate);
    }

    let mut outcome = PassOutcome::default();
    for (repo_id, group) in by_repo {
        outcome.merge(process_repo(state, repo_id, group, now, policy).await);
    }
    outcome
}

/// Everything waiting on one repository, settled with at most one SSH probe.
async fn process_repo(
    state: &AppState,
    repo_id: i64,
    group: Vec<RepoCatchUpCandidate>,
    now: DateTime<Utc>,
    policy: ProbePolicy,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    let mut waiting = Vec::new();
    let mut due = false;

    for candidate in group {
        match next_action(&candidate, now) {
            PendingAction::Drop => {
                if drop_marker(state, &candidate, "no longer eligible").await {
                    outcome.dropped = outcome.dropped.saturating_add(1);
                }
            }
            PendingAction::GiveUp => {
                if abandon(state, &candidate, now).await {
                    outcome.abandoned = outcome.abandoned.saturating_add(1);
                }
            }
            PendingAction::Probe => {
                due = true;
                waiting.push(candidate);
            }
            PendingAction::Wait => waiting.push(candidate),
        }
    }

    if waiting.is_empty() || (!due && policy == ProbePolicy::Scheduled) {
        return outcome;
    }

    let Ok(repo) = db::get_repo_by_id(&state.pool, repo_id).await else {
        tracing::error!(repo_id, "pending repository catch-up: repository is gone");
        return outcome;
    };
    let reachable = crate::power::repo_reachable(&repo).await;
    outcome.probed = outcome.probed.saturating_add(1);
    if reachable {
        outcome.reachable = outcome.reachable.saturating_add(1);
    }

    let schedule_ids: Vec<i64> = waiting.iter().map(|c| c.schedule_id).collect();
    if let Err(e) =
        db::catch_up::record_repo_catch_up_probe(&state.pool, &schedule_ids, repo_id, now).await
    {
        // Without a recorded probe the interval means nothing, so stop rather
        // than probe this host again on every pass from here on.
        tracing::error!(repo_id, error = %e, "failed to record a repository catch-up probe");
        return outcome;
    }

    if !reachable {
        tracing::debug!(
            repo_id,
            ssh_host = %repo.ssh_host,
            waiting = waiting.len(),
            "pending repository catch-up: still not answering"
        );
        return outcome;
    }

    for candidate in waiting {
        if dispatch(state, &candidate, now).await {
            outcome.started = outcome.started.saturating_add(1);
        } else {
            outcome.dropped = outcome.dropped.saturating_add(1);
        }
    }
    outcome
}

/// Runs one schedule's catch-up for the repository that just answered, unless
/// the next regular run is close enough that it would do the same work anyway.
///
/// Returns whether a run was started. Either way the marker is gone: the
/// decision is made once, here.
async fn dispatch(state: &AppState, candidate: &RepoCatchUpCandidate, now: DateTime<Utc>) -> bool {
    // Handed off rather than simply dropped, under the run id the catch-up
    // will carry: should that run fail against the repository again, the new
    // wait keeps this occurrence instead of starting its window over.
    let run_id = Uuid::new_v4().to_string();
    if let Err(e) = db::catch_up::hand_off_repo_catch_up(
        &state.pool,
        candidate.schedule_id,
        candidate.repo_id,
        &run_id,
    )
    .await
    {
        tracing::error!(
            schedule_id = candidate.schedule_id,
            repo_id = candidate.repo_id,
            error = %e,
            "repository catch-up: failed to clear the marker, leaving it for the next pass"
        );
        return false;
    }
    if !has_room_before_next_run(candidate.next_run_at, candidate.min_lead_minutes, now) {
        tracing::info!(
            schedule_id = candidate.schedule_id,
            repo_id = candidate.repo_id,
            missed_occurrence = %candidate.pending_for,
            "repository catch-up: skipped, the next scheduled run is too close"
        );
        return false;
    }
    let Ok(schedule_type) = candidate.schedule_type.parse::<ScheduleType>() else {
        tracing::warn!(
            schedule_id = candidate.schedule_id,
            "repository catch-up: skipped, the schedule type is not recognised"
        );
        return false;
    };
    let targets = match db::get_schedule_targets_for_run(&state.pool, candidate.schedule_id).await {
        Ok(targets) if !targets.is_empty() => targets,
        Ok(_) => {
            tracing::warn!(
                schedule_id = candidate.schedule_id,
                "repository catch-up: no target host left to run it, skipping"
            );
            return false;
        }
        Err(e) => {
            tracing::error!(
                schedule_id = candidate.schedule_id,
                error = %e,
                "repository catch-up: failed to resolve target hosts"
            );
            return false;
        }
    };

    tracing::info!(
        schedule_id = candidate.schedule_id,
        repo_id = candidate.repo_id,
        missed_occurrence = %candidate.pending_for,
        "repository catch-up: repository is back, running the occurrence it missed"
    );
    record_system_event(
        state,
        SystemEventType::ScheduleCatchUp,
        &candidate.repo_name,
        &format!(
            "Catching up the run schedule '{}' missed at {} while repository '{}' was unreachable",
            candidate.schedule_name, candidate.pending_for, candidate.repo_name
        ),
    )
    .await;

    // Only the repository that was away: every other repository of this
    // schedule was written on the day, and re-running them would duplicate a
    // backup that already succeeded.
    spawn_catch_up_run(
        state,
        CatchUpRun {
            schedule_id: candidate.schedule_id,
            schedule_type,
            cron_expression: candidate.cron_expression.clone(),
            targets,
            repo_ids: vec![candidate.repo_id],
            now,
            run_id,
        },
    )
    .await;
    true
}

/// Gives up on a catch-up whose window has run out, and reports the run as the
/// failure it turned out to be.
///
/// Returns whether the wait was actually ended: a marker that could not be
/// cleared is left for the next pass rather than reported now and again then.
async fn abandon(state: &AppState, candidate: &RepoCatchUpCandidate, now: DateTime<Utc>) -> bool {
    if !drop_marker(state, candidate, "give-up window passed").await {
        return false;
    }
    tracing::warn!(
        schedule_id = candidate.schedule_id,
        repo_id = candidate.repo_id,
        missed_occurrence = %candidate.pending_for,
        waited_minutes = candidate.give_up_minutes,
        "repository catch-up: abandoned, the repository did not come back in time"
    );
    // Every host that would have written this repository, and only this
    // repository: the schedule's others were written on the day.
    let pairs = db::get_schedule_targets_for_run(&state.pool, candidate.schedule_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|target| AbandonedPair {
            agent_id: target.agent_id,
            hostname: target.hostname,
            repo_id: candidate.repo_id,
            repo_name: candidate.repo_name.clone(),
        })
        .collect();
    report_abandoned_catch_up(
        state,
        &AbandonedCatchUp {
            schedule_id: candidate.schedule_id,
            schedule_name: &candidate.schedule_name,
            pending_for: candidate.pending_for,
            give_up_minutes: candidate.give_up_minutes,
            absent: format!("repository '{}'", candidate.repo_name),
            event_host: &candidate.repo_name,
            next_run_at: candidate.next_run_at,
            pairs,
            now,
        },
    )
    .await;
    true
}

/// Clears one marker, returning whether it is now safe to act on it. A failed
/// clear means the marker is still set, and acting anyway would run the same
/// catch-up again on the next pass.
async fn drop_marker(state: &AppState, candidate: &RepoCatchUpCandidate, why: &str) -> bool {
    match db::catch_up::clear_repo_catch_up_pending(
        &state.pool,
        candidate.schedule_id,
        candidate.repo_id,
    )
    .await
    {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(
                schedule_id = candidate.schedule_id,
                repo_id = candidate.repo_id,
                why,
                error = %e,
                "failed to clear a repository catch-up marker; leaving it for the next pass"
            );
            false
        }
    }
}

impl PassOutcome {
    /// Folds one repository's result into the pass total.
    fn merge(&mut self, other: Self) {
        self.probed = self.probed.saturating_add(other.probed);
        self.reachable = self.reachable.saturating_add(other.reachable);
        self.started = self.started.saturating_add(other.started);
        self.abandoned = self.abandoned.saturating_add(other.abandoned);
        self.dropped = self.dropped.saturating_add(other.dropped);
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 6, 9, 0, 0).unwrap()
    }

    /// A miss recorded at 02:00 this morning, seven hours before `now()`,
    /// re-checked every 15 minutes and never yet probed.
    fn candidate() -> RepoCatchUpCandidate {
        RepoCatchUpCandidate {
            schedule_id: 1,
            schedule_name: "Nightly servers".to_owned(),
            schedule_type: "backup".to_owned(),
            cron_expression: "0 2 * * *".to_owned(),
            repo_id: 7,
            repo_name: "borg-nas".to_owned(),
            pending_for: Utc.with_ymd_and_hms(2026, 9, 6, 2, 0, 0).unwrap(),
            last_probe_at: None,
            next_run_at: Some(Utc.with_ymd_and_hms(2026, 9, 7, 2, 0, 0).unwrap()),
            min_lead_minutes: 120,
            recheck_minutes: 15,
            give_up_minutes: 0,
            intermittent: true,
            schedule_enabled: true,
            repo_enabled: true,
        }
    }

    #[test]
    fn a_marker_never_probed_is_due_one_interval_after_the_miss() {
        let c = candidate();
        assert_eq!(next_probe_at(&c), c.pending_for + TimeDelta::minutes(15));
        assert_eq!(next_action(&c, now()), PendingAction::Probe);
    }

    #[test]
    fn a_marker_probed_moments_ago_waits_out_its_interval() {
        let mut c = candidate();
        c.last_probe_at = Some(now() - TimeDelta::minutes(6));
        assert_eq!(next_action(&c, now()), PendingAction::Wait);
    }

    #[test]
    fn a_marker_probed_a_full_interval_ago_is_due_again() {
        let mut c = candidate();
        c.last_probe_at = Some(now() - TimeDelta::minutes(15));
        assert_eq!(next_action(&c, now()), PendingAction::Probe);
    }

    /// Every gate is the operator saying they no longer want this run, so none
    /// of them reports a failure - the marker is simply dropped.
    #[test]
    fn a_marker_whose_schedule_stopped_qualifying_is_dropped() {
        for break_it in [
            (|c: &mut RepoCatchUpCandidate| c.intermittent = false) as fn(&mut _),
            |c: &mut RepoCatchUpCandidate| c.schedule_enabled = false,
            |c: &mut RepoCatchUpCandidate| c.repo_enabled = false,
        ] {
            let mut c = candidate();
            break_it(&mut c);
            assert_eq!(next_action(&c, now()), PendingAction::Drop);
        }
    }

    #[test]
    fn zero_means_wait_indefinitely() {
        let c = candidate();
        assert_eq!(c.give_up_minutes, 0);
        assert_eq!(give_up_at(&c), None);
        // A week past the miss and it is still only ever due another probe.
        assert_eq!(
            next_action(&c, now() + TimeDelta::days(7)),
            PendingAction::Probe
        );
    }

    #[test]
    fn a_wait_past_its_window_is_given_up_on() {
        let mut c = candidate();
        c.give_up_minutes = 3 * 24 * 60;
        assert_eq!(give_up_at(&c), Some(c.pending_for + TimeDelta::days(3)));
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::days(3)),
            PendingAction::GiveUp
        );
    }

    /// The window is measured from the missed occurrence, not from the last
    /// probe, so probing does not extend it.
    #[test]
    fn probing_does_not_extend_the_give_up_window() {
        let mut c = candidate();
        c.give_up_minutes = 60;
        c.last_probe_at = Some(c.pending_for + TimeDelta::minutes(45));
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::minutes(61)),
            PendingAction::GiveUp
        );
    }

    /// Giving up wins over probing: a marker that is both past its window and
    /// due a probe is abandoned rather than asked one more time, so the window
    /// means what it says.
    #[test]
    fn giving_up_is_decided_before_probing() {
        let mut c = candidate();
        c.give_up_minutes = 30;
        let later = c.pending_for + TimeDelta::hours(2);
        assert!(later >= next_probe_at(&c));
        assert_eq!(next_action(&c, later), PendingAction::GiveUp);
    }

    /// A disqualified marker is dropped even after its window has passed: there
    /// is no failure to report for a run nobody is waiting for any more.
    #[test]
    fn dropping_wins_over_giving_up() {
        let mut c = candidate();
        c.give_up_minutes = 30;
        c.schedule_enabled = false;
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::days(1)),
            PendingAction::Drop
        );
    }

    #[test]
    fn a_wait_inside_its_window_keeps_going() {
        let mut c = candidate();
        c.give_up_minutes = 3 * 24 * 60;
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::days(2)),
            PendingAction::Probe
        );
    }

    #[test]
    fn pass_outcomes_add_up() {
        let mut a = PassOutcome {
            probed: 1,
            reachable: 1,
            started: 2,
            abandoned: 0,
            dropped: 1,
        };
        let b = PassOutcome {
            probed: 2,
            reachable: 0,
            started: 0,
            abandoned: 3,
            dropped: 1,
        };
        a.merge(b);
        assert_eq!(
            a,
            PassOutcome {
                probed: 3,
                reachable: 1,
                started: 2,
                abandoned: 3,
                dropped: 2,
            }
        );
    }
}
