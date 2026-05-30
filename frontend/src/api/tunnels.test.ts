// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

type MockedApiClient = {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
  put: ReturnType<typeof vi.fn>
  delete: ReturnType<typeof vi.fn>
}

const apiClient = vi.hoisted<MockedApiClient>(() => ({
  get: vi.fn(),
  post: vi.fn(),
  put: vi.fn(),
  delete: vi.fn(),
}))

vi.mock('./client', () => ({
  apiClient,
}))

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
    apiClient.get.mockReset()
    apiClient.post.mockReset()
    apiClient.put.mockReset()
    apiClient.delete.mockReset()
  })

  it('lists tunnels', async () => {
    apiClient.get.mockResolvedValue({ data: [] })

    await listTunnels()

    expect(apiClient.get).toHaveBeenCalledWith('/tunnels')
  })

  it('gets a tunnel', async () => {
    apiClient.get.mockResolvedValue({ data: {} })

    await getTunnel(7)

    expect(apiClient.get).toHaveBeenCalledWith('/tunnels/7')
  })

  it('creates a tunnel', async () => {
    apiClient.post.mockResolvedValue({ data: {} })

    await createTunnel({
      client_id: 1,
      ssh_host: 'ssh.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      tunnel_port: 2222,
      enabled: true,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels', {
      client_id: 1,
      ssh_host: 'ssh.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      tunnel_port: 2222,
      enabled: true,
    })
  })

  it('updates a tunnel', async () => {
    apiClient.put.mockResolvedValue({ data: {} })

    await updateTunnel(7, { enabled: false })

    expect(apiClient.put).toHaveBeenCalledWith('/tunnels/7', { enabled: false })
  })

  it('deletes a tunnel', async () => {
    apiClient.delete.mockResolvedValue({})

    await deleteTunnel(7)

    expect(apiClient.delete).toHaveBeenCalledWith('/tunnels/7')
  })

  it('enables a tunnel', async () => {
    apiClient.post.mockResolvedValue({})

    await enableTunnel(7)

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels/7/enable')
  })

  it('disables a tunnel', async () => {
    apiClient.post.mockResolvedValue({})

    await disableTunnel(7)

    expect(apiClient.post).toHaveBeenCalledWith('/tunnels/7/disable')
  })
})
