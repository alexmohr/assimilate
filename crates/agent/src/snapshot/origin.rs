// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a snapshot is taken of: the ZFS dataset, btrfs subvolume or LVM
//! logical volume a backup source, or a filesystem mounted below one,
//! lives on.

use std::path::{Path, PathBuf};

use super::{
    mountinfo::{Mount, MountTable},
    tools::{self, Tools},
};

/// The inode of every btrfs subvolume's root directory
/// (`BTRFS_FIRST_FREE_OBJECTID`).
const BTRFS_SUBVOLUME_ROOT_INODE: u64 = 256;

/// Something a snapshot can be taken of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A ZFS dataset, mounted at `mount_point`. `root` is the directory of
    /// the dataset that mount shows, and `dataset_point` where the whole
    /// dataset is mounted, the only place ZFS shows its snapshots.
    Zfs {
        dataset: String,
        mount_point: PathBuf,
        dataset_point: PathBuf,
        root: PathBuf,
    },
    /// A btrfs subvolume, whose root directory is `subvolume`.
    Btrfs { subvolume: PathBuf },
    /// An LVM logical volume, mounted at `mount_point`. `root` is the
    /// directory of the filesystem that mount shows.
    Lvm {
        vg: String,
        lv: String,
        /// A thin volume, whose snapshot takes no size.
        thin: bool,
        fstype: String,
        mount_point: PathBuf,
        root: PathBuf,
    },
}

impl Origin {
    /// The directory the snapshot stands in for during the backup.
    #[must_use]
    pub fn covers(&self) -> &Path {
        match self {
            Self::Zfs { mount_point, .. } | Self::Lvm { mount_point, .. } => mount_point,
            Self::Btrfs { subvolume } => subvolume,
        }
    }

    /// What the snapshot is of, in words, for the run's timeline.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Zfs {
                dataset,
                mount_point,
                ..
            } => format!("ZFS dataset {dataset} ({})", mount_point.display()),
            Self::Btrfs { subvolume } => format!("btrfs subvolume {}", subvolume.display()),
            Self::Lvm {
                vg,
                lv,
                mount_point,
                ..
            } => format!("LVM volume {vg}/{lv} ({})", mount_point.display()),
        }
    }
}

/// A disk filesystem at or below a backup source that cannot be
/// snapshotted, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsnapshottable {
    pub path: PathBuf,
    pub reason: String,
}

/// What the backup sources are snapshotted from.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Origins {
    /// Each origin once, outermost first.
    pub origins: Vec<Origin>,
    /// The filesystems holding data that cannot be snapshotted.
    pub unsnapshottable: Vec<Unsnapshottable>,
}

/// Finds what each of `sources` is snapshotted from: the filesystem it
/// lies on and every disk filesystem mounted below it.
pub async fn find(sources: &[String], table: &MountTable, tools: &Tools) -> Origins {
    let mut found = Origins::default();
    for source in sources {
        let source = Path::new(source);
        let source = tokio::fs::canonicalize(source)
            .await
            .unwrap_or_else(|_| source.to_path_buf());
        // A relative or empty source lies on no mount at all, so it would
        // otherwise be read live without a word, even when snapshots are
        // required.
        if !source.is_absolute() {
            let unsnapshottable = Unsnapshottable {
                reason: format!("{} is not an absolute path", source.display()),
                path: source,
            };
            if !found.unsnapshottable.contains(&unsnapshottable) {
                found.unsnapshottable.push(unsnapshottable);
            }
            continue;
        }
        let mut mounts: Vec<(PathBuf, Mount)> = Vec::new();
        if let Some(mount) = table.mount_of(&source) {
            mounts.push((source.clone(), mount.clone()));
        }
        for point in table.mount_points_below(&source) {
            if let Some(mount) = table.mount_of(&point) {
                mounts.push((point.clone(), mount.clone()));
            }
        }
        for (path, mount) in mounts {
            match origins_of(&path, &mount, table, tools).await {
                Ok(origins) => {
                    for origin in origins {
                        if !found.origins.contains(&origin) {
                            found.origins.push(origin);
                        }
                    }
                }
                Err(Skip::Pseudo) => {}
                Err(Skip::Unsupported(reason)) => {
                    let unsnapshottable = Unsnapshottable { path, reason };
                    if !found.unsnapshottable.contains(&unsnapshottable) {
                        found.unsnapshottable.push(unsnapshottable);
                    }
                }
            }
        }
    }
    found
        .origins
        .sort_by_key(|origin| origin.covers().components().count());
    found
}

/// Why a mount is left as it is.
enum Skip {
    /// A filesystem that holds no data of its own on disk, such as `/proc`
    /// or a `tmpfs` (see [`NoDataFs`]), read live without a word.
    Pseudo,
    /// A disk filesystem that cannot be snapshotted.
    Unsupported(String),
}

/// The origins `path`, on `mount`, is snapshotted from.
async fn origins_of(
    path: &Path,
    mount: &Mount,
    table: &MountTable,
    tools: &Tools,
) -> Result<Vec<Origin>, Skip> {
    match mount.fstype.as_str().parse::<DiskFs>() {
        Ok(DiskFs::Zfs) => zfs_origin(mount, table)
            .map(|origin| vec![origin])
            .map_err(Skip::Unsupported),
        Ok(DiskFs::Btrfs) => btrfs_origins(path, mount, tools, BTRFS_SUBVOLUME_ROOT_INODE)
            .await
            .map_err(Skip::Unsupported),
        Err(_) if mount.fstype.parse::<NoDataFs>().is_ok() => Err(Skip::Pseudo),
        Err(_) if mount.source.starts_with("/dev/") => lvm_origin(mount, tools)
            .await
            .map(|origin| vec![origin])
            .map_err(Skip::Unsupported),
        // Data that is not on a local block device, such as an NFS share or
        // a FUSE filesystem, has no snapshot to take here.
        Err(_) => Err(Skip::Unsupported(not_snapshottable(mount, ""))),
    }
}

/// Why `mount` cannot be snapshotted, followed by `detail`.
fn not_snapshottable(mount: &Mount, detail: &str) -> String {
    format!(
        "{} ({} on {}) is not on ZFS, btrfs or an LVM volume{detail}",
        mount.point.display(),
        mount.fstype,
        mount.source
    )
}

/// The filesystems that keep no data of their own on disk: the kernel's
/// views of itself, and memory. Any other filesystem holds data a backup
/// needs, so one that cannot be snapshotted is named as such.
#[derive(Debug, Clone, Copy, strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
enum NoDataFs {
    Proc,
    Sysfs,
    Tmpfs,
    Ramfs,
    Devtmpfs,
    Devpts,
    Mqueue,
    Hugetlbfs,
    Cgroup,
    Cgroup2,
    Securityfs,
    Debugfs,
    Tracefs,
    Configfs,
    Pstore,
    Bpf,
    Efivarfs,
    Fusectl,
    Nsfs,
    Autofs,
    Selinuxfs,
    #[strum(serialize = "binfmt_misc")]
    BinfmtMisc,
    #[strum(serialize = "rpc_pipefs")]
    RpcPipefs,
}

/// The options an LVM snapshot of a `fstype` filesystem is mounted with:
/// read-only, and for XFS also without its UUID check, which refuses a
/// second filesystem with the origin's UUID, and without replaying the log,
/// which a read-only mount must not write.
#[must_use]
pub fn mount_options(fstype: &str) -> &'static str {
    match fstype.parse::<UuidCheckedFs>() {
        Ok(UuidCheckedFs::Xfs) => "ro,nouuid,norecovery",
        Err(_) => "ro",
    }
}

/// The filesystems that refuse to mount a copy next to the original.
#[derive(Debug, Clone, Copy, strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
enum UuidCheckedFs {
    Xfs,
}

/// The filesystems snapshotted by their own tools; every other one on a
/// block device is tried as an LVM volume.
#[derive(Debug, Clone, Copy, strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
enum DiskFs {
    Zfs,
    Btrfs,
}

/// The subvolume `path` lies in, and every subvolume nested below `path`,
/// which a snapshot of the outer one would show as an empty directory. A
/// subvolume's root directory has the inode `root_inode`.
async fn btrfs_origins(
    path: &Path,
    mount: &Mount,
    tools: &Tools,
    root_inode: u64,
) -> Result<Vec<Origin>, String> {
    let subvolume = subvolume_root(path, &mount.point, root_inode).await?;
    let mut origins = vec![Origin::Btrfs {
        subvolume: subvolume.clone(),
    }];
    let listing = tools::run(
        &tools.btrfs,
        &[
            "subvolume".as_ref(),
            "list".as_ref(),
            "-o".as_ref(),
            subvolume.as_os_str(),
        ],
    )
    .await
    .map_err(|e| {
        format!(
            "could not list the subvolumes below {}: {e}",
            subvolume.display()
        )
    })?;
    for nested in nested_subvolumes(&listing, mount) {
        if nested.starts_with(path) && nested != subvolume {
            origins.push(Origin::Btrfs { subvolume: nested });
        }
    }
    Ok(origins)
}

/// The root directory of the btrfs subvolume `path` lies in: the nearest
/// directory at or above it, up to `mount_point`, whose inode is
/// `root_inode`.
async fn subvolume_root(
    path: &Path,
    mount_point: &Path,
    root_inode: u64,
) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    for dir in path
        .ancestors()
        .take_while(|dir| dir.starts_with(mount_point))
    {
        match tokio::fs::metadata(dir).await {
            Ok(meta) if meta.is_dir() && meta.ino() == root_inode => {
                return Ok(dir.to_path_buf());
            }
            Ok(_) | Err(_) => {}
        }
    }
    Err(format!(
        "found no btrfs subvolume root at or above {}",
        path.display()
    ))
}

/// The absolute paths of the subvolumes `btrfs subvolume list -o` lists.
/// Its paths are relative to the filesystem's top level; `mount` shows the
/// part of it below `mount.root` at `mount.point`.
fn nested_subvolumes(listing: &str, mount: &Mount) -> Vec<PathBuf> {
    let mounted = mount.root.strip_prefix("/").unwrap_or(&mount.root);
    listing
        .lines()
        .filter_map(|line| line.split_once(" path ").map(|(_, path)| path.trim()))
        .map(|path| path.strip_prefix("<FS_TREE>/").unwrap_or(path))
        .filter_map(|path| {
            Path::new(path)
                .strip_prefix(mounted)
                .ok()
                .map(Path::to_path_buf)
        })
        .map(|relative| mount.point.join(relative))
        .collect()
}

/// The ZFS dataset mounted at `mount`. A mount of part of a dataset, such
/// as a bind mount of one of its directories, reads the snapshot through
/// the mount of the whole dataset, as only that one shows `.zfs`.
fn zfs_origin(mount: &Mount, table: &MountTable) -> Result<Origin, String> {
    let whole = table.whole_mount_of(&mount.source).ok_or_else(|| {
        format!(
            "{} shows part of ZFS dataset {}, which is not mounted whole here",
            mount.point.display(),
            mount.source
        )
    })?;
    Ok(Origin::Zfs {
        dataset: mount.source.clone(),
        mount_point: mount.point.clone(),
        dataset_point: whole.point.clone(),
        root: mount.root.clone(),
    })
}

/// The LVM logical volume mounted at `mount`, asked of `lvs`.
async fn lvm_origin(mount: &Mount, tools: &Tools) -> Result<Origin, String> {
    let not_lvm = |detail: String| not_snapshottable(mount, &detail);
    let answer = tools::run(
        &tools.lvs,
        &[
            "--noheadings",
            "--separator",
            ":",
            "--options",
            "vg_name,lv_name,lv_attr",
            mount.source.as_str(),
        ],
    )
    .await
    .map_err(|e| not_lvm(format!(": {e}")))?;
    let mut fields = answer.trim().split(':');
    match (fields.next(), fields.next(), fields.next()) {
        (Some(vg), Some(lv), Some(attr)) if !vg.is_empty() && !lv.is_empty() => Ok(Origin::Lvm {
            vg: vg.to_owned(),
            lv: lv.to_owned(),
            thin: attr.starts_with('V'),
            fstype: mount.fstype.clone(),
            mount_point: mount.point.clone(),
            root: mount.root.clone(),
        }),
        _ => Err(not_lvm(String::new())),
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::{super::tools::fake, *};

    fn mount(root: &str, point: &str, fstype: &str, source: &str) -> Mount {
        Mount {
            root: PathBuf::from(root),
            point: PathBuf::from(point),
            fstype: fstype.to_owned(),
            source: source.to_owned(),
        }
    }

    #[test]
    fn nested_subvolumes_are_found_below_the_mounted_one() {
        let listing = "ID 257 gen 10 top level 5 path @home/alice/vm\nID 258 gen 11 top level 5 \
                       path <FS_TREE>/@home/bob/data\nID 259 gen 12 top level 5 path \
                       @other/x\nnot a line\n";
        assert_eq!(
            nested_subvolumes(listing, &mount("/@home", "/home", "btrfs", "/dev/sda2")),
            [
                PathBuf::from("/home/alice/vm"),
                PathBuf::from("/home/bob/data")
            ]
        );
        assert_eq!(
            nested_subvolumes(
                "ID 1 gen 1 top level 5 path srv\n",
                &mount("/", "/mnt", "btrfs", "d")
            ),
            [PathBuf::from("/mnt/srv")]
        );
    }

    #[test]
    fn an_lvm_snapshot_of_xfs_skips_the_uuid_check_and_log_replay() {
        assert_eq!(mount_options("xfs"), "ro,nouuid,norecovery");
        assert_eq!(mount_options("ext4"), "ro");
    }

    #[test]
    fn an_origin_says_what_it_covers() {
        let zfs = Origin::Zfs {
            dataset: "tank/data".to_owned(),
            mount_point: PathBuf::from("/srv/tank"),
            dataset_point: PathBuf::from("/srv/tank"),
            root: PathBuf::from("/"),
        };
        assert_eq!(
            (zfs.covers(), zfs.describe().as_str()),
            (Path::new("/srv/tank"), "ZFS dataset tank/data (/srv/tank)")
        );
        let btrfs = Origin::Btrfs {
            subvolume: PathBuf::from("/home"),
        };
        assert_eq!(
            (btrfs.covers(), btrfs.describe().as_str()),
            (Path::new("/home"), "btrfs subvolume /home")
        );
        let lvm = Origin::Lvm {
            vg: "vg0".to_owned(),
            lv: "pg".to_owned(),
            thin: false,
            fstype: "ext4".to_owned(),
            mount_point: PathBuf::from("/var/lib/postgresql"),
            root: PathBuf::from("/"),
        };
        assert_eq!(lvm.describe(), "LVM volume vg0/pg (/var/lib/postgresql)");
    }

    /// A bind mount of part of a dataset reads the snapshot through the
    /// dataset's own mount point; without one, it cannot be snapshotted.
    #[tokio::test]
    async fn part_of_a_zfs_dataset_is_read_through_the_whole_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(dir.path(), "").await;
        let table = MountTable::parse(
            "1 0 0:3 / /srv/tank rw - zfs tank/data rw\n2 0 0:3 /www /var/www rw - zfs tank/data \
             rw\n3 0 0:4 /logs /var/log/app rw - zfs tank/logs rw\n",
        );
        let found = find(
            &["/var/www".to_owned(), "/var/log/app".to_owned()],
            &table,
            &tools,
        )
        .await;
        assert_eq!(
            found.origins,
            [Origin::Zfs {
                dataset: "tank/data".to_owned(),
                mount_point: PathBuf::from("/var/www"),
                dataset_point: PathBuf::from("/srv/tank"),
                root: PathBuf::from("/www"),
            }]
        );
        assert_eq!(
            found.unsnapshottable,
            [Unsnapshottable {
                path: PathBuf::from("/var/log/app"),
                reason: "/var/log/app shows part of ZFS dataset tank/logs, which is not mounted \
                         whole here"
                    .to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn every_disk_filesystem_at_or_below_a_source_is_an_origin() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(dir.path(), "  vg0:pg:-wi-ao----").await;
        let table = MountTable::parse(
            "1 0 0:1 / / rw - zfs rpool/ROOT rw\n2 1 0:2 / /proc rw - proc proc rw\n3 1 0:3 / \
             /srv/tank rw - zfs tank/data rw\n4 1 253:1 / /var/lib/postgresql rw - ext4 \
             /dev/mapper/vg0-pg rw\n",
        );
        let found = find(
            &["/srv".to_owned(), "/var/lib/postgresql/16".to_owned()],
            &table,
            &tools,
        )
        .await;
        assert_eq!(
            found.origins,
            [
                Origin::Zfs {
                    dataset: "rpool/ROOT".to_owned(),
                    mount_point: PathBuf::from("/"),
                    dataset_point: PathBuf::from("/"),
                    root: PathBuf::from("/"),
                },
                Origin::Zfs {
                    dataset: "tank/data".to_owned(),
                    mount_point: PathBuf::from("/srv/tank"),
                    dataset_point: PathBuf::from("/srv/tank"),
                    root: PathBuf::from("/"),
                },
                Origin::Lvm {
                    vg: "vg0".to_owned(),
                    lv: "pg".to_owned(),
                    thin: false,
                    fstype: "ext4".to_owned(),
                    mount_point: PathBuf::from("/var/lib/postgresql"),
                    root: PathBuf::from("/"),
                },
            ]
        );
        assert_eq!(found.unsnapshottable.len(), 0);
        let whole = find(&["/".to_owned(), "/srv/tank".to_owned()], &table, &tools).await;
        assert_eq!(
            whole.origins.len(),
            3,
            "each origin is taken once: {whole:?}"
        );
    }

    /// A network or FUSE filesystem holds data but has no snapshot here, so
    /// it is named like any other; only memory and the kernel's own views
    /// are read live without a word.
    #[tokio::test]
    async fn only_filesystems_without_data_are_read_live_without_a_word() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(dir.path(), "").await;
        let table = MountTable::parse(
            "1 0 0:1 / / rw - zfs rpool/ROOT rw\n2 1 0:2 / /proc rw - proc proc rw\n3 1 0:3 / \
             /run rw - tmpfs tmpfs rw\n4 1 0:4 / /srv/share rw - nfs4 nas:/export rw\n5 1 0:5 / \
             /srv/remote rw - fuse.sshfs user@host:/data rw\n6 1 0:6 / /proc/sys/fs/binfmt_misc \
             rw - binfmt_misc binfmt_misc rw\n",
        );
        let found = find(&["/".to_owned()], &table, &tools).await;
        assert_eq!(found.origins.len(), 1);
        assert_eq!(
            found.unsnapshottable,
            [
                Unsnapshottable {
                    path: PathBuf::from("/srv/share"),
                    reason: "/srv/share (nfs4 on nas:/export) is not on ZFS, btrfs or an LVM \
                             volume"
                        .to_owned(),
                },
                Unsnapshottable {
                    path: PathBuf::from("/srv/remote"),
                    reason: "/srv/remote (fuse.sshfs on user@host:/data) is not on ZFS, btrfs or \
                             an LVM volume"
                        .to_owned(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_disk_filesystem_that_is_no_lvm_volume_cannot_be_snapshotted() {
        let dir = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(dir.path(), "").await;
        let table = MountTable::parse("1 0 8:1 / / rw - ext4 /dev/sda1 rw\n");
        let found = find(&["/etc".to_owned()], &table, &tools).await;
        assert_eq!(found.origins.len(), 0);
        assert_eq!(found.unsnapshottable.len(), 1);
        let unsnapshottable = found.unsnapshottable.first().unwrap();
        assert_eq!(unsnapshottable.path, Path::new("/etc"));
        assert_eq!(
            unsnapshottable.reason,
            "/ (ext4 on /dev/sda1) is not on ZFS, btrfs or an LVM volume"
        );

        tools.lvs = fake::script(dir.path(), "lvs-fails", "echo 'not an LV' >&2; exit 5").await;
        let found = find(&["/etc".to_owned()], &table, &tools).await;
        let reason = &found.unsnapshottable.first().unwrap().reason;
        assert!(reason.ends_with("failed: not an LV"), "{reason}");
    }

    /// A source that is not an absolute path lies on no mount, so it is
    /// reported as one that cannot be snapshotted instead of being read live
    /// without a word.
    #[tokio::test]
    async fn a_relative_or_empty_source_cannot_be_snapshotted() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(dir.path(), "").await;
        let table = MountTable::parse("1 0 0:1 / / rw - zfs rpool/ROOT rw\n");
        let found = find(&["relative/dir".to_owned(), String::new()], &table, &tools).await;
        assert_eq!(found.origins.len(), 0);
        let reasons: Vec<&str> = found
            .unsnapshottable
            .iter()
            .map(|skipped| skipped.reason.as_str())
            .collect();
        assert_eq!(
            reasons,
            [
                "relative/dir is not an absolute path",
                " is not an absolute path"
            ]
        );
    }

    #[tokio::test]
    async fn a_thin_volume_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(dir.path(), "  vg0:data:Vwi-aotz--").await;
        let table = MountTable::parse("1 0 253:3 /sub /data rw - xfs /dev/vg0/data rw\n");
        let found = find(&["/data".to_owned()], &table, &tools).await;
        assert_eq!(
            found.origins,
            [Origin::Lvm {
                vg: "vg0".to_owned(),
                lv: "data".to_owned(),
                thin: true,
                fstype: "xfs".to_owned(),
                mount_point: PathBuf::from("/data"),
                root: PathBuf::from("/sub"),
            }]
        );
        assert_eq!(
            fake::calls(dir.path()).await,
            ["lvs --noheadings --separator : --options vg_name,lv_name,lv_attr /dev/vg0/data"]
        );
    }

    #[tokio::test]
    async fn btrfs_snapshots_the_subvolume_a_source_is_in_and_those_below_it() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let fs = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(dir.path(), "").await;
        let point = fs.path().to_path_buf();
        let source = point.join("data");
        tokio::fs::create_dir_all(source.join("vm")).await.unwrap();
        let table = MountTable::parse(&format!(
            "1 0 0:30 /@data {} rw - btrfs /dev/sdb1 rw\n",
            point.display()
        ));
        // A temporary directory is no subvolume root, so none is found.
        let found = find(&[source.display().to_string()], &table, &tools).await;
        let reason = &found.unsnapshottable.first().unwrap().reason;
        assert!(
            reason.starts_with("found no btrfs subvolume root"),
            "{reason}"
        );

        // Take the mount point's own inode for a subvolume root's.
        let root_inode = tokio::fs::metadata(&point).await.unwrap().ino();
        tools.btrfs = fake::script(
            dir.path(),
            "btrfs",
            "echo 'ID 257 gen 1 top level 5 path @data/data/vm'; echo 'ID 258 gen 1 top level 5 \
             path @data/elsewhere'",
        )
        .await;
        let mount = table.mount_of(&source).unwrap().clone();
        let origins = btrfs_origins(&source, &mount, &tools, root_inode)
            .await
            .unwrap();
        assert_eq!(
            origins,
            [
                Origin::Btrfs {
                    subvolume: point.clone()
                },
                Origin::Btrfs {
                    subvolume: source.join("vm")
                },
            ],
            "only the subvolumes below the source come along"
        );
        let calls = fake::calls(dir.path()).await;
        assert_eq!(
            calls.last().unwrap(),
            &format!("btrfs subvolume list -o {}", point.display())
        );

        tools.btrfs = fake::script(dir.path(), "btrfs", "echo denied >&2; exit 1").await;
        let error = btrfs_origins(&source, &mount, &tools, root_inode)
            .await
            .unwrap_err();
        assert!(
            error.starts_with("could not list the subvolumes below"),
            "{error}"
        );
    }
}
