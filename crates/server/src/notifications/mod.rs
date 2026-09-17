// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Email notification channel dispatcher.
pub mod email;
/// Outbound URL validation and DNS resolution helpers.
pub mod net;
/// Per-channel `{{placeholder}}` content template shared by every channel type.
pub(crate) mod template;
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
    /// The scheduler auto-disabled a schedule after it reached its
    /// `missed_backup_threshold` of consecutive missed backups.
    ScheduleAutoDisabled,
    /// A scheduled backup could not be started because its target agent was
    /// offline (not connected to the server) when the run came due.
    BackupSkippedAgentOffline,
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
        "schedule_auto_disabled",
        "backup_skipped_agent_offline",
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
    /// Correlation ID for the specific run (`BackupReport::run_id`), used to deep-link this
    /// event to its exact entry in the Activity Log. Only available for backup completion
    /// events; other event types (check, schedule, quota) have no single run to point to.
    pub run_id: Option<String>,
    /// How long the operation took, in seconds.
    pub duration_secs: Option<i64>,
    /// Total uncompressed size processed, in bytes.
    pub original_size: Option<i64>,
    /// Total compressed size written, in bytes.
    pub compressed_size: Option<i64>,
    /// Size after deduplication against existing repository chunks, in bytes.
    pub deduplicated_size: Option<i64>,
    /// Number of files processed.
    pub files_processed: Option<i64>,
    /// Warning messages emitted during an otherwise-successful operation.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// When the schedule that triggered this event is next due to run.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Absolute URL deep-linking to this event's entry in the Activity Log. Set by
    /// [`dispatch`] from the `public_url` system setting when one is configured and the
    /// event is precise enough to link (backup failures and warnings); channels that
    /// resolve links client-side (web push) build their own relative link instead and do
    /// not depend on this field.
    pub activity_url: Option<String>,
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

/// Resolves `event`'s Activity Log deep link (see [`build_activity_path`]) into an absolute
/// URL using the `public_url` system setting, when one is configured and the event is
/// precise enough to link. Returns `None` when no link applies or no base URL is set --
/// email and webhook deliveries simply omit the link in that case, the same as today.
async fn build_absolute_activity_url(
    service: &NotificationService,
    event: &NotificationEvent,
) -> Option<String> {
    let payload = serde_json::to_value(event).ok()?;
    let path = build_activity_path(&payload)?;

    let base_url = crate::db::get_setting(&service.pool, "public_url")
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to read public_url setting");
            None
        })?;
    let base_url = base_url.trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }

    Some(format!("{base_url}{path}"))
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
    mut event: NotificationEvent,
    task_registry: &TaskRegistry,
) -> Result<(), NotificationError> {
    if event.activity_url.is_none() {
        event.activity_url = build_absolute_activity_url(service, &event).await;
    }

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
    // Backfills `title_template`/`body_template` the same way `create_channel` does, so a
    // channel that predates this feature (or was inserted directly, bypassing the API --
    // e.g. the demo seed) delivers the same content this channel's own "Edit content" panel
    // shows, rather than silently falling back to the old fixed-format builders below.
    let mut config = config.clone();
    template::apply_default_template(&mut config, channel_type);

    match channel_type {
        ChannelType::Email => {
            let cfg: email::EmailConfig = serde_json::from_value(config)?;
            email::send(&cfg, payload).await
        }
        ChannelType::Webhook => {
            let cfg: webhook::WebhookConfig = serde_json::from_value(config)?;
            webhook::send(&cfg, payload).await
        }
        ChannelType::WebPush => deliver_web_push(&config, payload, pool).await,
    }
}

#[derive(Deserialize)]
struct WebPushChannelConfig {
    user_id: i64,
    /// Optional custom template for the push notification title, in place of
    /// [`build_push_title`]'s fixed `event: host` default. See [`template::render_template`]
    /// for the placeholder syntax.
    #[serde(default)]
    title_template: Option<String>,
    /// Optional custom template for the push notification body, in place of
    /// [`build_push_body`]'s fixed default. See [`template::render_template`] for the
    /// placeholder syntax.
    #[serde(default)]
    body_template: Option<String>,
}

/// Resolves the push notification's title and body: each channel's own
/// `title_template`/`body_template` when set, falling back to [`build_push_title`]/
/// [`build_push_body`]'s fixed defaults for a channel created before this feature existed.
fn push_title_and_body(
    cfg: &WebPushChannelConfig,
    payload: &serde_json::Value,
) -> (String, String) {
    let title = cfg.title_template.as_deref().map_or_else(
        || build_push_title(payload),
        |tpl| template::render_template(tpl, payload),
    );
    let body = cfg.body_template.as_deref().map_or_else(
        || build_push_body(payload),
        |tpl| template::render_template(tpl, payload),
    );
    (title, body)
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
    let tag = if event_type_str.is_empty() {
        "notification"
    } else {
        event_type_str
    };
    let (title, body) = push_title_and_body(&cfg, payload);
    let push_payload = serde_json::json!({
        "title": title,
        "body": body,
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

/// Human-readable label for an event type string (e.g. `"backup_failed"` -> `"Backup
/// failed"`), shared by the email subject and web push title builders. Falls back to
/// `"Notification"` for an empty or unrecognized event type.
pub(crate) fn event_label(event_type_str: &str) -> &'static str {
    let Ok(event_type) = event_type_str.parse::<EventType>() else {
        return "Notification";
    };
    match event_type {
        EventType::BackupSuccess => "Backup succeeded",
        EventType::BackupWarning => "Backup warning",
        EventType::BackupFailed => "Backup failed",
        EventType::CheckSuccess => "Check succeeded",
        EventType::CheckFailed => "Check failed",
        EventType::AgentConnected => "Agent connected",
        EventType::AgentDisconnected => "Agent disconnected",
        EventType::ScheduleAutoDisabled => "Schedule auto-disabled",
        EventType::BackupSkippedAgentOffline => "Backup skipped",
    }
}

/// Percent-encodes a value for safe inclusion in a URL query string, keeping only the
/// unreserved character set (letters, digits, `-`, `.`, `_`, `~`) literal.
fn percent_encode_query_value(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn truncate_for_notification(message: &str) -> String {
    let mut chars = message.chars();
    let short: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        format!("{short}...")
    } else {
        short
    }
}

/// Builds a relative deep link into the Activity Log for a backup failure/warning, using
/// whatever context the event carries: a `run_id` links to the exact run, otherwise
/// `hostname`/`schedule_id` narrow the feed as far as possible. Returns `None` for event
/// types with no meaningful Activity Log entry. Notably excludes `CheckFailed`: the
/// Activity Log's backup category reads only from `backup_reports`, and a repository
/// check is never persisted there, so a check-failure link could only ever land on that
/// host's unrelated backup history, not the check that actually failed (see
/// `build_push_url`'s fallback -- check outcomes link to the agent overview instead).
pub(crate) fn build_activity_path(payload: &serde_json::Value) -> Option<String> {
    use std::fmt::Write as _;

    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let Ok(event_type) = event_type_str.parse::<EventType>() else {
        return None;
    };
    if !matches!(
        event_type,
        EventType::BackupWarning | EventType::BackupFailed
    ) {
        return None;
    }

    if let Some(run_id) = payload
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(format!(
            "/activity?category=backup&run_id={}",
            percent_encode_query_value(run_id)
        ));
    }

    let hostname = payload
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    let schedule_id = payload
        .get("schedule_id")
        .and_then(serde_json::Value::as_i64);
    if hostname.is_none() && schedule_id.is_none() {
        return None;
    }

    let mut path = "/activity?category=backup".to_owned();
    if let Some(h) = hostname {
        let _ = write!(path, "&hostname={}", percent_encode_query_value(h));
    }
    if let Some(sid) = schedule_id {
        let _ = write!(path, "&schedule_id={sid}");
    }
    Some(path)
}

pub(crate) fn build_push_title(payload: &serde_json::Value) -> String {
    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let label = event_label(event_type_str);
    match payload
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(hostname) => format!("{label}: {hostname}"),
        None => label.to_owned(),
    }
}

pub(crate) fn build_push_body(payload: &serde_json::Value) -> String {
    let event_type_str = payload
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let repo_name = payload
        .get("repo_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty());
    let is_problem = event_type_str.parse::<EventType>().is_ok_and(|event_type| {
        matches!(
            event_type,
            EventType::BackupWarning
                | EventType::BackupFailed
                | EventType::CheckFailed
                | EventType::ScheduleAutoDisabled
                | EventType::BackupSkippedAgentOffline
        )
    });
    let error_message = payload
        .get("error_message")
        .and_then(serde_json::Value::as_str)
        .filter(|_| is_problem)
        .map(truncate_for_notification);

    match (repo_name, error_message) {
        (Some(repo), Some(err)) => format!("{repo} - {err}"),
        (Some(repo), None) => repo.to_owned(),
        (None, Some(err)) => err,
        (None, None) => payload
            .get("hostname")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map_or_else(|| "Notification".to_owned(), str::to_owned),
    }
}

pub(crate) fn build_push_url(payload: &serde_json::Value) -> String {
    if let Some(path) = build_activity_path(payload) {
        return path;
    }

    if let Some(schedule_id) = payload
        .get("schedule_id")
        .and_then(serde_json::Value::as_i64)
    {
        format!("/schedules/{schedule_id}")
    } else if let Some(hostname) = payload.get("hostname").and_then(serde_json::Value::as_str) {
        format!("/agents/{}", percent_encode_query_value(hostname))
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
            run_id: None,
            duration_secs: None,
            original_size: None,
            compressed_size: None,
            deduplicated_size: None,
            files_processed: None,
            warnings: Vec::new(),
            next_run_at: None,
            activity_url: None,
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
            run_id: None,
            duration_secs: None,
            original_size: None,
            compressed_size: None,
            deduplicated_size: None,
            files_processed: None,
            warnings: Vec::new(),
            next_run_at: None,
            activity_url: None,
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

    /// Regression test for the core new glue behind the Activity Log deep-linking feature:
    /// `dispatch` resolving an event's relative Activity Log path into an absolute
    /// `activity_url` using the `public_url` system setting, and baking it into the payload
    /// every channel receives. Checked via the persisted `notification_deliveries.payload`
    /// rather than a live webhook call -- the webhook target is deliberately unreachable, but
    /// `payload` (and therefore `activity_url`) is recorded regardless of delivery outcome.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn dispatch_resolves_activity_url_from_public_url_setting(pool: sqlx::PgPool) {
        crate::db::set_setting(&pool, "public_url", "https://backups.example.com")
            .await
            .unwrap();

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
             'backup_failed', true)",
            channel_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let service = NotificationService::new(pool.clone());
        let task_registry = TaskRegistry::default();
        let event = NotificationEvent {
            event_type: EventType::BackupFailed,
            hostname: "myhost".to_owned(),
            repo_name: "test-repo".to_owned(),
            status: "failed".to_owned(),
            error_message: Some("repository is locked".to_owned()),
            timestamp: Utc::now(),
            repo_id: None,
            agent_id: None,
            schedule_id: None,
            schedule_name: None,
            archive_name: None,
            run_id: Some("run-123".to_owned()),
            duration_secs: Some(10),
            original_size: None,
            compressed_size: None,
            deduplicated_size: None,
            files_processed: None,
            warnings: Vec::new(),
            next_run_at: None,
            activity_url: None,
        };

        dispatch(&service, event, &task_registry).await.unwrap();
        task_registry
            .shutdown(std::time::Duration::from_secs(5))
            .await;

        let payload: serde_json::Value = sqlx::query_scalar!(
            "SELECT payload FROM notification_deliveries WHERE channel_id = $1",
            channel_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(
            payload
                .get("activity_url")
                .and_then(serde_json::Value::as_str),
            Some("https://backups.example.com/activity?category=backup&run_id=run-123"),
            "dispatch must resolve the Activity Log deep link into an absolute URL using the \
             public_url setting"
        );
    }

    /// Companion to the above: when `public_url` is not configured, `activity_url` must stay
    /// absent rather than, say, falling back to a relative (and therefore broken outside a
    /// browser) link for channels like email/webhook that don't resolve it client-side.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn dispatch_omits_activity_url_when_public_url_not_configured(pool: sqlx::PgPool) {
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
             'backup_failed', true)",
            channel_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let service = NotificationService::new(pool.clone());
        let task_registry = TaskRegistry::default();
        let event = NotificationEvent {
            event_type: EventType::BackupFailed,
            hostname: "myhost".to_owned(),
            repo_name: "test-repo".to_owned(),
            status: "failed".to_owned(),
            error_message: Some("repository is locked".to_owned()),
            timestamp: Utc::now(),
            repo_id: None,
            agent_id: None,
            schedule_id: None,
            schedule_name: None,
            archive_name: None,
            run_id: Some("run-123".to_owned()),
            duration_secs: Some(10),
            original_size: None,
            compressed_size: None,
            deduplicated_size: None,
            files_processed: None,
            warnings: Vec::new(),
            next_run_at: None,
            activity_url: None,
        };

        dispatch(&service, event, &task_registry).await.unwrap();
        task_registry
            .shutdown(std::time::Duration::from_secs(5))
            .await;

        let payload: serde_json::Value = sqlx::query_scalar!(
            "SELECT payload FROM notification_deliveries WHERE channel_id = $1",
            channel_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(
            payload
                .get("activity_url")
                .is_none_or(serde_json::Value::is_null),
            "activity_url must stay absent when public_url is not configured, not payload: \
             {payload}"
        );
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
        assert_eq!(
            EventType::from_str("schedule_auto_disabled"),
            Ok(EventType::ScheduleAutoDisabled)
        );
        assert_eq!(
            EventType::from_str("backup_skipped_agent_offline"),
            Ok(EventType::BackupSkippedAgentOffline)
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
        assert_eq!(
            EventType::ScheduleAutoDisabled.to_string(),
            "schedule_auto_disabled"
        );
        assert_eq!(
            EventType::BackupSkippedAgentOffline.to_string(),
            "backup_skipped_agent_offline"
        );
    }

    #[test]
    fn all_db_strs_matches_every_event_type_variant() {
        for s in EventType::ALL_DB_STRS {
            assert!(
                s.parse::<EventType>().is_ok(),
                "ALL_DB_STRS entry {s:?} does not parse back into an EventType"
            );
        }
    }

    #[test]
    fn backup_warning_with_run_id_links_to_exact_run() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "run_id": "8f2e1a3c-0000-0000-0000-000000000000",
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&run_id=8f2e1a3c-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn backup_warning_without_run_id_links_to_activity_by_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&hostname=myhost"
        );
    }

    #[test]
    fn backup_failed_with_run_id_links_to_exact_run() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "run_id": "run-42",
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&run_id=run-42"
        );
    }

    #[test]
    fn backup_failed_without_run_id_links_to_activity_by_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&hostname=myhost"
        );
    }

    #[test]
    fn check_failed_goes_to_agent_overview_not_activity_log() {
        // A check run is never persisted to backup_reports, so the Activity Log's backup
        // category has nothing to show for it -- build_activity_path deliberately excludes
        // CheckFailed, and this falls through to the plain agent-overview link instead of a
        // deep link implying detail that doesn't exist.
        let p = payload(serde_json::json!({
            "event_type": "check_failed",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_url(&p), "/agents/myhost");
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
    fn schedule_auto_disabled_goes_to_the_schedule_detail_page() {
        let p = payload(serde_json::json!({
            "event_type": "schedule_auto_disabled",
            "hostname": "myhost",
            "schedule_id": 3,
        }));
        assert_eq!(build_push_url(&p), "/schedules/3");
    }

    // backup_skipped_agent_offline isn't one of build_activity_path's two
    // matched event types (BackupWarning/BackupFailed), so it falls through
    // to the plain schedule page rather than an Activity Log deep link.
    #[test]
    fn backup_skipped_agent_offline_goes_to_the_schedule_detail_page() {
        let p = payload(serde_json::json!({
            "event_type": "backup_skipped_agent_offline",
            "hostname": "myhost",
            "schedule_id": 5,
        }));
        assert_eq!(build_push_url(&p), "/schedules/5");
    }

    #[test]
    fn backup_warning_includes_schedule_id_alongside_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "schedule_id": 42,
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&hostname=myhost&schedule_id=42"
        );
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
    fn hostname_with_special_characters_is_encoded() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "my host/db",
        }));
        assert_eq!(
            build_push_url(&p),
            "/activity?category=backup&hostname=my%20host%2Fdb"
        );
    }

    #[test]
    fn push_title_includes_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_title(&p), "Backup failed: myhost");
    }

    #[test]
    fn push_title_missing_hostname_omits_colon() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
        }));
        assert_eq!(build_push_title(&p), "Backup failed");
    }

    #[test]
    fn push_title_agent_connected() {
        let p = payload(serde_json::json!({
            "event_type": "agent_connected",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_title(&p), "Agent connected: myhost");
    }

    #[test]
    fn push_title_empty_payload_returns_notification() {
        let p = payload(serde_json::json!({}));
        assert_eq!(build_push_title(&p), "Notification");
    }

    #[test]
    fn push_body_combines_repo_and_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "repo_name": "daily-backup",
            "error_message": "repository is locked",
        }));
        assert_eq!(build_push_body(&p), "daily-backup - repository is locked");
    }

    #[test]
    fn push_body_backup_warning_combines_repo_and_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "backup_warning",
            "hostname": "myhost",
            "repo_name": "daily-backup",
            "error_message": "quota exceeded",
        }));
        assert_eq!(build_push_body(&p), "daily-backup - quota exceeded");
    }

    #[test]
    fn push_body_check_failed_combines_repo_and_error_message() {
        let p = payload(serde_json::json!({
            "event_type": "check_failed",
            "hostname": "myhost",
            "repo_name": "daily-backup",
            "error_message": "integrity check failed",
        }));
        assert_eq!(build_push_body(&p), "daily-backup - integrity check failed");
    }

    #[test]
    fn push_body_schedule_auto_disabled_falls_back_to_error_message_without_repo() {
        let p = payload(serde_json::json!({
            "event_type": "schedule_auto_disabled",
            "hostname": "myhost",
            "error_message": "agent 'myhost' stayed unreachable",
        }));
        assert_eq!(build_push_body(&p), "agent 'myhost' stayed unreachable");
    }

    #[test]
    fn push_body_backup_skipped_agent_offline_falls_back_to_error_message_without_repo() {
        let p = payload(serde_json::json!({
            "event_type": "backup_skipped_agent_offline",
            "hostname": "myhost",
            "error_message": "agent 'myhost' is offline",
        }));
        assert_eq!(build_push_body(&p), "agent 'myhost' is offline");
    }

    #[test]
    fn push_body_success_shows_repo_name() {
        let p = payload(serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "repo_name": "daily-backup",
            "error_message": "should be ignored",
        }));
        assert_eq!(build_push_body(&p), "daily-backup");
    }

    #[test]
    fn push_body_long_error_message_truncated() {
        let long_msg = "x".repeat(150);
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "error_message": long_msg,
        }));
        let body = build_push_body(&p);
        assert!(body.ends_with("..."));
        assert_eq!(body, format!("{}...", "x".repeat(100)));
    }

    #[test]
    fn push_body_failed_without_error_message_shows_repo_name() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "repo_name": "daily-backup",
        }));
        assert_eq!(build_push_body(&p), "daily-backup");
    }

    #[test]
    fn push_body_empty_payload_returns_notification_fallback() {
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
        }));
        assert_eq!(build_push_body(&p), "Notification");
    }

    #[test]
    fn push_body_agent_connected_falls_back_to_hostname_not_literal_notification() {
        let p = payload(serde_json::json!({
            "event_type": "agent_connected",
            "hostname": "myhost",
        }));
        assert_eq!(build_push_body(&p), "myhost");
    }

    #[test]
    fn push_url_agent_fallback_percent_encodes_hostname() {
        let p = payload(serde_json::json!({
            "event_type": "agent_connected",
            "hostname": "my host/../etc",
        }));
        assert_eq!(build_push_url(&p), "/agents/my%20host%2F..%2Fetc");
    }

    #[test]
    fn push_title_and_body_falls_back_to_fixed_defaults_when_no_template_configured() {
        let cfg = WebPushChannelConfig {
            user_id: 1,
            title_template: None,
            body_template: None,
        };
        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "repo_name": "daily-backup",
        }));
        let (title, body) = push_title_and_body(&cfg, &p);
        assert_eq!(title, build_push_title(&p));
        assert_eq!(body, build_push_body(&p));
    }

    #[test]
    fn push_title_and_body_uses_channel_template_when_configured() {
        let cfg = WebPushChannelConfig {
            user_id: 1,
            title_template: Some("{{event}} on {{host}}".to_owned()),
            body_template: Some("{{dedup_size}} new".to_owned()),
        };
        let p = payload(serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "deduplicated_size": 524_288_000i64,
        }));
        let (title, body) = push_title_and_body(&cfg, &p);
        assert_eq!(title, "Backup succeeded on myhost");
        assert_eq!(body, "500.0 MiB new");
    }

    #[test]
    fn deliver_to_channel_backfill_gives_push_the_short_default_not_the_legacy_one() {
        // Mirrors what `deliver_to_channel` does before deserializing into
        // `WebPushChannelConfig`: a pre-existing channel with no `body_template` in its raw
        // config would otherwise fall through to `build_push_body` below.
        let mut raw_config = serde_json::json!({ "user_id": 1 });
        template::apply_default_template(&mut raw_config, ChannelType::WebPush);
        let cfg: WebPushChannelConfig = serde_json::from_value(raw_config).unwrap();

        let p = payload(serde_json::json!({
            "event_type": "backup_failed",
            "hostname": "myhost",
            "repo_name": "daily-backup",
            "error_message": "repository is locked",
        }));
        let (_, body) = push_title_and_body(&cfg, &p);
        assert_eq!(body, "daily-backup repository is locked");
        assert_ne!(
            body,
            build_push_body(&p),
            "the backfilled config must use DEFAULT_PUSH_BODY_TEMPLATE, not the legacy builder"
        );
    }
}
