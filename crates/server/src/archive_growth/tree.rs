// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Folds an archive's files, each with the bytes only it holds, into the
//! folder tree the UI drills into.
//!
//! Only files holding such bytes are added, but on an archive with little
//! shared data (the first one of a host) that is nearly every file, so the
//! builder never keeps all of them: each folder keeps its largest files and
//! counts the rest, and the finished tree lists only entries big enough to
//! matter, folding the others into one node per folder.

use std::{cmp::Reverse, collections::BTreeMap};

use shared::archive_growth::{ArchiveGrowth, GrowthFile, GrowthNode, GrowthNodeKind};

/// Files kept one by one per folder while reading; the rest are only counted.
const FILES_KEPT_PER_FOLDER: usize = 50;

/// Entries of a folder listed whatever their size, so a small folder opened
/// in the UI still shows what is in it.
const ALWAYS_LISTED: usize = 5;

/// Most entries listed per folder.
const MOST_LISTED: usize = 50;

/// Beyond [`ALWAYS_LISTED`], an entry is listed only if it holds at least
/// one part in this many of the archive's total.
const LISTED_SHARE_DIVISOR: i64 = 1000;

/// Files listed among the archive's largest.
const LARGEST_FILES: usize = 25;

#[derive(Debug, Default)]
struct Folder {
    bytes: i64,
    files: i64,
    folders: BTreeMap<String, Folder>,
    /// The largest files directly in this folder, by name.
    kept_files: Vec<(String, i64)>,
    /// Bytes and count of the files directly in it that were not kept.
    other_bytes: i64,
    other_files: i64,
}

impl Folder {
    fn add_file(&mut self, name: &str, bytes: i64) {
        if self.kept_files.len() < FILES_KEPT_PER_FOLDER {
            self.kept_files.push((name.to_owned(), bytes));
            return;
        }
        let smallest = self
            .kept_files
            .iter_mut()
            .min_by_key(|(_, kept)| *kept)
            .filter(|(_, kept)| *kept < bytes);
        let (dropped_bytes, dropped_files) = smallest.map_or((bytes, 1), |slot| {
            let (_, dropped) = std::mem::replace(slot, (name.to_owned(), bytes));
            (dropped, 1)
        });
        self.other_bytes = self.other_bytes.saturating_add(dropped_bytes);
        self.other_files = self.other_files.saturating_add(dropped_files);
    }

    /// The single folder this one consists of, if it holds nothing else.
    fn only_subfolder(&self) -> bool {
        self.folders.len() == 1 && self.kept_files.is_empty() && self.other_files == 0
    }
}

/// One entry of a folder, before it is decided whether it is listed.
enum Candidate {
    Folder(String, Folder),
    File(String, i64),
}

impl Candidate {
    const fn bytes(&self) -> i64 {
        match self {
            Self::Folder(_, folder) => folder.bytes,
            Self::File(_, bytes) => *bytes,
        }
    }

    const fn files(&self) -> i64 {
        match self {
            Self::Folder(_, folder) => folder.files,
            Self::File(..) => 1,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Folder(name, _) | Self::File(name, _) => name,
        }
    }

    fn into_node(self, total: i64) -> GrowthNode {
        match self {
            Self::Folder(name, folder) => {
                let (name, folder) = merge_single_subfolders(name, folder);
                GrowthNode {
                    name,
                    kind: GrowthNodeKind::Directory,
                    bytes: folder.bytes,
                    files: folder.files,
                    children: nodes(folder, total),
                }
            }
            Self::File(name, bytes) => GrowthNode {
                name,
                kind: GrowthNodeKind::File,
                bytes,
                files: 1,
                children: Vec::new(),
            },
        }
    }
}

/// Merges a folder that holds nothing but one other folder into it, so a
/// path like `home/alex` is one step to open rather than two.
fn merge_single_subfolders(mut name: String, mut folder: Folder) -> (String, Folder) {
    while let Some((child_name, child)) = folder
        .only_subfolder()
        .then(|| folder.folders.pop_first())
        .flatten()
    {
        name = format!("{name}/{child_name}");
        folder = child;
    }
    (name, folder)
}

/// The listed entries of `folder`, largest first, with what is not listed
/// folded into a trailing [`GrowthNodeKind::Others`] node.
fn nodes(folder: Folder, total: i64) -> Vec<GrowthNode> {
    let Folder {
        folders,
        kept_files,
        other_bytes,
        other_files,
        ..
    } = folder;
    let mut candidates: Vec<Candidate> = folders
        .into_iter()
        .map(|(name, folder)| Candidate::Folder(name, folder))
        .chain(
            kept_files
                .into_iter()
                .map(|(name, bytes)| Candidate::File(name, bytes)),
        )
        .collect();
    candidates.sort_by(|a, b| {
        b.bytes()
            .cmp(&a.bytes())
            .then_with(|| a.name().cmp(b.name()))
    });

    let threshold = total / LISTED_SHARE_DIVISOR;
    let listed_count = candidates
        .iter()
        .enumerate()
        .take_while(|(rank, candidate)| {
            *rank < ALWAYS_LISTED || (*rank < MOST_LISTED && candidate.bytes() >= threshold)
        })
        .count();
    let unlisted = candidates.split_off(listed_count);

    let others_bytes = unlisted
        .iter()
        .map(Candidate::bytes)
        .fold(other_bytes, i64::saturating_add);
    let others_files = unlisted
        .iter()
        .map(Candidate::files)
        .fold(other_files, i64::saturating_add);

    candidates
        .into_iter()
        .map(|candidate| candidate.into_node(total))
        .chain((others_files > 0).then(|| GrowthNode {
            name: String::new(),
            kind: GrowthNodeKind::Others,
            bytes: others_bytes,
            files: others_files,
            children: Vec::new(),
        }))
        .collect()
}

/// Collects an archive's files and the bytes only each of them holds.
#[derive(Debug, Default)]
pub struct GrowthBuilder {
    root: Folder,
    /// The largest files so far, smallest first once full.
    largest: Vec<GrowthFile>,
}

impl GrowthBuilder {
    /// Adds a file at `path` (as borg lists it, without a leading `/`) whose
    /// own size is `size`, of which `bytes` no other archive holds. A file
    /// holding no such bytes changes nothing.
    pub fn add(&mut self, path: &str, bytes: i64, size: i64) {
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let Some(name) = segments.pop() else {
            return;
        };
        if bytes <= 0 {
            return;
        }

        let folder = segments
            .into_iter()
            .fold(&mut self.root, |folder, segment| {
                folder.bytes = folder.bytes.saturating_add(bytes);
                folder.files = folder.files.saturating_add(1);
                folder.folders.entry(segment.to_owned()).or_default()
            });
        folder.bytes = folder.bytes.saturating_add(bytes);
        folder.files = folder.files.saturating_add(1);
        folder.add_file(name, bytes);

        self.keep_if_largest(path, bytes, size);
    }

    fn keep_if_largest(&mut self, path: &str, bytes: i64, size: i64) {
        let file = GrowthFile {
            path: path.to_owned(),
            bytes,
            size,
        };
        if self.largest.len() < LARGEST_FILES {
            self.largest.push(file);
            return;
        }
        if let Some(smallest) = self
            .largest
            .iter_mut()
            .min_by_key(|kept| kept.bytes)
            .filter(|kept| kept.bytes < bytes)
        {
            *smallest = file;
        }
    }

    /// The breakdown of everything added.
    #[must_use]
    pub fn finish(self) -> ArchiveGrowth {
        let Self { root, mut largest } = self;
        largest.sort_by_key(|file| (Reverse(file.bytes), file.path.clone()));
        let total = root.bytes;
        let files = root.files;
        ArchiveGrowth {
            total_bytes: total,
            files,
            children: nodes(root, total),
            largest_files: largest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(files: &[(&str, i64)]) -> ArchiveGrowth {
        let mut builder = GrowthBuilder::default();
        for (path, bytes) in files {
            builder.add(path, *bytes, bytes.saturating_mul(2));
        }
        builder.finish()
    }

    /// `count` files `<folder>/<i>` holding `bytes(i)` each.
    fn numbered(folder: &str, count: usize, bytes: impl Fn(i64) -> i64) -> Vec<(String, i64)> {
        (0..count)
            .map(|i| {
                let i = i64::try_from(i).unwrap();
                (format!("{folder}/{i:03}"), bytes(i))
            })
            .collect()
    }

    fn borrowed(files: &[(String, i64)]) -> Vec<(&str, i64)> {
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), *bytes))
            .collect()
    }

    fn summary(nodes: &[GrowthNode]) -> Vec<(&str, GrowthNodeKind, i64, i64)> {
        nodes
            .iter()
            .map(|node| (node.name.as_str(), node.kind, node.bytes, node.files))
            .collect()
    }

    fn child<'a>(nodes: &'a [GrowthNode], name: &str) -> &'a [GrowthNode] {
        &nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("no entry {name} in {nodes:?}"))
            .children
    }

    #[test]
    fn folders_add_up_the_files_inside_them() {
        let growth = build(&[
            ("home/alex/Downloads/ubuntu.iso", 1900),
            ("home/alex/Downloads/photos.zip", 100),
            ("home/alex/.cache/pip.whl", 300),
            ("var/log/syslog", 700),
        ]);

        assert_eq!((growth.total_bytes, growth.files), (3000, 4));
        assert_eq!(
            summary(&growth.children),
            [
                ("home/alex", GrowthNodeKind::Directory, 2300, 3),
                ("var/log", GrowthNodeKind::Directory, 700, 1),
            ]
        );
        let alex = child(&growth.children, "home/alex");
        assert_eq!(
            summary(alex),
            [
                ("Downloads", GrowthNodeKind::Directory, 2000, 2),
                (".cache", GrowthNodeKind::Directory, 300, 1),
            ]
        );
        assert_eq!(
            summary(child(alex, "Downloads")),
            [
                ("ubuntu.iso", GrowthNodeKind::File, 1900, 1),
                ("photos.zip", GrowthNodeKind::File, 100, 1),
            ]
        );
    }

    #[test]
    fn files_holding_nothing_of_their_own_are_left_out() {
        let growth = build(&[("etc/hosts", 0), ("etc/fstab", 12), ("", 5), ("/", 5)]);

        assert_eq!((growth.total_bytes, growth.files), (12, 1));
        assert_eq!(
            summary(&growth.children),
            [("etc", GrowthNodeKind::Directory, 12, 1)]
        );
        assert_eq!(
            summary(child(&growth.children, "etc")),
            [("fstab", GrowthNodeKind::File, 12, 1)]
        );
    }

    #[test]
    fn a_folder_with_files_beside_its_subfolder_is_not_merged() {
        let growth = build(&[("srv/app/data.bin", 50), ("srv/readme", 10)]);

        assert_eq!(
            summary(&growth.children),
            [("srv", GrowthNodeKind::Directory, 60, 2)]
        );
        assert_eq!(
            summary(child(&growth.children, "srv")),
            [
                ("app", GrowthNodeKind::Directory, 50, 1),
                ("readme", GrowthNodeKind::File, 10, 1),
            ]
        );
    }

    #[test]
    fn small_entries_past_the_first_few_are_folded_together() {
        let mut files = numbered("data", 20, |_| 10);
        files.push(("data/big".to_owned(), 100_000));

        let growth = build(&borrowed(&files));

        let listed = summary(child(&growth.children, "data"));
        assert_eq!(
            listed,
            [
                ("big", GrowthNodeKind::File, 100_000, 1),
                ("000", GrowthNodeKind::File, 10, 1),
                ("001", GrowthNodeKind::File, 10, 1),
                ("002", GrowthNodeKind::File, 10, 1),
                ("003", GrowthNodeKind::File, 10, 1),
                ("", GrowthNodeKind::Others, 160, 16),
            ]
        );
        assert_eq!(listed.len(), ALWAYS_LISTED.saturating_add(1));
    }

    #[test]
    fn small_folders_are_folded_together_with_small_files() {
        let mut files = numbered("data", 4, |_| 10);
        files.extend((0..3).map(|i| (format!("data/sub-{i}/f"), 5)));
        files.push(("data/big".to_owned(), 100_000));

        let growth = build(&borrowed(&files));

        let listed = summary(child(&growth.children, "data"));
        assert_eq!(listed.len(), ALWAYS_LISTED.saturating_add(1));
        assert_eq!(listed.last(), Some(&("", GrowthNodeKind::Others, 15, 3)));
    }

    #[test]
    fn a_folder_lists_at_most_the_cap_however_big_its_entries() {
        let files = numbered("big", MOST_LISTED.saturating_add(10), |_| 1000);

        let growth = build(&borrowed(&files));

        let entries = child(&growth.children, "big");
        assert_eq!(entries.len(), MOST_LISTED.saturating_add(1));
        let others = entries.last().unwrap();
        assert_eq!(
            (others.kind, others.files, others.bytes),
            (GrowthNodeKind::Others, 10, 10_000)
        );
    }

    #[test]
    fn a_folder_with_too_many_files_counts_the_smallest_without_keeping_them() {
        let files = numbered("many", FILES_KEPT_PER_FOLDER.saturating_add(3), |i| {
            i.saturating_add(1)
        });
        let mut builder = GrowthBuilder::default();
        for (path, bytes) in &files {
            builder.add(path, *bytes, 0);
        }

        let many = builder.root.folders.get("many").unwrap();
        assert_eq!(many.kept_files.len(), FILES_KEPT_PER_FOLDER);
        assert_eq!((many.other_files, many.other_bytes), (3, 6));
        assert!(many.kept_files.iter().all(|(_, bytes)| *bytes > 3));
        assert_eq!(
            many.files,
            i64::try_from(FILES_KEPT_PER_FOLDER.saturating_add(3)).unwrap()
        );
    }

    #[test]
    fn the_largest_files_are_listed_biggest_first_with_their_size() {
        let files = numbered("d", LARGEST_FILES.saturating_add(5), |i| {
            i.saturating_mul(10).saturating_add(1)
        });

        let growth = build(&borrowed(&files));

        assert_eq!(growth.largest_files.len(), LARGEST_FILES);
        let first = growth.largest_files.first().unwrap();
        assert_eq!(
            (first.path.as_str(), first.bytes, first.size),
            ("d/029", 291, 582)
        );
        assert!(
            growth
                .largest_files
                .windows(2)
                .all(|pair| pair.first().map(|f| f.bytes) >= pair.last().map(|f| f.bytes))
        );
        assert_eq!(growth.largest_files.last().unwrap().bytes, 51);
    }

    #[test]
    fn an_empty_archive_has_no_breakdown() {
        let growth = build(&[]);

        assert_eq!((growth.total_bytes, growth.files), (0, 0));
        assert_eq!(growth.children, []);
        assert_eq!(growth.largest_files, []);
    }
}
