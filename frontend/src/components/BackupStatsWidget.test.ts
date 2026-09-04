// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import BackupStatsWidget from './BackupStatsWidget.vue'
import { apiClient } from '../api/client'

// jscpd:ignore-start -- test setup boilerplate (vi.mock factories cannot reference module-scoped helpers)
vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))
// jscpd:ignore-end

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }))
vi.mock('../composables/useToast', () => ({
  useToast: (): { success: typeof toast.success; error: typeof toast.error } => toast,
}))

const mockGet = vi.mocked(apiClient.get)
const mockPost = vi.mocked(apiClient.post)

function activityEntry(id: number, status: string, acknowledged = false): Record<string, unknown> {
  return {
    id,
    hostname: `h${id}`,
    target_name: `t${id}`,
    started_at: '',
    finished_at: '',
    status,
    duration_secs: 10,
    acknowledged,
  }
}

/**
 * The widget reads two endpoints - the activity feed it renders and the count
 * of what a reset would clear - so the mock answers by URL rather than handing
 * the same payload to both.
 */
function mockApi(entries: Record<string, unknown>[], outstandingReports = 0): void {
  mockGet.mockImplementation((url: string) => {
    if (url.startsWith('/stats/activity/outstanding')) {
      return Promise.resolve({
        data: { backup_reports: outstandingReports, system_events: 0 },
      })
    }
    return Promise.resolve({ data: entries })
  })
  mockPost.mockResolvedValue({ data: { backup_reports: outstandingReports, system_events: 0 } })
}

/** The activity-feed calls only, so an outstanding probe cannot mask one. */
function activityCalls(): string[] {
  return mockGet.mock.calls
    .map((call) => String(call[0]))
    .filter((url) => url.startsWith('/stats/activity?'))
}

describe('BackupStatsWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    mockApi([])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('displays the success rate percentage', async () => {
    mockApi([activityEntry(1, 'success'), activityEntry(2, 'success'), activityEntry(3, 'failed')])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('67%')
  })

  it('displays failed count', async () => {
    mockApi([activityEntry(1, 'success'), activityEntry(2, 'failed'), activityEntry(3, 'failed')])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('2')
  })

  // The range buttons drive the query, so a click has to reach the fetch -
  // a control that only repaints itself looks like it worked and does not.
  it('refetches over the chosen range', async () => {
    mockApi([])
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()
    expect(activityCalls().at(-1)).toContain('days=30')

    await wrapper
      .findAll('.segmented-option')
      .find((b) => b.text() === '7d')!
      .trigger('click')
    await flushPromises()

    expect(activityCalls().at(-1)).toContain('days=7')
  })

  it('shows 0% when no backups have run', async () => {
    mockApi([])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('0%')
  })

  it('navigates to activity, filtered by status, when a mini-stat is clicked', async () => {
    mockApi([])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()

    const links = wrapper.findAll('.mini-stat-link')
    await links[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.name).toBe('activity')
    expect(wrapper.vm.$route.query.status).toBeUndefined()

    await links[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.query.status).toBe('success')

    await links[2]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.query.status).toBe('failed')
  })

  it('refetches stats for the chosen repo', async () => {
    mockApi([])
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [{ id: 4, name: 'repo-beta' }] },
    })
    await flushPromises()

    await wrapper.find('select').setValue('4')
    await flushPromises()

    expect(activityCalls().at(-1)).toContain('repo_id=4')
  })

  // The whole point of marking runs reviewed: a failure somebody has looked at
  // stops counting against the tile, without leaving the history.
  it('counts only unreviewed failures, and says how many were reviewed', async () => {
    mockApi([
      activityEntry(1, 'failed'),
      activityEntry(2, 'failed', true),
      activityEntry(3, 'failed', true),
      activityEntry(4, 'success'),
    ])
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    const failedTile = wrapper.findAll('.mini-stat')[2]!
    expect(failedTile.find('.stat-value').text()).toBe('1')
    expect(failedTile.text()).toContain('2 reviewed')
  })

  // A warned run is acknowledgeable, so the reset clears it - the panel has to
  // account for it, or the button appears over a tile reading zero with
  // nothing on screen explaining what it would mark reviewed.
  it('names the warnings still awaiting review beside the failed count', async () => {
    mockApi(
      [
        activityEntry(1, 'warning'),
        activityEntry(2, 'warning'),
        activityEntry(3, 'warning', true),
        activityEntry(4, 'failed', true),
        activityEntry(5, 'success'),
      ],
      2,
    )
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    const failedTile = wrapper.findAll('.mini-stat')[2]!
    expect(failedTile.find('.stat-value').text()).toBe('0')
    expect(failedTile.find('.stat-sub').text()).toContain('2 warned')
    // The reviewed side counts the acknowledged warning as well as the
    // acknowledged failure - both were looked at, and the reset that retires
    // them says it covers "failed and warned runs".
    expect(failedTile.find('.stat-sub').text()).toContain('2 reviewed')
    expect(wrapper.find('.stats-actions .btn').exists()).toBe(true)
  })

  // The state the panel lands in once the reset has run: nothing left warned,
  // and every run it retired credited as reviewed rather than just gone.
  it('credits reviewed warnings once nothing is left awaiting review', async () => {
    mockApi([
      activityEntry(1, 'warning', true),
      activityEntry(2, 'warning', true),
      activityEntry(3, 'failed', true),
      activityEntry(4, 'success'),
    ])
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    const failedTile = wrapper.findAll('.mini-stat')[2]!
    expect(failedTile.find('.stat-value').text()).toBe('0')
    expect(failedTile.find('.stat-sub').text()).toBe('3 reviewed')
  })

  it('offers the reset only when the server says something is outstanding', async () => {
    mockApi([activityEntry(1, 'failed')], 0)
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()
    expect(wrapper.find('.stats-actions').exists()).toBe(false)

    mockApi([activityEntry(1, 'failed')], 3)
    const withOutstanding = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()
    expect(withOutstanding.find('.stats-actions .btn').text()).toContain('Mark reviewed')
  })

  it('acknowledges only the repo and range on screen, then rereads both counts', async () => {
    mockApi([activityEntry(1, 'failed')], 1)
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [{ id: 4, name: 'repo-beta' }] },
    })
    await flushPromises()
    await wrapper.find('select').setValue('4')
    await flushPromises()

    await wrapper.find('.stats-actions .btn').trigger('click')
    await flushPromises()
    const confirm = wrapper.findAll('.modal-footer .btn').find((b) => b.text() === 'Mark reviewed')!
    const callsBefore = activityCalls().length

    await confirm.trigger('click')
    await flushPromises()

    expect(mockPost).toHaveBeenCalledWith('/stats/activity/acknowledge-all', undefined, {
      params: { days: 30, repo_id: 4 },
    })
    expect(activityCalls().length).toBeGreaterThan(callsBefore)
    expect(mockGet).toHaveBeenLastCalledWith('/stats/activity/outstanding', {
      params: { days: 30, repo_id: 4 },
    })
  })

  // Both ways out of the dialog without acting: the footer's Cancel and the
  // modal's own close. Neither may acknowledge anything on the way.
  it('leaves everything outstanding when the dialog is cancelled', async () => {
    mockApi([activityEntry(1, 'failed')], 1)
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    await wrapper.find('.stats-actions .btn').trigger('click')
    await flushPromises()
    const cancel = wrapper.findAll('.modal-footer .btn').find((b) => b.text() === 'Cancel')!
    await cancel.trigger('click')
    await flushPromises()

    expect(wrapper.find('.modal-footer').exists()).toBe(false)
    expect(mockPost).not.toHaveBeenCalled()
    expect(wrapper.find('.stats-actions .btn').exists()).toBe(true)
  })

  it('closes the dialog when the modal itself asks to close', async () => {
    mockApi([activityEntry(1, 'failed')], 1)
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    await wrapper.find('.stats-actions .btn').trigger('click')
    await flushPromises()
    await wrapper.find('.modal-close').trigger('click')
    await flushPromises()

    expect(wrapper.find('.modal-footer').exists()).toBe(false)
    expect(mockPost).not.toHaveBeenCalled()
  })

  // The runs are acknowledged by the time the panel re-reads itself, so a
  // failed re-read is a stale panel, not a failed reset - saying otherwise
  // right after the success toast would contradict what just happened.
  it('does not report a failed refresh as a failed reset', async () => {
    mockApi([activityEntry(1, 'failed')], 1)
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    await wrapper.find('.stats-actions .btn').trigger('click')
    await flushPromises()
    // Every read from here on fails, including the two the reset triggers.
    mockGet.mockRejectedValue(new Error('network'))
    const confirm = wrapper.findAll('.modal-footer .btn').find((b) => b.text() === 'Mark reviewed')!
    await confirm.trigger('click')
    await flushPromises()

    expect(toast.success).toHaveBeenCalledTimes(1)
    expect(toast.error).not.toHaveBeenCalled()
    expect(wrapper.find('.modal-footer').exists()).toBe(false)
  })

  it('keeps the dialog open and reports the failure when the acknowledge fails', async () => {
    mockApi([activityEntry(1, 'failed')], 1)
    mockPost.mockRejectedValue(new Error('nope'))
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()

    await wrapper.find('.stats-actions .btn').trigger('click')
    await flushPromises()
    const confirm = wrapper.findAll('.modal-footer .btn').find((b) => b.text() === 'Mark reviewed')!
    await confirm.trigger('click')
    await flushPromises()

    expect(wrapper.find('.modal-footer').exists()).toBe(true)
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(toast.success).not.toHaveBeenCalled()
  })
})
