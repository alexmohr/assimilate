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
})
