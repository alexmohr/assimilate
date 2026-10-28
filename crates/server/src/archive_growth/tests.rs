// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use shared::archive_growth::GrowthNodeKind;
use sqlx::PgPool;

use super::*;
use crate::{
    api::{
        archive_growth::{get_archive_growth, recompute_archive_growth},
        auth::AuthUser,
    },
    db,
    test_support::{build_test_state, insert_auth_user, install_fake_borg},
};

/// A borg whose `list` prints a small archive with its per-file `dcsize`,
/// refuses to list `missing`, and fails unless asked for exactly the keys
/// the breakdown reads.
const FAKE_BORG: &str = r#"#!/bin/sh
[ "$1" = list ] && [ "$2" = --json-lines ] && [ "$3" = --format ] \
  && [ "$4" = '{size}{dcsize}' ] || { echo "unexpected arguments: $*" >&2; exit 2; }
case "$7" in
  *::missing) echo "Archive missing does not exist" >&2; exit 1 ;;
esac
printf '%s\n' \
  '{"type": "d", "path": "home", "size": 0, "dcsize": 0}' \
  '{"type": "-", "path": "home/alex/Downloads/ubuntu.iso", "size": 6000, "dcsize": 1900}' \
  '{"type": "-", "path": "home/alex/.cache/pip.whl", "size": 400, "dcsize": 300}' \
  'Synchronizing chunks cache...' \
  '{"type": "-", "path": "etc/hosts", "size": 30, "dcsize": 0}' \
  '{"type": "-", "path": "var/log/syslog", "size": 900, "dcsize": 700}'
"#;

struct Fixture {
    state: AppState,
    viewer: AuthUser,
    stranger: AuthUser,
    repo_id: i64,
}

async fn fixture(pool: PgPool) -> Fixture {
    let state = build_test_state(pool, b"archive-growth-test-key");
    let passphrase = shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
    let repo_id = db::insert_repo(
        &state.pool,
        &db::InsertRepoParams {
            name: "growth-nightly",
            repo_path: "/backups/growth-nightly",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: &passphrase,
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap()
    .id;
    let viewer = insert_auth_user(&state.pool, "growth-viewer").await;
    let stranger = insert_auth_user(&state.pool, "growth-stranger").await;
    db::upsert_repo_permission(
        &state.pool,
        &db::UpsertRepoPermissionParams {
            user_id: viewer.user_id,
            repo_id,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract: false,
            can_delete: false,
        },
    )
    .await
    .unwrap();
    Fixture {
        state,
        viewer,
        stranger,
        repo_id,
    }
}

impl Fixture {
    async fn get(&self, user: &AuthUser, archive: &str) -> Result<ArchiveGrowthResponse, ApiError> {
        get_archive_growth(
            State(self.state.clone()),
            user.clone(),
            AxumPath((self.repo_id, archive.to_owned())),
        )
        .await
        .map(|Json(response)| response)
    }

    async fn settled(&self, archive: &str) -> ArchiveGrowthResponse {
        assert!(
            self.state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await,
            "the breakdown never finished"
        );
        read(&self.state.pool, self.repo_id, archive)
            .await
            .unwrap()
            .unwrap()
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn the_first_request_starts_the_breakdown_and_later_ones_read_it(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    // Held, the repository keeps the job waiting, so the answer is the
    // claim's and not a job that already finished.
    let repository = fixture.state.repo_lock.acquire(fixture.repo_id).await;
    let first = fixture.get(&fixture.viewer, "nightly-1").await.unwrap();
    assert!(first.status.is_in_progress(), "got {first:?}");
    assert_eq!(first.growth, None);
    drop(repository);

    let settled = fixture.settled("nightly-1").await;
    assert_eq!(settled.status, ArchiveGrowthStatus::Done);
    assert!(settled.computed_at.is_some());
    let growth = settled.growth.clone().unwrap();
    assert_eq!((growth.total_bytes, growth.files), (2900, 3));
    let top: Vec<(&str, GrowthNodeKind, i64)> = growth
        .children
        .iter()
        .map(|node| (node.name.as_str(), node.kind, node.bytes))
        .collect();
    assert_eq!(
        top,
        [
            ("home/alex", GrowthNodeKind::Directory, 2200),
            ("var/log", GrowthNodeKind::Directory, 700),
        ]
    );
    let largest = growth.largest_files.first().unwrap();
    assert_eq!(
        (largest.path.as_str(), largest.bytes, largest.size),
        ("home/alex/Downloads/ubuntu.iso", 1900, 6000)
    );

    let again = fixture.get(&fixture.viewer, "nightly-1").await.unwrap();
    assert_eq!(again, settled);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_failed_listing_is_recorded_with_borgs_reason(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    fixture.get(&fixture.viewer, "missing").await.unwrap();
    let settled = fixture.settled("missing").await;

    assert_eq!(settled.status, ArchiveGrowthStatus::Failed);
    assert_eq!(settled.growth, None);
    assert!(
        settled
            .error
            .as_deref()
            .is_some_and(|error| error.contains("does not exist")),
        "got {settled:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn recomputing_works_a_failed_breakdown_out_again(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
    fixture.get(&fixture.viewer, "nightly-2").await.unwrap();
    fixture.settled("nightly-2").await;
    let archive_id = archive_id(&fixture.state.pool, fixture.repo_id, "nightly-2")
        .await
        .unwrap()
        .unwrap();
    sqlx::query!(
        "UPDATE archive_growth SET status = 'failed', result = NULL, error_message = 'boom' WHERE \
         archive_id = $1",
        archive_id,
    )
    .execute(&fixture.state.pool)
    .await
    .unwrap();

    let repository = fixture.state.repo_lock.acquire(fixture.repo_id).await;
    let (status, Json(response)) = recompute_archive_growth(
        State(fixture.state.clone()),
        fixture.viewer.clone(),
        AxumPath((fixture.repo_id, "nightly-2".to_owned())),
    )
    .await
    .unwrap();

    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(response.status.is_in_progress(), "got {response:?}");
    assert_eq!(response.error, None);
    drop(repository);
    let settled = fixture.settled("nightly-2").await;
    assert_eq!(settled.status, ArchiveGrowthStatus::Done);
    assert_eq!(settled.growth.map(|growth| growth.total_bytes), Some(2900));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn recomputing_while_one_is_under_way_changes_nothing(pool: PgPool) {
    let fixture = fixture(pool).await;
    let archive_id = get_or_create_archive_id(&fixture.state.pool, fixture.repo_id, "nightly-3")
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO archive_growth (archive_id, status) VALUES ($1, 'computing')",
        archive_id,
    )
    .execute(&fixture.state.pool)
    .await
    .unwrap();

    let response = recompute(&fixture.state, fixture.repo_id, "nightly-3")
        .await
        .unwrap();

    assert_eq!(response.status, ArchiveGrowthStatus::Computing);
    assert!(!fixture.state.background_task_tracker.any_active());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_restart_fails_breakdowns_left_under_way(pool: PgPool) {
    let fixture = fixture(pool).await;
    let ids = [
        ("nightly-4", "pending"),
        ("nightly-5", "computing"),
        ("nightly-6", "done"),
    ];
    for (name, status) in ids {
        let archive_id = get_or_create_archive_id(&fixture.state.pool, fixture.repo_id, name)
            .await
            .unwrap();
        sqlx::query!(
            "INSERT INTO archive_growth (archive_id, status) VALUES ($1, $2)",
            archive_id,
            status,
        )
        .execute(&fixture.state.pool)
        .await
        .unwrap();
    }

    release_interrupted(&fixture.state.pool).await;

    let read_status = |name: &'static str| {
        let pool = fixture.state.pool.clone();
        let repo_id = fixture.repo_id;
        async move { read(&pool, repo_id, name).await.unwrap().unwrap().status }
    };
    assert_eq!(read_status("nightly-4").await, ArchiveGrowthStatus::Failed);
    assert_eq!(read_status("nightly-5").await, ArchiveGrowthStatus::Failed);
    assert_eq!(read_status("nightly-6").await, ArchiveGrowthStatus::Done);
    assert_eq!(fail_interrupted(&fixture.state.pool).await.unwrap(), 0);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_restart_that_cannot_reach_the_database_still_starts(pool: PgPool) {
    pool.close().await;

    release_interrupted(&pool).await;

    assert!(fail_interrupted(&pool).await.is_err());
}

async fn computing(fixture: &Fixture, name: &str) -> i64 {
    let archive_id = get_or_create_archive_id(&fixture.state.pool, fixture.repo_id, name)
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO archive_growth (archive_id, status) VALUES ($1, 'computing')",
        archive_id,
    )
    .execute(&fixture.state.pool)
    .await
    .unwrap();
    archive_id
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_panicking_job_is_recorded_as_failed(pool: PgPool) {
    let fixture = fixture(pool).await;
    let archive_id = computing(&fixture, "nightly-7").await;

    supervise(&fixture.state.pool, archive_id, "nightly-7", async {
        panic!("chunk index exploded")
    })
    .await;

    let settled = read(&fixture.state.pool, fixture.repo_id, "nightly-7")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.status, ArchiveGrowthStatus::Failed);
    assert_eq!(
        settled.error.as_deref(),
        Some("archive growth panicked: chunk index exploded")
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_failure_that_cannot_be_recorded_is_only_logged(pool: PgPool) {
    let fixture = fixture(pool).await;
    let archive_id = computing(&fixture, "nightly-8").await;
    fixture.state.pool.close().await;

    supervise(&fixture.state.pool, archive_id, "nightly-8", async {
        Err(ApiError::Internal("borg went away".to_owned()))
    })
    .await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_user_who_cannot_view_the_repository_is_refused(pool: PgPool) {
    let fixture = fixture(pool).await;

    let result = fixture.get(&fixture.stranger, "nightly-1").await;

    assert!(
        matches!(result, Err(ApiError::Forbidden(_))),
        "got {result:?}"
    );
    assert_eq!(
        read(&fixture.state.pool, fixture.repo_id, "nightly-1")
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_user_who_cannot_view_the_repository_cannot_recompute(pool: PgPool) {
    let fixture = fixture(pool).await;

    let result = recompute_archive_growth(
        State(fixture.state.clone()),
        fixture.stranger.clone(),
        AxumPath((fixture.repo_id, "nightly-1".to_owned())),
    )
    .await;

    assert!(
        matches!(result, Err(ApiError::Forbidden(_))),
        "got {result:?}"
    );
    assert_eq!(
        read(&fixture.state.pool, fixture.repo_id, "nightly-1")
            .await
            .unwrap(),
        None
    );
}

#[test]
fn lines_that_are_not_listed_entries_are_skipped() {
    let mut builder = GrowthBuilder::default();
    for line in [
        "",
        "Synchronizing chunks cache...",
        r#"{"type": "-", "size": 5, "dcsize": 5}"#,
        r#"{"type": "-", "path": "a/b", "size": 10, "dcsize": 4}"#,
    ] {
        add_line(&mut builder, line);
    }

    let growth = builder.finish();
    assert_eq!((growth.total_bytes, growth.files), (4, 1));
}

#[test]
fn paths_of_an_archive_made_from_dot_lose_their_dot_prefix() {
    let mut builder = GrowthBuilder::default();
    add_line(
        &mut builder,
        r#"{"type": "-", "path": "./var/cache/app.tar", "size": 10, "dcsize": 4}"#,
    );

    let growth = builder.finish();
    let names: Vec<&str> = growth
        .children
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(names, ["var/cache"]);
    let largest: Vec<&str> = growth
        .largest_files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(largest, ["var/cache/app.tar"]);
}
