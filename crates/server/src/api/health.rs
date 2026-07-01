// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, http::StatusCode};
use shared::responses::HealthCheckResponse;

#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    operation_id = "healthCheck",
    summary = "Server health check",
    responses(
        (status = 200, description = "Server is healthy", body = HealthCheckResponse),
    )
)]
pub async fn health() -> (StatusCode, Json<HealthCheckResponse>) {
    (
        StatusCode::OK,
        Json(HealthCheckResponse {
            status: "ok".to_string(),
        }),
    )
}
