// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  acceptRepoHostKey,
  deleteRepoHost,
  getRepoHost,
  listRepoHosts,
  scanRepoHostKey,
  updateRepoHost,
  updateRepoHostPower,
} from './repoHosts'

const HOST = {
  id: 5,
  ssh_host: 'nas.lan',
  ssh_port: 22,
  ssh_host_key: 'ssh-ed25519 AAAA',
  intermittent: true,
  power: {
    wake_enabled: false,
    wake_mac_address: null,
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    shutdown_after_backup: false,
  },
  repositories: [],
}

describe('repo hosts api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists the repository hosts', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [HOST] })

    await expect(listRepoHosts()).resolves.toEqual([HOST])

    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts')
  })

  it('gets one repository host', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: HOST })

    await expect(getRepoHost(5)).resolves.toEqual(HOST)

    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts/5')
  })

  it('changes where a host is reached', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: HOST })

    await expect(updateRepoHost(5, { ssh_host: 'nas.lan', ssh_port: 2222 })).resolves.toEqual(HOST)

    expect(apiClient.put).toHaveBeenCalledWith('/repo-hosts/5', {
      ssh_host: 'nas.lan',
      ssh_port: 2222,
    })
  })

  it('removes a host', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteRepoHost(5)

    expect(apiClient.delete).toHaveBeenCalledWith('/repo-hosts/5')
  })

  it("saves a host's power settings", async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: HOST })

    await expect(updateRepoHostPower(5, HOST.power)).resolves.toEqual(HOST)

    expect(apiClient.put).toHaveBeenCalledWith('/repo-hosts/5/power', HOST.power)
  })

  it('scans the key a host presents', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { ssh_host_key: 'ssh-ed25519 AAAA' } })

    await expect(scanRepoHostKey(5)).resolves.toEqual({ ssh_host_key: 'ssh-ed25519 AAAA' })

    expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key/scan')
  })

  it('accepts a key for every repository on the host', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await acceptRepoHostKey(5, 'ssh-ed25519 AAAA')

    expect(apiClient.post).toHaveBeenCalledWith('/repo-hosts/5/ssh-host-key', {
      ssh_host_key: 'ssh-ed25519 AAAA',
    })
  })
})
