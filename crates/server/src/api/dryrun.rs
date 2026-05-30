// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use shared::{protocol::ServerToAgent, types::RepoId};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::auth::AuthUser;
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DryRunRequest {
    pub schedule_id: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DryRunFileEntry {
    pub path: String,
    pub size: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DryRunResponse {
    pub files: Vec<DryRunFileEntry>,
    pub total_size: i64,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/dry-run",
    tag = "Repos",
    operation_id = "dryRun",
    summary = "Preview what a backup schedule would include without creating an archive",
    params(("repo_id" = i64, Path, description = "Repository ID")),
    request_body = DryRunRequest,
    responses(
        (status = 200, description = "Dry-run result", body = DryRunResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Agent operation failed"),
        (status = 503, description = "Agent offline or timed out"),
    )
)]
pub async fn dry_run(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(repo_id): Path<i64>,
    ApiJson(req): ApiJson<DryRunRequest>,
) -> Result<Json<DryRunResponse>, ApiError> {
    let schedule = db::get_schedule_by_id(&state.pool, req.schedule_id).await?;

    if schedule.repo_id != repo_id {
        return Err(ApiError::NotFound(format!(
            "schedule {} not found for repo {repo_id}",
            req.schedule_id
        )));
    }

    let hostname = db::get_client_hostname_for_schedule(&state.pool, req.schedule_id).await?;

    if !state.registry.is_connected(&hostname).await {
        return Err(ApiError::ServiceUnavailable("agent is offline".to_owned()));
    }

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();

    state
        .pending_dryruns
        .lock()
        .await
        .insert(request_id.clone(), tx);

    let msg = ServerToAgent::DryRun {
        request_id: request_id.clone(),
        repo_id: RepoId(repo_id),
        schedule_id: req.schedule_id,
    };

    if state.registry.send_to(&hostname, msg).await.is_err() {
        state.pending_dryruns.lock().await.remove(&request_id);
        return Err(ApiError::ServiceUnavailable("agent is offline".to_owned()));
    }

    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok((_files, _total_size, Some(error)))) => {
            Err(ApiError::Internal(format!("dry-run failed: {error}")))
        }
        Ok(Ok((files, total_size, None))) => Ok(Json(DryRunResponse {
            files: files
                .into_iter()
                .map(|f| DryRunFileEntry {
                    path: f.path,
                    size: f.size,
                })
                .collect(),
            total_size,
        })),
        Ok(Err(_)) => Err(ApiError::Internal(
            "dry-run response channel closed unexpectedly".to_owned(),
        )),
        Err(_) => {
            state.pending_dryruns.lock().await.remove(&request_id);
            Err(ApiError::ServiceUnavailable(
                "dry-run timed out after 30 seconds".to_owned(),
            ))
        }
    }
}
