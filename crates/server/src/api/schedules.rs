// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    hooks::{HookCommand, MAX_HOOK_COMMAND_TIMEOUT_SECONDS},
    protocol::{ServerToAgent, ServerToUi},
    responses::{
        DeleteFailedReportsResponse, FailedReportCountResponse, PerAgentBackupSourcesResponse,
        PerAgentCommandsResponse, PerAgentExcludePatternsResponse,
        PerAgentFileChangePatternsResponse, ScheduleBackupSourcesResponse, ScheduleRepoResponse,
        ScheduleTargetResponse,
    },
    schedule::{calculate_next_run, validate_cron},
    types::{OnFailure, RepoId, ScheduleType, ScheduleWakeOverride},
};
use sqlx::PgPool;

impl From<db::ScheduleTargetRow> for ScheduleTargetResponse {
    fn from(t: db::ScheduleTargetRow) -> Self {
        Self {
            agent_id: t.agent_id,
            execution_order: t.execution_order,
            catch_up_pending_for: t.catch_up_pending_for,
        }
    }
}

impl From<db::ScheduleRepoRow> for ScheduleRepoResponse {
    fn from(t: db::ScheduleRepoRow) -> Self {
        Self {
            repo_id: t.repo_id,
            execution_order: t.execution_order,
            required: t.required,
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
    run_dispatch,
    ssh::{self, TestConnectionRequest},
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
    pub pre_backup_commands: Vec<HookCommand>,
    /// Commands to run after the backup.
    pub post_backup_commands: Vec<HookCommand>,
}

/// Per-agent file change detection patterns for a schedule target.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AgentFileChangePatterns {
    /// The agent's ID.
    pub agent_id: i64,
    /// Raw file change pattern text.
    pub raw_text: String,
}

/// One repository a schedule writes into.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ScheduleRepoInput {
    /// Repository ID.
    pub repo_id: i64,
    /// Whether a failure on this repository fails the whole run. Defaults to
    /// true; a best-effort target only warns and never stops the run.
    pub required: Option<bool>,
}

/// Request payload for creating a new backup schedule.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateScheduleRequest {
    /// IDs of agents to assign as targets.
    pub agent_ids: Vec<i64>,
    /// Repository ID to back up to. When `repo_targets` is given, this is
    /// ignored in favour of its first entry.
    pub repo_id: i64,
    /// Every repository this schedule writes into, in write order. Omit for a
    /// single-target schedule writing to `repo_id`.
    pub repo_targets: Option<Vec<ScheduleRepoInput>>,
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
    /// Whether this schedule stages the host's virtual machines before
    /// backing up. Requires the host itself to have staging enabled.
    pub vm_snapshot_enabled: Option<bool>,
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
    pub pre_backup_commands: Option<Vec<HookCommand>>,
    /// Commands to run after the backup.
    pub post_backup_commands: Option<Vec<HookCommand>>,
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: Option<i32>,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled (defaults to 3).
    pub missed_backup_threshold: Option<i32>,
    /// Whether a run missed while a target host was unreachable is caught up once
    /// that host reconnects (defaults to false).
    pub catch_up_missed_runs: Option<bool>,
    /// How much time, in minutes, must be left before the next scheduled run for a
    /// catch-up to still start (defaults to 120).
    pub catch_up_min_lead_minutes: Option<i32>,
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
    /// Whether this schedule wakes the hosts it needs, overriding the hosts'
    /// own defaults (defaults to `host_default`).
    #[schema(value_type = Option<String>)]
    pub wake_override: Option<ScheduleWakeOverride>,
}

/// Request payload for updating an existing schedule.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateScheduleRequest {
    /// Optional new display name.
    pub name: Option<String>,
    /// Updated cron expression.
    pub cron_expression: String,
    /// New repository ID to assign. Replaces the whole target list; send
    /// `repo_targets` instead to keep more than one.
    pub repo_id: Option<i64>,
    /// Replacement list of repositories this schedule writes into, in write
    /// order. Takes precedence over `repo_id`.
    pub repo_targets: Option<Vec<ScheduleRepoInput>>,
    /// Whether the schedule is enabled.
    pub enabled: Option<bool>,
    /// Whether canary backups are enabled.
    pub canary_enabled: Option<bool>,
    /// Whether this schedule stages the host's virtual machines before
    /// backing up. Requires the host itself to have staging enabled.
    pub vm_snapshot_enabled: Option<bool>,
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
    pub pre_backup_commands: Option<Vec<HookCommand>>,
    /// Commands to run after the backup.
    pub post_backup_commands: Option<Vec<HookCommand>>,
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: Option<i32>,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled.
    pub missed_backup_threshold: Option<i32>,
    /// Whether a run missed while a target host was unreachable is caught up once
    /// that host reconnects.
    pub catch_up_missed_runs: Option<bool>,
    /// How much time, in minutes, must be left before the next scheduled run for a
    /// catch-up to still start.
    pub catch_up_min_lead_minutes: Option<i32>,
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
    /// Whether this schedule wakes the hosts it needs, overriding the hosts'
    /// own defaults. Left unchanged when omitted.
    #[schema(value_type = Option<String>)]
    pub wake_override: Option<ScheduleWakeOverride>,
}

/// Reads a schedule row's stored `wake_override`. A value the database
/// somehow holds outside the enum's set falls back to the default rather
/// than failing the request, matching how the scheduler treats `on_failure`.
fn stored_wake_override(schedule: &db::ScheduleRow) -> ScheduleWakeOverride {
    ScheduleWakeOverride::from_db_value(schedule.id, &schedule.wake_override)
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

/// A backup schedule needs sources from somewhere: the request, its per-agent
/// overrides, or the first agent's own defaults. Checked before the schedule is
/// written, so one that could never back anything up is refused rather than
/// created and left to fail at run time.
async fn ensure_backup_sources_available(
    state: &AppState,
    req: &CreateScheduleRequest,
    schedule_type: ScheduleType,
) -> Result<(), ApiError> {
    if schedule_type != ScheduleType::Backup {
        return Ok(());
    }
    let has_backup_sources = req.backup_sources.as_ref().is_some_and(|v| !v.is_empty());
    let has_per_agent_sources = req
        .backup_sources_per_agent
        .as_ref()
        .is_some_and(|v| !v.is_empty());
    if has_backup_sources || has_per_agent_sources {
        return Ok(());
    }

    let Some(&first_agent_id) = req.agent_ids.first() else {
        return Err(ApiError::BadRequest(
            "agent_ids must contain at least one entry".into(),
        ));
    };
    let agent = db::get_agent_by_id(&state.pool, first_agent_id).await?;
    if agent.default_backup_paths.is_empty() {
        return Err(ApiError::BadRequest(
            "no backup sources provided and agent has no default backup paths configured".into(),
        ));
    }
    Ok(())
}

/// Normalises the repositories a create/update request asks for into
/// `(repo_id, required)` pairs in write order.
///
/// `repo_targets` wins when present; otherwise the request is a single-target
/// one and `fallback_repo_id` is that target. Rejects a list that is empty,
/// names the same repository twice, or leaves no required target - the last
/// would let a run report success without a single copy having been written.
fn resolve_repo_targets(
    repo_targets: Option<&[ScheduleRepoInput]>,
    fallback_repo_id: Option<i64>,
) -> Result<Vec<(i64, bool)>, ApiError> {
    let Some(targets) = repo_targets else {
        return Ok(fallback_repo_id.map_or_else(Vec::new, |repo_id| vec![(repo_id, true)]));
    };
    if targets.is_empty() {
        return Err(ApiError::BadRequest(
            "repo_targets must contain at least one entry".into(),
        ));
    }
    let resolved: Vec<(i64, bool)> = targets
        .iter()
        .map(|t| (t.repo_id, t.required.unwrap_or(true)))
        .collect();
    let unique: std::collections::HashSet<i64> = resolved.iter().map(|(id, _)| *id).collect();
    if unique.len() != resolved.len() {
        return Err(ApiError::BadRequest(
            "repo_targets must not name the same repository twice".into(),
        ));
    }
    if !resolved.iter().any(|(_, required)| *required) {
        return Err(ApiError::BadRequest(
            "at least one target repository must be required".into(),
        ));
    }
    Ok(resolved)
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
    let repo_targets = resolve_repo_targets(req.repo_targets.as_deref(), Some(req.repo_id))?;
    for (repo_id, _) in &repo_targets {
        check_repo_permission(&state.pool, &auth, *repo_id, |p| p.can_modify_schedules).await?;
    }
    let primary_repo_id = repo_targets
        .first()
        .map_or(req.repo_id, |(repo_id, _)| *repo_id);
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let schedule_type_enum = req.schedule_type.unwrap_or_default();
    let schedule_type = schedule_type_to_str(schedule_type_enum);
    // Before any field is taken out of `req`, which would leave it partially
    // moved and unborrowable.
    ensure_backup_sources_available(&state, &req, schedule_type_enum).await?;

    let exclude_patterns_raw = req.exclude_patterns_raw.unwrap_or_default();
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        for (repo_id, _) in &repo_targets {
            check_ssh_reachability(&state.pool, *repo_id).await?;
        }
    }

    let on_failure = req.on_failure.unwrap_or_default();
    let on_failure_str = on_failure.to_string();
    let pre_backup_commands = req.pre_backup_commands.unwrap_or_default();
    let post_backup_commands = req.post_backup_commands.unwrap_or_default();
    validate_hook_commands(&pre_backup_commands)?;
    validate_hook_commands(&post_backup_commands)?;
    let hook_timeout_seconds =
        validate_hook_timeout_seconds(req.hook_timeout_seconds.unwrap_or(60))?;
    let missed_backup_threshold =
        validate_missed_backup_threshold(req.missed_backup_threshold.unwrap_or(3))?;
    let catch_up_min_lead_minutes =
        validate_catch_up_min_lead_minutes(req.catch_up_min_lead_minutes.unwrap_or(120))?;

    let params = ScheduleParams {
        wake_override: req.wake_override.unwrap_or_default(),
        name: req.name.as_deref().unwrap_or(""),
        schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(true),
        vm_snapshot_enabled: req.vm_snapshot_enabled.unwrap_or(false),
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
        pre_backup_commands: &pre_backup_commands,
        post_backup_commands: &post_backup_commands,
        hook_timeout_seconds,
        missed_backup_threshold,
        catch_up_missed_runs: req.catch_up_missed_runs.unwrap_or(false),
        catch_up_min_lead_minutes,
        on_failure: &on_failure_str,
    };

    let schedule =
        db::insert_schedule(&state.pool, primary_repo_id, &params, Some(auth.user_id)).await?;

    // `insert_schedule` seeds the primary target; anything beyond a single
    // required repository replaces that seed with the requested list.
    //
    // Two transactions rather than one, which is safe only because a schedule
    // is not dispatchable until `refresh_next_run` below sets `next_run_at` -
    // `list_due_schedules` requires it to be non-null, and it is written last,
    // after both the target list and the agent targets. Keep it last: moving
    // it earlier would open a window where a tick dispatches a run that
    // silently skips every secondary target.
    if req.repo_targets.is_some() {
        db::replace_schedule_repos(&state.pool, schedule.id, &repo_targets).await?;
    }

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

/// What an update leaves a schedule pointed at, worked out once so the
/// permission check, the reachability check and the write all agree.
struct RepoTargetPlan {
    /// The replacement target list, or `None` when the request did not send one.
    requested: Option<Vec<(i64, bool)>>,
    /// The repository `schedules.repo_id` ends up holding.
    effective_repo_id: Option<i64>,
    /// The repositories the schedule was already writing to.
    existing_targets: Vec<i64>,
}

/// Every repository an enabled schedule is about to write to has to be
/// reachable over SSH, not just the primary one - a second target that cannot
/// be reached would only surface as a failed run hours later.
async fn check_targets_reachable(
    pool: &PgPool,
    plan: &RepoTargetPlan,
    existing_primary: Option<i64>,
) -> Result<(), ApiError> {
    if plan.effective_repo_id.is_none() {
        return Err(ApiError::BadRequest(
            "cannot enable a schedule with no repository assigned".into(),
        ));
    }
    for repo_id in resulting_repo_targets(plan, existing_primary) {
        check_ssh_reachability(pool, repo_id).await?;
    }
    Ok(())
}

/// The repositories the schedule writes to once the update lands.
///
/// A request carrying `repo_targets` replaces the list outright, and a legacy
/// bare-`repo_id` update that moves the primary collapses the schedule onto
/// that one repository (`db::update_schedule_repo`). Anything else - a rename,
/// a re-time, a plain re-enable - keeps the list the schedule already has, so
/// re-enabling a multi-target schedule has to reach its secondary targets too,
/// not just the primary.
fn resulting_repo_targets(plan: &RepoTargetPlan, existing_primary: Option<i64>) -> Vec<i64> {
    if let Some(targets) = plan.requested.as_deref() {
        return targets.iter().map(|(repo_id, _)| *repo_id).collect();
    }
    let Some(repo_id) = plan.effective_repo_id else {
        return Vec::new();
    };
    if plan.effective_repo_id != existing_primary || plan.existing_targets.is_empty() {
        return vec![repo_id];
    }
    plan.existing_targets.clone()
}

/// Works out which repositories an update leaves the schedule writing to, and
/// checks the caller may point it at each one it did not already own.
///
/// Returns the plan the rest of the update works from.
async fn authorize_repo_targets(
    state: &AppState,
    auth: &AuthUser,
    req: &UpdateScheduleRequest,
    existing: &db::ScheduleRow,
) -> Result<RepoTargetPlan, ApiError> {
    let requested = req
        .repo_targets
        .as_deref()
        .map(|targets| resolve_repo_targets(Some(targets), None))
        .transpose()?;
    let effective_repo_id: Option<i64> = requested
        .as_ref()
        .and_then(|targets| targets.first().map(|(repo_id, _)| *repo_id))
        .or(req.repo_id)
        .or(existing.repo_id);
    let existing_targets: Vec<i64> = db::list_schedule_repos(&state.pool, existing.id)
        .await?
        .into_iter()
        .map(|target| target.repo_id)
        .collect();
    for repo_id in newly_targeted_repos(
        requested.as_deref(),
        effective_repo_id,
        &existing_targets,
        existing.repo_id,
    ) {
        check_repo_permission(&state.pool, auth, repo_id, |p| p.can_modify_schedules).await?;
    }
    Ok(RepoTargetPlan {
        requested,
        effective_repo_id,
        existing_targets,
    })
}

/// The repositories an update points the schedule at that it was not already
/// writing to - the only ones the caller needs fresh permission for.
///
/// Measured against the schedule's whole target list, not just its primary.
/// `ScheduleDetailView` sends the full list on every save, so measuring
/// against the primary alone re-checks every secondary target on every edit:
/// an operator with `can_modify_schedules` on the local repository but not on
/// the offsite one it also writes to could never rename, pause or re-time
/// that schedule again.
fn newly_targeted_repos(
    requested: Option<&[(i64, bool)]>,
    effective_repo_id: Option<i64>,
    existing_targets: &[i64],
    existing_primary: Option<i64>,
) -> Vec<i64> {
    let already_targeted =
        |repo_id: &i64| existing_targets.contains(repo_id) || existing_primary == Some(*repo_id);
    requested.map_or_else(
        || {
            effective_repo_id
                .filter(|repo_id| !already_targeted(repo_id))
                .into_iter()
                .collect()
        },
        |targets| {
            targets
                .iter()
                .map(|(repo_id, _)| *repo_id)
                .filter(|repo_id| !already_targeted(repo_id))
                .collect()
        },
    )
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
    let target_plan = authorize_repo_targets(&state, &auth, &req, &existing).await?;
    validate_cron(&req.cron_expression)
        .map_err(|e| ApiError::BadRequest(format!("invalid cron expression: {e}")))?;
    let exclude_patterns_raw = req
        .exclude_patterns_raw
        .clone()
        .unwrap_or_else(|| existing.exclude_patterns_raw.clone());
    let enabled = req.enabled.unwrap_or(true);
    if enabled {
        check_targets_reachable(&state.pool, &target_plan, existing.repo_id).await?;
    }

    let pre_backup_commands = req
        .pre_backup_commands
        .clone()
        .unwrap_or_else(|| existing.pre_backup_commands.0.clone());
    let post_backup_commands = req
        .post_backup_commands
        .clone()
        .unwrap_or_else(|| existing.post_backup_commands.0.clone());
    validate_hook_commands(&pre_backup_commands)?;
    validate_hook_commands(&post_backup_commands)?;
    let hook_timeout_seconds = validate_hook_timeout_seconds(
        req.hook_timeout_seconds
            .unwrap_or(existing.hook_timeout_seconds),
    )?;
    let missed_backup_threshold = validate_missed_backup_threshold(
        req.missed_backup_threshold
            .unwrap_or(existing.missed_backup_threshold),
    )?;
    let catch_up_missed_runs = req
        .catch_up_missed_runs
        .unwrap_or(existing.catch_up_missed_runs);
    let catch_up_min_lead_minutes = validate_catch_up_min_lead_minutes(
        req.catch_up_min_lead_minutes
            .unwrap_or(existing.catch_up_min_lead_minutes),
    )?;

    let on_failure = req
        .on_failure
        .map_or_else(|| existing.on_failure.clone(), |f| f.to_string());

    let name = req.name.clone().unwrap_or_else(|| existing.name.clone());

    let params = ScheduleParams {
        wake_override: req
            .wake_override
            .unwrap_or_else(|| stored_wake_override(&existing)),
        name: &name,
        schedule_type: &existing.schedule_type,
        cron_expression: &req.cron_expression,
        enabled,
        canary_enabled: req.canary_enabled.unwrap_or(existing.canary_enabled),
        vm_snapshot_enabled: req
            .vm_snapshot_enabled
            .unwrap_or(existing.vm_snapshot_enabled),
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
        pre_backup_commands: &pre_backup_commands,
        post_backup_commands: &post_backup_commands,
        hook_timeout_seconds,
        missed_backup_threshold,
        catch_up_missed_runs,
        catch_up_min_lead_minutes,
        on_failure: &on_failure,
    };

    match &target_plan.requested {
        Some(targets) => db::replace_schedule_repos(&state.pool, id, targets).await?,
        None => {
            if target_plan.effective_repo_id != existing.repo_id
                && let Some(new_rid) = target_plan.effective_repo_id
            {
                db::update_schedule_repo(&state.pool, id, new_rid).await?;
            }
        }
    }
    let schedule = db::update_schedule(&state.pool, id, &params).await?;

    // A miss recorded while catch-up was on must not run days later because
    // somebody switched the setting back on in the meantime.
    if !catch_up_missed_runs {
        db::catch_up::clear_catch_up_pending_for_schedule(&state.pool, schedule.id).await?;
    }

    apply_schedule_target_overrides(&state.pool, schedule.id, &req).await?;

    if enabled {
        refresh_next_run(&state.pool, schedule.id, &req.cron_expression).await?;
    } else {
        db::set_next_run_at(&state.pool, schedule.id, chrono::Utc::now()).await?;
    }

    config_assembler::push_config_to_all_schedule_targets(&state, schedule.id).await;

    Ok(Json(schedule))
}

/// Whether this caller may edit `existing`, including moving it to a different
/// repository. An orphaned schedule (no repository to check against) is admin-only;
/// a move is checked against both the old and the new repository.
async fn check_schedule_edit_permission(
    state: &AppState,
    auth: &AuthUser,
    existing: &ScheduleRow,
    effective_repo_id: Option<i64>,
) -> Result<(), ApiError> {
    if let Some(rid) = existing.repo_id {
        check_repo_permission(&state.pool, auth, rid, |p| p.can_modify_schedules).await?;
    } else if !db::get_effective_permissions(&state.pool, auth.user_id)
        .await?
        .can_delete_repo
    {
        return Err(ApiError::Forbidden(
            "only admins can edit orphaned schedules".into(),
        ));
    }
    if effective_repo_id != existing.repo_id
        && let Some(new_rid) = effective_repo_id
    {
        check_repo_permission(&state.pool, auth, new_rid, |p| p.can_modify_schedules).await?;
    }
    Ok(())
}

/// Not wrapped in a transaction or a `lock_schedules_targeting_agent`-style row lock,
/// unlike `delete_agent`/`merge_agent`: a concurrent `record_schedule_failure` for
/// this schedule can in principle land between the `old_agent_ids` read below and the
/// `reset_schedule_failure_tracking_if_target_dropped` call at the end, using
/// `old_agent_ids`/`new_agent_ids` values that no longer reflect that write.
///
/// Accepted as a narrower, self-correcting case rather than hardened like delete/merge:
/// `reset_schedule_failure_tracking_if_target_dropped`'s "already auto-disabled" branch
/// (the one that matters once a concurrent failure write has landed) decides purely
/// from the schedule row's *live* `auto_disabled_by_agent_id` against `new_agent_ids` -
/// it never reads `old_agent_ids` in that branch - so a failure write racing in here
/// doesn't make it decide wrong, only (at worst) run one write behind, which the next
/// scheduler tick or retarget naturally corrects. Contrast `delete_agent`/`merge_agent`,
/// where the same race can permanently strand a schedule with no agent left to ever
/// reconnect and fix it - that's what actually needed the row lock.
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
        let old_agent_ids: Vec<i64> = db::list_schedule_targets(pool, schedule_id)
            .await?
            .into_iter()
            .map(|t| t.agent_id)
            .collect();
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
        db::reset_schedule_failure_tracking_if_target_dropped(
            pool,
            schedule_id,
            &old_agent_ids,
            agent_ids,
        )
        .await?;
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

    let targets = db::get_schedule_targets_for_run(&state.pool, id).await.ok();

    db::delete_schedule(&state.pool, id).await?;

    if let Some(targets) = targets {
        for target in &targets {
            config_assembler::push_config_to_agent(&state, target.agent_id).await;
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

/// Upper bound on a pre/post-backup hook command's timeout. Generous enough for
/// a slow disk-snapshot commit or database dump, but still short enough that a
/// stuck hook can't stall a schedule indefinitely.
///
/// `pub(crate)`: also used by `config_io::import_schedule` to clamp an
/// imported config's value into the same bound the REST create/update paths
/// enforce, rather than letting an uploaded export bypass it entirely.
pub(crate) const MAX_HOOK_TIMEOUT_SECONDS: i32 = 3600;

fn validate_hook_timeout_seconds(seconds: i32) -> Result<i32, ApiError> {
    if seconds <= 0 || seconds > MAX_HOOK_TIMEOUT_SECONDS {
        return Err(ApiError::BadRequest(format!(
            "hook_timeout_seconds must be between 1 and {MAX_HOOK_TIMEOUT_SECONDS}"
        )));
    }
    Ok(seconds)
}

/// Validates the per-command timeout a hook may carry instead of inheriting
/// the schedule's [`MAX_HOOK_TIMEOUT_SECONDS`]-bounded default.
///
/// `pub(crate)`: also used by `agents::update_agent` for an agent's default
/// hook commands, and by `config_io` for imported configurations, so every
/// path that can store a hook command enforces the same bound.
pub(crate) fn validate_hook_commands(commands: &[HookCommand]) -> Result<(), ApiError> {
    commands
        .iter()
        .try_for_each(|cmd| match cmd.timeout_seconds {
            Some(seconds) if seconds == 0 || seconds > MAX_HOOK_COMMAND_TIMEOUT_SECONDS => {
                Err(ApiError::BadRequest(format!(
                    "hook command timeout_seconds must be between 1 and \
                     {MAX_HOOK_COMMAND_TIMEOUT_SECONDS}"
                )))
            }
            Some(_) | None => Ok(()),
        })
}

/// Upper bound on how many consecutive missed backups a schedule can tolerate
/// before being marked failed and auto-disabled. Generous enough to ride out an
/// extended agent outage, but still bounded so a schedule can't be configured to
/// never fail.
///
/// `pub(crate)`: also used by `config_io::import_schedule` to clamp an imported
/// config's value into the same bound the REST create/update paths enforce.
pub(crate) const MAX_MISSED_BACKUP_THRESHOLD: i32 = 1000;

fn validate_missed_backup_threshold(threshold: i32) -> Result<i32, ApiError> {
    if threshold <= 0 || threshold > MAX_MISSED_BACKUP_THRESHOLD {
        return Err(ApiError::BadRequest(format!(
            "missed_backup_threshold must be between 1 and {MAX_MISSED_BACKUP_THRESHOLD}"
        )));
    }
    Ok(threshold)
}

/// Upper bound on the catch-up lead time: a week, past which the floor would
/// exceed any realistic gap between two runs and switch catch-up off by accident
/// rather than by choice.
///
/// The lower bound is one minute, not zero: the reconnect handler re-enables a
/// schedule its host's outage auto-disabled by setting `next_run_at` to the
/// reconnect moment, and a zero floor would let that schedule both catch up and
/// run again on the scheduler tick seconds later.
///
/// `pub(crate)`: also used by `config_io::import_schedule` to clamp an imported
/// config's value into the same bound the REST create/update paths enforce.
pub(crate) const MAX_CATCH_UP_MIN_LEAD_MINUTES: i32 = 10_080;

fn validate_catch_up_min_lead_minutes(minutes: i32) -> Result<i32, ApiError> {
    if minutes <= 0 || minutes > MAX_CATCH_UP_MIN_LEAD_MINUTES {
        return Err(ApiError::BadRequest(format!(
            "catch_up_min_lead_minutes must be between 1 and {MAX_CATCH_UP_MIN_LEAD_MINUTES}"
        )));
    }
    Ok(minutes)
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
        validate_hook_commands(&entry.pre_backup_commands)?;
        validate_hook_commands(&entry.post_backup_commands)?;
        db::upsert_per_agent_commands(
            pool,
            schedule_id,
            entry.agent_id,
            &entry.pre_backup_commands,
            &entry.post_backup_commands,
        )
        .await?;
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

/// Request payload for triggering a schedule run.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RunScheduleRequest {
    /// Restrict the run to these agent IDs (must all be targets of the
    /// schedule). Omit or leave empty to run every target.
    #[serde(default)]
    pub agent_ids: Option<Vec<i64>>,
}

#[utoipa::path(
    post,
    path = "/api/schedules/{id}/run",
    tag = "Schedules",
    operation_id = "runScheduleNow",
    params(("id" = i64, Path, description = "Schedule ID")),
    request_body = RunScheduleRequest,
    responses(
        (status = 202, description = "Accepted"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Trigger a schedule to run immediately, optionally restricted to a subset
/// of its target agents. The body is optional - a request sent with no
/// `Content-Type` header (as any caller predating the `agent_ids` filter
/// would send) is treated the same as `{}`, running every target, so this
/// stays backwards compatible with callers outside this codebase.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid, e.g. an
/// `agent_ids` entry that is not a target of this schedule.
pub async fn run_schedule_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    payload: Option<Json<RunScheduleRequest>>,
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

    let agent_ids = payload.and_then(|Json(p)| p.agent_ids);
    let targets = db::get_schedule_targets_for_run(&state.pool, id).await?;
    let targets = match agent_ids {
        Some(agent_ids) if !agent_ids.is_empty() => {
            let requested: std::collections::HashSet<i64> = agent_ids.into_iter().collect();
            let filtered: Vec<_> = targets
                .into_iter()
                .filter(|t| requested.contains(&t.agent_id))
                .collect();
            if filtered.len() != requested.len() {
                return Err(ApiError::BadRequest(
                    "one or more agent_ids are not targets of this schedule".into(),
                ));
            }
            filtered
        }
        _ => targets,
    };
    let repo_id = RepoId(schedule_repo_id);
    let schedule_type = schedule
        .schedule_type
        .parse::<ScheduleType>()
        .map_err(|e: strum::ParseError| ApiError::BadRequest(e.to_string()))?;
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

    tokio::spawn(run_dispatch::run_targets_sequential(
        state,
        targets,
        run_dispatch::RunRequest {
            repo_id,
            schedule_type,
            schedule_id: id,
            run_id,
            origin: run_dispatch::RunOrigin::Manual,
        },
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

    let targets = db::get_schedule_targets_for_run(&state.pool, id).await?;
    let repo_id = RepoId(schedule_repo_id);

    for target in &targets {
        let msg = ServerToAgent::CancelBackup { repo_id };
        if let Err(e) = state.registry.send_to(target.agent_id, msg).await {
            tracing::warn!(
                hostname = %target.hostname,
                error = %e,
                "agent not connected for cancel_running_backup"
            );
            // Agent is offline - cancel the backup directly in the DB
            if let Err(e) =
                db::cancel_backup_report(&state.pool, target.agent_id, schedule_repo_id).await
            {
                tracing::error!(
                    hostname = %target.hostname,
                    error = %e,
                    "failed to cancel backup in DB after agent not connected"
                );
            }
            state
                .completion_bus
                .publish(crate::ws::completion_bus::OperationOutcome {
                    agent_id: target.agent_id,
                    repo_id: schedule_repo_id,
                    success: false,
                });
            state.ui_broadcast.clear_active_backup(schedule_repo_id);
            state.ui_broadcast.send(ServerToUi::DataChanged);
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
    delete,
    path = "/api/schedules/{id}/reports/failed",
    tag = "Schedules",
    operation_id = "deleteFailedScheduleReports",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Failed reports deleted", body = DeleteFailedReportsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
/// Delete all failed backup reports for a schedule.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn delete_failed_schedule_reports(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<DeleteFailedReportsResponse>, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, id).await?;
    if let Some(rid) = schedule.repo_id {
        check_repo_permission(&state.pool, &auth, rid, |p| p.can_modify_schedules).await?;
    } else {
        let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
        if !effective.can_delete_repo {
            return Err(ApiError::Forbidden(
                "only admins can delete orphaned schedules' reports".into(),
            ));
        }
    }
    let deleted = db::delete_failed_backup_reports_for_schedule(&state.pool, id).await?;
    Ok(Json(DeleteFailedReportsResponse { deleted }))
}

#[utoipa::path(
    get,
    path = "/api/schedules/{id}/reports/failed/count",
    tag = "Schedules",
    operation_id = "countFailedScheduleReports",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Failed report count", body = FailedReportCountResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// Count a schedule's failed backup reports, unbounded by the report list's
/// own pagination window - the true number a "clean up failed backups"
/// confirmation is about to delete.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn count_failed_schedule_reports(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<FailedReportCountResponse>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let count = db::count_failed_backup_reports_for_schedule(&state.pool, id).await?;
    Ok(Json(FailedReportCountResponse { count }))
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
    path = "/api/schedules/{id}/repos",
    tag = "Schedules",
    operation_id = "listScheduleRepos",
    params(("id" = i64, Path, description = "Schedule ID")),
    responses(
        (status = 200, description = "Target repositories", body = Vec<ScheduleRepoResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
/// List the repositories a schedule writes into, in write order.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_schedule_repos(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Vec<ScheduleRepoResponse>>, ApiError> {
    let _schedule = db::get_schedule_by_id(&state.pool, id).await?;
    let repos: Vec<ScheduleRepoResponse> = db::list_schedule_repos(&state.pool, id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(repos))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_command_without_a_timeout_is_accepted() {
        assert!(validate_hook_commands(&[HookCommand::new("echo hi")]).is_ok());
    }

    #[test]
    fn hook_command_timeout_is_accepted_across_the_whole_range() {
        for seconds in [1, 3600, MAX_HOOK_COMMAND_TIMEOUT_SECONDS] {
            assert!(
                validate_hook_commands(&[HookCommand {
                    command: "sleep 1".to_owned(),
                    timeout_seconds: Some(seconds),
                }])
                .is_ok(),
                "{seconds} should be accepted"
            );
        }
    }

    /// Zero would be a command that can never finish in time, and anything
    /// past the bound would let a stuck hook hold a schedule open for longer
    /// than a day - the two things a hook timeout exists to prevent.
    #[test]
    fn hook_command_timeout_outside_the_range_is_rejected() {
        for seconds in [0, MAX_HOOK_COMMAND_TIMEOUT_SECONDS + 1] {
            let err = validate_hook_commands(&[HookCommand {
                command: "sleep 1".to_owned(),
                timeout_seconds: Some(seconds),
            }])
            .unwrap_err();
            assert!(
                matches!(err, ApiError::BadRequest(_)),
                "{seconds} should be a 400, got {err:?}"
            );
        }
    }

    /// The whole list is checked, not just its first entry.
    #[test]
    fn a_bad_timeout_later_in_the_list_is_still_rejected() {
        let commands = vec![
            HookCommand::new("echo one"),
            HookCommand {
                command: "sleep 1".to_owned(),
                timeout_seconds: Some(0),
            },
        ];
        assert!(validate_hook_commands(&commands).is_err());
    }

    /// Builds an `AppState` around `pool` for tests that only need
    /// `release_manual_target_power`'s dependencies (pool, registry,
    /// `ui_broadcast`, `power_sessions`) -- the rest are populated with inert
    /// defaults, matching `scheduler.rs`'s own test `AppState` boilerplate.
    fn test_app_state(pool: sqlx::PgPool) -> AppState {
        let ui_broadcast = UiBroadcast::new();
        AppState {
            pool: pool.clone(),
            encryption_key: shared::crypto::derive_key(b"schedules-power-test-key").unwrap(),
            registry: AgentRegistry::new(),
            ui_broadcast: ui_broadcast.clone(),
            tunnel_manager: TunnelManager::new(
                pool.clone(),
                ui_broadcast,
                "127.0.0.1:0".parse().unwrap(),
            ),
            log_buffer: crate::log_buffer::LogBuffer::default(),
            notification_service: crate::notifications::NotificationService::new(pool),
            completion_bus: CompletionBus::new(),
            repo_op_tracker: RepoOpTracker::default(),
            background_task_tracker: crate::background_tasks::BackgroundTaskTracker::default(),
            repo_lock: crate::RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: crate::new_pending_map(),
            pending_restores: crate::new_pending_map(),
            pending_vm_scans: crate::new_pending_map(),
            pending_vm_builds: crate::new_pending_map(),
            pending_migrations: crate::new_pending_map(),
            pending_deletes: crate::new_pending_map(),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            client_ip_resolver: crate::client_ip::ClientIpResolver::new(),
            task_registry: shared::task_registry::TaskRegistry::default(),
            user_rate_limiter: crate::rate_limit::UserRateLimiter::new(
                60,
                std::time::Duration::from_mins(1),
            ),
            session_idle_timeout_minutes: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
                480,
            )),
            power_sessions: power::PowerSessionTracker::default(),
        }
    }

    async fn insert_power_enabled_agent_and_repo(
        pool: &sqlx::PgPool,
    ) -> (db::AgentRow, db::RepoRow) {
        let agent = db::insert_agent(pool, "manual-power-host", None, "hash", None, None)
            .await
            .unwrap();
        let agent = db::update_agent_power(
            pool,
            agent.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
                start_agent_enabled: false,
                stop_agent_after_backup: false,
                // Nothing listens here -- the SSH attempt is expected to
                // fail, only the run-event trail is under test.
                ssh_host: Some("127.0.0.1"),
                ssh_port: 1,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();

        let passphrase_encrypted = shared::crypto::encrypt_passphrase(
            "test-pass",
            &shared::crypto::derive_key(b"test-secret-key-for-schedules").unwrap(),
        )
        .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "manual-power-repo",
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "127.0.0.1",
                ssh_port: 1,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        let repo = db::update_repo_power(
            pool,
            repo.id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
            },
        )
        .await
        .unwrap();

        (agent, repo)
    }

    /// Regression test for the "manual Run Now doesn't participate in
    /// `PowerSessionTracker`" bug: a sole reservation on both hosts must be
    /// torn down once the manual run releases it, exactly like the
    /// scheduler's own targets are.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn release_manual_target_power_tears_down_sole_participant(pool: sqlx::PgPool) {
        let (agent, repo) = insert_power_enabled_agent_and_repo(&pool).await;
        let state = test_app_state(pool.clone());

        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Agent(agent.id), true, false)
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Repo(repo.id), true, false)
            .await;

        release_manual_target_power(
            &state,
            agent.id,
            repo.id,
            "run-manual-1",
            "manual-power-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-manual-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            event_types,
            vec!["shutdown_sent", "shutdown_sent"],
            "release as the sole participant must attempt shutdown for both the agent and repo \
             hosts"
        );
    }

    /// Regression test for the same bug's other half: releasing while a
    /// sibling schedule's reservation is still held on both hosts must do
    /// nothing, since a concurrent run is still relying on them staying up.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn release_manual_target_power_is_a_noop_with_a_sibling_reservation_held(
        pool: sqlx::PgPool,
    ) {
        let (agent, repo) = insert_power_enabled_agent_and_repo(&pool).await;
        let state = test_app_state(pool.clone());

        // Two reservations on each host, simulating this manual run racing
        // a concurrent scheduled run that targets the same agent/repo.
        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Agent(agent.id), true, false)
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo.id))
            .await;
        state
            .power_sessions
            .record_outcome(power::PowerHostKey::Repo(repo.id), true, false)
            .await;

        release_manual_target_power(
            &state,
            agent.id,
            repo.id,
            "run-manual-2",
            "manual-power-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-manual-2", agent.id, repo.id)
            .await
            .unwrap();
        assert!(
            events.is_empty(),
            "release while a sibling reservation is still held must not tear anything down: \
             {events:?}"
        );
    }

    /// Regression test: if the agent/repo row re-fetch inside
    /// `release_manual_target_power` fails (transient DB error, or the row
    /// was deleted mid-run), the `PowerSessionTracker` reservation must
    /// still be released. Otherwise the session's count never returns to
    /// zero, silently and permanently disabling `shutdown_after_backup`/
    /// `stop_agent_after_backup` for that host until the server restarts.
    /// Uses a lazily-connected pool to a nonexistent database (matching
    /// `scheduler.rs`'s own `run_returns_promptly_when_shutdown_token_is_cancelled`
    /// test) so both row fetches fail deterministically without needing
    /// `DATABASE_URL`.
    #[tokio::test]
    async fn release_manual_target_power_releases_the_reservation_even_when_the_row_fetch_fails() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let state = test_app_state(pool);
        let agent_id = 999_999;
        let repo_id = 888_888;

        state
            .power_sessions
            .reserve(power::PowerHostKey::Agent(agent_id))
            .await;
        state
            .power_sessions
            .reserve(power::PowerHostKey::Repo(repo_id))
            .await;

        release_manual_target_power(&state, agent_id, repo_id, "run-manual-leak", "leak-host")
            .await;

        // If the reservation had leaked (the bug this regresses), this
        // would be the *first* decrement and return Some(..) instead of
        // None -- the tracker would still think a participant is present.
        assert!(
            state
                .power_sessions
                .end(power::PowerHostKey::Agent(agent_id))
                .await
                .is_none(),
            "the agent reservation must already be released by the failed fetch's fallback"
        );
        assert!(
            state
                .power_sessions
                .end(power::PowerHostKey::Repo(repo_id))
                .await
                .is_none(),
            "the repo reservation must already be released by the failed fetch's fallback"
        );
    }

    /// Regression test: when `assemble_config` fails - a transient DB error,
    /// or the agent row deleted mid-run - `push_config_and_trigger_target`
    /// must report the target unreachable and send nothing, rather than
    /// triggering a run against a config the agent never received.
    ///
    /// This arm was previously covered only by chance: no test drove it, and
    /// it registered as covered only when some unrelated test happened to
    /// fail a config assembly first. That made the line flap between covered
    /// and uncovered from run to run and moved the repository's aggregate
    /// coverage by a few hundredths of a percent either way, which is enough
    /// to fail `analyze-coverage-diff.js`'s strict comparison on an unrelated
    /// pull request.
    ///
    /// Uses a lazily-connected pool to a nonexistent database - the same
    /// deterministic, no-`DATABASE_URL` pattern as
    /// `release_manual_target_power_releases_the_reservation_even_when_the_row_fetch_fails`
    /// above - so the fetch inside `assemble_config` fails on every run.
    #[tokio::test]
    async fn push_config_and_trigger_target_reports_unreachable_when_config_assembly_fails() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let state = test_app_state(pool);
        let target = db::ScheduleRunTarget {
            agent_id: 999_999,
            hostname: "unreachable-host".to_owned(),
        };

        let reachable = push_config_and_trigger_target(
            &state,
            &target,
            RepoId(888_888),
            ScheduleType::Backup,
            777_777,
            "run-manual-config-assembly-failure",
        )
        .await;

        assert!(
            !reachable,
            "a target whose config could not be assembled must be reported unreachable"
        );
    }

    fn repo_input(repo_id: i64, required: Option<bool>) -> ScheduleRepoInput {
        ScheduleRepoInput { repo_id, required }
    }

    #[test]
    fn resolve_repo_targets_falls_back_to_the_single_repo_id() {
        let resolved = resolve_repo_targets(None, Some(7)).unwrap();
        assert_eq!(resolved, vec![(7, true)]);

        assert_eq!(resolve_repo_targets(None, None).unwrap(), []);
    }

    #[test]
    fn resolve_repo_targets_keeps_write_order_and_defaults_to_required() {
        let resolved = resolve_repo_targets(
            Some(&[repo_input(3, None), repo_input(9, Some(false))]),
            Some(1),
        )
        .unwrap();
        assert_eq!(resolved, vec![(3, true), (9, false)]);
    }

    #[test]
    fn resolve_repo_targets_rejects_an_empty_list() {
        assert!(matches!(
            resolve_repo_targets(Some(&[]), Some(1)),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn resolve_repo_targets_rejects_the_same_repository_twice() {
        assert!(matches!(
            resolve_repo_targets(Some(&[repo_input(4, None), repo_input(4, None)]), None),
            Err(ApiError::BadRequest(_))
        ));
    }

    /// Without a required target a run would report success having written
    /// nothing, which is worse than no schedule at all.
    #[test]
    fn resolve_repo_targets_rejects_an_all_best_effort_list() {
        assert!(matches!(
            resolve_repo_targets(
                Some(&[repo_input(4, Some(false)), repo_input(5, Some(false))]),
                None
            ),
            Err(ApiError::BadRequest(_))
        ));
    }

    /// The bug this guards: `ScheduleDetailView` sends the whole target list
    /// on every save, so measuring "new" against the primary alone made every
    /// secondary target need permission on every edit.
    #[test]
    fn a_target_the_schedule_already_writes_to_needs_no_fresh_permission() {
        let requested = [(1, true), (2, false)];
        assert_eq!(
            newly_targeted_repos(Some(&requested), Some(1), &[1, 2], Some(1)),
            Vec::<i64>::new(),
        );
    }

    #[test]
    fn only_a_repository_the_schedule_did_not_have_needs_permission() {
        let requested = [(1, true), (2, false), (3, false)];
        assert_eq!(
            newly_targeted_repos(Some(&requested), Some(1), &[1, 2], Some(1)),
            vec![3],
        );
    }

    /// Falls back to the primary when the target list has not been backfilled
    /// for this schedule, so an unknown repository is still checked.
    #[test]
    fn an_empty_target_list_still_recognises_the_primary() {
        let requested = [(1, true), (4, true)];
        assert_eq!(
            newly_targeted_repos(Some(&requested), Some(1), &[], Some(1)),
            vec![4],
        );
    }

    fn plan(
        requested: Option<&[(i64, bool)]>,
        effective_repo_id: Option<i64>,
        existing_targets: &[i64],
    ) -> RepoTargetPlan {
        RepoTargetPlan {
            requested: requested.map(<[(i64, bool)]>::to_vec),
            effective_repo_id,
            existing_targets: existing_targets.to_vec(),
        }
    }

    /// The bug this guards: re-enabling a multi-target schedule through the
    /// legacy bare-`repo_id` contract only pinged the primary, so a schedule
    /// paused while its offsite target was down came back enabled with that
    /// target still unreachable.
    #[test]
    fn an_update_that_keeps_the_target_list_has_to_reach_every_target() {
        assert_eq!(
            resulting_repo_targets(&plan(None, Some(1), &[1, 2]), Some(1)),
            vec![1, 2],
        );
    }

    #[test]
    fn a_request_sending_targets_is_checked_against_exactly_those() {
        let requested = [(2, true), (3, false)];
        assert_eq!(
            resulting_repo_targets(&plan(Some(&requested), Some(2), &[1, 2]), Some(1)),
            vec![2, 3],
        );
    }

    /// Moving the primary collapses the schedule onto that one repository, so
    /// the targets it is about to drop are not worth reaching.
    #[test]
    fn moving_the_primary_only_has_to_reach_the_new_repository() {
        assert_eq!(
            resulting_repo_targets(&plan(None, Some(9), &[1, 2]), Some(1)),
            vec![9],
        );
    }

    #[test]
    fn a_schedule_with_no_targets_falls_back_to_its_primary() {
        assert_eq!(
            resulting_repo_targets(&plan(None, Some(1), &[]), Some(1)),
            vec![1],
        );
        assert_eq!(
            resulting_repo_targets(&plan(None, None, &[1]), Some(1)),
            Vec::<i64>::new(),
        );
    }

    #[test]
    fn a_bare_repo_id_update_is_checked_only_when_it_moves_the_schedule() {
        assert_eq!(
            newly_targeted_repos(None, Some(2), &[1, 2], Some(1)),
            Vec::<i64>::new(),
        );
        assert_eq!(
            newly_targeted_repos(None, Some(9), &[1, 2], Some(1)),
            vec![9]
        );
        assert_eq!(
            newly_targeted_repos(None, None, &[1], Some(1)),
            Vec::<i64>::new(),
        );
    }
}
