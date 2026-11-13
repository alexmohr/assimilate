// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Agent deploys running on the server.
//!
//! A deploy uploads a binary over SSH, restarts a service and waits for the
//! agent to come back, which takes long enough that a proxy in front of the
//! server can cut the request that started it. The request therefore only
//! starts the deploy; the work runs here, one deploy per agent, and the UI
//! reads its progress from [`DeployJobs::status`].
//!
//! The new agent token is the one secret a deploy produces. It is kept only
//! for the user who started the deploy, in memory, until they dismiss the
//! result or [`FINISHED_JOB_TTL`] passes, and it is never part of `Debug`
//! output.

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use shared::responses::{
    DeployJobState, DeployStatusResponse, DeployStep, DeployStepResponse, DeployStepStatus,
};
use strum::IntoEnumIterator;

/// How long a finished deploy's result stays readable.
const FINISHED_JOB_TTL: Duration = Duration::from_mins(10);

/// How long a deploy may stay in progress before it counts as abandoned, so a
/// task that died without reporting cannot block the agent's deploys forever.
const RUNNING_JOB_TTL: Duration = Duration::from_mins(30);

/// Why [`DeployJobs::start`] refused to start a deploy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// The agent already has a deploy in progress.
    #[error("a deploy is already running for this agent")]
    AlreadyRunning,
}

/// What a finished deploy leaves for the UI to show.
pub struct Outcome {
    /// Whether nothing was done because the agent was already current.
    pub skipped: bool,
    /// The agent token that was written to the host.
    pub token: Option<String>,
    /// Latest available agent version.
    pub available_version: Option<String>,
    /// Why the deploy failed; `None` when it succeeded.
    pub error: Option<String>,
}

#[derive(Clone, Copy)]
struct StepRecord {
    status: DeployStepStatus,
    started: Option<Instant>,
    duration: Option<Duration>,
}

struct Job {
    requested_by: i64,
    started: Instant,
    finished: Option<Instant>,
    steps: Vec<(DeployStep, StepRecord)>,
    outcome: Option<Outcome>,
}

impl fmt::Debug for Job {
    // The outcome holds the agent token, so it is left out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job")
            .field("requested_by", &self.requested_by)
            .field("finished", &self.finished.is_some())
            .field("outcome", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl Job {
    fn new(requested_by: i64) -> Self {
        let pending = StepRecord {
            status: DeployStepStatus::Pending,
            started: None,
            duration: None,
        };
        Self {
            requested_by,
            started: Instant::now(),
            finished: None,
            steps: DeployStep::iter().map(|step| (step, pending)).collect(),
            outcome: None,
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        self.finished.map_or_else(
            || now.duration_since(self.started) > RUNNING_JOB_TTL,
            |finished| now.duration_since(finished) > FINISHED_JOB_TTL,
        )
    }

    /// Finishes whichever step is running, as `status`.
    fn settle_running(&mut self, status: DeployStepStatus, now: Instant) {
        self.steps
            .iter_mut()
            .filter(|(_, record)| record.status == DeployStepStatus::Running)
            .for_each(|(_, record)| {
                record.status = status;
                record.duration = record.started.map(|started| now.duration_since(started));
            });
    }
}

/// Every agent's current deploy, by agent id.
#[derive(Clone, Default)]
pub struct DeployJobs {
    jobs: Arc<Mutex<HashMap<i64, Job>>>,
}

impl DeployJobs {
    fn lock(&self) -> MutexGuard<'_, HashMap<i64, Job>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a deploy of `agent_id` for `user_id`, replacing a finished or
    /// abandoned one.
    ///
    /// # Errors
    ///
    /// Returns [`StartError::AlreadyRunning`] if one is still in progress.
    pub fn start(&self, agent_id: i64, user_id: i64) -> Result<(), StartError> {
        let now = Instant::now();
        let mut jobs = self.lock();
        let running = jobs
            .get(&agent_id)
            .is_some_and(|job| job.finished.is_none() && !job.is_expired(now));
        if running {
            return Err(StartError::AlreadyRunning);
        }
        jobs.insert(agent_id, Job::new(user_id));
        Ok(())
    }

    /// Moves the deploy on to `step`: whichever step was running is done.
    pub fn begin_step(&self, agent_id: i64, step: DeployStep) {
        let now = Instant::now();
        if let Some(job) = self.lock().get_mut(&agent_id) {
            job.settle_running(DeployStepStatus::Done, now);
            if let Some((_, record)) = job.steps.iter_mut().find(|(s, _)| *s == step) {
                record.status = DeployStepStatus::Running;
                record.started = Some(now);
            }
        }
    }

    /// Ends the deploy with `outcome`. A step still running when it failed is
    /// the one that failed; on success every step is done.
    pub fn finish(&self, agent_id: i64, outcome: Outcome) {
        let now = Instant::now();
        if let Some(job) = self.lock().get_mut(&agent_id) {
            let settled = if outcome.error.is_some() {
                DeployStepStatus::Failed
            } else {
                DeployStepStatus::Done
            };
            job.settle_running(settled, now);
            job.finished = Some(now);
            job.outcome = Some(outcome);
        }
    }

    /// Forgets the deploy, wherever it stands. Used when it could not even
    /// begin, and when the user dismisses a result.
    pub fn remove(&self, agent_id: i64) {
        self.lock().remove(&agent_id);
    }

    /// Forgets the deploy if `user_id` started it.
    pub fn dismiss(&self, agent_id: i64, user_id: i64) {
        let mut jobs = self.lock();
        if jobs
            .get(&agent_id)
            .is_some_and(|job| job.requested_by == user_id)
        {
            jobs.remove(&agent_id);
        }
    }

    /// The state of the agent's deploy as `user_id` may see it: the token goes
    /// only to the user who started it. `None` when there is none, or it has
    /// expired.
    #[must_use]
    pub fn status(&self, agent_id: i64, user_id: i64) -> Option<DeployStatusResponse> {
        let now = Instant::now();
        let mut jobs = self.lock();
        if jobs.get(&agent_id).is_some_and(|job| job.is_expired(now)) {
            jobs.remove(&agent_id);
            return None;
        }
        let job = jobs.get(&agent_id)?;
        let owner = job.requested_by == user_id;
        let state = match (&job.outcome, job.finished) {
            (Some(outcome), Some(_)) if outcome.error.is_some() => DeployJobState::Failed,
            (Some(_), Some(_)) => DeployJobState::Succeeded,
            _ => DeployJobState::Running,
        };
        Some(DeployStatusResponse {
            state,
            steps: job
                .steps
                .iter()
                .map(|(step, record)| DeployStepResponse {
                    step: *step,
                    status: record.status,
                    duration_ms: record
                        .duration
                        .map(|d| u32::try_from(d.as_millis()).unwrap_or(u32::MAX)),
                })
                .collect(),
            skipped: job.outcome.as_ref().is_some_and(|o| o.skipped),
            token: job
                .outcome
                .as_ref()
                .filter(|_| owner)
                .and_then(|o| o.token.clone()),
            available_version: job
                .outcome
                .as_ref()
                .and_then(|o| o.available_version.clone()),
            error: job.outcome.as_ref().and_then(|o| o.error.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: i64 = 7;
    const OWNER: i64 = 1;
    const OTHER: i64 = 2;

    fn outcome(error: Option<&str>) -> Outcome {
        Outcome {
            skipped: false,
            token: Some("tok-secret".to_owned()),
            available_version: Some("1.2.0".to_owned()),
            error: error.map(str::to_owned),
        }
    }

    fn statuses(jobs: &DeployJobs) -> Vec<DeployStepStatus> {
        jobs.status(AGENT, OWNER)
            .expect("a job")
            .steps
            .iter()
            .map(|s| s.status)
            .collect()
    }

    #[test]
    fn a_new_deploy_starts_with_every_step_pending() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();

        let status = jobs.status(AGENT, OWNER).unwrap();
        assert_eq!(status.state, DeployJobState::Running);
        assert_eq!(
            status.steps.iter().map(|s| s.step).collect::<Vec<_>>(),
            DeployStep::iter().collect::<Vec<_>>()
        );
        assert!(
            status
                .steps
                .iter()
                .all(|s| s.status == DeployStepStatus::Pending && s.duration_ms.is_none())
        );
    }

    #[test]
    fn beginning_a_step_finishes_the_one_before_it() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.begin_step(AGENT, DeployStep::Connect);
        jobs.begin_step(AGENT, DeployStep::Upload);

        assert_eq!(
            statuses(&jobs),
            [
                DeployStepStatus::Done,
                DeployStepStatus::Running,
                DeployStepStatus::Pending,
                DeployStepStatus::Pending,
                DeployStepStatus::Pending,
            ]
        );
        let status = jobs.status(AGENT, OWNER).unwrap();
        assert_eq!(
            status
                .steps
                .iter()
                .map(|s| s.duration_ms.is_some())
                .collect::<Vec<_>>(),
            [true, false, false, false, false]
        );
    }

    #[test]
    fn a_second_deploy_of_the_same_agent_is_refused_while_one_runs() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();

        assert_eq!(jobs.start(AGENT, OTHER), Err(StartError::AlreadyRunning));
        jobs.start(AGENT + 1, OTHER)
            .expect("another agent is unaffected");
    }

    #[test]
    fn a_finished_deploy_can_be_replaced() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));

        jobs.start(AGENT, OTHER)
            .expect("a finished deploy no longer blocks");
        assert_eq!(
            jobs.status(AGENT, OTHER).unwrap().state,
            DeployJobState::Running
        );
    }

    #[test]
    fn a_failure_marks_the_running_step_failed_and_leaves_later_ones_pending() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.begin_step(AGENT, DeployStep::Connect);
        jobs.begin_step(AGENT, DeployStep::Upload);
        jobs.begin_step(AGENT, DeployStep::Install);
        jobs.finish(AGENT, outcome(Some("cp: Text file busy")));

        assert_eq!(
            statuses(&jobs),
            [
                DeployStepStatus::Done,
                DeployStepStatus::Done,
                DeployStepStatus::Failed,
                DeployStepStatus::Pending,
                DeployStepStatus::Pending,
            ]
        );
        let status = jobs.status(AGENT, OWNER).unwrap();
        assert_eq!(status.state, DeployJobState::Failed);
        assert_eq!(status.error.as_deref(), Some("cp: Text file busy"));
    }

    #[test]
    fn success_leaves_every_started_step_done() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        DeployStep::iter().for_each(|step| jobs.begin_step(AGENT, step));
        jobs.finish(AGENT, outcome(None));

        assert!(statuses(&jobs).iter().all(|s| *s == DeployStepStatus::Done));
        assert_eq!(
            jobs.status(AGENT, OWNER).unwrap().state,
            DeployJobState::Succeeded
        );
    }

    #[test]
    fn only_the_user_who_started_the_deploy_gets_the_token() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));

        assert_eq!(
            jobs.status(AGENT, OWNER).unwrap().token.as_deref(),
            Some("tok-secret")
        );
        let other = jobs.status(AGENT, OTHER).unwrap();
        assert_eq!(other.token, None);
        assert_eq!(
            other.state,
            DeployJobState::Succeeded,
            "others still see how it went"
        );
    }

    #[test]
    fn the_token_stays_readable_until_it_is_dismissed() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));

        // A reply that never reached the browser must not lose the token.
        assert!(jobs.status(AGENT, OWNER).unwrap().token.is_some());
        assert!(jobs.status(AGENT, OWNER).unwrap().token.is_some());

        jobs.dismiss(AGENT, OWNER);
        assert!(jobs.status(AGENT, OWNER).is_none());
    }

    #[test]
    fn dismissing_someone_elses_deploy_does_nothing() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));

        jobs.dismiss(AGENT, OTHER);
        assert!(jobs.status(AGENT, OWNER).is_some());
    }

    #[test]
    fn a_skipped_deploy_reports_it_without_a_token() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(
            AGENT,
            Outcome {
                skipped: true,
                token: None,
                available_version: Some("1.2.0".to_owned()),
                error: None,
            },
        );

        let status = jobs.status(AGENT, OWNER).unwrap();
        assert!(status.skipped);
        assert_eq!(status.token, None);
        assert_eq!(status.state, DeployJobState::Succeeded);
    }

    #[test]
    fn an_abandoned_running_deploy_stops_blocking() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.lock().get_mut(&AGENT).unwrap().started = Instant::now()
            .checked_sub(RUNNING_JOB_TTL + Duration::from_secs(1))
            .expect("an instant in the past");

        jobs.start(AGENT, OTHER)
            .expect("an abandoned deploy no longer blocks");
    }

    #[test]
    fn an_expired_result_is_gone() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));
        jobs.lock().get_mut(&AGENT).unwrap().finished =
            Instant::now().checked_sub(FINISHED_JOB_TTL + Duration::from_secs(1));

        assert!(jobs.status(AGENT, OWNER).is_none());
    }

    #[test]
    fn the_debug_output_never_shows_the_token() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.finish(AGENT, outcome(None));

        let shown = format!("{:?}", jobs.lock().get(&AGENT).unwrap());
        assert!(!shown.contains("tok-secret"), "{shown}");
    }

    #[test]
    fn removing_a_deploy_frees_the_agent() {
        let jobs = DeployJobs::default();
        jobs.start(AGENT, OWNER).unwrap();
        jobs.remove(AGENT);

        assert!(jobs.status(AGENT, OWNER).is_none());
        jobs.start(AGENT, OTHER).unwrap();
    }
}
