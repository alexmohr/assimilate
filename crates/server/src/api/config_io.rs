// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use chrono::Utc;
use shared::responses::{
    ConfigExportResponse as ConfigExport, HostExportResponse as HostExport,
    ImportResultResponse as ImportResult, ScheduleExportResponse as ScheduleExport,
    ScheduleTargetExportResponse as ScheduleTargetExport,
};

use super::auth::RequireAdmin;
use crate::{
    AppState,
    db::{self, IMPORTED_TOKEN_HASH, ScheduleParams},
    error::{ApiError, ApiJson},
};

const EXPORT_VERSION: u32 = 1;

#[utoipa::path(
    get,
    path = "/api/config/export",
    tag = "Config",
    operation_id = "exportConfig",
    summary = "Export all hosts and schedules as a portable JSON snapshot",
    responses(
        (status = 200, description = "Config export", body = ConfigExport),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn export_config(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<ConfigExport>, ApiError> {
    let agents = db::list_agents(&state.pool, false).await?;
    let agent_id_to_hostname: HashMap<i64, &str> =
        agents.iter().map(|c| (c.id, c.hostname.as_str())).collect();

    let mut hosts = Vec::new();
    for agent in &agents {
        if agent.agent_token_hash == IMPORTED_TOKEN_HASH {
            continue;
        }
        let patterns = db::patterns::list_patterns_for_agent(&state.pool, agent.id).await?;
        hosts.push(HostExport {
            hostname: agent.hostname.clone(),
            display_name: agent.display_name.clone(),
            default_backup_paths: agent.default_backup_paths.clone(),
            default_exclude_patterns: agent.default_exclude_patterns.clone(),
            default_pre_backup_commands: agent.default_pre_backup_commands.clone(),
            default_post_backup_commands: agent.default_post_backup_commands.clone(),
            default_file_change_patterns_raw: agent.default_file_change_patterns_raw.clone(),
            hostname_patterns: patterns.into_iter().map(|p| p.pattern).collect(),
        });
    }

    let schedule_rows = db::list_schedules(&state.pool).await?;
    let repos = db::list_all_repos(&state.pool).await?;
    let repo_id_to_name: HashMap<i64, &str> =
        repos.iter().map(|r| (r.id, r.name.as_str())).collect();

    let mut schedules = Vec::new();
    for sched in &schedule_rows {
        let repo_name = sched
            .repo_id
            .and_then(|rid| repo_id_to_name.get(&rid).copied())
            .map(str::to_owned);

        let target_rows = db::list_schedule_targets(&state.pool, sched.id).await?;
        let backup_sources = db::list_backup_sources_for_schedule(&state.pool, sched.id).await?;
        let per_agent_sources =
            db::list_all_per_agent_backup_sources_for_schedule(&state.pool, sched.id).await?;
        let per_agent_excludes =
            db::list_all_per_agent_excludes_for_schedule(&state.pool, sched.id).await?;

        let per_agent_sources_map: HashMap<i64, &Vec<String>> = per_agent_sources
            .iter()
            .map(|s| (s.agent_id, &s.paths))
            .collect();
        let per_agent_excludes_map: HashMap<i64, &str> = per_agent_excludes
            .iter()
            .map(|e| (e.agent_id, e.raw_text.as_str()))
            .collect();
        let per_agent_file_change_patterns =
            db::list_all_per_agent_file_change_patterns_for_schedule(&state.pool, sched.id).await?;
        let per_agent_file_change_patterns_map: HashMap<i64, &str> = per_agent_file_change_patterns
            .iter()
            .map(|f| (f.agent_id, f.raw_text.as_str()))
            .collect();

        let targets = target_rows
            .iter()
            .filter_map(|t| {
                let hostname = agent_id_to_hostname.get(&t.agent_id).copied()?.to_owned();
                Some(ScheduleTargetExport {
                    hostname,
                    execution_order: t.execution_order,
                    backup_sources: per_agent_sources_map
                        .get(&t.agent_id)
                        .map(|v| (*v).clone())
                        .unwrap_or_default(),
                    exclude_patterns: per_agent_excludes_map
                        .get(&t.agent_id)
                        .copied()
                        .unwrap_or("")
                        .to_owned(),
                    file_change_patterns: per_agent_file_change_patterns_map
                        .get(&t.agent_id)
                        .copied()
                        .unwrap_or("")
                        .to_owned(),
                })
            })
            .collect();

        let pre_backup_commands =
            serde_json::from_str(&sched.pre_backup_commands).unwrap_or_default();
        let post_backup_commands =
            serde_json::from_str(&sched.post_backup_commands).unwrap_or_default();

        schedules.push(ScheduleExport {
            name: sched.name.clone(),
            schedule_type: sched.schedule_type.parse().unwrap_or_default(),
            cron_expression: sched.cron_expression.clone(),
            enabled: sched.enabled,
            canary_enabled: sched.canary_enabled,
            execution_mode: sched.execution_mode.parse().unwrap_or_default(),
            on_failure: sched.on_failure.parse().unwrap_or_default(),
            exclude_patterns_raw: sched.exclude_patterns_raw.clone(),
            file_change_patterns_raw: sched.file_change_patterns_raw.clone(),
            ignore_global_excludes: sched.ignore_global_excludes,
            keep_hourly: sched.keep_hourly,
            keep_daily: sched.keep_daily,
            keep_weekly: sched.keep_weekly,
            keep_monthly: sched.keep_monthly,
            keep_yearly: sched.keep_yearly,
            compact_enabled: sched.compact_enabled,
            rate_limit_kbps: sched.rate_limit_kbps,
            pre_backup_commands,
            post_backup_commands,
            repo_name,
            backup_sources,
            targets,
        });
    }

    Ok(Json(ConfigExport {
        version: EXPORT_VERSION,
        exported_at: Utc::now(),
        hosts,
        schedules,
    }))
}

#[utoipa::path(
    post,
    path = "/api/config/import",
    tag = "Config",
    operation_id = "importConfig",
    summary = "Import hosts and schedules from a JSON export",
    request_body = ConfigExport,
    responses(
        (status = 200, description = "Import results", body = ImportResult),
        (status = 400, description = "Invalid version or format"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn import_config(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    ApiJson(payload): ApiJson<ConfigExport>,
) -> Result<Json<ImportResult>, ApiError> {
    if payload.version != EXPORT_VERSION {
        return Err(ApiError::BadRequest(format!(
            "unsupported export version: {}",
            payload.version
        )));
    }

    let mut result = ImportResult {
        hosts_created: 0,
        hosts_updated: 0,
        schedules_created: 0,
        warnings: Vec::new(),
    };

    let existing_agents = db::list_agents(&state.pool, true).await?;
    let mut hostname_to_id: HashMap<String, i64> = existing_agents
        .iter()
        .map(|a| (a.hostname.clone(), a.id))
        .collect();

    for host in &payload.hosts {
        if let Some(&existing_id) = hostname_to_id.get(&host.hostname) {
            db::update_agent(
                &state.pool,
                &host.hostname,
                &host.hostname,
                db::AgentDefaults {
                    display_name: host.display_name.as_deref(),
                    default_backup_paths: &host.default_backup_paths,
                    default_exclude_patterns: &host.default_exclude_patterns,
                    default_pre_backup_commands: &host.default_pre_backup_commands,
                    default_post_backup_commands: &host.default_post_backup_commands,
                    default_file_change_patterns_raw: &host.default_file_change_patterns_raw,
                },
            )
            .await?;

            let existing_patterns =
                db::patterns::list_patterns_for_agent(&state.pool, existing_id).await?;
            let existing_set: HashSet<&str> = existing_patterns
                .iter()
                .map(|p| p.pattern.as_str())
                .collect();
            for pattern in &host.hostname_patterns {
                if !existing_set.contains(pattern.as_str()) {
                    db::patterns::add_hostname_pattern(&state.pool, existing_id, pattern).await?;
                }
            }

            result.hosts_updated += 1;
        } else {
            let agent = db::insert_agent_with_paths(
                &state.pool,
                &host.hostname,
                IMPORTED_TOKEN_HASH,
                db::AgentDefaults {
                    display_name: host.display_name.as_deref(),
                    default_backup_paths: &host.default_backup_paths,
                    default_exclude_patterns: &host.default_exclude_patterns,
                    default_pre_backup_commands: &host.default_pre_backup_commands,
                    default_post_backup_commands: &host.default_post_backup_commands,
                    default_file_change_patterns_raw: &host.default_file_change_patterns_raw,
                },
            )
            .await?;
            for pattern in &host.hostname_patterns {
                db::patterns::add_hostname_pattern(&state.pool, agent.id, pattern).await?;
            }
            hostname_to_id.insert(host.hostname.clone(), agent.id);
            result.hosts_created += 1;
        }
    }

    let repos = db::list_all_repos(&state.pool).await?;
    let repo_name_to_id: HashMap<&str, i64> =
        repos.iter().map(|r| (r.name.as_str(), r.id)).collect();

    for sched in &payload.schedules {
        let Some(repo_name) = &sched.repo_name else {
            result.warnings.push(format!(
                "skipped schedule {:?}: no repository assigned",
                sched.name
            ));
            continue;
        };

        let Some(&repo_id) = repo_name_to_id.get(repo_name.as_str()) else {
            result.warnings.push(format!(
                "skipped schedule {:?}: repository {:?} not found",
                sched.name, repo_name
            ));
            continue;
        };

        if sched.targets.is_empty() {
            result
                .warnings
                .push(format!("skipped schedule {:?}: no targets", sched.name));
            continue;
        }

        let mut target_ids: Vec<(i64, i32)> = Vec::new();
        for target in &sched.targets {
            let agent_id = if let Some(&cid) = hostname_to_id.get(&target.hostname) {
                cid
            } else {
                let agent = db::insert_agent(
                    &state.pool,
                    &target.hostname,
                    None,
                    IMPORTED_TOKEN_HASH,
                    None,
                )
                .await?;
                result.warnings.push(format!(
                    "created placeholder agent {:?} referenced by schedule {:?}",
                    target.hostname, sched.name
                ));
                hostname_to_id.insert(target.hostname.clone(), agent.id);
                agent.id
            };
            target_ids.push((agent_id, target.execution_order));
        }

        let pre_cmds_json =
            serde_json::to_string(&sched.pre_backup_commands).unwrap_or_else(|_| "[]".to_owned());
        let post_cmds_json =
            serde_json::to_string(&sched.post_backup_commands).unwrap_or_else(|_| "[]".to_owned());

        let params = ScheduleParams {
            name: &sched.name,
            schedule_type: &sched.schedule_type.to_string(),
            cron_expression: &sched.cron_expression,
            enabled: sched.enabled,
            canary_enabled: sched.canary_enabled,
            exclude_patterns_raw: &sched.exclude_patterns_raw,
            file_change_patterns_raw: &sched.file_change_patterns_raw,
            ignore_global_excludes: sched.ignore_global_excludes,
            keep_hourly: sched.keep_hourly,
            keep_daily: sched.keep_daily,
            keep_weekly: sched.keep_weekly,
            keep_monthly: sched.keep_monthly,
            keep_yearly: sched.keep_yearly,
            compact_enabled: sched.compact_enabled,
            rate_limit_kbps: sched.rate_limit_kbps,
            pre_backup_commands: &pre_cmds_json,
            post_backup_commands: &post_cmds_json,
            on_failure: &sched.on_failure.to_string(),
        };

        let new_sched = db::insert_schedule(&state.pool, repo_id, &params, None).await?;
        db::insert_schedule_targets(&state.pool, new_sched.id, &target_ids).await?;

        for (i, path) in sched.backup_sources.iter().enumerate() {
            let sort_order = i32::try_from(i).unwrap_or(0);
            db::insert_backup_source_for_schedule(&state.pool, new_sched.id, path, sort_order)
                .await?;
        }

        for target in &sched.targets {
            let Some(&agent_id) = hostname_to_id.get(&target.hostname) else {
                continue;
            };
            for (i, path) in target.backup_sources.iter().enumerate() {
                let sort_order = i32::try_from(i).unwrap_or(0);
                db::insert_backup_source_for_schedule_agent(
                    &state.pool,
                    new_sched.id,
                    agent_id,
                    path,
                    sort_order,
                )
                .await?;
            }
            if !target.exclude_patterns.is_empty() {
                db::upsert_per_agent_excludes_raw(
                    &state.pool,
                    new_sched.id,
                    agent_id,
                    &target.exclude_patterns,
                )
                .await?;
            }
            if !target.file_change_patterns.is_empty() {
                db::upsert_per_agent_file_change_patterns_raw(
                    &state.pool,
                    new_sched.id,
                    agent_id,
                    &target.file_change_patterns,
                )
                .await?;
            }
        }

        result.schedules_created += 1;
    }

    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use shared::types::{ExecutionMode, OnFailure, ScheduleType};

    use super::*;

    #[test]
    fn test_config_export_roundtrip() {
        let export = ConfigExport {
            version: 1,
            exported_at: Utc::now(),
            hosts: vec![HostExport {
                hostname: "test-host".to_string(),
                display_name: Some("Test Host".to_string()),
                default_backup_paths: vec!["/home".to_string()],
                default_exclude_patterns: vec!["*.tmp".to_string()],
                default_pre_backup_commands: "echo pre".to_string(),
                default_post_backup_commands: "echo post".to_string(),
                default_file_change_patterns_raw: "".to_string(),
                hostname_patterns: vec!["test-*".to_string()],
            }],
            schedules: vec![ScheduleExport {
                name: "test-schedule".to_string(),
                schedule_type: ScheduleType::Backup,
                cron_expression: "0 2 * * *".to_string(),
                enabled: true,
                canary_enabled: false,
                execution_mode: ExecutionMode::Sequential,
                on_failure: OnFailure::Stop,
                exclude_patterns_raw: "".to_string(),
                file_change_patterns_raw: "".to_string(),
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 0,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: vec![],
                post_backup_commands: vec![],
                repo_name: Some("test-repo".to_string()),
                backup_sources: vec!["/data".to_string()],
                targets: vec![ScheduleTargetExport {
                    hostname: "test-host".to_string(),
                    execution_order: 0,
                    backup_sources: vec![],
                    exclude_patterns: "".to_string(),
                    file_change_patterns: "".to_string(),
                }],
            }],
        };

        let json = serde_json::to_string_pretty(&export).expect("serialize");

        let deserialized: ConfigExport = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.version, export.version);
        assert_eq!(deserialized.hosts.len(), export.hosts.len());
        assert_eq!(deserialized.hosts[0].hostname, export.hosts[0].hostname);
        assert_eq!(deserialized.schedules.len(), export.schedules.len());
        assert_eq!(deserialized.schedules[0].name, export.schedules[0].name);
        assert_eq!(
            deserialized.schedules[0].targets.len(),
            export.schedules[0].targets.len()
        );
        assert_eq!(
            deserialized.schedules[0].targets[0].hostname,
            export.schedules[0].targets[0].hostname
        );
    }
}
