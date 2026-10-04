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
    use shared::types::{AcknowledgedFilter, ReportStatus, SystemEventType};

    use super::*;
    use crate::{AppState, db::dependency_catch_ups::DependencyCatchUpFilter};

    /// A port nothing listens on, so a probe is refused at once.
    pub(super) const CLOSED_PORT: i32 = 1;

    const KEY_MATERIAL: &[u8] = b"dependencies-test-key";

    /// One schedule with one target and one repository, for a dependency to be
    /// required by.
    pub(super) struct Fixture {
        pub state: AppState,
        pub agent: db::AgentRow,
        pub repo: db::RepoRow,
        pub schedule: db::ScheduleRow,
    }

    impl Fixture {
        /// Its repository can have a config assembled for it - passphrase
        /// under the state's own key, a known SSH host key - so a run that
        /// passes its dependency check reaches a connected agent.
        pub(super) async fn new(pool: sqlx::PgPool, name: &str) -> Self {
            let agent = db::insert_agent(&pool, &format!("{name}-host"), None, "hash", None, None)
                .await
                .unwrap();
            let passphrase_encrypted = shared::crypto::encrypt_passphrase(
                "test-pass",
                &shared::crypto::derive_key(KEY_MATERIAL).unwrap(),
            )
            .unwrap();
            let repo = db::insert_repo(
                &pool,
                &db::InsertRepoParams {
                    name: &format!("{name}-repo"),
                    repo_path: "/backup/dependency",
                    ssh_user: "borg",
                    ssh_host: "127.0.0.1",
                    ssh_port: CLOSED_PORT,
                    passphrase_encrypted: &passphrase_encrypted,
                    compression: "lz4",
                    encryption: "none",
                    owner_id: None,
                    sync_schedule: None,
                },
            )
            .await
            .unwrap();
            db::update_repo_ssh_host_key(&pool, repo.id, "ssh-ed25519 AAAATEST")
                .await
                .unwrap();
            let repo = db::get_repo_by_id(&pool, repo.id).await.unwrap();
            let schedule = db::insert_schedule(
                &pool,
                repo.id,
                &db::ScheduleParams {
                    keep_yearly: 0,
                    ..db::ScheduleParams::for_test(&format!("{name}-schedule"), "0 2 * * *")
                },
                None,
            )
            .await
            .unwrap();
            db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
                .await
                .unwrap();
            let state = crate::test_support::build_test_state(pool, KEY_MATERIAL);
            Self {
                state,
                agent,
                repo,
                schedule,
            }
        }

        pub(super) fn pool(&self) -> &sqlx::PgPool {
            &self.state.pool
        }

        pub(super) fn ctx(&self) -> PowerCtx<'_> {
            PowerCtx {
                pool: &self.state.pool,
                registry: &self.state.registry,
                ui_broadcast: &self.state.ui_broadcast,
                power_sessions: &self.state.power_sessions,
            }
        }

        pub(super) fn site<'a>(&'a self, run_id: &'a str) -> RunEventSite<'a> {
            RunEventSite {
                run_id,
                agent_id: self.agent.id,
                repo_id: self.repo.id,
                hostname: &self.agent.hostname,
            }
        }

        /// A dependency on `127.0.0.1:port`, required by this target and
        /// marked as not always online when `intermittent`.
        pub(super) async fn require(
            &self,
            name: &str,
            port: i32,
            intermittent: bool,
        ) -> db::dependency_hosts::DependencyHostRow {
            let dependency = db::dependency_hosts::insert_dependency_host(
                self.pool(),
                &db::dependency_hosts::NewDependencyHost {
                    name,
                    address: "127.0.0.1",
                    port,
                    description: "",
                    repo_host_id: None,
                },
            )
            .await
            .unwrap();
            let dependency = db::dependency_hosts::update_dependency_host_availability(
                self.pool(),
                dependency.id,
                db::dependency_hosts::DependencyAvailability {
                    intermittent,
                    recheck_minutes: 15,
                    give_up_minutes: 0,
                },
            )
            .await
            .unwrap();
            let mut pairs =
                db::dependency_hosts::list_schedule_dependencies(self.pool(), self.schedule.id)
                    .await
                    .unwrap();
            pairs.push((self.agent.id, dependency.id));
            db::dependency_hosts::replace_schedule_dependencies(
                self.pool(),
                self.schedule.id,
                &pairs,
            )
            .await
            .unwrap();
            dependency
        }

        pub(super) async fn run_events(&self, run_id: &str) -> Vec<(RunEventTarget, RunEventType)> {
            db::run_events::list_run_events(self.pool(), run_id, self.agent.id, self.repo.id)
                .await
                .unwrap()
                .into_iter()
                .map(|e| (e.target, e.event_type))
                .collect()
        }

        pub(super) async fn system_events(&self) -> Vec<(SystemEventType, String)> {
            db::get_system_events(self.pool(), 50, AcknowledgedFilter::All)
                .await
                .unwrap()
                .into_iter()
                .map(|e| (e.event_type, e.message))
                .collect()
        }

        pub(super) async fn insert_pending_report(&self, run_id: &str) {
            db::insert_backup_pending(
                self.pool(),
                self.agent.id,
                self.repo.id,
                Some(self.schedule.id),
                run_id,
                Utc::now(),
            )
            .await
            .unwrap();
        }

        pub(super) async fn reports(&self) -> Vec<(ReportStatus, Option<String>)> {
            db::list_reports_for_schedule(self.pool(), self.schedule.id, 50, 0)
                .await
                .unwrap()
                .into_iter()
                .map(|r| (r.status, r.error_message))
                .collect()
        }

        pub(super) async fn markers(
            &self,
        ) -> Vec<db::dependency_catch_ups::DependencyCatchUpCandidate> {
            db::dependency_catch_ups::list_dependency_catch_up_candidates(
                self.pool(),
                DependencyCatchUpFilter::Schedule(self.schedule.id),
            )
            .await
            .unwrap()
        }

        pub(super) fn scheduled_down<'a>(
            &'a self,
            run_id: &'a str,
            due_at: chrono::DateTime<Utc>,
            first_for_target: bool,
        ) -> ScheduledDown<'a> {
            ScheduledDown {
                pool: self.pool(),
                notification_service: &self.state.notification_service,
                task_registry: &self.state.task_registry,
                schedule_id: self.schedule.id,
                schedule_name: &self.schedule.name,
                agent_id: self.agent.id,
                hostname: &self.agent.hostname,
                repo_id: self.repo.id,
                run_id,
                due_at,
                now: Utc::now(),
                first_for_target,
            }
        }

        pub(super) fn dispatched_run<'a>(&self, run_id: &'a str) -> DispatchedRun<'a> {
            DispatchedRun {
                schedule_id: self.schedule.id,
                agent_id: self.agent.id,
                run_id,
                now: Utc::now(),
            }
        }
    }

    async fn required(pool: &sqlx::PgPool, id: i64) -> RequiredDependency {
        db::dependency_hosts::get_required_dependency(pool, id)
            .await
            .unwrap()
    }

    async fn last_check(pool: &sqlx::PgPool, id: i64) -> Option<bool> {
        db::dependency_hosts::get_dependency_host(pool, id)
            .await
            .unwrap()
            .last_check_reachable
    }

    /// A port that was free a moment ago, for a listener a test opens later.
    /// A port that refuses connections until the returned socket listens.
    /// Bound but not listening, it stays this test's for its whole length,
    /// so nothing else can take it between the refusal and the answer.
    fn refusing_port() -> (tokio::net::TcpSocket, u16) {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        (socket, port)
    }

    /// Dependencies are checked in name order and the first that does not
    /// answer decides the run: the ones after it are not asked at all, and
    /// the one that was is on the run's timeline and remembered as down.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_check_stops_at_the_first_dependency_that_does_not_answer(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "check-stops").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = i32::from(listener.local_addr().unwrap().port());
        let up = fx.require("a-up", open_port, false).await;
        let down = fx.require("b-down", CLOSED_PORT, false).await;
        let never_asked = fx.require("c-later", open_port, false).await;

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-check-stops"),
            WakePolicy::CheckOnly,
        )
        .await;

        assert_eq!(outcome.down.map(|d| d.id), Some(down.id));
        assert!(
            outcome.held_repo_hosts.is_empty(),
            "no repository host is involved"
        );
        assert_eq!(last_check(fx.pool(), up.id).await, Some(true));
        assert_eq!(last_check(fx.pool(), down.id).await, Some(false));
        assert_eq!(
            last_check(fx.pool(), never_asked.id).await,
            None,
            "a dependency after the one that decided the run must not be asked"
        );
        assert_eq!(
            fx.run_events("run-check-stops").await,
            vec![
                (RunEventTarget::Dependency, RunEventType::ReachabilityCheck),
                (RunEventTarget::Dependency, RunEventType::HostUnreachable),
            ]
        );
    }

    /// A target with nothing to check passes without a timeline entry.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_target_without_dependencies_passes_the_check(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "check-none").await;

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-check-none"),
            WakePolicy::Wake(ScheduleWakeOverride::Enabled),
        )
        .await;

        assert!(outcome.down.is_none());
        assert!(
            fx.run_events("run-check-none").await.is_empty(),
            "nothing was checked, so nothing is on the timeline"
        );
    }

    /// A dependency that is the same machine as a repository host holds that
    /// host's power session while the run needs it, and gives it back on
    /// release - so a repository run finishing first cannot shut it down.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_dependency_on_a_repository_host_holds_its_session_until_released(
        pool: sqlx::PgPool,
    ) {
        let fx = Fixture::new(pool, "check-holds").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = i32::from(listener.local_addr().unwrap().port());
        let dependency = fx.require("nas-shared", open_port, false).await;
        db::dependency_hosts::update_dependency_host_power(
            fx.pool(),
            dependency.id,
            &db::dependency_hosts::DependencyPowerPatch {
                repo_host_id: Some(fx.repo.repo_host_id),
                wake_enabled: false,
                wake_mac_address: None,
                wake_broadcast_address: None,
                wake_timeout_seconds: 1,
            },
        )
        .await
        .unwrap();
        let site = fx.site("run-check-holds");

        let outcome = check(fx.ctx(), fx.schedule.id, site, WakePolicy::CheckOnly).await;
        assert!(outcome.down.is_none());
        assert_eq!(outcome.held_repo_hosts, vec![fx.repo.repo_host_id]);

        // A sibling run on the same host, so the hold is visible: ending it
        // while the check still holds the host leaves the session open.
        let key = PowerHostKey::RepoHost(fx.repo.repo_host_id);
        fx.state.power_sessions.reserve(key).await;
        assert_eq!(fx.state.power_sessions.end(key).await, None);

        release(fx.ctx(), &outcome, site).await;
        assert_eq!(
            fx.state.power_sessions.end(key).await,
            None,
            "release must have ended the check's hold, leaving no session"
        );
        assert!(
            fx.run_events("run-check-holds").await.is_empty(),
            "a host that was not woken is not shut down"
        );
    }

    /// A dependency that does not answer and has no MAC address cannot be
    /// woken even when the schedule asks for it; the timeline says so.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_dependency_without_a_mac_address_cannot_be_woken(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "wake-no-mac").await;
        let down = fx.require("nas-no-mac", CLOSED_PORT, false).await;

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-wake-no-mac"),
            WakePolicy::Wake(ScheduleWakeOverride::Enabled),
        )
        .await;

        assert_eq!(outcome.down.map(|d| d.id), Some(down.id));
        assert_eq!(
            fx.run_events("run-wake-no-mac").await,
            vec![
                (RunEventTarget::Dependency, RunEventType::ReachabilityCheck),
                (RunEventTarget::Dependency, RunEventType::WakeUnavailable),
                (RunEventTarget::Dependency, RunEventType::HostUnreachable),
            ]
        );
    }

    /// The schedule's override can switch waking off for a dependency whose
    /// own setting has it on: nothing is sent.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_schedule_that_disables_waking_does_not_wake_a_dependency(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "wake-disabled").await;
        let down = fx.require("nas-wake-off", CLOSED_PORT, false).await;
        db::dependency_hosts::update_dependency_host_power(
            fx.pool(),
            down.id,
            &db::dependency_hosts::DependencyPowerPatch {
                repo_host_id: None,
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: Some("127.0.0.1"),
                wake_timeout_seconds: 1,
            },
        )
        .await
        .unwrap();

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-wake-disabled"),
            WakePolicy::Wake(ScheduleWakeOverride::Disabled),
        )
        .await;

        assert_eq!(outcome.down.map(|d| d.id), Some(down.id));
        assert!(
            !fx.run_events("run-wake-disabled")
                .await
                .contains(&(RunEventTarget::Dependency, RunEventType::WakeSent))
        );
    }

    /// Woken and still silent once its wake window is out, a dependency
    /// decides the run - and the failed wait is remembered as a check.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_dependency_that_stays_silent_after_a_wake_decides_the_run(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "wake-silent").await;
        let down = fx.require("nas-silent", CLOSED_PORT, false).await;
        db::dependency_hosts::update_dependency_host_power(
            fx.pool(),
            down.id,
            &db::dependency_hosts::DependencyPowerPatch {
                repo_host_id: None,
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: Some("127.0.0.1"),
                wake_timeout_seconds: 1,
            },
        )
        .await
        .unwrap();

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-wake-silent"),
            WakePolicy::Wake(ScheduleWakeOverride::HostDefault),
        )
        .await;

        assert_eq!(outcome.down.map(|d| d.id), Some(down.id));
        assert_eq!(last_check(fx.pool(), down.id).await, Some(false));
        assert_eq!(
            fx.run_events("run-wake-silent").await,
            vec![
                (RunEventTarget::Dependency, RunEventType::ReachabilityCheck),
                (RunEventTarget::Dependency, RunEventType::WakeSent),
                (RunEventTarget::Dependency, RunEventType::HostUnreachable),
            ]
        );
    }

    /// A dependency sharing a repository host's machine is woken with that
    /// host's settings, the wake is credited to the host's session so its own
    /// shut-down setting applies later, and a dependency that comes up within
    /// the window lets the run go ahead.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_dependency_woken_through_its_repository_host_lets_the_run_go_ahead(
        pool: sqlx::PgPool,
    ) {
        let fx = Fixture::new(pool, "wake-up").await;
        let (socket, port) = refusing_port();
        let dependency = fx.require("nas-wakes", i32::from(port), false).await;
        db::repo_hosts::update_repo_host_power(
            fx.pool(),
            fx.repo.repo_host_id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: Some("127.0.0.1"),
                wake_timeout_seconds: 10,
                shutdown_after_backup: false,
            },
        )
        .await
        .unwrap();
        db::dependency_hosts::update_dependency_host_power(
            fx.pool(),
            dependency.id,
            &db::dependency_hosts::DependencyPowerPatch {
                repo_host_id: Some(fx.repo.repo_host_id),
                wake_enabled: false,
                wake_mac_address: None,
                wake_broadcast_address: None,
                wake_timeout_seconds: 1,
            },
        )
        .await
        .unwrap();
        // The machine comes up shortly after the packet: well after the
        // first probe was refused, well inside the wake window.
        let comes_up = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            socket.listen(16).unwrap()
        });

        let outcome = check(
            fx.ctx(),
            fx.schedule.id,
            fx.site("run-wake-up"),
            WakePolicy::Wake(ScheduleWakeOverride::HostDefault),
        )
        .await;
        let _listener = comes_up.await.unwrap();

        assert!(outcome.down.is_none());
        assert_eq!(last_check(fx.pool(), dependency.id).await, Some(true));
        assert_eq!(
            fx.run_events("run-wake-up").await,
            vec![
                (RunEventTarget::Dependency, RunEventType::ReachabilityCheck),
                (RunEventTarget::Dependency, RunEventType::WakeSent),
                (RunEventTarget::Dependency, RunEventType::HostOnline),
            ]
        );
        assert_eq!(
            fx.state
                .power_sessions
                .end(PowerHostKey::RepoHost(fx.repo.repo_host_id))
                .await,
            Some((true, false)),
            "the wake must be credited to the repository host's session"
        );
    }

    /// The last hold on a repository host this session woke shuts it down
    /// when the host asks for that; the attempt is on the run's timeline.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn releasing_the_last_hold_on_a_woken_host_shuts_it_down(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "release-shutdown").await;
        db::repo_hosts::update_repo_host_power(
            fx.pool(),
            fx.repo.repo_host_id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 10,
                shutdown_after_backup: true,
            },
        )
        .await
        .unwrap();
        let key = PowerHostKey::RepoHost(fx.repo.repo_host_id);
        let held = DependencyCheck {
            down: None,
            held_repo_hosts: vec![fx.repo.repo_host_id],
        };

        // Not woken this session: released without a shut-down.
        fx.state.power_sessions.reserve(key).await;
        release(fx.ctx(), &held, fx.site("run-release-not-woken")).await;
        assert!(
            fx.run_events("run-release-not-woken").await.is_empty(),
            "a host this session did not wake is not shut down"
        );

        fx.state.power_sessions.reserve(key).await;
        fx.state
            .power_sessions
            .record_outcome(key, true, false)
            .await;
        release(fx.ctx(), &held, fx.site("run-release-shutdown")).await;
        assert_eq!(
            fx.run_events("run-release-shutdown").await.first(),
            Some(&(RunEventTarget::Repository, RunEventType::ShutdownSent))
        );
        assert_eq!(fx.state.power_sessions.end(key).await, None);
    }

    /// A woken host whose own setting keeps it running is left alone.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn releasing_a_woken_host_that_stays_up_does_not_shut_it_down(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "release-stays-up").await;
        let key = PowerHostKey::RepoHost(fx.repo.repo_host_id);
        fx.state.power_sessions.reserve(key).await;
        fx.state
            .power_sessions
            .record_outcome(key, true, false)
            .await;

        power::release_repo_host_for_dependency(
            fx.ctx(),
            fx.repo.repo_host_id,
            fx.site("run-release-stays-up"),
        )
        .await;

        assert!(
            fx.run_events("run-release-stays-up").await.is_empty(),
            "a host set to stay up is not shut down"
        );
        assert_eq!(fx.state.power_sessions.end(key).await, None);
    }

    /// A scheduled run kept from starting by a dependency marked as not
    /// always online is skipped: its reports say why, the activity log has a
    /// warning, and a catch-up waits for the dependency from the occurrence.
    /// Only the first row of the target writes those; the others only alert.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scheduled_run_an_intermittent_dependency_kept_from_starting_is_skipped(
        pool: sqlx::PgPool,
    ) {
        let fx = Fixture::new(pool, "scheduled-skip").await;
        let dependency = fx.require("nas-away", CLOSED_PORT, true).await;
        let dependency = required(fx.pool(), dependency.id).await;
        fx.insert_pending_report("run-scheduled-skip").await;
        let due_at = Utc::now()
            .checked_sub_signed(chrono::TimeDelta::minutes(5))
            .unwrap();

        report_scheduled_down(
            &fx.scheduled_down("run-scheduled-skip", due_at, false),
            &dependency,
        )
        .await;
        assert_eq!(fx.reports().await, vec![(ReportStatus::Pending, None)]);
        assert!(
            fx.markers().await.is_empty(),
            "nothing may wait to be caught up"
        );

        report_scheduled_down(
            &fx.scheduled_down("run-scheduled-skip", due_at, true),
            &dependency,
        )
        .await;

        assert_eq!(
            fx.reports().await,
            vec![(ReportStatus::Skipped, Some(down_reason(&dependency)))]
        );
        let events = fx.system_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events.first().map(|e| e.0),
            Some(SystemEventType::BackupSkippedDependencyOffline)
        );
        assert!(events.first().is_some_and(|e| e.1.contains("skipped")));
        let markers = fx.markers().await;
        assert_eq!(markers.len(), 1);
        let marker = markers.first().unwrap();
        assert_eq!(marker.dependency_host_id, dependency.id);
        assert_eq!(marker.pending_for.timestamp(), due_at.timestamp());
        assert_eq!(marker.dispatched_run_id, None);
    }

    /// One that should always be there and does not answer fails the run, and
    /// nothing waits for it to come back.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_scheduled_run_an_always_online_dependency_kept_from_starting_fails(
        pool: sqlx::PgPool,
    ) {
        let fx = Fixture::new(pool, "scheduled-fail").await;
        let dependency = fx.require("files-01", CLOSED_PORT, false).await;
        let dependency = required(fx.pool(), dependency.id).await;
        fx.insert_pending_report("run-scheduled-fail").await;

        report_scheduled_down(
            &fx.scheduled_down("run-scheduled-fail", Utc::now(), true),
            &dependency,
        )
        .await;

        assert_eq!(
            fx.reports().await,
            vec![(ReportStatus::Failed, Some(down_reason(&dependency)))]
        );
        let events = fx.system_events().await;
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            vec![SystemEventType::BackupFailedDependencyOffline]
        );
        assert!(events.first().is_some_and(|e| e.1.contains("failed")));
        assert!(
            fx.markers().await.is_empty(),
            "nothing may wait to be caught up"
        );
    }

    /// A manual run fails at once and says that it does not wake dependencies;
    /// nothing is left waiting.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_manual_run_a_dependency_kept_from_starting_fails(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "dispatch-manual").await;
        let dependency = fx.require("nas-manual", CLOSED_PORT, true).await;
        let dependency = required(fx.pool(), dependency.id).await;
        fx.insert_pending_report("run-dispatch-manual").await;

        report_dispatch_down(
            fx.pool(),
            crate::run_dispatch::RunOrigin::Manual,
            fx.dispatched_run("run-dispatch-manual"),
            &dependency,
        )
        .await;

        assert_eq!(
            fx.reports().await,
            vec![(
                ReportStatus::Failed,
                Some(format!(
                    "{}; a manual run does not wake dependencies",
                    down_reason(&dependency)
                ))
            )]
        );
        assert!(
            fx.markers().await.is_empty(),
            "nothing may wait to be caught up"
        );
        assert!(
            fx.system_events().await.is_empty(),
            "whoever pressed the button already has the answer"
        );
    }

    /// A catch-up that finds an intermittent dependency away again is skipped
    /// and hands its marker back, now waiting on that dependency, with the
    /// original occurrence kept.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_catch_up_that_finds_a_dependency_away_again_waits_on_it_again(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "dispatch-catch-up").await;
        let first = fx.require("nas-first", CLOSED_PORT, true).await;
        let again = fx.require("nas-again", CLOSED_PORT, true).await;
        let again = required(fx.pool(), again.id).await;
        let pending_for = Utc::now()
            .checked_sub_signed(chrono::TimeDelta::hours(3))
            .unwrap();
        db::dependency_catch_ups::mark_dependency_catch_up(
            fx.pool(),
            fx.schedule.id,
            fx.agent.id,
            first.id,
            pending_for,
        )
        .await
        .unwrap();
        assert!(
            db::dependency_catch_ups::hand_off_dependency_catch_up(
                fx.pool(),
                fx.schedule.id,
                fx.agent.id,
                "run-dispatch-catch-up",
            )
            .await
            .unwrap()
        );
        fx.insert_pending_report("run-dispatch-catch-up").await;

        report_dispatch_down(
            fx.pool(),
            crate::run_dispatch::RunOrigin::CatchUp,
            fx.dispatched_run("run-dispatch-catch-up"),
            &again,
        )
        .await;

        assert_eq!(
            fx.reports().await,
            vec![(ReportStatus::Skipped, Some(down_reason(&again)))]
        );
        let markers = fx.markers().await;
        assert_eq!(markers.len(), 1);
        let marker = markers.first().unwrap();
        assert_eq!(marker.dependency_host_id, again.id);
        assert_eq!(marker.dispatched_run_id, None);
        assert_eq!(marker.pending_for.timestamp(), pending_for.timestamp());
    }

    /// A catch-up kept from starting by a dependency that should always be
    /// there fails, and the marker it was handed stays with it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_catch_up_kept_from_starting_by_an_always_online_dependency_fails(
        pool: sqlx::PgPool,
    ) {
        let fx = Fixture::new(pool, "dispatch-catch-up-fail").await;
        let dependency = fx.require("files-02", CLOSED_PORT, false).await;
        let dependency = required(fx.pool(), dependency.id).await;
        fx.insert_pending_report("run-dispatch-catch-up-fail").await;

        report_dispatch_down(
            fx.pool(),
            crate::run_dispatch::RunOrigin::CatchUp,
            fx.dispatched_run("run-dispatch-catch-up-fail"),
            &dependency,
        )
        .await;

        assert_eq!(
            fx.reports().await,
            vec![(ReportStatus::Failed, Some(down_reason(&dependency)))]
        );
        assert!(
            fx.markers().await.is_empty(),
            "nothing may wait to be caught up"
        );
    }

    /// A catch-up of another kind that finds a dependency away starts a wait
    /// on it from now; one that only hands its marker back starts nothing.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn waiting_again_without_a_marker_starts_one_only_for_a_dependency(pool: sqlx::PgPool) {
        let fx = Fixture::new(pool, "wait-again").await;
        let dependency = fx.require("nas-wait-again", CLOSED_PORT, true).await;
        let run = fx.dispatched_run("run-wait-again");

        wait_again(fx.pool(), run, None).await;
        assert!(
            fx.markers().await.is_empty(),
            "nothing may wait to be caught up"
        );

        wait_again(fx.pool(), run, Some(dependency.id)).await;
        let markers = fx.markers().await;
        assert_eq!(markers.len(), 1);
        let marker = markers.first().unwrap();
        assert_eq!(marker.dependency_host_id, dependency.id);
        assert_eq!(marker.pending_for.timestamp(), run.now.timestamp());
    }

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
