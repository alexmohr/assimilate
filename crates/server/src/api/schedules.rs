// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    protocol::ServerToAgent,
    schedule::{calculate_next_run, validate_cron},
    types::{RepoId, ScheduleType},
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
pub struct CreateScheduleRequest {
    pub client_id: i64,
    pub repo_id: i64,
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
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateScheduleRequest {
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

    if !has_backup_sources && schedule_type_enum == ScheduleType::Backup {
        let client = db::get_client_by_id(&state.pool, req.client_id).await?;
        if client.default_backup_paths.is_empty() {
            return Err(ApiError::BadRequest(
                "no backup sources provided and client has no default backup paths configured"
                    .into(),
            ));
        }
    }

    let params = ScheduleParams {
        schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(false),
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
    };

    let schedule = db::insert_schedule(
        &state.pool,
        req.client_id,
        req.repo_id,
        &params,
        Some(auth.user_id),
    )
    .await?;

    if let Some(sources) = &req.backup_sources {
        for (i, path) in sources.iter().enumerate() {
            let sort_order =
                i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
            db::insert_backup_source_for_schedule(&state.pool, schedule.id, path, sort_order)
                .await?;
        }
    }

    if enabled {
        let now = chrono::Utc::now();
        let tz = db::get_schedule_timezone(&state.pool).await?;
        let next = calculate_next_run(&req.cron_expression, now, tz)
            .map_err(|e| ApiError::Internal(format!("failed to calculate next run: {e}")))?;
        db::set_next_run_at(&state.pool, schedule.id, next).await?;
    }

    if let Ok(hostname) = db::get_client_hostname_for_schedule(&state.pool, schedule.id).await {
        config_assembler::push_config_to_agent(&state, &hostname).await;
    }

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

    let params = ScheduleParams {
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
    };

    let schedule = db::update_schedule(&state.pool, id, &params).await?;

    if let Some(sources) = &req.backup_sources {
        db::delete_backup_sources_for_schedule(&state.pool, schedule.id).await?;
        for (i, path) in sources.iter().enumerate() {
            let sort_order =
                i32::try_from(i).map_err(|_| ApiError::BadRequest("too many sources".into()))?;
            db::insert_backup_source_for_schedule(&state.pool, schedule.id, path, sort_order)
                .await?;
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

    if let Ok(hostname) = db::get_client_hostname_for_schedule(&state.pool, schedule.id).await {
        config_assembler::push_config_to_agent(&state, &hostname).await;
    }

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

    let hostname = db::get_client_hostname_for_schedule(&state.pool, id)
        .await
        .inspect_err(|e| {
            tracing::warn!(schedule_id = id, error = %e, "failed to look up hostname for schedule");
        })
        .ok();

    db::delete_schedule(&state.pool, id).await?;

    if let Some(hostname) = hostname {
        config_assembler::push_config_to_agent(&state, &hostname).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/schedules/{id}/clone",
    tag = "Schedules",
    operation_id = "cloneSchedule",
    summary = "Clone an existing schedule",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 201, description = "Cloned schedule", body = crate::db::ScheduleRow),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn clone_schedule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<(StatusCode, Json<ScheduleRow>), ApiError> {
    let existing = db::get_schedule_by_id(&state.pool, id).await?;
    check_repo_permission(&state.pool, &auth, existing.repo_id, |p| {
        p.can_modify_schedules
    })
    .await?;

    let Some(client_id) = existing.client_id else {
        return Err(ApiError::BadRequest(
            "cannot clone schedule without a client".into(),
        ));
    };

    let params = ScheduleParams {
        schedule_type: &existing.schedule_type,
        cron_expression: &existing.cron_expression,
        enabled: false,
        canary_enabled: existing.canary_enabled,
        exclude_patterns: &existing.exclude_patterns,
        ignore_global_excludes: existing.ignore_global_excludes,
        keep_daily: existing.keep_daily,
        keep_weekly: existing.keep_weekly,
        keep_monthly: existing.keep_monthly,
        keep_yearly: existing.keep_yearly,
        compact_enabled: existing.compact_enabled,
        rate_limit_kbps: existing.rate_limit_kbps,
        pre_backup_commands: &existing.pre_backup_commands,
        post_backup_commands: &existing.post_backup_commands,
    };

    let schedule = db::insert_schedule(
        &state.pool,
        client_id,
        existing.repo_id,
        &params,
        existing.owner_id,
    )
    .await?;

    let backup_sources = db::list_backup_sources_for_schedule(&state.pool, existing.id).await?;
    for (i, path) in backup_sources.iter().enumerate() {
        let sort_order =
            i32::try_from(i).map_err(|_| ApiError::BadRequest("too many backup sources".into()))?;
        db::insert_backup_source_for_schedule(&state.pool, schedule.id, path, sort_order).await?;
    }

    if let Ok(hostname) = db::get_client_hostname_for_schedule(&state.pool, schedule.id).await {
        config_assembler::push_config_to_agent(&state, &hostname).await;
    }

    Ok((StatusCode::CREATED, Json(schedule)))
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

    let hostname = db::get_client_hostname_for_schedule(&state.pool, id).await?;
    let repo_id = RepoId(schedule.repo_id);
    let schedule_type = schedule
        .schedule_type
        .parse::<ScheduleType>()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

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

    state
        .registry
        .send_to(&hostname, msg)
        .await
        .map_err(|e| ApiError::Internal(format!("agent not connected: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}
