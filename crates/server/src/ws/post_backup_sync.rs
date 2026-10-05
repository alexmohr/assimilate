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

/// What the sync needs after a run the agent aborted. A cancellation carries
/// no report, so there is no archive name to protect and no `run_id` to hold
/// the repository host against: like a report without a `run_id`, the sync
/// takes no host hold. It still runs, since an archive written before the
/// abort, or one a prune removed, has changed the repository all the same.
#[must_use]
pub fn cancelled(agent_id: i64, hostname: &str, repo_id: i64) -> FinishedRun {
    FinishedRun {
        repo_id,
        agent_id,
        hostname: hostname.to_owned(),
        archive_name: None,
        host_hold: None,
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
///
/// The hold is armed before the lock is awaited, so a panic anywhere after
/// this point (or the task being dropped while it queues) still releases it.
async fn run(state: AppState, finished: FinishedRun) {
    let FinishedRun {
        repo_id,
        agent_id,
        hostname,
        archive_name,
        host_hold,
    } = finished;
    let host_hold = HostHoldGuard::new(
        state.clone(),
        host_hold.map(|hold| HeldHost {
            repo_id,
            agent_id,
            hostname,
            hold,
        }),
    );
    let repo_guard = state.repo_lock.acquire(repo_id).await;
    sync(&state, repo_id, archive_name.as_deref()).await;
    host_hold.release_now().await;
    drop(repo_guard);
}

/// A [`RepoHostHold`] together with what its teardown records against.
#[derive(Debug)]
struct HeldHost {
    repo_id: i64,
    agent_id: i64,
    hostname: String,
    hold: RepoHostHold,
}

/// Releases a [`RepoHostHold`] exactly once, whether the sync returns or
/// unwinds. [`PowerSessionTracker`](power::PowerSessionTracker) is in memory,
/// so a reservation that is never ended keeps the host's count above zero and
/// silently disables "shut down after backup" for it until the server
/// restarts.
///
/// `Drop` can't await, so the unwind path spawns the release, registered with
/// the task registry (so shutdown joins it) and counted by the background task
/// tracker - the same pattern as [`db::ImportingGuard`]. The spawned release
/// takes the repository lock first, as the normal path releases under it.
/// [`Self::release_now`] disarms that, so the release never runs twice.
struct HostHoldGuard {
    state: AppState,
    held: Option<HeldHost>,
}

impl HostHoldGuard {
    fn new(state: AppState, held: Option<HeldHost>) -> Self {
        Self { state, held }
    }

    /// Releases the hold now, awaiting the teardown instead of leaving it to
    /// the deferred `Drop` release.
    async fn release_now(mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        release_host_hold(&self.state, &held).await;
    }
}

impl Drop for HostHoldGuard {
    fn drop(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                repo_id = held.repo_id,
                "post-backup sync: no runtime to release the repository host hold on"
            );
            return;
        };
        let state = self.state.clone();
        let in_flight = state.background_task_tracker.begin();
        let handle = runtime.spawn(async move {
            let _in_flight = in_flight;
            let _repo_guard = state.repo_lock.acquire(held.repo_id).await;
            release_host_hold(&state, &held).await;
        });
        self.state.task_registry.register(handle);
    }
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
async fn release_host_hold(state: &AppState, held: &HeldHost) {
    let hold = &held.hold;
    let key = PowerHostKey::RepoHost(hold.repo_host_id);
    let repo = match db::get_repo_by_id(&state.pool, held.repo_id).await {
        Ok(repo) => repo,
        Err(e) => {
            tracing::warn!(
                repo_id = held.repo_id,
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
        held.agent_id,
        &hold.run_id,
        &held.hostname,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::PgPool;

    use super::*;

    const REPO_ID: i64 = 888_881;
    const REPO_HOST_ID: i64 = 777_771;
    const HOST: PowerHostKey = PowerHostKey::RepoHost(REPO_HOST_ID);
    const KEY_MATERIAL: &[u8] = b"post-backup-sync-hold-test-key";

    /// The repository lookup in the release fails on this pool, which drives
    /// the release down its reservation-only fallback without `DATABASE_URL`.
    /// The short acquire timeout makes that lookup fail fast: with sqlx's
    /// default of 30 s it would outlast the tests' wait for the task registry
    /// to drain.
    fn unreachable_state() -> AppState {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(200))
            .connect_lazy("postgres://localhost/nonexistent_test_db")
            .unwrap();
        crate::test_support::build_test_state(pool, KEY_MATERIAL)
    }

    /// A held host as `prepare` leaves it: the run's own reservation and the
    /// sync's, both on the same repository host.
    async fn hold_alongside_the_run(state: &AppState) -> HostHoldGuard {
        state.power_sessions.reserve(HOST).await;
        state.power_sessions.reserve(HOST).await;
        HostHoldGuard::new(
            state.clone(),
            Some(HeldHost {
                repo_id: REPO_ID,
                agent_id: 999_991,
                hostname: "hold-test-host".to_owned(),
                hold: RepoHostHold {
                    repo_host_id: REPO_HOST_ID,
                    run_id: "run-hold-test".to_owned(),
                },
            }),
        )
    }

    /// Regression test: the hold used to be released only after the sync
    /// returned, so a panic in it left the reservation counted forever and
    /// the host was never shut down after a backup again.
    #[tokio::test]
    async fn the_host_hold_is_released_when_the_sync_panics() {
        let state = unreachable_state();
        let guard = hold_alongside_the_run(&state).await;

        let sync = tokio::spawn(async move {
            let _guard = guard;
            panic!("sync blew up");
        });
        assert!(sync.await.expect_err("the sync panics").is_panic());

        assert_eq!(
            state.task_registry.shutdown(Duration::from_secs(30)).await,
            0
        );
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
        assert_eq!(
            state.power_sessions.end(HOST).await,
            Some((false, false)),
            "with the sync's hold released, the run's teardown is the last one out"
        );
    }

    /// The normal path releases the hold itself and disarms the deferred
    /// release, so the reservation is ended once, not twice.
    #[tokio::test]
    async fn releasing_the_host_hold_disarms_its_drop() {
        let state = unreachable_state();
        let guard = hold_alongside_the_run(&state).await;

        guard.release_now().await;

        assert_eq!(
            state.task_registry.pending_count(),
            0,
            "nothing may be left to release on drop"
        );
        assert!(!state.background_task_tracker.any_active());
        assert_eq!(
            state.power_sessions.end(HOST).await,
            Some((false, false)),
            "the run's own reservation must still be there for its teardown"
        );
    }

    /// A sync without a hold - a cancelled run, or a report without a
    /// `run_id` - has nothing to release either way.
    #[tokio::test]
    async fn a_sync_without_a_host_hold_releases_nothing() {
        let state = unreachable_state();
        state.power_sessions.reserve(HOST).await;

        drop(HostHoldGuard::new(state.clone(), None));
        HostHoldGuard::new(state.clone(), None).release_now().await;

        assert_eq!(state.task_registry.pending_count(), 0);
        assert_eq!(state.power_sessions.end(HOST).await, Some((false, false)));
    }

    #[test]
    fn a_cancelled_run_takes_no_host_hold_and_protects_no_archive() {
        let finished = cancelled(5, "cancelled-host", 7);
        assert_eq!(finished.repo_id, 7);
        assert_eq!(finished.agent_id, 5);
        assert_eq!(finished.hostname, "cancelled-host");
        assert!(finished.archive_name.is_none());
        assert!(finished.host_hold.is_none());
    }

    /// A run whose repository can't be loaded takes no hold, so the run's
    /// own reservation stays the only one and its teardown is the last out.
    #[tokio::test]
    async fn a_run_whose_repository_cannot_be_loaded_takes_no_host_hold() {
        let state = unreachable_state();
        state.power_sessions.reserve(HOST).await;

        let hold = hold_repo_host(&state, REPO_ID, Some("run-unloadable-repo")).await;

        assert!(hold.is_none());
        assert_eq!(state.power_sessions.end(HOST).await, Some((false, false)));
    }

    /// Dropped where no runtime exists, the guard can't spawn its release:
    /// it gives up without registering anything, leaving both reservations.
    #[test]
    fn a_host_hold_dropped_outside_a_runtime_is_left_reserved() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (state, guard) = runtime.block_on(async {
            let state = unreachable_state();
            let guard = hold_alongside_the_run(&state).await;
            (state, guard)
        });

        drop(guard);

        runtime.block_on(async move {
            assert_eq!(state.task_registry.pending_count(), 0);
            assert!(!state.background_task_tracker.any_active());
            assert_eq!(
                state.power_sessions.end(HOST).await,
                None,
                "the sync's reservation must still be counted"
            );
            assert_eq!(state.power_sessions.end(HOST).await, Some((false, false)));
        });
    }

    async fn insert_repo(pool: &PgPool) -> i64 {
        let passphrase_encrypted = shared::crypto::encrypt_passphrase(
            "post-backup-sync-passphrase",
            &shared::crypto::derive_key(KEY_MATERIAL).unwrap(),
        )
        .unwrap();
        db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: "post-backup-sync-repo",
                repo_path: "/backups/post-backup-sync",
                ssh_user: "borg",
                ssh_host: "sync.local",
                ssh_port: 2222,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "zstd",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    /// The repository is deleted while the sync lists it, so the listing
    /// succeeds but recording `last_synced_at` and the import error both fail
    /// on the missing row. Neither failure may cut the sync short: it still
    /// clears its progress and tells the UI the data changed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_sync_whose_repository_is_deleted_mid_listing_still_finishes(pool: PgPool) {
        let repo_id = insert_repo(&pool).await;
        let handshake = tempfile::tempdir().unwrap();
        let listing = handshake.path().join("listing");
        let resume = handshake.path().join("resume");
        let script = format!(
            r#"#!/bin/sh
case "$1" in
  list)
    : > '{listing}'
    while [ ! -e '{resume}' ]; do sleep 0.05; done
    echo '{{"archives": []}}'
    ;;
  *)
    exit 2
    ;;
esac
"#,
            listing = listing.display(),
            resume = resume.display(),
        );
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _borg_guard) = crate::test_support::install_fake_borg(&script).await;
        let state = crate::test_support::build_test_state(pool.clone(), KEY_MATERIAL);
        let mut ui_events = state.ui_broadcast.subscribe();

        let sync_task = tokio::spawn({
            let state = state.clone();
            async move { sync(&state, repo_id, None).await }
        });
        tokio::time::timeout(Duration::from_secs(30), async {
            while !tokio::fs::try_exists(&listing).await.unwrap() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the sync lists the repository");
        db::delete_repo(&pool, repo_id).await.unwrap();
        tokio::fs::write(&resume, b"").await.unwrap();

        tokio::time::timeout(Duration::from_secs(30), sync_task)
            .await
            .expect("the sync finishes")
            .expect("the sync does not panic");

        let data_changed = std::iter::from_fn(|| ui_events.try_recv().ok())
            .any(|event| matches!(event, ServerToUi::DataChanged));
        assert!(data_changed, "the UI must still be told the data changed");
        assert!(
            db::get_repo_with_stats(&pool, repo_id).await.is_err(),
            "the failed writes must not have recreated the repository"
        );
        assert_eq!(
            state.task_registry.shutdown(Duration::from_secs(30)).await,
            0
        );
    }
}
