// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { flushPromises, type DOMWrapper } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import NotificationContentEditor from './NotificationContentEditor.vue'
import {
  DEFAULT_BODY_TEMPLATE,
  DEFAULT_PUSH_BODY_TEMPLATE,
  DEFAULT_TITLE_TEMPLATE,
} from '../utils/notificationTemplate'
import type { NotificationChannel } from '../types/notifications'

vi.mock('../api/notifications', () => ({
  updateChannel: vi.fn(),
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback?: string) => fallback ?? 'Unknown error',
}))

import { updateChannel } from '../api/notifications'

const mockUpdateChannel = vi.mocked(updateChannel)

function channel(overrides: Partial<NotificationChannel> = {}): NotificationChannel {
  return {
    id: 1,
    name: 'Ops Email',
    channel_type: 'email',
    config: {
      smtp_host: 'smtp.example.com',
      smtp_port: 587,
      smtp_user: 'user',
      smtp_password: 'pass',
      from_address: 'alerts@example.com',
      to_addresses: ['ops@example.com'],
      security: 'starttls',
    },
    enabled: true,
    scope: {},
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as unknown as NotificationChannel
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(NotificationContentEditor, {
    props: { channel: channel(), ...props },
  })
}

/** Expands the panel and edits the title field, the common setup for the save tests below. */
async function mountExpandedWithEditedTitle(
  title = 'custom title',
): Promise<ReturnType<typeof mount>> {
  const wrapper = mount()
  await wrapper.find('button.content-toggle').trigger('click')
  const titleInput = wrapper.find<HTMLInputElement>('input[type="text"]')
  await titleInput.setValue(title)
  return wrapper
}

function findSaveButton(wrapper: ReturnType<typeof mount>): DOMWrapper<Element> {
  return wrapper.findAll('button').find((b) => b.text().includes('Save content'))!
}

describe('NotificationContentEditor', () => {
  it('starts collapsed', () => {
    const wrapper = mount()
    expect(wrapper.find('textarea').exists()).toBe(false)
    expect(wrapper.text()).toContain('Edit content')
  })

  it('pre-fills the default template when the channel has no template of its own', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')

    const titleInput = wrapper.find<HTMLInputElement>('input[type="text"]')
    const bodyInput = wrapper.find<HTMLTextAreaElement>('textarea')
    expect(titleInput.element.value).toBe(DEFAULT_TITLE_TEMPLATE)
    expect(bodyInput.element.value).toBe(DEFAULT_BODY_TEMPLATE)
  })

  it('shows the deduplicated size in the live preview by default for a successful backup', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')
    expect(wrapper.find('.content-preview-pre').text()).toContain('Dedup:')
    expect(wrapper.find('.content-preview-pre').text()).toContain('500.0 MiB')
    expect(wrapper.text()).toContain('Dedup size shown by default')
  })

  it('pre-fills web push channels with the short push default instead of the email default', async () => {
    const wrapper = mount({
      channel: channel({ channel_type: 'web_push', config: { user_id: 1 } }),
    })
    await wrapper.find('button.content-toggle').trigger('click')

    const bodyInput = wrapper.find<HTMLTextAreaElement>('textarea')
    expect(bodyInput.element.value).toBe(DEFAULT_PUSH_BODY_TEMPLATE)
    expect(bodyInput.element.value).not.toBe(DEFAULT_BODY_TEMPLATE)
  })

  it('loads an existing per-channel template instead of the default', async () => {
    const wrapper = mount({
      channel: channel({
        config: {
          smtp_host: 'smtp.example.com',
          smtp_port: 587,
          smtp_user: 'user',
          smtp_password: 'pass',
          from_address: 'alerts@example.com',
          to_addresses: ['ops@example.com'],
          security: 'starttls',
          title_template: '🚨 {{event}} — {{host}}',
          body_template: '{{dedup_size}} new',
        },
      }),
    })
    await wrapper.find('button.content-toggle').trigger('click')
    const titleInput = wrapper.find<HTMLInputElement>('input[type="text"]')
    expect(titleInput.element.value).toBe('🚨 {{event}} — {{host}}')
  })

  it('inserts a clicked variable at the end of the last-focused field', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')

    const titleInput = wrapper.find<HTMLInputElement>('input[type="text"]')
    await titleInput.setValue('Prefix ')
    await titleInput.trigger('focus')

    const chip = wrapper.findAll('button.content-chip').find((c) => c.text() === '{{host}}')
    expect(chip).toBeDefined()
    await chip!.trigger('click')

    expect(titleInput.element.value).toBe('Prefix {{host}}')
  })

  it('resets both fields back to the default content', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')

    const titleInput = wrapper.find<HTMLInputElement>('input[type="text"]')
    await titleInput.setValue('something custom')

    const resetButton = wrapper.findAll('button').find((b) => b.text().includes('Reset'))
    await resetButton!.trigger('click')

    expect(titleInput.element.value).toBe(DEFAULT_TITLE_TEMPLATE)
  })

  it('saves the edited template and emits the updated channel', async () => {
    const updated = channel({
      config: {
        smtp_host: 'smtp.example.com',
        smtp_port: 587,
        smtp_user: 'user',
        smtp_password: 'pass',
        from_address: 'alerts@example.com',
        to_addresses: ['ops@example.com'],
        security: 'starttls',
        title_template: 'custom title',
        body_template: DEFAULT_BODY_TEMPLATE,
      },
    })
    mockUpdateChannel.mockResolvedValue(updated)

    const wrapper = await mountExpandedWithEditedTitle()
    await findSaveButton(wrapper).trigger('click')
    await flushPromises()

    expect(mockUpdateChannel).toHaveBeenCalledWith(
      1,
      expect.objectContaining({
        config: expect.objectContaining({ title_template: 'custom title' }),
      }),
    )
    expect(wrapper.emitted('updated')?.[0]).toEqual([updated])
  })

  it('disables saving until the template has actually changed', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')
    expect(findSaveButton(wrapper).attributes('disabled')).toBeDefined()
  })

  it('disables saving and warns when the title is blanked out, even if it counts as dirty', async () => {
    const wrapper = await mountExpandedWithEditedTitle('   ')
    expect(findSaveButton(wrapper).attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain("Title and message can't be blank")
  })

  it('disables saving when the message is blanked out', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')
    const bodyInput = wrapper.find<HTMLTextAreaElement>('textarea')
    await bodyInput.setValue('')
    expect(findSaveButton(wrapper).attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain("Title and message can't be blank")
  })

  it('edits the message field directly and inserts a variable into it once focused', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')

    const bodyInput = wrapper.find<HTMLTextAreaElement>('textarea')
    await bodyInput.setValue('Prefix ')
    await bodyInput.trigger('focus')

    const chip = wrapper.findAll('button.content-chip').find((c) => c.text() === '{{host}}')
    await chip!.trigger('click')

    expect(bodyInput.element.value).toBe('Prefix {{host}}')
  })

  it('re-renders the preview for a different sample event', async () => {
    const wrapper = mount()
    await wrapper.find('button.content-toggle').trigger('click')

    expect(wrapper.find('.content-preview-subject').text()).toContain('Backup succeeded')

    const select = wrapper.find<HTMLSelectElement>('select.content-preview-select')
    await select.setValue('backup_failed')

    expect(wrapper.find('.content-preview-subject').text()).toContain('Backup failed')
    expect(wrapper.find('.content-preview-subject').text()).toContain('db-server-02')
  })

  it('shows the error when saving fails', async () => {
    mockUpdateChannel.mockRejectedValue(new Error('network error'))

    const wrapper = await mountExpandedWithEditedTitle()
    await findSaveButton(wrapper).trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toBe('Unknown error')
    expect(wrapper.emitted('updated')).toBeUndefined()
  })
})
