// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Releasing the holds a previous server process left behind.
//!
//! A run's holds are normally released by the task that awaits the run. When
//! the server restarts mid-run that task is gone, but the agent's borg run
//! carries on and settles its backup report once it reconnects.
//! [`PowerSessionTracker::recover`](super::PowerSessionTracker::recover)
//! counts those holds as held again at startup; this releases each one the
//! way its run would have, once the run is done.

use std::time::Duration;

use chrono::Utc;
use shared::types::SystemEventType;
use tracing::warn;

use super::{
    PowerCtx, PowerHostKey, release_dependency_hold, teardown_agent_power, teardown_repo_power,
};
use crate::db::{
    self,
    power_holds::{self, PowerHoldRow},
};

/// How often orphaned holds are checked by default.
const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// How long after it was taken an orphaned hold whose run has still not
/// reported back is given up on. Its machine is then left as it is: a run
/// that never reports back may still be using it, and shutting a machine
/// down under a backup is worse than leaving it on.
pub(crate) const ABANDON_AFTER: Duration = Duration::from_hours(24);

/// How often [`release_orphaned_holds`] runs. Overridable via
/// `SCHEDULER_POWER_HOLD_SWEEP_INTERVAL_SECS`, like the scheduler's other
/// intervals.
#[must_use]
pub fn power_hold_sweep_interval() -> Duration {
    crate::scheduler::duration_from_env_secs(
        "SCHEDULER_POWER_HOLD_SWEEP_INTERVAL_SECS",
        DEFAULT_SWEEP_INTERVAL,
    )
}

/// What becomes of one orphaned hold on this pass.
enum Fate {
    /// Its run is still going: kept for a later pass.
    Waiting,
    /// Its run is done (or never got as far as a report): released the way
    /// the run would have released it.
    Settled,
    /// Its run has not reported back for [`ABANDON_AFTER`]: dropped without
    /// touching the machine.
    Abandoned,
}

/// Releases every orphaned hold whose run is done, and gives up on those
/// whose run has not reported back for [`ABANDON_AFTER`]. A hold whose run
/// cannot be looked up right now is kept for the next pass.
pub async fn release_orphaned_holds(ctx: PowerCtx<'_>) {
    let mut kept = Vec::new();
    for orphan in ctx.power_sessions.take_orphans().await {
        match fate(ctx.pool, &orphan).await {
            Some(Fate::Settled) => release_settled(ctx, &orphan).await,
            Some(Fate::Abandoned) => abandon(ctx, &orphan).await,
            Some(Fate::Waiting) | None => kept.push(orphan),
        }
    }
    ctx.power_sessions.keep_orphans(kept).await;
}

async fn fate(pool: &sqlx::PgPool, orphan: &PowerHoldRow) -> Option<Fate> {
    let in_flight = match power_holds::hold_run_in_flight(pool, orphan.owner()).await {
        Ok(in_flight) => in_flight,
        Err(e) => {
            warn!(run_id = %orphan.run_id, error = %e, "failed to check on the run of an orphaned power hold");
            return None;
        }
    };
    if !in_flight {
        return Some(Fate::Settled);
    }
    let overdue = Utc::now()
        .signed_duration_since(orphan.created_at)
        .to_std()
        .is_ok_and(|age| age >= ABANDON_AFTER);
    Some(if overdue {
        Fate::Abandoned
    } else {
        Fate::Waiting
    })
}

/// Releases `orphan` through the same teardown its run would have gone
/// through, on fresh agent and repository rows - so the machine is shut down
/// (or its agent stopped) only if this was the last hold, the session woke
/// (or started) it, and the machine's current settings ask for it.
async fn release_settled(ctx: PowerCtx<'_>, orphan: &PowerHoldRow) {
    let owner = orphan.owner();
    let agent = match db::get_agent_by_id(ctx.pool, owner.agent_id).await {
        Ok(agent) => agent,
        Err(e) => {
            warn!(agent_id = owner.agent_id, error = %e, "failed to load the agent of an orphaned power hold");
            release_bookkeeping(ctx, orphan).await;
            return;
        }
    };
    match (orphan.key, owner.repo_id) {
        (PowerHostKey::Agent(_), Some(repo_id)) => {
            teardown_agent_power(ctx, &agent, repo_id, owner.run_id).await;
        }
        (PowerHostKey::RepoHost(host_id), Some(repo_id)) => {
            match db::get_repo_by_id(ctx.pool, repo_id).await {
                Ok(repo) => {
                    teardown_repo_power(
                        ctx,
                        &repo,
                        host_id,
                        owner.agent_id,
                        owner.run_id,
                        &agent.hostname,
                    )
                    .await;
                }
                Err(e) => {
                    warn!(repo_id, error = %e, "failed to load the repository of an orphaned power hold");
                    release_bookkeeping(ctx, orphan).await;
                }
            }
        }
        (PowerHostKey::RepoHost(host_id), None) => {
            release_dependency_hold(ctx, host_id, owner, None, &agent.hostname).await;
        }
        // An agent's own host is only ever held by a target with a
        // repository; there is no teardown to replay for anything else.
        (PowerHostKey::Agent(_), None) => {
            release_bookkeeping(ctx, orphan).await;
        }
    }
}

/// Ends `orphan`'s hold without touching its machine.
async fn release_bookkeeping(ctx: PowerCtx<'_>, orphan: &PowerHoldRow) -> Option<(bool, bool)> {
    ctx.power_sessions.end(orphan.key, orphan.owner()).await
}

/// Gives up on `orphan`: ends its hold without touching its machine, and,
/// when that skipped a teardown the session had earned, says so in the
/// Activity Log - the machine may still be on.
async fn abandon(ctx: PowerCtx<'_>, orphan: &PowerHoldRow) {
    let Some((woke, started_agent)) = release_bookkeeping(ctx, orphan).await else {
        return;
    };
    if !woke && !started_agent {
        return;
    }
    let machine = machine_name(ctx.pool, orphan.key).await;
    warn!(
        run_id = %orphan.run_id,
        machine = machine.as_deref().unwrap_or("unknown"),
        "run never reported back after a server restart; leaving its machine as it is"
    );
    let message = format!(
        "Run {} never reported back after the server restarted, so {} was not shut down or \
         stopped after the backup. It may still be on.",
        orphan.run_id,
        machine.as_deref().unwrap_or("its machine"),
    );
    if let Err(e) = db::insert_system_event(
        ctx.pool,
        SystemEventType::PowerReleaseAbandoned,
        machine.as_deref(),
        &message,
    )
    .await
    {
        warn!(error = %e, "failed to record an abandoned power release");
    }
}

/// The name a machine goes by in the Activity Log: an agent's hostname, or a
/// repository host's address.
async fn machine_name(pool: &sqlx::PgPool, key: PowerHostKey) -> Option<String> {
    match key {
        PowerHostKey::Agent(agent_id) => db::get_agent_by_id(pool, agent_id)
            .await
            .ok()
            .map(|agent| agent.hostname),
        PowerHostKey::RepoHost(repo_host_id) => db::repo_hosts::get_repo_host(pool, repo_host_id)
            .await
            .ok()
            .map(|host| host.ssh_host),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_runs_every_minute_by_default() {
        assert_eq!(power_hold_sweep_interval(), DEFAULT_SWEEP_INTERVAL);
        assert_eq!(DEFAULT_SWEEP_INTERVAL, Duration::from_mins(1));
        assert_eq!(ABANDON_AFTER, Duration::from_hours(24));
    }
}
