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

import { apiClient } from '../api/client'
import AuditLogView from './AuditLogView.vue'

const mockGet = vi.mocked(apiClient.get)

interface AuditEntry {
  id: number
  user_id: number | null
  username: string
  action: string
  target_type: string | null
  target_id: number | null
  details: Record<string, unknown> | null
  ip_address: string | null
  created_at: string
}

const AUDIT_ENTRIES: AuditEntry[] = [
  {
    id: 1,
    user_id: 1,
    username: 'admin',
    action: 'create',
    target_type: 'repository',
    target_id: 10,
    details: { name: 'main-repo', compression: 'lz4' },
    ip_address: '192.168.1.1',
    created_at: '2026-01-01T10:00:00Z',
  },
  {
    id: 2,
    user_id: 1,
    username: 'admin',
    action: 'login',
    target_type: null,
    target_id: null,
    details: null,
    ip_address: '10.0.0.1',
    created_at: '2026-01-01T09:00:00Z',
  },
  {
    id: 3,
    user_id: 2,
    username: 'operator1',
    action: 'update',
    target_type: 'quota',
    target_id: 5,
    details: { warn_bytes: 1073741824, critical_bytes: 2147483648 },
    ip_address: null,
    created_at: '2026-01-01T08:00:00Z',
  },
]

function createTestRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div />' } }],
  })
}

function mountView(userRole: 'admin' | 'viewer' | null = 'admin'): ReturnType<typeof mount> {
  const pinia = createPinia()
  pinia.use(({ store }) => {
    if (store.$id === 'auth') {
      store.$patch({
        user: userRole
          ? {
              id: 1,
              username: 'admin',
              role: userRole,
              must_change_password: false,
              created_at: '2026-01-01T00:00:00Z',
              last_login_at: null,
            }
          : null,
      })
    }
  })

  return mount(AuditLogView, {
    global: {
      plugins: [pinia, createTestRouter()],
      stubs: {
        DataTable: { template: '<div class="p-datatable"><slot /><slot name="empty" /></div>' },
        Column: true,
        BaseSpinner: { template: '<div class="spinner" />' },
        EmptyState: {
          props: ['title', 'description'],
          template: '<div class="empty-state"><span class="empty-title">{{ title }}</span></div>',
        },
        ShieldAlert: { template: '<span />' },
      },
    },
  })
}

describe('AuditLogView', () => {
  beforeEach(() => {
    mockGet.mockReset()
  })

  describe('access control', () => {
    it('shows access denied message for non-admin users', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('viewer')
      await flushPromises()

      expect(wrapper.find('.access-denied').exists()).toBe(true)
      expect(wrapper.find('.audit-log').exists()).toBe(false)
    })

    it('shows audit log for admin users', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.audit-log').exists()).toBe(true)
      expect(wrapper.find('.access-denied').exists()).toBe(false)
    })

    it('shows access denied when user is null', async () => {
      const wrapper = mountView(null)
      await flushPromises()

      expect(wrapper.find('.access-denied').exists()).toBe(true)
    })
  })

  describe('empty state', () => {
    it('renders empty state when no audit entries are returned', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const emptyState = wrapper.find('.empty-state')
      expect(emptyState.exists()).toBe(true)
      expect(emptyState.text()).toContain('No audit entries')
    })

    it('does not show the table wrapper when entries list is empty', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.table-wrap').exists()).toBe(false)
    })
  })

  describe('entries table', () => {
    it('renders the table wrapper when entries are present', async () => {
      mockGet.mockResolvedValue({
        data: { items: AUDIT_ENTRIES, total: AUDIT_ENTRIES.length, page: 1, per_page: 25 },
      })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.table-wrap').exists()).toBe(true)
      expect(wrapper.find('.empty-state').exists()).toBe(false)
    })

    it('displays the total entry count in the header', async () => {
      mockGet.mockResolvedValue({
        data: { items: AUDIT_ENTRIES, total: 42, page: 1, per_page: 25 },
      })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.row-count').text()).toBe('42 entries')
    })

    it('fetches audit log on mount', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      mountView('admin')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledWith(
        '/audit-log',
        expect.objectContaining({ params: expect.any(Object) }),
      )
    })
  })

  describe('filter controls', () => {
    it('renders action filter input', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const inputs = wrapper.findAll('input.filter-input')
      expect(inputs.length).toBeGreaterThanOrEqual(1)
    })

    it('renders date range filter inputs', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const dateInputs = wrapper.findAll('input.date-input')
      expect(dateInputs.length).toBe(2)
    })

    it('renders Apply and Clear buttons', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const buttons = wrapper.findAll('button')
      const texts = buttons.map((b) => b.text())
      expect(texts).toContain('Apply')
      expect(texts).toContain('Clear')
    })

    it('re-fetches when Apply button is clicked', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()
      mockGet.mockClear()

      const applyBtn = wrapper.findAll('button').find((b) => b.text() === 'Apply')
      await applyBtn?.trigger('click')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledTimes(1)
    })

    it('re-fetches and resets filters when Clear button is clicked', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const actionInput = wrapper.find('input.filter-input')
      await actionInput.setValue('create')

      mockGet.mockClear()

      const clearBtn = wrapper.findAll('button').find((b) => b.text() === 'Clear')
      await clearBtn?.trigger('click')
      await flushPromises()

      expect(mockGet).toHaveBeenCalledTimes(1)
      expect((actionInput.element as HTMLInputElement).value).toBe('')
    })
  })

  describe('error state', () => {
    it('shows error message when API call fails', async () => {
      mockGet.mockRejectedValue({ response: { data: { error: 'Unauthorized' } } })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.state-error').exists()).toBe(true)
    })

    it('does not render the table on API error', async () => {
      mockGet.mockRejectedValue({ response: { data: { error: 'Server error' } } })

      const wrapper = mountView('admin')
      await flushPromises()

      expect(wrapper.find('.table-wrap').exists()).toBe(false)
    })
  })

  describe('per-page selector', () => {
    it('renders rows-per-page select with options', async () => {
      mockGet.mockResolvedValue({ data: { items: [], total: 0, page: 1, per_page: 25 } })

      const wrapper = mountView('admin')
      await flushPromises()

      const select = wrapper.find('select.select-input')
      expect(select.exists()).toBe(true)
      const options = select.findAll('option')
      expect(options.map((o) => o.text())).toEqual(['10', '25', '50'])
    })
  })
})
