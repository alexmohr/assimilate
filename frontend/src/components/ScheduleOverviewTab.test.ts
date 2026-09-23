// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleOverviewTab from './ScheduleOverviewTab.vue'
import type { ScheduleRepoOption, ScheduleRow } from '../types/schedule'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'
import type { ScheduleTargetResponse } from '../types/generated/ScheduleTargetResponse'

const SCHEDULE = {
  id: 1,
  name: 'Nightly production backup',
  schedule_type: 'backup',
  repo_id: 20,
  on_failure: 'continue',
  next_run_at: '2026-08-19T02:00:00Z',
  last_run_at: '2026-08-18T02:00:00Z',
  enabled: true,
} as unknown as ScheduleRow

const AGENT_LABELS: Record<number, string> = { 10: 'web-server-01', 11: 'db-server-01' }

const REPOS = {
  primary: { id: 20, name: 'server-daily', required: true },
  offsite: { id: 21, name: 'offsite-weekly', required: false },
} satisfies Record<string, ScheduleRepoOption>

const TARGETS: ScheduleTargetResponse[] = [
  { agent_id: 10, execution_order: 0, catch_up_pending_for: null },
  { agent_id: 11, execution_order: 1, catch_up_pending_for: null },
]

function mount(overrides: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleOverviewTab, {
    props: {
      schedule: SCHEDULE,
      targets: TARGETS,
      repoName: 'server-daily',
      repoOptions: [REPOS.primary],
      cronSummary: 'Daily at 02:00',
      agentIds: [10, 11],
      agentLabel: (id: number) => AGENT_LABELS[id] ?? `#${id}`,
      healthForAgent: (): HealthSummaryResponse | null => null,
      connectivityNote: () => '',
      retryingAgentId: null,
      reports: [] as ReportRow[],
      agents: new Map<number, AgentRow>(),
      backupRunning: false,
      backupHostname: null,
      backupArchiveName: null,
      backupElapsedSecs: 0,
      estimatedRemainingSecs: null,
      archiveProgress: null,
      ...overrides,
    },
  })
}

describe('ScheduleOverviewTab', () => {
  it('shows the schedule info summary', () => {
    const wrapper = mount()
    const text = wrapper.text()
    expect(text).toContain('server-daily')
    expect(text).toContain('Continue')
    expect(text).toContain('Daily at 02:00')
  })

  /**
   * Whether a host is waited for is that host's own setting now, so the
   * schedule has no catch-up on/off of its own to report - only its floor.
   */
  it('spells out the floor a catch-up still has to clear, with no schedule-wide switch', () => {
    const wrapper = mount({
      schedule: { ...SCHEDULE, catch_up_min_lead_minutes: 120 },
    })
    expect(wrapper.text()).toContain('Catch-up')
    expect(wrapper.text()).toContain('Only if the next run is at least 2 hours away')
    expect(wrapper.text()).not.toContain('Off')
  })

  it('reads a floor in days as days', () => {
    const wrapper = mount({
      schedule: { ...SCHEDULE, catch_up_min_lead_minutes: 2 * 24 * 60 },
    })
    expect(wrapper.text()).toContain('at least 2 days away')
  })

  it('keeps a sub-hour floor in minutes rather than rounding it to zero hours', () => {
    const wrapper = mount({
      schedule: { ...SCHEDULE, catch_up_min_lead_minutes: 45 },
    })
    expect(wrapper.text()).toContain('45 minutes')
  })

  it('names the host a pending catch-up is waiting on', () => {
    const wrapper = mount({
      schedule: { ...SCHEDULE, catch_up_min_lead_minutes: 120 },
      targets: [
        { agent_id: 10, execution_order: 0, catch_up_pending_for: '2026-08-18T02:00:00Z' },
        { agent_id: 11, execution_order: 1, catch_up_pending_for: null },
      ],
    })
    expect(wrapper.text()).toContain('Pending for web-server-01')
    expect(wrapper.text()).not.toContain('Pending for db-server-01')
    // The target row carries its own marker, so the badge is next to the host
    // it belongs to and not only in the summary above.
    expect(wrapper.text()).toContain('Catch-up pending')
  })

  it('names the repository a pending catch-up is waiting on', () => {
    const wrapper = mount({
      repoOptions: [REPOS.primary, REPOS.offsite],
      pendingRepoCatchUps: [REPOS.offsite.id],
    })
    expect(wrapper.text()).toContain(`Pending for ${REPOS.offsite.name}`)
    expect(wrapper.text()).not.toContain(`Pending for ${REPOS.primary.name}`)
  })

  it('shows no catch-up badge when nothing is pending', () => {
    const wrapper = mount()
    expect(wrapper.text()).not.toContain('Catch-up pending')
  })

  it('names every repository a multi-target schedule writes into', () => {
    const wrapper = mount({ repoOptions: [REPOS.primary, REPOS.offsite] })
    const labels = wrapper.findAll('.info-grid dt').map((d) => d.text())
    expect(labels).toContain('Repositories')
    const names = wrapper.findAll('.repo-run .repo-link').map((l) => l.text())
    expect(names).toEqual(['server-daily', 'offsite-weekly'])
  })

  // A schedule copies into every target, so one occurrence leaves one report
  // per repository behind. Merged, they cannot say which copy is broken -
  // which is the whole reason anyone opens this page after a failure.
  describe('per-repository outcome', () => {
    const REPO_REPORTS = [
      {
        id: 1,
        agent_id: 10,
        repo_id: 20,
        repo_name: 'server-daily',
        status: 'success',
        finished_at: '2026-08-18T02:06:41Z',
        original_size: 2_100_000_000,
        duration_secs: 401,
        archive_name: 'web-server-01-2026-08-18',
        error_message: null,
        warnings: [],
      },
      {
        id: 2,
        agent_id: 10,
        repo_id: 21,
        repo_name: 'offsite-weekly',
        status: 'failed',
        finished_at: '2026-08-18T02:07:02Z',
        original_size: 0,
        duration_secs: 3,
        archive_name: null,
        error_message: 'Repository lock could not be acquired',
        warnings: [],
      },
    ] as unknown as ReportRow[]

    const AGENTS = new Map<number, AgentRow>([
      [10, { id: 10, hostname: 'web-server-01', display_name: null } as unknown as AgentRow],
    ])

    function multiRepoMount(over: Record<string, unknown> = {}) {
      return mount({
        agents: AGENTS,
        agentIds: [10],
        repoOptions: [REPOS.primary, REPOS.offsite],
        reports: REPO_REPORTS,
        ...over,
      })
    }

    it('gives each repository its own last outcome', () => {
      const rows = multiRepoMount().findAll('.repo-run')
      expect(rows).toHaveLength(2)
      expect(rows[0].text()).toContain('server-daily')
      expect(rows[0].find('.badge--success').exists()).toBe(true)
      expect(rows[1].text()).toContain('offsite-weekly')
      expect(rows[1].find('.badge--danger').exists()).toBe(true)
    })

    it('marks a best-effort target, whose failure never stops the run', () => {
      const rows = multiRepoMount().findAll('.repo-run')
      expect(rows[0].text()).not.toContain('best effort')
      expect(rows[1].text()).toContain('best effort')
    })

    it('says so for a target that has never run rather than calling it failed', () => {
      const rows = multiRepoMount({ reports: [REPO_REPORTS[0]] }).findAll('.repo-run')
      expect(rows[1].text()).toContain('never run')
      expect(rows[1].find('.badge--danger').exists()).toBe(false)
    })

    // The gate on this row used to be `repoRuns.length > 0`, which is one
    // entry per target and so true of every schedule that has a repository at
    // all - putting the multi-repo status treatment on the single-repo page
    // this change is meant to leave untouched.
    it('leaves a single-repository schedule with the plain repository name', () => {
      const wrapper = mount({ reports: REPO_REPORTS, agents: AGENTS })

      expect(wrapper.findAll('.repo-run')).toHaveLength(0)
      expect(wrapper.findAll('.info-grid dt')[0].text()).toBe('Repository')
      const value = wrapper.findAll('.info-grid dd')[0]
      expect(value.text()).toBe('server-daily')
      expect(value.find('.badge').exists()).toBe(false)
    })

    it('draws one run strip per repository', () => {
      const strips = multiRepoMount().findAll('.repo-strip')
      expect(strips).toHaveLength(2)
      expect(strips[0].find('.group-label').text()).toBe('server-daily')
      expect(strips[1].find('.group-label').text()).toBe('offsite-weekly')
    })

    it('keeps the single merged strip when the schedule writes to one repository', () => {
      const wrapper = mount({ reports: REPO_REPORTS, agents: AGENTS })
      expect(wrapper.findAll('.repo-strip')).toHaveLength(0)
      expect(wrapper.find('.run-strip').exists()).toBe(true)
    })

    // Without this, the two rows below are the same host, the same minute and
    // opposite outcomes, with nothing on screen saying where either went.
    it('names the repository each recent backup wrote into', () => {
      const pills = multiRepoMount()
        .findAll('.agent-row .meta-pill')
        .map((p) => p.text())
      expect(pills).toEqual(expect.arrayContaining(['server-daily', 'offsite-weekly']))
    })

    it('leaves the repository off the rows of a single-repository schedule', () => {
      const wrapper = mount({ reports: REPO_REPORTS, agents: AGENTS })
      expect(wrapper.findAll('.agent-row .meta-pill')).toHaveLength(0)
    })

    it('counts how much of a target host fan-out is failing', () => {
      const target = multiRepoMount().findAll('.agent-row')[0]
      expect(target.text()).toContain('1 of 2 repos failing')
    })

    it('says nothing about repositories on a target whose copies all landed', () => {
      const wrapper = multiRepoMount({ reports: [REPO_REPORTS[0]] })
      expect(wrapper.text()).not.toContain('repos failing')
    })

    it('counts the repositories a schedule fans out into', () => {
      expect(multiRepoMount().text()).toContain('into 2 repositories')
    })

    // A repository dropped from the schedule leaves its old runs behind, and
    // reporting a failure against a target the schedule no longer writes to
    // is a problem nobody can act on. Asserted against a schedule that still
    // has two live targets: one target renders the plain name, with no rows
    // to count.
    it('ignores runs against a repository that is no longer a target', () => {
      const retired = {
        ...REPO_REPORTS[0],
        id: 3,
        repo_id: 99,
        repo_name: 'retired-repo',
      } as unknown as ReportRow
      const wrapper = multiRepoMount({ reports: [...REPO_REPORTS, retired] })

      // Scoped to the status list, which is what `scheduleRepoRuns` filters:
      // the run itself still shows up in Recent backups under the name it was
      // given, which is the honest account of a run that did happen.
      expect(wrapper.findAll('.repo-run')).toHaveLength(2)
      expect(wrapper.find('.repo-runs').text()).not.toContain('retired-repo')
    })

    // That retired run keeps its Recent backups row, but the Backups tab can
    // only scope to a *current* target - so offering the jump would silently
    // land on the primary repository's pane instead of the archive clicked.
    it('offers no archive jump for a run against a retired repository', () => {
      const retired = {
        ...REPO_REPORTS[0],
        id: 3,
        repo_id: 99,
        repo_name: 'retired-repo',
        archive_name: 'on-retired',
      } as unknown as ReportRow
      const wrapper = multiRepoMount({ reports: [retired] })

      // The Targets rows share the `.agent-row` class, so the Recent backups
      // row is the last one, as the preview hand-off tests below also read it.
      const rows = wrapper.findAll('.agent-row')
      const row = rows[rows.length - 1]
      expect(row.text()).toContain('retired-repo')
      expect(row.find('button.agent-row-name').exists()).toBe(false)
    })

    it('still offers the jump for a run against a live target', () => {
      const rows = multiRepoMount({ reports: [REPO_REPORTS[0]] }).findAll('.agent-row')
      expect(rows[rows.length - 1].find('button.agent-row-name').exists()).toBe(true)
    })

    // Until the targets arrive there is nothing to judge a run against, and
    // withholding the jump on an empty list would take it away from every
    // schedule for as long as the fetch takes - including the ordinary
    // single-repository one, which is not what this is guarding.
    it('offers the jump while the targets are still loading', () => {
      const rows = mount({
        repoOptions: [],
        reports: [REPO_REPORTS[0]],
        agents: AGENTS,
      }).findAll('.agent-row')
      expect(rows[rows.length - 1].find('button.agent-row-name').exists()).toBe(true)
    })

    // The badge already says "never run"; a note restating that in different
    // words beside it read as a second, separate claim about the same run.
    it('leaves the run note empty for a target that has never run', () => {
      const rows = multiRepoMount({ reports: [REPO_REPORTS[0]] }).findAll('.repo-run')
      expect(rows[1].text()).toContain('never run')
      expect(rows[1].text()).not.toContain('no run yet')
      expect(rows[1].find('.repo-run-note').exists()).toBe(false)
    })
  })

  it('shows Never for a null last run', () => {
    const wrapper = mount({ schedule: { ...SCHEDULE, last_run_at: null } })
    expect(wrapper.text()).toContain('Never')
  })

  it('shows the live progress card only while a backup is running', () => {
    expect(mount({ backupRunning: true }).find('.live-log-card').exists()).toBe(true)
    expect(mount({ backupRunning: false }).find('.live-log-card').exists()).toBe(false)
  })

  it('lists every target as a row', () => {
    const wrapper = mount()
    const rows = wrapper.findAll('.agent-row')
    expect(rows.some((r) => r.text().includes('web-server-01'))).toBe(true)
    expect(rows.some((r) => r.text().includes('db-server-01'))).toBe(true)
  })

  it('shows no attention block when nothing is overdue', () => {
    expect(mount().find('.attention').exists()).toBe(false)
  })

  it('flags an overdue target with a connectivity note and a Retry button', async () => {
    const wrapper = mount({
      healthForAgent: (id: number): HealthSummaryResponse | null =>
        id === 10
          ? ({
              is_overdue: true,
              last_backup_at: '2026-08-15T02:00:00Z',
              last_status: 'success',
            } as unknown as HealthSummaryResponse)
          : null,
      connectivityNote: (id: number) => (id === 10 ? 'Agent offline (last seen 3 days ago)' : ''),
    })

    expect(wrapper.find('.attention').exists()).toBe(true)
    expect(wrapper.text()).toContain('web-server-01 has not run since')
    expect(wrapper.text()).toContain('Agent offline (last seen 3 days ago)')

    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Retry')
    expect(retryButton).toBeTruthy()
    await retryButton!.trigger('click')
    expect(wrapper.emitted('retry')).toEqual([[10]])
  })

  it('gives a target whose last run only warned its own stripe, not a clean green one', () => {
    const wrapper = mount({
      healthForAgent: (): HealthSummaryResponse | null =>
        ({
          is_overdue: false,
          last_status: 'warning',
          last_backup_at: '2026-08-18T02:00:00Z',
        }) as unknown as HealthSummaryResponse,
    })

    const row = wrapper.findAll('.agent-row').find((r) => r.text().includes('web-server-01'))
    const stripe = row!.find('.agent-row-stripe')
    expect(stripe.classes()).toContain('agent-row-stripe--warning')
    expect(stripe.classes()).not.toContain('agent-row-stripe--success')
  })

  it('does not mark a target that has never run as failed', () => {
    // /stats/health LEFT JOINs the latest report, so a target with no backup
    // yet comes back as a real health row with last_status null. Normalizing
    // that directly would fall through to 'failed' and paint a brand new
    // target red, contradicting the "last never" text in the same row.
    const wrapper = mount({
      healthForAgent: (): HealthSummaryResponse | null =>
        ({
          is_overdue: false,
          last_status: null,
          last_backup_at: null,
        }) as unknown as HealthSummaryResponse,
    })

    const row = wrapper.findAll('.agent-row').find((r) => r.text().includes('web-server-01'))
    expect(row!.text()).toContain('last never')
    const stripe = row!.find('.agent-row-stripe')
    expect(stripe.classes()).not.toContain('agent-row-stripe--danger')
    expect(stripe.classes()).toContain('agent-row-stripe--success')
  })

  it('gives the actively-backing-up target an accent stripe', () => {
    const wrapper = mount({
      backupRunning: true,
      backupHostname: 'web-server-01',
    })
    const targetRow = wrapper.findAll('.agent-row').find((r) => r.text().includes('web-server-01'))
    expect(targetRow!.find('.agent-row-stripe').classes()).toContain('agent-row-stripe--accent')
  })

  it('shows an Overdue badge and a Retry button on the target row itself, not just in the attention banner', async () => {
    const wrapper = mount({
      healthForAgent: (id: number): HealthSummaryResponse | null =>
        id === 10
          ? ({ is_overdue: true, last_backup_at: null } as unknown as HealthSummaryResponse)
          : null,
    })

    const targetRow = wrapper.findAll('.agent-row').find((r) => r.text().includes('web-server-01'))
    expect(targetRow).toBeTruthy()
    expect(targetRow!.find('.badge--warning').text()).toBe('Overdue')

    const retryButton = targetRow!.findAll('button').find((b) => b.text() === 'Retry')
    expect(retryButton).toBeTruthy()
    await retryButton!.trigger('click')
    expect(wrapper.emitted('retry')).toEqual([[10]])
  })

  it('disables Retry for the agent currently retrying', () => {
    const wrapper = mount({
      healthForAgent: (): HealthSummaryResponse | null =>
        ({ is_overdue: true, last_backup_at: null }) as unknown as HealthSummaryResponse,
      retryingAgentId: 10,
    })

    const retryButtons = wrapper
      .findAll('button')
      .filter((b) => b.text().includes('Retry') || b.text() === '...')
    expect(retryButtons.some((b) => b.attributes('disabled') !== undefined)).toBe(true)
  })

  it('hides the backups preview when there are no settled reports', () => {
    expect(
      mount({ reports: [{ id: 1, status: 'started' }] as unknown as ReportRow[] }).text(),
    ).not.toContain('Recent backups')
  })

  it('previews recent backups and emits openLogs from the view-all link', async () => {
    const agents = new Map<number, AgentRow>([
      [10, { id: 10, hostname: 'web-server-01', display_name: null } as unknown as AgentRow],
    ])
    const wrapper = mount({
      agents,
      reports: [
        {
          id: 1,
          agent_id: 10,
          status: 'success',
          finished_at: '2026-08-18T02:06:41Z',
          original_size: 2_100_000_000,
          duration_secs: 401,
        },
        {
          id: 2,
          agent_id: 10,
          status: 'failed',
          finished_at: '2026-08-15T02:00:00Z',
          original_size: 0,
          duration_secs: 5,
        },
      ] as unknown as ReportRow[],
    })

    expect(wrapper.text()).toContain('Recent backups')
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('failed')

    await wrapper.find('.section-link').trigger('click')
    expect(wrapper.emitted('openLogs')).toHaveLength(1)
  })

  // The preview is where a run's outcome is noticed, so it is also where the
  // trip to the run itself starts: its archive on this schedule's Backups
  // tab, or its output on the host that produced it.
  describe('preview hand-off', () => {
    const AGENTS = new Map<number, AgentRow>([
      [10, { id: 10, hostname: 'web-server-01', display_name: null } as unknown as AgentRow],
    ])

    function previewMount(over: Record<string, unknown>) {
      return mount({
        agents: AGENTS,
        reports: [
          {
            id: 7,
            agent_id: 10,
            // Every report carries the repository it was written to
            // (`ReportResponse.repo_id` is non-nullable), and the archive jump
            // is only offered for a repository the schedule still targets - so
            // a fixture without one is not a report the server can produce.
            repo_id: REPOS.primary.id,
            status: 'success',
            finished_at: '2026-08-18T02:06:41Z',
            original_size: 2_100_000_000,
            duration_secs: 401,
            archive_name: 'web-server-01-2026-08-18',
            error_message: null,
            warnings: [],
            ...over,
          },
        ] as unknown as ReportRow[],
      })
    }

    it('opens the archive of a run that produced one', async () => {
      const wrapper = previewMount({})
      const rows = wrapper.findAll('.agent-row')
      const button = rows[rows.length - 1].find('button.agent-row-name')
      await button.trigger('click')
      expect(wrapper.emitted('openArchive')).toHaveLength(1)
    })

    // A failed run wrote no archive, so this tab's archive browser has
    // nothing to select - the name stays inert and the output is elsewhere.
    it('offers the output of a failed run instead of an archive', async () => {
      const wrapper = previewMount({
        status: 'failed',
        archive_name: null,
        error_message: 'Repository lock could not be acquired',
      })
      const rows = wrapper.findAll('.agent-row')
      expect(rows[rows.length - 1].find('button.agent-row-name').exists()).toBe(false)
      const button = wrapper.findAll('button').find((b) => b.text() === 'View error')
      await button!.trigger('click')
      expect(wrapper.emitted('openReportDetail')).toEqual([[expect.objectContaining({ id: 7 })]])
    })

    it('offers both on a warned run, and names which is which', async () => {
      const wrapper = previewMount({
        status: 'warning',
        warnings: ['file changed while we read it'],
      })
      const button = wrapper.findAll('button').find((b) => b.text() === 'View warnings')
      expect(button).toBeTruthy()
      const rows = wrapper.findAll('.agent-row')
      await rows[rows.length - 1].find('button.agent-row-name').trigger('click')
      expect(wrapper.emitted('openArchive')).toHaveLength(1)
    })

    it('offers nothing to read on a clean run', () => {
      const wrapper = previewMount({})
      expect(wrapper.findAll('button').some((b) => b.text().startsWith('View e'))).toBe(false)
      expect(wrapper.findAll('button').some((b) => b.text().startsWith('View w'))).toBe(false)
    })
  })

  it('gives a cancelled backup a muted stripe, not a success-green one', () => {
    // `filterSettledReports` keeps cancelled runs (only pending/started are
    // dropped), so they do reach this preview - and the badge beside the
    // stripe renders them in a neutral tone. A green stripe would say the
    // opposite of the badge in the same row.
    const wrapper = mount({
      reports: [
        {
          id: 1,
          agent_id: 10,
          status: 'cancelled',
          finished_at: '2026-08-18T02:06:41Z',
          original_size: 0,
          duration_secs: 12,
        },
      ] as unknown as ReportRow[],
    })

    const row = wrapper.findAll('.agent-row').find((r) => r.text().includes('cancelled'))
    expect(row).toBeTruthy()
    const stripe = row!.find('.agent-row-stripe')
    expect(stripe.classes()).toContain('agent-row-stripe--muted')
    expect(stripe.classes()).not.toContain('agent-row-stripe--success')
  })
})
