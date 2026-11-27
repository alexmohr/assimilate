// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The checked values a console command passes on to borg: option values,
//! archives of this repository, and paths inside an archive.

use std::fmt;

const MAX_TEXT_LEN: usize = 1024;
const MAX_PATH_LEN: usize = 4096;
const MAX_ARCHIVE_NAME_LEN: usize = 255;

/// Why a value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ValueError {
    #[error("it is empty")]
    Empty,
    #[error("it is too long")]
    TooLong,
    #[error("it starts with '-' or '@'")]
    LeadingOptionChar,
    #[error("it contains a control character")]
    ControlChar,
    #[error("it is not a whole number")]
    NotANumber,
    #[error("it is not an interval such as 7d")]
    NotAnInterval,
    #[error("it looks like a repository location; the console only addresses this repository")]
    RepoLocation,
    #[error("an archive of this repository is given as ::NAME")]
    NotAnArchiveRef,
    #[error("an archive name cannot contain '/'")]
    SlashInArchiveName,
}

/// The checks every value passes: something borg will not read as an option,
/// an argument file or more than one argument.
fn plain(raw: &str, max_len: usize) -> Result<&str, ValueError> {
    if raw.is_empty() {
        return Err(ValueError::Empty);
    }
    if raw.len() > max_len {
        return Err(ValueError::TooLong);
    }
    if raw.starts_with(['-', '@']) {
        return Err(ValueError::LeadingOptionChar);
    }
    if raw.chars().any(char::is_control) {
        return Err(ValueError::ControlChar);
    }
    Ok(raw)
}

/// Whether `raw` would make borg open a repository other than `BORG_REPO`:
/// a URL, an absolute local path, an `ssh`/`scp`-style `host:path`, or an
/// explicit `REPO::ARCHIVE`.
fn looks_like_repo_location(raw: &str) -> bool {
    raw.starts_with('/')
        || raw.contains("://")
        || raw.contains("::")
        || raw
            .split_once(':')
            .is_some_and(|(before, _)| !before.contains('/'))
}

/// A whole-number option value.
pub(super) fn number(raw: &str) -> Result<u32, ValueError> {
    let raw = plain(raw, MAX_TEXT_LEN)?;
    if !raw.chars().all(|c| c.is_ascii_digit()) {
        return Err(ValueError::NotANumber);
    }
    raw.parse().map_err(|_| ValueError::NotANumber)
}

/// A borg interval such as `7d` or `12H`, returned as given.
pub(super) fn interval(raw: &str) -> Result<String, ValueError> {
    let raw = plain(raw, MAX_TEXT_LEN)?;
    let mut chars = raw.chars();
    let unit = chars.next_back().ok_or(ValueError::NotAnInterval)?;
    let count = chars.as_str();
    if !unit.is_ascii_alphabetic() || count.is_empty() || !count.chars().all(|c| c.is_ascii_digit())
    {
        return Err(ValueError::NotAnInterval);
    }
    Ok(raw.to_owned())
}

/// Free text borg only matches or formats with.
pub(super) fn text(raw: &str) -> Result<String, ValueError> {
    plain(raw, MAX_TEXT_LEN).map(str::to_owned)
}

/// The name of an archive in this repository, without the `::`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArchiveName(String);

impl ArchiveName {
    /// Parses a bare archive name, or one written `::NAME`.
    pub(super) fn parse(raw: &str) -> Result<Self, ValueError> {
        let name = raw.strip_prefix("::").unwrap_or(raw);
        let name = plain(name, MAX_ARCHIVE_NAME_LEN)?;
        // Archive names commonly hold a timestamp, so a colon is not taken
        // for a `host:path`: borg resolves `::NAME` against `BORG_REPO` and
        // reads a bare archive argument as a name, never as a location.
        if name.contains('/') {
            return Err(ValueError::SlashInArchiveName);
        }
        Ok(Self(name.to_owned()))
    }

    /// Parses an archive written `::NAME`, which borg resolves against
    /// `BORG_REPO` and so can only name an archive of this repository.
    pub(super) fn parse_ref(raw: &str) -> Result<Self, ValueError> {
        if !raw.starts_with("::") {
            return Err(if looks_like_repo_location(raw) {
                ValueError::RepoLocation
            } else {
                ValueError::NotAnArchiveRef
            });
        }
        Self::parse(raw)
    }
}

impl fmt::Display for ArchiveName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A path or pattern inside an archive, relative as borg stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArchivePath(String);

impl ArchivePath {
    pub(super) fn parse(raw: &str) -> Result<Self, ValueError> {
        let raw = plain(raw, MAX_PATH_LEN)?;
        if looks_like_repo_location(raw) {
            return Err(ValueError::RepoLocation);
        }
        Ok(Self(raw.to_owned()))
    }
}

impl fmt::Display for ArchivePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_plain_digits() {
        assert_eq!(number("7"), Ok(7));
        assert_eq!(number("+7"), Err(ValueError::NotANumber));
        assert_eq!(number("-1"), Err(ValueError::LeadingOptionChar));
        assert_eq!(number("1e3"), Err(ValueError::NotANumber));
        assert_eq!(number("99999999999"), Err(ValueError::NotANumber));
        assert_eq!(number(""), Err(ValueError::Empty));
    }

    #[test]
    fn intervals_are_a_count_and_a_unit() {
        assert_eq!(interval("7d").as_deref(), Ok("7d"));
        assert_eq!(interval("12H").as_deref(), Ok("12H"));
        assert_eq!(interval("d"), Err(ValueError::NotAnInterval));
        assert_eq!(interval("7"), Err(ValueError::NotAnInterval));
        assert_eq!(interval("7dd"), Err(ValueError::NotAnInterval));
    }

    #[test]
    fn text_is_refused_when_borg_would_read_it_as_an_option_or_argument_file() {
        assert_eq!(text("web-*").as_deref(), Ok("web-*"));
        assert_eq!(text("sh:home/*").as_deref(), Ok("sh:home/*"));
        assert_eq!(text("--rsh"), Err(ValueError::LeadingOptionChar));
        assert_eq!(text("@args.txt"), Err(ValueError::LeadingOptionChar));
        assert_eq!(text("a\nb"), Err(ValueError::ControlChar));
        assert_eq!(text("a\0b"), Err(ValueError::ControlChar));
        assert_eq!(text(&"x".repeat(1025)), Err(ValueError::TooLong));
    }

    #[test]
    fn an_archive_ref_must_name_an_archive_of_this_repository() {
        assert_eq!(
            ArchiveName::parse_ref("::nightly-2026")
                .unwrap()
                .to_string(),
            "nightly-2026"
        );
        for raw in [
            "ssh://evil.example/repo::x",
            "user@host:repo::x",
            "host:repo",
            "/etc",
            "/srv/other-repo::x",
            "file:///tmp/r::x",
        ] {
            assert_eq!(
                ArchiveName::parse_ref(raw),
                Err(ValueError::RepoLocation),
                "{raw}"
            );
        }
        assert_eq!(
            ArchiveName::parse_ref("other-repo"),
            Err(ValueError::NotAnArchiveRef)
        );
        assert_eq!(
            ArchiveName::parse_ref("::a/b"),
            Err(ValueError::SlashInArchiveName)
        );
        assert_eq!(ArchiveName::parse_ref("::"), Err(ValueError::Empty));
        assert_eq!(
            ArchiveName::parse_ref("::-x"),
            Err(ValueError::LeadingOptionChar)
        );
        assert_eq!(
            ArchiveName::parse_ref("::web-01-2026-03-04T02:00:00")
                .unwrap()
                .to_string(),
            "web-01-2026-03-04T02:00:00"
        );
    }

    #[test]
    fn a_bare_archive_name_may_be_written_with_or_without_the_colons() {
        assert_eq!(ArchiveName::parse("b").unwrap().to_string(), "b");
        assert_eq!(ArchiveName::parse("::b").unwrap().to_string(), "b");
        assert_eq!(
            ArchiveName::parse("../b"),
            Err(ValueError::SlashInArchiveName)
        );
    }

    #[test]
    fn an_archive_path_is_relative_and_not_a_repository() {
        assert_eq!(
            ArchivePath::parse("home/user/a:b").unwrap().to_string(),
            "home/user/a:b"
        );
        for raw in ["/etc/passwd", "ssh://h/r", "host:path", "::archive"] {
            assert_eq!(
                ArchivePath::parse(raw),
                Err(ValueError::RepoLocation),
                "{raw}"
            );
        }
        assert_eq!(
            ArchivePath::parse("--rsh"),
            Err(ValueError::LeadingOptionChar)
        );
    }
}
