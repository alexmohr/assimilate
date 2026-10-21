// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::PathBuf;

use shared::{
    hooks::HookCommand,
    types::{Compression, FileChangePattern},
};

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
