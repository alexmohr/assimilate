// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use shared::responses::{
    AgentTagEntryResponse, ArchiveTagResponse, RepoTagEntryResponse, TagResponse,
};

use super::auth::{AuthUser, RequireAdmin};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

impl From<db::TagRow> for TagResponse {
    fn from(t: db::TagRow) -> Self {
        Self {
            id: t.id,
            name: t.name,
            color: t.color,
            scope: t.scope,
        }
    }
}

impl From<db::tags::ArchiveTag> for ArchiveTagResponse {
    fn from(a: db::tags::ArchiveTag) -> Self {
        Self {
            id: a.id,
            repo_id: a.repo_id.unwrap_or_default(),
            archive_name: a.archive_name.unwrap_or_default(),
            tag: a.tag,
            created_by: a.created_by,
            created_at: a.created_at,
        }
    }
}

impl From<db::AgentTagRow> for AgentTagEntryResponse {
    fn from(a: db::AgentTagRow) -> Self {
        Self {
            agent_id: a.agent_id,
            tag_name: a.tag_name,
            tag_color: a.tag_color,
        }
    }
}

impl From<db::RepoTagRow> for RepoTagEntryResponse {
    fn from(r: db::RepoTagRow) -> Self {
        Self {
            repo_id: r.repo_id,
            tag_name: r.tag_name,
            tag_color: r.tag_color,
        }
    }
}

/// Query parameter for filtering tags by scope.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TagScopeQuery {
    /// Tag scope (e.g. "repo" or "agent").
    pub scope: String,
}

/// Request payload for creating a new tag.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTagRequest {
    /// Tag name.
    pub name: String,
    /// Tag color (hex, e.g. "#6b7280").
    pub color: Option<String>,
    /// Tag scope ("repo" or "agent").
    pub scope: String,
}

/// Request payload for assigning tags to a resource.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetTagsRequest {
    /// Tag IDs to assign.
    pub tag_ids: Vec<i64>,
}

/// Request payload for adding a tag to an archive.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ArchiveTagRequest {
    /// Tag string to add.
    pub tag: String,
}

fn normalize_tag(tag: &str) -> Result<String, ApiError> {
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
    let effective = db::get_effective_permissions(&state.pool, auth.user_id).await?;
    if effective.can_delete_repo || effective.can_manage_tags {
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
    params(("scope" = String, Query, description = "Tag scope (e.g. 'repo' or 'agent')")),
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List tags by scope.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Query(query): Query<TagScopeQuery>,
) -> Result<Json<Vec<TagResponse>>, ApiError> {
    let tags: Vec<TagResponse> = db::list_tags(&state.pool, &query.scope)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    post,
    path = "/api/tags",
    tag = "Tags",
    operation_id = "createTag",
    request_body = CreateTagRequest,
    responses(
        (status = 201, description = "Created", body = TagResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Create a new tag.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn create_tag(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    ApiJson(req): ApiJson<CreateTagRequest>,
) -> Result<(StatusCode, Json<TagResponse>), ApiError> {
    let color = req.color.as_deref().unwrap_or("#6b7280");
    let tag: TagResponse = db::insert_tag(&state.pool, &req.name, color, &req.scope)
        .await?
        .into();
    Ok((StatusCode::CREATED, Json(tag)))
}

#[utoipa::path(
    delete,
    path = "/api/tags/{id}",
    tag = "Tags",
    operation_id = "deleteTag",
    params(("id" = i64, Path, description = "Tag ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
        (status = 404, description = "Not found"),
    )
)]
/// Delete a tag.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
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
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = SetTagsRequest,
    responses(
        (status = 204, description = "Tags set"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Set tags for a repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
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
    params(("repo_id" = i64, Path, description = "Repository ID")),
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Get tags for a repository.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_repo_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<TagResponse>>, ApiError> {
    let tags: Vec<TagResponse> = db::list_tags_for_repo(&state.pool, repo_id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    put,
    path = "/api/agents/{hostname}/tags",
    tag = "Tags",
    operation_id = "setHostTags",
    params(("hostname" = String, Path, description = "Agent hostname")),
    request_body = SetTagsRequest,
    responses(
        (status = 204, description = "Tags set"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Set tags for a host.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
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
    params(("hostname" = String, Path, description = "Agent hostname")),
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Get tags for a host.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_agent_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<TagResponse>>, ApiError> {
    let agent = db::get_agent_by_hostname(&state.pool, &hostname).await?;
    let tags: Vec<TagResponse> = db::list_tags_for_agent(&state.pool, agent.id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/agent-tags",
    tag = "Tags",
    operation_id = "listHostTagAssociations",
    responses(
        (status = 200, description = "Host-tag associations",
            body = Vec<AgentTagEntryResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List all host-tag associations.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_agent_tag_associations(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<AgentTagEntryResponse>>, ApiError> {
    let tags: Vec<AgentTagEntryResponse> = db::list_all_agent_tags(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/repo-tags",
    tag = "Tags",
    operation_id = "listRepoTagAssociations",
    responses(
        (status = 200, description = "Repo-tag associations",
            body = Vec<RepoTagEntryResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// List all repo-tag associations.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_repo_tag_associations(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<RepoTagEntryResponse>>, ApiError> {
    let tags: Vec<RepoTagEntryResponse> = db::list_all_repo_tags(&state.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags",
    tag = "Archives",
    operation_id = "listArchiveTags",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    responses(
        (status = 200, description = "List of archive tags", body = Vec<ArchiveTagResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// List tags for an archive.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_archive_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name)): Path<(i64, String)>,
) -> Result<Json<Vec<ArchiveTagResponse>>, ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tags: Vec<ArchiveTagResponse> =
        db::tags::list_tags_for_archive(&state.pool, repo_id, &archive_name)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(tags))
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags",
    tag = "Archives",
    operation_id = "addArchiveTag",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = ArchiveTagRequest,
    responses(
        (status = 201, description = "Created", body = ArchiveTagResponse),
        (status = 400, description = "Invalid tag"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "Tag already exists"),
    )
)]
/// Add a tag to an archive.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Conflict`]: the request conflicts with the current state
/// - [`ApiError::Database`]: the database query fails
pub async fn add_archive_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name)): Path<(i64, String)>,
    ApiJson(req): ApiJson<ArchiveTagRequest>,
) -> Result<(StatusCode, Json<ArchiveTagResponse>), ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tag = normalize_tag(&req.tag)?;

    let created: ArchiveTagResponse = db::tags::add_tag(
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
    })?
    .into();

    Ok((StatusCode::CREATED, Json(created)))
}

#[utoipa::path(
    delete,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tags/{tag}",
    tag = "Archives",
    operation_id = "removeArchiveTag",
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
/// Remove a tag from an archive.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn remove_archive_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((repo_id, archive_name, tag)): Path<(i64, String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_manage_tags(&state, &auth).await?;
    let tag = normalize_tag(&tag)?;

    let removed = db::tags::remove_tag(&state.pool, repo_id, &archive_name, &tag).await?;
    if !removed {
        return Err(ApiError::NotFound("archive tag not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}
