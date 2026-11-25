// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Per-repository permissions granted to groups, and a user's effective
//! permissions: their own grants combined with those of every group they
//! belong to.

use shared::audit::RepoPermission;
use sqlx::PgPool;

use super::RepoPermissionRow;
use crate::error::ApiError;

/// What one group may do on one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRepoGrant {
    /// The repository.
    pub repo_id: i64,
    /// What the group's members may do on it.
    pub permissions: Vec<RepoPermission>,
}

/// A user's effective permissions on one repository: everything their own
/// grant and their groups' grants allow. `None` when none of them names the
/// repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_effective_repo_permission(
    pool: &PgPool,
    user_id: i64,
    repo_id: i64,
) -> Result<Option<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT $1::BIGINT AS \"user_id!\", $2::BIGINT AS \"repo_id!\", BOOL_OR(can_view) AS \
         \"can_view!\", BOOL_OR(can_backup) AS \"can_backup!\", BOOL_OR(can_modify_schedules) AS \
         \"can_modify_schedules!\", BOOL_OR(can_extract) AS \"can_extract!\", BOOL_OR(can_delete) \
         AS \"can_delete!\" FROM ( SELECT can_view, can_backup, can_modify_schedules, \
         can_extract, can_delete FROM repo_permissions WHERE user_id = $1 AND repo_id = $2 UNION \
         ALL SELECT g.can_view, g.can_backup, g.can_modify_schedules, g.can_extract, g.can_delete \
         FROM group_repo_permissions g JOIN user_groups ug ON ug.group_id = g.group_id WHERE \
         ug.user_id = $1 AND g.repo_id = $2 ) grants HAVING COUNT(*) > 0",
        user_id,
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// A user's effective permissions on every repository their own grants or
/// their groups' grants name, one row per repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_effective_repo_permissions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT $1::BIGINT AS \"user_id!\", repo_id AS \"repo_id!\", BOOL_OR(can_view) AS \
         \"can_view!\", BOOL_OR(can_backup) AS \"can_backup!\", BOOL_OR(can_modify_schedules) AS \
         \"can_modify_schedules!\", BOOL_OR(can_extract) AS \"can_extract!\", BOOL_OR(can_delete) \
         AS \"can_delete!\" FROM ( SELECT repo_id, can_view, can_backup, can_modify_schedules, \
         can_extract, can_delete FROM repo_permissions WHERE user_id = $1 UNION ALL SELECT \
         g.repo_id, g.can_view, g.can_backup, g.can_modify_schedules, g.can_extract, g.can_delete \
         FROM group_repo_permissions g JOIN user_groups ug ON ug.group_id = g.group_id WHERE \
         ug.user_id = $1 ) grants GROUP BY repo_id ORDER BY repo_id",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Every repository grant of a group, ordered by repository.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_group_repo_permissions(
    pool: &PgPool,
    group_id: i64,
) -> Result<Vec<GroupRepoGrant>, ApiError> {
    let rows = sqlx::query!(
        "SELECT repo_id, can_view, can_backup, can_modify_schedules, can_extract, can_delete FROM \
         group_repo_permissions WHERE group_id = $1 ORDER BY repo_id",
        group_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows
        .into_iter()
        .map(|row| GroupRepoGrant {
            repo_id: row.repo_id,
            permissions: RepoPermission::from_flags([
                row.can_view,
                row.can_backup,
                row.can_modify_schedules,
                row.can_extract,
                row.can_delete,
            ]),
        })
        .collect())
}

/// What a group may do on one repository; empty when it holds nothing there.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_group_repo_permission(
    pool: &PgPool,
    group_id: i64,
    repo_id: i64,
) -> Result<Vec<RepoPermission>, ApiError> {
    let row = sqlx::query!(
        "SELECT can_view, can_backup, can_modify_schedules, can_extract, can_delete FROM \
         group_repo_permissions WHERE group_id = $1 AND repo_id = $2",
        group_id,
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.map_or_else(Vec::new, |row| {
        RepoPermission::from_flags([
            row.can_view,
            row.can_backup,
            row.can_modify_schedules,
            row.can_extract,
            row.can_delete,
        ])
    }))
}

/// Sets what a group may do on a repository. An empty `permissions` removes
/// the grant.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails, for example
/// when the group or the repository does not exist.
pub async fn set_group_repo_permission(
    pool: &PgPool,
    group_id: i64,
    repo_id: i64,
    permissions: &[RepoPermission],
) -> Result<(), ApiError> {
    if permissions.is_empty() {
        sqlx::query!(
            "DELETE FROM group_repo_permissions WHERE group_id = $1 AND repo_id = $2",
            group_id,
            repo_id,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
        return Ok(());
    }
    sqlx::query!(
        "INSERT INTO group_repo_permissions (group_id, repo_id, can_view, can_backup, \
         can_modify_schedules, can_extract, can_delete) VALUES ($1, $2, $3, $4, $5, $6, $7) ON \
         CONFLICT (group_id, repo_id) DO UPDATE SET can_view = $3, can_backup = $4, \
         can_modify_schedules = $5, can_extract = $6, can_delete = $7",
        group_id,
        repo_id,
        RepoPermission::View.is_in(permissions),
        RepoPermission::Backup.is_in(permissions),
        RepoPermission::ModifySchedules.is_in(permissions),
        RepoPermission::Extract.is_in(permissions),
        RepoPermission::Delete.is_in(permissions),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}
