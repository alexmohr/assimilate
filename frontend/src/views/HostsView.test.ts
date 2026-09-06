// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, ref, type ComponentPublicInstance } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
// Imported before '../api/client' on purpose: the hoisted vi.mock factory
// below calls this, so it has to be initialised by the time that module is
// first imported.
import { mockApiClientRw } from '../test-utils/sharedMocks'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import type { CurrentUserResponse } from '../stores/auth'
import HostsView from './HostsView.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import { dismissModal } from '../test-utils'

vi.mock('../api/client', () => mockApiClientRw())

// The handlers the view registers are kept so a test can deliver a live
// backup event, which is otherwise unreachable behind the mock.
const ws = vi.hoisted(() => {
  const handlers: Record<string, (payload: { hostname: string; target_name: string }) => void> = {}
  return {
    handlers,
    onMessage: vi.fn(
      (type: string, cb: (payload: { hostname: string; target_name: string }) => void) => {
        handlers[type] = cb
      },
    ),
  }
})

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn>; status: ReturnType<typeof ref> } => ({
    onMessage: ws.onMessage,
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
  authUserOverrides: Record<string, unknown> = {},
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
          running_operations: [],
        },
      })
    return Promise.resolve({ data: [] })
  })
  const router = makeRouter()
  await router.push('/agents')
  await router.isReady()
  const pinia = createPinia()
  const authStore = useAuthStore(pinia)
  authStore.user = {
    id: 1,
    username: 'test-user',
    role: 'admin',
    must_change_password: false,
    session_expires_at: null,
    remember_me: false,
    can_upgrade_agent: true,
    totp_enabled: false,
    ...authUserOverrides,
  } as CurrentUserResponse
  const wrapper = mount(HostsView, { global: { plugins: [pinia, router] } })
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
            running_operations: [],
          },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({ data: { agent_version: null } })
      }
      return Promise.resolve({ data: [] })
    })
  })

  it('shows a running pill on the agent card when a backup is in progress after reload', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agents })
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
            running_operations: [
              {
                report_id: 1,
                status: 'started',
                hostname: 'protected-host',
                schedule_id: 1,
                schedule_name: 'nightly',
                repo_id: 1,
                repo_name: 'server-daily',
                started_at: '2026-06-01T10:00:00Z',
                destination: { kind: 'schedule', schedule_id: 1 },
              },
            ],
          },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({ data: { agent_version: null } })
      }
      return Promise.resolve({ data: [] })
    })

    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    const cards = wrapper.findAll('.entity-card')
    const protectedCard = cards.find((c) => c.text().includes('protected-host'))
    expect(protectedCard?.find('.entity-running-pill').exists()).toBe(true)
    expect(protectedCard?.text()).toContain('Backing up: server-daily')

    const otherCard = cards.find((c) => c.text().includes('never-succeeded-host'))
    expect(otherCard?.find('.entity-running-pill').exists()).toBe(false)
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

  it('applies the unassigned and disabled-only coverage filters', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agents })
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [],
              unassigned_agents: [{ agent_id: 1, hostname: 'protected-host' }],
              never_succeeded_agents: [],
              disabled_only_agents: [{ agent_id: 2, hostname: 'never-succeeded-host' }],
            },
            running_operations: [],
          },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({ data: { agent_version: null } })
      }
      return Promise.resolve({ data: [] })
    })

    const router = makeRouter()
    await router.push('/agents?coverage=unassigned')
    await router.isReady()
    const wrapper = mount(HostsView, {
      global: { plugins: [createPinia(), router] },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('protected-host')
    expect(wrapper.text()).not.toContain('never-succeeded-host')

    await wrapper.get<HTMLSelectElement>('select[aria-label="Coverage"]').setValue('disabled-only')

    expect(wrapper.text()).toContain('never-succeeded-host')
    expect(wrapper.text()).not.toContain('protected-host')
  })

  it('shows the fleet summary band with agent, online and schedule counts', async () => {
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    const band = wrapper.find('.fleet-summary')
    expect(band.exists()).toBe(true)
    // Fixture has 2 agents, one connected (protected-host).
    expect(band.text()).toContain('2 agents')
    expect(band.text()).toContain('1 online')
  })

  async function mountFleetWithScheduleRoutes(
    overrides: Record<string, () => Promise<unknown>>,
  ): Promise<ReturnType<typeof mount>> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (overrides[url]) return overrides[url]()
      if (url === '/agents') return Promise.resolve({ data: agents })
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            protection: {
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
            running_operations: [],
          },
        })
      }
      return Promise.resolve({ data: [] })
    })

    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()
    return wrapper
  }

  it('counts a schedule targeting multiple agents once in the fleet total, not once per agent', async () => {
    // Schedule 1 targets both agents, so /stats/schedule-counts (grouped
    // per agent_id) reports it twice - the fleet total must still be 1.
    const wrapper = await mountFleetWithScheduleRoutes({
      '/stats/schedule-counts': () =>
        Promise.resolve({
          data: [
            { agent_id: 1, count: 1 },
            { agent_id: 2, count: 1 },
          ],
        }),
      '/schedules': () => Promise.resolve({ data: [{ id: 1 }] }),
    })

    expect(wrapper.find('.fleet-summary').text()).toContain('1 schedule')
  })

  it('falls back to 0 fleet schedules when the /schedules request fails', async () => {
    const wrapper = await mountFleetWithScheduleRoutes({
      '/schedules': () => Promise.reject(new Error('network error')),
    })

    expect(wrapper.find('.fleet-summary').text()).toContain('0 schedules')
  })

  it('still renders the agent list when tags, health, schedule-count and overview requests all fail', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agents })
      return Promise.reject(new Error('network error'))
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect(wrapper.findAll('.entity-card')).toHaveLength(2)
    expect(wrapper.find('.fleet-summary').text()).toContain('2 agents')
  })

  it('groups the fleet summary by agent version', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            { ...agents[0], agent_version: '0.1.89' },
            { ...agents[1], agent_version: '0.1.87' },
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
            running_operations: [],
          },
        })
      }
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: '0.1.89' } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    const chips = wrapper.findAll('.fleet-version-chip')
    expect(chips.map((c) => c.text())).toEqual(
      expect.arrayContaining([
        expect.stringContaining('0.1.89 (current): 1'),
        expect.stringContaining('0.1.87: 1'),
      ]),
    )
  })

  /**
   * The version is the grid's grouping now, not a per-card stat: mounts a
   * fleet spanning two builds plus an agent that never reported one.
   */
  async function mountVersionedFleet(
    available: string | null = '0.1.108',
  ): Promise<ReturnType<typeof mount>> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            { ...agents[0], id: 1, hostname: 'old-host', agent_version: '0.1.98' },
            { ...agents[0], id: 2, hostname: 'new-host', agent_version: '0.1.108' },
            { ...agents[0], id: 3, hostname: 'quiet-host', agent_version: null },
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
            running_operations: [],
          },
        })
      }
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: available } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()
    return wrapper
  }

  it('groups the agent grid by reported version, current first and unknown last', async () => {
    const wrapper = await mountVersionedFleet()

    expect(wrapper.findAll('.list-group-title').map((h) => h.text())).toEqual([
      '0.1.108',
      '0.1.98',
      'Unknown',
    ])
    // Numeric collation, or 0.1.98 would sort above 0.1.108 as a string.
    const groups = wrapper.findAll('.list-group')
    expect(groups[0].find('.card-name').text()).toContain('new-host')
    expect(groups[1].find('.card-name').text()).toContain('old-host')
    expect(groups[2].find('.card-name').text()).toContain('quiet-host')
    expect(groups[0].find('.list-group-count').text()).toBe('1 agent')
  })

  it('names each version group against the binary the server has', async () => {
    const wrapper = await mountVersionedFleet()

    const badges = wrapper.findAll('.list-group-header .badge')
    expect(badges.map((b) => b.text())).toEqual(['Current', 'Behind', 'Never reported'])
    expect(badges[0].classes()).toContain('badge--success')
    expect(badges[1].classes()).toContain('badge--warning')
  })

  it('claims no version is behind when the server has no binary to compare against', async () => {
    const wrapper = await mountVersionedFleet(null)

    // Only the never-reported group can still be named without a comparison.
    expect(wrapper.findAll('.list-group-header .badge').map((b) => b.text())).toEqual([
      'Never reported',
    ])
  })

  it('drops the agent version from the card, since the group header carries it', async () => {
    const wrapper = await mountVersionedFleet()

    const labels = wrapper.findAll('.stat-label').map((l) => l.text())
    expect(labels).toContain('Schedules')
    expect(labels).not.toContain('Agent')
  })

  it('marks outdated fleet version chips as behind', async () => {
    const wrapper = await mountVersionedFleet()

    const chips = wrapper.findAll('.fleet-version-chip')
    const current = chips.find((c) => c.text().includes('(current)'))
    const outdated = chips.find((c) => c.text().startsWith('0.1.98'))
    expect(current?.classes()).toContain('fleet-version-chip-current')
    expect(outdated?.classes()).toContain('fleet-version-chip-outdated')
    // An agent that never reported one is neither current nor behind.
    expect(chips.find((c) => c.text().startsWith('unknown'))?.classes()).toEqual([
      'fleet-version-chip',
    ])
  })

  it('splits the fleet band into one health segment per state', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            { ...agents[0], id: 1, hostname: 'clean-host', is_connected: true },
            { ...agents[0], id: 2, hostname: 'failing-host', is_connected: true },
            { ...agents[0], id: 3, hostname: 'bare-host', is_connected: true },
            { ...agents[0], id: 4, hostname: 'gone-host', is_connected: false },
          ],
        })
      }
      if (url === '/stats/schedule-counts') {
        return Promise.resolve({
          data: [
            { agent_id: 1, count: 1 },
            { agent_id: 2, count: 1 },
            { agent_id: 4, count: 1 },
          ],
        })
      }
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [
            {
              hostname: 'clean-host',
              target_name: 'daily',
              last_status: 'success',
              last_backup_at: '2026-01-02T00:00:00Z',
              last_backup_status: 'success',
              is_overdue: false,
              last_error_message: null,
            },
            {
              hostname: 'failing-host',
              target_name: 'daily',
              last_status: 'failed',
              last_backup_at: null,
              last_backup_status: null,
              is_overdue: false,
              last_error_message: 'boom',
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
            running_operations: [],
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

    const segments = wrapper.findAll('.fleet-seg')
    expect(segments.map((s) => s.attributes('title'))).toEqual([
      '1 clean',
      '1 failing',
      '1 unprotected',
      '1 offline',
    ])
    expect(segments[0].classes()).toContain('fleet-tone--clean')
    expect(segments[3].classes()).toContain('fleet-tone--offline')
    expect(wrapper.find('.fleet-track').attributes('aria-label')).toBe(
      'Fleet health: 1 clean, 1 failing, 1 unprotected, 1 offline',
    )
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
            running_operations: [],
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

  const emptyOverviewData = {
    protection: {
      protected_agent_links: [],
      unassigned_agents: [],
      never_succeeded_agents: [],
      disabled_only_agents: [],
    },
    running_operations: [],
  }

  async function mountAgentsList(
    agentsData: unknown[],
    healthData: unknown[] = [],
    scheduleCounts: unknown[] = [],
  ): Promise<{
    wrapper: ReturnType<typeof mount>
    router: ReturnType<typeof createRouter>
  }> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agentsData })
      if (url === '/stats/health') return Promise.resolve({ data: healthData })
      if (url === '/stats/schedule-counts') return Promise.resolve({ data: scheduleCounts })
      if (url === '/stats/dashboard-overview') return Promise.resolve({ data: emptyOverviewData })
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

  async function mountWithHealth(): Promise<{
    wrapper: ReturnType<typeof mount>
    router: ReturnType<typeof createRouter>
  }> {
    return mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          last_status: 'failed',
          last_backup_at: '2026-01-01T00:00:00Z',
          last_backup_status: 'failed',
          is_overdue: false,
          last_error_message: 'Network is unreachable',
        },
        {
          hostname: 'flaky-host',
          target_name: 'onsite',
          last_status: 'success',
          last_backup_at: '2026-01-01T00:00:00Z',
          last_backup_status: 'success',
          is_overdue: true,
          last_error_message: null,
        },
      ],
    )
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders separate failed and overdue issue chips instead of a combined label', async () => {
    const { wrapper } = await mountWithHealth()

    const failedChip = wrapper.find('.entity-issue-chip.sev-danger')
    const overdueChip = wrapper.find('.entity-issue-chip.sev-warning')
    expect(failedChip.exists()).toBe(true)
    expect(overdueChip.exists()).toBe(true)
    expect(failedChip.text()).toContain('1 failed')
    expect(overdueChip.text()).toContain('1 overdue')
  })

  it('navigates to the backups tab filtered to failed when the failed chip is clicked', async () => {
    const { wrapper, router } = await mountWithHealth()

    await wrapper.find('.entity-issue-chip.sev-danger').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'backups', status: 'failed' })
  })

  it('navigates to the schedules tab filtered to overdue when the overdue chip is clicked', async () => {
    const { wrapper, router } = await mountWithHealth()

    await wrapper.find('.entity-issue-chip.sev-warning').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'schedules', health: 'overdue' })
  })

  async function mountSingleAgent(
    overrides: Record<string, unknown>,
    scheduleCounts: unknown[] = [{ agent_id: 42, count: 1 }],
  ): Promise<ReturnType<typeof mount>> {
    const { wrapper } = await mountAgentsList([{ ...issueAgent, ...overrides }], [], scheduleCounts)
    return wrapper
  }

  it('shows an Offline pill and tints the card when the agent is disconnected', async () => {
    const wrapper = await mountSingleAgent({ is_connected: false })

    expect(wrapper.find('.entity-card').classes()).toContain('entity-card--notable')
    expect(wrapper.find('.entity-status-pill').text()).toBe('Offline')
  })

  it('shows nothing in the badge row for a healthy online agent with no issues', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true })

    expect(wrapper.find('.entity-card').classes()).not.toContain('entity-card--notable')
    expect(wrapper.find('.entity-badge-row').exists()).toBe(false)
  })

  it('badges a connected agent as online', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true })

    const badge = wrapper.find('.card-top-badges .badge--success')
    expect(badge.text()).toBe('Online')
    // Live state, so the badge carries the dot; classification badges do not.
    expect(badge.find('.badge-dot').exists()).toBe(true)
  })

  it('does not badge a disconnected agent as online', async () => {
    const wrapper = await mountSingleAgent({ is_connected: false })

    expect(wrapper.find('.card-top-badges .badge--success').exists()).toBe(false)
  })

  it('flags a host nothing is scheduled to back up', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true }, [{ agent_id: 42, count: 0 }])

    const chip = wrapper.find('.entity-issue-chip.sev-danger')
    expect(chip.text()).toBe('No schedules')
  })

  it('opens the schedules tab when the no-schedules chip is clicked', async () => {
    const { wrapper, router } = await mountAgentsList(
      [issueAgent],
      [],
      [{ agent_id: 42, count: 0 }],
    )
    const push = vi.spyOn(router, 'push')

    await wrapper.find('.entity-issue-chip.sev-danger').trigger('click')

    expect(push).toHaveBeenCalledWith(
      expect.objectContaining({
        path: '/agents/flaky-host',
        query: expect.objectContaining({ tab: 'schedules' }),
      }),
    )
  })

  it('does not claim a host is unprotected while the schedule counts are unknown', async () => {
    // The count request failing leaves the map empty, which is not the same
    // as every host having no schedule - the accusation waits for an answer.
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [issueAgent] })
      if (url === '/stats/schedule-counts') return Promise.reject(new Error('down'))
      if (url === '/stats/dashboard-overview') return Promise.resolve({ data: emptyOverviewData })
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect(wrapper.find('.entity-issue-chip.sev-danger').exists()).toBe(false)
  })

  it('does not call a host with health entries unprotected, whatever the counts say', async () => {
    // Health entries only exist for scheduled targets, so the two sources have
    // to agree before the card accuses the host of having nothing.
    const { wrapper } = await mountWithHealth()

    expect(wrapper.findAll('.entity-issue-chip').map((c) => c.text())).not.toContain('No schedules')
  })

  function statTone(wrapper: ReturnType<typeof mount>, label: string): string[] {
    const stat = wrapper.findAll('.stat').find((s) => s.find('.stat-label').text() === label)
    return stat?.find('.stat-value').classes() ?? []
  }

  it('tones the last-backup figure by how the host is actually doing', async () => {
    const fresh = new Date(Date.now() - 60 * 60 * 1000).toISOString()
    const healthy = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'daily',
          last_status: 'success',
          last_backup_at: fresh,
          last_backup_status: 'success',
          is_overdue: false,
          last_error_message: null,
        },
      ],
      [{ agent_id: 42, count: 1 }],
    )
    expect(statTone(healthy.wrapper, 'Last backup')).toContain('stat-value--success')

    const overdue = await mountWithHealth()
    expect(statTone(overdue.wrapper, 'Last backup')).toContain('stat-value--warning')

    const never = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'daily',
          last_status: null,
          last_backup_at: null,
          last_backup_status: null,
          is_overdue: false,
          last_error_message: null,
        },
      ],
      [{ agent_id: 42, count: 1 }],
    )
    expect(statTone(never.wrapper, 'Last backup')).toContain('stat-value--danger')
  })

  it('tones the last-seen figure only once an agent is gone', async () => {
    const online = await mountSingleAgent({ is_connected: true })
    expect(statTone(online, 'Last seen')).toEqual(['stat-value'])

    const recentlyGone = await mountSingleAgent({
      is_connected: false,
      last_seen_at: new Date(Date.now() - 60 * 60 * 1000).toISOString(),
    })
    expect(statTone(recentlyGone, 'Last seen')).toContain('stat-value--warning')

    const longGone = await mountSingleAgent({
      is_connected: false,
      last_seen_at: new Date(Date.now() - 5 * 24 * 60 * 60 * 1000).toISOString(),
    })
    expect(statTone(longGone, 'Last seen')).toContain('stat-value--danger')
  })

  // The card reports the freshest completed backup as a stat; whether the
  // agent is *behind* comes from the server-computed overdue chip, so these
  // cases only assert which backup time the card picks.
  function lastBackupStat(wrapper: ReturnType<typeof mount>): string {
    const stat = wrapper
      .findAll('.stat')
      .find((s) => s.find('.stat-label').text() === 'Last backup')
    return stat?.find('.stat-value').text() ?? ''
  }

  it('shows a running pill while a backup is in flight and clears it when it completes', async () => {
    const { wrapper } = await mountAgentsList([issueAgent])

    ws.handlers['BackupStarted']({ hostname: 'flaky-host', target_name: 'offsite' })
    await flushPromises()
    expect(wrapper.text()).toContain('Backing up: offsite')

    // A repeated start for the same target must not list it twice.
    ws.handlers['BackupStarted']({ hostname: 'flaky-host', target_name: 'offsite' })
    await flushPromises()
    expect(wrapper.text()).toContain('Backing up: offsite')

    ws.handlers['BackupCompleted']({ hostname: 'flaky-host', target_name: 'offsite' })
    await flushPromises()
    expect(wrapper.text()).not.toContain('Backing up')
  })

  it('surfaces a load failure when there is nothing already listed', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('backend is down'))
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect(wrapper.get('.error-banner').text()).toContain('backend is down')
  })

  // The list's own filter/sort pipeline had no coverage: every case below
  // drives a control a user actually operates on the Agents page.
  const filterAgents = [
    {
      id: 1,
      hostname: 'alpha-host',
      display_name: 'Alpha Box',
      domain: null,
      is_connected: false,
      last_seen_at: '2026-01-03T00:00:00Z',
      agent_version: '0.1.9',
      is_hidden: false,
      is_imported: false,
    },
    {
      id: 2,
      hostname: 'beta-host',
      display_name: null,
      domain: null,
      is_connected: true,
      last_seen_at: '2026-01-01T00:00:00Z',
      agent_version: '0.1.11',
      is_hidden: false,
      is_imported: false,
    },
  ]

  async function mountFilterList(): Promise<ReturnType<typeof mount>> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: filterAgents })
      if (url === '/agent-tags')
        return Promise.resolve({
          data: [{ agent_id: 2, tag_name: 'production', tag_color: '#f00' }],
        })
      if (url === '/tags')
        return Promise.resolve({ data: [{ id: 7, name: 'production', color: '#f00' }] })
      if (url === '/stats/dashboard-overview') return Promise.resolve({ data: emptyOverviewData })
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()
    return wrapper
  }

  function cardNames(wrapper: ReturnType<typeof mount>): string[] {
    return wrapper.findAll('.card-name').map((c) => c.text())
  }

  it('filters the list by connection status', async () => {
    const wrapper = await mountFilterList()

    await wrapper.findAll('select')[0].setValue('online')
    expect(cardNames(wrapper)).toEqual(['beta-host'])

    await wrapper.findAll('select')[0].setValue('offline')
    expect(cardNames(wrapper)).toEqual(['alpha-host'])
  })

  it('matches the text filter against hostname, display name and tag name', async () => {
    const wrapper = await mountFilterList()
    const search = wrapper.get('.search-input')

    await search.setValue('beta')
    expect(cardNames(wrapper)).toEqual(['beta-host'])

    // Display name is searchable even though the card leads with the hostname.
    await search.setValue('alpha box')
    expect(cardNames(wrapper)).toEqual(['alpha-host'])

    // ...and so is a tag the agent carries, which appears nowhere in its name.
    await search.setValue('production')
    expect(cardNames(wrapper)).toEqual(['beta-host'])
  })

  it('filters by a selected tag and restores the full list when it is cleared', async () => {
    const wrapper = await mountFilterList()

    await wrapper.get('.tag-filter-wrapper button').trigger('click')
    const tagCheckbox = wrapper.get('.tag-dropdown-item input')
    await tagCheckbox.trigger('change')
    expect(cardNames(wrapper)).toEqual(['beta-host'])

    // Toggling the same tag off is a deselect, not a second filter. Both are
    // back, in version-group order: beta-host reports 0.1.11, alpha-host the
    // older 0.1.9, and the grid groups before it sorts.
    await tagCheckbox.trigger('change')
    expect(cardNames(wrapper)).toEqual(['beta-host', 'alpha-host'])
  })

  it('sorts by status, last seen and version', async () => {
    const wrapper = await mountFilterList()
    const sortButton = (label: string): ReturnType<typeof wrapper.get> =>
      wrapper.findAll('button').filter((b) => b.text().startsWith(label))[0]

    // Online first, against alphabetical order rather than with it.
    await sortButton('Status').trigger('click')
    expect(cardNames(wrapper)).toEqual(['beta-host', 'alpha-host'])

    // Oldest last-seen first while ascending.
    await sortButton('Last seen').trigger('click')
    expect(cardNames(wrapper)).toEqual(['beta-host', 'alpha-host'])

    // Version sorts as a string, so 0.1.11 precedes 0.1.9.
    await sortButton('Version').trigger('click')
    expect(cardNames(wrapper)).toEqual(['beta-host', 'alpha-host'])
  })

  it('reports "Never" as the last backup for an agent that has never run one', async () => {
    const wrapper = await mountSingleAgent({})

    expect(lastBackupStat(wrapper)).toBe('Never')
  })

  it("reports the agent's most recent backup across all of its schedules", async () => {
    const { wrapper } = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          last_status: 'success',
          last_backup_at: new Date(Date.now() - 90 * 60_000).toISOString(),
          last_backup_status: 'success',
          is_overdue: false,
          last_error_message: null,
          cron_expression: '0 */8 * * *',
          schedule_enabled: true,
        },
        {
          hostname: 'flaky-host',
          target_name: 'onsite',
          last_status: 'success',
          last_backup_at: new Date(Date.now() - 150 * 60_000).toISOString(),
          last_backup_status: 'success',
          is_overdue: false,
          last_error_message: null,
          cron_expression: '0 */1 * * *',
          schedule_enabled: true,
        },
      ],
    )

    // Two schedules, 90m and 150m ago: the card reports the fresher one.
    expect(lastBackupStat(wrapper)).toBe('1h ago')
  })

  it("does not report a failed run's finish time as the last backup, even when it is the most recent report", async () => {
    const { wrapper } = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          // Most recent report by far, but it failed - must not count as
          // "covered" just because it has the latest finished_at.
          last_status: 'failed',
          last_backup_at: new Date(Date.now() - 5 * 60_000).toISOString(),
          last_backup_status: 'failed',
          is_overdue: false,
          last_error_message: 'disk full',
          cron_expression: '0 */1 * * *',
          schedule_enabled: true,
        },
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          last_status: 'success',
          last_backup_at: new Date(Date.now() - 5 * 3600_000).toISOString(),
          last_backup_status: 'success',
          is_overdue: false,
          last_error_message: null,
          cron_expression: '0 */1 * * *',
          schedule_enabled: true,
        },
      ],
    )

    // The 5h-old success is the last real backup; the 5m-old failure is not
    // one, however recent its report.
    expect(lastBackupStat(wrapper)).toBe('5h ago')
  })

  it('reports the last completed backup while a newer run is in flight', async () => {
    const { wrapper } = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          // The only schedule for this host, currently mid-run: the backend
          // can't represent "pending"/"started" as a `BackupStatus`, so
          // `last_status` is null - exactly like the API response while a
          // backup is in progress. `last_backup_status` must still carry the
          // prior completed run's outcome so this doesn't read as "no
          // backups yet".
          last_status: null,
          last_backup_at: new Date(Date.now() - 5 * 3600_000).toISOString(),
          last_backup_status: 'success',
          is_overdue: false,
          last_error_message: null,
          cron_expression: '0 */1 * * *',
          schedule_enabled: true,
        },
      ],
    )

    // The prior completed run is still what the card reports - the opposite
    // of 'Never', proving the timestamp was used even though last_status is
    // null while the new run is in flight.
    expect(lastBackupStat(wrapper)).toBe('5h ago')
  })

  // A caller that hand-builds a health entry - an e2e mock intercepting
  // /api/stats/health, an older cached payload - may omit a field the real
  // API always sends, leaving it `undefined` rather than `null`. The health
  // aggregation must tolerate that instead of throwing partway through
  // `healthRes.forEach`, which would abort before `healthByHost.value` is
  // ever assigned and silently blank out every badge on the page - exactly
  // what broke hosts.spec.ts's Failed/Overdue chip tests, whose mocked
  // response has no `last_backup_status` key at all.
  it('still renders failed/overdue chips when a health entry omits last_backup_status', async () => {
    const { wrapper } = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          last_status: 'failed',
          last_backup_at: new Date().toISOString(),
          is_overdue: true,
          last_error_message: 'disk full',
        },
      ],
    )

    expect(wrapper.find('.entity-issue-chip.sev-danger').exists()).toBe(true)
    expect(wrapper.find('.entity-issue-chip.sev-warning').exists()).toBe(true)
  })

  // navigateToAgent has to merge the domain into the query on every card
  // click, not just when a hostname happens to be unique, or a link into an
  // agent sharing its hostname with another silently resolves to whichever
  // one the ambiguous-hostname route happens to pick.
  it('clicking a host card navigates to its detail page with the domain in the query', async () => {
    const { wrapper, router } = await mountAgentsList([
      { ...issueAgent, domain: 'lab.example.com' },
    ])

    await wrapper.find('.entity-card').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ domain: 'lab.example.com' })
  })

  it('unhides an agent, scoped to its domain', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} } as never)
    const { wrapper } = await mountAgentsList([
      { ...issueAgent, is_hidden: true, domain: 'lab.example.com' },
    ])

    const unhideButton = wrapper.findAll('button').find((b) => b.text().trim() === 'Unhide')
    if (!unhideButton) throw new Error('no Unhide button found')
    await unhideButton.trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/flaky-host/unhide',
      {},
      { params: { domain: 'lab.example.com' } },
    )
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

  it('hides the Deploy/Upgrade button without can_upgrade_agent permission', async () => {
    const wrapper = await mountWithAgent(
      { agent_version: '0.1.0', agent_commit_count: null },
      { agent_version: '0.2.0', server_commit_count: null },
      { can_upgrade_agent: false },
    )
    expect(wrapper.text()).not.toContain('Upgrade')
    expect(wrapper.text()).not.toContain('Deploy')
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

  describe('add and adopt dialogs', () => {
    /** The dialog teleports, so its controls are queried off the document. */
    function dialogButton(label: string): HTMLButtonElement {
      const match = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
        (b) => b.textContent?.trim() === label,
      )
      if (!match) throw new Error(`no button labelled "${label}"`)
      return match
    }

    async function openAdd(wrapper: Awaited<ReturnType<typeof mountWithAgent>>) {
      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'New')!
        .trigger('click')
      await flushPromises()
    }

    async function setByPlaceholder(placeholder: string, value: string): Promise<void> {
      const control = document.querySelector<HTMLInputElement>(
        `input[placeholder="${placeholder}"]`,
      )
      if (!control) throw new Error(`no field with placeholder "${placeholder}"`)
      control.value = value
      control.dispatchEvent(new Event('input'))
      await flushPromises()
    }

    /**
     * The hostname requirement is enforced by disabling Create, not by
     * reporting an error afterwards, so submitAdd's own guard is unreachable
     * from the UI. This asserts the gate that actually holds.
     */
    it('keeps Create disabled until a hostname is entered', async () => {
      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)

      expect(dialogButton('Create').disabled).toBe(true)

      await setByPlaceholder('e.g. workstation-01', 'workstation-01')
      expect(dialogButton('Create').disabled).toBe(false)

      await setByPlaceholder('e.g. workstation-01', '   ')
      expect(dialogButton('Create').disabled).toBe(true)
    })

    // Hostnames cannot contain whitespace, so it is stripped rather than
    // rejected - a pasted name with a stray space should still work.
    it('strips whitespace from the hostname and nulls an empty display name', async () => {
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 7, hostname: 'workstation-01' }, token: 'tok_abc' },
      } as never)

      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)
      await setByPlaceholder('e.g. workstation-01', '  workstation 01  ')
      dialogButton('Create').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/agents', {
        hostname: 'workstation01',
        display_name: null,
        domain: null,
      })
    })

    it('sends a trimmed display name when one is given', async () => {
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 7, hostname: 'workstation-01' }, token: 'tok_abc' },
      } as never)

      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)
      await setByPlaceholder('e.g. workstation-01', 'workstation-01')
      await setByPlaceholder('Optional friendly name', '  Front desk  ')
      dialogButton('Create').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/agents', {
        hostname: 'workstation-01',
        display_name: 'Front desk',
        domain: null,
      })
    })

    it('sends a trimmed domain when one is given', async () => {
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 7, hostname: 'workstation-01' }, token: 'tok_abc' },
      } as never)

      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)
      await setByPlaceholder('e.g. workstation-01', 'workstation-01')
      await setByPlaceholder('Optional, e.g. lab.example.com', '  lab.example.com  ')
      dialogButton('Create').click()
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/agents', {
        hostname: 'workstation-01',
        display_name: null,
        domain: 'lab.example.com',
      })
    })

    // The enrolment token is shown once, so the dialog swaps to a reveal step
    // instead of closing, and closing has to clear it.
    it('reveals the enrolment token and clears it on close', async () => {
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 7, hostname: 'workstation-01' }, token: 'tok_abc' },
      } as never)

      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)
      await setByPlaceholder('e.g. workstation-01', 'workstation-01')
      dialogButton('Create').click()
      await flushPromises()

      expect(document.querySelector('.token-text')?.textContent).toBe('tok_abc')
      ;[...document.querySelectorAll<HTMLButtonElement>('button')]
        .find((b) => b.textContent?.includes('Cop'))
        ?.click()
      await flushPromises()

      dialogButton('Done').click()
      await flushPromises()

      expect(document.querySelector('.token-text')).toBeNull()
    })

    it('reports a create failure without revealing a token', async () => {
      vi.mocked(apiClient.post).mockRejectedValue(new Error('hostname taken'))

      const wrapper = await mountWithAgent({}, {})
      await openAdd(wrapper)
      await setByPlaceholder('e.g. workstation-01', 'workstation-01')
      dialogButton('Create').click()
      await flushPromises()

      expect(document.querySelector('.token-text')).toBeNull()
      expect(document.querySelector('.form-error')).not.toBeNull()
    })

    async function adopt(wrapper: Awaited<ReturnType<typeof mountWithAgent>>): Promise<void> {
      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Adopt')!
        .trigger('click')
      await flushPromises()
    }

    // Adopting drops the "(imported)" suffix borg's import added and mints a
    // token, which is shown once - so the token has to reach the dialog and
    // the row has to stop being imported.
    it('adopts an imported agent and reveals its new token once', async () => {
      vi.mocked(apiClient.put).mockResolvedValue({ data: {} } as never)
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 99, hostname: 'test-agent' }, token: 'tok_adopted' },
      } as never)

      const wrapper = await mountWithAgent(
        { is_imported: true, display_name: 'Test (imported)' },
        {},
      )
      await adopt(wrapper)

      expect(apiClient.put).toHaveBeenCalledWith(
        '/agents/test-agent',
        { display_name: 'Test', domain: undefined },
        { params: {} },
      )
      expect(apiClient.post).toHaveBeenCalledWith(
        '/agents/test-agent/regenerate-token',
        {},
        { params: {} },
      )
      expect(document.querySelector('.token-text')?.textContent).toBe('tok_adopted')
      expect(document.body.textContent).toContain('Agent Adopted')
      // The row is no longer imported, so Adopt is gone from it.
      expect(wrapper.findAll('button').some((b) => b.text().trim() === 'Adopt')).toBe(false)

      dialogButton('Copy').click()
      await flushPromises()
      expect(dialogButton('Copied!')).toBeDefined()

      dialogButton('Done').click()
      await flushPromises()
      expect(document.querySelector('.token-text')).toBeNull()
    })

    it('keeps the agent imported when adopting fails', async () => {
      vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))

      const wrapper = await mountWithAgent({ is_imported: true }, {})
      await adopt(wrapper)

      expect(document.querySelector('.token-text')).toBeNull()
      expect(wrapper.findAll('button').some((b) => b.text().trim() === 'Adopt')).toBe(true)
    })

    it('closes the adopt dialog when it is dismissed', async () => {
      vi.mocked(apiClient.put).mockResolvedValue({ data: {} } as never)
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { agent: { id: 99, hostname: 'test-agent' }, token: 'tok_adopted' },
      } as never)

      const wrapper = await mountWithAgent({ is_imported: true }, {})
      await adopt(wrapper)
      expect(document.querySelector('.token-text')).not.toBeNull()

      await dismissModal(wrapper as VueWrapper<ComponentPublicInstance>)

      expect(document.querySelector('.token-text')).toBeNull()
    })

    it('opens the merge dialog for an imported agent and closes it on cancel', async () => {
      const wrapper = await mountWithAgent({ is_imported: true }, {})

      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Merge into...')!
        .trigger('click')
      await flushPromises()

      const dialog = wrapper.findComponent(MergeAgentDialog)
      expect(dialog.exists()).toBe(true)

      dialog.vm.$emit('cancel')
      await flushPromises()

      expect(wrapper.findComponent(MergeAgentDialog).exists()).toBe(false)
    })

    it('opens the deploy dialog for an upgradable agent', async () => {
      const wrapper = await mountWithAgent(
        { agent_version: '0.1.0' },
        { agent_version: '0.2.0', server_commit_count: null },
      )

      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Upgrade')!
        .trigger('click')
      await flushPromises()

      const dialog = wrapper.findComponent(AgentDeployDialog)
      expect(dialog.exists()).toBe(true)
      expect(dialog.props('hostname')).toBe('test-agent')
      expect(dialog.props('agentVersion')).toBe('0.1.0')
      expect(dialog.props('availableVersion')).toBe('0.2.0')

      dialog.vm.$emit('close')
      await flushPromises()

      expect(wrapper.findComponent(AgentDeployDialog).exists()).toBe(false)
    })
  })

  it('unhides a hidden agent', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} } as never)

    const wrapper = await mountWithAgent({ is_hidden: true }, {})

    await wrapper
      .findAll('button')
      .find((b) => b.text().trim() === 'Unhide')!
      .trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/agents/test-agent/unhide', {}, { params: {} })
  })
})
