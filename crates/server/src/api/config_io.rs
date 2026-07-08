// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::auth::RequireAdmin;
use crate::{
    AppState,
    db::{self, IMPORTED_TOKEN_HASH, ScheduleParams},
    error::{ApiError, ApiJson},
};

const EXPORT_VERSION: u32 = 1;

/// Exported host (agent) data for config import/export.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HostExport {
    /// Agent hostname.
    pub hostname: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Default backup source paths.
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns.
    pub default_exclude_patterns: Vec<String>,
    /// Default pre-backup commands (JSON-encoded).
    pub default_pre_backup_commands: String,
    /// Default post-backup commands (JSON-encoded).
    pub default_post_backup_commands: String,
    /// Default file change detection patterns.
    #[serde(default)]
    pub default_file_change_patterns_raw: String,
    /// Hostname pattern globs for archive matching.
    pub hostname_patterns: Vec<String>,
}

/// Exported per-target overrides for a schedule.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleTargetExport {
    /// Target agent hostname.
    pub hostname: String,
    /// Execution order among targets.
    pub execution_order: i32,
    /// Per-agent backup source paths.
    pub backup_sources: Vec<String>,
    /// Per-agent exclude patterns.
    pub exclude_patterns: String,
    /// Per-agent file change patterns.
    #[serde(default)]
    pub file_change_patterns: String,
}

/// Exported schedule data for config import/export.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct ScheduleExport {
    /// Schedule display name.
    pub name: String,
    /// Schedule type (backup, check, verify).
    pub schedule_type: String,
    /// Cron expression.
    pub cron_expression: String,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// Whether canary backups are enabled.
    pub canary_enabled: bool,
    /// Execution mode (e.g. sequential).
    pub execution_mode: String,
    /// Behaviour on backup failure.
    pub on_failure: String,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: String,
    /// Raw file change pattern text.
    #[serde(default)]
    pub file_change_patterns_raw: String,
    /// Whether global excludes are ignored.
    pub ignore_global_excludes: bool,
    /// Hourly retention count.
    pub keep_hourly: i32,
    /// Daily retention count.
    pub keep_daily: i32,
    /// Weekly retention count.
    pub keep_weekly: i32,
    /// Monthly retention count.
    pub keep_monthly: i32,
    /// Yearly retention count.
    pub keep_yearly: i32,
    /// Whether compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in KB/s.
    pub rate_limit_kbps: Option<i32>,
    /// Pre-backup commands.
    pub pre_backup_commands: Vec<String>,
    /// Post-backup commands.
    pub post_backup_commands: Vec<String>,
    /// Repository name this schedule targets.
    pub repo_name: Option<String>,
    /// Schedule-level backup source paths.
    pub backup_sources: Vec<String>,
    /// Per-target overrides.
    pub targets: Vec<ScheduleTargetExport>,
}

/// Top-level config export payload wrapping hosts and schedules.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConfigExport {
    /// Export format version.
    pub version: u32,
    /// Timestamp when the export was created.
    pub exported_at: DateTime<Utc>,
    /// Exported host configurations.
    pub hosts: Vec<HostExport>,
    /// Exported schedule configurations.
    pub schedules: Vec<ScheduleExport>,
}

/// Result summary after importing config from a JSON export.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ImportResult {
    /// Number of new agent hosts created.
    pub hosts_created: u32,
    /// Number of existing agent hosts updated.
    pub hosts_updated: u32,
    /// Number of schedules created.
    pub schedules_created: u32,
    /// Warnings encountered during import.
    pub warnings: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/api/config/export",
    tag = "Config",
    operation_id = "exportConfig",
    responses(
        (status = 200, description = "Config export", body = ConfigExport),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Export all hosts and schedules as a portable JSON snapshot.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
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
        schedules.push(
            build_schedule_export(&state.pool, sched, &repo_id_to_name, &agent_id_to_hostname)
                .await?,
        );
    }

    Ok(Json(ConfigExport {
        version: EXPORT_VERSION,
        exported_at: Utc::now(),
        hosts,
        schedules,
    }))
}

async fn build_schedule_export(
    pool: &sqlx::PgPool,
    sched: &db::ScheduleRow,
    repo_id_to_name: &HashMap<i64, &str>,
    agent_id_to_hostname: &HashMap<i64, &str>,
) -> Result<ScheduleExport, ApiError> {
    let repo_name = sched
        .repo_id
        .and_then(|rid| repo_id_to_name.get(&rid).copied())
        .map(str::to_owned);

    let target_rows = db::list_schedule_targets(pool, sched.id).await?;
    let backup_sources = db::list_backup_sources_for_schedule(pool, sched.id).await?;
    let per_agent_sources =
        db::list_all_per_agent_backup_sources_for_schedule(pool, sched.id).await?;
    let per_agent_excludes = db::list_all_per_agent_excludes_for_schedule(pool, sched.id).await?;

    let per_agent_sources_map: HashMap<i64, &Vec<String>> = per_agent_sources
        .iter()
        .map(|s| (s.agent_id, &s.paths))
        .collect();
    let per_agent_excludes_map: HashMap<i64, &str> = per_agent_excludes
        .iter()
        .map(|e| (e.agent_id, e.raw_text.as_str()))
        .collect();
    let per_agent_file_change_patterns =
        db::list_all_per_agent_file_change_patterns_for_schedule(pool, sched.id).await?;
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

    let pre_backup_commands = serde_json::from_str(&sched.pre_backup_commands).unwrap_or_default();
    let post_backup_commands =
        serde_json::from_str(&sched.post_backup_commands).unwrap_or_default();

    Ok(ScheduleExport {
        name: sched.name.clone(),
        schedule_type: sched.schedule_type.clone(),
        cron_expression: sched.cron_expression.clone(),
        enabled: sched.enabled,
        canary_enabled: sched.canary_enabled,
        execution_mode: sched.execution_mode.clone(),
        on_failure: sched.on_failure.clone(),
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
    })
}

#[utoipa::path(
    post,
    path = "/api/config/import",
    tag = "Config",
    operation_id = "importConfig",
    request_body = ConfigExport,
    responses(
        (status = 200, description = "Import results", body = ImportResult),
        (status = 400, description = "Invalid version or format"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Import hosts and schedules from a JSON export.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
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
        import_host(&state.pool, host, &mut hostname_to_id, &mut result).await?;
    }

    let repos = db::list_all_repos(&state.pool).await?;
    let repo_name_to_id: HashMap<&str, i64> =
        repos.iter().map(|r| (r.name.as_str(), r.id)).collect();

    for sched in &payload.schedules {
        import_schedule(
            &state.pool,
            sched,
            &mut hostname_to_id,
            &repo_name_to_id,
            &mut result,
        )
        .await?;
    }

    Ok(Json(result))
}

async fn import_host(
    pool: &sqlx::PgPool,
    host: &HostExport,
    hostname_to_id: &mut HashMap<String, i64>,
    result: &mut ImportResult,
) -> Result<(), ApiError> {
    if let Some(&existing_id) = hostname_to_id.get(&host.hostname) {
        db::update_agent(
            pool,
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

        let existing_patterns = db::patterns::list_patterns_for_agent(pool, existing_id).await?;
        let existing_set: HashSet<&str> = existing_patterns
            .iter()
            .map(|p| p.pattern.as_str())
            .collect();
        for pattern in &host.hostname_patterns {
            if !existing_set.contains(pattern.as_str()) {
                db::patterns::add_hostname_pattern(pool, existing_id, pattern).await?;
            }
        }

        result.hosts_updated = result.hosts_updated.saturating_add(1);
    } else {
        let agent = db::insert_agent_with_paths(
            pool,
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
            db::patterns::add_hostname_pattern(pool, agent.id, pattern).await?;
        }
        hostname_to_id.insert(host.hostname.clone(), agent.id);
        result.hosts_created = result.hosts_created.saturating_add(1);
    }
    Ok(())
}

async fn import_schedule(
    pool: &sqlx::PgPool,
    sched: &ScheduleExport,
    hostname_to_id: &mut HashMap<String, i64>,
    repo_name_to_id: &HashMap<&str, i64>,
    result: &mut ImportResult,
) -> Result<(), ApiError> {
    let Some(repo_name) = &sched.repo_name else {
        result.warnings.push(format!(
            "skipped schedule {:?}: no repository assigned",
            sched.name
        ));
        return Ok(());
    };

    let Some(&repo_id) = repo_name_to_id.get(repo_name.as_str()) else {
        result.warnings.push(format!(
            "skipped schedule {:?}: repository {:?} not found",
            sched.name, repo_name
        ));
        return Ok(());
    };

    if sched.targets.is_empty() {
        result
            .warnings
            .push(format!("skipped schedule {:?}: no targets", sched.name));
        return Ok(());
    }

    let target_ids = resolve_schedule_target_agent_ids(pool, sched, hostname_to_id, result).await?;

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
        on_failure: &sched.on_failure,
    };

    let new_sched = db::insert_schedule(pool, repo_id, &params, None).await?;
    db::insert_schedule_targets(pool, new_sched.id, &target_ids).await?;

    for (i, path) in sched.backup_sources.iter().enumerate() {
        let sort_order = i32::try_from(i).unwrap_or(0);
        db::insert_backup_source_for_schedule(pool, new_sched.id, path, sort_order).await?;
    }

    insert_schedule_target_overrides(pool, new_sched.id, sched, hostname_to_id).await?;

    result.schedules_created = result.schedules_created.saturating_add(1);
    Ok(())
}

/// Resolves each target's hostname to an agent ID, creating a placeholder
/// agent (and recording a warning) for any hostname not already known.
async fn resolve_schedule_target_agent_ids(
    pool: &sqlx::PgPool,
    sched: &ScheduleExport,
    hostname_to_id: &mut HashMap<String, i64>,
    result: &mut ImportResult,
) -> Result<Vec<(i64, i32)>, ApiError> {
    let mut target_ids: Vec<(i64, i32)> = Vec::new();
    for target in &sched.targets {
        let agent_id = if let Some(&cid) = hostname_to_id.get(&target.hostname) {
            cid
        } else {
            let agent =
                db::insert_agent(pool, &target.hostname, None, IMPORTED_TOKEN_HASH, None).await?;
            result.warnings.push(format!(
                "created placeholder agent {:?} referenced by schedule {:?}",
                target.hostname, sched.name
            ));
            hostname_to_id.insert(target.hostname.clone(), agent.id);
            agent.id
        };
        target_ids.push((agent_id, target.execution_order));
    }
    Ok(target_ids)
}

/// Inserts per-target backup source, exclude pattern, and file-change
/// pattern overrides for a newly imported schedule. Targets whose hostname
/// isn't in `hostname_to_id` are skipped (this only happens if agent
/// resolution above failed to insert a placeholder, which itself returns an
/// error, so in practice every target is present here).
async fn insert_schedule_target_overrides(
    pool: &sqlx::PgPool,
    new_schedule_id: i64,
    sched: &ScheduleExport,
    hostname_to_id: &HashMap<String, i64>,
) -> Result<(), ApiError> {
    for target in &sched.targets {
        let Some(&agent_id) = hostname_to_id.get(&target.hostname) else {
            continue;
        };
        for (i, path) in target.backup_sources.iter().enumerate() {
            let sort_order = i32::try_from(i).unwrap_or(0);
            db::insert_backup_source_for_schedule_agent(
                pool,
                new_schedule_id,
                agent_id,
                path,
                sort_order,
            )
            .await?;
        }
        if !target.exclude_patterns.is_empty() {
            db::upsert_per_agent_excludes_raw(
                pool,
                new_schedule_id,
                agent_id,
                &target.exclude_patterns,
            )
            .await?;
        }
        if !target.file_change_patterns.is_empty() {
            db::upsert_per_agent_file_change_patterns_raw(
                pool,
                new_schedule_id,
                agent_id,
                &target.file_change_patterns,
            )
            .await?;
        }
    }
    Ok(())
}
