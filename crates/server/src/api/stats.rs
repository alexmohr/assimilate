// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Query, State},
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
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub repo_id: i64,
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
    pub online_clients: usize,
    pub total_clients: i64,
    pub total_repos: i64,
    pub last_backup_at: Option<chrono::DateTime<Utc>>,
    pub next_backup_at: Option<chrono::DateTime<Utc>>,
    pub last_backup_schedule_id: Option<i64>,
    pub next_backup_schedule_id: Option<i64>,
    pub active_schedules: i64,
    pub total_schedules: i64,
    pub total_storage_bytes: i64,
    pub success_30d: i64,
    pub failed_30d: i64,
    pub total_30d: i64,
    pub storage_by_repo: Vec<StorageRepoEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StorageRepoEntry {
    pub name: String,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub percentage: f64,
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
    let online_clients = state.registry.connected_agents().await.len();

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
        online_clients,
        total_clients: row.total_clients,
        total_repos: row.total_repos,
        last_backup_at: row.last_backup_at,
        next_backup_at: row.next_backup_at,
        last_backup_schedule_id: row.last_backup_schedule_id,
        next_backup_schedule_id: row.next_backup_schedule_id,
        active_schedules: row.active_schedules,
        total_schedules: row.total_schedules,
        total_storage_bytes: row.total_storage_bytes,
        success_30d: row.success_30d,
        failed_30d: row.failed_30d,
        total_30d: row.total_30d,
        storage_by_repo,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StorageGroupBy {
    #[default]
    Repo,
    Host,
    Server,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StorageBreakdownQuery {
    pub group_by: Option<StorageGroupBy>,
}

#[utoipa::path(
    get,
    path = "/api/stats/storage-breakdown",
    tag = "Statistics",
    operation_id = "getStorageBreakdown",
    summary = "Get storage breakdown grouped by repo, host, or server",
    params(("group_by" = Option<String>, Query,
        description = "Group by: repo (default), host, or server")),
    responses(
        (status = 200, description = "Storage breakdown", body = Vec<StorageRepoEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn storage_breakdown(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<StorageBreakdownQuery>,
) -> Result<Json<Vec<StorageRepoEntry>>, ApiError> {
    let group_by = query.group_by.unwrap_or_default();
    let breakdown = match group_by {
        StorageGroupBy::Host => db::get_storage_breakdown_by_host(&state.pool).await?,
        StorageGroupBy::Server => db::get_storage_breakdown_by_server(&state.pool).await?,
        StorageGroupBy::Repo => db::get_storage_breakdown(&state.pool).await?,
    };

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
        db::get_activity_feed_days(&state.pool, days).await?
    } else {
        let limit = query.limit.unwrap_or(20);
        db::get_activity_feed(&state.pool, limit).await?
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
        return true;
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
    pub time: String,
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

    let rows = db::get_calendar_events(&state.pool, year, month, query.repo_id).await?;

    let tz = db::get_schedule_timezone(&state.pool).await?;
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
                time: row.time,
            });
    }

    let repos = db::list_all_repos(&state.pool).await?;

    for schedule in &schedules {
        if query.repo_id.is_some_and(|rid| schedule.repo_id != rid) {
            continue;
        }
        let repo_name = repos
            .iter()
            .find(|r| r.id == schedule.repo_id)
            .map(|r| r.name.clone())
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
                        time: time_str,
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
