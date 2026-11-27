// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Opening a file below a backup source without following a symbolic link.
//!
//! The source itself is trusted as configured, links in its own path
//! included: an administrator chose it. Everything below it is not, since
//! anyone who can write inside a source can plant a link there. Borg stores
//! such a link as a link, so the file "now" at that path is the link, not
//! what it points to, and the agent refuses it rather than read through it.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use shared::live_file::LiveReadRefusal;
use tokio::fs::File;

/// Why a file below a source was not opened.
#[derive(Debug)]
enum Unopened {
    /// A name on the way to it, or the file itself, is a link.
    ThroughLink,
    /// Anything else.
    Refused(LiveReadRefusal),
}

impl From<std::io::Error> for Unopened {
    fn from(error: std::io::Error) -> Self {
        Self::Refused(error.into())
    }
}

/// Why a path that runs through a link is refused. This only picks the
/// reason: nothing is ever read through the link. A link that leads out of
/// the source is outside the sources; one that stays inside it, or that
/// cannot be resolved, is not a regular file.
async fn through_link(root: &Path, names: &[OsString]) -> LiveReadRefusal {
    let path = names
        .iter()
        .fold(root.to_path_buf(), |path, name| path.join(name));
    match tokio::join!(tokio::fs::canonicalize(path), tokio::fs::canonicalize(root)) {
        (Ok(target), Ok(root)) if !target.starts_with(&root) => LiveReadRefusal::NotInSources,
        _ => LiveReadRefusal::NotRegularFile,
    }
}

/// The source `root` names itself, as its parent directory and its own name.
fn split_source(root: &Path) -> Result<(PathBuf, Vec<OsString>), LiveReadRefusal> {
    root.parent()
        .zip(root.file_name())
        .map(|(parent, name)| (parent.to_path_buf(), vec![name.to_owned()]))
        .ok_or(LiveReadRefusal::NotRegularFile)
}

#[cfg(unix)]
mod platform {
    use std::{
        ffi::{OsStr, OsString},
        os::{
            fd::{AsRawFd as _, FromRawFd as _, OwnedFd},
            unix::fs::OpenOptionsExt as _,
        },
        path::{Path, PathBuf},
    };

    use nix::{
        errno::Errno,
        fcntl::{AtFlags, OFlag, openat},
        sys::stat::{Mode, SFlag, fstatat},
    };
    use shared::live_file::LiveReadRefusal;
    use tokio::fs::File;

    use super::Unopened;

    /// Opens the regular file `names` lead to from the directory `root`.
    /// Each name is opened relative to the directory before it with
    /// `O_NOFOLLOW`, and the walk holds each directory it passed open, so
    /// no link below `root` is followed, and renaming or replacing a
    /// directory on the way meanwhile cannot lead outside `root` either.
    pub(super) async fn beneath(root: PathBuf, names: Vec<OsString>) -> Result<File, Unopened> {
        tokio::task::spawn_blocking(move || walk(&root, &names))
            .await
            .map_err(|_| Unopened::Refused(LiveReadRefusal::Unreadable))?
            .map(File::from_std)
    }

    fn walk(root: &Path, names: &[OsString]) -> Result<std::fs::File, Unopened> {
        let Some((name, dirs)) = names.split_last() else {
            return Err(Unopened::Refused(LiveReadRefusal::NotRegularFile));
        };
        let dir = dirs.iter().try_fold(open_root(root)?, |dir, name| {
            open_at(&dir, name, OFlag::O_DIRECTORY).map_err(unopened)
        })?;
        // Checked before opening, so a pipe or a device is never opened.
        let stat = fstatat(
            Some(dir.as_raw_fd()),
            name.as_os_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        )
        .map_err(unopened)?;
        match SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT {
            SFlag::S_IFREG => {}
            SFlag::S_IFLNK => return Err(Unopened::ThroughLink),
            _ => return Err(Unopened::Refused(LiveReadRefusal::NotRegularFile)),
        }
        // Not blocking: should the file have been replaced by a pipe since
        // the check, opening it does not wait for a writer.
        let file = open_at(&dir, name, OFlag::O_NONBLOCK | OFlag::O_NOCTTY).map_err(unopened)?;
        Ok(std::fs::File::from(file))
    }

    /// Opens the source directory itself, following links in its own path.
    fn open_root(root: &Path) -> Result<OwnedFd, Unopened> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(OFlag::O_DIRECTORY.bits())
            .open(root)?;
        Ok(OwnedFd::from(file))
    }

    /// Opens `name` inside `dir` for reading, refusing a link.
    fn open_at(dir: &OwnedFd, name: &OsStr, flags: OFlag) -> Result<OwnedFd, Errno> {
        let fd = openat(
            Some(dir.as_raw_fd()),
            name,
            flags | OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        // SAFETY: `openat` has just returned `fd` as a new descriptor, which
        // nothing else owns or closes.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// Why opening one name on the way failed. `O_NOFOLLOW` reports a link
    /// as `ELOOP` (`EMLINK` on FreeBSD), and a name on the way that is not a
    /// directory, a link to one included, as `ENOTDIR`.
    fn unopened(errno: Errno) -> Unopened {
        match errno {
            Errno::ELOOP | Errno::EMLINK | Errno::ENOTDIR => Unopened::ThroughLink,
            other => std::io::Error::from(other).into(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::OsString, path::PathBuf};

    use shared::live_file::LiveReadRefusal;
    use tokio::fs::File;

    use super::Unopened;

    /// Opens a link or junction itself instead of what it points to.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    /// Opens the regular file `names` lead to from the directory `root`,
    /// refusing it when any name on the way is a link or a junction. Windows
    /// has no `openat`, so the names are checked one after another before
    /// the file is opened, and the file itself is opened without following
    /// a link.
    pub(super) async fn beneath(root: PathBuf, names: Vec<OsString>) -> Result<File, Unopened> {
        let mut path = root;
        for name in &names {
            path.push(name);
            if tokio::fs::symlink_metadata(&path)
                .await?
                .file_type()
                .is_symlink()
            {
                return Err(Unopened::ThroughLink);
            }
        }
        // Checked before opening as well as after: Windows refuses to open
        // a directory as a file at all, which would read as unreadable.
        if names.is_empty() || !tokio::fs::symlink_metadata(&path).await?.is_file() {
            return Err(Unopened::Refused(LiveReadRefusal::NotRegularFile));
        }
        Ok(tokio::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .await?)
    }
}

/// Opens the regular file `names` lead to from the source `root`, without
/// following a link anywhere below `root`. With no names, `root` is the
/// file asked for, and is opened from its parent directory.
pub(super) async fn beneath(root: &Path, names: &[OsString]) -> Result<File, LiveReadRefusal> {
    let (root, names) = if names.is_empty() {
        split_source(root)?
    } else {
        (root.to_path_buf(), names.to_vec())
    };
    match platform::beneath(root.clone(), names.clone()).await {
        Ok(file) => Ok(file),
        Err(Unopened::Refused(reason)) => Err(reason),
        Err(Unopened::ThroughLink) => Err(through_link(&root, &names).await),
    }
}

/// A resolved source as archive paths spell it. Windows resolves a path to
/// its verbatim form (`\\?\C:\Users`), which an archive path never takes.
#[cfg(windows)]
fn plain(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let Prefix::VerbatimDisk(drive) = prefix.kind() else {
        return path;
    };
    PathBuf::from(format!(r"{}:\", char::from(drive))).join(components.as_path())
}

/// A resolved source as archive paths spell it: elsewhere, as resolved.
#[cfg(not(windows))]
fn plain(path: PathBuf) -> PathBuf {
    path
}

/// The source `path` lies in, and the names that lead from it to `path`. A
/// source matches as configured and as it resolves, so a host whose source
/// runs through a link (`/var` on macOS, `\\?\C:\` on Windows) matches
/// either spelling. The match is on the names alone: nothing below the
/// source is looked at.
pub(super) async fn within_sources(
    sources: &[PathBuf],
    path: &Path,
) -> Option<(PathBuf, Vec<OsString>)> {
    let mut roots = Vec::with_capacity(sources.len().saturating_mul(2));
    for source in sources {
        roots.push(source.clone());
        if let Ok(resolved) = tokio::fs::canonicalize(source).await {
            roots.push(plain(resolved));
        }
    }
    roots.into_iter().find_map(|root| {
        let names = path
            .strip_prefix(&root)
            .ok()?
            .components()
            .map(|part| match part {
                std::path::Component::Normal(name) => Some(name.to_owned()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some((root, names))
    })
}
