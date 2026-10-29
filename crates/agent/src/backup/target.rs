// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::PathBuf;

use shared::{
    hooks::HookCommand,
    types::{Compression, FileChangePattern},
};

/// How many archives of each age bucket `borg prune` keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retention {
    pub hourly: u32,
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
}

impl Retention {
    pub fn is_unset(self) -> bool {
        self == Self::default()
    }

    /// The bucket-wise maximum: keeps everything either side would keep.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            hourly: self.hourly.max(other.hourly),
            daily: self.daily.max(other.daily),
            weekly: self.weekly.max(other.weekly),
            monthly: self.monthly.max(other.monthly),
            yearly: self.yearly.max(other.yearly),
        }
    }
}

#[derive(Clone)]
pub struct BackupTarget {
    pub target_name: String,
    pub schedule_id: Option<i64>,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_host_key: String,
    pub known_hosts_path: Option<PathBuf>,
    pub passphrase: String,
    pub hostname: String,
    pub compression: Compression,
    pub backup_sources: Vec<String>,
    pub keep_hourly: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub keep_yearly: u32,
    /// Retention for archives named before they carried a schedule id
    /// (`<host>-<timestamp>`). Their schedule is unknown, so this is the
    /// union over the repo's schedules - never less than any of them keeps.
    pub legacy_retention: Retention,
    pub compact_enabled: bool,
    /// How to stage this host's virtual machines before the backup, when the
    /// schedule opts in and the host has staging enabled.
    pub vm_snapshot: Option<shared::vm::VmSnapshotConfig>,
    /// Which databases to dump into the archive, when the schedule opts in
    /// and the host has dumps enabled.
    pub database_backup: Option<shared::database::DatabaseBackupConfig>,
    /// Whether the backup reads from filesystem snapshots of its sources.
    pub fs_snapshot: shared::snapshot::FsSnapshotMode,
    /// Size of an LVM snapshot, in percent of its volume.
    pub lvm_snapshot_percent: u8,
    /// Where the snapshot steps go on the run's timeline, for a run the
    /// server started.
    pub run_events: Option<crate::snapshot::RunEvents>,
    pub pre_backup_commands: Vec<HookCommand>,
    pub post_backup_commands: Vec<HookCommand>,
    pub hook_timeout_seconds: u32,
    pub skip_targets: Vec<String>,
    pub exclude_patterns: Vec<String>,
    /// Patterns rescued from `exclude_patterns` - checked first, so a path
    /// matching one is backed up even if a broader exclude would otherwise
    /// skip it.
    pub include_patterns: Vec<String>,
    pub rate_limit_kbps: Option<u32>,
    pub ssh_auth_sock: Option<PathBuf>,
    pub canary_enabled: bool,
    pub accept_relocation: bool,
    pub file_change_patterns: Vec<FileChangePattern>,
}

impl Default for BackupTarget {
    fn default() -> Self {
        Self {
            target_name: String::new(),
            schedule_id: None,
            repo_path: String::new(),
            ssh_user: String::new(),
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_host_key: String::new(),
            known_hosts_path: None,
            passphrase: String::new(),
            hostname: String::new(),
            compression: Compression::Lz4,
            backup_sources: Vec::new(),
            keep_hourly: 0,
            keep_daily: 0,
            keep_weekly: 0,
            keep_monthly: 0,
            keep_yearly: 0,
            legacy_retention: Retention::default(),
            compact_enabled: false,
            vm_snapshot: None,
            database_backup: None,
            fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
            lvm_snapshot_percent: shared::snapshot::DEFAULT_LVM_SNAPSHOT_PERCENT,
            run_events: None,
            pre_backup_commands: Vec::new(),
            post_backup_commands: Vec::new(),
            hook_timeout_seconds: 60,
            skip_targets: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
            rate_limit_kbps: None,
            ssh_auth_sock: None,
            canary_enabled: false,
            accept_relocation: false,
            file_change_patterns: Vec::new(),
        }
    }
}

impl BackupTarget {
    pub fn retention(&self) -> Retention {
        Retention {
            hourly: self.keep_hourly,
            daily: self.keep_daily,
            weekly: self.keep_weekly,
            monthly: self.keep_monthly,
            yearly: self.keep_yearly,
        }
    }

    /// Archives are named `<host>-s<schedule id>-<timestamp>` so a schedule's
    /// prune can match its own archives only. Without a schedule they keep the
    /// legacy `<host>-<timestamp>` shape.
    pub fn archive_name(&self, timestamp: impl std::fmt::Display) -> String {
        let hostname = &self.hostname;
        self.schedule_id.map_or_else(
            || format!("{hostname}-{timestamp}"),
            |id| format!("{hostname}-s{id}-{timestamp}"),
        )
    }

    /// Matches this schedule's archives; `None` without a schedule.
    pub fn schedule_archive_glob(&self) -> Option<String> {
        let hostname = &self.hostname;
        self.schedule_id.map(|id| format!("*{hostname}-s{id}-*"))
    }

    /// Matches archives named before schedule ids were added to the name.
    /// Spelling out the date keeps it from matching the newer names.
    pub fn legacy_archive_glob(&self) -> String {
        let hostname = &self.hostname;
        format!("*{hostname}-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*")
    }

    /// Construct a maintenance target (check/verify) from an existing backup target,
    /// overriding the hostname and clearing backup-specific fields irrelevant to
    /// maintenance operations.
    pub fn for_maintenance(base: &Self, hostname: String) -> Self {
        Self {
            target_name: base.target_name.clone(),
            schedule_id: base.schedule_id,
            repo_path: base.repo_path.clone(),
            ssh_user: base.ssh_user.clone(),
            ssh_host: base.ssh_host.clone(),
            ssh_port: base.ssh_port,
            ssh_host_key: base.ssh_host_key.clone(),
            passphrase: base.passphrase.clone(),
            compression: base.compression.clone(),
            rate_limit_kbps: base.rate_limit_kbps,
            accept_relocation: base.accept_relocation,
            hostname,
            ..Self::default()
        }
    }
}
