// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};

use crate::{AppState, db};

pub async fn ui_ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(session_id) = extract_session_from_headers(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if db::get_session(&state.pool, &session_id).await.is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(|socket| handle_ui_socket(socket, state))
        .into_response()
}

fn extract_session_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix("session=")
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

async fn handle_ui_socket(socket: WebSocket, state: AppState) {
    let mut rx = state.ui_broadcast.subscribe();
    let (mut sink, mut stream) = socket.split();

    for (repo_id, snap) in state.ui_broadcast.current_import_snapshots() {
        let event = shared::protocol::ServerToUi::ImportProgress {
            repo_id,
            progress: snap.progress,
            total: snap.total,
            message: snap.message,
        };
        if let Ok(json) = serde_json::to_string(&event)
            && sink.send(Message::Text(json.into())).await.is_err()
        {
            return;
        }
    }

    for snap in state.ui_broadcast.current_active_backups() {
        let started = shared::protocol::ServerToUi::BackupStarted {
            hostname: snap.hostname.clone(),
            target_name: snap.target_name.clone(),
            archive_name: snap.archive_name.clone(),
            schedule_id: snap.schedule_id,
        };
        if let Ok(json) = serde_json::to_string(&started)
            && sink.send(Message::Text(json.into())).await.is_err()
        {
            return;
        }
        if let Some(line) = snap.progress_line {
            let progress = shared::protocol::ServerToUi::BackupLog {
                hostname: snap.hostname,
                schedule_id: snap.schedule_id,
                repo_id: snap.repo_id,
                line,
            };
            if let Ok(json) = serde_json::to_string(&progress)
                && sink.send(Message::Text(json.into())).await.is_err()
            {
                return;
            }
        }
    }

    let recv_task = async {
        while let Some(Ok(msg)) = stream.next().await {
            if matches!(msg, Message::Close(_)) {
                return;
            }
        }
    };

    let send_task = async {
        while let Ok(event) = rx.recv().await {
            let Ok(json) = serde_json::to_string(&event) else {
                continue;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
    };

    tokio::select! {
        () = send_task => {}
        () = recv_task => {}
    }
}
