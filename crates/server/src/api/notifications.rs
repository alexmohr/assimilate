// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::notifications::{
    ChannelConfig, ChannelScope, CreateChannelRequest, CreateRuleRequest, DeliveryStatus,
    EventType, NotificationChannelResponse, NotificationDeliveryResponse, NotificationRuleResponse,
    UpdateChannelRequest,
};
use sqlx::FromRow;

use super::auth::{AuthUser, RequireAdmin};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
};

/// A `notification_channels` row as stored, before its `channel_type`/`config` pair and its
/// `scope` are parsed into their types.
#[derive(Debug, FromRow)]
struct ChannelRow {
    id: i64,
    name: String,
    channel_type: String,
    config: serde_json::Value,
    enabled: bool,
    scope: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ChannelRow> for NotificationChannelResponse {
    type Error = ApiError;

    fn try_from(row: ChannelRow) -> Result<Self, Self::Error> {
        let id = row.id;
        let config = crate::notifications::stored_channel_config(&row.channel_type, row.config)
            .map_err(|e| ApiError::Internal(format!("channel {id} has an invalid config: {e}")))?;
        let scope = serde_json::from_value(row.scope)
            .map_err(|e| ApiError::Internal(format!("channel {id} has an invalid scope: {e}")))?;
        Ok(Self {
            id,
            name: row.name,
            config,
            enabled: row.enabled,
            scope,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A `notification_rules` row as stored.
#[derive(Debug, FromRow)]
struct RuleRow {
    id: i64,
    channel_id: i64,
    event_type: String,
    repo_id: Option<i64>,
    agent_id: Option<i64>,
    enabled: bool,
}

impl TryFrom<RuleRow> for NotificationRuleResponse {
    type Error = ApiError;

    fn try_from(row: RuleRow) -> Result<Self, Self::Error> {
        let event_type = parse_event_type(&row.event_type)
            .map_err(|e| ApiError::Internal(format!("rule {}: {e}", row.id)))?;
        Ok(Self {
            id: row.id,
            channel_id: row.channel_id,
            event_type,
            repo_id: row.repo_id,
            agent_id: row.agent_id,
            enabled: row.enabled,
        })
    }
}

/// A `notification_deliveries` row as stored.
#[derive(Debug, FromRow)]
struct DeliveryRow {
    id: i64,
    channel_id: i64,
    event_type: String,
    payload: serde_json::Value,
    status: String,
    error_message: Option<String>,
    attempted_at: DateTime<Utc>,
}

impl TryFrom<DeliveryRow> for NotificationDeliveryResponse {
    type Error = ApiError;

    fn try_from(row: DeliveryRow) -> Result<Self, Self::Error> {
        let id = row.id;
        let invalid = |what: &str, e: &dyn std::fmt::Display| {
            ApiError::Internal(format!("delivery {id} has an invalid {what}: {e}"))
        };
        Ok(Self {
            id,
            channel_id: row.channel_id,
            event_type: parse_event_type(&row.event_type).map_err(|e| invalid("event type", &e))?,
            payload: serde_json::from_value(row.payload).map_err(|e| invalid("payload", &e))?,
            status: row
                .status
                .parse::<DeliveryStatus>()
                .map_err(|e| invalid("status", &e))?,
            error_message: row.error_message,
            attempted_at: row.attempted_at,
        })
    }
}

fn parse_event_type(event_type: &str) -> Result<EventType, String> {
    event_type
        .parse::<EventType>()
        .map_err(|e| format!("unknown event type {event_type:?}: {e}"))
}

/// A browser web-push subscription registered by a user, used to deliver push notifications.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PushSubscription {
    /// Unique identifier of the subscription.
    pub id: i64,
    /// User the subscription belongs to.
    pub user_id: i64,
    /// Push service endpoint URL supplied by the browser.
    pub endpoint: String,
    /// Client public key used to encrypt push payloads.
    pub p256dh: String,
    /// Client authentication secret used to encrypt push payloads.
    pub auth: String,
    /// User agent string of the browser that registered the subscription, if known.
    pub user_agent: Option<String>,
    /// Timestamp when the subscription was created.
    pub created_at: DateTime<Utc>,
}

/// Request body for registering a new web-push subscription.
#[derive(Debug, Deserialize)]
pub struct SubscribePushRequest {
    /// Push service endpoint URL supplied by the browser.
    pub endpoint: String,
    /// Encryption keys supplied by the browser's push subscription.
    pub keys: PushKeys,
}

/// Encryption keys for a web-push subscription, as supplied by the browser.
#[derive(Debug, Deserialize)]
pub struct PushKeys {
    /// Client public key used to encrypt push payloads.
    pub p256dh: String,
    /// Client authentication secret used to encrypt push payloads.
    pub auth: String,
}

/// Request body for removing a web-push subscription.
#[derive(Debug, Deserialize)]
pub struct UnsubscribePushRequest {
    /// Endpoint of the subscription to remove.
    pub endpoint: String,
}

/// Response describing the server's VAPID public key for web push.
#[derive(Debug, Serialize)]
pub struct VapidKeyResponse {
    /// VAPID public key, or an empty string if none is configured.
    pub public_key: String,
    /// Whether a VAPID key pair is currently configured on the server.
    pub configured: bool,
}

/// Request body for setting the server's VAPID key pair used for web push.
#[derive(Debug, Deserialize)]
pub struct SetVapidKeysRequest {
    /// VAPID public key to store.
    pub public_key: String,
    /// VAPID private key to store.
    pub private_key: String,
}

/// Query parameters for listing notification deliveries.
#[derive(Debug, Deserialize)]
pub struct DeliveryQuery {
    /// Maximum number of deliveries to return; defaults to 50.
    pub limit: Option<i64>,
}

/// Rejects a channel configuration whose content templates are present but blank. Its shape
/// is already checked by the time it is typed.
fn validate_channel_config(config: &ChannelConfig) -> Result<(), ApiError> {
    crate::notifications::template::validate_template_fields(config)
        .map_err(|field| ApiError::BadRequest(format!("{field} must not be blank")))
}

fn stored_config(config: &ChannelConfig) -> Result<serde_json::Value, ApiError> {
    config
        .to_stored()
        .map_err(|e| ApiError::Internal(format!("failed to serialize channel config: {e}")))
}

fn stored_scope(scope: &ChannelScope) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(scope)
        .map_err(|e| ApiError::Internal(format!("failed to serialize channel scope: {e}")))
}

async fn fetch_channel(pool: &sqlx::PgPool, id: i64) -> Result<ChannelRow, ApiError> {
    sqlx::query_as!(
        ChannelRow,
        "SELECT id, name, channel_type, config, enabled, scope, created_at, updated_at FROM \
         notification_channels WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))
}

/// Lists all configured notification channels, ordered by ID.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_channels(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<NotificationChannelResponse>>, ApiError> {
    let rows = sqlx::query_as!(
        ChannelRow,
        "SELECT id, name, channel_type, config, enabled, scope, created_at, updated_at FROM \
         notification_channels ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let channels = rows
        .into_iter()
        .map(NotificationChannelResponse::try_from)
        .collect::<Result<_, _>>()?;
    Ok(Json(channels))
}

/// Creates a new notification channel. Requires admin privileges. A web push channel pushes
/// to the creating admin's own subscribed devices.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn create_channel(
    State(state): State<AppState>,
    admin: RequireAdmin,
    ApiJson(req): ApiJson<CreateChannelRequest>,
) -> Result<(StatusCode, Json<NotificationChannelResponse>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }

    let mut config = req.config.into_config(admin.0.user_id);
    crate::notifications::template::apply_default_template(&mut config);
    validate_channel_config(&config)?;

    let row = sqlx::query_as!(
        ChannelRow,
        r#"
        INSERT INTO notification_channels
            (name, channel_type, config, enabled, scope, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
        RETURNING id, name, channel_type, config, enabled, scope, created_at, updated_at
        "#,
        &req.name,
        config.channel_type().to_string(),
        stored_config(&config)?,
        req.enabled.unwrap_or(true),
        stored_scope(&req.scope.unwrap_or_default())?,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(row.try_into()?)))
}

/// Partially updates an existing notification channel's fields. A new configuration must be
/// for the channel's own transport; a web push channel keeps pushing to the devices of the
/// user it was created for.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_channel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
    ApiJson(req): ApiJson<UpdateChannelRequest>,
) -> Result<Json<NotificationChannelResponse>, ApiError> {
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }

    let config = match req.config {
        None => None,
        Some(input) => {
            let existing =
                NotificationChannelResponse::try_from(fetch_channel(&state.pool, id).await?)?;
            let config = existing
                .config
                .replace_with(input)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            validate_channel_config(&config)?;
            Some(stored_config(&config)?)
        }
    };
    let scope = req.scope.as_ref().map(stored_scope).transpose()?;

    let row = sqlx::query_as!(
        ChannelRow,
        r#"
        UPDATE notification_channels
        SET name = COALESCE($1::text, name),
            config = COALESCE($2::jsonb, config),
            enabled = COALESCE($3::bool, enabled),
            scope = COALESCE($4::jsonb, scope),
            updated_at = NOW()
        WHERE id = $5
        RETURNING id, name, channel_type, config, enabled, scope, created_at, updated_at
        "#,
        req.name.as_deref(),
        config,
        req.enabled,
        scope,
        id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;

    Ok(Json(row.try_into()?))
}

/// Deletes a notification channel by ID.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn delete_channel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query!("DELETE FROM notification_channels WHERE id = $1", id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("channel {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a synthetic test notification through the given channel so an admin can verify its
/// configuration works end-to-end.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Internal`]: an internal error occurs
pub async fn test_channel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let channel = NotificationChannelResponse::try_from(fetch_channel(&state.pool, id).await?)?;

    let payload = serde_json::json!({
        "event_type": "backup_success",
        "hostname": "test-host",
        "repo_name": "test-repo",
        "status": "This is a test notification from Assimilate",
        "timestamp": Utc::now().to_rfc3339(),
    });

    crate::notifications::deliver_to_channel(
        &channel.config,
        &payload,
        state.notification_service.pool(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Lists all configured notification rules, ordered by ID.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_rules(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<NotificationRuleResponse>>, ApiError> {
    let rows = sqlx::query_as!(
        RuleRow,
        "SELECT id, channel_id, event_type, repo_id, agent_id, enabled FROM notification_rules \
         ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let rules = rows
        .into_iter()
        .map(NotificationRuleResponse::try_from)
        .collect::<Result<_, _>>()?;
    Ok(Json(rules))
}

/// Creates a new notification rule routing a given event type to a channel.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn create_rule(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    ApiJson(req): ApiJson<CreateRuleRequest>,
) -> Result<(StatusCode, Json<NotificationRuleResponse>), ApiError> {
    let row = sqlx::query_as!(
        RuleRow,
        r#"
        INSERT INTO notification_rules (channel_id, event_type, repo_id, agent_id, enabled)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, channel_id, event_type, repo_id, agent_id, enabled
        "#,
        req.channel_id,
        req.event_type.to_string(),
        req.repo_id,
        req.agent_id,
        req.enabled.unwrap_or(true),
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(row.try_into()?)))
}

/// Deletes a notification rule by ID.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the requested resource does not exist.
pub async fn delete_rule(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query!("DELETE FROM notification_rules WHERE id = $1", id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("rule {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Returns the server's configured VAPID public key for web push, if any.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_vapid_key(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<VapidKeyResponse>, ApiError> {
    let public_key = db::get_setting(&state.pool, "vapid_public_key")
        .await?
        .or_else(|| std::env::var("VAPID_PUBLIC_KEY").ok());
    match public_key {
        Some(key) => Ok(Json(VapidKeyResponse {
            public_key: key,
            configured: true,
        })),
        None => Ok(Json(VapidKeyResponse {
            public_key: String::new(),
            configured: false,
        })),
    }
}

/// Stores the server's VAPID key pair used to sign web-push notifications.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn set_vapid_keys(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    ApiJson(req): ApiJson<SetVapidKeysRequest>,
) -> Result<StatusCode, ApiError> {
    if req.public_key.trim().is_empty() || req.private_key.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "both public_key and private_key are required".to_owned(),
        ));
    }
    db::set_setting(&state.pool, "vapid_public_key", req.public_key.trim()).await?;
    db::set_setting(&state.pool, "vapid_private_key", req.private_key.trim()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Registers a web-push subscription for the authenticated user, validating that the push
/// endpoint is a permitted outbound destination.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn subscribe_push(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<SubscribePushRequest>,
) -> Result<(StatusCode, Json<PushSubscription>), ApiError> {
    if req.endpoint.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "endpoint must not be empty".to_owned(),
        ));
    }

    let (_url, _addrs) = crate::notifications::net::validate_outbound_url(&req.endpoint)
        .await
        .map_err(|e| match e {
            crate::notifications::NotificationError::Config(msg) => ApiError::BadRequest(msg),
            other => ApiError::Internal(other.to_string()),
        })?;

    let sub: PushSubscription = sqlx::query_as!(
        PushSubscription,
        r#"
        INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth, created_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (endpoint) DO UPDATE SET p256dh = $3, auth = $4
        RETURNING id, user_id, endpoint, p256dh, auth, user_agent, created_at
        "#,
        user.user_id,
        &req.endpoint,
        &req.keys.p256dh,
        &req.keys.auth,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(sub)))
}

/// Removes the authenticated user's web-push subscription for a given endpoint.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn unsubscribe_push(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<UnsubscribePushRequest>,
) -> Result<StatusCode, ApiError> {
    sqlx::query!(
        "DELETE FROM push_subscriptions WHERE user_id = $1 AND endpoint = $2",
        user.user_id,
        &req.endpoint,
    )
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_push_subscriptions(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<PushSubscription>>, ApiError> {
    let subs: Vec<PushSubscription> = sqlx::query_as!(
        PushSubscription,
        "SELECT id, user_id, endpoint, p256dh, auth, user_agent, created_at FROM \
         push_subscriptions WHERE user_id = $1 ORDER BY id",
        user.user_id,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(subs))
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_deliveries(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<Vec<NotificationDeliveryResponse>>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    let rows = sqlx::query_as!(
        DeliveryRow,
        "SELECT id, channel_id, event_type, payload, status, error_message, attempted_at FROM \
         notification_deliveries ORDER BY attempted_at DESC LIMIT $1",
        limit,
    )
    .fetch_all(&state.pool)
    .await?;
    let deliveries = rows
        .into_iter()
        .map(NotificationDeliveryResponse::try_from)
        .collect::<Result<_, _>>()?;
    Ok(Json(deliveries))
}

/// Request payload for test-connecting to an SMTP server before saving it as a
/// notification channel.
#[derive(Debug, Deserialize)]
pub struct ValidateSmtpRequest {
    /// Hostname or IP address of the SMTP server.
    pub smtp_host: String,
    /// Port to connect to on the SMTP server.
    pub smtp_port: u16,
    /// Username to authenticate with, if the server requires it.
    pub smtp_user: String,
    /// Password to authenticate with, if the server requires it.
    pub smtp_password: String,
    /// Transport security mode to use for the connection.
    #[serde(default)]
    pub security: shared::notifications::SmtpSecurity,
    /// Deprecated alias for `security`; kept for backward-compatible clients.
    #[serde(default)]
    pub use_tls: bool,
}

/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn validate_smtp(
    _admin: RequireAdmin,
    ApiJson(req): ApiJson<ValidateSmtpRequest>,
) -> Result<StatusCode, ApiError> {
    crate::notifications::email::validate_credentials(
        &req.smtp_host,
        req.smtp_port,
        &req.smtp_user,
        &req.smtp_password,
        req.effective_security(),
    )
    .await
    .map_err(|e| ApiError::BadRequest(format!("SMTP validation failed: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

impl ValidateSmtpRequest {
    fn effective_security(&self) -> shared::notifications::SmtpSecurity {
        if self.security != shared::notifications::SmtpSecurity::Starttls {
            return self.security;
        }
        if self.use_tls {
            shared::notifications::SmtpSecurity::Tls
        } else {
            shared::notifications::SmtpSecurity::Starttls
        }
    }
}
