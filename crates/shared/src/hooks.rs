// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Pre- and post-backup hook commands.
//!
//! A hook is a shell script the agent runs via `sh -c` around a backup. Each
//! one carries an optional timeout of its own, so a slow hook (a hypervisor
//! dump, a database export) does not force every other hook on the same
//! schedule to share its generous budget.

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Upper bound for a hook command's own `timeout_seconds`, in seconds (24h).
///
/// Deliberately more generous than the schedule-level default bound: that
/// value applies blanket to every hook, whereas this one is a per-script
/// statement by the operator that *this* command legitimately runs long.
pub const MAX_HOOK_COMMAND_TIMEOUT_SECONDS: u32 = 86_400;

/// A single pre- or post-backup hook command.
///
/// Serialises as an object; deserialises from either an object or a bare
/// string, so configurations written before per-command timeouts existed keep
/// loading unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct HookCommand {
    /// The shell script, executed as a single `sh -c` invocation. May span
    /// multiple lines.
    pub command: String,
    /// Timeout for this command alone, in seconds. Falls back to the
    /// schedule's `hook_timeout_seconds` when unset.
    #[ts(type = "number | null")]
    pub timeout_seconds: Option<u32>,
}

impl HookCommand {
    /// Creates a hook command that inherits the schedule's hook timeout.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            timeout_seconds: None,
        }
    }

    /// Returns this command's own timeout, falling back to `default_seconds`.
    #[must_use]
    pub fn timeout_or(&self, default_seconds: u32) -> u32 {
        self.timeout_seconds.unwrap_or(default_seconds)
    }
}

impl From<String> for HookCommand {
    fn from(command: String) -> Self {
        Self::new(command)
    }
}

impl From<&str> for HookCommand {
    fn from(command: &str) -> Self {
        Self::new(command)
    }
}

/// Accepts both the historic bare-string form and the object form carrying a
/// per-command timeout.
impl<'de> Deserialize<'de> for HookCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Bare(String),
            Structured {
                command: String,
                #[serde(default)]
                timeout_seconds: Option<u32>,
            },
        }

        Ok(match Repr::deserialize(deserializer)? {
            Repr::Bare(command) => Self::new(command),
            Repr::Structured {
                command,
                timeout_seconds,
            } => Self {
                command,
                timeout_seconds,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_bare_string_without_timeout() {
        let parsed: Vec<HookCommand> = serde_json::from_str(r#"["echo hi"]"#).unwrap();
        assert_eq!(parsed, vec![HookCommand::new("echo hi")]);
    }

    #[test]
    fn deserializes_object_with_timeout() {
        let parsed: HookCommand =
            serde_json::from_str(r#"{"command":"vzdump --all 1","timeout_seconds":7200}"#).unwrap();
        assert_eq!(
            parsed,
            HookCommand {
                command: "vzdump --all 1".to_owned(),
                timeout_seconds: Some(7200),
            }
        );
    }

    #[test]
    fn deserializes_object_without_timeout() {
        let parsed: HookCommand = serde_json::from_str(r#"{"command":"echo hi"}"#).unwrap();
        assert_eq!(parsed.timeout_seconds, None);
    }

    #[test]
    fn deserializes_mixed_array() {
        let parsed: Vec<HookCommand> =
            serde_json::from_str(r#"["a",{"command":"b","timeout_seconds":5}]"#).unwrap();
        assert_eq!(
            parsed,
            vec![
                HookCommand::new("a"),
                HookCommand {
                    command: "b".to_owned(),
                    timeout_seconds: Some(5),
                },
            ]
        );
    }

    #[test]
    fn serializes_as_object() {
        let json = serde_json::to_string(&HookCommand::new("echo hi")).unwrap();
        assert_eq!(json, r#"{"command":"echo hi","timeout_seconds":null}"#);
    }

    #[test]
    fn round_trips_through_json() {
        let original = HookCommand {
            command: "line one\nline two".to_owned(),
            timeout_seconds: Some(120),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(
            serde_json::from_str::<HookCommand>(&json).unwrap(),
            original
        );
    }

    #[test]
    fn timeout_or_prefers_own_value() {
        assert_eq!(HookCommand::new("x").timeout_or(60), 60);
        assert_eq!(
            HookCommand {
                command: "x".to_owned(),
                timeout_seconds: Some(900),
            }
            .timeout_or(60),
            900
        );
    }
}
