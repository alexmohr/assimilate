// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The archive browser's reads of an archive's content index: a search
//! across the whole archive, and how many files and bytes each of several
//! paths holds, for its folder rows and its selection.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};
use chrono::NaiveDate;
use serde::Deserialize;
use shared::{
    responses::{ArchiveTreeSearchResponse, ArchiveTreeTotalsResponse},
    types::IndexStatus,
};
use utoipa::{IntoParams, ToSchema};

use super::{archives::validate_path, auth::AuthUser, permissions::check_repo_permission};
use crate::{
    AppState,
    archive_index::{
        self,
        tree::{EntryKind, TreeSearch},
    },
    error::ApiError,
};

/// Default number of matches a tree search returns.
const DEFAULT_SEARCH_LIMIT: usize = 200;
/// Most matches a tree search returns.
const MAX_SEARCH_LIMIT: usize = 1000;
/// Most paths one totals request may ask about.
pub const MAX_TOTALS_PATHS: usize = 1000;
/// Longest search text accepted, in characters.
const MAX_QUERY_CHARS: usize = 256;

/// Query parameters for searching an archive's content index.
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct TreeSearchQuery {
    /// Text the entry's path must contain, ignoring case.
    pub q: Option<String>,
    /// Keep only files or only directories.
    pub kind: Option<EntryKind>,
    /// Smallest size kept, in bytes.
    pub min_size: Option<i64>,
    /// Largest size kept, in bytes.
    pub max_size: Option<i64>,
    /// Earliest modification day kept (`YYYY-MM-DD`).
    pub modified_from: Option<NaiveDate>,
    /// Latest modification day kept (`YYYY-MM-DD`).
    pub modified_to: Option<NaiveDate>,
    /// Most matches to return (default 200, at most 1000).
    pub limit: Option<usize>,
}

impl TryFrom<TreeSearchQuery> for TreeSearch {
    type Error = ApiError;

    fn try_from(query: TreeSearchQuery) -> Result<Self, Self::Error> {
        let needle = query
            .q
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        if needle
            .as_deref()
            .is_some_and(|q| q.chars().count() > MAX_QUERY_CHARS)
        {
            return Err(ApiError::BadRequest(format!(
                "search text must be at most {MAX_QUERY_CHARS} characters"
            )));
        }
        if needle.as_deref().is_some_and(|q| q.contains('\0')) {
            return Err(ApiError::BadRequest(
                "null bytes not allowed in search text".to_owned(),
            ));
        }
        if query.min_size.is_some_and(i64::is_negative)
            || query.max_size.is_some_and(i64::is_negative)
        {
            return Err(ApiError::BadRequest(
                "sizes must not be negative".to_owned(),
            ));
        }
        if let (Some(min), Some(max)) = (query.min_size, query.max_size)
            && min > max
        {
            return Err(ApiError::BadRequest(
                "min_size must not exceed max_size".to_owned(),
            ));
        }
        if let (Some(from), Some(to)) = (query.modified_from, query.modified_to)
            && from > to
        {
            return Err(ApiError::BadRequest(
                "modified_from must not be after modified_to".to_owned(),
            ));
        }
        let search = Self {
            needle,
            kind: query.kind,
            min_size: query.min_size,
            max_size: query.max_size,
            modified_from: query.modified_from,
            modified_to: query.modified_to,
        };
        if search.is_unbounded() {
            return Err(ApiError::BadRequest(
                "give search text or at least one filter".to_owned(),
            ));
        }
        Ok(search)
    }
}

#[utoipa::path(
    get,
    path = "/api/repos/{repo_id}/archives/{archive_name}/tree-search",
    tag = "Archives",
    operation_id = "searchArchiveTree",
    params(
        ("repo_id" = i64, Path, description = "Repository ID"),
        ("archive_name" = String, Path, description = "Archive name"),
        TreeSearchQuery,
    ),
    responses(
        (status = 200, description = "Matching entries", body = ArchiveTreeSearchResponse),
        (status = 400, description = "No search text or filter, or an invalid one"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
/// Search an archive's content index for entries anywhere in the archive.
///
/// Reads the index only, never borg: an archive whose index is not `done`
/// yet answers with its index status and no entries.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: neither search text nor a filter was given,
///   or one is invalid
/// - [`ApiError::Database`]: a query fails
pub async fn search_archive_tree(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath((repo_id, archive_name)): AxumPath<(i64, String)>,
    Query(query): Query<TreeSearchQuery>,
) -> Result<Json<ArchiveTreeSearchResponse>, ApiError> {
    check_repo_permission(&state.pool, &auth, repo_id, |p| p.can_view).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let search = TreeSearch::try_from(query)?;

    let status = archive_index::read_index_status_for_use(&state.pool, repo_id, &archive_name)
        .await?
        .unwrap_or(IndexStatus::Pending);
    if status != IndexStatus::Done {
        return Ok(Json(ArchiveTreeSearchResponse {
            index_status: status,
            entries: Vec::new(),
            truncated: false,
        }));
    }

    let (entries, truncated) =
        archive_index::tree::search_tree(&state.pool, repo_id, &archive_name, &search, limit)
            .await?;
    Ok(Json(ArchiveTreeSearchResponse {
        index_status: status,
        entries,
        truncated,
    }))
}

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

    #[test]
    fn a_search_needs_text_or_a_filter() {
        assert!(matches!(
            TreeSearch::try_from(TreeSearchQuery::default()),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            TreeSearch::try_from(TreeSearchQuery {
                q: Some("   ".to_owned()),
                ..TreeSearchQuery::default()
            }),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(
            TreeSearch::try_from(TreeSearchQuery {
                kind: Some(EntryKind::Dir),
                ..TreeSearchQuery::default()
            }),
            Ok(TreeSearch {
                kind: Some(EntryKind::Dir),
                ..
            })
        ));
    }

    #[test]
    fn search_text_is_trimmed_and_lower_cased() {
        let search = TreeSearch::try_from(TreeSearchQuery {
            q: Some("  Nginx.CONF ".to_owned()),
            ..TreeSearchQuery::default()
        })
        .unwrap();

        assert_eq!(search.needle.as_deref(), Some("nginx.conf"));
    }

    #[test]
    fn contradictory_or_invalid_filters_are_refused() {
        let refused = [
            TreeSearchQuery {
                min_size: Some(-1),
                ..TreeSearchQuery::default()
            },
            TreeSearchQuery {
                min_size: Some(10),
                max_size: Some(5),
                ..TreeSearchQuery::default()
            },
            TreeSearchQuery {
                modified_from: NaiveDate::from_ymd_opt(2026, 2, 1),
                modified_to: NaiveDate::from_ymd_opt(2026, 1, 1),
                ..TreeSearchQuery::default()
            },
            TreeSearchQuery {
                q: Some("x".repeat(300)),
                ..TreeSearchQuery::default()
            },
            TreeSearchQuery {
                q: Some("a\0b".to_owned()),
                ..TreeSearchQuery::default()
            },
        ];

        assert!(
            refused
                .into_iter()
                .all(|query| matches!(TreeSearch::try_from(query), Err(ApiError::BadRequest(_))))
        );
    }

    #[test]
    fn search_filters_deserialize_from_the_query_string() {
        let query: TreeSearchQuery = serde_json::from_value(serde_json::json!({
            "q": "conf",
            "kind": "file",
            "min_size": 1,
            "modified_from": "2026-01-01",
        }))
        .unwrap();

        assert_eq!(query.kind, Some(EntryKind::File));
        assert_eq!(query.modified_from, NaiveDate::from_ymd_opt(2026, 1, 1));
    }
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
    async fn a_search_of_an_archive_not_yet_indexed_reports_the_status_only(pool: PgPool) {
        let (state, viewer, repo_id) = viewer_of_pending_index(&pool).await;

        let Json(response) = search_archive_tree(
            State(state),
            viewer,
            AxumPath((repo_id, "daily-1".to_owned())),
            Query(TreeSearchQuery {
                q: Some("conf".to_owned()),
                ..TreeSearchQuery::default()
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.index_status, IndexStatus::Pending);
        assert!(
            matches!(response.entries.as_slice(), []),
            "{:?}",
            response.entries
        );
        assert!(!response.truncated);
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

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_search_of_an_indexed_archive_returns_its_matches(pool: PgPool) {
        let (state, viewer, repo_id) = viewer_of_pending_index(&pool).await;
        mark_indexed(&pool, repo_id).await;

        let Json(response) = search_archive_tree(
            State(state),
            viewer,
            AxumPath((repo_id, "daily-1".to_owned())),
            Query(TreeSearchQuery {
                q: Some("NGINX".to_owned()),
                kind: Some(EntryKind::File),
                limit: Some(1),
                ..TreeSearchQuery::default()
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.index_status, IndexStatus::Done);
        assert_eq!(response.entries.len(), 1);
        assert!(response.entries.iter().all(|e| e.path.contains("nginx")));
        assert!(response.truncated);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_search_is_refused_without_view_permission(pool: PgPool) {
        let repo_id = index_archive(&pool, demo_tree()).await;
        let stranger = insert_auth_user(&pool, "search-stranger").await;
        let state = build_test_state(pool.clone(), b"archive-tree-test-key");

        let refused = search_archive_tree(
            State(state),
            stranger,
            AxumPath((repo_id, "daily-1".to_owned())),
            Query(TreeSearchQuery {
                q: Some("conf".to_owned()),
                ..TreeSearchQuery::default()
            }),
        )
        .await;

        assert!(
            matches!(refused, Err(ApiError::Forbidden(_))),
            "{refused:?}"
        );
    }
}
