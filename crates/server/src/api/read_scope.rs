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
//! - sees a dependency host that an agent they see needs when it runs a
//!   schedule they see, or that the backup defaults of an agent they see
//!   require. One nothing they see needs is admin-only;
//! - sees which repository host a dependency shares a machine with only when
//!   they may read a repository on that host.
//!
//! A resource's `owner_id` and `visibility` columns grant nothing.

use std::collections::{HashMap, HashSet};

use super::auth::AuthUser;
use crate::{
    db::{self, dependency_hosts::DependencyLink},
    error::ApiError,
};

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

/// The repository hosts `user_id` may see named: an admin every one, anyone
/// else those holding a repository they may read.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn visible_repo_hosts(pool: &sqlx::PgPool, user_id: i64) -> Result<ReadScope, ApiError> {
    if db::get_effective_permissions(pool, user_id)
        .await?
        .can_delete_repo
    {
        return Ok(ReadScope::All);
    }
    let repos = readable_repos(pool, user_id).await?;
    Ok(ReadScope::Only(
        db::list_all_repos(pool)
            .await?
            .into_iter()
            .filter(|repo| repos.allows(repo.id))
            .map(|repo| repo.repo_host_id)
            .collect(),
    ))
}

/// The dependency hosts one user may see, and how much of what uses each one
/// they may count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyHostScope {
    /// An admin: every dependency, counted over everything that uses it.
    All,
    /// Anyone else: only what the schedules and agents they see need.
    Only(VisibleDependencies),
}

/// What the schedules and agents a user who is not an admin sees need of the
/// dependency hosts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibleDependencies {
    /// Per dependency, the visible schedules that need it on a visible agent.
    schedules: HashMap<i64, HashSet<i64>>,
    /// Per dependency, how many visible agents' backup defaults require it.
    agent_defaults: HashMap<i64, i64>,
    /// Per dependency, how many visible targets wait on it.
    waiting: HashMap<i64, i64>,
}

impl VisibleDependencies {
    /// Keeps what `schedules` and `agents` allow of every (dependency,
    /// schedule, agent) need in `links`, every `(agent_id,
    /// dependency_host_id)` default in `defaults` and every wait in `waits`.
    #[must_use]
    pub fn new(
        links: &[DependencyLink],
        defaults: &[(i64, i64)],
        waits: &[DependencyLink],
        schedules: &ReadScope,
        agents: &ReadScope,
    ) -> Self {
        let visible = |link: &&DependencyLink| {
            schedules.allows(link.schedule_id) && agents.allows(link.agent_id)
        };
        let mut out = Self::default();
        for link in links.iter().filter(visible) {
            out.schedules
                .entry(link.dependency_host_id)
                .or_default()
                .insert(link.schedule_id);
        }
        for (agent_id, dependency_host_id) in defaults {
            if agents.allows(*agent_id) {
                let count = out.agent_defaults.entry(*dependency_host_id).or_default();
                *count = count.saturating_add(1);
            }
        }
        for link in waits.iter().filter(visible) {
            let count = out.waiting.entry(link.dependency_host_id).or_default();
            *count = count.saturating_add(1);
        }
        out
    }
}

impl DependencyHostScope {
    /// Whether the dependency with this id is visible.
    #[must_use]
    pub fn allows(&self, id: i64) -> bool {
        match self {
            Self::All => true,
            Self::Only(visible) => {
                visible.schedules.contains_key(&id) || visible.agent_defaults.contains_key(&id)
            }
        }
    }

    /// How many schedules need the dependency, agents' defaults require it
    /// and targets wait on it, as far as the viewer may count them: `all`
    /// for an admin, only what they see for anyone else.
    #[must_use]
    pub fn counts(&self, id: i64, all: (i64, i64, i64)) -> (i64, i64, i64) {
        match self {
            Self::All => all,
            Self::Only(visible) => (
                visible
                    .schedules
                    .get(&id)
                    .map_or(0, |s| i64::try_from(s.len()).unwrap_or(i64::MAX)),
                visible.agent_defaults.get(&id).copied().unwrap_or(0),
                visible.waiting.get(&id).copied().unwrap_or(0),
            ),
        }
    }
}

/// The dependency hosts `user_id` may see.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn dependency_host_scope(
    pool: &sqlx::PgPool,
    user_id: i64,
) -> Result<DependencyHostScope, ApiError> {
    if db::get_effective_permissions(pool, user_id)
        .await?
        .can_delete_repo
    {
        return Ok(DependencyHostScope::All);
    }
    let schedules = visible_schedules(pool, user_id).await?;
    let agents = visible_agents(pool, user_id).await?;
    Ok(DependencyHostScope::Only(VisibleDependencies::new(
        &db::dependency_hosts::list_all_dependency_links(pool).await?,
        &db::dependency_hosts::list_all_agent_default_dependencies(pool).await?,
        &db::dependency_hosts::list_all_dependency_waits(pool).await?,
        &schedules,
        &agents,
    )))
}

/// Requires that the caller may see dependency `id`. One they may not is
/// reported exactly like one that does not exist.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for a dependency the caller may not see.
pub fn require_visible_dependency(scope: &DependencyHostScope, id: i64) -> Result<(), ApiError> {
    if scope.allows(id) {
        Ok(())
    } else {
        Err(ApiError::NotFound(format!("dependency {id} not found")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(dependency_host_id: i64, schedule_id: i64, agent_id: i64) -> DependencyLink {
        DependencyLink {
            dependency_host_id,
            schedule_id,
            agent_id,
        }
    }

    fn only(ids: &[i64]) -> ReadScope {
        ReadScope::Only(ids.iter().copied().collect())
    }

    #[test]
    fn an_admin_sees_every_dependency_with_every_count() {
        assert!(DependencyHostScope::All.allows(42));
        assert_eq!(DependencyHostScope::All.counts(42, (3, 2, 1)), (3, 2, 1));
    }

    /// Schedule 10 runs on agents 1 and 2, schedule 20 on agent 3. The
    /// viewer sees schedule 10 and agent 1 only.
    #[test]
    fn a_dependency_is_visible_through_what_the_viewer_sees_needs_it() {
        let links = [
            link(100, 10, 1),
            link(100, 10, 2),
            link(100, 20, 3),
            link(200, 10, 2),
            link(300, 20, 3),
        ];
        let defaults = [(1, 400), (3, 300), (3, 500)];
        let waits = [link(100, 10, 1), link(100, 20, 3)];
        let scope = DependencyHostScope::Only(VisibleDependencies::new(
            &links,
            &defaults,
            &waits,
            &only(&[10]),
            &only(&[1]),
        ));
        assert!(scope.allows(100), "schedule 10 needs it on agent 1");
        assert!(!scope.allows(200), "only on agent 2, which is not visible");
        assert!(!scope.allows(300), "only for what is not visible");
        assert!(scope.allows(400), "agent 1's defaults require it");
        assert!(!scope.allows(500), "agent 3's defaults are not visible");
        assert!(!scope.allows(600), "nothing needs it");
        assert_eq!(scope.counts(100, (2, 0, 2)), (1, 0, 1));
        assert_eq!(scope.counts(400, (0, 1, 0)), (0, 1, 0));
        assert_eq!(scope.counts(600, (0, 0, 0)), (0, 0, 0));
    }

    #[test]
    fn an_unseen_dependency_reads_as_not_found() {
        let scope = DependencyHostScope::Only(VisibleDependencies::default());
        assert!(matches!(
            require_visible_dependency(&scope, 7),
            Err(ApiError::NotFound(_))
        ));
        assert!(require_visible_dependency(&DependencyHostScope::All, 7).is_ok());
    }

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
