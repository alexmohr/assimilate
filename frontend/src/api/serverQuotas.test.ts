// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { deleteServerQuota, listServerQuotas, upsertServerQuota } from './serverQuotas'

describe('serverQuotas api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists the server quotas', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await expect(listServerQuotas()).resolves.toEqual([])
    expect(apiClient.get).toHaveBeenCalledWith('/server-quotas')
  })

  it('upserts a quota under the encoded ssh host and returns the stored quota', async () => {
    const stored = { ssh_host: 'backup host' }
    vi.mocked(apiClient.put).mockResolvedValue({ data: stored })
    const request = {} as Parameters<typeof upsertServerQuota>[1]

    await expect(upsertServerQuota('backup host', request)).resolves.toBe(stored)
    expect(apiClient.put).toHaveBeenCalledWith('/server-quotas/backup%20host', request)
  })

  it('deletes a quota under the encoded ssh host', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({ data: undefined })

    await deleteServerQuota('backup host')
    expect(apiClient.delete).toHaveBeenCalledWith('/server-quotas/backup%20host')
  })
})
