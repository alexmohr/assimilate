// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Wire types for machines: what is woken, shut down, and expected to be
//! online or not. An agent, a repository host and a dependency host are each a
//! role of one machine, and every one of them shows the same Power &
//! dependencies pane for it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::{
    responses::HostWakeSettingsResponse,
    types::{MachineWakeMode, ScheduleWakeOverride},
};

/// The agent a machine runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineAgentRef {
    #[ts(type = "number")]
    /// Agent ID.
    pub id: i64,
    /// Its hostname.
    pub hostname: String,
    /// Its domain, for an agent that shares its hostname.
    pub domain: Option<String>,
}

/// The repository host a machine is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineRepoHostRef {
    #[ts(type = "number")]
    /// Repository host ID.
    pub id: i64,
    /// Its hostname.
    pub ssh_host: String,
}

/// A dependency host on a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineDependencyRef {
    #[ts(type = "number")]
    /// Dependency host ID.
    pub id: i64,
    /// Its name.
    pub name: String,
}

/// Every role a machine plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineRolesResponse {
    /// The agent running on it.
    pub agent: Option<MachineAgentRef>,
    /// The repository host it is.
    pub repo_host: Option<MachineRepoHostRef>,
    /// The dependency hosts on it, by name.
    pub dependencies: Vec<MachineDependencyRef>,
}

/// Whether a machine is expected to be online, and how a run waiting on it
/// behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineAvailabilityResponse {
    /// Whether it is marked as not always online. Off, a machine that does not
    /// answer fails the run - unless a schedule is set to wait for it; on, the
    /// run is skipped and caught up once it answers.
    pub intermittent: bool,
    /// How often, in minutes, it is asked whether it is back while a run waits
    /// on it. Not used for a machine that is only an agent, which reconnects
    /// on its own.
    pub catch_up_recheck_minutes: i32,
    /// How long, in minutes, a run waits on it before it is abandoned. Zero
    /// waits indefinitely.
    pub catch_up_give_up_minutes: i32,
}

/// Which role of a machine a run waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MachineRoleKind {
    /// The agent: waited for to reconnect.
    Agent,
    /// The repository host: asked over SSH.
    RepoHost,
    /// A dependency host: asked on its port.
    Dependency,
}

/// One run waiting on a machine to catch up a skipped occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineWaitResponse {
    /// Which role of the machine it waits on.
    pub role: MachineRoleKind,
    #[ts(type = "number")]
    /// The schedule whose run was skipped.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    /// The agent whose run was skipped, for an agent or a dependency.
    pub hostname: Option<String>,
    #[ts(type = "number | null")]
    /// The repository that was not written, for a repository host.
    pub repo_id: Option<i64>,
    /// That repository's name.
    pub repo_name: Option<String>,
    /// The dependency it waits on, for a dependency.
    pub dependency_name: Option<String>,
    /// The occurrence that was skipped.
    pub pending_for: DateTime<Utc>,
    /// When the machine was last asked whether it is back; always `null` for
    /// an agent, which announces its own return.
    pub last_probe_at: Option<DateTime<Utc>>,
    /// When it is next due to be asked; `null` for an agent, or while a
    /// catch-up is running.
    pub next_probe_at: Option<DateTime<Utc>>,
    /// When the wait is abandoned; `null` when it is waited for indefinitely.
    pub give_up_at: Option<DateTime<Utc>>,
}

/// A schedule that treats a machine differently from the machine's own
/// setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineScheduleOverrideResponse {
    #[ts(type = "number")]
    /// The schedule.
    pub schedule_id: i64,
    /// Its name.
    pub schedule_name: String,
    /// Its schedule-wide wake setting.
    pub wake_override: ScheduleWakeOverride,
    /// Its own choice for this machine, if it made one.
    pub mode: Option<MachineWakeMode>,
}

/// A machine: its roles, how it is woken and shut down, whether it is always
/// online, and what waits on it - the Power & dependencies pane.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// A short name: the repository host's hostname, then the agent's, then
    /// the first dependency's name.
    pub label: String,
    /// Every role it plays.
    pub roles: MachineRolesResponse,
    /// How it is woken and shut down. The MAC and broadcast addresses are
    /// `null` for a viewer who may not see them.
    pub power: HostWakeSettingsResponse,
    /// Whether it can be shut down after a backup at all: a machine that is
    /// only a dependency never is.
    pub can_shut_down: bool,
    /// Whether it is expected to be online.
    pub availability: MachineAvailabilityResponse,
    /// When it was last checked.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// What that check found; `null` when it never has been.
    pub last_check_reachable: Option<bool>,
    /// Runs waiting on it.
    pub waiting: Vec<MachineWaitResponse>,
    /// Schedules that do not leave it to the machine's own settings. Empty for
    /// anyone but an admin.
    pub schedule_overrides: Vec<MachineScheduleOverrideResponse>,
}

fn default_wake_timeout_seconds() -> i32 {
    180
}

/// How a machine is woken and shut down.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateMachinePowerRequest {
    /// Whether a run wakes it by default when it does not answer.
    pub wake_enabled: bool,
    /// MAC address a wake is sent to; required to wake it or shut it down.
    pub wake_mac_address: Option<String>,
    /// IPv4 broadcast address the magic packet is sent to; the global default
    /// when unset.
    pub wake_broadcast_address: Option<String>,
    /// How long a run waits for it after a wake, in seconds.
    #[serde(default = "default_wake_timeout_seconds")]
    pub wake_timeout_seconds: i32,
    /// Whether it is shut down after a backup, when that run is what woke it.
    pub shutdown_after_backup: bool,
}

/// Whether a machine is expected to be online.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateMachineAvailabilityRequest {
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// How often, in minutes, it is asked whether it is back (1-10080).
    pub catch_up_recheck_minutes: i32,
    /// How long, in minutes, a run waits on it. Zero waits indefinitely;
    /// anything else must leave room for one re-check (up to 43200).
    pub catch_up_give_up_minutes: i32,
}

/// What asking a machine whether it is back did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct MachineCheckResponse {
    /// Whether it answered: on SSH as a repository host, on its port as a
    /// dependency, or by being connected as an agent.
    pub reachable: bool,
    #[ts(type = "number")]
    /// Catch-up runs started as a result.
    pub started: usize,
    #[ts(type = "number")]
    /// Waits abandoned because their give-up window had passed.
    pub abandoned: usize,
    #[ts(type = "number")]
    /// Waits dropped without a run, because they no longer qualify or the
    /// schedule's next run is too close.
    pub dropped: usize,
}

/// How a machine a schedule needs is woken by its own settings - what the
/// schedule's choice is resolved against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleMachinePowerResponse {
    /// Whether the machine wakes by default.
    pub wake_enabled: bool,
    /// Whether it has a MAC address on file. `true` for a viewer who may not
    /// see it, so the pane never reports "cannot wake" for lack of
    /// permission.
    pub has_mac_address: bool,
    /// Whether it is shut down after a backup that woke it.
    pub shutdown_after_backup: bool,
}

/// One machine a schedule needs, and what the schedule does about it - a row
/// of its Power pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleMachineResponse {
    #[ts(type = "number")]
    /// The machine.
    pub machine_id: i64,
    /// Its short name.
    pub label: String,
    /// What the schedule needs it as.
    pub roles: Vec<MachineRoleKind>,
    /// How it is woken by its own settings.
    pub power: ScheduleMachinePowerResponse,
    /// Whether it is marked as not always online.
    pub intermittent: bool,
    /// The schedule's own choice for it, if it made one.
    pub mode: Option<MachineWakeMode>,
    /// Whether [`MachineWakeMode::Wait`] may be chosen: not for an agent,
    /// which has nothing to run the backup when it is not there.
    pub can_wait: bool,
}

/// Every machine a schedule needs, in the order a run meets them: agents,
/// then repository hosts, then dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleMachinesResponse {
    /// The machines.
    pub machines: Vec<ScheduleMachineResponse>,
}

/// One machine a schedule makes its own choice for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ScheduleMachineInput {
    #[ts(type = "number")]
    /// The machine.
    pub machine_id: i64,
    /// What the schedule does about it.
    pub mode: MachineWakeMode,
}

/// Replaces every per-machine choice a schedule makes. A machine left out
/// follows the schedule-wide setting.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateScheduleMachinesRequest {
    /// The choices.
    pub machines: Vec<ScheduleMachineInput>,
}
