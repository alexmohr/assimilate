<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useClipboard } from '../composables/useClipboard'
import { formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { Plus, Key, Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'

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
      <div class="modal">
        <template v-if="!newTokenPlaintext">
          <h2>Create API Token</h2>
          <form
            class="modal-form"
            @submit.prevent="submitCreate"
          >
            <div class="form-group">
              <label for="token-name">Token Name</label>
              <input
                id="token-name"
                v-model="createName"
                type="text"
                required
                placeholder="e.g. CI pipeline"
              />
            </div>
            <div
              v-if="createError"
              class="modal-error"
            >
              {{ createError }}
            </div>
            <div class="modal-actions">
              <button
                type="button"
                class="btn btn-ghost"
                @click="closeCreateModal"
              >
                Cancel
              </button>
              <button
                type="submit"
                class="btn btn-primary"
                :disabled="createSubmitting || !createName.trim()"
              >
                Create
              </button>
            </div>
          </form>
        </template>
        <template v-else>
          <h2>Token Created</h2>
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
          <div class="modal-actions">
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

    <div
      v-if="showDeleteModal"
      class="overlay"
      @click.self="showDeleteModal = false"
    >
      <div class="modal">
        <h2>Delete Token</h2>
        <p>
          Are you sure you want to delete token <strong>{{ deleteTarget?.name }}</strong
          >? This action cannot be undone.
        </p>
        <div class="modal-actions">
          <button
            class="btn btn-ghost"
            @click="showDeleteModal = false"
          >
            Cancel
          </button>
          <button
            class="btn btn-danger"
            :disabled="deleteSubmitting"
            @click="confirmDelete"
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tokens-page {
  max-width: 900px;
}

.loading {
  color: var(--text-muted);
  padding: 2rem 0;
}

.empty-state {
  color: var(--text-muted);
  padding: 2rem 0;
  font-size: 0.875rem;
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

.modal {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;
  width: 100%;
  max-width: 480px;
  box-shadow: var(--shadow-lg);
}

.modal h2 {
  font-size: 1.05rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0 0 1rem;
}

.modal-form {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.form-group label {
  font-size: 0.8125rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.form-group input {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.form-group input:focus {
  outline: none;
  border-color: var(--accent);
}

.modal-error {
  font-size: 0.8125rem;
  color: var(--danger);
  padding: 0.5rem 0.75rem;
  background: var(--danger-subtle);
  border-radius: var(--radius-sm);
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.5rem;
}
</style>
