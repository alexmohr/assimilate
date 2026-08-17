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
import type { AuthUser } from '../stores/auth'
import HostsView from './HostsView.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import { dismissModal } from '../test-utils'

vi.mock('../api/client', () => mockApiClientRw())

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
    created_at: '2026-01-01T00:00:00Z',
    last_login_at: null,
    can_upgrade_agent: true,
    ...authUserOverrides,
  } as AuthUser
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

    const cards = wrapper.findAll('.host-card')
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
  ): Promise<{
    wrapper: ReturnType<typeof mount>
    router: ReturnType<typeof createRouter>
  }> {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: agentsData })
      if (url === '/stats/health') return Promise.resolve({ data: healthData })
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
  ): Promise<ReturnType<typeof mount>> {
    const { wrapper } = await mountAgentsList([{ ...issueAgent, ...overrides }])
    return wrapper
  }

  it('shows an Offline pill and tints the card when the agent is disconnected', async () => {
    const wrapper = await mountSingleAgent({ is_connected: false })

    expect(wrapper.find('.host-card').classes()).toContain('host-card-notable')
    expect(wrapper.find('.entity-status-pill').text()).toBe('Offline')
  })

  it('shows nothing in the badge row for a healthy online agent with no issues', async () => {
    const wrapper = await mountSingleAgent({ is_connected: true })

    expect(wrapper.find('.host-card').classes()).not.toContain('host-card-notable')
    expect(wrapper.find('.entity-badge-row').exists()).toBe(false)
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

      expect(apiClient.put).toHaveBeenCalledWith('/agents/test-agent', { display_name: 'Test' })
      expect(apiClient.post).toHaveBeenCalledWith('/agents/test-agent/regenerate-token')
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

      dialog.vm.$emit('close')
      await flushPromises()

      expect(wrapper.findComponent(AgentDeployDialog).exists()).toBe(false)
    })
  })
})
