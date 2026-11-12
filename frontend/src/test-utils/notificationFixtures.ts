// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { vi } from 'vitest'
import type {
  EmailConfig,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
  NotificationRuleResponse,
  WebhookConfig,
} from '../types/generated'

/**
 * Notification channels, rules and deliveries shared by the Notifications
 * page, a channel's own page and the new-channel wizard, which all read the
 * same API and so were each growing a copy of the same fixtures.
 */
export const WEBHOOK_CHANNEL: NotificationChannelResponse = {
  id: 1,
  name: 'Ops Webhook',
  channel_type: 'webhook',
  config: { url: 'https://hooks.example.com/notify' } as WebhookConfig,
  has_password: false,
  webhook_headers: [],
  enabled: true,
  scope: {},
  link: { target: 'automatic' },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

export const EMAIL_CHANNEL: NotificationChannelResponse = {
  id: 2,
  name: 'Ops Email',
  channel_type: 'email',
  config: {
    smtp_host: 'smtp.example.com',
    smtp_port: 587,
    smtp_user: 'user',
    from_address: 'noreply@example.com',
    to_addresses: ['admin@example.com'],
    security: 'starttls',
  } as EmailConfig,
  has_password: true,
  webhook_headers: [],
  enabled: true,
  scope: {},
  link: { target: 'automatic' },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

export const PUSH_CHANNEL: NotificationChannelResponse = {
  ...WEBHOOK_CHANNEL,
  id: 3,
  name: 'My Browser',
  channel_type: 'web_push',
  config: {} as WebhookConfig,
}

/** The webhook channel has one event switched on; the email channel none. */
export const MOCK_RULES: NotificationRuleResponse[] = [
  {
    id: 1,
    channel_id: 1,
    event_type: 'backup_failed',
    repo_id: null,
    agent_id: null,
    enabled: true,
  },
]

/** A delivery attempt `hoursAgo` hours before now. */
export function delivery(
  id: number,
  channelId: number,
  status: 'sent' | 'failed',
  hoursAgo: number,
  error: string | null = null,
): NotificationDeliveryResponse {
  return {
    id,
    channel_id: channelId,
    event_type: 'backup_failed',
    status,
    error_message: error,
    payload: {},
    attempted_at: new Date(Date.now() - hoursAgo * 3_600_000).toISOString(),
  } as unknown as NotificationDeliveryResponse
}

/** What a channel can be scoped to, as `/repos`, `/agents` and `/schedules` serve it. */
export const SCOPE_REPOS = [{ id: 7, name: 'server-daily' }]
export const SCOPE_AGENTS = [{ id: 3, hostname: 'web-01', display_name: null }]
export const SCOPE_SCHEDULES = [{ id: 5, name: 'Nightly web', agent_id: 3, repo_id: 7 }]

/** An `apiClient.get` that serves the scope lists and nothing else. */
export function scopeOptionsGet(url: string): Promise<{ data: unknown }> {
  if (url === '/repos') return Promise.resolve({ data: SCOPE_REPOS })
  if (url === '/agents') return Promise.resolve({ data: SCOPE_AGENTS })
  if (url === '/schedules') return Promise.resolve({ data: SCOPE_SCHEDULES })
  return Promise.resolve({ data: [] })
}

/**
 * jsdom has no service worker or Push API, so a browser that supports web
 * push is assembled here: a ready registration whose push manager hands out
 * subscriptions, and a Notification permission that answers in turn. Undo it
 * with `vi.unstubAllGlobals()` and by deleting `navigator.serviceWorker`.
 */
export function stubPushBrowser(
  permission: NotificationPermission,
  answers: NotificationPermission[],
): {
  existing: { toJSON: () => unknown; unsubscribe: ReturnType<typeof vi.fn> }
  pushManager: { getSubscription: ReturnType<typeof vi.fn>; subscribe: ReturnType<typeof vi.fn> }
  requestPermission: ReturnType<typeof vi.fn>
} {
  const existing = {
    toJSON: (): unknown => ({ endpoint: 'https://push.example.com/old' }),
    unsubscribe: vi.fn().mockResolvedValue(true),
  }
  const fresh = { toJSON: (): unknown => ({ endpoint: 'https://push.example.com/new' }) }
  const pushManager = {
    getSubscription: vi.fn().mockResolvedValue(existing),
    subscribe: vi.fn().mockResolvedValue(fresh),
  }
  Object.defineProperty(navigator, 'serviceWorker', {
    configurable: true,
    value: { ready: Promise.resolve({ pushManager }) },
  })
  vi.stubGlobal('PushManager', class {})
  const requestPermission = vi.fn()
  for (const answer of answers) requestPermission.mockResolvedValueOnce(answer)
  vi.stubGlobal('Notification', { permission, requestPermission })
  return { existing, pushManager, requestPermission }
}

/** Undoes {@link stubPushBrowser}. */
export function unstubPushBrowser(): void {
  vi.unstubAllGlobals()
  Reflect.deleteProperty(navigator, 'serviceWorker')
}
