// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Audit log database queries.
pub mod audit;
/// Missed-run catch-up queries.
pub mod catch_up;
/// Dashboard summary queries.
pub mod dashboard;
/// A host's database dump settings and its database sources.
pub mod databases;
/// Runs skipped because a dependency host was away, waiting to be caught up.
pub mod dependency_catch_ups;
/// Dependency hosts: machines a backup needs besides its agent and repository.
pub mod dependency_hosts;
/// Machines: the wake, shutdown and availability settings agents,
/// repository hosts and dependency hosts share.
pub mod machines;
/// What the Prometheus `/metrics` endpoint reads.
pub mod metrics;
/// Hostname pattern-matching queries.
pub mod patterns;
/// Persisted holds on power-managed hosts, so a restart mid-run does not
/// forget a machine it woke.
pub mod power_holds;
/// Quota database queries.
pub mod quota;
/// Repository hosts: the machines borg writes to, and their address, key,
/// power and availability settings.
pub mod repo_hosts;
/// Restores of archive files onto an agent.
pub mod restore_runs;
/// Backup run power-management event log queries.
pub mod run_events;
/// Server-level quota database queries.
pub mod server_quota;
/// Tag database queries.
pub mod tags;
/// Queries for a host's virtual-machine staging settings and its domains.
pub mod vms;

mod access;
mod agents;
mod api_tokens;
mod events;
mod reports;
mod repos;
mod schedule_sources;
mod schedules;
mod settings;
mod stats;
mod tag_catalog;
mod tunnels;
mod users;

pub use access::*;
pub use agents::*;
pub use api_tokens::*;
pub use events::*;
pub use reports::*;
pub use repos::*;
pub use schedule_sources::*;
pub use schedules::*;
pub use settings::*;
use shared::hooks::HookCommand;
pub use stats::*;
pub use tag_catalog::*;
pub use tunnels::*;
pub use users::*;

/// Stored shape of a pre/post-backup hook command list.
///
/// Named rather than spelled out at every use site because a `query_as!`
/// column type override travels to the database as the column's alias, and
/// Postgres truncates identifiers at 63 bytes - the fully qualified
/// `sqlx::types::Json<Vec<HookCommand>>` pushes the longest of these aliases
/// past that limit and the macro then fails to parse the truncated name.
pub type HookCommands = sqlx::types::Json<Vec<HookCommand>>;
