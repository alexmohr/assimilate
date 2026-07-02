// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  NotificationChannelResponse,
  NotificationRuleResponse,
  NotificationDeliveryResponse,
  PushSubscriptionResponse,
} from './generated'

export type NotificationChannel = Omit<NotificationChannelResponse, 'scope' | 'config' | 'channel_type'> & {
  scope: ChannelScope
  config: ChannelConfig
  channel_type: ChannelType
}
export type NotificationRule = Omit<
  NotificationRuleResponse,
  'repo_id' | 'agent_id' | 'event_type'
> & {
  repo_id: number | null
  agent_id: number | null
  event_type: NotificationEventType
}
export type NotificationDelivery = Omit<NotificationDeliveryResponse, 'status'> & {
  status: 'pending' | 'sent' | 'failed'
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

export type ChannelConfig = EmailConfig | WebhookConfig | WebPushConfig

export interface ChannelScope {
  repo_ids?: number[]
  client_ids?: number[]
  schedule_ids?: number[]
}

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
