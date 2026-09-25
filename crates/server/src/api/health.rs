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
    responses(
        (status = 200, description = "Server is healthy", body = HealthCheckResponse),
    )
)]
/// Server health check
///
/// Server health check endpoint.
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthCheckResponse>) {
    // Read every source unconditionally rather than short-circuiting with `||`:
    // the answer is the same, but which reads run no longer depends on what
    // happens to be in flight at the moment the request lands.
    let repo_ops_active = state.repo_op_tracker.any_active().await;
    let deliveries_active = state.notification_service.in_flight_deliveries() > 0;
    let background_tasks_active = state.background_task_tracker.any_active();
    let background_ops_in_flight = repo_ops_active || deliveries_active || background_tasks_active;

    (
        StatusCode::OK,
        Json(HealthCheckResponse {
            status: "ok".to_string(),
            background_ops_in_flight,
        }),
    )
}
