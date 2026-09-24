// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Wire types for notification channels, rules and deliveries.
//!
//! These are the one definition of what a channel's configuration, scope and
//! delivered payload look like: the server stores and delivers them, the API
//! returns them, and the frontend's bindings are generated from them, so no
//! side has to re-declare (or guess at) a shape the other one owns.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

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
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[ts(export)]
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

/// Outcome of a single attempt to deliver a notification event through a
/// channel. Mirrors the `notification_deliveries.status` CHECK constraint
/// (`0002_notifications.sql`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[ts(export)]
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

/// Notification event categories that can trigger delivery rules.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[ts(export)]
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
    /// A scheduled backup could not be started because the host holding its
    /// target repository did not answer SSH when the run came due.
    BackupSkippedRepoOffline,
}

/// SMTP security mode for email delivery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    /// Unencrypted SMTP.
    None,
    /// STARTTLS upgrade on the standard port.
    #[default]
    Starttls,
    /// Implicit TLS on the standard port.
    Tls,
}

/// Configuration for an SMTP email notification channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(from = "EmailConfigWire")]
pub struct EmailConfig {
    /// SMTP server hostname.
    pub smtp_host: String,
    /// SMTP server port.
    pub smtp_port: u16,
    /// SMTP authentication username.
    pub smtp_user: String,
    /// SMTP authentication password.
    pub smtp_password: String,
    /// From-address for outgoing emails.
    pub from_address: String,
    /// Recipient addresses for the notification.
    pub to_addresses: Vec<String>,
    /// Transport security mode.
    pub security: SmtpSecurity,
    /// This channel's own title (subject line) template, in place of the
    /// fixed `event: host / repo` default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title_template: Option<String>,
    /// This channel's own body template, in place of the fixed field-by-field
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub body_template: Option<String>,
}

/// [`EmailConfig`] as older clients and stored configs may still spell it:
/// with the legacy `use_tls` flag, which forced implicit TLS when `security`
/// was left at its `starttls` default. Folded into `security` on the way in so
/// the flag never survives past deserialization.
#[derive(Deserialize)]
struct EmailConfigWire {
    smtp_host: String,
    smtp_port: u16,
    smtp_user: String,
    smtp_password: String,
    from_address: String,
    to_addresses: Vec<String>,
    #[serde(default)]
    security: SmtpSecurity,
    #[serde(default)]
    use_tls: bool,
    #[serde(default)]
    title_template: Option<String>,
    #[serde(default)]
    body_template: Option<String>,
}

impl From<EmailConfigWire> for EmailConfig {
    fn from(wire: EmailConfigWire) -> Self {
        let security = if wire.use_tls && wire.security == SmtpSecurity::Starttls {
            SmtpSecurity::Tls
        } else {
            wire.security
        };
        Self {
            smtp_host: wire.smtp_host,
            smtp_port: wire.smtp_port,
            smtp_user: wire.smtp_user,
            smtp_password: wire.smtp_password,
            from_address: wire.from_address,
            to_addresses: wire.to_addresses,
            security,
            title_template: wire.title_template,
            body_template: wire.body_template,
        }
    }
}

/// Configuration for an HTTP webhook notification channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct WebhookConfig {
    /// Target URL to POST the notification payload to.
    pub url: String,
    /// Custom HTTP headers to include in the request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// This channel's own title template, rendered into a `title` field
    /// alongside the raw event fields in the JSON payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title_template: Option<String>,
    /// This channel's own body template, rendered into a `message` field
    /// alongside the raw event fields in the JSON payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub body_template: Option<String>,
}

/// The part of a web push channel's configuration a client chooses: its
/// content templates. Which user's devices it pushes to is not a client
/// choice - see [`WebPushConfig::user_id`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct WebPushSettings {
    /// This channel's own title template, in place of the fixed `event: host`
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title_template: Option<String>,
    /// This channel's own body template, in place of the fixed default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub body_template: Option<String>,
}

/// A stored web push channel's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct WebPushConfig {
    /// The user whose subscribed devices receive this channel's pushes: the
    /// admin who created the channel, set by the server.
    #[ts(type = "number")]
    pub user_id: i64,
    /// This channel's content templates.
    #[serde(flatten)]
    pub settings: WebPushSettings,
}

/// A channel's transport and that transport's configuration, stored and sent
/// as the adjacent `channel_type` and `config` fields of the channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(tag = "channel_type", content = "config", rename_all = "snake_case")]
pub enum ChannelConfig {
    /// SMTP email delivery.
    Email(EmailConfig),
    /// HTTP POST to a webhook URL.
    Webhook(WebhookConfig),
    /// Web push to the owning user's subscribed browsers.
    WebPush(WebPushConfig),
}

/// A channel configuration as a client submits it. Identical to
/// [`ChannelConfig`] except that a web push channel carries only its
/// [`WebPushSettings`]: the server decides whose devices it pushes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(tag = "channel_type", content = "config", rename_all = "snake_case")]
pub enum ChannelConfigInput {
    /// SMTP email delivery.
    Email(EmailConfig),
    /// HTTP POST to a webhook URL.
    Webhook(WebhookConfig),
    /// Web push to the owning user's subscribed browsers.
    WebPush(WebPushSettings),
}

/// A configuration submitted for a channel of a different transport. A
/// channel's transport is fixed when it is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a {current} channel cannot be given a {submitted} configuration")]
pub struct ChannelTypeMismatch {
    /// The channel's own transport.
    pub current: ChannelType,
    /// The transport the submitted configuration is for.
    pub submitted: ChannelType,
}

/// A channel's `title_template` and `body_template`, whichever transport it
/// uses.
pub struct ContentTemplates<'a> {
    /// The title (email subject, push title, webhook `title`) template.
    pub title: &'a mut Option<String>,
    /// The body (email body, push body, webhook `message`) template.
    pub body: &'a mut Option<String>,
}

impl ChannelConfig {
    /// Parses the configuration stored for a channel of `channel_type`.
    ///
    /// # Errors
    ///
    /// Returns an error if `config` is not a valid configuration for
    /// `channel_type`.
    pub fn from_stored(
        channel_type: ChannelType,
        config: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        match channel_type {
            ChannelType::Email => serde_json::from_value(config).map(Self::Email),
            ChannelType::Webhook => serde_json::from_value(config).map(Self::Webhook),
            ChannelType::WebPush => serde_json::from_value(config).map(Self::WebPush),
        }
    }

    /// This channel's configuration replaced by `input`, keeping what a client
    /// does not choose: which user's devices a web push channel pushes to.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelTypeMismatch`] if `input` is for another transport.
    pub fn replace_with(self, input: ChannelConfigInput) -> Result<Self, ChannelTypeMismatch> {
        let mismatch = ChannelTypeMismatch {
            current: self.channel_type(),
            submitted: input.channel_type(),
        };
        match (self, input) {
            (Self::Email(_), ChannelConfigInput::Email(cfg)) => Ok(Self::Email(cfg)),
            (Self::Webhook(_), ChannelConfigInput::Webhook(cfg)) => Ok(Self::Webhook(cfg)),
            (Self::WebPush(current), ChannelConfigInput::WebPush(settings)) => {
                Ok(Self::WebPush(WebPushConfig {
                    user_id: current.user_id,
                    settings,
                }))
            }
            (Self::Email(_), ChannelConfigInput::Webhook(_) | ChannelConfigInput::WebPush(_))
            | (Self::Webhook(_), ChannelConfigInput::Email(_) | ChannelConfigInput::WebPush(_))
            | (Self::WebPush(_), ChannelConfigInput::Email(_) | ChannelConfigInput::Webhook(_)) => {
                Err(mismatch)
            }
        }
    }

    /// The configuration to store alongside [`Self::channel_type`]: the
    /// counterpart of [`Self::from_stored`].
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized.
    pub fn to_stored(&self) -> Result<serde_json::Value, serde_json::Error> {
        match self {
            Self::Email(cfg) => serde_json::to_value(cfg),
            Self::Webhook(cfg) => serde_json::to_value(cfg),
            Self::WebPush(cfg) => serde_json::to_value(cfg),
        }
    }

    /// The transport this configuration is for.
    #[must_use]
    pub const fn channel_type(&self) -> ChannelType {
        match self {
            Self::Email(_) => ChannelType::Email,
            Self::Webhook(_) => ChannelType::Webhook,
            Self::WebPush(_) => ChannelType::WebPush,
        }
    }

    /// This channel's title and body templates, whichever transport it uses.
    #[must_use]
    pub fn templates(&self) -> (Option<&str>, Option<&str>) {
        let (title, body) = match self {
            Self::Email(cfg) => (&cfg.title_template, &cfg.body_template),
            Self::Webhook(cfg) => (&cfg.title_template, &cfg.body_template),
            Self::WebPush(cfg) => (&cfg.settings.title_template, &cfg.settings.body_template),
        };
        (title.as_deref(), body.as_deref())
    }

    /// This channel's content templates, for filling in defaults uniformly
    /// across transports.
    pub fn templates_mut(&mut self) -> ContentTemplates<'_> {
        match self {
            Self::Email(cfg) => ContentTemplates {
                title: &mut cfg.title_template,
                body: &mut cfg.body_template,
            },
            Self::Webhook(cfg) => ContentTemplates {
                title: &mut cfg.title_template,
                body: &mut cfg.body_template,
            },
            Self::WebPush(cfg) => ContentTemplates {
                title: &mut cfg.settings.title_template,
                body: &mut cfg.settings.body_template,
            },
        }
    }
}

impl ChannelConfigInput {
    /// The transport this configuration is for.
    #[must_use]
    pub const fn channel_type(&self) -> ChannelType {
        match self {
            Self::Email(_) => ChannelType::Email,
            Self::Webhook(_) => ChannelType::Webhook,
            Self::WebPush(_) => ChannelType::WebPush,
        }
    }

    /// The configuration to store, with a web push channel pushing to
    /// `push_user_id`'s devices.
    #[must_use]
    pub fn into_config(self, push_user_id: i64) -> ChannelConfig {
        match self {
            Self::Email(cfg) => ChannelConfig::Email(cfg),
            Self::Webhook(cfg) => ChannelConfig::Webhook(cfg),
            Self::WebPush(settings) => ChannelConfig::WebPush(WebPushConfig {
                user_id: push_user_id,
                settings,
            }),
        }
    }
}

/// Restricts which repositories, agents and schedules trigger a channel.
/// An absent or empty list places no restriction on that dimension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ChannelScope {
    /// Repositories whose events reach this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<Vec<f64>>")]
    pub repo_ids: Option<Vec<i64>>,
    /// Agents whose events reach this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<Vec<f64>>")]
    pub agent_ids: Option<Vec<i64>>,
    /// Schedules whose events reach this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<Vec<f64>>")]
    pub schedule_ids: Option<Vec<i64>>,
}

/// A notification event carrying all context for delivery to a channel, and
/// the payload recorded for each delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
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
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    /// Optional agent ID for scoping.
    #[ts(type = "number | null")]
    pub agent_id: Option<i64>,
    /// Optional schedule ID for scoping.
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    /// Optional human-readable schedule name.
    pub schedule_name: Option<String>,
    /// Optional borg archive name.
    pub archive_name: Option<String>,
    /// Correlation ID for the specific run (`BackupReport::run_id`), used to
    /// deep-link this event to its exact entry in the Activity Log. Only
    /// available for backup completion events.
    pub run_id: Option<String>,
    /// How long the operation took, in seconds.
    #[ts(type = "number | null")]
    pub duration_secs: Option<i64>,
    /// Total uncompressed size processed, in bytes.
    #[ts(type = "number | null")]
    pub original_size: Option<i64>,
    /// Total compressed size written, in bytes.
    #[ts(type = "number | null")]
    pub compressed_size: Option<i64>,
    /// Size after deduplication against existing repository chunks, in bytes.
    #[ts(type = "number | null")]
    pub deduplicated_size: Option<i64>,
    /// Number of files processed.
    #[ts(type = "number | null")]
    pub files_processed: Option<i64>,
    /// Warning messages emitted during an otherwise-successful operation.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// When the schedule that triggered this event is next due to run.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Absolute URL deep-linking to this event's entry in the Activity Log,
    /// when the `public_url` system setting is configured and the event is
    /// precise enough to link.
    pub activity_url: Option<String>,
}

/// A configured destination that notification events are delivered to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NotificationChannelResponse {
    /// Unique identifier.
    #[ts(type = "number")]
    pub id: i64,
    /// Display name.
    pub name: String,
    /// The channel's transport and its configuration.
    #[serde(flatten)]
    pub config: ChannelConfig,
    /// Whether this channel is eligible for delivery.
    pub enabled: bool,
    /// Which repositories, agents and schedules trigger this channel.
    pub scope: ChannelScope,
    /// When the channel was created.
    pub created_at: DateTime<Utc>,
    /// When the channel was last updated.
    pub updated_at: DateTime<Utc>,
}

/// A rule routing events of one type to a notification channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NotificationRuleResponse {
    /// Unique identifier.
    #[ts(type = "number")]
    pub id: i64,
    /// Channel that matching events are delivered to.
    #[ts(type = "number")]
    pub channel_id: i64,
    /// Event type this rule triggers on.
    pub event_type: EventType,
    /// Restricts the rule to one repository's events, if set.
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    /// Restricts the rule to one agent's events, if set.
    #[ts(type = "number | null")]
    pub agent_id: Option<i64>,
    /// Whether the rule is active.
    pub enabled: bool,
}

/// A record of one attempt to deliver a notification event through a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct NotificationDeliveryResponse {
    /// Unique identifier.
    #[ts(type = "number")]
    pub id: i64,
    /// Channel the notification was delivered through.
    #[ts(type = "number")]
    pub channel_id: i64,
    /// Event type that triggered the delivery.
    pub event_type: EventType,
    /// The event as delivered to the channel.
    pub payload: NotificationEvent,
    /// Outcome of the attempt.
    pub status: DeliveryStatus,
    /// Error message, if the attempt failed.
    pub error_message: Option<String>,
    /// When the delivery was attempted.
    pub attempted_at: DateTime<Utc>,
}

/// Request body for creating a notification channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateChannelRequest {
    /// Display name.
    pub name: String,
    /// The channel's transport and its configuration.
    #[serde(flatten)]
    pub config: ChannelConfigInput,
    /// Whether the channel is enabled immediately; defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub enabled: Option<bool>,
    /// Which repositories, agents and schedules trigger this channel; defaults
    /// to all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<ChannelScope>,
}

/// Request body for partially updating a notification channel. Omitted fields
/// are left unchanged. A channel's transport is fixed when it is created, so a
/// new `config` must carry the channel's own `channel_type`.
///
/// Deserialized through [`UpdateChannelRequestWire`] rather than a flattened
/// `Option`: serde turns *any* failure to parse a flattened `Option` into
/// `None`, which would make a malformed config (or one sent without its
/// `channel_type`) a silent no-op instead of a rejected request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UpdateChannelRequestWire")]
pub struct UpdateChannelRequest {
    /// New display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New configuration.
    #[serde(flatten)]
    pub config: Option<ChannelConfigInput>,
    /// New enabled state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// New scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ChannelScope>,
}

/// [`UpdateChannelRequest`] as it arrives, before `channel_type` and `config`
/// are checked to come as a pair and parsed into a [`ChannelConfigInput`].
#[derive(Deserialize)]
struct UpdateChannelRequestWire {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    channel_type: Option<ChannelType>,
    #[serde(default)]
    config: Option<serde_json::Value>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    scope: Option<ChannelScope>,
}

impl TryFrom<UpdateChannelRequestWire> for UpdateChannelRequest {
    type Error = String;

    fn try_from(wire: UpdateChannelRequestWire) -> Result<Self, Self::Error> {
        let config = match (wire.channel_type, wire.config) {
            (None, None) => None,
            (Some(channel_type), Some(config)) => Some(parse_config_input(channel_type, config)?),
            (Some(_), None) | (None, Some(_)) => {
                return Err("channel_type and config must be sent together".to_owned());
            }
        };
        Ok(Self {
            name: wire.name,
            config,
            enabled: wire.enabled,
            scope: wire.scope,
        })
    }
}

fn parse_config_input(
    channel_type: ChannelType,
    config: serde_json::Value,
) -> Result<ChannelConfigInput, String> {
    let parsed = match channel_type {
        ChannelType::Email => serde_json::from_value(config).map(ChannelConfigInput::Email),
        ChannelType::Webhook => serde_json::from_value(config).map(ChannelConfigInput::Webhook),
        ChannelType::WebPush => serde_json::from_value(config).map(ChannelConfigInput::WebPush),
    };
    parsed.map_err(|e| format!("invalid {channel_type} channel config: {e}"))
}

/// Request body for creating a notification rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateRuleRequest {
    /// Channel that matching events are delivered to.
    #[ts(type = "number")]
    pub channel_id: i64,
    /// Event type to match.
    pub event_type: EventType,
    /// Restricts the rule to one repository's events, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<f64>")]
    pub repo_id: Option<i64>,
    /// Restricts the rule to one agent's events, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<f64>")]
    pub agent_id: Option<i64>,
    /// Whether the rule is enabled immediately; defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub enabled: Option<bool>,
}

/// TypeScript-only shapes for types whose Rust definition ts-rs cannot
/// express. Test-only because ts-rs writes bindings from its generated tests,
/// and nothing constructs these outside of that.
#[cfg(test)]
pub mod ts_shapes {
    use ts_rs::TS;

    use super::{ChannelConfigInput, ChannelScope};

    /// The TypeScript shape of [`super::UpdateChannelRequest`]: its plain
    /// fields, plus either a complete `channel_type`/`config` pair or neither.
    #[derive(TS)]
    #[ts(export, rename = "UpdateChannelRequest")]
    pub struct UpdateChannelRequestShape {
        /// New display name.
        #[ts(optional)]
        pub name: Option<String>,
        /// New enabled state.
        #[ts(optional)]
        pub enabled: Option<bool>,
        /// New scope.
        #[ts(optional)]
        pub scope: Option<ChannelScope>,
        /// Replacement configuration, or none.
        #[ts(flatten)]
        pub config: ConfigChange,
    }

    /// Replace the configuration, or leave it as it is.
    #[derive(TS)]
    #[ts(untagged)]
    pub enum ConfigChange {
        /// Replace it with this configuration.
        Replace(ChannelConfigInput),
        /// Keep the current configuration.
        Keep(NoConfigChange),
    }

    /// Neither `channel_type` nor `config`.
    #[derive(TS)]
    pub struct NoConfigChange {
        /// Absent.
        #[ts(optional, type = "never")]
        pub channel_type: Option<()>,
        /// Absent.
        #[ts(optional, type = "never")]
        pub config: Option<()>,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn email_config_json() -> serde_json::Value {
        json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "smtp_user": "user",
            "smtp_password": "pass",
            "from_address": "backups@example.com",
            "to_addresses": ["admin@example.com"],
            "security": "starttls",
        })
    }

    #[test]
    fn channel_response_keeps_channel_type_and_config_side_by_side() {
        let channel = NotificationChannelResponse {
            id: 1,
            name: "Ops".to_owned(),
            config: ChannelConfig::WebPush(WebPushConfig {
                user_id: 7,
                settings: WebPushSettings::default(),
            }),
            enabled: true,
            scope: ChannelScope::default(),
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let value = serde_json::to_value(&channel).unwrap();
        assert_eq!(value.get("channel_type"), Some(&json!("web_push")));
        assert_eq!(value.get("config"), Some(&json!({ "user_id": 7 })));
        assert_eq!(value.get("scope"), Some(&json!({})));
    }

    #[test]
    fn stored_config_round_trips_for_every_transport() {
        let configs = [
            ChannelConfig::Email(serde_json::from_value(email_config_json()).unwrap()),
            ChannelConfig::Webhook(WebhookConfig {
                url: "https://hooks.example.com".to_owned(),
                headers: HashMap::from([("X-Token".to_owned(), "t".to_owned())]),
                title_template: Some("{{event}}".to_owned()),
                body_template: None,
            }),
            ChannelConfig::WebPush(WebPushConfig {
                user_id: 4,
                settings: WebPushSettings::default(),
            }),
        ];
        for config in configs {
            let stored = config.to_stored().unwrap();
            assert!(stored.get("channel_type").is_none(), "{stored}");
            assert_eq!(
                ChannelConfig::from_stored(config.channel_type(), stored).unwrap(),
                config
            );
        }
    }

    #[test]
    fn email_config_folds_the_legacy_use_tls_flag_into_security() {
        let mut raw = email_config_json();
        raw.as_object_mut()
            .unwrap()
            .insert("use_tls".to_owned(), json!(true));
        let config: EmailConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(config.security, SmtpSecurity::Tls);
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("use_tls")
                .is_none()
        );
    }

    #[test]
    fn email_config_keeps_an_explicit_security_over_use_tls() {
        let mut raw = email_config_json();
        let fields = raw.as_object_mut().unwrap();
        fields.insert("security".to_owned(), json!("none"));
        fields.insert("use_tls".to_owned(), json!(true));
        let config: EmailConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(config.security, SmtpSecurity::None);
    }

    #[test]
    fn web_push_input_takes_the_pushing_user_from_the_server() {
        let input: ChannelConfigInput = serde_json::from_value(json!({
            "channel_type": "web_push",
            "config": { "user_id": 99, "title_template": "{{event}}" },
        }))
        .unwrap();
        let ChannelConfig::WebPush(config) = input.into_config(3) else {
            panic!("a web push input stays a web push config");
        };
        assert_eq!(config.user_id, 3);
        assert_eq!(config.settings.title_template.as_deref(), Some("{{event}}"));
    }

    #[test]
    fn replacing_a_web_push_config_keeps_its_user() {
        let current = ChannelConfig::WebPush(WebPushConfig {
            user_id: 5,
            settings: WebPushSettings::default(),
        });
        let replaced = current
            .replace_with(ChannelConfigInput::WebPush(WebPushSettings {
                title_template: Some("t".to_owned()),
                body_template: None,
            }))
            .unwrap();
        let ChannelConfig::WebPush(replaced) = replaced else {
            panic!("a web push channel stays one");
        };
        assert_eq!(replaced.user_id, 5);
        assert_eq!(replaced.settings.title_template.as_deref(), Some("t"));
    }

    #[test]
    fn replacing_a_config_with_another_transports_is_rejected() {
        let current = ChannelConfig::Email(serde_json::from_value(email_config_json()).unwrap());
        let err = current
            .replace_with(ChannelConfigInput::WebPush(WebPushSettings::default()))
            .unwrap_err();
        assert_eq!(
            err,
            ChannelTypeMismatch {
                current: ChannelType::Email,
                submitted: ChannelType::WebPush,
            }
        );
    }

    #[test]
    fn templates_mut_reaches_every_transport() {
        let mut configs = [
            ChannelConfig::Email(serde_json::from_value(email_config_json()).unwrap()),
            ChannelConfig::Webhook(WebhookConfig {
                url: "https://hooks.example.com".to_owned(),
                headers: HashMap::new(),
                title_template: None,
                body_template: None,
            }),
            ChannelConfig::WebPush(WebPushConfig {
                user_id: 1,
                settings: WebPushSettings::default(),
            }),
        ];
        for config in &mut configs {
            *config.templates_mut().title = Some("t".to_owned());
            *config.templates_mut().body = Some("b".to_owned());
        }
        for config in &configs {
            assert_eq!(config.templates(), (Some("t"), Some("b")));
        }
    }

    #[test]
    fn update_request_without_a_config_changes_nothing_else() {
        let req: UpdateChannelRequest =
            serde_json::from_value(json!({ "enabled": false })).unwrap();
        assert_eq!(
            req,
            UpdateChannelRequest {
                enabled: Some(false),
                ..UpdateChannelRequest::default()
            }
        );
    }

    #[test]
    fn update_request_parses_a_config_for_its_channel_type() {
        let req: UpdateChannelRequest = serde_json::from_value(json!({
            "channel_type": "webhook",
            "config": { "url": "https://hooks.example.com" },
        }))
        .unwrap();
        assert!(
            matches!(req.config, Some(ChannelConfigInput::Webhook(ref cfg)) if cfg.url == "https://hooks.example.com")
        );
    }

    #[test]
    fn update_request_rejects_a_config_that_does_not_fit_its_channel_type() {
        let err = serde_json::from_value::<UpdateChannelRequest>(json!({
            "channel_type": "email",
            "config": { "url": "https://hooks.example.com" },
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("invalid email channel config"),
            "{err}"
        );
    }

    #[test]
    fn update_request_rejects_a_config_without_its_channel_type() {
        let err = serde_json::from_value::<UpdateChannelRequest>(json!({
            "config": { "url": "https://hooks.example.com" },
        }))
        .unwrap_err();
        assert!(err.to_string().contains("must be sent together"), "{err}");
    }

    #[test]
    fn update_request_rejects_a_channel_type_without_its_config() {
        let err =
            serde_json::from_value::<UpdateChannelRequest>(json!({ "channel_type": "webhook" }))
                .unwrap_err();
        assert!(err.to_string().contains("must be sent together"), "{err}");
    }

    #[test]
    fn update_request_round_trips_through_its_own_serialization() {
        let req = UpdateChannelRequest {
            name: Some("Ops".to_owned()),
            config: Some(ChannelConfigInput::WebPush(WebPushSettings::default())),
            enabled: None,
            scope: Some(ChannelScope {
                repo_ids: Some(vec![1]),
                ..ChannelScope::default()
            }),
        };
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(
            serde_json::from_value::<UpdateChannelRequest>(value).unwrap(),
            req
        );
    }

    #[test]
    fn notification_event_reads_a_payload_recorded_before_newer_fields_existed() {
        let event: NotificationEvent = serde_json::from_value(json!({
            "event_type": "backup_failed",
            "hostname": "web-01",
            "repo_name": "daily",
            "status": "failed",
            "timestamp": "2026-01-15T03:00:12Z",
        }))
        .unwrap();
        assert_eq!(event.event_type, EventType::BackupFailed);
        assert_eq!(event.warnings, Vec::<String>::new());
        assert_eq!(event.activity_url, None);
    }
}
