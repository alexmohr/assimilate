// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The options the Borg console passes on to borg, and which subcommand
//! accepts which.
//!
//! This is an allowlist: an option not named here is refused, whatever it
//! does. That keeps out every option that runs a command (`--rsh`,
//! `--remote-path`), reads or writes a local file (`--exclude-from`,
//! `--patterns-from`, `--debug-profile`, `--timestamp` with a reference
//! file), changes how borg talks to the repository (`--upload-ratelimit`,
//! `--upload-buffer`, `--bypass-lock`, `--umask`), or needs an interactive
//! confirmation the console cannot give (`check --repair`).

use shared::audit::BorgConsoleSubcommand;

/// An option the console accepts, parsed from its long name or a short alias.
/// It is passed to borg by its long name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::EnumString)]
pub(crate) enum ConsoleOption {
    #[strum(to_string = "--info", serialize = "-v", serialize = "--verbose")]
    Info,
    #[strum(to_string = "--warning")]
    Warning,
    #[strum(to_string = "--error")]
    Error,
    #[strum(to_string = "--critical")]
    Critical,
    #[strum(to_string = "--progress", serialize = "-p")]
    Progress,
    #[strum(to_string = "--log-json")]
    LogJson,
    #[strum(to_string = "--lock-wait")]
    LockWait,
    #[strum(to_string = "--show-rc")]
    ShowRc,
    #[strum(to_string = "--show-version")]
    ShowVersion,
    #[strum(to_string = "--iec")]
    Iec,
    #[strum(to_string = "--glob-archives", serialize = "-a")]
    GlobArchives,
    #[strum(to_string = "--first")]
    First,
    #[strum(to_string = "--last")]
    Last,
    #[strum(to_string = "--sort-by")]
    SortBy,
    #[strum(to_string = "--consider-checkpoints")]
    ConsiderCheckpoints,
    #[strum(to_string = "--json")]
    Json,
    #[strum(to_string = "--json-lines")]
    JsonLines,
    #[strum(to_string = "--short")]
    Short,
    #[strum(to_string = "--format")]
    Format,
    #[strum(to_string = "--exclude", serialize = "-e")]
    Exclude,
    #[strum(to_string = "--pattern")]
    Pattern,
    #[strum(to_string = "--repository-only")]
    RepositoryOnly,
    #[strum(to_string = "--archives-only")]
    ArchivesOnly,
    #[strum(to_string = "--verify-data")]
    VerifyData,
    #[strum(to_string = "--max-duration")]
    MaxDuration,
    #[strum(to_string = "--threshold")]
    Threshold,
    #[strum(to_string = "--dry-run", serialize = "-n")]
    DryRun,
    #[strum(to_string = "--stats", serialize = "-s")]
    Stats,
    #[strum(to_string = "--list")]
    List,
    #[strum(to_string = "--keep-within")]
    KeepWithin,
    #[strum(to_string = "--keep-last")]
    KeepLast,
    #[strum(to_string = "--keep-secondly")]
    KeepSecondly,
    #[strum(to_string = "--keep-minutely")]
    KeepMinutely,
    #[strum(to_string = "--keep-hourly", serialize = "-H")]
    KeepHourly,
    #[strum(to_string = "--keep-daily", serialize = "-d")]
    KeepDaily,
    #[strum(to_string = "--keep-weekly", serialize = "-w")]
    KeepWeekly,
    #[strum(to_string = "--keep-monthly", serialize = "-m")]
    KeepMonthly,
    #[strum(to_string = "--keep-yearly", serialize = "-y")]
    KeepYearly,
    #[strum(to_string = "--numeric-ids", serialize = "--numeric-owner")]
    NumericIds,
    #[strum(to_string = "--same-chunker-params")]
    SameChunkerParams,
    #[strum(to_string = "--sort")]
    Sort,
    #[strum(to_string = "--content-only")]
    ContentOnly,
    #[strum(to_string = "--filter")]
    Filter,
    #[strum(to_string = "--exclude-caches")]
    ExcludeCaches,
    #[strum(to_string = "--exclude-if-present")]
    ExcludeIfPresent,
    #[strum(to_string = "--keep-exclude-tags")]
    KeepExcludeTags,
    #[strum(to_string = "--target")]
    Target,
    #[strum(to_string = "--comment")]
    Comment,
    #[strum(to_string = "--compression", serialize = "-C")]
    Compression,
}

/// What an option's value must look like, for an option that takes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// A whole number, such as a count or a number of seconds.
    Number,
    /// A borg interval: a number and a unit, such as `7d`.
    Interval,
    /// Free text that borg only matches or formats with: a glob, a pattern,
    /// a format string, a sort key, a comment.
    Text,
    /// The name of an archive in this repository.
    ArchiveName,
}

impl ConsoleOption {
    /// What the option's value must look like, or `None` for a flag.
    pub(super) fn value_kind(self) -> Option<ValueKind> {
        match self {
            Self::LockWait
            | Self::First
            | Self::Last
            | Self::MaxDuration
            | Self::Threshold
            | Self::KeepLast
            | Self::KeepSecondly
            | Self::KeepMinutely
            | Self::KeepHourly
            | Self::KeepDaily
            | Self::KeepWeekly
            | Self::KeepMonthly
            | Self::KeepYearly => Some(ValueKind::Number),
            Self::KeepWithin => Some(ValueKind::Interval),
            Self::GlobArchives
            | Self::SortBy
            | Self::Format
            | Self::Exclude
            | Self::Pattern
            | Self::Filter
            | Self::ExcludeIfPresent
            | Self::Comment
            | Self::Compression => Some(ValueKind::Text),
            Self::Target => Some(ValueKind::ArchiveName),
            Self::Info
            | Self::Warning
            | Self::Error
            | Self::Critical
            | Self::Progress
            | Self::LogJson
            | Self::ShowRc
            | Self::ShowVersion
            | Self::Iec
            | Self::ConsiderCheckpoints
            | Self::Json
            | Self::JsonLines
            | Self::Short
            | Self::RepositoryOnly
            | Self::ArchivesOnly
            | Self::VerifyData
            | Self::DryRun
            | Self::Stats
            | Self::List
            | Self::NumericIds
            | Self::SameChunkerParams
            | Self::Sort
            | Self::ContentOnly
            | Self::ExcludeCaches
            | Self::KeepExcludeTags => None,
        }
    }

    /// Whether `borg <subcommand>` accepts this option from the console.
    pub(super) fn is_permitted_for(self, subcommand: BorgConsoleSubcommand) -> bool {
        COMMON.contains(&self)
            || groups_for(subcommand)
                .iter()
                .any(|group| group.contains(&self))
    }
}

/// Options every subcommand takes: logging and lock waiting.
const COMMON: &[ConsoleOption] = &[
    ConsoleOption::Info,
    ConsoleOption::Warning,
    ConsoleOption::Error,
    ConsoleOption::Critical,
    ConsoleOption::Progress,
    ConsoleOption::LogJson,
    ConsoleOption::LockWait,
    ConsoleOption::ShowRc,
    ConsoleOption::ShowVersion,
    ConsoleOption::Iec,
];

const ARCHIVE_FILTERS: &[ConsoleOption] = &[
    ConsoleOption::GlobArchives,
    ConsoleOption::First,
    ConsoleOption::Last,
    ConsoleOption::SortBy,
    ConsoleOption::ConsiderCheckpoints,
];

const PATH_MATCHING: &[ConsoleOption] = &[ConsoleOption::Exclude, ConsoleOption::Pattern];

fn groups_for(subcommand: BorgConsoleSubcommand) -> &'static [&'static [ConsoleOption]] {
    match subcommand {
        BorgConsoleSubcommand::Info => &[ARCHIVE_FILTERS, &[ConsoleOption::Json]],
        BorgConsoleSubcommand::List => &[
            ARCHIVE_FILTERS,
            PATH_MATCHING,
            &[
                ConsoleOption::Json,
                ConsoleOption::JsonLines,
                ConsoleOption::Short,
                ConsoleOption::Format,
            ],
        ],
        BorgConsoleSubcommand::Check => &[
            ARCHIVE_FILTERS,
            &[
                ConsoleOption::RepositoryOnly,
                ConsoleOption::ArchivesOnly,
                ConsoleOption::VerifyData,
                ConsoleOption::MaxDuration,
            ],
        ],
        BorgConsoleSubcommand::Compact => &[&[ConsoleOption::Threshold]],
        BorgConsoleSubcommand::Prune => &[&[
            ConsoleOption::GlobArchives,
            ConsoleOption::DryRun,
            ConsoleOption::Stats,
            ConsoleOption::List,
            ConsoleOption::KeepWithin,
            ConsoleOption::KeepLast,
            ConsoleOption::KeepSecondly,
            ConsoleOption::KeepMinutely,
            ConsoleOption::KeepHourly,
            ConsoleOption::KeepDaily,
            ConsoleOption::KeepWeekly,
            ConsoleOption::KeepMonthly,
            ConsoleOption::KeepYearly,
        ]],
        BorgConsoleSubcommand::Delete => &[
            ARCHIVE_FILTERS,
            &[
                ConsoleOption::DryRun,
                ConsoleOption::Stats,
                ConsoleOption::List,
            ],
        ],
        BorgConsoleSubcommand::Diff => &[
            PATH_MATCHING,
            &[
                ConsoleOption::NumericIds,
                ConsoleOption::SameChunkerParams,
                ConsoleOption::Sort,
                ConsoleOption::ContentOnly,
                ConsoleOption::JsonLines,
            ],
        ],
        BorgConsoleSubcommand::Rename => &[],
        BorgConsoleSubcommand::Recreate => &[
            ARCHIVE_FILTERS,
            PATH_MATCHING,
            &[
                ConsoleOption::List,
                ConsoleOption::Filter,
                ConsoleOption::DryRun,
                ConsoleOption::Stats,
                ConsoleOption::ExcludeCaches,
                ConsoleOption::ExcludeIfPresent,
                ConsoleOption::KeepExcludeTags,
                ConsoleOption::Target,
                ConsoleOption::Comment,
                ConsoleOption::Compression,
            ],
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_alias_parses_to_the_option_and_is_passed_on_by_its_long_name() {
        let option: ConsoleOption = "-d".parse().unwrap();
        assert_eq!(option, ConsoleOption::KeepDaily);
        assert_eq!(option.to_string(), "--keep-daily");
        assert_eq!("-v".parse::<ConsoleOption>().unwrap().to_string(), "--info");
    }

    #[test]
    fn options_are_matched_exactly_never_by_prefix_or_case() {
        for raw in ["--rs", "--remote", "--JSON", "--json=", "-sv", "--", "-"] {
            assert!(raw.parse::<ConsoleOption>().is_err(), "{raw}");
        }
    }

    #[test]
    fn logging_options_are_permitted_for_every_subcommand() {
        for sub in BorgConsoleSubcommand::ALL {
            assert!(ConsoleOption::Info.is_permitted_for(sub), "{sub}");
            assert!(ConsoleOption::LockWait.is_permitted_for(sub), "{sub}");
        }
    }

    #[test]
    fn an_option_is_permitted_only_for_the_subcommands_that_take_it() {
        assert!(ConsoleOption::KeepDaily.is_permitted_for(BorgConsoleSubcommand::Prune));
        assert!(!ConsoleOption::KeepDaily.is_permitted_for(BorgConsoleSubcommand::List));
        assert!(ConsoleOption::Short.is_permitted_for(BorgConsoleSubcommand::List));
        assert!(!ConsoleOption::Short.is_permitted_for(BorgConsoleSubcommand::Info));
        assert!(ConsoleOption::Target.is_permitted_for(BorgConsoleSubcommand::Recreate));
        assert!(!ConsoleOption::Target.is_permitted_for(BorgConsoleSubcommand::Rename));
    }

    #[test]
    fn value_kinds_match_what_borg_expects() {
        assert_eq!(ConsoleOption::Last.value_kind(), Some(ValueKind::Number));
        assert_eq!(
            ConsoleOption::KeepWithin.value_kind(),
            Some(ValueKind::Interval)
        );
        assert_eq!(
            ConsoleOption::GlobArchives.value_kind(),
            Some(ValueKind::Text)
        );
        assert_eq!(
            ConsoleOption::Target.value_kind(),
            Some(ValueKind::ArchiveName)
        );
        assert_eq!(ConsoleOption::Json.value_kind(), None);
    }
}
