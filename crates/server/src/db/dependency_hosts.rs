// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dependency hosts: machines a backup needs besides its agent and its
//! repository - the server whose share a pre-backup command mounts, say.
//!
//! A dependency is checked by connecting to one TCP port. How it is woken and
//! whether it is always online are its machine's settings (see
//! [`super::machines`]); a dependency that is the same machine as a repository
//! host shares that host's machine, so one box has one MAC address and one
//! shut-down switch. Which dependencies a run needs is set per schedule and
//! agent, plus whatever the agent's backup defaults and the repository hosts
//! the schedule writes to require.

use chrono::{DateTime, Utc};
use shared::types::{MachinePolicy, MachineWakeMode, ScheduleWakeOverride, WaitReason};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `dependency_hosts` table, with the parts of its machine a
/// card shows.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DependencyHostRow {
    /// Unique identifier.
    pub id: i64,
    /// Display name, unique.
    pub name: String,
    /// Hostname or IP address the server connects to.
    pub address: String,
    /// TCP port whose answer means the machine is up.
    pub port: i32,
    /// Free-form note on what the dependency is for.
    pub description: String,
    /// The machine it is on.
    pub machine_id: i64,
    /// The repository host on the same machine, if any.
    pub repo_host_id: Option<i64>,
    /// Whether the machine is marked as not always online.
    pub intermittent: bool,
    /// Whether the machine wakes by default.
    pub wake_enabled: bool,
    /// When the machine was last checked, by anything.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// What that check found.
    pub last_check_reachable: Option<bool>,
}

/// A dependency with how much uses it - a card on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyHostSummaryRow {
    /// The dependency itself.
    pub host: DependencyHostRow,
    /// Distinct schedules that need it, set on the schedule or inherited from
    /// an agent's defaults or a repository host's.
    pub schedule_count: i64,
    /// Agents whose backup defaults require it.
    pub agent_default_count: i64,
    /// Targets waiting on it to catch up a skipped run.
    pub waiting_count: i64,
}

/// The fields a dependency is created with.
#[derive(Debug, Clone, Copy)]
pub struct NewDependencyHost<'a> {
    /// Display name.
    pub name: &'a str,
    /// Hostname or IP address.
    pub address: &'a str,
    /// TCP port to check.
    pub port: i32,
    /// What it is for.
    pub description: &'a str,
    /// Repository host it is the same machine as. `None` puts it on a machine
    /// of its own.
    pub repo_host_id: Option<i64>,
}

/// The connection fields an edit can change.
#[derive(Debug, Clone, Copy)]
pub struct DependencyConnectionPatch<'a> {
    /// Display name.
    pub name: &'a str,
    /// Hostname or IP address.
    pub address: &'a str,
    /// TCP port to check.
    pub port: i32,
    /// What it is for.
    pub description: &'a str,
}

/// Where a schedule's need for a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySource {
    /// Set on the schedule itself, for this agent.
    Schedule,
    /// Inherited from the agent's backup defaults.
    AgentDefault,
    /// Inherited from a repository host the schedule writes to.
    RepoHost,
}

/// One (schedule, agent) that needs a dependency - a row of its "Used by".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyUsageRow {
    /// Schedule that needs it.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    /// Agent the schedule runs on.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    /// Where the need comes from. Set on the schedule wins, then agent
    /// defaults, when several apply.
    pub source: DependencySource,
}

/// A dependency one target needs, with its machine's wake settings and the
/// schedule's own choice for that machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredDependency {
    /// Dependency ID.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// Address to connect to.
    pub address: String,
    /// Port to connect to.
    pub port: i32,
    /// The machine it is on.
    pub machine_id: i64,
    /// Whether that machine is marked as not always online.
    pub intermittent: bool,
    /// Repository host on the same machine, if any.
    pub repo_host_id: Option<i64>,
    /// Whether the machine wakes by default.
    pub wake_enabled: bool,
    /// MAC address to wake.
    pub wake_mac_address: Option<String>,
    /// Broadcast address for the magic packet.
    pub wake_broadcast_address: Option<String>,
    /// How long to wait after a wake.
    pub wake_timeout_seconds: i32,
    /// The schedule's choice for this machine, if it made one. Always `None`
    /// outside a schedule.
    pub mode: Option<MachineWakeMode>,
}

impl RequiredDependency {
    /// What a run does about this dependency's machine under the schedule's
    /// schedule-wide setting.
    #[must_use]
    pub const fn policy(&self, schedule_override: ScheduleWakeOverride) -> MachinePolicy {
        MachinePolicy::resolve(
            schedule_override,
            self.mode,
            self.wake_enabled,
            self.intermittent,
        )
    }

    /// Why a run waits for this dependency rather than failing when it does
    /// not answer, if it does. Waiting does not depend on the schedule-wide
    /// wake setting, only on the machine and the schedule's choice for it.
    #[must_use]
    pub const fn wait_reason(&self) -> Option<WaitReason> {
        self.policy(ScheduleWakeOverride::HostDefault).wait
    }
}

fn not_found(id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("dependency {id} not found")),
        other => ApiError::Database(other),
    }
}

/// Turns a unique-name violation into a 409 the form can show next to the
/// field, and anything else into the database error it is.
fn name_taken(name: &str) -> impl FnOnce(sqlx::Error) -> ApiError + '_ {
    move |e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            ApiError::Conflict(format!("a dependency named '{name}' already exists"))
        }
        _ => ApiError::Database(e),
    }
}

/// Every dependency, with how much uses it, ordered by name.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_dependency_hosts(
    pool: &PgPool,
) -> Result<Vec<DependencyHostSummaryRow>, ApiError> {
    query_dependency_hosts(pool, None).await
}

/// One dependency, with how much uses it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_dependency_host_summary(
    pool: &PgPool,
    id: i64,
) -> Result<DependencyHostSummaryRow, ApiError> {
    query_dependency_hosts(pool, Some(id))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::NotFound(format!("dependency {id} not found")))
}

/// Dependencies with their usage counts: every one, or only `id`.
async fn query_dependency_hosts(
    pool: &PgPool,
    id: Option<i64>,
) -> Result<Vec<DependencyHostSummaryRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT d.id, d.name, d.address, d.port, d.description, d.machine_id, (SELECT h.id FROM \
         repo_hosts h WHERE h.machine_id = d.machine_id) AS \"repo_host_id?\", m.intermittent, \
         m.wake_enabled, m.last_checked_at, m.last_check_reachable, (SELECT COUNT(DISTINCT \
         u.schedule_id) FROM (SELECT sd.schedule_id FROM schedule_dependencies sd WHERE \
         sd.dependency_host_id = d.id UNION SELECT st.schedule_id FROM agent_default_dependencies \
         ad JOIN schedule_targets st ON st.agent_id = ad.agent_id WHERE ad.dependency_host_id = \
         d.id UNION SELECT sr.schedule_id FROM repo_host_dependencies rd JOIN repos r ON \
         r.repo_host_id = rd.repo_host_id JOIN schedule_repos sr ON sr.repo_id = r.id WHERE \
         rd.dependency_host_id = d.id) u) AS \"schedule_count!\", (SELECT COUNT(*) FROM \
         agent_default_dependencies ad WHERE ad.dependency_host_id = d.id) AS \
         \"agent_default_count!\", (SELECT COUNT(*) FROM dependency_catch_ups c WHERE \
         c.dependency_host_id = d.id) AS \"waiting_count!\" FROM dependency_hosts d JOIN machines \
         m ON m.id = d.machine_id WHERE ($1::bigint IS NULL OR d.id = $1) ORDER BY d.name",
        id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| DependencyHostSummaryRow {
            host: DependencyHostRow {
                id: r.id,
                name: r.name,
                address: r.address,
                port: r.port,
                description: r.description,
                machine_id: r.machine_id,
                repo_host_id: r.repo_host_id,
                intermittent: r.intermittent,
                wake_enabled: r.wake_enabled,
                last_checked_at: r.last_checked_at,
                last_check_reachable: r.last_check_reachable,
            },
            schedule_count: r.schedule_count,
            agent_default_count: r.agent_default_count,
            waiting_count: r.waiting_count,
        })
        .collect())
}

/// One dependency.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_dependency_host(pool: &PgPool, id: i64) -> Result<DependencyHostRow, ApiError> {
    sqlx::query_as!(
        DependencyHostRow,
        "SELECT d.id, d.name, d.address, d.port, d.description, d.machine_id, (SELECT h.id FROM \
         repo_hosts h WHERE h.machine_id = d.machine_id) AS \"repo_host_id?\", m.intermittent, \
         m.wake_enabled, m.last_checked_at, m.last_check_reachable FROM dependency_hosts d JOIN \
         machines m ON m.id = d.machine_id WHERE d.id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))
}

/// Creates a dependency, on the machine of the repository host it is the same
/// machine as, or on a new machine of its own.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the name is taken,
/// [`ApiError::BadRequest`] if the repository host does not exist, or
/// [`ApiError::Database`] if the insert fails otherwise.
pub async fn insert_dependency_host(
    pool: &PgPool,
    new: &NewDependencyHost<'_>,
) -> Result<DependencyHostRow, ApiError> {
    let machine_id = match new.repo_host_id {
        Some(repo_host_id) => Some(machine_of_repo_host(pool, repo_host_id).await?),
        None => None,
    };
    let id = sqlx::query_scalar!(
        "INSERT INTO dependency_hosts (name, address, port, description, machine_id) VALUES ($1, \
         $2, $3, $4, $5) RETURNING id",
        new.name,
        new.address,
        new.port,
        new.description,
        machine_id,
    )
    .fetch_one(pool)
    .await
    .map_err(name_taken(new.name))?;
    get_dependency_host(pool, id).await
}

/// The machine of a repository host a dependency is to share, refusing one
/// that does not exist as the caller's mistake.
async fn machine_of_repo_host(pool: &PgPool, repo_host_id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar!(
        "SELECT machine_id FROM repo_hosts WHERE id = $1",
        repo_host_id
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::BadRequest(format!("repository host {repo_host_id} not found")))
}

/// Changes a dependency's name, address, port and description. The machine's
/// last check is forgotten, since it was an answer from the old address.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist,
/// [`ApiError::Conflict`] if the new name is taken, or
/// [`ApiError::Database`] if the update fails otherwise.
pub async fn update_dependency_host_connection(
    pool: &PgPool,
    id: i64,
    patch: &DependencyConnectionPatch<'_>,
) -> Result<DependencyHostRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let machine_id = sqlx::query_scalar!(
        "UPDATE dependency_hosts SET name = $2, address = $3, port = $4, description = $5 WHERE \
         id = $1 RETURNING machine_id",
        id,
        patch.name,
        patch.address,
        patch.port,
        patch.description,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(name_taken(patch.name))?
    .ok_or_else(|| ApiError::NotFound(format!("dependency {id} not found")))?;
    sqlx::query!(
        "UPDATE machines SET last_checked_at = NULL, last_check_reachable = NULL WHERE id = $1",
        machine_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    get_dependency_host(pool, id).await
}

/// Moves a dependency onto the machine of the repository host it is the same
/// machine as, or - with `None` - onto a new machine of its own with default
/// settings. A machine nothing else is on any more is removed by the
/// database.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the dependency does not exist,
/// [`ApiError::BadRequest`] if the repository host does not, or
/// [`ApiError::Database`] if the update fails.
pub async fn set_dependency_host_repo_host(
    pool: &PgPool,
    id: i64,
    repo_host_id: Option<i64>,
) -> Result<DependencyHostRow, ApiError> {
    let current = get_dependency_host(pool, id).await?;
    if current.repo_host_id == repo_host_id && repo_host_id.is_some() {
        return Ok(current);
    }
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let machine_id = match repo_host_id {
        Some(repo_host_id) => machine_of_repo_host(pool, repo_host_id).await?,
        None if current.repo_host_id.is_none() => return Ok(current),
        None => sqlx::query_scalar!("INSERT INTO machines DEFAULT VALUES RETURNING id")
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::Database)?,
    };
    sqlx::query!(
        "UPDATE dependency_hosts SET machine_id = $2 WHERE id = $1",
        id,
        machine_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    get_dependency_host(pool, id).await
}

/// Deletes a dependency. Schedules and agent defaults that needed it stop
/// needing it, and catch-ups waiting on it are dropped.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the delete fails.
pub async fn delete_dependency_host(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let deleted = sqlx::query!("DELETE FROM dependency_hosts WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::NotFound(format!("dependency {id} not found")));
    }
    Ok(())
}

/// Every (schedule, agent) that needs a dependency, with where the need comes
/// from. A pair that needs it for several reasons is listed once, by the one
/// closest to the schedule: set on it, then the agent's defaults, then a
/// repository host's.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_dependency_usage(
    pool: &PgPool,
    id: i64,
) -> Result<Vec<DependencyUsageRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT u.schedule_id AS \"schedule_id!\", s.name AS schedule_name, u.agent_id AS \
         \"agent_id!\", a.hostname, MIN(u.rank) AS \"rank!\" FROM (SELECT sd.schedule_id, \
         sd.agent_id, 0 AS rank FROM schedule_dependencies sd WHERE sd.dependency_host_id = $1 \
         UNION ALL SELECT st.schedule_id, st.agent_id, 1 FROM agent_default_dependencies ad JOIN \
         schedule_targets st ON st.agent_id = ad.agent_id WHERE ad.dependency_host_id = $1 UNION \
         ALL SELECT st.schedule_id, st.agent_id, 2 FROM repo_host_dependencies rd JOIN repos r ON \
         r.repo_host_id = rd.repo_host_id JOIN schedule_repos sr ON sr.repo_id = r.id JOIN \
         schedule_targets st ON st.schedule_id = sr.schedule_id WHERE rd.dependency_host_id = $1) \
         u JOIN schedules s ON s.id = u.schedule_id JOIN agents a ON a.id = u.agent_id GROUP BY \
         u.schedule_id, s.name, u.agent_id, a.hostname ORDER BY s.name, a.hostname",
        id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| DependencyUsageRow {
            schedule_id: r.schedule_id,
            schedule_name: r.schedule_name,
            agent_id: r.agent_id,
            hostname: r.hostname,
            source: match r.rank {
                0 => DependencySource::Schedule,
                1 => DependencySource::AgentDefault,
                _ => DependencySource::RepoHost,
            },
        })
        .collect())
}

/// The `(agent_id, dependency_host_id)` pairs a schedule sets itself, ordered
/// by agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_schedule_dependencies(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<(i64, i64)>, ApiError> {
    let rows = sqlx::query!(
        "SELECT agent_id, dependency_host_id FROM schedule_dependencies WHERE schedule_id = $1 \
         ORDER BY agent_id, dependency_host_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| (r.agent_id, r.dependency_host_id))
        .collect())
}

/// Replaces what a schedule sets itself with `pairs` of `(agent_id,
/// dependency_host_id)`.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a dependency does not exist, or
/// [`ApiError::Database`] if a write fails otherwise.
pub async fn replace_schedule_dependencies(
    pool: &PgPool,
    schedule_id: i64,
    pairs: &[(i64, i64)],
) -> Result<(), ApiError> {
    let (agent_ids, dependency_ids): (Vec<i64>, Vec<i64>) = pairs.iter().copied().unzip();
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM schedule_dependencies WHERE schedule_id = $1",
        schedule_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "INSERT INTO schedule_dependencies (schedule_id, agent_id, dependency_host_id) SELECT $1, \
         pair.agent_id, pair.dependency_host_id FROM UNNEST($2::BIGINT[], $3::BIGINT[]) AS \
         pair(agent_id, dependency_host_id) ON CONFLICT DO NOTHING",
        schedule_id,
        &agent_ids,
        &dependency_ids,
    )
    .execute(&mut *tx)
    .await
    .map_err(unknown_dependency)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// The dependencies an agent's backup defaults require of every schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_agent_default_dependencies(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT dependency_host_id FROM agent_default_dependencies WHERE agent_id = $1 ORDER BY \
         dependency_host_id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every agent's default dependencies at once, as `(agent_id,
/// dependency_host_id)` pairs - for a list that would otherwise ask once per
/// agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_all_agent_default_dependencies(
    pool: &PgPool,
) -> Result<Vec<(i64, i64)>, ApiError> {
    let rows = sqlx::query!(
        "SELECT agent_id, dependency_host_id FROM agent_default_dependencies ORDER BY agent_id, \
         dependency_host_id"
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| (r.agent_id, r.dependency_host_id))
        .collect())
}

/// Replaces an agent's default dependencies.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a dependency does not exist, or
/// [`ApiError::Database`] if a write fails otherwise.
pub async fn replace_agent_default_dependencies(
    pool: &PgPool,
    agent_id: i64,
    dependency_ids: &[i64],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM agent_default_dependencies WHERE agent_id = $1",
        agent_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "INSERT INTO agent_default_dependencies (agent_id, dependency_host_id) SELECT $1, id FROM \
         UNNEST($2::BIGINT[]) AS id ON CONFLICT DO NOTHING",
        agent_id,
        dependency_ids,
    )
    .execute(&mut *tx)
    .await
    .map_err(unknown_dependency)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// A foreign-key violation on a dependency list means one of the IDs does not
/// exist, which is the caller's mistake rather than the server's.
fn unknown_dependency(e: sqlx::Error) -> ApiError {
    match &e {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
            ApiError::BadRequest("unknown dependency".to_owned())
        }
        _ => ApiError::Database(e),
    }
}

/// The dependencies a repository host requires of every schedule that writes
/// to it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_repo_host_dependencies(
    pool: &PgPool,
    repo_host_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT dependency_host_id FROM repo_host_dependencies WHERE repo_host_id = $1 ORDER BY \
         dependency_host_id",
        repo_host_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Replaces the dependencies a repository host requires.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a dependency does not exist, or
/// [`ApiError::Database`] if a write fails otherwise.
pub async fn replace_repo_host_dependencies(
    pool: &PgPool,
    repo_host_id: i64,
    dependency_ids: &[i64],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM repo_host_dependencies WHERE repo_host_id = $1",
        repo_host_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "INSERT INTO repo_host_dependencies (repo_host_id, dependency_host_id) SELECT $1, id FROM \
         UNNEST($2::BIGINT[]) AS id ON CONFLICT DO NOTHING",
        repo_host_id,
        dependency_ids,
    )
    .execute(&mut *tx)
    .await
    .map_err(unknown_dependency)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Every dependency the repository hosts of a schedule require, as
/// `(repo_host_id, dependency_host_id)` pairs - what a schedule inherits from
/// the hosts it writes to.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_schedule_repo_host_dependencies(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<(i64, i64)>, ApiError> {
    let rows = sqlx::query!(
        "SELECT DISTINCT rd.repo_host_id, rd.dependency_host_id FROM repo_host_dependencies rd \
         JOIN repos r ON r.repo_host_id = rd.repo_host_id JOIN schedule_repos sr ON sr.repo_id = \
         r.id WHERE sr.schedule_id = $1 ORDER BY rd.repo_host_id, rd.dependency_host_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| (r.repo_host_id, r.dependency_host_id))
        .collect())
}

/// Every dependency one agent needs when it runs one schedule - set on the
/// schedule, required by the agent's defaults, or required by a repository
/// host the schedule writes to - with its machine's wake settings and the
/// schedule's choice for that machine, ordered by name so the run checks them
/// in a stable order.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_required_dependencies(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Vec<RequiredDependency>, ApiError> {
    sqlx::query_as!(
        RequiredDependency,
        "SELECT d.id, d.name, d.address, d.port, d.machine_id, m.intermittent, (SELECT h.id FROM \
         repo_hosts h WHERE h.machine_id = d.machine_id) AS \"repo_host_id?\", m.wake_enabled, \
         m.wake_mac_address, m.wake_broadcast_address, m.wake_timeout_seconds, w.mode AS \"mode?: \
         MachineWakeMode\" FROM dependency_hosts d JOIN machines m ON m.id = d.machine_id LEFT \
         JOIN schedule_machine_wake w ON w.schedule_id = $1 AND w.machine_id = d.machine_id WHERE \
         d.id IN (SELECT dependency_host_id FROM schedule_dependencies WHERE schedule_id = $1 AND \
         agent_id = $2 UNION SELECT dependency_host_id FROM agent_default_dependencies WHERE \
         agent_id = $2 UNION SELECT rd.dependency_host_id FROM repo_host_dependencies rd JOIN \
         repos r ON r.repo_host_id = rd.repo_host_id JOIN schedule_repos sr ON sr.repo_id = r.id \
         WHERE sr.schedule_id = $1) ORDER BY d.name",
        schedule_id,
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// One dependency resolved the same way [`list_required_dependencies`]
/// resolves each of its rows, outside any schedule - for Check now and Test
/// connection.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_required_dependency(
    pool: &PgPool,
    id: i64,
) -> Result<RequiredDependency, ApiError> {
    sqlx::query_as!(
        RequiredDependency,
        "SELECT d.id, d.name, d.address, d.port, d.machine_id, m.intermittent, (SELECT h.id FROM \
         repo_hosts h WHERE h.machine_id = d.machine_id) AS \"repo_host_id?\", m.wake_enabled, \
         m.wake_mac_address, m.wake_broadcast_address, m.wake_timeout_seconds, NULL::TEXT AS \
         \"mode?: MachineWakeMode\" FROM dependency_hosts d JOIN machines m ON m.id = \
         d.machine_id WHERE d.id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))
}

/// Settles the `pending` report rows a run inserted for one agent, for a run a
/// dependency kept from starting: `skipped` when it will be caught up,
/// `failed` when it will not. Left `pending`, they would be replayed as a
/// backup the next time the agent reconnects.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn settle_pending_reports_for_dependency(
    pool: &PgPool,
    run_id: &str,
    agent_id: i64,
    status: shared::types::ReportStatus,
    reason: &str,
) -> Result<(), ApiError> {
    let status = status.to_string();
    sqlx::query!(
        "UPDATE backup_reports SET status = $3, finished_at = NOW(), error_message = $4 WHERE \
         run_id = $1 AND agent_id = $2 AND status = 'pending'",
        run_id,
        agent_id,
        status,
        reason,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
