// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::Datelike as _;

use super::*;
use crate::api::repos::test_helpers::{
    REPO_URL, closed_pool, install_fake_script, locked_once_then, missing_borg,
};

#[test]
fn parse_borg_timestamp_rfc3339() {
    let ts = parse_borg_timestamp("2024-03-15T10:30:00+00:00");
    assert!(ts.is_some());
    let dt = ts.unwrap();
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 3);
    assert_eq!(dt.day(), 15);
}

#[test]
fn parse_borg_timestamp_naive_with_fraction() {
    let ts = parse_borg_timestamp("2024-06-01T08:00:00.123456");
    assert!(ts.is_some());
}

#[test]
fn parse_borg_timestamp_naive_without_fraction() {
    let ts = parse_borg_timestamp("2024-06-01T08:00:00");
    assert!(ts.is_some());
}

#[test]
fn parse_borg_timestamp_empty_returns_none() {
    assert!(parse_borg_timestamp("").is_none());
}

#[test]
fn parse_borg_timestamp_invalid_returns_none() {
    assert!(parse_borg_timestamp("not-a-date").is_none());
    assert!(parse_borg_timestamp("2024-13-01T00:00:00").is_none());
}

#[test]
fn archive_hostname_returns_hostname_when_present() {
    let archive = serde_json::json!({"hostname": "web-01.example.com"});
    assert_eq!(archive_hostname(&archive), Some("web-01.example.com"));
}

#[test]
fn archive_hostname_returns_none_when_missing_or_empty() {
    assert_eq!(archive_hostname(&serde_json::json!({})), None);
    assert_eq!(archive_hostname(&serde_json::json!({"hostname": ""})), None);
    assert_eq!(
        archive_hostname(&serde_json::json!({"hostname": null})),
        None
    );
}

#[test]
fn archive_finish_time_uses_end_timestamp_when_present() {
    let archive = serde_json::json!({"end": "2024-06-01T08:00:00+00:00"});
    let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
        .unwrap()
        .to_utc();
    let finished = archive_finish_time(&archive, started_at);
    assert_eq!(finished, started_at + chrono::Duration::hours(1));
}

#[test]
fn archive_finish_time_falls_back_to_duration_when_end_missing() {
    let archive = serde_json::json!({"duration": 3600.0});
    let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
        .unwrap()
        .to_utc();
    let finished = archive_finish_time(&archive, started_at);
    assert_eq!(finished, started_at + chrono::Duration::hours(1));
}

#[test]
fn archive_finish_time_falls_back_to_started_at_when_no_end_or_duration() {
    let archive = serde_json::json!({});
    let started_at = chrono::DateTime::parse_from_rfc3339("2024-06-01T07:00:00+00:00")
        .unwrap()
        .to_utc();
    let finished = archive_finish_time(&archive, started_at);
    assert_eq!(finished, started_at);
}

#[test]
fn archive_metadata_missing_true_when_hostname_or_end_missing() {
    assert!(archive_metadata_missing(&serde_json::json!({})));
    assert!(archive_metadata_missing(
        &serde_json::json!({"hostname": "h1"})
    ));
    assert!(archive_metadata_missing(
        &serde_json::json!({"end": "2024-01-01T00:00:00Z"})
    ));
}

#[test]
fn archive_metadata_missing_false_when_hostname_and_end_present() {
    assert!(!archive_metadata_missing(&serde_json::json!({
        "hostname": "h1",
        "end": "2024-01-01T00:00:00Z"
    })));
}

async fn archive_metadata(
    timeout: Duration,
    retry_interval: Duration,
) -> Result<serde_json::Value, ApiError> {
    fetch_archive_metadata_with_retry(
        REPO_URL,
        &HashMap::new(),
        "a1",
        timeout,
        retry_interval,
        &shared::task_registry::TaskRegistry::default(),
    )
    .await
}

#[tokio::test]
async fn fetch_archive_metadata_names_the_archive_in_each_failure() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let timeout = Duration::from_secs(30);
    let interval = Duration::from_millis(10);

    let result = {
        let _guard = missing_borg();
        archive_metadata(timeout, interval).await
    };
    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.starts_with("failed to execute borg info for archive 'a1'")),
        "{result:?}"
    );

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nsleep 5\n").await;
    let result = archive_metadata(Duration::from_millis(200), interval).await;
    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.ends_with("while reading archive 'a1'")),
        "{result:?}"
    );

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nprintf 'not json'\n").await;
    let result = archive_metadata(timeout, interval).await;
    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg.starts_with("borg info returned malformed JSON for archive 'a1'")),
        "{result:?}"
    );

    let (_borg_dir, _guard) = install_fake_script("#!/bin/sh\nprintf '{\"archives\":[]}'\n").await;
    let result = archive_metadata(timeout, interval).await;
    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg == "borg info JSON missing archive entry for 'a1'"),
        "{result:?}"
    );

    let (_borg_dir, _guard) =
        install_fake_script("#!/bin/sh\necho 'Archive a1 does not exist' >&2\nexit 1\n").await;
    let result = archive_metadata(timeout, interval).await;
    assert!(
        matches!(result, Err(ApiError::Internal(ref msg)) if msg == "borg info failed for archive 'a1': Archive a1 does not exist"),
        "{result:?}"
    );
}

#[tokio::test]
async fn fetch_archive_metadata_waits_out_a_held_repository_lock() {
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_script(&locked_once_then(
        "{\"archives\":[{\"name\":\"a1\",\"hostname\":\"web1\"}]}",
    ))
    .await;

    let mut result = None;
    let warnings = crate::test_support::warnings_while(async {
        result = Some(archive_metadata(Duration::from_secs(30), Duration::from_millis(10)).await);
    })
    .await;

    let metadata = result.expect("ran").expect("the second attempt succeeds");
    assert_eq!(
        metadata,
        serde_json::json!({"name": "a1", "hostname": "web1"})
    );
    assert!(crate::test_support::logged(
        &warnings,
        "borg info lock contention"
    ));
}

#[tokio::test]
async fn hydrating_an_archive_without_a_name_leaves_it_as_it_is() {
    let ui_broadcast = UiBroadcast::new();
    let mut events = ui_broadcast.subscribe();
    let nameless = serde_json::json!({});

    let hydrated = hydrate_archives_with_metadata(HydrateArchivesArgs {
        pool: &closed_pool().await,
        ui_broadcast: &ui_broadcast,
        repo_id: 1,
        borg_repo: REPO_URL,
        env: &HashMap::new(),
        timeout: Duration::from_secs(30),
        archives: &[&nameless],
        task_registry: &shared::task_registry::TaskRegistry::default(),
    })
    .await
    .unwrap();

    assert_eq!(hydrated, vec![nameless]);
    let mut messages = Vec::new();
    while let Ok(shared::protocol::ServerToUi::ImportProgress { message, .. }) = events.try_recv() {
        messages.push(message.unwrap_or_default());
    }
    assert_eq!(
        messages,
        [
            "Loading archive metadata... (1/1)",
            "Loading archive metadata... (1/1)"
        ]
    );
}
