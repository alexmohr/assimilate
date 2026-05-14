// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::auth::RequireAdmin;
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

#[utoipa::path(
    get,
    path = "/api/tags",
    tag = "Tags",
    operation_id = "listTags",
    summary = "List tags by scope",
    params(("scope" = String, Query, description = "Tag scope (e.g. 'repo' or 'host')")),
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
    path = "/api/clients/{hostname}/tags",
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
pub async fn set_host_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
    ApiJson(req): ApiJson<SetTagsRequest>,
) -> Result<StatusCode, ApiError> {
    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;
    db::set_host_tags(&state.pool, client.id, &req.tag_ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/clients/{hostname}/tags",
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
pub async fn get_host_tags(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(hostname): Path<String>,
) -> Result<Json<Vec<db::TagRow>>, ApiError> {
    let client = db::get_client_by_hostname(&state.pool, &hostname).await?;
    let tags = db::list_tags_for_host(&state.pool, client.id).await?;
    Ok(Json(tags))
}

#[utoipa::path(
    get,
    path = "/api/host-tags",
    tag = "Tags",
    operation_id = "listHostTagAssociations",
    summary = "List all host-tag associations",
    responses(
        (status = 200, description = "Host-tag associations",
            body = Vec<crate::db::HostTagRow>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub async fn list_host_tag_associations(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<db::HostTagRow>>, ApiError> {
    let tags = db::list_all_host_tags(&state.pool).await?;
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
