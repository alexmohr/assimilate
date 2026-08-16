// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
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

  it('resets loading to false even when fetch fails', async () => {
    const { loading, fetchTokens } = useApiTokens()
    mockApiGet.mockRejectedValue(new Error('network error'))

    // fetchTokens uses try/finally without catch, so the promise rejects
    await expect(fetchTokens()).rejects.toThrow('network error')
    expect(loading.value).toBe(false)
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
})
