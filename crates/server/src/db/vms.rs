// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Queries for a host's virtual-machine staging settings and for the domains
//! its agent reported. The settings live on the agent rather than on a
//! schedule because the staging directory belongs to the host: two schedules
//! backing up the same host would otherwise each claim it with their own
//! limits.

use chrono::{DateTime, Utc};
use shared::vm::{DiscoveredVm, VmDomainConfig, VmSnapshotConfig, VmSnapshotOutcome};
use sqlx::PgPool;

use crate::error::ApiError;

/// A host's staging settings as stored on its agent row.
#[derive(Debug, Clone)]
pub struct AgentVmSnapshotRow {
    /// Whether this host stages its domains at all.
    pub vm_snapshot_enabled: bool,
    /// Absolute directory receiving one subdirectory per domain.
    pub vm_snapshot_dir: String,
    /// Increments written before a new full image is taken.
    pub vm_snapshot_full_interval: i32,
    /// Seconds one domain's snapshot may take.
    pub vm_snapshot_timeout_seconds: i32,
    /// Bytes a domain may occupy unless it carries its own limit. Zero is
    /// unlimited.
    pub vm_snapshot_default_limit_bytes: i64,
}

/// New staging settings for a host.
#[derive(Debug, Clone, Copy)]
pub struct VmSnapshotPatch<'a> {
    /// Whether this host stages its domains at all.
    pub enabled: bool,
    /// Absolute directory receiving one subdirectory per domain.
    pub dir: &'a str,
    /// Increments written before a new full image is taken.
    pub full_interval: i32,
    /// Seconds one domain's snapshot may take.
    pub timeout_seconds: i32,
    /// Bytes a domain may occupy unless it carries its own limit.
    pub default_limit_bytes: i64,
}

/// One domain of a host: what the last scan saw, what the last run staged, and
/// the settings the operator made.
#[derive(Debug, Clone)]
pub struct AgentVmRow {
    /// Row identifier.
    pub id: i64,
    /// libvirt domain name, unique on its host.
    pub name: String,
    /// Whether the domain is staged at all.
    pub included: bool,
    /// Bytes this domain may occupy, or `None` to inherit the host default.
    pub limit_bytes: Option<i64>,
    /// Run state at the last scan, as [`shared::vm::VmState`] renders it.
    pub state: String,
    /// Capture mode at the last scan, as [`shared::vm::VmSnapshotMode`]
    /// renders it.
    pub mode: String,
    /// Writable disks that would be staged.
    pub disk_count: i32,
    /// Space the domain's disks occupy on the host.
    pub disk_bytes: i64,
    /// Space the domain occupies below the staging directory.
    pub staged_bytes: i64,
    /// Increments in the chain after the last run.
    pub chain_length: i32,
    /// Why the last run failed for this domain, when it did.
    pub last_error: Option<String>,
    /// When the host last reported this domain.
    pub last_scanned_at: Option<DateTime<Utc>>,
    /// When a run last staged this domain.
    pub last_staged_at: Option<DateTime<Utc>>,
}

/// Saturating conversion for figures the agent reports as unsigned but
/// Postgres stores signed. A domain larger than 8 EiB is not a case worth
/// failing a scan over.
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Reads a host's staging settings.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when the query fails.
pub async fn get_agent_vm_snapshot(
    pool: &PgPool,
    agent_id: i64,
) -> Result<AgentVmSnapshotRow, ApiError> {
    sqlx::query_as!(
        AgentVmSnapshotRow,
        "SELECT vm_snapshot_enabled, vm_snapshot_dir, vm_snapshot_full_interval, \
         vm_snapshot_timeout_seconds, vm_snapshot_default_limit_bytes FROM agents WHERE id = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// Writes a host's staging settings.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when the update fails.
pub async fn update_agent_vm_snapshot(
    pool: &PgPool,
    agent_id: i64,
    patch: VmSnapshotPatch<'_>,
) -> Result<AgentVmSnapshotRow, ApiError> {
    sqlx::query_as!(
        AgentVmSnapshotRow,
        "UPDATE agents SET vm_snapshot_enabled = $2, vm_snapshot_dir = $3, \
         vm_snapshot_full_interval = $4, vm_snapshot_timeout_seconds = $5, \
         vm_snapshot_default_limit_bytes = $6 WHERE id = $1 RETURNING vm_snapshot_enabled, \
         vm_snapshot_dir, vm_snapshot_full_interval, vm_snapshot_timeout_seconds, \
         vm_snapshot_default_limit_bytes",
        agent_id,
        patch.enabled,
        patch.dir,
        patch.full_interval,
        patch.timeout_seconds,
        patch.default_limit_bytes,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// Lists a host's domains, oldest known first by name so the table does not
/// reorder itself between scans.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when the query fails.
pub async fn list_agent_vms(pool: &PgPool, agent_id: i64) -> Result<Vec<AgentVmRow>, ApiError> {
    sqlx::query_as!(
        AgentVmRow,
        "SELECT id, name, included, limit_bytes, state, mode, disk_count, disk_bytes, \
         staged_bytes, chain_length, last_error, last_scanned_at, last_staged_at FROM agent_vms \
         WHERE agent_id = $1 ORDER BY name",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records the result of a host scan: every reported domain is inserted or
/// refreshed, and a domain that has disappeared from the host is dropped
/// unless the operator configured it, in which case it is kept with an unknown
/// state so their settings are not silently lost.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when any of the statements fails.
pub async fn record_scan(
    pool: &PgPool,
    agent_id: i64,
    vms: &[DiscoveredVm],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    for vm in vms {
        sqlx::query!(
            "INSERT INTO agent_vms (agent_id, name, state, mode, disk_count, disk_bytes, \
             last_scanned_at) VALUES ($1, $2, $3, $4, $5, $6, NOW()) ON CONFLICT (agent_id, name) \
             DO UPDATE SET state = EXCLUDED.state, mode = EXCLUDED.mode, disk_count = \
             EXCLUDED.disk_count, disk_bytes = EXCLUDED.disk_bytes, last_scanned_at = NOW()",
            agent_id,
            vm.name,
            vm.state.to_string(),
            vm.mode.to_string(),
            i32::try_from(vm.disk_count).unwrap_or(i32::MAX),
            to_i64(vm.disk_bytes),
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    let seen: Vec<String> = vms.iter().map(|vm| vm.name.clone()).collect();

    sqlx::query!(
        "DELETE FROM agent_vms WHERE agent_id = $1 AND NOT (name = ANY($2)) AND included AND \
         limit_bytes IS NULL",
        agent_id,
        &seen,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!(
        "UPDATE agent_vms SET state = 'unknown', mode = 'unknown' WHERE agent_id = $1 AND NOT \
         (name = ANY($2))",
        agent_id,
        &seen,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)
}

/// Applies the operator's settings for one domain. The row is created when the
/// host has not been scanned yet, so a limit can be set ahead of the first
/// scan.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when the upsert fails.
pub async fn set_vm_settings(
    pool: &PgPool,
    agent_id: i64,
    name: &str,
    included: bool,
    limit_bytes: Option<i64>,
) -> Result<AgentVmRow, ApiError> {
    sqlx::query_as!(
        AgentVmRow,
        "INSERT INTO agent_vms (agent_id, name, included, limit_bytes) VALUES ($1, $2, $3, $4) ON \
         CONFLICT (agent_id, name) DO UPDATE SET included = EXCLUDED.included, limit_bytes = \
         EXCLUDED.limit_bytes RETURNING id, name, included, limit_bytes, state, mode, disk_count, \
         disk_bytes, staged_bytes, chain_length, last_error, last_scanned_at, last_staged_at",
        agent_id,
        name,
        included,
        limit_bytes,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records what a backup's snapshot phase did to each domain, so the figures
/// shown per domain come from a real run rather than an estimate.
///
/// # Errors
///
/// Returns [`ApiError::Database`] when any of the updates fails.
pub async fn record_outcomes(
    pool: &PgPool,
    agent_id: i64,
    outcomes: &[VmSnapshotOutcome],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    for outcome in outcomes {
        sqlx::query!(
            "INSERT INTO agent_vms (agent_id, name, mode, staged_bytes, chain_length, last_error, \
             last_staged_at) VALUES ($1, $2, $3, $4, $5, $6, NOW()) ON CONFLICT (agent_id, name) \
             DO UPDATE SET mode = EXCLUDED.mode, staged_bytes = EXCLUDED.staged_bytes, \
             chain_length = EXCLUDED.chain_length, last_error = EXCLUDED.last_error, \
             last_staged_at = NOW()",
            agent_id,
            outcome.name,
            outcome.mode.to_string(),
            to_i64(outcome.staged_bytes),
            i32::try_from(outcome.chain_length).unwrap_or(i32::MAX),
            outcome.error.as_deref(),
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)
}

/// Assembles the staging configuration delivered to an agent: the host's
/// settings plus the per-domain entries the operator made.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] when no agent has that id, or
/// [`ApiError::Database`] when a query fails.
pub async fn load_config(pool: &PgPool, agent_id: i64) -> Result<VmSnapshotConfig, ApiError> {
    let settings = get_agent_vm_snapshot(pool, agent_id).await?;
    let vms = list_agent_vms(pool, agent_id).await?;

    Ok(VmSnapshotConfig {
        enabled: settings.vm_snapshot_enabled,
        staging_dir: settings.vm_snapshot_dir,
        full_interval: u32::try_from(settings.vm_snapshot_full_interval).unwrap_or(1),
        timeout_seconds: u32::try_from(settings.vm_snapshot_timeout_seconds).unwrap_or(1),
        default_limit_bytes: u64::try_from(settings.vm_snapshot_default_limit_bytes).unwrap_or(0),
        domains: vms
            .into_iter()
            .map(|vm| VmDomainConfig {
                name: vm.name,
                included: vm.included,
                limit_bytes: vm
                    .limit_bytes
                    .map(|limit| u64::try_from(limit).unwrap_or(0)),
            })
            .collect(),
    })
}
