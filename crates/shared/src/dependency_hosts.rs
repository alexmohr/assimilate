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
    #[ts(type = "number")]
    /// The machine it is on, which holds how it is woken and whether it is
    /// always online.
    pub machine_id: i64,
    /// The repository host on the same machine, if any.
    pub repo_host: Option<DependencyRepoHostRef>,
    /// Whether its machine is marked as not always online.
    pub intermittent: bool,
    /// Whether its machine wakes by default when a run needs it.
    pub wake_enabled: bool,
    /// When its machine was last checked, by a run, a poller, Check now or
    /// Test connection.
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
    /// Required by a repository host the schedule writes to.
    RepoHost,
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
    /// How long the port was given to answer, in seconds.
    pub timeout_seconds: u32,
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
    /// Where the need comes from. A dependency the schedule both sets and
    /// inherits is reported once, as inherited: it cannot be removed on the
    /// schedule.
    pub source: DependencySourceKind,
    /// The repository host it is inherited from, for
    /// [`DependencySourceKind::RepoHost`].
    pub repo_host: Option<DependencyRepoHostRef>,
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

/// A repository host's dependencies: the ones every schedule writing to it
/// needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RepoHostDependenciesBody {
    #[ts(type = "number[]")]
    /// The dependencies, by ID.
    pub dependency_host_ids: Vec<i64>,
}

/// Moves a dependency onto the machine of a repository host, or onto a
/// machine of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateDependencyMachineRequest {
    #[ts(type = "number | null")]
    /// The repository host it is the same machine as, or `null` for a machine
    /// of its own with default settings.
    pub repo_host_id: Option<i64>,
}

/// An agent's default dependencies: the ones every schedule on it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct AgentDependenciesBody {
    #[ts(type = "number[]")]
    /// The dependencies, by ID.
    pub dependency_host_ids: Vec<i64>,
}
