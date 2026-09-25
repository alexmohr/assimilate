// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use serde::{Deserialize, Serialize};

/// Notification event categories that can trigger delivery rules.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EventType {
    /// Backup completed successfully.
    BackupSuccess,
    /// Backup completed with warnings.
    BackupWarning,
    /// Backup failed.
    BackupFailed,
    /// Repository integrity check succeeded.
    CheckSuccess,
    /// Repository integrity check failed.
    CheckFailed,
    /// Agent connected to the server.
    AgentConnected,
    /// Agent disconnected from the server.
    AgentDisconnected,
    /// The scheduler auto-disabled a schedule after it reached its
    /// `missed_backup_threshold` of consecutive missed backups.
    ScheduleAutoDisabled,
    /// A scheduled backup could not be started because its target agent was
    /// offline (not connected to the server) when the run came due.
    BackupSkippedAgentOffline,
    /// A scheduled backup could not be started because the host holding its
    /// target repository did not answer SSH when the run came due.
    BackupSkippedRepoOffline,
}

impl EventType {
    /// All event type names as static string slices for DB queries.
    pub const ALL_DB_STRS: &[&'static str] = &[
        "backup_success",
        "backup_warning",
        "backup_failed",
        "check_success",
        "check_failed",
        "agent_connected",
        "agent_disconnected",
        "schedule_auto_disabled",
        "backup_skipped_agent_offline",
        "backup_skipped_repo_offline",
    ];
}

/// Human-readable label for an event type string (e.g. `"backup_failed"` -> `"Backup
/// failed"`), shared by the email subject and web push title builders. Falls back to
/// `"Notification"` for an empty or unrecognized event type.
#[must_use]
pub fn event_label(event_type_str: &str) -> &'static str {
    let Ok(event_type) = event_type_str.parse::<EventType>() else {
        return "Notification";
    };
    match event_type {
        EventType::BackupSuccess => "Backup succeeded",
        EventType::BackupWarning => "Backup warning",
        EventType::BackupFailed => "Backup failed",
        EventType::CheckSuccess => "Check succeeded",
        EventType::CheckFailed => "Check failed",
        EventType::AgentConnected => "Agent connected",
        EventType::AgentDisconnected => "Agent disconnected",
        EventType::ScheduleAutoDisabled => "Schedule auto-disabled",
        EventType::BackupSkippedAgentOffline | EventType::BackupSkippedRepoOffline => {
            "Backup skipped"
        }
    }
}
