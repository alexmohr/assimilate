// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { createToken, deleteToken, listTokens, type ApiToken } from '../api/tokens'
import type { ApiTokenScope } from '../types/generated'
import { extractError } from '../utils/error'
import { tokenExpiresAt, type TokenExpiryChoice } from '../utils/apiToken'
import { useClipboard } from './useClipboard'

interface UseApiTokensReturn {
  tokens: Ref<ApiToken[]>
  loading: Ref<boolean>
  /** Why the list is empty, when it is empty because the load failed. */
  loadError: Ref<string>
  showCreateModal: Ref<boolean>
  createName: Ref<string>
  /** What the token being created may do; a full token unless changed. */
  createScope: Ref<ApiTokenScope>
  /** When the token being created expires: a preset length, or a picked date. */
  createExpiry: Ref<TokenExpiryChoice>
  /** The picked date, `YYYY-MM-DD`, when `createExpiry` is `date`. */
  createExpiryDate: Ref<string>
  /** The expiry of the token just created, while its plaintext is shown. */
  newTokenExpiresAt: Ref<string | null>
  /**
   * Whether the token being created can be submitted: it has a name, and
   * "On a date" has its day picked. Without the day the token would never
   * expire, the opposite of what was asked.
   */
  createReady: ComputedRef<boolean>
  /** The scope of the token just created, while its plaintext is shown. */
  newTokenScope: Ref<ApiTokenScope>
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
  const createScope = ref<ApiTokenScope>('full')
  const createExpiry = ref<TokenExpiryChoice>('never')
  const createExpiryDate = ref('')
  const newTokenExpiresAt = ref<string | null>(null)
  const createReady = computed(
    () =>
      createName.value.trim() !== '' &&
      !(createExpiry.value === 'date' && createExpiryDate.value === ''),
  )
  const newTokenScope = ref<ApiTokenScope>('full')
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
    createScope.value = 'full'
    createExpiry.value = 'never'
    createExpiryDate.value = ''
    createError.value = ''
    newTokenPlaintext.value = ''
    showCreateModal.value = true
  }

  async function submitCreate(): Promise<void> {
    if (!createReady.value) return
    createError.value = ''
    createSubmitting.value = true
    try {
      const expiresAt = tokenExpiresAt(createExpiry.value, createExpiryDate.value)
      const res = await createToken(createName.value, createScope.value, expiresAt)
      newTokenScope.value = createScope.value
      newTokenExpiresAt.value = expiresAt
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
    createScope,
    createExpiry,
    createExpiryDate,
    createReady,
    newTokenExpiresAt,
    newTokenScope,
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
