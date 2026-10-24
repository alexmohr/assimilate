// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, time::Duration};

use sqlx::PgPool;
use tracing::{info, warn};

use super::{
    archive_metadata::{
        HydrateArchivesArgs, archive_finish_time, archive_hostname, hydrate_archives_with_metadata,
        parse_borg_timestamp,
    },
    archive_stats::enrich_archive_stats_background,
    borg_ops::{
        BorgListRetryArgs, LOCK_RETRY_INTERVAL, get_borg_list_stage_timeout, get_borg_timeout,
        run_borg_list_with_retry,
    },
    indexing::{QueueArchiveIndexingArgs, queue_archive_indexing},
};
use crate::{
    AppState, RepoLock, api::archives::LOCK_WAIT_SECS, borg::Borg, db, error::ApiError,
    ws::ui_broadcast::UiBroadcast,
};

/// Refreshes the authoritative repo statistics from `borg info --json`
/// (`cache.stats`) plus the archive count from the just-completed `borg list`.
/// These are the single source of truth for repo size/archive numbers; failures
/// are logged but never abort a sync.
pub(super) async fn refresh_repo_info_stats(
    pool: &PgPool,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    repo_id: i64,
    archive_count: i64,
    timeout: Duration,
    task_registry: &shared::task_registry::TaskRegistry,
) {
    let args = [
        "info",
        "--json",
        "--lock-wait",
        LOCK_WAIT_SECS,
        "--",
        borg_repo,
    ];
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
            warn!(repo_id, error = %e, "failed to run borg info for repo stats");
            return;
        }
        Err(_) => {
            let timeout_secs = timeout.as_secs();
            warn!(
                repo_id,
                timeout_secs, "borg info timed out refreshing repo stats"
            );
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(repo_id, %stderr, "borg info (repo stats) failed");
        return;
    }

    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            warn!(repo_id, error = %e, "failed to parse borg info repo stats");
            return;
        }
    };

    let stats = json.get("cache").and_then(|v| v.get("stats"));
    let info_stats = db::RepoInfoStats {
        original_size: stats
            .and_then(|s| s.get("total_size"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        compressed_size: stats
            .and_then(|s| s.get("total_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        deduplicated_size: stats
            .and_then(|s| s.get("unique_csize"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        total_chunks: stats
            .and_then(|s| s.get("total_chunks"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        unique_chunks: stats
            .and_then(|s| s.get("total_unique_chunks"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        archive_count,
    };

    if let Err(e) = db::update_repo_info_stats(pool, repo_id, &info_stats).await {
        warn!(repo_id, error = %e, "failed to persist repo info stats");
    }
}

pub(super) async fn publish_import_progress(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    progress: i32,
    total: i32,
    message: Option<&str>,
) {
    ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
        repo_id,
        progress,
        total,
        message: message.map(str::to_owned),
    });
    let _ =
        db::update_repo_import_progress(pool, repo_id, i64::from(progress), i64::from(total)).await;
    let _ = db::set_import_status_message(pool, repo_id, message).await;
}

/// Clear the import progress state for a repository.
pub async fn clear_import_progress_state(pool: &PgPool, ui_broadcast: &UiBroadcast, repo_id: i64) {
    ui_broadcast.clear_import_progress(repo_id);
    let _ = db::update_repo_import_progress(pool, repo_id, 0, 0).await;
    let _ = db::set_import_status_message(pool, repo_id, None).await;
}

/// Mark a repository as being synced by the server, returning a guard that
/// clears the marker when dropped (including on panic), so a cancelled or
/// panicking task doesn't leave the entry permanently "active". Callers must
/// clear via the guard (`clear_now`), not [`clear_server_sync_op`]: the
/// guard's token-checked clear can't clobber a *different* `ServerSync` op
/// that's since claimed the same `repo_id` (e.g. a scheduled sync started
/// after this manual one was cancelled before ever acquiring `repo_lock`),
/// whereas an unconditional plain clear could.
#[must_use]
pub async fn set_server_sync_op(
    state: &AppState,
    repo_id: i64,
) -> crate::repo_op_tracker::RepoOpGuard {
    let guard = state
        .repo_op_tracker
        .set_guarded(
            repo_id,
            shared::protocol::RepoOpKind::ServerSync,
            "server".to_owned(),
            state.task_registry.clone(),
        )
        .await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged {
            repo_id,
            op: state.repo_op_tracker.get(repo_id).await,
        });
    guard
}

/// Shared tail for every task that claimed [`set_server_sync_op`]'s guard:
/// marks the import task finished, clears the guard's own entry (a no-op if
/// a different operation already reclaimed this `repo_id`), and broadcasts
/// the cleared state to the UI.
pub async fn finish_server_sync_task(
    import_tasks: &crate::ImportTaskRegistry,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    task_id: u64,
    op_clear_guard: crate::repo_op_tracker::RepoOpGuard,
) {
    import_tasks.finish(repo_id, task_id).await;
    op_clear_guard.clear_now().await;
    ui_broadcast.send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });
}

/// Forcibly clear the server-sync operation marker for a repository,
/// regardless of which operation currently holds it. Only for
/// [`reset_import`]'s "unstick a stuck import" recovery path, which is
/// meant to override whatever's there - anything holding its own
/// [`set_server_sync_op`] guard should clear via `clear_now` instead so it
/// only ever clears its own operation.
pub(super) async fn clear_server_sync_op(state: &AppState, repo_id: i64) {
    state.repo_op_tracker.clear(repo_id).await;
    state
        .ui_broadcast
        .send(shared::protocol::ServerToUi::RepoOpChanged { repo_id, op: None });
}

/// Builds backup-report rows for a set of borg archives, resolving each archive's
/// owning agent by hostname (creating an imported placeholder when unmatched).
///
/// Shared by full and incremental sync so agent resolution lives in one place.
/// Hostname and end time are expected to be present by the time this runs.
/// Progress is published per archive against `total = archives.len()`.
pub(super) async fn build_import_reports(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    archives: &[&serde_json::Value],
) -> Result<Vec<db::InsertReportParams>, ApiError> {
    let total = archives.len();

    let mut hostname_cache: HashMap<String, (i64, bool)> = HashMap::new();
    let mut report_params = Vec::with_capacity(total);
    for (processed, archive) in archives.iter().enumerate() {
        let name = archive
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let Some(hostname) = archive_hostname(archive) else {
            warn!(repo_id, archive = %name, "skipping archive with missing hostname metadata");
            continue;
        };

        let (agent_id, matched) = if let Some(&cached) = hostname_cache.get(hostname) {
            cached
        } else {
            let (agent, matched) = match db::resolve_agent_for_hostname(pool, hostname).await? {
                db::ResolveResult::ExactMatch(c) | db::ResolveResult::PatternMatch(c) => (c, true),
                db::ResolveResult::Unmatched => {
                    let c = db::get_or_create_agent_by_hostname(pool, hostname).await?;
                    (c, false)
                }
            };
            let entry = (agent.id, matched);
            hostname_cache.insert(hostname.to_string(), entry);
            entry
        };

        let Some(started_at) = parse_borg_timestamp(
            archive
                .get("start")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        ) else {
            warn!(repo_id, archive = %name, "skipping archive with unparseable start timestamp");
            continue;
        };
        let finished_at = archive_finish_time(archive, started_at);
        let duration_secs = finished_at
            .signed_duration_since(started_at)
            .num_seconds()
            .max(0);

        let processed_count = i32::try_from(processed.saturating_add(1)).unwrap_or(i32::MAX);
        info!(repo_id, archive = %name, processed = processed_count, total, "archive imported");
        let progress_msg = format!("Imported \u{2018}{name}\u{2019} ({processed_count}/{total})");
        publish_import_progress(
            pool,
            ui_broadcast,
            repo_id,
            processed_count,
            i32::try_from(total).unwrap_or(i32::MAX),
            Some(&progress_msg),
        )
        .await;

        report_params.push(db::InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: None,
            started_at,
            finished_at,
            status: shared::types::BackupStatus::Success,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched,
            archive_name: Some(name.to_string()),
            borg_command: None,
            run_id: None,
        });
    }

    Ok(report_params)
}

#[derive(Clone, Copy)]
pub(super) enum SyncMode<'a> {
    /// Full sync: import every archive and prune DB records for archives no
    /// longer present in the repository.
    Existing,
    /// Incremental sync after a backup run: import only archives not already
    /// known (pruning the ones gone upstream, as every mode does), and queue
    /// content indexing for the newly imported archives.
    New {
        repo_lock: &'a RepoLock,
        /// The archive the run itself reported creating, which is never
        /// pruned: the run is the authority that it exists, even if the
        /// listing (say, of a repository mid-relocation) does not show it.
        just_written: Option<&'a str>,
    },
}

/// Known archives `borg list` no longer reports, except `just_written`.
pub(super) fn stale_archive_names(
    known_names: &std::collections::HashSet<String>,
    borg_names: &std::collections::HashSet<String>,
    just_written: Option<&str>,
) -> Vec<String> {
    known_names
        .difference(borg_names)
        .filter(|name| just_written.is_none_or(|kept| kept != name.as_str()))
        .cloned()
        .collect()
}

/// Returns `true` when a borg archive JSON entry has a non-empty name that is
/// not already recorded for the repository (i.e. it is new and should be
/// imported during an incremental sync).
pub(super) fn is_unknown_archive(
    archive: &serde_json::Value,
    known_names: &std::collections::HashSet<String>,
) -> bool {
    archive["name"]
        .as_str()
        .is_some_and(|n| !n.is_empty() && !known_names.contains(n))
}

pub(super) struct ArchiveSyncDiff<'a> {
    pub(super) borg_names: std::collections::HashSet<String>,
    pub(super) removed: u64,
    pub(super) to_import: Vec<&'a serde_json::Value>,
}

/// Compares the archives reported by `borg list` against what's already
/// known for this repository, and prunes local records for archives that no
/// longer exist upstream. Both modes prune: the incremental sync runs after a
/// backup, whose own `borg prune` is what removes archives in the first place.
pub(super) async fn partition_archives_to_sync<'a>(
    pool: &PgPool,
    repo_id: i64,
    archives: &'a [serde_json::Value],
    mode: SyncMode<'_>,
) -> Result<ArchiveSyncDiff<'a>, ApiError> {
    let borg_names: std::collections::HashSet<String> = archives
        .iter()
        .filter_map(|a| a["name"].as_str())
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect();

    let known_names = db::list_archive_names_for_repo(pool, repo_id).await?;

    // A `borg list` that comes back empty while the DB still has archive
    // records on file is far more likely to mean the repo is temporarily
    // unreachable, relocated, or otherwise misreporting than that every
    // archive genuinely vanished upstream. Treat it as a hard error rather
    // than pruning every existing archive (and its backup reports): see
    // the matching guard in `run_borg_list_with_retry` for malformed JSON.
    if borg_names.is_empty() && !known_names.is_empty() {
        return Err(ApiError::Internal(format!(
            "borg list returned 0 archives but {} were previously known for this repository; \
             refusing to prune all archive records",
            known_names.len()
        )));
    }
    let just_written = match mode {
        SyncMode::Existing => None,
        SyncMode::New { just_written, .. } => just_written,
    };
    let stale = stale_archive_names(&known_names, &borg_names, just_written);
    let removed = db::delete_archive_records_by_names(pool, repo_id, &stale).await?;
    if removed > 0 {
        info!(repo_id, removed, "removed stale archives during sync");
    }

    let to_import: Vec<&serde_json::Value> = match mode {
        SyncMode::Existing => archives.iter().collect(),
        SyncMode::New { .. } => archives
            .iter()
            .filter(|a| is_unknown_archive(a, &known_names))
            .collect(),
    };

    Ok(ArchiveSyncDiff {
        borg_names,
        removed,
        to_import,
    })
}

pub(super) struct ImportOutcome {
    pub(super) processed: u64,
    pub(super) archive_names: Vec<String>,
}

/// Hydrates metadata for the archives selected for import, builds and
/// persists their backup report rows, and kicks off background stat
/// enrichment. Returns how many reports were saved and the archive names
/// that were imported.
#[allow(
    clippy::too_many_arguments,
    reason = "grouping these into a struct would obscure the call site more than it would clarify \
              it; all params are single-use scalars/refs from the caller's own locals"
)]
pub(super) async fn import_and_persist_archives(
    pool: &PgPool,
    ui_broadcast: &UiBroadcast,
    repo_id: i64,
    borg_repo: &str,
    env: &std::collections::HashMap<String, String>,
    timeout: Duration,
    to_import: &[&serde_json::Value],
    importing_msg: &str,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<ImportOutcome, ApiError> {
    let total = to_import.len();
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        i32::try_from(total).unwrap_or(i32::MAX),
        Some(importing_msg),
    )
    .await;

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        i32::try_from(total).unwrap_or(i32::MAX),
        Some("Loading archive metadata..."),
    )
    .await;
    let hydrated_archives = hydrate_archives_with_metadata(HydrateArchivesArgs {
        pool,
        ui_broadcast,
        repo_id,
        borg_repo,
        env,
        timeout,
        archives: to_import,
        task_registry,
    })
    .await?;
    let hydrated_refs: Vec<&serde_json::Value> = hydrated_archives.iter().collect();

    let report_params = build_import_reports(pool, ui_broadcast, repo_id, &hydrated_refs).await?;

    let processed = u64::try_from(report_params.len()).unwrap_or(u64::MAX);
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);
    let save_msg = format!("Saving {processed} backup reports\u{2026}");
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some(&save_msg),
    )
    .await;

    let archive_names: Vec<String> = report_params
        .iter()
        .filter_map(|params| params.archive_name.clone())
        .collect();
    db::bulk_insert_backup_reports(pool, &report_params).await?;
    enrich_archive_stats_background(
        pool.clone(),
        borg_repo.to_owned(),
        env.clone(),
        repo_id,
        archive_names.clone(),
        background_task_tracker,
        task_registry.clone(),
    );

    Ok(ImportOutcome {
        processed,
        archive_names,
    })
}

pub(super) async fn sync_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    mode: SyncMode<'_>,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(u64, u64), ApiError> {
    let (borg_repo, env) =
        crate::api::archives::get_repo_env(pool, encryption_key, repo_id).await?;
    let timeout = get_borg_timeout(pool).await;
    let stage_timeout = get_borg_list_stage_timeout(pool).await;

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        0,
        0,
        Some("Listing archives\u{2026}"),
    )
    .await;

    let archives = run_borg_list_with_retry(BorgListRetryArgs {
        borg_repo: &borg_repo,
        env: &env,
        timeout,
        stage_timeout,
        retry_interval: LOCK_RETRY_INTERVAL,
        pool,
        ui_broadcast,
        repo_id,
        task_registry,
    })
    .await?;

    let ArchiveSyncDiff {
        borg_names,
        removed,
        to_import,
    } = partition_archives_to_sync(pool, repo_id, &archives, mode).await?;

    let repo_archive_count = i64::try_from(borg_names.len()).unwrap_or(i64::MAX);

    if to_import.is_empty() {
        refresh_repo_info_stats(
            pool,
            &borg_repo,
            &env,
            repo_id,
            repo_archive_count,
            timeout,
            task_registry,
        )
        .await;
        return Ok((0, removed));
    }

    let total = to_import.len();
    let importing_msg = match mode {
        SyncMode::Existing => format!("Importing {total} archives\u{2026}"),
        SyncMode::New { .. } => format!("Importing {total} new archives\u{2026}"),
    };

    let ImportOutcome {
        processed,
        archive_names,
    } = import_and_persist_archives(
        pool,
        ui_broadcast,
        repo_id,
        &borg_repo,
        &env,
        timeout,
        &to_import,
        &importing_msg,
        background_task_tracker,
        task_registry,
    )
    .await?;

    finish_archive_sync(FinishArchiveSyncArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        borg_repo: &borg_repo,
        env: &env,
        mode,
        background_task_tracker,
        task_registry,
        archive_names,
        repo_archive_count,
        timeout,
        total,
        processed,
    })
    .await;

    Ok((processed, removed))
}

pub(super) struct FinishArchiveSyncArgs<'a> {
    pub(super) pool: &'a PgPool,
    pub(super) encryption_key: &'a [u8; 32],
    pub(super) repo_id: i64,
    pub(super) ui_broadcast: &'a UiBroadcast,
    pub(super) borg_repo: &'a str,
    pub(super) env: &'a std::collections::HashMap<String, String>,
    pub(super) mode: SyncMode<'a>,
    pub(super) background_task_tracker: &'a crate::background_tasks::BackgroundTaskTracker,
    pub(super) task_registry: &'a shared::task_registry::TaskRegistry,
    pub(super) archive_names: Vec<String>,
    pub(super) repo_archive_count: i64,
    pub(super) timeout: Duration,
    pub(super) total: usize,
    pub(super) processed: u64,
}

/// Queues content indexing for newly-imported archives (incremental sync only),
/// refreshes the repo's authoritative stats, and logs completion. Split out of
/// [`sync_archives`] to stay under clippy's function-length limit.
pub(super) async fn finish_archive_sync(args: FinishArchiveSyncArgs<'_>) {
    let FinishArchiveSyncArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        borg_repo,
        env,
        mode,
        background_task_tracker,
        task_registry,
        archive_names,
        repo_archive_count,
        timeout,
        total,
        processed,
    } = args;

    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);

    if let SyncMode::New { repo_lock, .. } = mode {
        queue_archive_indexing(QueueArchiveIndexingArgs {
            pool,
            encryption_key,
            repo_id,
            archive_names: &archive_names,
            repo_lock,
            background_task_tracker,
            task_registry,
            sync_kind: "incremental sync",
        })
        .await;
    }

    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some("Refreshing repository statistics\u{2026}"),
    )
    .await;
    refresh_repo_info_stats(
        pool,
        borg_repo,
        env,
        repo_id,
        repo_archive_count,
        timeout,
        task_registry,
    )
    .await;

    match mode {
        SyncMode::Existing => {
            info!(
                repo_id,
                imported = processed,
                total,
                "synced existing archives"
            );
        }
        SyncMode::New { .. } => {
            info!(
                repo_id,
                added = processed,
                total,
                "incremental sync complete"
            );
        }
    }
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn sync_existing_archives(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    repo_id: i64,
    ui_broadcast: &UiBroadcast,
    background_task_tracker: &crate::background_tasks::BackgroundTaskTracker,
    task_registry: &shared::task_registry::TaskRegistry,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        SyncMode::Existing,
        background_task_tracker,
        task_registry,
    )
    .await
}

/// Reconciles the database with the repository after a backup run: imports
/// archives it does not know yet and drops records of archives the run's
/// `borg prune` removed, never `just_written` (the archive the run reported
/// creating). Does not take the repository lock itself; the caller must
/// already hold it.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn sync_new_archives(
    state: &AppState,
    repo_id: i64,
    just_written: Option<&str>,
) -> Result<(u64, u64), ApiError> {
    sync_archives(
        &state.pool,
        &state.encryption_key,
        repo_id,
        &state.ui_broadcast,
        SyncMode::New {
            repo_lock: &state.repo_lock,
            just_written,
        },
        &state.background_task_tracker,
        &state.task_registry,
    )
    .await
}

#[cfg(test)]
mod tests;
