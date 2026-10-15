// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The persisted mirror of [`PowerSessionTracker`]'s holds, reading it back
//! after a restart, and releasing what it read back once each run is done.

use shared::types::{AcknowledgedFilter, ReportStatus, SystemEventType};

use super::*;
use crate::{
    db::power_holds::{self, PowerHoldRow},
    test_support::{logged, warnings_while},
};

const RUN: &str = "run-restarted";

async fn holds(pool: &sqlx::PgPool) -> Vec<PowerHoldRow> {
    power_holds::list_power_holds(pool).await.unwrap()
}

/// An agent whose host was woken and is shut down after a backup, over an
/// SSH address that refuses at once - so a teardown shows up as the attempt
/// on the run's timeline.
async fn shutdown_agent(pool: &sqlx::PgPool) -> AgentRow {
    crate::test_support::update_agent_power(
        pool,
        test_agent(pool).await.id,
        crate::test_support::AgentPowerPatch {
            wake: crate::db::machines::MachinePowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
            },
            process: crate::db::AgentProcessPatch {
                start_agent_enabled: false,
                stop_agent_after_backup: false,
                ssh_host: Some(UNREACHABLE_HOST),
                ssh_port: UNREACHABLE_PORT,
                agent_service_name: "assimilate-agent",
            },
        },
    )
    .await
    .unwrap()
}

/// A repository on a host that is shut down after a backup.
async fn shutdown_repo(pool: &sqlx::PgPool) -> RepoRow {
    set_repo_host_power(
        pool,
        test_repo(pool).await,
        crate::test_support::RepoPowerPatch {
            wake_enabled: true,
            wake_mac_address: Some("9C:B6:D0:1A:44:7F"),
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: true,
        },
    )
    .await
    .unwrap()
}

/// Leaves behind what a server that died mid-run would: `holds` taken on a
/// tracker that is then dropped, the session marked as woken. Returns a new
/// tracker that has read them back, as the next server process would.
async fn restart_with(
    pool: &sqlx::PgPool,
    holds: &[(PowerHostKey, HoldOwner<'_>)],
) -> PowerSessionTracker {
    let before = PowerSessionTracker::persistent(pool.clone());
    for (key, owner) in holds {
        before.reserve(*key, *owner).await;
        before.record_outcome(*key, true, false).await;
    }
    restart(pool, holds.len()).await
}

/// A new tracker that has read back the `expected` holds in the database.
async fn restart(pool: &sqlx::PgPool, expected: usize) -> PowerSessionTracker {
    let after = PowerSessionTracker::persistent(pool.clone());
    assert_eq!(after.recover().await, expected);
    after
}

async fn sweep(pool: &sqlx::PgPool, sessions: &PowerSessionTracker) {
    let registry = AgentRegistry::new();
    let bus = UiBroadcast::new();
    release_orphaned_holds(ctx(pool, &registry, &bus, sessions)).await;
}

async fn run_event_types(pool: &sqlx::PgPool, agent_id: i64, repo_id: i64) -> Vec<RunEventType> {
    db::run_events::list_run_events(pool, RUN, agent_id, repo_id)
        .await
        .unwrap()
        .iter()
        .map(|e| e.event_type)
        .collect()
}

async fn abandoned_events(pool: &sqlx::PgPool) -> Vec<db::SystemEventRow> {
    db::get_system_events(pool, 100, AcknowledgedFilter::All)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == SystemEventType::PowerReleaseAbandoned)
        .collect()
}

/// Makes every persisted hold look as old as `hours`.
async fn age_holds(pool: &sqlx::PgPool, hours: i32) {
    sqlx::query!(
        "UPDATE power_holds SET created_at = now() - make_interval(hours => $1)",
        hours
    )
    .execute(pool)
    .await
    .unwrap();
}

#[test]
fn a_host_key_round_trips_through_its_stored_kind_and_id() {
    for key in [PowerHostKey::Agent(7), PowerHostKey::RepoHost(9)] {
        assert_eq!(PowerHostKey::new(key.kind(), key.host_id()), key);
    }
    assert_eq!(PowerHostKind::Agent.to_string(), "agent");
    assert_eq!(PowerHostKind::RepoHost.to_string(), "repo_host");
    assert_eq!(
        "repo_host".parse::<PowerHostKind>().unwrap(),
        PowerHostKind::RepoHost
    );
}

#[tokio::test]
async fn a_tracker_without_a_store_recovers_nothing() {
    assert_eq!(PowerSessionTracker::default().recover().await, 0);
}

/// Each hold is one row while it is held. A hold taken after the session woke
/// the host carries that too, so whichever hold outlives a restart knows.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_persistent_tracker_mirrors_each_hold_to_the_database(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    let sessions = PowerSessionTracker::persistent(pool.clone());
    let host = PowerHostKey::RepoHost(repo.repo_host_id);
    let target = HoldOwner::target(RUN, agent.id, repo.id);
    let dependency = HoldOwner::dependency(RUN, agent.id);

    sessions.reserve(host, target).await;
    sessions.record_outcome(host, false, false).await;
    assert!(holds(&pool).await.iter().all(|row| !row.woke));
    sessions.record_outcome(host, true, false).await;
    sessions.reserve(host, dependency).await;

    let rows = holds(&pool).await;
    assert_eq!(
        rows.iter()
            .map(|r| (r.key, r.owner(), r.woke, r.started_agent))
            .collect::<Vec<_>>(),
        vec![(host, target, true, false), (host, dependency, true, false)]
    );

    assert_eq!(sessions.end(host, dependency).await, None);
    assert_eq!(
        holds(&pool)
            .await
            .iter()
            .map(PowerHoldRow::owner)
            .collect::<Vec<_>>(),
        vec![target],
        "ending a hold removes that hold's row, not another one on the host"
    );
    assert_eq!(sessions.end(host, target).await, Some((true, false)));
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}

/// The teardowns end the same hold the run reserved, so a finished run
/// leaves no row behind.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_finished_runs_teardown_removes_its_holds(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    let sessions = PowerSessionTracker::persistent(pool.clone());
    let registry = AgentRegistry::new();
    let bus = UiBroadcast::new();
    let owner = HoldOwner::target(RUN, agent.id, repo.id);
    sessions.reserve(PowerHostKey::Agent(agent.id), owner).await;
    sessions
        .reserve(PowerHostKey::RepoHost(repo.repo_host_id), owner)
        .await;
    sessions
        .reserve(
            PowerHostKey::RepoHost(repo.repo_host_id),
            HoldOwner::dependency(RUN, agent.id),
        )
        .await;
    let ctx = ctx(&pool, &registry, &bus, &sessions);
    let site = RunEventSite {
        run_id: RUN,
        agent_id: agent.id,
        repo_id: repo.id,
        hostname: "power-test-agent",
    };

    release_repo_host_for_dependency(ctx, repo.repo_host_id, site).await;
    teardown_agent_power(ctx, &agent, repo.id, RUN).await;
    teardown_repo_power(ctx, &repo, repo.repo_host_id, agent.id, RUN, "host").await;

    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}

/// After a restart the holds left behind count again: a run on the same host
/// ending first does not get the teardown, and the last hold out still has
/// the woken session's credit.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn recovered_holds_count_again_and_keep_the_sessions_flags(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    let key = PowerHostKey::Agent(agent.id);
    let interrupted = HoldOwner::target(RUN, agent.id, repo.id);
    let sessions = restart_with(&pool, &[(key, interrupted)]).await;
    assert_eq!(sessions.take_orphans().await.len(), 1);

    let newer = HoldOwner::target("run-after-restart", agent.id, repo.id);
    sessions.reserve(key, newer).await;
    assert!(
        holds(&pool).await.iter().all(|r| r.woke),
        "a hold taken after the restart joins the woken session"
    );
    assert_eq!(sessions.end(key, newer).await, None);
    assert_eq!(sessions.end(key, interrupted).await, Some((true, false)));
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}

/// A database that cannot be written never gets in a run's way: each failed
/// write is logged and the counts in memory carry on as before.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_failed_write_is_logged_and_the_counts_still_work(pool: sqlx::PgPool) {
    pool.close().await;
    let sessions = PowerSessionTracker::persistent(pool.clone());
    let key = PowerHostKey::Agent(1);

    let mut flags = None;
    let mut recovered = None;
    let warnings = warnings_while(async {
        sessions.reserve(key, TEST_HOLD).await;
        sessions.record_outcome(key, true, false).await;
        flags = Some(sessions.end(key, TEST_HOLD).await);
        recovered = Some(sessions.recover().await);
    })
    .await;

    assert_eq!(flags, Some(Some((true, false))));
    assert_eq!(recovered, Some(0));
    for message in [
        "failed to persist a power hold",
        "failed to persist a power hold's outcome",
        "failed to delete a power hold",
        "failed to read back the power holds of interrupted runs",
    ] {
        assert!(logged(&warnings, message), "missing warning: {message}");
    }
}

/// An orphaned hold on an agent's host whose run has since reported back is
/// released the way the run would have: the woken host is shut down.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_orphan_whose_run_reported_back_is_torn_down(pool: sqlx::PgPool) {
    let agent = shutdown_agent(&pool).await;
    let repo = test_repo(&pool).await;
    db::insert_backup_pending(&pool, agent.id, repo.id, None, RUN, chrono::Utc::now())
        .await
        .unwrap();
    let key = PowerHostKey::Agent(agent.id);
    let sessions = restart_with(&pool, &[(key, HoldOwner::target(RUN, agent.id, repo.id))]).await;

    sweep(&pool, &sessions).await;
    assert!(
        run_event_types(&pool, agent.id, repo.id).await.is_empty(),
        "nothing happens while the run is still in flight"
    );
    assert_eq!(holds(&pool).await.len(), 1);

    db::settle_pending_report_for_target(
        &pool,
        RUN,
        agent.id,
        repo.id,
        ReportStatus::Success,
        "done",
    )
    .await
    .unwrap();
    sweep(&pool, &sessions).await;

    assert_eq!(
        run_event_types(&pool, agent.id, repo.id).await,
        vec![RunEventType::ShutdownSent]
    );
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
    assert_eq!(sessions.take_orphans().await, Vec::<PowerHoldRow>::new());
}

/// A run that never got as far as a report has nothing left to settle it, so
/// its holds are released at once - a repository target's hold through the
/// repository teardown, a dependency's through the dependency release.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn orphans_of_a_run_without_a_report_are_released_at_once(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = shutdown_repo(&pool).await;
    let host = PowerHostKey::RepoHost(repo.repo_host_id);
    let sessions = restart_with(
        &pool,
        &[
            (host, HoldOwner::target(RUN, agent.id, repo.id)),
            (host, HoldOwner::dependency(RUN, agent.id)),
        ],
    )
    .await;

    sweep(&pool, &sessions).await;

    assert_eq!(
        run_event_types(&pool, agent.id, repo.id).await,
        vec![RunEventType::ShutdownSent],
        "the last of the two holds out shuts the woken host down, once"
    );
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
    assert_eq!(sessions.take_orphans().await, Vec::<PowerHoldRow>::new());
}

/// A dependency's hold has no repository of its own, so the shut-down it
/// leads to is recorded on the timeline of the repository whose login it
/// borrows.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_orphaned_dependency_hold_shuts_down_its_host(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = shutdown_repo(&pool).await;
    let sessions = restart_with(
        &pool,
        &[(
            PowerHostKey::RepoHost(repo.repo_host_id),
            HoldOwner::dependency(RUN, agent.id),
        )],
    )
    .await;

    sweep(&pool, &sessions).await;

    assert_eq!(
        run_event_types(&pool, agent.id, repo.id).await,
        vec![RunEventType::ShutdownSent]
    );
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}

/// A hold whose agent, or whose repository, is gone has no teardown to
/// replay, but its count is still given back - and so is one that has no
/// repository on an agent's host, which no run takes.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn orphans_with_nothing_to_tear_down_still_give_their_count_back(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let gone_agent = db::insert_agent(&pool, "power-gone-agent", None, "hash", None, None)
        .await
        .unwrap();
    let repo = test_repo(&pool).await;
    let gone_repo = db::insert_repo(
        &pool,
        &db::InsertRepoParams {
            name: "power-gone-repo",
            repo_path: "/backup/power-gone",
            ssh_user: "borg",
            ssh_host: "gone.example",
            ssh_port: UNREACHABLE_PORT,
            passphrase_encrypted: b"irrelevant",
            compression: "lz4",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let gone_agent_key = PowerHostKey::Agent(gone_agent.id);
    let gone_repo_key = PowerHostKey::RepoHost(gone_repo.repo_host_id);
    let no_repo_key = PowerHostKey::Agent(agent.id);
    let held = [
        (
            gone_agent_key,
            HoldOwner::target(RUN, gone_agent.id, repo.id),
        ),
        (
            gone_repo_key,
            HoldOwner::target(RUN, agent.id, gone_repo.id),
        ),
        (no_repo_key, HoldOwner::dependency(RUN, agent.id)),
    ];
    let sessions = restart_with(&pool, &held).await;
    db::delete_agent(&pool, gone_agent.id).await.unwrap();
    db::delete_repo(&pool, gone_repo.id).await.unwrap();

    let warnings = warnings_while(sweep(&pool, &sessions)).await;

    assert!(logged(
        &warnings,
        "failed to load the agent of an orphaned power hold"
    ));
    assert!(logged(
        &warnings,
        "failed to load the repository of an orphaned power hold"
    ));
    for (key, owner) in held {
        assert_eq!(
            sessions.end(key, owner).await,
            None,
            "{key:?} must have been given back already"
        );
    }
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}

/// A hold whose run cannot be looked up is kept for the next pass.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_orphan_whose_run_cannot_be_checked_is_kept(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    let sessions = restart_with(
        &pool,
        &[(
            PowerHostKey::Agent(agent.id),
            HoldOwner::target(RUN, agent.id, repo.id),
        )],
    )
    .await;
    pool.close().await;

    let warnings = warnings_while(sweep(&pool, &sessions)).await;

    assert!(logged(
        &warnings,
        "failed to check on the run of an orphaned power hold"
    ));
    assert_eq!(sessions.take_orphans().await.len(), 1);
}

/// A run still in flight a day after it took its hold is given up on: the
/// machines are left as they are, and the Activity Log says each one the
/// session woke may still be on - once per machine, from its last hold.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_orphan_whose_run_never_reports_back_is_abandoned_after_a_day(pool: sqlx::PgPool) {
    let agent = shutdown_agent(&pool).await;
    let repo = shutdown_repo(&pool).await;
    db::insert_backup_pending(&pool, agent.id, repo.id, None, RUN, chrono::Utc::now())
        .await
        .unwrap();
    let owner = HoldOwner::target(RUN, agent.id, repo.id);
    let host = PowerHostKey::RepoHost(repo.repo_host_id);
    let sessions = restart_with(
        &pool,
        &[
            (PowerHostKey::Agent(agent.id), owner),
            (host, owner),
            (host, HoldOwner::dependency(RUN, agent.id)),
        ],
    )
    .await;

    drop(sessions);
    age_holds(&pool, 23).await;
    let sessions = restart(&pool, 3).await;
    sweep(&pool, &sessions).await;
    assert_eq!(holds(&pool).await.len(), 3, "a day has not passed yet");
    assert!(abandoned_events(&pool).await.is_empty());

    drop(sessions);
    age_holds(&pool, 25).await;
    let sessions = restart(&pool, 3).await;
    sweep(&pool, &sessions).await;

    assert!(
        run_event_types(&pool, agent.id, repo.id).await.is_empty(),
        "nothing is shut down"
    );
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
    assert_eq!(sessions.take_orphans().await, Vec::<PowerHoldRow>::new());
    let events = abandoned_events(&pool).await;
    let mut machines: Vec<_> = events.iter().map(|e| e.hostname.clone()).collect();
    machines.sort();
    assert_eq!(
        machines,
        vec![
            Some(UNREACHABLE_HOST.to_owned()),
            Some(agent.hostname.clone())
        ]
    );
    assert!(events.iter().all(|e| e.message.contains(RUN)
        && e.message.contains("may still be on")
        && e.acknowledgeable));
}

/// A session that woke nothing leaves nothing on, so giving up on it is not
/// worth an Activity Log entry.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn abandoning_a_hold_that_woke_nothing_records_nothing(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    db::insert_backup_pending(&pool, agent.id, repo.id, None, RUN, chrono::Utc::now())
        .await
        .unwrap();
    let key = PowerHostKey::Agent(agent.id);
    let before = PowerSessionTracker::persistent(pool.clone());
    before
        .reserve(key, HoldOwner::target(RUN, agent.id, repo.id))
        .await;
    age_holds(&pool, 25).await;
    let sessions = restart(&pool, 1).await;

    sweep(&pool, &sessions).await;

    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
    assert!(abandoned_events(&pool).await.is_empty());
}

/// An Activity Log entry that cannot be written is logged instead.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_abandoned_release_that_cannot_be_recorded_is_logged(pool: sqlx::PgPool) {
    let agent = test_agent(&pool).await;
    let repo = test_repo(&pool).await;
    db::insert_backup_pending(&pool, agent.id, repo.id, None, RUN, chrono::Utc::now())
        .await
        .unwrap();
    drop(
        restart_with(
            &pool,
            &[(
                PowerHostKey::Agent(agent.id),
                HoldOwner::target(RUN, agent.id, repo.id),
            )],
        )
        .await,
    );
    age_holds(&pool, 25).await;
    let sessions = restart(&pool, 1).await;
    sqlx::query!("ALTER TABLE system_events RENAME TO system_events_unavailable")
        .execute(&pool)
        .await
        .unwrap();

    let warnings = warnings_while(sweep(&pool, &sessions)).await;

    assert!(logged(
        &warnings,
        "failed to record an abandoned power release"
    ));
    assert_eq!(holds(&pool).await, Vec::<PowerHoldRow>::new());
}
