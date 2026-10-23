// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Whole-subtree reads of the content index: adding up what a set of paths
//! holds.
//!
//! The stored directory blobs are decoded in the server rather than asking
//! borg, so a read costs one query over the archive's index rows and never
//! touches the repository.

use std::collections::HashMap;

use shared::responses::PathTotals;
use sqlx::PgPool;

use super::codec::{self, DirEntry};
use crate::error::ApiError;

/// Whether an index entry is a directory or something a restore writes as a
/// file (a regular file, symlink or device).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
}

impl From<&str> for EntryKind {
    /// Reads borg's one-character entry type, where only `d` is a directory.
    fn from(borg_type: &str) -> Self {
        match borg_type {
            "d" => Self::Dir,
            _ => Self::File,
        }
    }
}

fn full_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent directory of an archive path; the root's is itself.
fn parent_dir(path: &str) -> &str {
    path.rfind('/')
        .map_or("", |i| path.get(..i).unwrap_or_default())
}

/// `path` and each directory above it, innermost first, ending at the root
/// (`""`).
fn path_and_ancestors(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(path), |current| {
        (!current.is_empty()).then(|| parent_dir(current))
    })
}

async fn archive_id(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Option<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT id FROM archives WHERE repo_id = $1 AND name = $2",
        repo_id,
        archive_name,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

fn decode_blob(blob: &[u8]) -> Result<Vec<DirEntry>, ApiError> {
    codec::decode(blob)
        .map_err(|e| ApiError::Internal(format!("corrupt archive directory index: {e}")))
}

/// Adds up, for each of `paths`, the non-directory entries at or under it.
/// `listings` are `(directory, children)` pairs; a directory may appear in
/// several pairs when its listing spans several chunks.
fn accumulate_totals<'a>(
    paths: &[String],
    listings: impl IntoIterator<Item = (&'a str, &'a [DirEntry])>,
) -> Vec<PathTotals> {
    let mut totals: Vec<PathTotals> = paths
        .iter()
        .map(|path| PathTotals {
            path: path.clone(),
            files: 0,
            bytes: 0,
        })
        .collect();
    let mut wanted: HashMap<&str, Vec<usize>> = HashMap::new();
    paths.iter().enumerate().for_each(|(index, path)| {
        wanted.entry(path.as_str()).or_default().push(index);
    });

    listings.into_iter().for_each(|(dir, entries)| {
        entries
            .iter()
            .filter(|entry| EntryKind::from(entry.entry_type.as_str()) == EntryKind::File)
            .for_each(|entry| {
                let path = full_path(dir, &entry.name);
                path_and_ancestors(&path)
                    .filter_map(|covering| wanted.get(covering))
                    .flatten()
                    .for_each(|&index| {
                        if let Some(total) = totals.get_mut(index) {
                            total.files = total.files.saturating_add(1);
                            total.bytes = total.bytes.saturating_add(entry.size);
                        }
                    });
            });
    });
    totals
}

/// What each of `paths` holds according to the archive's index: a file
/// counts itself, a directory everything beneath it. A path the index does
/// not know totals zero.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails, or [`ApiError::Internal`]
/// if a stored directory blob cannot be decoded.
pub async fn path_totals(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    paths: &[String],
) -> Result<Vec<PathTotals>, ApiError> {
    let Some(archive_id) = archive_id(pool, repo_id, archive_name).await? else {
        return Ok(accumulate_totals(paths, std::iter::empty()));
    };

    // A file lives in its parent's listing, everything under a directory in
    // the listings of the directory itself and of the directories below it.
    let exact_dirs: Vec<String> = paths
        .iter()
        .flat_map(|path| [path.clone(), parent_dir(path).to_owned()])
        .collect();
    let dir_prefixes: Vec<String> = paths.iter().map(|path| format!("{path}/")).collect();

    let rows = sqlx::query!(
        "SELECT p.path AS dir_path, d.entries FROM archive_dirs d JOIN archive_paths p ON p.id = \
         d.dir_path_id WHERE d.archive_id = $1 AND (p.path = ANY($2::text[]) OR EXISTS (SELECT 1 \
         FROM unnest($3::text[]) AS prefix(value) WHERE starts_with(p.path, prefix.value)))",
        archive_id,
        &exact_dirs,
        &dir_prefixes,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let listings: Vec<(String, Vec<DirEntry>)> = rows
        .into_iter()
        .map(|row| decode_blob(&row.entries).map(|entries| (row.dir_path, entries)))
        .collect::<Result<_, _>>()?;

    Ok(accumulate_totals(
        paths,
        listings
            .iter()
            .map(|(dir, entries)| (dir.as_str(), entries.as_slice())),
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::archives::ContentEntry;

    fn entry(name: &str, entry_type: &str, size: i64, mtime: &str) -> DirEntry {
        DirEntry {
            name: name.to_owned(),
            entry_type: entry_type.to_owned(),
            size,
            mtime: mtime.to_owned(),
            mode: String::new(),
        }
    }

    #[test]
    fn borg_entry_types_map_to_kinds() {
        assert_eq!(EntryKind::from("d"), EntryKind::Dir);
        assert_eq!(EntryKind::from("-"), EntryKind::File);
        assert_eq!(EntryKind::from("l"), EntryKind::File);
    }

    #[test]
    fn ancestors_walk_up_to_the_root() {
        assert_eq!(
            path_and_ancestors("etc/nginx/nginx.conf").collect::<Vec<_>>(),
            ["etc/nginx/nginx.conf", "etc/nginx", "etc", ""]
        );
        assert_eq!(path_and_ancestors("").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn totals_count_files_under_directories_and_files_themselves() {
        let root = vec![entry("etc", "d", 0, ""), entry("README", "-", 5, "")];
        let etc = vec![
            entry("nginx", "d", 0, ""),
            entry("hosts", "-", 100, ""),
            entry("link", "l", 1, ""),
        ];
        let nginx = vec![
            entry("nginx.conf", "-", 40, ""),
            entry("conf.d", "d", 0, ""),
        ];
        let conf_d = vec![entry("default.conf", "-", 7, "")];
        let paths = vec![
            "etc".to_owned(),
            "etc/nginx".to_owned(),
            "etc/hosts".to_owned(),
            "README".to_owned(),
            "missing".to_owned(),
        ];

        let totals = accumulate_totals(
            &paths,
            [
                ("", root.as_slice()),
                ("etc", etc.as_slice()),
                ("etc/nginx", nginx.as_slice()),
                ("etc/nginx/conf.d", conf_d.as_slice()),
            ],
        );

        let summary: Vec<(&str, i64, i64)> = totals
            .iter()
            .map(|t| (t.path.as_str(), t.files, t.bytes))
            .collect();
        assert_eq!(
            summary,
            [
                ("etc", 4, 148),
                ("etc/nginx", 2, 47),
                ("etc/hosts", 1, 100),
                ("README", 1, 5),
                ("missing", 0, 0),
            ]
        );
    }

    #[test]
    fn a_directory_split_across_chunks_is_counted_once_per_entry() {
        let first = vec![entry("a", "-", 1, "")];
        let second = vec![entry("b", "-", 2, "")];
        let paths = vec!["dir".to_owned(), "dir".to_owned()];

        let totals = accumulate_totals(
            &paths,
            [("dir", first.as_slice()), ("dir", second.as_slice())],
        );

        assert!(totals.iter().all(|t| t.files == 2 && t.bytes == 3));
    }

    fn indexed(path: &str, entry_type: &str, size: i64) -> ContentEntry {
        ContentEntry {
            entry_type: entry_type.to_owned(),
            path: path.to_owned(),
            size,
            mtime: "2026-01-10T02:00:00.000000".to_owned(),
            mode: String::new(),
        }
    }

    /// Stores an index for `daily-1` the way indexing does, and returns the
    /// repository id.
    pub(crate) async fn index_archive(pool: &PgPool, raw: Vec<ContentEntry>) -> i64 {
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "tree-repo",
                repo_path: "/backups/tree",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"encrypted_data",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        let archive_id = super::super::get_or_create_archive_id(pool, repo.id, "daily-1")
            .await
            .unwrap();
        let expanded = super::super::expand_entries_with_ancestors(raw);
        let dir_paths: Vec<String> = expanded.dirs.iter().map(|(dir, _)| dir.clone()).collect();
        let ids = super::super::ensure_archive_paths(pool, repo.id, &dir_paths)
            .await
            .unwrap();
        let dirs: Vec<(i64, Vec<DirEntry>)> = expanded
            .dirs
            .into_iter()
            .map(|(dir, entries)| (*ids.get(&dir).unwrap(), entries))
            .collect();
        super::super::replace_archive_dirs(pool, archive_id, &dirs)
            .await
            .unwrap();
        repo.id
    }

    /// Marks every archive of `repo_id` as fully indexed.
    pub(crate) async fn mark_indexed(pool: &PgPool, repo_id: i64) {
        sqlx::query!(
            "INSERT INTO archive_index_jobs (archive_id, status, finished_at) SELECT id, 'done', \
             NOW() FROM archives WHERE repo_id = $1 ON CONFLICT (archive_id) DO UPDATE SET status \
             = 'done', finished_at = NOW()",
            repo_id,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    pub(crate) fn demo_tree() -> Vec<ContentEntry> {
        vec![
            indexed("etc/nginx/nginx.conf", "-", 40),
            indexed("etc/nginx/conf.d/default.conf", "-", 7),
            indexed("etc/hosts", "-", 100),
            indexed("var/www/html/index.html", "-", 30),
            indexed("var/www/html/app.js", "-", 2000),
        ]
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_add_up_whole_subtrees_from_the_index(pool: PgPool) {
        let repo_id = index_archive(&pool, demo_tree()).await;
        let paths = vec![
            "etc".to_owned(),
            "var/www/html/app.js".to_owned(),
            "var".to_owned(),
            "nope".to_owned(),
        ];

        let totals = path_totals(&pool, repo_id, "daily-1", &paths)
            .await
            .unwrap();

        let summary: Vec<(&str, i64, i64)> = totals
            .iter()
            .map(|t| (t.path.as_str(), t.files, t.bytes))
            .collect();
        assert_eq!(
            summary,
            [
                ("etc", 3, 147),
                ("var/www/html/app.js", 1, 2000),
                ("var", 2, 2030),
                ("nope", 0, 0),
            ]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_archive_the_index_does_not_know_totals_zero_for_every_path(pool: PgPool) {
        let repo_id = index_archive(&pool, demo_tree()).await;
        let paths = vec!["etc".to_owned(), "var/www/html/app.js".to_owned()];

        let totals = path_totals(&pool, repo_id, "never-indexed", &paths)
            .await
            .unwrap();

        assert_eq!(
            totals,
            [
                PathTotals {
                    path: "etc".to_owned(),
                    files: 0,
                    bytes: 0,
                },
                PathTotals {
                    path: "var/www/html/app.js".to_owned(),
                    files: 0,
                    bytes: 0,
                },
            ]
        );
    }
}
