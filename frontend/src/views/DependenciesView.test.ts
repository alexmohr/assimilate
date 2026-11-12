// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, type ComponentPublicInstance } from 'vue'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'
// Imported before '../api/client' on purpose: the hoisted vi.mock factory
// below calls this, so it has to be initialised by the time that module is
// first imported.
import { mockApiClientRw } from '../test-utils/sharedMocks'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import type { CurrentUserResponse } from '../stores/auth'
import { routes } from '../router'
import DependenciesView from './DependenciesView.vue'

vi.mock('../api/client', () => mockApiClientRw())

const ws = vi.hoisted(() => {
  const handlers: Record<string, () => void> = {}
  return {
    handlers,
    onMessage: vi.fn((type: string, cb: () => void) => {
      handlers[type] = cb
    }),
  }
})

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn> } => ({ onMessage: ws.onMessage }),
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

const nasMedia = {
  id: 3,
  name: 'nas-media',
  address: 'nas-media.lan',
  port: 445,
  description: '',
  power: {
    repo_host: null,
    wake_enabled: false,
    wake_mac_address: null,
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    effective_wake_enabled: false,
    effective_wake_mac_address: null,
    effective_wake_broadcast_address: null,
    effective_wake_timeout_seconds: 180,
  },
  intermittent: false,
  catch_up_recheck_minutes: 15,
  catch_up_give_up_minutes: 0,
  last_checked_at: null,
  last_check_reachable: null,
  schedule_count: 1,
  agent_default_count: 0,
  waiting_count: 0,
}

function makeRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/:pathMatch(.*)*', component: defineComponent({ render: (): null => null }) },
    ],
  })
}

async function mountPage(
  role: string,
  dependencies: unknown[] = [nasMedia],
): Promise<{
  wrapper: VueWrapper<ComponentPublicInstance>
  router: ReturnType<typeof makeRouter>
}> {
  vi.clearAllMocks()
  vi.mocked(apiClient.get).mockImplementation((url: string) =>
    Promise.resolve({ data: url === '/dependency-hosts' ? dependencies : [] }),
  )
  const router = makeRouter()
  await router.push('/dependencies')
  await router.isReady()
  const pinia = createPinia()
  useAuthStore(pinia).user = { id: 1, username: 'u', role } as CurrentUserResponse
  const wrapper = mount(DependenciesView, { global: { plugins: [pinia, router] } })
  await flushPromises()
  return { wrapper, router }
}

function newButton(wrapper: VueWrapper<ComponentPublicInstance>): VueWrapper | undefined {
  return wrapper.findAll('button').find((b) => b.text().trim() === 'New')
}

describe('DependenciesView', () => {
  it('lists every dependency, each row linking to its page', async () => {
    const { wrapper } = await mountPage('admin')
    expect(wrapper.get('h1').text()).toBe('Dependencies')
    expect(wrapper.get('.page-description').text()).toBe(
      'Hosts a backup needs before it can run. Runs wait for them, and Assimilate can wake them.',
    )
    expect(wrapper.get('[data-testid="dependency-row"] a.list-row-name').attributes('href')).toBe(
      '/dependency-hosts/3',
    )
    expect(wrapper.text()).toContain('nas-media')
  })

  it('offers no Check now or Wake on the list: those live on the dependency page', async () => {
    const { wrapper } = await mountPage('admin')
    const labels = wrapper.findAll('button').map((b) => b.text().trim())
    expect(labels).not.toContain('Check now')
    expect(labels).not.toContain('Wake')
  })

  it('makes New open the new-dependency wizard page', async () => {
    const { wrapper, router } = await mountPage('admin')
    await newButton(wrapper)!.trigger('click')
    await flushPromises()

    expect(router.currentRoute.value.path).toBe('/dependencies/new')
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('offers no New to a non-admin', async () => {
    const { wrapper } = await mountPage('viewer')
    expect(newButton(wrapper)).toBeUndefined()
    expect(wrapper.find('.empty-action').exists()).toBe(false)
  })

  it("opens the new-dependency wizard page from the empty list's New", async () => {
    const { wrapper, router } = await mountPage('admin', [])
    const action = wrapper.get('.empty-action')
    expect(action.text()).toBe('New')
    expect(router.currentRoute.value.path).toBe('/dependencies')

    await action.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/dependencies/new')
  })

  it('reloads the list when the server says data changed', async () => {
    const { wrapper } = await mountPage('admin')
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [nasMedia, { ...nasMedia, id: 4, name: 'files-01' }],
    })
    ws.handlers['DataChanged']!()
    await flushPromises()
    expect(wrapper.findAll('[data-testid="dependency-row"]')).toHaveLength(2)
  })
})

describe('dependencies route', () => {
  function appRouter(): ReturnType<typeof createRouter> {
    return createRouter({ history: createMemoryHistory(), routes })
  }

  it('is routed for every signed-in user, not admins only', () => {
    const route = routes.find((r) => r.path === '/dependencies')
    expect(route?.name).toBe('dependencies')
    expect(route?.meta?.requiresAdmin).toBeUndefined()
  })

  it('routes the new-dependency wizard for admins only', () => {
    const route = routes.find((r) => r.path === '/dependencies/new')
    expect(route?.name).toBe('dependency-create')
    expect(route?.meta?.requiresAdmin).toBe(true)
  })

  it("sends an old link to the Agents page's dependencies tab to the Dependencies page", async () => {
    const router = appRouter()
    await router.push('/agents?tab=dependencies')
    expect(router.currentRoute.value.path).toBe('/dependencies')
  })

  it('leaves the Agents page alone otherwise', async () => {
    const router = appRouter()
    await router.push('/agents?status=online')
    expect(router.currentRoute.value.fullPath).toBe('/agents?status=online')
  })
})
