// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, ref } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from '../api/client'
import HostsView from './HostsView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn>; status: ReturnType<typeof ref> } => ({
    onMessage: vi.fn(),
    status: ref('disconnected'),
  }),
}))

vi.mock('../composables/useMobile', () => ({
  useMobile: (): { isMobile: ReturnType<typeof ref<boolean>> } => ({ isMobile: ref(false) }),
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

const clients = [
  {
    id: 1,
    hostname: 'protected-host',
    display_name: null,
    agent_version: null,
    agent_git_sha: null,
    agent_build_time: null,
    created_at: '2026-06-01T00:00:00Z',
    last_seen_at: null,
    is_connected: true,
    is_imported: false,
    is_hidden: false,
    default_backup_paths: [],
  },
  {
    id: 2,
    hostname: 'never-succeeded-host',
    display_name: null,
    agent_version: null,
    agent_git_sha: null,
    agent_build_time: null,
    created_at: '2026-06-01T00:00:00Z',
    last_seen_at: null,
    is_connected: false,
    is_imported: false,
    is_hidden: false,
    default_backup_paths: [],
  },
]

describe('HostsView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: clients })
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_host_links: [{ client_id: 1, hostname: 'protected-host' }],
              unassigned_hosts: [],
              never_succeeded_hosts: [{ client_id: 2, hostname: 'never-succeeded-host' }],
              disabled_only_hosts: [],
            },
          },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({ data: { agent_version: null } })
      }
      return Promise.resolve({ data: [] })
    })
  })

  it('applies the coverage filter from the route query', async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: '/:pathMatch(.*)*',
          component: defineComponent({ render: (): null => null }),
        },
      ],
    })
    await router.push('/agents?coverage=never-succeeded')
    await router.isReady()
    const wrapper = mount(HostsView, {
      global: { plugins: [createPinia(), router] },
    })

    await flushPromises()

    expect(wrapper.get<HTMLSelectElement>('select[aria-label="Coverage"]').element.value).toBe(
      'never-succeeded',
    )
    expect(wrapper.text()).toContain('never-succeeded-host')
    expect(wrapper.text()).not.toContain('protected-host')
  })
})
