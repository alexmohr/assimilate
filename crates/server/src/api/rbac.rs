// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::{
    audit::{AuditEvent, RolePermission},
    responses::{GroupResponse, RoleResponse},
};

use super::{
    audit_trail::{self, Actor, AuditTarget, ClientIp},
    auth::{AuthUser, RequireAdmin},
    helpers::{self, MaxLen},
};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::GroupRow> for GroupResponse {
    fn from(g: db::GroupRow) -> Self {
        Self {
            id: g.id,
            name: g.name,
            description: g.description,
            created_at: g.created_at,
        }
    }
}

impl From<db::RoleRow> for RoleResponse {
    fn from(r: db::RoleRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            created_at: r.created_at,
            can_create_agent: r.can_create_agent,
            can_delete_agent: r.can_delete_agent,
            can_delete_own_agent: r.can_delete_own_agent,
            can_create_repo: r.can_create_repo,
            can_delete_repo: r.can_delete_repo,
            can_delete_own_repo: r.can_delete_own_repo,
            can_create_schedule: r.can_create_schedule,
            can_delete_schedule: r.can_delete_schedule,
            can_delete_own_schedule: r.can_delete_own_schedule,
            can_manage_tags: r.can_manage_tags,
            can_view_all_repos: r.can_view_all_repos,
            can_manage_tunnels: r.can_manage_tunnels,
            can_upgrade_agent: r.can_upgrade_agent,
        }
    }
}

/// Request payload for creating a new group.
#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    /// Group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
}

/// Request payload for updating a group.
#[derive(Debug, Deserialize)]
pub struct UpdateGroupRequest {
    /// Updated group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
}

/// Request payload for setting group membership.
#[derive(Debug, Deserialize)]
pub struct SetGroupMembersRequest {
    /// User IDs to include in the group.
    pub user_ids: Vec<i64>,
}

/// Shared permission fields used in role create/update payloads.
#[derive(Debug, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RolePermissionFields {
    /// Permission to create agents.
    pub can_create_agent: bool,
    /// Permission to delete any agent.
    pub can_delete_agent: bool,
    /// Permission to delete own agents.
    pub can_delete_own_agent: bool,
    /// Permission to create repositories.
    pub can_create_repo: bool,
    /// Permission to delete any repository.
    pub can_delete_repo: bool,
    /// Permission to delete own repositories.
    pub can_delete_own_repo: bool,
    /// Permission to create schedules.
    pub can_create_schedule: bool,
    /// Permission to delete any schedule.
    pub can_delete_schedule: bool,
    /// Permission to delete own schedules.
    pub can_delete_own_schedule: bool,
    /// Permission to manage tags.
    pub can_manage_tags: bool,
    /// Permission to view all repositories.
    pub can_view_all_repos: bool,
    /// Permission to manage tunnels.
    pub can_manage_tunnels: bool,
    /// Permission to upgrade agents.
    pub can_upgrade_agent: bool,
}

/// Holds role name and flattened permission fields for create/update operations.
#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    /// Role name.
    pub name: String,
    /// Role permissions (flattened so they appear at the same JSON level as `name`).
    #[serde(flatten)]
    pub perms: RolePermissionFields,
}

/// Request payload for updating a role: same shape as [`CreateRoleRequest`].
pub type UpdateRoleRequest = CreateRoleRequest;
/// Request body for setting a user's role assignments.
#[derive(Debug, Deserialize)]
pub struct SetUserRolesRequest {
    /// Role IDs to assign to the user.
    pub role_ids: Vec<i64>,
}

/// List all groups (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let groups: Vec<GroupResponse> = db::list_groups(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(groups))
}

/// Create a new group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the group name is empty.
pub async fn create_group(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    ApiJson(req): ApiJson<CreateGroupRequest>,
) -> Result<(StatusCode, Json<GroupResponse>), ApiError> {
    let name = validate_group_fields(&req.name, req.description.as_deref())?;
    let group: GroupResponse = db::insert_group(&state.pool, name, req.description.as_deref())
        .await?
        .into();
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Group(group.id)),
        AuditEvent::CreateGroup {
            name: group.name.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(group)))
}

/// Update a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the updated name is empty.
pub async fn update_group(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateGroupRequest>,
) -> Result<Json<GroupResponse>, ApiError> {
    let name = validate_group_fields(&req.name, req.description.as_deref())?;
    let previous = db::get_group(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("group {id} not found")))?;
    let group: GroupResponse = db::update_group(&state.pool, id, name, req.description.as_deref())
        .await?
        .into();
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Group(id)),
        AuditEvent::UpdateGroup {
            name: group.name.clone(),
            previous_name: previous.name,
        },
    )
    .await;
    Ok(Json(group))
}

/// Delete a group (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn delete_group(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let group = db::get_group(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("group {id} not found")))?;
    db::delete_group(&state.pool, id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Group(id)),
        AuditEvent::DeleteGroup { name: group.name },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List members of a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn list_group_members(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Json<shared::responses::GroupMembersResponse>, ApiError> {
    let group = db::get_group(&state.pool, id).await?;
    if group.is_none() {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    let user_ids = db::list_group_members(&state.pool, id).await?;
    Ok(Json(shared::responses::GroupMembersResponse { user_ids }))
}

/// Set the member list of a group (admin only).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn set_group_members(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<SetGroupMembersRequest>,
) -> Result<StatusCode, ApiError> {
    let Some(group) = db::get_group(&state.pool, id).await? else {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    };
    // Everything the audit entry names is read before the members change, so
    // a failed lookup can never turn a completed change into an error.
    let before = db::list_group_members(&state.pool, id).await?;
    let involved: Vec<i64> = before.iter().chain(&req.user_ids).copied().collect();
    let users = db::list_usernames_by_ids(&state.pool, &involved).await?;
    let usernames = |ids: &[i64]| -> Vec<String> {
        users
            .iter()
            .filter(|(user_id, _)| ids.contains(user_id))
            .map(|(_, username)| username.clone())
            .collect()
    };
    db::set_group_members(&state.pool, id, &req.user_ids).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Group(id)),
        AuditEvent::SetGroupMembers {
            group: group.name,
            before: usernames(&before),
            after: usernames(&req.user_ids),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List all roles (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_roles(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(roles))
}

/// Validate role name and build [`InsertRoleParams`] from a request with [`RolePermissionFields`].
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the role name is empty.
/// Trims a group's name and rejects it if empty, or if it or the
/// description exceeds its [`MaxLen`] cap.
fn validate_group_fields<'a>(
    name: &'a str,
    description: Option<&str>,
) -> Result<&'a str, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "group name must not be empty".to_string(),
        ));
    }
    helpers::validate_max_len(name, "name", MaxLen::Name)?;
    helpers::validate_opt_max_len(description, "description", MaxLen::Description)?;
    Ok(name)
}

fn build_role_params<'a>(
    name: &'a str,
    perms: &'a RolePermissionFields,
) -> Result<db::InsertRoleParams<'a>, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "role name must not be empty".to_string(),
        ));
    }
    helpers::validate_max_len(name, "name", MaxLen::Name)?;
    // Populate all permission fields from the request
    Ok(db::InsertRoleParams {
        name,
        can_create_agent: perms.can_create_agent,
        can_delete_agent: perms.can_delete_agent,
        can_delete_own_agent: perms.can_delete_own_agent,
        can_create_repo: perms.can_create_repo,
        can_delete_repo: perms.can_delete_repo,
        can_delete_own_repo: perms.can_delete_own_repo,
        can_create_schedule: perms.can_create_schedule,
        can_delete_schedule: perms.can_delete_schedule,
        can_delete_own_schedule: perms.can_delete_own_schedule,
        can_manage_tags: perms.can_manage_tags,
        can_view_all_repos: perms.can_view_all_repos,
        can_manage_tunnels: perms.can_manage_tunnels,
        can_upgrade_agent: perms.can_upgrade_agent,
    })
}

/// Create a new role (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the role name is empty.
pub async fn create_role(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    ApiJson(req): ApiJson<CreateRoleRequest>,
) -> Result<(StatusCode, Json<RoleResponse>), ApiError> {
    let params = build_role_params(&req.name, &req.perms)?;
    let role: RoleResponse = db::insert_role(&state.pool, &params).await?.into();
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Role(role.id)),
        AuditEvent::CreateRole {
            name: role.name.clone(),
            permissions: RolePermission::granted_by(&role),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(role)))
}

/// Update a role (admin only).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the updated name is empty.
pub async fn update_role(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateRoleRequest>,
) -> Result<Json<RoleResponse>, ApiError> {
    let params = build_role_params(&req.name, &req.perms)?;
    let previous: RoleResponse = db::get_role(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("role {id} not found")))?
        .into();
    let role: RoleResponse = db::update_role(&state.pool, id, &params).await?.into();
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Role(id)),
        AuditEvent::UpdateRole {
            name: role.name.clone(),
            previous_name: previous.name.clone(),
            before: RolePermission::granted_by(&previous),
            after: RolePermission::granted_by(&role),
        },
    )
    .await;
    Ok(Json(role))
}

const PROTECTED_ROLE_NAMES: &[&str] = &["admin", "operator", "viewer"];

/// Delete a role (admin only). Built-in roles cannot be deleted.
///
/// # Errors
///
/// - [`ApiError::NotFound`] if the role does not exist.
/// - [`ApiError::BadRequest`] if the role is built-in.
pub async fn delete_role(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let role = db::get_role(&state.pool, id).await?;
    let Some(role) = role else {
        return Err(ApiError::NotFound(format!("role {id} not found")));
    };
    if PROTECTED_ROLE_NAMES.contains(&role.name.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "cannot delete built-in role '{}'",
            role.name
        )));
    }
    db::delete_role(&state.pool, id).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::Role(id)),
        AuditEvent::DeleteRole { name: role.name },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List roles assigned to a user (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database query fails.
pub async fn list_user_roles(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<RoleResponse>>, ApiError> {
    let roles: Vec<RoleResponse> = db::list_user_roles(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(roles))
}

async fn user_role_names(pool: &sqlx::PgPool, user_id: i64) -> Result<Vec<String>, ApiError> {
    Ok(db::list_user_roles(pool, user_id)
        .await?
        .into_iter()
        .map(|role| role.name)
        .collect())
}

/// Set roles for a user (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn set_user_roles(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    Path(user_id): Path<i64>,
    ApiJson(req): ApiJson<SetUserRolesRequest>,
) -> Result<StatusCode, ApiError> {
    // Everything the audit entry names is read before the roles change, so a
    // failed lookup can never turn a completed change into an error.
    let user = db::get_user_by_id(&state.pool, user_id).await?;
    let before = user_role_names(&state.pool, user_id).await?;
    let after: Vec<String> = db::list_roles(&state.pool)
        .await?
        .into_iter()
        .filter(|role| req.role_ids.contains(&role.id))
        .map(|role| role.name)
        .collect();
    db::set_user_roles(&state.pool, user_id, &req.role_ids).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        Some(AuditTarget::User(user_id)),
        AuditEvent::SetUserRoles {
            username: user.username,
            before,
            after,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// List groups a user belongs to (admin only).
///
/// # Errors
///
/// Returns an error if the underlying database operation fails.
pub async fn list_user_groups(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let groups: Vec<GroupResponse> = db::list_user_groups(&state.pool, user_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(groups))
}

/// Get effective permissions for a user. Admins see any user; users see only themselves.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller lacks permission.
pub async fn get_effective_permissions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<Json<RoleResponse>, ApiError> {
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if !effective.can_delete_repo && auth.user_id != user_id {
        return Err(ApiError::Forbidden(
            "admin access required or must be own user".to_string(),
        ));
    }
    let perms: RoleResponse = db::get_effective_permissions(&state.pool, user_id)
        .await?
        .into();
    Ok(Json(perms))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{audit_entries, audit_events, build_test_state, insert_auth_user};

    const KEY: &[u8] = b"rbac-audit-test-key";

    fn role_request(name: &str, can_delete_repo: bool) -> CreateRoleRequest {
        CreateRoleRequest {
            name: name.to_owned(),
            perms: RolePermissionFields {
                can_create_agent: false,
                can_delete_agent: false,
                can_delete_own_agent: false,
                can_create_repo: true,
                can_delete_repo,
                can_delete_own_repo: false,
                can_create_schedule: false,
                can_delete_schedule: false,
                can_delete_own_schedule: false,
                can_manage_tags: false,
                can_view_all_repos: false,
                can_manage_tunnels: false,
                can_upgrade_agent: false,
            },
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_role_lifecycle_records_what_it_grants_before_and_after(pool: PgPool) {
        let state = build_test_state(pool.clone(), KEY);
        let admin = insert_auth_user(&pool, "role-admin").await;

        let (_, Json(role)) = create_role(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            ApiJson(role_request("operators", false)),
        )
        .await
        .unwrap();
        let Json(_) = update_role(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            Path(role.id),
            ApiJson(role_request("ops", true)),
        )
        .await
        .unwrap();
        delete_role(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(role.id),
        )
        .await
        .unwrap();

        let entries = audit_entries(&pool).await;
        assert!(entries.iter().all(|entry| {
            entry.target_type.as_deref() == Some("role") && entry.target_id == Some(role.id)
        }));
        let events: Vec<_> = entries.into_iter().map(|entry| entry.event).collect();
        assert_eq!(
            events,
            [
                AuditEvent::DeleteRole {
                    name: "ops".to_owned()
                },
                AuditEvent::UpdateRole {
                    name: "ops".to_owned(),
                    previous_name: "operators".to_owned(),
                    before: vec![RolePermission::CreateRepo],
                    after: vec![RolePermission::CreateRepo, RolePermission::DeleteRepo],
                },
                AuditEvent::CreateRole {
                    name: "operators".to_owned(),
                    permissions: vec![RolePermission::CreateRepo],
                },
            ]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_refused_built_in_role_delete_is_not_audited(pool: PgPool) {
        let state = build_test_state(pool.clone(), KEY);
        let admin = insert_auth_user(&pool, "role-admin").await;
        let admin_role = db::list_roles(&pool)
            .await
            .unwrap()
            .into_iter()
            .find(|role| PROTECTED_ROLE_NAMES.contains(&role.name.as_str()))
            .unwrap();

        let result = delete_role(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(admin_role.id),
        )
        .await;

        assert!(matches!(result, Err(ApiError::BadRequest(_))));
        assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn setting_a_missing_users_roles_fails_before_anything_is_written(pool: PgPool) {
        let state = build_test_state(pool.clone(), KEY);
        let admin = insert_auth_user(&pool, "role-admin").await;

        let result = set_user_roles(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(987_654),
            ApiJson(SetUserRolesRequest { role_ids: vec![] }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::NotFound(_))));
        assert_eq!(audit_events(&pool).await, Vec::<AuditEvent>::new());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn changing_a_users_roles_records_before_and_after(pool: PgPool) {
        let state = build_test_state(pool.clone(), KEY);
        let admin = insert_auth_user(&pool, "role-admin").await;
        let target = insert_auth_user(&pool, "promoted").await;
        let roles = db::list_roles(&pool).await.unwrap();
        let role_ids = |names: &[&str]| -> Vec<i64> {
            roles
                .iter()
                .filter(|role| names.contains(&role.name.as_str()))
                .map(|role| role.id)
                .collect()
        };
        db::set_user_roles(&pool, target.user_id, &role_ids(&["viewer"]))
            .await
            .unwrap();

        set_user_roles(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(target.user_id),
            ApiJson(SetUserRolesRequest {
                role_ids: role_ids(&["admin"]),
            }),
        )
        .await
        .unwrap();

        let entries = audit_entries(&pool).await;
        let [entry] = entries.as_slice() else {
            panic!("expected exactly one audit entry, got {entries:?}");
        };
        assert_eq!(
            entry.event,
            AuditEvent::SetUserRoles {
                username: "promoted".to_owned(),
                before: vec!["viewer".to_owned()],
                after: vec!["admin".to_owned()],
            }
        );
        assert_eq!(entry.target_type.as_deref(), Some("user"));
        assert_eq!(entry.target_id, Some(target.user_id));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_group_lifecycle_and_its_membership_are_audited(pool: PgPool) {
        let state = build_test_state(pool.clone(), KEY);
        let admin = insert_auth_user(&pool, "group-admin").await;
        let alice = insert_auth_user(&pool, "alice").await;
        let bob = insert_auth_user(&pool, "bob").await;

        let (_, Json(group)) = create_group(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            ApiJson(CreateGroupRequest {
                name: "backend".to_owned(),
                description: None,
            }),
        )
        .await
        .unwrap();
        db::set_group_members(&pool, group.id, &[alice.user_id])
            .await
            .unwrap();
        set_group_members(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            Path(group.id),
            ApiJson(SetGroupMembersRequest {
                user_ids: vec![bob.user_id],
            }),
        )
        .await
        .unwrap();
        let Json(_) = update_group(
            State(state.clone()),
            RequireAdmin(admin.clone()),
            ClientIp::default(),
            Path(group.id),
            ApiJson(UpdateGroupRequest {
                name: "backend-team".to_owned(),
                description: Some("API owners".to_owned()),
            }),
        )
        .await
        .unwrap();
        delete_group(
            State(state),
            RequireAdmin(admin),
            ClientIp::default(),
            Path(group.id),
        )
        .await
        .unwrap();

        assert_eq!(
            audit_events(&pool).await,
            [
                AuditEvent::DeleteGroup {
                    name: "backend-team".to_owned()
                },
                AuditEvent::UpdateGroup {
                    name: "backend-team".to_owned(),
                    previous_name: "backend".to_owned(),
                },
                AuditEvent::SetGroupMembers {
                    group: "backend".to_owned(),
                    before: vec!["alice".to_owned()],
                    after: vec!["bob".to_owned()],
                },
                AuditEvent::CreateGroup {
                    name: "backend".to_owned()
                },
            ]
        );
    }

    #[test]
    fn group_name_is_trimmed_and_capped_at_the_name_limit() {
        assert_eq!(validate_group_fields("  ops  ", None).unwrap(), "ops");
        let at_limit = "a".repeat(MaxLen::Name.chars());
        assert!(validate_group_fields(&at_limit, None).is_ok());
        assert!(matches!(
            validate_group_fields(&format!("{at_limit}a"), None),
            Err(ApiError::BadRequest(message)) if message.starts_with("name ")
        ));
        assert!(validate_group_fields("   ", None).is_err());
    }

    #[test]
    fn group_description_is_capped_at_the_description_limit() {
        let at_limit = "a".repeat(MaxLen::Description.chars());
        assert!(validate_group_fields("ops", Some(&at_limit)).is_ok());
        assert!(matches!(
            validate_group_fields("ops", Some(&format!("{at_limit}a"))),
            Err(ApiError::BadRequest(message)) if message.starts_with("description ")
        ));
    }

    #[test]
    fn role_name_is_capped_at_the_name_limit() {
        let perms: RolePermissionFields = serde_json::from_value(serde_json::json!({
            "can_create_agent": false, "can_delete_agent": false, "can_delete_own_agent": false,
            "can_create_repo": false, "can_delete_repo": false, "can_delete_own_repo": false,
            "can_create_schedule": false, "can_delete_schedule": false,
            "can_delete_own_schedule": false, "can_manage_tags": false,
            "can_view_all_repos": false, "can_manage_tunnels": false, "can_upgrade_agent": false,
        }))
        .unwrap();
        let at_limit = "a".repeat(MaxLen::Name.chars());
        assert!(build_role_params(&at_limit, &perms).is_ok());
        assert!(matches!(
            build_role_params(&format!("{at_limit}a"), &perms),
            Err(ApiError::BadRequest(message)) if message.starts_with("name ")
        ));
    }

    #[test]
    fn create_role_request_includes_can_upgrade_agent() {
        let json = serde_json::json!({
            "name": "custom-role",
            "can_create_agent": true,
            "can_delete_agent": false,
            "can_delete_own_agent": true,
            "can_create_repo": false,
            "can_delete_repo": true,
            "can_delete_own_repo": false,
            "can_create_schedule": true,
            "can_delete_schedule": false,
            "can_delete_own_schedule": true,
            "can_manage_tags": false,
            "can_view_all_repos": true,
            "can_manage_tunnels": false,
            "can_upgrade_agent": true
        });
        let req: CreateRoleRequest = serde_json::from_value(json).unwrap();
        assert!(req.perms.can_upgrade_agent);
        assert_eq!(req.name, "custom-role");
        assert!(req.perms.can_create_agent);
    }

    #[test]
    fn update_role_request_includes_can_upgrade_agent() {
        let json = serde_json::json!({
            "name": "updated-role",
            "can_create_agent": false,
            "can_delete_agent": true,
            "can_delete_own_agent": false,
            "can_create_repo": true,
            "can_delete_repo": false,
            "can_delete_own_repo": true,
            "can_create_schedule": false,
            "can_delete_schedule": true,
            "can_delete_own_schedule": false,
            "can_manage_tags": true,
            "can_view_all_repos": false,
            "can_manage_tunnels": true,
            "can_upgrade_agent": true
        });
        let req: UpdateRoleRequest = serde_json::from_value(json).unwrap();
        assert!(req.perms.can_upgrade_agent);
        assert_eq!(req.name, "updated-role");
        assert!(!req.perms.can_create_agent);
    }
}
