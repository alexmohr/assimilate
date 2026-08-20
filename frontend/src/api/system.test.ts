// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  exportConfig,
  getDatabaseStorage,
  getHealth,
  getSshPublicKey,
  getSystemSettings,
  getSystemVersion,
  importConfig,
  regenerateSshKey,
  resetSystem,
  updateSystemSettings,
} from './system'

describe('system api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('gets the ssh public key', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { public_key: 'ssh-ed25519 AAAA' } })

    await expect(getSshPublicKey()).resolves.toEqual({ public_key: 'ssh-ed25519 AAAA' })
    expect(apiClient.get).toHaveBeenCalledWith('/system/ssh-public-key')
  })

  it('regenerates the ssh key', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { public_key: 'ssh-ed25519 BBBB' } })

    await expect(regenerateSshKey()).resolves.toEqual({ public_key: 'ssh-ed25519 BBBB' })
    expect(apiClient.post).toHaveBeenCalledWith('/system/ssh-regenerate-key')
  })

  it('gets system settings', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { timezone: 'Europe/Berlin', retention_days: 7 },
    })

    await getSystemSettings()

    expect(apiClient.get).toHaveBeenCalledWith('/system/settings')
  })

  it('updates system settings', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} })

    await updateSystemSettings({
      retention_days: 7,
      report_retention_days: 0,
      failed_report_retention_days: 365,
      system_event_retention_days: 90,
      notification_delivery_retention_days: 30,
      timezone: 'Europe/Berlin',
      borg_query_timeout_secs: 300,
      session_idle_timeout_minutes: 480,
    })

    expect(apiClient.put).toHaveBeenCalledWith('/system/settings', {
      retention_days: 7,
      report_retention_days: 0,
      failed_report_retention_days: 365,
      system_event_retention_days: 90,
      notification_delivery_retention_days: 30,
      timezone: 'Europe/Berlin',
      borg_query_timeout_secs: 300,
      session_idle_timeout_minutes: 480,
    })
  })

  it('gets the system version', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        server_version: '1.0.0',
        server_git_sha: 'abc123',
        build_timestamp: '2026-01-01T00:00:00Z',
        agent_version: '1.0.0',
      },
    })

    await getSystemVersion()

    expect(apiClient.get).toHaveBeenCalledWith('/system/version')
  })

  it('gets database storage', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { database_bytes: 100, other_bytes: 10, relations: [] },
    })

    await getDatabaseStorage()

    expect(apiClient.get).toHaveBeenCalledWith('/system/database-storage')
  })

  it('resets the system', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { cancelled_backups: 2, notified_agents: 1 },
    })

    await expect(resetSystem()).resolves.toEqual({ cancelled_backups: 2, notified_agents: 1 })
    expect(apiClient.post).toHaveBeenCalledWith('/system/reset')
  })

  it('exports config', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { version: 1, hosts: [] } })

    await expect(exportConfig()).resolves.toEqual({ version: 1, hosts: [] })
    expect(apiClient.get).toHaveBeenCalledWith('/config/export')
  })

  it('imports config', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: {
        hosts_created: 1,
        hosts_updated: 0,
        schedules_created: 0,
        repos_created: 0,
        repos_updated: 0,
        warnings: [],
      },
    })

    const payload = { version: 1, hosts: [] }
    await importConfig(payload)

    expect(apiClient.post).toHaveBeenCalledWith('/config/import', payload)
  })

  it('checks health', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { status: 'ok' } })

    await getHealth()

    expect(apiClient.get).toHaveBeenCalledWith('/health')
  })
})
