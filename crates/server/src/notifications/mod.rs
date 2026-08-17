// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Email notification channel dispatcher.
pub mod email;
/// Outbound URL validation and DNS resolution helpers.
pub mod net;
/// Web push (VAPID) notification channel dispatcher.
pub mod web_push;
/// Webhook notification channel dispatcher.
pub mod webhook;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::task_registry::TaskRegistry;
use sqlx::{FromRow, PgPool};

/// Supported notification channel types.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChannelType {
    /// SMTP email delivery.
    #[default]
    Email,
    /// HTTP POST to a configured webhook URL.
    Webhook,
    /// Web push notification via browser push API.
    WebPush,
}

impl sqlx::Type<sqlx::Postgres> for ChannelType {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <&str as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ChannelType {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(s.parse::<ChannelType>()?)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for ChannelType {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <String as sqlx::Encode<sqlx::Postgres>>::encode(self.to_string(), buf)
    }
}

/// Outcome of a single attempt to deliver a notification event through a channel. Mirrors the
/// `notification_deliveries.status` CHECK constraint (`0002_notifications.sql`) so a mismatch
/// between this type and the schema is a compile-time (rather than a silently-dropped runtime
/// INSERT) failure.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DeliveryStatus {
    /// Delivery has not been attempted yet.
    Pending,
    /// Delivery completed successfully.
    Sent,
    /// Delivery was attempted and failed.
    Failed,
}

impl sqlx::Type<sqlx::Postgres> for DeliveryStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <&str as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for DeliveryStatus {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(s.parse::<DeliveryStatus>()?)
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for DeliveryStatus {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <String as sqlx::Encode<sqlx::Postgres>>::encode(self.to_string(), buf)
    }
}

/// Errors that can occur during notification delivery.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// SMTP transport error.
    #[error("smtp error: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    /// HTTP request error.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// Web push protocol error.
    #[error("web push error: {0}")]
    WebPush(#[from] ::web_push::WebPushError),
    /// Database query or connection error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Invalid or missing configuration.
    #[error("configuration error: {0}")]
    Config(String),
    /// JSON serialization or deserialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Notification event categories that can trigger delivery rules.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EventType {
    /// Backup completed successfully.
    BackupSuccess,
    /// Backup completed with warnings.
    BackupWarning,
    /// Backup failed.
    BackupFailed,
    /// Repository integrity check succeeded.
    CheckSuccess,
    /// Repository integrity check failed.
    CheckFailed,
    /// Agent connected to the server.
    AgentConnected,
    /// Agent disconnected from the server.
    AgentDisconnected,
}

impl EventType {
    /// All event type names as static string slices for DB queries.
    pub const ALL_DB_STRS: &[&'static str] = &[
        "backup_success",
        "backup_warning",
        "backup_failed",
        "check_success",
        "check_failed",
        "agent_connected",
        "agent_disconnected",
    ];
}

/// A notification event carrying all context for delivery to a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEvent {
    /// The category of event that occurred.
    pub event_type: EventType,
    /// Hostname of the agent that triggered the event.
    pub hostname: String,
    /// Repository name associated with the event.
    pub repo_name: String,
    /// Outcome status string (e.g. "success", "failed", "warning").
    pub status: String,
    /// Optional error or warning message from the operation.
    pub error_message: Option<String>,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Optional repository ID for scoping.
    pub repo_id: Option<i64>,
    /// Optional agent ID for scoping.
    pub agent_id: Option<i64>,
    /// Optional schedule ID for scoping.
    pub schedule_id: Option<i64>,
    /// Optional human-readable schedule name.
    pub schedule_name: Option<String>,
    /// Optional borg archive name.
    pub archive_name: Option<String>,
}

/// Service for dispatching notification events to configured channels.
#[derive(Debug, Clone)]
pub struct NotificationService {
    pool: PgPool,
    in_flight_deliveries: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// Decrements the in-flight delivery counter when a spawned delivery task ends,
/// whether it completes normally or panics.
struct DeliveryGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl NotificationService {
    /// Create a new notification service backed by the given database pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            in_flight_deliveries: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Access the underlying database pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Number of spawned per-channel deliveries that have not yet finished. Used by
    /// the health check so e2e coverage teardown can wait for these to complete
    /// before stopping containers, instead of racing a fixed timeout against a
    /// variable-duration webhook/email/push delivery.
    #[must_use]
    pub fn in_flight_deliveries(&self) -> usize {
        self.in_flight_deliveries
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn begin_delivery(&self) -> DeliveryGuard {
        self.in_flight_deliveries
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        DeliveryGuard(std::sync::Arc::clone(&self.in_flight_deliveries))
    }

    /// # Errors
    ///
    /// Returns [`NotificationError::Config`] if the notification channel is misconfigured.
    pub async fn ensure_vapid_keys(&self) -> Result<(), NotificationError> {
        use base64::Engine;
        use p256::ecdsa::SigningKey;

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

/// # Errors
///
/// Returns an error if the underlying operation fails.
///
/// Each matched channel's delivery is dispatched as a spawned task registered with
/// `task_registry`, so shutdown can join it (bounded by `task_registry.shutdown`'s
/// timeout) instead of the runtime aborting a still-in-flight webhook/email/push
/// delivery when the process exits.
pub async fn dispatch(
    service: &NotificationService,
    event: NotificationEvent,
    task_registry: &TaskRegistry,
) -> Result<(), NotificationError> {
    let channels: Vec<MatchedChannel> = sqlx::query_as!(
        MatchedChannel,
        r#"
        SELECT DISTINCT nc.id, nc.channel_type as "channel_type: ChannelType", nc.config
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
                     OR NOT nc.scope ? 'agent_ids'
                     OR nc.scope->'agent_ids' = '[]'::jsonb
                     OR nc.scope->'agent_ids' @> to_jsonb($3::bigint))
                AND ($4::bigint IS NULL
                     OR NOT nc.scope ? 'schedule_ids'
                     OR nc.scope->'schedule_ids' = '[]'::jsonb
                     OR nc.scope->'schedule_ids' @> to_jsonb($4::bigint))))
        "#,
        event.event_type.to_string(),
        event.repo_id,
        event.agent_id,
        event.schedule_id,
    )
    .fetch_all(&service.pool)
    .await?;

    let payload = serde_json::to_value(&event)?;

    for channel in channels {
        let pool = service.pool.clone();
        let payload = payload.clone();
        let channel_config = channel.config.clone();
        let channel_id = channel.id;
        let event_type_str = event.event_type.to_string();
        let delivery_guard = service.begin_delivery();

        let handle = tokio::spawn(async move {
            let _delivery_guard = delivery_guard;
            let result =
                deliver_to_channel(channel.channel_type, &channel_config, &payload, &pool).await;

            let (status, error_message) = match &result {
                Ok(()) => (DeliveryStatus::Sent, None),
                Err(e) => {
                    tracing::error!(channel_id, error = %e, "notification delivery failed");
                    (DeliveryStatus::Failed, Some(e.to_string()))
                }
            };

            if let Err(e) = sqlx::query!(
                r#"
                INSERT INTO notification_deliveries
                    (channel_id, event_type, payload, status,
                     error_message, attempted_at)
                VALUES ($1, $2, $3, $4, $5, NOW())
                "#,
                channel_id,
                &event_type_str,
                &payload,
                status.to_string(),
                error_message,
            )
            .execute(&pool)
            .await
            {
                tracing::error!(channel_id, error = %e, "failed to record delivery attempt");
            }
        });
        task_registry.register(handle);
    }

    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`NotificationError::Config`]: the notification channel is misconfigured
/// - [`NotificationError::WebPush`]: the operation fails
pub async fn deliver_to_channel(
    channel_type: ChannelType,
    config: &serde_json::Value,
    payload: &serde_json::Value,
    pool: &PgPool,
) -> Result<(), NotificationError> {
    match channel_type {
        ChannelType::Email => {
            let cfg: email::EmailConfig = serde_json::from_value(config.clone())?;
            email::send(&cfg, payload).await
        }
        ChannelType::Webhook => {
            let cfg: webhook::WebhookConfig = serde_json::from_value(config.clone())?;
            webhook::send(&cfg, payload).await
        }
        ChannelType::WebPush => deliver_web_push(config, payload, pool).await,
    }
}

#[derive(Deserialize)]
struct WebPushChannelConfig {
    user_id: i64,
}

/// Sends `payload` as a web push notification to every subscription registered for the
/// channel's user.
///
/// A channel can fan out to several subscribed devices; as long as one actually received the
/// push, the delivery counts as successful. But if every subscription fails (network/VAPID/
/// endpoint errors) or none exist, this returns `Err` instead of `Ok(())` -- silently reporting
/// success here would hide a complete delivery failure behind a "sent" status, leaving no way
/// to tell why nothing showed up client-side.
async fn deliver_web_push(
    config: &serde_json::Value,
    payload: &serde_json::Value,
    pool: &PgPool,
) -> Result<(), NotificationError> {
    let cfg: WebPushChannelConfig = serde_json::from_value(config.clone())?;
    let vapid_private_key = crate::db::get_setting(pool, "vapid_private_key")
        .await
        .map_err(|e| NotificationError::Config(format!("DB error reading VAPID key: {e}")))?
        .or_else(|| std::env::var("VAPID_PRIVATE_KEY").ok())
        .ok_or_else(|| NotificationError::Config("VAPID private key not configured".to_owned()))?;

    let subscriptions: Vec<PushSubscriptionRow> = sqlx::query_as!(
        PushSubscriptionRow,
        "SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = $1",
        cfg.user_id,
    )
    .fetch_all(pool)
    .await?;

    if subscriptions.is_empty() {
        return Err(NotificationError::Config(
            "no push subscriptions registered for this channel's user".to_owned(),
        ));
    }

    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let title = if event_type_str.is_empty() {
        "Assimilate".to_owned()
    } else {
        event_type_str.replace('_', " ")
    };

    let tag = if event_type_str.is_empty() {
        "notification"
    } else {
        event_type_str
    };
    let push_payload = serde_json::json!({
        "title": title,
        "body": build_push_body(payload),
        "tag": tag,
        "url": build_push_url(payload),
    });

    let mut delivered_to_any = false;
    let mut last_error: Option<NotificationError> = None;

    for sub in &subscriptions {
        let (url, addrs) = match self::net::validate_outbound_url(&sub.endpoint).await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(endpoint = %sub.endpoint, error = %e, "skipping push subscription with non-routable endpoint");
                last_error = Some(e);
                continue;
            }
        };
        match web_push::send(
            &vapid_private_key,
            sub.endpoint.clone(),
            sub.p256dh.clone(),
            sub.auth.clone(),
            &push_payload,
            &url,
            &addrs,
        )
        .await
        {
            Ok(()) => delivered_to_any = true,
            Err(NotificationError::WebPush(::web_push::WebPushError::EndpointNotValid(_))) => {
                tracing::warn!(endpoint = %sub.endpoint, "removing stale push subscription (410 Gone)");
                let _ = sqlx::query!(
                    "DELETE FROM push_subscriptions WHERE endpoint = $1",
                    &sub.endpoint,
                )
                .execute(pool)
                .await;
            }
            Err(e) => {
                tracing::error!(endpoint = %sub.endpoint, error = %e, "web push delivery failed");
                last_error = Some(e);
            }
        }
    }

    if delivered_to_any {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| {
            NotificationError::Config(
                "all push subscriptions were stale and have been removed".to_owned(),
            )
        }))
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
    use std::str::FromStr;

    use super::*;

    fn payload(json: serde_json::Value) -> serde_json::Value {
        json
    }

    /// A dispatched channel's delivery must be registered with `task_registry`
    /// before `dispatch` returns, so shutdown can join it (bounded by
    /// `task_registry.shutdown`'s timeout) instead of a still-in-flight
    /// webhook/email/push delivery being silently aborted when the process
    /// exits. The webhook points at an unreachable address deliberately -
    /// this test only cares that the task was registered and is joinable, not
    /// that delivery succeeds.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn dispatch_registers_each_channel_delivery_with_task_registry(pool: sqlx::PgPool) {
        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'webhook', $2, true) RETURNING id",
            "test-webhook",
            serde_json::json!({ "url": "http://127.0.0.1:1/unreachable" }),
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO notification_rules (channel_id, event_type, enabled) VALUES ($1, \
             'backup_success', true)",
            channel_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let service = NotificationService::new(pool);
        let task_registry = TaskRegistry::default();
        let event = NotificationEvent {
            event_type: EventType::BackupSuccess,
            hostname: "test-host".to_owned(),
            repo_name: "test-repo".to_owned(),
            status: "success".to_owned(),
            error_message: None,
            timestamp: Utc::now(),
            repo_id: None,
            agent_id: None,
            schedule_id: None,
            schedule_name: None,
            archive_name: None,
        };

        dispatch(&service, event, &task_registry).await.unwrap();

        assert_eq!(
            task_registry.pending_count(),
            1,
            "dispatch must register the spawned per-channel delivery before returning"
        );

        let outstanding = task_registry
            .shutdown(std::time::Duration::from_secs(5))
            .await;
        assert_eq!(
            outstanding, 0,
            "task_registry.shutdown must join the delivery task instead of abandoning it"
        );
    }

    /// Regression test for a bug where a successful delivery was recorded with
    /// `status = "delivered"`, a value the `notification_deliveries` CHECK
    /// constraint doesn't allow (only `pending`/`sent`/`failed`). The mismatched
    /// insert failed silently (logged, not propagated), so every successful
    /// delivery -- across every channel type -- never appeared in the delivery
    /// history at all. Exercises every `DeliveryStatus` variant's `Display`
    /// output directly against the real schema, independent of any network
    /// call, so it fails the same way the schema itself would reject a
    /// mismatched literal.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn delivery_status_values_satisfy_db_check_constraint(pool: sqlx::PgPool) {
        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'webhook', $2, true) RETURNING id",
            "test-webhook",
            serde_json::json!({ "url": "https://example.invalid/hook" }),
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        for status in [
            DeliveryStatus::Pending,
            DeliveryStatus::Sent,
            DeliveryStatus::Failed,
        ] {
            sqlx::query!(
                "INSERT INTO notification_deliveries (channel_id, event_type, payload, status, \
                 attempted_at) VALUES ($1, $2, $3, $4, NOW())",
                channel_id,
                "backup_success",
                serde_json::json!({}),
                status.to_string(),
            )
            .execute(&pool)
            .await
            .unwrap_or_else(|e| {
                panic!("status {status} must satisfy the DB CHECK constraint: {e}")
            });
        }
    }

    /// Regression test for a bug where a web-push channel with no subscribed
    /// devices (or where every subscription failed to deliver) still returned
    /// `Ok(())` from `deliver_to_channel`, so `dispatch` recorded the attempt
    /// as a success. That masked genuine delivery failures -- including the
    /// literal "nothing showed up on any client" case -- behind a "sent"
    /// status with no error message to diagnose from.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn dispatch_records_failure_when_no_push_subscriptions_exist(pool: sqlx::PgPool) {
        crate::db::set_setting(&pool, "vapid_private_key", "dummy")
            .await
            .unwrap();

        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'web_push', $2, true) RETURNING id",
            "test-web-push",
            serde_json::json!({ "user_id": 1 }),
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO notification_rules (channel_id, event_type, enabled) VALUES ($1, \
             'backup_success', true)",
            channel_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let service = NotificationService::new(pool.clone());
        let task_registry = TaskRegistry::default();
        let event = NotificationEvent {
            event_type: EventType::BackupSuccess,
            hostname: "test-host".to_owned(),
            repo_name: "test-repo".to_owned(),
            status: "success".to_owned(),
            error_message: None,
            timestamp: Utc::now(),
            repo_id: None,
            agent_id: None,
            schedule_id: None,
            schedule_name: None,
            archive_name: None,
        };

        dispatch(&service, event, &task_registry).await.unwrap();
        task_registry
            .shutdown(std::time::Duration::from_secs(5))
            .await;

        let delivery = sqlx::query!(
            r#"SELECT status as "status: DeliveryStatus", error_message
               FROM notification_deliveries WHERE channel_id = $1"#,
            channel_id,
        )
        .fetch_one(&pool)
        .await
        .expect("the delivery attempt must be recorded in notification_deliveries");

        assert_eq!(delivery.status, DeliveryStatus::Failed);
        assert!(delivery.error_message.is_some());
    }

    #[test]
    fn channel_type_from_str() {
        assert_eq!(ChannelType::from_str("email"), Ok(ChannelType::Email));
        assert_eq!(ChannelType::from_str("webhook"), Ok(ChannelType::Webhook));
        assert_eq!(ChannelType::from_str("web_push"), Ok(ChannelType::WebPush));
        assert!(ChannelType::from_str("unknown").is_err());
    }

    #[test]
    fn channel_type_display() {
        assert_eq!(ChannelType::Email.to_string(), "email");
        assert_eq!(ChannelType::Webhook.to_string(), "webhook");
        assert_eq!(ChannelType::WebPush.to_string(), "web_push");
    }

    #[test]
    fn channel_type_default_is_email() {
        assert_eq!(ChannelType::default(), ChannelType::Email);
    }

    #[test]
    fn delivery_status_from_str() {
        assert_eq!(
            DeliveryStatus::from_str("pending"),
            Ok(DeliveryStatus::Pending)
        );
        assert_eq!(DeliveryStatus::from_str("sent"), Ok(DeliveryStatus::Sent));
        assert_eq!(
            DeliveryStatus::from_str("failed"),
            Ok(DeliveryStatus::Failed)
        );
        assert!(DeliveryStatus::from_str("delivered").is_err());
    }

    #[test]
    fn delivery_status_display() {
        assert_eq!(DeliveryStatus::Pending.to_string(), "pending");
        assert_eq!(DeliveryStatus::Sent.to_string(), "sent");
        assert_eq!(DeliveryStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn event_type_from_str() {
        use std::str::FromStr;

        assert_eq!(
            EventType::from_str("backup_success"),
            Ok(EventType::BackupSuccess)
        );
        assert_eq!(
            EventType::from_str("backup_warning"),
            Ok(EventType::BackupWarning)
        );
        assert_eq!(
            EventType::from_str("backup_failed"),
            Ok(EventType::BackupFailed)
        );
        assert_eq!(
            EventType::from_str("check_success"),
            Ok(EventType::CheckSuccess)
        );
        assert_eq!(
            EventType::from_str("check_failed"),
            Ok(EventType::CheckFailed)
        );
        assert_eq!(
            EventType::from_str("agent_connected"),
            Ok(EventType::AgentConnected)
        );
        assert_eq!(
            EventType::from_str("agent_disconnected"),
            Ok(EventType::AgentDisconnected)
        );
        assert!(EventType::from_str("unknown_event").is_err());
    }

    #[test]
    fn event_type_display() {
        assert_eq!(EventType::BackupSuccess.to_string(), "backup_success");
        assert_eq!(EventType::BackupWarning.to_string(), "backup_warning");
        assert_eq!(EventType::BackupFailed.to_string(), "backup_failed");
        assert_eq!(EventType::CheckSuccess.to_string(), "check_success");
        assert_eq!(EventType::CheckFailed.to_string(), "check_failed");
        assert_eq!(EventType::AgentConnected.to_string(), "agent_connected");
        assert_eq!(
            EventType::AgentDisconnected.to_string(),
            "agent_disconnected"
        );
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
    fn backup_success_goes_to_agent_overview() {
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
