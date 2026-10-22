// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! System events and acknowledging events and backup reports.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::{AcknowledgedFilter, BackupStatus, SystemEventSeverity, SystemEventType};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `system_events` table.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SystemEventRow {
    /// Unique identifier.
    pub id: i64,
    /// When the event occurred.
    pub created_at: DateTime<Utc>,
    /// Event type.
    pub event_type: SystemEventType,
    /// How the event reads - drives the badge and whether it can be
    /// acknowledged.
    pub severity: SystemEventSeverity,
    /// Whether this event reports a problem a human can acknowledge.
    pub acknowledgeable: bool,
    /// Whether a human has acknowledged this event.
    pub acknowledged: bool,
    /// Hostname the event relates to, if any.
    pub hostname: Option<String>,
    /// Human-readable event message.
    pub message: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_system_event(
    pool: &PgPool,
    event_type: SystemEventType,
    hostname: Option<&str>,
    message: &str,
) -> Result<(), ApiError> {
    let event_type_str = event_type.to_string();
    sqlx::query!(
        "INSERT INTO system_events (event_type, hostname, message) VALUES ($1, $2, $3)",
        event_type_str,
        hostname,
        message,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_system_events(
    pool: &PgPool,
    limit: i64,
    acknowledged_filter: AcknowledgedFilter,
) -> Result<Vec<SystemEventRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, created_at, event_type, hostname, message, acknowledged FROM system_events \
         WHERE ($1::bool IS NULL OR acknowledged = $1) ORDER BY created_at DESC LIMIT $2",
        acknowledged_filter.as_sql_predicate(),
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .filter_map(|r| match r.event_type.parse::<SystemEventType>() {
            Ok(event_type) => Some(SystemEventRow {
                id: r.id,
                created_at: r.created_at,
                event_type,
                severity: event_type.severity(),
                acknowledgeable: event_type.is_acknowledgeable(),
                acknowledged: r.acknowledged,
                hostname: r.hostname,
                message: r.message,
            }),
            Err(e) => {
                tracing::warn!(
                    row_id = r.id,
                    event_type = %r.event_type,
                    error = %e,
                    "skipping system event row with unrecognized event_type"
                );
                None
            }
        })
        .collect())
}

/// Looks up a system event's type so the caller can reject acknowledging one
/// that reports nothing to review.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no event has this id,
/// [`ApiError::Unprocessable`] if the event's type is not acknowledgeable, and
/// [`ApiError::Database`] if the query fails.
pub async fn get_acknowledgeable_system_event_type(
    pool: &PgPool,
    id: i64,
) -> Result<SystemEventType, ApiError> {
    let raw = sqlx::query_scalar!("SELECT event_type FROM system_events WHERE id = $1", id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("system event id '{id}' not found")))?;

    let event_type = raw.parse::<SystemEventType>().map_err(|e| {
        ApiError::Unprocessable(format!("system event {id} has an unknown type: {e}"))
    })?;
    if !event_type.is_acknowledgeable() {
        return Err(ApiError::Unprocessable(format!(
            "system event {id} cannot be acknowledged: only warning or failed events can be \
             reviewed"
        )));
    }
    Ok(event_type)
}

/// Sets or clears the acknowledged flag on a system event. Like a backup
/// report's flag, this is shared across all users rather than scoped to
/// whoever clicked it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no event has this id, or
/// [`ApiError::Database`] if the query fails.
pub async fn set_system_event_acknowledged(
    pool: &PgPool,
    id: i64,
    acknowledged: bool,
) -> Result<(), ApiError> {
    let rows_affected = sqlx::query!(
        "UPDATE system_events SET acknowledged = $1 WHERE id = $2",
        acknowledged,
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("system event {id} not found")));
    }
    Ok(())
}

/// The backup report statuses that carry something to acknowledge, as the
/// bind parameters the bulk acknowledgment queries pass to `status IN (..)`.
///
/// Shared so the acknowledge, the "which repositories are affected" lookup and
/// the count can never disagree about what counts as outstanding.
fn acknowledgeable_report_statuses() -> [String; 2] {
    [
        BackupStatus::Warning.to_string(),
        BackupStatus::Failed.to_string(),
    ]
}

#[cfg(test)]
mod acknowledgeable_status_tests {
    use super::{BackupStatus, acknowledgeable_report_statuses};

    /// The bulk queries bind these strings while the per-report check calls
    /// [`BackupStatus::is_acknowledgeable`]; this pins the two to the same set.
    #[test]
    fn bound_statuses_are_exactly_the_acknowledgeable_ones() {
        let bound = acknowledgeable_report_statuses();
        for status in [
            BackupStatus::Success,
            BackupStatus::Warning,
            BackupStatus::Failed,
        ] {
            assert_eq!(
                bound.contains(&status.to_string()),
                status.is_acknowledgeable(),
                "{status} disagrees between the bulk filter and is_acknowledgeable"
            );
        }
    }
}

/// The system event types that report a problem, as the bind parameter the
/// bulk acknowledgment queries pass to `event_type = ANY(..)`.
///
/// The system-event counterpart to [`acknowledgeable_report_statuses`], shared
/// for the same reason: a change to which [`SystemEventType`] variants are
/// acknowledgeable must reach every query at once rather than depending on
/// whoever makes it remembering there is more than one call site.
fn acknowledgeable_system_event_types() -> Vec<String> {
    SystemEventType::ALL
        .iter()
        .filter(|event_type| event_type.is_acknowledgeable())
        .map(ToString::to_string)
        .collect()
}

/// The slice of the activity feed a bulk acknowledge acts on, beyond the
/// repositories the caller is allowed to touch.
///
/// [`Default`] is "everything outstanding", which is what the Activity Log's
/// own Acknowledge all asks for. The dashboard's Backup stats panel counts one
/// repository over one window and clears exactly what it counted, so it fills
/// these in from the same selectors its feed query uses -- `days` is applied
/// with the same `NOW() - make_interval(...)` bound as
/// [`get_activity_feed_days`], so the two can never disagree about which runs
/// fall inside the window.
///
/// [`get_activity_feed_days`]: super::get_activity_feed_days
#[derive(Debug, Clone, Copy, Default)]
pub struct BulkAcknowledgeFilters {
    /// Only reports for this repository, or every repository when `None`.
    pub repo_id: Option<i64>,
    /// Only reports started within the last N days, or all history when
    /// `None`.
    pub days: Option<i32>,
}

/// Acknowledges every unacknowledged warning/failed backup report belonging to
/// one of `repo_ids` and matching `filters`, and reports how many rows that
/// touched.
///
/// The caller narrows `repo_ids` to the repositories the user may act on, so
/// a bulk acknowledge can never reach further than the same user's per-report
/// acknowledge would. `filters` narrows it further, to the slice of the feed
/// the caller is actually looking at.
///
/// Hidden and imported-placeholder agents are excluded, with the same
/// predicate [`get_activity_feed`] and [`get_activity_feed_days`] apply. Their
/// reports never reach any feed or dashboard tile, so acknowledging one here
/// would silently retire a run nobody was shown - and would break the promise
/// the button rests on, that a bulk acknowledge clears no more than what was
/// on screen.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
///
/// [`get_activity_feed`]: super::get_activity_feed
/// [`get_activity_feed_days`]: super::get_activity_feed_days
pub async fn acknowledge_backup_reports_in_repos<'e, E>(
    executor: E,
    repo_ids: &[i64],
    filters: BulkAcknowledgeFilters,
) -> Result<u64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let [warning, failed] = acknowledgeable_report_statuses();
    Ok(sqlx::query!(
        "UPDATE backup_reports SET acknowledged = true WHERE id IN (SELECT br.id FROM \
         backup_reports br JOIN agents a ON a.id = br.agent_id WHERE br.acknowledged = false AND \
         br.repo_id = ANY($1) AND br.status IN ($2, $3) AND ($4::bigint IS NULL OR br.repo_id = \
         $4) AND ($5::int IS NULL OR br.started_at > NOW() - make_interval(days => $5)) AND \
         a.is_hidden = false AND COALESCE(a.display_name, '') NOT ILIKE '%(imported)%')",
        repo_ids,
        warning,
        failed,
        filters.repo_id,
        filters.days,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?
    .rows_affected())
}

/// The repositories that still hold at least one unacknowledged warning or
/// failed backup report from an agent a feed would actually show, so the
/// caller can permission-check just those. Carries the same agent-visibility
/// predicate as the count and the write it feeds, so a repository whose only
/// outstanding reports belong to a hidden or imported agent is not offered as
/// a candidate for something that would then acknowledge nothing.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn repos_with_unacknowledged_reports(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let [warning, failed] = acknowledgeable_report_statuses();
    sqlx::query_scalar!(
        "SELECT DISTINCT br.repo_id FROM backup_reports br JOIN agents a ON a.id = br.agent_id \
         WHERE br.acknowledged = false AND br.status IN ($1, $2) AND a.is_hidden = false AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' ORDER BY br.repo_id",
        warning,
        failed,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Counts the unacknowledged warning/failed backup reports belonging to one of
/// `repo_ids` and matching `filters`, so the UI can tell whether a bulk
/// acknowledge would do anything without first loading the feed.
///
/// Deliberately mirrors [`acknowledge_backup_reports_in_repos`]'s filter -
/// the agent-visibility predicate included - so "the button is shown" and
/// "the button would acknowledge something" can never disagree.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_unacknowledged_reports_in_repos(
    pool: &PgPool,
    repo_ids: &[i64],
    filters: BulkAcknowledgeFilters,
) -> Result<i64, ApiError> {
    let [warning, failed] = acknowledgeable_report_statuses();
    Ok(sqlx::query_scalar!(
        "SELECT COUNT(*) FROM backup_reports br JOIN agents a ON a.id = br.agent_id WHERE \
         br.acknowledged = false AND br.repo_id = ANY($1) AND br.status IN ($2, $3) AND \
         ($4::bigint IS NULL OR br.repo_id = $4) AND ($5::int IS NULL OR br.started_at > NOW() - \
         make_interval(days => $5)) AND a.is_hidden = false AND COALESCE(a.display_name, '') NOT \
         ILIKE '%(imported)%'",
        repo_ids,
        warning,
        failed,
        filters.repo_id,
        filters.days,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?
    .unwrap_or(0))
}

/// Counts the unacknowledged system events whose type reports a problem, the
/// counterpart to [`count_unacknowledged_reports_in_repos`] for the events an
/// admin can acknowledge.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_unacknowledged_system_events(pool: &PgPool) -> Result<i64, ApiError> {
    let acknowledgeable = acknowledgeable_system_event_types();
    Ok(sqlx::query_scalar!(
        "SELECT COUNT(*) FROM system_events WHERE acknowledged = false AND event_type = ANY($1)",
        &acknowledgeable,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?
    .unwrap_or(0))
}

/// Acknowledges every unacknowledged system event whose type reports a
/// problem, and reports how many rows that touched.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_all_system_events<'e, E>(executor: E) -> Result<u64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let acknowledgeable = acknowledgeable_system_event_types();
    Ok(sqlx::query!(
        "UPDATE system_events SET acknowledged = true WHERE acknowledged = false AND event_type = \
         ANY($1)",
        &acknowledgeable,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?
    .rows_affected())
}

/// Acknowledges everything the caller may retire in one step - the warning and
/// failed backup reports in `repo_ids` matching `filters`, and, when
/// `include_system_events`, the problem-reporting system events - and reports
/// the two counts.
///
/// One transaction, so a request the client sees fail leaves nothing
/// half-acknowledged: without it a failure on the second write would return
/// 500 while the first had already committed.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_all_outstanding(
    pool: &PgPool,
    repo_ids: &[i64],
    include_system_events: bool,
    filters: BulkAcknowledgeFilters,
) -> Result<(u64, u64), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let backup_reports = acknowledge_backup_reports_in_repos(&mut *tx, repo_ids, filters).await?;
    let system_events = if include_system_events {
        acknowledge_all_system_events(&mut *tx).await?
    } else {
        0
    };
    tx.commit().await.map_err(ApiError::Database)?;
    Ok((backup_reports, system_events))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_system_events_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM system_events WHERE created_at < $1", before)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Prunes old notification delivery-attempt history by age. The table is
/// kept "for debugging and retry" (see `0002_notifications.sql`), not as a
/// permanent audit log, so it grows without bound otherwise.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_notification_deliveries_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM notification_deliveries WHERE attempted_at < $1",
        before
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}
