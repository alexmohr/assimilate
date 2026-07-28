// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { useClipboard } from './useClipboard'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

interface UseApiTokensReturn {
  tokens: Ref<ApiToken[]>
  loading: Ref<boolean>
  showCreateModal: Ref<boolean>
  createName: Ref<string>
  createError: Ref<string>
  createSubmitting: Ref<boolean>
  newTokenPlaintext: Ref<string>
  tokenCopied: Ref<boolean>
  copyToClipboard: (text: string) => Promise<void>
  showDeleteModal: Ref<boolean>
  deleteTarget: Ref<ApiToken | null>
  deleteSubmitting: Ref<boolean>
  fetchTokens: () => Promise<void>
  openCreate: () => void
  submitCreate: () => Promise<void>
  closeCreateModal: () => void
  openDelete: (token: ApiToken) => void
  confirmDelete: () => Promise<void>
}

export function useApiTokens(): UseApiTokensReturn {
  const tokens = ref<ApiToken[]>([])
  const loading = ref(true)
  const showCreateModal = ref(false)
  const createName = ref('')
  const createError = ref('')
  const createSubmitting = ref(false)
  const newTokenPlaintext = ref('')
  const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()
  const showDeleteModal = ref(false)
  const deleteTarget = ref<ApiToken | null>(null)
  const deleteSubmitting = ref(false)

  async function fetchTokens(): Promise<void> {
    loading.value = true
    try {
      const res = await apiClient.get<{ tokens: ApiToken[] }>('/tokens')
      tokens.value = res.data.tokens
    } finally {
      loading.value = false
    }
  }

  function openCreate(): void {
    createName.value = ''
    createError.value = ''
    newTokenPlaintext.value = ''
    showCreateModal.value = true
  }

  async function submitCreate(): Promise<void> {
    createError.value = ''
    createSubmitting.value = true
    try {
      const res = await apiClient.post<{ token: ApiToken; plaintext: string }>('/tokens', {
        name: createName.value,
      })
      newTokenPlaintext.value = res.data.plaintext
      await fetchTokens()
    } catch (e: unknown) {
      createError.value = extractError(e, 'Failed to create token')
    } finally {
      createSubmitting.value = false
    }
  }

  function closeCreateModal(): void {
    showCreateModal.value = false
    newTokenPlaintext.value = ''
    tokenCopied.value = false
  }

  function openDelete(token: ApiToken): void {
    deleteTarget.value = token
    showDeleteModal.value = true
  }

  async function confirmDelete(): Promise<void> {
    if (!deleteTarget.value) return
    deleteSubmitting.value = true
    try {
      await apiClient.delete(`/tokens/${deleteTarget.value.id}`)
      showDeleteModal.value = false
      deleteTarget.value = null
      await fetchTokens()
    } finally {
      deleteSubmitting.value = false
    }
  }

  return {
    tokens,
    loading,
    showCreateModal,
    createName,
    createError,
    createSubmitting,
    newTokenPlaintext,
    tokenCopied,
    copyToClipboard,
    showDeleteModal,
    deleteTarget,
    deleteSubmitting,
    fetchTokens,
    openCreate,
    submitCreate,
    closeCreateModal,
    openDelete,
    confirmDelete,
  }
}
