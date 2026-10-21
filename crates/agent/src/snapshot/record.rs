// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What each run has snapshotted, written down before the snapshot is taken
//! so that one an agent crash or restart left behind is removed the next
//! time the agent starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::tools::{self, ToolError, Tools};

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

    /// Removes the snapshot, and for LVM unmounts it first. A snapshot that
    /// is already gone, or was never taken, is left alone.
    ///
    /// # Errors
    ///
    /// Returns an error when a tool refuses to remove one that is there.
    pub async fn remove(&self, tools: &Tools) -> Result<(), ToolError> {
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
                if is_mounted(mount_dir).await {
                    tools::run(&tools.umount, &[mount_dir]).await?;
                }
                let device = Path::new("/dev").join(vg).join(name);
                if tokio::fs::try_exists(&device).await.unwrap_or(false) {
                    tools::run(&tools.lvremove, &["--force", &format!("{vg}/{name}")]).await?;
                }
                // Left empty by the unmount; one not there is fine.
                let _ = tokio::fs::remove_dir(mount_dir).await;
            }
        }
        Ok(())
    }
}

/// Whether something is mounted on `dir`.
async fn is_mounted(dir: &Path) -> bool {
    super::mountinfo::MountTable::read()
        .await
        .is_ok_and(|table| table.mount_of(dir).is_some_and(|mount| mount.point == dir))
}

/// The record of one run's snapshots, a JSON file in the agent's state
/// directory.
#[derive(Debug)]
pub struct Record {
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
        write(&self.path, &self.taken).await
    }

    /// Forgets the record once every snapshot in it is removed.
    pub async fn discard(self) {
        if let Err(e) = tokio::fs::remove_file(&self.path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "could not remove the snapshot record {}: {e}",
                self.path.display()
            );
        }
    }
}

/// Writes `taken` to `path` through a temporary file, so a crash never
/// leaves a half-written record.
async fn write(path: &Path, taken: &[Taken]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let partial = path.with_extension("json.partial");
    let text = serde_json::to_vec_pretty(taken).map_err(std::io::Error::other)?;
    tokio::fs::write(&partial, text).await?;
    tokio::fs::rename(&partial, path).await
}

/// Removes every snapshot the records in `dir` name, newest first within a
/// run, then the records. A record whose snapshots cannot all be removed is
/// kept for the next attempt. Returns why each removal failed.
pub async fn remove_leftovers(dir: &Path, tools: &Tools) -> Vec<String> {
    let mut failures = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return failures;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let taken: Vec<Taken> = match tokio::fs::read(&path)
            .await
            .map(|text| serde_json::from_slice(&text))
        {
            Ok(Ok(taken)) => taken,
            Ok(Err(e)) => {
                failures.push(format!(
                    "could not read the snapshot record {}: {e}",
                    path.display()
                ));
                continue;
            }
            Err(e) => {
                failures.push(format!(
                    "could not read the snapshot record {}: {e}",
                    path.display()
                ));
                continue;
            }
        };
        let before = failures.len();
        for snapshot in taken.iter().rev() {
            if let Err(e) = snapshot.remove(tools).await {
                failures.push(format!("could not remove a leftover snapshot: {e}"));
            }
        }
        if failures.len() == before {
            tracing::info!(
                "Removed the snapshots a stopped run left behind ({})",
                path.display()
            );
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

    #[tokio::test]
    async fn leftovers_are_removed_newest_first_and_a_stuck_record_is_kept() {
        let tools_dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(tools_dir.path(), "").await;
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

        let failures = remove_leftovers(state.path(), &tools).await;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures.first().unwrap().contains("broken.json"));
        assert_eq!(
            fake::calls(tools_dir.path()).await,
            [
                format!("btrfs subvolume delete {}", btrfs_snapshot.display()),
                "zfs list -H -o name tank@assimilate-manual-1".to_owned(),
                "zfs destroy tank@assimilate-manual-1".to_owned(),
            ],
            "the LVM snapshot was never created, so neither unmount nor lvremove runs"
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
        let failures = remove_leftovers(state.path(), &tools).await;
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
            remove_leftovers(&state.path().join("missing"), &tools)
                .await
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn the_state_directory_holds_the_records() {
        assert!(Record::dir().ends_with("snapshots"));
    }
}
