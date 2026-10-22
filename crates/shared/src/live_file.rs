// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The largest file, in bytes, an agent reads for a comparison: the same
/// limit the server compares archived versions under.
pub const LIVE_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// A file's text as it is on the host right now. It may hold anything the
/// file holds, secrets included, so its debug output is redacted and it is
/// never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LiveText(pub String);

impl fmt::Debug for LiveText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LiveText([REDACTED], {} bytes)", self.0.len())
    }
}

/// What an agent found reading a file for the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LiveFile {
    /// The file is UTF-8 text no larger than [`LIVE_FILE_MAX_BYTES`].
    Text {
        /// The file's contents.
        text: LiveText,
        /// Its size in bytes.
        size: u64,
        /// When it was last modified, if the platform reports it.
        modified: Option<DateTime<Utc>>,
    },
    /// The file is not UTF-8 text.
    Binary,
    /// The file is larger than [`LIVE_FILE_MAX_BYTES`].
    TooLarge {
        /// Its size in bytes.
        size: u64,
    },
    /// The agent did not read the file.
    Refused {
        /// Why not.
        reason: LiveReadRefusal,
    },
}

/// Why a file could not be read live. Shared by the agent's answer and the
/// server's reply to the UI, which adds the cases only the server sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LiveReadRefusal {
    /// The path is outside every backup source configured for the host.
    NotInSources,
    /// Nothing exists at the path on the host now.
    NotFound,
    /// The path is a directory, a symbolic link or another special file.
    NotRegularFile,
    /// The file exists but the agent could not read it.
    Unreadable,
    /// The host's agent is not connected.
    AgentOffline,
    /// The agent did not answer in time.
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_text_is_redacted_in_debug_output() {
        let file = LiveFile::Text {
            text: LiveText("password=hunter2".to_owned()),
            size: 16,
            modified: None,
        };
        let debug = format!("{file:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn live_files_round_trip() {
        for file in [
            LiveFile::Text {
                text: LiveText("a\nb\n".to_owned()),
                size: 4,
                modified: Some(DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
            },
            LiveFile::Binary,
            LiveFile::TooLarge { size: 5_000_000 },
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources,
            },
        ] {
            let json = serde_json::to_string(&file).unwrap();
            assert_eq!(serde_json::from_str::<LiveFile>(&json).unwrap(), file);
        }
    }

    #[test]
    fn the_text_travels_as_a_plain_string() {
        let json = serde_json::to_value(LiveFile::Text {
            text: LiveText("hello".to_owned()),
            size: 5,
            modified: None,
        })
        .unwrap();
        assert_eq!(json.get("text"), Some(&serde_json::json!("hello")));
        assert_eq!(json.get("status"), Some(&serde_json::json!("text")));
    }
}
