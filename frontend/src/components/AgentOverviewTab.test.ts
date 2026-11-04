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
  const found = wrapper.findAll('.fact').find((t) => t.find('.stat-label').text() === label)
  if (!found) throw new Error(`no fact labelled "${label}"`)
  return found
}

describe('AgentOverviewTab', () => {
  it('shows the last and next run beside the verdict, and nothing previewed from other tabs', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.stat-label').map((l) => l.text())).toEqual(['Last backup', 'Next run'])
    expect(wrapper.find('.verdict').exists()).toBe(true)
    // Schedules and archives have tabs of their own.
    expect(wrapper.findAll('.section-title').map((t) => t.text())).toEqual(['Repositories'])
    expect(wrapper.findAll('[id^="report-"]')).toHaveLength(0)
    expect(wrapper.find('[data-schedule-id]').exists()).toBe(false)
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

  describe('verdict', () => {
    it('says all is healthy when nothing needs attention', () => {
      const verdict = mount().find('.verdict')
      expect(verdict.classes()).toContain('verdict--success')
      expect(verdict.text()).toContain('All backups are healthy')
      expect(verdict.text()).toContain('All 1 recent runs were clean')
    })

    // "All good" worn by an agent that never ran anything would be a lie.
    it('is neutral rather than healthy for an agent that has never run', () => {
      const verdict = mount({ reports: [], health: [] }).find('.verdict')
      expect(verdict.classes()).toContain('verdict--neutral')
      expect(verdict.text()).toContain('No backups yet')
    })

    it('counts warnings among the recent runs', () => {
      const verdict = mount({
        reports: [report({ id: 2, status: 'warning' }), report({ id: 1 })],
      }).find('.verdict')
      expect(verdict.text()).toContain('1 of the last 2 runs finished with warnings')
    })

    it('turns red when a failure is among the things to report', () => {
      const wrapper = mount({ reports: [report({ status: 'failed' })] })
      expect(wrapper.find('.verdict').classes()).toContain('verdict--danger')
    })

    it('counts the things that need attention when there are several', () => {
      const wrapper = mount({
        agent: { ...AGENT, is_connected: false },
        health: [health({ is_overdue: true })],
      })
      expect(wrapper.find('.verdict-title').text()).toBe('2 things need attention')
    })
  })

  describe('repositories', () => {
    it('lists one row per repository, linking to it', () => {
      const wrapper = mount({
        repos: [
          { id: 10, name: 'server-daily' },
          { id: 11, name: 'media' },
        ],
      })
      const rows = wrapper.findAll('.repo-row')
      expect(rows.map((r) => r.find('.repo-name').text())).toEqual(['server-daily', 'media'])
      expect(rows[1].attributes('href')).toBe('/repos/11')
    })

    it('reports each repository on its own, not the agent as a whole', () => {
      const wrapper = mount({
        repos: [
          { id: 10, name: 'server-daily' },
          { id: 11, name: 'media' },
        ],
        reports: [report({ id: 2, repo_id: 11, status: 'failed' }), report({ id: 1, repo_id: 10 })],
      })
      const states = wrapper.findAll('.repo-row .badge').map((b) => b.text())
      expect(states).toEqual(['Healthy', 'Last run failed'])
    })

    it("draws only that repository's runs in its strip", () => {
      const wrapper = mount({
        repos: [
          { id: 10, name: 'server-daily' },
          { id: 11, name: 'media' },
        ],
        reports: [report({ id: 2, repo_id: 11 }), report({ id: 1, repo_id: 10 })],
      })
      for (const row of wrapper.findAll('.repo-row')) {
        expect(row.findAll('.run-cell')).toHaveLength(1)
      }
    })

    it('shows when the repository next runs and how much the last run added', () => {
      const text = mount().find('.repo-meta').text()
      expect(text).toContain('next')
      expect(text).toContain('256.0 B new')
    })

    it('marks a repository with a backup in flight as running', () => {
      const wrapper = mount({
        liveBackups: [
          {
            targetName: 'server-daily',
            repoId: 10,
            archiveName: 'a',
            elapsedSecs: 1,
            progress: null,
          },
        ],
      })
      expect(wrapper.find('.repo-row .badge').text()).toBe('Running now')
    })

    it('is absent when the agent has no repositories', () => {
      expect(mount({ repos: [] }).find('.repo-row').exists()).toBe(false)
    })
  })

  describe('needs-attention strip', () => {
    // The verdict is always there; the rows listing what is wrong are not, so
    // they only ever appear when they have something to say.
    it('has no rows when there is nothing to say', () => {
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

    // A failed run produced no archive, so its output is only rendered on the
    // Logs tab; this is the jump there.
    it("offers the failed run's output", async () => {
      const wrapper = mount({ reports: [report({ id: 7, status: 'failed' })] })
      await wrapper.find('.attention-action').trigger('click')
      expect(wrapper.emitted('openReportDetail')?.[0]?.[0]).toMatchObject({ id: 7 })
    })

    // The newest run overall is another repository's success, which used to
    // hide this failure and its jump to the error.
    it('flags a repository whose last run failed even when another ran later', async () => {
      const wrapper = mount({
        repos: [
          { id: 10, name: 'server-daily' },
          { id: 11, name: 'media' },
        ],
        reports: [
          report({ id: 2, repo_id: 10, finished_at: '2026-06-01T10:00:00Z' }),
          report({
            id: 1,
            repo_id: 11,
            repo_name: 'media',
            status: 'failed',
            finished_at: '2026-06-01T09:00:00Z',
          }),
        ],
      })
      expect(wrapper.find('.verdict').classes()).toContain('verdict--danger')
      expect(wrapper.find('.attention').text()).toContain('media')
      await wrapper.find('.attention-action').trigger('click')
      expect(wrapper.emitted('openReportDetail')?.[0]?.[0]).toMatchObject({ id: 1 })
    })

    it('gives each failing repository its own row and jump', () => {
      const wrapper = mount({
        repos: [
          { id: 10, name: 'server-daily' },
          { id: 11, name: 'media' },
        ],
        reports: [
          report({ id: 2, repo_id: 10, status: 'failed' }),
          report({ id: 1, repo_id: 11, status: 'failed' }),
        ],
      })
      expect(wrapper.findAll('.attention-action')).toHaveLength(2)
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

  it('shows a backup queued for this host while it is offline', () => {
    const wrapper = mount({
      agent: { ...AGENT, is_connected: false },
      liveBackups: [
        {
          targetName: 'server-daily',
          repoId: 10,
          archiveName: null,
          elapsedSecs: 0,
          progress: null,
        },
      ],
    })
    expect(wrapper.find('.live-log-title').text()).toBe('Backup queued')
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
