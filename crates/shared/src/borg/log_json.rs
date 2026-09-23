// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Typed parsing for borg's `--log-json` stderr format: one JSON object per
//! line, either a `log_message` diagnostic record or another progress/status
//! record type this module leaves alone. Used to turn a run's raw stderr
//! into the warnings and context a caller reports.

use serde::Deserialize;

/// The `type` field of a borg `--log-json` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorgLogRecordType {
    /// A diagnostic log record - the only record type this module
    /// interprets further.
    LogMessage,
    /// Any other record type (e.g. `archive_progress`, `file_status`),
    /// which carries no diagnostic to extract.
    #[serde(other)]
    Other,
}

/// The `levelname` field of a borg `--log-json` log message line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum BorgLogLevel {
    /// Verbose diagnostic detail, not surfaced as a warning or context.
    #[serde(rename = "DEBUG")]
    Debug,
    /// Informational record, kept as context when nothing more severe
    /// explains a run.
    #[serde(rename = "INFO")]
    Info,
    /// A warning-level diagnostic.
    #[serde(rename = "WARNING")]
    Warning,
    /// An error-level diagnostic.
    #[serde(rename = "ERROR")]
    Error,
    /// A critical-level diagnostic - borg's most severe log level.
    #[serde(rename = "CRITICAL")]
    Critical,
    /// Any level this module doesn't otherwise recognize.
    #[serde(other)]
    Other,
}

impl BorgLogLevel {
    /// Whether a record at this level says something went wrong. `CRITICAL`
    /// counts: borg logs hard failures at that level, and dropping them left
    /// a failed run with nothing to show.
    #[must_use]
    pub fn is_diagnostic(self) -> bool {
        matches!(self, Self::Warning | Self::Error | Self::Critical)
    }
}

/// The `msgid` field of a borg `--log-json` log message line. Borg emits many
/// message ids; a caller matches the ones it cares about (e.g.
/// `BackupFileNotFoundError`) and treats the rest as opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum BorgMsgId {
    /// A configured backup source path did not exist at backup time.
    BackupFileNotFoundError,
    /// Any other message id.
    #[serde(other)]
    Other,
}

/// A single parsed line of borg `--log-json` output.
#[derive(Debug, Deserialize)]
pub struct BorgLogLine {
    /// The record's kind (see [`BorgLogRecordType`]).
    #[serde(rename = "type")]
    pub record_type: BorgLogRecordType,
    /// The record's severity, present on `log_message` records.
    #[serde(default)]
    pub levelname: Option<BorgLogLevel>,
    /// The record's message id, present on some `log_message` records.
    #[serde(default)]
    pub msgid: Option<BorgMsgId>,
    /// The record's human-readable message, present on `log_message`
    /// records.
    #[serde(default)]
    pub message: Option<String>,
}

/// borg's `--show-rc` footer, e.g. `terminating with warning status, rc 1`.
/// It restates the exit code the caller already has and never says what
/// caused it, so it is kept out of the diagnostics a report shows.
#[must_use]
pub fn is_exit_status_footer(message: &str) -> bool {
    message.starts_with("terminating with ") && message.contains(" status, rc ")
}

/// Truncates `line` to at most `max_chars` characters, appending `...` when
/// it was cut short.
#[must_use]
pub fn truncate_chars(mut line: String, max_chars: usize) -> String {
    if let Some((idx, _)) = line.char_indices().nth(max_chars) {
        line.truncate(idx);
        line.push_str("...");
    }
    line
}

/// How many of borg's non-diagnostic stderr lines are kept as context for a
/// run that ends non-zero without saying why.
const MAX_CONTEXT_LINES: usize = 10;

/// How much of a single context line is kept - borg prints very long paths.
const MAX_CONTEXT_LINE_CHARS: usize = 300;

/// What borg's stderr said about a run.
#[derive(Debug, Default)]
pub struct BorgDiagnostics {
    /// The `WARNING`, `ERROR` and `CRITICAL` log records, minus the
    /// `--show-rc` footer: the messages that say what actually happened.
    pub warnings: Vec<String>,
    /// The tail of everything else borg printed - its `INFO` records and any
    /// line it did not emit as JSON (ssh notices, tracebacks). Noise while
    /// there are real diagnostics, and the only clue when there are none.
    pub context: Vec<String>,
    /// The subset of those records that were `ERROR` or worse. A caller
    /// matching warning text against patterns can match one of these just as
    /// easily as a genuine warning; keeping them apart lets it say so instead
    /// of losing the diagnostic.
    pub error_level: Vec<String>,
}

impl BorgDiagnostics {
    fn push_line(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<BorgLogLine>(line) else {
            self.push_context(line.to_owned());
            return;
        };
        if record.record_type != BorgLogRecordType::LogMessage {
            return;
        }
        let Some(message) = record.message else {
            return;
        };
        match record.levelname {
            Some(BorgLogLevel::Warning | BorgLogLevel::Error | BorgLogLevel::Critical) => {
                if !is_exit_status_footer(&message) {
                    if matches!(
                        record.levelname,
                        Some(BorgLogLevel::Error | BorgLogLevel::Critical)
                    ) {
                        self.error_level.push(message.clone());
                    }
                    self.warnings.push(message);
                }
            }
            Some(BorgLogLevel::Info) => self.push_context(message),
            Some(BorgLogLevel::Debug | BorgLogLevel::Other) | None => {}
        }
    }

    fn push_context(&mut self, line: String) {
        if self.context.len() >= MAX_CONTEXT_LINES {
            self.context.remove(0);
        }
        self.context
            .push(truncate_chars(line, MAX_CONTEXT_LINE_CHARS));
    }
}

/// Borg's stderr, one record per line. `\r` separates lines as well as `\n`:
/// borg's progress output ends its updates with a carriage return, which
/// would otherwise glue a whole run's progress and the log records printed
/// between them into a single unparsable line.
pub fn stderr_lines(stderr: &str) -> impl Iterator<Item = &str> {
    stderr
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Split borg's stderr into the diagnostics that explain a run and the
/// context that is left when it explains nothing.
#[must_use]
pub fn parse_diagnostics(stderr: &str) -> BorgDiagnostics {
    stderr_lines(stderr).fold(BorgDiagnostics::default(), |mut diagnostics, line| {
        diagnostics.push_line(line);
        diagnostics
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diagnostics_drops_the_show_rc_footer() {
        let stderr = [
            concat!(
                r#"{"type": "log_message", "levelname": "WARNING", "#,
                r#""message": "/tmp/test.log: file changed"}"#,
            ),
            concat!(
                r#"{"type": "log_message", "levelname": "WARNING", "#,
                r#""name": "borg.archiver", "#,
                r#""message": "terminating with warning status, rc 1"}"#,
            ),
        ]
        .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.warnings, vec!["/tmp/test.log: file changed"]);
    }

    #[test]
    fn parse_diagnostics_keeps_critical_records() {
        let stderr = concat!(
            r#"{"type": "log_message", "levelname": "CRITICAL", "#,
            r#""message": "Repository /repo does not exist."}"#,
        );

        let diagnostics = parse_diagnostics(stderr);

        assert_eq!(
            diagnostics.warnings,
            vec!["Repository /repo does not exist."]
        );
    }

    #[test]
    fn parse_diagnostics_collects_context_from_non_json_and_info_lines() {
        let stderr = [
            r#"{"type": "archive_progress", "original_size": 100}"#,
            r#"{"type": "log_message", "levelname": "INFO", "message": "Creating archive"}"#,
            "Warning: Permanently added 'storage' to the list of known hosts.",
        ]
        .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.warnings, [] as [String; 0]);
        assert_eq!(
            diagnostics.context,
            vec![
                "Creating archive",
                "Warning: Permanently added 'storage' to the list of known hosts."
            ]
        );
    }

    #[test]
    fn parse_diagnostics_keeps_only_the_tail_of_the_context() {
        let stderr = (0..MAX_CONTEXT_LINES + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let diagnostics = parse_diagnostics(&stderr);

        assert_eq!(diagnostics.context.len(), MAX_CONTEXT_LINES);
        assert_eq!(diagnostics.context.first().unwrap(), "line 5");
        assert_eq!(
            diagnostics.context.last().unwrap(),
            &format!("line {}", MAX_CONTEXT_LINES + 4)
        );
    }

    #[test]
    fn parse_diagnostics_truncates_an_overlong_context_line() {
        let stderr = "x".repeat(MAX_CONTEXT_LINE_CHARS + 50);

        let diagnostics = parse_diagnostics(&stderr);

        let line = diagnostics.context.first().unwrap();
        assert_eq!(line.chars().count(), MAX_CONTEXT_LINE_CHARS + 3);
        assert!(line.ends_with("..."));
    }

    #[test]
    fn parse_diagnostics_flags_error_level_records() {
        let warning_only = r#"{"type": "log_message", "levelname": "WARNING", "message": "oops"}"#;
        assert_eq!(
            parse_diagnostics(warning_only).error_level,
            [] as [String; 0]
        );

        let with_error = r#"{"type": "log_message", "levelname": "ERROR", "message": "boom"}"#;
        assert_eq!(parse_diagnostics(with_error).error_level, vec!["boom"]);

        let with_critical = r#"{"type": "log_message", "levelname": "CRITICAL", "message": "b"}"#;
        assert_eq!(parse_diagnostics(with_critical).error_level, vec!["b"]);
    }

    #[test]
    fn stderr_lines_splits_on_carriage_return_as_well_as_newline() {
        let stderr = "a\rb\nc\r\n\rd";
        let lines: Vec<_> = stderr_lines(stderr).collect();
        assert_eq!(lines, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn is_exit_status_footer_matches_borgs_show_rc_line() {
        assert!(is_exit_status_footer(
            "terminating with warning status, rc 1"
        ));
        assert!(is_exit_status_footer("terminating with error status, rc 2"));
        assert!(!is_exit_status_footer("some other message"));
    }

    #[test]
    fn truncate_chars_appends_ellipsis_only_when_cut_short() {
        assert_eq!(truncate_chars("short".to_owned(), 10), "short");
        assert_eq!(truncate_chars("abcdefgh".to_owned(), 4), "abcd...");
    }
}
