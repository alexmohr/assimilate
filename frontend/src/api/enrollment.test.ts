// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { createEnrollmentToken, listEnrollmentTokens, revokeEnrollmentToken } from './enrollment'

describe('enrollment api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('creates a token with the lifetime, hostname and tags asked for', async () => {
    const created = { token: { id: 4 }, plaintext: 'enr_abc' }
    vi.mocked(apiClient.post).mockResolvedValue({ data: created })

    const result = await createEnrollmentToken({
      lifetime_secs: 900,
      hostname: 'db-02',
      tag_ids: [1, 2],
    })

    expect(apiClient.post).toHaveBeenCalledWith('/enrollment-tokens', {
      lifetime_secs: 900,
      hostname: 'db-02',
      tag_ids: [1, 2],
    })
    expect(result).toBe(created)
  })

  it('lists the tokens', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [] } })

    expect(await listEnrollmentTokens()).toEqual({ tokens: [] })
    expect(apiClient.get).toHaveBeenCalledWith('/enrollment-tokens')
  })

  it('revokes a token by id', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({ data: { revoked: true } })

    await revokeEnrollmentToken(7)

    expect(apiClient.delete).toHaveBeenCalledWith('/enrollment-tokens/7')
  })
})
