// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{Json, http::StatusCode};
use serde_json::{Value, json};

#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    operation_id = "healthCheck",
    summary = "Server health check",
    responses(
        (status = 200, description = "Server is healthy", body = serde_json::Value),
    )
)]
pub async fn health() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}
