// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Whether an agent's host can run borg, as the agent found out by asking the
/// executable it would use for its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum BorgStatus {
    /// Borg ran and reported its version.
    Available {
        /// Borg's version, such as `1.4.1`.
        version: String,
    },
    /// There is no executable at the path the agent looked at.
    Missing {
        /// The executable the agent tried to run: `borg` when it relies on
        /// `PATH`.
        binary: String,
    },
    /// The executable exists but did not report a version.
    Broken {
        /// The executable the agent tried to run.
        binary: String,
        /// Why asking it for its version failed.
        reason: String,
    },
}

impl BorgStatus {
    /// The version out of `borg --version` output such as `borg 1.4.1`.
    #[must_use]
    pub fn version_from_output(stdout: &str) -> Option<String> {
        stdout
            .split_whitespace()
            .nth(1)
            .map(ToOwned::to_owned)
            .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_the_second_word_of_the_output() {
        assert_eq!(
            BorgStatus::version_from_output("borg 1.4.1\n").as_deref(),
            Some("1.4.1")
        );
    }

    #[test]
    fn output_that_is_not_a_version_gives_none() {
        for output in ["", "borg", "borg unknown", "command not found: borg"] {
            assert_eq!(BorgStatus::version_from_output(output), None, "{output}");
        }
    }

    #[test]
    fn the_status_is_tagged_by_state_on_the_wire() {
        let json = serde_json::to_string(&BorgStatus::Missing {
            binary: "borg".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"state":"missing","binary":"borg"}"#);
        let back: BorgStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back,
            BorgStatus::Missing {
                binary: "borg".into()
            }
        );
    }
}
