// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Checking, waking and waiting for dependency hosts: machines a backup needs
//! besides its agent and its repository, such as the server whose share a
//! pre-backup command mounts.
//!
//! A scheduled backup checks every dependency its target needs after the
//! agent and repository hosts are woken and before anything is sent to the
//! agent. A dependency that does not answer is woken if it is set up for
//! that, and given its wake window to come up. One that still does not answer
//! decides the target's run:
//!
//! * marked as not always online, the run is **skipped** and caught up once it
//!   answers again ([`catch_up`]);
//! * otherwise the run has **failed**, the same as any other error.
//!
//! Either way the target's pre-backup commands never run and nothing is
//! written: a mount against a machine that is not there would only fail more
//! slowly, or - worse - leave the mount point empty for borg to back up.
//!
//! A manual or catch-up run checks without waking. A manual run never wakes
//! hosts; a catch-up run is only started once the poller has seen the
//! dependency answer.

pub mod catch_up;

use std::time::Duration;

use chrono::Utc;
use shared::types::{RunEventTarget, RunEventType, ScheduleWakeOverride};

use crate::{
    db::{self, dependency_hosts::RequiredDependency},
    power::{self, MacAddress, PowerCtx, PowerHostKey, RunEventSite},
};

/// How long one connection attempt may take before the dependency counts as
/// not answering.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a dependency answers on its port right now.
///
/// A TCP connection is all it takes: an open port says the machine is up, not
/// that the share on it is exported, which is why the pre-backup command that
/// mounts it keeps its own check of the mount point.
pub(crate) async fn probe(address: &str, port: i32) -> bool {
    let Ok(port) = u16::try_from(port) else {
        return false;
    };
    matches!(
        tokio::time::timeout(
            PROBE_TIMEOUT,
            tokio::net::TcpStream::connect((address, port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// Probes a dependency and remembers the answer, so the list can show it.
pub(crate) async fn probe_and_record(pool: &sqlx::PgPool, dependency: &RequiredDependency) -> bool {
    let reachable = probe(&dependency.address, dependency.port).await;
    if let Err(e) =
        db::dependency_hosts::record_dependency_check(pool, dependency.id, reachable, Utc::now())
            .await
    {
        tracing::warn!(
            dependency = %dependency.name,
            error = %e,
            "failed to record a dependency check"
        );
    }
    reachable
}

/// Whether a check may wake a dependency that does not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WakePolicy {
    /// A scheduled run: wake per the dependency's own setting, overridden by
    /// the schedule's.
    Wake(ScheduleWakeOverride),
    /// A manual or catch-up run: check only.
    CheckOnly,
}

/// What checking a target's dependencies found.
#[derive(Debug, Default)]
pub(crate) struct DependencyCheck {
    /// The first dependency that did not answer, if any did not.
    pub down: Option<RequiredDependency>,
    /// Repository hosts this check holds a power session on, because a
    /// dependency is the same machine. Released by [`release`] once the
    /// target's run is done.
    pub held_repo_hosts: Vec<i64>,
}

/// Why a run could not go ahead, phrased for an alert and a report.
pub(crate) fn down_reason(dependency: &RequiredDependency) -> String {
    format!(
        "dependency '{}' did not answer on port {} ({})",
        dependency.name, dependency.port, dependency.address
    )
}

/// Checks every dependency one target needs, in name order, waking each that
/// does not answer if `policy` allows. Stops at the first that still does not
/// answer: the run is decided either way.
///
/// Infallible like the rest of the power step: a lookup that fails is logged
/// and treated as "no dependencies", so a database hiccup does not turn into a
/// backup that never ran.
pub(crate) async fn check(
    ctx: PowerCtx<'_>,
    schedule_id: i64,
    site: RunEventSite<'_>,
    policy: WakePolicy,
) -> DependencyCheck {
    let mut outcome = DependencyCheck::default();
    let required = match db::dependency_hosts::list_required_dependencies(
        ctx.pool,
        schedule_id,
        site.agent_id,
    )
    .await
    {
        Ok(required) => required,
        Err(e) => {
            tracing::error!(
                schedule_id,
                hostname = %site.hostname,
                error = %e,
                "failed to look up the dependencies of this target; checking none"
            );
            return outcome;
        }
    };

    for dependency in required {
        // Held before the check, the same way a repository target reserves
        // its host before waking it: a repository run on the same machine
        // that finishes while this one waits must not shut it down.
        if let Some(host_id) = dependency.repo_host_id
            && !outcome.held_repo_hosts.contains(&host_id)
        {
            ctx.power_sessions
                .reserve(PowerHostKey::RepoHost(host_id))
                .await;
            outcome.held_repo_hosts.push(host_id);
        }
        if !ensure_one(ctx, &dependency, site, policy).await {
            outcome.down = Some(dependency);
            break;
        }
    }
    outcome
}

/// Makes sure one dependency answers, waking it first if called for. Returns
/// whether it does.
async fn ensure_one(
    ctx: PowerCtx<'_>,
    dependency: &RequiredDependency,
    site: RunEventSite<'_>,
    policy: WakePolicy,
) -> bool {
    if probe_and_record(ctx.pool, dependency).await {
        return true;
    }
    let event = |event_type, message: String| {
        power::record_run_event(ctx, site, RunEventTarget::Dependency, event_type, message)
    };
    event(
        RunEventType::ReachabilityCheck,
        format!(
            "Checked {} on port {} -- no response",
            dependency.name, dependency.port
        ),
    )
    .await;

    let wake = match policy {
        WakePolicy::Wake(wake_override) => wake_override.resolve(dependency.wake_enabled),
        WakePolicy::CheckOnly => false,
    };
    if wake && wake_and_wait(ctx, dependency, site).await {
        return true;
    }

    event(
        RunEventType::HostUnreachable,
        format!(
            "{} did not answer on port {}",
            dependency.name, dependency.port
        ),
    )
    .await;
    false
}

/// Sends the Wake-on-LAN packet and waits out the wake window. Returns whether
/// the dependency answered within it.
async fn wake_and_wait(
    ctx: PowerCtx<'_>,
    dependency: &RequiredDependency,
    site: RunEventSite<'_>,
) -> bool {
    let event = |event_type, message: String| {
        power::record_run_event(ctx, site, RunEventTarget::Dependency, event_type, message)
    };
    let Some(mac) = dependency
        .wake_mac_address
        .as_deref()
        .and_then(|s| s.parse::<MacAddress>().ok())
    else {
        event(
            RunEventType::WakeUnavailable,
            format!(
                "Cannot wake {} -- no MAC address configured",
                dependency.name
            ),
        )
        .await;
        return false;
    };
    let broadcast = dependency
        .wake_broadcast_address
        .as_deref()
        .unwrap_or(power::DEFAULT_BROADCAST_ADDR);
    if let Err(e) = power::send_wol_packet(mac, broadcast).await {
        tracing::warn!(
            dependency = %dependency.name,
            error = %e,
            "failed to send a Wake-on-LAN packet to a dependency"
        );
        return false;
    }
    // Credit the wake to the repository host's session when they are one
    // machine, so that host's own shut-down setting applies once every run
    // relying on it is done - the same as a repository target that woke it.
    if let Some(host_id) = dependency.repo_host_id {
        ctx.power_sessions
            .record_outcome(PowerHostKey::RepoHost(host_id), true, false)
            .await;
    }
    event(
        RunEventType::WakeSent,
        format!("Sent Wake-on-LAN packet to {} ({mac})", dependency.name),
    )
    .await;

    let answered = power::wait_for(
        power::timeout_duration(dependency.wake_timeout_seconds),
        || probe(&dependency.address, dependency.port),
    )
    .await;
    if let Err(e) =
        db::dependency_hosts::record_dependency_check(ctx.pool, dependency.id, answered, Utc::now())
            .await
    {
        tracing::warn!(dependency = %dependency.name, error = %e, "failed to record a dependency check");
    }
    if answered {
        event(
            RunEventType::HostOnline,
            format!("{} answered on port {}", dependency.name, dependency.port),
        )
        .await;
    }
    answered
}

/// Everything [`report_scheduled_down`] needs about the target whose run a
/// dependency kept from starting.
pub(crate) struct ScheduledDown<'a> {
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
    /// The target's agent.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: &'a str,
    /// The repository this row of the target would have written.
    pub repo_id: i64,
    /// The run.
    pub run_id: &'a str,
    /// The occurrence the run stood for.
    pub due_at: chrono::DateTime<Utc>,
    /// When the run was decided on.
    pub now: chrono::DateTime<Utc>,
    /// Whether this is the first row of the target to report it. A target
    /// writing several repositories is one row per repository, but it skipped
    /// once: the report rows, the activity entry and the catch-up are written
    /// for the first, the alert for every one so a rule scoped to a
    /// repository still matches.
    pub first_for_target: bool,
}

/// Reports a scheduled run a dependency kept from starting.
///
/// Marked as not always online, the dependency's absence was expected: the
/// run is **skipped** - its report rows settled as `skipped`, a
/// [`SystemEventType::BackupSkippedDependencyOffline`] warning, the matching
/// skip alert, and a catch-up waiting for the dependency to answer. Otherwise
/// the run has **failed** - `failed` report rows, a
/// [`SystemEventType::BackupFailedDependencyOffline`] entry, and a plain
/// `backup_failed` alert that the rules people already have for failed backups
/// cover.
///
/// [`SystemEventType::BackupSkippedDependencyOffline`]:
/// shared::types::SystemEventType::BackupSkippedDependencyOffline
/// [`SystemEventType::BackupFailedDependencyOffline`]:
/// shared::types::SystemEventType::BackupFailedDependencyOffline
pub(crate) async fn report_scheduled_down(
    down: &ScheduledDown<'_>,
    dependency: &RequiredDependency,
) {
    use shared::types::{ReportStatus, SystemEventType};

    use crate::notifications::EventType;

    let reason = down_reason(dependency);
    let (status, system_event, event_type, verb) = if dependency.intermittent {
        (
            ReportStatus::Skipped,
            SystemEventType::BackupSkippedDependencyOffline,
            EventType::BackupSkippedDependencyOffline,
            "skipped",
        )
    } else {
        (
            ReportStatus::Failed,
            SystemEventType::BackupFailedDependencyOffline,
            EventType::BackupFailed,
            "failed",
        )
    };

    if down.first_for_target {
        if let Err(e) = db::dependency_hosts::settle_pending_reports_for_dependency(
            down.pool,
            down.run_id,
            down.agent_id,
            status,
            &reason,
        )
        .await
        {
            tracing::error!(
                schedule_id = down.schedule_id,
                hostname = %down.hostname,
                error = %e,
                "failed to settle the reports of a run a dependency kept from starting"
            );
        }
        let msg = format!(
            "Backup for schedule '{}' on '{}' {verb}: {reason}",
            down.schedule_name, down.hostname
        );
        if let Err(e) =
            db::insert_system_event(down.pool, system_event, Some(down.hostname), &msg).await
        {
            tracing::error!(
                schedule_id = down.schedule_id,
                error = %e,
                "failed to record the dependency-offline system event"
            );
        }
        if dependency.intermittent
            && let Err(e) = db::dependency_catch_ups::mark_dependency_catch_up(
                down.pool,
                down.schedule_id,
                down.agent_id,
                dependency.id,
                down.due_at,
            )
            .await
        {
            tracing::error!(
                schedule_id = down.schedule_id,
                hostname = %down.hostname,
                error = %e,
                "failed to record a pending dependency catch-up"
            );
        }
    }

    notify_down(down, event_type, status, reason).await;
}

/// Sends the alert for one row of a target a dependency kept from starting.
async fn notify_down(
    down: &ScheduledDown<'_>,
    event_type: crate::notifications::EventType,
    status: shared::types::ReportStatus,
    reason: String,
) {
    let repo_name = db::get_repo_name(down.pool, down.repo_id)
        .await
        .unwrap_or_default();
    let event = crate::notifications::NotificationEvent {
        event_type,
        hostname: down.hostname.to_owned(),
        repo_name,
        status: status.to_string(),
        error_message: Some(reason),
        timestamp: down.now,
        repo_id: Some(down.repo_id),
        agent_id: Some(down.agent_id),
        schedule_id: Some(down.schedule_id),
        schedule_name: Some(down.schedule_name.to_owned()),
        archive_name: None,
        run_id: Some(down.run_id.to_owned()),
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
        crate::notifications::dispatch(down.notification_service, event, down.task_registry).await
    {
        tracing::error!(
            schedule_id = down.schedule_id,
            error = %e,
            "failed to dispatch the dependency-offline notification"
        );
    }
}

/// Reports a manual or catch-up run a dependency kept from starting.
///
/// A manual run fails at once - its report rows say why, and that a manual
/// run does not wake dependencies - with no alert: whoever pressed the button
/// is looking at the answer. A catch-up run that finds a dependency marked as
/// not always online away again goes back to waiting on it, keeping the
/// occurrence it stood for; the skip it stands for was already alerted on.
pub(crate) async fn report_dispatch_down(
    pool: &sqlx::PgPool,
    origin: crate::run_dispatch::RunOrigin,
    run: DispatchedRun<'_>,
    dependency: &RequiredDependency,
) {
    use shared::types::ReportStatus;

    use crate::run_dispatch::RunOrigin;

    let reason = down_reason(dependency);
    let (status, reason) = match origin {
        RunOrigin::Manual => (
            ReportStatus::Failed,
            format!("{reason}; a manual run does not wake dependencies"),
        ),
        RunOrigin::CatchUp if dependency.intermittent => (ReportStatus::Skipped, reason),
        RunOrigin::CatchUp => (ReportStatus::Failed, reason),
    };
    if let Err(e) = db::dependency_hosts::settle_pending_reports_for_dependency(
        pool,
        run.run_id,
        run.agent_id,
        status,
        &reason,
    )
    .await
    {
        tracing::error!(
            schedule_id = run.schedule_id,
            error = %e,
            "failed to settle the reports of a run a dependency kept from starting"
        );
    }
    if matches!(origin, RunOrigin::CatchUp) && dependency.intermittent {
        wait_again(pool, run, Some(dependency.id)).await;
    }
}

/// The target of a manual or catch-up run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DispatchedRun<'a> {
    /// The schedule.
    pub schedule_id: i64,
    /// The target's agent.
    pub agent_id: i64,
    /// The run.
    pub run_id: &'a str,
    /// When the run was decided on.
    pub now: chrono::DateTime<Utc>,
}

/// Puts a target back to waiting on a dependency after a catch-up run could
/// not go ahead - its own marker handed back with the original occurrence, or
/// a new one when the run was a catch-up of another kind.
pub(crate) async fn wait_again(
    pool: &sqlx::PgPool,
    run: DispatchedRun<'_>,
    dependency_host_id: Option<i64>,
) {
    if let Err(e) = db::dependency_catch_ups::release_dependency_catch_up(
        pool,
        run.schedule_id,
        run.agent_id,
        run.run_id,
        dependency_host_id,
    )
    .await
    {
        tracing::error!(
            schedule_id = run.schedule_id,
            error = %e,
            "failed to hand a dependency catch-up back"
        );
    }
    let Some(dependency_host_id) = dependency_host_id else {
        return;
    };
    if let Err(e) = db::dependency_catch_ups::mark_dependency_catch_up_if_absent(
        pool,
        run.schedule_id,
        run.agent_id,
        dependency_host_id,
        run.now,
    )
    .await
    {
        tracing::error!(
            schedule_id = run.schedule_id,
            error = %e,
            "failed to record a pending dependency catch-up"
        );
    }
}

/// Gives back the repository host sessions a [`check`] held, once the
/// target's run is done (or never started).
pub(crate) async fn release(ctx: PowerCtx<'_>, check: &DependencyCheck, site: RunEventSite<'_>) {
    for host_id in &check.held_repo_hosts {
        power::release_repo_host_for_dependency(ctx, *host_id, site).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_listening_port_answers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = i32::from(listener.local_addr().unwrap().port());
        assert!(probe("127.0.0.1", port).await);
    }

    #[tokio::test]
    async fn a_closed_port_does_not_answer() {
        // Bind and drop to find a port nothing listens on.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = i32::from(listener.local_addr().unwrap().port());
        drop(listener);
        assert!(!probe("127.0.0.1", port).await);
    }

    #[tokio::test]
    async fn a_port_outside_the_tcp_range_never_answers() {
        assert!(!probe("127.0.0.1", 70_000).await);
        assert!(!probe("127.0.0.1", -1).await);
    }

    #[test]
    fn the_reason_names_the_dependency_and_where_it_was_asked() {
        let dependency = RequiredDependency {
            id: 1,
            name: "nas-media".to_owned(),
            address: "nas-media.lan".to_owned(),
            port: 445,
            intermittent: true,
            repo_host_id: None,
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
        };
        assert_eq!(
            down_reason(&dependency),
            "dependency 'nas-media' did not answer on port 445 (nas-media.lan)"
        );
    }
}
