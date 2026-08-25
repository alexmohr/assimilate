// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  createTunnel,
  deleteTunnel,
  disableTunnel,
  enableTunnel,
  getTunnel,
  listTunnels,
  updateTunnel,
} from './tunnels'

describe('tunnels api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists tunnels', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listTunnels()

    expect(apiClient.get).toHaveBeenCalledWith('/tunnels')
  })

  it('gets a tunnel', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: {} })

    await getTunnel(7)

    expect(apiClient.get).toHaveBeenCalledWith('/tunnels/7')
  })

  it('creates a tunnel', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await createTunnel({
      agent_id: 1,
      ssh_host: 'ssh.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      tunnel_port: 2222,
      enabled: true,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels', {
      agent_id: 1,
      ssh_host: 'ssh.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      tunnel_port: 2222,
      enabled: true,
    })
  })

  it('updates a tunnel', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} })

    await updateTunnel(7, { enabled: false })

    expect(apiClient.put).toHaveBeenCalledWith('/tunnels/7', { enabled: false })
  })

  it('deletes a tunnel', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteTunnel(7)

    expect(apiClient.delete).toHaveBeenCalledWith('/tunnels/7')
  })

  it('enables a tunnel', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await enableTunnel(7)

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels/7/enable')
  })

  it('disables a tunnel', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await disableTunnel(7)

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels/7/disable')
  })
})
