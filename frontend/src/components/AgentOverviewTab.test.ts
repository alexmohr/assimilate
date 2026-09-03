// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import AgentOverviewTab from './AgentOverviewTab.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'

const AGENT = {
  id: 1,
  hostname: 'web-01',
  is_connected: true,
  is_imported: false,
} as unknown as AgentRow

function report(over: Record<string, unknown> = {}): ReportRow {
  return {
    id: 1,
    repo_id: 10,
    repo_name: 'server-daily',
    schedule_id: 100,
    schedule_name: 'Nightly',
    status: 'success',
    started_at: '2026-06-01T09:00:00Z',
    finished_at: '2026-06-01T10:00:00Z',
    duration_secs: 120,
    original_size: 1024,
    deduplicated_size: 256,
    files_processed: 10,
    error_message: null,
    warnings: [],
    archive_name: 'web-01-2026-06-01',
    ...over,
  } as unknown as ReportRow
}

function schedule(over: Record<string, unknown> = {}): ScheduleRow {
  return {
    id: 100,
    repo_id: 10,
    name: 'Nightly',
    target_hostnames: ['web-01'],
    cron_expression: '0 2 * * *',
    enabled: true,
    next_run_at: '2026-06-02T02:00:00Z',
    last_run_at: '2026-06-01T02:00:00Z',
    ...over,
  } as unknown as ScheduleRow
}

function health(over: Record<string, unknown> = {}): ScheduleHealthEntry {
  return {
    repo_id: 10,
    schedule_id: 100,
    hostname: 'web-01',
    target_name: 'server-daily',
    last_status: 'success',
    last_backup_at: '2026-06-01T02:00:00Z',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    ...over,
  } as ScheduleHealthEntry
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentOverviewTab, {
    props: {
      agent: AGENT,
      repos: [{ id: 10, name: 'server-daily' }],
      schedules: [schedule()],
      health: [health()],
      reports: [report()],
      liveBackups: [],
      cancellingRepoIds: [],
      repoNameFor: () => 'server-daily',
      ...props,
    },
  })
}

function tile(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('.tile').find((t) => t.find('.stat-label').text() === label)
  if (!found) throw new Error(`no tile labelled "${label}"`)
  return found
}

describe('AgentOverviewTab', () => {
  it('answers the four questions the page is opened for', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.stat-label').map((l) => l.text())).toEqual([
      'Last backup',
      'Next run',
      'Repositories',
      'Recent runs',
    ])
  })

  it('reports the last backup and its outcome', () => {
    const last = tile(mount(), 'Last backup')
    expect(last.find('.badge').text()).toBe('success')
    expect(last.find('.stat-sub').text()).toContain('server-daily')
  })

  it('says so when the agent has never run a backup', () => {
    expect(tile(mount({ reports: [] }), 'Last backup').text()).toContain('Never')
  })

  // The server still reports a next_run_at for a disabled schedule, but it
  // will not fire, so counting it would promise a backup that never comes.
  it('ignores disabled schedules when picking the next run', () => {
    const wrapper = mount({ schedules: [schedule({ enabled: false })] })
    expect(tile(wrapper, 'Next run').text()).toContain('None scheduled')
  })

  it('picks the soonest upcoming run across every schedule', () => {
    const wrapper = mount({
      schedules: [
        schedule({ id: 1, name: 'later', next_run_at: '2026-06-05T02:00:00Z' }),
        schedule({ id: 2, name: 'sooner', next_run_at: '2026-06-02T02:00:00Z' }),
      ],
    })
    expect(tile(wrapper, 'Next run').find('.stat-sub').text()).toContain('sooner')
  })

  it('names an unnamed next run by its repository', () => {
    const wrapper = mount({ schedules: [schedule({ name: '' })] })
    expect(tile(wrapper, 'Next run').find('.stat-sub').text()).toContain('server-daily')
  })

  it('counts the repositories the agent backs up to', () => {
    const wrapper = mount({
      repos: [
        { id: 10, name: 'server-daily' },
        { id: 11, name: 'media' },
      ],
    })
    const repos = tile(wrapper, 'Repositories')
    expect(repos.find('.stat-value').text()).toBe('2')
    expect(repos.find('.stat-sub').text()).toBe('server-daily, media')
  })

  describe('needs-attention strip', () => {
    // An always-present "all good" banner trains people to skip the place a
    // real warning will appear.
    it('is absent when there is nothing to say', () => {
      expect(mount().find('.attention').exists()).toBe(false)
    })

    it('names an overdue schedule and links to it', () => {
      const wrapper = mount({ health: [health({ is_overdue: true })] })
      const row = wrapper.find('.attention-row')
      expect(row.text()).toContain('Overdue')
      expect(row.text()).toContain('Nightly')
      expect(row.find('.attention-link').attributes('href')).toBe('/schedules/100')
    })

    it('says how long an offline agent has been gone', () => {
      const wrapper = mount({
        agent: { ...AGENT, is_connected: false, last_seen_at: '2026-06-01T00:00:00Z' },
      })
      expect(wrapper.find('.attention').text()).toContain('last checked in')
    })

    it('says an agent has never checked in when it never has', () => {
      const wrapper = mount({
        agent: { ...AGENT, is_connected: false, last_seen_at: null },
      })
      expect(wrapper.find('.attention').text()).toContain('never checked in')
    })

    // A schedule that has never produced a backup is overdue from the start.
    it('says an overdue schedule has never run when it never has', () => {
      const wrapper = mount({
        health: [health({ is_overdue: true, last_backup_at: null })],
      })
      expect(wrapper.find('.attention').text()).toContain('has never run')
    })

    // An unnamed schedule is identified by what it writes to.
    it('falls back to the target name for an unnamed overdue schedule', () => {
      const wrapper = mount({
        schedules: [schedule({ name: '' })],
        health: [health({ is_overdue: true })],
      })
      expect(wrapper.find('.attention').text()).toContain('server-daily')
    })

    it('reports a failed last backup', () => {
      const wrapper = mount({ reports: [report({ status: 'failed' })] })
      expect(wrapper.find('.attention').text()).toContain('failed')
    })

    it('reports an offline agent', () => {
      const wrapper = mount({ agent: { ...AGENT, is_connected: false } })
      expect(wrapper.find('.attention').text()).toContain('Offline')
    })

    // An imported host has no agent to be offline in the first place.
    it('does not call an imported host offline', () => {
      const wrapper = mount({
        agent: { ...AGENT, is_connected: false, is_imported: true },
      })
      expect(wrapper.find('.attention').exists()).toBe(false)
    })
  })

  describe('previews', () => {
    it('offers a way through to the full list when there is more to see', () => {
      const wrapper = mount({
        schedules: Array.from({ length: 5 }, (_, i) => schedule({ id: i, name: `s${i}` })),
      })
      const link = wrapper.findAll('.section-link').find((l) => l.text().includes('View all 5'))
      expect(link).toBeDefined()
    })

    it('omits the link when the preview already shows everything', () => {
      expect(mount().findAll('.section-link')).toHaveLength(0)
    })

    it('asks the view to switch tabs rather than navigating away', async () => {
      const wrapper = mount({
        schedules: Array.from({ length: 5 }, (_, i) => schedule({ id: i, name: `s${i}` })),
      })
      await wrapper.findAll('.section-link')[0].trigger('click')
      expect(wrapper.emitted('showTab')).toEqual([['schedules']])
    })

    it('asks the view to open a schedule from the preview', async () => {
      const wrapper = mount()
      await wrapper.find('.agent-row-name').trigger('click')
      expect(wrapper.emitted('openSchedule')).toHaveLength(1)
    })

    it('asks the view to open a backup from the preview', async () => {
      const wrapper = mount()
      // Scoped to a backup row: the schedule rows above use the same class.
      await wrapper.find('[id^="report-"] button.agent-row-name').trigger('click')
      expect(wrapper.emitted('openReport')).toHaveLength(1)
    })

    // A failed run in the preview shows a badge and no output; the jump is
    // the only thing on the row that answers "why".
    it('asks the view to open the output of a failed run', async () => {
      const wrapper = mount({
        reports: [
          report({ id: 2, status: 'failed', archive_name: null, error_message: 'Lock held' }),
        ],
      })
      const button = wrapper.findAll('button').find((b) => b.text() === 'View error')
      await button!.trigger('click')
      expect(wrapper.emitted('openReportDetail')).toHaveLength(1)
    })

    it('switches to the backups tab from its View all link', async () => {
      const wrapper = mount({
        reports: Array.from({ length: 8 }, (_, i) => report({ id: i + 1 })),
      })
      const link = wrapper.findAll('.section-link').find((l) => l.text().includes('View all 8'))
      await link!.trigger('click')
      expect(wrapper.emitted('showTab')).toEqual([['backups']])
    })

    // A run still in flight is shown by the progress card above, not as a
    // finished entry in the history below it.
    it('leaves in-flight runs out of the recent backups list', () => {
      const wrapper = mount({
        reports: [report({ id: 2, status: 'started' }), report({ id: 1 })],
      })
      expect(wrapper.findAll('[id^="report-"]')).toHaveLength(1)
    })
  })

  it('leads with a running backup', () => {
    const wrapper = mount({
      liveBackups: [
        {
          targetName: 'server-daily',
          repoId: 10,
          archiveName: 'web-01-now',
          elapsedSecs: 42,
          progress: null,
        },
      ],
    })
    expect(wrapper.findComponent({ name: 'BackupProgressCard' }).exists()).toBe(true)
  })

  it('links the running backup to its repository and forwards cancel', async () => {
    const wrapper = mount({
      liveBackups: [
        {
          targetName: 'server-daily',
          repoId: 10,
          archiveName: 'web-01-now',
          elapsedSecs: 42,
          progress: null,
        },
      ],
    })
    expect(wrapper.find('.live-log-host-badge').attributes('href')).toBe('/repos/10')

    await wrapper.find('.live-log-header-actions button').trigger('click')
    expect(wrapper.emitted('cancelBackup')).toEqual([[10]])
  })
})
