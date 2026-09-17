// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'
import { renderWithPlugins } from '../test-utils'
import { mockErrorUtils, mockFormatBytes } from '../test-utils/sharedMocks'
import QuotaPanel from './QuotaPanel.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
  },
}))

vi.mock('../utils/format', () => mockFormatBytes())

vi.mock('../utils/error', () => mockErrorUtils())

// Stubbed to the real component's contract - `label`, `aria-checked` and an
// `update:modelValue` on click - so a test can actually drive the switch. The
// previous stub was a bare checkbox that took neither the label nor a click,
// which is why nothing here had ever exercised the enabled toggle.
vi.mock('./ToggleSwitch.vue', () => ({
  default: {
    props: ['modelValue', 'label'],
    emits: ['update:modelValue'],
    template:
      '<button type="button" role="switch" :aria-checked="String(modelValue)" :aria-label="label" @click="$emit(\'update:modelValue\', !modelValue)" />',
  },
}))

const mockGet = vi.mocked(apiClient.get)

describe('QuotaPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  /** Seeds a quota, mounts as an admin and opens the edit form. */
  async function openEditForm(): Promise<{
    wrapper: ReturnType<typeof renderWithPlugins>
    mockPut: ReturnType<typeof vi.mocked<typeof apiClient.put>>
  }> {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 1_073_741_824,
        critical_bytes: 2_147_483_648,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const mockPut = vi.mocked(apiClient.put)
    mockPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()

    await wrapper.find('button.btn-ghost').trigger('click')
    await nextTick()
    return { wrapper, mockPut }
  }

  it('shows loading state initially', async () => {
    mockGet.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await nextTick()
    expect(wrapper.text()).toContain('Loading quota')
  })

  it('renders quota bar and usage in normal (ok) state', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 1_073_741_824 },
    })
    await flushPromises()
    expect(wrapper.find('.progress-bar-fill').exists()).toBe(true)
    expect(wrapper.find('.bar-ok').exists()).toBe(true)
    expect(wrapper.find('.badge-ok').exists()).toBe(true)
    expect(wrapper.text()).toContain('OK')
  })

  it('renders warning state when usage exceeds warn threshold', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 1_073_741_824,
        critical_bytes: 10_737_418_240,
        warn_action: 'block_backups',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 5_368_709_120 },
    })
    await flushPromises()
    expect(wrapper.find('.bar-warn').exists()).toBe(true)
    expect(wrapper.find('.badge-warn').exists()).toBe(true)
    expect(wrapper.text()).toContain('Warning')
    expect(wrapper.text()).toContain('Block backups')
  })

  it('renders critical state when usage exceeds critical threshold', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 1_073_741_824,
        critical_bytes: 5_368_709_120,
        warn_action: 'notify_only',
        critical_action: 'disable_schedule',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 6_442_450_944 },
    })
    await flushPromises()
    expect(wrapper.find('.bar-crit').exists()).toBe(true)
    expect(wrapper.find('.badge-crit').exists()).toBe(true)
    expect(wrapper.text()).toContain('Critical')
    expect(wrapper.text()).toContain('Disable schedule')
  })

  it('shows disabled message when quota is not enabled', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 0,
        critical_bytes: 0,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: false,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('disabled')
  })

  it('shows Edit button for admin users', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 0,
        critical_bytes: 0,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Edit')
  })

  it('does not show Edit button for non-admin users', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 0,
        critical_bytes: 0,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).not.toContain('Edit')
  })

  it('saves selected quota actions when editing', async () => {
    const { wrapper, mockPut } = await openEditForm()

    const selects = wrapper.findAll('select')
    await selects[0]?.setValue('block_backups')
    await selects[1]?.setValue('disable_schedule')

    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      '/repos/1/quota',
      expect.objectContaining({
        warn_action: 'block_backups',
        critical_action: 'disable_schedule',
      }),
    )
  })

  it('round-trips the enabled toggle through the edit form', async () => {
    const { wrapper, mockPut } = await openEditForm()

    // Driven through the rendered switch rather than the component instance:
    // that is what a user clicks, and it survives the row markup changing.
    const enabled = wrapper.find('button[role="switch"][aria-label="Enabled"]')
    expect(enabled.attributes('aria-checked')).toBe('true')
    await enabled.trigger('click')

    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      '/repos/1/quota',
      expect.objectContaining({ enabled: false }),
    )
  })

  it('shows error message when API fails', async () => {
    mockGet.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })

  // A repository with no quota row answers 404, which is an empty state rather
  // than an error - the panel has to tell those two apart, since `error` and
  // the "not configured" branch render different things.
  it('treats a 404 as no quota configured rather than an error', async () => {
    mockGet.mockRejectedValue({ response: { status: 404 } })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('No quota configured for this repository.')
    expect(wrapper.text()).not.toContain('API error')
    expect(wrapper.find('.quota-empty-action').exists()).toBe(true)
  })

  it('hides the configure action from a non-admin on an unconfigured repository', async () => {
    mockGet.mockRejectedValue({ response: { status: 404 } })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('No quota configured for this repository.')
    expect(wrapper.find('.quota-empty-action').exists()).toBe(false)
  })

  // The first quota for a repository starts from defaults rather than from an
  // existing row, so this path fills the form itself.
  it('opens the form on defaults when configuring a first quota', async () => {
    mockGet.mockRejectedValue({ response: { status: 404 } })
    const mockPut = vi.mocked(apiClient.put)
    mockPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()

    await wrapper.find('.quota-empty-action').trigger('click')
    await nextTick()

    expect(
      wrapper.find('button[role="switch"][aria-label="Enabled"]').attributes('aria-checked'),
    ).toBe('true')
    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith('/repos/1/quota', {
      warn_bytes: 0,
      critical_bytes: 0,
      warn_action: 'notify_only',
      critical_action: 'notify_only',
      enabled: true,
    })
  })

  // The two GB fields are the only inputs that convert on the way out, and
  // `v-model.number`'s cast only runs on a real input event - so typing into
  // them is the only thing that exercises the GB-to-bytes round trip.
  it('converts typed GB thresholds to bytes on save', async () => {
    const { wrapper, mockPut } = await openEditForm()

    await wrapper.find('input[aria-label="Warning (GB)"]').setValue('3')
    await wrapper.find('input[aria-label="Critical (GB)"]').setValue('4.5')
    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      '/repos/1/quota',
      expect.objectContaining({
        warn_bytes: 3 * 1024 ** 3,
        critical_bytes: 4.5 * 1024 ** 3,
      }),
    )
  })

  it('closes the form without saving when the edit is cancelled', async () => {
    const { wrapper, mockPut } = await openEditForm()

    await wrapper.find('button[role="switch"][aria-label="Enabled"]').trigger('click')
    await wrapper.find('.edit-actions .btn-ghost').trigger('click')
    await nextTick()

    expect(wrapper.find('button[role="switch"][aria-label="Enabled"]').exists()).toBe(false)
    expect(mockPut).not.toHaveBeenCalled()
  })

  // A failed save keeps the form open with its error, rather than closing and
  // losing what was typed.
  it('keeps the form open and reports the error when saving fails', async () => {
    const { wrapper, mockPut } = await openEditForm()
    mockPut.mockRejectedValue(new Error('nope'))

    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toBe('API error')
    expect(wrapper.find('button.btn-primary').exists()).toBe(true)
  })

  it('discloses the space limits explanation on demand', async () => {
    mockGet.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await wrapper.find('[aria-label="Help: space limits"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toBe(
      'How much space this repository may use, and what Assimilate does as it fills up.',
    )
  })
})
