// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
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
      expect.arrayContaining(['SMTP Host', 'SMTP User', 'Port', 'SMTP Password']),
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
})
