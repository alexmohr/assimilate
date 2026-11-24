// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Which live UI events one signed-in user may receive.
//!
//! Every UI socket subscribes to the same broadcast, so each one filters the
//! stream down to the agents and repositories its user can see through the
//! list endpoints (`GET /api/agents`, `GET /api/repos`): an admin sees
//! everything, anyone else only what they own or what is shared with a group
//! they belong to.

use std::collections::HashSet;

use shared::protocol::ServerToUi;

use crate::{api::permissions::is_visible_to_user, db, error::ApiError};

/// The agents and repositories one UI socket's user may see events for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAudience {
    /// An admin: every event is delivered.
    Everything,
    /// Anyone else: only events whose agent and repository are both visible.
    Limited(VisibleScope),
}

/// The agents and repositories a non-admin user can see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibleScope {
    /// Ids of the visible agents.
    pub agent_ids: HashSet<i64>,
    /// Hostnames of the visible agents. Events that name an agent by
    /// hostname only are matched against this.
    pub hostnames: HashSet<String>,
    /// Ids of the visible repositories.
    pub repo_ids: HashSet<i64>,
    /// Names of the visible repositories. Events that name a repository by
    /// its display name only (`target_name`) are matched against this.
    pub repo_names: HashSet<String>,
}

impl VisibleScope {
    fn hostname(&self, hostname: &str) -> bool {
        self.hostnames.contains(hostname)
    }

    fn repo(&self, repo_id: i64) -> bool {
        self.repo_ids.contains(&repo_id)
    }

    fn repo_name(&self, name: &str) -> bool {
        self.repo_names.contains(name)
    }

    fn agent(&self, agent_id: i64) -> bool {
        self.agent_ids.contains(&agent_id)
    }

    /// Whether every agent and repository `event` names is visible.
    fn covers(&self, event: &ServerToUi) -> bool {
        match event {
            ServerToUi::AgentConnected { hostname }
            | ServerToUi::AgentDisconnected { hostname }
            | ServerToUi::ConfigUpdated { hostname } => self.hostname(hostname),
            ServerToUi::BackupStarted {
                hostname,
                target_name,
                ..
            }
            | ServerToUi::CheckCompleted {
                hostname,
                target_name,
                ..
            }
            | ServerToUi::VerifyCompleted {
                hostname,
                target_name,
                ..
            }
            | ServerToUi::CanaryVerified {
                hostname,
                target_name,
                ..
            } => self.hostname(hostname) && self.repo_name(target_name),
            ServerToUi::BackupCompleted {
                hostname,
                target_name,
                report,
            } => {
                self.hostname(hostname)
                    && self.repo_name(target_name)
                    && self.repo(report.repo_id.0)
            }
            ServerToUi::BackupStopped {
                hostname,
                target_name,
                repo_id,
                ..
            } => self.hostname(hostname) && self.repo_name(target_name) && self.repo(*repo_id),
            ServerToUi::BackupLog {
                hostname, repo_id, ..
            } => self.hostname(hostname) && self.repo(*repo_id),
            ServerToUi::ImportProgress { repo_id, .. }
            | ServerToUi::RepoOpChanged { repo_id, .. }
            | ServerToUi::ArchiveDeleted { repo_id, .. } => self.repo(*repo_id),
            ServerToUi::TunnelStatusChanged {
                agent_id, hostname, ..
            } => self.agent(*agent_id) && self.hostname(hostname),
            ServerToUi::RunEvent {
                hostname,
                agent_id,
                repo_id,
                ..
            } => self.hostname(hostname) && self.agent(*agent_id) && self.repo(*repo_id),
            // Carry no agent or repository: a bare "refetch" signal, and a
            // restore's id and status only (its details stay admin-only
            // behind `GET /api/restores/{id}`).
            ServerToUi::DataChanged | ServerToUi::RestoreRunChanged { .. } => true,
        }
    }
}

impl UiAudience {
    /// Loads what `user_id` may see, using the same visibility rules as the
    /// agent and repository list endpoints.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if a query fails.
    pub async fn load(pool: &sqlx::PgPool, user_id: i64) -> Result<Self, ApiError> {
        let effective = db::get_effective_permissions(pool, user_id).await?;
        if effective.can_delete_repo {
            return Ok(Self::Everything);
        }

        let mut scope = VisibleScope::default();
        for agent in db::list_agents(pool, false).await? {
            if is_visible_to_user(pool, user_id, agent.owner_id, agent.visibility, false).await? {
                scope.agent_ids.insert(agent.id);
                scope.hostnames.insert(agent.hostname);
            }
        }
        for repo in db::list_all_repos(pool).await? {
            if is_visible_to_user(pool, user_id, repo.owner_id, repo.visibility, false).await? {
                scope.repo_ids.insert(repo.id);
                scope.repo_names.insert(repo.name);
            }
        }
        Ok(Self::Limited(scope))
    }

    /// Whether this audience may see a backup by `hostname` into `repo_id`.
    /// Used to replay a running backup's snapshot, whose repository name
    /// can be empty when the snapshot was started from a progress line.
    #[must_use]
    pub fn may_see_backup(&self, hostname: &str, repo_id: i64) -> bool {
        match self {
            Self::Everything => true,
            Self::Limited(scope) => scope.hostname(hostname) && scope.repo(repo_id),
        }
    }

    /// Whether this audience may receive `event`.
    #[must_use]
    pub fn may_see(&self, event: &ServerToUi) -> bool {
        match self {
            Self::Everything => true,
            Self::Limited(scope) => scope.covers(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::{
        protocol::TunnelStatus,
        time::Timestamp,
        types::{RestoreRunStatus, RunEventTarget, RunEventType},
    };

    use super::*;

    fn scope() -> VisibleScope {
        VisibleScope {
            agent_ids: HashSet::from([1]),
            hostnames: HashSet::from(["mine".to_owned()]),
            repo_ids: HashSet::from([10]),
            repo_names: HashSet::from(["my-repo".to_owned()]),
        }
    }

    fn limited() -> UiAudience {
        UiAudience::Limited(scope())
    }

    fn backup_log(hostname: &str, repo_id: i64) -> ServerToUi {
        ServerToUi::BackupLog {
            hostname: hostname.to_owned(),
            schedule_id: None,
            repo_id,
            line: "/home/someone/secret.txt".to_owned(),
        }
    }

    fn started(hostname: &str, target_name: &str) -> ServerToUi {
        ServerToUi::BackupStarted {
            hostname: hostname.to_owned(),
            target_name: target_name.to_owned(),
            archive_name: None,
            schedule_id: None,
            started_at: Timestamp::now(),
        }
    }

    fn run_event(agent_id: i64, hostname: &str, repo_id: i64) -> ServerToUi {
        ServerToUi::RunEvent {
            run_id: "run".to_owned(),
            target: RunEventTarget::Source,
            event_type: RunEventType::WakeSent,
            message: String::new(),
            occurred_at: Timestamp::now(),
            hostname: hostname.to_owned(),
            agent_id,
            repo_id,
        }
    }

    #[test]
    fn an_admin_sees_events_for_any_agent_or_repo() {
        assert!(UiAudience::Everything.may_see(&backup_log("theirs", 99)));
        assert!(UiAudience::Everything.may_see(&started("theirs", "their-repo")));
    }

    #[test]
    fn a_limited_user_sees_events_for_their_own_agent_and_repo() {
        let audience = limited();
        assert!(audience.may_see(&backup_log("mine", 10)));
        assert!(audience.may_see(&started("mine", "my-repo")));
        assert!(audience.may_see(&run_event(1, "mine", 10)));
        assert!(audience.may_see(&ServerToUi::AgentConnected {
            hostname: "mine".to_owned()
        }));
        assert!(audience.may_see(&ServerToUi::RepoOpChanged {
            repo_id: 10,
            op: None
        }));
    }

    #[test]
    fn a_limited_user_does_not_see_another_hosts_backup_lines() {
        assert!(!limited().may_see(&backup_log("theirs", 10)));
    }

    #[test]
    fn a_limited_user_does_not_see_their_host_writing_to_a_hidden_repo() {
        let audience = limited();
        assert!(!audience.may_see(&backup_log("mine", 99)));
        assert!(!audience.may_see(&started("mine", "their-repo")));
        assert!(!audience.may_see(&ServerToUi::CheckCompleted {
            hostname: "mine".to_owned(),
            target_name: "their-repo".to_owned(),
            success: true,
            error_message: None,
        }));
    }

    #[test]
    fn a_limited_user_does_not_see_hidden_agents_or_repos_by_id() {
        let audience = limited();
        assert!(!audience.may_see(&ServerToUi::AgentDisconnected {
            hostname: "theirs".to_owned()
        }));
        assert!(!audience.may_see(&ServerToUi::ImportProgress {
            repo_id: 99,
            progress: 1,
            total: 2,
            message: None,
        }));
        assert!(!audience.may_see(&ServerToUi::ArchiveDeleted {
            repo_id: 99,
            archive_name: "a".to_owned(),
        }));
        assert!(!audience.may_see(&ServerToUi::TunnelStatusChanged {
            agent_id: 2,
            hostname: "mine".to_owned(),
            status: TunnelStatus::Connected,
        }));
        assert!(!audience.may_see(&run_event(2, "mine", 10)));
        assert!(!audience.may_see(&run_event(1, "mine", 99)));
    }

    fn report(repo_id: i64) -> Box<shared::types::BackupReport> {
        Box::new(crate::test_support::backup_report(1, repo_id, None))
    }

    /// Every maintenance and completion event is shown only when both its
    /// host and its repository are visible.
    #[test]
    fn completion_events_need_both_their_host_and_their_repo() {
        let audience = limited();
        for (hostname, target_name, repo_id, visible) in [
            ("mine", "my-repo", 10, true),
            ("theirs", "my-repo", 10, false),
            ("mine", "their-repo", 10, false),
            ("mine", "my-repo", 99, false),
        ] {
            let events = [
                ServerToUi::VerifyCompleted {
                    hostname: hostname.to_owned(),
                    target_name: target_name.to_owned(),
                    success: true,
                    error_message: None,
                },
                ServerToUi::CanaryVerified {
                    hostname: hostname.to_owned(),
                    target_name: target_name.to_owned(),
                    success: true,
                    error_message: None,
                },
                ServerToUi::BackupCompleted {
                    hostname: hostname.to_owned(),
                    target_name: target_name.to_owned(),
                    report: report(repo_id),
                },
                ServerToUi::BackupStopped {
                    hostname: hostname.to_owned(),
                    target_name: target_name.to_owned(),
                    repo_id,
                    schedule_id: None,
                },
            ];
            for event in &events {
                // VerifyCompleted and CanaryVerified carry no repo id, so
                // only the repo-id case differs between them and the rest.
                let names_repo_id = matches!(
                    event,
                    ServerToUi::BackupCompleted { .. } | ServerToUi::BackupStopped { .. }
                );
                let expected = visible || (repo_id == 99 && !names_repo_id);
                assert_eq!(audience.may_see(event), expected, "{event:?}");
            }
        }
    }

    #[test]
    fn a_running_backup_is_replayed_by_host_and_repo_id() {
        let audience = limited();
        assert!(audience.may_see_backup("mine", 10));
        assert!(!audience.may_see_backup("theirs", 10));
        assert!(!audience.may_see_backup("mine", 99));
        assert!(UiAudience::Everything.may_see_backup("theirs", 99));
    }

    #[test]
    fn events_that_name_no_agent_or_repo_reach_everyone() {
        let nobody = UiAudience::Limited(VisibleScope::default());
        assert!(nobody.may_see(&ServerToUi::DataChanged));
        assert!(nobody.may_see(&ServerToUi::RestoreRunChanged {
            id: "r".to_owned(),
            status: RestoreRunStatus::Pending,
        }));
    }
}
