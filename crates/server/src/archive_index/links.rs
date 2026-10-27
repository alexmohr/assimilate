// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Symbolic links in an archive: telling one from borg's item type, and
//! finding what its target is in the archive's content index.

use std::collections::{HashMap, HashSet};

use futures_util::TryStreamExt as _;
use shared::responses::{LinkResolution, SymlinkTarget};
use sqlx::PgPool;

use super::{decode_dir_blob, split_parent, tree::EntryKind};
use crate::{api::archives::ContentEntry, error::ApiError};

/// Borg's one-character item type, as far as links are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemType {
    Symlink,
    Other,
}

impl From<&str> for ItemType {
    fn from(borg_type: &str) -> Self {
        match borg_type {
            "l" => Self::Symlink,
            _ => Self::Other,
        }
    }
}

/// The link an entry of borg type `borg_type` is, with the `target` borg
/// reported for it, or `None` when it is not a symbolic link: borg 1 also
/// fills `linktarget` in for a hard link, naming the first path of the same
/// file, which is not where anything points.
#[must_use]
pub fn link_of(borg_type: &str, target: Option<String>) -> Option<SymlinkTarget> {
    match ItemType::from(borg_type) {
        ItemType::Symlink => Some(SymlinkTarget {
            target: target.filter(|target| !target.is_empty()),
            resolved: LinkResolution::Unknown,
        }),
        ItemType::Other => None,
    }
}

/// One segment of a link target.
enum Segment<'a> {
    /// `.` or an empty segment from a doubled or trailing slash.
    Current,
    /// `..`.
    Parent,
    Name(&'a str),
}

impl<'a> From<&'a str> for Segment<'a> {
    fn from(segment: &'a str) -> Self {
        match segment {
            "" | "." => Self::Current,
            ".." => Self::Parent,
            name => Self::Name(name),
        }
    }
}

/// The archive path a link in `link_dir` points to, worked out from the
/// target's text alone, the way the kernel would walk it from the link's
/// directory: an absolute target starts at the root, and `..` at the root
/// stays there.
///
/// An archive stores a backed-up absolute path without its leading `/`, so
/// the root of the archive stands for `/`. A link whose path runs through
/// another link is not followed through it.
#[must_use]
pub fn resolve_target(link_dir: &str, target: &str) -> String {
    let start: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        link_dir
            .split('/')
            .filter(|part| !part.is_empty())
            .collect()
    };
    target
        .split('/')
        .fold(start, |mut parts, segment| {
            match Segment::from(segment) {
                Segment::Current => {}
                Segment::Parent => {
                    parts.pop();
                }
                Segment::Name(name) => parts.push(name),
            }
            parts
        })
        .join("/")
}

/// Looks up where each link among `entries` points in the index of
/// `archive_id`, replacing its [`LinkResolution::Unknown`].
///
/// One query reads every directory the targets sit in, so a listing costs
/// one extra round trip however many links it holds, and none when it holds
/// none.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails, or
/// [`ApiError::Internal`] if a stored directory blob cannot be decoded.
pub async fn resolve_links(
    pool: &PgPool,
    repo_id: i64,
    archive_id: i64,
    entries: &mut [ContentEntry],
) -> Result<(), ApiError> {
    let targets: Vec<(usize, String)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let target = entry.link.as_ref()?.target.as_deref()?;
            let (dir, _) = split_parent(&entry.path);
            Some((index, resolve_target(dir, target)))
        })
        .collect();
    if targets.is_empty() {
        return Ok(());
    }

    let wanted: HashSet<(&str, &str)> = targets
        .iter()
        .filter(|(_, path)| !path.is_empty())
        .map(|(_, path)| split_parent(path))
        .collect();
    let dirs: Vec<String> = wanted
        .iter()
        .map(|(dir, _)| (*dir).to_owned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut rows = sqlx::query!(
        "SELECT p.path, d.entries FROM archive_dirs d JOIN archive_paths p ON p.id = \
         d.dir_path_id WHERE d.archive_id = $1 AND p.repo_id = $2 AND p.path = ANY($3) ORDER BY \
         p.path, d.chunk_no",
        archive_id,
        repo_id,
        &dirs,
    )
    .fetch(pool);

    let mut found: HashMap<(String, String), bool> = HashMap::new();
    while let Some(row) = rows.try_next().await.map_err(ApiError::Database)? {
        found.extend(
            decode_dir_blob(&row.entries)?
                .into_iter()
                .filter(|entry| wanted.contains(&(row.path.as_str(), entry.name.as_str())))
                .map(|entry| {
                    let directory = EntryKind::from(entry.entry_type.as_str()) == EntryKind::Dir;
                    ((row.path.clone(), entry.name), directory)
                }),
        );
    }

    for (index, path) in targets {
        let resolved = if path.is_empty() {
            LinkResolution::InArchive {
                path,
                directory: true,
            }
        } else {
            let (dir, name) = split_parent(&path);
            found.get(&(dir.to_owned(), name.to_owned())).map_or(
                LinkResolution::NotInArchive,
                |&directory| LinkResolution::InArchive {
                    path: path.clone(),
                    directory,
                },
            )
        };
        if let Some(link) = entries.get_mut(index).and_then(|entry| entry.link.as_mut()) {
            link.resolved = resolved;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_index::{
        query_dir,
        tree::{
            TreeSearch, search_tree,
            tests::{demo_tree, index_archive},
        },
    };

    fn link(path: &str, target: &str) -> ContentEntry {
        ContentEntry {
            entry_type: "l".to_owned(),
            path: path.to_owned(),
            size: 0,
            mtime: "2026-01-10T02:00:00.000000".to_owned(),
            mode: "lrwxrwxrwx".to_owned(),
            link: link_of("l", Some(target.to_owned())),
        }
    }

    async fn resolved_in(pool: &PgPool, repo_id: i64, dir: &str) -> Vec<(String, LinkResolution)> {
        query_dir(pool, repo_id, "daily-1", dir, 100)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|entry| entry.link.map(|link| (entry.path, link.resolved)))
            .collect()
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_listing_says_where_each_link_points(pool: PgPool) {
        let mut tree = demo_tree();
        tree.extend([
            link("var/www/current", "html"),
            link("var/www/index", "html/index.html"),
            link("var/www/hosts", "/etc/hosts"),
            link("var/www/up", "../.."),
            link("var/www/tz", "/usr/share/zoneinfo/UTC"),
            link("var/www/gone", "html/missing.js"),
            ContentEntry {
                link: link_of("l", None),
                ..link("var/www/untold", "")
            },
        ]);
        let repo_id = index_archive(&pool, tree).await;

        let mut links = resolved_in(&pool, repo_id, "var/www").await;
        links.sort_by(|(left, _), (right, _)| left.cmp(right));

        let in_archive = |path: &str, directory: bool| LinkResolution::InArchive {
            path: path.to_owned(),
            directory,
        };
        assert_eq!(
            links,
            [
                (
                    "var/www/current".to_owned(),
                    in_archive("var/www/html", true)
                ),
                ("var/www/gone".to_owned(), LinkResolution::NotInArchive),
                ("var/www/hosts".to_owned(), in_archive("etc/hosts", false)),
                (
                    "var/www/index".to_owned(),
                    in_archive("var/www/html/index.html", false)
                ),
                ("var/www/tz".to_owned(), LinkResolution::NotInArchive),
                ("var/www/untold".to_owned(), LinkResolution::Unknown),
                ("var/www/up".to_owned(), in_archive("", true)),
            ]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_listing_without_links_has_none(pool: PgPool) {
        let repo_id = index_archive(&pool, demo_tree()).await;

        assert_eq!(resolved_in(&pool, repo_id, "etc").await, []);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_search_says_where_the_links_it_finds_point(pool: PgPool) {
        let mut tree = demo_tree();
        tree.extend([
            link("var/www/current", "html"),
            link("srv/app/hosts", "/etc/hosts"),
            link("srv/app/gone", "missing"),
        ]);
        let repo_id = index_archive(&pool, tree).await;
        let everything = TreeSearch {
            needle: Some("/".to_owned()),
            ..TreeSearch::default()
        };

        let (found, _) = search_tree(&pool, repo_id, "daily-1", &everything, 100)
            .await
            .unwrap();
        let mut links: Vec<(String, LinkResolution)> = found
            .into_iter()
            .filter_map(|entry| entry.link.map(|link| (entry.path, link.resolved)))
            .collect();
        links.sort_by(|(left, _), (right, _)| left.cmp(right));

        assert_eq!(
            links,
            [
                ("srv/app/gone".to_owned(), LinkResolution::NotInArchive),
                (
                    "srv/app/hosts".to_owned(),
                    LinkResolution::InArchive {
                        path: "etc/hosts".to_owned(),
                        directory: false,
                    }
                ),
                (
                    "var/www/current".to_owned(),
                    LinkResolution::InArchive {
                        path: "var/www/html".to_owned(),
                        directory: true,
                    }
                ),
            ]
        );
    }

    #[test]
    fn only_a_symlink_is_a_link() {
        let link = |target: Option<&str>| SymlinkTarget {
            target: target.map(str::to_owned),
            resolved: LinkResolution::Unknown,
        };
        assert_eq!(
            link_of("l", Some("releases/1".to_owned())),
            Some(link(Some("releases/1")))
        );
        assert_eq!(link_of("l", Some(String::new())), Some(link(None)));
        assert_eq!(link_of("l", None), Some(link(None)));
        assert_eq!(link_of("h", Some("srv/first-name".to_owned())), None);
        assert_eq!(link_of("-", None), None);
        assert_eq!(link_of("d", None), None);
    }

    #[test]
    fn a_relative_target_starts_in_the_links_directory() {
        assert_eq!(
            resolve_target("srv/app", "releases/2026"),
            "srv/app/releases/2026"
        );
        assert_eq!(
            resolve_target("srv/app", "./shared/.env"),
            "srv/app/shared/.env"
        );
        assert_eq!(resolve_target("", "etc"), "etc");
    }

    #[test]
    fn dot_dot_climbs_out_of_the_links_directory() {
        assert_eq!(
            resolve_target("srv/app/current", "../shared"),
            "srv/app/shared"
        );
        assert_eq!(resolve_target("srv/app", "../../etc/hosts"), "etc/hosts");
    }

    #[test]
    fn an_absolute_target_starts_at_the_archive_root() {
        assert_eq!(
            resolve_target("srv/app", "/usr/share/zoneinfo/UTC"),
            "usr/share/zoneinfo/UTC"
        );
    }

    #[test]
    fn dot_dot_at_the_root_stays_there() {
        assert_eq!(resolve_target("srv", "../../../etc"), "etc");
        assert_eq!(resolve_target("", "/.."), "");
    }

    #[test]
    fn doubled_and_trailing_slashes_are_ignored() {
        assert_eq!(resolve_target("srv", "app//data/"), "srv/app/data");
        assert_eq!(resolve_target("srv", "/"), "");
    }
}
