// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Machines: what is woken, shut down, and expected to be online or not.
//!
//! An agent, a repository host and a dependency host are each a role of one
//! machine. The machine holds the facts about the box itself - its MAC
//! address, whether it is shut down after a backup, whether it is always
//! online - so a NAS that is both a repository host and an SMB dependency has
//! one of each rather than one per role.

use chrono::{DateTime, Utc};
use shared::types::{MachineWakeMode, ScheduleWakeOverride};
use sqlx::PgPool;

use crate::error::ApiError;

/// One machine's power and availability settings.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct MachineRow {
    /// Unique identifier.
    pub id: i64,
    /// Whether a run wakes it by default when it does not answer.
    pub wake_enabled: bool,
    /// MAC address a wake is sent to.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to; the global default
    /// when unset.
    pub wake_broadcast_address: Option<String>,
    /// How long a run waits for it after a wake.
    pub wake_timeout_seconds: i32,
    /// Whether it is shut down after a backup, when that run is what woke it.
    pub shutdown_after_backup: bool,
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back while a run waits on it.
    pub catch_up_recheck_minutes: i32,
    /// How long a run waits on it before giving up; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
    /// When it was last checked.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// What that check found.
    pub last_check_reachable: Option<bool>,
}

fn not_found(id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("machine {id} not found")),
        other => ApiError::Database(other),
    }
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such machine, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_machine(pool: &PgPool, id: i64) -> Result<MachineRow, ApiError> {
    sqlx::query_as!(
        MachineRow,
        "SELECT id, wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, \
         last_checked_at, last_check_reachable FROM machines WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))
}

/// How a machine is woken and shut down.
pub struct MachinePowerPatch<'a> {
    /// Whether a run wakes it by default.
    pub wake_enabled: bool,
    /// MAC address a wake is sent to.
    pub wake_mac_address: Option<&'a str>,
    /// Broadcast address the magic packet is sent to.
    pub wake_broadcast_address: Option<&'a str>,
    /// How long a run waits for it after a wake.
    pub wake_timeout_seconds: i32,
    /// Whether it is shut down after a backup that woke it.
    pub shutdown_after_backup: bool,
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such machine, or
/// [`ApiError::Database`] if the update fails (a MAC address the database
/// rejects included).
pub async fn update_machine_power(
    pool: &PgPool,
    id: i64,
    patch: &MachinePowerPatch<'_>,
) -> Result<MachineRow, ApiError> {
    sqlx::query!(
        "UPDATE machines SET wake_enabled = $2, wake_mac_address = $3, wake_broadcast_address = \
         $4, wake_timeout_seconds = $5, shutdown_after_backup = $6 WHERE id = $1 RETURNING id",
        id,
        patch.wake_enabled,
        patch.wake_mac_address,
        patch.wake_broadcast_address,
        patch.wake_timeout_seconds,
        patch.shutdown_after_backup,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))?;
    get_machine(pool, id).await
}

/// Whether a machine is expected to be online, and how a run waiting on it
/// behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineAvailabilityPatch {
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back.
    pub catch_up_recheck_minutes: i32,
    /// How long a run waits on it; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such machine, or
/// [`ApiError::Database`] if the update fails.
pub async fn update_machine_availability(
    pool: &PgPool,
    id: i64,
    patch: MachineAvailabilityPatch,
) -> Result<MachineRow, ApiError> {
    sqlx::query!(
        "UPDATE machines SET intermittent = $2, catch_up_recheck_minutes = $3, \
         catch_up_give_up_minutes = $4 WHERE id = $1 RETURNING id",
        id,
        patch.intermittent,
        patch.catch_up_recheck_minutes,
        patch.catch_up_give_up_minutes,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))?;
    get_machine(pool, id).await
}

/// Remembers what a check of a machine found, so a list can show it without
/// probing on every load.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn record_machine_check(
    pool: &PgPool,
    id: i64,
    reachable: bool,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE machines SET last_checked_at = $2, last_check_reachable = $3 WHERE id = $1",
        id,
        at,
        reachable,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// The agent a machine runs, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineAgentRole {
    /// Agent ID.
    pub id: i64,
    /// Its hostname.
    pub hostname: String,
    /// Its domain, for an agent that shares its hostname.
    pub domain: Option<String>,
    /// SSH host used to shut the machine down, if set.
    pub ssh_host: Option<String>,
    /// SSH user the server's key was deployed for, if it ever was.
    pub last_ssh_user: Option<String>,
}

/// The repository host a machine is, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRepoHostRole {
    /// Repository host ID.
    pub id: i64,
    /// Its hostname.
    pub ssh_host: String,
}

/// A dependency host on a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineDependencyRole {
    /// Dependency host ID.
    pub id: i64,
    /// Its name.
    pub name: String,
}

/// Every role a machine plays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineRoles {
    /// The agent running on it.
    pub agent: Option<MachineAgentRole>,
    /// The repository host it is.
    pub repo_host: Option<MachineRepoHostRole>,
    /// The dependency hosts on it.
    pub dependencies: Vec<MachineDependencyRole>,
}

impl MachineRoles {
    /// A short name for the machine, for a heading or a log line: the
    /// repository host's hostname, then the agent's, then the first
    /// dependency's name.
    #[must_use]
    pub fn label(&self) -> String {
        self.repo_host
            .as_ref()
            .map(|h| h.ssh_host.clone())
            .or_else(|| self.agent.as_ref().map(|a| a.hostname.clone()))
            .or_else(|| self.dependencies.first().map(|d| d.name.clone()))
            .unwrap_or_default()
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn get_machine_roles(pool: &PgPool, id: i64) -> Result<MachineRoles, ApiError> {
    let agent = sqlx::query_as!(
        MachineAgentRole,
        "SELECT id, hostname, domain, ssh_host, last_ssh_user FROM agents WHERE machine_id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    let repo_host = sqlx::query_as!(
        MachineRepoHostRole,
        "SELECT id, ssh_host FROM repo_hosts WHERE machine_id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    let dependencies = sqlx::query_as!(
        MachineDependencyRole,
        "SELECT id, name FROM dependency_hosts WHERE machine_id = $1 ORDER BY name",
        id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(MachineRoles {
        agent,
        repo_host,
        dependencies,
    })
}

/// The machine a repository host is.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if there is no such host, or
/// [`ApiError::Database`] if the query fails.
pub async fn repo_host_machine_id(pool: &PgPool, repo_host_id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar!(
        "SELECT machine_id FROM repo_hosts WHERE id = $1",
        repo_host_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repository host {repo_host_id} not found"))
        }
        other => ApiError::Database(other),
    })
}

/// One schedule's own choice for one machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleMachineWake {
    /// The machine.
    pub machine_id: i64,
    /// What the schedule does about it.
    pub mode: MachineWakeMode,
}

/// Every per-machine choice a schedule makes.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_schedule_machine_wake(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleMachineWake>, ApiError> {
    sqlx::query_as!(
        ScheduleMachineWake,
        "SELECT machine_id, mode AS \"mode: MachineWakeMode\" FROM schedule_machine_wake WHERE \
         schedule_id = $1 ORDER BY machine_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Replaces every per-machine choice a schedule makes.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a statement fails (an unknown machine
/// included).
pub async fn replace_schedule_machine_wake(
    pool: &PgPool,
    schedule_id: i64,
    choices: &[ScheduleMachineWake],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM schedule_machine_wake WHERE schedule_id = $1",
        schedule_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    let machine_ids: Vec<i64> = choices.iter().map(|c| c.machine_id).collect();
    let modes: Vec<String> = choices.iter().map(|c| c.mode.to_string()).collect();
    sqlx::query!(
        "INSERT INTO schedule_machine_wake (schedule_id, machine_id, mode) SELECT $1, m, mode \
         FROM UNNEST($2::BIGINT[], $3::TEXT[]) AS u(m, mode)",
        schedule_id,
        &machine_ids,
        &modes,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)
}

/// A schedule that treats one machine differently from the machine's own
/// setting: either through a choice for that machine, or through its
/// schedule-wide override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineScheduleOverride {
    /// The schedule.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    /// Its schedule-wide setting.
    pub wake_override: ScheduleWakeOverride,
    /// Its choice for this machine, if it made one.
    pub mode: Option<MachineWakeMode>,
}

/// Every schedule that uses a machine - as its agent, a repository host it
/// writes to, or a dependency it needs - and does not leave it to the
/// machine's own settings.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_machine_schedule_overrides(
    pool: &PgPool,
    machine_id: i64,
) -> Result<Vec<MachineScheduleOverride>, ApiError> {
    sqlx::query_as!(
        MachineScheduleOverride,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, s.wake_override AS \"wake_override: \
         ScheduleWakeOverride\", w.mode AS \"mode?: MachineWakeMode\" FROM schedules s LEFT JOIN \
         schedule_machine_wake w ON w.schedule_id = s.id AND w.machine_id = $1 WHERE (w.mode IS \
         NOT NULL OR s.wake_override <> 'host_default') AND (EXISTS (SELECT 1 FROM \
         schedule_targets st JOIN agents a ON a.id = st.agent_id WHERE st.schedule_id = s.id AND \
         a.machine_id = $1) OR EXISTS (SELECT 1 FROM schedule_repos sr JOIN repos r ON r.id = \
         sr.repo_id JOIN repo_hosts h ON h.id = r.repo_host_id WHERE sr.schedule_id = s.id AND \
         h.machine_id = $1) OR EXISTS (SELECT 1 FROM schedule_dependencies sd JOIN \
         dependency_hosts d ON d.id = sd.dependency_host_id WHERE sd.schedule_id = s.id AND \
         d.machine_id = $1)) ORDER BY s.name, s.id",
        machine_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
