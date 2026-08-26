// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
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
})
