// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    borg::log_json::{
        BorgLogLevel, BorgLogLine, BorgLogRecordType, BorgMsgId, parse_diagnostics, stderr_lines,
        truncate_chars,
    },
    types::{BackupWarningKind, FileChangePattern},
};
use tracing::{info, warn};

use super::{BackupError, ParsedStats};

pub(super) fn parse_json_stats(stdout: &[u8]) -> Result<ParsedStats, BackupError> {
    let output = String::from_utf8_lossy(stdout);
    let json: serde_json::Value = serde_json::from_str(output.trim())
        .map_err(|e| BackupError::StatsParse(format!("invalid JSON: {e}")))?;

    let stats = json
        .get("archive")
        .and_then(|a| a.get("stats"))
        .ok_or_else(|| BackupError::StatsParse("missing archive.stats".to_owned()))?;

    let original_size = stats
        .get("original_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing original_size".to_owned()))?;

    let compressed_size = stats
        .get("compressed_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing compressed_size".to_owned()))?;

    let deduplicated_size = stats
        .get("deduplicated_size")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing deduplicated_size".to_owned()))?;

    let repo_unique_csize = json
        .get("cache")
        .and_then(|c| c.get("stats"))
        .and_then(|s| s.get("unique_csize"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    let files_processed = stats
        .get("nfiles")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| BackupError::StatsParse("missing nfiles".to_owned()))?;

    Ok(ParsedStats {
        original_size,
        compressed_size,
        deduplicated_size,
        repo_unique_csize,
        files_processed,
    })
}

/// The message a report carries when borg ended with `exit_code` and no
/// diagnostic to explain it: the bare exit code tells a user nothing, so
/// whatever else borg printed is attached.
pub(super) fn suppressed_error_exit(exit_code: i32) -> String {
    format!(
        "borg exited with code {exit_code} after an error-level message that an ignore file \
         change pattern suppressed; ignore patterns are meant for file-change warnings, so the \
         run is reported rather than passed - see the backup log for borg's own output"
    )
}

pub(super) fn unexplained_exit(exit_code: i32, context: &[String]) -> String {
    format!(
        "borg exited with code {exit_code} but reported no warning or error explaining why{suffix}",
        suffix = context_suffix(context)
    )
}

fn context_suffix(context: &[String]) -> String {
    if context.is_empty() {
        String::new()
    } else {
        format!("; last borg output: {}", context.join(" | "))
    }
}

/// Logs what became of the `reported` warnings of a warning-status run and
/// returns the report's error message.
///
/// The message is kept populated (not None) despite duplicating `warnings`:
/// `dispatch_backup_completion_notification`'s `backup_warning` path reads only
/// this field for the email/push body, so clearing it silently drops warning
/// text from notifications. The duplicate-display bug this was meant to fix is
/// handled at the UI layer instead (report detail views hide the Error box
/// when status is Warning).
pub(super) fn summarize_warnings(warnings: &[String], reported: usize) -> Option<String> {
    let summary = warnings.join("; ");
    if warnings.is_empty() {
        info!("Borg reported {reported} warning(s), all suppressed by ignore patterns");
    } else {
        warn!("Borg reported warnings: {summary}");
    }
    (!summary.is_empty()).then_some(summary)
}

/// What the warnings a run reports are about. Only a run whose every warning
/// was a file changing under borg counts as [`BackupWarningKind::FileChanged`]:
/// one that also warned about anything else - including the messages this
/// module adds itself for an unexplained or suppressed exit - is a general
/// warning, so it still reaches whoever alerts on those.
pub(super) fn warning_kind(warnings: &[String], file_changed: &[String]) -> BackupWarningKind {
    if !warnings.is_empty() && warnings.iter().all(|w| file_changed.contains(w)) {
        BackupWarningKind::FileChanged
    } else {
        BackupWarningKind::General
    }
}

/// What to log when `borg <subcommand>` ends with borg's warning status. These
/// runs report no status of their own, so the log line is the only trace they
/// leave - and since the `--show-rc` footer is stripped from the diagnostics,
/// a run whose footer was its only output must still say something.
pub(crate) fn warning_status_log(subcommand: &str, exit_code: i32, stderr: &str) -> String {
    let diagnostics = parse_diagnostics(stderr);
    if diagnostics.warnings.is_empty() {
        format!(
            "borg {subcommand} exited with code {exit_code} but reported no warning or error \
             explaining why{suffix}",
            suffix = context_suffix(&diagnostics.context)
        )
    } else {
        format!(
            "borg {subcommand} warnings: {}",
            diagnostics.warnings.join("; ")
        )
    }
}

#[cfg(test)]
pub(crate) fn parse_warnings(stderr: &str) -> Vec<String> {
    parse_diagnostics(stderr).warnings
}

/// How much of a failure description is kept; it is stored on the report and
/// shown in full in its Error box.
pub(super) const MAX_FAILURE_CHARS: usize = 4000;

/// Renders borg's stderr into an error message. The raw buffer is a wall of
/// JSON progress records that ends up in the report's Error box, so prefer the
/// diagnostics and fall back to the tail of whatever else borg printed.
pub(crate) fn describe_borg_failure(stderr: &str) -> String {
    let diagnostics = parse_diagnostics(stderr);
    let described = diagnostics
        .warnings
        .iter()
        .chain(diagnostics.context.iter())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("; ");
    if described.is_empty() {
        // Not the same as "borg printed nothing": its `--show-rc` footer is
        // stripped from the diagnostics, and on a failed run that footer may be
        // the only line there was.
        "borg reported no diagnostic beyond its exit status".to_owned()
    } else {
        truncate_chars(described, MAX_FAILURE_CHARS)
    }
}

/// Whether borg signalled a warning or worse on stderr. Unlike
/// [`parse_diagnostics`] this counts the `--show-rc` footer: an exit code of 1
/// with nothing but the footer is still borg reporting a warning, it just is
/// not saying why.
pub(crate) fn stderr_has_warnings(stderr: &str) -> bool {
    stderr_lines(stderr).any(|line| {
        let Ok(record) = serde_json::from_str::<BorgLogLine>(line) else {
            return false;
        };
        record.record_type == BorgLogRecordType::LogMessage
            && record.levelname.is_some_and(BorgLogLevel::is_diagnostic)
    })
}

/// Returns the messages for any log entries whose `msgid` indicates a backup
/// source path was not found.  These are emitted as `WARNING` by borg (rc=1)
/// but represent a configuration error - a configured source directory did
/// not exist at backup time - and must be surfaced as a hard failure rather
/// than a silent warning.
pub(crate) fn filter_file_change_warnings(
    warnings: Vec<String>,
    patterns: &[FileChangePattern],
) -> Result<Vec<String>, BackupError> {
    let mut filtered = Vec::new();
    for warning in warnings {
        let found = patterns
            .iter()
            .find(|p| glob_match::glob_match(&p.path, &warning));
        if let Some(pattern) = found {
            match &pattern.action {
                shared::types::FileChangeAction::Ignore => {}
                shared::types::FileChangeAction::Fatal => {
                    return Err(BackupError::BorgFailed(format!(
                        "file change pattern matched fatal: {warning}"
                    )));
                }
                shared::types::FileChangeAction::Warn => {
                    filtered.push(warning);
                }
            }
        } else {
            filtered.push(warning);
        }
    }
    Ok(filtered)
}

pub(crate) fn parse_source_not_found_errors(stderr: &str) -> Vec<String> {
    stderr_lines(stderr)
        .filter_map(|line| {
            let record: BorgLogLine = serde_json::from_str(line).ok()?;
            if record.record_type != BorgLogRecordType::LogMessage {
                return None;
            }
            if record.msgid == Some(BorgMsgId::BackupFileNotFoundError) {
                record.message
            } else {
                None
            }
        })
        .collect()
}
