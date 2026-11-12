// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useApiTokens } from './useApiTokens'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({
    copied: { value: false },
    copy: vi.fn().mockResolvedValue(undefined),
  }),
}))

vi.mock('../utils/error', () => ({
  extractError: (e: unknown, fallback: string) =>
    typeof e === 'object' && e !== null && 'message' in e
      ? String((e as { message: string }).message)
      : fallback,
}))

import { apiClient } from '../api/client'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>
const mockApiPost = apiClient.post as ReturnType<typeof vi.fn>
const mockApiDelete = apiClient.delete as ReturnType<typeof vi.fn>

const mockTokens: ApiToken[] = [
  {
    id: 1,
    user_id: 1,
    name: 'CI pipeline',
    created_at: '2026-01-01T00:00:00Z',
    last_used_at: null,
  },
  {
    id: 2,
    user_id: 1,
    name: 'deploy-bot',
    created_at: '2026-01-02T00:00:00Z',
    last_used_at: '2026-05-01T00:00:00Z',
  },
]

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockResolvedValue({ data: { tokens: mockTokens } })
  mockApiPost.mockResolvedValue({ data: { token: mockTokens[0], plaintext: 'tok_secret_abc123' } })
  mockApiDelete.mockResolvedValue({ data: {} })
})

describe('useApiTokens', () => {
  it('returns tokens after fetchTokens completes', async () => {
    const { tokens, loading, fetchTokens } = useApiTokens()

    expect(loading.value).toBe(true)
    await fetchTokens()

    expect(loading.value).toBe(false)
    expect(tokens.value).toEqual(mockTokens)
  })

  it('reports a failed fetch rather than leaving an empty list unexplained', async () => {
    const { loading, loadError, fetchTokens } = useApiTokens()
    mockApiGet.mockRejectedValue(new Error('network error'))

    await fetchTokens()

    expect(loading.value).toBe(false)
    expect(loadError.value).toContain('network error')
  })

  it('openCreate resets state and opens modal', () => {
    const { tokens, createName, createError, newTokenPlaintext, showCreateModal, openCreate } =
      useApiTokens()

    tokens.value = mockTokens
    createName.value = 'old name'
    createError.value = 'some error'
    newTokenPlaintext.value = 'old token'
    showCreateModal.value = true

    openCreate()

    expect(createName.value).toBe('')
    expect(createError.value).toBe('')
    expect(newTokenPlaintext.value).toBe('')
    expect(showCreateModal.value).toBe(true)
  })

  it('submitCreate posts and sets plaintext on success', async () => {
    const { tokens, createName, newTokenPlaintext, showCreateModal, submitCreate } = useApiTokens()

    createName.value = 'my-token'
    tokens.value = []

    await submitCreate()

    expect(mockApiPost).toHaveBeenCalledWith('/tokens', { name: 'my-token' })
    expect(newTokenPlaintext.value).toBe('tok_secret_abc123')
    expect(showCreateModal.value).toBe(false)
    expect(tokens.value).toEqual(mockTokens)
  })

  it('submitCreate sets createError on failure', async () => {
    const { createName, createError, submitCreate } = useApiTokens()

    mockApiPost.mockRejectedValue(new Error('rate limited'))
    createName.value = 'fail-token'

    await submitCreate()

    expect(createError.value).toBe('rate limited')
  })

  it('closeCreateModal resets modal and tokenCopied', () => {
    const { showCreateModal, newTokenPlaintext, tokenCopied, closeCreateModal } = useApiTokens()

    showCreateModal.value = true
    newTokenPlaintext.value = 'tok_secret'
    tokenCopied.value = true

    closeCreateModal()

    expect(showCreateModal.value).toBe(false)
    expect(newTokenPlaintext.value).toBe('')
    expect(tokenCopied.value).toBe(false)
  })

  it('openDelete sets the target and opens modal', () => {
    const { deleteTarget, showDeleteModal, openDelete } = useApiTokens()

    openDelete(mockTokens[0])

    expect(deleteTarget.value?.id).toBe(1)
    expect(deleteTarget.value?.name).toBe('CI pipeline')
    expect(showDeleteModal.value).toBe(true)
  })

  it('confirmDelete posts deletion and refetches on success', async () => {
    const { deleteTarget, showDeleteModal, tokens, confirmDelete } = useApiTokens()

    deleteTarget.value = mockTokens[0]
    tokens.value = []

    await confirmDelete()

    expect(mockApiDelete).toHaveBeenCalledWith('/tokens/1')
    expect(showDeleteModal.value).toBe(false)
    expect(deleteTarget.value).toBeNull()
    expect(tokens.value).toEqual(mockTokens)
  })

  it('confirmDelete is a no-op when deleteTarget is null', async () => {
    const { deleteTarget, confirmDelete, tokens } = useApiTokens()

    tokens.value = []
    deleteTarget.value = null

    await confirmDelete()

    expect(mockApiDelete).not.toHaveBeenCalled()
    expect(tokens.value).toEqual([])
  })

  it('submitCreate resets createSubmitting in finally block', async () => {
    const { createName, createSubmitting, submitCreate } = useApiTokens()

    createName.value = 'test'
    await submitCreate()

    expect(createSubmitting.value).toBe(false)
  })

  describe('expiry', () => {
    const NOW = new Date('2026-06-01T12:00:00Z')

    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['Date'] })
      vi.setSystemTime(NOW)
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('starts a new token as never expiring', () => {
      const { createExpiry, createExpiryDate, newTokenExpiresAt } = useApiTokens()
      expect(createExpiry.value).toBe('never')
      expect(createExpiryDate.value).toBe('')
      expect(newTokenExpiresAt.value).toBeNull()
    })

    it('openCreate forgets the expiry chosen for the previous token', () => {
      const { createExpiry, createExpiryDate, openCreate } = useApiTokens()
      createExpiry.value = 'date'
      createExpiryDate.value = '2026-07-15'

      openCreate()

      expect(createExpiry.value).toBe('never')
      expect(createExpiryDate.value).toBe('')
    })

    it('creates a never-expiring token without an expiry', async () => {
      const { createName, newTokenExpiresAt, submitCreate } = useApiTokens()
      createName.value = 'forever'

      await submitCreate()

      expect(mockApiPost).toHaveBeenCalledWith('/tokens', { name: 'forever' })
      expect(newTokenExpiresAt.value).toBeNull()
    })

    it('sends a preset length counted from now, and remembers it for the reveal', async () => {
      const { createName, createExpiry, newTokenExpiresAt, submitCreate } = useApiTokens()
      createName.value = 'ci'
      createExpiry.value = '90d'

      await submitCreate()

      const expected = new Date(NOW.getTime() + 90 * 86_400_000).toISOString()
      expect(mockApiPost).toHaveBeenCalledWith('/tokens', { name: 'ci', expires_at: expected })
      expect(newTokenExpiresAt.value).toBe(expected)
    })

    it('sends the end of a picked day alongside the scope', async () => {
      const { createName, createScope, createExpiry, createExpiryDate, submitCreate } =
        useApiTokens()
      createName.value = 'prometheus'
      createScope.value = 'monitoring'
      createExpiry.value = 'date'
      createExpiryDate.value = '2026-07-15'

      await submitCreate()

      expect(mockApiPost).toHaveBeenCalledWith('/tokens', {
        name: 'prometheus',
        scope: 'monitoring',
        expires_at: new Date(2026, 6, 15, 23, 59, 59).toISOString(),
      })
    })

    it('is not ready on a date choice until a day is picked, and creates nothing', async () => {
      const { createName, createExpiry, createExpiryDate, createReady, submitCreate } =
        useApiTokens()
      expect(createReady.value).toBe(false)
      createName.value = 'ci'
      expect(createReady.value).toBe(true)
      createExpiry.value = 'date'
      expect(createReady.value).toBe(false)

      await submitCreate()
      expect(mockApiPost).not.toHaveBeenCalled()

      createExpiryDate.value = '2026-07-15'
      expect(createReady.value).toBe(true)
    })

    it('does not record an expiry for a token whose creation failed', async () => {
      const { createName, createExpiry, newTokenExpiresAt, newTokenPlaintext, submitCreate } =
        useApiTokens()
      mockApiPost.mockRejectedValue(new Error('nope'))
      createName.value = 'ci'
      createExpiry.value = '30d'

      await submitCreate()

      expect(newTokenExpiresAt.value).toBeNull()
      expect(newTokenPlaintext.value).toBe('')
    })
  })
})
