// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What an audit log entry records: the action taken and that action's own
//! details, as one type rather than a free-form action string beside an
//! arbitrary JSON blob.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

mod grants;

pub use grants::{RepoPermission, RolePermission};

use crate::{
    notifications::{ChannelType, EventType},
    types::BorgEncryption,
};

/// How a user proved who they are when logging in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    /// A password alone, for an account without two-factor authentication.
    Password,
    /// A password followed by a TOTP code.
    Totp,
    /// A password followed by a single-use recovery code.
    RecoveryCode,
}

/// An audited action and its details, stored as the adjacent `action` and
/// `details` columns of the `audit_log` table and sent the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(tag = "action", content = "details", rename_all = "snake_case")]
pub enum AuditEvent {
    /// An archive was deleted from a repository.
    DeleteArchive {
        /// The deleted archive.
        archive: String,
    },
    /// Files from an archive were downloaded.
    DownloadFiles {
        /// The archive the files came from.
        archive: String,
        /// The downloaded paths inside the archive.
        paths: Vec<String>,
    },
    /// Files from an archive were restored onto an agent.
    RestoreFiles {
        /// The archive the files came from.
        archive: String,
        /// The restored paths inside the archive.
        paths: Vec<String>,
        /// Where on the agent they were restored to.
        target_path: String,
        /// The agent they were restored onto.
        hostname: String,
    },
    /// A repository's key was exported.
    KeyExport {},
    /// A repository's key was imported.
    KeyImport {},
    /// A repository's key passphrase was changed.
    KeyChangePassphrase {},
    /// A repository was re-created with a different encryption mode.
    MigrateEncryption {
        /// The encryption mode it had.
        from: BorgEncryption,
        /// The encryption mode it has now.
        to: BorgEncryption,
        /// Where the original repository was preserved.
        migrated_path: String,
    },
    /// A user logged in and was given a session.
    Login {
        /// How they proved who they are.
        method: LoginMethod,
    },
    /// A user logged out, ending their session.
    Logout {},
    /// A user account was created.
    CreateUser {
        /// The new account's username.
        username: String,
    },
    /// A user account was deleted.
    DeleteUser {
        /// The deleted account's username.
        username: String,
    },
    /// An admin set another user's password. The password is never recorded.
    ResetPassword {
        /// The account whose password was set.
        username: String,
    },
    /// The roles a user holds were replaced.
    SetUserRoles {
        /// The user whose roles changed.
        username: String,
        /// The roles they held.
        before: Vec<String>,
        /// The roles they hold now.
        after: Vec<String>,
    },
    /// A group was created.
    CreateGroup {
        /// The new group's name.
        name: String,
    },
    /// A group was renamed or its description changed.
    UpdateGroup {
        /// The group's name now.
        name: String,
        /// The name it had.
        previous_name: String,
    },
    /// A group was deleted.
    DeleteGroup {
        /// The deleted group's name.
        name: String,
    },
    /// The members of a group were replaced.
    SetGroupMembers {
        /// The group whose members changed.
        group: String,
        /// The usernames of its members before.
        before: Vec<String>,
        /// The usernames of its members now.
        after: Vec<String>,
    },
    /// A role was created.
    CreateRole {
        /// The new role's name.
        name: String,
        /// What it grants.
        permissions: Vec<RolePermission>,
    },
    /// A role was renamed or what it grants changed.
    UpdateRole {
        /// The role's name now.
        name: String,
        /// The name it had.
        previous_name: String,
        /// What it granted.
        before: Vec<RolePermission>,
        /// What it grants now.
        after: Vec<RolePermission>,
    },
    /// A role was deleted.
    DeleteRole {
        /// The deleted role's name.
        name: String,
    },
    /// A user's permissions on one repository were granted or changed.
    SetRepoPermission {
        /// The user whose permissions changed.
        username: String,
        /// What they held on the repository; empty if they held nothing.
        before: Vec<RepoPermission>,
        /// What they hold on it now; empty if everything was revoked.
        after: Vec<RepoPermission>,
    },
    /// A user created an API token for themselves. The token is never recorded.
    CreateApiToken {
        /// The token's name.
        name: String,
    },
    /// An API token was revoked.
    DeleteApiToken {
        /// The token's name.
        name: String,
        /// The user the token belonged to.
        owner: String,
    },
    /// An agent's token was replaced with a new one. Neither token is recorded.
    RegenerateAgentToken {
        /// The agent's hostname.
        hostname: String,
        /// The agent's domain, if it has one.
        domain: Option<String>,
    },
    /// A notification channel was created.
    CreateNotificationChannel {
        /// The channel's name.
        name: String,
        /// How it delivers.
        channel_type: ChannelType,
    },
    /// A notification channel's name, configuration, scope or state was changed.
    /// Its configuration is never recorded, since it can hold credentials.
    UpdateNotificationChannel {
        /// The channel's name now.
        name: String,
        /// How it delivers.
        channel_type: ChannelType,
    },
    /// A notification channel was deleted, along with its rules.
    DeleteNotificationChannel {
        /// The deleted channel's name.
        name: String,
        /// How it delivered.
        channel_type: ChannelType,
    },
    /// A notification rule was created, routing an event to a channel.
    CreateNotificationRule {
        /// The channel the event is routed to.
        #[ts(type = "number")]
        channel_id: i64,
        /// The event routed.
        event_type: EventType,
        /// The repository the rule is limited to, if any.
        #[ts(type = "number | null")]
        repo_id: Option<i64>,
        /// The agent the rule is limited to, if any.
        #[ts(type = "number | null")]
        agent_id: Option<i64>,
    },
    /// A notification rule was deleted.
    DeleteNotificationRule {
        /// The channel the event was routed to.
        #[ts(type = "number")]
        channel_id: i64,
        /// The event it routed.
        event_type: EventType,
        /// The repository the rule was limited to, if any.
        #[ts(type = "number | null")]
        repo_id: Option<i64>,
        /// The agent the rule was limited to, if any.
        #[ts(type = "number | null")]
        agent_id: Option<i64>,
    },
    /// The server's web push (VAPID) key pair was replaced. Neither key is recorded.
    SetVapidKeys {},
}

impl AuditEvent {
    /// The `action` and `details` column values to store for this event.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be serialized.
    pub fn to_stored(&self) -> Result<(String, serde_json::Value), serde_json::Error> {
        let mut stored = serde_json::to_value(self)?;
        let details = stored
            .get_mut("details")
            .map_or(serde_json::Value::Null, serde_json::Value::take);
        let action = stored
            .get("action")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                <serde_json::Error as serde::ser::Error>::custom("audit event has no action tag")
            })?;
        Ok((action, details))
    }

    /// Parses a stored entry's `action` and `details` columns: the
    /// counterpart of [`Self::to_stored`].
    ///
    /// # Errors
    ///
    /// Returns an error if `action` is unknown or `details` do not fit it.
    pub fn from_stored(
        action: &str,
        details: Option<serde_json::Value>,
    ) -> Result<Self, serde_json::Error> {
        let details = details
            .filter(|details| !details.is_null())
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        serde_json::from_value(serde_json::json!({ "action": action, "details": details }))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stored_columns_round_trip_for_an_event_with_details() {
        let event = AuditEvent::MigrateEncryption {
            from: BorgEncryption::Repokey,
            to: BorgEncryption::RepokeyBlake2,
            migrated_path: "/backups/repo.migrated-2026-01-01".to_owned(),
        };
        let (action, details) = event.to_stored().unwrap();
        assert_eq!(action, "migrate_encryption");
        assert_eq!(details.get("to"), Some(&json!("repokey-blake2")));
        assert_eq!(
            AuditEvent::from_stored(&action, Some(details)).unwrap(),
            event
        );
    }

    #[test]
    fn stored_columns_round_trip_for_an_event_without_details() {
        let (action, details) = AuditEvent::KeyExport {}.to_stored().unwrap();
        assert_eq!(action, "key_export");
        assert_eq!(
            AuditEvent::from_stored(&action, Some(details)).unwrap(),
            AuditEvent::KeyExport {}
        );
        assert_eq!(
            AuditEvent::from_stored(&action, None).unwrap(),
            AuditEvent::KeyExport {}
        );
    }

    #[test]
    fn a_key_entry_recorded_with_the_old_redundant_details_still_reads() {
        let event = AuditEvent::from_stored(
            "key_import",
            Some(json!({ "action": "key_import", "repo_id": 3 })),
        )
        .unwrap();
        assert_eq!(event, AuditEvent::KeyImport {});
    }

    #[test]
    fn a_login_records_how_the_user_proved_who_they_are() {
        let (action, details) = AuditEvent::Login {
            method: LoginMethod::RecoveryCode,
        }
        .to_stored()
        .unwrap();
        assert_eq!(action, "login");
        assert_eq!(details, json!({ "method": "recovery_code" }));
    }

    #[test]
    fn a_role_change_round_trips_its_before_and_after() {
        let event = AuditEvent::UpdateRole {
            name: "ops".to_owned(),
            previous_name: "operators".to_owned(),
            before: vec![RolePermission::CreateRepo],
            after: vec![RolePermission::CreateRepo, RolePermission::DeleteRepo],
        };
        let (action, details) = event.to_stored().unwrap();
        assert_eq!(action, "update_role");
        assert_eq!(
            details.get("after"),
            Some(&json!(["create_repo", "delete_repo"]))
        );
        assert_eq!(
            AuditEvent::from_stored(&action, Some(details)).unwrap(),
            event
        );
    }

    #[test]
    fn a_notification_rule_round_trips_with_its_optional_scope() {
        let event = AuditEvent::CreateNotificationRule {
            channel_id: 4,
            event_type: EventType::BackupFailed,
            repo_id: None,
            agent_id: Some(9),
        };
        let (action, details) = event.to_stored().unwrap();
        assert_eq!(action, "create_notification_rule");
        assert_eq!(details.get("event_type"), Some(&json!("backup_failed")));
        assert_eq!(
            AuditEvent::from_stored(&action, Some(details)).unwrap(),
            event
        );
    }

    #[test]
    fn an_unknown_action_is_rejected() {
        assert!(AuditEvent::from_stored("repo.create", Some(json!({}))).is_err());
    }
}
