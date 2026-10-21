// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Filesystem snapshots a backup reads from instead of the live files.
//!
//! With a schedule's snapshots on, the agent snapshots the ZFS dataset,
//! btrfs subvolume or LVM logical volume under each backup source right
//! before `borg create`, runs borg against the snapshots under the sources'
//! original paths, and removes them again once borg is done.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Size of an LVM snapshot when a schedule names none, in percent of the
/// volume it snapshots.
pub const DEFAULT_LVM_SNAPSHOT_PERCENT: u8 = 10;

/// Whether a schedule's backups read from filesystem snapshots, and what a
/// source that cannot be snapshotted does to the run.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum FsSnapshotMode {
    /// The backup reads the live files.
    #[default]
    Off,
    /// Every source is read from a snapshot; a source that cannot be
    /// snapshotted fails the run before borg starts.
    Required,
    /// Sources are read from snapshots where they can be; one that cannot is
    /// backed up live and the run ends with a warning saying so.
    BestEffort,
}

impl FsSnapshotMode {
    /// Whether the backup snapshots its sources at all.
    #[must_use]
    pub const fn is_on(self) -> bool {
        match self {
            Self::Off => false,
            Self::Required | Self::BestEffort => true,
        }
    }
}

/// The kind of filesystem snapshot an agent can take, named after the tool
/// that takes it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum FsSnapshotKind {
    /// A ZFS dataset snapshot (`zfs snapshot`).
    Zfs,
    /// A read-only btrfs subvolume snapshot (`btrfs subvolume snapshot -r`).
    Btrfs,
    /// An LVM logical volume snapshot (`lvcreate --snapshot`).
    Lvm,
}

/// Whether `percent` can size an LVM snapshot: 1 to 100 percent of the
/// volume it snapshots.
#[must_use]
pub fn is_valid_lvm_snapshot_percent(percent: u8) -> bool {
    (1..=100).contains(&percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_kinds_read_back_from_their_names() {
        for (mode, name) in [
            (FsSnapshotMode::Off, "off"),
            (FsSnapshotMode::Required, "required"),
            (FsSnapshotMode::BestEffort, "best_effort"),
        ] {
            assert_eq!(mode.to_string(), name);
            assert_eq!(name.parse::<FsSnapshotMode>().ok(), Some(mode));
        }
        assert!(!FsSnapshotMode::Off.is_on());
        assert!(FsSnapshotMode::Required.is_on() && FsSnapshotMode::BestEffort.is_on());
        for (kind, name) in [
            (FsSnapshotKind::Zfs, "zfs"),
            (FsSnapshotKind::Btrfs, "btrfs"),
            (FsSnapshotKind::Lvm, "lvm"),
        ] {
            assert_eq!(kind.to_string(), name);
            assert_eq!(name.parse::<FsSnapshotKind>().ok(), Some(kind));
        }
    }

    #[test]
    fn an_lvm_snapshot_is_one_to_a_hundred_percent() {
        assert!(is_valid_lvm_snapshot_percent(1));
        assert!(is_valid_lvm_snapshot_percent(DEFAULT_LVM_SNAPSHOT_PERCENT));
        assert!(is_valid_lvm_snapshot_percent(100));
        assert!(!is_valid_lvm_snapshot_percent(0));
        assert!(!is_valid_lvm_snapshot_percent(101));
    }
}
