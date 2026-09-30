// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import CronBuilder from './CronBuilder.vue'
import { previewCron } from '../api/schedules'
import { logger } from '../utils/logger'
import type { CronPreviewResponse } from '../types/generated'

// Stands in for the server's validator: anything that is not five fields is
// rejected with the kind of message the server sends.
vi.mock('../api/schedules', () => ({
  previewCron: vi.fn(
    async (expr: string): Promise<CronPreviewResponse> =>
      expr.trim().split(/\s+/).length === 5
        ? {
            status: 'valid',
            next_runs: ['2026-01-01T02:00:00Z', '2026-01-02T02:00:00Z', '2026-01-03T02:00:00Z'],
          }
        : { status: 'invalid', error: 'invalid cron expression: expected 5 fields' },
  ),
}))

const mockPreviewCron = vi.mocked(previewCron)

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: () => 'UTC',
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (expr: string): string => {
    if (expr === '0 2 * * *') return 'Daily at 02:00'
    if (expr === '0 */6 * * *') return 'Every 6 hours'
    return ''
  },
  CRON_ANY: '*',
  CRON_TOP_OF_HOUR: '0',
}))

function mountCronBuilder(modelValue: string): ReturnType<typeof mount> {
  return mount(CronBuilder, {
    props: { modelValue },
  })
}

describe('CronBuilder', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the cron expression input with the given modelValue', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const input = wrapper.find('input.cron-input')
    expect(input.exists()).toBe(true)
    expect(input.attributes('value')).toBe('0 2 * * *')
  })

  it('renders the hint text', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.text()).toContain('minute hour day-of-month month day-of-week')
  })

  it('shows Helper toggle button', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const btn = wrapper.find('button.helper-toggle')
    expect(btn.exists()).toBe(true)
    expect(btn.text()).toBe('Helper')
  })

  it('emits update:modelValue when user types in the input', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const input = wrapper.find('input.cron-input')
    const el = input.element as HTMLInputElement
    el.value = '0 3 * * *'
    await input.trigger('input')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
    expect((emitted as string[][])[0][0]).toBe('0 3 * * *')
  })

  it('shows validation error for invalid cron expression', async () => {
    const wrapper = mountCronBuilder('invalid')
    await flushPromises()
    expect(wrapper.find('.cron-error').exists()).toBe(true)
  })

  it('does not show error for valid 5-field cron', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.find('.cron-error').exists()).toBe(false)
  })

  it('shows human description preview for valid cron', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.text()).toContain('Daily at 02:00')
  })

  it('toggles the helper panel when Helper button is clicked', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.find('.helper-panel').exists()).toBe(false)
    await wrapper.find('button.helper-toggle').trigger('click')
    expect(wrapper.find('.helper-panel').exists()).toBe(true)
    expect(wrapper.find('button.helper-toggle').text()).toBe('Hide Helper')
  })

  it('emits the helper expression when Apply is clicked', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    await wrapper.find('button.helper-apply-btn').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
  })

  it('parses daily cron into helper fields correctly', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    const select = wrapper.find('select.helper-select')
    expect(select.exists()).toBe(true)
    expect((select.element as HTMLSelectElement).value).toBe('daily')
  })

  it('parses hourly cron into helper fields correctly', async () => {
    const wrapper = mountCronBuilder('0 */6 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    const select = wrapper.find('select.helper-select')
    expect((select.element as HTMLSelectElement).value).toBe('hourly')
  })

  describe('server-side preview', () => {
    afterEach(() => {
      vi.useRealTimers()
    })

    const display = (iso: string): string =>
      new Intl.DateTimeFormat(undefined, {
        timeZone: 'UTC',
        weekday: 'short',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }).format(new Date(iso))

    it('shows the next runs the server computed', async () => {
      const wrapper = mountCronBuilder('0 2 * * MON-FRI')
      await flushPromises()
      expect(mockPreviewCron).toHaveBeenCalledWith('0 2 * * MON-FRI')
      const runs = wrapper.findAll('.next-run').map((r) => r.text())
      expect(runs).toHaveLength(3)
      expect(runs[0]).toContain(display('2026-01-01T02:00:00Z'))
      expect(wrapper.find('.cron-error').exists()).toBe(false)
    })

    it("shows the server's error message", async () => {
      const wrapper = mountCronBuilder('60 2 * *')
      await flushPromises()
      expect(wrapper.find('.cron-error').text()).toBe('invalid cron expression: expected 5 fields')
      expect(wrapper.find('.next-runs').exists()).toBe(false)
    })

    it('waits for typing to pause before asking the server again', async () => {
      vi.useFakeTimers()
      const wrapper = mountCronBuilder('0 2 * * *')
      await flushPromises()
      mockPreviewCron.mockClear()

      await wrapper.setProps({ modelValue: '0 3 * * *' })
      await wrapper.setProps({ modelValue: '0 4 * * *' })
      expect(mockPreviewCron).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(300)
      expect(mockPreviewCron).toHaveBeenCalledTimes(1)
      expect(mockPreviewCron).toHaveBeenCalledWith('0 4 * * *')
    })

    it('ignores a response that arrives after a newer one was requested', async () => {
      vi.useFakeTimers()
      let resolveStale: (value: CronPreviewResponse) => void = () => {}
      mockPreviewCron.mockImplementationOnce(
        () => new Promise((resolve) => (resolveStale = resolve)),
      )
      const wrapper = mountCronBuilder('0 2 * * *')

      await wrapper.setProps({ modelValue: 'bad' })
      await vi.advanceTimersByTimeAsync(300)
      await flushPromises()
      resolveStale({ status: 'valid', next_runs: ['2026-01-01T02:00:00Z'] })
      await flushPromises()

      expect(wrapper.find('.cron-error').exists()).toBe(true)
      expect(wrapper.findAll('.next-run')).toHaveLength(0)
    })

    it('does not ask the server about an empty expression', async () => {
      mountCronBuilder('   ')
      await flushPromises()
      expect(mockPreviewCron).not.toHaveBeenCalled()
    })

    it('shows no next runs when the preview request fails', async () => {
      const debug = vi.spyOn(logger, 'debug').mockImplementation(() => {})
      mockPreviewCron.mockRejectedValueOnce(new Error('network down'))
      const wrapper = mountCronBuilder('0 2 * * *')
      await flushPromises()
      expect(wrapper.find('.next-runs').exists()).toBe(false)
      expect(debug).toHaveBeenCalledWith('cron preview failed', expect.any(Error))
    })

    it('stops a pending check when the builder is removed', async () => {
      vi.useFakeTimers()
      const wrapper = mountCronBuilder('0 2 * * *')
      await flushPromises()
      mockPreviewCron.mockClear()
      await wrapper.setProps({ modelValue: '0 3 * * *' })
      wrapper.unmount()
      await vi.advanceTimersByTimeAsync(300)
      expect(mockPreviewCron).not.toHaveBeenCalled()
    })
  })
})
