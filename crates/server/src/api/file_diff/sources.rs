// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Whether a path in an archive lies inside a backup source configured for
//! its host, decided by the server before it asks the host's agent for the
//! file. The agent checks again, and alone knows what is on disk; this keeps
//! the server from asking for anything its own configuration does not back
//! up, whatever an agent does with the request.
//!
//! Borg stores a path without its leading `/`, and a Windows path with its
//! drive letter as the first name (`C/Users/me`), so both sides are compared
//! as lists of names. The server runs on Linux whichever system the host
//! runs, so this does not use the platform's own path parsing.

use std::str::FromStr;

/// One `/`- or `\`-separated part of a path.
enum Part<'a> {
    /// An empty part (from a doubled separator) or `.`: no name at all.
    Current,
    /// `..`.
    Parent,
    /// A file or directory name.
    Name(&'a str),
}

impl<'a> From<&'a str> for Part<'a> {
    fn from(part: &'a str) -> Self {
        match part {
            "" | "." => Self::Current,
            ".." => Self::Parent,
            name => Self::Name(name),
        }
    }
}

/// The names `parts` hold, or `None` if one of them is `..`.
fn names<'a>(parts: impl Iterator<Item = &'a str>) -> Option<Vec<String>> {
    parts
        .map(Part::from)
        .filter_map(|part| match part {
            Part::Current => None,
            Part::Parent => Some(None),
            Part::Name(name) => Some(Some(name.to_owned())),
        })
        .collect()
}

/// A path is not one borg could have stored, or a source is not absolute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a path within a backup source")]
pub(super) struct NotASourcePath;

/// A file's path as borg stores it in an archive, as its names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArchivePath(Vec<String>);

impl FromStr for ArchivePath {
    type Err = NotASourcePath;

    /// Parses an archive path; one that names nothing, or that holds `..`
    /// or a NUL byte, is refused.
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        if path.contains('\0') {
            return Err(NotASourcePath);
        }
        names(path.split('/'))
            .filter(|names| !names.is_empty())
            .map(Self)
            .ok_or(NotASourcePath)
    }
}

/// A backup source as it is configured for a host: an absolute Unix path
/// (`/etc/nginx`) or a Windows path from a drive (`C:\Users\me`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourceRoot {
    /// The source's names, the drive letter first for a Windows path.
    names: Vec<String>,
    /// Whether the first name is a drive letter, which Windows compares
    /// without regard to case.
    on_drive: bool,
}

impl FromStr for SourceRoot {
    type Err = NotASourcePath;

    /// Parses a configured source. A relative source, or one holding `..`
    /// or a NUL byte, is refused: no archive path lies in it.
    fn from_str(source: &str) -> Result<Self, Self::Err> {
        if source.contains('\0') {
            return Err(NotASourcePath);
        }
        let mut chars = source.chars();
        let drive = chars
            .next()
            .filter(char::is_ascii_alphabetic)
            .filter(|_| chars.next() == Some(':'));
        let rest = chars.as_str();
        let parsed = match drive {
            Some(letter) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                names(rest.split(['/', '\\'])).map(|names| Self {
                    names: std::iter::once(letter.to_string()).chain(names).collect(),
                    on_drive: true,
                })
            }
            _ => source
                .strip_prefix('/')
                .and_then(|rest| names(rest.split('/')))
                .map(|names| Self {
                    names,
                    on_drive: false,
                }),
        };
        parsed.ok_or(NotASourcePath)
    }
}

impl SourceRoot {
    /// Whether `path` is this source or lies below it.
    fn contains(&self, path: &ArchivePath) -> bool {
        path.0.len() >= self.names.len()
            && self
                .names
                .iter()
                .zip(&path.0)
                .enumerate()
                .all(|(index, (source, path))| {
                    if index == 0 && self.on_drive {
                        source.eq_ignore_ascii_case(path)
                    } else {
                        source == path
                    }
                })
    }
}

/// Whether `path` lies inside one of `sources`. Sources that are not
/// absolute paths hold nothing.
pub(super) fn within_sources(path: &ArchivePath, sources: &[String]) -> bool {
    sources
        .iter()
        .filter_map(|source| source.parse::<SourceRoot>().ok())
        .any(|source| source.contains(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(path: &str) -> ArchivePath {
        path.parse().unwrap()
    }

    fn sources(sources: &[&str]) -> Vec<String> {
        sources.iter().map(|source| (*source).to_owned()).collect()
    }

    #[test]
    fn a_path_below_a_source_is_within_it() {
        let sources = sources(&["/etc/nginx", "/srv/"]);
        assert!(within_sources(&path("etc/nginx/nginx.conf"), &sources));
        assert!(within_sources(&path("etc/nginx"), &sources));
        assert!(within_sources(&path("srv/app/./config.yml"), &sources));
        assert!(within_sources(&path("/srv//app"), &sources));
    }

    #[test]
    fn a_path_outside_every_source_is_not() {
        let sources = sources(&["/etc/nginx"]);
        assert!(!within_sources(&path("etc/shadow"), &sources));
        assert!(!within_sources(&path("etc/nginx-old/nginx.conf"), &sources));
        assert!(!within_sources(&path("etc"), &sources));
        assert!(!within_sources(&path("etc/nginx/x"), &[]));
    }

    #[test]
    fn the_root_holds_every_path() {
        assert!(within_sources(&path("etc/shadow"), &sources(&["/"])));
    }

    #[test]
    fn a_source_that_is_not_absolute_holds_nothing() {
        let sources = sources(&["etc", "", "/etc/../root", "./etc", "/etc\0"]);
        assert!(!within_sources(&path("etc/hosts"), &sources));
        assert!(!within_sources(&path("root/.ssh/id_ed25519"), &sources));
    }

    #[test]
    fn a_windows_source_matches_its_drive_in_any_case() {
        let sources = sources(&[r"C:\Users\me", "d:/data", "E:"]);
        assert!(within_sources(&path("C/Users/me/notes.txt"), &sources));
        assert!(within_sources(&path("c/Users/me/notes.txt"), &sources));
        assert!(within_sources(&path("D/data/x"), &sources));
        assert!(within_sources(&path("E/anything"), &sources));
        assert!(!within_sources(
            &path("C/Users/someone/notes.txt"),
            &sources
        ));
        assert!(!within_sources(&path("C/users/me/notes.txt"), &sources));
        let no_separator = vec!["C:Users".to_owned()];
        assert!(!within_sources(&path("C:Users/me"), &no_separator));
    }

    #[test]
    fn a_path_that_names_nothing_or_climbs_out_is_refused() {
        for invalid in ["", "/", "./.", "etc/../root", "..", "etc/\0"] {
            assert_eq!(
                invalid.parse::<ArchivePath>(),
                Err(NotASourcePath),
                "{invalid:?}"
            );
        }
    }
}
