// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { createToken, deleteToken, listTokens } from './tokens'

describe('tokens api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists tokens', async () => {
    const token = {
      id: 1,
      user_id: 2,
      name: 'ci-token',
      created_at: '2026-01-01T00:00:00Z',
      last_used_at: null,
    }
    vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [token] } })

    await expect(listTokens()).resolves.toEqual([token])
    expect(apiClient.get).toHaveBeenCalledWith('/tokens')
  })

  it('creates a token', async () => {
    const response = {
      token: {
        id: 1,
        user_id: 2,
        name: 'ci-token',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
      },
      plaintext: 'secret-plaintext-token',
    }
    vi.mocked(apiClient.post).mockResolvedValue({ data: response })

    await expect(createToken('ci-token')).resolves.toEqual(response)
    expect(apiClient.post).toHaveBeenCalledWith('/tokens', { name: 'ci-token' })
  })

  it('deletes a token', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteToken(4)

    expect(apiClient.delete).toHaveBeenCalledWith('/tokens/4')
  })
})
