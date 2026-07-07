// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, extract::State, http::StatusCode};
use shared::responses::HealthCheckResponse;

use crate::AppState;

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
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthCheckResponse>) {
    let background_ops_in_flight = state.repo_op_tracker.any_active().await
        || state.notification_service.in_flight_deliveries() > 0;

    (
        StatusCode::OK,
        Json(HealthCheckResponse {
            status: "ok".to_string(),
            background_ops_in_flight,
        }),
    )
}
