// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::auth::AuthUser;
use crate::{AppState, db, error::ApiError};

/// Computes `(part / total) * 100.0` without using `as` casts.
/// Uses integer division scaled by 10000 to maintain precision for display percentages.
fn percentage_of(part: i64, total: i64) -> f64 {
    let scaled = part.saturating_mul(10_000) / total;
    f64::from(i32::try_from(scaled).unwrap_or(10_000)) / 100.0
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ActivityQuery {
    pub limit: Option<i64>,
    pub days: Option<i64>,
    pub category: Option<String>,
    pub repo_id: Option<i64>,
    pub hostname: Option<String>,
    pub schedule_id: Option<i64>,
    pub run_id: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub repo_id: i64,
    pub schedule_id: i64,
    pub hostname: String,
    pub target_name: String,
    pub last_status: Option<String>,
    pub last_backup_at: Option<chrono::DateTime<Utc>>,
    pub is_overdue: bool,
    pub last_error_message: Option<String>,
    pub cron_expression: Option<String>,
    pub schedule_enabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DashboardSummaryResponse {
    pub online_agents: usize,
    pub total_agents: i64,
    pub total_repos: i64,
    pub last_backup_at: Option<chrono::DateTime<Utc>>,
    pub next_backup_at: Option<chrono::DateTime<Utc>>,
    pub last_backup_schedule_id: Option<i64>,
    pub last_backup_repo_id: Option<i64>,
    pub last_backup_archive_name: Option<String>,
    pub next_backup_schedule_id: Option<i64>,
    pub active_schedules: i64,
    pub total_schedules: i64,
    pub total_storage_bytes: i64,
    pub success_30d: i64,
    pub failed_30d: i64,
    pub total_30d: i64,
    pub storage_by_repo: Vec<StorageRepoEntry>,
    pub last_failure_at: Option<chrono::DateTime<Utc>>,
    pub last_warning_at: Option<chrono::DateTime<Utc>>,
    pub last_failure_schedule_id: Option<i64>,
    pub last_warning_schedule_id: Option<i64>,
    pub last_failure_message: Option<String>,
    pub last_warning_message: Option<String>,
    pub last_failure_repo_id: Option<i64>,
    pub last_warning_repo_id: Option<i64>,
    pub last_failure_repo_name: Option<String>,
    pub last_warning_repo_name: Option<String>,
    pub last_failure_schedule_name: Option<String>,
    pub last_warning_schedule_name: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StorageRepoEntry {
    pub name: String,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DashboardSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DashboardStatus {
    Healthy,
    Warning,
    Failed,
    Overdue,
    NeverSucceeded,
    Running,
    Disabled,
    OfflineDueSoon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DashboardFindingKind {
    BackupFailed,
    BackupWarning,
    ScheduleTargetOverdue,
    ScheduleTargetNeverSucceeded,
    HostOfflineDueSoon,
    HostUnassigned,
    RepositoryUnscheduled,
    RepositoryQuotaWarning,
    RepositoryQuotaCritical,
    RepositoryImportFailed,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DashboardDestination {
    Host { hostname: String },
    Schedule { schedule_id: i64 },
    Repository { repo_id: i64 },
    Activity { report_id: i64 },
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardFinding {
    pub id: String,
    pub kind: DashboardFindingKind,
    pub severity: DashboardSeverity,
    pub status: DashboardStatus,
    pub hostname: Option<String>,
    pub schedule_id: Option<i64>,
    pub schedule_name: Option<String>,
    pub repo_id: Option<i64>,
    pub repo_name: Option<String>,
    pub reason: String,
    pub occurred_at: Option<chrono::DateTime<Utc>>,
    pub deadline: Option<chrono::DateTime<Utc>>,
    pub destination: DashboardDestination,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardSummaryCounters {
    pub protected_hosts: i64,
    pub eligible_hosts: i64,
    pub needs_attention: usize,
    pub running_operations: usize,
    pub total_storage_bytes: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardAgentLink {
    pub agent_id: i64,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardProtectionCoverage {
    pub protected_hosts: i64,
    pub eligible_hosts: i64,
    pub protected_agent_links: Vec<DashboardAgentLink>,
    pub unassigned_agents: Vec<DashboardAgentLink>,
    pub never_succeeded_targets: i64,
    pub never_succeeded_agents: Vec<DashboardAgentLink>,
    pub disabled_only_agents: Vec<DashboardAgentLink>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardOperation {
    pub report_id: i64,
    pub status: DashboardStatus,
    pub hostname: String,
    pub schedule_id: i64,
    pub schedule_name: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub started_at: chrono::DateTime<Utc>,
    pub destination: DashboardDestination,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardUpcomingSchedule {
    pub schedule_id: i64,
    pub schedule_name: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub next_run_at: chrono::DateTime<Utc>,
    pub target_count: i64,
    pub offline_target_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DashboardQuotaStatus {
    Unconfigured,
    Healthy,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardRepositoryCapacity {
    pub repo_id: i64,
    pub repo_name: String,
    pub deduplicated_size: i64,
    pub quota_bytes: Option<i64>,
    pub quota_utilization_percent: Option<f64>,
    pub quota_status: DashboardQuotaStatus,
    pub storage_change_bytes: Option<i64>,
    pub threshold_estimate: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DashboardOverviewResponse {
    pub summary: DashboardSummaryCounters,
    pub findings: Vec<DashboardFinding>,
    pub protection: DashboardProtectionCoverage,
    pub running_operations: Vec<DashboardOperation>,
    pub upcoming_schedules: Vec<DashboardUpcomingSchedule>,
    pub repository_capacity: Vec<DashboardRepositoryCapacity>,
}

#[utoipa::path(
    get,
    path = "/api/stats/dashboard-overview",
    tag = "Statistics",
    operation_id = "getDashboardOverview",
    summary = "Get actionable dashboard state",
    responses(
        (status = 200, description = "Dashboard overview", body = DashboardOverviewResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn dashboard_overview(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<DashboardOverviewResponse>, ApiError> {
    let (targets, hosts, upcoming, repositories, dismissed) = tokio::try_join!(
        db::dashboard::targets(&state.pool),
        db::dashboard::eligible_hosts(&state.pool),
        db::dashboard::upcoming_schedules(&state.pool),
        db::dashboard::repositories(&state.pool),
        db::dashboard::dismissed_finding_ids(&state.pool, auth.user_id),
    )?;
    let connected: HashSet<String> = state
        .registry
        .connected_agents()
        .await
        .into_iter()
        .collect();
    let timezone = db::get_schedule_timezone(&state.pool).await?;
    let now = Utc::now();
    let due_soon = now + chrono::Duration::hours(2);

    let mut findings = targets
        .iter()
        .filter(|target| target.schedule_enabled)
        .filter_map(|target| target_finding(target, &connected, now, due_soon, timezone))
        .collect::<Vec<_>>();

    findings.extend(
        hosts
            .iter()
            .filter(|host| host.enabled_assignment_count == Some(0))
            .map(|host| DashboardFinding {
                id: format!("agent:{}:unassigned", host.agent_id),
                kind: DashboardFindingKind::HostUnassigned,
                severity: DashboardSeverity::Warning,
                status: DashboardStatus::Warning,
                hostname: Some(host.hostname.clone()),
                schedule_id: None,
                schedule_name: None,
                repo_id: None,
                repo_name: None,
                reason: "No enabled backup schedule is assigned".to_owned(),
                occurred_at: None,
                deadline: None,
                destination: DashboardDestination::Host {
                    hostname: host.hostname.clone(),
                },
            }),
    );

    repositories.iter().for_each(|repo| {
        if repo.enabled_schedule_count == Some(0) {
            findings.push(repository_finding(
                repo,
                DashboardFindingKind::RepositoryUnscheduled,
                DashboardSeverity::Warning,
                DashboardStatus::Warning,
                "No enabled backup schedule uses this repository",
            ));
        }
        match repository_quota_status(repo) {
            DashboardQuotaStatus::Critical => findings.push(repository_finding(
                repo,
                DashboardFindingKind::RepositoryQuotaCritical,
                DashboardSeverity::Critical,
                DashboardStatus::Failed,
                "Repository storage is at or above its critical quota",
            )),
            DashboardQuotaStatus::Warning => findings.push(repository_finding(
                repo,
                DashboardFindingKind::RepositoryQuotaWarning,
                DashboardSeverity::Warning,
                DashboardStatus::Warning,
                "Repository storage is at or above its warning quota",
            )),
            DashboardQuotaStatus::Unconfigured | DashboardQuotaStatus::Healthy => {}
        }
        if repo.import_error.is_some() {
            findings.push(repository_finding(
                repo,
                DashboardFindingKind::RepositoryImportFailed,
                DashboardSeverity::Critical,
                DashboardStatus::Failed,
                repo.import_error
                    .as_deref()
                    .unwrap_or("Repository import failed"),
            ));
        }
    });
    findings.sort_by_key(|finding| severity_rank(finding.severity));
    findings.retain(|finding| !dismissed.contains(&finding.id));

    let running_operations = targets
        .iter()
        .filter_map(|target| {
            let (Some(report_id), Some(started_at), Some(true)) = (
                target.latest_report_id,
                target.latest_started_at,
                target.latest_started,
            ) else {
                return None;
            };
            Some(DashboardOperation {
                report_id,
                status: DashboardStatus::Running,
                hostname: target.hostname.clone(),
                schedule_id: target.schedule_id,
                schedule_name: target.schedule_name.clone().unwrap_or_default(),
                repo_id: target.repo_id,
                repo_name: target.repo_name.clone(),
                started_at,
                destination: DashboardDestination::Activity { report_id },
            })
        })
        .collect::<Vec<_>>();

    let protected_hosts = hosts
        .iter()
        .filter(|host| host.successful_enabled_assignment_count > Some(0))
        .count();
    let protected_hosts = i64::try_from(protected_hosts).unwrap_or(i64::MAX);
    let eligible_hosts = i64::try_from(hosts.len()).unwrap_or(i64::MAX);
    let protected_agent_links = hosts
        .iter()
        .filter(|host| host.successful_enabled_assignment_count > Some(0))
        .map(agent_link)
        .collect();
    let unassigned_agents = hosts
        .iter()
        .filter(|host| host.enabled_assignment_count == Some(0))
        .map(agent_link)
        .collect();
    let disabled_only_agents = hosts
        .iter()
        .filter(|host| host.enabled_assignment_count == Some(0) && host.disabled_assignment_count > Some(0))
        .map(agent_link)
        .collect();
    let never_succeeded_targets = targets
        .iter()
        .filter(|target| target.schedule_enabled && target.last_success_at.is_none())
        .count();
    let never_succeeded_targets = i64::try_from(never_succeeded_targets).unwrap_or(i64::MAX);
    let never_succeeded_agent_ids = targets
        .iter()
        .filter(|target| target.schedule_enabled && target.last_success_at.is_none())
        .map(|target| target.agent_id)
        .collect::<HashSet<_>>();
    let never_succeeded_agents = hosts
        .iter()
        .filter(|host| never_succeeded_agent_ids.contains(&host.agent_id))
        .map(agent_link)
        .collect();

    let upcoming_schedules = upcoming
        .into_iter()
        .map(|schedule| {
            let offline_target_count = targets
                .iter()
                .filter(|target| target.schedule_id == schedule.schedule_id)
                .filter(|target| !connected.contains(&target.hostname))
                .count();
            DashboardUpcomingSchedule {
                schedule_id: schedule.schedule_id,
                schedule_name: schedule.schedule_name.unwrap_or_default(),
                repo_id: schedule.repo_id,
                repo_name: schedule.repo_name,
                next_run_at: schedule.next_run_at.unwrap(),
                target_count: schedule.target_count.unwrap_or(0),
                offline_target_count,
            }
        })
        .collect();

    let total_storage_bytes = repositories.iter().map(|repo| repo.deduplicated_size).sum();
    let repository_capacity = repositories.iter().map(repository_capacity).collect();

    Ok(Json(DashboardOverviewResponse {
        summary: DashboardSummaryCounters {
            protected_hosts,
            eligible_hosts,
            needs_attention: findings.len(),
            running_operations: running_operations.len(),
            total_storage_bytes,
        },
        findings,
        protection: DashboardProtectionCoverage {
            protected_hosts,
            eligible_hosts,
            protected_agent_links,
            unassigned_agents,
            never_succeeded_targets,
            never_succeeded_agents,
            disabled_only_agents,
        },
        running_operations,
        upcoming_schedules,
        repository_capacity,
    }))
}

fn target_finding(
    target: &db::dashboard::TargetRow,
    connected: &HashSet<String>,
    now: chrono::DateTime<Utc>,
    due_soon: chrono::DateTime<Utc>,
    timezone: chrono_tz::Tz,
) -> Option<DashboardFinding> {
    if target.latest_started == Some(true) {
        return None;
    }

    let overdue_at = target.last_success_at.and_then(|last_success| {
        shared::schedule::calculate_next_run(&target.cron_expression, last_success, timezone)
            .ok()
            .map(|expected| expected + chrono::Duration::minutes(30))
    });

    let (kind, severity, status, reason, occurred_at, deadline, destination) =
        if target.latest_failed == Some(true) {
            (
                DashboardFindingKind::BackupFailed,
                DashboardSeverity::Critical,
                DashboardStatus::Failed,
                target
                    .latest_message
                    .clone()
                    .unwrap_or_else(|| "Latest backup failed".to_owned()),
                target.latest_finished_at,
                None,
                DashboardDestination::Activity {
                    report_id: target.latest_report_id?,
                },
            )
        } else if overdue_at.is_some_and(|deadline| now > deadline) {
            (
                DashboardFindingKind::ScheduleTargetOverdue,
                DashboardSeverity::Critical,
                DashboardStatus::Overdue,
                "No successful backup completed in the expected cron window".to_owned(),
                target.last_success_at,
                overdue_at,
                DashboardDestination::Schedule {
                    schedule_id: target.schedule_id,
                },
            )
        } else if target.latest_warning == Some(true) {
            (
                DashboardFindingKind::BackupWarning,
                DashboardSeverity::Warning,
                DashboardStatus::Warning,
                target
                    .latest_message
                    .clone()
                    .unwrap_or_else(|| "Latest backup completed with warnings".to_owned()),
                target.latest_finished_at,
                None,
                DashboardDestination::Activity {
                    report_id: target.latest_report_id?,
                },
            )
        } else if target.last_success_at.is_none() && target.schedule_last_run_at.is_some() {
            (
                DashboardFindingKind::ScheduleTargetNeverSucceeded,
                DashboardSeverity::Critical,
                DashboardStatus::NeverSucceeded,
                "This enabled schedule target has run but never succeeded".to_owned(),
                target.latest_finished_at,
                target.next_run_at,
                DashboardDestination::Schedule {
                    schedule_id: target.schedule_id,
                },
            )
        } else if target
            .next_run_at
            .is_some_and(|deadline| deadline >= now && deadline <= due_soon)
            && !connected.contains(&target.hostname)
        {
            (
                DashboardFindingKind::HostOfflineDueSoon,
                DashboardSeverity::Warning,
                DashboardStatus::OfflineDueSoon,
                "Agent is offline and this schedule is due within two hours".to_owned(),
                None,
                target.next_run_at,
                DashboardDestination::Host {
                    hostname: target.hostname.clone(),
                },
            )
        } else {
            return None;
        };

    Some(DashboardFinding {
        id: format!("target:{}:{}:{kind:?}", target.schedule_id, target.agent_id),
        kind,
        severity,
        status,
        hostname: Some(target.hostname.clone()),
        schedule_id: Some(target.schedule_id),
        schedule_name: target.schedule_name.clone(),
        repo_id: Some(target.repo_id),
        repo_name: Some(target.repo_name.clone()),
        reason,
        occurred_at,
        deadline,
        destination,
    })
}

fn agent_link(host: &db::dashboard::EligibleAgentRow) -> DashboardAgentLink {
    DashboardAgentLink {
        agent_id: host.agent_id,
        hostname: host.hostname.clone(),
    }
}

fn repository_finding(
    repo: &db::dashboard::RepositoryRow,
    kind: DashboardFindingKind,
    severity: DashboardSeverity,
    status: DashboardStatus,
    reason: &str,
) -> DashboardFinding {
    DashboardFinding {
        id: format!("repository:{}:{kind:?}", repo.repo_id),
        kind,
        severity,
        status,
        hostname: None,
        schedule_id: None,
        schedule_name: None,
        repo_id: Some(repo.repo_id),
        repo_name: Some(repo.repo_name.clone()),
        reason: reason.to_owned(),
        occurred_at: repo.last_synced_at,
        deadline: None,
        destination: DashboardDestination::Repository {
            repo_id: repo.repo_id,
        },
    }
}

const fn severity_rank(severity: DashboardSeverity) -> u8 {
    match severity {
        DashboardSeverity::Critical => 0,
        DashboardSeverity::Warning => 1,
        DashboardSeverity::Info => 2,
    }
}

fn repository_quota_status(repo: &db::dashboard::RepositoryRow) -> DashboardQuotaStatus {
    if repo.quota_enabled != Some(true) {
        return DashboardQuotaStatus::Unconfigured;
    }
    if repo
        .critical_bytes
        .is_some_and(|limit| repo.deduplicated_size >= limit)
    {
        return DashboardQuotaStatus::Critical;
    }
    if repo
        .warn_bytes
        .is_some_and(|limit| repo.deduplicated_size >= limit)
    {
        return DashboardQuotaStatus::Warning;
    }
    DashboardQuotaStatus::Healthy
}

fn repository_capacity(repo: &db::dashboard::RepositoryRow) -> DashboardRepositoryCapacity {
    let quota_bytes = repo.critical_bytes.or(repo.warn_bytes);
    let quota_utilization_percent = quota_bytes
        .filter(|limit| *limit > 0)
        .map(|limit| percentage_of(repo.deduplicated_size, limit));
    DashboardRepositoryCapacity {
        repo_id: repo.repo_id,
        repo_name: repo.repo_name.clone(),
        deduplicated_size: repo.deduplicated_size,
        quota_bytes,
        quota_utilization_percent,
        quota_status: repository_quota_status(repo),
        storage_change_bytes: None,
        threshold_estimate: None,
    }
}

#[utoipa::path(
    get,
    path = "/api/stats/summary",
    tag = "Statistics",
    operation_id = "getDashboardSummary",
    summary = "Get dashboard summary statistics",
    responses(
        (status = 200, description = "Dashboard summary", body = DashboardSummaryResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn summary(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<DashboardSummaryResponse>, ApiError> {
    let row = db::get_dashboard_summary(&state.pool).await?;
    let breakdown = db::get_storage_breakdown(&state.pool).await?;
    let online_agents = state.registry.connected_agents().await.len();

    let total_storage = row.total_storage_bytes;
    let storage_by_repo = breakdown
        .into_iter()
        .map(|b| {
            let percentage = if total_storage > 0 {
                percentage_of(b.deduplicated_size, total_storage)
            } else {
                0.0
            };
            StorageRepoEntry {
                name: b.name,
                compressed_size: b.compressed_size,
                deduplicated_size: b.deduplicated_size,
                percentage,
            }
        })
        .collect();

    Ok(Json(DashboardSummaryResponse {
        online_agents,
        total_agents: row.total_agents,
        total_repos: row.total_repos,
        last_backup_at: row.last_backup_at,
        next_backup_at: row.next_backup_at,
        last_backup_schedule_id: row.last_backup_schedule_id,
        last_backup_repo_id: row.last_backup_repo_id,
        last_backup_archive_name: row.last_backup_archive_name,
        next_backup_schedule_id: row.next_backup_schedule_id,
        active_schedules: row.active_schedules,
        total_schedules: row.total_schedules,
        total_storage_bytes: row.total_storage_bytes,
        success_30d: row.success_30d,
        failed_30d: row.failed_30d,
        total_30d: row.total_30d,
        storage_by_repo,
        last_failure_at: row.last_failure_at,
        last_warning_at: row.last_warning_at,
        last_failure_schedule_id: row.last_failure_schedule_id,
        last_warning_schedule_id: row.last_warning_schedule_id,
        last_failure_message: row.last_failure_message,
        last_warning_message: row.last_warning_message,
        last_failure_repo_id: row.last_failure_repo_id,
        last_warning_repo_id: row.last_warning_repo_id,
        last_failure_repo_name: row.last_failure_repo_name,
        last_warning_repo_name: row.last_warning_repo_name,
        last_failure_schedule_name: row.last_failure_schedule_name,
        last_warning_schedule_name: row.last_warning_schedule_name,
    }))
}

#[utoipa::path(
    get,
    path = "/api/stats/storage",
    tag = "Statistics",
    operation_id = "getStorageStats",
    summary = "Get per-repo storage statistics",
    responses(
        (status = 200, description = "Storage stats", body = Vec<crate::db::StorageStatRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn storage(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<db::StorageStatRow>>, ApiError> {
    let rows = db::get_storage_stats(&state.pool).await?;
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/stats/schedule-counts",
    tag = "Statistics",
    operation_id = "getScheduleCountsByAgent",
    summary = "Get schedule counts per agent",
    responses(
        (
            status = 200,
            description = "Schedule counts by agent",
            body = Vec<crate::db::ScheduleCountByAgent>,
        ),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn schedule_counts(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<db::ScheduleCountByAgent>>, ApiError> {
    let counts = db::get_schedule_counts_by_agent(&state.pool).await?;
    Ok(Json(counts))
}

#[utoipa::path(
    get,
    path = "/api/stats/storage-breakdown",
    tag = "Statistics",
    operation_id = "getStorageBreakdown",
    summary = "Get per-repo storage breakdown (sourced from borg info)",
    responses(
        (status = 200, description = "Storage breakdown", body = Vec<StorageRepoEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn storage_breakdown(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<StorageRepoEntry>>, ApiError> {
    let breakdown = db::get_storage_breakdown(&state.pool).await?;

    let total: i64 = breakdown.iter().map(|b| b.deduplicated_size).sum();
    let entries = breakdown
        .into_iter()
        .map(|b| {
            let percentage = if total > 0 {
                percentage_of(b.deduplicated_size, total)
            } else {
                0.0
            };
            StorageRepoEntry {
                name: b.name,
                compressed_size: b.compressed_size,
                deduplicated_size: b.deduplicated_size,
                percentage,
            }
        })
        .collect();

    Ok(Json(entries))
}

#[utoipa::path(
    get,
    path = "/api/stats/activity",
    tag = "Statistics",
    operation_id = "getActivity",
    summary = "Get backup activity feed",
    params(
        ("limit" = Option<i64>, Query, description = "Max entries to return"),
        ("days" = Option<i64>, Query, description = "Return entries from last N days"),
        ("repo_id" = Option<i64>, Query, description = "Filter by repository ID"),
        ("hostname" = Option<String>, Query, description = "Filter by agent hostname"),
        ("schedule_id" = Option<i64>, Query, description = "Filter by schedule ID"),
        ("run_id" = Option<String>, Query, description = "Filter by run ID"),
    ),
    responses(
        (status = 200, description = "Activity feed", body = Vec<crate::db::ActivityRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn activity(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<db::ActivityRow>>, ApiError> {
    let rows = if let Some(days) = query.days {
        db::get_activity_feed_days(
            &state.pool,
            days,
            query.repo_id,
            query.hostname.as_deref(),
            query.schedule_id,
            query.run_id.as_deref(),
        )
        .await?
    } else {
        let limit = query.limit.unwrap_or(20);
        db::get_activity_feed(
            &state.pool,
            limit,
            query.repo_id,
            query.hostname.as_deref(),
            query.schedule_id,
            query.run_id.as_deref(),
        )
        .await?
    };
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/stats/system-events",
    tag = "Statistics",
    operation_id = "getSystemEvents",
    summary = "Get system event log",
    params(("limit" = Option<i64>, Query, description = "Max entries to return")),
    responses(
        (status = 200, description = "System events", body = Vec<crate::db::SystemEventRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn system_events(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Vec<db::SystemEventRow>>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    let rows = db::get_system_events(&state.pool, limit).await?;
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/stats/health",
    tag = "Statistics",
    operation_id = "getHealthSummary",
    summary = "Get backup health summary for all schedules",
    responses(
        (status = 200, description = "Health summary", body = Vec<HealthResponse>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn health(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<HealthResponse>>, ApiError> {
    let rows = db::get_health_summary(&state.pool).await?;
    let tz = db::get_schedule_timezone(&state.pool).await?;
    let response = rows
        .into_iter()
        .map(|row| {
            let overdue = is_overdue(row.last_backup_at, row.cron_expression.as_deref(), tz);
            HealthResponse {
                repo_id: row.repo_id,
                schedule_id: row.schedule_id,
                hostname: row.hostname,
                target_name: row.target_name,
                last_status: row.last_status,
                last_backup_at: row.last_backup_at,
                is_overdue: overdue,
                last_error_message: row.last_error_message,
                cron_expression: row.cron_expression,
                schedule_enabled: row.schedule_enabled,
            }
        })
        .collect();
    Ok(Json(response))
}

fn is_overdue(
    last_backup_at: Option<chrono::DateTime<Utc>>,
    cron_expression: Option<&str>,
    tz: chrono_tz::Tz,
) -> bool {
    let Some(last) = last_backup_at else {
        return false;
    };
    let Some(cron_expr) = cron_expression else {
        return false;
    };
    let grace = chrono::Duration::minutes(30);
    let Ok(expected_next) = shared::schedule::calculate_next_run(cron_expr, last, tz) else {
        return false;
    };
    Utc::now() > expected_next + grace
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TrendsQuery {
    pub repo_id: Option<i64>,
    pub days: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TrendEntry {
    pub date: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub dedup_ratio: f64,
    pub file_count: i64,
    pub duration_seconds: i64,
}

#[utoipa::path(
    get,
    path = "/api/stats/trends",
    tag = "Statistics",
    operation_id = "getBackupTrends",
    summary = "Get backup size trends over time",
    params(
        ("repo_id" = Option<i64>, Query, description = "Filter by repository ID"),
        ("days" = Option<i64>, Query, description = "Number of days (30, 90, 365)"),
    ),
    responses(
        (status = 200, description = "Backup trends", body = Vec<TrendEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn trends(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<Vec<TrendEntry>>, ApiError> {
    let days = query.days.unwrap_or(30);
    let rows = db::get_backup_trends(&state.pool, query.repo_id, days).await?;
    let entries = rows
        .into_iter()
        .map(|row| {
            let dedup_ratio = if row.original_size > 0 {
                let scaled = row.deduplicated_size.saturating_mul(10_000) / row.original_size;
                f64::from(i32::try_from(scaled).unwrap_or(10_000)) / 100.0
            } else {
                0.0
            };
            TrendEntry {
                date: row.date.to_string(),
                original_size: row.original_size,
                compressed_size: row.compressed_size,
                deduplicated_size: row.deduplicated_size,
                dedup_ratio,
                file_count: row.file_count,
                duration_seconds: row.duration_seconds,
            }
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StorageTrendEntry {
    pub date: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StorageTrendByRepoEntry {
    pub date: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
}

#[utoipa::path(
    get,
    path = "/api/stats/storage-trends",
    tag = "Statistics",
    operation_id = "getStorageTrends",
    summary = "Get total repository disk usage over time",
    params(
        ("repo_id" = Option<i64>, Query, description = "Filter by repository ID"),
        ("days" = Option<i64>, Query, description = "Number of days (14, 30, 90, 365)"),
    ),
    responses(
        (status = 200, description = "Storage trends", body = Vec<StorageTrendEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn storage_trends(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<Vec<StorageTrendEntry>>, ApiError> {
    let days = query.days.unwrap_or(30);
    let rows = db::get_storage_trends(&state.pool, query.repo_id, days).await?;
    let entries = rows
        .into_iter()
        .map(|row| StorageTrendEntry {
            date: row.date.to_string(),
            original_size: row.original_size,
            compressed_size: row.compressed_size,
            deduplicated_size: row.deduplicated_size,
        })
        .collect();
    Ok(Json(entries))
}

#[utoipa::path(
    get,
    path = "/api/stats/storage-trends/by-repo",
    tag = "Statistics",
    operation_id = "getStorageTrendsByRepo",
    summary = "Get per-repo storage usage over time for stacked view",
    params(
        ("days" = Option<i64>, Query, description = "Number of days (14, 30, 90, 365)"),
    ),
    responses(
        (status = 200, description = "Per-repo storage trends",
            body = Vec<StorageTrendByRepoEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn storage_trends_by_repo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<Vec<StorageTrendByRepoEntry>>, ApiError> {
    let days = query.days.unwrap_or(30);
    let rows = db::get_storage_trends_by_repo(&state.pool, days).await?;
    let entries = rows
        .into_iter()
        .map(|row| StorageTrendByRepoEntry {
            date: row.date.to_string(),
            repo_id: row.repo_id,
            repo_name: row.repo_name,
            original_size: row.original_size,
            compressed_size: row.compressed_size,
            deduplicated_size: row.deduplicated_size,
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CalendarQuery {
    pub month: String,
    pub repo_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CalendarEventType {
    Backup,
    Check,
    Verify,
}

impl TryFrom<&str> for CalendarEventType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "backup" => Ok(Self::Backup),
            "check" => Ok(Self::Check),
            "verify" => Ok(Self::Verify),
            other => Err(format!("unknown calendar event type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CalendarEventStatus {
    Success,
    Failed,
    Scheduled,
}

impl TryFrom<&str> for CalendarEventStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "scheduled" => Ok(Self::Scheduled),
            other => Err(format!("unknown calendar event status: {other}")),
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CalendarEvent {
    #[serde(rename = "type")]
    pub event_type: CalendarEventType,
    pub status: CalendarEventStatus,
    pub repo_name: String,
    pub hostname: String,
    pub time: String,
    pub report_id: Option<i64>,
    pub repo_id: Option<i64>,
    pub schedule_id: Option<i64>,
    pub archive_name: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CalendarDay {
    pub date: String,
    pub events: Vec<CalendarEvent>,
}

#[utoipa::path(
    get,
    path = "/api/stats/calendar",
    tag = "Statistics",
    operation_id = "getCalendar",
    summary = "Get calendar view of backups for a month",
    params(
        ("month" = String, Query, description = "Month in YYYY-MM format"),
        ("repo_id" = Option<i64>, Query, description = "Filter by repository ID"),
    ),
    responses(
        (status = 200, description = "Calendar events", body = Vec<CalendarDay>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn calendar(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<CalendarQuery>,
) -> Result<Json<Vec<CalendarDay>>, ApiError> {
    let parts: Vec<&str> = query.month.split('-').collect();
    if parts.len() != 2 {
        return Err(ApiError::BadRequest(
            "month must be in YYYY-MM format".to_string(),
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| ApiError::BadRequest("invalid year".to_string()))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| ApiError::BadRequest("invalid month".to_string()))?;

    let tz = db::get_schedule_timezone(&state.pool).await?;
    let rows = db::get_calendar_events(&state.pool, year, month, query.repo_id, tz).await?;

    let schedules = db::get_enabled_schedules_for_calendar(&state.pool).await?;
    let now = Utc::now();

    let month_start = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;
    let month_end = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;

    let mut day_map: std::collections::BTreeMap<String, Vec<CalendarEvent>> =
        std::collections::BTreeMap::new();

    for row in rows {
        let Ok(event_type) = CalendarEventType::try_from(row.event_type.as_str()) else {
            continue;
        };
        let Ok(status) = CalendarEventStatus::try_from(row.status.as_str()) else {
            continue;
        };
        day_map
            .entry(row.date.to_string())
            .or_default()
            .push(CalendarEvent {
                event_type,
                status,
                repo_name: row.repo_name,
                hostname: row.hostname,
                time: row.time,
                report_id: row.report_id,
                repo_id: row.repo_id,
                schedule_id: None,
                archive_name: row.archive_name,
                error_message: row.error_message,
            });
    }

    let repos = db::list_all_repos(&state.pool).await?;

    for schedule in &schedules {
        if query
            .repo_id
            .is_some_and(|rid| schedule.repo_id != Some(rid))
        {
            continue;
        }
        let repo_name = schedule
            .repo_id
            .and_then(|rid| repos.iter().find(|r| r.id == rid))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        let hostname = db::get_schedule_target_hostnames(&state.pool, schedule.id)
            .await
            .ok()
            .and_then(|h| h.into_iter().next())
            .unwrap_or_default();

        let mut cursor = now;
        for _ in 0..62 {
            let Ok(next) =
                shared::schedule::calculate_next_run(&schedule.cron_expression, cursor, tz)
            else {
                break;
            };
            let next_date = next.date_naive();
            if next_date >= month_end {
                break;
            }
            if next_date >= month_start && next > now {
                let time_str = next.format("%H:%M").to_string();
                day_map
                    .entry(next_date.to_string())
                    .or_default()
                    .push(CalendarEvent {
                        event_type: CalendarEventType::Backup,
                        status: CalendarEventStatus::Scheduled,
                        repo_name: repo_name.clone(),
                        hostname: hostname.clone(),
                        time: time_str,
                        report_id: None,
                        repo_id: schedule.repo_id,
                        schedule_id: Some(schedule.id),
                        archive_name: None,
                        error_message: None,
                    });
            }
            cursor = next;
        }
    }

    let result = day_map
        .into_iter()
        .map(|(date, events)| CalendarDay { date, events })
        .collect();

    Ok(Json(result))
}

pub async fn dismiss_finding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(finding_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    db::dashboard::dismiss_finding(&state.pool, auth.user_id, &finding_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn undismiss_finding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(finding_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    db::dashboard::undismiss_finding(&state.pool, auth.user_id, &finding_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
