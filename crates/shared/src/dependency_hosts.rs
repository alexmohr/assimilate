// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Wire types for dependency hosts: machines a backup needs besides its agent
//! and its repository, such as the server whose share a pre-backup command
//! mounts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// The repository host a dependency is the same machine as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyRepoHostRef {
    #[ts(type = "number")]
    /// Repository host ID.
    pub id: i64,
    /// Its hostname.
    pub ssh_host: String,
}

/// How a dependency is woken.
///
/// `repo_host` set means the dependency shares that repository host's wake
/// settings, and `effective_*` are that host's. Otherwise the `own_*` values
/// apply and `effective_*` repeat them. The `own_*` values are kept either way,
/// so switching back to them restores what was there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyPowerResponse {
    /// The repository host whose wake settings apply, if any.
    pub repo_host: Option<DependencyRepoHostRef>,
    /// The dependency's own wake switch.
    pub wake_enabled: bool,
    /// Its own MAC address. `null` for a viewer who may not see it.
    pub wake_mac_address: Option<String>,
    /// Its own broadcast address. `null` for a viewer who may not see it.
    pub wake_broadcast_address: Option<String>,
    /// Its own wait after a wake, in seconds.
    pub wake_timeout_seconds: i32,
    /// Whether a run wakes it, after resolving `repo_host`.
    pub effective_wake_enabled: bool,
    /// The MAC address a wake is sent to. `null` for a viewer who may not see it.
    pub effective_wake_mac_address: Option<String>,
    /// The broadcast address it is sent to. `null` for a viewer who may not
    /// see it, or for the global default.
    pub effective_wake_broadcast_address: Option<String>,
    /// How long a run waits after a wake, in seconds.
    pub effective_wake_timeout_seconds: i32,
}

/// A dependency host - a card on the list, and the detail page's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyHostResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Hostname or IP address the server connects to.
    pub address: String,
    /// TCP port whose answer means the machine is up.
    pub port: i32,
    /// What it is for.
    pub description: String,
    /// How it is woken.
    pub power: DependencyPowerResponse,
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back while a run waits on it.
    pub catch_up_recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
    /// When it was last checked, by a run, the poller, Check now or Test
    /// connection.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// What that check found; `null` when it has never been checked.
    pub last_check_reachable: Option<bool>,
    #[ts(type = "number")]
    /// Schedules that need it.
    pub schedule_count: i64,
    #[ts(type = "number")]
    /// Agents whose backup defaults require it.
    pub agent_default_count: i64,
    #[ts(type = "number")]
    /// Skipped runs waiting on it to catch up.
    pub waiting_count: i64,
}

/// Where a schedule's need for a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DependencySourceKind {
    /// Set on the schedule, for that agent.
    Schedule,
    /// Required by the agent's backup defaults.
    AgentDefault,
}

/// One (schedule, agent) that needs a dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyUsageResponse {
    #[ts(type = "number")]
    /// Schedule that needs it.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    #[ts(type = "number")]
    /// Agent the schedule runs on.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    /// Where the need comes from.
    pub source: DependencySourceKind,
}

/// One target waiting on a dependency to catch up a run it skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyWaitResponse {
    #[ts(type = "number")]
    /// Schedule whose run was skipped.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    #[ts(type = "number")]
    /// Agent whose run was skipped.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    #[ts(type = "number")]
    /// The dependency it waits on.
    pub dependency_host_id: i64,
    /// Its name.
    pub dependency_name: String,
    /// The occurrence that was skipped.
    pub pending_for: DateTime<Utc>,
    /// When the dependency was last asked whether it is back.
    pub last_probe_at: Option<DateTime<Utc>>,
    /// When it is next due to be asked; `null` while a catch-up is running.
    pub next_probe_at: Option<DateTime<Utc>>,
    /// When the wait is abandoned; `null` when it is waited for indefinitely.
    pub give_up_at: Option<DateTime<Utc>>,
    /// Whether the dependency answered and the catch-up run is under way.
    pub catching_up: bool,
}

/// A dependency's "when the host is offline" settings and what is waiting on
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyAvailabilityResponse {
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back.
    pub catch_up_recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub catch_up_give_up_minutes: i32,
    /// Skipped runs waiting on it. Empty while it is not marked as not always
    /// online, since nothing then waits.
    pub waiting: Vec<DependencyWaitResponse>,
}

/// What a connection test found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DependencyTestResponse {
    /// Whether the port answered.
    pub reachable: bool,
    /// The address that was asked.
    pub address: String,
    /// The port that was asked.
    pub port: i32,
}

/// One dependency one agent needs when it runs a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleDependencyResponse {
    #[ts(type = "number")]
    /// The agent.
    pub agent_id: i64,
    #[ts(type = "number")]
    /// The dependency.
    pub dependency_host_id: i64,
    /// Its name.
    pub dependency_name: String,
    /// Where the need comes from. A dependency both set on the schedule and
    /// required by the agent's defaults is reported once, as the latter: it
    /// cannot be removed on the schedule.
    pub source: DependencySourceKind,
    /// Whether it answered the last time it was checked; `null` when it never
    /// has been.
    pub last_check_reachable: Option<bool>,
}

/// The dependencies of every agent of one schedule, and the runs waiting on
/// them - the schedule's Dependencies pane and Overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleDependenciesResponse {
    /// One entry per (agent, dependency), ordered by agent, then name.
    pub dependencies: Vec<ScheduleDependencyResponse>,
    /// Targets of this schedule waiting on a dependency.
    pub waiting: Vec<DependencyWaitResponse>,
}

/// One (agent, dependency) pair a schedule sets itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleDependencyInput {
    #[ts(type = "number")]
    /// The agent.
    pub agent_id: i64,
    #[ts(type = "number")]
    /// The dependency.
    pub dependency_host_id: i64,
}

/// Replaces the dependencies a schedule sets itself. Those an agent's
/// backup defaults require are added on top and need not be listed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateScheduleDependenciesRequest {
    /// Every (agent, dependency) pair the schedule sets.
    pub dependencies: Vec<ScheduleDependencyInput>,
}

/// An agent's default dependencies: the ones every schedule on it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct AgentDependenciesBody {
    #[ts(type = "number[]")]
    /// The dependencies, by ID.
    pub dependency_host_ids: Vec<i64>,
}
