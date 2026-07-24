// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: vi.fn((_e: unknown, fallback?: string) => fallback ?? 'API error'),
}))

vi.mock('./useClipboard', () => ({
  useClipboard: () => ({
    copied: { value: false },
    copy: vi.fn(),
  }),
}))

import { apiClient } from '../api/client'
import { useTokenManagement } from './useTokenManagement'
import type { ApiTokenResponse as ApiToken } from '../types/generated'

const mockToken: ApiToken = {
  id: 'tok-1',
  name: 'test-token',
  created_at: '2026-07-01T00:00:00Z',
  last_used_at: null,
}

describe('useTokenManagement', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })
  })

  it('fetchTokens loads tokens and clears loading', async () => {
    const mgmt = useTokenManagement()
    expect(mgmt.tokensLoading.value).toBe(true)

    await mgmt.fetchTokens()

    expect(apiClient.get).toHaveBeenCalledWith('/tokens')
    expect(mgmt.tokens.value).toEqual([mockToken])
    expect(mgmt.tokensLoading.value).toBe(false)
  })

  it('fetchTokens clears loading even on failure', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('fail'))
    const mgmt = useTokenManagement()

    await mgmt.fetchTokens().catch(() => {})

    expect(mgmt.tokensLoading.value).toBe(false)
  })

  it('openCreate resets form fields and opens the modal', () => {
    const mgmt = useTokenManagement()
    mgmt.createName.value = 'old'
    mgmt.createError.value = 'old error'
    mgmt.newTokenPlaintext.value = 'old plaintext'
    mgmt.showCreateModal.value = false

    mgmt.openCreate()

    expect(mgmt.createName.value).toBe('')
    expect(mgmt.createError.value).toBe('')
    expect(mgmt.newTokenPlaintext.value).toBe('')
    expect(mgmt.showCreateModal.value).toBe(true)
  })

  it('submitCreate posts the token, stores plaintext, and refreshes list', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { token: mockToken, plaintext: 'secret-abc' },
    })
    const mgmt = useTokenManagement()
    mgmt.createName.value = 'my-token'

    await mgmt.submitCreate()

    expect(apiClient.post).toHaveBeenCalledWith('/tokens', { name: 'my-token' })
    expect(mgmt.newTokenPlaintext.value).toBe('secret-abc')
    expect(mgmt.createSubmitting.value).toBe(false)
  })

  it('submitCreate captures error and clears submitting on failure', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('create failed'))
    const mgmt = useTokenManagement()

    await mgmt.submitCreate()

    expect(mgmt.createError.value).toBe('Failed to create token')
    expect(mgmt.createSubmitting.value).toBe(false)
  })

  it('closeCreateModal resets modal state', () => {
    const mgmt = useTokenManagement()
    mgmt.showCreateModal.value = true
    mgmt.newTokenPlaintext.value = 'secret'
    mgmt.tokenCopied.value = true

    mgmt.closeCreateModal()

    expect(mgmt.showCreateModal.value).toBe(false)
    expect(mgmt.newTokenPlaintext.value).toBe('')
    expect(mgmt.tokenCopied.value).toBe(false)
  })

  it('openDelete sets the delete target and opens the modal', () => {
    const mgmt = useTokenManagement()

    mgmt.openDelete(mockToken)

    expect(mgmt.deleteTarget.value).toEqual(mockToken)
    expect(mgmt.showDeleteModal.value).toBe(true)
  })

  it('confirmDelete sends DELETE and refreshes list', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({ data: {} })
    const mgmt = useTokenManagement()
    mgmt.deleteTarget.value = mockToken

    await mgmt.confirmDelete()

    expect(apiClient.delete).toHaveBeenCalledWith('/tokens/tok-1')
    expect(mgmt.showDeleteModal.value).toBe(false)
    expect(mgmt.deleteTarget.value).toBeNull()
    expect(mgmt.deleteSubmitting.value).toBe(false)
  })

  it('confirmDelete does nothing when no target is set', async () => {
    const mgmt = useTokenManagement()
    mgmt.deleteTarget.value = null

    await mgmt.confirmDelete()

    expect(apiClient.delete).not.toHaveBeenCalled()
  })

  it('confirmDelete clears submitting on failure', async () => {
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('delete failed'))
    const mgmt = useTokenManagement()
    mgmt.deleteTarget.value = mockToken

    await mgmt.confirmDelete().catch(() => {})

    expect(mgmt.deleteSubmitting.value).toBe(false)
  })
})
