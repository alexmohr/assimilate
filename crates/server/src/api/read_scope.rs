// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Which repositories, schedules and agents a signed-in user may read.
//!
//! Everything follows from per-repository grants, and one rule per kind of
//! resource is shared by the list endpoints, the single-item endpoints, the
//! statistics and the UI socket, so that none of them shows what another
//! hides. An admin (`can_delete_repo`) and a holder of **View All Repos** read
//! everything. Anyone else:
//!
//! - reads a repository they, or a group they belong to, were granted
//!   `can_view` on;
//! - sees a schedule whose repository they may read - the repository its
//!   edit permission is checked against too. A schedule left with no
//!   repository is admin-only;
//! - sees an agent that one of those schedules runs on. A role with **View
//!   All Agents** or **Upgrade Agent** sees every agent, including a new one
//!   no schedule backs up yet.
//!
//! A resource's `owner_id` and `visibility` columns grant nothing.

use std::collections::HashSet;

use super::auth::AuthUser;
use crate::{db, error::ApiError};

/// The ids of one kind of resource a user may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadScope {
    /// Every one of them, including those created later.
    All,
    /// Only these.
    Only(HashSet<i64>),
}

impl ReadScope {
    /// Whether the resource with this id is readable.
    #[must_use]
    pub fn allows(&self, id: i64) -> bool {
        match self {
            Self::All => true,
            Self::Only(ids) => ids.contains(&id),
        }
    }

    /// The scope as a query bind: `None` for every id, otherwise the ids, for
    /// a `($n::BIGINT[] IS NULL OR id = ANY($n))` filter.
    #[must_use]
    pub fn sql_ids(&self) -> Option<Vec<i64>> {
        match self {
            Self::All => None,
            Self::Only(ids) => Some(ids.iter().copied().collect()),
        }
    }
}

/// The repositories `user_id` may read.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn readable_repos(pool: &sqlx::PgPool, user_id: i64) -> Result<ReadScope, ApiError> {
    let Some(granted) = granted_repos(pool, user_id).await? else {
        return Ok(ReadScope::All);
    };
    Ok(ReadScope::Only(granted.into_iter().collect()))
}

/// The repositories a restricted user was granted `can_view` on, or `None`
/// for a user who reads everything.
async fn granted_repos(pool: &sqlx::PgPool, user_id: i64) -> Result<Option<Vec<i64>>, ApiError> {
    let effective = db::get_effective_permissions(pool, user_id).await?;
    if effective.can_delete_repo || effective.can_view_all_repos {
        return Ok(None);
    }
    Ok(Some(view_grants(pool, user_id).await?))
}

/// The repositories `user_id`'s own grants or their groups' grants allow
/// them to view.
async fn view_grants(pool: &sqlx::PgPool, user_id: i64) -> Result<Vec<i64>, ApiError> {
    Ok(db::list_effective_repo_permissions_for_user(pool, user_id)
        .await?
        .into_iter()
        .filter(|grant| grant.can_view)
        .map(|grant| grant.repo_id)
        .collect())
}

/// The agents `user_id` may see. Only an admin sees a hidden agent: hiding an
/// agent takes it out of every other user's view, View All Agents included.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn visible_agents(pool: &sqlx::PgPool, user_id: i64) -> Result<ReadScope, ApiError> {
    let effective = db::get_effective_permissions(pool, user_id).await?;
    if effective.can_delete_repo {
        return Ok(ReadScope::All);
    }
    if effective.sees_all_agents() {
        return Ok(ReadScope::Only(
            db::list_agents(pool, false)
                .await?
                .into_iter()
                .map(|agent| agent.id)
                .collect(),
        ));
    }
    // Whoever reads every repository also sees every agent, so only the
    // user's own view grants are left to decide.
    let granted = view_grants(pool, user_id).await?;
    Ok(ReadScope::Only(
        db::list_agent_ids_backing_up_to(pool, &granted)
            .await?
            .into_iter()
            .collect(),
    ))
}

/// The hostnames of the agents `user_id` may see, as a query bind: `None`
/// for every agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn visible_agent_hostnames(
    pool: &sqlx::PgPool,
    user_id: i64,
) -> Result<Option<Vec<String>>, ApiError> {
    let ReadScope::Only(ids) = visible_agents(pool, user_id).await? else {
        return Ok(None);
    };
    Ok(Some(
        db::list_agents(pool, true)
            .await?
            .into_iter()
            .filter(|agent| ids.contains(&agent.id))
            .map(|agent| agent.hostname)
            .collect(),
    ))
}

/// The schedules `user_id` may see: those that write into a repository they
/// may read, as their primary target or as one of their further copies.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn visible_schedules(pool: &sqlx::PgPool, user_id: i64) -> Result<ReadScope, ApiError> {
    let ReadScope::Only(repos) = readable_repos(pool, user_id).await? else {
        return Ok(ReadScope::All);
    };
    let repos: Vec<i64> = repos.into_iter().collect();
    Ok(ReadScope::Only(
        db::list_schedule_ids_writing_to(pool, &repos)
            .await?
            .into_iter()
            .collect(),
    ))
}

/// Requires that the caller may see `schedule`. One they may not is reported
/// exactly like one that does not exist.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for a schedule the caller may not see.
pub async fn require_visible_schedule(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    schedule: &db::ScheduleRow,
) -> Result<(), ApiError> {
    if visible_schedules(pool, auth.user_id)
        .await?
        .allows(schedule.id)
    {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!(
            "schedule {} not found",
            schedule.id
        )))
    }
}

/// Requires that the caller may read the repository. One they may not is
/// reported exactly like one that does not exist.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for a repository the caller may not read.
pub async fn require_readable_repo(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    repo_id: i64,
) -> Result<(), ApiError> {
    if readable_repos(pool, auth.user_id).await?.allows(repo_id) {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!("repo {repo_id} not found")))
    }
}

/// Requires that the caller may see `agent`. One they may not is reported
/// exactly like a hostname that does not exist.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for an agent the caller may not see.
pub async fn require_visible_agent(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    agent: &db::AgentRow,
) -> Result<(), ApiError> {
    if visible_agents(pool, auth.user_id).await?.allows(agent.id) {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!(
            "agent '{}' not found",
            agent.hostname
        )))
    }
}

/// Loads an agent by hostname, as [`db::get_agent_by_hostname`] does, and
/// requires that the caller may see it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for an agent that does not exist or that
/// the caller may not see, or [`ApiError::Conflict`] for an ambiguous
/// hostname.
pub async fn get_visible_agent(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    hostname: &str,
    domain: Option<&str>,
) -> Result<db::AgentRow, ApiError> {
    let agent = db::get_agent_by_hostname(pool, hostname, domain).await?;
    require_visible_agent(pool, auth, &agent).await?;
    Ok(agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_allows_any_id() {
        assert!(ReadScope::All.allows(1));
        assert!(ReadScope::All.allows(i64::MAX));
    }

    #[test]
    fn only_allows_exactly_its_ids() {
        let scope = ReadScope::Only(HashSet::from([3, 7]));
        assert!(scope.allows(3));
        assert!(scope.allows(7));
        assert!(!scope.allows(4));
        assert!(!ReadScope::Only(HashSet::new()).allows(3));
    }

    #[test]
    fn sql_ids_is_none_for_all_and_the_ids_otherwise() {
        assert_eq!(ReadScope::All.sql_ids(), None);
        assert_eq!(ReadScope::Only(HashSet::from([5])).sql_ids(), Some(vec![5]));
        assert_eq!(ReadScope::Only(HashSet::new()).sql_ids(), Some(vec![]));
    }
}
