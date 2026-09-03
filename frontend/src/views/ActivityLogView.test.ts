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

function triggerWsMessage(type: string): void {
  for (const handler of wsMessageHandlers.get(type) ?? []) handler()
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
  },
  {
    id: 102,
    hostname: 'db-server-01',
    target_name: '/var/lib/postgres',
    started_at: '2026-01-01T09:00:00Z',
    finished_at: '2026-01-01T09:03:00Z',
    status: 'failed',
    duration_secs: 180,
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
        DataTable: {
          name: 'DataTable',
          // `rowClass` is declared so a test can exercise the callback the
          // view passes down; the real DataTable calls it per rendered row.
          props: ['value', 'rowClass'],
          template: '<div class="p-datatable"><slot /><slot name="empty" /></div>',
        },
        Column: true,
        BaseSpinner: { template: '<div class="spinner" />' },
        EmptyState: {
          props: ['title', 'description'],
          template: '<div class="empty-state"><span class="empty-title">{{ title }}</span></div>',
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
      return Promise.resolve({ data: WARNING_MOCK_REPORTS })
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

function findWarningRow(wrapper: ReturnType<typeof mount>) {
  return wrapper
    .findAll('.run-card:not(.run-card-system) .run-card-summary')
    .filter(
      (r) =>
        r.find('.badge--warning').exists() &&
        r.find('.run-card-hostname').text() === 'web-server-01' &&
        r.find('.run-card-meta').text().startsWith('/var/www'),
    )
}

function findFailedRow(wrapper: ReturnType<typeof mount>) {
  return wrapper
    .findAll('.run-card:not(.run-card-system) .run-card-summary')
    .filter((r) => r.find('.badge--danger').exists())
}

function findSegmentBtn(wrapper: ReturnType<typeof mount>, text: string) {
  return wrapper.findAll('.segmented-option').find((b) => b.text() === text)
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

      const btnTexts = wrapper.findAll('.segmented-option').map((b) => b.text())
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

    it('does not show the table when there is no data', async () => {
      mockEmptyData()

      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.run-list').exists()).toBe(false)
    })
  })

  describe('activity list with data', () => {
    it('renders the run list when backup rows are present', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.run-list').exists()).toBe(true)
      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })

    it('renders backup cards with hostname and target', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const rows = wrapper.findAll('.run-card:not(.run-card-system) .run-card-summary')
      expect(rows.length).toBeGreaterThan(0)

      const firstRow = rows[0]
      expect(firstRow.find('.run-card-hostname').text()).toBeTruthy()
      expect(firstRow.find('.run-card-meta').text()).toBeTruthy()
    })

    it('filters to a single run when its View run button is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const cardWithRun = wrapper
        .findAll('.run-card:not(.run-card-system)')
        .find((c) => c.findAll('button').some((b) => b.text() === 'View run'))
      expect(cardWithRun).toBeTruthy()

      await cardWithRun!
        .findAll('button')
        .find((b) => b.text() === 'View run')!
        .trigger('click')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith(
        '/stats/activity',
        expect.objectContaining({ params: expect.objectContaining({ run_id: 'run-101' }) }),
      )
    })

    it('renders backup rows with status badges', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const badges = wrapper.findAll('.badge')
      expect(badges.length).toBeGreaterThan(0)
    })

    it('renders a success badge for successful backups', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const successBadges = wrapper.findAll('.badge--success')
      expect(successBadges.length).toBeGreaterThan(0)
    })

    it('renders a failed badge for failed backups', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const failedBadges = wrapper.findAll('.badge--danger')
      expect(failedBadges.length).toBeGreaterThan(0)
    })

    it('renders a warning badge for backups with warnings', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const warningBadges = wrapper.findAll('.badge--warning')
      expect(warningBadges.length).toBeGreaterThan(0)
    })

    it('expands a warning row and shows warnings in the detail panel', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const warningRows = findWarningRow(wrapper)
      expect(warningRows.length).toBeGreaterThan(0)

      await warningRows[0].trigger('click')
      await flushPromises()

      const warningPre = wrapper.find('pre.warning-pre')
      expect(warningPre.exists()).toBe(true)
      expect(warningPre.text()).toContain('some file changed during backup')
      expect(warningPre.text()).toContain('slow read on /var/www/logs')

      // A warning-only report still carries a non-null error_message (the
      // backup_warning notification path reads it), but the Error box must
      // not render alongside the Warnings box for the same report.
      expect(wrapper.find('pre.error-pre').exists()).toBe(false)
    })
    it('keeps the expanded detail panel open when a background DataChanged event arrives', async () => {
      const wrapper = await mountDefault()

      const warningRows = findWarningRow(wrapper)
      await warningRows[0].trigger('click')
      await flushPromises()
      expect(wrapper.find('pre.warning-pre').exists()).toBe(true)

      triggerWsMessage('DataChanged')
      await flushPromises()

      const warningPre = wrapper.find('pre.warning-pre')
      expect(warningPre.exists()).toBe(true)
      expect(warningPre.text()).toContain('some file changed during backup')
    })
  })

  describe('acknowledging warnings and failures', () => {
    function ackButton(row: ReturnType<typeof mount>) {
      return row.findAll('button').find((b) => b.text() === 'Acknowledge')
    }

    it('offers Acknowledge on a warning row', async () => {
      const wrapper = await mountDefault()
      const rows = findWarningRow(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      expect(ackButton(rows[0])).toBeTruthy()
    })

    it('offers Acknowledge on a failed row', async () => {
      const wrapper = await mountDefault()
      const rows = findFailedRow(wrapper)
      expect(rows.length).toBeGreaterThan(0)
      expect(ackButton(rows[0])).toBeTruthy()
    })

    it('does not offer Acknowledge on a successful row', async () => {
      const wrapper = await mountDefault()
      const successRow = wrapper
        .findAll('.run-card:not(.run-card-system) .run-card-summary')
        .find((r) => r.find('.badge--success').exists())
      expect(successRow).toBeTruthy()
      expect(ackButton(successRow!)).toBeUndefined()
    })

    it('acknowledges the report and drops it from the default feed', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = await mountDefault()
      const before = findWarningRow(wrapper).length

      await ackButton(findWarningRow(wrapper)[0])!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/stats/activity/103/acknowledge')
      // The default filter hides acknowledged entries, so the row it was just
      // applied to leaves the feed rather than sitting there dimmed.
      expect(findWarningRow(wrapper).length).toBe(before - 1)
    })

    it('keeps the acknowledged row visible when the filter shows them', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      const wrapper = await mountDefault()
      await setAcknowledgedFilter(wrapper, 'all')

      const row = findWarningRow(wrapper)[0]
      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(row.find('.badge--neutral').text()).toBe('Acknowledged')
      expect(row.findAll('button').find((b) => b.text() === 'Unacknowledge')).toBeTruthy()
    })

    it('clicking Acknowledge does not also expand the row', async () => {
      const wrapper = await mountDefault()
      const row = findWarningRow(wrapper)[0]

      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(wrapper.find('pre.warning-pre').exists()).toBe(false)
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
        return Promise.resolve({ data: [] })
      })
      const wrapper = mountView()
      await flushPromises()
      // An acknowledged row is only on screen at all once the filter asks for
      // it; the fixture above stands in for that server response.
      await setAcknowledgedFilter(wrapper, 'all')

      const row = findWarningRow(wrapper)[0]
      expect(row.find('.badge--neutral').text()).toBe('Acknowledged')

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
      const row = findWarningRow(wrapper)[0]

      await ackButton(row)!.trigger('click')
      await flushPromises()

      expect(row.findAll('button').find((b) => b.text() === 'Acknowledge')).toBeTruthy()
      expect(row.find('.badge--neutral').exists()).toBe(false)
    })

    it('stacks the row actions in one column instead of spreading them', async () => {
      const wrapper = await mountDefault()
      const row = findWarningRow(wrapper)[0]

      const actions = row.find('.run-card-actions')
      expect(actions.exists()).toBe(true)
      expect(actions.findAll('button').length).toBeGreaterThan(1)
      // Every action lives inside the one group; none is a direct child of the
      // space-between footer, which is what spread them across the card.
      expect(row.findAll('.run-card-foot > button')).toHaveLength(0)
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

      const backupBtn = wrapper.findAll('.segmented-option').find((b) => b.text() === 'Backup')
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
    function systemAckButton(wrapper: ReturnType<typeof mount>) {
      return wrapper
        .findAll('.run-card-system button')
        .find((b) => b.text() === 'Acknowledge' || b.text() === 'Unacknowledge')
    }

    it('offers Acknowledge on a failed periodic sync for an admin', async () => {
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      const button = systemAckButton(wrapper)
      expect(button).toBeTruthy()
      expect(button!.text()).toBe('Acknowledge')
    })

    it('acknowledges the event and drops it from the default feed', async () => {
      mockPost.mockResolvedValue({ data: undefined })
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()
      const before = wrapper.findAll('.run-card-system').length

      await systemAckButton(wrapper)!.trigger('click')
      await flushPromises()

      expect(mockPost).toHaveBeenCalledWith('/stats/system-events/2/acknowledge')
      expect(wrapper.findAll('.run-card-system').length).toBe(before - 1)
    })

    it('hides the action from a non-admin, who the server would reject', async () => {
      setupDefaultMocks()
      const wrapper = mountView('operator')
      await flushPromises()

      expect(systemAckButton(wrapper)).toBeUndefined()
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

      const button = systemAckButton(wrapper)
      expect(button!.text()).toBe('Unacknowledge')
      await button!.trigger('click')
      await flushPromises()

      expect(mockDelete).toHaveBeenCalledWith('/stats/system-events/2/acknowledge')
      expect(systemAckButton(wrapper)!.text()).toBe('Acknowledge')
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

      const before = wrapper.findAll('.run-card-system').length
      await systemAckButton(wrapper)!.trigger('click')
      await flushPromises()

      // Clearing the acknowledgment makes it no longer match a view that asks
      // for acknowledged entries only.
      expect(wrapper.findAll('.run-card-system').length).toBe(before - 1)
    })

    it('reports a failure without changing the button state', async () => {
      mockPost.mockRejectedValue(new Error('forbidden'))
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      await systemAckButton(wrapper)!.trigger('click')
      await flushPromises()

      expect(systemAckButton(wrapper)!.text()).toBe('Acknowledge')
      expect(wrapper.findAll('.run-card-system .badge--neutral')).toHaveLength(0)
    })

    it('leaves a success event unacknowledgeable', async () => {
      setupDefaultMocks()
      const wrapper = mountView('admin')
      await flushPromises()

      const successCard = wrapper
        .findAll('.run-card-system')
        .find((c) => c.find('.badge--success').exists())
      expect(successCard).toBeTruthy()
      expect(successCard!.findAll('button')).toHaveLength(0)
    })
  })

  describe('system events', () => {
    it('renders system event rows alongside backup rows in All view', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemCards = wrapper.findAll('.run-card-system')
      expect(systemCards.length).toBe(SYSTEM_EVENTS.length)
    })

    it('renders system event messages', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemCards = wrapper.findAll('.run-card-system')
      const messages = systemCards.map((r) => r.find('.run-card-message').text())
      expect(messages).toContain('Repository sync completed')
      expect(messages).toContain('Periodic sync failed')
    })

    it('shows only system events when System category is active', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemBtn = wrapper.findAll('.segmented-option').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      const backupRows = wrapper.findAll('.run-card:not(.run-card-system)')
      expect(backupRows.length).toBe(0)
    })

    it('expands a system event row on click to show full message', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemCards = wrapper.findAll('.run-card-system')
      expect(systemCards.length).toBeGreaterThan(0)

      expect(wrapper.find('.run-card-system .detail-panel').exists()).toBe(false)

      await systemCards[0].find('.run-card-summary').trigger('click')
      await flushPromises()

      expect(wrapper.find('.run-card-system .detail-panel').exists()).toBe(true)
      expect(wrapper.find('.run-card-system .detail-panel pre.error-pre').text()).toBe(
        SYSTEM_EVENTS[0].message,
      )
    })

    it('collapses a system event row on second click', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemCards = wrapper.findAll('.run-card-system')
      await systemCards[0].find('.run-card-summary').trigger('click')
      await flushPromises()
      expect(wrapper.find('.run-card-system .detail-panel').exists()).toBe(true)

      await systemCards[0].find('.run-card-summary').trigger('click')
      await flushPromises()
      expect(wrapper.find('.run-card-system .detail-panel').exists()).toBe(false)
    })

    it('adds expanded class to the clicked system event row', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemCards = wrapper.findAll('.run-card-system')
      expect(systemCards[0].classes()).not.toContain('expanded')

      await systemCards[0].find('.run-card-summary').trigger('click')
      await flushPromises()

      expect(systemCards[0].classes()).toContain('expanded')
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

      const rows = wrapper.findAll('.run-card:not(.run-card-system)')
      const nonFailedBadges = rows.filter((r) => r.find('.badge--danger').exists())
      expect(nonFailedBadges.length).toBe(rows.length)
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
      return wrapper.findAll('.run-card:not(.run-card-system) .run-card-summary').length
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

      const logsBtn = wrapper.findAll('.segmented-option').find((b) => b.text() === 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const levelSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Error'))
      expect(levelSelect?.exists()).toBe(true)
      expect(wrapper.find('input.search-input').exists()).toBe(true)
    })

    it('gives each log row a class carrying its level in lower case', async () => {
      // The row class reaches the table as a callback, so rendering alone
      // never runs it - the level modifier is what colours the row, and it
      // has to survive the server reporting the level in upper case.
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: AGENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        if (url === '/logs')
          return Promise.resolve({
            data: [
              {
                timestamp: '2026-01-01T10:00:00Z',
                level: 'ERROR',
                target: 'server',
                message: 'Boom',
              },
            ],
          })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const logsBtn = wrapper.findAll('.segmented-option').find((b) => b.text() === 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const table = wrapper
        .findAllComponents({ name: 'DataTable' })
        .find((t) => typeof t.props('rowClass') === 'function')
      expect(table).toBeTruthy()

      const rowClass = table!.props('rowClass') as (entry: { level: string }) => string
      expect(rowClass({ level: 'ERROR' })).toBe('log-entry-row log-level-error')
      expect(rowClass({ level: 'warn' })).toBe('log-entry-row log-level-warn')
    })
  })

  describe('system event badges', () => {
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

      const systemBtn = wrapper.findAll('.segmented-option').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      const badges = wrapper.findAll('.run-card .badge')
      for (const { label, badge } of SYSTEM_EVENTS) {
        const found = badges.find((b) => b.text() === label)
        expect(found, `no badge for ${label}`).toBeDefined()
        expect(found!.classes()).toContain(badge)
      }
      // A failure must never be reachable by any other arm.
      expect(badges.filter((b) => b.classes().includes('badge--danger'))).toHaveLength(1)
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
})
