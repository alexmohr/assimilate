// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What an audit log entry records: the action taken and that action's own
//! details, as one type rather than a free-form action string beside an
//! arbitrary JSON blob.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::types::BorgEncryption;

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
    fn an_unknown_action_is_rejected() {
        assert!(AuditEvent::from_stored("repo.create", Some(json!({}))).is_err());
    }
}
