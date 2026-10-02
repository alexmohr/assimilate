// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Length caps on the strings an imported configuration carries, applied
//! before anything is written so an upload cannot store what the REST
//! create/update paths would refuse.

use shared::hooks::HookCommand;

use super::{ConfigExport, HostExport, RepoExport, ScheduleExport, ScheduleTargetExport};
use crate::{
    api::helpers::{self, MaxLen},
    error::ApiError,
};

/// Rejects the first string in `config` that exceeds its [`MaxLen`] cap,
/// naming it by its position in the upload (e.g. `repos[2].ssh_host`).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] naming the offending field.
pub(super) fn validate(config: &ConfigExport) -> Result<(), ApiError> {
    config
        .repos
        .iter()
        .enumerate()
        .try_for_each(|(i, repo)| validate_repo(repo, &format!("repos[{i}]")))?;
    config
        .hosts
        .iter()
        .enumerate()
        .try_for_each(|(i, host)| validate_host(host, &format!("hosts[{i}]")))?;
    config
        .schedules
        .iter()
        .enumerate()
        .try_for_each(|(i, schedule)| validate_schedule(schedule, &format!("schedules[{i}]")))
}

fn validate_repo(repo: &RepoExport, at: &str) -> Result<(), ApiError> {
    helpers::validate_max_len(&repo.name, &format!("{at}.name"), MaxLen::Name)?;
    helpers::validate_max_len(&repo.repo_path, &format!("{at}.repo_path"), MaxLen::Path)?;
    helpers::validate_max_len(&repo.ssh_user, &format!("{at}.ssh_user"), MaxLen::Name)?;
    helpers::validate_max_len(&repo.ssh_host, &format!("{at}.ssh_host"), MaxLen::Hostname)?;
    helpers::validate_opt_max_len(
        repo.sync_schedule.as_deref(),
        &format!("{at}.sync_schedule"),
        MaxLen::Name,
    )?;
    helpers::validate_opt_max_len(
        repo.ssh_host_key.as_deref(),
        &format!("{at}.ssh_host_key"),
        MaxLen::Text,
    )?;
    helpers::validate_each_max_len(&repo.tags, &format!("{at}.tags"), MaxLen::Name)
}

fn validate_host(host: &HostExport, at: &str) -> Result<(), ApiError> {
    helpers::validate_max_len(&host.hostname, &format!("{at}.hostname"), MaxLen::Hostname)?;
    helpers::validate_opt_max_len(
        host.display_name.as_deref(),
        &format!("{at}.display_name"),
        MaxLen::Name,
    )?;
    helpers::validate_opt_max_len(
        host.domain.as_deref(),
        &format!("{at}.domain"),
        MaxLen::Hostname,
    )?;
    helpers::validate_each_max_len(
        &host.default_backup_paths,
        &format!("{at}.default_backup_paths"),
        MaxLen::Path,
    )?;
    helpers::validate_each_max_len(
        &host.default_exclude_patterns,
        &format!("{at}.default_exclude_patterns"),
        MaxLen::Path,
    )?;
    validate_commands(
        &host.default_pre_backup_commands,
        &format!("{at}.default_pre_backup_commands"),
    )?;
    validate_commands(
        &host.default_post_backup_commands,
        &format!("{at}.default_post_backup_commands"),
    )?;
    helpers::validate_max_len(
        &host.default_file_change_patterns_raw,
        &format!("{at}.default_file_change_patterns_raw"),
        MaxLen::Text,
    )?;
    helpers::validate_each_max_len(
        &host.hostname_patterns,
        &format!("{at}.hostname_patterns"),
        MaxLen::Name,
    )
}

fn validate_schedule(schedule: &ScheduleExport, at: &str) -> Result<(), ApiError> {
    helpers::validate_max_len(&schedule.name, &format!("{at}.name"), MaxLen::Name)?;
    helpers::validate_max_len(
        &schedule.cron_expression,
        &format!("{at}.cron_expression"),
        MaxLen::Name,
    )?;
    helpers::validate_max_len(
        &schedule.exclude_patterns_raw,
        &format!("{at}.exclude_patterns_raw"),
        MaxLen::Text,
    )?;
    helpers::validate_max_len(
        &schedule.include_patterns_raw,
        &format!("{at}.include_patterns_raw"),
        MaxLen::Text,
    )?;
    helpers::validate_max_len(
        &schedule.file_change_patterns_raw,
        &format!("{at}.file_change_patterns_raw"),
        MaxLen::Text,
    )?;
    validate_commands(
        &schedule.pre_backup_commands,
        &format!("{at}.pre_backup_commands"),
    )?;
    validate_commands(
        &schedule.post_backup_commands,
        &format!("{at}.post_backup_commands"),
    )?;
    helpers::validate_each_max_len(
        &schedule.backup_sources,
        &format!("{at}.backup_sources"),
        MaxLen::Path,
    )?;
    helpers::validate_opt_max_len(
        schedule.repo_name.as_deref(),
        &format!("{at}.repo_name"),
        MaxLen::Name,
    )?;
    schedule
        .targets
        .iter()
        .enumerate()
        .try_for_each(|(i, target)| validate_target(target, &format!("{at}.targets[{i}]")))
}

fn validate_target(target: &ScheduleTargetExport, at: &str) -> Result<(), ApiError> {
    helpers::validate_max_len(
        &target.hostname,
        &format!("{at}.hostname"),
        MaxLen::Hostname,
    )?;
    helpers::validate_opt_max_len(
        target.domain.as_deref(),
        &format!("{at}.domain"),
        MaxLen::Hostname,
    )?;
    helpers::validate_each_max_len(
        &target.backup_sources,
        &format!("{at}.backup_sources"),
        MaxLen::Path,
    )?;
    helpers::validate_max_len(
        &target.exclude_patterns,
        &format!("{at}.exclude_patterns"),
        MaxLen::Text,
    )?;
    helpers::validate_max_len(
        &target.include_patterns,
        &format!("{at}.include_patterns"),
        MaxLen::Text,
    )?;
    helpers::validate_max_len(
        &target.file_change_patterns,
        &format!("{at}.file_change_patterns"),
        MaxLen::Text,
    )
}

fn validate_commands(commands: &[HookCommand], at: &str) -> Result<(), ApiError> {
    commands.iter().enumerate().try_for_each(|(i, command)| {
        helpers::validate_max_len(&command.command, &format!("{at}[{i}]"), MaxLen::Text)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::validate;
    use crate::{
        api::{config_io::ConfigExport, helpers::MaxLen},
        error::ApiError,
    };

    fn repo() -> Value {
        json!({
            "name": "offsite",
            "repo_path": "/srv/borg/offsite",
            "ssh_user": "borg",
            "ssh_host": "backup.example.com",
            "ssh_port": 22,
            "compression": "lz4",
            "encryption": "repokey-blake2",
            "enabled": true,
            "sync_schedule": null,
            "ssh_host_key": null,
            "tags": ["critical"],
        })
    }

    fn host() -> Value {
        json!({
            "hostname": "web-server-01",
            "display_name": "Web",
            "domain": "dc1.example.com",
            "default_backup_paths": ["/etc"],
            "default_exclude_patterns": ["*.tmp"],
            "default_pre_backup_commands": [{ "command": "true", "timeout_seconds": null }],
            "default_post_backup_commands": [],
            "hostname_patterns": ["web-server-*"],
        })
    }

    fn schedule() -> Value {
        json!({
            "name": "nightly",
            "schedule_type": "backup",
            "cron_expression": "0 2 * * *",
            "enabled": true,
            "canary_enabled": true,
            "execution_mode": "sequential",
            "on_failure": "continue",
            "exclude_patterns_raw": "",
            "ignore_global_excludes": false,
            "keep_hourly": 0,
            "keep_daily": 7,
            "keep_weekly": 4,
            "keep_monthly": 6,
            "keep_yearly": 1,
            "compact_enabled": true,
            "rate_limit_kbps": null,
            "pre_backup_commands": [],
            "post_backup_commands": [],
            "backup_sources": ["/var/www"],
            "targets": [{
                "hostname": "web-server-01",
                "execution_order": 0,
                "backup_sources": ["/srv"],
                "exclude_patterns": "",
            }],
            "repo_name": "offsite",
        })
    }

    fn config(repo: &Value, host: &Value, schedule: &Value) -> ConfigExport {
        serde_json::from_value(json!({
            "version": 1,
            "exported_at": "2026-01-01T00:00:00Z",
            "repos": [repo],
            "hosts": [host],
            "schedules": [schedule],
        }))
        .unwrap()
    }

    /// Sets `value[path...] = replacement`, where a numeric step indexes an array.
    fn with(mut value: Value, path: &[&str], replacement: Value) -> Value {
        let slot = path
            .iter()
            .fold(&mut value, |node, step| match step.parse::<usize>() {
                Ok(index) => &mut node[index],
                Err(_) => &mut node[*step],
            });
        *slot = replacement;
        value
    }

    fn over(max: MaxLen) -> Value {
        Value::String("a".repeat(max.chars().saturating_add(1)))
    }

    #[test]
    fn a_config_within_every_limit_is_accepted() {
        assert!(validate(&config(&repo(), &host(), &schedule())).is_ok());
    }

    #[derive(Clone, Copy)]
    enum Section {
        Repo,
        Host,
        Schedule,
    }

    #[test]
    fn each_over_limit_field_is_rejected_by_its_position() {
        let cases: [(Section, &[&str], MaxLen, &str); 10] = [
            (Section::Repo, &["name"], MaxLen::Name, "repos[0].name "),
            (
                Section::Repo,
                &["ssh_host"],
                MaxLen::Hostname,
                "repos[0].ssh_host ",
            ),
            (
                Section::Repo,
                &["tags", "0"],
                MaxLen::Name,
                "repos[0].tags[0] ",
            ),
            (
                Section::Host,
                &["hostname"],
                MaxLen::Hostname,
                "hosts[0].hostname ",
            ),
            (
                Section::Host,
                &["default_backup_paths", "0"],
                MaxLen::Path,
                "hosts[0].default_backup_paths[0] ",
            ),
            (
                Section::Host,
                &["default_pre_backup_commands", "0", "command"],
                MaxLen::Text,
                "hosts[0].default_pre_backup_commands[0] ",
            ),
            (
                Section::Host,
                &["hostname_patterns", "0"],
                MaxLen::Name,
                "hosts[0].hostname_patterns[0] ",
            ),
            (
                Section::Schedule,
                &["name"],
                MaxLen::Name,
                "schedules[0].name ",
            ),
            (
                Section::Schedule,
                &["backup_sources", "0"],
                MaxLen::Path,
                "schedules[0].backup_sources[0] ",
            ),
            (
                Section::Schedule,
                &["targets", "0", "exclude_patterns"],
                MaxLen::Text,
                "schedules[0].targets[0].exclude_patterns ",
            ),
        ];
        for (section, path, max, field) in cases {
            let (repo, host, schedule) = match section {
                Section::Repo => (with(repo(), path, over(max)), host(), schedule()),
                Section::Host => (repo(), with(host(), path, over(max)), schedule()),
                Section::Schedule => (repo(), host(), with(schedule(), path, over(max))),
            };
            let Err(ApiError::BadRequest(message)) = validate(&config(&repo, &host, &schedule))
            else {
                panic!("{field}over its limit must be a 400");
            };
            assert!(message.starts_with(field), "{field}: {message}");
        }
    }
}
