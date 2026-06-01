// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::Deserialize;
use shared::{
    protocol::ServerToAgent,
    schedule::{calculate_next_run, validate_cron},
    types::{ExecutionMode, OnFailure, RepoId, ScheduleType},
};

use super::{
    auth::{AuthUser, Role},
    permissions::{check_repo_permission, is_visible_to_user},
};
use crate::{
    AppState, config_assembler,
    db::{self, ScheduleParams, ScheduleRow},
    error::{ApiError, ApiJson},
    ssh::{self, TestConnectionRequest},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct HostBackupSources {
    pub client_id: i64,
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct HostExcludePatterns {
    pub client_id: i64,
    pub patterns: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateScheduleRequest {
    pub client_ids: Vec<i64>,
    pub repo_id: i64,
    pub name: Option<String>,
    #[schema(value_type = Option<String>)]
    pub schedule_type: Option<ScheduleType>,
    pub cron_expression: String,
    pub enabled: Option<bool>,
    pub canary_enabled: Option<bool>,
    pub exclude_patterns: Option<Vec<String>>,
    pub ignore_global_excludes: Option<bool>,
    pub keep_daily: Option<i32>,
    pub keep_weekly: Option<i32>,
    pub keep_monthly: Option<i32>,
    pub keep_yearly: Option<i32>,
    pub compact_enabled: Option<bool>,
    pub rate_limit_kbps: Option<u32>,
    pub pre_backup_commands: Option<Vec<String>>,
    pub post_backup_commands: Option<Vec<String>>,
    pub backup_sources: Option<Vec<String>>,
    pub backup_sources_per_host: Option<Vec<HostBackupSources>>,
    pub exclude_patterns_per_host: Option<Vec<HostExcludePatterns>>,
    #[schema(value_type = Option<String>)]
    pub execution_mode: Option<ExecutionMode>,
    #[schema(value_type = Option<String>)]
    pub on_failure: Option<OnFailure>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateScheduleRequest {
    pub name: Option<String>,
    pub cron_expression: String,
    pub enabled: Option<bool>,
    pub canary_enabled: Option<bool>,
    pub exclude_patterns: Option<Vec<String>>,
    pub ignore_global_excludes: Option<bool>,
    pub keep_daily: Option<i32>,
    pub keep_weekly: Option<i32>,
    pub keep_monthly: Option<i32>,
    pub keep_yearly: Option<i32>,
    pub compact_enabled: Option<bool>,
    pub rate_limit_kbps: Option<u32>,
    pub pre_backup_commands: Option<Vec<String>>,
    pub post_backup_commands: Option<Vec<String>>,
    pub backup_sources: Option<Vec<String>>,
    pub backup_sources_per_host: Option<Vec<HostBackupSources>>,
    pub exclude_patterns_per_host: Option<Vec<HostExcludePatterns>>,
    pub client_ids: Option<Vec<i64>>,
    #[schema(value_type = Option<String>)]
    pub execution_mode: Option<ExecutionMode>,
    #[schema(value_type = Option<String>)]
    pub on_failure: Option<OnFailure>,
}

#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "Schedules",
    operation_id = "listSchedules",
    summary = "List all schedules visible to the current user",
    responses(
        (status = 200, description = "List of schedules", body = Vec<crate::db::ScheduleRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_schedules(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ScheduleRow>>, ApiError> {
    let schedules = db::list_schedules(&state.pool).await?;
    let is_admin = auth.role == Role::Admin;
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
    summary = "Create a new backup schedule",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = crate::db::ScheduleRow),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 422, description = "Unprocessable -- SSH unreachable"),
    )
)]
pub async fn create_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(req): ApiJson<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleRow>), ApiError> {
    if req.client_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "client_ids must contain at least one entry".into(),
        ));
    }
    check_repo_permission(&state.pool, &auth, req.repo_id, |p| p.can_modify_schedules).await?;
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let exclude_patterns = req.exclude_patterns.unwrap_or_default();
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        let repo = db::get_repo_connection(&state.pool, req.repo_id).await?;
        let ssh_port = u16::try_from(repo.ssh_port).map_err(|_| {
            ApiError::Unprocessable("Cannot reach repository: invalid SSH port".into())
        })?;
        let response = ssh::test_connection(&TestConnectionRequest {
            ssh_host: repo.ssh_host,
            ssh_user: repo.ssh_user,
            ssh_port: Some(ssh_port),
        })
        .await;
        if !response.ssh_ok {
            return Err(ApiError::Unprocessable(format!(
                "Cannot reach repository: {}",
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            )));
        }
    }
    let schedule_type_enum = req.schedule_type.unwrap_or_default();
    let schedule_type = schedule_type_to_str(schedule_type_enum);

    let has_backup_sources = req.backup_sources.as_ref().is_some_and(|v| !v.is_empty());
    let has_per_host_sources = req
        .backup_sources_per_host
        .as_ref()
        .is_some_and(|v| !v.is_empty());

    if !has_backup_sources && !has_per_host_sources && schedule_type_enum == ScheduleType::Backup {
        let client = db::get_client_by_id(&state.pool, req.client_ids[0]).await?;
        if client.default_backup_paths.is_empty() {
            return Err(ApiError::BadRequest(
                "no backup sources provided and client has no default backup paths configured"
                    .into(),
            ));
        }
    }

    let execution_mode = req.execution_mode.unwrap_or_default();
    let on_failure = req.on_failure.unwrap_or_default();
    let execution_mode_str = execution_mode.to_string();
    let on_failure_str = on_failure.to_string();

    let params = ScheduleParams {
        name: req.name.as_deref().unwrap_or(""),
        schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(true),
        exclude_patterns: &exclude_patterns,
        ignore_global_excludes: req.ignore_global_excludes.unwrap_or(false),
        keep_daily: req.keep_daily.unwrap_or(7),
        keep_weekly: req.keep_weekly.unwrap_or(4),
        keep_monthly: req.keep_monthly.unwrap_or(6),
        keep_yearly: req.keep_yearly.unwrap_or(0),
        compact_enabled: req.compact_enabled.unwrap_or(true),
        rate_limit_kbps: match req.rate_limit_kbps {
            Some(rate_limit_kbps) => Some(
                i32::try_from(rate_limit_kbps)
                    .map_err(|_| ApiError::BadRequest("rate_limit_kbps is too large".into()))?,
            ),
            None => None,
        },
        pre_backup_commands: &serde_json::to_string(&req.pre_backup_commands.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_owned()),
        post_backup_commands: &serde_json::to_string(&req.post_backup_commands.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_owned()),
        execution_mode: &execution_mode_str,
        on_failure: &on_failure_str,
    };

    let schedule =
        db::insert_schedule(&state.pool, req.repo_id, &params, Some(auth.user_id)).await?;

    let targets: Vec<(i64, i32)> = req
        .client_ids
        .iter()
        .enumerate()
        .map(|(i, &cid)| {
            let order = i32::try_from(i).unwrap_or(0);
            (cid, order)
        })
        .collect();
    db::insert_schedule_targets(&state.pool, schedule.id, &targets).await?;

    if let Some(sources) = &req.backup_sources {
        for (i, path) in sources.iter().enumerate() {
            let sort_order =
                i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
            db::insert_backup_source_for_schedule(&state.pool, schedule.id, path, sort_order)
                .await?;
        }
    }

    if let Some(per_host) = &req.backup_sources_per_host {
        for entry in per_host {
            for (i, path) in entry.paths.iter().enumerate() {
                let sort_order = i32::try_from(i)
                    .map_err(|_| ApiError::BadRequest("too many sources".into()))?;
                db::insert_backup_source_for_schedule_client(
                    &state.pool,
                    schedule.id,
                    entry.client_id,
                    path,
                    sort_order,
                )
                .await?;
            }
        }
    }

    if let Some(per_host) = &req.exclude_patterns_per_host {
        for entry in per_host {
            for (i, pattern) in entry.patterns.iter().enumerate() {
                let sort_order = i32::try_from(i)
                    .map_err(|_| ApiError::BadRequest("too many exclude patterns".into()))?;
                db::insert_exclude_for_schedule_client(
                    &state.pool,
                    schedule.id,
                    entry.client_id,
                    pattern,
                    sort_order,
                )
                .await?;
            }
        }
    }

    if enabled {
        let now = chrono::Utc::now();
        let tz = db::get_schedule_timezone(&state.pool).await?;
        let next = calculate_next_run(&req.cron_expression, now, tz)
            .map_err(|e| ApiError::Internal(format!("failed to calculate next run: {e}")))?;
        db::set_next_run_at(&state.pool, schedule.id, next).await?;
    }

    config_assembler::push_config_to_all_schedule_targets(&state, schedule.id).await;

    Ok((StatusCode::CREATED, Json(schedule)))
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}",
    tag = "Schedules",
    operation_id = "getSchedule",
    summary = "Get a single schedule by ID",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Schedule details", body = crate::db::ScheduleRow),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
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
    summary = "Update an existing schedule",
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
pub async fn update_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateScheduleRequest>,
) -> Result<Json<ScheduleRow>, ApiError> {
    let existing = db::get_schedule_by_id(&state.pool, id).await?;
    check_repo_permission(&state.pool, &auth, existing.repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let exclude_patterns = req.exclude_patterns.unwrap_or_default();
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        let repo = db::get_repo_connection(&state.pool, existing.repo_id).await?;
        let ssh_port = u16::try_from(repo.ssh_port).map_err(|_| {
            ApiError::Unprocessable("Cannot reach repository: invalid SSH port".into())
        })?;
        let response = ssh::test_connection(&TestConnectionRequest {
            ssh_host: repo.ssh_host,
            ssh_user: repo.ssh_user,
            ssh_port: Some(ssh_port),
        })
        .await;
        if !response.ssh_ok {
            return Err(ApiError::Unprocessable(format!(
                "Cannot reach repository: {}",
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            )));
        }
    }

    let pre_cmds_json = req.pre_backup_commands.map_or_else(
        || existing.pre_backup_commands.clone(),
        |cmds| serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".to_owned()),
    );
    let post_cmds_json = req.post_backup_commands.map_or_else(
        || existing.post_backup_commands.clone(),
        |cmds| serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".to_owned()),
    );

    let execution_mode = req
        .execution_mode
        .map_or_else(|| existing.execution_mode.clone(), |m| m.to_string());
    let on_failure = req
        .on_failure
        .map_or_else(|| existing.on_failure.clone(), |f| f.to_string());

    let name = req.name.unwrap_or_else(|| existing.name.clone());

    let params = ScheduleParams {
        name: &name,
        schedule_type: &existing.schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(existing.canary_enabled),
        exclude_patterns: &exclude_patterns,
        ignore_global_excludes: req.ignore_global_excludes.unwrap_or(false),
        keep_daily: req.keep_daily.unwrap_or(existing.keep_daily),
        keep_weekly: req.keep_weekly.unwrap_or(existing.keep_weekly),
        keep_monthly: req.keep_monthly.unwrap_or(existing.keep_monthly),
        keep_yearly: req.keep_yearly.unwrap_or(existing.keep_yearly),
        compact_enabled: req.compact_enabled.unwrap_or(existing.compact_enabled),
        rate_limit_kbps: match req.rate_limit_kbps {
            Some(rate_limit_kbps) => Some(
                i32::try_from(rate_limit_kbps)
                    .map_err(|_| ApiError::BadRequest("rate_limit_kbps is too large".into()))?,
            ),
            None => existing.rate_limit_kbps,
        },
        pre_backup_commands: &pre_cmds_json,
        post_backup_commands: &post_cmds_json,
        execution_mode: &execution_mode,
        on_failure: &on_failure,
    };

    let schedule = db::update_schedule(&state.pool, id, &params).await?;

    if let Some(client_ids) = &req.client_ids {
        if client_ids.is_empty() {
            return Err(ApiError::BadRequest(
                "client_ids must contain at least one entry".into(),
            ));
        }
        db::delete_schedule_targets(&state.pool, schedule.id).await?;
        let targets: Vec<(i64, i32)> = client_ids
            .iter()
            .enumerate()
            .map(|(i, &cid)| {
                let order = i32::try_from(i).unwrap_or(0);
                (cid, order)
            })
            .collect();
        db::insert_schedule_targets(&state.pool, schedule.id, &targets).await?;
    }

    if let Some(sources) = &req.backup_sources {
        db::delete_backup_sources_for_schedule(&state.pool, schedule.id).await?;
        for (i, path) in sources.iter().enumerate() {
            let sort_order =
                i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
            db::insert_backup_source_for_schedule(&state.pool, schedule.id, path, sort_order)
                .await?;
        }
    }

    if let Some(per_host) = &req.backup_sources_per_host {
        db::delete_per_host_backup_sources_for_schedule(&state.pool, schedule.id).await?;
        for entry in per_host {
            for (i, path) in entry.paths.iter().enumerate() {
                let sort_order = i32::try_from(i)
                    .map_err(|_| ApiError::BadRequest("too many sources".into()))?;
                db::insert_backup_source_for_schedule_client(
                    &state.pool,
                    schedule.id,
                    entry.client_id,
                    path,
                    sort_order,
                )
                .await?;
            }
        }
    }

    if let Some(per_host) = &req.exclude_patterns_per_host {
        db::delete_per_host_excludes_for_schedule(&state.pool, schedule.id).await?;
        for entry in per_host {
            for (i, pattern) in entry.patterns.iter().enumerate() {
                let sort_order = i32::try_from(i)
                    .map_err(|_| ApiError::BadRequest("too many exclude patterns".into()))?;
                db::insert_exclude_for_schedule_client(
                    &state.pool,
                    schedule.id,
                    entry.client_id,
                    pattern,
                    sort_order,
                )
                .await?;
            }
        }
    }

    if enabled {
        let now = chrono::Utc::now();
        let tz = db::get_schedule_timezone(&state.pool).await?;
        let next = calculate_next_run(&req.cron_expression, now, tz)
            .map_err(|e| ApiError::Internal(format!("failed to calculate next run: {e}")))?;
        db::set_next_run_at(&state.pool, schedule.id, next).await?;
    } else {
        db::set_next_run_at(&state.pool, schedule.id, chrono::Utc::now()).await?;
    }

    config_assembler::push_config_to_all_schedule_targets(&state, schedule.id).await;

    Ok(Json(schedule))
}

#[utoipa::path(
    delete,
    path = "/api/schedules/{id}",
    tag = "Schedules",
    operation_id = "deleteSchedule",
    summary = "Delete a schedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let existing = db::get_schedule_by_id(&state.pool, id).await?;
    check_repo_permission(&state.pool, &auth, existing.repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;

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

#[utoipa::path(
    post,
    path = "/api/schedules/{id}/run",
    tag = "Schedules",
    operation_id = "runScheduleNow",
    summary = "Trigger a schedule to run immediately",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 202, description = "Accepted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn run_schedule_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    check_repo_permission(&state.pool, &auth, schedule.repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;

    let hostnames = db::get_schedule_target_hostnames(&state.pool, id).await?;
    let repo_id = RepoId(schedule.repo_id);
    let schedule_type = schedule
        .schedule_type
        .parse::<ScheduleType>()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let mut any_sent = false;
    for hostname in &hostnames {
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
                request_id: None,
            },
        };

        match state.registry.send_to(hostname, msg).await {
            Ok(()) => any_sent = true,
            Err(e) => tracing::warn!(
                hostname = %hostname,
                error = %e,
                "agent not connected for run_schedule_now"
            ),
        }
    }

    if any_sent {
        let now = Utc::now();
        let tz = db::get_schedule_timezone(&state.pool).await?;
        let next = calculate_next_run(&schedule.cron_expression, now, tz)
            .map_err(|e| ApiError::Internal(format!("cron error: {e}")))?;
        db::mark_schedule_triggered(&state.pool, id, now, next).await?;
    }

    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
pub struct ListScheduleReportsQuery {
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/reports",
    tag = "Schedules",
    operation_id = "listScheduleReports",
    summary = "List backup reports for a schedule",
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
    summary = "List target hosts for a schedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "List of targets", body = Vec<crate::db::ScheduleTargetRow>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn list_schedule_targets(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Vec<db::ScheduleTargetRow>>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let targets = db::list_schedule_targets(&state.pool, id).await?;
    Ok(Json(targets))
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ScheduleBackupSourcesResponse {
    pub backup_sources: Vec<String>,
    pub backup_sources_per_host: Vec<db::PerHostBackupSources>,
    pub exclude_patterns_per_host: Vec<db::PerHostExcludePatterns>,
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/sources",
    tag = "Schedules",
    operation_id = "listScheduleBackupSources",
    summary = "List backup sources for a schedule (schedule-level and per-host)",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Backup sources", body = ScheduleBackupSourcesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn list_schedule_backup_sources(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScheduleBackupSourcesResponse>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let backup_sources = db::list_backup_sources_for_schedule(&state.pool, id).await?;
    let backup_sources_per_host =
        db::list_all_per_host_backup_sources_for_schedule(&state.pool, id).await?;
    let exclude_patterns_per_host =
        db::list_all_per_host_excludes_for_schedule(&state.pool, id).await?;
    Ok(Json(ScheduleBackupSourcesResponse {
        backup_sources,
        backup_sources_per_host,
        exclude_patterns_per_host,
    }))
}
