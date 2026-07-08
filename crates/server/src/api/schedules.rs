// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    responses::{
        PerAgentBackupSourcesResponse, PerAgentCommandsResponse, PerAgentExcludePatternsResponse,
        PerAgentFileChangePatternsResponse, ScheduleBackupSourcesResponse, ScheduleTargetResponse,
    },
    schedule::{calculate_next_run, validate_cron},
    types::{OnFailure, RepoId, ScheduleType},
};
use sqlx::PgPool;

impl From<db::ScheduleTargetRow> for ScheduleTargetResponse {
    fn from(t: db::ScheduleTargetRow) -> Self {
        Self {
            agent_id: t.agent_id,
            execution_order: t.execution_order,
        }
    }
}

impl From<db::PerAgentBackupSources> for PerAgentBackupSourcesResponse {
    fn from(s: db::PerAgentBackupSources) -> Self {
        Self {
            agent_id: s.agent_id,
            paths: s.paths,
        }
    }
}

impl From<db::PerAgentExcludePatterns> for PerAgentExcludePatternsResponse {
    fn from(e: db::PerAgentExcludePatterns) -> Self {
        Self {
            agent_id: e.agent_id,
            raw_text: e.raw_text,
        }
    }
}

impl From<db::PerAgentCommands> for PerAgentCommandsResponse {
    fn from(c: db::PerAgentCommands) -> Self {
        Self {
            agent_id: c.agent_id,
            pre_backup_commands: c.pre_backup_commands,
            post_backup_commands: c.post_backup_commands,
        }
    }
}

impl From<db::PerAgentFileChangePatterns> for PerAgentFileChangePatternsResponse {
    fn from(f: db::PerAgentFileChangePatterns) -> Self {
        Self {
            agent_id: f.agent_id,
            raw_text: f.raw_text,
        }
    }
}
use uuid::Uuid;

use super::{
    auth::AuthUser,
    permissions::{check_repo_permission, is_visible_to_user},
};
use crate::{
    AppState, config_assembler,
    db::{self, ScheduleParams, ScheduleRow},
    error::{ApiError, ApiJson},
    ssh::{self, TestConnectionRequest},
    ws::{completion_bus, ui_broadcast::ActiveBackupSnapshot},
};

/// Per-agent backup sources for a schedule target.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentBackupSources {
    /// The agent's ID.
    pub agent_id: i64,
    /// Paths to back up on this agent.
    pub paths: Vec<String>,
}

/// Per-agent exclude patterns for a schedule target.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentExcludePatterns {
    /// The agent's ID.
    pub agent_id: i64,
    /// Raw exclude pattern text.
    pub raw_text: String,
}

/// Per-agent pre/post backup commands for a schedule target.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentCommands {
    /// The agent's ID.
    pub agent_id: i64,
    /// Commands to run before the backup.
    pub pre_backup_commands: Vec<String>,
    /// Commands to run after the backup.
    pub post_backup_commands: Vec<String>,
}

/// Per-agent file change detection patterns for a schedule target.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentFileChangePatterns {
    /// The agent's ID.
    pub agent_id: i64,
    /// Raw file change pattern text.
    pub raw_text: String,
}

/// Request payload for creating a new backup schedule.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateScheduleRequest {
    /// IDs of agents to assign as targets.
    pub agent_ids: Vec<i64>,
    /// Repository ID to back up to.
    pub repo_id: i64,
    /// Optional display name for the schedule.
    pub name: Option<String>,
    /// Schedule type (backup, check, verify).
    #[schema(value_type = Option<String>)]
    pub schedule_type: Option<ScheduleType>,
    /// Cron expression defining the schedule.
    pub cron_expression: String,
    /// Whether the schedule is enabled (defaults to true).
    pub enabled: Option<bool>,
    /// Whether canary backups are enabled (defaults to true).
    pub canary_enabled: Option<bool>,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: Option<String>,
    /// Whether to ignore global excludes.
    pub ignore_global_excludes: Option<bool>,
    /// Number of hourly backups to keep.
    pub keep_hourly: Option<i32>,
    /// Number of daily backups to keep.
    pub keep_daily: Option<i32>,
    /// Number of weekly backups to keep.
    pub keep_weekly: Option<i32>,
    /// Number of monthly backups to keep.
    pub keep_monthly: Option<i32>,
    /// Number of yearly backups to keep.
    pub keep_yearly: Option<i32>,
    /// Whether compaction is enabled.
    pub compact_enabled: Option<bool>,
    /// Rate limit in KB/s.
    pub rate_limit_kbps: Option<u32>,
    /// Commands to run before the backup.
    pub pre_backup_commands: Option<Vec<String>>,
    /// Commands to run after the backup.
    pub post_backup_commands: Option<Vec<String>>,
    /// Backup sources (schedule-level).
    pub backup_sources: Option<Vec<String>>,
    /// Per-agent backup sources.
    pub backup_sources_per_agent: Option<Vec<AgentBackupSources>>,
    /// Per-agent exclude patterns.
    pub exclude_patterns_per_agent: Option<Vec<AgentExcludePatterns>>,
    /// Per-agent pre/post commands.
    pub commands_per_agent: Option<Vec<AgentCommands>>,
    /// Raw file change detection pattern text (schedule-level).
    pub file_change_patterns_raw: Option<String>,
    /// Per-agent file change patterns.
    pub file_change_patterns_per_agent: Option<Vec<AgentFileChangePatterns>>,
    /// Behaviour when the backup fails.
    #[schema(value_type = Option<String>)]
    pub on_failure: Option<OnFailure>,
}

/// Request payload for updating an existing schedule.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateScheduleRequest {
    /// Optional new display name.
    pub name: Option<String>,
    /// Updated cron expression.
    pub cron_expression: String,
    /// New repository ID to assign.
    pub repo_id: Option<i64>,
    /// Whether the schedule is enabled.
    pub enabled: Option<bool>,
    /// Whether canary backups are enabled.
    pub canary_enabled: Option<bool>,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: Option<String>,
    /// Whether to ignore global excludes.
    pub ignore_global_excludes: Option<bool>,
    /// Number of hourly backups to keep.
    pub keep_hourly: Option<i32>,
    /// Number of daily backups to keep.
    pub keep_daily: Option<i32>,
    /// Number of weekly backups to keep.
    pub keep_weekly: Option<i32>,
    /// Number of monthly backups to keep.
    pub keep_monthly: Option<i32>,
    /// Number of yearly backups to keep.
    pub keep_yearly: Option<i32>,
    /// Whether compaction is enabled.
    pub compact_enabled: Option<bool>,
    /// Rate limit in KB/s.
    pub rate_limit_kbps: Option<u32>,
    /// Commands to run before the backup.
    pub pre_backup_commands: Option<Vec<String>>,
    /// Commands to run after the backup.
    pub post_backup_commands: Option<Vec<String>>,
    /// Backup sources (schedule-level, replaces all).
    pub backup_sources: Option<Vec<String>>,
    /// Per-agent backup sources (replaces all).
    pub backup_sources_per_agent: Option<Vec<AgentBackupSources>>,
    /// Per-agent exclude patterns (replaces all).
    pub exclude_patterns_per_agent: Option<Vec<AgentExcludePatterns>>,
    /// Per-agent pre/post commands (replaces all).
    pub commands_per_agent: Option<Vec<AgentCommands>>,
    /// Raw file change pattern text (schedule-level).
    pub file_change_patterns_raw: Option<String>,
    /// Per-agent file change patterns (replaces all).
    pub file_change_patterns_per_agent: Option<Vec<AgentFileChangePatterns>>,
    /// Agent IDs to assign as targets (replaces all).
    pub agent_ids: Option<Vec<i64>>,
    /// Behaviour when the backup fails.
    #[schema(value_type = Option<String>)]
    pub on_failure: Option<OnFailure>,
}

#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "Schedules",
    operation_id = "listSchedules",
    responses(
        (status = 200, description = "List of schedules", body = Vec<crate::db::ScheduleRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
/// List all schedules visible to the current user.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedules(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ScheduleRow>>, ApiError> {
    let schedules = db::list_schedules(&state.pool).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    let is_admin = effective.can_delete_repo;
    let mut visible = Vec::with_capacity(schedules.len());
    for s in schedules {
        if is_visible_to_user(
            &state.pool,
            auth.user_id,
            s.owner_id,
            &s.visibility,
            is_admin,
        )
        .await?
        {
            visible.push(s);
        }
    }
    Ok(Json(visible))
}

#[utoipa::path(
    post,
    path = "/api/schedules",
    tag = "Schedules",
    operation_id = "createSchedule",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = crate::db::ScheduleRow),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 422, description = "Unprocessable -- SSH unreachable"),
    )
)]
/// Create a new backup schedule.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn create_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleRow>), ApiError> {
    if req.agent_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "agent_ids must contain at least one entry".into(),
        ));
    }
    check_repo_permission(&state.pool, &auth, req.repo_id, |p| p.can_modify_schedules).await?;
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let exclude_patterns_raw = req.exclude_patterns_raw.unwrap_or_default();
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        check_ssh_reachability(&state.pool, req.repo_id).await?;
    }
    let schedule_type_enum = req.schedule_type.unwrap_or_default();
    let schedule_type = schedule_type_to_str(schedule_type_enum);

    let has_backup_sources = req.backup_sources.as_ref().is_some_and(|v| !v.is_empty());
    let has_per_agent_sources = req
        .backup_sources_per_agent
        .as_ref()
        .is_some_and(|v| !v.is_empty());

    if !has_backup_sources && !has_per_agent_sources && schedule_type_enum == ScheduleType::Backup {
        let Some(&first_agent_id) = req.agent_ids.first() else {
            return Err(ApiError::BadRequest(
                "agent_ids must contain at least one entry".into(),
            ));
        };
        let agent = db::get_agent_by_id(&state.pool, first_agent_id).await?;
        if agent.default_backup_paths.is_empty() {
            return Err(ApiError::BadRequest(
                "no backup sources provided and agent has no default backup paths configured"
                    .into(),
            ));
        }
    }

    let on_failure = req.on_failure.unwrap_or_default();
    let on_failure_str = on_failure.to_string();

    let params = ScheduleParams {
        name: req.name.as_deref().unwrap_or(""),
        schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(true),
        exclude_patterns_raw: &exclude_patterns_raw,
        ignore_global_excludes: req.ignore_global_excludes.unwrap_or(false),
        keep_hourly: req.keep_hourly.unwrap_or(24),
        keep_daily: req.keep_daily.unwrap_or(7),
        keep_weekly: req.keep_weekly.unwrap_or(4),
        keep_monthly: req.keep_monthly.unwrap_or(6),
        keep_yearly: req.keep_yearly.unwrap_or(0),
        compact_enabled: req.compact_enabled.unwrap_or(true),
        rate_limit_kbps: convert_rate_limit(req.rate_limit_kbps)?,
        file_change_patterns_raw: req.file_change_patterns_raw.as_deref().unwrap_or(""),
        pre_backup_commands: &serde_json::to_string(&req.pre_backup_commands.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_owned()),
        post_backup_commands: &serde_json::to_string(&req.post_backup_commands.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_owned()),
        on_failure: &on_failure_str,
    };

    let schedule =
        db::insert_schedule(&state.pool, req.repo_id, &params, Some(auth.user_id)).await?;

    let targets: Vec<(i64, i32)> = req
        .agent_ids
        .iter()
        .enumerate()
        .map(|(i, &cid)| {
            let order = i32::try_from(i).unwrap_or(0);
            (cid, order)
        })
        .collect();
    db::insert_schedule_targets(&state.pool, schedule.id, &targets).await?;

    if let Some(sources) = &req.backup_sources {
        insert_schedule_sources(&state.pool, schedule.id, sources).await?;
    }

    if let Some(per_agent) = &req.backup_sources_per_agent {
        insert_per_agent_sources(&state.pool, schedule.id, per_agent).await?;
    }

    if let Some(per_agent) = &req.exclude_patterns_per_agent {
        insert_per_agent_excludes(&state.pool, schedule.id, per_agent).await?;
    }

    if let Some(per_agent) = &req.commands_per_agent {
        insert_per_agent_commands(&state.pool, schedule.id, per_agent).await?;
    }

    if let Some(per_agent) = &req.file_change_patterns_per_agent {
        insert_per_agent_file_change_patterns(&state.pool, schedule.id, per_agent).await?;
    }

    if enabled {
        refresh_next_run(&state.pool, schedule.id, &req.cron_expression).await?;
    }

    config_assembler::push_config_to_all_schedule_targets(&state, schedule.id).await;

    Ok((StatusCode::CREATED, Json(schedule)))
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}",
    tag = "Schedules",
    operation_id = "getSchedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Schedule details", body = crate::db::ScheduleRow),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Get a single schedule by ID.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_schedule(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScheduleRow>, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    Ok(Json(schedule))
}

#[utoipa::path(
    put,
    path = "/api/schedules/{id}",
    tag = "Schedules",
    operation_id = "updateSchedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Updated schedule", body = crate::db::ScheduleRow),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Update an existing schedule.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Forbidden`]: the caller lacks permission for this operation
/// - [`ApiError::BadRequest`]: the request is invalid
pub async fn update_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateScheduleRequest>,
) -> Result<Json<ScheduleRow>, ApiError> {
    let existing = db::get_schedule_by_id(&state.pool, id).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if let Some(rid) = existing.repo_id {
        check_repo_permission(&state.pool, &auth, rid, |p| p.can_modify_schedules).await?;
    } else if !effective.can_delete_repo {
        return Err(ApiError::Forbidden(
            "only admins can edit orphaned schedules".into(),
        ));
    }
    let effective_repo_id: Option<i64> = req.repo_id.or(existing.repo_id);
    if effective_repo_id != existing.repo_id
        && let Some(new_rid) = effective_repo_id
    {
        check_repo_permission(&state.pool, &auth, new_rid, |p| p.can_modify_schedules).await?;
    }
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let exclude_patterns_raw = req
        .exclude_patterns_raw
        .clone()
        .unwrap_or_else(|| existing.exclude_patterns_raw.clone());
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        let Some(eff_rid) = effective_repo_id else {
            return Err(ApiError::BadRequest(
                "cannot enable a schedule with no repository assigned".into(),
            ));
        };
        check_ssh_reachability(&state.pool, eff_rid).await?;
    }

    let pre_cmds_json = req.pre_backup_commands.clone().map_or_else(
        || existing.pre_backup_commands.clone(),
        |cmds| serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".to_owned()),
    );
    let post_cmds_json = req.post_backup_commands.clone().map_or_else(
        || existing.post_backup_commands.clone(),
        |cmds| serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".to_owned()),
    );

    let on_failure = req
        .on_failure
        .map_or_else(|| existing.on_failure.clone(), |f| f.to_string());

    let name = req.name.clone().unwrap_or_else(|| existing.name.clone());

    let params = ScheduleParams {
        name: &name,
        schedule_type: &existing.schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(existing.canary_enabled),
        exclude_patterns_raw: &exclude_patterns_raw,
        ignore_global_excludes: req.ignore_global_excludes.unwrap_or(false),
        keep_hourly: req.keep_hourly.unwrap_or(existing.keep_hourly),
        keep_daily: req.keep_daily.unwrap_or(existing.keep_daily),
        keep_weekly: req.keep_weekly.unwrap_or(existing.keep_weekly),
        keep_monthly: req.keep_monthly.unwrap_or(existing.keep_monthly),
        keep_yearly: req.keep_yearly.unwrap_or(existing.keep_yearly),
        compact_enabled: req.compact_enabled.unwrap_or(existing.compact_enabled),
        rate_limit_kbps: match convert_rate_limit(req.rate_limit_kbps)? {
            Some(v) => Some(v),
            None => existing.rate_limit_kbps,
        },
        file_change_patterns_raw: req.file_change_patterns_raw.as_deref().unwrap_or(""),
        pre_backup_commands: &pre_cmds_json,
        post_backup_commands: &post_cmds_json,
        on_failure: &on_failure,
    };

    if effective_repo_id != existing.repo_id
        && let Some(new_rid) = effective_repo_id
    {
        db::update_schedule_repo(&state.pool, id, new_rid).await?;
    }
    let schedule = db::update_schedule(&state.pool, id, &params).await?;

    apply_schedule_target_overrides(&state.pool, schedule.id, &req).await?;

    if enabled {
        refresh_next_run(&state.pool, schedule.id, &req.cron_expression).await?;
    } else {
        db::set_next_run_at(&state.pool, schedule.id, chrono::Utc::now()).await?;
    }

    config_assembler::push_config_to_all_schedule_targets(&state, schedule.id).await;

    Ok(Json(schedule))
}

async fn apply_schedule_target_overrides(
    pool: &sqlx::PgPool,
    schedule_id: i64,
    req: &UpdateScheduleRequest,
) -> Result<(), ApiError> {
    if let Some(agent_ids) = &req.agent_ids {
        if agent_ids.is_empty() {
            return Err(ApiError::BadRequest(
                "agent_ids must contain at least one entry".into(),
            ));
        }
        db::delete_schedule_targets(pool, schedule_id).await?;
        let targets: Vec<(i64, i32)> = agent_ids
            .iter()
            .enumerate()
            .map(|(i, &cid)| {
                let order = i32::try_from(i).unwrap_or(0);
                (cid, order)
            })
            .collect();
        db::insert_schedule_targets(pool, schedule_id, &targets).await?;
    }

    if let Some(sources) = &req.backup_sources {
        db::delete_backup_sources_for_schedule(pool, schedule_id).await?;
        insert_schedule_sources(pool, schedule_id, sources).await?;
    }

    if let Some(per_agent) = &req.backup_sources_per_agent {
        db::delete_per_agent_backup_sources_for_schedule(pool, schedule_id).await?;
        insert_per_agent_sources(pool, schedule_id, per_agent).await?;
    }

    if let Some(per_agent) = &req.exclude_patterns_per_agent {
        db::delete_per_agent_excludes_for_schedule(pool, schedule_id).await?;
        insert_per_agent_excludes(pool, schedule_id, per_agent).await?;
    }

    if let Some(per_agent) = &req.commands_per_agent {
        db::delete_per_agent_commands_for_schedule(pool, schedule_id).await?;
        insert_per_agent_commands(pool, schedule_id, per_agent).await?;
    }

    if let Some(per_agent) = &req.file_change_patterns_per_agent {
        db::delete_per_agent_file_change_patterns_for_schedule(pool, schedule_id).await?;
        insert_per_agent_file_change_patterns(pool, schedule_id, per_agent).await?;
    }

    Ok(())
}

#[utoipa::path(
    delete,
    path = "/api/schedules/{id}",
    tag = "Schedules",
    operation_id = "deleteSchedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Delete a schedule.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller lacks permission for this operation.
pub async fn delete_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let existing = db::get_schedule_by_id(&state.pool, id).await?;
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if let Some(rid) = existing.repo_id {
        check_repo_permission(&state.pool, &auth, rid, |p| p.can_modify_schedules).await?;
    } else if !effective.can_delete_repo {
        return Err(ApiError::Forbidden(
            "only admins can delete orphaned schedules".into(),
        ));
    }

    let hostnames = db::get_schedule_target_hostnames(&state.pool, id)
        .await
        .ok();

    db::delete_schedule(&state.pool, id).await?;

    if let Some(hostnames) = hostnames {
        for hostname in &hostnames {
            config_assembler::push_config_to_agent(&state, hostname).await;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

fn schedule_type_to_str(st: ScheduleType) -> &'static str {
    match st {
        ScheduleType::Backup => "backup",
        ScheduleType::Check => "check",
        ScheduleType::Verify => "verify",
    }
}

async fn check_ssh_reachability(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let repo = db::get_repo_connection(pool, repo_id).await?;
    let ssh_port = u16::try_from(repo.ssh_port)
        .map_err(|_| ApiError::Unprocessable("Cannot reach repository: invalid SSH port".into()))?;
    let response = ssh::test_connection(&TestConnectionRequest {
        ssh_host: repo.ssh_host,
        ssh_user: repo.ssh_user,
        ssh_port: Some(ssh_port),
    })
    .await;
    if response.ssh_ok {
        Ok(())
    } else {
        Err(ApiError::Unprocessable(format!(
            "Cannot reach repository: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        )))
    }
}

fn convert_rate_limit(rate_limit_kbps: Option<u32>) -> Result<Option<i32>, ApiError> {
    rate_limit_kbps
        .map(|v| {
            i32::try_from(v)
                .map_err(|_| ApiError::BadRequest("rate_limit_kbps is too large".into()))
        })
        .transpose()
}

async fn insert_schedule_sources(
    pool: &PgPool,
    schedule_id: i64,
    sources: &[String],
) -> Result<(), ApiError> {
    for (i, path) in sources.iter().enumerate() {
        let sort_order =
            i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
        db::insert_backup_source_for_schedule(pool, schedule_id, path, sort_order).await?;
    }
    Ok(())
}

async fn insert_per_agent_sources(
    pool: &PgPool,
    schedule_id: i64,
    per_agent: &[AgentBackupSources],
) -> Result<(), ApiError> {
    for entry in per_agent {
        for (i, path) in entry.paths.iter().enumerate() {
            let sort_order =
                i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
            db::insert_backup_source_for_schedule_agent(
                pool,
                schedule_id,
                entry.agent_id,
                path,
                sort_order,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_per_agent_excludes(
    pool: &PgPool,
    schedule_id: i64,
    per_agent: &[AgentExcludePatterns],
) -> Result<(), ApiError> {
    for entry in per_agent {
        db::upsert_per_agent_excludes_raw(pool, schedule_id, entry.agent_id, &entry.raw_text)
            .await?;
    }
    Ok(())
}

async fn insert_per_agent_commands(
    pool: &PgPool,
    schedule_id: i64,
    per_agent: &[AgentCommands],
) -> Result<(), ApiError> {
    for entry in per_agent {
        let pre =
            serde_json::to_string(&entry.pre_backup_commands).unwrap_or_else(|_| "[]".to_owned());
        let post =
            serde_json::to_string(&entry.post_backup_commands).unwrap_or_else(|_| "[]".to_owned());
        db::upsert_per_agent_commands(pool, schedule_id, entry.agent_id, &pre, &post).await?;
    }
    Ok(())
}

async fn insert_per_agent_file_change_patterns(
    pool: &PgPool,
    schedule_id: i64,
    per_agent: &[AgentFileChangePatterns],
) -> Result<(), ApiError> {
    for entry in per_agent {
        db::upsert_per_agent_file_change_patterns_raw(
            pool,
            schedule_id,
            entry.agent_id,
            &entry.raw_text,
        )
        .await?;
    }
    Ok(())
}

async fn refresh_next_run(
    pool: &PgPool,
    schedule_id: i64,
    cron_expression: &str,
) -> Result<(), ApiError> {
    let now = chrono::Utc::now();
    let tz = db::get_schedule_timezone(pool).await?;
    let next = calculate_next_run(cron_expression, now, tz)
        .map_err(|e| ApiError::Internal(format!("failed to calculate next run: {e}")))?;
    db::set_next_run_at(pool, schedule_id, next).await
}

#[utoipa::path(
    post,
    path = "/api/schedules/{id}/run",
    tag = "Schedules",
    operation_id = "runScheduleNow",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 202, description = "Accepted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Trigger a schedule to run immediately.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn run_schedule_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let Some(schedule_repo_id) = schedule.repo_id else {
        return Err(ApiError::BadRequest(
            "schedule has no repository assigned".into(),
        ));
    };
    check_repo_permission(&state.pool, &auth, schedule_repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;

    let targets = db::get_schedule_targets_for_run(&state.pool, id).await?;
    let repo_id = RepoId(schedule_repo_id);
    let schedule_type = schedule
        .schedule_type
        .parse::<ScheduleType>()
        .map_err(ApiError::BadRequest)?;
    let run_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();

    for target in &targets {
        if let Err(e) = db::insert_backup_pending(
            &state.pool,
            target.agent_id,
            schedule_repo_id,
            Some(id),
            &run_id,
            now,
        )
        .await
        {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                "manual run: failed to insert pending record"
            );
        }
    }

    tokio::spawn(run_manual_sequential(
        state,
        targets,
        repo_id,
        schedule_type,
        id,
        run_id,
    ));

    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    post,
    path = "/api/schedules/{id}/cancel",
    tag = "Schedules",
    operation_id = "cancelRunningBackup",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 202, description = "Accepted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Cancel a running backup for a schedule.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn cancel_running_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let Some(schedule_repo_id) = schedule.repo_id else {
        return Err(ApiError::BadRequest(
            "schedule has no repository assigned".into(),
        ));
    };
    check_repo_permission(&state.pool, &auth, schedule_repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;

    let hostnames = db::get_schedule_target_hostnames(&state.pool, id).await?;
    let repo_id = RepoId(schedule_repo_id);

    for hostname in &hostnames {
        let msg = ServerToAgent::CancelBackup { repo_id };
        if let Err(e) = state.registry.send_to(hostname, msg).await {
            tracing::warn!(
                hostname = %hostname,
                error = %e,
                "agent not connected for cancel_running_backup"
            );
            // Agent is offline - cancel the backup directly in the DB
            if let Some(target) = db::resolve_agent_for_hostname(&state.pool, hostname)
                .await
                .ok()
                .and_then(|r| match r {
                    db::ResolveResult::ExactMatch(a) | db::ResolveResult::PatternMatch(a) => {
                        Some(a)
                    }
                    db::ResolveResult::Unmatched => None,
                })
            {
                if let Err(e) =
                    db::cancel_backup_report(&state.pool, target.id, schedule_repo_id).await
                {
                    tracing::error!(
                        hostname = %hostname,
                        error = %e,
                        "failed to cancel backup in DB after agent not connected"
                    );
                }
                state
                    .completion_bus
                    .publish(crate::ws::completion_bus::OperationOutcome {
                        hostname: hostname.clone(),
                        repo_id: schedule_repo_id,
                        success: false,
                    });
                state.ui_broadcast.clear_active_backup(schedule_repo_id);
                state.ui_broadcast.send(ServerToUi::DataChanged);
            }
        }
    }

    Ok(StatusCode::ACCEPTED)
}

/// Query parameters for listing backup reports for a schedule.
#[derive(Debug, Deserialize)]
pub struct ListScheduleReportsQuery {
    /// Maximum number of reports to return.
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/reports",
    tag = "Schedules",
    operation_id = "listScheduleReports",
    params(
        ("id" = i64, Path, description = "Schedule ID"),
        ("limit" = Option<i64>, Query, description = "Max entries to return"),
    ),
    responses(
        (status = 200, description = "List of backup reports", body = Vec<crate::db::ReportRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List backup reports for a schedule.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedule_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<ListScheduleReportsQuery>,
) -> Result<Json<Vec<db::ReportRow>>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let limit = query.limit.unwrap_or(20);
    let reports = db::list_reports_for_schedule(&state.pool, id, limit).await?;
    Ok(Json(reports))
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/targets",
    tag = "Schedules",
    operation_id = "listScheduleTargets",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "List of targets", body = Vec<ScheduleTargetResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List target hosts for a schedule.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedule_targets(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Vec<ScheduleTargetResponse>>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let targets: Vec<ScheduleTargetResponse> = db::list_schedule_targets(&state.pool, id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(targets))
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/sources",
    tag = "Schedules",
    operation_id = "listScheduleBackupSources",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Backup sources", body = ScheduleBackupSourcesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List backup sources for a schedule (schedule-level and per-host).
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedule_backup_sources(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScheduleBackupSourcesResponse>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let backup_sources = db::list_backup_sources_for_schedule(&state.pool, id).await?;
    let backup_sources_per_agent: Vec<PerAgentBackupSourcesResponse> =
        db::list_all_per_agent_backup_sources_for_schedule(&state.pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    let exclude_patterns_per_agent: Vec<PerAgentExcludePatternsResponse> =
        db::list_all_per_agent_excludes_for_schedule(&state.pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    let commands_per_agent: Vec<PerAgentCommandsResponse> =
        db::list_all_per_agent_commands_for_schedule(&state.pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    let file_change_patterns_per_agent: Vec<PerAgentFileChangePatternsResponse> =
        db::list_all_per_agent_file_change_patterns_for_schedule(&state.pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(ScheduleBackupSourcesResponse {
        backup_sources,
        backup_sources_per_agent,
        exclude_patterns_per_agent,
        commands_per_agent,
        file_change_patterns_per_agent,
    }))
}

async fn run_manual_sequential(
    state: AppState,
    targets: Vec<db::ScheduleRunTarget>,
    repo_id: RepoId,
    schedule_type: ScheduleType,
    schedule_id: i64,
    run_id: String,
) {
    for target in &targets {
        run_manual_target(&state, target, repo_id, schedule_type, schedule_id, &run_id).await;
    }
}

async fn run_manual_target(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    repo_id: RepoId,
    schedule_type: ScheduleType,
    schedule_id: i64,
    run_id: &str,
) {
    let rx = state.completion_bus.subscribe();

    let _repo_guard = state.repo_lock.acquire(repo_id.0).await;

    let command_sent =
        push_config_and_trigger_target(state, target, repo_id, schedule_type, schedule_id, run_id)
            .await;

    // For backup schedules, broadcast BackupStarted even when the agent is
    // offline so the UI can immediately show the "Cancel Backup" button. The
    // backup_report is already in the DB as 'pending' from run_schedule_now.
    let is_backup = matches!(schedule_type, ScheduleType::Backup);

    if command_sent || is_backup {
        state
            .repo_op_tracker
            .set(
                repo_id.0,
                crate::scheduler::repo_op_kind_for(schedule_type),
                target.hostname.clone(),
            )
            .await;
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id: repo_id.0,
            op: state.repo_op_tracker.get(repo_id.0).await,
        });
    }

    if is_backup {
        broadcast_manual_backup_started(state, target, repo_id, schedule_id).await;
    }

    if command_sent {
        let hostname = target.hostname.clone();
        let repo_id_val = repo_id.0;
        let outcome =
            completion_bus::wait_for_completion(&state.registry, rx, &hostname, repo_id_val).await;

        state.repo_op_tracker.clear(repo_id_val).await;
        state.ui_broadcast.send(ServerToUi::RepoOpChanged {
            repo_id: repo_id_val,
            op: None,
        });

        if outcome == completion_bus::CompletionOutcome::AgentDisconnected {
            tracing::warn!(
                hostname = %hostname,
                schedule_id,
                "manual run: agent disconnected before reporting completion"
            );
        }
    }
}

/// Pushes a fresh config to the target agent, then sends the run-now
/// command for the schedule type. Returns whether the command was actually
/// sent (i.e. the agent was reachable for both steps).
async fn push_config_and_trigger_target(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    repo_id: RepoId,
    schedule_type: ScheduleType,
    schedule_id: i64,
    run_id: &str,
) -> bool {
    let agent_reachable = match config_assembler::assemble_config(
        &state.pool,
        &state.encryption_key,
        &target.hostname,
    )
    .await
    {
        Ok(config) => {
            if state
                .registry
                .send_to(&target.hostname, ServerToAgent::ConfigUpdate(config))
                .await
                .is_ok()
            {
                true
            } else {
                tracing::warn!(
                    hostname = %target.hostname,
                    "manual run: agent not connected for config push"
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                "manual run: failed to assemble config"
            );
            false
        }
    };

    if !agent_reachable {
        return false;
    }

    let msg = match schedule_type {
        ScheduleType::Check => ServerToAgent::RunCheckNow {
            repo_id,
            request_id: None,
        },
        ScheduleType::Verify => ServerToAgent::RunVerifyNow {
            repo_id,
            request_id: None,
        },
        ScheduleType::Backup => ServerToAgent::RunBackupNow {
            repo_id,
            schedule_id: Some(schedule_id),
            request_id: None,
            run_id: Some(run_id.to_owned()),
        },
    };

    match state.registry.send_to(&target.hostname, msg).await {
        Ok(()) => {
            tracing::info!(
                hostname = %target.hostname,
                schedule_id,
                "manual run: triggered"
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                "manual run: agent not connected"
            );
            false
        }
    }
}

async fn broadcast_manual_backup_started(
    state: &AppState,
    target: &db::ScheduleRunTarget,
    repo_id: RepoId,
    schedule_id: i64,
) {
    if let Ok(target_name) = db::get_repo_name(&state.pool, repo_id.0).await {
        state.ui_broadcast.set_active_backup(ActiveBackupSnapshot {
            hostname: target.hostname.clone(),
            target_name: target_name.clone(),
            archive_name: None,
            schedule_id: Some(schedule_id),
            repo_id: repo_id.0,
            progress_line: None,
        });
        state.ui_broadcast.send(ServerToUi::BackupStarted {
            hostname: target.hostname.clone(),
            target_name,
            archive_name: None,
            schedule_id: Some(schedule_id),
        });
    }
    state.ui_broadcast.send(ServerToUi::DataChanged);
}
