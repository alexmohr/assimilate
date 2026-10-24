// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use futures_util::future::join_all;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::{api::archives::LOCK_WAIT_SECS, borg::Borg, db};

pub(super) fn enrich_archive_stats_background(
    pool: PgPool,
    borg_repo: String,
    env: std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_names: Vec<String>,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: shared::task_registry::TaskRegistry,
) {
    background_task_tracker.spawn_tracked(async move {
        // Immutable archives that already have stats never change, so only query
        // borg for the ones still missing them.
        let needing = match db::list_archive_names_needing_stats(&pool, repo_id).await {
            Ok(names) => names,
            Err(e) => {
                warn!(repo_id, error = %e, "stat enrichment: no archives needing stats");
                return;
            }
        };
        let archive_names: Vec<String> = archive_names
            .into_iter()
            .filter(|name| needing.contains(name))
            .collect();
        if archive_names.is_empty() {
            return;
        }
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        let futures: Vec<_> = archive_names
            .into_iter()
            .map(|archive_name| {
                let pool = pool.clone();
                let borg_repo = borg_repo.clone();
                let env = env.clone();
                let semaphore = std::sync::Arc::clone(&semaphore);
                let task_registry = task_registry.clone();
                async move {
                    let _permit = semaphore.acquire_owned().await;
                    enrich_single_archive_stats(
                        &pool,
                        &borg_repo,
                        &env,
                        repo_id,
                        &archive_name,
                        &task_registry,
                    )
                    .await;
                }
            })
            .collect();
        join_all(futures).await;
        info!(repo_id, "stat enrichment: completed");
    });
}

/// Runs `borg info --json` for a single archive and returns the parsed
/// top-level JSON document, logging and returning `None` on any failure
/// (spawn error, non-zero exit, unparseable output).
pub(super) async fn run_borg_info_for_stats(
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_name: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Option<serde_json::Value> {
    let repo_archive = format!("{borg_repo}::{archive_name}");
    let output = match Borg::new()
        .with_registry(task_registry.clone())
        .run(
            &[
                "info",
                "--json",
                "--lock-wait",
                LOCK_WAIT_SECS,
                &repo_archive,
            ],
            env,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => {
            warn!(
                repo_id,
                archive = %archive_name,
                error = %e,
                "stat enrichment: failed to run borg info"
            );
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(
            repo_id,
            archive = %archive_name,
            %stderr,
            "stat enrichment: borg info non-zero exit"
        );
        return None;
    }

    match serde_json::from_slice(&output.stdout) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(
                repo_id,
                archive = %archive_name,
                error = %e,
                "stat enrichment: failed to parse borg info output"
            );
            None
        }
    }
}

pub(super) fn parse_archive_stats(
    json: &serde_json::Value,
    info: &serde_json::Value,
) -> db::ArchiveStats {
    let raw_stats = info.get("stats");
    db::ArchiveStats {
        original_size: raw_stats
            .and_then(|s| s.get("original_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        compressed_size: raw_stats
            .and_then(|s| s.get("compressed_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        deduplicated_size: raw_stats
            .and_then(|s| s.get("deduplicated_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        files_processed: raw_stats
            .and_then(|s| s.get("nfiles"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        duration_secs: info
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .map_or(0, |d| {
                format!("{:.0}", d.round()).parse::<i64>().unwrap_or(0)
            }),
        repo_unique_csize: json
            .get("cache")
            .and_then(|v| v.get("stats"))
            .and_then(|v| v.get("unique_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
    }
}

pub(super) async fn enrich_single_archive_stats(
    pool: &PgPool,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_name: &str,
    task_registry: &shared::task_registry::TaskRegistry,
) {
    let Some(json) =
        run_borg_info_for_stats(borg_repo, env, repo_id, archive_name, task_registry).await
    else {
        return;
    };

    let Some(info) = json
        .get("archives")
        .and_then(serde_json::Value::as_array)
        .and_then(|a| a.first())
    else {
        warn!(
            repo_id,
            archive = %archive_name,
            "stat enrichment: borg info returned no archive entry"
        );
        return;
    };

    let archive_stats = parse_archive_stats(&json, info);

    if let Err(e) =
        db::update_backup_report_stats(pool, repo_id, archive_name, &archive_stats).await
    {
        warn!(
            repo_id,
            archive = %archive_name,
            error = %e,
            "stat enrichment: failed to update stats"
        );
    } else {
        info!(
            repo_id,
            archive = %archive_name,
            original_size = archive_stats.original_size,
            "stat enrichment: updated archive stats"
        );
    }
}

#[cfg(test)]
mod tests;
