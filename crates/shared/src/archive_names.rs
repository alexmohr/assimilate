// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! How backup archives are named, and the patterns that find them again.
//!
//! The agent names an archive when it writes it and prunes with these
//! patterns; the server prunes append-only repositories with the same ones.
//! Keeping both ends on one definition means a change to the naming cannot
//! leave one of them matching the wrong archives.

/// The name of an archive written by `hostname` at `timestamp`.
///
/// With a schedule it is `<host>-s<schedule id>-<timestamp>`, so a schedule's
/// retention can match its own archives only. Without one it keeps the legacy
/// `<host>-<timestamp>` shape.
#[must_use]
pub fn archive_name(
    hostname: &str,
    schedule_id: Option<i64>,
    timestamp: impl std::fmt::Display,
) -> String {
    schedule_id.map_or_else(
        || format!("{hostname}-{timestamp}"),
        |id| format!("{hostname}-s{id}-{timestamp}"),
    )
}

/// Matches the archives a schedule wrote under `hostname`.
#[must_use]
pub fn schedule_glob(hostname: &str, schedule_id: i64) -> String {
    format!("*{hostname}-s{schedule_id}-*")
}

/// Matches the archives named before they carried a schedule id
/// (`<host>-<timestamp>`). Spelling out the date keeps the pattern from
/// matching the newer names.
#[must_use]
pub fn legacy_glob(hostname: &str) -> String {
    format!("*{hostname}-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_name_carries_the_schedule_when_there_is_one() {
        assert_eq!(
            archive_name("web-1", Some(12), "2026-11-03T02:00:00"),
            "web-1-s12-2026-11-03T02:00:00"
        );
        assert_eq!(
            archive_name("web-1", None, "2026-11-03T02:00:00"),
            "web-1-2026-11-03T02:00:00"
        );
    }

    #[test]
    fn a_schedule_prunes_only_the_archives_it_wrote() {
        assert_eq!(schedule_glob("web-1", 12), "*web-1-s12-*");
        // Not another schedule's, even one whose id starts with the same digits.
        assert!(!schedule_glob("web-1", 1).contains("s12"));
    }

    #[test]
    fn archives_without_a_schedule_id_are_matched_by_their_date() {
        assert_eq!(
            legacy_glob("web-1"),
            "*web-1-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*"
        );
    }
}
