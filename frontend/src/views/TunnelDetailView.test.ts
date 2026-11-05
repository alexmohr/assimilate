// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

type Handler = (data: unknown) => void
const wsHandlers = new Map<string, Handler>()

vi.mock('../api/tunnels', () => ({
  listTunnels: vi.fn().mockResolvedValue([]),
  getTunnel: vi.fn(),
  updateTunnel: vi.fn(),
  deleteTunnel: vi.fn(),
  enableTunnel: vi.fn(),
  disableTunnel: vi.fn(),
  reconnectTunnel: vi.fn(),
}))

vi.mock('../api/agents', () => ({
  listAgents: vi.fn(),
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: (type: string, handler: Handler) => void } => ({
    onMessage: (type: string, handler: Handler): void => {
      wsHandlers.set(type, handler)
    },
  }),
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn() },
}))

import {
  deleteTunnel,
  disableTunnel,
  enableTunnel,
  getTunnel,
  reconnectTunnel,
  updateTunnel,
} from '../api/tunnels'
import { listAgents } from '../api/agents'
import { fieldByLabel, renderWithPlugins, setFieldByLabel } from '../test-utils'
import type { TunnelWithStatus } from '../types/tunnel'
import TunnelDetailView from './TunnelDetailView.vue'

const baseTunnel: TunnelWithStatus = {
  id: 101,
  agent_id: 1,
  ssh_host: '10.0.0.11',
  ssh_user: 'root',
  ssh_port: 22,
  tunnel_port: 2222,
  enabled: true,
  created_at: '2026-05-31T00:00:00Z',
  status: 'connected',
}

type Wrapper = ReturnType<typeof renderWithPlugins>

async function render(overrides: Partial<TunnelWithStatus> = {}): Promise<Wrapper> {
  vi.mocked(getTunnel).mockResolvedValue({ ...baseTunnel, ...overrides })
  vi.mocked(listAgents).mockResolvedValue([
    { id: 1, hostname: 'web-server-01', display_name: 'Front web server' },
  ] as never)
  const wrapper = renderWithPlugins(TunnelDetailView, { props: { id: '101' } })
  await flushPromises()
  return wrapper
}

function button(wrapper: Wrapper, label: string): ReturnType<Wrapper['get']> | undefined {
  return wrapper.findAll('button').find((b) => b.text().trim() === label)
}

async function click(wrapper: Wrapper, label: string): Promise<void> {
  const match = button(wrapper, label)
  if (!match) throw new Error(`no button labelled "${label}"`)
  await match.trigger('click')
  await flushPromises()
}

describe('TunnelDetailView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    wsHandlers.clear()
  })

  it('names the tunnel by its agent and shows its route', async () => {
    const wrapper = await render()

    expect(getTunnel).toHaveBeenCalledWith(101)
    expect(wrapper.get('.detail-name').text()).toBe('web-server-01')
    expect(wrapper.get('.detail-subtitle').text()).toBe('Front web server')
    expect(wrapper.get('.crumb-current').text()).toBe('web-server-01')
    expect(wrapper.get('[data-testid="tunnel-status"]').text()).toBe('Connected')
    expect(wrapper.text()).toContain('root@10.0.0.11:22')
    expect(wrapper.text()).toContain('ws://127.0.0.1:2222')
    expect(wrapper.find('a[href="/agents/web-server-01"]').exists()).toBe(true)
  })

  it('offers Reconnect only for an enabled tunnel that is not connected', async () => {
    expect(button(await render(), 'Reconnect')).toBeUndefined()
    expect(button(await render({ enabled: false, status: 'disconnected' }), 'Reconnect')).toBe(
      undefined,
    )
    expect(button(await render({ status: 'reconnecting' }), 'Reconnect')).toBeDefined()
  })

  it('reconnects the tunnel and shows its new status', async () => {
    vi.mocked(reconnectTunnel).mockResolvedValue({ ...baseTunnel, status: 'connected' })
    const wrapper = await render({ status: { error: { message: 'refused' } } })

    await click(wrapper, 'Reconnect')

    expect(reconnectTunnel).toHaveBeenCalledWith(101)
    expect(wrapper.get('[data-testid="tunnel-status"]').text()).toBe('Connected')
    expect(button(wrapper, 'Reconnect')).toBeUndefined()
  })

  it('reports a failed reconnect', async () => {
    vi.mocked(reconnectTunnel).mockRejectedValue(new Error('still refused'))
    const wrapper = await render({ status: 'reconnecting' })

    await click(wrapper, 'Reconnect')

    expect(wrapper.get('.error-banner').text()).toContain('still refused')
  })

  it('switches the tunnel off and on from the header', async () => {
    const wrapper = await render()
    vi.mocked(getTunnel).mockResolvedValue({
      ...baseTunnel,
      enabled: false,
      status: 'disconnected',
    })

    await wrapper.get('[role="switch"]').trigger('click')
    await flushPromises()

    expect(disableTunnel).toHaveBeenCalledWith(101)
    expect(wrapper.get('[role="switch"]').attributes('aria-checked')).toBe('false')
    expect(wrapper.text()).toContain('Disabled: the server does not open this tunnel.')

    vi.mocked(getTunnel).mockResolvedValue({ ...baseTunnel, status: 'reconnecting' })
    await wrapper.get('[role="switch"]').trigger('click')
    await flushPromises()

    expect(enableTunnel).toHaveBeenCalledWith(101)
    expect(wrapper.get('[role="switch"]').attributes('aria-checked')).toBe('true')
  })

  it('shows the error a failed tunnel reports, with what to check', async () => {
    const wrapper = await render({
      ssh_user: 'borg',
      status: { error: { message: 'Permission denied (publickey)' } },
    })

    expect(wrapper.get('[data-testid="tunnel-status"]').text()).toBe('Error')
    expect(wrapper.get('[data-testid="tunnel-error"]').text()).toBe('Permission denied (publickey)')
    expect(wrapper.text()).toContain('authorized_keys')
    expect(wrapper.text()).toContain('borg')
  })

  it('follows live status changes for its own agent only', async () => {
    const wrapper = await render()
    const handler = wsHandlers.get('TunnelStatusChanged')!

    handler({ agent_id: 2, status: 'reconnecting' })
    await flushPromises()
    expect(wrapper.get('[data-testid="tunnel-status"]').text()).toBe('Connected')

    handler({ agent_id: 1, status: 'reconnecting' })
    await flushPromises()
    expect(wrapper.get('[data-testid="tunnel-status"]').text()).toBe('Reconnecting')
  })

  describe('settings', () => {
    it('prefills the form and keeps Save off until something changes', async () => {
      const wrapper = await render({ ssh_user: 'borg', ssh_port: 2200 })

      expect((fieldByLabel(wrapper, 'SSH host').element as HTMLInputElement).value).toBe(
        '10.0.0.11',
      )
      expect((fieldByLabel(wrapper, 'SSH user').element as HTMLInputElement).value).toBe('borg')
      expect((fieldByLabel(wrapper, 'SSH port').element as HTMLInputElement).value).toBe('2200')
      expect(button(wrapper, 'Save')!.attributes('disabled')).toBeDefined()
      expect(button(wrapper, 'Reset')!.attributes('disabled')).toBeDefined()
    })

    it('sends every edited field and shows the saved values', async () => {
      vi.mocked(updateTunnel).mockResolvedValue({
        ...baseTunnel,
        ssh_host: '10.0.0.88',
        ssh_user: 'operator',
        ssh_port: 2200,
        tunnel_port: 2299,
      })
      const wrapper = await render()

      await setFieldByLabel(wrapper, 'SSH host', '10.0.0.88')
      await setFieldByLabel(wrapper, 'SSH user', 'operator')
      await setFieldByLabel(wrapper, 'SSH port', '2200')
      await setFieldByLabel(wrapper, 'Tunnel port', '2299')
      await click(wrapper, 'Save')

      expect(updateTunnel).toHaveBeenCalledWith(101, {
        ssh_host: '10.0.0.88',
        ssh_user: 'operator',
        ssh_port: 2200,
        tunnel_port: 2299,
      })
      expect(wrapper.text()).toContain('operator@10.0.0.88:2200')
      expect(button(wrapper, 'Save')!.attributes('disabled')).toBeDefined()
    })

    it('keeps Save off while the SSH host is empty', async () => {
      const wrapper = await render()

      await setFieldByLabel(wrapper, 'SSH host', '   ')
      await flushPromises()

      expect(button(wrapper, 'Save')!.attributes('disabled')).toBeDefined()
    })

    it('puts the stored values back on Reset', async () => {
      const wrapper = await render()

      await setFieldByLabel(wrapper, 'SSH host', '10.0.0.99')
      await click(wrapper, 'Reset')

      expect((fieldByLabel(wrapper, 'SSH host').element as HTMLInputElement).value).toBe(
        '10.0.0.11',
      )
      expect(updateTunnel).not.toHaveBeenCalled()
    })

    it('reports a save failure and keeps the edits', async () => {
      vi.mocked(updateTunnel).mockRejectedValue(new Error('port in use'))
      const wrapper = await render()

      await setFieldByLabel(wrapper, 'Tunnel port', '2299')
      await click(wrapper, 'Save')

      expect(wrapper.get('.form-error').text()).toContain('port in use')
      expect((fieldByLabel(wrapper, 'Tunnel port').element as HTMLInputElement).value).toBe('2299')
    })
  })

  describe('delete', () => {
    it('deletes the tunnel on confirmation and goes back to the list', async () => {
      vi.mocked(deleteTunnel).mockResolvedValue(undefined)
      const wrapper = await render()
      const push = vi.spyOn(wrapper.vm.$router, 'push')

      await click(wrapper, 'Delete tunnel')
      expect(wrapper.text()).toContain('Delete the tunnel for web-server-01?')
      await click(wrapper, 'Delete')

      expect(deleteTunnel).toHaveBeenCalledWith(101)
      expect(push).toHaveBeenCalledWith('/tunnels')
    })

    it('keeps the tunnel when the delete is cancelled', async () => {
      const wrapper = await render()

      await click(wrapper, 'Delete tunnel')
      await click(wrapper, 'Cancel')

      expect(deleteTunnel).not.toHaveBeenCalled()
    })

    it('reports a delete failure and stays on the page', async () => {
      vi.mocked(deleteTunnel).mockRejectedValue(new Error('in use'))
      const wrapper = await render()
      const push = vi.spyOn(wrapper.vm.$router, 'push')

      await click(wrapper, 'Delete tunnel')
      await click(wrapper, 'Delete')

      expect(wrapper.text()).toContain('in use')
      expect(push).not.toHaveBeenCalled()
    })
  })

  it('shows why the tunnel could not be loaded', async () => {
    vi.mocked(getTunnel).mockRejectedValue(new Error('Tunnel not found'))
    const wrapper = renderWithPlugins(TunnelDetailView, { props: { id: '999' } })
    await flushPromises()

    expect(wrapper.get('.error-banner').text()).toContain('Tunnel not found')
    expect(wrapper.find('.detail-name').exists()).toBe(false)
  })
})
