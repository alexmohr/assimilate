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
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn numbered_lines(count: usize) -> String {
        (1..=count).fold(String::new(), |mut text, n| {
            writeln!(text, "line {n}").unwrap();
            text
        })
    }

    fn text_of(line: &FileDiffLine) -> String {
        line.segments.iter().map(|s| s.text.as_str()).collect()
    }

    fn hunks_of(response: FileDiffResponse) -> (usize, usize, Vec<FileDiffHunk>) {
        let FileDiffResponse::Text {
            added,
            removed,
            hunks,
        } = response
        else {
            panic!("expected a text diff, got {response:?}");
        };
        (added, removed, hunks)
    }

    #[test]
    fn identical_files_have_no_hunks() {
        let (added, removed, hunks) = hunks_of(diff_text("a\nb\n", "a\nb\n", DiffScope::Changes));
        assert_eq!((added, removed), (0, 0));
        assert_eq!(hunks, Vec::new());
    }

    #[test]
    fn a_changed_line_is_a_removal_and_an_addition_with_line_numbers() {
        let base = "user www;\nworker_processes auto;\npid /run/nginx.pid;\n";
        let target = "user www;\nworker_processes 4;\npid /run/nginx.pid;\n";
        let (added, removed, hunks) = hunks_of(diff_text(base, target, DiffScope::Changes));

        assert_eq!((added, removed), (1, 1));
        let [hunk] = hunks.as_slice() else {
            panic!("expected one hunk, got {hunks:?}");
        };
        assert_eq!(
            (
                hunk.base_start,
                hunk.base_count,
                hunk.target_start,
                hunk.target_count
            ),
            (1, 3, 1, 3)
        );
        let summary: Vec<_> = hunk
            .lines
            .iter()
            .map(|l| (l.kind, l.base_line, l.target_line, text_of(l)))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    FileDiffLineKind::Context,
                    Some(1),
                    Some(1),
                    "user www;".to_owned()
                ),
                (
                    FileDiffLineKind::Removed,
                    Some(2),
                    None,
                    "worker_processes auto;".to_owned()
                ),
                (
                    FileDiffLineKind::Added,
                    None,
                    Some(2),
                    "worker_processes 4;".to_owned()
                ),
                (
                    FileDiffLineKind::Context,
                    Some(3),
                    Some(3),
                    "pid /run/nginx.pid;".to_owned()
                ),
            ]
        );
    }

    /// Words are split on whitespace, so the `;` stays with the word it ends.
    #[test]
    fn only_the_changed_word_of_a_changed_line_is_emphasized() {
        let (_, _, hunks) = hunks_of(diff_text(
            "worker_processes auto;\n",
            "worker_processes 4;\n",
            DiffScope::Changes,
        ));
        let emphasized: Vec<_> = hunks
            .iter()
            .flat_map(|h| &h.lines)
            .map(|l| {
                l.segments
                    .iter()
                    .filter(|s| s.emphasized)
                    .map(|s| s.text.as_str())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(emphasized, vec!["auto;".to_owned(), "4;".to_owned()]);
    }

    #[test]
    fn changes_far_apart_are_separate_hunks_with_three_lines_of_context() {
        let base = numbered_lines(30);
        let target = base
            .replace("line 2\n", "line two\n")
            .replace("line 28\n", "line twenty-eight\n");
        let (_, _, hunks) = hunks_of(diff_text(&base, &target, DiffScope::Changes));

        let ranges: Vec<_> = hunks.iter().map(|h| (h.base_start, h.base_count)).collect();
        assert_eq!(ranges, vec![(1, 5), (25, 6)]);
    }

    #[test]
    fn the_whole_file_scope_keeps_every_line_in_one_hunk() {
        let base = numbered_lines(30);
        let target = base.replace("line 15\n", "line fifteen\n");
        let (_, _, hunks) = hunks_of(diff_text(&base, &target, DiffScope::WholeFile));

        let [hunk] = hunks.as_slice() else {
            panic!("expected one hunk, got {hunks:?}");
        };
        assert_eq!(hunk.lines.len(), 31);
    }

    #[test]
    fn the_whole_file_scope_shows_an_identical_file() {
        let (_, _, hunks) = hunks_of(diff_text("a\nb\n", "a\nb\n", DiffScope::WholeFile));
        let [hunk] = hunks.as_slice() else {
            panic!("expected one hunk, got {hunks:?}");
        };
        assert!(
            hunk.lines
                .iter()
                .all(|l| matches!(l.kind, FileDiffLineKind::Context))
        );
    }

    #[test]
    fn two_empty_files_have_nothing_to_show_even_whole() {
        let (_, _, hunks) = hunks_of(diff_text("", "", DiffScope::WholeFile));
        assert_eq!(hunks, Vec::new());
    }

    #[test]
    fn a_file_added_from_nothing_is_all_additions() {
        let (added, removed, hunks) = hunks_of(diff_text("", "a\nb\n", DiffScope::Changes));
        assert_eq!((added, removed), (2, 0));
        assert!(
            hunks
                .iter()
                .flat_map(|h| &h.lines)
                .all(|l| matches!(l.kind, FileDiffLineKind::Added))
        );
    }

    #[test]
    fn line_endings_are_not_part_of_the_text() {
        let (_, _, hunks) = hunks_of(diff_text("a\r\nb\r\n", "a\r\nc", DiffScope::Changes));
        let texts: Vec<_> = hunks.iter().flat_map(|h| &h.lines).map(text_of).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn the_scope_follows_the_whole_file_flag() {
        assert_eq!(DiffScope::from(true), DiffScope::WholeFile);
        assert_eq!(DiffScope::from(false), DiffScope::Changes);
    }
}
