// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { flushPromises } from '@vue/test-utils'
import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import { SRC } from '../test-utils/vueFiles'
import BackupCalendar from './BackupCalendar.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockResolvedValue({ data: [] }),
  },
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

describe('BackupCalendar', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows loading state initially', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Loading')
  })

  it('displays the panel title', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Backup calendar')
  })

  it('renders repo options in select', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: {
        repos: [
          { id: 1, name: 'daily-backups' },
          { id: 2, name: 'weekly-archive' },
        ],
      },
    })
    expect(wrapper.text()).toContain('daily-backups')
    expect(wrapper.text()).toContain('weekly-archive')
  })

  it('sizes the seven day columns so none can be clipped away', () => {
    // A plain `repeat(7, 1fr)` track will not shrink below its content's
    // min-content width, so a busy day widened the whole grid past the panel
    // and the clip took Saturday with it. `minmax(0, 1fr)` lets the columns
    // fit the panel and confines any overflow to the cell that caused it.
    const css = readFileSync(join(SRC, 'components', 'BackupCalendar.vue'), 'utf-8')
    expect(css).toContain('grid-template-columns: repeat(7, minmax(0, 1fr))')
    expect(css).not.toMatch(/grid-template-columns:\s*repeat\(7,\s*1fr\)/)
  })

  it('renders navigation buttons for month switching', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    const buttons = wrapper.findAll('button')
    expect(buttons.length).toBeGreaterThanOrEqual(2)
  })

  describe('failed run details', () => {
    // A day in the month the calendar opens on, so the grid renders it.
    function today(): string {
      const d = new Date()
      const mm = String(d.getMonth() + 1).padStart(2, '0')
      const dd = String(d.getDate()).padStart(2, '0')
      return `${d.getFullYear()}-${mm}-${dd}`
    }

    async function openFailedRun(): Promise<ReturnType<typeof renderWithPlugins>> {
      vi.mocked(apiClient.get).mockResolvedValueOnce({
        data: [
          {
            date: today(),
            events: [
              {
                repo_id: 3,
                repo_name: 'nightly',
                schedule_id: 9,
                time: '02:00',
                status: 'failed',
                error_message: 'Connection closed by remote host',
              },
            ],
          },
        ],
      })
      const wrapper = renderWithPlugins(BackupCalendar, { props: { repos: [] } })
      await flushPromises()
      await wrapper.find('.cal-cell-has-events').trigger('click')
      await wrapper.find('.cal-event-clickable').trigger('click')
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
      return wrapper
    }

    it('opens the error in an accessible dialog', async () => {
      const wrapper = await openFailedRun()
      const dialog = wrapper.find('[role="dialog"]')
      expect(dialog.exists()).toBe(true)
      expect(dialog.attributes('aria-modal')).toBe('true')
      const titleId = dialog.attributes('aria-labelledby')
      expect(titleId).toBeTruthy()
      expect(wrapper.find(`[id="${titleId}"]`).text()).toBe('Backup failed')
      expect(dialog.text()).toContain('nightly')
      expect(dialog.text()).toContain('02:00')
      expect(wrapper.find('.cal-error-msg').text()).toBe('Connection closed by remote host')
    })

    it('closes on Escape', async () => {
      const wrapper = await openFailedRun()
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
      await flushPromises()
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
    })

    it('closes from the close button', async () => {
      const wrapper = await openFailedRun()
      await wrapper.find('button[aria-label="Close"]').trigger('click')
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
    })

    it('closes on a click on the backdrop but not inside the dialog', async () => {
      const wrapper = await openFailedRun()
      await wrapper.find('.cal-error-msg').trigger('mousedown')
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
      await wrapper.find('.modal-backdrop').trigger('mousedown')
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
    })
  })
})
