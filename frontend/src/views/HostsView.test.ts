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
import { useSystemModeStore } from '../stores/systemMode'
import type { CurrentUserResponse } from '../stores/auth'
import type { DeploymentMode } from '../types/generated'
import HostsView from './HostsView.vue'

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

// Desktop unless a test says otherwise, so the mobile filter toggle can be reached.
const mobile = vi.hoisted(() => ({ isMobile: false }))

vi.mock('../composables/useMobile', () => ({
  useMobile: (): { isMobile: ReturnType<typeof ref<boolean>> } => ({
    isMobile: ref(mobile.isMobile),
  }),
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

/** The status pill whose label starts with `label`. */
function pill(wrapper: ReturnType<typeof mount>, label: string): ReturnType<typeof wrapper.get> {
  const match = wrapper.findAll('.filter-pill').find((b) => b.text().startsWith(label))
  if (!match) throw new Error(`no pill labelled "${label}"`)
  return match
}

/** The label of the status pill that is switched on. */
function activePill(wrapper: ReturnType<typeof mount>): string {
  return wrapper.get('.filter-pill.active').text()
}

/**
 * Opens the filters card behind the Filters button, where coverage, tags,
 * grouping, sort and hidden agents live; a no-op when it is already open.
 */
async function openFilters(wrapper: ReturnType<typeof mount>): Promise<void> {
  const toggle = wrapper.get('.agents-search-row button[aria-expanded]')
  if (toggle.attributes('aria-expanded') === 'true') return
  await toggle.trigger('click')
  await flushPromises()
}

/** The coverage select in the filters card. */
async function coverageSelect(
  wrapper: ReturnType<typeof mount>,
): Promise<ReturnType<typeof wrapper.get<HTMLSelectElement>>> {
  await openFilters(wrapper)
  return wrapper.get<HTMLSelectElement>('#agents-coverage')
}

/** The fleet tile with the given label. */
function tile(wrapper: ReturnType<typeof mount>, label: string): ReturnType<typeof wrapper.get> {
  const match = wrapper.findAll('.fleet-cell').find((c) => c.find('.stat-label').text() === label)
  if (!match) throw new Error(`no tile labelled "${label}"`)
  return match
}

function tileValue(wrapper: ReturnType<typeof mount>, label: string): string {
  return tile(wrapper, label).get('.stat-value').text()
}

/** The hostnames listed, in order, across every group. */
function rowNames(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.list-row-name').map((c) => c.text())
}

/** The group with the given title. */
function group(wrapper: ReturnType<typeof mount>, title: string): ReturnType<typeof wrapper.get> {
  const match = wrapper
    .findAll('.list-group')
    .find((g) => g.find('.list-group-title').text() === title)
  if (!match) throw new Error(`no group titled "${title}"`)
  return match
}

async function mountWithAgent(
  agentOverrides: Record<string, unknown>,
  versionData: Record<string, unknown>,
  authUserOverrides: Record<string, unknown> = {},
  mode: DeploymentMode | null = null,
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
  useSystemModeStore(pinia).mode = mode
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

  it('shows a running pill on the agent row when a backup is in progress after reload', async () => {
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
      // A scheduled host, so no "No schedules" issue takes the status column.
      if (url === '/stats/schedule-counts') {
        return Promise.resolve({ data: [{ agent_id: 1, count: 1 }] })
      }
      return Promise.resolve({ data: [] })
    })

    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    // The status column holds the running backup's destination as plain text;
    // the dot says it is running.
    const rows = wrapper.findAll('.list-row')
    const protectedRow = rows.find((c) => c.text().includes('protected-host'))
    expect(protectedRow?.get('.agents-running').text()).toBe('Backing up: server-daily')
    expect(protectedRow?.get('.agents-running').attributes('title')).toBe(
      'Backing up: server-daily',
    )
    // A backup in flight colours the row's dot, ahead of plain "online".
    expect(protectedRow?.find('.list-row-main .status-dot').classes()).toContain('status-dot--info')
    expect(wrapper.find('.fleet-strip').text()).toContain('1 backing up now')

    const otherRow = rows.find((c) => c.text().includes('never-succeeded-host'))
    expect(otherRow?.find('.agents-running').exists()).toBe(false)
  })

  it('applies the coverage filter from the route query', async () => {
    const router = makeRouter()
    await router.push('/agents?coverage=never-succeeded')
    await router.isReady()
    const wrapper = mount(HostsView, {
      global: { plugins: [createPinia(), router] },
    })

    await flushPromises()

    expect((await coverageSelect(wrapper)).element.value).toBe('never-succeeded')
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

    await (await coverageSelect(wrapper)).setValue('disabled-only')

    expect(wrapper.text()).toContain('never-succeeded-host')
    expect(wrapper.text()).not.toContain('protected-host')
  })

  it('reacts to a coverage query arriving after the page has already mounted', async () => {
    // Distinct from "applies the coverage filter from the route query" above:
    // that one covers the initial-mount override path, this one covers the
    // reactive watcher that handles an in-app navigation (a dashboard link
    // clicked while Agents is already open) landing on a live component.
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect((await coverageSelect(wrapper)).element.value).toBe('all')

    await router.push('/agents?coverage=never-succeeded')
    await flushPromises()

    expect((await coverageSelect(wrapper)).element.value).toBe('never-succeeded')
  })

  it('reacts to a status query arriving after the page has already mounted', async () => {
    // Mirrors "reacts to a coverage query arriving after the page has already
    // mounted" above - filterStatus takes the same query-override treatment
    // as filterCoverage (see the comment above both in HostsView.vue), so it
    // needs the same reactive-watcher coverage.
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    expect(activePill(wrapper)).toMatch(/^All/)
    expect(wrapper.text()).toContain('never-succeeded-host')

    await router.push('/agents?status=offline')
    await flushPromises()

    expect(activePill(wrapper)).toMatch(/^Offline/)
    expect(wrapper.text()).toContain('never-succeeded-host')
    expect(wrapper.text()).not.toContain('protected-host')
  })

  describe('persisted view settings', () => {
    it('restores the status filter from a previous visit', async () => {
      localStorage.setItem('assimilate-agents-filter-status', 'offline')
      const router = makeRouter()
      await router.push('/agents')
      await router.isReady()
      const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
      await flushPromises()

      expect(activePill(wrapper)).toMatch(/^Offline/)
    })

    it('persists a status filter change for the next visit', async () => {
      const router = makeRouter()
      await router.push('/agents')
      await router.isReady()
      const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
      await flushPromises()

      await pill(wrapper, 'Online').trigger('click')
      await flushPromises()

      expect(localStorage.getItem('assimilate-agents-filter-status')).toBe('online')
    })

    it('restores the grouping from a previous visit and persists a change', async () => {
      localStorage.setItem('assimilate-agents-group', 'version')
      const router = makeRouter()
      await router.push('/agents')
      await router.isReady()
      const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
      await flushPromises()
      await openFilters(wrapper)

      const segment = (label: string): ReturnType<typeof wrapper.get> =>
        wrapper.findAll('[role="radio"]').find((b) => b.text() === label)!
      expect(segment('Version').attributes('aria-checked')).toBe('true')
      // Both fixture agents never reported a version.
      expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual(['Unknown'])

      await segment('Status').trigger('click')
      await flushPromises()

      expect(localStorage.getItem('assimilate-agents-group')).toBe('status')
      // Neither fixture agent has a schedule, so both need attention.
      expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual(['Needs attention'])
    })
  })

  it('shows the fleet tiles with agent, online and schedule counts', async () => {
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const wrapper = mount(HostsView, { global: { plugins: [createPinia(), router] } })
    await flushPromises()

    // Fixture has 2 agents, one connected (protected-host).
    expect(tileValue(wrapper, 'Agents')).toBe('2')
    expect(tileValue(wrapper, 'Online')).toBe('1')
    expect(tile(wrapper, 'Online').find('.stat-value').classes()).toContain('stat-value--success')
    // Nothing is backing up, so the Online tile says nothing more.
    expect(tile(wrapper, 'Online').find('.stat-sub').exists()).toBe(false)
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

    expect(tile(wrapper, 'Agents').find('.stat-sub').text()).toBe('1 schedule')
  })

  it('falls back to 0 fleet schedules when the /schedules request fails', async () => {
    const wrapper = await mountFleetWithScheduleRoutes({
      '/schedules': () => Promise.reject(new Error('network error')),
    })

    expect(tile(wrapper, 'Agents').find('.stat-sub').text()).toBe('0 schedules')
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

    expect(wrapper.findAll('.list-row')).toHaveLength(2)
    expect(tileValue(wrapper, 'Agents')).toBe('2')
  })

  /**
   * The version is the grid's grouping now, not a per-card stat: mounts a
   * fleet spanning two builds plus an agent that never reported one.
   */
  async function mountVersionedFleet(
    available: string | null = '0.1.108',
  ): Promise<ReturnType<typeof mount>> {
    localStorage.setItem('assimilate-agents-group', 'version')
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

  it('groups the agent list by reported version, current first and unknown last', async () => {
    const wrapper = await mountVersionedFleet()

    expect(wrapper.findAll('.list-group-title').map((h) => h.text())).toEqual([
      '0.1.108',
      '0.1.98',
      'Unknown',
    ])
    // Numeric collation, or 0.1.98 would sort above 0.1.108 as a string.
    const groups = wrapper.findAll('.list-group')
    expect(groups[0].find('.list-row-name').text()).toContain('new-host')
    expect(groups[1].find('.list-row-name').text()).toContain('old-host')
    expect(groups[2].find('.list-row-name').text()).toContain('quiet-host')
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

  it('names the columns once per group and shows each version in its own column', async () => {
    const wrapper = await mountVersionedFleet()

    const groups = wrapper.findAll('.list-group')
    expect(groups[0].findAll('.list-cols span').map((l) => l.text())).toEqual([
      'Agent',
      'Status',
      'Schedules',
      'Last backup',
      'Last seen',
      'Version',
      '',
    ])
    expect(groups[0].get('.list-row [data-col="version"]').text()).toBe('0.1.108')
    expect(groups[2].get('.list-row [data-col="version"]').text()).toBe('-')
  })

  it('groups agents by status: needs attention (failing first), online, offline', async () => {
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

    expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual([
      'Needs attention',
      'Online',
      'Offline',
    ])
    // Alphabetical order would put bare-host first; a failure outranks a host
    // with nothing scheduled.
    expect(
      group(wrapper, 'Needs attention')
        .findAll('.list-row-name')
        .map((n) => n.text()),
    ).toEqual(['failing-host', 'bare-host'])
    expect(group(wrapper, 'Needs attention').get('.list-group-count').text()).toBe('2 agents')
    expect(
      group(wrapper, 'Needs attention').get('.list-group-header .status-dot').classes(),
    ).toContain('status-dot--warning')
    expect(group(wrapper, 'Online').get('.list-row-name').text()).toBe('clean-host')
    expect(group(wrapper, 'Online').get('.list-group-count').text()).toBe('1 agent')
    expect(group(wrapper, 'Offline').get('.list-row-name').text()).toBe('gone-host')

    // Each row's dot says how that agent is doing.
    const dot = (hostname: string): string[] =>
      wrapper.get(`[data-hostname="${hostname}"] .list-row-main .status-dot`).classes()
    expect(dot('failing-host')).toContain('status-dot--danger')
    expect(dot('bare-host')).toContain('status-dot--warning')
    expect(dot('clean-host')).toContain('status-dot--success')
    expect(dot('gone-host')).toEqual(['status-dot'])

    // The tiles count the same thing the groups show.
    expect(tileValue(wrapper, 'Needs attention')).toBe('2')
    expect(tile(wrapper, 'Needs attention').get('.stat-sub').text()).toBe(
      '1 failing · 0 overdue · 1 unprotected',
    )
    expect(wrapper.findAll('.filter-pill').map((p) => p.text())).toEqual([
      'All · 4',
      'Needs attention · 2',
      'Online · 3',
      'Offline · 1',
    ])

    // The attention pill narrows the list to that group alone.
    await pill(wrapper, 'Needs attention').trigger('click')
    expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual(['Needs attention'])
    expect(rowNames(wrapper)).toEqual(['failing-host', 'bare-host'])
    expect(localStorage.getItem('assimilate-agents-filter-status')).toBe('attention')
  })

  it('ranks a late-only agent after failing and unscheduled ones in the attention group', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            { ...agents[0], id: 1, hostname: 'a-late', is_connected: true },
            { ...agents[0], id: 2, hostname: 'b-bare', is_connected: true },
            { ...agents[0], id: 3, hostname: 'c-failing', is_connected: true },
          ],
        })
      }
      if (url === '/stats/schedule-counts') {
        return Promise.resolve({
          data: [
            { agent_id: 1, count: 1 },
            { agent_id: 3, count: 1 },
          ],
        })
      }
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [
            {
              hostname: 'a-late',
              target_name: 'daily',
              last_status: 'success',
              last_backup_at: '2026-01-02T00:00:00Z',
              last_backup_status: 'success',
              is_overdue: true,
              last_error_message: null,
            },
            {
              hostname: 'c-failing',
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

    // Alphabetical order would put a-late first; a failure outranks a host
    // with nothing scheduled, which outranks one that is only late.
    expect(
      group(wrapper, 'Needs attention')
        .findAll('.list-row-name')
        .map((n) => n.text()),
    ).toEqual(['c-failing', 'b-bare', 'a-late'])
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

  /** A host whose one schedule is late but whose last run succeeded. */
  async function mountOverdueOnly(): Promise<{
    wrapper: ReturnType<typeof mount>
    router: ReturnType<typeof createRouter>
  }> {
    return mountAgentsList(
      [issueAgent],
      [
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
      [{ agent_id: 42, count: 1 }],
    )
  }

  /** The issue chips a row's status column shows. */
  function chips(wrapper: ReturnType<typeof mount>): string[] {
    return wrapper.findAll('.list-row-chips .entity-issue-chip').map((c) => c.text())
  }

  // A row's status column holds one item: its most pressing issue. A failure
  // outranks a late schedule, which the agent's own page still lists.
  it('shows only the most pressing issue: a failure over an overdue schedule', async () => {
    const { wrapper } = await mountWithHealth()

    expect(chips(wrapper)).toEqual(['1 failed'])
    expect(wrapper.get('.list-row-chips .entity-issue-chip').classes()).toContain('sev-danger')
    // The overdue schedule still counts towards the attention tile.
    expect(tile(wrapper, 'Needs attention').get('.stat-sub').text()).toContain('1 overdue')
  })

  it('shows the overdue issue when nothing more pressing is wrong', async () => {
    const { wrapper } = await mountOverdueOnly()

    expect(chips(wrapper)).toEqual(['1 overdue'])
    expect(wrapper.get('.list-row-chips .entity-issue-chip').classes()).toContain('sev-warning')
    expect(wrapper.get('.list-row-main .status-dot').classes()).toContain('status-dot--warning')
  })

  // The issue outranks a running backup too: the column says what is wrong,
  // the dot that something is running.
  it('shows an issue rather than a backup in flight', async () => {
    const { wrapper } = await mountOverdueOnly()

    ws.handlers['BackupStarted']({ hostname: 'flaky-host', target_name: 'onsite' })
    await flushPromises()

    expect(chips(wrapper)).toEqual(['1 overdue'])
    expect(wrapper.find('.agents-running').exists()).toBe(false)
    expect(wrapper.get('.list-row-main .status-dot').classes()).toContain('status-dot--info')
  })

  // A failure is read in the Activity log, where its output is, rather than
  // on the agent's page, which only says that it failed.
  it('opens the activity log filtered to failed runs when the failed chip is clicked', async () => {
    const { wrapper, router } = await mountWithHealth()

    await wrapper.find('.entity-issue-chip.sev-danger').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/activity')
    expect(router.currentRoute.value.query).toEqual({
      category: 'backup',
      hostname: 'flaky-host',
      status: 'failed',
    })
  })

  it('navigates to the schedules tab filtered to overdue when the overdue chip is clicked', async () => {
    const { wrapper, router } = await mountOverdueOnly()

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

  // Offline is said by the dimmed row, its grey dot and its last-seen time;
  // the status column is kept for an issue.
  it('dims the row of a disconnected agent without an Offline pill', async () => {
    const wrapper = await mountSingleAgent({ is_connected: false })

    expect(wrapper.find('.list-row').classes()).toContain('agents-row--dim')
    expect(wrapper.get('.list-row-chips').text()).toBe('')
    expect(wrapper.get('.list-row [data-col="last-seen"]').text()).toBe('Never')
  })

  it('shows nothing in the badge row for a healthy online agent with no issues', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true })

    expect(wrapper.find('.list-row').classes()).not.toContain('agents-row--dim')
    expect(wrapper.find('.entity-badge-row').exists()).toBe(false)
  })

  it('marks a connected agent online with a green dot', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true })

    expect(wrapper.get('.list-row-main .status-dot').classes()).toContain('status-dot--success')
    expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual(['Online'])
  })

  it('does not mark a disconnected agent as online', async () => {
    const wrapper = await mountSingleAgent({ is_connected: false })

    expect(wrapper.get('.list-row-main .status-dot').classes()).toEqual(['status-dot'])
    expect(wrapper.findAll('.list-group-title').map((t) => t.text())).toEqual(['Offline'])
  })

  // Hidden and Imported moved from the status column to the line under the
  // hostname.
  it('dims a hidden agent and names it Hidden under the hostname', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true, is_hidden: true })

    expect(wrapper.find('.list-row').classes()).toContain('agents-row--dim')
    expect(wrapper.get('.list-row-sub.list-wide-only').text()).toBe('Hidden')
    expect(wrapper.get('.list-row-chips').text()).toBe('')
  })

  it('joins the display name, Imported and Hidden on the line under the hostname', async () => {
    const wrapper = await mountSingleAgent({
      display_name: 'Flaky Box',
      is_imported: true,
      is_hidden: true,
    })

    expect(wrapper.get('.list-row-sub.list-wide-only').text()).toBe('Flaky Box · Imported · Hidden')
  })

  it('leaves out the line under the hostname when there is nothing to say', async () => {
    const wrapper = await mountSingleAgent({})

    expect(wrapper.find('.list-row-sub.list-wide-only').exists()).toBe(false)
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

  /** The tone classes on one of the row's figures, by its `data-col`. */
  function cellTone(wrapper: ReturnType<typeof mount>, col: string): string[] {
    return wrapper
      .get(`.list-row [data-col="${col}"]`)
      .classes()
      .filter((c) => c.startsWith('stat-value--'))
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
    expect(cellTone(healthy.wrapper, 'last-backup')).toContain('stat-value--success')

    const overdue = await mountWithHealth()
    expect(cellTone(overdue.wrapper, 'last-backup')).toContain('stat-value--warning')

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
    expect(cellTone(never.wrapper, 'last-backup')).toContain('stat-value--danger')
  })

  it('tones the last-seen figure only once an agent is gone', async () => {
    const online = await mountSingleAgent({ is_connected: true })
    expect(cellTone(online, 'last-seen')).toEqual([])

    const recentlyGone = await mountSingleAgent({
      is_connected: false,
      last_seen_at: new Date(Date.now() - 60 * 60 * 1000).toISOString(),
    })
    expect(cellTone(recentlyGone, 'last-seen')).toContain('stat-value--warning')

    const longGone = await mountSingleAgent({
      is_connected: false,
      last_seen_at: new Date(Date.now() - 5 * 24 * 60 * 60 * 1000).toISOString(),
    })
    expect(cellTone(longGone, 'last-seen')).toContain('stat-value--danger')
  })

  // The row reports the freshest completed backup in its Last backup column;
  // whether the agent is *behind* comes from the server-computed overdue
  // chip, so these cases only assert which backup time the row picks.
  function lastBackupStat(wrapper: ReturnType<typeof mount>): string {
    return wrapper.get('.list-row [data-col="last-backup"]').text()
  }

  it('shows a running backup while it is in flight and clears it when it completes', async () => {
    // Scheduled, so no "No schedules" issue takes the status column.
    const { wrapper } = await mountAgentsList([issueAgent], [], [{ agent_id: 42, count: 1 }])

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

  it('clears the running backup when it is stopped instead of completing', async () => {
    // Scheduled, so no "No schedules" issue takes the status column.
    const { wrapper } = await mountAgentsList([issueAgent], [], [{ agent_id: 42, count: 1 }])

    ws.handlers['BackupStarted']({ hostname: 'flaky-host', target_name: 'offsite' })
    ws.handlers['BackupStarted']({ hostname: 'flaky-host', target_name: 'nightly' })
    await flushPromises()
    expect(wrapper.text()).toContain('Backing up: offsite, nightly')

    ws.handlers['BackupStopped']({ hostname: 'flaky-host', target_name: 'offsite' })
    await flushPromises()
    expect(wrapper.text()).toContain('Backing up: nightly')

    ws.handlers['BackupStopped']({ hostname: 'flaky-host', target_name: 'nightly' })
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
      // Both are scheduled, so neither needs attention and the status groups
      // split them by connection alone.
      if (url === '/stats/schedule-counts')
        return Promise.resolve({
          data: [
            { agent_id: 1, count: 1 },
            { agent_id: 2, count: 1 },
          ],
        })
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

  it('filters the list by connection status with counted pills', async () => {
    const wrapper = await mountFilterList()

    expect(wrapper.findAll('.filter-pill').map((p) => p.text())).toEqual([
      'All · 2',
      'Needs attention · 0',
      'Online · 1',
      'Offline · 1',
    ])

    await pill(wrapper, 'Online').trigger('click')
    expect(rowNames(wrapper)).toEqual(['beta-host'])
    expect(activePill(wrapper)).toMatch(/^Online/)

    await pill(wrapper, 'Offline').trigger('click')
    expect(rowNames(wrapper)).toEqual(['alpha-host'])

    await pill(wrapper, 'All').trigger('click')
    expect(rowNames(wrapper)).toEqual(['beta-host', 'alpha-host'])
  })

  it('counts the pills over the search, not over the whole fleet', async () => {
    const wrapper = await mountFilterList()

    await wrapper.get('.search-input').setValue('beta')

    expect(pill(wrapper, 'All').text()).toBe('All · 1')
    expect(pill(wrapper, 'Offline').text()).toBe('Offline · 0')
  })

  it('matches the text filter against hostname, display name and tag name', async () => {
    const wrapper = await mountFilterList()
    const search = wrapper.get('.search-input')

    await search.setValue('beta')
    expect(rowNames(wrapper)).toEqual(['beta-host'])

    // Display name is searchable even though the card leads with the hostname.
    await search.setValue('alpha box')
    expect(rowNames(wrapper)).toEqual(['alpha-host'])

    // ...and so is a tag the agent carries, which appears nowhere in its name.
    await search.setValue('production')
    expect(rowNames(wrapper)).toEqual(['beta-host'])
  })

  it('filters by a selected tag and restores the full list when it is cleared', async () => {
    const wrapper = await mountFilterList()

    // Tags are pills in the filters card now, not a dropdown of checkboxes.
    await openFilters(wrapper)
    const tagPill = (): ReturnType<typeof wrapper.get> =>
      wrapper.get('[role="group"][aria-label="Tags"] .filter-pill')
    expect(tagPill().text()).toBe('production')
    await tagPill().trigger('click')
    expect(rowNames(wrapper)).toEqual(['beta-host'])
    expect(tagPill().attributes('aria-pressed')).toBe('true')
    // The Filters button says it is narrowing the list.
    expect(wrapper.get('.agents-search-row button[aria-expanded]').text()).toBe('Filters · 1')

    // Toggling the same tag off is a deselect, not a second filter. Both are
    // back, in status-group order: beta-host is online, alpha-host offline,
    // and the list groups before it sorts.
    await tagPill().trigger('click')
    expect(rowNames(wrapper)).toEqual(['beta-host', 'alpha-host'])
    expect(tagPill().attributes('aria-pressed')).toBe('false')
    expect(wrapper.get('.agents-search-row button[aria-expanded]').text()).toBe('Filters')
  })

  it('sorts within a group by name, last seen and version', async () => {
    // All three are offline with nothing due, so they share one group and the
    // sort alone decides their order.
    const { wrapper } = await mountAgentsList(
      [
        {
          ...issueAgent,
          id: 1,
          hostname: 'a-host',
          is_connected: false,
          last_seen_at: '2026-01-03T00:00:00Z',
          agent_version: '0.1.9',
        },
        {
          ...issueAgent,
          id: 2,
          hostname: 'b-host',
          is_connected: false,
          last_seen_at: '2026-01-01T00:00:00Z',
          agent_version: '0.1.11',
        },
        {
          ...issueAgent,
          id: 3,
          hostname: 'c-host',
          is_connected: false,
          last_seen_at: '2026-01-02T00:00:00Z',
          agent_version: '0.1.10',
        },
      ],
      [],
      [1, 2, 3].map((agent_id) => ({ agent_id, count: 1 })),
    )
    await openFilters(wrapper)
    const sortButton = (label: string): ReturnType<typeof wrapper.get> =>
      wrapper.findAll('.sort-controls button').filter((b) => b.text().startsWith(label))[0]

    expect(rowNames(wrapper)).toEqual(['a-host', 'b-host', 'c-host'])

    // Oldest last-seen first while ascending.
    await sortButton('Last seen').trigger('click')
    expect(rowNames(wrapper)).toEqual(['b-host', 'c-host', 'a-host'])

    // Version sorts as a string, so 0.1.10 and 0.1.11 precede 0.1.9.
    await sortButton('Version').trigger('click')
    expect(rowNames(wrapper)).toEqual(['c-host', 'b-host', 'a-host'])

    // A second click on the same field flips the direction.
    await sortButton('Name').trigger('click')
    await sortButton('Name').trigger('click')
    expect(rowNames(wrapper)).toEqual(['c-host', 'b-host', 'a-host'])
  })

  it('sorts online agents first by status, within the attention group', async () => {
    const failed = (hostname: string): Record<string, unknown> => ({
      hostname,
      target_name: 'daily',
      last_status: 'failed',
      last_backup_at: null,
      last_backup_status: 'failed',
      is_overdue: false,
      last_error_message: 'boom',
    })
    const { wrapper } = await mountAgentsList(
      [
        { ...issueAgent, id: 1, hostname: 'off-host', is_connected: false },
        { ...issueAgent, id: 2, hostname: 'on-host', is_connected: true },
      ],
      [failed('off-host'), failed('on-host')],
    )

    expect(rowNames(wrapper)).toEqual(['off-host', 'on-host'])

    await openFilters(wrapper)
    await wrapper
      .findAll('.sort-controls button')
      .filter((b) => b.text().startsWith('Status'))[0]
      .trigger('click')
    expect(rowNames(wrapper)).toEqual(['on-host', 'off-host'])
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

  it('reports the last successful backup when the latest completed run failed', async () => {
    const { wrapper } = await mountAgentsList(
      [issueAgent],
      [
        {
          hostname: 'flaky-host',
          target_name: 'offsite',
          // The host's only schedule: last night's run failed, the one before
          // it succeeded. The card must show that success, not "Never".
          last_status: 'failed',
          last_backup_at: new Date(Date.now() - 12 * 3600_000).toISOString(),
          last_backup_status: 'failed',
          last_success_at: new Date(Date.now() - 36 * 3600_000).toISOString(),
          is_overdue: false,
          last_error_message: 'disk full',
          cron_expression: '0 22 * * *',
          schedule_enabled: true,
        },
      ],
    )

    expect(lastBackupStat(wrapper)).toBe('1d ago')
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

    // Only the most pressing issue is shown, so the failure stands for both;
    // the overdue one still counts towards the attention tile.
    expect(chips(wrapper)).toEqual(['1 failed'])
    expect(tile(wrapper, 'Needs attention').get('.stat-sub').text()).toContain('1 overdue')
  })

  // The row link has to merge the domain into the query on every row, not
  // just when a hostname happens to be unique, or a link into an agent
  // sharing its hostname with another silently resolves to whichever one the
  // ambiguous-hostname route happens to pick.
  it('clicking a host row navigates to its detail page with the domain in the query', async () => {
    const { wrapper, router } = await mountAgentsList([
      { ...issueAgent, domain: 'lab.example.com' },
    ])

    expect(wrapper.get('.list-row').attributes('href')).toBe(
      '/agents/flaky-host?domain=lab.example.com',
    )
    expect(wrapper.get('.list-row-name').text()).toBe('flaky-host (lab.example.com)')
    await wrapper.find('.list-row').trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/agents/flaky-host')
    expect(router.currentRoute.value.query).toMatchObject({ domain: 'lab.example.com' })
  })
})

describe('HostsView add-agent dialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('add dialog', () => {
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
  })
})

describe('HostsView without the dependencies', () => {
  // Dependencies have a page of their own now (DependenciesView.test.ts):
  // the Agents page lists agents only, and its New only ever adds one.
  async function mountPage(): Promise<VueWrapper<ComponentPublicInstance>> {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agents })
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
      return Promise.resolve({ data: [] })
    })
    const router = makeRouter()
    await router.push('/agents')
    await router.isReady()
    const pinia = createPinia()
    useAuthStore(pinia).user = { id: 1, username: 'u', role: 'admin' } as CurrentUserResponse
    const wrapper = mount(HostsView, { global: { plugins: [pinia, router] } })
    await flushPromises()
    return wrapper
  }

  it('shows the agents with no tabs, and never asks for the dependencies', async () => {
    const wrapper = await mountPage()
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(0)
    expect(wrapper.findAll('.list-row')).toHaveLength(2)
    expect(apiClient.get).not.toHaveBeenCalledWith('/dependency-hosts')
  })

  it('makes New open the add-agent dialog', async () => {
    const wrapper = await mountPage()
    await wrapper
      .findAll('button')
      .find((b) => b.text().trim() === 'New')!
      .trigger('click')
    await flushPromises()
    const open = wrapper.findAllComponents({ name: 'BaseModal' }).filter((m) => m.props('open'))
    expect(open.map((m) => m.props('title'))).toEqual(['Add Agent'])
  })
})

describe('HostsView toolbar and empty state', () => {
  async function mountAgentsPage(
    agentList: unknown[],
    role = 'admin',
  ): Promise<VueWrapper<ComponentPublicInstance>> {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agentList })
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
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
    useAuthStore(pinia).user = { id: 1, username: 'u', role } as CurrentUserResponse
    const wrapper = mount(HostsView, { global: { plugins: [pinia, router] } })
    await flushPromises()
    return wrapper
  }

  it('opens the add-agent dialog from the empty state when no agents are registered', async () => {
    const wrapper = await mountAgentsPage([])
    expect(wrapper.text()).toContain('No agents registered')
    const addModal = (): VueWrapper | undefined =>
      wrapper.findAllComponents({ name: 'BaseModal' }).find((m) => m.props('title') === 'Add Agent')
    expect(addModal()?.props('open')).toBe(false)

    await wrapper.get('.empty-action').trigger('click')
    await flushPromises()

    expect(addModal()?.props('open')).toBe(true)
  })

  it('lists hidden agents too once Show hidden is switched on', async () => {
    const wrapper = await mountAgentsPage(agents)
    expect(apiClient.get).toHaveBeenCalledWith('/agents', { params: undefined })
    await openFilters(wrapper)
    const hiddenSwitch = (): ReturnType<typeof wrapper.get> =>
      wrapper.get('[role="switch"][aria-label="Show hidden agents"]')
    expect(hiddenSwitch().attributes('aria-checked')).toBe('false')

    await hiddenSwitch().trigger('click')
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/agents', {
      params: { include_hidden: true },
    })
    expect(hiddenSwitch().attributes('aria-checked')).toBe('true')
    expect(localStorage.getItem('assimilate-agents-show-hidden')).toBe('true')
  })

  // The filters card sits behind the Filters button on every screen now,
  // not only on a phone.
  it.each([
    ['a phone', true],
    ['a wide screen', false],
  ])('keeps the filters behind the Filters button on %s', async (_screen, isMobile) => {
    mobile.isMobile = isMobile
    try {
      const wrapper = await mountAgentsPage(agents)
      const toggle = wrapper.get('.agents-search-row button[aria-expanded]')
      expect(toggle.attributes('aria-expanded')).toBe('false')
      expect(wrapper.find('select').exists()).toBe(false)
      expect(wrapper.find('[role="switch"]').exists()).toBe(false)
      expect(wrapper.find('[role="radiogroup"]').exists()).toBe(false)
      // The status pills stay out: they are the filter reached for most.
      expect(wrapper.findAll('.filter-pill')).toHaveLength(4)

      await toggle.trigger('click')
      expect(toggle.attributes('aria-expanded')).toBe('true')
      expect(wrapper.findAll('select')).toHaveLength(1)
      expect(wrapper.find('[role="switch"][aria-label="Show hidden agents"]').exists()).toBe(true)
      expect(wrapper.find('[role="radiogroup"]').exists()).toBe(true)

      await toggle.trigger('click')
      expect(wrapper.find('select').exists()).toBe(false)
    } finally {
      mobile.isMobile = false
    }
  })

  it('counts the coverage and hidden filters on the Filters button', async () => {
    const wrapper = await mountAgentsPage(agents)
    const toggle = (): ReturnType<typeof wrapper.get> =>
      wrapper.get('.agents-search-row button[aria-expanded]')
    expect(toggle().text()).toBe('Filters')

    await (await coverageSelect(wrapper)).setValue('unassigned')
    expect(toggle().text()).toBe('Filters · 1')

    await wrapper.get('[role="switch"][aria-label="Show hidden agents"]').trigger('click')
    await flushPromises()
    expect(toggle().text()).toBe('Filters · 2')
  })
})

describe('HostsView list rows', () => {
  function offlineAgent(id: number): Record<string, unknown> {
    return {
      ...agents[1],
      id,
      hostname: `offline-${String(id).padStart(2, '0')}`,
      display_name: id === 1 ? 'First box' : null,
      agent_version: '0.1.0',
    }
  }

  async function mountRows(
    agentList: unknown[],
    scheduleCounts: unknown[] = [],
  ): Promise<{
    wrapper: VueWrapper<ComponentPublicInstance>
    router: ReturnType<typeof createRouter>
  }> {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agentList })
      if (url === '/stats/schedule-counts') return Promise.resolve({ data: scheduleCounts })
      if (url === '/system/version') return Promise.resolve({ data: { agent_version: null } })
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
    useAuthStore(pinia).user = {
      id: 1,
      username: 'u',
      role: 'admin',
      can_upgrade_agent: true,
    } as CurrentUserResponse
    const wrapper = mount(HostsView, { global: { plugins: [pinia, router] } })
    await flushPromises()
    return { wrapper, router }
  }

  it('shows the first five offline agents and toggles the rest', async () => {
    const list = [1, 2, 3, 4, 5, 6, 7].map(offlineAgent)
    const { wrapper } = await mountRows(
      list,
      list.map((a) => ({ agent_id: a.id, count: 1 })),
    )

    const offline = group(wrapper, 'Offline')
    expect(offline.get('.list-group-count').text()).toBe('7 agents')
    expect(rowNames(wrapper)).toEqual([
      'offline-01',
      'offline-02',
      'offline-03',
      'offline-04',
      'offline-05',
    ])
    const more = wrapper.get('.list-more')
    expect(more.text()).toBe('Show 2 more offline agents')

    await more.trigger('click')
    expect(rowNames(wrapper)).toHaveLength(7)
    expect(wrapper.get('.list-more').text()).toBe('Show fewer')

    await wrapper.get('.list-more').trigger('click')
    expect(rowNames(wrapper)).toHaveLength(5)
  })

  it('shows every row when grouped by version', async () => {
    localStorage.setItem('assimilate-agents-group', 'version')
    const list = [1, 2, 3, 4, 5, 6, 7].map(offlineAgent)
    const { wrapper } = await mountRows(
      list,
      list.map((a) => ({ agent_id: a.id, count: 1 })),
    )

    expect(rowNames(wrapper)).toHaveLength(7)
    expect(wrapper.find('.list-more').exists()).toBe(false)
  })

  it('puts a phone summary of display name, schedules and version under the name', async () => {
    const { wrapper } = await mountRows([offlineAgent(1)], [{ agent_id: 1, count: 2 }])

    expect(wrapper.get('.list-row-sub.list-narrow-only').text()).toBe(
      'First box · 2 schedules · 0.1.0',
    )
    expect(wrapper.get('.list-row-sub.list-wide-only').text()).toBe('First box')
    expect(wrapper.get('.agents-row-when').text()).toBe('Never')
  })

  // Rows used to end in a "..." menu when there was something to do from the
  // list (adopt, merge, deploy, unhide). Those live on the agent's page now,
  // so every row is just a link to it.
  it('ends every row in a chevron, with no row menu', async () => {
    const { wrapper } = await mountRows([
      { ...agents[0], hostname: 'imported-host', is_imported: true },
      { ...agents[0], id: 2, hostname: 'hidden-host', is_hidden: true },
      { ...agents[0], id: 3, hostname: 'bare-host', agent_version: null },
    ])

    const rows = wrapper.findAll('.list-row')
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) {
      expect(row.find('.list-row-chevron').exists()).toBe(true)
      expect(row.find('.overflow-toggle').exists()).toBe(false)
    }
  })

  it('lets an issue chip inside a row go to its own target, not the row link', async () => {
    const { wrapper, router } = await mountRows(
      [{ ...agents[0], hostname: 'bare-host' }],
      [{ agent_id: 1, count: 0 }],
    )
    const push = vi.spyOn(router, 'push')

    await wrapper.get('.list-row .entity-issue-chip').trigger('click')
    await flushPromises()

    expect(push).toHaveBeenCalledTimes(1)
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'schedules' })
  })

  it('shows an empty state when the filters leave nothing', async () => {
    const { wrapper } = await mountRows([offlineAgent(1)], [{ agent_id: 1, count: 1 }])

    await pill(wrapper, 'Online').trigger('click')

    expect(wrapper.find('.list-group').exists()).toBe(false)
    expect(wrapper.text()).toContain('No agents match the current filter.')
  })
})
