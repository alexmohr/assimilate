// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use chrono::Utc;
use shared::crypto::encrypt_passphrase;
// Re-exported for openapi.rs etc.
pub use shared::responses::{
    ConfigExportResponse as ConfigExport, HostExportResponse as HostExport,
    ImportResultResponse as ImportResult, RepoExportResponse as RepoExport,
    ScheduleExportResponse as ScheduleExport, ScheduleTargetExportResponse as ScheduleTargetExport,
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

    // Export repositories with quotas, and tags (passphrases are NOT exported
    // for security - they are encrypted at rest with a server-specific key
    // and must be re-set on the target server after import).
    let mut repo_exports = Vec::new();
    for repo in &repos {
        let quota = db::quota::get_quota(&state.pool, repo.id)
            .await
            .unwrap_or(None);
        let tags = db::list_tags_for_repo(&state.pool, repo.id)
            .await?
            .into_iter()
            .map(|t| t.name)
            .collect();

        repo_exports.push(RepoExport {
            name: repo.name.clone(),
            repo_path: repo.repo_path.clone(),
            ssh_user: repo.ssh_user.clone(),
            ssh_host: repo.ssh_host.clone(),
            ssh_port: repo.ssh_port,
            compression: repo.compression.clone(),
            encryption: repo.encryption.clone(),
            enabled: repo.enabled,
            sync_schedule: repo.sync_schedule.clone(),
            ssh_host_key: None,
            quota_warn_bytes: quota.as_ref().and_then(|q| q.warn_bytes),
            quota_critical_bytes: quota.as_ref().and_then(|q| q.critical_bytes),
            quota_warn_action: quota
                .as_ref()
                .map_or(String::new(), |q| q.warn_action.clone()),
            quota_critical_action: quota
                .as_ref()
                .map_or(String::new(), |q| q.critical_action.clone()),
            tags,
        });
    }

    // Query SSH host keys for all repos
    for repo_export in &mut repo_exports {
        if let Ok(Some(host_key)) = db::get_repo_ssh_host_key(&state.pool, &repo_export.name).await
        {
            repo_export.ssh_host_key = Some(host_key);
        }
    }

    Ok(Json(ConfigExport {
        version: EXPORT_VERSION,
        exported_at: Utc::now(),
        hosts,
        schedules,
        repos: repo_exports,
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
        repos_created: 0,
        repos_updated: 0,
        warnings: Vec::new(),
    };

    // Phase 1: Import repositories (before schedules, which reference them by name)
    import_repos(
        &state.pool,
        &payload.repos,
        &state.encryption_key,
        &mut result,
    )
    .await?;

    // Phase 2: Import hosts
    let existing_agents = db::list_agents(&state.pool, true).await?;
    let mut hostname_to_id: HashMap<String, i64> = existing_agents
        .iter()
        .map(|a| (a.hostname.clone(), a.id))
        .collect();

    for host in &payload.hosts {
        import_host(&state.pool, host, &mut hostname_to_id, &mut result).await?;
    }

    // Phase 3: Import schedules (repo_name_to_id now includes newly created repos)
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

/// Import all repos from the export payload, creating or updating as needed.
async fn import_repos(
    pool: &sqlx::PgPool,
    repos: &[RepoExport],
    encryption_key: &[u8; 32],
    result: &mut ImportResult,
) -> Result<HashMap<String, i64>, ApiError> {
    let existing_repos = db::list_all_repos(pool).await?;
    let mut repo_name_to_id: HashMap<String, i64> = existing_repos
        .iter()
        .map(|r| (r.name.clone(), r.id))
        .collect();

    for repo_export in repos {
        result.warnings.push(format!(
            "repo {:?}: passphrase must be set manually after import (passphrases are not \
             exported for security)",
            repo_export.name,
        ));

        let passphrase_encrypted = encrypt_passphrase("", encryption_key)?;

        if let Some(&existing_id) = repo_name_to_id.get(&repo_export.name) {
            // Update existing repo -- skip passphrase change
            db::update_repo(
                pool,
                &db::UpdateRepoParams {
                    repo_id: existing_id,
                    name: &repo_export.name,
                    repo_path: &repo_export.repo_path,
                    ssh_user: &repo_export.ssh_user,
                    ssh_host: &repo_export.ssh_host,
                    ssh_port: repo_export.ssh_port,
                    compression: &repo_export.compression,
                    encryption: &repo_export.encryption,
                    enabled: repo_export.enabled,
                    sync_schedule: repo_export.sync_schedule.as_deref(),
                },
            )
            .await?;

            // Update SSH host key if provided
            if let Some(host_key) = &repo_export.ssh_host_key {
                db::update_repo_ssh_host_key(pool, existing_id, host_key).await?;
            }

            // Upsert quota
            upsert_repo_quota(pool, existing_id, repo_export).await?;

            // Sync tags
            sync_repo_tags(pool, existing_id, &repo_export.tags).await?;

            result.repos_updated = result.repos_updated.saturating_add(1);
        } else {
            // Create new repo
            let new_repo = db::insert_repo(
                pool,
                &db::InsertRepoParams {
                    name: &repo_export.name,
                    repo_path: &repo_export.repo_path,
                    ssh_user: &repo_export.ssh_user,
                    ssh_host: &repo_export.ssh_host,
                    ssh_port: repo_export.ssh_port,
                    passphrase_encrypted: &passphrase_encrypted,
                    compression: &repo_export.compression,
                    encryption: &repo_export.encryption,
                    owner_id: None,
                    sync_schedule: Some(repo_export.sync_schedule.as_deref()),
                },
            )
            .await?;

            // Set SSH host key if provided
            if let Some(host_key) = &repo_export.ssh_host_key {
                db::update_repo_ssh_host_key(pool, new_repo.id, host_key).await?;
            }

            // Upsert quota
            upsert_repo_quota(pool, new_repo.id, repo_export).await?;

            // Sync tags
            sync_repo_tags(pool, new_repo.id, &repo_export.tags).await?;

            repo_name_to_id.insert(repo_export.name.clone(), new_repo.id);
            result.repos_created = result.repos_created.saturating_add(1);
        }
    }

    Ok(repo_name_to_id)
}

async fn upsert_repo_quota(
    pool: &sqlx::PgPool,
    repo_id: i64,
    repo_export: &RepoExport,
) -> Result<(), ApiError> {
    let warn_action = if repo_export.quota_warn_action.is_empty() {
        shared::types::QuotaAction::default()
    } else {
        repo_export
            .quota_warn_action
            .parse()
            .map_err(|e| ApiError::BadRequest(format!("invalid quota_warn_action: {e}")))?
    };
    let critical_action = if repo_export.quota_critical_action.is_empty() {
        shared::types::QuotaAction::default()
    } else {
        repo_export
            .quota_critical_action
            .parse()
            .map_err(|e| ApiError::BadRequest(format!("invalid quota_critical_action: {e}")))?
    };

    db::quota::upsert_quota(
        pool,
        repo_id,
        repo_export.quota_warn_bytes,
        repo_export.quota_critical_bytes,
        warn_action,
        critical_action,
        true,
    )
    .await
    .map_err(ApiError::Database)?;

    Ok(())
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

    let schedule_type_str = sched.schedule_type.to_string();
    let on_failure_str = sched.on_failure.to_string();

    let params = ScheduleParams {
        name: &sched.name,
        schedule_type: &schedule_type_str,
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
        on_failure: &on_failure_str,
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

/// Look up a tag by name in the given scope, creating it if it does not exist.
async fn get_or_create_tag(
    pool: &sqlx::PgPool,
    name: &str,
    scope: &str,
) -> Result<db::TagRow, ApiError> {
    let existing = sqlx::query_as!(
        db::TagRow,
        "SELECT id, name, color, scope FROM tags WHERE name = $1 AND scope = $2",
        name,
        scope,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Some(tag) = existing {
        return Ok(tag);
    }

    let color = tag_color_from_name(name);
    db::insert_tag(pool, name, &color, scope).await
}

fn tag_color_from_name(name: &str) -> String {
    const PALETTE: [&str; 12] = [
        "#3B82F6", "#EF4444", "#10B981", "#F59E0B", "#8B5CF6", "#EC4899", "#06B6D4", "#84CC16",
        "#F97316", "#6366F1", "#14B8A6", "#E11D48",
    ];
    let hash: u64 = name.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u64::from(b))
    });
    let palette_len = PALETTE.len();
    let remainder = hash
        .checked_rem(u64::try_from(palette_len).unwrap_or(12))
        .unwrap_or(0);
    let idx = usize::try_from(remainder).unwrap_or(0);
    PALETTE.get(idx).unwrap_or(&"#6366F1").to_string()
}

/// Sync tags for a repo: set tags by name (creating tags as needed).
async fn sync_repo_tags(
    pool: &sqlx::PgPool,
    repo_id: i64,
    tag_names: &[String],
) -> Result<(), ApiError> {
    if tag_names.is_empty() {
        return Ok(());
    }

    let mut tag_ids = Vec::new();
    for name in tag_names {
        let tag = get_or_create_tag(pool, name, "repo").await?;
        tag_ids.push(tag.id);
    }
    db::set_repo_tags(pool, repo_id, &tag_ids).await
}
