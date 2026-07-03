// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    protocol::ServerToAgent,
    types::{AgentConfig, RepoConfig, RepoId, ScheduleConfig, ScheduleType},
};
use sqlx::PgPool;

use crate::{
    AppState,
    db::{self, compression_from_str},
    error::ApiError,
};

pub async fn assemble_config(
    pool: &PgPool,
    encryption_key: &[u8; 32],
    hostname: &str,
) -> Result<AgentConfig, ApiError> {
    let agent = db::get_agent_by_hostname(pool, hostname).await?;

    let global_excludes = parse_raw_excludes(&db::get_global_excludes_raw(pool).await?);

    let schedule_rows = db::list_schedules_for_agent(pool, agent.id).await?;

    let mut repo_map: std::collections::HashMap<
        i64,
        (db::RepoWithPassphraseRow, Vec<ScheduleConfig>),
    > = std::collections::HashMap::new();

    for schedule in schedule_rows {
        let Some(repo_id) = schedule.repo_id else {
            continue;
        };

        if !repo_map.contains_key(&repo_id) {
            let repo_rows = db::list_repos_for_agent(pool, agent.id).await?;
            for repo in repo_rows {
                repo_map
                    .entry(repo.id)
                    .or_insert_with(|| (repo, Vec::new()));
            }
        }

        let keep_hourly = u32::try_from(schedule.keep_hourly).map_err(|_| {
            ApiError::Internal(format!(
                "keep_hourly {} out of u32 range",
                schedule.keep_hourly
            ))
        })?;
        let keep_daily = u32::try_from(schedule.keep_daily).map_err(|_| {
            ApiError::Internal(format!(
                "keep_daily {} out of u32 range",
                schedule.keep_daily
            ))
        })?;
        let keep_weekly = u32::try_from(schedule.keep_weekly).map_err(|_| {
            ApiError::Internal(format!(
                "keep_weekly {} out of u32 range",
                schedule.keep_weekly
            ))
        })?;
        let keep_monthly = u32::try_from(schedule.keep_monthly).map_err(|_| {
            ApiError::Internal(format!(
                "keep_monthly {} out of u32 range",
                schedule.keep_monthly
            ))
        })?;
        let keep_yearly = u32::try_from(schedule.keep_yearly).map_err(|_| {
            ApiError::Internal(format!(
                "keep_yearly {} out of u32 range",
                schedule.keep_yearly
            ))
        })?;
        let rate_limit_kbps = match schedule.rate_limit_kbps {
            Some(rate_limit_kbps) => Some(u32::try_from(rate_limit_kbps).map_err(|_| {
                ApiError::Internal(format!(
                    "rate_limit_kbps {} out of u32 range",
                    rate_limit_kbps
                ))
            })?),
            None => None,
        };

        let mut backup_sources =
            db::list_backup_sources_for_schedule_agent(pool, schedule.id, agent.id).await?;

        if backup_sources.is_empty() {
            backup_sources = db::list_backup_sources_for_schedule(pool, schedule.id).await?;
        }

        if backup_sources.is_empty() {
            backup_sources.extend(agent.default_backup_paths.iter().cloned());
        }

        let per_agent_excludes_raw =
            db::get_per_agent_excludes_raw(pool, schedule.id, agent.id).await?;
        let effective_excludes_raw = per_agent_excludes_raw
            .as_deref()
            .unwrap_or(&schedule.exclude_patterns_raw);

        let mut exclude_patterns: Vec<String> = Vec::new();
        if !schedule.ignore_global_excludes {
            exclude_patterns.extend(global_excludes.iter().cloned());
        }
        exclude_patterns.extend(agent.default_exclude_patterns.iter().cloned());
        exclude_patterns.extend(parse_raw_excludes(effective_excludes_raw));

        let mut seen = std::collections::HashSet::new();
        exclude_patterns.retain(|p| seen.insert(p.clone()));

        let per_agent_cmds = db::get_per_agent_commands(pool, schedule.id, agent.id).await?;

        let per_agent_file_change_patterns_raw =
            db::get_per_agent_file_change_patterns_raw(pool, schedule.id, agent.id).await?;
        let effective_file_change_patterns_raw = per_agent_file_change_patterns_raw
            .as_deref()
            .unwrap_or(&schedule.file_change_patterns_raw);
        // Schedule-level (or per-agent-schedule override) patterns are checked
        // first, since `filter_file_change_warnings` on the agent uses
        // first-match-wins; the agent's own defaults are appended as a
        // fallback so they only apply to warnings the schedule didn't cover.
        let mut file_change_patterns =
            parse_raw_file_change_patterns(effective_file_change_patterns_raw);
        file_change_patterns.extend(parse_raw_file_change_patterns(
            &agent.default_file_change_patterns_raw,
        ));

        let schedule_config = ScheduleConfig {
            id: schedule.id,
            schedule_type: schedule_type_from_str(&schedule.schedule_type)?,
            cron_expression: schedule.cron_expression,
            enabled: schedule.enabled,
            backup_sources,
            rate_limit_kbps,
            canary_enabled: schedule.canary_enabled,
            exclude_patterns,
            ignore_global_excludes: schedule.ignore_global_excludes,
            keep_hourly,
            keep_daily,
            keep_weekly,
            keep_monthly,
            keep_yearly,
            compact_enabled: schedule.compact_enabled,
            file_change_patterns,
            pre_backup_commands: {
                let agent_defaults: Vec<String> =
                    serde_json::from_str(&agent.default_pre_backup_commands)
                        .inspect_err(|e| {
                            tracing::warn!(
                                agent_id = agent.id,
                                error = %e,
                                "failed to parse agent default_pre_backup_commands"
                            );
                        })
                        .unwrap_or_default();
                let effective_pre = per_agent_cmds
                    .as_ref()
                    .map(|c| c.pre_backup_commands.as_str())
                    .unwrap_or(&schedule.pre_backup_commands);
                let schedule_cmds: Vec<String> = serde_json::from_str(effective_pre)
                    .inspect_err(|e| {
                        tracing::warn!(
                            schedule_id = schedule.id,
                            error = %e,
                            "failed to parse pre_backup_commands, defaulting to empty"
                        );
                    })
                    .unwrap_or_default();
                agent_defaults.into_iter().chain(schedule_cmds).collect()
            },
            post_backup_commands: {
                let effective_post = per_agent_cmds
                    .as_ref()
                    .map(|c| c.post_backup_commands.as_str())
                    .unwrap_or(&schedule.post_backup_commands);
                let schedule_cmds: Vec<String> = serde_json::from_str(effective_post)
                    .inspect_err(|e| {
                        tracing::warn!(
                            schedule_id = schedule.id,
                            error = %e,
                            "failed to parse post_backup_commands, defaulting to empty"
                        );
                    })
                    .unwrap_or_default();
                let agent_defaults: Vec<String> =
                    serde_json::from_str(&agent.default_post_backup_commands)
                        .inspect_err(|e| {
                            tracing::warn!(
                                agent_id = agent.id,
                                error = %e,
                                "failed to parse agent default_post_backup_commands"
                            );
                        })
                        .unwrap_or_default();
                schedule_cmds.into_iter().chain(agent_defaults).collect()
            },
        };

        if let Some((_, schedules)) = repo_map.get_mut(&repo_id) {
            schedules.push(schedule_config);
        }
    }

    let mut repos = Vec::with_capacity(repo_map.len());
    for (_, (repo, schedules)) in repo_map {
        let passphrase =
            shared::crypto::decrypt_passphrase(&repo.passphrase_encrypted, encryption_key)
                .map_err(|e| ApiError::Internal(format!("failed to decrypt passphrase: {e}")))?;

        let compression = compression_from_str(&repo.compression)?;

        let ssh_port = u16::try_from(repo.ssh_port).map_err(|_| {
            ApiError::Internal(format!("ssh_port {} out of u16 range", repo.ssh_port))
        })?;

        let ssh_host_key = match repo.ssh_host_key {
            Some(ssh_host_key) => ssh_host_key,
            None => {
                let ssh_host_key = crate::ssh::scan_host_key(&repo.ssh_host, ssh_port)
                    .await
                    .map_err(|e| {
                        ApiError::BadGateway(format!(
                            "failed to obtain SSH host key for {}:{}: {e}",
                            repo.ssh_host, ssh_port
                        ))
                    })?;
                db::update_repo_ssh_host_key(pool, repo.id, &ssh_host_key).await?;
                ssh_host_key
            }
        };

        repos.push(RepoConfig {
            repo_id: RepoId(repo.id),
            name: repo.name,
            repo_path: repo.repo_path,
            ssh_user: repo.ssh_user,
            ssh_host: repo.ssh_host,
            ssh_port,
            ssh_host_key,
            passphrase,
            compression,
            enabled: repo.enabled,
            accept_relocation: repo.relocation_pending,
            schedules,
        });
    }

    Ok(AgentConfig {
        agent_hostname: hostname.to_string(),
        skip_targets: Vec::new(),
        repos,
    })
}

pub async fn push_config_to_agent(state: &AppState, hostname: &str) {
    match assemble_config(&state.pool, &state.encryption_key, hostname).await {
        Ok(config) => {
            let msg = ServerToAgent::ConfigUpdate(config);
            if let Err(e) = state.registry.send_to(hostname, msg).await {
                tracing::debug!(
                    hostname = %hostname,
                    error = %e,
                    "agent not connected, config push skipped"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                hostname = %hostname,
                error = %e,
                "failed to assemble config for push"
            );
        }
    }
}

pub async fn push_config_to_all_schedule_targets(state: &AppState, schedule_id: i64) {
    let hostnames = match db::get_schedule_target_hostnames(&state.pool, schedule_id).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(
                schedule_id,
                error = %e,
                "failed to get schedule target hostnames for config push"
            );
            return;
        }
    };
    for hostname in &hostnames {
        push_config_to_agent(state, hostname).await;
    }
}

fn parse_raw_excludes(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn schedule_type_from_str(s: &str) -> Result<ScheduleType, ApiError> {
    s.parse()
        .map_err(|e| ApiError::Internal(format!("invalid schedule type in database: {e}")))
}

// Mirrors `parseFileChangePatterns` in
// `frontend/src/utils/fileChangePatterns.ts` - keep the two grammars in
// sync when changing either one.
fn parse_raw_file_change_patterns(raw: &str) -> Vec<shared::types::FileChangePattern> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let parts: Vec<&str> = line.rsplitn(2, ' ').collect();
            let (path, action_str) =
                if parts.len() == 2 && matches!(parts[0], "ignore" | "warn" | "fatal") {
                    (parts[1].trim(), parts[0])
                } else {
                    (line, "warn")
                };
            let action = action_str.parse().unwrap_or_default();
            shared::types::FileChangePattern {
                path: path.to_string(),
                action,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_raw_excludes, parse_raw_file_change_patterns};

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_raw_excludes("").is_empty());
    }

    #[test]
    fn blank_lines_stripped() {
        let input = "*.log\n\n*.tmp\n\n";
        assert_eq!(parse_raw_excludes(input), vec!["*.log", "*.tmp"]);
    }

    #[test]
    fn comment_lines_stripped() {
        let input = "# cache files\n*.cache\n# runtime\n/proc";
        assert_eq!(parse_raw_excludes(input), vec!["*.cache", "/proc"]);
    }

    #[test]
    fn leading_trailing_whitespace_trimmed_per_line() {
        let input = "  *.log  \n\t/proc\t";
        assert_eq!(parse_raw_excludes(input), vec!["*.log", "/proc"]);
    }

    #[test]
    fn whitespace_only_lines_stripped() {
        let input = "*.log\n   \n\t\n/proc";
        assert_eq!(parse_raw_excludes(input), vec!["*.log", "/proc"]);
    }

    #[test]
    fn mixed_content_produces_only_effective_patterns() {
        let input = "# System paths\n/proc\n/sys\n\n# Cache\n*.cache\npp:__pycache__\n\n# \
                     Downloads\n/home/*/Downloads";
        assert_eq!(
            parse_raw_excludes(input),
            vec![
                "/proc",
                "/sys",
                "*.cache",
                "pp:__pycache__",
                "/home/*/Downloads"
            ]
        );
    }

    #[test]
    fn order_is_preserved() {
        let input = "/z\n/a\n/m";
        assert_eq!(parse_raw_excludes(input), vec!["/z", "/a", "/m"]);
    }

    #[test]
    fn inline_comments_are_not_stripped() {
        // Only full-line comments (trimmed line starts with #) are stripped;
        // inline # is part of a valid borg pattern.
        let input = "re:/tmp/[^/]+\\.sock$";
        assert_eq!(parse_raw_excludes(input), vec!["re:/tmp/[^/]+\\.sock$"]);
    }

    #[test]
    fn parse_raw_file_change_patterns_defaults_to_warn() {
        let input = "/etc/passwd\n/var/log";
        let patterns = parse_raw_file_change_patterns(input);
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].path, "/etc/passwd");
        assert_eq!(patterns[0].action, shared::types::FileChangeAction::Warn);
        assert_eq!(patterns[1].path, "/var/log");
        assert_eq!(patterns[1].action, shared::types::FileChangeAction::Warn);
    }

    #[test]
    fn parse_raw_file_change_patterns_with_actions() {
        let input = "/tmp ignore\n/etc warn\n/var/log fatal";
        let patterns = parse_raw_file_change_patterns(input);
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0].path, "/tmp");
        assert_eq!(patterns[0].action, shared::types::FileChangeAction::Ignore);
        assert_eq!(patterns[1].path, "/etc");
        assert_eq!(patterns[1].action, shared::types::FileChangeAction::Warn);
        assert_eq!(patterns[2].path, "/var/log");
        assert_eq!(patterns[2].action, shared::types::FileChangeAction::Fatal);
    }

    #[test]
    fn parse_raw_file_change_patterns_blank_and_comment_lines_stripped() {
        let input = "# comment\n/tmp ignore\n\n# another\n/var/log fatal";
        let patterns = parse_raw_file_change_patterns(input);
        assert_eq!(patterns.len(), 2);
    }

    #[test]
    fn parse_raw_file_change_patterns_empty_input() {
        let patterns = parse_raw_file_change_patterns("");
        assert!(patterns.is_empty());
    }
}
