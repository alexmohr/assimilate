// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import AgentBackupRow from './AgentBackupRow.vue'
import type { ReportRow } from '../types/report'

function report(over: Record<string, unknown> = {}): ReportRow {
  return {
    id: 7,
    repo_id: 10,
    repo_name: 'server-daily',
    schedule_id: 100,
    schedule_name: 'Nightly',
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
    ...over,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentBackupRow, { props: { report: report(), ...props } })
}

describe('AgentBackupRow', () => {
  it.each([
    ['success', 'success'],
    ['warning', 'warning'],
    ['failed', 'danger'],
  ])('stripes a %s run as %s', (status, stripe) => {
    const wrapper = mount({ report: report({ status }) })
    expect(wrapper.find(`.agent-row-stripe--${stripe}`).exists()).toBe(true)
  })

  // A run that never reached an outcome is neither good nor bad news, so it
  // takes the neutral stripe rather than borrowing one of the three that mean
  // something.
  it('stripes a cancelled run as neither', () => {
    const wrapper = mount({ report: report({ status: 'cancelled' }) })
    expect(wrapper.find('.agent-row-stripe--muted').exists()).toBe(true)
  })

  it('opens the archive list from a successful run', async () => {
    const wrapper = mount()
    await wrapper.find('button.agent-row-name').trigger('click')
    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  // A failed run has no archive to open, so its name is plain text.
  it('offers nothing to open on a failed run', () => {
    const wrapper = mount({ report: report({ status: 'failed' }) })
    expect(wrapper.find('button.agent-row-name').exists()).toBe(false)
    expect(wrapper.find('span.agent-row-name').text()).toBe('server-daily')
  })

  it('links to the schedule that produced the run', () => {
    expect(mount().find('a.row-schedule-link').attributes('href')).toBe('/schedules/100')
  })

  // Naming the schedule only helps when it says something the repo name does
  // not.
  it('omits the schedule link when it would just repeat the repository', () => {
    const wrapper = mount({ report: report({ schedule_name: 'server-daily' }) })
    expect(wrapper.find('a.row-schedule-link').exists()).toBe(false)
  })

  it('reports size and duration for a completed run', () => {
    const stats = mount().find('.agent-row-stats').text()
    expect(stats).toContain('128 files')
    expect(stats).toContain('dedup')
  })

  // A failed run has no sizes to report, only how long it took to fail.
  it('reports only the duration for a failed run', () => {
    const stats = mount({ report: report({ status: 'failed' }) })
      .find('.agent-row-stats')
      .text()
    expect(stats).not.toContain('dedup')
  })

  describe('detail', () => {
    it('offers no toggle when there is nothing to expand', () => {
      expect(mount({ showDetail: true }).find('button[aria-expanded]').exists()).toBe(false)
    })

    it('asks the parent to expand a failed run', async () => {
      const wrapper = mount({
        report: report({ status: 'failed', error_message: 'Connection refused' }),
        showDetail: true,
      })
      await wrapper.find('button[aria-expanded]').trigger('click')
      expect(wrapper.emitted('toggle')).toHaveLength(1)
    })

    it('shows the error output when expanded', () => {
      const wrapper = mount({
        report: report({ status: 'failed', error_message: 'Connection refused' }),
        showDetail: true,
        expanded: true,
      })
      expect(wrapper.find('.detail-output--danger').text()).toContain('Connection refused')
    })

    // A warned run carries the same message in `error_message`, and rendering
    // both would print it twice.
    it('shows warnings without repeating them as an error', () => {
      const wrapper = mount({
        report: report({
          status: 'warning',
          warnings: ['file changed while we backed it up'],
          error_message: 'file changed while we backed it up',
        }),
        showDetail: true,
        expanded: true,
      })
      expect(wrapper.find('.detail-label--warning').exists()).toBe(true)
      expect(wrapper.find('.detail-label--danger').exists()).toBe(false)
    })

    // The Overview preview is a summary, not a log reader.
    it('hides the toggle in preview mode', () => {
      const wrapper = mount({
        report: report({ status: 'failed', error_message: 'Connection refused' }),
      })
      expect(wrapper.find('button[aria-expanded]').exists()).toBe(false)
    })
  })
})
