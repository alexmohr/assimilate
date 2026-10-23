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
//!
//! The server does not know which system the agent runs, so a destination
//! may be a POSIX path (`/srv/restore`) or a Windows drive path
//! (`C:\restore`). The agent then refuses one its own system cannot root:
//! a drive path on Linux or macOS.

use std::fmt;

/// Longest destination accepted, in bytes: Linux's `PATH_MAX`.
const MAX_DESTINATION_LEN: usize = 4096;

/// A checked restore destination: an absolute directory path, POSIX or a
/// Windows drive path, without `.` or `..` components, control characters or
/// repeated separators.
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
    #[error(
        "the destination must be an absolute path, starting with '/' or with a drive letter like \
         'C:\\'"
    )]
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

    /// Whether this system can root the destination: a POSIX path anywhere,
    /// a drive path only on Windows.
    #[must_use]
    pub fn is_rooted_here(&self) -> bool {
        std::path::Path::new(&self.0).has_root()
    }
}

/// The drive letter, upper-cased, and the rest of a Windows drive path
/// (`C:\\restore` or `C:/restore`), or `None` for anything else.
fn drive_root(raw: &str) -> Option<(char, &str)> {
    let mut chars = raw.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    chars.next().filter(|c| *c == ':')?;
    chars.next().filter(|c| matches!(c, '\\' | '/'))?;
    Some((drive.to_ascii_uppercase(), raw.get(3..)?))
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
        let (root, segments) = if let Some(rest) = raw.strip_prefix('/') {
            ("/".to_owned(), rest.split('/').collect::<Vec<_>>())
        } else if let Some((drive, rest)) = drive_root(raw) {
            (format!("{drive}:\\"), rest.split(['/', '\\']).collect())
        } else {
            return Err(InvalidDestination::NotAbsolute);
        };
        let segments: Vec<&str> = segments.into_iter().filter(|s| !s.is_empty()).collect();
        if segments.iter().any(|s| matches!(*s, "." | "..")) {
            return Err(InvalidDestination::DotComponent);
        }
        let separator = if root.starts_with('/') { "/" } else { "\\" };
        Ok(Self(format!("{root}{}", segments.join(separator))))
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
    fn a_backslash_in_a_posix_path_is_part_of_a_name() {
        assert_eq!(
            RestoreDestination::try_from("/srv/odd\\name").map(|d| d.to_string()),
            Ok("/srv/odd\\name".to_owned())
        );
    }

    #[test]
    fn a_windows_drive_path_is_accepted_and_normalised() {
        assert_eq!(
            RestoreDestination::try_from("c:\\Restore\\\\Files\\").map(|d| d.to_string()),
            Ok("C:\\Restore\\Files".to_owned())
        );
        assert_eq!(
            RestoreDestination::try_from("D:/restore/files").map(|d| d.to_string()),
            Ok("D:\\restore\\files".to_owned())
        );
        assert_eq!(
            RestoreDestination::try_from("E:\\").map(|d| d.to_string()),
            Ok("E:\\".to_owned())
        );
    }

    #[test]
    fn unsafe_windows_paths_are_refused() {
        assert_eq!(
            RestoreDestination::try_from("C:relative"),
            Err(InvalidDestination::NotAbsolute)
        );
        assert_eq!(
            RestoreDestination::try_from("C:\\restore\\..\\Windows"),
            Err(InvalidDestination::DotComponent)
        );
        assert_eq!(
            RestoreDestination::try_from("C:/restore/../Windows"),
            Err(InvalidDestination::DotComponent)
        );
        assert_eq!(
            RestoreDestination::try_from("1:\\restore"),
            Err(InvalidDestination::NotAbsolute)
        );
        assert_eq!(
            RestoreDestination::try_from("\\\\server\\share"),
            Err(InvalidDestination::NotAbsolute)
        );
    }

    #[test]
    fn a_path_is_rooted_where_its_system_can_root_it() {
        let posix = RestoreDestination::try_from("/srv/restore").unwrap();
        let drive = RestoreDestination::try_from("C:\\restore").unwrap();

        assert!(posix.is_rooted_here());
        assert!(RestoreDestination::original().is_rooted_here());
        assert_eq!(drive.is_rooted_here(), cfg!(windows));
    }

    #[test]
    fn names_that_only_start_with_dots_are_ordinary_directories() {
        assert_eq!(
            RestoreDestination::try_from("/srv/..restore/.cache").map(|d| d.to_string()),
            Ok("/srv/..restore/.cache".to_owned())
        );
    }
}
