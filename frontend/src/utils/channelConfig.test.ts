// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { EmailConfig, WebhookConfig } from '../types/generated'
import { configInputFor, withTemplates } from './channelConfig'

const email: EmailConfig = {
  smtp_host: 'smtp.example.com',
  smtp_port: 587,
  smtp_user: 'user',
  smtp_password: 'pass',
  from_address: 'alerts@example.com',
  to_addresses: ['ops@example.com'],
  security: 'starttls',
}
const webhook: WebhookConfig = { url: 'https://hooks.example.com', headers: {} }

describe('configInputFor', () => {
  it('submits the draft for the chosen transport', () => {
    expect(configInputFor('email', { email, webhook })).toEqual({
      channel_type: 'email',
      config: email,
    })
    expect(configInputFor('webhook', { email, webhook })).toEqual({
      channel_type: 'webhook',
      config: webhook,
    })
  })

  it('submits no settings for a web push channel', () => {
    expect(configInputFor('web_push', { email, webhook })).toEqual({
      channel_type: 'web_push',
      config: {},
    })
  })
})

describe('withTemplates', () => {
  it('keeps the rest of an email or webhook config', () => {
    expect(withTemplates({ channel_type: 'email', config: email }, 't', 'b')).toEqual({
      channel_type: 'email',
      config: { ...email, title_template: 't', body_template: 'b' },
    })
    expect(withTemplates({ channel_type: 'webhook', config: webhook }, 't', 'b')).toEqual({
      channel_type: 'webhook',
      config: { ...webhook, title_template: 't', body_template: 'b' },
    })
  })

  it('leaves out the server-owned user of a web push config', () => {
    expect(withTemplates({ channel_type: 'web_push', config: { user_id: 3 } }, 't', 'b')).toEqual({
      channel_type: 'web_push',
      config: { title_template: 't', body_template: 'b' },
    })
  })
})
