// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What each run has snapshotted, written down before the snapshot is taken
//! so that one an agent crash or restart left behind is removed the next
//! time the agent starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    mountinfo::MountTable,
    tools::{self, ToolError, Tools},
};

/// Where the agent keeps state across restarts, unless
/// `ASSIMILATE_STATE_DIR` names another directory.
const DEFAULT_STATE_DIR: &str = "/var/lib/assimilate";

/// One snapshot a run took, and how to remove it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Taken {
    /// `dataset@name`, removed with `zfs destroy`.
    Zfs { dataset: String, name: String },
    /// A read-only btrfs subvolume, removed with `btrfs subvolume delete`.
    Btrfs { path: PathBuf },
    /// A snapshot logical volume, mounted at `mount_dir` while the backup
    /// reads it, then unmounted and removed with `lvremove`.
    Lvm {
        vg: String,
        name: String,
        mount_dir: PathBuf,
    },
}

impl Taken {
    /// The snapshot, in words, for the run's timeline.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Zfs { dataset, name } => format!("ZFS snapshot {dataset}@{name}"),
            Self::Btrfs { path } => format!("btrfs snapshot {}", path.display()),
            Self::Lvm { vg, name, .. } => format!("LVM snapshot {vg}/{name}"),
        }
    }

    /// Removes the snapshot, and for LVM unmounts it first if `mounts` has
    /// it mounted. A snapshot that is already gone, or was never taken, is
    /// left alone.
    ///
    /// # Errors
    ///
    /// Returns an error when a tool refuses to remove one that is there.
    pub async fn remove(&self, tools: &Tools, mounts: &MountTable) -> Result<(), ToolError> {
        match self {
            Self::Zfs { dataset, name } => {
                let snapshot = format!("{dataset}@{name}");
                let listed = tools::run(&tools.zfs, &["list", "-H", "-o", "name", &snapshot]).await;
                if listed.is_ok() {
                    tools::run(&tools.zfs, &["destroy", &snapshot]).await?;
                }
            }
            Self::Btrfs { path } => {
                if tokio::fs::try_exists(path).await.unwrap_or(false) {
                    tools::run(
                        &tools.btrfs,
                        &["subvolume".as_ref(), "delete".as_ref(), path.as_os_str()],
                    )
                    .await?;
                }
            }
            Self::Lvm {
                vg,
                name,
                mount_dir,
            } => {
                if mounts
                    .mount_of(mount_dir)
                    .is_some_and(|mount| mount.point == *mount_dir)
                {
                    tools::run(&tools.umount, &[mount_dir]).await?;
                }
                let volume = format!("{vg}/{name}");
                if tools::run(&tools.lvs, &[&volume]).await.is_ok() {
                    tools::run(&tools.lvremove, &["--force", &volume]).await?;
                }
                // Left empty by the unmount; one not there is fine.
                let _ = tokio::fs::remove_dir(mount_dir).await;
            }
        }
        Ok(())
    }
}

/// The record of one run's snapshots, a JSON file in the agent's state
/// directory.
#[derive(Debug)]
pub struct Record {
    dir: PathBuf,
    path: PathBuf,
    taken: Vec<Taken>,
}

impl Record {
    /// The directory the records live in.
    #[must_use]
    pub fn dir() -> PathBuf {
        std::env::var_os("ASSIMILATE_STATE_DIR")
            .map_or_else(|| PathBuf::from(DEFAULT_STATE_DIR), PathBuf::from)
            .join("snapshots")
    }

    /// An empty record for the run `tag`, in `dir`. Nothing is written until
    /// the first snapshot is noted.
    #[must_use]
    pub fn new(dir: &Path, tag: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join(format!("{tag}.json")),
            taken: Vec::new(),
        }
    }

    /// Notes `taken` before it is taken, so a crash in between still finds
    /// it.
    ///
    /// # Errors
    ///
    /// Returns an error when the record cannot be written.
    pub async fn note(&mut self, taken: Taken) -> std::io::Result<()> {
        self.taken.push(taken);
        tokio::fs::create_dir_all(&self.dir).await?;
        write(&self.path, &self.taken).await
    }

    /// Forgets the record once every snapshot in it is removed.
    pub async fn discard(self) {
        if let Err(e) = tokio::fs::remove_file(&self.path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            let path = self.path.display();
            tracing::warn!("could not remove the snapshot record {path}: {e}");
        }
    }
}

/// The files a record directory holds, by their extension.
#[derive(Debug, Clone, Copy, strum_macros::EnumString)]
enum RecordFile {
    /// A run's record.
    #[strum(serialize = "json")]
    Record,
    /// A record [`write`] had not finished.
    #[strum(serialize = "partial")]
    Partial,
}

/// Writes `taken` to `path` through a temporary file, so a crash never
/// leaves a half-written record.
async fn write(path: &Path, taken: &[Taken]) -> std::io::Result<()> {
    let partial = path.with_extension("json.partial");
    let text = serde_json::to_vec_pretty(taken).map_err(std::io::Error::other)?;
    tokio::fs::write(&partial, text).await?;
    tokio::fs::rename(&partial, path).await
}

/// Reads the snapshots a record names.
async fn read(path: &Path) -> std::io::Result<Vec<Taken>> {
    let text = tokio::fs::read(path).await?;
    serde_json::from_slice(&text).map_err(std::io::Error::other)
}

/// Removes every snapshot the records in `dir` name, newest first within a
/// run, then the records; `mounts` says which LVM snapshots are still
/// mounted. A record whose snapshots cannot all be removed is kept for the
/// next attempt. Returns why each removal failed.
pub async fn remove_leftovers(dir: &Path, tools: &Tools, mounts: &MountTable) -> Vec<String> {
    let mut failures = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return failures;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| extension.parse::<RecordFile>().ok())
        {
            Some(RecordFile::Record) => {}
            // A record the agent stopped while writing. The snapshot it was
            // about to name was never taken, so there is nothing to remove.
            Some(RecordFile::Partial) => {
                let _ = tokio::fs::remove_file(&path).await;
                continue;
            }
            None => continue,
        }
        let taken = match read(&path).await {
            Ok(taken) => taken,
            Err(e) => {
                let path = path.display();
                failures.push(format!("could not read the snapshot record {path}: {e}"));
                continue;
            }
        };
        let before = failures.len();
        for snapshot in taken.iter().rev() {
            if let Err(e) = snapshot.remove(tools, mounts).await {
                failures.push(format!("could not remove a leftover snapshot: {e}"));
            }
        }
        if failures.len() == before {
            let path_text = path.display();
            tracing::info!("Removed the snapshots a stopped run left behind ({path_text})");
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    failures
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::{super::tools::fake, *};

    #[tokio::test]
    async fn a_record_is_written_before_each_snapshot_and_discarded_after() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = Record::new(dir.path(), "schedule-7-1");
        record
            .note(Taken::Zfs {
                dataset: "tank/data".to_owned(),
                name: "assimilate-schedule-7-1".to_owned(),
            })
            .await
            .unwrap();
        record
            .note(Taken::Btrfs {
                path: PathBuf::from("/home/.assimilate-snapshot-schedule-7-1"),
            })
            .await
            .unwrap();
        let written: Vec<Taken> = serde_json::from_slice(
            &tokio::fs::read(dir.path().join("schedule-7-1.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(written.len(), 2);
        let text = tokio::fs::read_to_string(dir.path().join("schedule-7-1.json"))
            .await
            .unwrap();
        assert!(text.contains(r#""kind": "zfs""#), "{text}");
        record.discard().await;
        assert!(
            !tokio::fs::try_exists(dir.path().join("schedule-7-1.json"))
                .await
                .unwrap()
        );
        Record::new(dir.path(), "never-written").discard().await;
    }

    /// A record the agent stopped while writing named no snapshot yet, so it
    /// is only removed; other files are left alone.
    #[tokio::test]
    async fn a_half_written_record_is_removed_and_other_files_are_left() {
        let tools_dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let tools = fake::tools(tools_dir.path(), "").await;
        let partial = state.path().join("manual-0.json.partial");
        let notes = state.path().join("notes.txt");
        tokio::fs::write(&partial, "[").await.unwrap();
        tokio::fs::write(&notes, "not a record").await.unwrap();

        let failures = remove_leftovers(state.path(), &tools, &MountTable::default()).await;
        assert_eq!(failures.len(), 0, "{failures:?}");
        assert!(!tokio::fs::try_exists(&partial).await.unwrap());
        assert!(tokio::fs::try_exists(&notes).await.unwrap());
        assert_eq!(fake::calls(tools_dir.path()).await.len(), 0);
    }

    #[tokio::test]
    async fn leftovers_are_removed_newest_first_and_a_stuck_record_is_kept() {
        let tools_dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(tools_dir.path(), "").await;
        tools.lvs = fake::script(tools_dir.path(), "lvs", "exit 5").await;
        let btrfs_snapshot = state.path().join("snap");
        tokio::fs::create_dir(&btrfs_snapshot).await.unwrap();
        let mut record = Record::new(state.path(), "manual-1");
        record
            .note(Taken::Zfs {
                dataset: "tank".to_owned(),
                name: "assimilate-manual-1".to_owned(),
            })
            .await
            .unwrap();
        record
            .note(Taken::Btrfs {
                path: btrfs_snapshot.clone(),
            })
            .await
            .unwrap();
        record
            .note(Taken::Lvm {
                vg: "assimilate-test-vg".to_owned(),
                name: "assimilate-manual-1".to_owned(),
                mount_dir: state.path().join("lvm-mount"),
            })
            .await
            .unwrap();
        tokio::fs::write(state.path().join("notes.txt"), "not a record")
            .await
            .unwrap();
        tokio::fs::write(state.path().join("broken.json"), "{")
            .await
            .unwrap();

        let failures = remove_leftovers(state.path(), &tools, &MountTable::default()).await;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures.first().unwrap().contains("broken.json"));
        assert_eq!(
            fake::calls(tools_dir.path()).await,
            [
                "lvs assimilate-test-vg/assimilate-manual-1".to_owned(),
                format!("btrfs subvolume delete {}", btrfs_snapshot.display()),
                "zfs list -H -o name tank@assimilate-manual-1".to_owned(),
                "zfs destroy tank@assimilate-manual-1".to_owned(),
            ],
            "lvs knows no such LVM snapshot, so neither unmount nor lvremove runs"
        );
        assert!(
            !tokio::fs::try_exists(state.path().join("manual-1.json"))
                .await
                .unwrap()
        );

        // A zfs that refuses to destroy keeps the record for next time.
        let mut record = Record::new(state.path(), "manual-2");
        record
            .note(Taken::Zfs {
                dataset: "tank".to_owned(),
                name: "assimilate-manual-2".to_owned(),
            })
            .await
            .unwrap();
        tools.zfs = fake::script(
            tools_dir.path(),
            "zfs",
            r#"[ "$1" = destroy ] && { echo busy >&2; exit 1; }; exit 0"#,
        )
        .await;
        let failures = remove_leftovers(state.path(), &tools, &MountTable::default()).await;
        assert!(
            failures
                .iter()
                .any(|failure| failure.ends_with("failed: busy")),
            "{failures:?}"
        );
        assert!(
            tokio::fs::try_exists(state.path().join("manual-2.json"))
                .await
                .unwrap()
        );
        assert_eq!(
            remove_leftovers(
                &state.path().join("missing"),
                &tools,
                &MountTable::default()
            )
            .await
            .len(),
            0
        );
    }

    /// A mounted LVM snapshot is unmounted before lvremove, one lvs knows is
    /// removed, and a ZFS snapshot that is already gone is left alone.
    #[tokio::test]
    async fn a_snapshot_is_removed_only_as_far_as_it_still_exists() {
        let tools_dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(tools_dir.path(), "").await;
        tools.zfs = fake::script(tools_dir.path(), "zfs", "exit 1").await;
        let mount_dir = state.path().join("vg0-data");
        tokio::fs::create_dir(&mount_dir).await.unwrap();
        let mounts = MountTable::parse(&format!(
            "1 0 253:4 / {} ro - ext4 /dev/vg0/assimilate-data-1 ro",
            mount_dir.display()
        ));
        let lvm = Taken::Lvm {
            vg: "vg0".to_owned(),
            name: "assimilate-data-1".to_owned(),
            mount_dir: mount_dir.clone(),
        };
        lvm.remove(&tools, &mounts).await.unwrap();
        Taken::Zfs {
            dataset: "tank".to_owned(),
            name: "assimilate-1".to_owned(),
        }
        .remove(&tools, &mounts)
        .await
        .unwrap();
        assert_eq!(
            fake::calls(tools_dir.path()).await,
            [
                format!("umount {}", mount_dir.display()),
                "lvs vg0/assimilate-data-1".to_owned(),
                "lvremove --force vg0/assimilate-data-1".to_owned(),
                "zfs list -H -o name tank@assimilate-1".to_owned(),
            ]
        );
        assert!(!tokio::fs::try_exists(&mount_dir).await.unwrap());

        tools.umount = fake::script(tools_dir.path(), "umount", "echo busy >&2; exit 32").await;
        tokio::fs::create_dir(&mount_dir).await.unwrap();
        let error = lvm.remove(&tools, &mounts).await.unwrap_err();
        assert!(error.to_string().ends_with("failed: busy"), "{error}");
    }

    /// A record that is not a file can neither be read nor discarded; the
    /// first is a failure to report, the second only a warning.
    #[tokio::test]
    async fn a_record_that_is_a_directory_is_reported_and_kept() {
        let tools_dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let tools = fake::tools(tools_dir.path(), "").await;
        tokio::fs::create_dir(state.path().join("odd.json"))
            .await
            .unwrap();
        let failures = remove_leftovers(state.path(), &tools, &MountTable::default()).await;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures
                .first()
                .unwrap()
                .starts_with("could not read the snapshot record"),
            "{failures:?}"
        );
        Record::new(state.path(), "odd").discard().await;
        let kept = tokio::fs::metadata(state.path().join("odd.json")).await;
        assert!(kept.unwrap().is_dir());
    }

    #[tokio::test]
    async fn the_state_directory_holds_the_records() {
        assert!(Record::dir().ends_with("snapshots"));
    }
}
