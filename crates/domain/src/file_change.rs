// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use serde::{Deserialize, Serialize};

/// What to do when a backup source's set of file changes looks unusually
/// large or small compared to prior runs (a possible ransomware/corruption signal).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[cfg_attr(feature = "schema", derive(ts_rs::TS, utoipa::ToSchema))]
#[strum(serialize_all = "lowercase")]
pub enum FileChangeAction {
    /// Take no action; let the backup proceed regardless of the change volume.
    Ignore,
    /// Log a warning but let the backup proceed.
    #[default]
    Warn,
    /// Abort the backup rather than let it complete.
    Fatal,
}

/// A glob pattern paired with the action to take when a changed file matches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(ts_rs::TS, utoipa::ToSchema))]
pub struct FileChangePattern {
    /// Glob pattern matched against changed file paths within the backup source.
    pub path: String,
    /// What to do when a change matching this pattern is detected.
    pub action: FileChangeAction,
}

impl From<&str> for FileChangePattern {
    /// Parses one trimmed, non-comment line. A trailing word that is not an
    /// action keyword is part of the glob, and the action defaults to `warn`.
    fn from(line: &str) -> Self {
        line.rsplit_once(' ')
            .and_then(|(path, action)| {
                action.parse().ok().map(|action| Self {
                    path: path.trim().to_owned(),
                    action,
                })
            })
            .unwrap_or_else(|| Self {
                path: line.to_owned(),
                action: FileChangeAction::default(),
            })
    }
}

impl std::fmt::Display for FileChangePattern {
    /// Writes the line form, omitting the default `warn` action.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.action {
            FileChangeAction::Warn => write!(f, "{}", self.path),
            FileChangeAction::Ignore | FileChangeAction::Fatal => {
                write!(f, "{} {}", self.path, self.action)
            }
        }
    }
}

/// Parses the raw textarea form: blank lines and `#`-prefixed comments are dropped.
#[must_use]
pub fn parse_file_change_patterns(raw: &str) -> Vec<FileChangePattern> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(FileChangePattern::from)
        .collect()
}

/// Serializes patterns back into the raw textarea form, one per line.
#[must_use]
pub fn serialize_file_change_patterns(patterns: &[FileChangePattern]) -> String {
    patterns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        FileChangeAction, FileChangePattern, parse_file_change_patterns,
        serialize_file_change_patterns,
    };

    fn pattern(path: &str, action: FileChangeAction) -> FileChangePattern {
        FileChangePattern {
            path: path.to_owned(),
            action,
        }
    }

    #[test]
    fn action_display_roundtrip() {
        let variants = [
            (FileChangeAction::Ignore, "ignore"),
            (FileChangeAction::Warn, "warn"),
            (FileChangeAction::Fatal, "fatal"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            assert_eq!(expected.parse::<FileChangeAction>().unwrap(), variant);
        }
        assert!("invalid".parse::<FileChangeAction>().is_err());
    }

    #[test]
    fn defaults_to_warn() {
        assert_eq!(
            parse_file_change_patterns("/etc/passwd\n/var/log"),
            vec![
                pattern("/etc/passwd", FileChangeAction::Warn),
                pattern("/var/log", FileChangeAction::Warn),
            ]
        );
    }

    #[test]
    fn parses_trailing_actions() {
        assert_eq!(
            parse_file_change_patterns("/tmp ignore\n/etc warn\n/var/log fatal"),
            vec![
                pattern("/tmp", FileChangeAction::Ignore),
                pattern("/etc", FileChangeAction::Warn),
                pattern("/var/log", FileChangeAction::Fatal),
            ]
        );
    }

    #[test]
    fn blank_and_comment_lines_stripped() {
        let input = "# comment\n/tmp ignore\n\n   \n# another\n/var/log fatal";
        assert_eq!(parse_file_change_patterns(input).len(), 2);
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_file_change_patterns(""), Vec::new());
    }

    #[test]
    fn unknown_trailing_word_is_part_of_the_glob() {
        assert_eq!(
            parse_file_change_patterns("/my docs/report final"),
            vec![pattern("/my docs/report final", FileChangeAction::Warn)]
        );
    }

    #[test]
    fn action_keyword_is_case_sensitive() {
        assert_eq!(
            parse_file_change_patterns("/tmp IGNORE"),
            vec![pattern("/tmp IGNORE", FileChangeAction::Warn)]
        );
    }

    #[test]
    fn surrounding_and_inner_whitespace_trimmed() {
        assert_eq!(
            parse_file_change_patterns("  /tmp   ignore  \r\n\t/var\t"),
            vec![
                pattern("/tmp", FileChangeAction::Ignore),
                pattern("/var", FileChangeAction::Warn),
            ]
        );
    }

    #[test]
    fn bare_keyword_is_a_glob() {
        assert_eq!(
            parse_file_change_patterns("fatal"),
            vec![pattern("fatal", FileChangeAction::Warn)]
        );
    }

    #[test]
    fn serialize_omits_default_action() {
        let patterns = [
            pattern("/tmp", FileChangeAction::Ignore),
            pattern("/etc", FileChangeAction::Warn),
            pattern("/var/log", FileChangeAction::Fatal),
        ];
        assert_eq!(
            serialize_file_change_patterns(&patterns),
            "/tmp ignore\n/etc\n/var/log fatal"
        );
    }

    #[test]
    fn serialize_empty() {
        assert_eq!(serialize_file_change_patterns(&[]), "");
    }

    #[test]
    fn serialize_then_parse_roundtrips() {
        let patterns = vec![
            pattern("*/tmp*", FileChangeAction::Ignore),
            pattern("*/home/*", FileChangeAction::Warn),
            pattern("*/etc/shadow", FileChangeAction::Fatal),
        ];
        assert_eq!(
            parse_file_change_patterns(&serialize_file_change_patterns(&patterns)),
            patterns
        );
    }
}
