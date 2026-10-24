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
mod tests;
