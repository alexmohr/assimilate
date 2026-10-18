// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The Prometheus `/metrics` endpoint and its settings.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use shared::responses::MetricsSettingsResponse;

use super::{audit_trail::ClientIp, auth::RequireAdmin};
use crate::{
    AppState,
    api::{auth::bearer_token, tokens::hash_token},
    db,
    error::{ApiError, ApiJson},
    metrics::{
        self,
        exposition::CONTENT_TYPE,
        settings::{MetricsSettings, parse_networks},
    },
};

/// Why a scrape was let in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrapeAccess {
    /// The client's address is inside an allowed network.
    AllowedNetwork,
    /// The request carried a valid monitoring token.
    MonitoringToken,
}

/// Lets a scrape in from an allowed network, or with a monitoring token.
/// A session cookie or a full API token is not enough: `/metrics` has its own
/// credential, so a scraper's configuration never holds one that could change
/// anything.
async fn authorize(
    state: &AppState,
    settings: &MetricsSettings,
    ip: ClientIp,
    headers: &HeaderMap,
) -> Result<ScrapeAccess, ApiError> {
    if ip.0.is_some_and(|ip| settings.allows_without_token(ip)) {
        return Ok(ScrapeAccess::AllowedNetwork);
    }
    let Some(token) = bearer_token(headers) else {
        return Err(ApiError::Unauthorized(
            "a monitoring token is required".to_string(),
        ));
    };
    if db::touch_monitoring_token(&state.pool, &hash_token(token)).await? {
        Ok(ScrapeAccess::MonitoringToken)
    } else {
        Err(ApiError::Unauthorized(
            "invalid monitoring token".to_string(),
        ))
    }
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "Metrics",
    operation_id = "scrapeMetrics",
    responses(
        (
            status = 200,
            description = "Metrics in the Prometheus text exposition format (version 0.0.4)",
            body = String,
            content_type = "text/plain; version=0.0.4; charset=utf-8"
        ),
        (status = 401, description = "No monitoring token, from outside the allowed networks"),
        (status = 404, description = "The metrics endpoint is disabled"),
        (status = 500, description = "Internal server error"),
    )
)]
/// Scrape Prometheus metrics.
///
/// Off until enabled in the system settings. Authenticate with a monitoring
/// API token as `Authorization: Bearer <token>`, or scrape from a network the
/// settings allow without one. Session cookies and full API tokens are not
/// accepted.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] while the endpoint is disabled,
/// [`ApiError::Unauthorized`] without a valid monitoring token from outside
/// the allowed networks, and [`ApiError::Database`] if a query fails.
pub async fn scrape(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let settings = MetricsSettings::load(&state.pool).await?;
    if !settings.enabled {
        return Err(ApiError::NotFound(
            "the metrics endpoint is disabled".to_string(),
        ));
    }
    let access = authorize(&state, &settings, ip, &headers).await?;
    tracing::debug!(?access, "metrics scrape");
    let body = metrics::render(&state).await?;
    Ok((
        [(header::CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE))],
        body.to_string(),
    )
        .into_response())
}

impl From<MetricsSettings> for MetricsSettingsResponse {
    fn from(settings: MetricsSettings) -> Self {
        Self {
            enabled: settings.enabled,
            allowed_networks: settings
                .allowed_networks
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/system/metrics",
    tag = "System",
    operation_id = "getMetricsSettings",
    responses(
        (status = 200, description = "Metrics endpoint settings", body = MetricsSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Get the Prometheus metrics endpoint's settings.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_metrics_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<MetricsSettingsResponse>, ApiError> {
    Ok(Json(MetricsSettings::load(&state.pool).await?.into()))
}

/// Request payload for updating the metrics endpoint's settings.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateMetricsSettingsRequest {
    /// Whether `/metrics` is served.
    pub enabled: bool,
    /// Networks whose clients may scrape without a monitoring token: CIDR
    /// ranges or bare addresses. Empty requires a token from everyone.
    #[serde(default)]
    pub allowed_networks: Vec<String>,
}

#[utoipa::path(
    put,
    path = "/api/system/metrics",
    tag = "System",
    operation_id = "updateMetricsSettings",
    request_body = UpdateMetricsSettingsRequest,
    responses(
        (status = 200, description = "Updated settings", body = MetricsSettingsResponse),
        (status = 400, description = "An invalid network, or too many networks"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden -- admin only"),
    )
)]
/// Update the Prometheus metrics endpoint's settings.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for an allowed network that does not
/// parse, or a list that is too long.
pub async fn update_metrics_settings(
    admin: RequireAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<UpdateMetricsSettingsRequest>,
) -> Result<Json<MetricsSettingsResponse>, ApiError> {
    let settings = MetricsSettings {
        enabled: body.enabled,
        allowed_networks: parse_networks(body.allowed_networks.iter().map(String::as_str))?,
    };
    settings.save(&state.pool).await?;
    let allowed_network_count = settings.allowed_networks.len();
    tracing::info!(
        user = %admin.0.username,
        enabled = settings.enabled,
        allowed_networks = allowed_network_count,
        "metrics endpoint settings updated"
    );
    Ok(Json(MetricsSettings::load(&state.pool).await?.into()))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::test_support::{build_test_state, formatted_logs_while, insert_auth_user};

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_update_is_saved_and_logged_with_who_made_it(pool: PgPool) {
        let state = build_test_state(pool.clone(), b"metrics-settings-test-key");
        let admin = insert_auth_user(&pool, "metrics-admin").await;

        let (updated, logs) = formatted_logs_while(update_metrics_settings(
            RequireAdmin(admin),
            State(state),
            ApiJson(UpdateMetricsSettingsRequest {
                enabled: true,
                allowed_networks: vec!["10.0.0.0/8".to_owned(), "192.168.1.5".to_owned()],
            }),
        ))
        .await;

        let Json(updated) = updated.unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.allowed_networks, ["10.0.0.0/8", "192.168.1.5/32"]);
        assert_eq!(
            MetricsSettings::load(&pool).await.unwrap(),
            MetricsSettings {
                enabled: true,
                allowed_networks: parse_networks(["10.0.0.0/8,192.168.1.5"]).unwrap(),
            }
        );
        let line = logs
            .lines()
            .find(|line| line.contains("metrics endpoint settings updated"))
            .unwrap_or_default();
        assert!(line.contains("user=metrics-admin"), "{logs}");
        assert!(line.contains("enabled=true"), "{logs}");
        assert!(line.contains("allowed_networks=2"), "{logs}");
    }
}
