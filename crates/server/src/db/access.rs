// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Repository permissions, groups and roles.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from the `repo_permissions` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RepoPermissionRow {
    /// User ID.
    pub user_id: i64,
    /// Repository ID.
    pub repo_id: i64,
    /// Whether the user can view the repo.
    pub can_view: bool,
    /// Whether the user can trigger backups.
    pub can_backup: bool,
    /// Whether the user can modify schedules.
    pub can_modify_schedules: bool,
    /// Whether the user can extract archives.
    pub can_extract: bool,
    /// Whether the user can delete archives.
    pub can_delete: bool,
}

/// Parameters for upserting a repo permission.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct UpsertRepoPermissionParams {
    /// User ID.
    pub user_id: i64,
    /// Repository ID.
    pub repo_id: i64,
    /// View permission.
    pub can_view: bool,
    /// Backup permission.
    pub can_backup: bool,
    /// Schedule modification permission.
    pub can_modify_schedules: bool,
    /// Extract permission.
    pub can_extract: bool,
    /// Delete permission.
    pub can_delete: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_repo_permission(
    pool: &PgPool,
    params: &UpsertRepoPermissionParams,
) -> Result<RepoPermissionRow, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "INSERT INTO repo_permissions (user_id, repo_id, can_view, can_backup, \
         can_modify_schedules, can_extract, can_delete) VALUES ($1, $2, $3, $4, $5, $6, $7) ON \
         CONFLICT (user_id, repo_id) DO UPDATE SET can_view = $3, can_backup = $4, \
         can_modify_schedules = $5, can_extract = $6, can_delete = $7 RETURNING user_id, repo_id, \
         can_view, can_backup, can_modify_schedules, can_extract, can_delete",
        params.user_id,
        params.repo_id,
        params.can_view,
        params.can_backup,
        params.can_modify_schedules,
        params.can_extract,
        params.can_delete,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_repo_permission(
    pool: &PgPool,
    user_id: i64,
    repo_id: i64,
) -> Result<Option<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 AND repo_id = $2",
        user_id,
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repo_permissions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 ORDER BY repo_id",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repo_permissions_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE repo_id = $1 ORDER BY user_id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A row from the `groups` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GroupRow {
    /// Unique identifier.
    pub id: i64,
    /// Group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
    /// When the group was created.
    pub created_at: DateTime<Utc>,
}

/// A row from the `roles` table representing an RBAC role.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RoleRow {
    /// Unique identifier.
    pub id: i64,
    /// Role name.
    pub name: String,
    /// Permission to create agents.
    pub can_create_agent: bool,
    /// Permission to delete any agent.
    pub can_delete_agent: bool,
    /// Permission to delete own agents.
    pub can_delete_own_agent: bool,
    /// Permission to create repos.
    pub can_create_repo: bool,
    /// Permission to delete any repo.
    pub can_delete_repo: bool,
    /// Permission to delete own repos.
    pub can_delete_own_repo: bool,
    /// Permission to create schedules.
    pub can_create_schedule: bool,
    /// Permission to delete any schedule.
    pub can_delete_schedule: bool,
    /// Permission to delete own schedules.
    pub can_delete_own_schedule: bool,
    /// Permission to manage tags.
    pub can_manage_tags: bool,
    /// Permission to view all repos.
    pub can_view_all_repos: bool,
    /// Permission to manage tunnels.
    pub can_manage_tunnels: bool,
    /// Permission to upgrade agents.
    pub can_upgrade_agent: bool,
    /// When the role was created.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod wake_secret_permission_tests {
    use chrono::Utc;

    use super::RoleRow;

    /// A role holding no permissions at all.
    fn none() -> RoleRow {
        RoleRow {
            id: 0,
            name: String::from("effective"),
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: false,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn a_plain_viewer_may_not_see_wake_secrets() {
        assert!(!none().can_view_wake_secrets());
    }

    #[test]
    fn an_admin_may_see_wake_secrets() {
        let role = RoleRow {
            can_delete_repo: true,
            ..none()
        };
        assert!(role.can_view_wake_secrets());
    }

    /// The operator tier: no repo deletion, but sight of every repo. It sees
    /// the addresses too, which is why the frontend must not gate the wake
    /// read-out on the admin role alone.
    #[test]
    fn an_operator_may_see_wake_secrets() {
        let role = RoleRow {
            can_view_all_repos: true,
            ..none()
        };
        assert!(role.can_view_wake_secrets());
    }
}

impl RoleRow {
    /// Whether these permissions may see a host's Wake-on-LAN MAC and
    /// broadcast address.
    ///
    /// The addresses let anyone holding them power the host on remotely, so
    /// they are gated to operators and admins rather than embedded for every
    /// viewer who can merely see the host. Encoded here once because three
    /// endpoints ask the same question: the agent and repo listings redact
    /// the fields, and `/auth/me` reports the answer so the frontend knows
    /// whether an absent address means "none configured" or "not shown to
    /// you".
    #[must_use]
    pub const fn can_view_wake_secrets(&self) -> bool {
        self.can_delete_repo || self.can_view_all_repos
    }
}

/// A row from the `user_groups` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserGroupRow {
    /// User ID.
    pub user_id: i64,
    /// Group ID.
    pub group_id: i64,
}

/// A row from the `user_roles` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserRoleRow {
    /// User ID.
    pub user_id: i64,
    /// Role ID.
    pub role_id: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_groups(pool: &PgPool) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT id, name, description, created_at FROM groups ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_group(pool: &PgPool, id: i64) -> Result<Option<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT id, name, description, created_at FROM groups WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_group(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "INSERT INTO groups (name, description) VALUES ($1, $2) RETURNING id, name, description, \
         created_at",
        name,
        description,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_group(
    pool: &PgPool,
    id: i64,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "UPDATE groups SET name = $2, description = $3 WHERE id = $1 RETURNING id, name, \
         description, created_at",
        id,
        name,
        description,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("group {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_group(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM groups WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_group_members(pool: &PgPool, group_id: i64) -> Result<Vec<i64>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT user_id FROM user_groups WHERE group_id = $1 ORDER BY user_id",
        group_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_group_members(
    pool: &PgPool,
    group_id: i64,
    user_ids: &[i64],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM user_groups WHERE group_id = $1", group_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for user_id in user_ids {
        sqlx::query!(
            "INSERT INTO user_groups (user_id, group_id) VALUES ($1, $2)",
            user_id,
            group_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_user_groups(pool: &PgPool, user_id: i64) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT g.id, g.name, g.description, g.created_at FROM groups g JOIN user_groups ug ON \
         ug.group_id = g.id WHERE ug.user_id = $1 ORDER BY g.name",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn user_shares_group_with(
    pool: &PgPool,
    user_id: i64,
    other_user_id: i64,
) -> Result<bool, ApiError> {
    #[derive(sqlx::FromRow)]
    struct ExistsRow {
        shared: Option<bool>,
    }

    let row = sqlx::query_as!(
        ExistsRow,
        "SELECT EXISTS(SELECT 1 FROM user_groups a JOIN user_groups b ON a.group_id = b.group_id \
         WHERE a.user_id = $1 AND b.user_id = $2) AS shared",
        user_id,
        other_user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.shared.unwrap_or(false))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_roles(pool: &PgPool) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent, created_at FROM roles ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_role(pool: &PgPool, id: i64) -> Result<Option<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent, created_at FROM roles WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Parameters for inserting a new role.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct InsertRoleParams<'a> {
    /// Role name.
    pub name: &'a str,
    /// Create agents permission.
    pub can_create_agent: bool,
    /// Delete any agent permission.
    pub can_delete_agent: bool,
    /// Delete own agents permission.
    pub can_delete_own_agent: bool,
    /// Create repos permission.
    pub can_create_repo: bool,
    /// Delete any repo permission.
    pub can_delete_repo: bool,
    /// Delete own repos permission.
    pub can_delete_own_repo: bool,
    /// Create schedules permission.
    pub can_create_schedule: bool,
    /// Delete any schedule permission.
    pub can_delete_schedule: bool,
    /// Delete own schedules permission.
    pub can_delete_own_schedule: bool,
    /// Manage tags permission.
    pub can_manage_tags: bool,
    /// View all repos permission.
    pub can_view_all_repos: bool,
    /// Manage tunnels permission.
    pub can_manage_tunnels: bool,
    /// Upgrade agents permission.
    pub can_upgrade_agent: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_role(
    pool: &PgPool,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "INSERT INTO roles (name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
         $11, $12, $13, $14) RETURNING id, name, can_create_agent, can_delete_agent, \
         can_delete_own_agent, can_create_repo, can_delete_repo, can_delete_own_repo, \
         can_create_schedule, can_delete_schedule, can_delete_own_schedule, can_manage_tags, \
         can_view_all_repos, can_manage_tunnels, can_upgrade_agent, created_at",
        params.name,
        params.can_create_agent,
        params.can_delete_agent,
        params.can_delete_own_agent,
        params.can_create_repo,
        params.can_delete_repo,
        params.can_delete_own_repo,
        params.can_create_schedule,
        params.can_delete_schedule,
        params.can_delete_own_schedule,
        params.can_manage_tags,
        params.can_view_all_repos,
        params.can_manage_tunnels,
        params.can_upgrade_agent,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_role(
    pool: &PgPool,
    id: i64,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "UPDATE roles SET name = $2, can_create_agent = $3, can_delete_agent = $4, \
         can_delete_own_agent = $5, can_create_repo = $6, can_delete_repo = $7, \
         can_delete_own_repo = $8, can_create_schedule = $9, can_delete_schedule = $10, \
         can_delete_own_schedule = $11, can_manage_tags = $12, can_view_all_repos = $13, \
         can_manage_tunnels = $14, can_upgrade_agent = $15 WHERE id = $1 RETURNING id, name, \
         can_create_agent, can_delete_agent, can_delete_own_agent, can_create_repo, \
         can_delete_repo, can_delete_own_repo, can_create_schedule, can_delete_schedule, \
         can_delete_own_schedule, can_manage_tags, can_view_all_repos, can_manage_tunnels, \
         can_upgrade_agent, created_at",
        id,
        params.name,
        params.can_create_agent,
        params.can_delete_agent,
        params.can_delete_own_agent,
        params.can_create_repo,
        params.can_delete_repo,
        params.can_delete_own_repo,
        params.can_create_schedule,
        params.can_delete_schedule,
        params.can_delete_own_schedule,
        params.can_manage_tags,
        params.can_view_all_repos,
        params.can_manage_tunnels,
        params.can_upgrade_agent,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("role {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_role(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM roles WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("role {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_user_roles(pool: &PgPool, user_id: i64) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT r.id, r.name, r.can_create_agent, r.can_delete_agent, r.can_delete_own_agent, \
         r.can_create_repo, r.can_delete_repo, r.can_delete_own_repo, r.can_create_schedule, \
         r.can_delete_schedule, r.can_delete_own_schedule, r.can_manage_tags, \
         r.can_view_all_repos, r.can_manage_tunnels, r.can_upgrade_agent, r.created_at FROM roles \
         r JOIN user_roles ur ON ur.role_id = r.id WHERE ur.user_id = $1 ORDER BY r.name",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_user_roles(pool: &PgPool, user_id: i64, role_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM user_roles WHERE user_id = $1", user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for role_id in role_ids {
        sqlx::query!(
            "INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)",
            user_id,
            role_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_effective_permissions(pool: &PgPool, user_id: i64) -> Result<RoleRow, ApiError> {
    #[derive(sqlx::FromRow)]
    #[allow(
        clippy::struct_field_names,
        reason = "matches the can_* RBAC column/field naming used consistently across the codebase"
    )]
    struct AggRow {
        can_create_agent: Option<bool>,
        can_delete_agent: Option<bool>,
        can_delete_own_agent: Option<bool>,
        can_create_repo: Option<bool>,
        can_delete_repo: Option<bool>,
        can_delete_own_repo: Option<bool>,
        can_create_schedule: Option<bool>,
        can_delete_schedule: Option<bool>,
        can_delete_own_schedule: Option<bool>,
        can_manage_tags: Option<bool>,
        can_view_all_repos: Option<bool>,
        can_manage_tunnels: Option<bool>,
        can_upgrade_agent: Option<bool>,
    }

    let row = sqlx::query_as!(
        AggRow,
        "SELECT BOOL_OR(r.can_create_agent) AS can_create_agent, BOOL_OR(r.can_delete_agent) AS \
         can_delete_agent, BOOL_OR(r.can_delete_own_agent) AS can_delete_own_agent, \
         BOOL_OR(r.can_create_repo) AS can_create_repo, BOOL_OR(r.can_delete_repo) AS \
         can_delete_repo, BOOL_OR(r.can_delete_own_repo) AS can_delete_own_repo, \
         BOOL_OR(r.can_create_schedule) AS can_create_schedule, BOOL_OR(r.can_delete_schedule) AS \
         can_delete_schedule, BOOL_OR(r.can_delete_own_schedule) AS can_delete_own_schedule, \
         BOOL_OR(r.can_manage_tags) AS can_manage_tags, BOOL_OR(r.can_view_all_repos) AS \
         can_view_all_repos, BOOL_OR(r.can_manage_tunnels) AS can_manage_tunnels, \
         BOOL_OR(r.can_upgrade_agent) AS can_upgrade_agent FROM roles r JOIN user_roles ur ON \
         ur.role_id = r.id WHERE ur.user_id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(RoleRow {
        id: 0,
        name: String::from("effective"),
        can_create_agent: row.can_create_agent.unwrap_or(false),
        can_delete_agent: row.can_delete_agent.unwrap_or(false),
        can_delete_own_agent: row.can_delete_own_agent.unwrap_or(false),
        can_create_repo: row.can_create_repo.unwrap_or(false),
        can_delete_repo: row.can_delete_repo.unwrap_or(false),
        can_delete_own_repo: row.can_delete_own_repo.unwrap_or(false),
        can_create_schedule: row.can_create_schedule.unwrap_or(false),
        can_delete_schedule: row.can_delete_schedule.unwrap_or(false),
        can_delete_own_schedule: row.can_delete_own_schedule.unwrap_or(false),
        can_manage_tags: row.can_manage_tags.unwrap_or(false),
        can_view_all_repos: row.can_view_all_repos.unwrap_or(false),
        can_manage_tunnels: row.can_manage_tunnels.unwrap_or(false),
        can_upgrade_agent: row.can_upgrade_agent.unwrap_or(false),
        created_at: Utc::now(),
    })
}
