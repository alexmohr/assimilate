// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RunLogTab from './RunLogTab.vue'
import type { ReportRow } from '../types/report'

vi.mock('../api/runs', () => ({
  getRunEvents: vi.fn().mockResolvedValue([]),
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({ onMessage: (): void => undefined }),
}))

function report(overrides: Partial<ReportRow>): ReportRow {
  return {
    id: 1,
    agent_id: 5,
    repo_id: 10,
    repo_name: 'server-daily',
    schedule_id: null,
    schedule_name: null,
    status: 'success',
    started_at: '2026-06-01T09:00:00Z',
    finished_at: '2026-06-01T10:00:00Z',
    duration_secs: 221,
    original_size: 1024,
    deduplicated_size: 256,
    files_processed: 128,
    error_message: null,
    warnings: [],
    archive_name: 'web-01-2026-06-01',
    run_id: null,
    ...overrides,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RunLogTab, {
    props: {
      reports: [report({ id: 1 }), report({ id: 2, status: 'failed' })],
      total: 2,
      loadingMore: false,
      filter: 'all',
      sortAscending: false,
      expandedReportId: null,
      highlightedArchiveName: undefined,
      pinnedReportId: null,
      ...props,
    },
  })
}

describe('RunLogTab', () => {
  it('counts each status against the loaded reports', () => {
    const wrapper = mount()
    const text = wrapper.text()
    expect(text).toContain('All 2')
    expect(text).toContain('Success 1')
    expect(text).toContain('Failed 1')
  })

  it('hides the load-more button once every report is loaded', () => {
    const wrapper = mount({ total: 2 })
    expect(wrapper.find('.load-more-row button').exists()).toBe(false)
    expect(wrapper.text()).toContain('Showing 2 of 2 runs')
  })

  it('offers to load more when the total exceeds what is loaded, capped at 50', () => {
    const wrapper = mount({ total: 312 })
    const button = wrapper.find('.load-more-row button')
    expect(button.exists()).toBe(true)
    expect(button.text()).toBe('Load 50 more')
    expect(wrapper.text()).toContain('Showing 2 of 312 runs')
  })

  it('offers the exact remainder when fewer than a full page is left', () => {
    const wrapper = mount({ total: 5 })
    expect(wrapper.find('.load-more-row button').text()).toBe('Load 3 more')
  })

  it('emits loadMore when the button is clicked', async () => {
    const wrapper = mount({ total: 312 })
    await wrapper.find('.load-more-row button').trigger('click')
    expect(wrapper.emitted('loadMore')).toHaveLength(1)
  })

  it('disables the button and shows a loading label while a page is in flight', () => {
    const wrapper = mount({ total: 312, loadingMore: true })
    const button = wrapper.find('.load-more-row button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Loading...')
  })

  it('shows nothing below the rows when there are none loaded yet', () => {
    const wrapper = mount({ reports: [], total: 0 })
    expect(wrapper.find('.load-more-row').exists()).toBe(false)
  })
})
