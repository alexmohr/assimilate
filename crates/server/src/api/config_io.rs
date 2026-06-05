// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::auth::RequireAdmin;
use crate::{
    AppState,
    db::{self, ScheduleParams},
    error::{ApiError, ApiJson},
};

const EXPORT_VERSION: u32 = 1;
const IMPORTED_TOKEN: &str = "imported:no-auth";

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HostExport {
    pub hostname: String,
    pub display_name: Option<String>,
    pub default_backup_paths: Vec<String>,
    pub default_exclude_patterns: Vec<String>,
    pub hostname_patterns: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleTargetExport {
    pub hostname: String,
    pub execution_order: i32,
    pub backup_sources: Vec<String>,
    pub exclude_patterns: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleExport {
    pub name: String,
    pub schedule_type: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub canary_enabled: bool,
    pub execution_mode: String,
    pub on_failure: String,
    pub exclude_patterns_raw: String,
    pub ignore_global_excludes: bool,
    pub keep_hourly: i32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
    pub keep_yearly: i32,
    pub compact_enabled: bool,
    pub rate_limit_kbps: Option<i32>,
    pub pre_backup_commands: Vec<String>,
    pub post_backup_commands: Vec<String>,
    pub repo_name: Option<String>,
    pub backup_sources: Vec<String>,
    pub targets: Vec<ScheduleTargetExport>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConfigExport {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub hosts: Vec<HostExport>,
    pub schedules: Vec<ScheduleExport>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ImportResult {
    pub hosts_created: u32,
    pub hosts_updated: u32,
    pub schedules_created: u32,
    pub warnings: Vec<String>,
}

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
    let clients = db::list_clients(&state.pool, false).await?;
    let client_id_to_hostname: HashMap<i64, &str> = clients
        .iter()
        .map(|c| (c.id, c.hostname.as_str()))
        .collect();

    let mut hosts = Vec::new();
    for client in &clients {
        if client.agent_token_hash == IMPORTED_TOKEN {
            continue;
        }
        let patterns = db::patterns::list_patterns_for_client(&state.pool, client.id).await?;
        hosts.push(HostExport {
            hostname: client.hostname.clone(),
            display_name: client.display_name.clone(),
            default_backup_paths: client.default_backup_paths.clone(),
            default_exclude_patterns: client.default_exclude_patterns.clone(),
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
        let per_host_sources =
            db::list_all_per_host_backup_sources_for_schedule(&state.pool, sched.id).await?;
        let per_host_excludes =
            db::list_all_per_host_excludes_for_schedule(&state.pool, sched.id).await?;

        let per_host_sources_map: HashMap<i64, &Vec<String>> = per_host_sources
            .iter()
            .map(|s| (s.client_id, &s.paths))
            .collect();
        let per_host_excludes_map: HashMap<i64, &str> = per_host_excludes
            .iter()
            .map(|e| (e.client_id, e.raw_text.as_str()))
            .collect();

        let targets = target_rows
            .iter()
            .filter_map(|t| {
                let hostname = client_id_to_hostname.get(&t.client_id).copied()?.to_owned();
                Some(ScheduleTargetExport {
                    hostname,
                    execution_order: t.execution_order,
                    backup_sources: per_host_sources_map
                        .get(&t.client_id)
                        .map(|v| (*v).clone())
                        .unwrap_or_default(),
                    exclude_patterns: per_host_excludes_map
                        .get(&t.client_id)
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
            schedule_type: sched.schedule_type.clone(),
            cron_expression: sched.cron_expression.clone(),
            enabled: sched.enabled,
            canary_enabled: sched.canary_enabled,
            execution_mode: sched.execution_mode.clone(),
            on_failure: sched.on_failure.clone(),
            exclude_patterns_raw: sched.exclude_patterns_raw.clone(),
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

    let existing_clients = db::list_clients(&state.pool, true).await?;
    let mut hostname_to_id: HashMap<String, i64> = existing_clients
        .iter()
        .map(|c| (c.hostname.clone(), c.id))
        .collect();

    for host in &payload.hosts {
        if let Some(&existing_id) = hostname_to_id.get(&host.hostname) {
            db::update_client(
                &state.pool,
                &host.hostname,
                &host.hostname,
                host.display_name.as_deref(),
                &host.default_backup_paths,
                &host.default_exclude_patterns,
            )
            .await?;

            let existing_patterns =
                db::patterns::list_patterns_for_client(&state.pool, existing_id).await?;
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
            let client = db::insert_client_with_paths(
                &state.pool,
                &host.hostname,
                host.display_name.as_deref(),
                IMPORTED_TOKEN,
                &host.default_backup_paths,
                &host.default_exclude_patterns,
            )
            .await?;
            for pattern in &host.hostname_patterns {
                db::patterns::add_hostname_pattern(&state.pool, client.id, pattern).await?;
            }
            hostname_to_id.insert(host.hostname.clone(), client.id);
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
            let client_id = if let Some(&cid) = hostname_to_id.get(&target.hostname) {
                cid
            } else {
                let client =
                    db::insert_client(&state.pool, &target.hostname, None, IMPORTED_TOKEN, None)
                        .await?;
                result.warnings.push(format!(
                    "created placeholder host {:?} referenced by schedule {:?}",
                    target.hostname, sched.name
                ));
                hostname_to_id.insert(target.hostname.clone(), client.id);
                client.id
            };
            target_ids.push((client_id, target.execution_order));
        }

        let pre_cmds_json =
            serde_json::to_string(&sched.pre_backup_commands).unwrap_or_else(|_| "[]".to_owned());
        let post_cmds_json =
            serde_json::to_string(&sched.post_backup_commands).unwrap_or_else(|_| "[]".to_owned());

        let params = ScheduleParams {
            name: &sched.name,
            schedule_type: &sched.schedule_type,
            cron_expression: &sched.cron_expression,
            enabled: sched.enabled,
            canary_enabled: sched.canary_enabled,
            exclude_patterns_raw: &sched.exclude_patterns_raw,
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
            execution_mode: &sched.execution_mode,
            on_failure: &sched.on_failure,
        };

        let new_sched = db::insert_schedule(&state.pool, repo_id, &params, None).await?;
        db::insert_schedule_targets(&state.pool, new_sched.id, &target_ids).await?;

        for (i, path) in sched.backup_sources.iter().enumerate() {
            let sort_order = i32::try_from(i).unwrap_or(0);
            db::insert_backup_source_for_schedule(&state.pool, new_sched.id, path, sort_order)
                .await?;
        }

        for target in &sched.targets {
            let Some(&client_id) = hostname_to_id.get(&target.hostname) else {
                continue;
            };
            for (i, path) in target.backup_sources.iter().enumerate() {
                let sort_order = i32::try_from(i).unwrap_or(0);
                db::insert_backup_source_for_schedule_client(
                    &state.pool,
                    new_sched.id,
                    client_id,
                    path,
                    sort_order,
                )
                .await?;
            }
            if !target.exclude_patterns.is_empty() {
                db::upsert_per_host_excludes_raw(
                    &state.pool,
                    new_sched.id,
                    client_id,
                    &target.exclude_patterns,
                )
                .await?;
            }
        }

        result.schedules_created += 1;
    }

    Ok(Json(result))
}
