// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The borg subcommands a repository's Borg console may run, shared by the
//! server that validates a console command and the audit entry it records.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// A borg subcommand the Borg console may run.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BorgConsoleSubcommand {
    /// `borg info`.
    Info,
    /// `borg list`.
    List,
    /// `borg check`.
    Check,
    /// `borg compact`.
    Compact,
    /// `borg prune`.
    Prune,
    /// `borg delete`.
    Delete,
    /// `borg diff`.
    Diff,
    /// `borg rename`.
    Rename,
    /// `borg recreate`.
    Recreate,
}

impl BorgConsoleSubcommand {
    /// Every subcommand, in the order the console lists them.
    pub const ALL: [Self; 9] = [
        Self::Info,
        Self::List,
        Self::Check,
        Self::Compact,
        Self::Prune,
        Self::Delete,
        Self::Diff,
        Self::Rename,
        Self::Recreate,
    ];

    /// The subcommand as borg spells it on the command line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subcommand_round_trips_through_its_command_line_name() {
        for sub in BorgConsoleSubcommand::ALL {
            assert_eq!(sub.as_str().parse::<BorgConsoleSubcommand>().unwrap(), sub);
            assert_eq!(sub.to_string(), sub.as_str());
            assert_eq!(
                serde_json::to_value(sub).unwrap(),
                serde_json::json!(sub.as_str())
            );
        }
    }

    #[test]
    fn unknown_or_differently_cased_subcommands_are_rejected() {
        for raw in ["rm", "", "INFO", "key", "serve", "with-lock"] {
            assert!(raw.parse::<BorgConsoleSubcommand>().is_err(), "{raw}");
        }
    }
}
