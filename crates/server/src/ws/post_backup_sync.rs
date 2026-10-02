// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Reconciles a repository's archive records with borg once a backup run
//! has finished, whatever its outcome: a run that failed after `borg create`
//! (in prune, compact, or a post-backup hook) has still changed the
//! repository, and its own `borg prune` removes archives on every run.

use shared::{protocol::ServerToUi, types::BackupReport};

use crate::{
    AppState,
    api::repos::{clear_import_progress_state, sync_new_archives},
    db,
    power::{self, PowerHostKey},
};

/// The finished run a post-backup sync follows.
#[derive(Debug, Clone)]
pub struct FinishedRun {
    /// Repository the run wrote to.
    pub(crate) repo_id: i64,
    /// Agent that ran it.
    pub(crate) agent_id: i64,
    /// Hostname of that agent, for run events.
    pub(crate) hostname: String,
    /// Archive the run reported creating, if it got that far.
    pub(crate) archive_name: Option<String>,
    /// The repository host held up for the sync, if any.
    pub(crate) host_hold: Option<RepoHostHold>,
}

/// A reservation on the repository host's power session, taken for the sync
/// before the run's completion is announced. The run's own teardown waits
/// for that announcement and then releases its reservation; without this one
/// it would shut a woken host down before the sync, which queues behind the
/// run on the repository lock, has listed it.
#[derive(Debug, Clone)]
pub(crate) struct RepoHostHold {
    repo_host_id: i64,
    run_id: String,
}

/// Captures what the sync needs from a run's report and holds its repository
/// host up for it. Must be called before the run's completion is published.
pub async fn prepare(
    state: &AppState,
    agent_id: i64,
    hostname: &str,
    report: &BackupReport,
) -> FinishedRun {
    let repo_id = report.repo_id.0;
    FinishedRun {
        repo_id,
        agent_id,
        hostname: hostname.to_owned(),
        archive_name: report.archive_name.clone(),
        host_hold: hold_repo_host(state, repo_id, report.run_id.as_deref()).await,
    }
}

/// Reserves the repository host for the sync. Only a run with a `run_id` is
/// one the server dispatched (and so may have woken the host); without one
/// there is no session to keep open and no run to record a shutdown against.
async fn hold_repo_host(
    state: &AppState,
    repo_id: i64,
    run_id: Option<&str>,
) -> Option<RepoHostHold> {
    let run_id = run_id?;
    let repo = match db::get_repo_by_id(&state.pool, repo_id).await {
        Ok(repo) => repo,
        Err(e) => {
            tracing::warn!(
                repo_id,
                error = %e,
                "post-backup sync: failed to load repo to hold its host"
            );
            return None;
        }
    };
    state
        .power_sessions
        .reserve(PowerHostKey::RepoHost(repo.repo_host_id))
        .await;
    Some(RepoHostHold {
        repo_host_id: repo.repo_host_id,
        run_id: run_id.to_owned(),
    })
}

/// Spawns the sync in the background, so the completion that triggered it is
/// never held up by borg. Tracked here rather than inside [`run`]: calling an
/// async fn runs none of it, so a guard claimed in its body would only count
/// once the runtime first polled the task, which is the race the tracker
/// exists to close.
pub fn spawn(state: &AppState, finished: FinishedRun) {
    state
        .background_task_tracker
        .spawn_tracked(run(state.clone(), finished));
}

/// Syncs under the repository lock, so it queues behind the run it follows
/// and anything else touching the repository instead of contending for
/// borg's own lock, then releases the host hold - still under the lock, as
/// the run's own teardown is, so nothing starts on a host about to go down.
async fn run(state: AppState, finished: FinishedRun) {
    let _repo_guard = state.repo_lock.acquire(finished.repo_id).await;
    sync(&state, finished.repo_id, finished.archive_name.as_deref()).await;
    release_host_hold(&state, &finished).await;
}

/// Marks the repository as importing, syncs it, and clears importing/error
/// state regardless of outcome.
async fn sync(state: &AppState, repo_id: i64, just_written: Option<&str>) {
    let pool = &state.pool;
    let ui_broadcast = &state.ui_broadcast;
    // Held for the rest of this function so a panic inside sync_new_archives
    // still clears repo_import_state.importing (via spawned cleanup, since
    // Drop can't await) instead of leaving it permanently "importing" - see
    // db::ImportingGuard.
    let importing_guard =
        match db::ImportingGuard::acquire(pool, repo_id, state.task_registry.clone()).await {
            Ok(guard) => guard,
            Err(e) => {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "post-backup sync: failed to set importing flag"
                );
                return;
            }
        };
    let import_error = match sync_new_archives(state, repo_id, just_written).await {
        Ok((added, removed)) => {
            if let Err(e) = db::update_repo_last_synced(pool, repo_id).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "post-backup sync: failed to update last_synced_at"
                );
            }
            tracing::debug!(repo_id, added, removed, "post-backup sync completed");
            None
        }
        Err(e) => {
            tracing::error!(repo_id, error = %e, "post-backup sync failed");
            Some(e.to_string())
        }
    };
    importing_guard.clear_now().await;
    if let Err(e) = db::set_repo_import_error(pool, repo_id, import_error.as_deref()).await {
        tracing::error!(
            repo_id,
            error = %e,
            "post-backup sync: failed to record import_error"
        );
    }
    clear_import_progress_state(pool, ui_broadcast, repo_id).await;
    ui_broadcast.send(ServerToUi::DataChanged);
}

/// Ends the sync's part in the repository host's power session, shutting the
/// host down if the sync was the last participant of a session that woke it -
/// the teardown the run itself would otherwise have done.
async fn release_host_hold(state: &AppState, finished: &FinishedRun) {
    let Some(hold) = &finished.host_hold else {
        return;
    };
    let key = PowerHostKey::RepoHost(hold.repo_host_id);
    let repo = match db::get_repo_by_id(&state.pool, finished.repo_id).await {
        Ok(repo) => repo,
        Err(e) => {
            tracing::warn!(
                repo_id = finished.repo_id,
                error = %e,
                "post-backup sync: failed to load repo for power teardown"
            );
            // Releasing the reservation alone still matters: a count that
            // never returns to zero disables shutdown for this host for good.
            state.power_sessions.end(key).await;
            return;
        }
    };
    power::teardown_repo_power(
        power::PowerCtx {
            pool: &state.pool,
            registry: &state.registry,
            ui_broadcast: &state.ui_broadcast,
            power_sessions: &state.power_sessions,
        },
        &repo,
        hold.repo_host_id,
        finished.agent_id,
        &hold.run_id,
        &finished.hostname,
    )
    .await;
}
