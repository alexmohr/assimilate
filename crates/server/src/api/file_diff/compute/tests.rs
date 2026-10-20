// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

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
