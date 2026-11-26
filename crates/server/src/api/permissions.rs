// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use shared::{
    audit::{AuditEvent, RepoPermission},
    responses::RepoPermissionResponse,
};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    auth::{AuthUser, RequireAdmin},
};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::RepoPermissionRow> for RepoPermissionResponse {
    fn from(row: db::RepoPermissionRow) -> Self {
        Self {
            user_id: row.user_id,
            repo_id: row.repo_id,
            can_view: row.can_view,
            can_backup: row.can_backup,
            can_modify_schedules: row.can_modify_schedules,
            can_extract: row.can_extract,
            can_delete: row.can_delete,
        }
    }
}

/// Request payload for setting a user's repository permissions.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct UpsertPermissionRequest {
    /// Permission to view the repository.
    pub can_view: bool,
    /// Permission to back up to the repository.
    pub can_backup: bool,
    /// Permission to modify schedules for the repository.
    pub can_modify_schedules: bool,
    /// Permission to extract files from the repository.
    pub can_extract: bool,
    /// Permission to delete archives from the repository.
    pub can_delete: bool,
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/permissions",
    tag = "Permissions",
    operation_id = "listPermissionsForRepo",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "List of permissions",
            body = Vec<RepoPermissionResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List all user permissions for a repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_for_repo(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<RepoPermissionResponse>>, ApiError> {
    let perms: Vec<RepoPermissionResponse> =
        db::list_repo_permissions_for_repo(&state.pool, repo_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(perms))
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/permissions/{user_id}",
    tag = "Permissions",
    operation_id = "upsertPermission",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("user_id" = i64, Path, description = "User ID"),
    ),
    request_body = UpsertPermissionRequest,
    responses(
        (status = 200, description = "Updated permission", body = RepoPermissionResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Set or update a user's permissions for a repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn upsert(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path((repo_id, user_id)): Path<(i64, i64)>,
    ApiJson(req): ApiJson<UpsertPermissionRequest>,
) -> Result<Json<RepoPermissionResponse>, ApiError> {
    let before = db::get_repo_permission(&state.pool, user_id, repo_id)
        .await?
        .map_or_else(Vec::new, |row| RepoPermission::granted_by(&row.into()));
    let user = db::get_user_by_id(&state.pool, user_id).await?;
    let perm: RepoPermissionResponse = db::upsert_repo_permission(
        &state.pool,
        &db::UpsertRepoPermissionParams {
            user_id,
            repo_id,
            can_view: req.can_view,
            can_backup: req.can_backup,
            can_modify_schedules: req.can_modify_schedules,
            can_extract: req.can_extract,
            can_delete: req.can_delete,
        },
    )
    .await?
    .into();
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Repo(repo_id)),
        AuditEvent::SetRepoPermission {
            username: user.username,
            before,
            after: RepoPermission::granted_by(&perm),
        },
    )
    .await;
    Ok(Json(perm))
}

#[utoipa::path(
    get,
    path = "/api/users/{id}/permissions",
    tag = "Permissions",
    operation_id = "listPermissionsForUser",
    params(("id" = i64, Path, description = "User ID")),
    responses(
        (status = 200, description = "List of permissions",
            body = Vec<RepoPermissionResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List all repository permissions for a user.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_for_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<RepoPermissionResponse>>, ApiError> {
    let perms: Vec<RepoPermissionResponse> =
        db::list_repo_permissions_for_user(&state.pool, user_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(perms))
}

/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller lacks permission for this operation.
pub async fn check_repo_permission(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    repo_id: i64,
    check: impl Fn(&db::RepoPermissionRow) -> bool,
) -> Result<(), ApiError> {
    let effective = db::get_effective_permissions(pool, auth.user_id).await?;
    if effective.can_delete_repo {
        return Ok(());
    }

    let perm = db::get_effective_repo_permission(pool, auth.user_id, repo_id).await?;
    if perm.is_some_and(|p| check(&p)) {
        return Ok(());
    }

    if effective.can_view_all_repos {
        let view_only = db::RepoPermissionRow {
            user_id: auth.user_id,
            repo_id,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract: false,
            can_delete: false,
        };
        if check(&view_only) {
            return Ok(());
        }
    }

    Err(ApiError::Forbidden(
        "insufficient repo permission".to_string(),
    ))
}

/// Loads a schedule the caller may read, under the rule in
/// [`super::read_scope`] that `GET /api/schedules` filters its list by.
///
/// A schedule carries its hook commands, which often hold credentials, so
/// every read of one goes through here. A schedule the caller cannot see is
/// reported exactly like one that does not exist.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for a schedule that does not exist or that
/// the caller cannot see.
pub async fn get_visible_schedule(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    schedule_id: i64,
) -> Result<db::ScheduleRow, ApiError> {
    let schedule = db::get_schedule_by_id(pool, schedule_id).await?;
    super::read_scope::require_visible_schedule(pool, auth, &schedule).await?;
    Ok(schedule)
}

/// Requires that the caller can see every agent in `agent_ids` in the agent
/// list, the same rule `GET /api/agents` applies.
///
/// A schedule runs its hook commands as a shell on each agent it targets, so
/// pointing one at an agent is code execution on that host. An agent the
/// caller cannot see is reported exactly like one that does not exist, so
/// the check cannot be used to discover private agent ids.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] for an agent that does not exist or that
/// the caller cannot see.
pub async fn check_agents_visible(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    agent_ids: impl IntoIterator<Item = i64>,
) -> Result<(), ApiError> {
    let visible = super::read_scope::visible_agents(pool, auth.user_id).await?;
    for agent_id in agent_ids {
        db::get_agent_by_id(pool, agent_id).await?;
        if !visible.allows(agent_id) {
            return Err(ApiError::NotFound(format!(
                "agent id '{agent_id}' not found"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{audit_entries, audit_events, build_test_state, insert_auth_user};

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn granting_and_changing_a_repo_permission_records_before_and_after(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"permissions-audit-test-key");
        let admin = insert_auth_user(&pool, "perm-admin").await;
        let grantee = insert_auth_user(&pool, "grantee").await;
        let repo = db::insert_repo(
            &pool,
            &db::InsertRepoParams {
                name: "audited-repo",
                repo_path: "/backups/audited",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"unused",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        let grant = |can_extract: bool| UpsertPermissionRequest {
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract,
            can_delete: false,
        };

        for can_extract in [false, true] {
            let Json(_) = upsert(
                State(state.clone()),
                RequireAdmin(admin.clone()),
                ClientIp::default(),
                Path((repo.id, grantee.user_id)),
                ApiJson(grant(can_extract)),
            )
            .await
            .unwrap();
        }

        let entries = audit_entries(&pool).await;
        let events: Vec<_> = entries.iter().map(|entry| entry.event.clone()).collect();
        assert_eq!(
            events,
            [
                AuditEvent::SetRepoPermission {
                    username: "grantee".to_owned(),
                    before: vec![RepoPermission::View],
                    after: vec![RepoPermission::View, RepoPermission::Extract],
                },
                AuditEvent::SetRepoPermission {
                    username: "grantee".to_owned(),
                    before: vec![],
                    after: vec![RepoPermission::View],
                },
            ]
        );
        assert!(entries.iter().all(|entry| {
            entry.target_type.as_deref() == Some("repo") && entry.target_id == Some(repo.id)
        }));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn granting_a_missing_user_a_permission_fails_before_anything_is_written(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"permissions-audit-test-key");
        let admin = insert_auth_user(&pool, "perm-admin").await;
        let repo = db::insert_repo(
            &pool,
            &db::InsertRepoParams {
                name: "unwritten-repo",
                repo_path: "/backups/unwritten",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"unused",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap();
        let missing_user_id = 987_654;

        let result = upsert(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path((repo.id, missing_user_id)),
            ApiJson(UpsertPermissionRequest {
                can_view: true,
                can_backup: false,
                can_modify_schedules: false,
                can_extract: false,
                can_delete: false,
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::NotFound(_))));
        assert!(
            db::get_repo_permission(&pool, missing_user_id, repo.id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
    }
}
