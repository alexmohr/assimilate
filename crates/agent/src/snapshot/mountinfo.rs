// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The host's mount table, read from `/proc/self/mountinfo`: which
//! filesystem each backup source sits on, and which mounts lie below it.

use std::path::{Path, PathBuf};

/// Where the kernel lists the calling process's mounts.
pub const MOUNTINFO: &str = "/proc/self/mountinfo";

/// One mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// The directory inside the filesystem this mount shows: `/` for a
    /// whole filesystem, a subdirectory for a bind mount of part of one, or
    /// for btrfs the mounted subvolume.
    pub root: PathBuf,
    /// Where it is mounted.
    pub point: PathBuf,
    /// The filesystem type, such as `zfs`, `btrfs` or `ext4`.
    pub fstype: String,
    /// What is mounted: a ZFS dataset name, or a block device.
    pub source: String,
}

/// Every mount of the host, in the kernel's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountTable {
    mounts: Vec<Mount>,
}

impl MountTable {
    /// Parses the text of a `mountinfo` file. A line that is not a mount is
    /// skipped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self {
            mounts: text.lines().filter_map(parse_line).collect(),
        }
    }

    /// Reads the calling process's mount table.
    ///
    /// # Errors
    ///
    /// Returns an error when the table cannot be read.
    pub async fn read() -> std::io::Result<Self> {
        Ok(Self::parse(&tokio::fs::read_to_string(MOUNTINFO).await?))
    }

    /// The mount `path` lies on: the one with the longest mount point that
    /// holds it. Of several mounts on the same point the last one wins, as it
    /// hides the others.
    #[must_use]
    pub fn mount_of(&self, path: &Path) -> Option<&Mount> {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, mount)| path.starts_with(&mount.point))
            .max_by_key(|(index, mount)| (mount.point.components().count(), *index))
            .map(|(_, mount)| mount)
    }

    /// A mount of the whole of `source`, rather than of one of its
    /// directories. Of several, the last one wins, as in [`Self::mount_of`].
    #[must_use]
    pub fn whole_mount_of(&self, source: &str) -> Option<&Mount> {
        self.mounts
            .iter()
            .rfind(|mount| mount.source == source && mount.root == Path::new("/"))
    }

    /// The mount points strictly below `dir`, outermost first.
    #[must_use]
    pub fn mount_points_below(&self, dir: &Path) -> Vec<PathBuf> {
        let mut below: Vec<PathBuf> = self
            .mounts
            .iter()
            .map(|mount| mount.point.clone())
            .filter(|point| point != dir && point.starts_with(dir))
            .collect();
        below.sort_by_key(|point| point.components().count());
        // A point mounted on more than once is listed once. Its repeats need
        // not be next to each other, even sorted by depth.
        let mut unique: Vec<PathBuf> = Vec::with_capacity(below.len());
        for point in below {
            if !unique.contains(&point) {
                unique.push(point);
            }
        }
        unique
    }
}

/// Reads one `mountinfo` line:
/// `id parent major:minor root mount_point options [optional...] - fstype source super_options`.
fn parse_line(line: &str) -> Option<Mount> {
    let (before, after) = line.split_once(" - ")?;
    let mut fields = before.split(' ');
    let root = fields.nth(3)?;
    let mount_point = fields.next()?;
    let mut after = after.split(' ');
    let fstype = after.next()?;
    let source = after.next()?;
    Some(Mount {
        root: PathBuf::from(unescape(root)),
        point: PathBuf::from(unescape(mount_point)),
        fstype: fstype.to_owned(),
        source: unescape(source),
    })
}

/// Undoes the octal escapes the kernel writes for a space, tab, newline or
/// backslash in a path, such as `\040` for a space.
fn unescape(field: &str) -> String {
    let mut out = Vec::with_capacity(field.len());
    let mut rest = field.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        let octal = tail
            .get(..3)
            .filter(|_| byte == b'\\')
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        if let Some(decoded) = octal {
            out.push(decoded);
            rest = tail.get(3..).unwrap_or_default();
        } else {
            out.push(byte);
            rest = tail;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
22 1 0:21 / / rw,relatime shared:1 - btrfs /dev/sda2 rw,subvol=/@
23 22 0:22 / /proc rw,nosuid shared:5 - proc proc rw
30 22 0:21 /@home /home rw,relatime shared:2 - btrfs /dev/sda2 rw,subvol=/@home
41 22 0:40 / /srv/tank rw shared:9 - zfs tank/data rw,xattr
42 41 0:41 / /srv/tank/media rw shared:10 - zfs tank/data/media rw,xattr
50 22 253:1 / /var/lib/postgresql rw shared:11 - ext4 /dev/mapper/vg0-pg rw
51 22 253:2 / /mnt/with\\040space rw - xfs /dev/mapper/vg0-x rw
52 22 0:21 / /proc rw shared:5 - proc proc rw
garbage line without a separator
";

    #[test]
    fn the_last_whole_mount_of_a_source_wins() {
        let table = MountTable::parse(
            "1 0 0:3 / /a rw - zfs tank rw\n2 0 0:3 /sub /b rw - zfs tank rw\n3 0 0:3 / /c rw - \
             zfs tank rw\n",
        );
        assert_eq!(
            table
                .whole_mount_of("tank")
                .map(|mount| mount.point.clone()),
            Some(PathBuf::from("/c"))
        );
        assert_eq!(table.whole_mount_of("pool"), None);
    }

    #[test]
    fn a_path_lies_on_the_innermost_mount_holding_it() {
        let table = MountTable::parse(TABLE);
        let mount = |path: &str| table.mount_of(Path::new(path)).unwrap().clone();
        assert_eq!(mount("/etc/fstab").point, Path::new("/"));
        assert_eq!(mount("/etc/fstab").root, Path::new("/"));
        let home = mount("/home/alice");
        assert_eq!(
            (
                home.point.as_path(),
                home.root.as_path(),
                home.fstype.as_str()
            ),
            (Path::new("/home"), Path::new("/@home"), "btrfs")
        );
        assert_eq!(mount("/srv/tank/media/x").source, "tank/data/media");
        assert_eq!(mount("/srv/tank/docs").source, "tank/data");
        assert_eq!(mount("/var/lib/postgresql/16").source, "/dev/mapper/vg0-pg");
        assert_eq!(
            mount("/homework").point,
            Path::new("/"),
            "a mount point holds only the paths below it, not every path it is a prefix of"
        );
    }

    #[test]
    fn the_last_mount_on_a_point_hides_the_earlier_ones() {
        let table = MountTable::parse(TABLE);
        assert_eq!(table.mount_of(Path::new("/proc/1")).unwrap().fstype, "proc");
        let doubled = MountTable::parse(
            "1 0 0:1 / /data rw - ext4 /dev/a rw\n2 0 0:2 / /data rw - xfs /dev/b rw\n",
        );
        assert_eq!(
            doubled.mount_of(Path::new("/data/x")).unwrap().source,
            "/dev/b"
        );
    }

    #[test]
    fn escaped_paths_are_read_back() {
        let table = MountTable::parse(TABLE);
        let spaced = table.mount_of(Path::new("/mnt/with space/f")).unwrap();
        assert_eq!(spaced.point, Path::new("/mnt/with space"));
        assert_eq!(unescape("a\\134b\\011c\\0"), "a\\b\tc\\0");
    }

    #[test]
    fn the_mounts_below_a_directory_are_listed_outermost_first() {
        let table = MountTable::parse(TABLE);
        assert_eq!(
            table.mount_points_below(Path::new("/srv")),
            [PathBuf::from("/srv/tank"), PathBuf::from("/srv/tank/media")]
        );
        assert_eq!(
            table.mount_points_below(Path::new("/srv/tank/media")).len(),
            0
        );
        assert!(
            table
                .mount_points_below(Path::new("/"))
                .contains(&PathBuf::from("/proc"))
        );
    }

    #[test]
    fn a_point_mounted_on_twice_is_listed_once() {
        let table = MountTable::parse(
            "1 0 0:1 / /srv/a rw - ext4 /dev/a rw\n2 0 0:2 / /srv/b rw - ext4 /dev/b rw\n3 0 0:3 \
             / /srv/a rw - xfs /dev/c rw\n",
        );
        assert_eq!(
            table.mount_points_below(Path::new("/srv")),
            [PathBuf::from("/srv/a"), PathBuf::from("/srv/b")]
        );
    }

    #[tokio::test]
    async fn the_running_host_has_a_mount_table() {
        if cfg!(target_os = "linux") {
            let table = MountTable::read().await.unwrap();
            assert!(table.mount_of(Path::new("/")).is_some());
        } else {
            assert!(MountTable::read().await.is_err());
        }
    }
}
