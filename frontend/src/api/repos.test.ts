// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  acceptRepoSshHostKey,
  breakRepoLock,
  confirmRepoRelocation,
  createRepo,
  deleteRepo,
  destroyRepo,
  execRepoCommand,
  getRepo,
  getRepoPassphrase,
  getRepoQuota,
  initRepo,
  listRepoStats,
  listRepoTags,
  listRepos,
  rescanRepo,
  resetAndSyncRepo,
  resetImportRepo,
  scanRepoSshHostKey,
  syncRepo,
  testRepoConnection,
  updateRepo,
  updateRepoPermission,
  updateRepoQuota,
} from './repos'

describe('repos api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists repos', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listRepos()

    expect(apiClient.get).toHaveBeenCalledWith('/repos')
  })

  it('lists repo stats', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listRepoStats()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/stats')
  })

  it('lists repo tags', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listRepoTags()

    expect(apiClient.get).toHaveBeenCalledWith('/repo-tags')
  })

  it('gets a repo', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: {} })

    await getRepo(12)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12')
  })

  it('creates a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await createRepo({
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      passphrase: 'secret',
      compression: 'zstd',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/repos', {
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      passphrase: 'secret',
      compression: 'zstd',
    })
  })

  it('initializes a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await initRepo({
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      passphrase: 'secret',
      encryption: 'repokey-blake2',
      compression: 'zstd',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/repos/init', {
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      passphrase: 'secret',
      encryption: 'repokey-blake2',
      compression: 'zstd',
    })
  })

  it('updates a repo', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateRepo(12, {
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      compression: 'zstd',
      encryption: 'repokey-blake2',
      enabled: true,
      sync_schedule: null,
    })

    expect(apiClient.put).toHaveBeenCalledWith('/repos/12', {
      name: 'nas',
      repo_path: '/backup/repos/nas',
      ssh_user: 'root',
      ssh_host: '10.0.0.1',
      ssh_port: 22,
      compression: 'zstd',
      encryption: 'repokey-blake2',
      enabled: true,
      sync_schedule: null,
    })
  })

  it('deletes a repo', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteRepo(12)

    expect(apiClient.delete).toHaveBeenCalledWith('/repos/12')
  })

  it('destroys a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await destroyRepo(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/destroy')
  })

  it('syncs a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await syncRepo(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/sync?build_index=true')
  })

  it('resets and re-imports a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await resetImportRepo(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/reset-import')
  })

  it('resets and syncs a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await resetAndSyncRepo(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/reset-and-sync?build_index=true')
  })

  it('gets the repo passphrase', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { passphrase: 'secret' } })

    await getRepoPassphrase(12)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/passphrase')
  })

  it('scans the repo ssh host key', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { ssh_host_key: 'ssh-ed25519 AAAA' } })

    await scanRepoSshHostKey(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/ssh-host-key/scan')
  })

  it('accepts the repo ssh host key', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await acceptRepoSshHostKey(12, 'ssh-ed25519 AAAA')

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/ssh-host-key', {
      ssh_host_key: 'ssh-ed25519 AAAA',
    })
  })

  it('confirms a repo relocation', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await confirmRepoRelocation(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/confirm-relocation')
  })

  it('breaks a repo lock', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await breakRepoLock(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/break-lock')
  })

  it('executes a borg command on a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await execRepoCommand(12, ['list'])

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/exec', { args: ['list'] })
  })

  it('rescans a repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await rescanRepo(12)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/rescan')
  })

  it('tests an SSH connection for a would-be repo', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { ssh_ok: true, borg_installed: true, borg_version: '1.4.0' },
    })

    await testRepoConnection({
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/ssh/test-connection', {
      ssh_host: '10.0.0.1',
      ssh_user: 'root',
      ssh_port: 22,
    })
  })

  it('gets a repo quota', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        warn_bytes: 1000,
        critical_bytes: 2000,
        warn_action: 'notify_only',
        critical_action: 'block_backups',
        enabled: true,
      },
    })

    await getRepoQuota(12)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/quota')
  })

  it('updates a repo quota', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateRepoQuota(12, {
      warn_bytes: 1000,
      critical_bytes: 2000,
      warn_action: 'notify_only',
      critical_action: 'block_backups',
      enabled: true,
    })

    expect(apiClient.put).toHaveBeenCalledWith('/repos/12/quota', {
      warn_bytes: 1000,
      critical_bytes: 2000,
      warn_action: 'notify_only',
      critical_action: 'block_backups',
      enabled: true,
    })
  })

  it('updates a repo permission', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateRepoPermission(12, 4, {
      can_view: true,
      can_backup: true,
      can_modify_schedules: false,
      can_extract: true,
      can_delete: false,
    })

    expect(apiClient.put).toHaveBeenCalledWith('/repos/12/permissions/4', {
      can_view: true,
      can_backup: true,
      can_modify_schedules: false,
      can_extract: true,
      can_delete: false,
    })
  })
})
