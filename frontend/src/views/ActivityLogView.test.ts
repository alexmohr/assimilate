// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'
import { mockApiClientRw, mockTimezone } from '../test-utils/sharedMocks'

vi.mock('../composables/useTimezone', () => mockTimezone())
vi.mock('../api/client', () => mockApiClientRw())

const wsMessageHandlers = new Map<string, Array<(payload?: unknown) => void>>()

function triggerWsMessage(type: string, payload?: unknown): void {
  for (const handler of wsMessageHandlers.get(type) ?? []) handler(payload)
}

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    status: { value: 'connected' },
    onMessage: (type: string, handler: (payload?: unknown) => void) => {
      const handlers = wsMessageHandlers.get(type) ?? []
      handlers.push(handler)
      wsMessageHandlers.set(type, handlers)
    },
  }),
}))

import { apiClient } from '../api/client'
import type { ReportRow } from '../types/report'
import type { SystemEventSeverity } from '../types/generated'
import type { CurrentUserResponse } from '../api/auth'
import { useAuthStore } from '../stores/auth'
import ActivityLogView from './ActivityLogView.vue'
import { clientLogBuffer } from '../utils/clientLog'
import { useToast } from '../composables/useToast'

const mockGet = vi.mocked(apiClient.get)
const mockPost = vi.mocked(apiClient.post)
const mockDelete = vi.mocked(apiClient.delete)

interface ActivityRow {
  id: number
  hostname: string
  target_name: string
  acknowledged?: boolean
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
  run_id?: string
  schedule_id?: number | null
  schedule_name?: string | null
}

interface SystemEvent {
  id: number
  created_at: string
  event_type: string
  severity: SystemEventSeverity
  acknowledgeable: boolean
  acknowledged: boolean
  hostname: string | null
  message: string
}

interface Agent {
  id: number
  hostname: string
}

const AGENTS: Agent[] = [
  { id: 1, hostname: 'web-server-01' },
  { id: 2, hostname: 'db-server-01' },
]

const ACTIVITY_ROWS: ActivityRow[] = [
  {
    id: 101,
    hostname: 'web-server-01',
    target_name: '/var/www',
    started_at: '2026-01-01T10:00:00Z',
    finished_at: '2026-01-01T10:05:00Z',
    status: 'success',
    duration_secs: 300,
    run_id: 'run-101',
    schedule_id: 4,
    schedule_name: 'Web nightly',
  },
  {
    id: 102,
    hostname: 'db-server-01',
    target_name: '/var/lib/postgres',
    started_at: '2026-01-01T09:00:00Z',
    finished_at: '2026-01-01T09:03:00Z',
    status: 'failed',
    duration_secs: 180,
    // A schedule without a name: the feed sends its name as an empty string.
    schedule_id: 5,
    schedule_name: '',
  },
  {
    id: 103,
    hostname: 'web-server-01',
    target_name: '/var/www',
    started_at: '2026-01-01T08:00:00Z',
    finished_at: '2026-01-01T08:04:00Z',
    status: 'warning',
    duration_secs: 240,
    run_id: 'run-103',
  },
]

const WARNING_MOCK_REPORTS: ReportRow[] = [
  {
    id: 1,
    agent_id: 1,
    repo_id: 1,
    schedule_id: null,
    started_at: '2026-01-01T08:00:00Z',
    finished_at: '2026-01-01T08:04:00Z',
    status: 'warning',
    original_size: 1024,
    compressed_size: 512,
    deduplicated_size: 256,
    files_processed: 100,
    duration_secs: 240,
    error_message: 'some file changed during backup; slow read on /var/www/logs',
    warnings: ['some file changed during backup', 'slow read on /var/www/logs'],
    borg_version: '1.2.0',
    archive_name: null,
    borg_command: null,
    hostname: 'web-server-01',
    repo_name: null,
    schedule_name: null,
  },
]

const SYSTEM_EVENTS: SystemEvent[] = [
  {
    id: 1,
    created_at: '2026-01-01T07:00:00Z',
    event_type: 'repo_sync',
    severity: 'success',
    acknowledgeable: false,
    acknowledged: false,
    hostname: 'web-server-01',
    message: 'Repository sync completed',
  },
  {
    id: 2,
    created_at: '2026-01-01T06:00:00Z',
    event_type: 'repo_sync_failed',
    severity: 'failed',
    acknowledgeable: true,
    acknowledged: false,
    hostname: 'db-server-01',
    message: 'Periodic sync failed',
  },
]

/**
 * Counts the Acknowledge all button reads. Served by every mock below: the
 * button is deliberately driven by this unfiltered server signal rather than
 * by the rows on screen.
 */
function outstandingResponse(
  backup_reports = 0,
  system_events = 0,
): {
  data: { backup_reports: number; system_events: number }
} {
  return { data: { backup_reports, system_events } }
}

function createTestRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div />' } }],
  })
}

/**
 * Mounts the view with an optional signed-in user. Acknowledging a system
 * event is admin-only, so those tests need a store that says so.
 */
function mountView(
  role?: string,
  router?: ReturnType<typeof createRouter>,
): ReturnType<typeof mount> {
  const pinia = createPinia()
  if (role !== undefined) {
    useAuthStore(pinia).user = {
      id: 1,
      username: 'test-user',
      role,
      must_change_password: false,
      session_expires_at: null,
      remember_me: false,
      can_upgrade_agent: false,
      totp_enabled: false,
    } as CurrentUserResponse
  }
  return mount(ActivityLogView, {
    global: {
      plugins: [pinia, router ?? createTestRouter()],
      stubs: {
        BaseSpinner: { template: '<div class="spinner" />' },
        EmptyState: {
          props: ['title', 'description'],
          template:
            '<div class="empty-state"><span class="empty-title">{{ title }}</span><span class="empty-description">{{ description }}</span></div>',
        },
        Search: { template: '<span />' },
        SlidersHorizontal: { template: '<span />' },
        Activity: { template: '<span />' },
      },
    },
  })
}

function setupDefaultMocks(): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: AGENTS })
    // A fresh shallow clone each call: `toggleAcknowledge` mutates the row
    // object it receives in place, and reusing the shared fixture objects
    // would leak an acknowledged-in-one-test state into every test after it.
    if (url === '/stats/activity')
      return Promise.resolve({ data: ACTIVITY_ROWS.map((r) => ({ ...r })) })
    if (url === '/stats/system-events')
      return Promise.resolve({ data: SYSTEM_EVENTS.map((e) => ({ ...e })) })
    if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse(2, 1))
    if (url.startsWith('/agents/') && url.endsWith('/reports'))
      return Promise.resolve({
        data: { reports: WARNING_MOCK_REPORTS, total: WARNING_MOCK_REPORTS.length },
      })
    return Promise.resolve({ data: [] })
  })
}

function mockEmptyData(): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: [] })
    if (url === '/stats/activity') return Promise.resolve({ data: [] })
    if (url === '/stats/system-events') return Promise.resolve({ data: [] })
    if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse())
    return Promise.resolve({ data: [] })
  })
}

async function mountDefault(): Promise<ReturnType<typeof mount>> {
  setupDefaultMocks()
  const wrapper = mountView()
  await flushPromises()
  return wrapper
}

/** The Acknowledged filter is the only select offering "Only acknowledged". */
async function setAcknowledgedFilter(
  wrapper: ReturnType<typeof mount>,
  value: string,
): Promise<void> {
  const select = wrapper
    .findAll('select.select-input')
    .find((sel) => sel.findAll('option').some((o) => o.text() === 'Only acknowledged'))
  expect(select, 'no Acknowledged filter select').toBeDefined()
  await select!.setValue(value)
  await flushPromises()
}

async function mountEmpty(): Promise<ReturnType<typeof mount>> {
  mockEmptyData()
  const wrapper = mountView()
  await flushPromises()
  return wrapper
}

type Row = ReturnType<ReturnType<typeof mount>['findAll']>[number]

/** The backup runs in the feed, in the order shown. */
function backupRows(wrapper: ReturnType<typeof mount>): Row[] {
  return wrapper.findAll('.log-row:not(.run-card-system)')
}

/** The system events in the feed, in the order shown. */
function systemRows(wrapper: ReturnType<typeof mount>): Row[] {
  return wrapper.findAll('.log-row.run-card-system')
}

/** Whether a row's status dot is drawn in `tone`. */
function hasTone(row: Row, tone: string): boolean {
  return row.find('.log-row-dot').classes().includes(`status-dot--${tone}`)
}

function findWarningRow(wrapper: ReturnType<typeof mount>): Row[] {
  return backupRows(wrapper).filter(
    (r) =>
      hasTone(r, 'warning') &&
      r.find('.log-row-title').text() === 'web-server-01' &&
      r.find('.log-row-subtitle').text().startsWith('/var/www'),
  )
}

function findFailedRow(wrapper: ReturnType<typeof mount>): Row[] {
  return backupRows(wrapper).filter((r) => hasTone(r, 'danger'))
}

/** Opens (or closes) a row by clicking its summary line. */
async function toggleRow(row: Row): Promise<void> {
  await row.find('.log-row-summary').trigger('click')
  await flushPromises()
}

function findSegmentBtn(wrapper: ReturnType<typeof mount>, text: string) {
  return wrapper.findAll('.filter-pill').find((b) => b.text() === text)
}

describe('ActivityLogView', () => {
  beforeEach(() => {
    mockGet.mockReset()
    mockPost.mockReset()
    mockDelete.mockReset()
    wsMessageHandlers.clear()
  })

  describe('page header', () => {
    it('renders the Activity Log page title', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.page-title').text()).toBe('Activity Log')
    })

    it('displays the count of visible entries', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const count = wrapper.find('.row-count').text()
      expect(count).toMatch(/\d+ entries/)
    })
  })

  describe('category filter buttons', () => {
    it('renders All, Backup, System, and Server Logs category buttons', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const btnTexts = wrapper.findAll('.filter-pill').map((b) => b.text())
      expect(btnTexts).toContain('All')
      expect(btnTexts).toContain('Backup')
      expect(btnTexts).toContain('System')
      expect(btnTexts).toContain('Server Logs')
    })

    it('marks All button as active by default', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const allBtn = findSegmentBtn(wrapper, 'All')
      expect(allBtn?.classes()).toContain('active')
    })

    it('switches to Backup category when Backup button is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const backupBtn = findSegmentBtn(wrapper, 'Backup')
      await backupBtn?.trigger('click')
      await flushPromises()

      expect(backupBtn?.classes()).toContain('active')
    })

    it('switches to System category when System button is clicked', async () => {
      const wrapper = await mountDefault()

      const systemBtn = findSegmentBtn(wrapper, 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      expect(systemBtn?.classes()).toContain('active')
    })
  })

  describe('empty state', () => {
    it('shows empty state when no activity data is returned', async () => {
      const wrapper = await mountEmpty()

      expect(wrapper.find('.empty-state').exists()).toBe(true)
      expect(wrapper.find('.empty-title').text()).toBe('No activity')
    })

    it('does not show the entry list when there is no data', async () => {
      mockEmptyData()

      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('[data-testid="activity-entries"]').exists()).toBe(false)
    })
  })

  describe('activity list with data', () => {
    it('renders the run list when backup rows are present', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('[data-testid="activity-entries"]').exists()).toBe(true)
      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })

    it('renders backup rows with hostname and target', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const rows = backupRows(wrapper)
      expect(rows.map((r) => r.find('.log-row-title').text())).toEqual([
        'web-server-01',
        'db-server-01',
        'web-server-01',
      ])
      expect(rows.map((r) => r.find('.log-row-subtitle').text())).toEqual([
        '/var/www',
        '/var/lib/postgres',
        '/var/www',
      ])
    })

    it('names each run by its schedule, falling back to its number, and none without one', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      // The schedule leads the row's second line, ahead of the duration.
      const facts = backupRows(wrapper).map((r) => r.findAll('.log-row-fact').map((f) => f.text()))
      expect(facts.map((f) => (f.length > 1 ? f[0] : null))).toEqual([
        'Web nightly',
        'Schedule #5',
        null,
      ])
    })

    it('keeps the row actions inside the opened entry', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const [row] = backupRows(wrapper)
      // A closed row is one button: its own summary line.
      expect(row.findAll('button').map((b) => b.classes())).toEqual([
        expect.arrayContaining(['log-row-summary']),
      ])

      await toggleRow(row)
      expect(row.find('.log-row-detail .log-row-actions').text()).toContain('View run')
    })

    it('filters to a single run when its View run button is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const cardWithRun = backupRows(wrapper)[0]
      await toggleRow(cardWithRun)

      await cardWithRun
        .findAll('button')
        .find((b) => b.text() === 'View run')!
        .trigger('click')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({ params: expect.objectContaining({ run_id: 'run-101' }) }),
      )
      // The run filter shows itself, so it can be cleared again.
      expect(wrapper.find('.run-id-label').text()).toBe('run-101...')
    })

    it('does not offer View run once the feed is narrowed to that run', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      await toggleRow(backupRows(wrapper)[0])
      await backupRows(wrapper)[0]
        .findAll('button')
        .find((b) => b.text() === 'View run')!
        .trigger('click')
      await flushPromises()

      const row = backupRows(wrapper).find((r) => r.find('.log-row-fact').text() === 'Web nightly')!
      await toggleRow(row)
      expect(row.findAll('button').some((b) => b.text() === 'View run')).toBe(false)
    })

    it('draws each backup row with a status dot in its outcome tone', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const rows = backupRows(wrapper)
      expect(hasTone(rows[0], 'success')).toBe(true)
      expect(hasTone(rows[1], 'danger')).toBe(true)
      expect(hasTone(rows[2], 'warning')).toBe(true)
      // The dot carries the status: no pill repeats it on the row.
      expect(rows.some((r) => r.find('.badge').exists())).toBe(false)
    })

    it('expands a warning row and shows warnings in the detail panel', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const warningRows = findWarningRow(wrapper)
      expect(warningRows.length).toBeGreaterThan(0)

      await toggleRow(warningRows[0])

      const warningPre = wrapper.find('pre.warning-pre')
      expect(warningPre.exists()).toBe(true)
      expect(warningPre.text()).toContain('some file changed during backup')
      expect(warningPre.text()).toContain('slow read on /var/www/logs')

      // A warning-only report still carries a non-null error_message (the
      // backup_warning notification path reads it), but the Error box must
      // not render alongside the Warnings box for the same report.
      expect(wrapper.find('pre.error-pre').exists()).toBe(false)
    })
    it('closes an open run when its row is clicked again', async () => {
      const wrapper = await mountDefault()
      const [row] = findWarningRow(wrapper)

      await toggleRow(row!)
      expect(wrapper.find('pre.warning-pre').exists()).toBe(true)

      await toggleRow(row!)
      expect(wrapper.find('pre.warning-pre').exists()).toBe(false)
    })

    it('keeps the expanded detail panel open when a background DataChanged event arrives', async () => {
      const wrapper = await mountDefault()

      const warningRows = findWarningRow(wrapper)
      await toggleRow(warningRows[0])
      expect(wrapper.find('pre.warning-pre').exists()).toBe(true)

      triggerWsMessage('DataChanged')
      await flushPromises()

      const warningPre = wrapper.find('pre.warning-pre')
      expect(warningPre.exists()).toBe(true)
      expect(warningPre.text()).toContain('some file changed during backup')
    })
  })

  describe('acknowledging warnings and failures', () => {
    function ackButton(row: Row) {
      return row.findAll('button').find((b) => b.text() === 'Acknowledge')
    }

    /** The first warning row, opened so its actions show. */
    async function openWarningRow(wrapper: ReturnType<typeof mount>): Promise<Row> {
      const row = findWarningRow(wrapper)[0]
      await toggleRow(row)
      return row
    }

    it('offers Acknowledge on a warning row', async () => {
      const wrapper = await mountDefault()
      const rows = findWarningRow(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      // Only once the entry is open: the closed row carries no actions.
      expect(ackButton(rows[0])).toBeUndefined()
      await toggleRow(rows[0])
      expect(ackButton(rows[0])).toBeTruthy()
    })

    it('offers Acknowledge on a failed row', async () => {
      const wrapper = await mountDefault()
      const rows = findFailedRow(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      await toggleRow(rows[0])
      expect(ackButton(rows[0])).toBeTruthy()
    })

    it('does not offer Acknowledge on a successful row', async () => {
      const wrapper = await mountDefault()
      const successRow = backupRows(wrapper).find((r) => hasTone(r, 'success'))
      expect(successRow).toBeTruthy()
      await toggleRow(successRow!)
      expect(successRow!.find('.log-row-detail').exists()).toBe(true)
      expect(ackButton(successRow!)).toBeUndefined()
    })

    it('acknowledges the report and drops it from the default feed', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = await mountDefault()
      const before = findWarningRow(wrapper).length

      await ackButton(await openWarningRow(wrapper))!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/stats/activity/103/acknowledge')
      // The default filter hides acknowledged entries, so the row it was just
      // applied to leaves the feed rather than sitting there dimmed.
      expect(findWarningRow(wrapper).length).toBe(before - 1)
    })

    it('keeps the acknowledged row visible, dimmed, when the filter shows them', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = await mountDefault()
      await setAcknowledgedFilter(wrapper, 'all')

      const row = await openWarningRow(wrapper)
      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(row.find('.badge--neutral').text()).toBe('Acknowledged')
      expect(row.classes()).toContain('log-row--dim')
      expect(row.findAll('button').find((b) => b.text() === 'Unacknowledge')).toBeTruthy()
    })

    it('clicking Acknowledge does not also collapse the open entry', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = await mountDefault()
      await setAcknowledgedFilter(wrapper, 'all')
      const row = await openWarningRow(wrapper)
      expect(row.find('pre.warning-pre').exists()).toBe(true)

      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(row.find('pre.warning-pre').exists()).toBe(true)
    })

    it('unacknowledges a previously acknowledged report', async () => {
      mockDelete.mockResolvedValue({ data: undefined })
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') {
          return Promise.resolve({
            data: ACTIVITY_ROWS.map((r) => (r.id === 103 ? { ...r, acknowledged: true } : r)),
          })
        }
        if (url === '/stats/system-events')
          return Promise.resolve({ data: SYSTEM_EVENTS.map((e) => ({ ...e })) })
        if (url.startsWith('/agents/') && url.endsWith('/reports'))
          return Promise.resolve({
            data: { reports: WARNING_MOCK_REPORTS, total: WARNING_MOCK_REPORTS.length },
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()
      // An acknowledged row is only on screen at all once the filter asks for
      // it; the fixture above stands in for that server response.
      await setAcknowledgedFilter(wrapper, 'all')

      const row = findWarningRow(wrapper)[0]
      expect(row.find('.badge--neutral').text()).toBe('Acknowledged')
      await toggleRow(row)

      const unackButton = row.findAll('button').find((b) => b.text() === 'Unacknowledge')!
      await unackButton.trigger('click')
      await flushPromises()

      expect(mockDelete).toHaveBeenCalledWith('/stats/activity/103/acknowledge')
      expect(row.find('.badge--neutral').exists()).toBe(false)
      expect(ackButton(row)).toBeTruthy()
    })

    it('reports a failure without changing the button state', async () => {
      mockPost.mockRejectedValue(new Error('forbidden'))
      const wrapper = await mountDefault()
      const row = await openWarningRow(wrapper)

      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(row.findAll('button').find((b) => b.text() === 'Acknowledge')).toBeTruthy()
      expect(row.find('.badge--neutral').exists()).toBe(false)
    })

    it('groups the row actions at the top of the open entry', async () => {
      const wrapper = await mountDefault()
      const row = await openWarningRow(wrapper)

      const actions = row.find('.log-row-detail > .log-row-actions')
      expect(actions.exists()).toBe(true)
      expect(actions.findAll('button').map((b) => b.text())).toEqual(['View run', 'Acknowledge'])
      // Nothing acts on the entry from the closed summary line.
      expect(row.find('.log-row-summary').findAll('button')).toHaveLength(0)
    })
  })

  describe('acknowledged filter', () => {
    it('asks the server for unacknowledged entries by default', async () => {
      await mountDefault()

      expect(mockGet).toHaveBeenCalledWith('/stats/activity', {
        params: expect.objectContaining({ acknowledged: 'unacknowledged' }),
      })
      expect(mockGet).toHaveBeenCalledWith('/stats/system-events', {
        params: expect.objectContaining({ acknowledged: 'unacknowledged' }),
      })
    })

    it('refetches with the chosen state when the filter changes', async () => {
      const wrapper = await mountDefault()
      mockGet.mockClear()

      await setAcknowledgedFilter(wrapper, 'acknowledged')

      expect(mockGet).toHaveBeenCalledWith('/stats/activity', {
        params: expect.objectContaining({ acknowledged: 'acknowledged' }),
      })
    })

    it('counts a non-default acknowledged filter as an active filter', async () => {
      const wrapper = await mountDefault()
      await setAcknowledgedFilter(wrapper, 'all')

      const clearButton = wrapper.findAll('button').find((b) => b.text() === 'Clear')
      await clearButton!.trigger('click')
      await flushPromises()

      // Not `toHaveBeenLastCalledWith`: the outstanding-counts probe now
      // trails every feed load.
      expect(mockGet).toHaveBeenCalledWith(
        '/stats/system-events',
        expect.objectContaining({
          params: expect.objectContaining({ acknowledged: 'unacknowledged' }),
        }),
      )
    })
  })

  describe('acknowledge all', () => {
    it('acknowledges everything outstanding and reloads the feed', async () => {
      mockPost.mockResolvedValue({ data: { backup_reports: 2, system_events: 1 } })
      const wrapper = await mountDefault()

      const button = wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all'))
      expect(button).toBeTruthy()
      await button!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/stats/activity/acknowledge-all', undefined, {
        params: undefined,
      })
    })

    it('shows the button on the Backup tab when only system events are outstanding', async () => {
      // Regression: the button used to be gated on the rows currently loaded,
      // and the Backup tab never loads system events - so an outstanding sync
      // failure could not be cleared from the tab a dashboard finding link
      // drops you on.
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse(0, 1))
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()

      const backupBtn = wrapper.findAll('.filter-pill').find((b) => b.text() === 'Backup')
      await backupBtn?.trigger('click')
      await flushPromises()

      expect(
        wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all')),
      ).toBeTruthy()
    })

    it('keeps the button while a narrowing filter hides every outstanding row', async () => {
      // Regression: filtering to Status=success emptied the visible rows, and
      // the button vanished even though warnings and failures were still
      // outstanding elsewhere.
      const wrapper = await mountDefault()

      const statusSelect = wrapper
        .findAll('select.select-input')
        .find((sel) => sel.findAll('option').some((o) => o.text() === 'Started'))
      await statusSelect!.setValue('success')
      await flushPromises()

      expect(findWarningRow(wrapper)).toHaveLength(0)
      expect(
        wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all')),
      ).toBeTruthy()
    })

    it('keeps the entries listed when the bulk acknowledge fails', async () => {
      mockPost.mockRejectedValue(new Error('boom'))
      const wrapper = await mountDefault()

      const button = wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all'))
      await button!.trigger('click')
      await flushPromises()

      expect(findWarningRow(wrapper).length).toBeGreaterThan(0)
      expect(
        wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all')),
      ).toBeTruthy()
    })

    it('keeps the feed usable when the outstanding count cannot be loaded', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity')
          return Promise.resolve({ data: ACTIVITY_ROWS.map((r) => ({ ...r })) })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/stats/activity/outstanding') return Promise.reject(new Error('boom'))
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()

      // The probe failing must not take the feed down with it; the button
      // simply stays hidden because nothing is known to be outstanding.
      expect(findWarningRow(wrapper).length).toBeGreaterThan(0)
      expect(wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all'))).toBe(
        undefined,
      )
    })

    it('hides the button once nothing is left to acknowledge', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity')
          return Promise.resolve({ data: [ACTIVITY_ROWS[0]].map((r) => ({ ...r })) })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse())
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.findAll('button').find((b) => b.text().includes('Acknowledge all'))).toBe(
        undefined,
      )
    })
  })

  describe('acknowledging system events', () => {
    /** The failed periodic sync, opened so its actions show. */
    async function openFailedSync(wrapper: ReturnType<typeof mount>): Promise<Row> {
      const row = systemRows(wrapper).find((r) => hasTone(r, 'danger'))!
      await toggleRow(row)
      return row
    }

    function systemAckButton(row: Row) {
      return row
        .findAll('button')
        .find((b) => b.text() === 'Acknowledge' || b.text() === 'Unacknowledge')
    }

    it('offers Acknowledge on a failed periodic sync for an admin', async () => {
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      const button = systemAckButton(await openFailedSync(wrapper))
      expect(button).toBeTruthy()
      expect(button!.text()).toBe('Acknowledge')
    })

    it('acknowledges the event and drops it from the default feed', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()
      const before = systemRows(wrapper).length

      await systemAckButton(await openFailedSync(wrapper))!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/stats/system-events/2/acknowledge')
      expect(systemRows(wrapper).length).toBe(before - 1)
    })

    it('hides the action from a non-admin, who the server would reject', async () => {
      setupDefaultMocks()
      const wrapper = mountView('operator')
      await flushPromises()

      const row = await openFailedSync(wrapper)
      expect(row.find('.log-row-detail').exists()).toBe(true)
      expect(systemAckButton(row)).toBeUndefined()
    })

    it('unacknowledges an already acknowledged event', async () => {
      mockDelete.mockResolvedValue({ data: undefined })
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: SYSTEM_EVENTS.map((e) => (e.id === 2 ? { ...e, acknowledged: true } : { ...e })),
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView('admin')
      await flushPromises()
      // An acknowledged event only shows up once the filter asks for it.
      await setAcknowledgedFilter(wrapper, 'all')

      const row = await openFailedSync(wrapper)
      expect(row.find('.badge--neutral').text()).toBe('Acknowledged')
      const button = systemAckButton(row)
      expect(button!.text()).toBe('Unacknowledge')
      await button!.trigger('click')
      await flushPromises()

      expect(mockDelete).toHaveBeenCalledWith('/stats/system-events/2/acknowledge')
      expect(systemAckButton(row)!.text()).toBe('Acknowledge')
      expect(row.find('.badge--neutral').exists()).toBe(false)
    })

    it('drops an unacknowledged event from the "only acknowledged" view', async () => {
      mockDelete.mockResolvedValue({ data: undefined })
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: SYSTEM_EVENTS.map((e) => (e.id === 2 ? { ...e, acknowledged: true } : { ...e })),
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView('admin')
      await flushPromises()
      await setAcknowledgedFilter(wrapper, 'acknowledged')

      const before = systemRows(wrapper).length
      await systemAckButton(await openFailedSync(wrapper))!.trigger('click')
      await flushPromises()

      // Clearing the acknowledgment makes it no longer match a view that asks
      // for acknowledged entries only.
      expect(systemRows(wrapper).length).toBe(before - 1)
    })

    it('reports a failure without changing the button state', async () => {
      mockPost.mockRejectedValue(new Error('forbidden'))
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      const row = await openFailedSync(wrapper)
      await systemAckButton(row)!.trigger('click')
      await flushPromises()

      expect(systemAckButton(row)!.text()).toBe('Acknowledge')
      expect(wrapper.findAll('.run-card-system .badge--neutral')).toHaveLength(0)
    })

    it('leaves a success event unacknowledgeable', async () => {
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      const successCard = systemRows(wrapper).find((c) => hasTone(c, 'success'))
      expect(successCard).toBeTruthy()
      await toggleRow(successCard!)
      // Its one button is the summary line that opened it.
      expect(successCard!.findAll('button')).toHaveLength(1)
      expect(successCard!.find('.log-row-actions').exists()).toBe(false)
    })
  })

  describe('system events', () => {
    it('renders system event rows alongside backup rows in All view', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(systemRows(wrapper).length).toBe(SYSTEM_EVENTS.length)
      expect(backupRows(wrapper).length).toBe(ACTIVITY_ROWS.length)
    })

    it('renders system event messages', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const messages = systemRows(wrapper).map((r) => r.find('.log-row-fact').text())
      expect(messages).toContain('Repository sync completed')
      expect(messages).toContain('Periodic sync failed')
    })

    it('names the host and the event type on the row, or the server for a global event', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: [SYSTEM_EVENTS[1], { ...SYSTEM_EVENTS[0], hostname: null }],
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()

      const rows = systemRows(wrapper)
      expect(rows.map((r) => r.find('.log-row-title').text())).toEqual(['Server', 'db-server-01'])
      expect(rows.map((r) => r.find('.log-row-subtitle').text())).toEqual([
        'repo sync',
        'repo sync failed',
      ])
    })

    it('shows only system events when System category is active', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemBtn = findSegmentBtn(wrapper, 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      expect(backupRows(wrapper).length).toBe(0)
      expect(systemRows(wrapper).length).toBe(SYSTEM_EVENTS.length)
    })

    it('expands a system event row on click to show full message in its tone', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const [success, failed] = systemRows(wrapper)
      expect(wrapper.find('.run-card-system .log-row-detail').exists()).toBe(false)

      await toggleRow(success)
      expect(success.find('.log-row-detail pre.detail-pre').text()).toBe(SYSTEM_EVENTS[0].message)

      await toggleRow(failed)
      expect(failed.find('.log-row-detail pre.error-pre').text()).toBe(SYSTEM_EVENTS[1].message)
      // One system event open at a time.
      expect(wrapper.findAll('.run-card-system .log-row-detail')).toHaveLength(1)
    })

    it('collapses a system event row on second click', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const [row] = systemRows(wrapper)
      await toggleRow(row)
      expect(wrapper.find('.run-card-system .log-row-detail').exists()).toBe(true)

      await toggleRow(row)
      expect(wrapper.find('.run-card-system .log-row-detail').exists()).toBe(false)
    })

    it('marks the clicked system event row as open', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const [row] = systemRows(wrapper)
      expect(row.classes()).not.toContain('log-row--open')
      expect(row.find('.log-row-summary').attributes('aria-expanded')).toBe('false')

      await toggleRow(row)

      expect(row.classes()).toContain('log-row--open')
      expect(row.find('.log-row-summary').attributes('aria-expanded')).toBe('true')
    })
  })

  describe('live backup output', () => {
    const run = { hostname: 'web-01', target_name: '/var/www' }

    it('shows the lines of a running backup, without progress records, until it ends', async () => {
      const wrapper = await mountDefault()
      triggerWsMessage('BackupStarted', run)
      await flushPromises()
      // A started run with nothing logged yet has no card.
      expect(wrapper.find('.live-session-card').exists()).toBe(false)

      triggerWsMessage('BackupLog', { hostname: 'web-01', line: 'Creating archive' })
      triggerWsMessage('BackupLog', {
        hostname: 'web-01',
        line: JSON.stringify({ type: 'archive_progress', nfiles: 3 }),
      })
      triggerWsMessage('BackupLog', { hostname: 'web-01', line: 'Done' })
      await flushPromises()

      const card = wrapper.get('.live-session-card')
      expect(card.text()).toContain('web-01')
      expect(card.findAll('.live-session-line').map((l) => l.text())).toEqual([
        'Creating archive',
        'Done',
      ])

      triggerWsMessage('BackupCompleted', run)
      await flushPromises()
      expect(wrapper.find('.live-session-card').exists()).toBe(false)
    })
  })

  describe('filter controls', () => {
    it('renders Machine filter select', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const selects = wrapper.findAll('select.select-input')
      expect(selects.length).toBeGreaterThan(0)
    })

    it('renders Status filter select with all/success/warning/failed options', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const selects = wrapper.findAll('select.select-input')
      const allOptions = selects.flatMap((s) => s.findAll('option').map((o) => o.text()))
      expect(allOptions).toContain('Success')
      expect(allOptions).toContain('Warning')
      expect(allOptions).toContain('Failed')
    })

    it('renders date range inputs', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const dateInputs = wrapper.findAll('input.date-input')
      expect(dateInputs.length).toBe(2)
    })

    it('renders Clear button', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const clearBtn = wrapper.findAll('button').find((b) => b.text() === 'Clear')
      expect(clearBtn.exists()).toBe(true)
      expect(clearBtn.text()).toBe('Clear')
    })

    it('filters backup rows by status when status filter is changed', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const statusSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Failed'))
      await statusSelect?.setValue('failed')
      await flushPromises()

      const rows = backupRows(wrapper)
      expect(rows.length).toBe(1)
      expect(rows.filter((r) => hasTone(r, 'danger')).length).toBe(rows.length)
    })

    it('filters backup rows by machine', async () => {
      const wrapper = await mountDefault()
      const machineSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'All machines'))!

      await machineSelect.setValue('db-server-01')
      await flushPromises()

      const rows = backupRows(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      expect(rows.every((r) => r.text().includes('db-server-01'))).toBe(true)
    })

    it('filters backup rows by target', async () => {
      const wrapper = await mountDefault()
      const targetSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === '/var/lib/postgres'))!

      await targetSelect.setValue('/var/lib/postgres')
      await flushPromises()

      const rows = backupRows(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      expect(rows.every((r) => r.text().includes('/var/lib/postgres'))).toBe(true)
      expect(rows.some((r) => r.text().includes('/var/www'))).toBe(false)
    })

    it('clears all filters when Clear is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const statusSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Failed'))
      await statusSelect?.setValue('failed')
      await flushPromises()

      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Clear')!
        .trigger('click')
      await flushPromises()

      expect((statusSelect?.element as HTMLSelectElement).value).toBe('all')
    })

    it('refetches backup activity for the schedule picked in the Schedule filter', async () => {
      setupDefaultMocks()
      const defaultGet = mockGet.getMockImplementation()!
      mockGet.mockImplementation((url: string, config?: unknown) =>
        url === '/schedules'
          ? Promise.resolve({ data: [{ id: 7, name: 'Nightly' }] })
          : defaultGet(url, config),
      )
      const wrapper = mountView()
      await flushPromises()
      mockGet.mockClear()

      const scheduleSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'All schedules'))
      expect(scheduleSelect, 'no Schedule filter select').toBeDefined()
      await scheduleSelect!.setValue('7')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith('/stats/activity', {
        params: expect.objectContaining({ schedule_id: 7 }),
      })
    })
  })

  it('keeps a slow detail fetch from showing its run under another open row', async () => {
    const STALE: ReportRow = {
      ...WARNING_MOCK_REPORTS[0]!,
      id: 102,
      status: 'failed',
      error_message: 'stale run output',
      warnings: [],
      hostname: 'db-server-01',
    }
    let answerStale: (value: unknown) => void = () => undefined
    setupDefaultMocks()
    const fallback = mockGet.getMockImplementation()!
    mockGet.mockImplementation((url: string, config?: unknown) => {
      if (url === '/agents/db-server-01/reports')
        return new Promise((resolve) => (answerStale = resolve))
      return fallback(url, config)
    })
    const wrapper = mountView()
    await flushPromises()

    // Open the failed run, then the warned one before the first answers.
    await findFailedRow(wrapper)[0]!.find('.log-row-summary').trigger('click')
    const warning = findWarningRow(wrapper)[0]!
    await toggleRow(warning)
    expect(warning.text()).toContain('some file changed during backup')

    answerStale({ data: { reports: [STALE], total: 1 } })
    await flushPromises()

    const open = backupRows(wrapper).filter((r) => r.classes().includes('log-row--open'))
    expect(open).toHaveLength(1)
    expect(open[0]!.text()).toContain('some file changed during backup')
    expect(open[0]!.text()).not.toContain('stale run output')
  })

  describe('retrying a failed run', () => {
    /** db-server-01's report behind feed row 102, as its host's run list returns it. */
    const FAILED_REPORT: ReportRow = {
      ...WARNING_MOCK_REPORTS[0]!,
      id: 102,
      agent_id: 7,
      repo_id: 3,
      schedule_id: 5,
      status: 'failed',
      error_message: 'repository locked',
      warnings: [],
      hostname: 'db-server-01',
      repo_name: 'pg-repo',
    }

    function mountWithFeed(feed: ActivityRow[]): ReturnType<typeof mountView> {
      setupDefaultMocks()
      const fallback = mockGet.getMockImplementation()!
      mockGet.mockImplementation((url: string, config?: unknown) => {
        if (url === '/stats/activity') return Promise.resolve({ data: feed.map((r) => ({ ...r })) })
        if (url === '/agents/db-server-01/reports')
          return Promise.resolve({ data: { reports: [FAILED_REPORT], total: 1 } })
        return fallback(url, config)
      })
      return mountView()
    }

    function retryButton(row: Row): Row | undefined {
      return row.findAll('button').find((b) => b.text().includes('Retry'))
    }

    it('re-runs the schedule for just that host and repository', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = mountWithFeed(ACTIVITY_ROWS)
      await flushPromises()
      const row = findFailedRow(wrapper)[0]!
      expect(retryButton(row)).toBeUndefined()

      await toggleRow(row)
      await retryButton(row)!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/schedules/5/run', {
        agent_ids: [7],
        repo_ids: [3],
      })
    })

    it('offers nothing on a run that is not a failure', async () => {
      const wrapper = mountWithFeed(ACTIVITY_ROWS)
      await flushPromises()
      const warning = findWarningRow(wrapper)[0]!
      await toggleRow(warning)

      expect(retryButton(warning)).toBeUndefined()
    })

    it('offers nothing once a newer run of the same copy exists', async () => {
      const newer: ActivityRow = {
        ...ACTIVITY_ROWS[1]!,
        id: 150,
        status: 'success',
        started_at: '2026-01-02T09:00:00Z',
        finished_at: '2026-01-02T09:03:00Z',
      }
      const wrapper = mountWithFeed([newer, ...ACTIVITY_ROWS])
      await flushPromises()
      const failed = findFailedRow(wrapper)[0]!
      await toggleRow(failed)

      expect(retryButton(failed)).toBeUndefined()
    })

    describe('narrowed to one run with View run', () => {
      const failedInRun: ActivityRow = { ...ACTIVITY_ROWS[1]!, run_id: 'run-102' }
      const fixedLater: ActivityRow = {
        ...failedInRun,
        id: 150,
        status: 'success',
        run_id: 'run-150',
        started_at: '2026-01-02T09:00:00Z',
        finished_at: '2026-01-02T09:03:00Z',
      }

      /** The feed for that one run, and the schedule's own later runs beside it. */
      async function openNarrowed(later: () => Promise<unknown>): Promise<Row> {
        setupDefaultMocks()
        const fallback = mockGet.getMockImplementation()!
        mockGet.mockImplementation((url: string, config?: { params?: Record<string, unknown> }) => {
          if (url === '/stats/activity') {
            if (config?.params?.run_id === 'run-102')
              return Promise.resolve({ data: [{ ...failedInRun }] })
            if (config?.params?.schedule_id === 5) return later()
            return Promise.resolve({ data: [{ ...failedInRun }] })
          }
          if (url === '/agents/db-server-01/reports')
            return Promise.resolve({ data: { reports: [FAILED_REPORT], total: 1 } })
          return fallback(url, config)
        })
        const wrapper = mountView()
        await flushPromises()
        const row = findFailedRow(wrapper)[0]!
        await toggleRow(row)
        await row
          .findAll('button')
          .find((b) => b.text() === 'View run')!
          .trigger('click')
        await flushPromises()
        const narrowed = findFailedRow(wrapper)[0]!
        await toggleRow(narrowed)
        return narrowed
      }

      it('still offers Retry while the failure is the newest run of its copy', async () => {
        const row = await openNarrowed(() => Promise.resolve({ data: [{ ...failedInRun }] }))
        expect(retryButton(row)).toBeDefined()
      })

      it('offers nothing once a later run outside the narrowed one fixed it', async () => {
        const row = await openNarrowed(() =>
          Promise.resolve({ data: [{ ...fixedLater }, { ...failedInRun }] }),
        )
        expect(mockGet).toHaveBeenCalledWith(
          '/stats/activity',
          expect.objectContaining({
            params: expect.objectContaining({ schedule_id: 5, acknowledged: 'all' }),
          }),
        )
        expect(retryButton(row)).toBeUndefined()
      })

      it('withholds Retry when the later runs cannot be read', async () => {
        const row = await openNarrowed(() => Promise.reject(new Error('offline')))
        expect(retryButton(row)).toBeUndefined()
      })
    })

    it('offers nothing on a failed run with no schedule to re-run', async () => {
      const unscheduled = ACTIVITY_ROWS.map((r) => (r.id === 102 ? { ...r, schedule_id: null } : r))
      const wrapper = mountWithFeed(unscheduled)
      await flushPromises()
      const failed = findFailedRow(wrapper)[0]!
      await toggleRow(failed)

      expect(retryButton(failed)).toBeUndefined()
    })
  })

  describe('deep links from the dashboard', () => {
    // A Needs Attention finding links here with ?status=..., which is how a
    // user reaches the run they are about to acknowledge. The status is
    // untrusted query text, so it is validated before it reaches the filter.
    async function mountAtQuery(
      query: Record<string, string>,
    ): Promise<ReturnType<typeof mountView>> {
      setupDefaultMocks()
      const router = createTestRouter()
      await router.push({ path: '/', query })
      await router.isReady()
      const wrapper = mountView(undefined, router)
      await flushPromises()
      return wrapper
    }

    function statusSelect(
      wrapper: ReturnType<typeof mountView>,
    ): ReturnType<typeof wrapper.findAll>[number] | undefined {
      return wrapper
        .findAll('select.select-input')
        .find((sel) => sel.findAll('option').some((o) => o.text() === 'Failed'))
    }

    it('applies a status the dashboard linked to', async () => {
      const wrapper = await mountAtQuery({ status: 'failed' })

      expect((statusSelect(wrapper)?.element as HTMLSelectElement).value).toBe('failed')
    })

    it('ignores a status the backend would never report', async () => {
      const wrapper = await mountAtQuery({ status: 'not-a-status' })

      expect((statusSelect(wrapper)?.element as HTMLSelectElement).value).toBe('all')
    })

    // A link to one failed or warned run (`?report=`) opens it on arrival, so
    // its output is the first thing read.
    it('opens the run a link names, with its output showing', async () => {
      const wrapper = await mountAtQuery({ report: '103' })

      const open = backupRows(wrapper).filter((r) => r.classes().includes('log-row--open'))
      expect(open).toHaveLength(1)
      expect(open[0]!.find('.log-row-title').text()).toBe('web-server-01')
      expect(hasTone(open[0]!, 'warning')).toBe(true)
      expect(open[0]!.find('pre.warning-pre').text()).toContain('some file changed during backup')
      expect(mockGet).toHaveBeenCalledWith(
        '/agents/web-server-01/reports',
        expect.objectContaining({ params: expect.objectContaining({ target: '/var/www' }) }),
      )
    })

    it('opens the run behind a full run-log link, narrowed to it', async () => {
      const wrapper = await mountAtQuery({
        category: 'backup',
        hostname: 'web-server-01',
        status: 'warning',
        report: '103',
      })

      expect(findSegmentBtn(wrapper, 'Backup')?.classes()).toContain('active')
      expect((statusSelect(wrapper)?.element as HTMLSelectElement).value).toBe('warning')
      const rows = backupRows(wrapper)
      expect(rows).toHaveLength(1)
      expect(rows[0]!.classes()).toContain('log-row--open')
    })

    // An older or acknowledged run is not on the first page; the link still
    // opens it, by looking it up and narrowing the log to its run.
    it('opens a linked run the first page does not hold, narrowed to its run', async () => {
      const OLD = {
        ...ACTIVITY_ROWS[0]!,
        id: 300,
        run_id: 'run-300',
        status: 'failed',
        error_message: 'repository locked',
        started_at: '2025-06-01T10:00:00Z',
        finished_at: '2025-06-01T10:01:00Z',
        acknowledged: true,
      }
      setupDefaultMocks()
      const fallback = mockGet.getMockImplementation()!
      mockGet.mockImplementation((url: string, config?: { params?: Record<string, unknown> }) => {
        if (url === '/stats/activity' && config?.params?.report_id === 300)
          return Promise.resolve({ data: [{ ...OLD }] })
        if (url === '/stats/activity' && config?.params?.run_id === 'run-300')
          return Promise.resolve({ data: [{ ...OLD }] })
        return fallback(url, config)
      })
      const router = createTestRouter()
      await router.push({ path: '/', query: { report: '300' } })
      await router.isReady()
      const wrapper = mountView(undefined, router)
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({
          params: expect.objectContaining({ run_id: 'run-300', acknowledged: 'all' }),
        }),
      )
      const open = backupRows(wrapper).filter((r) => r.classes().includes('log-row--open'))
      expect(open).toHaveLength(1)
      // The run's own (failed) row, not one of the first page's.
      expect(hasTone(open[0]!, 'danger')).toBe(true)
      expect(backupRows(wrapper)).toHaveLength(1)
    })

    // A run with no run id cannot be narrowed to, so the log stays as it was
    // and the looked-up run is added at its end, still opened.
    it('appends a linked run with no run id that the refreshed page still lacks', async () => {
      const ORPHAN: ActivityRow = {
        ...ACTIVITY_ROWS[0]!,
        id: 400,
        hostname: 'old-host',
        run_id: undefined,
        status: 'failed',
        error_message: 'host went away',
        started_at: '2025-05-01T10:00:00Z',
        finished_at: '2025-05-01T10:01:00Z',
      }
      setupDefaultMocks()
      const fallback = mockGet.getMockImplementation()!
      mockGet.mockImplementation((url: string, config?: { params?: Record<string, unknown> }) => {
        if (url === '/stats/activity' && config?.params?.report_id === 400)
          return Promise.resolve({ data: [{ ...ORPHAN }] })
        return fallback(url, config)
      })
      const router = createTestRouter()
      await router.push({ path: '/', query: { report: '400' } })
      await router.isReady()
      const wrapper = mountView(undefined, router)
      await flushPromises()

      // Nothing to narrow by: no run filter, and acknowledged runs stay hidden.
      expect(mockGet).not.toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({ params: expect.objectContaining({ run_id: expect.anything() }) }),
      )
      expect(mockGet).not.toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({ params: expect.objectContaining({ acknowledged: 'all' }) }),
      )
      const rows = backupRows(wrapper)
      expect(rows.map((r) => r.find('.log-row-title').text())).toContain('old-host')
      const open = rows.filter((r) => r.classes().includes('log-row--open'))
      expect(open).toHaveLength(1)
      expect(open[0]!.find('.log-row-title').text()).toBe('old-host')
      // Its detail is the run's own, read from its host.
      expect(mockGet).toHaveBeenCalledWith('/agents/old-host/reports', expect.anything())
      // The page's own rows are still there, the linked run after them.
      expect(rows.at(-1)!.find('.log-row-title').text()).toBe('old-host')
      expect(rows.length).toBeGreaterThan(1)
    })

    it('opens nothing when the lookup answers for another run', async () => {
      // The default mock answers every activity request with the same rows,
      // none of them run 999: a stray answer must not open the wrong run.
      const wrapper = await mountAtQuery({ report: '999' })

      expect(mockGet).toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({ params: { report_id: 999 } }),
      )
      expect(wrapper.find('.log-row--open').exists()).toBe(false)
    })

    it.each([
      ['a run that is not in the feed', '999'],
      ['a report id that is not a number', 'abc'],
      ['a report id of zero', '0'],
    ])('opens nothing for %s', async (_case, report) => {
      const wrapper = await mountAtQuery({ report })

      expect(wrapper.find('.log-row--open').exists()).toBe(false)
      expect(mockGet).not.toHaveBeenCalledWith(
        expect.stringMatching(/^\/agents\/.+\/reports$/),
        expect.anything(),
      )
    })
  })

  describe('date filters', () => {
    // Rows a week apart, so From and To each have something to exclude.
    const DATED_ROWS = [
      { ...ACTIVITY_ROWS[0], id: 201, started_at: '2026-03-01T10:00:00Z' },
      { ...ACTIVITY_ROWS[0], id: 202, started_at: '2026-03-08T10:00:00Z' },
      { ...ACTIVITY_ROWS[0], id: 203, started_at: '2026-03-15T10:00:00Z' },
    ]

    function mountWithDatedRows(): ReturnType<typeof mountView> {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: DATED_ROWS })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse())
        return Promise.resolve({ data: [] })
      })
      return mountView()
    }

    function visibleRunCount(wrapper: ReturnType<typeof mountView>): number {
      return backupRows(wrapper).length
    }

    it('drops runs that started before the From date', async () => {
      const wrapper = mountWithDatedRows()
      await flushPromises()
      expect(visibleRunCount(wrapper)).toBe(3)

      const dateInputs = wrapper.findAll('input.date-input')
      expect(dateInputs.length).toBe(2)
      await dateInputs[0]!.setValue('2026-03-08')
      await flushPromises()

      expect(visibleRunCount(wrapper)).toBe(2)
    })

    it('drops runs that started after the To date, to the end of that day', async () => {
      const wrapper = mountWithDatedRows()
      await flushPromises()

      const dateInputs = wrapper.findAll('input.date-input')
      await dateInputs[1]!.setValue('2026-03-08')
      await flushPromises()

      // The 8th itself is kept: To is inclusive through 23:59:59 local.
      expect(visibleRunCount(wrapper)).toBe(2)
    })

    it('combines both bounds to a single day', async () => {
      const wrapper = mountWithDatedRows()
      await flushPromises()

      const dateInputs = wrapper.findAll('input.date-input')
      await dateInputs[0]!.setValue('2026-03-08')
      await dateInputs[1]!.setValue('2026-03-08')
      await flushPromises()

      expect(visibleRunCount(wrapper)).toBe(1)
    })

    it('counts a date bound as an active filter that Clear resets', async () => {
      const wrapper = mountWithDatedRows()
      await flushPromises()

      const dateInputs = wrapper.findAll('input.date-input')
      await dateInputs[0]!.setValue('2026-03-08')
      await flushPromises()
      expect(visibleRunCount(wrapper)).toBe(2)

      const clearButton = wrapper.findAll('button').find((b) => b.text() === 'Clear')
      expect(clearButton, 'a date bound must count as an active filter').toBeTruthy()
      await clearButton!.trigger('click')
      await flushPromises()

      expect(visibleRunCount(wrapper)).toBe(3)
    })
  })

  describe('load more', () => {
    it('shows Load more button when hasMore is true', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity')
          return Promise.resolve({
            data: Array.from({ length: 50 }, (_, i) => ({ ...ACTIVITY_ROWS[0], id: i + 1 })),
          })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const loadMore = wrapper.findAll('button').find((b) => b.text() === 'Load more')
      expect(loadMore).toBeDefined()
    })

    it('does not re-probe the outstanding counts when paging further back', async () => {
      // Paging into history cannot change what is still outstanding, so the
      // extra round trip would be pure waste. A filter change still re-probes,
      // because another session may have acknowledged something meanwhile.
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity')
          return Promise.resolve({
            data: Array.from({ length: 50 }, (_, i) => ({ ...ACTIVITY_ROWS[0], id: i + 1 })),
          })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/stats/activity/outstanding') return Promise.resolve(outstandingResponse(2, 1))
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const outstandingCalls = (): number =>
        mockGet.mock.calls.filter((call) => call[0] === '/stats/activity/outstanding').length
      const afterMount = outstandingCalls()
      expect(afterMount).toBeGreaterThan(0)

      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Load more')!
        .trigger('click')
      await flushPromises()

      expect(outstandingCalls()).toBe(afterMount)

      await setAcknowledgedFilter(wrapper, 'all')

      expect(outstandingCalls()).toBeGreaterThan(afterMount)
    })
  })

  describe('server logs tab', () => {
    it('shows log search and level filter when Server Logs tab is active', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/logs')
          return Promise.resolve({
            data: [
              {
                timestamp: '2026-01-01T10:00:00Z',
                level: 'info',
                target: 'server',
                message: 'Started',
              },
            ],
          })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const logsBtn = wrapper.findAll('.filter-pill').find((b) => b.text() === 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const levelSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Error'))
      expect(levelSelect?.exists()).toBe(true)
      expect(wrapper.find('input.search-input').exists()).toBe(true)
    })

    it('tones each log row by its level, whatever case the server reports it in', async () => {
      // The level is what colours the row, and it has to survive the server
      // reporting the level in upper case.
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/logs')
          return Promise.resolve({
            data: [
              {
                timestamp: '2026-01-01T10:03:00Z',
                level: 'ERROR',
                target: 'server',
                message: 'Boom',
              },
              { timestamp: '2026-01-01T10:02:00Z', level: 'warn', target: 'server', message: 'Hm' },
              { timestamp: '2026-01-01T10:01:00Z', level: 'info', target: 'server', message: 'Ok' },
              { timestamp: '2026-01-01T10:00:00Z', level: 'debug', target: 'db', message: 'Dbg' },
            ],
          })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const logsBtn = findSegmentBtn(wrapper, 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const rows = wrapper.findAll('[data-testid="log-entries"] .log-row')
      expect(rows).toHaveLength(4)
      expect(hasTone(rows[0], 'danger')).toBe(true)
      expect(hasTone(rows[1], 'warning')).toBe(true)
      expect(hasTone(rows[2], 'info')).toBe(true)
      // Debug is neutral: a plain dot with no tone of its own.
      expect(rows[3].find('.log-row-dot').classes()).toEqual(['status-dot', 'log-row-dot'])
      expect(rows[0].findAll('.log-row-fact').map((f) => f.text())).toEqual(['ERROR', 'server'])
      expect(wrapper.find('.row-count').text()).toBe('4 entries')
    })

    it('opens a log entry to show its full message and target', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/logs')
          return Promise.resolve({
            data: [
              { timestamp: '2026-01-01T10:00:00Z', level: 'info', target: 'srv::db', message: 'A' },
              { timestamp: '2026-01-01T09:00:00Z', level: 'info', target: 'srv::ws', message: 'B' },
            ],
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()
      await findSegmentBtn(wrapper, 'Server Logs')?.trigger('click')
      await flushPromises()

      const rows = wrapper.findAll('[data-testid="log-entries"] .log-row')
      await toggleRow(rows[1])
      const detail = rows[1].find('.log-row-detail')
      expect(detail.find('pre.detail-pre').text()).toBe('B')
      expect(detail.findAll('dt').map((d) => d.text())).toEqual(['Time', 'Level', 'Target'])
      expect(detail.find('.log-detail-value').text()).toBe('srv::ws')

      await toggleRow(rows[0])
      expect(wrapper.findAll('[data-testid="log-entries"] .log-row-detail')).toHaveLength(1)
    })

    it('debounces the log search into one fetch carrying the typed text', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const logsBtn = wrapper.findAll('.filter-pill').find((b) => b.text() === 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const searchCalls = (): unknown[] =>
        mockGet.mock.calls
          .filter(([url]) => url === '/logs')
          .map(([, config]) => (config as { params: Record<string, unknown> }).params.search)
          .filter((search) => search !== undefined)

      // Only the timer functions are faked, so the pending fetch promise
      // still settles on the real microtask queue once the debounce fires.
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
      try {
        const input = wrapper.find('input.search-input')
        await input.setValue('disk')
        await vi.advanceTimersByTimeAsync(200)
        await input.setValue('disk full')
        await vi.advanceTimersByTimeAsync(299)
        expect(searchCalls()).toEqual([])

        await vi.advanceTimersByTimeAsync(1)
        expect(searchCalls()).toEqual(['disk full'])
      } finally {
        vi.useRealTimers()
      }
    })
  })

  describe('system event tones', () => {
    // One event per severity the server can report, including the neutral
    // 'info' one: it must not borrow the success colour.
    const SYSTEM_EVENTS: Array<{
      type: string
      severity: SystemEventSeverity
      label: string
      badge: string
    }> = [
      { type: 'repo_sync', severity: 'success', label: 'repo sync', badge: 'badge--success' },
      {
        type: 'repo_sync_slow',
        severity: 'warning',
        label: 'repo sync slow',
        badge: 'badge--warning',
      },
      {
        type: 'repo_sync_failed',
        severity: 'failed',
        label: 'repo sync failed',
        badge: 'badge--danger',
      },
      {
        type: 'repo_sync_cancelled',
        severity: 'info',
        label: 'repo sync cancelled',
        badge: 'badge--info',
      },
    ]

    it('colors each system event by what it means', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: SYSTEM_EVENTS.map((e, i) => ({
              id: i + 1,
              created_at: `2026-01-01T0${7 - i}:00:00Z`,
              event_type: e.type,
              severity: e.severity,
              acknowledgeable: e.severity === 'warning' || e.severity === 'failed',
              acknowledged: false,
              hostname: null,
              message: e.label,
            })),
          })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const systemBtn = wrapper.findAll('.filter-pill').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      const rows = systemRows(wrapper)
      for (const { label, badge } of SYSTEM_EVENTS) {
        const found = rows.find((r) => r.find('.log-row-fact').text() === label)
        expect(found, `no row for ${label}`).toBeDefined()
        expect(hasTone(found!, badge.replace('badge--', ''))).toBe(true)
      }
      // A failure must never be reachable by any other arm.
      expect(rows.filter((r) => hasTone(r, 'danger'))).toHaveLength(1)
    })

    it('shows each expanded system event message in its own tone', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: SYSTEM_EVENTS.map((e, i) => ({
              id: i + 1,
              created_at: `2026-01-01T0${7 - i}:00:00Z`,
              event_type: e.type,
              severity: e.severity,
              acknowledgeable: false,
              acknowledged: false,
              hostname: null,
              message: `${e.label} message`,
            })),
          })
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()

      const preClassBySeverity: Record<SystemEventSeverity, string> = {
        success: 'detail-pre',
        warning: 'warning-pre',
        failed: 'error-pre',
        info: 'detail-pre',
      }
      await findSegmentBtn(wrapper, 'System')!.trigger('click')
      await flushPromises()
      for (const { label, severity } of SYSTEM_EVENTS) {
        const message = `${label} message`
        const row = systemRows(wrapper).find((r) => r.find('.log-row-fact').text() === message)
        expect(row, `no row for ${label}`).toBeDefined()
        await toggleRow(row!)
        const pre = row!.get('.log-row-detail pre')
        expect(pre.text(), label).toBe(message)
        expect(pre.classes(), label).toEqual([preClassBySeverity[severity]])
      }
    })
  })

  describe('API integration', () => {
    it('fetches agents and activity data on mount', async () => {
      setupDefaultMocks()
      mountView()
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith('/agents', { params: undefined })
      expect(mockGet).toHaveBeenCalledWith('/stats/activity', expect.any(Object))
      expect(mockGet).toHaveBeenCalledWith('/stats/system-events', expect.any(Object))
    })
  })
  describe('browser logs tab', () => {
    interface ShownLogRow {
      level: string
      target: string
      message: string
    }

    beforeEach(() => {
      clientLogBuffer.clear()
    })

    async function openBrowserTab(role = 'admin'): Promise<ReturnType<typeof mount>> {
      setupDefaultMocks()
      const wrapper = mountView(role)
      await flushPromises()
      await findSegmentBtn(wrapper, 'Browser logs')?.trigger('click')
      await flushPromises()
      return wrapper
    }

    /** The log rows on screen: the message is the title, then level and source as facts. */
    function logRows(wrapper: ReturnType<typeof mount>): Row[] {
      return wrapper.findAll('[data-testid="log-entries"] .log-row')
    }

    function tableRows(wrapper: ReturnType<typeof mount>): ShownLogRow[] {
      return logRows(wrapper).map((r) => {
        const [level, target] = r.findAll('.log-row-fact').map((f) => f.text())
        return {
          level: level ?? '',
          target: target ?? '',
          message: r.find('.log-row-title').text(),
        }
      })
    }

    function levelSelect(wrapper: ReturnType<typeof mount>) {
      return wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Error'))
    }

    function headerButton(wrapper: ReturnType<typeof mount>, text: string) {
      return wrapper.findAll('.header-actions button').find((b) => b.text().includes(text))
    }

    it('is offered to admins only', async () => {
      setupDefaultMocks()
      const viewer = mountView('viewer')
      await flushPromises()
      expect(findSegmentBtn(viewer, 'Browser logs')).toBeUndefined()

      const admin = mountView('admin')
      await flushPromises()
      expect(findSegmentBtn(admin, 'Browser logs')).toBeDefined()
    })

    it('falls back to All when a non-admin opens ?category=browser', async () => {
      setupDefaultMocks()
      const router = createTestRouter()
      await router.push({ path: '/', query: { category: 'browser' } })
      await router.isReady()
      const wrapper = mountView('viewer', router)
      await flushPromises()
      expect(wrapper.find('.filter-pill.active').text()).toBe('All')
      expect(wrapper.find('.header-actions').text()).not.toContain('Clear logs')
    })

    it('lists buffered entries newest first with their source and stack', async () => {
      clientLogBuffer.record('debug', ['first'], 'a (a.js:1:1)')
      clientLogBuffer.record('error', [new Error('second')], 'b (b.js:2:2)')
      const wrapper = await openBrowserTab()

      const rows = tableRows(wrapper)
      expect(rows.map((r) => r.message)).toEqual(['Error: second', 'first'])
      expect(rows[0]!.target).toBe('b (b.js:2:2)')
      expect(rows[0]!.level).toBe('ERROR')

      // The stack is read in the opened entry, under its own heading.
      const [errorRow, debugRow] = logRows(wrapper)
      await toggleRow(errorRow!)
      expect(errorRow!.find('.log-row-detail').text()).toContain('Stack trace')
      expect(errorRow!.findAll('.log-row-detail pre.detail-pre')[1]!.text()).toContain('second')
      expect(errorRow!.findAll('.log-row-detail dt').map((d) => d.text())).toContain('Source')
      await toggleRow(debugRow!)
      expect(debugRow!.find('.log-row-detail').text()).not.toContain('Stack trace')
      expect(debugRow!.findAll('.log-row-detail pre.detail-pre')).toHaveLength(1)
      expect(wrapper.find('.row-count').text()).toBe('2 entries')
      // Server-side fetches are not made for the browser tab.
      expect(mockGet).not.toHaveBeenCalledWith('/logs', expect.anything())
    })

    it('updates live as the app logs', async () => {
      const wrapper = await openBrowserTab()
      expect(tableRows(wrapper)).toEqual([])
      clientLogBuffer.record('warn', ['arrived later'])
      await flushPromises()
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['arrived later'])
    })

    it('says so when nothing has been logged yet', async () => {
      const wrapper = await openBrowserTab()
      expect(wrapper.find('.empty-description').text()).toBe(
        'Nothing has been logged in this browser tab yet.',
      )
    })

    it('offers only the browser levels and filters at that level and above', async () => {
      clientLogBuffer.record('error', ['e'])
      clientLogBuffer.record('warn', ['w'])
      clientLogBuffer.record('debug', ['d'])
      const wrapper = await openBrowserTab()

      const select = levelSelect(wrapper)!
      expect(select.findAll('option').map((o) => o.text())).toEqual([
        'All',
        'Error',
        'Warn',
        'Debug',
      ])
      await select.setValue('warn')
      await flushPromises()
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['w', 'e'])
      await select.setValue('error')
      await flushPromises()
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['e'])
    })

    it('drops a server-only level when switching to the browser tab', async () => {
      clientLogBuffer.record('debug', ['d'])
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()
      await findSegmentBtn(wrapper, 'Server Logs')?.trigger('click')
      await flushPromises()
      await levelSelect(wrapper)!.setValue('trace')
      await findSegmentBtn(wrapper, 'Browser logs')?.trigger('click')
      await flushPromises()
      expect((levelSelect(wrapper)!.element as HTMLSelectElement).value).toBe('')
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['d'])
    })

    it('filters by free text across message and source', async () => {
      clientLogBuffer.record('error', ['Fetch failed'], 'loadRepos (repos.js:1:1)')
      clientLogBuffer.record('debug', ['theme sync'], 'useTheme (theme.js:1:1)')
      const wrapper = await openBrowserTab()

      await wrapper.find('input.search-input').setValue('FETCH')
      await flushPromises()
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['Fetch failed'])
      await wrapper.find('input.search-input').setValue('usetheme')
      await flushPromises()
      expect(tableRows(wrapper).map((r) => r.message)).toEqual(['theme sync'])
      await wrapper.find('input.search-input').setValue('nothing')
      await flushPromises()
      expect(wrapper.find('.empty-description').text()).toBe(
        'No log entries match the current filters.',
      )
    })

    it('clears the buffer', async () => {
      clientLogBuffer.record('error', ['gone soon'])
      const wrapper = await openBrowserTab()
      await headerButton(wrapper, 'Clear logs')!.trigger('click')
      await flushPromises()
      expect(clientLogBuffer.size).toBe(0)
      expect(tableRows(wrapper)).toEqual([])
      expect(headerButton(wrapper, 'Clear logs')!.attributes('disabled')).toBeDefined()
    })

    it('copies the filtered entries, oldest first, as text', async () => {
      const writeText = vi.fn().mockResolvedValue(undefined)
      Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
      clientLogBuffer.record('error', ['one'], 'f (a.js:1:1)')
      clientLogBuffer.record('debug', ['two'])
      const wrapper = await openBrowserTab()

      await headerButton(wrapper, 'Copy')!.trigger('click')
      await flushPromises()
      expect(writeText).toHaveBeenCalledTimes(1)
      const text = writeText.mock.calls[0]![0] as string
      expect(text).toMatch(/^\[.+\] ERROR f \(a\.js:1:1\): one\n\[.+\] DEBUG: two$/)
      expect(headerButton(wrapper, 'Copied')).toBeDefined()
    })

    it('shows a toast when the clipboard refuses', async () => {
      const writeText = vi.fn().mockRejectedValue(new Error('denied'))
      Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
      clientLogBuffer.record('error', ['one'])
      const wrapper = await openBrowserTab()
      const { toasts } = useToast()
      await headerButton(wrapper, 'Copy')!.trigger('click')
      await flushPromises()
      expect(headerButton(wrapper, 'Copied')).toBeUndefined()
      expect(toasts.value.some((t) => t.type === 'error' && t.message.includes('denied'))).toBe(
        true,
      )
    })

    it('marks the mobile Filters toggle only while a browser log filter is set', async () => {
      const desktopWidth = window.innerWidth
      Object.defineProperty(window, 'innerWidth', { value: 375, configurable: true })
      try {
        clientLogBuffer.record('error', ['e'])
        const wrapper = await openBrowserTab()
        const toggle = wrapper.find('button.filter-toggle')
        expect(toggle.classes()).not.toContain('active')
        expect(toggle.find('.filter-badge').exists()).toBe(false)
        // The level and search filters stay hidden until the toggle opens them.
        expect(levelSelect(wrapper)).toBeUndefined()

        await toggle.trigger('click')
        await levelSelect(wrapper)!.setValue('error')
        await flushPromises()
        expect(wrapper.find('button.filter-toggle').classes()).toContain('active')
        expect(wrapper.find('button.filter-toggle .filter-badge').exists()).toBe(true)

        await levelSelect(wrapper)!.setValue('')
        await wrapper.find('input.search-input').setValue('e')
        await flushPromises()
        expect(wrapper.find('button.filter-toggle').classes()).toContain('active')

        await wrapper.find('input.search-input').setValue('')
        await flushPromises()
        expect(wrapper.find('button.filter-toggle').classes()).not.toContain('active')
      } finally {
        Object.defineProperty(window, 'innerWidth', { value: desktopWidth, configurable: true })
      }
    })

    it('stops listening to the buffer once unmounted', async () => {
      const before = clientLogBuffer.listenerCount
      const wrapper = await openBrowserTab()
      expect(clientLogBuffer.listenerCount).toBe(before + 1)
      wrapper.unmount()
      expect(clientLogBuffer.listenerCount).toBe(before)
    })
  })

  describe('restores tab', () => {
    it('is offered to admins only', async () => {
      setupDefaultMocks()
      const viewer = mountView('viewer')
      await flushPromises()
      expect(findSegmentBtn(viewer, 'Restores')).toBeUndefined()

      const admin = mountView('admin')
      await flushPromises()
      expect(findSegmentBtn(admin, 'Restores')).toBeDefined()
    })

    it('falls back to All when a non-admin opens ?category=restores', async () => {
      setupDefaultMocks()
      const router = createTestRouter()
      await router.push({ path: '/', query: { category: 'restores' } })
      await router.isReady()
      const wrapper = mountView('viewer', router)
      await flushPromises()

      expect(wrapper.find('.filter-pill.active').text()).toBe('All')
      expect(mockGet).not.toHaveBeenCalledWith('/restores', expect.anything())
    })

    it('lists restores in place of the activity, without its filters', async () => {
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()
      mockGet.mockClear()

      await findSegmentBtn(wrapper, 'Restores')?.trigger('click')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith('/restores', { params: {} })
      expect(mockGet).not.toHaveBeenCalledWith('/stats/activity', expect.anything())
      expect(wrapper.text()).toContain('No restores')
      expect(wrapper.find('.row-count').exists()).toBe(false)
      expect(wrapper.text()).not.toContain('All machines')
    })
  })
})
