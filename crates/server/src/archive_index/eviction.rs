// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Time-based eviction of the archive content index.
//!
//! The content index (`archive_dirs`, plus the directory paths in
//! `archive_paths` it references) is derived data: an archive without one is
//! indexed again from borg the next time it is browsed. When the
//! [`RETENTION_SETTING`] is set, the index of every archive that has been
//! neither indexed nor browsed for that many days is dropped, together with
//! its `archive_index_jobs` row, so the next browse finds the archive exactly
//! as it would a never-indexed one and rebuilds it.

use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{RepoLock, db, error::ApiError};

/// The `system_settings` key holding the retention, in days. `0` (and the
/// setting being absent) keeps every index forever.
pub const RETENTION_SETTING: &str = "archive_index_retention_days";

/// Archives evicted per transaction, so evicting a repository with thousands
/// of stale archives never holds one transaction open across all of them.
const EVICTION_BATCH: i64 = 100;

/// How long an archive's content index is kept after it was last indexed or
/// browsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexRetention {
    /// Never evict.
    #[default]
    Forever,
    /// Evict once the index has gone this many days unused.
    Days(NonZeroU32),
}

impl From<i64> for IndexRetention {
    /// The stored setting: `0` keeps forever, as every other `*_retention_days`
    /// setting does. The API rejects negative values; one written behind its
    /// back is read as "forever", which can never lose anything.
    fn from(days: i64) -> Self {
        u32::try_from(days)
            .ok()
            .and_then(NonZeroU32::new)
            .map_or(Self::Forever, Self::Days)
    }
}

impl IndexRetention {
    /// The point in time an index must have been used after to be kept, or
    /// `None` when nothing is evicted.
    #[must_use]
    pub fn cutoff(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Forever => None,
            Self::Days(days) => {
                now.checked_sub_signed(chrono::Duration::days(i64::from(days.get())))
            }
        }
    }
}

/// What one eviction pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvictionOutcome {
    /// Archives whose content index was dropped.
    pub archives: u64,
    /// Directory blobs (`archive_dirs` rows) deleted.
    pub dir_rows: u64,
    /// Directory paths no remaining index referenced.
    pub paths: u64,
}

impl EvictionOutcome {
    /// Both outcomes together, saturating rather than overflowing.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self {
            archives: self.archives.saturating_add(other.archives),
            dir_rows: self.dir_rows.saturating_add(other.dir_rows),
            paths: self.paths.saturating_add(other.paths),
        }
    }
}

/// Reads the configured retention. An unparseable value, or one outside
/// `0..=u32::MAX` (which the API rejects), is logged and read as "forever".
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn load_retention(pool: &PgPool) -> Result<IndexRetention, ApiError> {
    Ok(db::get_setting(pool, RETENTION_SETTING)
        .await?
        .and_then(|value| {
            value
                .parse::<i64>()
                .inspect_err(|e| {
                    tracing::warn!(
                        setting = RETENTION_SETTING,
                        value = %value,
                        error = %e,
                        "failed to parse retention setting"
                    );
                })
                .ok()
        })
        .map_or(IndexRetention::Forever, |days| {
            if u32::try_from(days).is_err() {
                tracing::warn!(
                    setting = RETENTION_SETTING,
                    value = days,
                    "retention setting out of range, keeping every index forever"
                );
            }
            IndexRetention::from(days)
        }))
}

/// Records that the archive browser read this archive's index.
///
/// Throttled to one write an hour per archive, so browsing does not write on
/// every request. That granularity is safe because the shortest retention is
/// a whole day: an archive browsed within the last hour can never be past its
/// cutoff.
///
/// The browser records the access *before* it reads the index status. An
/// eviction that committed first leaves no job row, so the status read sees a
/// never-indexed archive and triggers a rebuild; one that has not started yet
/// sees the fresh access and keeps the index. Either way the browser never
/// reads a `done` status whose directory blobs are already gone.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn record_index_access(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE archive_index_jobs j SET last_accessed_at = NOW() FROM archives a WHERE a.id = \
         j.archive_id AND a.repo_id = $1 AND a.name = $2 AND (j.last_accessed_at IS NULL OR \
         j.last_accessed_at < NOW() - INTERVAL '1 hour')",
        repo_id,
        archive_name,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Runs one eviction pass with the configured retention. Does nothing when
/// the retention is [`IndexRetention::Forever`].
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn run_index_eviction(
    pool: &PgPool,
    repo_lock: &RepoLock,
) -> Result<EvictionOutcome, ApiError> {
    let Some(cutoff) = load_retention(pool).await?.cutoff(Utc::now()) else {
        return Ok(EvictionOutcome::default());
    };
    evict_stale_indexes(pool, repo_lock, cutoff).await
}

/// Drops the content index of every archive whose index finished, and was
/// last browsed, before `cutoff`.
///
/// Only `done` jobs are touched: a `pending` or `indexing` job is a build in
/// progress, and a `failed` one is retried by the next sync. Each batch is
/// evicted under its repository's [`RepoLock`], which every indexing run holds
/// from its `borg list` until its last write. Without it, an index being built
/// for a newer archive could resolve a directory path that the evicted archive
/// was the last to reference, and lose it to the orphan GC before its own
/// blobs are written. The lock is taken per batch, not per repository, so a
/// long backlog never keeps that repository's backups and restores waiting
/// for more than one batch.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn evict_stale_indexes(
    pool: &PgPool,
    repo_lock: &RepoLock,
    cutoff: DateTime<Utc>,
) -> Result<EvictionOutcome, ApiError> {
    let repo_ids = sqlx::query_scalar!(
        "SELECT DISTINCT a.repo_id FROM archive_index_jobs j JOIN archives a ON a.id = \
         j.archive_id WHERE j.status = 'done' AND GREATEST(j.finished_at, j.last_accessed_at) < \
         $1 ORDER BY a.repo_id",
        cutoff,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut total = EvictionOutcome::default();
    for repo_id in repo_ids {
        total = total.plus(evict_stale_repo_indexes(pool, repo_lock, repo_id, cutoff).await?);
    }
    Ok(total)
}

/// Evicts one repository's stale indexes, batch by batch, until none is left.
/// Each batch takes the repository's [`RepoLock`] and releases it once its
/// transaction has committed, so anything queued for the repository runs
/// between two batches.
async fn evict_stale_repo_indexes(
    pool: &PgPool,
    repo_lock: &RepoLock,
    repo_id: i64,
    cutoff: DateTime<Utc>,
) -> Result<EvictionOutcome, ApiError> {
    let mut total = EvictionOutcome::default();
    loop {
        let batch = {
            let _repo_guard = repo_lock.acquire(repo_id).await;
            evict_batch(pool, repo_id, cutoff).await?
        };
        total = total.plus(batch);
        if batch.archives < EVICTION_BATCH.unsigned_abs() {
            return Ok(total);
        }
    }
}

/// Evicts up to [`EVICTION_BATCH`] archives in one transaction: their job
/// rows, their directory blobs and the paths only they referenced.
///
/// The job rows go first, and the outer `WHERE` repeats the eligibility
/// check, so a job whose status or access time changed after the subquery
/// read it is re-checked against its current row and left alone.
async fn evict_batch(
    pool: &PgPool,
    repo_id: i64,
    cutoff: DateTime<Utc>,
) -> Result<EvictionOutcome, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let archive_ids = sqlx::query_scalar!(
        "DELETE FROM archive_index_jobs WHERE archive_id IN (SELECT j.archive_id FROM \
         archive_index_jobs j JOIN archives a ON a.id = j.archive_id WHERE a.repo_id = $1 AND \
         j.status = 'done' AND GREATEST(j.finished_at, j.last_accessed_at) < $2 ORDER BY \
         j.archive_id LIMIT $3) AND status = 'done' AND GREATEST(finished_at, last_accessed_at) < \
         $2 RETURNING archive_id",
        repo_id,
        cutoff,
        EVICTION_BATCH,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    if archive_ids.is_empty() {
        tx.commit().await.map_err(ApiError::Database)?;
        return Ok(EvictionOutcome::default());
    }

    let candidate_ids = sqlx::query_scalar!(
        "SELECT DISTINCT dir_path_id FROM archive_dirs WHERE archive_id = ANY($1)",
        &archive_ids,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    let dir_rows = sqlx::query!(
        "DELETE FROM archive_dirs WHERE archive_id = ANY($1)",
        &archive_ids,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();

    let paths = db::gc_orphaned_archive_paths(&mut *tx, repo_id, &candidate_ids).await?;

    tx.commit().await.map_err(ApiError::Database)?;

    Ok(EvictionOutcome {
        archives: u64::try_from(archive_ids.len()).unwrap_or(u64::MAX),
        dir_rows,
        paths,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn days(n: u32) -> IndexRetention {
        IndexRetention::Days(NonZeroU32::new(n).unwrap())
    }

    #[test]
    fn zero_days_keeps_every_index_forever() {
        assert_eq!(IndexRetention::from(0), IndexRetention::Forever);
    }

    #[test]
    fn a_negative_setting_is_read_as_forever() {
        assert_eq!(IndexRetention::from(-3), IndexRetention::Forever);
    }

    #[test]
    fn a_setting_beyond_u32_is_read_as_forever() {
        assert_eq!(IndexRetention::from(i64::MAX), IndexRetention::Forever);
    }

    #[test]
    fn a_positive_setting_is_a_day_count() {
        assert_eq!(IndexRetention::from(30), days(30));
    }

    #[test]
    fn the_default_is_forever() {
        assert_eq!(IndexRetention::default(), IndexRetention::Forever);
    }

    #[test]
    fn forever_has_no_cutoff() {
        assert_eq!(IndexRetention::Forever.cutoff(Utc::now()), None);
    }

    #[test]
    fn the_cutoff_is_that_many_days_before_now() {
        let now = Utc.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap();
        let expected = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
        assert_eq!(days(30).cutoff(now), Some(expected));
    }

    #[test]
    fn a_cutoff_before_the_representable_range_evicts_nothing() {
        assert_eq!(days(u32::MAX).cutoff(DateTime::<Utc>::MIN_UTC), None);
    }

    #[test]
    fn outcomes_add_up() {
        let total = EvictionOutcome {
            archives: 1,
            dir_rows: 2,
            paths: 3,
        }
        .plus(EvictionOutcome {
            archives: 10,
            dir_rows: 20,
            paths: u64::MAX,
        });
        assert_eq!(
            total,
            EvictionOutcome {
                archives: 11,
                dir_rows: 22,
                paths: u64::MAX,
            }
        );
    }
}
