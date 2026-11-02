// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Free space on the filesystem that holds the staging directory.
//!
//! A domain's limit bounds what it keeps in the directory once a run is done.
//! It says nothing about the filesystem the images are written to. While a
//! new full image is written, the previous chain is still there, and with no
//! limit at all (the default) nothing bounds the directory. When that
//! filesystem fills during a push-mode backup job, QEMU cannot write the
//! clusters it copies out before the guest overwrites them. Its default
//! response to that is to fail the guest's own write. A guest then reports
//! I/O errors on its root disk (a `TrueNAS` guest reports them against its boot pool)
//! and often goes down with them. All the host sees is a job that ended as
//! `Failed`.
//!
//! So the stager measures the filesystem before a run starts, gives back the
//! previous chain when that is what it takes to fit a new full image, and
//! watches the free space while a job runs, aborting it before the disk is
//! actually full.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

/// Smallest amount of space a run leaves free on the staging filesystem: 2 GiB.
pub(super) const MIN_HEADROOM_BYTES: u64 = 2_147_483_648;

/// The share of the filesystem kept free on top of what a run writes, as a
/// divisor: one fiftieth, or 2 %. On a large filesystem a fixed 2 GiB is gone
/// within a few poll intervals at the speed QEMU writes.
const HEADROOM_DIVISOR: u64 = 50;

/// What the staging filesystem holds and has left, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FsSpace {
    /// Bytes an unprivileged writer can still use.
    pub(super) available: u64,
    /// Size of the filesystem.
    pub(super) total: u64,
}

impl FsSpace {
    /// Space a run must leave free once everything it writes is on disk: at
    /// least `minimum`, or a share of the filesystem when that is larger.
    pub(super) fn headroom(self, minimum: u64) -> u64 {
        minimum.max(self.total.checked_div(HEADROOM_DIVISOR).unwrap_or(0))
    }

    /// Free space below which a running job is aborted. It is half the
    /// headroom: low enough that an estimate a little short does not abort a
    /// healthy job, and still well short of a full disk.
    pub(super) fn abort_below(self, minimum: u64) -> u64 {
        self.headroom(minimum).checked_div(2).unwrap_or(0)
    }
}

/// How the stager finds out how much space the staging filesystem has.
#[derive(Debug, Clone)]
pub(super) enum SpaceProbe {
    /// Ask the filesystem the staging directory is on.
    Filesystem,
    /// Report nothing, so no free-space check applies. Used by tests that are
    /// about something else and must not depend on the space the machine
    /// running them happens to have.
    #[cfg(test)]
    Unmeasured,
    /// Read `<available> <total>` from a file, so a test can make the
    /// filesystem as full as it needs, and change that while a job runs.
    #[cfg(test)]
    Recorded(PathBuf),
}

impl SpaceProbe {
    /// The space on the filesystem holding `dir`, or `None` when it cannot be
    /// measured. A filesystem that cannot be measured is not refused: the
    /// limits still apply, and this check is a safeguard on top of them.
    pub(super) async fn measure(&self, dir: &Path) -> Option<FsSpace> {
        match self {
            Self::Filesystem => filesystem_space(dir).await,
            #[cfg(test)]
            Self::Unmeasured => None,
            #[cfg(test)]
            Self::Recorded(file) => {
                let text = tokio::fs::read_to_string(file).await.ok()?;
                let mut numbers = text.split_whitespace().map(str::parse::<u64>);
                let available = numbers.next()?.ok()?;
                let total = numbers.next()?.ok()?;
                Some(FsSpace { available, total })
            }
        }
    }
}

/// Asks the filesystem holding `dir` for its size and free space.
///
/// `statvfs` can block for as long as a network filesystem takes to answer,
/// so it runs off the executor.
#[cfg(unix)]
async fn filesystem_space(dir: &Path) -> Option<FsSpace> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let stats = nix::sys::statvfs::statvfs(&dir).ok()?;
        let fragment = widen(stats.fragment_size());
        Some(FsSpace {
            available: widen(stats.blocks_available()).saturating_mul(fragment),
            total: widen(stats.blocks()).saturating_mul(fragment),
        })
    })
    .await
    .ok()
    .flatten()
}

/// Only Linux hosts run libvirt, so elsewhere there is nothing to measure.
#[cfg(not(unix))]
async fn filesystem_space(dir: &Path) -> Option<FsSpace> {
    let _ = dir;
    None
}

/// Widens a `statvfs` field to `u64`. The fields are `u32` or `u64`
/// depending on the platform, and a conversion written against one of them
/// is a no-op on the other.
#[cfg(unix)]
fn widen<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_headroom_is_a_share_of_a_large_filesystem_and_a_floor_on_a_small_one() {
        let small = FsSpace {
            available: 0,
            total: 10 * 1024 * 1024 * 1024,
        };
        assert_eq!(small.headroom(MIN_HEADROOM_BYTES), MIN_HEADROOM_BYTES);

        let large = FsSpace {
            available: 0,
            total: 1024 * 1024 * 1024 * 1024,
        };
        assert_eq!(
            large.headroom(MIN_HEADROOM_BYTES),
            large.total / HEADROOM_DIVISOR
        );
        assert_eq!(
            large.abort_below(MIN_HEADROOM_BYTES),
            large.total / HEADROOM_DIVISOR / 2
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_filesystem_of_an_existing_directory_can_be_measured() {
        let dir = tempfile::TempDir::new().unwrap();
        let space = SpaceProbe::Filesystem
            .measure(dir.path())
            .await
            .expect("a temporary directory is on a real filesystem");
        assert!(space.total > 0);
        assert!(space.available <= space.total);
    }

    #[tokio::test]
    async fn a_recorded_probe_reads_what_the_test_wrote() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("space");
        let probe = SpaceProbe::Recorded(file.clone());
        assert_eq!(probe.measure(dir.path()).await, None);

        tokio::fs::write(&file, "100 400\n").await.unwrap();
        assert_eq!(
            probe.measure(dir.path()).await,
            Some(FsSpace {
                available: 100,
                total: 400,
            })
        );
    }
}
