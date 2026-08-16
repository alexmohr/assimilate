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
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn> } => ({
    onMessage: vi.fn(),
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
import { renderWithPlugins } from '../test-utils'
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
  })

  it('shows row action buttons for each tunnel', async () => {
    setupSuccessMocks()

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.findAll('tbody tr')).toHaveLength(3)
    expect(wrapper.findAll('tbody .row-actions button')).toHaveLength(mockTunnels.length * 2 + 1)
    expect(wrapper.findAll('button').some((button) => button.text() === 'Edit')).toBe(true)
    expect(wrapper.findAll('button').some((button) => button.text() === 'New')).toBe(true)
    expect(wrapper.findAll('button').some((button) => button.text() === 'Create')).toBe(false)
  })

  it('renders empty state when no tunnels exist', async () => {
    mockListTunnels.mockResolvedValue([])
    mockApiClient.get.mockResolvedValue({ data: mockAgents })

    const wrapper = renderWithPlugins(TunnelsView)
    await flushPromises()

    expect(wrapper.text()).toContain('No SSH tunnels configured')
    expect(wrapper.text()).toContain('Create a tunnel to access remote hosts.')
    expect(wrapper.text()).toContain('Add Tunnel')
  })

  describe('tunnel dialogs', () => {
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

    function fieldByLabel(wrapper: View, label: string) {
      const wrap = wrapper
        .findAll('.field')
        .find(
          (f) => f.find('.field-label').exists() && f.find('.field-label').text().includes(label),
        )
      const control = wrap?.find('input, select')
      if (!control || !control.exists()) throw new Error(`no field labelled "${label}"`)
      return control
    }

    async function setByLabel(wrapper: View, label: string, value: string): Promise<void> {
      await fieldByLabel(wrapper, label).setValue(value)
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

      await setByLabel(wrapper, 'SSH Host', '10.0.0.99')
      expect(create().attributes('disabled')).toBeDefined()

      await setByLabel(wrapper, 'Agent', '4')
      expect(create().attributes('disabled')).toBeDefined()

      await setByLabel(wrapper, 'Tunnel Port', '2225')
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
      await setByLabel(wrapper, 'SSH Host', '10.0.0.99')
      await setByLabel(wrapper, 'SSH Port', '2022')
      await setByLabel(wrapper, 'Tunnel Port', '2225')

      await clickButton(wrapper, 'Create')

      expect(vi.mocked(createTunnel)).toHaveBeenCalledWith(
        expect.objectContaining({ agent_id: 4, ssh_host: '10.0.0.99', ssh_port: 2022 }),
      )
      // A freshly created tunnel has not dialled out yet, so it must not be
      // shown as connected.
      expect(wrapper.findAll('tbody tr')).toHaveLength(4)
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
      await setByLabel(wrapper, 'SSH Host', '10.0.0.99')
      await setByLabel(wrapper, 'Tunnel Port', '2225')
      await setByLabel(wrapper, 'SSH User', 'operator')
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
      expect(wrapper.text()).toContain('Add Tunnel')

      await clickButton(wrapper, 'Cancel')

      expect(vi.mocked(createTunnel)).not.toHaveBeenCalled()
      expect(wrapper.find('.field-checkbox').exists()).toBe(false)
    })

    it('sends every edited field', async () => {
      const { updateTunnel } = await import('../api/tunnels')
      vi.mocked(updateTunnel).mockResolvedValue({ ...mockTunnels[0] } as never)

      const wrapper = await render()
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Edit')!
        .trigger('click')
      await flushPromises()

      await setByLabel(wrapper, 'SSH Host', '10.0.0.88')
      await setByLabel(wrapper, 'SSH User', 'operator')
      await setByLabel(wrapper, 'SSH Port', '2200')
      await setByLabel(wrapper, 'Tunnel Port', '2299')
      await wrapper.find('.field-checkbox input[type="checkbox"]').setValue(false)

      await clickButton(wrapper, 'Save')

      expect(vi.mocked(updateTunnel)).toHaveBeenCalledWith(101, {
        ssh_host: '10.0.0.88',
        ssh_user: 'operator',
        ssh_port: 2200,
        tunnel_port: 2299,
        enabled: false,
      })
    })

    it('closes the edit dialog on Cancel without saving', async () => {
      const { updateTunnel } = await import('../api/tunnels')
      const wrapper = await render()
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Edit')!
        .trigger('click')
      await flushPromises()

      await clickButton(wrapper, 'Cancel')

      expect(vi.mocked(updateTunnel)).not.toHaveBeenCalled()
    })

    it('reports a create failure and leaves the dialog open', async () => {
      const { createTunnel } = await import('../api/tunnels')
      vi.mocked(createTunnel).mockRejectedValue(new Error('port in use'))

      const wrapper = await render([...mockAgents, SPARE_AGENT])
      await openAdd(wrapper)
      await setByLabel(wrapper, 'Agent', '4')
      await setByLabel(wrapper, 'SSH Host', '10.0.0.99')
      await setByLabel(wrapper, 'Tunnel Port', '2225')

      await clickButton(wrapper, 'Create')

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.findAll('tbody tr')).toHaveLength(3)
    })

    it('prefills the edit dialog from the row it was opened on', async () => {
      const wrapper = await render()
      await wrapper
        .findAll('button')
        .filter((b) => b.text() === 'Edit')[1]
        .trigger('click')
      await flushPromises()

      expect((fieldByLabel(wrapper, 'SSH Host').element as HTMLInputElement).value).toBe(
        '10.0.0.12',
      )
      expect((fieldByLabel(wrapper, 'SSH User').element as HTMLInputElement).value).toBe('borg')
      expect((fieldByLabel(wrapper, 'SSH Port').element as HTMLInputElement).value).toBe('2222')
    })

    it('saves the edited tunnel back into the table', async () => {
      const { updateTunnel } = await import('../api/tunnels')
      vi.mocked(updateTunnel).mockResolvedValue({
        ...mockTunnels[0],
        ssh_host: '10.0.0.77',
      } as never)

      const wrapper = await render()
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Edit')!
        .trigger('click')
      await flushPromises()
      await setByLabel(wrapper, 'SSH Host', '10.0.0.77')

      await clickButton(wrapper, 'Save')

      expect(vi.mocked(updateTunnel)).toHaveBeenCalledWith(
        101,
        expect.objectContaining({ ssh_host: '10.0.0.77' }),
      )
      expect(wrapper.text()).toContain('10.0.0.77')
    })

    it('reports a save failure without closing the dialog', async () => {
      const { updateTunnel } = await import('../api/tunnels')
      vi.mocked(updateTunnel).mockRejectedValue(new Error('conflict'))

      const wrapper = await render()
      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Edit')!
        .trigger('click')
      await flushPromises()

      await clickButton(wrapper, 'Save')

      expect(wrapper.find('.form-error').exists()).toBe(true)
    })

    it('names the host it is about to delete', async () => {
      const wrapper = await render()
      await wrapper.findAll('tbody button.btn-danger-text')[0].trigger('click')
      await flushPromises()
      expect(wrapper.text()).toContain('web-server-01')
    })

    it('deletes the tunnel on confirmation and drops the row', async () => {
      const { deleteTunnel } = await import('../api/tunnels')
      vi.mocked(deleteTunnel).mockResolvedValue(undefined as never)

      const wrapper = await render()
      await wrapper.findAll('tbody button.btn-danger-text')[0].trigger('click')
      await flushPromises()
      await clickButton(wrapper, 'Delete')

      expect(vi.mocked(deleteTunnel)).toHaveBeenCalledWith(101)
      expect(wrapper.findAll('tbody tr')).toHaveLength(2)
    })

    it('keeps the tunnel when the delete is cancelled', async () => {
      const { deleteTunnel } = await import('../api/tunnels')
      const wrapper = await render()
      await wrapper.findAll('tbody button.btn-danger-text')[0].trigger('click')
      await flushPromises()
      await clickButton(wrapper, 'Cancel')

      expect(vi.mocked(deleteTunnel)).not.toHaveBeenCalled()
      expect(wrapper.findAll('tbody tr')).toHaveLength(3)
    })

    it('reports a delete failure and keeps the row', async () => {
      const { deleteTunnel } = await import('../api/tunnels')
      vi.mocked(deleteTunnel).mockRejectedValue(new Error('in use'))

      const wrapper = await render()
      await wrapper.findAll('tbody button.btn-danger-text')[0].trigger('click')
      await flushPromises()
      await clickButton(wrapper, 'Delete')

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.findAll('tbody tr')).toHaveLength(3)
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

    expect(wrapper.find('.badge--danger').exists()).toBe(true)
    expect(wrapper.text()).toContain('Error')
  })
})
