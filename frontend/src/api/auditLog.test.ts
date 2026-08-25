// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { getAuditLog } from './auditLog'

describe('auditLog api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('fetches the audit log with only pagination params', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { items: [], total: 0, page: 1, per_page: 25 },
    })

    await getAuditLog({ page: 1, per_page: 25 })

    expect(apiClient.get).toHaveBeenCalledWith('/audit-log', {
      params: { page: 1, per_page: 25 },
    })
  })

  it('includes optional filters when provided', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { items: [], total: 0, page: 2, per_page: 10 },
    })

    await getAuditLog({
      page: 2,
      per_page: 10,
      action: 'delete',
      user_id: 'alice',
      from: '2026-01-01',
      to: '2026-01-31',
    })

    expect(apiClient.get).toHaveBeenCalledWith('/audit-log', {
      params: {
        page: 2,
        per_page: 10,
        action: 'delete',
        user_id: 'alice',
        from: '2026-01-01',
        to: '2026-01-31',
      },
    })
  })

  it('omits empty string filters', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { items: [], total: 0, page: 1, per_page: 25 },
    })

    await getAuditLog({ page: 1, per_page: 25, action: '', user_id: '', from: '', to: '' })

    expect(apiClient.get).toHaveBeenCalledWith('/audit-log', {
      params: { page: 1, per_page: 25 },
    })
  })

  it('returns the response data', async () => {
    const data = {
      items: [
        {
          id: 1,
          user_id: 1,
          username: 'alice',
          action: 'create',
          target_type: 'agent',
          target_id: 5,
          details: null,
          ip_address: '127.0.0.1',
          created_at: '2026-01-01T00:00:00Z',
        },
      ],
      total: 1,
      page: 1,
      per_page: 25,
    }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(getAuditLog({ page: 1, per_page: 25 })).resolves.toEqual(data)
  })
})
