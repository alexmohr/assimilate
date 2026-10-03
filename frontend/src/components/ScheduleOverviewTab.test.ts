// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { getRunEvents } from '../api/runs'
import ScheduleOverviewTab from './ScheduleOverviewTab.vue'
import type { ScheduleRepoOption, ScheduleRow } from '../types/schedule'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'
import type { ScheduleTargetResponse } from '../types/generated/ScheduleTargetResponse'
import type { DependencyWaitResponse, ScheduleDependencyResponse } from '../types/generated'

vi.mock('../api/runs', () => ({
  getRunEvents: vi.fn(),
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({ onMessage: (): void => undefined }),
}))

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

    it('leaves the repository off the rows of a single-repository schedule', () => {
      const wrapper = mount({ reports: REPO_REPORTS, agents: AGENTS })
      expect(wrapper.findAll('.agent-row .meta-pill')).toHaveLength(0)
    })

    it('says nothing about repositories on a target whose copies all landed', () => {
      const wrapper = multiRepoMount({ reports: [REPO_REPORTS[0]] })
      expect(wrapper.text()).not.toContain('repos failing')
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

  // The Targets rows that used to carry the marker are gone; the summary
  // still names the host, and only that host.
  it('names the host a pending catch-up is waiting on in the schedule info', () => {
    const wrapper = mount({
      schedule: { ...SCHEDULE, catch_up_min_lead_minutes: 120 },
      targets: [
        { agent_id: 10, execution_order: 0, catch_up_pending_for: '2026-08-18T02:00:00Z' },
        { agent_id: 11, execution_order: 1, catch_up_pending_for: null },
      ],
    })
    expect(wrapper.text()).toContain('Pending for web-server-01')
    expect(wrapper.text()).not.toContain('Pending for db-server-01')
  })

  it('names the hosts the schedule runs on, in order', () => {
    const wrapper = mount()
    const dts = wrapper.findAll('.info-grid dt')
    const hosts = dts.find((d) => d.text() === 'Hosts')
    expect(hosts!.element.nextElementSibling!.textContent).toBe('web-server-01, db-server-01')
  })

  it('has no targets tile, targets list or recent backups list any more', () => {
    const wrapper = mount({
      reports: [
        {
          id: 1,
          agent_id: 10,
          repo_id: 20,
          status: 'success',
          finished_at: '2026-08-18T02:06:41Z',
          original_size: 1,
          duration_secs: 1,
          error_message: null,
          warnings: [],
          run_id: 'r1',
        },
      ] as unknown as ReportRow[],
    })
    expect(wrapper.find('.tiles').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Recent backups')
    expect(wrapper.findAll('.section-title')).toHaveLength(0)
  })

  describe('dependencies', () => {
    const IN_11_MIN = (): string => new Date(Date.now() + 11 * 60_000 + 30_000).toISOString()
    const IN_18_H = (): string => new Date(Date.now() + 18 * 3_600_000 + 60_000).toISOString()

    function wait(over: Partial<DependencyWaitResponse> = {}): DependencyWaitResponse {
      return {
        schedule_id: 1,
        schedule_name: 'Nightly production backup',
        agent_id: 10,
        hostname: 'web-server-01',
        dependency_host_id: 5,
        dependency_name: 'nas-media',
        pending_for: '2026-08-18T02:00:00Z',
        last_probe_at: null,
        next_probe_at: IN_11_MIN(),
        give_up_at: IN_18_H(),
        catching_up: false,
        ...over,
      }
    }

    function dep(over: Partial<ScheduleDependencyResponse> = {}): ScheduleDependencyResponse {
      return {
        agent_id: 10,
        dependency_host_id: 5,
        dependency_name: 'nas-media',
        source: 'schedule',
        last_check_reachable: false,
        ...over,
      }
    }

    it('shows a waiting row for a run skipped on a dependency, with when it is next asked', () => {
      const wrapper = mount({ dependencyWaits: [wait()] })

      const row = wrapper.find('.attention .attention-row')
      expect(row.find('.badge--warning').text()).toBe('Waiting')
      expect(row.text()).toContain('web-server-01 was skipped at')
      expect(row.text()).toContain('dependency nas-media did not answer.')
      expect(row.find('a').attributes('href')).toBe('/dependency-hosts/5')
      expect(row.find('.attention-note').text()).toBe('Next check in 11m · gives up in 18h')
    })

    it('names every host waiting on one dependency in one row, with one Check now', () => {
      const wrapper = mount({
        dependencyWaits: [wait({ agent_id: 11, pending_for: '2026-08-18T02:05:00Z' }), wait()],
        canCheckDependencies: true,
      })

      const rows = wrapper.findAll('.attention .attention-row')
      expect(rows).toHaveLength(1)
      expect(rows[0].text()).toMatch(/\S+ and \S+ were skipped at/)
      expect(rows[0].findAll('button').filter((b) => b.text() === 'Check now')).toHaveLength(1)
    })

    it('leaves out the give-up time of a wait that is never abandoned', () => {
      const wrapper = mount({ dependencyWaits: [wait({ give_up_at: null })] })
      expect(wrapper.find('.attention-note').text()).toBe('Next check in 11m')
    })

    it('says a wait whose dependency answered is catching up, with nothing to check', () => {
      const wrapper = mount({
        dependencyWaits: [wait({ catching_up: true, next_probe_at: null })],
        canCheckDependencies: true,
      })
      expect(wrapper.find('.attention-note').text()).toBe('Catching up now')
      expect(wrapper.findAll('button').some((b) => b.text() === 'Check now')).toBe(false)
    })

    it('offers Check now to an admin and asks the page to run it', async () => {
      const wrapper = mount({ dependencyWaits: [wait()], canCheckDependencies: true })
      const button = wrapper.findAll('button').find((b) => b.text() === 'Check now')
      await button!.trigger('click')
      expect(wrapper.emitted('checkDependency')).toEqual([[5]])
    })

    it('disables Check now while that check is in flight', () => {
      const wrapper = mount({
        dependencyWaits: [wait()],
        canCheckDependencies: true,
        checkingDependencyId: 5,
      })
      const button = wrapper.findAll('button').find((b) => b.text() === 'Checking...')
      expect(button!.attributes('disabled')).toBeDefined()
    })

    it('offers no Check now to anyone else', () => {
      const wrapper = mount({ dependencyWaits: [wait()], canCheckDependencies: false })
      expect(wrapper.findAll('button').some((b) => b.text() === 'Check now')).toBe(false)
    })

    it("ignores another schedule's wait", () => {
      const wrapper = mount({ dependencyWaits: [wait({ schedule_id: 99 })] })
      expect(wrapper.find('.attention').exists()).toBe(false)
    })

    it('lists each dependency once in the schedule info, with its state and where it applies', () => {
      const wrapper = mount({
        dependencies: [
          dep({ agent_id: 11, source: 'agent_default' }),
          dep({ agent_id: 10 }),
          dep({
            agent_id: 10,
            dependency_host_id: 6,
            dependency_name: 'files-01',
            last_check_reachable: null,
          }),
        ],
      })

      const dt = wrapper.findAll('.info-grid dt').find((d) => d.text() === 'Dependencies')
      expect(dt).toBeTruthy()
      const items = wrapper.findAll('.dependency-item')
      expect(items).toHaveLength(2)
      expect(items[0].find('a').text()).toBe('nas-media')
      expect(items[0].find('.badge--warning').text()).toBe('Not answering')
      expect(items[0].find('.dependency-note').text()).toBe(
        'db-server-01 (agent defaults), web-server-01',
      )
      expect(items[1].find('.badge--neutral').text()).toBe('Not checked yet')
    })

    it('marks a dependency that answered as reachable', () => {
      const wrapper = mount({ dependencies: [dep({ last_check_reachable: true })] })
      expect(wrapper.find('.dependency-item .badge--success').text()).toBe('Reachable')
    })

    it('leaves the dependencies row out when the schedule needs none', () => {
      const labels = mount({ dependencies: [] })
        .findAll('.info-grid dt')
        .map((d) => d.text())
      expect(labels).not.toContain('Dependencies')
    })
  })

  describe('recent runs', () => {
    const AGENTS = new Map<number, AgentRow>([
      [10, { id: 10, hostname: 'web-server-01', display_name: null } as unknown as AgentRow],
      [11, { id: 11, hostname: 'db-server-01', display_name: null } as unknown as AgentRow],
    ])

    function report(over: Record<string, unknown>): ReportRow {
      return {
        id: 1,
        agent_id: 10,
        repo_id: 20,
        repo_name: 'server-daily',
        hostname: 'web-server-01',
        status: 'success',
        started_at: '2026-08-18T02:00:00Z',
        finished_at: '2026-08-18T02:05:00Z',
        original_size: 2_100_000_000,
        duration_secs: 300,
        archive_name: 'a',
        error_message: null,
        warnings: [],
        run_id: null,
        ...over,
      } as unknown as ReportRow
    }

    // Newest first, as the reports endpoint returns them.
    const REPORTS = [
      report({
        id: 5,
        agent_id: 11,
        run_id: 'run-3',
        status: 'skipped',
        finished_at: '2026-08-18T02:03:00Z',
        original_size: 0,
        duration_secs: 180,
        archive_name: null,
        error_message:
          "dependency 'nas-media' did not answer on port 445 (nas-media.lan)\nnothing was mounted",
      }),
      report({ id: 4, run_id: 'run-3', finished_at: '2026-08-18T02:01:00Z' }),
      report({
        id: 3,
        run_id: 'run-2',
        status: 'failed',
        finished_at: '2026-08-17T02:01:00Z',
        original_size: 0,
        duration_secs: 41,
        archive_name: null,
        error_message: 'Failed to create/acquire the lock\nborg create exited with code 2',
      }),
      report({
        id: 2,
        agent_id: 11,
        run_id: 'run-2',
        finished_at: '2026-08-17T02:00:30Z',
      }),
      report({ id: 1, run_id: 'run-1', finished_at: '2026-08-16T02:05:00Z' }),
    ]

    function runsMount(over: Record<string, unknown> = {}) {
      return mount({ agents: AGENTS, reports: REPORTS, ...over })
    }

    beforeEach(() => {
      vi.mocked(getRunEvents).mockReset()
      vi.mocked(getRunEvents).mockResolvedValue([])
    })

    it('draws one pill per run, oldest first, coloured by its worst report', () => {
      const pills = runsMount().findAll('.run-pill')
      expect(pills).toHaveLength(3)
      expect(pills[0].classes()).toContain('run-tone--success')
      expect(pills[1].classes()).toContain('run-tone--failed')
      expect(pills[2].classes()).toContain('run-tone--skipped')
      expect(pills[2].attributes('aria-label')).toMatch(/: Skipped$/)
    })

    it('counts runs, not reports, in the headline and the legend', () => {
      const wrapper = runsMount()
      expect(wrapper.text()).toContain('1 of 3 completed')
      const legend = wrapper.find('.chart-legend').text()
      expect(legend).toContain('1 succeeded')
      expect(legend).toContain('1 skipped')
      expect(legend).toContain('1 failed')
      expect(legend).not.toContain('cancelled')
      expect(legend).toContain('avg 5m 0s')
    })

    it('says so when there are no runs yet', () => {
      const wrapper = mount()
      expect(wrapper.text()).toContain('No runs yet')
      expect(wrapper.findAll('.run-pill')).toHaveLength(0)
    })

    it('opens on the newest run, one row per host', () => {
      const wrapper = runsMount()
      const pills = wrapper.findAll('.run-pill')
      expect(pills[2].attributes('aria-pressed')).toBe('true')
      expect(pills[0].attributes('aria-pressed')).toBe('false')

      const rows = wrapper.findAll('.run-detail .agent-row')
      expect(rows).toHaveLength(2)
      expect(rows[0].text()).toContain('web-server-01')
      expect(rows[0].text()).toContain('2.0 GB')
      expect(rows[1].text()).toContain('db-server-01')
      expect(rows[1].find('.badge--warning').text()).toBe('skipped')
      // Only the first line of the reason; the rest is behind the toggle.
      expect(rows[1].find('.agent-row-sub').text()).toBe(
        "dependency 'nas-media' did not answer on port 445 (nas-media.lan)",
      )
      // A skipped run wrote nothing, so it has no size.
      expect(rows[1].find('.agent-row-stats').text()).toContain('–')
      expect(wrapper.find('.run-detail').text()).toContain('Skipped')
    })

    it('switches the detail to the run whose pill is clicked', async () => {
      const wrapper = runsMount()
      await wrapper.findAll('.run-pill')[1].trigger('click')

      expect(wrapper.findAll('.run-pill')[1].attributes('aria-pressed')).toBe('true')
      const detail = wrapper.find('.run-detail')
      expect(detail.find('.badge--danger').text()).toBe('Failed')
      const rows = detail.findAll('.agent-row')
      expect(rows.map((r) => r.find('.agent-row-name').text())).toEqual([
        'db-server-01',
        'web-server-01',
      ])
      expect(rows[1].text()).toContain('Failed to create/acquire the lock')
      expect(rows[1].text()).not.toContain('exited with code 2')
    })

    it('keeps the output collapsed until asked, and labels a skip as one', async () => {
      const wrapper = runsMount()
      const row = wrapper.findAll('.run-detail .agent-row')[1]
      const toggle = row.find('button[aria-expanded]')
      expect(toggle.text()).toBe('Show detail')
      expect(toggle.attributes('aria-expanded')).toBe('false')
      expect(wrapper.find('.agent-row-detail').exists()).toBe(false)

      await toggle.trigger('click')
      await flushPromises()

      expect(toggle.text()).toBe('Hide detail')
      expect(toggle.attributes('aria-expanded')).toBe('true')
      const detail = wrapper.find('.agent-row-detail')
      expect(detail.find('.group-label--warning').text()).toBe('Skipped')
      expect(detail.text()).not.toContain('Error')
      expect(detail.text()).toContain('nothing was mounted')
      expect(detail.find('.detail-output--danger').exists()).toBe(false)
      expect(getRunEvents).toHaveBeenCalledWith('run-3', 11, 20)
      expect(detail.text()).toContain('No power-management activity for this run.')

      await toggle.trigger('click')
      expect(wrapper.find('.agent-row-detail').exists()).toBe(false)
    })

    it("labels a failed report's output as an error", async () => {
      const wrapper = runsMount()
      await wrapper.findAll('.run-pill')[1].trigger('click')
      await wrapper.findAll('.run-detail .agent-row')[1].find('button').trigger('click')
      const detail = wrapper.find('.agent-row-detail')
      expect(detail.find('.group-label--danger').text()).toBe('Error')
      expect(detail.find('.detail-output--danger').text()).toContain('exited with code 2')
    })

    it('opens the run on the Logs tab at the first report that did not succeed', async () => {
      const wrapper = runsMount()
      const open = wrapper.findAll('button').find((b) => b.text() === 'Open in Logs')
      await open!.trigger('click')
      expect(wrapper.emitted('openReportDetail')).toEqual([[expect.objectContaining({ id: 5 })]])
    })

    it('opens a clean run at its first report', async () => {
      const wrapper = runsMount()
      await wrapper.findAll('.run-pill')[0].trigger('click')
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Open in Logs')!
        .trigger('click')
      expect(wrapper.emitted('openReportDetail')).toEqual([[expect.objectContaining({ id: 1 })]])
    })

    it('hands the whole run history to the Logs tab', async () => {
      const wrapper = runsMount()
      await wrapper.find('.section-link').trigger('click')
      expect(wrapper.emitted('openLogs')).toHaveLength(1)
    })

    it('stands a report without a run id on its own', () => {
      const wrapper = runsMount({
        reports: [
          report({ id: 2, finished_at: '2026-08-18T02:05:00Z' }),
          report({ id: 1, status: 'failed', finished_at: '2026-08-18T02:04:00Z' }),
        ],
      })
      expect(wrapper.findAll('.run-pill')).toHaveLength(2)
    })

    it('notes how many warnings each host raised, singular or plural', () => {
      const wrapper = runsMount({
        reports: [
          report({
            id: 2,
            agent_id: 11,
            run_id: 'run-w',
            status: 'warning',
            warnings: ['file changed while we backed it up', 'permission denied: /etc/shadow'],
          }),
          report({
            id: 1,
            run_id: 'run-w',
            status: 'warning',
            warnings: ['file changed while we backed it up'],
            finished_at: '2026-08-18T02:04:00Z',
          }),
        ],
      })
      const rows = wrapper.findAll('.run-detail .agent-row')
      expect(rows).toHaveLength(2)
      const notes = Object.fromEntries(
        rows.map((r) => [r.find('.agent-row-name').text(), r.find('.run-report-note').text()]),
      )
      expect(notes).toEqual({ 'web-server-01': '1 warning', 'db-server-01': '2 warnings' })
    })

    describe('on a schedule writing to several repositories', () => {
      const MULTI = [
        report({ id: 3, repo_id: 21, run_id: 'run-2', status: 'failed', error_message: 'lock' }),
        report({ id: 2, repo_id: 20, run_id: 'run-2' }),
        report({ id: 1, repo_id: 20, run_id: 'run-1', finished_at: '2026-08-17T02:05:00Z' }),
      ]

      function multiMount() {
        return runsMount({ repoOptions: [REPOS.primary, REPOS.offsite], reports: MULTI })
      }

      it("draws one labelled strip per repository, each pill that repository's share of a run", () => {
        const strips = multiMount().findAll('.repo-strip')
        expect(strips).toHaveLength(2)
        expect(strips[0].find('.group-label').text()).toBe('server-daily')
        expect(strips[0].findAll('.run-pill')).toHaveLength(2)
        expect(strips[0].findAll('.run-pill')[1].classes()).toContain('run-tone--success')
        expect(strips[1].find('.group-label').text()).toBe('offsite-weekly')
        expect(strips[1].findAll('.run-pill')).toHaveLength(1)
        expect(strips[1].find('.run-pill').classes()).toContain('run-tone--failed')
      })

      it('names the repository on each row of the run detail', () => {
        const pills = multiMount()
          .findAll('.run-detail .agent-row .meta-pill')
          .map((p) => p.text())
        expect(pills).toEqual(['server-daily', 'offsite-weekly'])
      })

      it("picks the whole run from either repository's strip", async () => {
        const wrapper = multiMount()
        await wrapper.findAll('.repo-strip')[0].findAll('.run-pill')[0].trigger('click')
        const rows = wrapper.findAll('.run-detail .agent-row')
        expect(rows).toHaveLength(1)
        expect(rows[0].find('.meta-pill').text()).toBe('server-daily')
        // The same run is marked on every strip it appears on.
        expect(wrapper.findAll('.run-pill[aria-pressed="true"]')).toHaveLength(1)
      })
    })
  })
})
