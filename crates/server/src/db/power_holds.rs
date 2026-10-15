// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `power_holds` table: a persisted mirror of
//! [`PowerSessionTracker`](crate::power::PowerSessionTracker)'s in-memory
//! holds, so a server restart mid-run does not forget a machine it woke.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    error::ApiError,
    power::{HoldOwner, PowerHostKey, PowerHostKind},
};

/// One persisted hold, as read back at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerHoldRow {
    /// Unique identifier.
    pub id: i64,
    /// The run holding the host.
    pub run_id: String,
    /// The agent of the target holding the host.
    pub agent_id: i64,
    /// The repository of the target holding the host, or `None` for a
    /// dependency's hold on a repository host.
    pub repo_id: Option<i64>,
    /// The host held.
    pub key: PowerHostKey,
    /// Whether the host's session had woken it, as of this hold.
    pub woke: bool,
    /// Whether the host's session had started its agent, as of this hold.
    pub started_agent: bool,
    /// When the hold was taken.
    pub created_at: DateTime<Utc>,
}

impl PowerHoldRow {
    /// Who holds this host.
    #[must_use]
    pub fn owner(&self) -> HoldOwner<'_> {
        HoldOwner {
            run_id: &self.run_id,
            agent_id: self.agent_id,
            repo_id: self.repo_id,
        }
    }
}

/// Records a new hold on `key`. It starts with the flags the host's other
/// holds carry, so that whichever hold outlives a restart still knows what
/// the session did.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_power_hold(
    pool: &PgPool,
    key: PowerHostKey,
    owner: HoldOwner<'_>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO power_holds (run_id, agent_id, repo_id, host_kind, host_id, woke, \
         started_agent) SELECT $1, $2, $3, $4, $5, COALESCE(bool_or(woke), false), \
         COALESCE(bool_or(started_agent), false) FROM power_holds WHERE host_kind = $4 AND \
         host_id = $5",
        owner.run_id,
        owner.agent_id,
        owner.repo_id,
        key.kind().to_string(),
        key.host_id(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Folds what a wake attempt did into every hold on `key`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn record_power_hold_outcome(
    pool: &PgPool,
    key: PowerHostKey,
    woke: bool,
    started_agent: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE power_holds SET woke = woke OR $3, started_agent = started_agent OR $4 WHERE \
         host_kind = $1 AND host_id = $2",
        key.kind().to_string(),
        key.host_id(),
        woke,
        started_agent,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Removes one of `owner`'s holds on `key`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_power_hold(
    pool: &PgPool,
    key: PowerHostKey,
    owner: HoldOwner<'_>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM power_holds WHERE id = (SELECT id FROM power_holds WHERE host_kind = $1 AND \
         host_id = $2 AND run_id = $3 AND agent_id = $4 AND repo_id IS NOT DISTINCT FROM $5 ORDER \
         BY id LIMIT 1)",
        key.kind().to_string(),
        key.host_id(),
        owner.run_id,
        owner.agent_id,
        owner.repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Every persisted hold, oldest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_power_holds(pool: &PgPool) -> Result<Vec<PowerHoldRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, run_id, agent_id, repo_id, host_kind AS \"host_kind: PowerHostKind\", \
         host_id, woke, started_agent, created_at FROM power_holds ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| PowerHoldRow {
            id: r.id,
            run_id: r.run_id,
            agent_id: r.agent_id,
            repo_id: r.repo_id,
            key: PowerHostKey::new(r.host_kind, r.host_id),
            woke: r.woke,
            started_agent: r.started_agent,
            created_at: r.created_at,
        })
        .collect())
}

/// Whether `owner`'s run still has a backup report in flight (pending or
/// started) - one that is yet to be settled by its agent. A hold without a
/// repository asks about every report of its run and agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn hold_run_in_flight(pool: &PgPool, owner: HoldOwner<'_>) -> Result<bool, ApiError> {
    let in_flight = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM backup_reports WHERE run_id = $1 AND agent_id = $2 AND \
         ($3::bigint IS NULL OR repo_id = $3) AND status IN ('pending', 'started')) AS \
         \"in_flight!\"",
        owner.run_id,
        owner.agent_id,
        owner.repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(in_flight)
}
