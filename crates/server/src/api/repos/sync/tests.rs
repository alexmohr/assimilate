// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;
use crate::api::repos::test_helpers::{REPO_URL, closed_pool, install_fake_script, missing_borg};

#[test]
fn is_unknown_archive_detects_new_and_known() {
    use std::collections::HashSet;
    let known: HashSet<String> = ["host-2024-01-01".to_string()].into_iter().collect();
    assert!(is_unknown_archive(
        &serde_json::json!({"name": "host-2024-02-02"}),
        &known
    ));
    assert!(!is_unknown_archive(
        &serde_json::json!({"name": "host-2024-01-01"}),
        &known
    ));
}

#[test]
fn is_unknown_archive_rejects_empty_or_missing_name() {
    use std::collections::HashSet;
    let known: HashSet<String> = HashSet::new();
    assert!(!is_unknown_archive(
        &serde_json::json!({"name": ""}),
        &known
    ));
    assert!(!is_unknown_archive(&serde_json::json!({"size": 1}), &known));
}

fn name_set(names: &[&str]) -> std::collections::HashSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn stale_archive_names_are_the_known_ones_borg_no_longer_lists() {
    let mut stale = stale_archive_names(
        &name_set(&["kept", "pruned-1", "pruned-2"]),
        &name_set(&["kept", "brand-new"]),
        None,
    );
    stale.sort();
    assert_eq!(stale, vec!["pruned-1".to_owned(), "pruned-2".to_owned()]);
}

#[test]
fn stale_archive_names_never_include_the_archive_the_run_just_wrote() {
    let stale = stale_archive_names(
        &name_set(&["just-written", "pruned"]),
        &name_set(&["older"]),
        Some("just-written"),
    );
    assert_eq!(stale, vec!["pruned".to_owned()]);
}

async fn refresh_stats_warnings(
    pool: &PgPool,
    timeout: Duration,
) -> Vec<crate::log_buffer::LogEntry> {
    crate::test_support::warnings_while(refresh_repo_info_stats(
        pool,
        REPO_URL,
        &HashMap::new(),
        1,
        0,
        timeout,
        &shared::task_registry::TaskRegistry::default(),
    ))
    .await
}

#[tokio::test]
async fn refresh_repo_info_stats_logs_each_way_it_can_fail() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let pool = closed_pool().await;
    let timeout = Duration::from_secs(30);

    let warnings = {
        let _guard = missing_borg();
        refresh_stats_warnings(&pool, timeout).await
    };
    assert!(crate::test_support::logged(
        &warnings,
        "failed to run borg info for repo stats"
    ));

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nsleep 5\n").await;
    let warnings = refresh_stats_warnings(&pool, Duration::from_millis(200)).await;
    assert!(crate::test_support::logged(
        &warnings,
        "borg info timed out refreshing repo stats"
    ));

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\necho boom >&2\nexit 2\n").await;
    let warnings = refresh_stats_warnings(&pool, timeout).await;
    assert!(crate::test_support::logged(
        &warnings,
        "borg info (repo stats) failed"
    ));

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nprintf 'not json'\n").await;
    let warnings = refresh_stats_warnings(&pool, timeout).await;
    assert!(crate::test_support::logged(
        &warnings,
        "failed to parse borg info repo stats"
    ));

    let (_borg_dir, _guard) =
        install_fake_script("#!/bin/sh\nprintf '{\"cache\":{\"stats\":{}}}'\n").await;
    let warnings = refresh_stats_warnings(&pool, timeout).await;
    assert!(crate::test_support::logged(
        &warnings,
        "failed to persist repo info stats"
    ));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn import_reports_skip_archives_they_cannot_place(pool: PgPool) {
    let nameless = serde_json::json!({});
    let hostless = serde_json::json!({"name": "no-host"});
    let undated =
        serde_json::json!({"name": "bad-start", "hostname": "web1", "start": "yesterday"});

    let mut reports = None;
    let warnings = crate::test_support::warnings_while(async {
        reports = Some(
            build_import_reports(
                &pool,
                &UiBroadcast::new(),
                1,
                &[&nameless, &hostless, &undated],
            )
            .await,
        );
    })
    .await;

    assert!(reports.expect("ran").unwrap().is_empty());
    assert!(crate::test_support::logged(
        &warnings,
        "skipping archive with missing hostname metadata"
    ));
    assert!(crate::test_support::logged(
        &warnings,
        "skipping archive with unparseable start timestamp"
    ));
}
