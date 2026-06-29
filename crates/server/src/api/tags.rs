// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::auth::{AuthUser, RequireAdmin, Role};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TagScopeQuery {
    pub scope: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTagRequest {
    pub name: String,
    pub color: Option<String>,
    pub scope: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetTagsRequest {
    pub tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ArchiveTagRequest {
    pub tag: String,
}

fn normalize_tag(tag: String) -> Result<String, ApiError> {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        return Err(ApiError::BadRequest("tag must not be empty".to_string()));
    }
    if tag.len() > 100 {
        return Err(ApiError::BadRequest(
            "tag must not exceed 100 characters".to_string(),
        ));
    }
    Ok(tag)
}

async fn ensure_manage_tags(state: &AppState, auth: &AuthUser) -> Result<(), ApiError> {
    if auth.role == Role::Admin {
        return Ok(());
    }

    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if effective.can_manage_tags {
        return Ok(());
    }

    Err(ApiError::Forbidden(
        "insufficient tag management permission".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/tags",
    tag = "Tags",
    operation_id = "listTags",
    summary = "List tags by scope",
    params(("scope" = String, Query, description = "Tag scope (e.g. 'repo' or 'agent')")),
    responses(
        (status = 200, description = "List of tags", body = Vec<crate::db::TagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Query(query): Query<TagScopeQuery>,
) -> Result<Json<Vec<db::TagRow>>, ApiError> {
    let tags = db::list_tags(&state.pool, &query.scope).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    post,
    path = "/api/tags",
    tag = "Tags",
    operation_id = "createTag",
    summary = "Create a new tag",
    request_body = CreateTagRequest,
    responses(
        (status = 201, description = "Created", body = crate::db::TagRow),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn create_tag(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateTagRequest>,
) -> Result<(StatusCode, Json<db::TagRow>), ApiError> {
    let color = req.color.as_deref().unwrap_or("#6b7280");
    let tag = db::insert_tag(&state.pool, &req.name, color, &req.scope).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

#[utoipa::path(
    delete,
    path = "/api/tags/{id}",
    tag = "Tags",
    operation_id = "deleteTag",
    summary = "Delete a tag",
    params(("id" = i64, Path, description = "Tag ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_tag(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::delete_tag(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/repos/{repo_id}/tags",
    tag = "Tags",
    operation_id = "setRepoTags",
    summary = "Set tags for a repository",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = SetTagsRequest,
    responses(
        (status = 204, description = "Tags set"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn set_repo_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<SetTagsRequest>,
) -> Result<StatusCode, ApiError> {
    db::set_repo_tags(&state.pool, repo_id, &req.tag_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/tags",
    tag = "Tags",
    operation_id = "getRepoTags",
    summary = "Get tags for a repository",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "List of tags", body = Vec<crate::db::TagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn get_repo_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<db::TagRow>>, ApiError> {
    let tags = db::list_tags_for_repo(&state.pool, repo_id).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/tags",
    tag = "Tags",
    operation_id = "setHostTags",
    summary = "Set tags for a host",
    params(("hostname" = String, Path, description = "Client hostname")),
    request_body = SetTagsRequest,
    responses(
        (status = 204, description = "Tags set"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn set_agent_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<SetTagsRequest>,
) -> Result<StatusCode, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    db::set_agent_tags(&state.pool, agent.id, &req.tag_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/agents/{hostname}/tags",
    tag = "Tags",
    operation_id = "getHostTags",
    summary = "Get tags for a host",
    params(("hostname" = String, Path, description = "Client hostname")),
    responses(
        (status = 200, description = "List of tags", body = Vec<crate::db::TagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn get_agent_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<db::TagRow>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let tags = db::list_tags_for_agent(&state.pool, agent.id).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/agent-tags",
    tag = "Tags",
    operation_id = "listHostTagAssociations",
    summary = "List all host-tag associations",
    responses(
        (status = 200, description = "Host-tag associations",
            body = Vec<crate::db::AgentTagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_agent_tag_associations(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::AgentTagRow>>, ApiError> {
    let tags = db::list_all_agent_tags(&state.pool).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/repo-tags",
    tag = "Tags",
    operation_id = "listRepoTagAssociations",
    summary = "List all repo-tag associations",
    responses(
        (status = 200, description = "Repo-tag associations",
            body = Vec<crate::db::RepoTagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_repo_tag_associations(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::RepoTagRow>>, ApiError> {
    let tags = db::list_all_repo_tags(&state.pool).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags",
    tag = "Archives",
    operation_id = "listArchiveTags",
    summary = "List tags for an archive",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "List of archive tags", body = Vec<db::tags::ArchiveTag>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn list_archive_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name)): Path<(i64, String)>,
) -> Result<Json<Vec<db::tags::ArchiveTag>>, ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tags = db::tags::list_tags_for_archive(&state.pool, repo_id, &archive_name).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags",
    tag = "Archives",
    operation_id = "addArchiveTag",
    summary = "Add a tag to an archive",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = ArchiveTagRequest,
    responses(
        (status = 201, description = "Created", body = db::tags::ArchiveTag),
        (status = 400, description = "Invalid tag"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "Tag already exists"),
    )
)]
pub async fn add_archive_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name)): Path<(i64, String)>,
    ApiJson(req): ApiJson<ArchiveTagRequest>,
) -> Result<(StatusCode, Json<db::tags::ArchiveTag>), ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tag = normalize_tag(req.tag)?;

    let created = db::tags::add_tag(
        &state.pool,
        repo_id,
        &archive_name,
        &tag,
        Some(auth.user_id),
    )
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            ApiError::Conflict("tag already exists for archive".to_string())
        }
        other => ApiError::Database(other),
    })?;

    Ok((StatusCode::CREATED, Json(created)))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags/{tag}",
    tag = "Archives",
    operation_id = "removeArchiveTag",
    summary = "Remove a tag from an archive",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        ("tag" = String, Path, description = "Tag"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn remove_archive_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name, tag)): Path<(i64, String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tag = normalize_tag(tag)?;

    let removed = db::tags::remove_tag(&state.pool, repo_id, &archive_name, &tag).await?;
    if !removed {
        return Err(ApiError::NotFound("archive tag not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}
