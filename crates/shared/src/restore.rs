// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Where on a host a restore writes its files.
//!
//! `borg extract` recreates every selected archive path, with its full
//! directory structure, underneath the directory it runs in. A restore to the
//! original locations therefore runs in `/`, and a restore to any other
//! destination runs in that directory. The destination is the one input of a
//! restore that names a place on the host's own filesystem, so the server
//! checks it before recording the restore and the agent checks it again
//! before running borg there.

use std::fmt;

/// Longest destination accepted, in bytes: Linux's `PATH_MAX`.
const MAX_DESTINATION_LEN: usize = 4096;

/// A checked restore destination: an absolute directory path without `.` or
/// `..` components, control characters or repeated slashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreDestination(String);

/// Why a restore destination was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidDestination {
    /// No destination was given where one was expected.
    #[error("the destination must not be empty")]
    Empty,
    /// The destination is relative, so it would depend on where the agent
    /// happens to run.
    #[error("the destination must be an absolute path starting with '/'")]
    NotAbsolute,
    /// The destination has a `.` or `..` component.
    #[error("the destination must not contain '.' or '..' components")]
    DotComponent,
    /// The destination holds a NUL byte or another control character.
    #[error("the destination must not contain control characters")]
    ControlCharacter,
    /// The destination is longer than any path the host could create.
    #[error("the destination must be at most {MAX_DESTINATION_LEN} bytes long")]
    TooLong,
}

impl RestoreDestination {
    /// The original locations: borg recreates each path where it was backed
    /// up from.
    #[must_use]
    pub fn original() -> Self {
        Self("/".to_owned())
    }

    /// Whether this restores to the original locations.
    #[must_use]
    pub fn is_original(&self) -> bool {
        self.0.len() == 1
    }

    /// The directory `borg extract` runs in.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where `archive_path`, as borg stores it (no leading `/`), ends up on
    /// the host once restored here.
    #[must_use]
    pub fn restored_location(&self, archive_path: &str) -> String {
        let archive_path = archive_path.trim_start_matches('/');
        if self.is_original() {
            format!("/{archive_path}")
        } else if archive_path.is_empty() {
            self.0.clone()
        } else {
            format!("{}/{archive_path}", self.0)
        }
    }
}

impl TryFrom<&str> for RestoreDestination {
    type Error = InvalidDestination;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if raw.is_empty() {
            return Err(InvalidDestination::Empty);
        }
        if raw.len() > MAX_DESTINATION_LEN {
            return Err(InvalidDestination::TooLong);
        }
        if raw.chars().any(char::is_control) {
            return Err(InvalidDestination::ControlCharacter);
        }
        let Some(rest) = raw.strip_prefix('/') else {
            return Err(InvalidDestination::NotAbsolute);
        };
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.iter().any(|s| matches!(*s, "." | "..")) {
            return Err(InvalidDestination::DotComponent);
        }
        Ok(Self(format!("/{}", segments.join("/"))))
    }
}

impl fmt::Display for RestoreDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_directory_is_accepted_and_normalised() {
        assert_eq!(
            RestoreDestination::try_from("/srv//restore/").map(|d| d.to_string()),
            Ok("/srv/restore".to_owned())
        );
        assert_eq!(
            RestoreDestination::try_from("/").map(|d| d.is_original()),
            Ok(true)
        );
        assert_eq!(
            RestoreDestination::try_from("//").map(|d| d.is_original()),
            Ok(true)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/my restore.d").map(|d| d.to_string()),
            Ok("/srv/my restore.d".to_owned())
        );
    }

    #[test]
    fn unsafe_destinations_are_refused() {
        assert_eq!(
            RestoreDestination::try_from(""),
            Err(InvalidDestination::Empty)
        );
        assert_eq!(
            RestoreDestination::try_from("srv/restore"),
            Err(InvalidDestination::NotAbsolute)
        );
        assert_eq!(
            RestoreDestination::try_from("-rf"),
            Err(InvalidDestination::NotAbsolute)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/../etc"),
            Err(InvalidDestination::DotComponent)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/./restore"),
            Err(InvalidDestination::DotComponent)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/.."),
            Err(InvalidDestination::DotComponent)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/\0restore"),
            Err(InvalidDestination::ControlCharacter)
        );
        assert_eq!(
            RestoreDestination::try_from("/srv/re\nstore"),
            Err(InvalidDestination::ControlCharacter)
        );
        assert_eq!(
            RestoreDestination::try_from(format!("/{}", "a".repeat(4096)).as_str()),
            Err(InvalidDestination::TooLong)
        );
    }

    #[test]
    fn names_that_only_start_with_dots_are_ordinary_directories() {
        assert_eq!(
            RestoreDestination::try_from("/srv/..restore/.cache").map(|d| d.to_string()),
            Ok("/srv/..restore/.cache".to_owned())
        );
    }

    #[test]
    fn a_path_keeps_its_archive_structure_under_the_destination() {
        let destination = RestoreDestination::try_from("/srv/restore").unwrap();

        assert_eq!(
            destination.restored_location("etc/nginx/nginx.conf"),
            "/srv/restore/etc/nginx/nginx.conf"
        );
        assert_eq!(
            destination.restored_location("/var/www"),
            "/srv/restore/var/www"
        );
        assert_eq!(destination.restored_location(""), "/srv/restore");
    }

    #[test]
    fn the_original_location_is_the_path_itself() {
        let original = RestoreDestination::original();

        assert_eq!(original.restored_location("etc/hosts"), "/etc/hosts");
        assert_eq!(original.restored_location(""), "/");
    }
}
