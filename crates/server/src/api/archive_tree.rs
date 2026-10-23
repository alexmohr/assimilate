// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Whole-subtree reads of an archive's content index: how many files and
//! bytes each of several paths holds, for the archive browser's folder rows
//! and its selection.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
};
use serde::Deserialize;
use shared::{responses::ArchiveTreeTotalsResponse, types::IndexStatus};
use utoipa::ToSchema;

use super::{archives::validate_path, auth::AuthUser, permissions::check_repo_permission};
use crate::{AppState, archive_index, error::ApiError};

/// Most paths one totals request may ask about.
pub const MAX_TOTALS_PATHS: usize = 1000;

/// Request payload for path totals.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TreeTotalsRequest {
    /// Archive paths to add up, as borg stores them (no leading `/`).
    pub paths: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tree-totals",
    tag = "Archives",
    operation_id = "archiveTreeTotals",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
    ),
    request_body = TreeTotalsRequest,
    responses(
        (status = 200, description = "Totals per path", body = ArchiveTreeTotalsResponse),
        (status = 400, description = "Invalid, empty or too many paths"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Count the files and bytes at or under each of several archive paths.
///
/// Reads the index only, never borg: an archive whose index is not `done`
/// yet answers with its index status and no totals.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: a path is invalid, or there are none or too many
/// - [`ApiError::Database`]: a query fails
pub async fn archive_tree_totals(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Json(body): Json<TreeTotalsRequest>,
) -> Result<Json<ArchiveTreeTotalsResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    if body.paths.is_empty() {
        return Err(ApiError::BadRequest(
            "paths array must not be empty".to_owned(),
        ));
    }
    if body.paths.len() > MAX_TOTALS_PATHS {
        return Err(ApiError::BadRequest(format!(
            "at most {MAX_TOTALS_PATHS} paths per request"
        )));
    }
    let paths: Vec<String> = body
        .paths
        .iter()
        .map(|path| {
            validate_path(path)?;
            Ok(path.trim_end_matches('/').to_owned())
        })
        .collect::<Result<_, ApiError>>()?;

    let status = archive_index::read_index_status_for_use(&state.pool, repo_id, &archive_name)
        .await?
        .unwrap_or(IndexStatus::Pending);
    if status != IndexStatus::Done {
        return Ok(Json(ArchiveTreeTotalsResponse {
            index_status: status,
            totals: Vec::new(),
        }));
    }

    let totals =
        archive_index::tree::path_totals(&state.pool, repo_id, &archive_name, &paths).await?;
    Ok(Json(ArchiveTreeTotalsResponse {
        index_status: status,
        totals,
    }))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::{
        archive_index::tree::tests::{demo_tree, index_archive, mark_indexed},
        db,
        test_support::{build_test_state, insert_auth_user},
    };

    /// A user who may view a repository whose `daily-1` archive has index
    /// rows but no finished index job, so its index reads as pending.
    async fn viewer_of_pending_index(pool: &PgPool) -> (AppState, AuthUser, i64) {
        let repo_id = index_archive(pool, demo_tree()).await;
        let viewer = insert_auth_user(pool, "tree-viewer").await;
        db::upsert_repo_permission(
            pool,
            &db::UpsertRepoPermissionParams {
                user_id: viewer.user_id,
                repo_id,
                can_view: true,
                can_backup: false,
                can_modify_schedules: false,
                can_extract: false,
                can_delete: false,
            },
        )
        .await
        .unwrap();
        let state = build_test_state(pool.clone(), b"archive-tree-test-key");
        (state, viewer, repo_id)
    }

    async fn totals(
        pool: &PgPool,
        paths: Vec<String>,
    ) -> Result<ArchiveTreeTotalsResponse, ApiError> {
        let (state, viewer, repo_id) = viewer_of_pending_index(pool).await;
        archive_tree_totals(
            State(state),
            viewer,
            AxumPath((repo_id, "daily-1".to_owned())),
            Json(TreeTotalsRequest { paths }),
        )
        .await
        .map(|Json(response)| response)
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_of_an_archive_not_yet_indexed_report_the_status_only(pool: PgPool) {
        let response = totals(&pool, vec!["etc/".to_owned()]).await.unwrap();

        assert_eq!(response.index_status, IndexStatus::Pending);
        assert_eq!(response.totals, Vec::new());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_need_at_least_one_path(pool: PgPool) {
        let refused = totals(&pool, Vec::new()).await;

        assert!(
            matches!(
                &refused,
                Err(ApiError::BadRequest(message)) if message.contains("must not be empty")
            ),
            "{refused:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_refuse_more_paths_than_one_request_may_ask_about(pool: PgPool) {
        let too_many = (0..=MAX_TOTALS_PATHS).map(|i| format!("etc/{i}")).collect();

        let refused = totals(&pool, too_many).await;

        assert!(
            matches!(
                &refused,
                Err(ApiError::BadRequest(message)) if message.contains("at most 1000 paths")
            ),
            "{refused:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_of_an_indexed_archive_add_up_each_path(pool: PgPool) {
        let (state, viewer, repo_id) = viewer_of_pending_index(&pool).await;
        mark_indexed(&pool, repo_id).await;

        let Json(response) = archive_tree_totals(
            State(state),
            viewer,
            AxumPath((repo_id, "daily-1".to_owned())),
            Json(TreeTotalsRequest {
                paths: vec!["etc/".to_owned(), "var/www/html/app.js".to_owned()],
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.index_status, IndexStatus::Done);
        let summary: Vec<(&str, i64, i64)> = response
            .totals
            .iter()
            .map(|t| (t.path.as_str(), t.files, t.bytes))
            .collect();
        assert_eq!(summary, [("etc", 3, 147), ("var/www/html/app.js", 1, 2000)]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_are_refused_without_view_permission(pool: PgPool) {
        let repo_id = index_archive(&pool, demo_tree()).await;
        let stranger = insert_auth_user(&pool, "tree-stranger").await;
        let state = build_test_state(pool.clone(), b"archive-tree-test-key");

        let refused = archive_tree_totals(
            State(state),
            stranger,
            AxumPath((repo_id, "daily-1".to_owned())),
            Json(TreeTotalsRequest {
                paths: vec!["etc".to_owned()],
            }),
        )
        .await;

        assert!(
            matches!(refused, Err(ApiError::Forbidden(_))),
            "{refused:?}"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn totals_refuse_a_path_that_climbs_out_of_the_archive(pool: PgPool) {
        let refused = totals(&pool, vec!["etc/../../root".to_owned()]).await;

        assert!(
            matches!(refused, Err(ApiError::BadRequest(_))),
            "{refused:?}"
        );
    }
}
