// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repositories: connection details, passphrases and host keys, import and
//! sync state, relocation and the stats views.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::{
    protocol::RepoOpKind,
    types::{BorgEncryption, Compression, QuotaAction, Visibility},
};
use sqlx::PgPool;

use super::repo_hosts;
use crate::error::ApiError;

/// A row from the `repos` table (sensitive fields excluded).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path on the remote host.
    pub repo_path: String,
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Compression algorithm (e.g. "lz4", "zstd").
    pub compression: Compression,
    /// Encryption mode (e.g. "repokey-blake2").
    pub encryption: BorgEncryption,
    /// Whether the repository is enabled for backups.
    pub enabled: bool,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: Visibility,
    /// Optional cron expression for automatic sync.
    pub sync_schedule: Option<String>,
    /// The repository host this repository lives on. The SSH host and port
    /// above and the power settings below are that host's.
    pub repo_host_id: i64,
    /// Whether to send a Wake-on-LAN packet before a backup if the
    /// repository host isn't already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to (defaults to the
    /// global broadcast address when unset).
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
    /// The machine the repository host is.
    pub machine_id: i64,
}

/// SSH connection details for a repository (passphrase omitted).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoConnectionRow {
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
}

/// Parameters for inserting a new repository.
pub struct InsertRepoParams<'a> {
    /// Repository display name.
    pub name: &'a str,
    /// Borg repository path on the remote host.
    pub repo_path: &'a str,
    /// SSH user for the remote host.
    pub ssh_user: &'a str,
    /// SSH hostname for the remote host.
    pub ssh_host: &'a str,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Encrypted passphrase bytes.
    pub passphrase_encrypted: &'a [u8],
    /// Compression algorithm.
    pub compression: &'a str,
    /// Encryption mode.
    pub encryption: &'a str,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Cron expression for automatic repository sync.
    /// `None` = use DB default; `Some(Some(s))` = set value; `Some(None)` = disable.
    pub sync_schedule: Option<Option<&'a str>>,
}

/// Parameters for updating an existing repository.
pub struct UpdateRepoParams<'a> {
    /// Repository ID to update.
    pub repo_id: i64,
    /// New display name.
    pub name: &'a str,
    /// New borg repository path.
    pub repo_path: &'a str,
    /// New SSH user.
    pub ssh_user: &'a str,
    /// New SSH hostname.
    pub ssh_host: &'a str,
    /// New SSH port.
    pub ssh_port: i32,
    /// New compression algorithm.
    pub compression: &'a str,
    /// New encryption mode.
    pub encryption: &'a str,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// New sync schedule cron expression.
    /// `None` = keep existing value; `Some(None)` = disable; `Some(Some(s))` = set to `s`.
    pub sync_schedule: Option<Option<&'a str>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_importing_repo_ids(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let rows = sqlx::query_scalar!("SELECT repo_id FROM repo_import_state WHERE importing = true")
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(rows)
}

/// Repositories left mid-import by a previous run, for startup to resume.
/// A repository held for its passphrase is importing too, but has nothing to
/// resume until the passphrase is set.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_resumable_import_repo_ids(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let rows = sqlx::query_scalar!(
        "SELECT repo_id FROM repo_import_state WHERE importing AND NOT awaiting_passphrase"
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}

/// Whether a sync owns the repository's `importing` flag - every sync sets it,
/// the scheduler's included, whether or not it registers an import task. A
/// passphrase hold sets the flag too, but is no sync.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn is_repo_syncing(pool: &PgPool, repo_id: i64) -> Result<bool, ApiError> {
    let syncing = sqlx::query_scalar!(
        "SELECT importing AND NOT awaiting_passphrase FROM repo_import_state WHERE repo_id = $1",
        repo_id
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .flatten();
    Ok(syncing.unwrap_or(false))
}

/// Holds a repository created without its passphrase (by a config import):
/// importing, so the scheduler and "Sync now" leave it alone, and awaiting its
/// passphrase, so startup does not resume it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn hold_repo_for_passphrase(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, importing, awaiting_passphrase) VALUES ($1, \
         true, true) ON CONFLICT (repo_id) DO UPDATE SET importing = true, awaiting_passphrase = \
         true",
        repo_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Releases the hold [`hold_repo_for_passphrase`] placed, in one statement so
/// it never touches an `importing` flag a sync set: a repository that is not
/// held is left as it is. Returns whether there was a hold to release.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn release_passphrase_hold(
    executor: impl sqlx::PgExecutor<'_>,
    repo_id: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE repo_import_state SET importing = false, awaiting_passphrase = false WHERE \
         repo_id = $1 AND awaiting_passphrase",
        repo_id
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_repo_importing(
    pool: &PgPool,
    repo_id: i64,
    importing: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, importing) VALUES ($1, $2) ON CONFLICT (repo_id) \
         DO UPDATE SET importing = EXCLUDED.importing",
        repo_id,
        importing
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Guards `repo_import_state.importing` for the duration of a sync/import
/// operation, clearing it back to `false` on drop - including during a panic
/// unwind - so a panic partway through the sync work can't leave a repo stuck
/// showing "syncing" forever. Unlike [`crate::repo_op_tracker::RepoOpTracker`]'s
/// in-memory active-op tracking (which already has a panic-safe
/// `RepoOpGuard`), this DB-persisted flag had no such net: a scheduled repo
/// sync skips any repo already marked `importing`, so a stuck flag also
/// permanently blocks that repo's own periodic sync, not just the UI's
/// "syncing" indicator.
///
/// `Drop` can't await, so cleanup runs as a spawned task (mirroring
/// `RepoOpGuard`); call [`Self::clear_now`] on the normal-completion path so
/// the flag is cleared before you return, instead of racing the spawned task.
/// Calling `clear_now` disarms the `Drop` cleanup, so a concurrent operation
/// that legitimately re-sets `importing = true` for this `repo_id` right
/// after can't be clobbered by a stale deferred clear.
pub struct ImportingGuard {
    pool: PgPool,
    repo_id: i64,
    task_registry: shared::task_registry::TaskRegistry,
    cleared: bool,
}

impl ImportingGuard {
    /// Sets `importing = true` for `repo_id` and returns a guard that clears
    /// it back to `false` on drop.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn acquire(
        pool: &PgPool,
        repo_id: i64,
        task_registry: shared::task_registry::TaskRegistry,
    ) -> Result<Self, ApiError> {
        set_repo_importing(pool, repo_id, true).await?;
        Ok(Self {
            pool: pool.clone(),
            repo_id,
            task_registry,
            cleared: false,
        })
    }

    /// Clears the importing flag immediately, awaiting the write instead of
    /// leaving it to the guard's deferred `Drop` cleanup.
    pub async fn clear_now(mut self) {
        match set_repo_importing(&self.pool, self.repo_id, false).await {
            Ok(()) => self.cleared = true,
            Err(e) => {
                tracing::error!(
                    repo_id = self.repo_id,
                    error = %e,
                    "failed to clear importing flag"
                );
            }
        }
    }
}

impl Drop for ImportingGuard {
    fn drop(&mut self) {
        if self.cleared {
            return;
        }
        let pool = self.pool.clone();
        let repo_id = self.repo_id;
        let handle = tokio::spawn(async move {
            if let Err(e) = set_repo_importing(&pool, repo_id, false).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "failed to clear importing flag on guard drop"
                );
            }
        });
        self.task_registry.register(handle);
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_repo_import_error(
    executor: impl sqlx::PgExecutor<'_>,
    repo_id: i64,
    error: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, error) VALUES ($1, $2) ON CONFLICT (repo_id) DO \
         UPDATE SET error = EXCLUDED.error",
        repo_id,
        error
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_import_status_message(
    pool: &PgPool,
    repo_id: i64,
    msg: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, status_message) VALUES ($1, $2) ON CONFLICT \
         (repo_id) DO UPDATE SET status_message = EXCLUDED.status_message",
        repo_id,
        msg
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_import_progress(
    pool: &PgPool,
    repo_id: i64,
    progress: i64,
    total: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, progress, total) VALUES ($1, $2, $3) ON CONFLICT \
         (repo_id) DO UPDATE SET progress = EXCLUDED.progress, total = EXCLUDED.total",
        repo_id,
        i32::try_from(progress).unwrap_or(i32::MAX),
        i32::try_from(total).unwrap_or(i32::MAX),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_last_synced(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_stats (repo_id, last_synced_at) VALUES ($1, NOW()) ON CONFLICT \
         (repo_id) DO UPDATE SET last_synced_at = EXCLUDED.last_synced_at",
        repo_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Returns `true` if the agent is linked to the repo via at least one
/// schedule target (i.e., the agent is assigned a schedule whose target
/// repo matches `repo_id`).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn check_agent_repo_access(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<bool, ApiError> {
    sqlx::query_scalar!(
        "SELECT EXISTS(
           SELECT 1 FROM schedule_targets st
           JOIN schedules s ON s.id = st.schedule_id
           WHERE st.agent_id = $1 AND s.repo_id = $2
         ) AS \"exists!\"",
        agent_id,
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Authoritative repository statistics parsed from `borg info --json`
/// (`cache.stats`) and `borg list --json`. This is the single source of truth
/// for repo size and archive counts; never derive these from `backup_reports`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoInfoStats {
    /// Total original (uncompressed) size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Total number of chunks.
    pub total_chunks: i64,
    /// Number of unique chunks.
    pub unique_chunks: i64,
    /// Number of archives in the repository.
    pub archive_count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_info_stats(
    pool: &PgPool,
    repo_id: i64,
    stats: &RepoInfoStats,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_stats (repo_id, original_size, compressed_size, deduplicated_size, \
         total_chunks, unique_chunks, archive_count, updated_at) VALUES ($1, $2, $3, $4, $5, $6, \
         $7, NOW()) ON CONFLICT (repo_id) DO UPDATE SET original_size = EXCLUDED.original_size, \
         compressed_size = EXCLUDED.compressed_size, deduplicated_size = \
         EXCLUDED.deduplicated_size, total_chunks = EXCLUDED.total_chunks, unique_chunks = \
         EXCLUDED.unique_chunks, archive_count = EXCLUDED.archive_count, updated_at = \
         EXCLUDED.updated_at",
        repo_id,
        stats.original_size,
        stats.compressed_size,
        stats.deduplicated_size,
        stats.total_chunks,
        stats.unique_chunks,
        i32::try_from(stats.archive_count).unwrap_or(i32::MAX),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "UPDATE repos SET relocation_pending = false WHERE id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Remove `hostname` from the pending-hosts set for this repo. Clears `relocation_pending`
/// on the repo itself once every registered host has confirmed the new location.
///
/// Only clears the flag when this host's entry was actually present (`rows_affected` > 0) AND
/// no other hosts remain pending. This prevents spurious clears when a host that was never
/// registered in the pending table completes a backup.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_relocation_for_host(
    pool: &PgPool,
    repo_id: i64,
    hostname: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let deleted = sqlx::query!(
        "DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1 AND hostname = $2",
        repo_id,
        hostname,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    if deleted.rows_affected() > 0 {
        let remaining: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*)::BIGINT AS \"COUNT!\" FROM repo_relocation_pending_hosts WHERE \
             repo_id = $1",
            repo_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        if remaining == 0 {
            sqlx::query!(
                "UPDATE repos SET relocation_pending = false WHERE id = $1",
                repo_id
            )
            .execute(&mut *tx)
            .await
            .map_err(ApiError::Database)?;
        }
    }
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "UPDATE repos SET relocation_pending = true WHERE id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname FROM \
         agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedule_repos sr ON \
         sr.schedule_id = st.schedule_id WHERE sr.repo_id = $1 ON CONFLICT DO NOTHING",
        repo_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_encryption(
    pool: &PgPool,
    repo_id: i64,
    encryption: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repos SET encryption = $2 WHERE id = $1",
        repo_id,
        encryption
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_repo(
    pool: &PgPool,
    params: &InsertRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let (repo_id, _) = insert_repo_row(&mut tx, params).await?;
    tx.commit().await.map_err(ApiError::Database)?;
    get_repo_by_id(pool, repo_id).await
}

/// Inserts a repository and pins the key its host presented, in one
/// transaction: a host seen for the first time takes this key, and one that
/// meanwhile had another key pinned refuses the repository rather than
/// leaving it behind unverified (see
/// [`repo_hosts::pin_first_repo_host_key`]).
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the host has another key pinned,
/// [`ApiError::BadRequest`] if it uses another port, or
/// [`ApiError::Database`] if a query fails.
pub async fn insert_repo_pinning_host_key(
    pool: &PgPool,
    params: &InsertRepoParams<'_>,
    ssh_host_key: &str,
) -> Result<RepoRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let (repo_id, repo_host_id) = insert_repo_row(&mut tx, params).await?;
    repo_hosts::pin_first_repo_host_key(&mut *tx, repo_host_id, params.ssh_host, ssh_host_key)
        .await?;
    tx.commit().await.map_err(ApiError::Database)?;
    get_repo_by_id(pool, repo_id).await
}

/// Inserts the repository row, joining or creating its host. Returns the
/// repository's id and its host's.
async fn insert_repo_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: &InsertRepoParams<'_>,
) -> Result<(i64, i64), ApiError> {
    let repo_host_id = repo_hosts::resolve_repo_host(tx, params.ssh_host, params.ssh_port).await?;

    let repo_id = if let Some(sync_schedule) = params.sync_schedule {
        sqlx::query_scalar!(
            "INSERT INTO repos (name, repo_path, ssh_user, repo_host_id, passphrase_encrypted, \
             compression, encryption, owner_id, sync_schedule) VALUES ($1, $2, $3, $4, $5, $6, \
             $7, $8, $9) RETURNING id",
            params.name,
            params.repo_path,
            params.ssh_user,
            repo_host_id,
            params.passphrase_encrypted,
            params.compression,
            params.encryption,
            params.owner_id,
            sync_schedule,
        )
        .fetch_one(&mut **tx)
        .await
    } else {
        sqlx::query_scalar!(
            "INSERT INTO repos (name, repo_path, ssh_user, repo_host_id, passphrase_encrypted, \
             compression, encryption, owner_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING \
             id",
            params.name,
            params.repo_path,
            params.ssh_user,
            repo_host_id,
            params.passphrase_encrypted,
            params.compression,
            params.encryption,
            params.owner_id,
        )
        .fetch_one(&mut **tx)
        .await
    }
    .map_err(ApiError::Database)?;

    Ok((repo_id, repo_host_id))
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_by_id(pool: &PgPool, repo_id: i64) -> Result<RepoRow, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, r.compression AS \
         \"compression: Compression\", r.encryption AS \"encryption: BorgEncryption\", r.enabled, \
         r.owner_id, r.visibility AS \"visibility: Visibility\", r.sync_schedule, r.repo_host_id, \
         hm.wake_enabled, hm.wake_mac_address, hm.wake_broadcast_address, \
         hm.wake_timeout_seconds, hm.shutdown_after_backup, h.machine_id FROM repos r JOIN \
         repo_hosts h ON h.id = r.repo_host_id JOIN machines hm ON hm.id = h.machine_id WHERE \
         r.id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_connection(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoConnectionRow, ApiError> {
    sqlx::query_as!(
        RepoConnectionRow,
        "SELECT r.ssh_user, h.ssh_host, h.ssh_port FROM repos r JOIN repo_hosts h ON h.id = \
         r.repo_host_id WHERE r.id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_repo(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    write_repo(pool, params, Relocation::Unchanged).await
}

/// Whether an update moved the repository somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relocation {
    /// Same host, same path.
    Unchanged,
    /// A different host or path: every agent that writes here has to confirm
    /// the move before backing up to it again.
    Moved,
}

async fn write_repo(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
    relocation: Relocation,
) -> Result<RepoRow, ApiError> {
    let not_found = |e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repo {} not found", params.repo_id))
        }
        other => ApiError::Database(other),
    };
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let repo_host_id =
        repo_hosts::resolve_repo_host(&mut tx, params.ssh_host, params.ssh_port).await?;
    let relocated = relocation == Relocation::Moved;

    if let Some(sync_schedule) = params.sync_schedule {
        sqlx::query_scalar!(
            "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, repo_host_id = $5, \
             compression = $6, encryption = $7, enabled = $8, sync_schedule = $9, \
             relocation_pending = relocation_pending OR $10 WHERE id = $1 RETURNING id",
            params.repo_id,
            params.name,
            params.repo_path,
            params.ssh_user,
            repo_host_id,
            params.compression,
            params.encryption,
            params.enabled,
            sync_schedule,
            relocated,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(not_found)?;
    } else {
        sqlx::query_scalar!(
            "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, repo_host_id = $5, \
             compression = $6, encryption = $7, enabled = $8, relocation_pending = \
             relocation_pending OR $9 WHERE id = $1 RETURNING id",
            params.repo_id,
            params.name,
            params.repo_path,
            params.ssh_user,
            repo_host_id,
            params.compression,
            params.encryption,
            params.enabled,
            relocated,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(not_found)?;
    }

    if relocated {
        sqlx::query!(
            "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname \
             FROM agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedule_repos sr \
             ON sr.schedule_id = st.schedule_id WHERE sr.repo_id = $1 ON CONFLICT DO NOTHING",
            params.repo_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;
    get_repo_by_id(pool, params.repo_id).await
}

/// Like [`update_repo`] but atomically sets `relocation_pending = true` and registers all
/// currently-scheduled agents as pending confirmation in the same transaction. Use this when
/// the repository location (host, port, or path) has changed so the scheduler never observes
/// the new path with the flag still `false`.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_repo_and_set_relocation_pending(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    write_repo(pool, params, Relocation::Moved).await
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_repo(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    // One transaction, for the same reason `replace_schedule_repos` uses one:
    // between the delete and the two repair statements below, `schedules` and
    // `schedule_repos` disagree - schedules point at a repository that is gone,
    // and a schedule can be left with only best-effort targets. A tick or a
    // config push observing that window would dispatch from a half-repaired
    // state, and a failure part-way through would leave it that way for good.
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    // Clears the auto-disable bookkeeping the same way set_schedule_enabled does for
    // any other direct `enabled` write - otherwise a schedule auto-disabled for an
    // unreachable agent keeps auto_disabled_by_agent_id pointing at that agent even
    // after its repo (and thus this schedule's only reason to run) is gone, so a
    // later reconnect from that agent would silently flip enabled back to true on an
    // orphaned schedule that nobody decided to re-enable.
    // Only the schedules this repository was the *last* target of: one that
    // also writes elsewhere still has somewhere to go and keeps running, it
    // just loses this copy.
    sqlx::query!(
        "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true WHERE id IN (SELECT schedule_id FROM \
         schedule_repos GROUP BY schedule_id HAVING bool_and(repo_id = $1))",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    let result = sqlx::query!("DELETE FROM repos WHERE id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }

    // The delete cascaded the `schedule_repos` rows and nulled the denormalised
    // `schedules.repo_id` that pointed here. Re-point every schedule that still
    // has targets at the first of them, so everything reading a schedule's
    // repository agrees with the list again.
    sqlx::query!(
        "UPDATE schedules s SET repo_id = first_target.repo_id FROM (SELECT DISTINCT ON \
         (schedule_id) schedule_id, repo_id FROM schedule_repos ORDER BY schedule_id, required \
         DESC, execution_order, repo_id) AS first_target WHERE s.id = first_target.schedule_id \
         AND s.repo_id IS NULL",
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // A schedule left with only best-effort targets could report success
    // without writing a copy, which the create/update API refuses to produce.
    // Promote its first target rather than leave that state reachable.
    sqlx::query!(
        "UPDATE schedule_repos SET required = true WHERE id IN (SELECT DISTINCT ON (schedule_id) \
         id FROM schedule_repos WHERE schedule_id IN (SELECT schedule_id FROM schedule_repos \
         GROUP BY schedule_id HAVING bool_and(NOT required)) ORDER BY schedule_id, \
         execution_order, repo_id)",
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_repo_passphrase(
    executor: impl sqlx::PgExecutor<'_>,
    repo_id: i64,
    passphrase_encrypted: &[u8],
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE repos SET passphrase_encrypted = $2 WHERE id = $1",
        repo_id,
        passphrase_encrypted,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_passphrase(pool: &PgPool, repo_id: i64) -> Result<Vec<u8>, ApiError> {
    sqlx::query_scalar!(
        "SELECT passphrase_encrypted FROM repos WHERE id = $1",
        repo_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_with_passphrase(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithPassphraseRow, ApiError> {
    sqlx::query_as!(
        RepoWithPassphraseRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, h.ssh_host_key, \
         r.passphrase_encrypted, r.compression AS \"compression: Compression\", r.encryption AS \
         \"encryption: BorgEncryption\", r.enabled, r.relocation_pending, r.sync_schedule FROM \
         repos r JOIN repo_hosts h ON h.id = r.repo_host_id WHERE r.id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// Pins `ssh_host_key` on the host `repo_id` lives on - and so for every
/// repository on that host, since one machine presents one key.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_ssh_host_key(
    pool: &PgPool,
    repo_id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repo_hosts SET ssh_host_key = $2 WHERE id = (SELECT repo_host_id FROM repos WHERE \
         id = $1)",
        repo_id,
        ssh_host_key,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_repo_ssh_host_key(pool: &PgPool, name: &str) -> Result<Option<String>, ApiError> {
    let row = sqlx::query_scalar!(
        "SELECT h.ssh_host_key FROM repos r JOIN repo_hosts h ON h.id = r.repo_host_id WHERE \
         r.name = $1",
        name,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.flatten())
}

/// A row from the `repos` table including the encrypted passphrase.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepoWithPassphraseRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path on the remote host.
    pub repo_path: String,
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Known host key of the remote host.
    pub ssh_host_key: Option<String>,
    /// Encrypted passphrase bytes.
    pub passphrase_encrypted: Vec<u8>,
    /// Compression algorithm.
    pub compression: Compression,
    /// Encryption mode.
    pub encryption: BorgEncryption,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Whether a relocation is pending confirmation.
    pub relocation_pending: bool,
    /// Sync schedule cron expression, if any.
    pub sync_schedule: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_repos(pool: &PgPool) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, r.compression AS \
         \"compression: Compression\", r.encryption AS \"encryption: BorgEncryption\", r.enabled, \
         r.owner_id, r.visibility AS \"visibility: Visibility\", r.sync_schedule, r.repo_host_id, \
         hm.wake_enabled, hm.wake_mac_address, hm.wake_broadcast_address, \
         hm.wake_timeout_seconds, hm.shutdown_after_backup, h.machine_id FROM repos r JOIN \
         repo_hosts h ON h.id = r.repo_host_id JOIN machines hm ON hm.id = h.machine_id ORDER BY \
         r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Repo row with sync schedule metadata.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoRowWithSync {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path.
    pub repo_path: String,
    /// SSH user.
    pub ssh_user: String,
    /// SSH hostname.
    pub ssh_host: String,
    /// SSH port.
    pub ssh_port: i32,
    /// Compression algorithm.
    pub compression: Compression,
    /// Encryption mode.
    pub encryption: BorgEncryption,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Owning user ID.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: Visibility,
    /// Sync schedule cron expression.
    pub sync_schedule: Option<String>,
    /// When the repo was last synced.
    pub last_synced_at: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_with_sync_schedule(
    pool: &PgPool,
) -> Result<Vec<RepoRowWithSync>, ApiError> {
    sqlx::query_as!(
        RepoRowWithSync,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, r.compression AS \
         \"compression: Compression\", r.encryption AS \"encryption: BorgEncryption\", r.enabled, \
         r.owner_id, r.visibility AS \"visibility: Visibility\", r.sync_schedule, \
         rs.last_synced_at FROM repos r JOIN repo_hosts h ON h.id = r.repo_host_id LEFT JOIN \
         repo_stats rs ON rs.repo_id = r.id ORDER BY r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoWithPassphraseRow>, ApiError> {
    sqlx::query_as!(
        RepoWithPassphraseRow,
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, \
         h.ssh_host_key, r.passphrase_encrypted, r.compression AS \"compression: Compression\", \
         r.encryption AS \"encryption: BorgEncryption\", r.enabled, r.relocation_pending, \
         r.sync_schedule FROM repos r JOIN repo_hosts h ON h.id = r.repo_host_id JOIN \
         schedule_repos sr ON sr.repo_id = r.id JOIN schedule_targets st ON st.schedule_id = \
         sr.schedule_id WHERE st.agent_id = $1 ORDER BY r.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_for_agent_public(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, \
         r.compression AS \"compression: Compression\", r.encryption AS \"encryption: \
         BorgEncryption\", r.enabled, r.owner_id, r.visibility AS \"visibility: Visibility\", \
         r.sync_schedule, r.repo_host_id, hm.wake_enabled, hm.wake_mac_address, \
         hm.wake_broadcast_address, hm.wake_timeout_seconds, hm.shutdown_after_backup, \
         h.machine_id FROM repos r JOIN repo_hosts h ON h.id = r.repo_host_id JOIN machines hm ON \
         hm.id = h.machine_id JOIN schedule_repos sr ON sr.repo_id = r.id JOIN schedule_targets \
         st ON st.schedule_id = sr.schedule_id WHERE st.agent_id = $1 ORDER BY r.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_name(pool: &PgPool, repo_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
    }

    let row = sqlx::query_as!(Row, "SELECT name FROM repos WHERE id = $1", repo_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
            other => ApiError::Database(other),
        })?;

    Ok(row.name)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_ssh_host(pool: &PgPool, repo_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        ssh_host: String,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT h.ssh_host FROM repos r JOIN repo_hosts h ON h.id = r.repo_host_id WHERE r.id = $1",
        repo_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })?;

    Ok(row.ssh_host)
}

/// Full repository row with aggregated stats (sizes, agent count, import state, last op).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring this query's own columns via query_as!, not \
              mutually-exclusive states; splitting into enums or sub-structs would require \
              restructuring the query_as! call in list_repos_with_stats/get_repo_with_stats for \
              no correctness benefit -- the API layer (RepoWithStatsResponse) already nests the \
              power-related ones"
)]
pub struct RepoWithStatsRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path.
    pub repo_path: String,
    /// SSH user.
    pub ssh_user: String,
    /// SSH hostname.
    pub ssh_host: String,
    /// SSH port.
    pub ssh_port: i32,
    /// Known host key.
    pub ssh_host_key: Option<String>,
    /// Compression algorithm.
    pub compression: Compression,
    /// Encryption mode.
    pub encryption: BorgEncryption,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Whether the repo is currently being imported.
    pub importing: bool,
    /// Import error message, if any.
    pub import_error: Option<String>,
    /// Import progress (items processed).
    pub import_progress: i32,
    /// Import total items.
    pub import_total: i32,
    /// Import status message.
    pub import_status_message: Option<String>,
    /// Owning user ID.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: Visibility,
    /// Sync schedule cron expression.
    pub sync_schedule: Option<String>,
    /// When the repo was last synced.
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Number of archives.
    pub archive_count: i64,
    /// When the last successful backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Total original size in bytes.
    pub total_original_size: i64,
    /// Total compressed size in bytes.
    pub total_compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub total_deduplicated_size: i64,
    /// Number of distinct agents that backed up to this repo.
    pub agent_count: i64,
    /// Number of unmatched agents (imported placeholders).
    pub unmatched_count: i64,
    /// Kind of the last operation performed on the repo.
    pub last_op_kind: Option<RepoOpKind>,
    /// Whether a relocation is pending confirmation.
    pub relocation_pending: bool,
    /// When the last operation was performed.
    pub last_op_at: Option<DateTime<Utc>>,
    /// Who performed the last operation.
    pub last_op_by: Option<String>,
    /// Own quota warn threshold in bytes, if a quota row exists.
    pub quota_warn_bytes: Option<i64>,
    /// Own quota critical threshold in bytes, if a quota row exists.
    pub quota_critical_bytes: Option<i64>,
    /// Own quota warn action, if a quota row exists.
    pub quota_warn_action: Option<QuotaAction>,
    /// Own quota critical action, if a quota row exists.
    pub quota_critical_action: Option<QuotaAction>,
    /// The repository host this repository lives on.
    pub repo_host_id: i64,
    /// Whether that host is marked as not always online.
    pub host_intermittent: bool,
    /// The machine the repository's host is.
    pub host_machine_id: i64,
    /// Whether a quota row exists for this repo, and if so, whether it's enabled.
    /// `NULL` means no quota is configured at all.
    pub quota_enabled: Option<bool>,
    /// Whether to send a Wake-on-LAN packet before a backup if the
    /// repository host isn't already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to (defaults to the
    /// global broadcast address when unset).
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_with_stats(pool: &PgPool) -> Result<Vec<RepoWithStatsRow>, ApiError> {
    sqlx::query_as!(
        RepoWithStatsRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, h.ssh_host_key, \
         r.compression AS \"compression: Compression\", r.encryption AS \"encryption: \
         BorgEncryption\", r.enabled, r.owner_id, r.visibility AS \"visibility: Visibility\", \
         r.sync_schedule, r.repo_host_id, h.machine_id AS host_machine_id, hm.intermittent AS \
         host_intermittent, hm.wake_enabled, hm.wake_mac_address, hm.wake_broadcast_address, \
         hm.wake_timeout_seconds, hm.shutdown_after_backup, r.relocation_pending, \
         COALESCE(rs.original_size, 0) AS \"total_original_size!\", COALESCE(rs.compressed_size, \
         0) AS \"total_compressed_size!\", COALESCE(rs.deduplicated_size, 0) AS \
         \"total_deduplicated_size!\", COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", \
         rs.last_synced_at AS \"last_synced_at?\", COALESCE(ris.importing, false) AS \
         \"importing!\", ris.error AS \"import_error?\", COALESCE(ris.progress, 0) AS \
         \"import_progress!\", COALESCE(ris.total, 0) AS \"import_total!\", ris.status_message AS \
         \"import_status_message?\", rlo.kind AS \"last_op_kind?: RepoOpKind\", rlo.at AS \
         \"last_op_at?\", rlo.by_text AS \"last_op_by?\", agg.last_backup_at AS \
         \"last_backup_at?\", COALESCE(agg.agent_count, 0) AS \"agent_count!\", \
         COALESCE(agg.unmatched_count, 0) AS \"unmatched_count!\", q.warn_bytes AS \
         \"quota_warn_bytes?\", q.critical_bytes AS \"quota_critical_bytes?\", q.warn_action AS \
         \"quota_warn_action?: QuotaAction\", q.critical_action AS \"quota_critical_action?: \
         QuotaAction\", q.enabled AS \"quota_enabled?\" FROM repos r JOIN repo_hosts h ON h.id = \
         r.repo_host_id JOIN machines hm ON hm.id = h.machine_id LEFT JOIN repo_stats rs ON \
         rs.repo_id = r.id LEFT JOIN repo_import_state ris ON ris.repo_id = r.id LEFT JOIN \
         repo_last_op rlo ON rlo.repo_id = r.id LEFT JOIN repo_quotas q ON q.repo_id = r.id LEFT \
         JOIN LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' THEN \
         br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true ORDER \
         BY r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_with_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithStatsRow, ApiError> {
    sqlx::query_as!(
        RepoWithStatsRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, h.ssh_host, h.ssh_port, h.ssh_host_key, \
         r.compression AS \"compression: Compression\", r.encryption AS \"encryption: \
         BorgEncryption\", r.enabled, r.owner_id, r.visibility AS \"visibility: Visibility\", \
         r.sync_schedule, r.repo_host_id, h.machine_id AS host_machine_id, hm.intermittent AS \
         host_intermittent, hm.wake_enabled, hm.wake_mac_address, hm.wake_broadcast_address, \
         hm.wake_timeout_seconds, hm.shutdown_after_backup, r.relocation_pending, \
         COALESCE(rs.original_size, 0) AS \"total_original_size!\", COALESCE(rs.compressed_size, \
         0) AS \"total_compressed_size!\", COALESCE(rs.deduplicated_size, 0) AS \
         \"total_deduplicated_size!\", COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", \
         rs.last_synced_at AS \"last_synced_at?\", COALESCE(ris.importing, false) AS \
         \"importing!\", ris.error AS \"import_error?\", COALESCE(ris.progress, 0) AS \
         \"import_progress!\", COALESCE(ris.total, 0) AS \"import_total!\", ris.status_message AS \
         \"import_status_message?\", rlo.kind AS \"last_op_kind?: RepoOpKind\", rlo.at AS \
         \"last_op_at?\", rlo.by_text AS \"last_op_by?\", agg.last_backup_at AS \
         \"last_backup_at?\", COALESCE(agg.agent_count, 0) AS \"agent_count!\", \
         COALESCE(agg.unmatched_count, 0) AS \"unmatched_count!\", q.warn_bytes AS \
         \"quota_warn_bytes?\", q.critical_bytes AS \"quota_critical_bytes?\", q.warn_action AS \
         \"quota_warn_action?: QuotaAction\", q.critical_action AS \"quota_critical_action?: \
         QuotaAction\", q.enabled AS \"quota_enabled?\" FROM repos r JOIN repo_hosts h ON h.id = \
         r.repo_host_id JOIN machines hm ON hm.id = h.machine_id LEFT JOIN repo_stats rs ON \
         rs.repo_id = r.id LEFT JOIN repo_import_state ris ON ris.repo_id = r.id LEFT JOIN \
         repo_last_op rlo ON rlo.repo_id = r.id LEFT JOIN repo_quotas q ON q.repo_id = r.id LEFT \
         JOIN LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' THEN \
         br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true WHERE \
         r.id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_last_op(
    pool: &PgPool,
    repo_id: i64,
    kind: RepoOpKind,
    at: chrono::DateTime<chrono::Utc>,
    by: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_last_op (repo_id, kind, at, by_text) VALUES ($1, $2, $3, $4) ON \
         CONFLICT (repo_id) DO UPDATE SET kind = EXCLUDED.kind, at = EXCLUDED.at, by_text = \
         EXCLUDED.by_text",
        repo_id,
        kind.to_string(),
        at,
        by,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
