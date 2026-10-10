// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Catching up a run that was skipped because a dependency was away.
//!
//! The third sibling of [`crate::catch_up`] (agents) and
//! [`crate::repo_catch_up`] (repositories), and closest to the latter: a
//! dependency has no connection to announce its return on, so it is asked -
//! one TCP connection per dependency per pass, on its
//! `catch_up_recheck_minutes` - until it answers or its give-up window runs
//! out. Both are set on the dependency, beside the switch that marks it as not
//! always online.
//!
//! It differs from both siblings in when a marker goes away. Theirs are
//! cleared the moment a catch-up is dispatched; a dependency marker is only
//! handed to the catch-up run, and the agent's report settles it. A catch-up
//! that finds a dependency away again - this one, or another the same target
//! needs - hands it back with the original occurrence intact, so a flapping
//! machine can neither lose the run nor stretch its give-up window.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use shared::{dependency_hosts::DependencyWaitResponse, types::SystemEventType};
use uuid::Uuid;

use crate::{
    AppState,
    catch_up::{
        AbandonedCatchUp, CatchUpRun, abandoned_pairs, has_room_before_next_run,
        record_system_event, report_abandoned_catch_up, spawn_catch_up_run,
    },
    db::{
        self,
        dependency_catch_ups::{DependencyCatchUpCandidate, DependencyCatchUpFilter},
    },
    error::ApiError,
    repo_catch_up::{PassOutcome, PendingAction, ProbePolicy, Triage, WaitWindow, triage},
};

fn window(candidate: &DependencyCatchUpCandidate) -> WaitWindow {
    WaitWindow {
        pending_for: candidate.pending_for,
        last_probe_at: candidate.last_probe_at,
        recheck_minutes: candidate.recheck_minutes,
        give_up_minutes: candidate.give_up_minutes,
    }
}

fn next_action(candidate: &DependencyCatchUpCandidate, now: DateTime<Utc>) -> PendingAction {
    if !candidate.intermittent || !candidate.schedule_enabled {
        return PendingAction::Drop;
    }
    match window(candidate).action(now) {
        // A catch-up already has it: its run settles the marker or hands it
        // back, so it is not probed for meanwhile. Giving up still applies.
        PendingAction::Probe if candidate.dispatched_run_id.is_some() => PendingAction::Wait,
        action => action,
    }
}

/// One poller pass over every dependency with a catch-up waiting on it.
pub async fn run_pending_dependency_catch_ups(state: &AppState) {
    let candidates = match db::dependency_catch_ups::list_dependency_catch_up_candidates(
        &state.pool,
        DependencyCatchUpFilter::All,
    )
    .await
    {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::error!(error = %e, "failed to look up pending dependency catch-ups");
            return;
        }
    };
    run_pass(state, candidates, Utc::now(), ProbePolicy::Scheduled).await;
}

/// Asks one dependency whether it is back, right now, and catches up every
/// target waiting on it if it is.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the candidate lookup fails.
pub(crate) async fn check_dependency_now(
    state: &AppState,
    dependency_host_id: i64,
) -> Result<PassOutcome, ApiError> {
    let candidates = db::dependency_catch_ups::list_dependency_catch_up_candidates(
        &state.pool,
        DependencyCatchUpFilter::Dependency(dependency_host_id),
    )
    .await?;
    Ok(run_pass(state, candidates, Utc::now(), ProbePolicy::Forced).await)
}

/// The targets waiting on one dependency - the list on its Power pane.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the lookup fails.
pub(crate) async fn waiting_for_dependency(
    state: &AppState,
    dependency_host_id: i64,
) -> Result<Vec<DependencyWaitResponse>, ApiError> {
    waiting(
        state,
        DependencyCatchUpFilter::Dependency(dependency_host_id),
    )
    .await
}

/// The targets of one schedule waiting on a dependency - its Overview.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the lookup fails.
pub(crate) async fn waiting_for_schedule(
    state: &AppState,
    schedule_id: i64,
) -> Result<Vec<DependencyWaitResponse>, ApiError> {
    waiting(state, DependencyCatchUpFilter::Schedule(schedule_id)).await
}

async fn waiting(
    state: &AppState,
    filter: DependencyCatchUpFilter,
) -> Result<Vec<DependencyWaitResponse>, ApiError> {
    let now = Utc::now();
    Ok(
        db::dependency_catch_ups::list_dependency_catch_up_candidates(&state.pool, filter)
            .await?
            .into_iter()
            .filter(|c| next_action(c, now) != PendingAction::Drop)
            .map(|c| DependencyWaitResponse {
                schedule_id: c.schedule_id,
                schedule_name: c.schedule_name.clone(),
                agent_id: c.agent_id,
                hostname: c.hostname.clone(),
                dependency_host_id: c.dependency_host_id,
                dependency_name: c.dependency_name.clone(),
                pending_for: c.pending_for,
                last_probe_at: c.last_probe_at,
                next_probe_at: c
                    .dispatched_run_id
                    .is_none()
                    .then(|| window(&c).next_probe_at()),
                give_up_at: window(&c).give_up_at(),
                catching_up: c.dispatched_run_id.is_some(),
            })
            .collect(),
    )
}

async fn run_pass(
    state: &AppState,
    candidates: Vec<DependencyCatchUpCandidate>,
    now: DateTime<Utc>,
    policy: ProbePolicy,
) -> PassOutcome {
    let mut by_dependency: BTreeMap<i64, Vec<DependencyCatchUpCandidate>> = BTreeMap::new();
    for candidate in candidates {
        by_dependency
            .entry(candidate.dependency_host_id)
            .or_default()
            .push(candidate);
    }
    let mut outcome = PassOutcome::default();
    for (dependency_host_id, group) in by_dependency {
        outcome.merge(process_dependency(state, dependency_host_id, group, now, policy).await);
    }
    outcome
}

/// Everything waiting on one dependency, settled with at most one probe.
async fn process_dependency(
    state: &AppState,
    dependency_host_id: i64,
    group: Vec<DependencyCatchUpCandidate>,
    now: DateTime<Utc>,
    policy: ProbePolicy,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    let Triage {
        to_drop,
        to_abandon,
        mut waiting,
        due,
    } = triage(group, |c| next_action(c, now));
    for candidate in &to_drop {
        let dropped = drop_marker(state, candidate).await;
        outcome.dropped = outcome.dropped.saturating_add(usize::from(dropped));
    }
    for candidate in &to_abandon {
        let abandoned = abandon(state, candidate, now).await;
        outcome.abandoned = outcome.abandoned.saturating_add(usize::from(abandoned));
    }
    // One a catch-up already has is not probed for: its run will settle it or
    // hand it back.
    waiting.retain(|c| c.dispatched_run_id.is_none());

    if waiting.is_empty() || (!due && policy == ProbePolicy::Scheduled) {
        return outcome;
    }

    let dependency = match db::dependency_hosts::get_required_dependency(
        &state.pool,
        dependency_host_id,
    )
    .await
    {
        Ok(dependency) => dependency,
        Err(e) => {
            tracing::error!(
                dependency_host_id,
                error = %e,
                "pending dependency catch-up: the dependency could not be loaded"
            );
            return outcome;
        }
    };
    let reachable = super::probe_and_record(&state.pool, &dependency).await;
    outcome.probed = outcome.probed.saturating_add(1);
    if let Err(e) = db::dependency_catch_ups::record_dependency_catch_up_probe(
        &state.pool,
        dependency_host_id,
        now,
    )
    .await
    {
        // Without a recorded probe the interval means nothing; stop rather
        // than ask this dependency on every pass from here on.
        tracing::error!(
            dependency_host_id,
            error = %e,
            "failed to record a dependency catch-up probe"
        );
        return outcome;
    }
    if !reachable {
        tracing::debug!(
            dependency = %dependency.name,
            waiting = waiting.len(),
            "pending dependency catch-up: still not answering"
        );
        return outcome;
    }
    outcome.reachable = outcome.reachable.saturating_add(1);

    for candidate in waiting {
        if dispatch(state, &candidate, now).await {
            outcome.started = outcome.started.saturating_add(1);
        } else {
            outcome.dropped = outcome.dropped.saturating_add(1);
        }
    }
    outcome
}

/// Runs one target's catch-up now that its dependency answers, unless the next
/// regular run is close enough to do the same work. Returns whether a run was
/// started.
async fn dispatch(
    state: &AppState,
    candidate: &DependencyCatchUpCandidate,
    now: DateTime<Utc>,
) -> bool {
    if !has_room_before_next_run(candidate.next_run_at, candidate.min_lead_minutes, now) {
        tracing::info!(
            schedule_id = candidate.schedule_id,
            hostname = %candidate.hostname,
            missed_occurrence = %candidate.pending_for,
            "dependency catch-up: skipped, the next scheduled run is too close"
        );
        // The next run does the same work; nothing is carried forward.
        drop_marker(state, candidate).await;
        return false;
    }
    let repo_ids =
        match db::catch_up::list_enabled_catch_up_repos(&state.pool, candidate.schedule_id).await {
            Ok(repo_ids) if !repo_ids.is_empty() => repo_ids,
            Ok(_) => {
                tracing::warn!(
                    schedule_id = candidate.schedule_id,
                    "dependency catch-up: no enabled repository left, dropping"
                );
                drop_marker(state, candidate).await;
                return false;
            }
            Err(e) => {
                tracing::error!(
                    schedule_id = candidate.schedule_id,
                    error = %e,
                    "dependency catch-up: failed to resolve the repositories"
                );
                return false;
            }
        };

    let run_id = Uuid::new_v4().to_string();
    match db::dependency_catch_ups::hand_off_dependency_catch_up(
        &state.pool,
        candidate.schedule_id,
        candidate.agent_id,
        &run_id,
    )
    .await
    {
        Ok(true) => {}
        // Taken by a concurrent pass: that one runs it.
        Ok(false) => return false,
        Err(e) => {
            tracing::error!(
                schedule_id = candidate.schedule_id,
                error = %e,
                "dependency catch-up: failed to hand the marker over, leaving it for the next pass"
            );
            return false;
        }
    }

    tracing::info!(
        schedule_id = candidate.schedule_id,
        hostname = %candidate.hostname,
        dependency = %candidate.dependency_name,
        missed_occurrence = %candidate.pending_for,
        "dependency catch-up: the dependency is back, running the occurrence it kept from running"
    );
    record_system_event(
        state,
        SystemEventType::ScheduleCatchUp,
        &candidate.hostname,
        &format!(
            "Catching up the run schedule '{}' skipped at {} on '{}' while dependency '{}' was \
             unreachable",
            candidate.schedule_name,
            candidate.pending_for,
            candidate.hostname,
            candidate.dependency_name
        ),
    )
    .await;

    spawn_catch_up_run(
        state,
        CatchUpRun {
            schedule_id: candidate.schedule_id,
            schedule_type: candidate.schedule_type,
            cron_expression: candidate.cron_expression.clone(),
            targets: vec![db::ScheduleRunTarget {
                agent_id: candidate.agent_id,
                hostname: candidate.hostname.clone(),
            }],
            repo_ids,
            now,
            run_id,
        },
    )
    .await;
    true
}

/// Gives up on a wait whose window has run out, and reports it.
async fn abandon(
    state: &AppState,
    candidate: &DependencyCatchUpCandidate,
    now: DateTime<Utc>,
) -> bool {
    if !drop_marker(state, candidate).await {
        return false;
    }
    tracing::warn!(
        schedule_id = candidate.schedule_id,
        hostname = %candidate.hostname,
        dependency = %candidate.dependency_name,
        missed_occurrence = %candidate.pending_for,
        waited_minutes = candidate.give_up_minutes,
        "dependency catch-up: abandoned, the dependency did not come back in time"
    );
    let pairs = abandoned_pairs(
        state,
        candidate.schedule_id,
        candidate.agent_id,
        &candidate.hostname,
    )
    .await;
    report_abandoned_catch_up(
        state,
        &AbandonedCatchUp {
            schedule_id: candidate.schedule_id,
            schedule_name: &candidate.schedule_name,
            pending_for: candidate.pending_for,
            give_up_minutes: candidate.give_up_minutes,
            absent: format!("dependency '{}'", candidate.dependency_name),
            event_host: &candidate.hostname,
            next_run_at: candidate.next_run_at,
            pairs,
            now,
        },
    )
    .await;
    true
}

/// Drops one marker, returning whether this call is the one that did.
async fn drop_marker(state: &AppState, candidate: &DependencyCatchUpCandidate) -> bool {
    match db::dependency_catch_ups::clear_dependency_catch_up(
        &state.pool,
        candidate.schedule_id,
        candidate.agent_id,
    )
    .await
    {
        Ok(taken) => taken,
        Err(e) => {
            tracing::error!(
                schedule_id = candidate.schedule_id,
                error = %e,
                "failed to clear a dependency catch-up marker; leaving it for the next pass"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone};

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap()
    }

    /// A skip recorded at 02:00, seven hours before `now()`, re-checked every
    /// 15 minutes and never yet probed.
    fn candidate() -> DependencyCatchUpCandidate {
        DependencyCatchUpCandidate {
            schedule_id: 1,
            schedule_name: "Media share nightly".to_owned(),
            schedule_type: shared::types::ScheduleType::Backup,
            cron_expression: "0 2 * * *".to_owned(),
            agent_id: 4,
            hostname: "media-store-01".to_owned(),
            dependency_host_id: 9,
            dependency_name: "nas-media".to_owned(),
            pending_for: Utc.with_ymd_and_hms(2026, 10, 2, 2, 0, 0).unwrap(),
            last_probe_at: None,
            dispatched_run_id: None,
            next_run_at: Some(Utc.with_ymd_and_hms(2026, 10, 3, 2, 0, 0).unwrap()),
            min_lead_minutes: 120,
            recheck_minutes: 15,
            give_up_minutes: 0,
            intermittent: true,
            schedule_enabled: true,
        }
    }

    #[test]
    fn a_marker_never_probed_is_due_one_interval_after_the_skip() {
        let c = candidate();
        assert_eq!(
            window(&c).next_probe_at(),
            c.pending_for + TimeDelta::minutes(15)
        );
        assert_eq!(next_action(&c, now()), PendingAction::Probe);
    }

    /// A catch-up in flight is not probed for again: its run settles the
    /// marker or hands it back.
    #[test]
    fn a_marker_handed_to_a_catch_up_waits_for_it() {
        let mut c = candidate();
        c.dispatched_run_id = Some("run-1".to_owned());
        assert_eq!(next_action(&c, now()), PendingAction::Wait);
    }

    #[test]
    fn a_marker_whose_dependency_or_schedule_stopped_qualifying_is_dropped() {
        let mut c = candidate();
        c.intermittent = false;
        assert_eq!(next_action(&c, now()), PendingAction::Drop);
        let mut c = candidate();
        c.schedule_enabled = false;
        assert_eq!(next_action(&c, now()), PendingAction::Drop);
    }

    #[test]
    fn zero_means_wait_indefinitely() {
        let c = candidate();
        assert_eq!(window(&c).give_up_at(), None);
        assert_eq!(
            next_action(&c, now() + TimeDelta::days(30)),
            PendingAction::Probe
        );
    }

    #[test]
    fn a_wait_past_its_window_is_given_up_on_even_while_a_catch_up_has_it() {
        let mut c = candidate();
        c.give_up_minutes = 24 * 60;
        c.dispatched_run_id = Some("run-1".to_owned());
        c.last_probe_at = Some(c.pending_for + TimeDelta::hours(23));
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::days(1)),
            PendingAction::GiveUp
        );
    }

    /// The window is measured from the skipped occurrence, so asking does not
    /// extend it.
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

    mod db_backed {
        use chrono::TimeDelta;
        use shared::{protocol::ServerToAgent, types::SystemEventType};

        use super::super::*;
        use crate::{
            dependencies::tests::{CLOSED_PORT, Fixture},
            ws::completion_bus::OperationOutcome,
        };

        /// Marks the fixture's target as waiting on `dependency_host_id` since
        /// `ago` before now.
        async fn wait_on(fx: &Fixture, dependency_host_id: i64, ago: TimeDelta) {
            db::dependency_catch_ups::mark_dependency_catch_up(
                fx.pool(),
                fx.schedule.id,
                fx.agent.id,
                dependency_host_id,
                Utc::now().checked_sub_signed(ago).unwrap(),
            )
            .await
            .unwrap();
        }

        async fn set_give_up(fx: &Fixture, dependency_host_id: i64, give_up_minutes: i32) {
            db::dependency_hosts::update_dependency_host_availability(
                fx.pool(),
                dependency_host_id,
                db::dependency_hosts::DependencyAvailability {
                    intermittent: true,
                    recheck_minutes: 15,
                    give_up_minutes,
                },
            )
            .await
            .unwrap();
        }

        async fn last_check(fx: &Fixture, dependency_host_id: i64) -> Option<bool> {
            db::dependency_hosts::get_dependency_host(fx.pool(), dependency_host_id)
                .await
                .unwrap()
                .last_check_reachable
        }

        async fn candidates(fx: &Fixture) -> Vec<DependencyCatchUpCandidate> {
            db::dependency_catch_ups::list_dependency_catch_up_candidates(
                fx.pool(),
                DependencyCatchUpFilter::All,
            )
            .await
            .unwrap()
        }

        /// The poller asks a dependency that is due, remembers that it did,
        /// and leaves the target waiting while it does not answer.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_pass_asks_a_silent_dependency_and_keeps_its_targets_waiting(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "poll-silent").await;
            let dependency = fx.require("nas-silent", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;

            run_pending_dependency_catch_ups(&fx.state).await;

            assert_eq!(last_check(&fx, dependency.id).await, Some(false));
            let markers = fx.markers().await;
            assert_eq!(markers.len(), 1);
            assert!(markers.first().unwrap().last_probe_at.is_some());
            assert!(
                fx.system_events().await.is_empty(),
                "nothing happened worth an activity entry"
            );

            let outcome = check_dependency_now(&fx.state, dependency.id)
                .await
                .unwrap();
            assert_eq!(
                outcome,
                PassOutcome {
                    probed: 1,
                    ..PassOutcome::default()
                }
            );
            assert_eq!(fx.markers().await.len(), 1);
        }

        /// A scheduled pass leaves a dependency asked moments ago alone until
        /// its re-check interval is up; Check now asks regardless.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_scheduled_pass_waits_out_the_recheck_interval_but_check_now_does_not(
            pool: sqlx::PgPool,
        ) {
            let fx = Fixture::new(pool, "poll-interval").await;
            let dependency = fx.require("nas-interval", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;
            let now = Utc::now();
            db::dependency_catch_ups::record_dependency_catch_up_probe(
                fx.pool(),
                dependency.id,
                now.checked_sub_signed(TimeDelta::minutes(2)).unwrap(),
            )
            .await
            .unwrap();

            let outcome = run_pass(
                &fx.state,
                candidates(&fx).await,
                now,
                ProbePolicy::Scheduled,
            )
            .await;
            assert_eq!(outcome, PassOutcome::default());
            assert_eq!(
                last_check(&fx, dependency.id).await,
                None,
                "a dependency not yet due must not be asked"
            );

            let outcome =
                run_pass(&fx.state, candidates(&fx).await, now, ProbePolicy::Forced).await;
            assert_eq!(outcome.probed, 1);
            assert_eq!(last_check(&fx, dependency.id).await, Some(false));
        }

        /// A catch-up already handed a run is not asked about: that run
        /// settles it or hands it back.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_marker_handed_to_a_run_is_not_probed_for(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "poll-handed").await;
            let dependency = fx.require("nas-handed", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;
            assert!(
                db::dependency_catch_ups::hand_off_dependency_catch_up(
                    fx.pool(),
                    fx.schedule.id,
                    fx.agent.id,
                    "run-in-flight",
                )
                .await
                .unwrap()
            );

            let outcome = check_dependency_now(&fx.state, dependency.id)
                .await
                .unwrap();

            assert_eq!(outcome, PassOutcome::default());
            assert_eq!(last_check(&fx, dependency.id).await, None);
            assert_eq!(
                fx.markers()
                    .await
                    .first()
                    .unwrap()
                    .dispatched_run_id
                    .as_deref(),
                Some("run-in-flight")
            );
        }

        /// A dependency that answers again starts the occurrence it kept from
        /// running: the activity log says so, the marker is handed to the new
        /// run, and the agent is sent that run.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_dependency_that_answers_again_starts_the_catch_up(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "poll-back").await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = i32::from(listener.local_addr().unwrap().port());
            let dependency = fx.require("nas-back", port, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;
            let (tx, mut rx) = tokio::sync::mpsc::channel(32);
            fx.state
                .registry
                .register(fx.agent.id, tx, false, None, None)
                .await;

            let outcome = check_dependency_now(&fx.state, dependency.id)
                .await
                .unwrap();

            assert_eq!(
                outcome,
                PassOutcome {
                    probed: 1,
                    reachable: 1,
                    started: 1,
                    ..PassOutcome::default()
                }
            );
            assert_eq!(last_check(&fx, dependency.id).await, Some(true));
            let events = fx.system_events().await;
            assert_eq!(
                events.iter().map(|e| e.0).collect::<Vec<_>>(),
                vec![SystemEventType::ScheduleCatchUp]
            );
            assert!(events.first().unwrap().1.contains("dependency 'nas-back'"));

            let run_id = loop {
                if let ServerToAgent::RunBackupNow { run_id, .. } = rx.recv().await.unwrap() {
                    break run_id;
                }
            };
            let marker = fx.markers().await;
            let marker = marker.first().unwrap();
            assert_eq!(
                marker.dispatched_run_id, run_id,
                "the marker must be handed to the run the agent was sent"
            );
            assert_eq!(
                fx.reports()
                    .await
                    .into_iter()
                    .map(|r| r.0)
                    .collect::<Vec<_>>(),
                vec![shared::types::ReportStatus::Pending]
            );
            fx.state.completion_bus.publish(OperationOutcome {
                agent_id: fx.agent.id,
                repo_id: fx.repo.id,
                success: true,
            });
        }

        /// A wait past its window is given up on and reported, and one whose
        /// schedule was switched off is dropped without a report - neither
        /// needs the dependency asked.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_pass_gives_up_on_an_expired_wait_and_drops_one_nobody_wants(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "poll-give-up").await;
            let dependency = fx.require("nas-gone", CLOSED_PORT, true).await;
            set_give_up(&fx, dependency.id, 60).await;
            wait_on(&fx, dependency.id, TimeDelta::days(2)).await;
            let disabled = db::insert_schedule(
                fx.pool(),
                fx.repo.id,
                &db::ScheduleParams::for_test("poll-give-up-disabled", "0 3 * * *"),
                None,
            )
            .await
            .unwrap();
            db::set_schedule_enabled(fx.pool(), disabled.id, false)
                .await
                .unwrap();
            db::dependency_catch_ups::mark_dependency_catch_up(
                fx.pool(),
                disabled.id,
                fx.agent.id,
                dependency.id,
                Utc::now(),
            )
            .await
            .unwrap();

            let outcome = check_dependency_now(&fx.state, dependency.id)
                .await
                .unwrap();

            assert_eq!(
                outcome,
                PassOutcome {
                    abandoned: 1,
                    dropped: 1,
                    ..PassOutcome::default()
                }
            );
            assert!(
                candidates(&fx).await.is_empty(),
                "both waits must have ended"
            );
            assert_eq!(last_check(&fx, dependency.id).await, None);
            let events = fx.system_events().await;
            assert_eq!(
                events.iter().map(|e| e.0).collect::<Vec<_>>(),
                vec![SystemEventType::ScheduleCatchUpAbandoned]
            );
            assert!(events.first().unwrap().1.contains("dependency 'nas-gone'"));
        }

        /// Giving up reports once: a marker another pass already took is not
        /// reported again.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_wait_already_taken_is_not_abandoned_twice(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "poll-taken").await;
            let dependency = fx.require("nas-taken", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::days(2)).await;
            let candidate = candidates(&fx).await.into_iter().next().unwrap();

            assert!(abandon(&fx.state, &candidate, Utc::now()).await);
            assert!(!abandon(&fx.state, &candidate, Utc::now()).await);

            assert_eq!(fx.system_events().await.len(), 1);
        }

        /// A dependency back too close to the schedule's next run is not
        /// caught up - that run does the same work - and nothing waits on.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_catch_up_too_close_to_the_next_run_is_dropped(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "dispatch-too-close").await;
            let dependency = fx.require("nas-late", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;
            let now = Utc::now();
            let candidate = DependencyCatchUpCandidate {
                next_run_at: now.checked_add_signed(TimeDelta::hours(1)),
                min_lead_minutes: 120,
                ..candidates(&fx).await.into_iter().next().unwrap()
            };

            assert!(!dispatch(&fx.state, &candidate, now).await);

            assert!(
                fx.markers().await.is_empty(),
                "the decision is made once: nothing waits on"
            );
            assert!(
                fx.system_events().await.is_empty(),
                "nothing happened worth an activity entry"
            );
        }

        /// Two passes racing for one marker start it once.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn a_marker_another_pass_took_is_not_started_again(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "dispatch-taken").await;
            let dependency = fx.require("nas-raced", CLOSED_PORT, true).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;
            let candidate = candidates(&fx).await.into_iter().next().unwrap();
            assert!(
                db::dependency_catch_ups::hand_off_dependency_catch_up(
                    fx.pool(),
                    fx.schedule.id,
                    fx.agent.id,
                    "run-other-pass",
                )
                .await
                .unwrap()
            );

            assert!(!dispatch(&fx.state, &candidate, Utc::now()).await);

            assert_eq!(
                fx.markers()
                    .await
                    .first()
                    .unwrap()
                    .dispatched_run_id
                    .as_deref(),
                Some("run-other-pass")
            );
            assert!(
                fx.system_events().await.is_empty(),
                "nothing happened worth an activity entry"
            );
            assert!(
                fx.reports().await.is_empty(),
                "no run may have been started"
            );
        }

        /// The waits a dependency's Power pane and a schedule's Overview list:
        /// when it is next asked while it waits, that it is catching up once
        /// handed to a run, and nothing for a wait nobody wants any more.
        #[ignore = "requires DATABASE_URL"]
        #[sqlx::test(migrations = "./migrations")]
        async fn the_waits_are_listed_by_dependency_and_by_schedule(pool: sqlx::PgPool) {
            let fx = Fixture::new(pool, "waiting-list").await;
            let dependency = fx.require("nas-listed", CLOSED_PORT, true).await;
            set_give_up(&fx, dependency.id, 24 * 60).await;
            wait_on(&fx, dependency.id, TimeDelta::hours(1)).await;

            let by_dependency = waiting_for_dependency(&fx.state, dependency.id)
                .await
                .unwrap();
            assert_eq!(by_dependency.len(), 1);
            let wait = by_dependency.first().unwrap();
            assert_eq!(wait.schedule_id, fx.schedule.id);
            assert_eq!(wait.agent_id, fx.agent.id);
            assert_eq!(wait.dependency_name, "nas-listed");
            assert_eq!(
                wait.next_probe_at,
                wait.pending_for.checked_add_signed(TimeDelta::minutes(15))
            );
            assert_eq!(
                wait.give_up_at,
                wait.pending_for.checked_add_signed(TimeDelta::days(1))
            );
            assert!(!wait.catching_up);

            db::dependency_catch_ups::hand_off_dependency_catch_up(
                fx.pool(),
                fx.schedule.id,
                fx.agent.id,
                "run-listed",
            )
            .await
            .unwrap();
            let by_schedule = waiting_for_schedule(&fx.state, fx.schedule.id)
                .await
                .unwrap();
            assert_eq!(by_schedule.len(), 1);
            let wait = by_schedule.first().unwrap();
            assert!(wait.catching_up);
            assert_eq!(wait.next_probe_at, None);

            db::set_schedule_enabled(fx.pool(), fx.schedule.id, false)
                .await
                .unwrap();
            assert!(
                waiting_for_schedule(&fx.state, fx.schedule.id)
                    .await
                    .unwrap()
                    .is_empty(),
                "a wait nobody wants any more is not listed"
            );
        }
    }

    #[test]
    fn dropping_wins_over_giving_up() {
        let mut c = candidate();
        c.give_up_minutes = 30;
        c.dispatched_run_id = Some("run-1".to_owned());
        c.intermittent = false;
        assert_eq!(
            next_action(&c, c.pending_for + TimeDelta::days(1)),
            PendingAction::Drop
        );
    }
}
