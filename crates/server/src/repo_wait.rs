// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Waiting for a repository host instead of waking it.
//!
//! A schedule waits for a repository host when the host's machine is marked
//! as not always online, or when the schedule is set to wait for it. A host
//! the schedule waits for and does not wake is checked *before* the target's
//! agent is woken, so a backup that cannot be written is never started: the
//! agent stays asleep and its pre-backup commands never run. Targets on such a
//! host that is down when the run starts are moved to the end of the run, so
//! everything that can be written is written first; when their turn comes
//! the host is asked again, and only one that still does not answer is
//! skipped and caught up once it does (see [`crate::repo_catch_up`]).
//!
//! A host that is woken is checked after its wake instead: one that is waited
//! for and still does not answer is skipped before anything is sent to the
//! agent. A host the schedule does not wait for is never gated - see
//! [`crate::power::repo_reachable`] for why.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use shared::types::{
    MachinePolicy, MachineWakeMode, ReportStatus, RunEventTarget, RunEventType,
    ScheduleWakeOverride, SystemEventType, WaitReason,
};

use crate::{
    db::{self, DueScheduleRow, RepoRow},
    notifications::EventType,
    power::{self, PowerCtx, RunEventSite},
};

/// What one run does about each repository it writes: its row and the
/// resolved policy for its host's machine.
#[derive(Debug, Default)]
pub(crate) struct RepoWaitPlan {
    modes: HashMap<i64, MachineWakeMode>,
    repos: HashMap<i64, (RepoRow, MachinePolicy)>,
}

impl RepoWaitPlan {
    /// Resolves the policy of every repository `targets` write to, under the
    /// schedule's own settings. A repository whose row cannot be read is left
    /// out: it is not gated, and the run goes ahead the way it always has.
    pub(crate) async fn load(
        pool: &sqlx::PgPool,
        schedule_id: i64,
        schedule_override: ScheduleWakeOverride,
        targets: &[DueScheduleRow],
    ) -> Self {
        let modes: HashMap<i64, MachineWakeMode> =
            match db::machines::list_schedule_machine_wake(pool, schedule_id).await {
                Ok(choices) => choices
                    .into_iter()
                    .map(|c| (c.machine_id, c.mode))
                    .collect(),
                Err(e) => {
                    tracing::error!(
                        schedule_id,
                        error = %e,
                        "failed to look up a schedule's per-machine wake choices"
                    );
                    HashMap::new()
                }
            };
        let repo_ids: HashSet<i64> = targets.iter().map(|t| t.repo_id).collect();
        let mut repos = HashMap::new();
        for repo_id in repo_ids {
            let loaded = match db::get_repo_by_id(pool, repo_id).await {
                Ok(repo) => db::machines::get_machine(pool, repo.machine_id)
                    .await
                    .map(|machine| (repo, machine)),
                Err(e) => Err(e),
            };
            match loaded {
                Ok((repo, machine)) => {
                    let policy = MachinePolicy::resolve(
                        schedule_override,
                        modes.get(&repo.machine_id).copied(),
                        machine.wake_enabled,
                        machine.intermittent,
                    );
                    repos.insert(repo_id, (repo, policy));
                }
                Err(e) => tracing::warn!(
                    schedule_id,
                    repo_id,
                    error = %e,
                    "failed to load a repository to decide whether to wait for its host"
                ),
            }
        }
        Self { modes, repos }
    }

    /// The schedule-wide override as it applies to one machine, given the
    /// schedule's own choice for it, if any.
    pub(crate) fn wake_override_for(
        &self,
        schedule_override: ScheduleWakeOverride,
        machine_id: i64,
    ) -> ScheduleWakeOverride {
        schedule_override.for_machine(self.modes.get(&machine_id).copied())
    }

    /// The policy for `repo_id`'s host, if its row was read.
    pub(crate) fn policy(&self, repo_id: i64) -> Option<MachinePolicy> {
        self.repos.get(&repo_id).map(|(_, policy)| *policy)
    }

    /// The repository row and the reason the run waits for its host, for one
    /// that is waited for and never woken - the only kind checked before the
    /// agent is.
    fn waited_without_wake(&self, repo_id: i64) -> Option<(&RepoRow, WaitReason)> {
        let (repo, policy) = self.repos.get(&repo_id)?;
        match policy.wait {
            Some(reason) if !policy.wake => Some((repo, reason)),
            Some(_) | None => None,
        }
    }

    /// Moves the targets whose repository host is waited for, never woken, and
    /// down right now to the end of the run, keeping the order within each
    /// group. Each such host is asked once, however many targets write to it.
    pub(crate) async fn order_targets(
        &self,
        pool: &sqlx::PgPool,
        targets: Vec<DueScheduleRow>,
    ) -> Vec<DueScheduleRow> {
        let mut hosts: BTreeMap<i64, &RepoRow> = BTreeMap::new();
        for target in &targets {
            if let Some((repo, _)) = self.waited_without_wake(target.repo_id) {
                hosts.entry(repo.machine_id).or_insert(repo);
            }
        }
        let mut down = HashSet::new();
        for (machine_id, repo) in hosts {
            if !probe(pool, repo).await {
                down.insert(machine_id);
            }
        }
        if down.is_empty() {
            return targets;
        }
        let (waiting, ready): (Vec<_>, Vec<_>) = targets.into_iter().partition(|target| {
            self.waited_without_wake(target.repo_id)
                .is_some_and(|(repo, _)| down.contains(&repo.machine_id))
        });
        ready.into_iter().chain(waiting).collect()
    }

    /// Asks the host of a target's repository whether it is up, if the run
    /// waits for it and never wakes it. Returns why the run waits for it when
    /// it does not answer, so the target can be skipped before its agent is
    /// woken; `None` lets the target go ahead.
    pub(crate) async fn check_before_power(
        &self,
        ctx: PowerCtx<'_>,
        site: RunEventSite<'_>,
    ) -> Option<WaitReason> {
        let (repo, reason) = self.waited_without_wake(site.repo_id)?;
        if probe(ctx.pool, repo).await {
            return None;
        }
        power::record_run_event(
            ctx,
            site,
            RunEventTarget::Repository,
            RunEventType::HostUnreachable,
            format!(
                "{} did not answer SSH -- not woken: this schedule waits for it",
                repo.ssh_host
            ),
        )
        .await;
        Some(reason)
    }
}

/// Asks a repository's host whether it answers SSH, and remembers the answer
/// on its machine.
async fn probe(pool: &sqlx::PgPool, repo: &RepoRow) -> bool {
    let reachable = power::repo_reachable(repo).await;
    if let Err(e) =
        db::machines::record_machine_check(pool, repo.machine_id, reachable, Utc::now()).await
    {
        tracing::warn!(repo_id = repo.id, error = %e, "failed to record a host check");
    }
    reachable
}

/// Why a target was skipped, phrased for its report and alert.
pub(crate) fn waiting_reason(repo_name: &str, ssh_host: &str, reason: WaitReason) -> String {
    match reason {
        WaitReason::Machine => {
            format!("the host for repository '{repo_name}' ({ssh_host}) did not answer SSH")
        }
        WaitReason::Schedule => format!(
            "waiting for '{ssh_host}', the host for repository '{repo_name}', which this schedule \
             never wakes"
        ),
    }
}

/// The two events a skipped target is recorded as, by why it was waited for.
/// A machine marked as not always online keeps the event rules already
/// follow; a schedule's own choice to wait gets one of its own.
pub(crate) const fn events_for(reason: WaitReason) -> (SystemEventType, EventType) {
    match reason {
        WaitReason::Machine => (
            SystemEventType::BackupSkippedRepoOffline,
            EventType::BackupSkippedRepoOffline,
        ),
        WaitReason::Schedule => (
            SystemEventType::BackupWaitingForHost,
            EventType::BackupWaitingForHost,
        ),
    }
}

/// Everything [`report_skipped`] needs about the target it reports.
pub(crate) struct SkippedTarget<'a> {
    /// Database pool.
    pub pool: &'a sqlx::PgPool,
    /// Where the alert goes.
    pub notification_service: &'a crate::notifications::NotificationService,
    /// Joins the alert's delivery task on shutdown.
    pub task_registry: &'a shared::task_registry::TaskRegistry,
    /// The schedule.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: &'a str,
    /// The run.
    pub run_id: &'a str,
    /// The target that was skipped.
    pub target: &'a DueScheduleRow,
    /// When the run was decided on.
    pub now: DateTime<Utc>,
}

/// Reports a target skipped because the repository host it writes to was
/// waited for and did not answer: its report row settled as `skipped` with the
/// reason, an Activity Log entry, the matching alert, and a catch-up marker so
/// the run follows once the host answers.
pub(crate) async fn report_skipped(skipped: &SkippedTarget<'_>, reason: WaitReason) {
    let target = skipped.target;
    let (repo_name, ssh_host) = match db::get_repo_by_id(skipped.pool, target.repo_id).await {
        Ok(repo) => (repo.name, repo.ssh_host),
        Err(_) => (String::new(), String::new()),
    };
    let message = waiting_reason(&repo_name, &ssh_host, reason);
    let (system_event, event_type) = events_for(reason);

    if let Err(e) = db::settle_pending_report_for_target(
        skipped.pool,
        skipped.run_id,
        target.agent_id,
        target.repo_id,
        ReportStatus::Skipped,
        &message,
    )
    .await
    {
        tracing::error!(
            schedule_id = skipped.schedule_id,
            repo_id = target.repo_id,
            error = %e,
            "failed to settle the report of a target skipped for its repository host"
        );
    }
    let log_line = format!(
        "Backup for schedule '{}' on '{}' skipped: {message}",
        skipped.schedule_name, target.hostname
    );
    if let Err(e) = db::insert_system_event(
        skipped.pool,
        system_event,
        Some(&target.hostname),
        &log_line,
    )
    .await
    {
        tracing::error!(
            schedule_id = skipped.schedule_id,
            error = %e,
            "failed to record a repository-host wait in the Activity Log"
        );
    }
    if let Err(e) = db::catch_up::mark_repo_catch_up_pending(
        skipped.pool,
        skipped.schedule_id,
        target.repo_id,
        target.due_at,
        None,
    )
    .await
    {
        tracing::error!(
            schedule_id = skipped.schedule_id,
            repo_id = target.repo_id,
            error = %e,
            "failed to record a pending repository catch-up"
        );
    }

    let event = crate::notifications::NotificationEvent {
        event_type,
        hostname: target.hostname.clone(),
        repo_name,
        status: ReportStatus::Skipped.to_string(),
        error_message: Some(message),
        timestamp: skipped.now,
        repo_id: Some(target.repo_id),
        agent_id: Some(target.agent_id),
        schedule_id: Some(skipped.schedule_id),
        schedule_name: Some(skipped.schedule_name.to_owned()),
        archive_name: None,
        run_id: Some(skipped.run_id.to_owned()),
        duration_secs: None,
        original_size: None,
        compressed_size: None,
        deduplicated_size: None,
        files_processed: None,
        warnings: Vec::new(),
        next_run_at: None,
        activity_url: None,
    };
    if let Err(e) =
        crate::notifications::dispatch(skipped.notification_service, event, skipped.task_registry)
            .await
    {
        tracing::error!(
            schedule_id = skipped.schedule_id,
            error = %e,
            "failed to dispatch the repository-host wait notification"
        );
    }
}
