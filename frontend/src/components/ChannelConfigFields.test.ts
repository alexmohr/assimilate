// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { fieldByLabel, renderWithPlugins } from '../test-utils'
import { validateSmtp } from '../api/notifications'
import ChannelConfigFields from './ChannelConfigFields.vue'
import type { EmailConfig, WebhookConfig } from '../types/notifications'

vi.mock('../api/notifications', () => ({ validateSmtp: vi.fn() }))

function emailConfig(): EmailConfig {
  return {
    smtp_host: 'smtp.example.com',
    smtp_port: 587,
    smtp_user: 'ops',
    smtp_password: 'hunter2',
    from_address: 'noreply@example.com',
    to_addresses: ['admin@example.com'],
    security: 'starttls',
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ChannelConfigFields, {
    props: {
      channelType: 'email',
      emailConfig: emailConfig(),
      webhookConfig: { url: '', headers: {} } as WebhookConfig,
      toAddresses: 'admin@example.com',
      ...props,
    },
  })
}

describe('ChannelConfigFields', () => {
  beforeEach(() => {
    vi.mocked(validateSmtp).mockReset().mockResolvedValue(undefined)
  })

  it('renders the SMTP fields for an email channel', () => {
    const wrapper = mount()
    const labels = wrapper.findAll('.field-label').map((l) => l.text())
    expect(labels).toEqual(
      expect.arrayContaining(['SMTP Host', 'SMTP user', 'Port', 'SMTP password']),
    )
  })

  it('renders only the URL field for a webhook channel', () => {
    const wrapper = mount({ channelType: 'webhook' })
    expect(wrapper.findAll('.field-label').map((l) => l.text())).toEqual(['URL'])
  })

  it('renders nothing for a web push channel, which has no transport config', () => {
    expect(mount({ channelType: 'web_push' }).findAll('.field')).toHaveLength(0)
  })

  it('marks the mandatory fields only when the caller asks', () => {
    expect(mount().findAll('.required')).toHaveLength(0)
    expect(mount({ showRequired: true }).findAll('.required').length).toBeGreaterThan(0)
  })

  it('reports the recipient field back through its model', async () => {
    const wrapper = mount()
    const inputs = wrapper.findAll('input')
    const recipients = inputs[inputs.length - 1]
    await recipients.setValue('a@example.com, b@example.com')
    expect(wrapper.emitted('update:toAddresses')?.at(-1)).toEqual(['a@example.com, b@example.com'])
  })

  it('validates the credentials it was given and reports success inline', async () => {
    const wrapper = mount()
    await wrapper.find('button').trigger('click')
    await flushPromises()

    expect(validateSmtp).toHaveBeenCalledWith({
      smtp_host: 'smtp.example.com',
      smtp_port: 587,
      smtp_user: 'ops',
      smtp_password: 'hunter2',
      security: 'starttls',
    })
    expect(wrapper.find('.smtp-validation-result').classes()).toContain('test-success')
  })

  it('reports a failed login inline without throwing', async () => {
    vi.mocked(validateSmtp).mockRejectedValue(new Error('535 auth failed'))
    const wrapper = mount()
    await wrapper.find('button').trigger('click')
    await flushPromises()

    const result = wrapper.find('.smtp-validation-result')
    expect(result.classes()).toContain('test-failure')
    expect(result.text()).toContain('535 auth failed')
  })

  it('defaults the security mode when the config has none', async () => {
    const wrapper = mount({ emailConfig: { ...emailConfig(), security: null } })
    await wrapper.find('button').trigger('click')
    await flushPromises()
    expect(vi.mocked(validateSmtp).mock.calls[0][0]).toMatchObject({ security: 'starttls' })
  })

  it('exposes validate and reset for the dialog that owns the save', async () => {
    const wrapper = mount()
    const vm = wrapper.vm as unknown as {
      validate: () => Promise<boolean>
      reset: () => void
      result: { success: boolean } | null
    }

    expect(await vm.validate()).toBe(true)
    await flushPromises()
    expect(wrapper.find('.smtp-validation-result').exists()).toBe(true)

    vm.reset()
    await flushPromises()
    expect(wrapper.find('.smtp-validation-result').exists()).toBe(false)
  })

  it('returns false from validate so the caller can block the save', async () => {
    vi.mocked(validateSmtp).mockRejectedValue(new Error('nope'))
    const wrapper = mount()
    const vm = wrapper.vm as unknown as { validate: () => Promise<boolean> }
    expect(await vm.validate()).toBe(false)
  })

  describe('field bindings', () => {
    // The fields edit the caller's config object in place, so each binding is
    // asserted against the object that was passed in. A v-model pointed at
    // the wrong key looks identical in the template and would quietly send
    // the old value.
    it('writes each SMTP field back onto the config it was given', async () => {
      const config = emailConfig()
      const wrapper = mount({ emailConfig: config })

      await fieldByLabel(wrapper, 'SMTP user').setValue('operator')
      await fieldByLabel(wrapper, 'Port').setValue('2525')
      await fieldByLabel(wrapper, 'SMTP password').setValue('correct horse')
      await fieldByLabel(wrapper, 'Security').setValue('tls')

      expect(config).toMatchObject({
        smtp_user: 'operator',
        smtp_port: 2525,
        smtp_password: 'correct horse',
        security: 'tls',
      })
    })

    it('keeps the port numeric rather than storing the raw string', async () => {
      const config = emailConfig()
      const wrapper = mount({ emailConfig: config })
      await fieldByLabel(wrapper, 'Port').setValue('465')
      expect(config.smtp_port).toBe(465)
    })

    it('writes the webhook URL back onto the webhook config', async () => {
      const webhook = { url: '', headers: {} } as WebhookConfig
      const wrapper = mount({ channelType: 'webhook', webhookConfig: webhook })

      await fieldByLabel(wrapper, 'URL').setValue('https://hooks.example.com/notify')

      expect(webhook.url).toBe('https://hooks.example.com/notify')
    })

    it('shows only the fields for the selected channel type', () => {
      const email = mount()
      expect(() => fieldByLabel(email, 'SMTP Host')).not.toThrow()
      expect(() => fieldByLabel(email, 'URL')).toThrow()

      const webhook = mount({ channelType: 'webhook' })
      expect(() => fieldByLabel(webhook, 'URL')).not.toThrow()
      expect(() => fieldByLabel(webhook, 'SMTP Host')).toThrow()
    })
  })
})
