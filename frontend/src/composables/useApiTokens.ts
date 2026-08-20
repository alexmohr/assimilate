// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { createToken, deleteToken, listTokens, type ApiToken } from '../api/tokens'
import { extractError } from '../utils/error'
import { useClipboard } from './useClipboard'

interface UseApiTokensReturn {
  tokens: Ref<ApiToken[]>
  loading: Ref<boolean>
  /** Why the list is empty, when it is empty because the load failed. */
  loadError: Ref<string>
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
  deleteError: Ref<string>
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
  const loadError = ref('')
  const showCreateModal = ref(false)
  const createName = ref('')
  const createError = ref('')
  const createSubmitting = ref(false)
  const newTokenPlaintext = ref('')
  const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()
  const showDeleteModal = ref(false)
  const deleteTarget = ref<ApiToken | null>(null)
  const deleteSubmitting = ref(false)
  const deleteError = ref('')

  async function fetchTokens(): Promise<void> {
    loading.value = true
    loadError.value = ''
    try {
      tokens.value = await listTokens()
    } catch (e: unknown) {
      loadError.value = extractError(e, 'Failed to load API tokens')
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
      const res = await createToken(createName.value)
      newTokenPlaintext.value = res.plaintext
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
    deleteError.value = ''
    showDeleteModal.value = true
  }

  async function confirmDelete(): Promise<void> {
    if (!deleteTarget.value) return
    deleteError.value = ''
    deleteSubmitting.value = true
    try {
      await deleteToken(deleteTarget.value.id)
      showDeleteModal.value = false
      deleteTarget.value = null
      await fetchTokens()
    } catch (e: unknown) {
      deleteError.value = extractError(e, 'Failed to delete token')
    } finally {
      deleteSubmitting.value = false
    }
  }

  return {
    tokens,
    loading,
    loadError,
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
    deleteError,
    fetchTokens,
    openCreate,
    submitCreate,
    closeCreateModal,
    openDelete,
    confirmDelete,
  }
}
