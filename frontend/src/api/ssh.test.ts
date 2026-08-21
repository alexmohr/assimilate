// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { createSshDirectory, listSshDirectory } from './ssh'

describe('ssh api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post).mockReset()
  })

  it('lists a remote directory', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { path: '/backup', entries: [{ name: 'repos', is_dir: true }] },
    })

    const result = await listSshDirectory({
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      path: '/backup',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/ssh/list-dir', {
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      path: '/backup',
    })
    expect(result).toEqual({ path: '/backup', entries: [{ name: 'repos', is_dir: true }] })
  })

  it('surfaces a listing error reported by the server', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { path: '/root', entries: [], error: 'Permission denied' },
    })

    const result = await listSshDirectory({
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      path: '/root',
    })

    expect(result.error).toBe('Permission denied')
  })

  it('creates a remote directory', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await createSshDirectory({
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      path: '/backup/nightly',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/ssh/mkdir', {
      ssh_host: 'backup.example.com',
      ssh_user: 'borg',
      ssh_port: 22,
      path: '/backup/nightly',
    })
  })
})
