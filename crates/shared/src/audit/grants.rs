// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a role or a repository permission grants, recorded as the list of
//! granted permissions rather than a block of flags, so an entry shows what
//! changed at a glance.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::responses::{RepoPermissionResponse, RoleResponse};

/// One permission a role grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum RolePermission {
    /// Create agents.
    CreateAgent,
    /// Delete any agent.
    DeleteAgent,
    /// Delete one's own agents.
    DeleteOwnAgent,
    /// Create repositories.
    CreateRepo,
    /// Delete any repository.
    DeleteRepo,
    /// Delete one's own repositories.
    DeleteOwnRepo,
    /// Create schedules.
    CreateSchedule,
    /// Delete any schedule.
    DeleteSchedule,
    /// Delete one's own schedules.
    DeleteOwnSchedule,
    /// Manage tags.
    ManageTags,
    /// View every repository.
    ViewAllRepos,
    /// Manage SSH tunnels.
    ManageTunnels,
    /// Upgrade agents.
    UpgradeAgent,
}

impl RolePermission {
    /// The permissions `role` grants, in a fixed order.
    #[must_use]
    pub fn granted_by(role: &RoleResponse) -> Vec<Self> {
        [
            (role.can_create_agent, Self::CreateAgent),
            (role.can_delete_agent, Self::DeleteAgent),
            (role.can_delete_own_agent, Self::DeleteOwnAgent),
            (role.can_create_repo, Self::CreateRepo),
            (role.can_delete_repo, Self::DeleteRepo),
            (role.can_delete_own_repo, Self::DeleteOwnRepo),
            (role.can_create_schedule, Self::CreateSchedule),
            (role.can_delete_schedule, Self::DeleteSchedule),
            (role.can_delete_own_schedule, Self::DeleteOwnSchedule),
            (role.can_manage_tags, Self::ManageTags),
            (role.can_view_all_repos, Self::ViewAllRepos),
            (role.can_manage_tunnels, Self::ManageTunnels),
            (role.can_upgrade_agent, Self::UpgradeAgent),
        ]
        .into_iter()
        .filter_map(|(granted, permission)| granted.then_some(permission))
        .collect()
    }
}

/// One permission a user holds on a single repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum RepoPermission {
    /// See the repository and its archives.
    View,
    /// Run backups into it.
    Backup,
    /// Change its schedules.
    ModifySchedules,
    /// Extract, download or restore files from it.
    Extract,
    /// Delete its archives.
    Delete,
}

impl RepoPermission {
    /// The permissions `permission` grants, in a fixed order.
    #[must_use]
    pub fn granted_by(permission: &RepoPermissionResponse) -> Vec<Self> {
        [
            (permission.can_view, Self::View),
            (permission.can_backup, Self::Backup),
            (permission.can_modify_schedules, Self::ModifySchedules),
            (permission.can_extract, Self::Extract),
            (permission.can_delete, Self::Delete),
        ]
        .into_iter()
        .filter_map(|(granted, permission)| granted.then_some(permission))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn a_role_lists_only_what_it_grants() {
        let role = RoleResponse {
            id: 1,
            name: "backup-operator".to_owned(),
            created_at: Utc::now(),
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: true,
            can_create_repo: true,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: true,
        };
        assert_eq!(
            RolePermission::granted_by(&role),
            [
                RolePermission::DeleteOwnAgent,
                RolePermission::CreateRepo,
                RolePermission::UpgradeAgent,
            ]
        );
    }

    #[test]
    fn a_repo_permission_lists_only_what_it_grants() {
        let permission = RepoPermissionResponse {
            user_id: 2,
            repo_id: 3,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract: true,
            can_delete: false,
        };
        assert_eq!(
            RepoPermission::granted_by(&permission),
            [RepoPermission::View, RepoPermission::Extract]
        );
    }

    #[test]
    fn permissions_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_value(RolePermission::ViewAllRepos).unwrap(),
            serde_json::json!("view_all_repos")
        );
        assert_eq!(
            serde_json::to_value(RepoPermission::ModifySchedules).unwrap(),
            serde_json::json!("modify_schedules")
        );
    }
}
