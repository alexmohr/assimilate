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
    let client = db::get_client_by_hostname(pool, hostname).await?;

    let global_excludes: Vec<String> = db::list_global_excludes(pool)
        .await?
        .into_iter()
        .map(|e| e.pattern)
        .collect();

    let schedule_rows = db::list_schedules_for_client(pool, client.id).await?;

    let mut repo_map: std::collections::HashMap<
        i64,
        (db::RepoWithPassphraseRow, Vec<ScheduleConfig>),
    > = std::collections::HashMap::new();

    for schedule in schedule_rows {
        let repo_id = schedule.repo_id;

        if !repo_map.contains_key(&repo_id) {
            let repo_rows = db::list_repos_for_client(pool, client.id).await?;
            for repo in repo_rows {
                repo_map
                    .entry(repo.id)
                    .or_insert_with(|| (repo, Vec::new()));
            }
        }

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
            db::list_backup_sources_for_schedule_client(pool, schedule.id, client.id).await?;

        if backup_sources.is_empty() {
            backup_sources = db::list_backup_sources_for_schedule(pool, schedule.id).await?;
        }

        if backup_sources.is_empty() {
            backup_sources.extend(client.default_backup_paths.iter().cloned());
        }

        let mut exclude_patterns: Vec<String> = Vec::new();
        if !schedule.ignore_global_excludes {
            exclude_patterns.extend(global_excludes.iter().cloned());
        }
        exclude_patterns.extend(client.default_exclude_patterns.iter().cloned());
        exclude_patterns.extend(schedule.exclude_patterns.iter().cloned());

        let mut seen = std::collections::HashSet::new();
        exclude_patterns.retain(|p| seen.insert(p.clone()));

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
            keep_daily,
            keep_weekly,
            keep_monthly,
            keep_yearly,
            compact_enabled: schedule.compact_enabled,
            pre_backup_commands: serde_json::from_str(&schedule.pre_backup_commands)
                .inspect_err(|e| {
                    tracing::warn!(
                        schedule_id = schedule.id,
                        error = %e,
                        "failed to parse pre_backup_commands, defaulting to empty"
                    );
                })
                .unwrap_or_default(),
            post_backup_commands: serde_json::from_str(&schedule.post_backup_commands)
                .inspect_err(|e| {
                    tracing::warn!(
                        schedule_id = schedule.id,
                        error = %e,
                        "failed to parse post_backup_commands, defaulting to empty"
                    );
                })
                .unwrap_or_default(),
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

        repos.push(RepoConfig {
            repo_id: RepoId(repo.id),
            name: repo.name,
            repo_path: repo.repo_path,
            ssh_user: repo.ssh_user,
            ssh_host: repo.ssh_host,
            ssh_port,
            passphrase,
            compression,
            enabled: repo.enabled,
            accept_relocation: repo.relocation_pending,
            schedules,
        });
    }

    Ok(AgentConfig {
        client_hostname: hostname.to_string(),
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

fn schedule_type_from_str(s: &str) -> Result<ScheduleType, ApiError> {
    s.parse()
        .map_err(|e| ApiError::Internal(format!("invalid schedule type in database: {e}")))
}
