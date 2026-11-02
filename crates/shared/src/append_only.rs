// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Append-only repositories: the API types for the key setup, the maintenance
//! schedule and the deletions waiting for it.
//!
//! Append-only is enforced by the repository host, not by borg's repository
//! config: the key the agents use is restricted to
//! `borg serve --append-only` in the host's `authorized_keys`, and the server
//! keeps a second, full-access key that it alone can use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How a maintenance run ended.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum MaintenanceOutcome {
    /// Everything ran.
    Ok,
    /// Archives vanished that Assimilate did not delete, so nothing was
    /// written.
    Stopped,
    /// `borg check` found a problem, so pruning was skipped.
    CheckFailed,
    /// A borg command or the connection failed.
    Failed,
}

/// Which key a deletion request came from.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DeletionReason {
    /// A user deleted the archive in the UI.
    Manual,
}

/// An SSH public key Assimilate manages on the repository host.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlyKeyResponse {
    /// `SHA256:...` fingerprint of the key.
    pub fingerprint: String,
    /// The line Assimilate keeps in the host's `authorized_keys`, for a host
    /// managed by hand.
    pub authorized_keys_line: String,
}

/// The SSH login (host and user) an append-only setting covers. sshd runs one
/// forced command per key per login, so every repository on the login shares it.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlyLoginResponse {
    /// Repository host name.
    pub ssh_host: String,
    /// SSH user the repositories log in as.
    pub ssh_user: String,
    /// The other repositories that log in the same way, which switch together
    /// with this one.
    pub sibling_repos: Vec<AppendOnlySiblingResponse>,
}

/// Another repository on the same SSH login.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlySiblingResponse {
    #[ts(type = "number")]
    /// Repository ID.
    pub id: i64,
    /// Repository name.
    pub name: String,
}

/// The result of connecting to the repository host with each key.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlyCheckResponse {
    /// When the connections were tried.
    pub checked_at: DateTime<Utc>,
    /// Whether the key agents use reaches borg.
    pub backup_key_ok: bool,
    /// Whether the maintenance key reaches the host.
    pub maintenance_key_ok: bool,
    /// Whether the host's `authorized_keys` restricts the agents' key to
    /// append-only access to exactly this login's repositories. borg cannot be
    /// asked whether a connection is append-only, so this is read from the
    /// file.
    pub restriction_in_place: bool,
    /// What failed, if anything.
    pub error: Option<String>,
}

/// Why maintenance stopped and what it saw.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MaintenanceStoppedResponse {
    /// When maintenance stopped.
    pub since: DateTime<Utc>,
    /// Archives that are gone from borg although Assimilate did not delete
    /// them.
    pub vanished_archives: Vec<String>,
}

/// One maintenance run.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MaintenanceRunResponse {
    #[ts(type = "number")]
    /// Run ID.
    pub id: i64,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the run ended, if it has.
    pub finished_at: Option<DateTime<Utc>>,
    #[ts(type = "string")]
    /// How the run ended.
    pub outcome: MaintenanceOutcome,
    #[ts(type = "number")]
    /// Archives the run removed.
    pub archives_removed: i64,
    #[ts(type = "number")]
    /// Repository space the run freed, in bytes.
    pub bytes_freed: i64,
    /// What happened, when the outcome is not `ok`.
    pub detail: Option<String>,
}

/// The repository's maintenance schedule and where it stands.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MaintenanceSettingsResponse {
    /// Cron expression, in the server's timezone.
    pub cron_expression: String,
    /// Whether `borg check --repository-only` runs before anything is written.
    pub check_before_prune: bool,
    /// When maintenance runs next, or `None` while it is stopped.
    pub next_run_at: Option<DateTime<Utc>>,
    /// When maintenance last ran.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Set while maintenance is stopped, waiting for an admin.
    pub stopped: Option<MaintenanceStoppedResponse>,
}

/// An archive deletion waiting for the next maintenance run.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PendingDeletionResponse {
    /// Archive name.
    pub archive_name: String,
    #[ts(type = "string")]
    /// Why it is queued.
    pub reason: DeletionReason,
    /// Who queued it.
    pub requested_by: Option<String>,
    /// When it was queued.
    pub requested_at: DateTime<Utc>,
}

/// Why setting append-only up, or taking it away, did not work.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlySetupFailure {
    /// Whether the host's SSH password is needed once, because the server's
    /// own key cannot edit the host's `authorized_keys`.
    pub needs_password: bool,
    /// What went wrong.
    pub message: String,
}

/// What a repository list entry says about a repository whose login is
/// append-only.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoAppendOnlyResponse {
    /// Whether maintenance is stopped, waiting for an admin to look at
    /// archives that vanished.
    pub maintenance_stopped: bool,
    #[ts(type = "number")]
    /// Archive deletions waiting for the next maintenance run.
    pub pending_deletion_count: i64,
}

/// Everything the Append-only settings section shows.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AppendOnlyStatusResponse {
    /// Whether the repository's login is append-only.
    pub enabled: bool,
    /// The login the setting covers.
    pub login: AppendOnlyLoginResponse,
    /// When the keys were last installed on the host.
    pub installed_at: Option<DateTime<Utc>>,
    /// The result of the last connection check.
    pub check: Option<AppendOnlyCheckResponse>,
    /// The key the agents use.
    pub backup_key: AppendOnlyKeyResponse,
    /// The key only the server uses. `None` until it has been generated.
    pub maintenance_key: Option<AppendOnlyKeyResponse>,
    /// The maintenance schedule. `None` while append-only is off.
    pub maintenance: Option<MaintenanceSettingsResponse>,
    /// Deletions waiting for the next maintenance run.
    pub pending_deletions: Vec<PendingDeletionResponse>,
    /// The most recent maintenance runs, newest first.
    pub recent_runs: Vec<MaintenanceRunResponse>,
    /// Set when the request that returned this status did not work.
    pub setup_failure: Option<AppendOnlySetupFailure>,
}

/// Turns append-only on or off for a repository's login.
#[derive(Clone, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SetAppendOnlyRequest {
    /// The new state.
    pub enabled: bool,
    /// The SSH password of the login, used once and never stored, for a host
    /// the server key cannot edit.
    pub password: Option<String>,
}

/// Rewrites the host's `authorized_keys` to match the current repositories.
#[derive(Clone, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ReapplyAppendOnlyRequest {
    /// The SSH password of the login, used once and never stored.
    pub password: Option<String>,
}

/// Changes the maintenance schedule.
#[derive(Debug, Clone, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct UpdateMaintenanceRequest {
    /// Cron expression, in the server's timezone.
    pub cron_expression: String,
    /// Whether to check the repository before anything is written.
    pub check_before_prune: bool,
}

/// The one-time SSH password is a secret: it is never shown, not even in a
/// debug print of the request that carries it.
fn redacted(password: Option<&String>) -> &'static str {
    password.map_or("None", |_| "Some([REDACTED])")
}

impl std::fmt::Debug for SetAppendOnlyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetAppendOnlyRequest")
            .field("enabled", &self.enabled)
            .field(
                "password",
                &format_args!("{}", redacted(self.password.as_ref())),
            )
            .finish()
    }
}

impl std::fmt::Debug for ReapplyAppendOnlyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReapplyAppendOnlyRequest")
            .field(
                "password",
                &format_args!("{}", redacted(self.password.as_ref())),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_never_prints_the_password() {
        let set = SetAppendOnlyRequest {
            enabled: true,
            password: Some("hunter2".to_owned()),
        };
        let reapply = ReapplyAppendOnlyRequest {
            password: Some("hunter2".to_owned()),
        };
        for printed in [format!("{set:?}"), format!("{reapply:?}")] {
            assert!(!printed.contains("hunter2"), "{printed}");
            assert!(printed.contains("[REDACTED]"), "{printed}");
        }
    }

    #[test]
    fn a_request_without_a_password_says_so() {
        let set = SetAppendOnlyRequest {
            enabled: false,
            password: None,
        };
        assert!(format!("{set:?}").contains("password: None"));
    }
}
