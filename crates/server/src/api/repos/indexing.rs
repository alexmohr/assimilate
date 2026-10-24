// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use futures_util::future::join_all;
use sqlx::PgPool;
use tracing::{info, warn};

use super::sync::publish_import_progress;
use crate::{RepoLock, archive_index, db, ws::ui_broadcast::UiBroadcast};

pub(super) struct QueueArchiveIndexingArgs<'a> {
    pub(super) pool: &'a PgPool,
    pub(super) encryption_key: &'a [u8; 32],
    pub(super) repo_id: i64,
    pub(super) archive_names: &'a [String],
    pub(super) repo_lock: &'a RepoLock,
    pub(super) background_task_tracker: &'a crate::background_tasks::BackgroundTaskTracker,
    pub(super) task_registry: &'a shared::task_registry::TaskRegistry,
    pub(super) sync_kind: &'a str,
}

pub(super) async fn queue_archive_indexing(args: QueueArchiveIndexingArgs<'_>) {
    let QueueArchiveIndexingArgs {
        pool,
        encryption_key,
        repo_id,
        archive_names,
        repo_lock,
        background_task_tracker,
        task_registry,
        sync_kind,
    } = args;
    join_all(archive_names.iter().map(|archive_name| {
        let pool = pool.clone();
        let repo_lock = repo_lock.clone();
        let task_registry = task_registry.clone();
        async move {
            if let Err(e) = archive_index::ensure_indexed(
                pool,
                *encryption_key,
                repo_id,
                archive_name.clone(),
                repo_lock,
                background_task_tracker,
                task_registry,
            )
            .await
            {
                warn!(
                    repo_id,
                    archive = %archive_name,
                    sync_kind,
                    error = %e,
                    "failed to queue archive indexing"
                );
            }
        }
    }))
    .await;
}

/// Builds the content index for every archive in the repository that is not
/// already fully indexed, one archive at a time, broadcasting progress so the
/// UI can show which archive (and how many files) is currently being processed.
/// Already-indexed archives are skipped: borg archives are immutable, so a
/// finished index never needs rebuilding.
pub(super) async fn index_archives_with_progress(
    pool: PgPool,
    encryption_key: [u8; 32],
    repo_id: i64,
    ui_broadcast: UiBroadcast,
    repo_lock: RepoLock,
    repo_lock_held: bool,
    task_registry: shared::task_registry::TaskRegistry,
) {
    let all = match db::list_archive_names_for_repo(&pool, repo_id).await {
        Ok(names) => names,
        Err(e) => {
            warn!(repo_id, error = %e, "content indexing: failed to list archives");
            return;
        }
    };
    let done = match archive_index::list_indexed_archive_names(&pool, repo_id).await {
        Ok(names) => names,
        Err(e) => {
            warn!(repo_id, error = %e, "content indexing: failed to list indexed archives");
            return;
        }
    };

    let mut pending: Vec<String> = all.difference(&done).cloned().collect();
    pending.sort_unstable();
    let total = pending.len();
    if total == 0 {
        return;
    }
    let total_i32 = i32::try_from(total).unwrap_or(i32::MAX);
    info!(
        repo_id,
        total, "content indexing: starting full resync index"
    );

    for (index, archive_name) in pending.iter().enumerate() {
        index_one_archive(IndexOneArchiveArgs {
            pool: &pool,
            encryption_key: &encryption_key,
            repo_id,
            ui_broadcast: &ui_broadcast,
            repo_lock: &repo_lock,
            repo_lock_held,
            archive_name,
            index,
            total,
            total_i32,
            task_registry: &task_registry,
        })
        .await;
    }

    publish_import_progress(
        &pool,
        &ui_broadcast,
        repo_id,
        total_i32,
        total_i32,
        Some("Indexing complete"),
    )
    .await;
    info!(repo_id, total, "content indexing: completed");
}

pub(super) struct IndexOneArchiveArgs<'a> {
    pub(super) pool: &'a PgPool,
    pub(super) encryption_key: &'a [u8; 32],
    pub(super) repo_id: i64,
    pub(super) ui_broadcast: &'a UiBroadcast,
    pub(super) repo_lock: &'a RepoLock,
    pub(super) repo_lock_held: bool,
    pub(super) archive_name: &'a str,
    pub(super) index: usize,
    pub(super) total: usize,
    pub(super) total_i32: i32,
    pub(super) task_registry: &'a shared::task_registry::TaskRegistry,
}

/// Indexes a single archive's contents, broadcasting progress as it goes.
///
/// The progress bar tracks *completed* archives, so it never reaches 100%
/// while an archive (including the last one) is still being scanned. The
/// live file count and current path keep the badge visibly moving even
/// while a single large archive is being scanned.
pub(super) async fn index_one_archive(args: IndexOneArchiveArgs<'_>) {
    let IndexOneArchiveArgs {
        pool,
        encryption_key,
        repo_id,
        ui_broadcast,
        repo_lock,
        repo_lock_held,
        archive_name,
        index,
        total,
        total_i32,
        task_registry,
    } = args;

    let completed = i32::try_from(index).unwrap_or(i32::MAX);
    let human_position = index.saturating_add(1);
    let archive_msg =
        format!("Indexing contents of \u{2018}{archive_name}\u{2019} ({human_position}/{total})");
    publish_import_progress(
        pool,
        ui_broadcast,
        repo_id,
        completed,
        total_i32,
        Some(&archive_msg),
    )
    .await;

    let archive_id = match archive_index::ensure_index_job(pool, repo_id, archive_name).await {
        Ok(archive_id) => archive_id,
        Err(e) => {
            warn!(repo_id, archive = %archive_name, error = %e, "content index job failed");
            return;
        }
    };

    let mut on_progress = |file_count: u64, current: Option<&str>| {
        let message = current.map_or_else(
            || {
                format!(
                    "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \u{2014} \
                     {file_count} files"
                )
            },
            |path| {
                format!(
                    "Indexing \u{2018}{archive_name}\u{2019} ({human_position}/{total}) \u{2014} \
                     {file_count} files \u{00b7} {path}"
                )
            },
        );
        ui_broadcast.send(shared::protocol::ServerToUi::ImportProgress {
            repo_id,
            progress: completed,
            total: total_i32,
            message: Some(message),
        });
    };

    let indexing = async {
        if repo_lock_held {
            archive_index::run_indexing_with_lock_held(
                pool,
                encryption_key,
                repo_id,
                archive_name,
                &mut on_progress,
                task_registry,
            )
            .await
        } else {
            archive_index::run_indexing(
                pool,
                encryption_key,
                repo_id,
                archive_name,
                repo_lock,
                &mut on_progress,
                task_registry,
            )
            .await
        }
    };
    // A failing (or panicking) archive is recorded as failed and logged, and
    // the batch moves on to the next archive.
    archive_index::supervise_index_job(pool, archive_id, archive_name, indexing).await;
}

#[cfg(test)]
mod tests;
