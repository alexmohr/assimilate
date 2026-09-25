// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

mod smtp_password;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use super::auth::{AuthUser, RequireAdmin};
use crate::{
    AppState, db,
    error::{ApiError, ApiJson},
    notifications::{
        ChannelType, EventType,
        email::{EncryptedSmtpPassword, EnteredSmtpPassword, SmtpLogin, SmtpPassword},
    },
};

/// A configured destination (email, webhook, or web push) that notification events can be
/// delivered to.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationChannel {
    /// Unique identifier of the channel.
    pub id: i64,
    /// Human-readable name shown in the UI.
    pub name: String,
    /// Transport used to deliver notifications for this channel.
    pub channel_type: ChannelType,
    /// Channel-specific configuration (e.g. SMTP settings, webhook URL) as raw JSON. Never
    /// contains the SMTP password - see `has_password`.
    pub config: serde_json::Value,
    /// Whether an SMTP password is stored for this channel. The password itself is stored
    /// encrypted and never returned.
    pub has_password: bool,
    /// Whether the channel is currently active and eligible for delivery.
    pub enabled: bool,
    /// Restricts which repositories/agents this channel applies to; empty means "all".
    pub scope: serde_json::Value,
    /// Timestamp when the channel was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the channel was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Request body for creating a new notification channel.
#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    /// Human-readable name for the new channel.
    pub name: String,
    /// Transport to use for delivery (email, webhook, or web push).
    pub channel_type: ChannelType,
    /// Channel-specific configuration, validated against `channel_type`. An email channel's
    /// `smtp_password` is taken out and stored encrypted, never as part of the config.
    pub config: serde_json::Value,
    /// Whether the channel should be enabled immediately; defaults to `true`.
    pub enabled: Option<bool>,
    /// Optional scope restricting which repositories/agents trigger this channel.
    pub scope: Option<serde_json::Value>,
}

/// Request body for partially updating an existing notification channel. Omitted fields are
/// left unchanged.
#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    /// New name for the channel, if changing.
    pub name: Option<String>,
    /// New transport type for the channel, if changing.
    pub channel_type: Option<ChannelType>,
    /// New channel-specific configuration, if changing. A missing or blank `smtp_password`
    /// keeps the stored one.
    pub config: Option<serde_json::Value>,
    /// New enabled/disabled state, if changing.
    pub enabled: Option<bool>,
    /// New scope restriction, if changing.
    pub scope: Option<serde_json::Value>,
}

/// A rule that routes events of a given type (optionally scoped to a repo or agent) to a
/// notification channel.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationRule {
    /// Unique identifier of the rule.
    pub id: i64,
    /// Channel that events matching this rule are delivered to.
    pub channel_id: i64,
    /// Event type this rule triggers on (see [`EventType`]).
    pub event_type: String,
    /// Restricts the rule to events for a specific repository, if set.
    pub repo_id: Option<i64>,
    /// Restricts the rule to events for a specific agent, if set.
    pub agent_id: Option<i64>,
    /// Whether the rule is currently active.
    pub enabled: bool,
}

/// Request body for creating a new notification rule.
#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    /// Channel that matching events should be delivered to.
    pub channel_id: i64,
    /// Event type to match; must be one of [`EventType::ALL_DB_STRS`].
    pub event_type: String,
    /// Restricts the rule to a specific repository, if set.
    pub repo_id: Option<i64>,
    /// Restricts the rule to a specific agent, if set.
    pub agent_id: Option<i64>,
    /// Whether the rule should be enabled immediately; defaults to `true`.
    pub enabled: Option<bool>,
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

/// A record of a single attempt to deliver a notification event through a channel.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NotificationDelivery {
    /// Unique identifier of the delivery attempt.
    pub id: i64,
    /// Channel the notification was delivered through.
    pub channel_id: i64,
    /// Event type that triggered the delivery.
    pub event_type: String,
    /// Payload sent to the channel.
    pub payload: serde_json::Value,
    /// Outcome of the delivery attempt (e.g. success or failure).
    pub status: crate::notifications::DeliveryStatus,
    /// Error message from the delivery attempt, if it failed.
    pub error_message: Option<String>,
    /// Timestamp when the delivery was attempted.
    pub attempted_at: DateTime<Utc>,
}

/// Query parameters for listing notification deliveries.
#[derive(Debug, Deserialize)]
pub struct DeliveryQuery {
    /// Maximum number of deliveries to return; defaults to 50.
    pub limit: Option<i64>,
}

/// Validates that a channel's configuration is well-formed for its declared `channel_type`.
fn validate_channel_config(
    channel_type: ChannelType,
    config: &serde_json::Value,
) -> Result<(), ApiError> {
    if let Err(field) = crate::notifications::template::validate_template_fields(config) {
        return Err(ApiError::BadRequest(format!("{field} must not be blank")));
    }
    match channel_type {
        ChannelType::Email => {
            serde_json::from_value::<crate::notifications::email::EmailConfig>(config.clone())
                .map_err(|e| ApiError::BadRequest(format!("invalid email channel config: {e}")))?;
        }
        ChannelType::Webhook => {
            serde_json::from_value::<crate::notifications::webhook::WebhookConfig>(config.clone())
                .map_err(|e| {
                    ApiError::BadRequest(format!("invalid webhook channel config: {e}"))
                })?;
        }
        ChannelType::WebPush => {
            let obj = config.as_object().ok_or_else(|| {
                ApiError::BadRequest("web_push config must be an object".to_owned())
            })?;
            obj.get("user_id")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| {
                    ApiError::BadRequest("web_push config requires user_id".to_owned())
                })?;
        }
    }
    Ok(())
}

/// Validates that a string is a recognized [`EventType`] variant.
fn validate_event_type(t: &str) -> Result<(), ApiError> {
    t.parse::<EventType>().map(|_| ()).map_err(|_| {
        ApiError::BadRequest(format!(
            "event_type must be one of: {:?}",
            EventType::ALL_DB_STRS
        ))
    })
}

/// Lists all configured notification channels, ordered by ID.
///
/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_channels(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<NotificationChannel>>, ApiError> {
    // `- 'smtp_password'`: the API never writes the password into `config`, but a row written
    // straight to the database can still carry one until the next startup encrypts it.
    let channels: Vec<NotificationChannel> = sqlx::query_as!(
        NotificationChannel,
        r#"
        SELECT id, name, channel_type as "channel_type: ChannelType",
               config - 'smtp_password' AS "config!",
               smtp_password_encrypted IS NOT NULL AS "has_password!",
               enabled, scope, created_at, updated_at
        FROM notification_channels
        ORDER BY id
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(channels))
}

/// Creates a new notification channel. Requires admin privileges. For web-push channels the
/// creating admin's user ID is injected into the config automatically.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn create_channel(
    State(state): State<AppState>,
    admin: RequireAdmin,
    ApiJson(req): ApiJson<CreateChannelRequest>,
) -> Result<(StatusCode, Json<NotificationChannel>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }

    let mut config = if req.channel_type == ChannelType::WebPush {
        let mut cfg = req.config.clone();
        cfg.as_object_mut()
            .map(|o| o.insert("user_id".to_owned(), serde_json::json!(admin.0.user_id)));
        cfg
    } else {
        req.config.clone()
    };
    let entered_password = smtp_password::take_entered_password(&mut config)?;
    crate::notifications::template::apply_default_template(&mut config, req.channel_type);
    validate_channel_config(req.channel_type, &config)?;
    let smtp_password_encrypted = smtp_password::for_new_channel(
        req.channel_type,
        entered_password.as_ref(),
        &state.encryption_key,
    )?;

    let enabled = req.enabled.unwrap_or(true);
    let scope = req.scope.unwrap_or(serde_json::json!({}));
    let channel: NotificationChannel = sqlx::query_as!(
        NotificationChannel,
        r#"
        INSERT INTO notification_channels
            (name, channel_type, config, smtp_password_encrypted, enabled, scope, created_at,
             updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        RETURNING id, name, channel_type as "channel_type: ChannelType",
            config - 'smtp_password' AS "config!",
            smtp_password_encrypted IS NOT NULL AS "has_password!",
            enabled, scope, created_at, updated_at
        "#,
        &req.name,
        &req.channel_type.to_string(),
        &config,
        smtp_password_encrypted
            .as_ref()
            .map(EncryptedSmtpPassword::as_bytes),
        enabled,
        &scope,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(channel)))
}

/// Partially updates an existing notification channel's fields.
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
) -> Result<Json<NotificationChannel>, ApiError> {
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }

    let mut config = req.config.clone();
    let entered_password = config
        .as_mut()
        .map(smtp_password::take_entered_password)
        .transpose()?
        .flatten();

    let password_change = if req.channel_type.is_some() || config.is_some() {
        let existing = sqlx::query_as!(
            smtp_password::StoredChannel,
            r#"
            SELECT channel_type as "channel_type: ChannelType", config,
                   smtp_password_encrypted IS NOT NULL AS "has_password!"
            FROM notification_channels
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;
        let effective_type = req.channel_type.unwrap_or(existing.channel_type);
        validate_channel_config(effective_type, config.as_ref().unwrap_or(&existing.config))?;
        smtp_password::for_update(
            &existing,
            effective_type,
            config.as_ref(),
            entered_password.as_ref(),
            &state.encryption_key,
        )?
    } else {
        smtp_password::SmtpPasswordChange::Keep
    };
    let (replace_password, new_password) = password_change.update_params();

    let channel: NotificationChannel = sqlx::query_as!(
        NotificationChannel,
        r#"
        UPDATE notification_channels
        SET name = COALESCE($1::text, name),
            channel_type = COALESCE($2::text, channel_type),
            config = COALESCE($3::jsonb, config),
            enabled = COALESCE($4::bool, enabled),
            scope = COALESCE($5::jsonb, scope),
            smtp_password_encrypted = CASE WHEN $6::bool THEN $7::bytea
                                           ELSE smtp_password_encrypted END,
            updated_at = NOW()
        WHERE id = $8
        RETURNING id, name, channel_type as "channel_type: ChannelType",
                 config - 'smtp_password' AS "config!",
                 smtp_password_encrypted IS NOT NULL AS "has_password!",
                 enabled, scope, created_at, updated_at
        "#,
        req.name.as_deref(),
        req.channel_type.map(|c| c.to_string()),
        config.as_ref(),
        req.enabled,
        req.scope.as_ref(),
        replace_password,
        new_password,
        id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;

    Ok(Json(channel))
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
    let channel = sqlx::query!(
        r#"
        SELECT channel_type as "channel_type: ChannelType", config, smtp_password_encrypted
        FROM notification_channels
        WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;
    let smtp_password = channel
        .smtp_password_encrypted
        .map(EncryptedSmtpPassword::from);

    let payload = serde_json::json!({
        "event_type": "backup_success",
        "hostname": "test-host",
        "repo_name": "test-repo",
        "status": "This is a test notification from Assimilate",
        "timestamp": Utc::now().to_rfc3339(),
    });

    crate::notifications::deliver_to_channel(
        &state.notification_service,
        channel.channel_type,
        &channel.config,
        smtp_password.as_ref(),
        &payload,
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
) -> Result<Json<Vec<NotificationRule>>, ApiError> {
    let rules: Vec<NotificationRule> = sqlx::query_as!(
        NotificationRule,
        "SELECT id, channel_id, event_type, repo_id, agent_id, enabled FROM notification_rules \
         ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
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
) -> Result<(StatusCode, Json<NotificationRule>), ApiError> {
    validate_event_type(&req.event_type)?;

    let enabled = req.enabled.unwrap_or(true);
    let rule: NotificationRule = sqlx::query_as!(
        NotificationRule,
        r#"
        INSERT INTO notification_rules (channel_id, event_type, repo_id, agent_id, enabled)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, channel_id, event_type, repo_id, agent_id, enabled
        "#,
        req.channel_id,
        &req.event_type,
        req.repo_id,
        req.agent_id,
        enabled,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(rule)))
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
) -> Result<Json<Vec<NotificationDelivery>>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    let deliveries: Vec<NotificationDelivery> = sqlx::query_as!(
        NotificationDelivery,
        r#"
        SELECT id, channel_id, event_type, payload,
               status as "status: crate::notifications::DeliveryStatus", error_message,
               attempted_at
        FROM notification_deliveries ORDER BY attempted_at DESC LIMIT $1
        "#,
        limit,
    )
    .fetch_all(&state.pool)
    .await?;
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
    /// Password to authenticate with, if the server requires it. Left blank while editing a
    /// saved channel (see `channel_id`), the channel's stored password is used instead.
    #[serde(default)]
    pub smtp_password: EnteredSmtpPassword,
    /// The saved channel being edited, whose stored password to try when `smtp_password` is
    /// blank. Only honoured while `smtp_host` is still that channel's host.
    #[serde(default)]
    pub channel_id: Option<i64>,
    /// Transport security mode to use for the connection.
    #[serde(default)]
    pub security: crate::notifications::email::SmtpSecurity,
    /// Deprecated alias for `security`; kept for backward-compatible clients.
    #[serde(default)]
    pub use_tls: bool,
}

/// Logs in to an SMTP server without sending anything, so the dialog can check credentials
/// before saving them.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the login fails, or `channel_id` is used with a different host
/// - [`ApiError::NotFound`]: `channel_id` names no channel
pub async fn validate_smtp(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    ApiJson(req): ApiJson<ValidateSmtpRequest>,
) -> Result<StatusCode, ApiError> {
    let stored = match req.channel_id.filter(|_| req.smtp_password.is_empty()) {
        Some(id) => {
            let channel = sqlx::query!(
                "SELECT config, smtp_password_encrypted FROM notification_channels WHERE id = $1",
                id,
            )
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;
            if channel.smtp_password_encrypted.is_some() {
                smtp_password::ensure_same_smtp_host(&channel.config, Some(&req.smtp_host))?;
            }
            channel
                .smtp_password_encrypted
                .map(EncryptedSmtpPassword::from)
        }
        None => None,
    };
    let password = if req.smtp_password.is_empty() {
        SmtpPassword::from(stored.as_ref())
    } else {
        SmtpPassword::Entered(&req.smtp_password)
    };

    crate::notifications::email::validate_credentials(
        SmtpLogin {
            host: &req.smtp_host,
            port: req.smtp_port,
            user: &req.smtp_user,
            password,
            security: req.effective_security(),
        },
        &state.encryption_key,
    )
    .await
    .map_err(|e| ApiError::BadRequest(format!("SMTP validation failed: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

impl ValidateSmtpRequest {
    fn effective_security(&self) -> crate::notifications::email::SmtpSecurity {
        if self.security != crate::notifications::email::SmtpSecurity::Starttls {
            return self.security;
        }
        if self.use_tls {
            crate::notifications::email::SmtpSecurity::Tls
        } else {
            crate::notifications::email::SmtpSecurity::Starttls
        }
    }
}
