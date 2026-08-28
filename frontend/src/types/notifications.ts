// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  NotificationChannelResponse,
  NotificationRuleResponse,
  NotificationDeliveryResponse,
  PushSubscriptionResponse,
} from './generated'

// `ChannelType`, `NotificationEventType`, `SmtpSecurity` and the delivery status literal below
// mirror enums that live in crates/server/src/notifications/ (ChannelType, EventType,
// SmtpSecurity, DeliveryStatus). They aren't on the ts-rs export surface: only crates/shared
// exports bindings, so crates/server serializes them as plain `String` in the response DTOs
// (see NotificationChannelResponse.channel_type / NotificationRuleResponse.event_type /
// NotificationDeliveryResponse.status in crates/shared/src/responses.rs) and this file narrows
// them back to a union by hand. Promoting them would mean moving the enums into crates/shared.
export type NotificationChannel = Omit<
  NotificationChannelResponse,
  'scope' | 'config' | 'channel_type'
> & {
  scope: ChannelScope
  config: ChannelConfig
  channel_type: ChannelType
}
export type NotificationRule = Omit<NotificationRuleResponse, 'event_type'> & {
  event_type: NotificationEventType
}
export type NotificationDelivery = Omit<NotificationDeliveryResponse, 'status' | 'event_type'> & {
  status: 'pending' | 'sent' | 'failed'
  event_type: NotificationEventType
}
export type PushSubscriptionInfo = PushSubscriptionResponse

export type ChannelType = 'email' | 'webhook' | 'web_push'

export type NotificationEventType =
  | 'backup_success'
  | 'backup_warning'
  | 'backup_failed'
  | 'check_success'
  | 'check_failed'
  | 'agent_connected'
  | 'agent_disconnected'
  | 'schedule_auto_disabled'

export type SmtpSecurity = 'none' | 'starttls' | 'tls'

export interface EmailConfig {
  smtp_host: string
  smtp_port: number
  smtp_user: string
  smtp_password: string
  from_address: string
  to_addresses: string[]
  security: SmtpSecurity
}

export interface WebhookConfig {
  url: string
  headers?: Record<string, string>
}

export type WebPushConfig = Record<string, never>

// `NotificationChannelResponse.config` is `serde_json::Value` on the Rust side (genuinely
// polymorphic JSONB storage keyed by `channel_type`), so there's no single Rust type to
// generate a union from - this is frontend-only by design, not an oversight.
export type ChannelConfig = EmailConfig | WebhookConfig | WebPushConfig

// No Rust struct backs this shape at all: the server reads/writes `scope` as raw JSONB via SQL
// `?`/`->` operators (crates/server/src/notifications/mod.rs), never deserializing it into a
// typed struct.
export interface ChannelScope {
  repo_ids?: number[]
  agent_ids?: number[]
  schedule_ids?: number[]
}

// Request-body shapes with unexported Rust counterparts (crates/server/src/api/notifications.rs)
// that were never response DTOs - left frontend-only rather than exporting simple POST/PUT
// bodies from a crate outside the ts-rs surface.
export interface CreateChannelRequest {
  name: string
  channel_type: ChannelType
  config: ChannelConfig
  enabled: boolean
  scope?: ChannelScope
}

export interface UpdateChannelRequest {
  name?: string
  config?: ChannelConfig
  enabled?: boolean
  scope?: ChannelScope
}

export interface CreateRuleRequest {
  channel_id: number
  event_type: NotificationEventType
  repo_id?: number | null
  agent_id?: number | null
  enabled: boolean
}
