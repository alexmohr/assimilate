// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use super::{auth::AuthUser, helpers};
use crate::{
    AppState,
    db::{self, ExcludeGlobalRow},
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateExcludeRequest {
    pub pattern: String,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateExcludeRequest {
    pub pattern: String,
    pub sort_order: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/api/excludes",
    tag = "Excludes",
    operation_id = "listExcludes",
    summary = "List global exclude patterns",
    responses(
        (status = 200, description = "List of exclude patterns", body = Vec<ExcludeGlobalRow>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn list_excludes(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<ExcludeGlobalRow>>, ApiError> {
    let global = db::list_global_excludes(&state.pool).await?;
    Ok(Json(global))
}

#[utoipa::path(
    post,
    path = "/api/excludes",
    tag = "Excludes",
    operation_id = "createExclude",
    summary = "Create a global exclude pattern",
    request_body = CreateExcludeRequest,
    responses(
        (status = 201, description = "Created", body = ExcludeGlobalRow),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn create_exclude(
    State(state): State<AppState>,
    _auth: AuthUser,
    ApiJson(req): ApiJson<CreateExcludeRequest>,
) -> Result<(StatusCode, Json<ExcludeGlobalRow>), ApiError> {
    helpers::validate_non_empty(&req.pattern, "pattern")?;

    let sort_order = req.sort_order.unwrap_or(0);
    let row = db::insert_global_exclude(&state.pool, &req.pattern, sort_order).await?;

    helpers::push_config_to_all_agents(&state).await;

    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    put,
    path = "/api/excludes/{id}",
    tag = "Excludes",
    operation_id = "updateExclude",
    summary = "Update a global exclude pattern",
    params(("id" = i64, Path, description = "Exclude ID")),
    request_body = UpdateExcludeRequest,
    responses(
        (status = 200, description = "Updated", body = ExcludeGlobalRow),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_exclude(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateExcludeRequest>,
) -> Result<Json<ExcludeGlobalRow>, ApiError> {
    helpers::validate_non_empty(&req.pattern, "pattern")?;

    let sort_order = req.sort_order.unwrap_or(0);
    let row = db::update_global_exclude(&state.pool, id, &req.pattern, sort_order).await?;

    helpers::push_config_to_all_agents(&state).await;

    Ok(Json(row))
}

#[utoipa::path(
    delete,
    path = "/api/excludes/{id}",
    tag = "Excludes",
    operation_id = "deleteExclude",
    summary = "Delete a global exclude pattern",
    params(("id" = i64, Path, description = "Exclude ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_exclude(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    db::delete_global_exclude(&state.pool, id).await?;

    helpers::push_config_to_all_agents(&state).await;

    Ok(StatusCode::NO_CONTENT)
}
