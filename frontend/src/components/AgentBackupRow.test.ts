// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { getRunEvents } from '../api/runs'
import AgentBackupRow from './AgentBackupRow.vue'
import type { ReportRow } from '../types/report'

vi.mock('../api/runs', () => ({
  getRunEvents: vi.fn(),
}))

// Captured WebSocket message handlers - populated during component setup().
// One row is mounted per test in this file, so a single-handler-per-type
// map (rather than a list) is enough, matching AgentDetailView.test.ts's
// mock.
const wsHandlers: Record<string, (payload: unknown) => void> = {}

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

function report(over: Record<string, unknown> = {}): ReportRow {
  return {
    id: 7,
    agent_id: 5,
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
    run_id: null,
    ...over,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentBackupRow, { props: { report: report(), ...props } })
}

describe('AgentBackupRow', () => {
  beforeEach(() => {
    vi.mocked(getRunEvents).mockReset()
    vi.mocked(getRunEvents).mockResolvedValue([])
    for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
  })

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

  // A warned run still produced an archive, so it should be just as
  // browsable as a successful one - the run's status only decides whether
  // detail is also offered.
  it('opens the archive list from a warned run that reached an archive', async () => {
    const wrapper = mount({ report: report({ status: 'warning' }) })
    await wrapper.find('button.agent-row-name').trigger('click')
    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  // A run that never reached the archive step has nothing to open.
  it('offers nothing to open on a run with no archive', () => {
    const wrapper = mount({ report: report({ status: 'failed', archive_name: null }) })
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

  // The wire type allows warnings to be absent rather than an empty list.
  it('treats a report with no warnings field as having none', () => {
    const wrapper = mount({ report: report({ warnings: null }), showDetail: true })
    expect(wrapper.find('button[aria-expanded]').exists()).toBe(false)
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
      expect(wrapper.find('.group-label--warning').exists()).toBe(true)
      expect(wrapper.find('.group-label--danger').exists()).toBe(false)
    })

    // The Overview preview is a summary, not a log reader.
    it('hides the toggle in preview mode', () => {
      const wrapper = mount({
        report: report({ status: 'failed', error_message: 'Connection refused' }),
      })
      expect(wrapper.find('button[aria-expanded]').exists()).toBe(false)
    })
  })

  // A run tied to a power-management timeline is worth offering to expand
  // even when it succeeded with no warnings - most runs have wake/start
  // disabled and simply won't have recorded anything, but a viewer can't
  // know that without a way to look.
  describe('power management timeline', () => {
    it('offers the toggle for a clean run that has a run_id', () => {
      const wrapper = mount({ report: report({ run_id: 'run-123' }), showDetail: true })
      expect(wrapper.find('button[aria-expanded]').exists()).toBe(true)
    })

    it('does not fetch events until the row is expanded', () => {
      mount({ report: report({ run_id: 'run-123' }), showDetail: true, expanded: false })
      expect(getRunEvents).not.toHaveBeenCalled()
    })

    it('fetches and renders the timeline once expanded', async () => {
      vi.mocked(getRunEvents).mockResolvedValue([
        {
          id: 1,
          run_id: 'run-123',
          target: 'source',
          event_type: 'wake_sent',
          message: 'Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44',
          occurred_at: '2026-06-01T09:00:00Z',
        },
      ])
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      expect(getRunEvents).toHaveBeenCalledWith('run-123', 5, 10)
      expect(wrapper.text()).toContain('Sent Wake-on-LAN packet')
    })

    // Most runs have wake/start disabled and record no power-management
    // events at all - that must not read as "the toggle does nothing", since
    // the toggle offers itself on every run with a run_id (the vast
    // majority), not just power-managed ones.
    it('shows an empty-state message rather than nothing when a run recorded no events', async () => {
      vi.mocked(getRunEvents).mockResolvedValue([])
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      expect(wrapper.text()).toContain('No power-management activity for this run.')
      expect(wrapper.findComponent({ name: 'RunEventTimeline' }).exists()).toBe(false)
    })

    // The docs promise the timeline updates live while the row is open, not
    // just on expand - a step recorded after the initial (possibly empty)
    // fetch must still show up without collapsing and re-expanding the row.
    it('appends a live RunEvent for this run once the initial fetch has completed', async () => {
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()
      expect(wrapper.text()).toContain('No power-management activity for this run.')

      wsHandlers['RunEvent']?.({
        run_id: 'run-123',
        agent_id: 5,
        repo_id: 10,
        target: 'source',
        event_type: 'wake_sent',
        message: 'Sent Wake-on-LAN packet',
        occurred_at: '2026-06-01T09:00:00Z',
      })
      await flushPromises()

      expect(wrapper.text()).toContain('Sent Wake-on-LAN packet')
      expect(wrapper.text()).not.toContain('No power-management activity for this run.')
    })

    it('ignores a live RunEvent for a different run', async () => {
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      wsHandlers['RunEvent']?.({
        run_id: 'some-other-run',
        agent_id: 5,
        repo_id: 10,
        target: 'source',
        event_type: 'wake_sent',
        message: 'Sent Wake-on-LAN packet',
        occurred_at: '2026-06-01T09:00:00Z',
      })
      await flushPromises()

      expect(wrapper.text()).toContain('No power-management activity for this run.')
    })

    // run_id is shared across every target of a multi-target schedule (e.g.
    // the same agent backing up to two different repos in one schedule
    // tick), so run_id alone can't tell this row's events apart from a
    // sibling target's that happens to share it.
    it('ignores a live RunEvent for the same run but a different target pairing', async () => {
      const wrapper = mount({
        report: report({ run_id: 'run-123', agent_id: 5, repo_id: 10 }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      wsHandlers['RunEvent']?.({
        run_id: 'run-123',
        agent_id: 5,
        repo_id: 99, // a different repo on the same agent, same run_id
        target: 'repository',
        event_type: 'wake_sent',
        message: 'Sent Wake-on-LAN packet',
        occurred_at: '2026-06-01T09:00:00Z',
      })
      await flushPromises()

      expect(wrapper.text()).toContain('No power-management activity for this run.')
    })

    it('merges a live RunEvent that arrives before the initial fetch resolves, instead of losing it', async () => {
      let resolveFetch: (events: []) => void = () => {}
      vi.mocked(getRunEvents).mockReturnValue(
        new Promise((resolve) => {
          resolveFetch = resolve
        }),
      )
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })

      wsHandlers['RunEvent']?.({
        run_id: 'run-123',
        agent_id: 5,
        repo_id: 10,
        target: 'source',
        event_type: 'wake_sent',
        message: 'Sent Wake-on-LAN packet',
        occurred_at: '2026-06-01T09:00:00Z',
      })
      resolveFetch([])
      await flushPromises()

      // The event that arrived mid-fetch is buffered and merged in once the
      // fetch resolves, rather than being silently lost because it arrived
      // just before the (now-stale) empty response.
      expect(wrapper.text()).toContain('Sent Wake-on-LAN packet')
      expect(wrapper.text()).not.toContain('No power-management activity for this run.')
    })

    it('drops a live RunEvent for a row that has never been expanded', async () => {
      mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: false,
      })

      // Must not throw or accumulate state for a row that never fetches -
      // a future expand's own fetch would return the full history anyway.
      expect(() =>
        wsHandlers['RunEvent']?.({
          run_id: 'run-123',
          agent_id: 5,
          repo_id: 10,
          target: 'source',
          event_type: 'wake_sent',
          message: 'Sent Wake-on-LAN packet',
          occurred_at: '2026-06-01T09:00:00Z',
        }),
      ).not.toThrow()
    })

    it('shows an error state rather than silently hiding the block when the fetch fails', async () => {
      vi.mocked(getRunEvents).mockRejectedValue(new Error('network error'))
      const wrapper = mount({
        report: report({ run_id: 'run-123' }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      expect(getRunEvents).toHaveBeenCalledWith('run-123', 5, 10)
      expect(wrapper.text()).toContain('Power management')
      expect(wrapper.text()).toContain("Couldn't load power-management activity for this run.")
      expect(wrapper.findComponent({ name: 'RunEventTimeline' }).exists()).toBe(false)
    })

    it('does not render a timeline section for a run with no run_id', async () => {
      const wrapper = mount({
        report: report({ status: 'failed', error_message: 'boom', run_id: null }),
        showDetail: true,
        expanded: true,
      })
      await flushPromises()

      expect(getRunEvents).not.toHaveBeenCalled()
      expect(wrapper.findComponent({ name: 'RunEventTimeline' }).exists()).toBe(false)
    })
  })
})
