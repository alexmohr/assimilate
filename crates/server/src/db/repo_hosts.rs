// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repository hosts: the machines borg writes to, and everything that is a
//! fact about that machine rather than about one repository on it - its
//! address, the SSH host key it presents, how to wake it, and whether it is
//! expected to be online.
//!
//! One hostname is one host, with exactly one port and one pinned key.
//! Repositories point at their host; they keep only the SSH user they log in
//! as and their path.

use sqlx::{PgPool, Postgres};

use super::RepoPowerPatch;
use crate::error::ApiError;

/// A row from the `repo_hosts` table.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RepoHostRow {
    /// Unique identifier.
    pub id: i64,
    /// Hostname or IP borg connects to.
    pub ssh_host: String,
    /// SSH port.
    pub ssh_port: i32,
    /// Pinned SSH host key every repository on this host is verified against.
    pub ssh_host_key: Option<String>,
    /// Whether to send a Wake-on-LAN packet before a backup if the host isn't
    /// already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to.
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run is
    /// what woke it.
    pub shutdown_after_backup: bool,
    /// Whether the host is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back while a catch-up waits on it.
    pub catch_up_recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
}

/// A repository host with the number of repositories that use it - a row of
/// the host list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoHostSummaryRow {
    /// The host itself.
    pub host: RepoHostRow,
    /// How many repositories use it.
    pub repo_count: i64,
}

/// One repository on a host, as the host page lists it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RepoOnHostRow {
    /// Repository ID.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// SSH user the repository logs in as.
    pub ssh_user: String,
    /// Borg repository path on the host.
    pub repo_path: String,
    /// Whether the repository is enabled.
    pub enabled: bool,
}

fn not_found(repo_host_id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repository host {repo_host_id} not found"))
        }
        other => ApiError::Database(other),
    }
}

/// Finds the host named `ssh_host`, creating it with `ssh_port` if there is
/// none, and returns its ID.
///
/// A host has exactly one port, so naming an existing host with a different
/// one is refused rather than silently pointing the repository at whichever
/// daemon the host's port reaches.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the host exists with a different port,
/// or [`ApiError::Database`] if a query fails.
pub async fn resolve_repo_host(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    ssh_host: &str,
    ssh_port: i32,
) -> Result<i64, ApiError> {
    sqlx::query!(
        "INSERT INTO repo_hosts (ssh_host, ssh_port) VALUES ($1, $2) ON CONFLICT (ssh_host) DO \
         NOTHING",
        ssh_host,
        ssh_port,
    )
    .execute(&mut **tx)
    .await
    .map_err(ApiError::Database)?;

    let host = sqlx::query!(
        "SELECT id, ssh_port FROM repo_hosts WHERE ssh_host = $1",
        ssh_host,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::Database)?;

    if host.ssh_port != ssh_port {
        return Err(ApiError::BadRequest(format!(
            "repository host {ssh_host} uses SSH port {}; change the port on the host, not on the \
             repository",
            host.ssh_port
        )));
    }
    Ok(host.id)
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the host does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_repo_host(pool: &PgPool, repo_host_id: i64) -> Result<RepoHostRow, ApiError> {
    sqlx::query_as!(
        RepoHostRow,
        "SELECT id, ssh_host, ssh_port, ssh_host_key, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, intermittent, \
         catch_up_recheck_minutes, catch_up_give_up_minutes FROM repo_hosts WHERE id = $1",
        repo_host_id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(repo_host_id))
}

/// The host named `ssh_host`, if there is one.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn find_repo_host_by_name(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<Option<RepoHostRow>, ApiError> {
    sqlx::query_as!(
        RepoHostRow,
        "SELECT id, ssh_host, ssh_port, ssh_host_key, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, intermittent, \
         catch_up_recheck_minutes, catch_up_give_up_minutes FROM repo_hosts WHERE ssh_host = $1",
        ssh_host,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every repository host, by name, with how many repositories use each.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_repo_hosts(pool: &PgPool) -> Result<Vec<RepoHostSummaryRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT h.id, h.ssh_host, h.ssh_port, h.ssh_host_key, h.wake_enabled, h.wake_mac_address, \
         h.wake_broadcast_address, h.wake_timeout_seconds, h.shutdown_after_backup, \
         h.intermittent, h.catch_up_recheck_minutes, h.catch_up_give_up_minutes, (SELECT COUNT(*) \
         FROM repos r WHERE r.repo_host_id = h.id) AS \"repo_count!\" FROM repo_hosts h ORDER BY \
         h.ssh_host",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|row| RepoHostSummaryRow {
            host: RepoHostRow {
                id: row.id,
                ssh_host: row.ssh_host,
                ssh_port: row.ssh_port,
                ssh_host_key: row.ssh_host_key,
                wake_enabled: row.wake_enabled,
                wake_mac_address: row.wake_mac_address,
                wake_broadcast_address: row.wake_broadcast_address,
                wake_timeout_seconds: row.wake_timeout_seconds,
                shutdown_after_backup: row.shutdown_after_backup,
                intermittent: row.intermittent,
                catch_up_recheck_minutes: row.catch_up_recheck_minutes,
                catch_up_give_up_minutes: row.catch_up_give_up_minutes,
            },
            repo_count: row.repo_count,
        })
        .collect())
}

/// The repositories that use `repo_host_id`, by name.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_repos_on_host(
    pool: &PgPool,
    repo_host_id: i64,
) -> Result<Vec<RepoOnHostRow>, ApiError> {
    sqlx::query_as!(
        RepoOnHostRow,
        "SELECT id, name, ssh_user, repo_path, enabled FROM repos WHERE repo_host_id = $1 ORDER \
         BY name",
        repo_host_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every repository, grouped by the host it uses and by name within each -
/// the host list's repositories in one query rather than one per host.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_repos_by_host(
    pool: &PgPool,
) -> Result<std::collections::HashMap<i64, Vec<RepoOnHostRow>>, ApiError> {
    let rows = sqlx::query!(
        "SELECT repo_host_id, id, name, ssh_user, repo_path, enabled FROM repos ORDER BY \
         repo_host_id, name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut by_host: std::collections::HashMap<i64, Vec<RepoOnHostRow>> =
        std::collections::HashMap::new();
    for row in rows {
        by_host
            .entry(row.repo_host_id)
            .or_default()
            .push(RepoOnHostRow {
                id: row.id,
                name: row.name,
                ssh_user: row.ssh_user,
                repo_path: row.repo_path,
                enabled: row.enabled,
            });
    }
    Ok(by_host)
}

/// Changes where a host is reached.
///
/// Every repository on the host now lives at a new location, so each is
/// marked as relocated exactly as editing one repository's location does, and
/// the host's storage quota follows the new name. The pinned key is kept: a
/// renamed machine presents the same key, and one that does not is refused
/// rather than trusted.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if another host already has `ssh_host`,
/// [`ApiError::NotFound`] if the host does not exist, or
/// [`ApiError::Database`] if a query fails.
pub async fn update_repo_host_address(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_host: &str,
    ssh_port: i32,
) -> Result<RepoHostRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let previous = sqlx::query!(
        "SELECT ssh_host, ssh_port FROM repo_hosts WHERE id = $1 FOR UPDATE",
        repo_host_id,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(not_found(repo_host_id))?;

    let host = sqlx::query_as!(
        RepoHostRow,
        "UPDATE repo_hosts SET ssh_host = $2, ssh_port = $3 WHERE id = $1 RETURNING id, ssh_host, \
         ssh_port, ssh_host_key, wake_enabled, wake_mac_address, wake_broadcast_address, \
         wake_timeout_seconds, shutdown_after_backup, intermittent, catch_up_recheck_minutes, \
         catch_up_give_up_minutes",
        repo_host_id,
        ssh_host,
        ssh_port,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => ApiError::Conflict(
            format!("there is already a repository host named {ssh_host}"),
        ),
        other => not_found(repo_host_id)(other),
    })?;

    if previous.ssh_host != host.ssh_host || previous.ssh_port != host.ssh_port {
        sqlx::query!(
            "UPDATE server_quotas SET ssh_host = $2 WHERE ssh_host = $1 AND NOT EXISTS (SELECT 1 \
             FROM server_quotas taken WHERE taken.ssh_host = $2)",
            previous.ssh_host,
            host.ssh_host,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        sqlx::query!(
            "UPDATE repos SET relocation_pending = true WHERE repo_host_id = $1",
            repo_host_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        sqlx::query!(
            "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT sr.repo_id, \
             a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN \
             schedule_repos sr ON sr.schedule_id = st.schedule_id JOIN repos r ON r.id = \
             sr.repo_id WHERE r.repo_host_id = $1 ON CONFLICT DO NOTHING",
            repo_host_id,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(host)
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the host does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn update_repo_host_power(
    pool: &PgPool,
    repo_host_id: i64,
    power: RepoPowerPatch<'_>,
) -> Result<RepoHostRow, ApiError> {
    sqlx::query_as!(
        RepoHostRow,
        "UPDATE repo_hosts SET wake_enabled = $2, wake_mac_address = $3, wake_broadcast_address = \
         $4, wake_timeout_seconds = $5, shutdown_after_backup = $6 WHERE id = $1 RETURNING id, \
         ssh_host, ssh_port, ssh_host_key, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, intermittent, \
         catch_up_recheck_minutes, catch_up_give_up_minutes",
        repo_host_id,
        power.wake_enabled,
        power.wake_mac_address,
        power.wake_broadcast_address,
        power.wake_timeout_seconds,
        power.shutdown_after_backup,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(repo_host_id))
}

/// Pins `ssh_host_key` as the key every repository on the host is verified
/// against.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the host does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn update_repo_host_key(
    pool: &PgPool,
    repo_host_id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE repo_hosts SET ssh_host_key = $2 WHERE id = $1",
        repo_host_id,
        ssh_host_key,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "repository host {repo_host_id} not found"
        )));
    }
    Ok(())
}

/// The refusal for a host that presents another key than the one pinned for
/// it, shared by the check before a repository is added and the pin itself.
#[must_use]
pub fn host_key_mismatch(ssh_host: &str) -> ApiError {
    ApiError::Conflict(format!(
        "{ssh_host} presents a different SSH host key than the one pinned on its repository host; \
         check the host and accept the new key there first"
    ))
}

/// Pins `ssh_host_key` on a host that has none yet, or confirms the one it
/// has. A different pinned key is left alone and refused: first trust is set
/// once, and a later request that saw another key - two repositories added on
/// a new host at the same time, say - cannot quietly replace it. The
/// conditional update is what makes that hold under concurrency, since a
/// check made before it could be overtaken.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the host already has another key pinned,
/// or [`ApiError::Database`] if the query fails.
pub async fn pin_first_repo_host_key(
    executor: impl sqlx::PgExecutor<'_>,
    repo_host_id: i64,
    ssh_host: &str,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE repo_hosts SET ssh_host_key = $2 WHERE id = $1 AND (ssh_host_key IS NULL OR \
         ssh_host_key = $2)",
        repo_host_id,
        ssh_host_key,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(host_key_mismatch(ssh_host));
    }
    Ok(())
}

/// Removes a host no repository uses any more. A host still in use cannot be
/// removed: its repositories would have nowhere to connect to.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if a repository still uses the host,
/// [`ApiError::NotFound`] if it does not exist, or [`ApiError::Database`] if
/// the query fails.
pub async fn delete_repo_host(pool: &PgPool, repo_host_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM repo_hosts WHERE id = $1", repo_host_id)
        .execute(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_foreign_key_violation() => {
                ApiError::Conflict(
                    "repositories still use this host; move or remove them first".to_owned(),
                )
            }
            other => ApiError::Database(other),
        })?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "repository host {repo_host_id} not found"
        )));
    }
    Ok(())
}

/// Every agent that backs up to any repository on `repo_host_id` - the agents
/// whose pushed config carries this host's key and address.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_target_agents_for_repo_host(
    pool: &PgPool,
    repo_host_id: i64,
) -> Result<Vec<super::ScheduleRunTarget>, ApiError> {
    sqlx::query_as!(
        super::ScheduleRunTarget,
        "SELECT DISTINCT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id JOIN schedule_repos sr ON sr.schedule_id = st.schedule_id JOIN repos \
         r ON r.id = sr.repo_id WHERE r.repo_host_id = $1",
        repo_host_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
