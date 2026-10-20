// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Turns two versions of a text file into the hunks a file comparison shows.

use std::time::{Duration, Instant};

use shared::responses::{
    FileDiffHunk, FileDiffLine, FileDiffLineKind, FileDiffResponse, FileDiffSegment,
};
use similar::{Algorithm, ChangeTag, DiffOp, InlineChange, TextDiff};

/// Unchanged lines kept around each change when only the changes are shown.
const CONTEXT_LINES: usize = 3;

/// How long the line and in-line diffs may search for a minimal result before
/// settling for a coarser one, so a pathological file cannot pin a CPU.
const DIFF_TIMEOUT: Duration = Duration::from_secs(2);

/// How much of the file a comparison shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffScope {
    /// Each change with [`CONTEXT_LINES`] of context around it.
    Changes,
    /// Every line of the file, as one hunk.
    WholeFile,
}

impl From<bool> for DiffScope {
    fn from(whole_file: bool) -> Self {
        if whole_file {
            Self::WholeFile
        } else {
            Self::Changes
        }
    }
}

/// Compares `base` with `target` line by line, with the changed part of each
/// changed line emphasized.
#[must_use]
pub fn diff_text(base: &str, target: &str, scope: DiffScope) -> FileDiffResponse {
    let deadline = Instant::now().checked_add(DIFF_TIMEOUT);
    let mut config = TextDiff::configure();
    config.algorithm(Algorithm::Patience);
    if let Some(deadline) = deadline {
        config.deadline(deadline);
    }
    let diff = config.diff_lines(base, target);

    let count = |tag: ChangeTag| {
        diff.iter_all_changes()
            .filter(|change| change.tag() == tag)
            .count()
    };
    let (added, removed) = (count(ChangeTag::Insert), count(ChangeTag::Delete));

    let groups = match scope {
        DiffScope::Changes => diff.grouped_ops(CONTEXT_LINES),
        DiffScope::WholeFile if diff.ops().is_empty() => Vec::new(),
        DiffScope::WholeFile => vec![diff.ops().to_vec()],
    };

    let hunks = groups
        .iter()
        .filter_map(|ops| hunk(&diff, ops, deadline))
        .collect();

    FileDiffResponse::Text {
        added,
        removed,
        hunks,
    }
}

fn hunk(
    diff: &TextDiff<'_, '_, str>,
    ops: &[DiffOp],
    deadline: Option<Instant>,
) -> Option<FileDiffHunk> {
    let (first, last) = (ops.first()?, ops.last()?);
    let base_range = first.old_range().start..last.old_range().end;
    let target_range = first.new_range().start..last.new_range().end;
    let lines = ops
        .iter()
        .flat_map(|op| diff.iter_inline_changes_deadline(op, deadline))
        .map(|change| line(&change))
        .collect();

    Some(FileDiffHunk {
        base_start: base_range.start.saturating_add(1),
        base_count: base_range.len(),
        target_start: target_range.start.saturating_add(1),
        target_count: target_range.len(),
        lines,
    })
}

fn line(change: &InlineChange<'_, str>) -> FileDiffLine {
    let kind = match change.tag() {
        ChangeTag::Equal => FileDiffLineKind::Context,
        ChangeTag::Delete => FileDiffLineKind::Removed,
        ChangeTag::Insert => FileDiffLineKind::Added,
    };
    // An unchanged line has nothing to point at, so it never carries emphasis.
    let emphasis_allowed = !matches!(kind, FileDiffLineKind::Context);
    let mut segments: Vec<FileDiffSegment> = change
        .iter_strings_lossy()
        .map(|(emphasized, text)| FileDiffSegment {
            text: text.into_owned(),
            emphasized: emphasized && emphasis_allowed,
        })
        .collect();
    if let Some(last) = segments.last_mut() {
        last.text = strip_line_ending(&last.text).to_owned();
    }
    segments.retain(|segment| !segment.text.is_empty());

    FileDiffLine {
        kind,
        base_line: change.old_index().map(|index| index.saturating_add(1)),
        target_line: change.new_index().map(|index| index.saturating_add(1)),
        segments,
    }
}

fn strip_line_ending(text: &str) -> &str {
    text.strip_suffix('\n')
        .map_or(text, |text| text.strip_suffix('\r').unwrap_or(text))
}

#[cfg(test)]
mod tests;
