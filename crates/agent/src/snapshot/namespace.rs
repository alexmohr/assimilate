// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The private mount namespace `borg create` runs in while it reads from
//! snapshots.
//!
//! Each snapshot is bind-mounted over the directory it stands in for, so
//! borg reads the snapshot under the original path and the archive holds
//! the original paths. The rest of the host never sees these mounts, and
//! they vanish with borg's process, even if the agent dies.
//!
//! Covering a directory hides everything mounted below it, and some places
//! below it must stay live: other filesystems mounted there (`/proc`, a
//! `tmpfs`, a filesystem that is not snapshotted), the database dump pipes
//! borg reads, and borg's own cache and configuration, which it writes. Each
//! of those is opened before anything is covered and bound back afterwards
//! from the open descriptor.

use std::{
    ffi::{CStr, CString, NulError},
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
};

use nix::{
    fcntl::{OFlag, open},
    mount::{MsFlags, mount},
    sched::{CloneFlags, unshare},
    sys::stat::Mode,
};

use super::mountinfo::MountTable;

/// One mount the namespace makes, in the order they are made.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Bind the snapshot `from` over `onto`.
    Snapshot { from: CString, onto: CString },
    /// Bind the live directory opened as `keep_live[index]` back over
    /// `onto`.
    Live { index: usize, onto: CString },
}

/// What to mount in borg's namespace. Built in the agent; [`Self::enter`]
/// runs in the forked child just before borg starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    /// The live directories to bind back, opened before anything is
    /// covered.
    keep_live: Vec<CString>,
    /// Every mount, outermost first, so a deeper one lands on top of the
    /// one covering its parent.
    steps: Vec<Step>,
}

impl MountPlan {
    /// The plan binding each `(snapshot, origin)` pair, keeping live every
    /// mount `table` lists below an origin that is not itself one, and each
    /// of `keep_dirs` that lies below an origin.
    ///
    /// # Errors
    ///
    /// Returns an error for a path holding a NUL byte, which no mount can
    /// name.
    pub fn new(
        binds: &[(PathBuf, PathBuf)],
        table: &MountTable,
        keep_dirs: &[PathBuf],
    ) -> Result<Self, NulError> {
        let origins: Vec<&Path> = binds.iter().map(|(_, onto)| onto.as_path()).collect();
        let mut live: Vec<PathBuf> = Vec::new();
        for origin in &origins {
            let below = table.mount_points_below(origin).into_iter().chain(
                keep_dirs
                    .iter()
                    .filter(|dir| dir.starts_with(origin))
                    .cloned(),
            );
            for dir in below {
                if !origins.contains(&dir.as_path()) && !live.contains(&dir) {
                    live.push(dir);
                }
            }
        }
        let mut ordered: Vec<(usize, Step)> = Vec::new();
        for (from, onto) in binds {
            ordered.push((
                depth(onto),
                Step::Snapshot {
                    from: c_path(from)?,
                    onto: c_path(onto)?,
                },
            ));
        }
        for (index, dir) in live.iter().enumerate() {
            ordered.push((
                depth(dir),
                Step::Live {
                    index,
                    onto: c_path(dir)?,
                },
            ));
        }
        ordered.sort_by_key(|(depth, _)| *depth);
        Ok(Self {
            keep_live: live
                .iter()
                .map(|dir| c_path(dir))
                .collect::<Result<_, _>>()?,
            steps: ordered.into_iter().map(|(_, step)| step).collect(),
        })
    }

    /// How many live directories [`Self::enter`] opens, for the caller to
    /// reserve room for their descriptors.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.keep_live.len()
    }

    /// Moves the calling process into a new mount namespace (`flags`, with
    /// `CLONE_NEWNS`) and makes the plan's mounts there. Runs in the forked
    /// child just before exec, so it only makes system calls: the paths are
    /// prepared beforehand and `fds` has room for every live directory.
    ///
    /// # Errors
    ///
    /// Returns the raw OS error of the first call that fails.
    pub fn enter(&self, flags: CloneFlags, fds: &mut Vec<RawFd>) -> std::io::Result<()> {
        unshare(flags)?;
        // Nothing mounted here may reach the host's namespace.
        mount(
            None::<&CStr>,
            c"/",
            None::<&CStr>,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            None::<&CStr>,
        )?;
        fds.clear();
        for dir in &self.keep_live {
            fds.push(open(
                dir.as_c_str(),
                OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
                Mode::empty(),
            )?);
        }
        for step in &self.steps {
            match step {
                Step::Snapshot { from, onto } => {
                    // Opened just before its own mount rather than with the
                    // live directories: a deeper origin lies inside the
                    // snapshot covering its parent. Without following a
                    // last component that was replaced by a symbolic link
                    // since the origin was found, so the snapshot never
                    // lands somewhere else under the origin's name.
                    let target = open(
                        onto.as_c_str(),
                        OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                        Mode::empty(),
                    )?;
                    let mut buffer = [0u8; FD_PATH_LEN];
                    let mounted = mount(
                        Some(from.as_c_str()),
                        fd_path(target, &mut buffer),
                        None::<&CStr>,
                        MsFlags::MS_BIND,
                        None::<&CStr>,
                    );
                    let _ = nix::unistd::close(target);
                    mounted?;
                }
                Step::Live { index, onto } => {
                    let fd = fds.get(*index).copied().unwrap_or(-1);
                    let mut buffer = [0u8; FD_PATH_LEN];
                    mount(
                        Some(fd_path(fd, &mut buffer)),
                        onto.as_c_str(),
                        None::<&CStr>,
                        MsFlags::MS_BIND | MsFlags::MS_REC,
                        None::<&CStr>,
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Room for `/proc/self/fd/` and any descriptor number, NUL included.
const FD_PATH_LEN: usize = 32;

/// Writes `/proc/self/fd/<fd>` into `buffer`, which names the directory the
/// descriptor holds open, without allocating.
fn fd_path(fd: RawFd, buffer: &mut [u8; FD_PATH_LEN]) -> &CStr {
    const PREFIX: &[u8] = b"/proc/self/fd/";
    const DIGITS: &[u8] = b"0123456789";
    let (head, tail) = buffer.split_at_mut(PREFIX.len());
    head.copy_from_slice(PREFIX);
    // The digits, last first.
    let mut reversed = [0u8; 10];
    let mut count = 0usize;
    let mut rest = u32::try_from(fd).unwrap_or(0);
    loop {
        let digit = usize::try_from(rest % 10).unwrap_or(0);
        if let (Some(slot), Some(character)) = (reversed.get_mut(count), DIGITS.get(digit)) {
            *slot = *character;
        }
        count = count.saturating_add(1);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    for (slot, digit) in tail.iter_mut().zip(reversed.iter().take(count).rev()) {
        *slot = *digit;
    }
    if let Some(end) = tail.get_mut(count) {
        *end = 0;
    }
    CStr::from_bytes_until_nul(buffer).unwrap_or(c"")
}

fn depth(path: &Path) -> usize {
    path.components().count()
}

fn c_path(path: &Path) -> Result<CString, NulError> {
    CString::new(path.as_os_str().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_is_named_by_its_proc_path() {
        let mut buffer = [0u8; FD_PATH_LEN];
        assert_eq!(fd_path(7, &mut buffer), c"/proc/self/fd/7");
        assert_eq!(fd_path(1024, &mut buffer), c"/proc/self/fd/1024");
        assert_eq!(fd_path(0, &mut buffer), c"/proc/self/fd/0");
        assert_eq!(fd_path(i32::MAX, &mut buffer), c"/proc/self/fd/2147483647");
    }

    #[test]
    fn mounts_are_made_outermost_first_and_live_ones_are_bound_back() {
        let table = MountTable::parse(
            "1 0 0:1 / / rw - btrfs /dev/sda2 rw\n2 1 0:2 / /proc rw - proc proc rw\n3 1 0:3 / \
             /srv/tank rw - zfs tank rw\n4 1 0:4 / /home rw - btrfs /dev/sda2 rw\n",
        );
        let binds = [
            (
                PathBuf::from("/srv/tank/.zfs/snapshot/s"),
                PathBuf::from("/srv/tank"),
            ),
            (PathBuf::from("/.assimilate-snapshot-s"), PathBuf::from("/")),
        ];
        let keep = [
            PathBuf::from("/var/lib/assimilate/database-dumps/schedule-7"),
            PathBuf::from("/root/.cache/borg"),
            PathBuf::from("/var/lib/assimilate/database-dumps/schedule-7"),
            PathBuf::from("/srv/tank"),
        ];
        let plan = MountPlan::new(&binds, &table, &keep).unwrap();
        let live = |dir: &str| CString::new(dir).unwrap();
        assert_eq!(
            plan.keep_live,
            [
                live("/proc"),
                live("/home"),
                live("/var/lib/assimilate/database-dumps/schedule-7"),
                live("/root/.cache/borg"),
            ],
            "the origins themselves and repeats are left out"
        );
        assert_eq!(plan.live_count(), 4);
        let order: Vec<String> = plan
            .steps
            .iter()
            .map(|step| match step {
                Step::Snapshot { onto, .. } => format!("snapshot {}", onto.to_string_lossy()),
                Step::Live { index, onto } => format!("live {index} {}", onto.to_string_lossy()),
            })
            .collect();
        assert_eq!(
            order,
            [
                "snapshot /",
                "live 0 /proc",
                "live 1 /home",
                "snapshot /srv/tank",
                "live 3 /root/.cache/borg",
                "live 2 /var/lib/assimilate/database-dumps/schedule-7",
            ]
        );
        assert!(
            MountPlan::new(
                &[(PathBuf::from("/a\0b"), PathBuf::from("/x"))],
                &table,
                &[]
            )
            .is_err()
        );
    }

    /// Reads up to 64 bytes of `path` with system calls only.
    fn read_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
        let fd = open(path, OFlag::O_RDONLY | OFlag::O_CLOEXEC, Mode::empty())?;
        let mut buffer = [0u8; 64];
        let read = nix::unistd::read(fd, &mut buffer);
        let _ = nix::unistd::close(fd);
        Ok(buffer.get(..read?).unwrap_or_default().to_vec())
    }

    /// Enters a namespace for real, in a forked child: a user namespace
    /// lets the test mount without being root. The child checks what it
    /// sees and reports through its exit status.
    #[tokio::test]
    async fn borg_sees_the_snapshot_under_the_original_path_and_live_dirs_stay_live() {
        use nix::{
            sys::wait::{WaitStatus, waitpid},
            unistd::{ForkResult, fork},
        };

        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("data");
        let snapshot = dir.path().join("snapshot");
        for path in [origin.join("dumps"), snapshot.join("dumps")] {
            tokio::fs::create_dir_all(path).await.unwrap();
        }
        tokio::fs::write(origin.join("file"), "live").await.unwrap();
        tokio::fs::write(snapshot.join("file"), "snapshot")
            .await
            .unwrap();
        tokio::fs::write(origin.join("dumps/pipe"), "live dump")
            .await
            .unwrap();
        let plan = MountPlan::new(
            &[(snapshot.clone(), origin.clone())],
            &MountTable::default(),
            &[origin.join("dumps")],
        )
        .unwrap();
        let mut fds = Vec::with_capacity(plan.live_count());
        let (file, dump) = (origin.join("file"), origin.join("dumps/pipe"));

        // SAFETY: the child enters the namespace, reads two files with
        // system calls and exits; it touches nothing the parent's other
        // threads hold.
        match unsafe { fork() }.unwrap() {
            ForkResult::Child => {
                let code = match plan.enter(
                    CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS,
                    &mut fds,
                ) {
                    Err(_) => 3,
                    Ok(()) => match (read_bytes(&file), read_bytes(&dump)) {
                        (Ok(file), Ok(dump)) => {
                            i32::from(
                                file.as_slice() != b"snapshot" || dump.as_slice() != b"live dump",
                            ) * 2
                        }
                        _ => 4,
                    },
                };
                std::process::exit(code);
            }
            ForkResult::Parent { child } => {
                assert_eq!(
                    waitpid(child, None).unwrap(),
                    WaitStatus::Exited(child, 0),
                    "2: the child saw the wrong files; 3: it could not enter the namespace; 4: it \
                     could not read them"
                );
            }
        }
        assert_eq!(
            tokio::fs::read(&file).await.unwrap(),
            b"live",
            "the host never sees the snapshot"
        );
    }

    /// Whether entering `plan` with `flags` fails, tried in a forked child
    /// so the test process keeps its own namespaces. The child makes only
    /// system calls and reports through its exit status.
    fn entering_fails(plan: &MountPlan, flags: CloneFlags) -> bool {
        use nix::{
            sys::wait::{WaitStatus, waitpid},
            unistd::{ForkResult, fork},
        };
        let mut fds = Vec::with_capacity(plan.live_count());
        // SAFETY: the child only enters the namespace and exits; it touches
        // nothing the parent's other threads hold.
        match unsafe { fork() }.unwrap() {
            ForkResult::Child => std::process::exit(i32::from(plan.enter(flags, &mut fds).is_ok())),
            ForkResult::Parent { child } => {
                waitpid(child, None).unwrap() == WaitStatus::Exited(child, 0)
            }
        }
    }

    /// Each mount the plan makes stops the namespace when it cannot be made,
    /// so borg never starts with a source half in place.
    #[tokio::test]
    async fn a_mount_that_cannot_be_made_stops_the_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("data");
        let snapshot = dir.path().join("snapshot");
        tokio::fs::create_dir_all(origin.join("dumps"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&snapshot).await.unwrap();
        let table = MountTable::default();
        let plan = |binds: &[(PathBuf, PathBuf)], live: &[PathBuf]| {
            MountPlan::new(binds, &table, live).unwrap()
        };
        let both = CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS;
        let bind = [(snapshot.clone(), origin.clone())];

        // Without a mount namespace of its own, the mounts cannot be made
        // private.
        assert!(entering_fails(&plan(&bind, &[]), CloneFlags::CLONE_NEWUSER));
        // A live directory that is gone cannot be opened.
        assert!(entering_fails(
            &plan(&bind, &[origin.join("missing")]),
            both
        ));
        // A snapshot that is gone cannot be bound.
        assert!(entering_fails(
            &plan(&[(dir.path().join("gone"), origin.clone())], &[]),
            both
        ));
        // The snapshot lacks the live directory, so it has nowhere to go.
        assert!(entering_fails(&plan(&bind, &[origin.join("dumps")]), both));
        // An origin replaced by a symbolic link is not followed.
        let link = dir.path().join("link");
        tokio::fs::symlink(&origin, &link).await.unwrap();
        assert!(!entering_fails(&plan(&bind, &[]), both));
        assert!(entering_fails(
            &plan(&[(snapshot.clone(), link)], &[]),
            both
        ));
    }

    /// A borg whose snapshots cannot be mounted is not started at all,
    /// whether entering the namespace is refused or its first mount fails.
    #[tokio::test]
    async fn borg_is_not_started_where_its_snapshots_cannot_be_mounted() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("data");
        tokio::fs::create_dir_all(&origin).await.unwrap();
        let file = origin.join("file");
        tokio::fs::write(&file, "live").await.unwrap();
        let borg = crate::borg::Borg::with_extra_env(PathBuf::from("cat"), Vec::new());
        assert_eq!(borg.run(&[&file], &[]).await.unwrap().stdout, b"live");

        let plan = MountPlan::new(
            &[(dir.path().join("gone"), origin.clone())],
            &MountTable::default(),
            &[],
        )
        .unwrap();
        let namespaced = borg.in_namespace(Some(std::sync::Arc::new(plan)));
        assert!(namespaced.run(&[&file], &[]).await.is_err());
        assert!(namespaced.spawn(&[&file], &[]).is_err());
    }
}
