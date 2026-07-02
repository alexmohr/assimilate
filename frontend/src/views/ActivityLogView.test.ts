// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'

vi.mock('../composables/useTimezone', () => ({
  useTimezone: vi.fn(),
  getConfiguredTimezone: vi.fn().mockReturnValue(undefined),
}))

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn() },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    status: { value: 'connected' },
    onMessage: vi.fn(),
  }),
}))

import { apiClient } from '../api/client'
import ActivityLogView from './ActivityLogView.vue'

const mockGet = vi.mocked(apiClient.get)

interface ActivityRow {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
}

interface SystemEvent {
  id: number
  created_at: string
  event_type: string
  hostname: string | null
  message: string
}

interface Agent {
  id: number
  hostname: string
}

const CLIENTS: Agent[] = [
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
  },
]

const SYSTEM_EVENTS: SystemEvent[] = [
  {
    id: 1,
    created_at: '2026-01-01T07:00:00Z',
    event_type: 'AgentConnected',
    hostname: 'web-server-01',
    message: 'Agent connected',
  },
  {
    id: 2,
    created_at: '2026-01-01T06:00:00Z',
    event_type: 'AgentDisconnected',
    hostname: 'db-server-01',
    message: 'Agent disconnected',
  },
]

function createTestRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div />' } }],
  })
}

function mountView(): ReturnType<typeof mount> {
  return mount(ActivityLogView, {
    global: {
      plugins: [createPinia(), createTestRouter()],
      stubs: {
        DataTable: { template: '<div class="p-datatable"><slot /><slot name="empty" /></div>' },
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
    if (url === '/agents') return Promise.resolve({ data: CLIENTS })
    if (url === '/stats/activity') return Promise.resolve({ data: ACTIVITY_ROWS })
    if (url === '/stats/system-events') return Promise.resolve({ data: SYSTEM_EVENTS })
    return Promise.resolve({ data: [] })
  })
}

describe('ActivityLogView', () => {
  beforeEach(() => {
    mockGet.mockReset()
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

      const btnTexts = wrapper.findAll('.segment-btn').map((b) => b.text())
      expect(btnTexts).toContain('All')
      expect(btnTexts).toContain('Backup')
      expect(btnTexts).toContain('System')
      expect(btnTexts).toContain('Server Logs')
    })

    it('marks All button as active by default', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const allBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'All')
      expect(allBtn?.classes()).toContain('active')
    })

    it('switches to Backup category when Backup button is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const backupBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'Backup')
      await backupBtn?.trigger('click')
      await flushPromises()

      expect(backupBtn?.classes()).toContain('active')
    })

    it('switches to System category when System button is clicked', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      expect(systemBtn?.classes()).toContain('active')
    })
  })

  describe('empty state', () => {
    it('shows empty state when no activity data is returned', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: [] })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.empty-state').exists()).toBe(true)
      expect(wrapper.find('.empty-title').text()).toBe('No activity')
    })

    it('does not show the table when there is no data', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: [] })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.table-wrap').exists()).toBe(false)
    })
  })

  describe('activity table with data', () => {
    it('renders the table wrapper when backup rows are present', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.table-wrap').exists()).toBe(true)
      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })

    it('renders backup rows with hostname and target columns', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const rows = wrapper.findAll('tr.log-row')
      expect(rows.length).toBeGreaterThan(0)

      const firstRow = rows[0]
      expect(firstRow.find('.cell-host').text()).toBeTruthy()
      expect(firstRow.find('.cell-target').text()).toBeTruthy()
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

      const successBadges = wrapper.findAll('.badge-success')
      expect(successBadges.length).toBeGreaterThan(0)
    })

    it('renders a failed badge for failed backups', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const failedBadges = wrapper.findAll('.badge-failed')
      expect(failedBadges.length).toBeGreaterThan(0)
    })

    it('renders a warning badge for backups with warnings', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const warningBadges = wrapper.findAll('.badge-warning')
      expect(warningBadges.length).toBeGreaterThan(0)
    })
  })

  describe('system events', () => {
    it('renders system event rows alongside backup rows in All view', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemRows = wrapper.findAll('tr.row-system')
      expect(systemRows.length).toBe(SYSTEM_EVENTS.length)
    })

    it('renders system event messages', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemRows = wrapper.findAll('tr.row-system')
      const messages = systemRows.map((r) => r.find('.cell-message').text())
      expect(messages).toContain('Agent connected')
      expect(messages).toContain('Agent disconnected')
    })

    it('shows only system events when System category is active', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      const backupRows = wrapper.findAll('tr.log-row:not(.row-system)')
      expect(backupRows.length).toBe(0)
    })

    it('expands a system event row on click to show full message', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemRows = wrapper.findAll('tr.row-system')
      expect(systemRows.length).toBeGreaterThan(0)

      expect(wrapper.find('tr.detail-row').exists()).toBe(false)

      await systemRows[0].trigger('click')
      await flushPromises()

      expect(wrapper.find('tr.detail-row').exists()).toBe(true)
      expect(wrapper.find('tr.detail-row pre.error-pre').text()).toBe(SYSTEM_EVENTS[0].message)
    })

    it('collapses a system event row on second click', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemRows = wrapper.findAll('tr.row-system')
      await systemRows[0].trigger('click')
      await flushPromises()
      expect(wrapper.find('tr.detail-row').exists()).toBe(true)

      await systemRows[0].trigger('click')
      await flushPromises()
      expect(wrapper.find('tr.detail-row').exists()).toBe(false)
    })

    it('adds expanded class to the clicked system event row', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      const systemRows = wrapper.findAll('tr.row-system')
      expect(systemRows[0].classes()).not.toContain('expanded')

      await systemRows[0].trigger('click')
      await flushPromises()

      expect(systemRows[0].classes()).toContain('expanded')
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

      const clearBtn = wrapper.find('.btn-clear')
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

      const rows = wrapper.findAll('tr.log-row:not(.row-system)')
      const nonFailedBadges = rows.filter((r) => r.find('.badge-failed').exists())
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

      await wrapper.find('.btn-clear').trigger('click')
      await flushPromises()

      expect((statusSelect?.element as HTMLSelectElement).value).toBe('all')
    })
  })

  describe('load more', () => {
    it('shows Load More button when hasMore is true', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: CLIENTS })
        if (url === '/stats/activity')
          return Promise.resolve({
            data: Array.from({ length: 50 }, (_, i) => ({ ...ACTIVITY_ROWS[0], id: i + 1 })),
          })
        if (url === '/stats/system-events') return Promise.resolve({ data: [] })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.btn-load-more').exists()).toBe(true)
      expect(wrapper.find('.btn-load-more').text()).toBe('Load More')
    })

    it('does not show Load More when data is fewer than page size', async () => {
      setupDefaultMocks()
      const wrapper = mountView()
      await flushPromises()

      expect(wrapper.find('.btn-load-more').exists()).toBe(false)
    })
  })

  describe('server logs tab', () => {
    it('shows log search and level filter when Server Logs tab is active', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: CLIENTS })
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

      const logsBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'Server Logs')
      await logsBtn?.trigger('click')
      await flushPromises()

      const levelSelect = wrapper
        .findAll('select.select-input')
        .find((s) => s.findAll('option').some((o) => o.text() === 'Error'))
      expect(levelSelect?.exists()).toBe(true)
      expect(wrapper.find('input.search-input').exists()).toBe(true)
    })
  })

  describe('system event badges', () => {
    it('colors non-error system events without using the failed badge', async () => {
      mockGet.mockImplementation((url: string) => {
        if (url === '/agents') return Promise.resolve({ data: CLIENTS })
        if (url === '/stats/activity') return Promise.resolve({ data: [] })
        if (url === '/stats/system-events')
          return Promise.resolve({
            data: [
              {
                id: 1,
                created_at: '2026-01-01T07:00:00Z',
                event_type: 'repo_sync',
                hostname: null,
                message: 'repo sync completed: imported 14, removed 0 archives in 0s',
              },
              {
                id: 2,
                created_at: '2026-01-01T06:00:00Z',
                event_type: 'repo_sync_failed',
                hostname: null,
                message: 'repo sync failed',
              },
            ],
          })
        return Promise.resolve({ data: [] })
      })

      const wrapper = mountView()
      await flushPromises()

      const systemBtn = wrapper.findAll('.segment-btn').find((b) => b.text() === 'System')
      await systemBtn?.trigger('click')
      await flushPromises()

      const badges = wrapper.findAll('td .badge')
      const successBadge = badges.find((b) => b.text() === 'repo_sync')
      const failedBadge = badges.find((b) => b.text() === 'repo_sync_failed')
      expect(successBadge?.classes()).toContain('badge-success')
      expect(successBadge?.classes()).not.toContain('badge-failed')
      expect(failedBadge?.classes()).toContain('badge-failed')
    })
  })

  describe('API integration', () => {
    it('fetches clients and activity data on mount', async () => {
      setupDefaultMocks()
      mountView()
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith('/agents')
      expect(mockGet).toHaveBeenCalledWith('/stats/activity', expect.any(Object))
      expect(mockGet).toHaveBeenCalledWith('/stats/system-events', expect.any(Object))
    })
  })
})
