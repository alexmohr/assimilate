// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

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
    notifications::{ChannelType, EventType},
};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationChannel {
    pub id: i64,
    pub name: String,
    pub channel_type: ChannelType,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub scope: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub channel_type: ChannelType,
    pub config: serde_json::Value,
    pub enabled: Option<bool>,
    pub scope: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub channel_type: Option<ChannelType>,
    pub config: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub scope: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationRule {
    pub id: i64,
    pub channel_id: i64,
    pub event_type: String,
    pub repo_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub channel_id: i64,
    pub event_type: String,
    pub repo_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PushSubscription {
    pub id: i64,
    pub user_id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SubscribePushRequest {
    pub endpoint: String,
    pub keys: PushKeys,
}

#[derive(Debug, Deserialize)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribePushRequest {
    pub endpoint: String,
}

#[derive(Debug, Serialize)]
pub struct VapidKeyResponse {
    pub public_key: String,
    pub configured: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetVapidKeysRequest {
    pub public_key: String,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NotificationDelivery {
    pub id: i64,
    pub channel_id: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub error_message: Option<String>,
    pub attempted_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryQuery {
    pub limit: Option<i64>,
}

fn validate_channel_config(
    channel_type: ChannelType,
    config: &serde_json::Value,
) -> Result<(), ApiError> {
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

fn validate_event_type(t: &str) -> Result<(), ApiError> {
    t.parse::<EventType>().map(|_| ()).map_err(|_| {
        ApiError::BadRequest(format!(
            "event_type must be one of: {:?}",
            EventType::ALL_DB_STRS
        ))
    })
}

pub async fn list_channels(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Vec<NotificationChannel>>, ApiError> {
    let channels: Vec<NotificationChannel> = sqlx::query_as!(
        NotificationChannel,
        r#"
        SELECT id, name, channel_type as "channel_type: ChannelType", config, enabled, scope,
               created_at, updated_at
        FROM notification_channels
        ORDER BY id
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(channels))
}

pub async fn create_channel(
    State(state): State<AppState>,
    admin: RequireAdmin,
    ApiJson(req): ApiJson<CreateChannelRequest>,
) -> Result<(StatusCode, Json<NotificationChannel>), ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".to_owned()));
    }

    let config = if req.channel_type == ChannelType::WebPush {
        let mut cfg = req.config.clone();
        cfg.as_object_mut()
            .map(|o| o.insert("user_id".to_owned(), serde_json::json!(admin.0.user_id)));
        cfg
    } else {
        req.config.clone()
    };
    validate_channel_config(req.channel_type, &config)?;

    let enabled = req.enabled.unwrap_or(true);
    let scope = req.scope.unwrap_or(serde_json::json!({}));
    let channel: NotificationChannel = sqlx::query_as!(
        NotificationChannel,
        r#"
        INSERT INTO notification_channels
            (name, channel_type, config, enabled, scope, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
        RETURNING id, name, channel_type as "channel_type: ChannelType", config, enabled, scope,
            created_at, updated_at
        "#,
        &req.name,
        &req.channel_type.to_string(),
        &config,
        enabled,
        &scope,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok((StatusCode::CREATED, Json(channel)))
}

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

    if req.channel_type.is_some() || req.config.is_some() {
        let (effective_type, effective_config) = if let (Some(ct), Some(cfg)) =
            (&req.channel_type, &req.config)
        {
            (*ct, cfg.clone())
        } else {
            let existing: NotificationChannel = sqlx::query_as!(
                NotificationChannel,
                r#"
                    SELECT id, name, channel_type as "channel_type: ChannelType", config, enabled,
                           scope, created_at, updated_at
                    FROM notification_channels
                    WHERE id = $1
                    "#,
                id,
            )
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;
            (
                req.channel_type.unwrap_or(existing.channel_type),
                req.config.clone().unwrap_or(existing.config),
            )
        };
        validate_channel_config(effective_type, &effective_config)?;
    }

    let channel: NotificationChannel = sqlx::query_as!(
        NotificationChannel,
        r#"
        UPDATE notification_channels
        SET name = COALESCE($1::text, name),
            channel_type = COALESCE($2::text, channel_type),
            config = COALESCE($3::jsonb, config),
            enabled = COALESCE($4::bool, enabled),
            scope = COALESCE($5::jsonb, scope),
            updated_at = NOW()
        WHERE id = $6
        RETURNING id, name, channel_type as "channel_type: ChannelType", config, enabled, scope,
                 created_at, updated_at
        "#,
        req.name.as_deref(),
        req.channel_type.map(|c| c.to_string()),
        req.config.as_ref(),
        req.enabled,
        req.scope.as_ref(),
        id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;

    Ok(Json(channel))
}

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

pub async fn test_channel(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let channel: NotificationChannel = sqlx::query_as!(
        NotificationChannel,
        r#"
        SELECT id, name, channel_type as "channel_type: ChannelType", config, enabled, scope,
               created_at, updated_at
        FROM notification_channels
        WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("channel {id} not found")))?;

    let payload = serde_json::json!({
        "event_type": "backup_success",
        "hostname": "test-host",
        "repo_name": "test-repo",
        "status": "This is a test notification from Assimilate",
        "timestamp": Utc::now().to_rfc3339(),
    });

    crate::notifications::deliver_to_channel(
        channel.channel_type,
        &channel.config,
        &payload,
        state.notification_service.pool(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

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

pub async fn list_deliveries(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<Vec<NotificationDelivery>>, ApiError> {
    let limit = query.limit.unwrap_or(50);
    let deliveries: Vec<NotificationDelivery> = sqlx::query_as!(
        NotificationDelivery,
        "SELECT id, channel_id, event_type, payload, status, error_message, attempted_at FROM \
         notification_deliveries ORDER BY attempted_at DESC LIMIT $1",
        limit,
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(deliveries))
}

#[derive(Debug, Deserialize)]
pub struct ValidateSmtpRequest {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_password: String,
    #[serde(default)]
    pub security: crate::notifications::email::SmtpSecurity,
    #[serde(default)]
    pub use_tls: bool,
}

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
