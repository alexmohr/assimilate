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

const agents = [
  {
    id: 1,
    hostname: 'protected-host',
    display_name: null,
    agent_version: null,
    agent_git_sha: null,
    agent_build_time: null,
    agent_commit_count: null,
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
    agent_commit_count: null,
    created_at: '2026-06-01T00:00:00Z',
    last_seen_at: null,
    is_connected: false,
    is_imported: false,
    is_hidden: false,
    default_backup_paths: [],
  },
]

function makeRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/:pathMatch(.*)*', component: defineComponent({ render: (): null => null }) },
    ],
  })
}

async function mountWithAgent(
  agentOverrides: Record<string, unknown>,
  versionData: Record<string, unknown>,
): Promise<ReturnType<typeof mount>> {
  const agent = {
    id: 99,
    hostname: 'test-agent',
    display_name: null,
    agent_version: '0.1.0',
    agent_git_sha: null,
    agent_build_time: null,
    agent_commit_count: null,
    created_at: '2026-01-01T00:00:00Z',
    last_seen_at: '2026-01-01T00:00:00Z',
    is_connected: true,
    is_imported: false,
    is_hidden: false,
    default_backup_paths: [],
    ...agentOverrides,
  }
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: [agent] })
    if (url === '/system/version') return Promise.resolve({ data: versionData })
    if (url === '/stats/dashboard-overview')
      return Promise.resolve({
        data: {
          protection: {
            protected_agent_links: [],
            unassigned_agents: [],
            never_succeeded_agents: [],
            disabled_only_agents: [],
          },
        },
      })
    return Promise.resolve({ data: [] })
  })
  const router = makeRouter()
  await router.push('/agents')
  await router.isReady()
  const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
  await flushPromises()
  return wrapper
}

describe('HostsView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agents })
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [{ agent_id: 1, hostname: 'protected-host' }],
              unassigned_agents: [],
              never_succeeded_agents: [{ agent_id: 2, hostname: 'never-succeeded-host' }],
              disabled_only_agents: [],
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
    const router = makeRouter()
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

  it('formats relative last-seen times and agent versions', async () => {
    const recent = new Date(Date.now() - 90 * 60 * 1000).toISOString()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            {
              id: 1,
              hostname: 'versioned-host',
              display_name: null,
              agent_version: 'v1.2.3',
              agent_git_sha: null,
              agent_build_time: null,
              created_at: '2026-06-01T00:00:00Z',
              last_seen_at: recent,
              is_connected: true,
              is_imported: false,
              is_hidden: false,
              default_backup_paths: [],
            },
          ],
        })
      }
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
          },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({ data: { agent_version: null } })
      }
      return Promise.resolve({ data: [] })
    })

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        {
          path: '/:pathMatch(.*)*',
          component: defineComponent({ render: (): null => null }),
        },
      ],
    })
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, {
      global: { plugins: [createPinia(), router] },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('versioned-host')
    expect(text).toContain('v1.2.3')
    expect(text).toContain('h ago')
  })
})

describe('HostsView issue rows', () => {
  const issueAgent = {
    id: 42,
    hostname: 'flaky-host',
    display_name: null,
    agent_version: null,
    agent_git_sha: null,
    agent_build_time: null,
    agent_commit_count: null,
    created_at: '2026-06-01T00:00:00Z',
    last_seen_at: null,
    is_connected: true,
    is_imported: false,
    is_hidden: false,
    default_backup_paths: [],
  }

  async function mountWithHealth(): Promise<{
    wrapper: ReturnType<typeof mount>
    router: ReturnType<typeof createRouter>
  }> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [issueAgent] })
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [
            {
              hostname: 'flaky-host',
              target_name: 'offsite',
              last_status: 'failed',
              last_backup_at: '2026-01-01T00:00:00Z',
              is_overdue: false,
              last_error_message: 'Network is unreachable',
            },
            {
              hostname: 'flaky-host',
              target_name: 'onsite',
              last_status: 'success',
              last_backup_at: '2026-01-01T00:00:00Z',
              is_overdue: true,
              last_error_message: null,
            },
          ],
        })
      }
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
          },
        })
      }
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()
    return { wrapper, router }
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders separate failed and overdue rows instead of a combined label', async () => {
    const { wrapper } = await mountWithHealth()

    const failedRow = wrapper.find('.issue-row-failed')
    const overdueRow = wrapper.find('.issue-row-overdue')
    expect(failedRow.exists()).toBe(true)
    expect(overdueRow.exists()).toBe(true)
    expect(failedRow.text()).toContain('1 failed')
    expect(overdueRow.text()).toContain('1 overdue')
  })

  it('navigates to the backups tab filtered to failed when the failed row is clicked', async () => {
    const { wrapper, router } = await mountWithHealth()

    await wrapper.find('.issue-row-failed').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'backups', status: 'failed' })
  })

  it('navigates to the schedules tab filtered to overdue when the overdue row is clicked', async () => {
    const { wrapper, router } = await mountWithHealth()

    await wrapper.find('.issue-row-overdue').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'schedules', health: 'overdue' })
  })
})

describe('HostsView deploy button label', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows Deploy for agent with no version', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: null, agent_commit_count: null },
      { agent_version: null, server_commit_count: null },
    )
    expect(wrapper.text()).toContain('Deploy')
    expect(wrapper.text()).not.toContain('Upgrade')
  })

  it('shows no button when no binary is available and no commit counts', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: null },
      { agent_version: null, server_commit_count: null },
    )
    expect(wrapper.text()).not.toContain('Upgrade')
    expect(wrapper.text()).not.toContain('Deploy')
  })

  it('shows no button when agent version matches available binary', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: null },
      { agent_version: '0.1.0', server_commit_count: null },
    )
    expect(wrapper.text()).not.toContain('Upgrade')
    expect(wrapper.text()).not.toContain('Deploy')
  })

  it('shows Upgrade when a newer binary is available', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: null },
      { agent_version: '0.2.0', server_commit_count: null },
    )
    expect(wrapper.text()).toContain('Upgrade')
  })

  it('shows no button when agent commit count matches server', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: 150 },
      { agent_version: '0.1.0', server_commit_count: 150 },
    )
    expect(wrapper.text()).not.toContain('Upgrade')
    expect(wrapper.text()).not.toContain('Deploy')
  })

  it('shows Upgrade when agent commit count is behind server', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: 100 },
      { agent_version: '0.1.0', server_commit_count: 200 },
    )
    expect(wrapper.text()).toContain('Upgrade')
  })
})
