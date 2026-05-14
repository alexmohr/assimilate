// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use super::auth::AuthUser;
use crate::{AppState, error::ApiError, log_buffer::LogEntry};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LogQuery {
    pub limit: Option<usize>,
    pub level: Option<String>,
    pub search: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/logs",
    tag = "System",
    operation_id = "getServerLogs",
    summary = "Get recent server log entries from the in-memory ring buffer",
    params(
        ("limit" = Option<usize>, Query,
            description = "Max entries to return (default 200)"),
        ("level" = Option<String>, Query,
            description = "Min log level: error, warn, info, debug, trace"),
        ("search" = Option<String>, Query,
            description = "Case-insensitive search in message/target"),
    ),
    responses(
        (status = 200, description = "Log entries (newest first)", body = Vec<LogEntry>),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn get_logs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<LogEntry>>, ApiError> {
    let limit = query.limit.unwrap_or(200);
    let entries = state
        .log_buffer
        .entries(limit, query.level.as_deref(), query.search.as_deref());
    Ok(Json(entries))
}
