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
    let repo_ops_active = state.repo_op_tracker.any_active().await;
    let notification_deliveries_in_flight = state.notification_service.in_flight_deliveries();
    let background_tasks_active = state.background_task_tracker.any_active();
    let background_ops_in_flight =
        repo_ops_active || notification_deliveries_in_flight > 0 || background_tasks_active;

    if background_ops_in_flight {
        // Debug, not info/warn: this is expected and frequent under normal
        // operation. Its only purpose is to be visible (RUST_LOG=debug in the
        // demo compose) when e2e coverage teardown's drain wait times out, so
        // a stuck subsystem can be identified from `docker logs` instead of
        // just the single `background_ops_in_flight: true` bit that poll
        // exposes - see the "Wait for background operations to quiesce" step
        // in ci.yml.
        tracing::debug!(
            repo_ops_active,
            notification_deliveries_in_flight,
            background_tasks_active,
            "background ops still in flight"
        );
    }

    (
        StatusCode::OK,
        Json(HealthCheckResponse {
            status: "ok".to_string(),
            background_ops_in_flight,
        }),
    )
}
