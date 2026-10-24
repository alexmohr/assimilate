// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, time::Duration};

use chrono::DateTime;
use sqlx::PgPool;
use tracing::warn;

use super::{
    borg_ops::{LOCK_RETRY_INTERVAL, LOCK_RETRY_MAX_ATTEMPTS, extract_borg_error, is_lock_error},
    sync::publish_import_progress,
};
use crate::{
    api::archives::LOCK_WAIT_SECS, borg::Borg, error::ApiError, ws::ui_broadcast::UiBroadcast,
};

pub(super) fn parse_borg_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
        .map(|dt| dt.to_utc())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
                .map(|naive| naive.and_utc())
        })
        .ok()
}

pub(super) fn archive_hostname(archive: &serde_json::Value) -> Option<&str> {
    archive["hostname"]
        .as_str()
        .filter(|hostname| !hostname.is_empty())
}

pub(super) fn archive_finish_time(
    archive: &serde_json::Value,
    started_at: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    parse_borg_timestamp(archive["end"].as_str().unwrap_or_default()).unwrap_or_else(|| {
        archive["duration"]
            .as_f64()
            .and_then(|duration| std::time::Duration::try_from_secs_f64(duration).ok())
            .and_then(|duration| chrono::Duration::from_std(duration).ok())
            .and_then(|duration| started_at.checked_add_signed(duration))
            .unwrap_or(started_at)
    })
}

pub(super) fn archive_metadata_missing(archive: &serde_json::Value) -> bool {
    archive_hostname(archive).is_none() || archive["end"].as_str().is_none_or(str::is_empty)
}

pub(super) async fn fetch_archive_metadata_with_retry(
    borg_repo: &str,
    env: &HashMap<String, String>,
    archive_name: &str,
    timeout: Duration,
    retry_interval: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<serde_json::Value, ApiError> {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let args = [
        "info",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        repo_archive.as_str(),
    ];

    let mut attempt = 1;
    loop {
        let output = match tokio::time::timeout(
            timeout,
            Borg::new()
                .with_registry(task_registry.clone())
                .run(&args, env),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(ApiError::Internal(format!(
                    "failed to execute borg info for archive '{archive_name}': {e}"
                )));
            }
            Err(_) => {
                return Err(ApiError::Internal(format!(
                    "borg info timed out after {}s while reading archive '{archive_name}'",
                    timeout.as_secs()
                )));
            }
        };

        if output.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
                ApiError::Internal(format!(
                    "borg info returned malformed JSON for archive '{archive_name}': {e}"
                ))
            })?;
            return json
                .get("archives")
                .and_then(serde_json::Value::as_array)
                .and_then(|archives| archives.first())
                .cloned()
                .ok_or_else(|| {
                    ApiError::Internal(format!(
                        "borg info JSON missing archive entry for '{archive_name}'"
                    ))
                });
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !is_lock_error(&stderr) || attempt == LOCK_RETRY_MAX_ATTEMPTS {
            let summary = extract_borg_error(&stderr);
            return Err(ApiError::Internal(format!(
                "borg info failed for archive '{archive_name}': {summary}"
            )));
        }

        let retry_secs = retry_interval.as_secs();
        warn!(
            attempt,
            max = LOCK_RETRY_MAX_ATTEMPTS,
            archive = archive_name,
            "borg info lock contention, retrying in {retry_secs}s"
        );
        tokio::time::sleep(retry_interval).await;
        attempt = attempt.saturating_add(1);
    }
}

pub(super) struct HydrateArchivesArgs<'a> {
    pub(super) pool: &'a PgPool,
    pub(super) ui_broadcast: &'a UiBroadcast,
    pub(super) repo_id: i64,
    pub(super) borg_repo: &'a str,
    pub(super) env: &'a HashMap<String, String>,
    pub(super) timeout: Duration,
    pub(super) archives: &'a [&'a serde_json::Value],
    pub(super) task_registry: &'a shared::task_registry::TaskRegistry,
}

/// Fills in `hostname`/`end` for a single archive if missing, via a fresh `borg info`
/// lookup. Archives that already have both, or have no `name`, are returned unchanged.
pub(super) async fn hydrate_one_archive(
    archive: serde_json::Value,
    borg_repo: &str,
    env: &HashMap<String, String>,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<serde_json::Value, ApiError> {
    if !archive_metadata_missing(&archive) {
        return Ok(archive);
    }

    let archive_name = archive
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if archive_name.is_empty() {
        return Ok(archive);
    }

    let metadata = fetch_archive_metadata_with_retry(
        borg_repo,
        env,
        archive_name,
        timeout,
        LOCK_RETRY_INTERVAL,
        task_registry,
    )
    .await?;

    let mut merged = archive;
    for field in ["hostname", "end"] {
        let needs_update = merged
            .get(field)
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(str::is_empty));
        if needs_update
            && let Some(new_value) = metadata.get(field)
            && let Some(obj) = merged.as_object_mut()
        {
            obj.insert(field.to_string(), new_value.clone());
        }
    }
    Ok(merged)
}

pub(super) async fn hydrate_archives_with_metadata(
    args: HydrateArchivesArgs<'_>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let HydrateArchivesArgs {
        pool,
        ui_broadcast,
        repo_id,
        borg_repo,
        env,
        timeout,
        archives,
        task_registry,
    } = args;
    let total = archives.len();
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);

    let mut hydrated = Vec::with_capacity(total);
    let mut completed = 0usize;
    #[allow(
        clippy::unnecessary_to_owned,
        reason = "false positive: dropping .copied() makes `archive` a &&Value, and the .clone() \
                  inside hydrate_one_archive then clones the outer reference instead of producing \
                  an owned Value, which fails to type-check"
    )]
    for archive in archives.iter().copied() {
        let archive_name = archive
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let next_position = completed.saturating_add(1);
        let loading_msg = if archive_name.is_empty() {
            format!("Loading archive metadata... ({next_position}/{total})")
        } else {
            format!("Loading metadata for '{archive_name}' ({next_position}/{total})")
        };
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            i32::try_from(next_position).unwrap_or(i32::MAX),
            total_i32,
            Some(&loading_msg),
        )
        .await;

        let archive =
            hydrate_one_archive(archive.clone(), borg_repo, env, timeout, task_registry).await?;

        completed = completed.saturating_add(1);
        let loaded_msg = if archive_name.is_empty() {
            format!("Loading archive metadata... ({completed}/{total})")
        } else {
            format!("Loaded metadata for '{archive_name}' ({completed}/{total})")
        };
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            i32::try_from(completed).unwrap_or(i32::MAX),
            total_i32,
            Some(&loaded_msg),
        )
        .await;
        hydrated.push(archive);
    }

    Ok(hydrated)
}

#[cfg(test)]
mod tests {
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

        let (_borg_dir, _guard) =
            install_fake_script("#!/bin/sh\nprintf '{\"archives\":[]}'\n").await;
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
            result =
                Some(archive_metadata(Duration::from_secs(30), Duration::from_millis(10)).await);
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
        while let Ok(shared::protocol::ServerToUi::ImportProgress { message, .. }) =
            events.try_recv()
        {
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
}
