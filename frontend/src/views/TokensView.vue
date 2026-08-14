<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useClipboard } from '../composables/useClipboard'
import { useAsyncAction } from '../composables/useAsyncAction'
import { formatDate } from '../utils/format'
import { Plus, Key, Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

const tokens = ref<ApiToken[]>([])
const loading = ref(true)

const showCreateModal = ref(false)
const createName = ref('')
const {
  loading: createSubmitting,
  error: createError,
  run: runCreate,
} = useAsyncAction('Failed to create token')

const newTokenPlaintext = ref('')
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

const showDeleteModal = ref(false)
const deleteTarget = ref<ApiToken | null>(null)
const { loading: deleteSubmitting, error: deleteError, run: runDelete } = useAsyncAction()

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
  createError.value = null
  newTokenPlaintext.value = ''
  showCreateModal.value = true
}

async function submitCreate(): Promise<void> {
  await runCreate(async () => {
    const res = await apiClient.post<{ token: ApiToken; plaintext: string }>('/tokens', {
      name: createName.value,
    })
    newTokenPlaintext.value = res.data.plaintext
    await fetchTokens()
  })
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
  const target = deleteTarget.value
  if (!target) return
  await runDelete(async () => {
    await apiClient.delete(`/tokens/${target.id}`)
    showDeleteModal.value = false
    deleteTarget.value = null
    await fetchTokens()
  })
}

onMounted(fetchTokens)
</script>

<template>
  <div class="tokens-page">
    <div class="page-header">
      <h1 class="page-title">API Tokens</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openCreate"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />

    <table
      v-else-if="tokens.length"
      class="tokens-table"
    >
      <thead>
        <tr>
          <th>Name</th>
          <th>Created</th>
          <th>Last Used</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="token in tokens"
          :key="token.id"
        >
          <td>{{ token.name }}</td>
          <td class="date-cell">
            {{ formatDate(token.created_at) }}
          </td>
          <td class="date-cell">
            {{ formatDate(token.last_used_at, 'Never') }}
          </td>
          <td>
            <button
              class="btn btn-sm btn-ghost btn-danger-text"
              title="Delete"
              @click="openDelete(token)"
            >
              <Trash2 :size="14" />
            </button>
          </td>
        </tr>
      </tbody>
    </table>

    <EmptyState
      v-else
      :icon="Key"
      title="No API tokens"
      description="Create one to get started."
      action="Create Token"
      @action="showCreateModal = true"
    />

    <div
      v-if="showCreateModal"
      class="overlay"
      @click.self="closeCreateModal"
    >
      <div class="dialog">
        <template v-if="!newTokenPlaintext">
          <div class="dialog-header">
            <h2 class="dialog-title">Create API Token</h2>
            <button
              class="close-btn"
              @click="closeCreateModal"
            >
              &times;
            </button>
          </div>
          <form @submit.prevent="submitCreate">
            <div class="dialog-body">
              <div class="field">
                <label
                  class="field-label"
                  for="token-name"
                  >Token Name</label
                >
                <input
                  id="token-name"
                  v-model="createName"
                  type="text"
                  class="input"
                  required
                  placeholder="e.g. CI pipeline"
                />
              </div>
            </div>
            <ModalFormActions
              :submitting="createSubmitting"
              :disabled="!createName.trim()"
              :error="createError"
              submit-label="Create"
              submitting-label="Create"
              @cancel="closeCreateModal"
            />
          </form>
        </template>
        <template v-else>
          <div class="dialog-header">
            <h2 class="dialog-title">Token Created</h2>
          </div>
          <div class="dialog-body">
            <div class="token-notice">
              <p class="token-warning">Copy this token now. It will not be shown again.</p>
              <div class="token-box">
                <code class="token-text">{{ newTokenPlaintext }}</code>
                <button
                  class="btn btn-sm"
                  @click="copyToClipboard(newTokenPlaintext)"
                >
                  {{ tokenCopied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="closeCreateModal"
            >
              Done
            </button>
          </div>
        </template>
      </div>
    </div>

    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete Token"
      :submitting="deleteSubmitting"
      :error="deleteError"
      @cancel="showDeleteModal = false"
      @confirm="confirmDelete"
    >
      Are you sure you want to delete token <strong>{{ deleteTarget?.name }}</strong
      >? This action cannot be undone.
    </ConfirmDeleteDialog>
  </div>
</template>

<style scoped>
.tokens-page {
  max-width: 900px;
}

.tokens-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
}

.tokens-table th {
  text-align: left;
  padding: 0.625rem 0.75rem;
  font-weight: 600;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
}

.tokens-table td {
  padding: 0.625rem 0.75rem;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-primary);
}

.date-cell {
  color: var(--text-secondary);
  font-size: 0.8125rem;
}

.token-notice {
  margin: 1rem 0;
}

.token-warning {
  font-size: 0.8125rem;
  color: var(--warning, var(--text-secondary));
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.token-box {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem 0.75rem;
}

.token-text {
  flex: 1;
  font-size: 0.75rem;
  font-family: monospace;
  word-break: break-all;
  color: var(--text-primary);
}
</style>
