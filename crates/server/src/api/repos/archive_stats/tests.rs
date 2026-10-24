// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, time::Duration};

use super::*;
use crate::api::repos::test_helpers::{REPO_URL, closed_pool, install_fake_script, missing_borg};

#[tokio::test]
async fn stat_enrichment_stops_when_it_cannot_tell_which_archives_need_stats() {
    let tracker = crate::background_tasks::BackgroundTaskTracker::default();
    let pool = closed_pool().await;

    let warnings = crate::test_support::warnings_while(async {
        enrich_archive_stats_background(
            pool,
            REPO_URL.to_owned(),
            HashMap::new(),
            1,
            vec!["a1".to_owned()],
            &tracker,
            shared::task_registry::TaskRegistry::default(),
        );
        tracker.assert_idle(Duration::from_secs(30)).await;
    })
    .await;

    assert!(crate::test_support::logged(
        &warnings,
        "stat enrichment: no archives needing stats"
    ));
}

async fn enrich_warnings(pool: &PgPool) -> Vec<crate::log_buffer::LogEntry> {
    crate::test_support::warnings_while(enrich_single_archive_stats(
        pool,
        REPO_URL,
        &HashMap::new(),
        1,
        "a1",
        &shared::task_registry::TaskRegistry::default(),
    ))
    .await
}

#[tokio::test]
async fn stat_enrichment_logs_each_way_an_archive_can_fail() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let pool = closed_pool().await;

    let warnings = {
        let _guard = missing_borg();
        enrich_warnings(&pool).await
    };
    assert!(crate::test_support::logged(
        &warnings,
        "stat enrichment: failed to run borg info"
    ));

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\necho boom >&2\nexit 2\n").await;
    let warnings = enrich_warnings(&pool).await;
    assert!(crate::test_support::logged(
        &warnings,
        "stat enrichment: borg info non-zero exit"
    ));

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nprintf 'not json'\n").await;
    let warnings = enrich_warnings(&pool).await;
    assert!(crate::test_support::logged(
        &warnings,
        "stat enrichment: failed to parse borg info output"
    ));

    let (_borg_dir, _guard) = install_fake_script(
        "#!/bin/sh\nprintf '{\"archives\":[{\"stats\":{\"original_size\":5}}]}'\n",
    )
    .await;
    let warnings = enrich_warnings(&pool).await;
    assert!(crate::test_support::logged(
        &warnings,
        "stat enrichment: failed to update stats"
    ));
}
