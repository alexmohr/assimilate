// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod email;
pub mod web_push;
pub mod webhook;

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Email,
    Webhook,
    WebPush,
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Email => write!(f, "email"),
            Self::Webhook => write!(f, "webhook"),
            Self::WebPush => write!(f, "web_push"),
        }
    }
}

impl ChannelType {
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Webhook => "webhook",
            Self::WebPush => "web_push",
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for ChannelType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <&str as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ChannelType {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        match s {
            "email" => Ok(Self::Email),
            "webhook" => Ok(Self::Webhook),
            "web_push" => Ok(Self::WebPush),
            other => Err(format!("unknown channel type: {other}").into()),
        }
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for ChannelType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode(self.as_db_str(), buf)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("smtp error: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("web push error: {0}")]
    WebPush(#[from] ::web_push::WebPushError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    BackupSuccess,
    BackupWarning,
    BackupFailed,
    CheckSuccess,
    CheckFailed,
    AgentConnected,
    AgentDisconnected,
}

impl EventType {
    pub const ALL_DB_STRS: &[&'static str] = &[
        "backup_success",
        "backup_warning",
        "backup_failed",
        "check_success",
        "check_failed",
        "agent_connected",
        "agent_disconnected",
    ];

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "backup_success" => Some(Self::BackupSuccess),
            "backup_warning" => Some(Self::BackupWarning),
            "backup_failed" => Some(Self::BackupFailed),
            "check_success" => Some(Self::CheckSuccess),
            "check_failed" => Some(Self::CheckFailed),
            "agent_connected" => Some(Self::AgentConnected),
            "agent_disconnected" => Some(Self::AgentDisconnected),
            _ => None,
        }
    }

    fn as_db_str(self) -> &'static str {
        match self {
            Self::BackupSuccess => "backup_success",
            Self::BackupWarning => "backup_warning",
            Self::BackupFailed => "backup_failed",
            Self::CheckSuccess => "check_success",
            Self::CheckFailed => "check_failed",
            Self::AgentConnected => "agent_connected",
            Self::AgentDisconnected => "agent_disconnected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEvent {
    pub event_type: EventType,
    pub hostname: String,
    pub repo_name: String,
    pub status: String,
    pub error_message: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub repo_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub schedule_id: Option<i64>,
    pub schedule_name: Option<String>,
    pub archive_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NotificationService {
    pool: PgPool,
    http_client: reqwest::Client,
}

impl NotificationService {
    #[must_use]
    pub fn new(pool: PgPool, http_client: reqwest::Client) -> Self {
        Self { pool, http_client }
    }

    #[must_use]
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ensure_vapid_keys(&self) -> Result<(), NotificationError> {
        let existing = crate::db::get_setting(&self.pool, "vapid_private_key")
            .await
            .map_err(|e| NotificationError::Config(format!("DB error: {e}")))?;

        if existing.is_some() {
            return Ok(());
        }

        if std::env::var("VAPID_PRIVATE_KEY").is_ok() {
            return Ok(());
        }

        tracing::info!("generating VAPID key pair for web push notifications");

        use base64::Engine;
        use p256::ecdsa::SigningKey;

        let signing_key = SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let verifying_key = signing_key.verifying_key();

        let private_bytes = signing_key.to_bytes();
        let public_bytes = verifying_key.to_encoded_point(false);

        let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let private_b64 = encoder.encode(private_bytes);
        let public_b64 = encoder.encode(public_bytes.as_bytes());

        crate::db::set_setting(&self.pool, "vapid_private_key", &private_b64)
            .await
            .map_err(|e| NotificationError::Config(format!("DB error saving private key: {e}")))?;
        crate::db::set_setting(&self.pool, "vapid_public_key", &public_b64)
            .await
            .map_err(|e| NotificationError::Config(format!("DB error saving public key: {e}")))?;

        tracing::info!("VAPID keys generated and stored in database");
        Ok(())
    }
}

#[derive(Debug, FromRow)]
struct MatchedChannel {
    id: i64,
    channel_type: ChannelType,
    config: serde_json::Value,
}

#[derive(Debug, FromRow)]
struct PushSubscriptionRow {
    endpoint: String,
    p256dh: String,
    auth: String,
}

pub async fn dispatch(
    service: &NotificationService,
    event: NotificationEvent,
) -> Result<(), NotificationError> {
    let channels: Vec<MatchedChannel> = sqlx::query_as(
        r#"
        SELECT DISTINCT nc.id, nc.channel_type, nc.config
        FROM notification_channels nc
        INNER JOIN notification_rules nr ON nr.channel_id = nc.id
        WHERE nr.event_type = $1
          AND nr.enabled = true
          AND nc.enabled = true
           AND (nc.scope = '{}' OR nc.scope IS NULL
               OR (($2::bigint IS NULL
                    OR NOT nc.scope ? 'repo_ids'
                    OR nc.scope->'repo_ids' = '[]'::jsonb
                    OR nc.scope->'repo_ids' @> to_jsonb($2::bigint))
               AND ($3::bigint IS NULL
                    OR NOT nc.scope ? 'client_ids'
                    OR nc.scope->'client_ids' = '[]'::jsonb
                    OR nc.scope->'client_ids' @> to_jsonb($3::bigint))
               AND ($4::bigint IS NULL
                    OR NOT nc.scope ? 'schedule_ids'
                    OR nc.scope->'schedule_ids' = '[]'::jsonb
                    OR nc.scope->'schedule_ids' @> to_jsonb($4::bigint))))
        "#,
    )
    .bind(event.event_type.as_db_str())
    .bind(event.repo_id)
    .bind(event.agent_id)
    .bind(event.schedule_id)
    .fetch_all(&service.pool)
    .await?;

    let payload = serde_json::to_value(&event)?;

    for channel in channels {
        let pool = service.pool.clone();
        let http_client = service.http_client.clone();
        let payload = payload.clone();
        let channel_config = channel.config.clone();
        let channel_id = channel.id;
        let event_type_str = event.event_type.as_db_str().to_owned();

        tokio::spawn(async move {
            let result = deliver_to_channel(
                channel.channel_type,
                &channel_config,
                &payload,
                &http_client,
                &pool,
            )
            .await;

            let (status, error_message) = match &result {
                Ok(()) => ("delivered".to_owned(), None),
                Err(e) => {
                    tracing::error!(channel_id, error = %e, "notification delivery failed");
                    ("failed".to_owned(), Some(e.to_string()))
                }
            };

            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO notification_deliveries
                    (channel_id, event_type, payload, status,
                     error_message, attempted_at)
                VALUES ($1, $2, $3, $4, $5, NOW())
                "#,
            )
            .bind(channel_id)
            .bind(&event_type_str)
            .bind(&payload)
            .bind(&status)
            .bind(&error_message)
            .execute(&pool)
            .await
            {
                tracing::error!(channel_id, error = %e, "failed to record delivery attempt");
            }
        });
    }

    Ok(())
}

pub async fn deliver_to_channel(
    channel_type: ChannelType,
    config: &serde_json::Value,
    payload: &serde_json::Value,
    http_client: &reqwest::Client,
    pool: &PgPool,
) -> Result<(), NotificationError> {
    match channel_type {
        ChannelType::Email => {
            let cfg: email::EmailConfig = serde_json::from_value(config.clone())?;
            email::send(&cfg, payload).await
        }
        ChannelType::Webhook => {
            let cfg: webhook::WebhookConfig = serde_json::from_value(config.clone())?;
            webhook::send(http_client, &cfg, payload).await
        }
        ChannelType::WebPush => {
            #[derive(Deserialize)]
            struct WebPushChannelConfig {
                user_id: i64,
            }
            let cfg: WebPushChannelConfig = serde_json::from_value(config.clone())?;
            let vapid_private_key = crate::db::get_setting(pool, "vapid_private_key")
                .await
                .map_err(|e| NotificationError::Config(format!("DB error reading VAPID key: {e}")))?
                .or_else(|| std::env::var("VAPID_PRIVATE_KEY").ok())
                .ok_or_else(|| {
                    NotificationError::Config("VAPID private key not configured".to_owned())
                })?;

            let subscriptions: Vec<PushSubscriptionRow> = sqlx::query_as(
                "SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = $1",
            )
            .bind(cfg.user_id)
            .fetch_all(pool)
            .await?;

            let event_type_str = payload
                .get("event_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let title = if event_type_str.is_empty() {
                "Assimilate".to_owned()
            } else {
                event_type_str.replace('_', " ")
            };
            let body = build_push_body(payload);
            let push_url = build_push_url(payload);

            let push_payload = serde_json::json!({
                "title": title,
                "body": body,
                "tag": payload.get("event_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("notification"),
                "url": push_url,
            });

            for sub in &subscriptions {
                match web_push::send(
                    &vapid_private_key,
                    sub.endpoint.clone(),
                    sub.p256dh.clone(),
                    sub.auth.clone(),
                    &push_payload,
                )
                .await
                {
                    Ok(()) => {}
                    Err(NotificationError::WebPush(
                        ::web_push::WebPushError::EndpointNotValid(_),
                    )) => {
                        tracing::warn!(endpoint = %sub.endpoint, "removing stale push subscription (410 Gone)");
                        let _ = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = $1")
                            .bind(&sub.endpoint)
                            .execute(pool)
                            .await;
                    }
                    Err(e) => {
                        tracing::error!(endpoint = %sub.endpoint, error = %e, "web push delivery failed");
                    }
                }
            }
            Ok(())
        }
    }
}

pub(crate) fn build_push_body(payload: &serde_json::Value) -> String {
    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let hostname = payload
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match payload
        .get("error_message")
        .and_then(serde_json::Value::as_str)
        .filter(|_| {
            matches!(
                event_type_str,
                "backup_warning" | "backup_failed" | "check_failed"
            )
        }) {
        Some(msg) => {
            let mut chars = msg.chars();
            let short: String = chars.by_ref().take(100).collect();
            let short = if chars.next().is_some() {
                format!("{short}...")
            } else {
                short
            };
            format!("{hostname} - {status}: {short}")
        }
        None => format!("{hostname} - {status}"),
    }
}

pub(crate) fn build_push_url(payload: &serde_json::Value) -> String {
    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let is_backup_problem = matches!(event_type_str, "backup_warning" | "backup_failed");

    if let Some(schedule_id) = payload
        .get("schedule_id")
        .and_then(serde_json::Value::as_i64)
    {
        format!("/schedules/{schedule_id}")
    } else if let Some(hostname) = payload.get("hostname").and_then(serde_json::Value::as_str) {
        if is_backup_problem {
            let archive_name = payload
                .get("archive_name")
                .and_then(serde_json::Value::as_str);
            if let Some(name) = archive_name {
                let encoded = name.replace(':', "%3A").replace(' ', "%20");
                format!("/agents/{hostname}?tab=backups&archive={encoded}")
            } else {
                format!("/agents/{hostname}?tab=backups")
            }
        } else {
            format!("/agents/{hostname}")
        }
    } else if let Some(repo_id) = payload.get("repo_id").and_then(serde_json::Value::as_i64) {
        format!("/repos/{repo_id}")
    } else {
        "/".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(json: serde_json::Value) -> serde_json::Value {
        json
    }

    #[test]
    fn backup_warning_with_archive_name_encodes_url() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "archive_name": "myhost-2026-06-03T12:30:00.000000",
        }));
        assert_eq!(
            build_push_url(&p),
            "/agents/myhost?tab=backups&archive=myhost-2026-06-03T12%3A30%3A00.000000"
        );
    }

    #[test]
    fn backup_warning_without_archive_name_goes_to_backups_tab() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_url(&p), "/agents/myhost?tab=backups");
    }

    #[test]
    fn backup_failed_with_archive_name_encodes_url() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "archive_name": "myhost-2026-06-03T08:00:00.000000",
        }));
        assert_eq!(
            build_push_url(&p),
            "/agents/myhost?tab=backups&archive=myhost-2026-06-03T08%3A00%3A00.000000"
        );
    }

    #[test]
    fn backup_failed_without_archive_name_goes_to_backups_tab() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_url(&p), "/agents/myhost?tab=backups");
    }

    #[test]
    fn backup_success_goes_to_client_overview() {
        let p = payload(serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "archive_name": "myhost-2026-06-03T08:00:00.000000",
        }));
        assert_eq!(build_push_url(&p), "/agents/myhost");
    }

    #[test]
    fn schedule_id_takes_priority_over_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "schedule_id": 42,
        }));
        assert_eq!(build_push_url(&p), "/schedules/42");
    }

    #[test]
    fn repo_id_used_when_no_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "repo_id": 7,
        }));
        assert_eq!(build_push_url(&p), "/repos/7");
    }

    #[test]
    fn empty_payload_returns_root() {
        let p = payload(serde_json::json!({}));
        assert_eq!(build_push_url(&p), "/");
    }

    #[test]
    fn archive_name_with_spaces_encoded() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "archive_name": "my host archive",
        }));
        assert_eq!(
            build_push_url(&p),
            "/agents/myhost?tab=backups&archive=my%20host%20archive"
        );
    }

    #[test]
    fn push_body_backup_failed_includes_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "status": "failed",
            "error_message": "repository is locked",
        }));
        assert_eq!(build_push_body(&p), "myhost - failed: repository is locked");
    }

    #[test]
    fn push_body_backup_warning_includes_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "status": "warning",
            "error_message": "quota exceeded",
        }));
        assert_eq!(build_push_body(&p), "myhost - warning: quota exceeded");
    }

    #[test]
    fn push_body_check_failed_includes_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "check_failed",
            "hostname": "myhost",
            "status": "failed",
            "error_message": "integrity check failed",
        }));
        assert_eq!(
            build_push_body(&p),
            "myhost - failed: integrity check failed"
        );
    }

    #[test]
    fn push_body_success_omits_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "status": "success",
            "error_message": "should be ignored",
        }));
        assert_eq!(build_push_body(&p), "myhost - success");
    }

    #[test]
    fn push_body_long_error_message_truncated() {
        let long_msg = "x".repeat(150);
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "status": "failed",
            "error_message": long_msg,
        }));
        let body = build_push_body(&p);
        assert!(body.ends_with("..."));
        assert_eq!(body, format!("myhost - failed: {}...", "x".repeat(100)));
    }

    #[test]
    fn push_body_no_error_message_omits_colon() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "status": "failed",
        }));
        assert_eq!(build_push_body(&p), "myhost - failed");
    }

    #[test]
    fn push_body_missing_hostname_uses_unknown() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "status": "failed",
        }));
        assert_eq!(build_push_body(&p), "unknown - failed");
    }
}
