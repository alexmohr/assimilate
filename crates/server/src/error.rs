// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unprocessable entity: {0}")]
    Unprocessable(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("crypto error: {0}")]
    Crypto(#[from] shared::crypto::CryptoError),
    #[error("bcrypt error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("bad gateway: {0}")]
    BadGateway(String),
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

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
            Self::Database(e) => {
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
