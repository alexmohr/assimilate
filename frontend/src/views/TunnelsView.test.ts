// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../api/tunnels', () => ({
  listTunnels: vi.fn(),
  createTunnel: vi.fn(),
  updateTunnel: vi.fn(),
  deleteTunnel: vi.fn(),
  enableTunnel: vi.fn(),
  disableTunnel: vi.fn(),
  reconnectTunnel: vi.fn(),
}))

type WsHandler = (data: unknown) => void
const wsHandlers = vi.hoisted(() => new Map<string, WsHandler>())

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: (type: string, handler: WsHandler) => void } => ({
    onMessage: (type: string, handler: WsHandler): void => {
      wsHandlers.set(type, handler)
    },
  }),
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: (): void => undefined,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn() },
}))

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../components/EmptyState.vue', () => ({
  default: {
    props: ['title', 'description', 'action'],
    emits: ['action'],
    template: `
      <div class="empty-state">
        <h2>{{ title }}</h2>
        <p>{{ description }}</p>
        <button @click="$emit('action')">{{ action }}</button>
      </div>
    `,
  },
}))

import { apiClient } from '../api/client'
import { listTunnels } from '../api/tunnels'
import {
  dismissModal,
  fieldByLabel,
  openModals,
  renderWithPlugins,
  setFieldByLabel,
} from '../test-utils'
import TunnelsView from './TunnelsView.vue'

const mockApiClient = apiClient as {
  get: ReturnType<typeof vi.fn>
}

const mockListTunnels = listTunnels as ReturnType<typeof vi.fn>

const mockAgents = [
  { id: 1, hostname: 'web-server-01' },
  { id: 2, hostname: 'db-server-01' },
  { id: 3, hostname: 'media-store-01' },
]

const mockTunnels = [
  {
    id: 101,
    agent_id: 1,
    agent_hostname: 'web-server-01',
    ssh_host: '10.0.0.11',
    ssh_user: 'root',
    ssh_port: 22,
    tunnel_port: 2222,
    enabled: true,
    created_at: '2026-05-31T00:00:00Z',
    status: 'connected' as const,
  },
  {
    id: 102,
    agent_id: 2,
    agent_hostname: 'db-server-01',
    ssh_host: '10.0.0.12',
    ssh_user: 'borg',
    ssh_port: 2222,
    tunnel_port: 2223,
    enabled: false,
    created_at: '2026-05-31T00:00:00Z',
    status: 'disconnected' as const,
  },
  {
    id: 103,
    agent_id: 3,
    agent_hostname: 'media-store-01',
    ssh_host: '10.0.0.13',
    ssh_user: 'root',
    ssh_port: 22,
    tunnel_port: 2224,
    enabled: true,
    created_at: '2026-05-31T00:00:00Z',
    status: 'reconnecting' as const,
  },
]

function setupSuccessMocks(): void {
  mockListTunnels.mockResolvedValue(mockTunnels)
  mockApiClient.get.mockResolvedValue({ data: mockAgents })
}

type Wrapper = ReturnType<typeof renderWithPlugins>

/** The row of the tunnel whose agent is `hostname`. */
function rowOf(wrapper: Wrapper, hostname: string): ReturnType<Wrapper['get']> {
  const row = wrapper
    .findAll('.tunnel-row')
    .find((r) => r.find('.list-row-name').text() === hostname)
  if (!row) throw new Error(`no row for ${hostname}`)
  return row
}

function rowNames(wrapper: Wrapper): string[] {
  return wrapper.findAll('.tunnel-row .list-row-name').map((n) => n.text())
}

describe('TunnelsView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders tunnel list with mock data', async () => {
    setupSuccessMocks()

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('db-server-01')
    expect(wrapper.text()).toContain('media-store-01')
    expect(wrapper.text()).toContain('10.0.0.13')
    expect(wrapper.text()).toContain('Connected')
    expect(wrapper.text()).toContain('Disconnected')
    expect(wrapper.text()).toContain('Reconnecting')

    // A switched-off tunnel says so, and its dot is grey rather than red.
    const enabledRow = rowOf(wrapper, 'web-server-01')
    const disabledRow = rowOf(wrapper, 'db-server-01')
    expect(enabledRow.text()).not.toContain('Disabled')
    expect(enabledRow.get('.status-dot').classes()).toContain('status-dot--success')
    expect(disabledRow.text()).toContain('Disabled')
    expect(disabledRow.get('.status-dot').classes()).not.toContain('status-dot--danger')
    expect(wrapper.get('.page-description').text()).toBe(
      'Reverse SSH tunnels that let an agent reach this server when it cannot connect to it directly.',
    )
  })

  it('shows the route from the server through the tunnel port to the SSH target', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    const route = rowOf(wrapper, 'db-server-01').get('.tunnel-route')
    expect(route.get('[data-testid="tunnel-port"]').text()).toBe(':2223')
    expect(route.get('.tunnel-target').text()).toBe('borg@10.0.0.12:2222')
    expect(route.text()).toContain('server')
  })

  it('names the agent by hostname over its display name', async () => {
    mockListTunnels.mockResolvedValue([{ ...mockTunnels[0], agent_hostname: undefined }])
    mockApiClient.get.mockResolvedValue({
      data: [{ id: 1, hostname: 'web-server-01', display_name: 'Front web server' }],
    })
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    const row = rowOf(wrapper, 'web-server-01')
    expect(row.get('.list-row-main .list-row-sub').text()).toBe('Front web server')
  })

  it('lists the tunnels that need attention first', async () => {
    mockListTunnels.mockResolvedValue([
      ...mockTunnels,
      {
        ...mockTunnels[0],
        id: 104,
        agent_id: 4,
        agent_hostname: 'nas-01',
        status: { error: { message: 'refused' } },
      },
    ])
    mockApiClient.get.mockResolvedValue({ data: mockAgents })
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(rowNames(wrapper)).toEqual(['nas-01', 'media-store-01', 'web-server-01', 'db-server-01'])
  })

  it('searches by agent and by SSH host', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()
    const input = wrapper.get('input[type="search"]')

    await input.setValue('MEDIA')
    expect(rowNames(wrapper)).toEqual(['media-store-01'])

    await input.setValue('10.0.0.12')
    expect(rowNames(wrapper)).toEqual(['db-server-01'])

    await input.setValue('nowhere')
    expect(rowNames(wrapper)).toEqual([])
    expect(wrapper.text()).toContain('No tunnel matches.')
  })

  it('counts tunnels into the filter pills by status, and filters by them', async () => {
    // web-server-01 is connected, db-server-01 is disconnected (counts as
    // needing attention), media-store-01 is reconnecting - so each pill
    // should land on a different tunnel, catching a filter that conflates
    // "needs attention" with any other status.
    setupSuccessMocks()

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    const pills = wrapper.findAll('.filter-pill')
    expect(pills.map((p) => p.text().replace(/\s+/g, ' '))).toEqual([
      'All · 3',
      'Connected · 1',
      'Reconnecting · 1',
      'Needs attention · 1',
    ])

    await pills[1].trigger('click')
    expect(rowNames(wrapper)).toEqual(['web-server-01'])
    await pills[2].trigger('click')
    expect(rowNames(wrapper)).toEqual(['media-store-01'])
    await pills[3].trigger('click')
    expect(rowNames(wrapper)).toEqual(['db-server-01'])
    expect(pills[3].attributes('aria-pressed')).toBe('true')
    await pills[0].trigger('click')
    expect(rowNames(wrapper)).toHaveLength(3)
  })

  // GET /tunnels never returns agent_hostname (only agent_id) - the view has
  // to resolve it against the agents list itself rather than trusting a
  // field the real API doesn't send.
  it('resolves the agent hostname from the agents list when the tunnel has none', async () => {
    mockListTunnels.mockResolvedValue([{ ...mockTunnels[0], agent_hostname: undefined }])
    mockApiClient.get.mockResolvedValue({ data: mockAgents })

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.find('.list-row-name').text()).toBe('web-server-01')
  })

  // Rows used to end in a "..." menu with Edit, Reconnect and Delete. Those
  // live on the tunnel's own page now, so a row is a link to it and nothing
  // else.
  it('links every row to its tunnel page, with no row menu', async () => {
    setupSuccessMocks()

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(rowOf(wrapper, 'web-server-01').attributes('href')).toBe('/tunnels/101')
    expect(rowOf(wrapper, 'media-store-01').attributes('href')).toBe('/tunnels/103')
    for (const row of wrapper.findAll('.tunnel-row')) {
      expect(row.find('.list-row-chevron').exists()).toBe(true)
      expect(row.find('.overflow-toggle').exists()).toBe(false)
      expect(row.findAll('button')).toHaveLength(0)
    }
  })

  it('follows live status changes on the matching row', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(rowOf(wrapper, 'media-store-01').get('.status-dot').classes()).not.toContain(
      'status-dot--success',
    )

    wsHandlers.get('TunnelStatusChanged')!({ agent_id: 3, status: 'connected' })
    await flushPromises()

    // A connected tunnel is said by its green dot alone; no state is named.
    const row = rowOf(wrapper, 'media-store-01')
    expect(row.get('.status-dot').classes()).toContain('status-dot--success')
    expect(row.get('.tunnel-status').text()).toBe('')
  })

  it('still lists the tunnels when the agents cannot be loaded', async () => {
    const { logger } = await import('../utils/logger')
    mockListTunnels.mockResolvedValue(mockTunnels)
    mockApiClient.get.mockRejectedValue(new Error('offline'))
    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(rowNames(wrapper)).toContain('web-server-01')
    expect(logger.error).toHaveBeenCalledWith('loadAgents failed', expect.any(Error))
  })

  it('renders empty state when no tunnels exist', async () => {
    mockListTunnels.mockResolvedValue([])
    mockApiClient.get.mockResolvedValue({ data: mockAgents })

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.text()).toContain('No SSH tunnels configured')
    expect(wrapper.text()).toContain('Create a tunnel to access remote hosts.')
    expect(wrapper.text()).toContain('New tunnel')
  })

  describe('add dialog', () => {
    type View = Awaited<ReturnType<typeof render>>

    /**
     * Driven through the wrapper rather than document.body: these dialogs are
     * rendered inline, and a native .click() would bypass Vue's handlers.
     */
    async function clickButton(wrapper: View, label: string): Promise<void> {
      const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
      if (!match) throw new Error(`no dialog button labelled "${label}"`)
      await match.trigger('click')
      await flushPromises()
    }

    async function setByLabel(wrapper: View, label: string, value: string): Promise<void> {
      await setFieldByLabel(wrapper, label, value)
      await flushPromises()
    }

    /**
     * All three fixture agents already own a tunnel, and the Agent picker
     * deliberately offers only agents that do not - one tunnel per agent. So
     * the add-dialog cases need a spare agent to have anything to select.
     */
    const SPARE_AGENT = { id: 4, hostname: 'spare-host' }

    async function render(agents = mockAgents) {
      // A fresh copy per test: the view pushes a newly created tunnel onto
      // the array it got back, so handing out the shared fixture would let
      // one test's creation leak into the next test's row count.
      mockListTunnels.mockResolvedValue(mockTunnels.map((t) => ({ ...t })))
      mockApiClient.get.mockResolvedValue({ data: agents })
      const wrapper = renderWithPlugins(TunnelsView)
      await flushPromises()
      return wrapper
    }

    async function openAdd(wrapper: Awaited<ReturnType<typeof render>>) {
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'New')!
        .trigger('click')
      await flushPromises()
    }

    it('offers only agents that do not already have a tunnel', async () => {
      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)

      const options = [
        ...(fieldByLabel(wrapper, 'Agent').element as HTMLSelectElement).options,
      ].map((o) => o.text.trim())
      expect(options).toContain('spare-host')
      expect(options).not.toContain('web-server-01')
    })

    it('offers nothing when every agent already has a tunnel', async () => {
      const wrapper = await render()
      await openAdd(wrapper)

      const selectable = [
        ...(fieldByLabel(wrapper, 'Agent').element as HTMLSelectElement).options,
      ].filter((o) => !o.disabled)
      expect(selectable).toHaveLength(0)
    })

    /**
     * The required fields are enforced by disabling Create rather than by
     * reporting an error after the fact, so the guards inside submitAdd are
     * unreachable from the UI. This asserts the gate that actually holds.
     */
    it('keeps Create disabled until agent, host and tunnel port are all set', async () => {
      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)

      const create = () => wrapper.findAll('button').find((b) => b.text() === 'Create')!
      expect(create().attributes('disabled')).toBeDefined()

      await setByLabel(wrapper, 'SSH host', '10.0.0.99')
      expect(create().attributes('disabled')).toBeDefined()

      await setByLabel(wrapper, 'Agent', '4')
      expect(create().attributes('disabled')).toBeDefined()

      await setByLabel(wrapper, 'Tunnel port', '2225')
      expect(create().attributes('disabled')).toBeUndefined()
    })

    it('creates the tunnel and adds it to the table', async () => {
      const { createTunnel } = await import('../api/tunnels')
      vi.mocked(createTunnel).mockResolvedValue({
        id: 104,
        agent_id: 4,
        ssh_host: '10.0.0.99',
        ssh_user: 'root',
        ssh_port: 2022,
        tunnel_port: 2225,
        enabled: true,
        created_at: '2026-05-31T00:00:00Z',
      } as never)

      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      await setByLabel(wrapper, 'Agent', '4')
      await setByLabel(wrapper, 'SSH host', '10.0.0.99')
      await setByLabel(wrapper, 'SSH port', '2022')
      await setByLabel(wrapper, 'Tunnel port', '2225')

      await clickButton(wrapper, 'Create')

      expect(vi.mocked(createTunnel)).toHaveBeenCalledWith(
        expect.objectContaining({ agent_id: 4, ssh_host: '10.0.0.99', ssh_port: 2022 }),
      )
      // A freshly created tunnel has not dialled out yet, so it must not be
      // shown as connected.
      expect(wrapper.findAll('.tunnel-row')).toHaveLength(4)
      expect(wrapper.text()).toContain('10.0.0.99')
    })

    it('sends the SSH user and the enable-immediately choice', async () => {
      const { createTunnel } = await import('../api/tunnels')
      vi.mocked(createTunnel).mockResolvedValue({
        id: 104,
        agent_id: 4,
        ssh_host: '10.0.0.99',
        ssh_user: 'operator',
        ssh_port: 22,
        tunnel_port: 2225,
        enabled: false,
        created_at: '2026-05-31T00:00:00Z',
      } as never)

      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      await setByLabel(wrapper, 'Agent', '4')
      await setByLabel(wrapper, 'SSH host', '10.0.0.99')
      await setByLabel(wrapper, 'Tunnel port', '2225')
      await setByLabel(wrapper, 'SSH user', 'operator')
      await wrapper.find('.field-checkbox input[type="checkbox"]').setValue(false)

      await clickButton(wrapper, 'Create')

      expect(vi.mocked(createTunnel)).toHaveBeenCalledWith(
        expect.objectContaining({ ssh_user: 'operator', enabled: false }),
      )
    })

    it('closes the add dialog on Cancel without creating anything', async () => {
      const { createTunnel } = await import('../api/tunnels')
      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      expect(wrapper.text()).toContain('New tunnel')

      await clickButton(wrapper, 'Cancel')

      expect(vi.mocked(createTunnel)).not.toHaveBeenCalled()
      expect(wrapper.find('.field-checkbox').exists()).toBe(false)
    })

    it('reports a create failure and leaves the dialog open', async () => {
      const { createTunnel } = await import('../api/tunnels')
      vi.mocked(createTunnel).mockRejectedValue(new Error('port in use'))

      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      await setByLabel(wrapper, 'Agent', '4')
      await setByLabel(wrapper, 'SSH host', '10.0.0.99')
      await setByLabel(wrapper, 'Tunnel port', '2225')

      await clickButton(wrapper, 'Create')

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.findAll('.tunnel-row')).toHaveLength(3)
    })

    // Escape and a backdrop click close the dialog through BaseModal, wired
    // separately from its own Cancel button.
    it('closes the add dialog when it is dismissed', async () => {
      const { createTunnel } = await import('../api/tunnels')
      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      expect(openModals(wrapper)).toHaveLength(1)

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(vi.mocked(createTunnel)).not.toHaveBeenCalled()
    })
  })

  // An unknown status must not read as healthy: anything that is not one of
  // the three known states is an error and is styled as one.
  it('styles an unrecognized tunnel status as an error', async () => {
    mockListTunnels.mockResolvedValue([
      { ...mockTunnels[0], status: { error: { message: 'handshake failed' } } },
    ])
    mockApiClient.get.mockResolvedValue({ data: mockAgents })

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.get('.tunnel-status-text--danger').text()).toBe('Error')
    // The error itself is on the row, under the state.
    expect(wrapper.get('[data-testid="tunnel-error"]').text()).toBe('handshake failed')
    expect(wrapper.get('.tunnel-row .status-dot').classes()).toContain('status-dot--danger')
  })
})
