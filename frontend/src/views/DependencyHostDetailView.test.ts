// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  mockApiClientRw,
  mockErrorUtilsPassthrough,
  mockWebSocket,
  resetWsHandlers,
  wsHandlers,
} from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())
vi.mock('../composables/useWebSocket', () => mockWebSocket())

const toast = vi.hoisted(() => ({ success: vi.fn(), warning: vi.fn(), error: vi.fn() }))
vi.mock('../composables/useToast', () => ({
  useToast: (): typeof toast => toast,
}))

import { renderWithPlugins } from '../test-utils'
import { dialogButton } from '../test-utils/dom'
import {
  dependencyAvailability,
  dependencyHost,
  dependencyPower,
} from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import { router } from '../router'
import DependencyHostDetailView from './DependencyHostDetailView.vue'
import type { DependencyHostResponse, DependencyUsageResponse } from '../types/generated'

const USAGE: DependencyUsageResponse[] = [
  {
    schedule_id: 4,
    schedule_name: 'Media share nightly',
    agent_id: 7,
    hostname: 'media-store-01',
    source: 'agent_default',
  },
  {
    schedule_id: 4,
    schedule_name: 'Media share nightly',
    agent_id: 8,
    hostname: 'web-server-01',
    source: 'schedule',
  },
]

let current: DependencyHostResponse
let usage: DependencyUsageResponse[]

function setupApi(
  host: DependencyHostResponse = dependencyHost({
    intermittent: true,
    last_check_reachable: false,
  }),
  used: DependencyUsageResponse[] = USAGE,
): void {
  current = host
  usage = used
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/dependency-hosts/3') return Promise.resolve({ data: current }) as never
    if (url === '/dependency-hosts/3/usage') return Promise.resolve({ data: usage }) as never
    if (url === '/dependency-hosts/3/availability') {
      return Promise.resolve({ data: dependencyAvailability({ waiting: [] }) }) as never
    }
    return Promise.resolve({ data: [] }) as never
  })
  vi.mocked(apiClient.delete).mockResolvedValue({} as never)
}

type Wrapper = ReturnType<typeof renderWithPlugins>

/** Mounts the page as `role`, then clicks the rail item labelled `section`, if any. */
async function render(role = 'admin', section?: string): Promise<Wrapper> {
  const page = renderWithPlugins(DependencyHostDetailView, {
    props: { id: '3' },
    storeState: { auth: { user: { role } } },
  })
  await flushPromises()
  if (section === undefined) return page
  const rail = page.findAll('.settings-nav-item')
  const index = railItems(page).findIndex((label) => label.toLowerCase() === section)
  expect(index, `rail item ${section}`).toBeGreaterThanOrEqual(0)
  await rail[index]!.trigger('click')
  await flushPromises()
  return page
}

function railItems(wrapper: Wrapper): string[] {
  return wrapper.findAll('.settings-nav-item').map((b) => b.text())
}

function button(wrapper: Wrapper, label: string) {
  return wrapper.findAll('button').find((b) => b.text().trim() === label)
}

describe('DependencyHostDetailView', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.clearAllMocks()
    resetWsHandlers()
    setupApi()
  })

  it('is routed for every signed-in user, not admins only', () => {
    const route = router.getRoutes().find((r) => r.name === 'dependency-host-detail')
    expect(route?.path).toBe('/dependency-hosts/:id')
    expect(route?.meta.requiresAdmin).toBeUndefined()
  })

  it('names the dependency, how it is checked and what its last check found', async () => {
    const wrapper = await render()
    expect(wrapper.get('h1').text()).toBe('nas-media')
    expect(wrapper.get('.detail-subtitle').text()).toBe('Dependency · SMB nas-media.lan:445')
    const badges = wrapper.findAll('.detail-header .badge').map((b) => b.text())
    expect(badges).toEqual(['Not answering', 'Not always online'])
    const crumbs = wrapper.findAll('.detail-breadcrumb a').map((a) => a.attributes('href'))
    expect(crumbs).toEqual(['/agents', '/agents?tab=dependencies'])
  })

  it('opens on the connection section, with the danger zone for admins only', async () => {
    const admin = await render()
    expect(admin.findComponent({ name: 'DependencyConnectionCard' }).exists()).toBe(true)
    expect(railItems(admin)).toEqual(['Connection', 'Power', 'Used by', 'Danger zone'])

    const viewer = await render('viewer')
    expect(railItems(viewer)).toEqual(['Connection', 'Power', 'Used by'])
  })

  it('lets admins edit and test, and shows everyone else the same page read-only', async () => {
    const admin = await render('admin', 'power')
    expect(admin.findAll('button').filter((b) => b.text() === 'Edit')).toHaveLength(2)
    expect(button(admin, 'Test connection')).toBeDefined()

    const viewer = await render('viewer', 'power')
    expect(viewer.findComponent({ name: 'DependencyPowerCard' }).exists()).toBe(true)
    expect(viewer.findComponent({ name: 'DependencyAvailabilityCard' }).exists()).toBe(true)
    expect(viewer.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
    expect(button(viewer, 'Test connection')).toBeUndefined()
  })

  it('holds the power and availability settings together', async () => {
    await render('admin', 'power')
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts/3/availability')
  })

  it('lists every backup that needs it, each row linking to its schedule', async () => {
    const wrapper = await render('admin', 'used by')
    const rows = wrapper.findAll('a.agent-row')
    expect(rows.map((r) => r.attributes('href'))).toEqual(['/schedules/4', '/schedules/4'])
    expect(rows[0]!.get('.meta-pill').text()).toBe('media-store-01')
    expect(rows[0]!.get('.agent-row-stats').text()).toBe('from agent defaults')
    expect(rows[1]!.get('.agent-row-stats').text()).toBe('set on the schedule')
  })

  it('says where to add it when nothing needs it yet', async () => {
    setupApi(dependencyHost(), [])
    const wrapper = await render('admin', 'used by')
    expect(wrapper.text()).toContain('No schedule needs this dependency yet.')
  })

  it.each([
    [true, 'success', 'nas-media.lan answered on port 445'],
    [false, 'warning', 'nas-media.lan did not answer on port 445 within 5 seconds'],
  ] as const)('reports a connection test that answered: %s', async (reachable, kind, text) => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { reachable, address: 'nas-media.lan', port: 445 },
    } as never)
    const wrapper = await render()
    await button(wrapper, 'Test connection')!.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/3/test')
    expect(toast[kind]).toHaveBeenCalledWith(text)
  })

  it('reports a connection test that could not run', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('forbidden'))
    const wrapper = await render()
    await button(wrapper, 'Test connection')!.trigger('click')
    await flushPromises()
    expect(toast.error).toHaveBeenCalledWith('forbidden')
  })

  it('removes the dependency after confirmation and returns to the list', async () => {
    const wrapper = await render('admin', 'danger zone')
    expect(wrapper.text()).toContain('runs waiting on it stop waiting')
    await button(wrapper, 'Remove dependency')!.trigger('click')
    await flushPromises()
    dialogButton('Remove').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/dependency-hosts/3')
    expect(wrapper.vm.$router.currentRoute.value.fullPath).toBe('/agents?tab=dependencies')
  })

  it('keeps the dialog open with the reason when removing fails', async () => {
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('still in use'))
    const wrapper = await render('admin', 'danger zone')
    await button(wrapper, 'Remove dependency')!.trigger('click')
    await flushPromises()
    dialogButton('Remove').click()
    await flushPromises()
    expect(document.body.textContent).toContain('still in use')
  })

  it('shows a failed load as an error', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('not found'))
    const wrapper = await render()
    expect(wrapper.get('.error-banner').text()).toContain('not found')
  })

  it('refreshes the header when something changes', async () => {
    const wrapper = await render()
    current = dependencyHost({
      last_check_reachable: true,
      power: dependencyPower({ repo_host: { id: 9, ssh_host: 'nas-01' } }),
    })
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.get('.detail-header .badge').text()).toBe('Reachable')
  })
})
