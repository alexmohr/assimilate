// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  checkAppendOnly,
  getAppendOnlyStatus,
  keepArchive,
  reapplyAppendOnly,
  resumeMaintenance,
  runMaintenance,
  setAppendOnly,
  updateMaintenance,
} from './appendOnly'

describe('appendOnly api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('gets the status', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { enabled: true } })
    await expect(getAppendOnlyStatus(4)).resolves.toEqual({ enabled: true })
    expect(apiClient.get).toHaveBeenCalledWith('/repos/4/append-only')
  })

  it('sets append-only with an optional password', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: { enabled: true } })
    await setAppendOnly(4, { enabled: true, password: 'pw' })
    expect(apiClient.put).toHaveBeenCalledWith('/repos/4/append-only', {
      enabled: true,
      password: 'pw',
    })
  })

  it('checks the connection', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { enabled: true } })
    await checkAppendOnly(4)
    expect(apiClient.post).toHaveBeenCalledWith('/repos/4/append-only/check')
  })

  it('reapplies the keys', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { enabled: true } })
    await reapplyAppendOnly(4, { password: null })
    expect(apiClient.post).toHaveBeenCalledWith('/repos/4/append-only/reapply', { password: null })
  })

  it('updates the maintenance schedule', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: { enabled: true } })
    await updateMaintenance(4, { cron_expression: '0 4 * * 0', check_before_prune: true })
    expect(apiClient.put).toHaveBeenCalledWith('/repos/4/append-only/maintenance', {
      cron_expression: '0 4 * * 0',
      check_before_prune: true,
    })
  })

  it('starts a maintenance run', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: undefined })
    await expect(runMaintenance(4)).resolves.toBeUndefined()
    expect(apiClient.post).toHaveBeenCalledWith('/repos/4/append-only/maintenance/run')
  })

  it('resumes maintenance', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { enabled: true } })
    await resumeMaintenance(4)
    expect(apiClient.post).toHaveBeenCalledWith('/repos/4/append-only/maintenance/resume')
  })

  it('keeps a queued archive, encoding its name', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({ data: undefined })
    await keepArchive(4, 'nas 2026/10')
    expect(apiClient.delete).toHaveBeenCalledWith('/repos/4/archives/nas%202026%2F10/deletion')
  })
})
