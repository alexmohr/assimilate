// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Dependency hosts: machines a backup needs besides its agent and its
//! repository - the server whose share a pre-backup command mounts, say.
//!
//! A dependency is checked by connecting to one TCP port. It can be woken,
//! either with its own Wake-on-LAN settings or, when it is the same machine as
//! a repository host, with that host's, so one machine has one MAC address and
//! one shut-down switch. Which dependencies a run needs is set per schedule and
//! agent, plus whatever the agent's backup defaults require of every schedule.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `dependency_hosts` table.
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
    /// The repository host this is the same machine as, whose wake settings
    /// it shares. `None` uses the columns below.
    pub repo_host_id: Option<i64>,
    /// Whether to wake it before a backup if it does not answer. Ignored
    /// while `repo_host_id` is set.
    pub wake_enabled: bool,
    /// MAC address to wake.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to.
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the port to answer after a wake.
    pub wake_timeout_seconds: i32,
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back while a catch-up waits on it.
    pub catch_up_recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
    /// When it was last checked, by anything.
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
    /// an agent's defaults.
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
    /// Repository host it is the same machine as.
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

/// The wake settings an edit can change.
#[derive(Debug, Clone, Copy)]
pub struct DependencyPowerPatch<'a> {
    /// Repository host to share wake settings with, or `None` for its own.
    pub repo_host_id: Option<i64>,
    /// Whether to wake it.
    pub wake_enabled: bool,
    /// MAC address to wake.
    pub wake_mac_address: Option<&'a str>,
    /// Broadcast address for the magic packet.
    pub wake_broadcast_address: Option<&'a str>,
    /// How long to wait after a wake.
    pub wake_timeout_seconds: i32,
}

/// The "when the host is offline" settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyAvailability {
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back.
    pub recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub give_up_minutes: i32,
}

/// Where a schedule's need for a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySource {
    /// Set on the schedule itself, for this agent.
    Schedule,
    /// Inherited from the agent's backup defaults.
    AgentDefault,
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
    /// Where the need comes from. Set on the schedule wins when both apply.
    pub source: DependencySource,
}

/// A dependency one target needs, with the wake settings that apply to it
/// already resolved - its own, or those of the repository host it shares a
/// machine with.
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
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// Repository host it shares a machine with, if any.
    pub repo_host_id: Option<i64>,
    /// Whether waking is on, by its own setting or its repository host's.
    pub wake_enabled: bool,
    /// MAC address to wake.
    pub wake_mac_address: Option<String>,
    /// Broadcast address for the magic packet.
    pub wake_broadcast_address: Option<String>,
    /// How long to wait after a wake.
    pub wake_timeout_seconds: i32,
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
    let rows = sqlx::query!(
        "SELECT d.id, d.name, d.address, d.port, d.description, d.repo_host_id, d.wake_enabled, \
         d.wake_mac_address, d.wake_broadcast_address, d.wake_timeout_seconds, d.intermittent, \
         d.catch_up_recheck_minutes, d.catch_up_give_up_minutes, d.last_checked_at, \
         d.last_check_reachable, (SELECT COUNT(DISTINCT u.schedule_id) FROM (SELECT \
         sd.schedule_id FROM schedule_dependencies sd WHERE sd.dependency_host_id = d.id UNION \
         SELECT st.schedule_id FROM agent_default_dependencies ad JOIN schedule_targets st ON \
         st.agent_id = ad.agent_id WHERE ad.dependency_host_id = d.id) u) AS \"schedule_count!\", \
         (SELECT COUNT(*) FROM agent_default_dependencies ad WHERE ad.dependency_host_id = d.id) \
         AS \"agent_default_count!\", (SELECT COUNT(*) FROM dependency_catch_ups c WHERE \
         c.dependency_host_id = d.id) AS \"waiting_count!\" FROM dependency_hosts d ORDER BY \
         d.name"
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
                repo_host_id: r.repo_host_id,
                wake_enabled: r.wake_enabled,
                wake_mac_address: r.wake_mac_address,
                wake_broadcast_address: r.wake_broadcast_address,
                wake_timeout_seconds: r.wake_timeout_seconds,
                intermittent: r.intermittent,
                catch_up_recheck_minutes: r.catch_up_recheck_minutes,
                catch_up_give_up_minutes: r.catch_up_give_up_minutes,
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
        "SELECT id, name, address, port, description, repo_host_id, wake_enabled, \
         wake_mac_address, wake_broadcast_address, wake_timeout_seconds, intermittent, \
         catch_up_recheck_minutes, catch_up_give_up_minutes, last_checked_at, \
         last_check_reachable FROM dependency_hosts WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))
}

/// Creates a dependency.
///
/// # Errors
///
/// Returns [`ApiError::Conflict`] if the name is taken, or
/// [`ApiError::Database`] if the insert fails otherwise.
pub async fn insert_dependency_host(
    pool: &PgPool,
    new: &NewDependencyHost<'_>,
) -> Result<DependencyHostRow, ApiError> {
    sqlx::query_as!(
        DependencyHostRow,
        "INSERT INTO dependency_hosts (name, address, port, description, repo_host_id) VALUES \
         ($1, $2, $3, $4, $5) RETURNING id, name, address, port, description, repo_host_id, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, last_checked_at, \
         last_check_reachable",
        new.name,
        new.address,
        new.port,
        new.description,
        new.repo_host_id,
    )
    .fetch_one(pool)
    .await
    .map_err(name_taken(new.name))
}

/// Changes a dependency's name, address, port and description.
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
    let row = sqlx::query_as!(
        DependencyHostRow,
        "UPDATE dependency_hosts SET name = $2, address = $3, port = $4, description = $5, \
         last_checked_at = NULL, last_check_reachable = NULL WHERE id = $1 RETURNING id, name, \
         address, port, description, repo_host_id, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, intermittent, catch_up_recheck_minutes, \
         catch_up_give_up_minutes, last_checked_at, last_check_reachable",
        id,
        patch.name,
        patch.address,
        patch.port,
        patch.description,
    )
    .fetch_optional(pool)
    .await
    .map_err(name_taken(patch.name))?;
    row.ok_or_else(|| ApiError::NotFound(format!("dependency {id} not found")))
}

/// Changes how a dependency is woken.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the update fails.
pub async fn update_dependency_host_power(
    pool: &PgPool,
    id: i64,
    patch: &DependencyPowerPatch<'_>,
) -> Result<DependencyHostRow, ApiError> {
    sqlx::query_as!(
        DependencyHostRow,
        "UPDATE dependency_hosts SET repo_host_id = $2, wake_enabled = $3, wake_mac_address = $4, \
         wake_broadcast_address = $5, wake_timeout_seconds = $6 WHERE id = $1 RETURNING id, name, \
         address, port, description, repo_host_id, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, intermittent, catch_up_recheck_minutes, \
         catch_up_give_up_minutes, last_checked_at, last_check_reachable",
        id,
        patch.repo_host_id,
        patch.wake_enabled,
        patch.wake_mac_address,
        patch.wake_broadcast_address,
        patch.wake_timeout_seconds,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))
}

/// Changes a dependency's "when the host is offline" settings. Switching
/// "not always online" off drops every catch-up waiting on it: nobody is
/// waiting for it any more, and a miss recorded while it was must not run days
/// later because somebody switched it back on.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the update fails.
pub async fn update_dependency_host_availability(
    pool: &PgPool,
    id: i64,
    availability: DependencyAvailability,
) -> Result<DependencyHostRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let row = sqlx::query_as!(
        DependencyHostRow,
        "UPDATE dependency_hosts SET intermittent = $2, catch_up_recheck_minutes = $3, \
         catch_up_give_up_minutes = $4 WHERE id = $1 RETURNING id, name, address, port, \
         description, repo_host_id, wake_enabled, wake_mac_address, wake_broadcast_address, \
         wake_timeout_seconds, intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, \
         last_checked_at, last_check_reachable",
        id,
        availability.intermittent,
        availability.recheck_minutes,
        availability.give_up_minutes,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(not_found(id))?;
    if !availability.intermittent {
        sqlx::query!(
            "DELETE FROM dependency_catch_ups WHERE dependency_host_id = $1",
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(row)
}

/// Remembers what the latest check of a dependency found, so the list can show
/// it without probing every machine on every load.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the update fails.
pub async fn record_dependency_check(
    pool: &PgPool,
    id: i64,
    reachable: bool,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE dependency_hosts SET last_checked_at = $2, last_check_reachable = $3 WHERE id = $1",
        id,
        at,
        reachable,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
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
/// from. A pair that both sets it and inherits it is listed once, as set on
/// the schedule.
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
         \"agent_id!\", a.hostname, bool_or(u.from_schedule) AS \"from_schedule!\" FROM (SELECT \
         sd.schedule_id, sd.agent_id, true AS from_schedule FROM schedule_dependencies sd WHERE \
         sd.dependency_host_id = $1 UNION ALL SELECT st.schedule_id, st.agent_id, false FROM \
         agent_default_dependencies ad JOIN schedule_targets st ON st.agent_id = ad.agent_id \
         WHERE ad.dependency_host_id = $1) u JOIN schedules s ON s.id = u.schedule_id JOIN agents \
         a ON a.id = u.agent_id GROUP BY u.schedule_id, s.name, u.agent_id, a.hostname ORDER BY \
         s.name, a.hostname",
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
            source: if r.from_schedule {
                DependencySource::Schedule
            } else {
                DependencySource::AgentDefault
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

/// Every dependency one agent needs when it runs one schedule - set on the
/// schedule or required by the agent's defaults - with the wake settings that
/// apply resolved, ordered by name so the run checks them in a stable order.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn list_required_dependencies(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Vec<RequiredDependency>, ApiError> {
    let rows = sqlx::query!(
        "SELECT d.id, d.name, d.address, d.port, d.intermittent, rh.id AS \"repo_host_id?\", \
         COALESCE(rh.wake_enabled, d.wake_enabled) AS \"wake_enabled!\", CASE WHEN rh.id IS NULL \
         THEN d.wake_mac_address ELSE rh.wake_mac_address END AS wake_mac_address, CASE WHEN \
         rh.id IS NULL THEN d.wake_broadcast_address ELSE rh.wake_broadcast_address END AS \
         wake_broadcast_address, COALESCE(rh.wake_timeout_seconds, d.wake_timeout_seconds) AS \
         \"wake_timeout_seconds!\" FROM dependency_hosts d LEFT JOIN repo_hosts rh ON rh.id = \
         d.repo_host_id WHERE d.id IN (SELECT dependency_host_id FROM schedule_dependencies WHERE \
         schedule_id = $1 AND agent_id = $2 UNION SELECT dependency_host_id FROM \
         agent_default_dependencies WHERE agent_id = $2) ORDER BY d.name",
        schedule_id,
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|r| RequiredDependency {
            id: r.id,
            name: r.name,
            address: r.address,
            port: r.port,
            intermittent: r.intermittent,
            repo_host_id: r.repo_host_id,
            wake_enabled: r.wake_enabled,
            wake_mac_address: r.wake_mac_address,
            wake_broadcast_address: r.wake_broadcast_address,
            wake_timeout_seconds: r.wake_timeout_seconds,
        })
        .collect())
}

/// One dependency resolved the same way [`list_required_dependencies`]
/// resolves each of its rows - for Check now and Test connection, which act
/// on a single dependency outside any run.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if it does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_required_dependency(
    pool: &PgPool,
    id: i64,
) -> Result<RequiredDependency, ApiError> {
    let r = sqlx::query!(
        "SELECT d.id, d.name, d.address, d.port, d.intermittent, rh.id AS \"repo_host_id?\", \
         COALESCE(rh.wake_enabled, d.wake_enabled) AS \"wake_enabled!\", CASE WHEN rh.id IS NULL \
         THEN d.wake_mac_address ELSE rh.wake_mac_address END AS wake_mac_address, CASE WHEN \
         rh.id IS NULL THEN d.wake_broadcast_address ELSE rh.wake_broadcast_address END AS \
         wake_broadcast_address, COALESCE(rh.wake_timeout_seconds, d.wake_timeout_seconds) AS \
         \"wake_timeout_seconds!\" FROM dependency_hosts d LEFT JOIN repo_hosts rh ON rh.id = \
         d.repo_host_id WHERE d.id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(not_found(id))?;
    Ok(RequiredDependency {
        id: r.id,
        name: r.name,
        address: r.address,
        port: r.port,
        intermittent: r.intermittent,
        repo_host_id: r.repo_host_id,
        wake_enabled: r.wake_enabled,
        wake_mac_address: r.wake_mac_address,
        wake_broadcast_address: r.wake_broadcast_address,
        wake_timeout_seconds: r.wake_timeout_seconds,
    })
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
