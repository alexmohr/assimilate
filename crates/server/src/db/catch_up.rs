// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use shared::types::ScheduleType;
use sqlx::PgPool;

use crate::error::ApiError;

/// One target of one schedule that has a run waiting to be caught up, together
/// with everything the reconnect handler needs to decide whether it may still
/// run - see `crate::catch_up`.
#[derive(Debug, Clone)]
pub struct CatchUpCandidate {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// Schedule display name, for the log line and system event.
    pub schedule_name: String,
    /// Agent that missed the occurrence.
    pub agent_id: i64,
    /// That agent's hostname.
    pub hostname: String,
    /// Schedule type, as stored - parsed at the call site.
    pub schedule_type: ScheduleType,
    /// The schedule's cron expression, needed to advance `next_run_at` past
    /// this catch-up run the same way a scheduled tick does.
    pub cron_expression: String,
    /// The occurrence that was missed.
    pub pending_for: DateTime<Utc>,
    /// When the schedule next runs on its own, which the lead-time floor is
    /// measured against. Never `None` in practice for an enabled schedule, but
    /// the column is nullable, and a schedule with no computed next run has no
    /// regular run for a catch-up to collide with.
    pub next_run_at: Option<DateTime<Utc>>,
    /// The schedule's configured floor: how much time must be left before
    /// `next_run_at` for the catch-up to still be worth running.
    pub min_lead_minutes: i32,
    /// How long this agent is waited for before the run is abandoned; zero
    /// waits indefinitely.
    pub give_up_minutes: i32,
}

/// The repositories a catch-up run for `schedule_id` writes to, in write
/// order, skipping any whose repository is disabled.
///
/// A catch-up has to cover the same targets the tick it stands in for would
/// have: resolving only the schedule's denormalised primary would quietly let
/// every secondary copy fall further behind on every miss.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_enabled_catch_up_repos(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT sr.repo_id FROM schedule_repos sr JOIN repos r ON r.id = sr.repo_id WHERE \
         sr.schedule_id = $1 AND r.enabled = true ORDER BY sr.execution_order, sr.repo_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records that `agent_id` missed `due_at` for `schedule_id`, so the run can be
/// caught up when that host reconnects.
///
/// Deliberately an overwrite rather than an accumulation: a host that misses
/// thirty-five occurrences ends up with the most recent one pending, and
/// therefore exactly one catch-up run - the "missed runs never stack" rule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    due_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = $3 WHERE schedule_id = $1 AND \
         agent_id = $2",
        schedule_id,
        agent_id,
        due_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Every run this agent has pending, restricted to what could actually run one
/// right now: the agent still marked as not always online, schedule and
/// repository enabled, and the host not hidden - the same gates
/// [`super::list_due_schedules`](crate::db::list_due_schedules) applies to a
/// regular tick.
///
/// A pending marker on a schedule that fails those gates is not returned here and
/// is cleared by [`clear_catch_up_pending`] all the same, so a stale miss can
/// never resurface days later when the schedule is re-enabled for unrelated
/// reasons.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_catch_up_candidates_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<CatchUpCandidate>, ApiError> {
    sqlx::query_as!(
        CatchUpCandidate,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, st.agent_id, a.hostname, \
         s.schedule_type AS \"schedule_type: ScheduleType\", s.cron_expression, \
         st.catch_up_pending_for AS \"pending_for!\", s.next_run_at, s.catch_up_min_lead_minutes \
         AS min_lead_minutes, a.catch_up_give_up_minutes AS give_up_minutes FROM schedule_targets \
         st JOIN schedules s ON s.id = st.schedule_id JOIN agents a ON a.id = st.agent_id WHERE \
         st.agent_id = $1 AND st.catch_up_pending_for IS NOT NULL AND a.intermittent = true AND \
         s.enabled = true AND a.is_hidden = false AND EXISTS (SELECT 1 FROM schedule_repos sr \
         JOIN repos r ON r.id = sr.repo_id WHERE sr.schedule_id = s.id AND r.enabled = true) \
         ORDER BY s.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every agent marker whose give-up window has already passed at `now`, so the
/// wait can be ended and reported rather than sitting there until a host that
/// may never return happens to reconnect.
///
/// Gated the same way [`list_catch_up_candidates_for_agent`] is: a marker whose
/// schedule is disabled, whose repositories are all disabled, or whose agent is
/// no longer marked as not always online, has nothing to report a failure
/// *for*, and is dropped by the reconnect handler in the usual way.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_expired_agent_catch_ups(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<CatchUpCandidate>, ApiError> {
    sqlx::query_as!(
        CatchUpCandidate,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, st.agent_id, a.hostname, \
         s.schedule_type AS \"schedule_type: ScheduleType\", s.cron_expression, \
         st.catch_up_pending_for AS \"pending_for!\", s.next_run_at, s.catch_up_min_lead_minutes \
         AS min_lead_minutes, a.catch_up_give_up_minutes AS give_up_minutes FROM schedule_targets \
         st JOIN schedules s ON s.id = st.schedule_id JOIN agents a ON a.id = st.agent_id WHERE \
         st.catch_up_pending_for IS NOT NULL AND a.intermittent = true AND s.enabled = true AND \
         a.is_hidden = false AND EXISTS (SELECT 1 FROM schedule_repos sr JOIN repos r ON r.id = \
         sr.repo_id WHERE sr.schedule_id = s.id AND r.enabled = true) AND \
         a.catch_up_give_up_minutes > 0 AND st.catch_up_pending_for + make_interval(mins => \
         a.catch_up_give_up_minutes) <= $1 ORDER BY s.id, st.agent_id",
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Drops one target's pending marker, for a wait that ended without the host
/// ever reconnecting - [`clear_catch_up_pending`] clears by agent across every
/// schedule, which is right at reconnect and wrong here.
///
/// Returns whether this call is the one that cleared it; `false` means the
/// reconnect path (or an earlier pass) already took the marker, and whoever
/// did is the one to act on it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_catch_up_pending_for_target(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<bool, ApiError> {
    let cleared = sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = NULL WHERE schedule_id = $1 AND \
         agent_id = $2 AND catch_up_pending_for IS NOT NULL",
        schedule_id,
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(cleared > 0)
}

/// Clears every pending marker this agent carries, whether or not the run it
/// represents is about to happen. The decision is made once, at reconnect: a miss
/// that is skipped (too close to the next run, catch-up switched off since,
/// schedule disabled) is dropped rather than carried forward, since the next
/// scheduled run supersedes it anyway.
///
/// Returns the number of markers cleared.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_catch_up_pending(pool: &PgPool, agent_id: i64) -> Result<u64, ApiError> {
    let cleared = sqlx::query!(
        "UPDATE schedule_targets SET catch_up_pending_for = NULL WHERE agent_id = $1 AND \
         catch_up_pending_for IS NOT NULL",
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(cleared)
}

/// [`clear_catch_up_pending`] for the reconnect path: clears every marker the
/// agent carries and returns the schedules whose marker *this* call cleared.
/// Only those may be acted on - a marker the give-up sweep cleared a moment
/// earlier has already been reported, and acting on it again would report the
/// same abandoned run twice.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn take_catch_up_pending(pool: &PgPool, agent_id: i64) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE schedule_targets SET catch_up_pending_for = NULL WHERE agent_id = $1 AND \
         catch_up_pending_for IS NOT NULL RETURNING schedule_id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// One repository of one schedule that has a run waiting to be caught up,
/// together with everything the poller needs to decide what to do with it -
/// see `crate::repo_catch_up`.
///
/// Unlike [`CatchUpCandidate`], the gates (the repository's own switch, both
/// `enabled` flags) are carried as columns rather than applied as a `WHERE`
/// clause. The agent side can filter them out because a reconnect clears every
/// marker the agent holds whether or not it ran; nothing clears a repository's
/// marker except this poller, so a marker the gates reject has to be *returned*
/// in order to be dropped, or it would sit in the table forever.
#[derive(Debug, Clone)]
pub struct RepoCatchUpCandidate {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// Schedule display name, for the log line and system event.
    pub schedule_name: String,
    /// Schedule type, as stored - parsed at the call site.
    pub schedule_type: ScheduleType,
    /// The schedule's cron expression, needed to advance `next_run_at` past a
    /// catch-up run the same way a scheduled tick does.
    pub cron_expression: String,
    /// The repository that was not there.
    pub repo_id: i64,
    /// That repository's display name.
    pub repo_name: String,
    /// The occurrence that was missed, and the point the give-up window is
    /// measured from.
    pub pending_for: DateTime<Utc>,
    /// When this repository was last asked whether it is back, or `None` if it
    /// has not been asked since the miss was recorded.
    pub last_probe_at: Option<DateTime<Utc>>,
    /// When the schedule next runs on its own, which the lead-time floor is
    /// measured against.
    pub next_run_at: Option<DateTime<Utc>>,
    /// How much time must be left before `next_run_at` for the catch-up to
    /// still be worth running - the schedule's setting.
    pub min_lead_minutes: i32,
    /// How often the repository is asked whether it is back - its own setting.
    pub recheck_minutes: i32,
    /// How long the repository is waited for before the run is abandoned; zero
    /// waits indefinitely - its own setting.
    pub give_up_minutes: i32,
    /// Whether the repository is still marked as not always online.
    pub intermittent: bool,
    /// Whether the schedule is still enabled.
    pub schedule_enabled: bool,
    /// Whether the repository is still enabled.
    pub repo_enabled: bool,
}

/// Records that `repo_id` was not reachable for `schedule_id`'s `due_at`
/// occurrence, so the run can be caught up once that repository answers again.
///
/// Resets `catch_up_last_probe_at` along with it: the probe clock belongs to
/// the wait that is starting now, not to one that ended however long ago.
///
/// Like its agent sibling this overwrites rather than accumulates, so however
/// many occurrences pass while the repository is away, exactly one catch-up run
/// follows.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_repo_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
    due_at: DateTime<Utc>,
    run_id: Option<&str>,
) -> Result<(), ApiError> {
    // A failed catch-up run keeps the occurrence it was catching up (see
    // hand_off_repo_catch_up), so retrying never pushes the give-up window
    // forward; any other run's failure names its own occurrence.
    sqlx::query!(
        "UPDATE schedule_repos SET catch_up_pending_for = CASE WHEN $4::text IS NOT NULL AND \
         catch_up_run_id = $4 THEN COALESCE(catch_up_run_for, $3) ELSE $3 END, \
         catch_up_last_probe_at = NULL, catch_up_run_id = NULL, catch_up_run_for = NULL WHERE \
         schedule_id = $1 AND repo_id = $2",
        schedule_id,
        repo_id,
        due_at,
        run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Clears a repository marker because its catch-up run `run_id` is being
/// dispatched, remembering which occurrence that run stands for. Should the
/// run itself fail against the repository again,
/// [`mark_repo_catch_up_pending`] restores that occurrence rather than dating
/// the new wait from the retry.
///
/// Returns whether this call took the marker. The poller and "Check now" can
/// look at the same marker at once; only the one that hands it off may run
/// it, or the same catch-up would be dispatched twice.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn hand_off_repo_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
    run_id: &str,
) -> Result<bool, ApiError> {
    let handed_off = sqlx::query!(
        "UPDATE schedule_repos SET catch_up_run_id = $3, catch_up_run_for = catch_up_pending_for, \
         catch_up_pending_for = NULL, catch_up_last_probe_at = NULL WHERE schedule_id = $1 AND \
         repo_id = $2 AND catch_up_pending_for IS NOT NULL",
        schedule_id,
        repo_id,
        run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(handed_off > 0)
}

/// When the run `run_id` started writing `repo_id`, read off its own backup
/// report - the occurrence a failed run stood for. `None` when the run left no
/// report for that repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn run_started_at(
    pool: &PgPool,
    run_id: &str,
    repo_id: i64,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    sqlx::query_scalar!(
        "SELECT MIN(started_at) FROM backup_reports WHERE run_id = $1 AND repo_id = $2",
        run_id,
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Which pending repository markers to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoCatchUpFilter {
    /// Every one - the poller's pass.
    All,
    /// Those of one schedule.
    Schedule(i64),
    /// Those waiting on one repository - its Power pane, and "check now".
    Repo(i64),
}

/// Every repository with a catch-up waiting on it, narrowed by `filter`.
/// Ordered by repository so the poller can probe each host once for however
/// many schedules are waiting on it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repo_catch_up_candidates(
    pool: &PgPool,
    filter: RepoCatchUpFilter,
) -> Result<Vec<RepoCatchUpCandidate>, ApiError> {
    let (schedule_id, repo_id) = match filter {
        RepoCatchUpFilter::All => (None, None),
        RepoCatchUpFilter::Schedule(id) => (Some(id), None),
        RepoCatchUpFilter::Repo(id) => (None, Some(id)),
    };
    sqlx::query_as!(
        RepoCatchUpCandidate,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, s.schedule_type AS \"schedule_type: \
         ScheduleType\", s.cron_expression, sr.repo_id, r.name AS repo_name, \
         sr.catch_up_pending_for AS \"pending_for!\", sr.catch_up_last_probe_at AS last_probe_at, \
         s.next_run_at, s.catch_up_min_lead_minutes AS min_lead_minutes, \
         r.catch_up_recheck_minutes AS recheck_minutes, r.catch_up_give_up_minutes AS \
         give_up_minutes, r.intermittent, s.enabled AS schedule_enabled, r.enabled AS \
         repo_enabled FROM schedule_repos sr JOIN schedules s ON s.id = sr.schedule_id JOIN repos \
         r ON r.id = sr.repo_id WHERE sr.catch_up_pending_for IS NOT NULL AND ($1::BIGINT IS NULL \
         OR s.id = $1) AND ($2::BIGINT IS NULL OR sr.repo_id = $2) ORDER BY sr.repo_id, s.id",
        schedule_id,
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Records that `repo_id` was asked, at `at`, whether it is back - whatever the
/// answer was. Written for every schedule waiting on that repository, because
/// the one probe answers all of them.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn record_repo_catch_up_probe(
    pool: &PgPool,
    schedule_ids: &[i64],
    repo_id: i64,
    at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedule_repos SET catch_up_last_probe_at = $3 WHERE schedule_id = ANY($1) AND \
         repo_id = $2",
        schedule_ids,
        repo_id,
        at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Drops one repository's pending marker, whether it is about to run, has been
/// abandoned, or no longer qualifies. Same "decided once" rule as the agent
/// side: a miss that is not run now is dropped rather than carried forward.
///
/// Returns whether this call took the marker: the poller and "Check now" can
/// look at the same marker at once, and only the one that clears it may act.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_repo_catch_up_pending(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
) -> Result<bool, ApiError> {
    let cleared = sqlx::query!(
        "UPDATE schedule_repos SET catch_up_pending_for = NULL, catch_up_last_probe_at = NULL \
         WHERE schedule_id = $1 AND repo_id = $2 AND catch_up_pending_for IS NOT NULL",
        schedule_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(cleared > 0)
}

/// Settles whatever `repo_id` was waiting to catch up for `schedule_id`, after
/// a backup of that schedule wrote into it: the pending marker and any record
/// of a catch-up run it was handed to. Returns whether there was anything to
/// settle.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn settle_repo_catch_up(
    pool: &PgPool,
    schedule_id: i64,
    repo_id: i64,
) -> Result<bool, ApiError> {
    let settled = sqlx::query!(
        "UPDATE schedule_repos SET catch_up_pending_for = NULL, catch_up_last_probe_at = NULL, \
         catch_up_run_id = NULL, catch_up_run_for = NULL WHERE schedule_id = $1 AND repo_id = $2 \
         AND (catch_up_pending_for IS NOT NULL OR catch_up_run_id IS NOT NULL)",
        schedule_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    Ok(settled > 0)
}

/// Drops every pending marker waiting on one repository, used when it stops
/// being marked as not always online: nothing is waiting for it any more, and
/// a miss recorded while it was must not run days later because somebody
/// switched it back on.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_repo_catch_up_pending_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedule_repos SET catch_up_pending_for = NULL, catch_up_last_probe_at = NULL, \
         catch_up_run_id = NULL, catch_up_run_for = NULL WHERE repo_id = $1 AND \
         (catch_up_pending_for IS NOT NULL OR catch_up_run_id IS NOT NULL)",
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A repository's "when the host is offline" settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoAvailabilityRow {
    /// Whether the host is marked as not always online.
    pub intermittent: bool,
    /// How often it is asked whether it is back while a catch-up waits on it.
    pub recheck_minutes: i32,
    /// How long it is waited for; zero waits indefinitely.
    pub give_up_minutes: i32,
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_repo_availability(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoAvailabilityRow, ApiError> {
    sqlx::query_as!(
        RepoAvailabilityRow,
        "SELECT intermittent, catch_up_recheck_minutes AS recheck_minutes, \
         catch_up_give_up_minutes AS give_up_minutes FROM repos WHERE id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repository {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the repository does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn update_repo_availability(
    pool: &PgPool,
    repo_id: i64,
    settings: RepoAvailabilityRow,
) -> Result<RepoAvailabilityRow, ApiError> {
    sqlx::query_as!(
        RepoAvailabilityRow,
        "UPDATE repos SET intermittent = $2, catch_up_recheck_minutes = $3, \
         catch_up_give_up_minutes = $4 WHERE id = $1 RETURNING intermittent, \
         catch_up_recheck_minutes AS recheck_minutes, catch_up_give_up_minutes AS give_up_minutes",
        repo_id,
        settings.intermittent,
        settings.recheck_minutes,
        settings.give_up_minutes,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repository {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// An agent's "when the host is offline" settings. No re-check interval: an
/// agent announces its own return by reconnecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAvailabilityRow {
    /// Whether the host is marked as not always online.
    pub intermittent: bool,
    /// How long it is waited for; zero waits indefinitely.
    pub give_up_minutes: i32,
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the agent does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_agent_availability(
    pool: &PgPool,
    agent_id: i64,
) -> Result<AgentAvailabilityRow, ApiError> {
    sqlx::query_as!(
        AgentAvailabilityRow,
        "SELECT intermittent, catch_up_give_up_minutes AS give_up_minutes FROM agents WHERE id = \
         $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent {agent_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::NotFound`] if the agent does not exist, or
/// [`ApiError::Database`] if the query fails.
pub async fn update_agent_availability(
    pool: &PgPool,
    agent_id: i64,
    settings: AgentAvailabilityRow,
) -> Result<AgentAvailabilityRow, ApiError> {
    sqlx::query_as!(
        AgentAvailabilityRow,
        "UPDATE agents SET intermittent = $2, catch_up_give_up_minutes = $3 WHERE id = $1 \
         RETURNING intermittent, catch_up_give_up_minutes AS give_up_minutes",
        agent_id,
        settings.intermittent,
        settings.give_up_minutes,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent {agent_id} not found")),
        other => ApiError::Database(other),
    })
}

/// One schedule waiting for an agent to reconnect.
#[derive(Debug, Clone)]
pub struct AgentCatchUpWait {
    /// Schedule the missed occurrence belongs to.
    pub schedule_id: i64,
    /// Its display name.
    pub schedule_name: String,
    /// The occurrence that was missed.
    pub pending_for: DateTime<Utc>,
}

/// Every schedule waiting for this agent, for its Power pane. Unfiltered by the
/// gates the reconnect handler applies: this is what is recorded, and the pane
/// only shows it while the agent is marked as not always online.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_agent_catch_up_waits(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<AgentCatchUpWait>, ApiError> {
    sqlx::query_as!(
        AgentCatchUpWait,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, st.catch_up_pending_for AS \
         \"pending_for!\" FROM schedule_targets st JOIN schedules s ON s.id = st.schedule_id \
         WHERE st.agent_id = $1 AND st.catch_up_pending_for IS NOT NULL ORDER BY \
         st.catch_up_pending_for, s.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A host or repository a schedule uses that is marked as not always online -
/// the chips beside the schedule's catch-up floor.
#[derive(Debug, Clone)]
pub struct CatchUpSourceRow {
    /// Agent or repository id.
    pub id: i64,
    /// Hostname for an agent, display name for a repository.
    pub name: String,
}

/// The schedule's targets that are marked as not always online, as
/// `(agents, repositories)`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn list_schedule_catch_up_sources(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(Vec<CatchUpSourceRow>, Vec<CatchUpSourceRow>), ApiError> {
    let agents = sqlx::query_as!(
        CatchUpSourceRow,
        "SELECT a.id, a.hostname AS name FROM schedule_targets st JOIN agents a ON a.id = \
         st.agent_id WHERE st.schedule_id = $1 AND a.intermittent = true ORDER BY \
         st.execution_order, a.hostname",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    let repos = sqlx::query_as!(
        CatchUpSourceRow,
        "SELECT r.id, r.name FROM schedule_repos sr JOIN repos r ON r.id = sr.repo_id WHERE \
         sr.schedule_id = $1 AND r.intermittent = true ORDER BY sr.execution_order, r.name",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok((agents, repos))
}
