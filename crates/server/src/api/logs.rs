// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use shared::responses::LogEntryResponse;

use super::auth::RequireAdmin;
use crate::{AppState, error::ApiError, log_buffer::LogEntry};

impl From<LogEntry> for LogEntryResponse {
    fn from(e: LogEntry) -> Self {
        Self {
            timestamp: e.timestamp,
            level: e.level,
            target: e.target,
            message: e.message,
        }
    }
}

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
        (status = 200, description = "Log entries (newest first)", body = Vec<LogEntryResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
pub fn get_logs(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<LogQuery>,
) -> impl std::future::Future<Output = Result<Json<Vec<LogEntryResponse>>, ApiError>> {
    let limit = query.limit.unwrap_or(200);
    let entries: Vec<LogEntryResponse> = state
        .log_buffer
        .entries(limit, query.level.as_deref(), query.search.as_deref())
        .into_iter()
        .map(Into::into)
        .collect();
    std::future::ready(Ok(Json(entries)))
}
