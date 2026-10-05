// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::io::Write;

use shared::{ssh::known_hosts_host, types::RepoConfig, vm::VmSnapshotConfig};
use tracing::{error, warn};

use crate::{
    backup::BackupTarget,
    ssh_forward::{SshForwardError, SshForwardSocket, run_ssh_forward},
};

pub(super) struct RepoTransport {
    _ssh_forward: Option<SshForwardSocket>,
    _known_hosts: Option<tempfile::NamedTempFile>,
}

pub(super) async fn setup_ssh_forward(
    target: &mut BackupTarget,
    hostname: &str,
    server_url: &str,
    token: &str,
) -> RepoTransport {
    let known_hosts = match write_known_hosts(target) {
        Ok(known_hosts) => known_hosts,
        Err(e) => {
            error!(error = %e, "failed to create pinned SSH known_hosts file");
            None
        }
    };

    let socket = match SshForwardSocket::create() {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "ssh forward: failed to create socket, proceeding without");
            return RepoTransport {
                _ssh_forward: None,
                _known_hosts: known_hosts,
            };
        }
    };

    if let Err(e) = run_ssh_forward(&socket, server_url, hostname, token).await {
        match e {
            SshForwardError::Url(msg) => warn!(error = %msg, "ssh forward: bad relay url"),
            other => warn!(error = %other, "ssh forward: setup failed, proceeding without"),
        }
        return RepoTransport {
            _ssh_forward: None,
            _known_hosts: known_hosts,
        };
    }

    target.ssh_auth_sock = Some(socket.socket_path.clone());
    RepoTransport {
        _ssh_forward: Some(socket),
        _known_hosts: known_hosts,
    }
}

pub(super) fn write_known_hosts(
    target: &mut BackupTarget,
) -> Result<Option<tempfile::NamedTempFile>, std::io::Error> {
    if target.ssh_host_key.is_empty() {
        return Ok(None);
    }

    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(
        file,
        "{} {}",
        known_hosts_host(&target.ssh_host, target.ssh_port),
        target.ssh_host_key
    )?;
    file.flush()?;
    target.known_hosts_path = Some(file.path().to_path_buf());
    Ok(Some(file))
}

pub fn backup_target_from_repo(
    repo: &RepoConfig,
    hostname: &str,
    schedule_id: Option<i64>,
    vm_snapshot: &VmSnapshotConfig,
) -> BackupTarget {
    let schedule = schedule_id
        .and_then(|id| repo.schedules.iter().find(|s| s.id == id))
        .or_else(|| repo.schedules.first());
    // Staging needs both halves: the schedule opting in, and the host having
    // it switched on at all.
    let schedule_wants_vms = schedule.is_some_and(|s| s.vm_snapshot_enabled);
    let stages_vms = schedule_wants_vms && vm_snapshot.enabled;
    if schedule_wants_vms && !vm_snapshot.enabled {
        // Both halves are required, so this is not a bug - but silently
        // backing up no virtual machines is the one outcome an operator who
        // ticked the box would not expect, and nothing else would ever say so.
        warn!(
            schedule_id = ?schedule.map(|s| s.id),
            "the schedule asks for virtual machines but this host has staging turned off, so \
             none will be included"
        );
    }
    let mut backup_sources = schedule.map_or_else(Vec::new, |s| s.backup_sources.clone());
    // The staging directory is part of the backup by virtue of staging, so the
    // operator never has to remember to list it as a source.
    if stages_vms
        && !backup_sources
            .iter()
            .any(|source| source == &vm_snapshot.staging_dir)
    {
        backup_sources.push(vm_snapshot.staging_dir.clone());
    }
    BackupTarget {
        target_name: repo.name.clone(),
        schedule_id: schedule.map(|s| s.id),
        repo_path: repo.repo_path.clone(),
        ssh_user: repo.ssh_user.clone(),
        ssh_host: repo.ssh_host.clone(),
        ssh_port: repo.ssh_port,
        ssh_host_key: repo.ssh_host_key.clone(),
        known_hosts_path: None,
        passphrase: repo.passphrase.clone(),
        hostname: hostname.to_owned(),
        compression: repo.compression.clone(),
        backup_sources,
        rate_limit_kbps: schedule.and_then(|s| s.rate_limit_kbps),
        keep_hourly: schedule.map_or(24, |s| s.keep_hourly),
        keep_daily: schedule.map_or(7, |s| s.keep_daily),
        keep_weekly: schedule.map_or(4, |s| s.keep_weekly),
        keep_monthly: schedule.map_or(6, |s| s.keep_monthly),
        keep_yearly: schedule.map_or(0, |s| s.keep_yearly),
        compact_enabled: schedule.is_none_or(|s| s.compact_enabled),
        vm_snapshot: stages_vms.then(|| vm_snapshot.clone()),
        pre_backup_commands: schedule.map_or_else(Vec::new, |s| s.pre_backup_commands.clone()),
        post_backup_commands: schedule.map_or_else(Vec::new, |s| s.post_backup_commands.clone()),
        hook_timeout_seconds: schedule.map_or(60, |s| s.hook_timeout_seconds),
        skip_targets: Vec::new(),
        exclude_patterns: schedule.map_or_else(Vec::new, |s| s.exclude_patterns.clone()),
        include_patterns: schedule.map_or_else(Vec::new, |s| s.include_patterns.clone()),
        file_change_patterns: schedule.map_or_else(Vec::new, |s| s.file_change_patterns.clone()),
        ssh_auth_sock: None,
        canary_enabled: schedule.is_some_and(|s| s.canary_enabled),
        accept_relocation: repo.accept_relocation,
    }
}
