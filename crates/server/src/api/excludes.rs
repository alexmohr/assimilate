// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, extract::State};
use serde::Deserialize;
use shared::responses::GlobalExcludesResponse;

use super::auth::AuthUser;
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

/// Request payload for setting global exclude patterns.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetGlobalExcludesRequest {
    /// Raw exclude pattern text (one pattern per line).
    pub raw_text: String,
}

#[utoipa::path(
    get,
    path = "/api/excludes",
    tag = "Excludes",
    operation_id = "getExcludes",
    summary = "Get global exclude patterns as raw text",
    responses(
        (status = 200, description = "Global excludes raw text", body = GlobalExcludesResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_excludes(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<GlobalExcludesResponse>, ApiError> {
    let raw_text = db::get_global_excludes_raw(&state.pool).await?;
    Ok(Json(GlobalExcludesResponse { raw_text }))
}

#[utoipa::path(
    put,
    path = "/api/excludes",
    tag = "Excludes",
    operation_id = "setExcludes",
    summary = "Set global exclude patterns from raw text",
    request_body = SetGlobalExcludesRequest,
    responses(
        (status = 200, description = "Updated", body = GlobalExcludesResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn set_excludes(
    State(state): State<AppState>,
    _auth: AuthUser,
    ApiJson(req): ApiJson<SetGlobalExcludesRequest>,
) -> Result<Json<GlobalExcludesResponse>, ApiError> {
    db::set_global_excludes_raw(&state.pool, &req.raw_text).await?;

    super::helpers::push_config_to_all_agents(&state).await;

    Ok(Json(GlobalExcludesResponse {
        raw_text: req.raw_text,
    }))
}
