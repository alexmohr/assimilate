// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;
use crate::api::repos::test_helpers::{closed_pool, install_fake_script};

#[tokio::test]
async fn queueing_indexing_logs_an_archive_it_could_not_queue() {
    let tracker = crate::background_tasks::BackgroundTaskTracker::default();
    let pool = closed_pool().await;

    let warnings =
        crate::test_support::warnings_while(queue_archive_indexing(QueueArchiveIndexingArgs {
            pool: &pool,
            encryption_key: &[0; 32],
            repo_id: 1,
            archive_names: &["a1".to_owned()],
            repo_lock: &RepoLock::default(),
            background_task_tracker: &tracker,
            task_registry: &shared::task_registry::TaskRegistry::default(),
            sync_kind: "test",
        }))
        .await;

    assert!(crate::test_support::logged(
        &warnings,
        "failed to queue archive indexing"
    ));
}

async fn index_all_warnings(pool: PgPool) -> Vec<crate::log_buffer::LogEntry> {
    crate::test_support::warnings_while(index_archives_with_progress(
        pool,
        [0; 32],
        1,
        UiBroadcast::new(),
        RepoLock::default(),
        false,
        shared::task_registry::TaskRegistry::default(),
    ))
    .await
}

#[tokio::test]
async fn indexing_stops_when_the_archives_cannot_be_listed() {
    let warnings = index_all_warnings(closed_pool().await).await;

    assert!(crate::test_support::logged(
        &warnings,
        "content indexing: failed to list archives"
    ));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn indexing_stops_when_the_finished_indexes_cannot_be_listed(pool: PgPool) {
    sqlx::query!("DROP TABLE archive_index_jobs CASCADE")
        .execute(&pool)
        .await
        .unwrap();

    let warnings = index_all_warnings(pool).await;

    assert!(crate::test_support::logged(
        &warnings,
        "content indexing: failed to list indexed archives"
    ));
}

#[tokio::test]
async fn indexing_an_archive_stops_when_its_job_cannot_be_recorded() {
    let pool = closed_pool().await;

    let warnings = crate::test_support::warnings_while(index_one_archive(IndexOneArchiveArgs {
        pool: &pool,
        encryption_key: &[0; 32],
        repo_id: 1,
        ui_broadcast: &UiBroadcast::new(),
        repo_lock: &RepoLock::default(),
        repo_lock_held: false,
        archive_name: "a1",
        index: 0,
        total: 1,
        total_i32: 1,
        task_registry: &shared::task_registry::TaskRegistry::default(),
    }))
    .await;

    assert!(crate::test_support::logged(
        &warnings,
        "content index job failed"
    ));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn indexing_an_archive_takes_the_repository_lock_when_it_is_not_held(pool: PgPool) {
    let state = crate::test_support::build_test_state(pool, b"repos-index-test-key");
    let passphrase = shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
    let repo = db::insert_repo(
        &state.pool,
        &db::InsertRepoParams {
            name: "indexed",
            repo_path: "/backups/indexed",
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
    .unwrap();
    let _gate = crate::borg::acquire_test_binary_gate().await;
    // An empty archive: borg lists nothing, so progress has no current path.
    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nexit 0\n").await;
    let mut events = state.ui_broadcast.subscribe();

    index_one_archive(IndexOneArchiveArgs {
        pool: &state.pool,
        encryption_key: &state.encryption_key,
        repo_id: repo.id,
        ui_broadcast: &state.ui_broadcast,
        repo_lock: &state.repo_lock,
        repo_lock_held: false,
        archive_name: "a1",
        index: 0,
        total: 1,
        total_i32: 1,
        task_registry: &state.task_registry,
    })
    .await;

    let mut messages = Vec::new();
    while let Ok(shared::protocol::ServerToUi::ImportProgress { message, .. }) = events.try_recv() {
        messages.push(message.unwrap_or_default());
    }
    assert!(
        messages.contains(&"Indexing \u{2018}a1\u{2019} (1/1) \u{2014} 0 files".to_owned()),
        "{messages:?}"
    );
    assert!(
        archive_index::list_indexed_archive_names(&state.pool, repo.id)
            .await
            .unwrap()
            .contains("a1"),
        "the empty archive's index is finished"
    );
}
