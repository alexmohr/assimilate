// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// API error types with structured HTTP response mapping.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Resource not found (404).
    #[error("not found: {0}")]
    NotFound(String),
    /// Invalid request (400).
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Unprocessable entity (422).
    #[error("unprocessable entity: {0}")]
    Unprocessable(String),
    /// Authentication required (401).
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// Insufficient permissions (403).
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// Database query error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Cryptographic operation error.
    #[error("crypto error: {0}")]
    Crypto(#[from] shared::crypto::CryptoError),
    /// Password hashing error.
    #[error("bcrypt error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
    /// Rate limited (429).
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    /// Resource conflict (409).
    #[error("conflict: {0}")]
    Conflict(String),
    /// Upstream service error (502).
    #[error("bad gateway: {0}")]
    BadGateway(String),
    /// Service temporarily unavailable (503).
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    /// Unexpected internal error (500).
    #[error("internal error: {0}")]
    Internal(String),
}

/// JSON body extractor that returns `ApiError::BadRequest` on deserialization failure.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    axum::Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                let message = simplify_serde_error(&rejection.body_text());
                Err(ApiError::BadRequest(message))
            }
        }
    }
}

fn simplify_serde_error(msg: &str) -> String {
    if let Some(rest) =
        msg.strip_prefix("Failed to deserialize the JSON body into the target type: ")
    {
        rest.to_string()
    } else {
        msg.to_string()
    }
}

fn generate_error_id() -> String {
    use std::fmt::Write as _;

    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

fn database_error_response(e: &sqlx::Error) -> (StatusCode, String, Option<String>) {
    if let sqlx::Error::RowNotFound = e {
        tracing::debug!("database row not found");
        (
            StatusCode::NOT_FOUND,
            "resource not found".to_string(),
            None,
        )
    } else if let Some(db_err) = e.as_database_error() {
        if db_err.is_unique_violation() {
            tracing::debug!(error = %db_err, "unique constraint violation");
            (
                StatusCode::CONFLICT,
                "resource already exists".to_string(),
                None,
            )
        } else {
            let id = generate_error_id();
            tracing::error!(error_id = %id, "database error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database error".to_string(),
                Some(id),
            )
        }
    } else {
        let id = generate_error_id();
        tracing::error!(error_id = %id, "database error: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "database error".to_string(),
            Some(id),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message, error_id) = match &self {
            Self::NotFound(msg) => {
                tracing::debug!(error = %msg, "not found");
                (StatusCode::NOT_FOUND, msg.clone(), None)
            }
            Self::BadRequest(msg) => {
                tracing::warn!(error = %msg, "bad request");
                (StatusCode::BAD_REQUEST, msg.clone(), None)
            }
            Self::Unprocessable(msg) => {
                tracing::warn!(error = %msg, "unprocessable entity");
                (StatusCode::UNPROCESSABLE_ENTITY, msg.clone(), None)
            }
            Self::Unauthorized(msg) => {
                tracing::warn!(error = %msg, "unauthorized");
                (StatusCode::UNAUTHORIZED, msg.clone(), None)
            }
            Self::Forbidden(msg) => {
                tracing::warn!(error = %msg, "forbidden");
                (StatusCode::FORBIDDEN, msg.clone(), None)
            }
            Self::TooManyRequests(msg) => {
                tracing::warn!(error = %msg, "too many requests");
                (StatusCode::TOO_MANY_REQUESTS, msg.clone(), None)
            }
            Self::Database(e) => database_error_response(e),
            Self::Crypto(e) => {
                let id = generate_error_id();
                tracing::error!(error_id = %id, "crypto error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "encryption error".to_string(),
                    Some(id),
                )
            }
            Self::Bcrypt(e) => {
                let id = generate_error_id();
                tracing::error!(error_id = %id, "bcrypt error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "credential hashing error".to_string(),
                    Some(id),
                )
            }
            Self::Conflict(msg) => {
                tracing::debug!(error = %msg, "conflict");
                (StatusCode::CONFLICT, msg.clone(), None)
            }
            Self::BadGateway(msg) => {
                let id = generate_error_id();
                tracing::error!(error_id = %id, "bad gateway: {msg}");
                (StatusCode::BAD_GATEWAY, msg.clone(), Some(id))
            }
            Self::ServiceUnavailable(msg) => {
                tracing::warn!(error = %msg, "service unavailable");
                (StatusCode::SERVICE_UNAVAILABLE, msg.clone(), None)
            }
            Self::Internal(msg) => {
                let id = generate_error_id();
                tracing::error!(error_id = %id, "internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("internal error: {msg}"),
                    Some(id),
                )
            }
        };

        let body = match error_id {
            Some(id) => json!({ "error": message, "error_id": id }),
            None => json!({ "error": message }),
        };

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use http_body_util::BodyExt;
    use sqlx::error::{DatabaseError, ErrorKind};

    use super::*;

    /// A driver-agnostic database error, so the constraint-violation mapping
    /// can be exercised without a live database. The message names a table
    /// so the tests can check it never reaches the client.
    #[derive(Debug)]
    struct FakeDbError {
        unique_violation: bool,
    }

    const FAKE_DB_MESSAGE: &str = "violates constraint on table secret_table";

    impl std::fmt::Display for FakeDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(FAKE_DB_MESSAGE)
        }
    }

    impl std::error::Error for FakeDbError {}

    impl DatabaseError for FakeDbError {
        fn message(&self) -> &str {
            FAKE_DB_MESSAGE
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> ErrorKind {
            if self.unique_violation {
                ErrorKind::UniqueViolation
            } else {
                ErrorKind::ForeignKeyViolation
            }
        }
    }

    fn fake_db_error(unique_violation: bool) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDbError { unique_violation }))
    }

    async fn status_and_body(err: ApiError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The client-facing `error` message of a response body.
    fn error_text(body: &serde_json::Value) -> &str {
        body.get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap()
    }

    /// An error id is 8 random bytes rendered as 16 lowercase hex digits.
    fn assert_error_id(body: &serde_json::Value) {
        let id = body
            .get("error_id")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert_eq!(id.len(), 16, "unexpected error id {id:?}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "error id {id:?} is not lowercase hex"
        );
    }

    #[test]
    fn simplify_serde_error_strips_only_axums_prefix() {
        assert_eq!(
            simplify_serde_error(
                "Failed to deserialize the JSON body into the target type: missing field `name`"
            ),
            "missing field `name`"
        );
        assert_eq!(
            simplify_serde_error("Failed to parse the request body as JSON: EOF"),
            "Failed to parse the request body as JSON: EOF"
        );
    }

    #[test]
    fn error_ids_are_unique_per_call() {
        assert_ne!(generate_error_id(), generate_error_id());
    }

    #[tokio::test]
    async fn client_errors_echo_their_message_without_an_error_id() {
        let cases = [
            (ApiError::NotFound("a".into()), StatusCode::NOT_FOUND, "a"),
            (
                ApiError::BadRequest("b".into()),
                StatusCode::BAD_REQUEST,
                "b",
            ),
            (
                ApiError::Unprocessable("c".into()),
                StatusCode::UNPROCESSABLE_ENTITY,
                "c",
            ),
            (
                ApiError::Unauthorized("d".into()),
                StatusCode::UNAUTHORIZED,
                "d",
            ),
            (ApiError::Forbidden("e".into()), StatusCode::FORBIDDEN, "e"),
            (
                ApiError::TooManyRequests("f".into()),
                StatusCode::TOO_MANY_REQUESTS,
                "f",
            ),
            (ApiError::Conflict("g".into()), StatusCode::CONFLICT, "g"),
            (
                ApiError::ServiceUnavailable("h".into()),
                StatusCode::SERVICE_UNAVAILABLE,
                "h",
            ),
        ];
        for (err, expected_status, expected_message) in cases {
            let (status, body) = status_and_body(err).await;
            assert_eq!(status, expected_status);
            assert_eq!(body, json!({ "error": expected_message }));
        }
    }

    #[tokio::test]
    async fn bad_gateway_carries_an_error_id() {
        let (status, body) = status_and_body(ApiError::BadGateway("agent gone".into())).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(error_text(&body), "agent gone");
        assert_error_id(&body);
    }

    #[tokio::test]
    async fn internal_errors_are_prefixed_and_carry_an_error_id() {
        let (status, body) = status_and_body(ApiError::Internal("boom".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error_text(&body), "internal error: boom");
        assert_error_id(&body);
    }

    #[tokio::test]
    async fn a_missing_row_is_a_plain_not_found() {
        let (status, body) = status_and_body(ApiError::Database(sqlx::Error::RowNotFound)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "resource not found" }));
    }

    #[tokio::test]
    async fn a_unique_violation_is_a_conflict() {
        let (status, body) = status_and_body(ApiError::Database(fake_db_error(true))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body, json!({ "error": "resource already exists" }));
    }

    #[tokio::test]
    async fn other_database_errors_hide_the_driver_message() {
        for err in [fake_db_error(false), sqlx::Error::PoolTimedOut] {
            let (status, body) = status_and_body(ApiError::Database(err)).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(error_text(&body), "database error");
            assert!(!body.to_string().contains("secret_table"));
            assert_error_id(&body);
        }
    }

    #[tokio::test]
    async fn crypto_errors_hide_the_cause() {
        let (status, body) = status_and_body(ApiError::Crypto(
            shared::crypto::CryptoError::DecryptionFailed,
        ))
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error_text(&body), "encryption error");
        assert!(!body.to_string().contains("decryption failed"));
        assert_error_id(&body);
    }

    #[tokio::test]
    async fn bcrypt_errors_hide_the_cause() {
        let (status, body) =
            status_and_body(ApiError::Bcrypt(bcrypt::BcryptError::CostNotAllowed(99))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error_text(&body), "credential hashing error");
        assert!(!body.to_string().contains("99"));
        assert_error_id(&body);
    }

    #[tokio::test]
    async fn a_body_of_the_wrong_shape_is_a_bad_request_without_axums_prefix() {
        let req = Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name": 1}"#))
            .unwrap();
        let Err(err) = ApiJson::<HashMap<String, String>>::from_request(req, &()).await else {
            unreachable!("a non-string value must be rejected");
        };
        let (status, body) = status_and_body(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let message = error_text(&body);
        assert!(
            message.starts_with("name: invalid type: integer `1`"),
            "{message}"
        );
    }
}
