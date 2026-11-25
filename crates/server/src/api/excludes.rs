// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, extract::State};
use serde::Deserialize;
use shared::{audit::AuditEvent, responses::GlobalExcludesResponse};

use super::{
    audit_trail::{self, Actor, ClientIp},
    auth::{AuthUser, RequireAdmin},
};
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
    responses(
        (status = 200, description = "Global excludes raw text", body = GlobalExcludesResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
/// Get global exclude patterns as raw text.
///
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
    request_body = SetGlobalExcludesRequest,
    responses(
        (status = 200, description = "Updated", body = GlobalExcludesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Set global exclude patterns from raw text.
///
/// The patterns apply to every schedule on every host, so only an admin may
/// change them, and each change is recorded in the audit log.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] if the caller is not an admin,
/// [`ApiError::BadRequest`] if the text is too long, or an error if the
/// underlying operation fails.
pub async fn set_excludes(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    ip: ClientIp,
    ApiJson(req): ApiJson<SetGlobalExcludesRequest>,
) -> Result<Json<GlobalExcludesResponse>, ApiError> {
    super::helpers::validate_max_len(&req.raw_text, "raw_text", super::helpers::MaxLen::Text)?;
    let previous = db::get_global_excludes_raw(&state.pool).await?;
    db::set_global_excludes_raw(&state.pool, &req.raw_text).await?;
    audit_trail::record(
        &state.pool,
        Actor::new(&admin, ip),
        None,
        AuditEvent::SetGlobalExcludes {
            before: previous,
            after: req.raw_text.clone(),
        },
    )
    .await;

    super::helpers::push_config_to_all_agents(&state).await;

    Ok(Json(GlobalExcludesResponse {
        raw_text: req.raw_text,
    }))
}
