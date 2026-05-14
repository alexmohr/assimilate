<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useTheme } from '../composables/useTheme'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { formatDate } from '../utils/format'
import { validatePassword } from '../utils/validation'
import { extractError } from '../utils/error'
import { Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'

type TabId = 'password' | 'tokens' | 'appearance'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

const authStore = useAuthStore()
const { theme, setTheme, loadFromBackend } = useTheme()
const activeTab = ref<TabId>('password')

const newPassword = ref('')
const confirmPassword = ref('')
const passwordError = ref('')
const passwordSuccess = ref('')
const passwordSubmitting = ref(false)

const tokens = ref<ApiToken[]>([])
const tokensLoading = ref(true)
const showCreateModal = ref(false)
const createName = ref('')
const createError = ref('')
const createSubmitting = ref(false)
const newTokenPlaintext = ref('')
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()
const showDeleteModal = ref(false)
const deleteTarget = ref<ApiToken | null>(null)
const deleteSubmitting = ref(false)

useEscapeKey(showCreateModal, closeCreateModal)

useEscapeKey(showDeleteModal, () => {
  showDeleteModal.value = false
})

async function handlePasswordSubmit(): Promise<void> {
  passwordError.value = ''
  passwordSuccess.value = ''

  const validationError = validatePassword(newPassword.value, confirmPassword.value)
  if (validationError) {
    passwordError.value = validationError
    return
  }

  passwordSubmitting.value = true
  try {
    await authStore.changePassword(newPassword.value)
    passwordSuccess.value = 'Password changed successfully.'
    newPassword.value = ''
    confirmPassword.value = ''
  } catch (e: unknown) {
    passwordError.value = extractError(e, 'Failed to change password')
  } finally {
    passwordSubmitting.value = false
  }
}

async function fetchTokens(): Promise<void> {
  tokensLoading.value = true
  try {
    const res = await apiClient.get<{ tokens: ApiToken[] }>('/tokens')
    tokens.value = res.data.tokens
  } finally {
    tokensLoading.value = false
  }
}

function openCreateToken(): void {
  createName.value = ''
  createError.value = ''
  newTokenPlaintext.value = ''
  showCreateModal.value = true
}

async function submitCreateToken(): Promise<void> {
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

function openDeleteToken(token: ApiToken): void {
  deleteTarget.value = token
  showDeleteModal.value = true
}

async function confirmDeleteToken(): Promise<void> {
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

onMounted(() => {
  fetchTokens()
  loadFromBackend()
})
</script>

<template>
  <div class="profile-view">
    <div class="page-header">
      <h1 class="page-title">Profile</h1>
    </div>
    <p class="page-subtitle">
      {{ authStore.user?.username }}
    </p>

    <div class="tabs">
      <button
        class="tab"
        :class="{ active: activeTab === 'password' }"
        @click="activeTab = 'password'"
      >
        Change Password
      </button>
      <button
        class="tab"
        :class="{ active: activeTab === 'tokens' }"
        @click="activeTab = 'tokens'"
      >
        API Tokens
      </button>
      <button
        class="tab"
        :class="{ active: activeTab === 'appearance' }"
        @click="activeTab = 'appearance'"
      >
        Appearance
      </button>
    </div>

    <!-- Password Tab -->
    <div
      v-if="activeTab === 'password'"
      class="tab-content"
    >
      <form
        class="password-form"
        @submit.prevent="handlePasswordSubmit"
      >
        <div class="form-group">
          <label
            class="form-label"
            for="profile-new-pw"
            >New Password</label
          >
          <input
            id="profile-new-pw"
            v-model="newPassword"
            type="password"
            class="form-input"
            autocomplete="new-password"
            placeholder="Minimum 8 characters"
            :disabled="passwordSubmitting"
          />
        </div>

        <div class="form-group">
          <label
            class="form-label"
            for="profile-confirm-pw"
            >Confirm Password</label
          >
          <input
            id="profile-confirm-pw"
            v-model="confirmPassword"
            type="password"
            class="form-input"
            autocomplete="new-password"
            :disabled="passwordSubmitting"
          />
        </div>

        <div
          v-if="passwordError"
          class="msg msg-error"
        >
          {{ passwordError }}
        </div>
        <div
          v-if="passwordSuccess"
          class="msg msg-success"
        >
          {{ passwordSuccess }}
        </div>

        <button
          type="submit"
          class="btn btn-primary"
          :disabled="passwordSubmitting"
        >
          {{ passwordSubmitting ? 'Saving...' : 'Update Password' }}
        </button>
      </form>
    </div>

    <!-- Tokens Tab -->
    <div
      v-if="activeTab === 'tokens'"
      class="tab-content"
    >
      <div class="tokens-header">
        <p class="tokens-desc">
          API tokens allow external tools to authenticate without your password.
        </p>
        <button
          class="btn btn-primary btn-sm"
          @click="openCreateToken"
        >
          Create Token
        </button>
      </div>

      <BaseSpinner
        v-if="tokensLoading"
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
            <td class="cell-name">
              {{ token.name }}
            </td>
            <td class="cell-date">
              {{ formatDate(token.created_at) }}
            </td>
            <td class="cell-date">
              {{ formatDate(token.last_used_at, 'Never') }}
            </td>
            <td>
              <button
                class="btn btn-sm btn-ghost btn-danger-text"
                title="Delete"
                @click="openDeleteToken(token)"
              >
                <Trash2 :size="14" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>

      <div
        v-else
        class="empty-state"
      >
        No API tokens yet.
      </div>
    </div>

    <!-- Appearance Tab -->
    <div
      v-if="activeTab === 'appearance'"
      class="tab-content"
    >
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">Theme</span>
          <span class="setting-desc">Choose your preferred theme or follow system settings</span>
        </div>
        <div class="theme-options">
          <button
            class="theme-option"
            :class="{ active: theme === 'auto' }"
            @click="setTheme('auto')"
          >
            <span class="theme-icon">&#9881;</span>
            Auto
          </button>
          <button
            class="theme-option"
            :class="{ active: theme === 'light' }"
            @click="setTheme('light')"
          >
            <span class="theme-icon">&#9788;</span>
            Light
          </button>
          <button
            class="theme-option"
            :class="{ active: theme === 'dark' }"
            @click="setTheme('dark')"
          >
            <span class="theme-icon">&#9789;</span>
            Dark
          </button>
        </div>
      </div>
    </div>

    <!-- Create Token Modal -->
    <Teleport to="body">
      <div
        v-if="showCreateModal"
        class="overlay"
        @click.self="closeCreateModal"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ newTokenPlaintext ? 'Token Created' : 'Create API Token' }}
            </h2>
            <button
              class="close-btn"
              @click="closeCreateModal"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <template v-if="!newTokenPlaintext">
              <div class="form-group">
                <label class="form-label">Token Name</label>
                <input
                  v-model="createName"
                  class="form-input"
                  placeholder="e.g. CI pipeline"
                  :disabled="createSubmitting"
                  @keydown.enter.prevent="submitCreateToken"
                />
              </div>
              <div
                v-if="createError"
                class="msg msg-error"
              >
                {{ createError }}
              </div>
            </template>
            <template v-else>
              <p class="token-warning">Copy this token now. It will not be shown again.</p>
              <div class="token-display">
                <code class="token-value">{{ newTokenPlaintext }}</code>
                <button
                  class="btn btn-sm btn-ghost"
                  @click="copyToClipboard(newTokenPlaintext)"
                >
                  {{ tokenCopied ? 'Copied' : 'Copy' }}
                </button>
              </div>
            </template>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="closeCreateModal"
            >
              {{ newTokenPlaintext ? 'Done' : 'Cancel' }}
            </button>
            <button
              v-if="!newTokenPlaintext"
              class="btn btn-primary"
              :disabled="createSubmitting || !createName.trim()"
              @click="submitCreateToken"
            >
              {{ createSubmitting ? 'Creating...' : 'Create' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Delete Token Modal -->
    <Teleport to="body">
      <div
        v-if="showDeleteModal"
        class="overlay"
        @click.self="showDeleteModal = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Token</h2>
            <button
              class="close-btn"
              @click="showDeleteModal = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Delete token <strong>{{ deleteTarget?.name }}</strong
              >? Any integrations using this token will stop working.
            </p>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showDeleteModal = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="deleteSubmitting"
              @click="confirmDeleteToken"
            >
              {{ deleteSubmitting ? 'Deleting...' : 'Delete' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.profile-view {
  max-width: 700px;
}

.page-subtitle {
  color: var(--text-muted);
  font-size: 0.9rem;
  margin-bottom: 1.5rem;
}

.tabs {
  display: flex;
  gap: 0;
  border-bottom: 1px solid var(--border);
  margin-bottom: 1.5rem;
}

.tab {
  padding: 0.6rem 1.25rem;
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-muted);
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
  transition:
    color 0.15s,
    border-color 0.15s;
}

.tab:hover {
  color: var(--text-primary);
}

.tab.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

.tab-content {
  animation: fadeIn 0.15s ease;
}

@keyframes fadeIn {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

.password-form {
  max-width: 380px;
}

.form-group {
  margin-bottom: 1rem;
}

.form-label {
  display: block;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 0.35rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.form-input {
  width: 100%;
  padding: 0.55rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  box-sizing: border-box;
  transition: border-color 0.15s;
}

.form-input:focus {
  outline: none;
  border-color: var(--accent);
}

.form-input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.msg {
  padding: 0.6rem 0.875rem;
  border-radius: var(--radius-sm);
  font-size: 0.85rem;
  margin-bottom: 1rem;
}

.msg-error {
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  color: var(--danger);
}

.msg-success {
  background: var(--success-subtle);
  border: 1px solid var(--success);
  color: var(--success);
}

.tokens-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1rem;
}

.tokens-desc {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin: 0;
}

.tokens-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.tokens-table th {
  text-align: left;
  padding: 0.7rem 1rem;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
}

.tokens-table td {
  padding: 0.65rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.tokens-table tr:last-child td {
  border-bottom: none;
}

.tokens-table tr:hover td {
  background: var(--bg-hover);
}

.cell-name {
  font-weight: 600;
  color: var(--text-primary);
}

.cell-date {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.loading {
  color: var(--text-muted);
  padding: 2rem;
  text-align: center;
}

.empty-state {
  color: var(--text-muted);
  padding: 2rem;
  text-align: center;
  font-size: 0.9rem;
}

.token-warning {
  color: var(--warning);
  font-size: 0.85rem;
  font-weight: 600;
  margin-bottom: 0.75rem;
}

.token-display {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.token-value {
  flex: 1;
  font-size: 0.8rem;
  font-family: var(--mono);
  word-break: break-all;
  color: var(--text-primary);
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 2rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem 1.5rem;
}

.setting-info {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.setting-label {
  font-weight: 600;
  font-size: 0.9rem;
}

.setting-desc {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.theme-options {
  display: flex;
  gap: 0.5rem;
}

.theme-option {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.45rem 1rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-secondary);
  font-size: 0.85rem;
  cursor: pointer;
  transition:
    border-color 0.15s,
    color 0.15s,
    background 0.15s;
}

.theme-option:hover {
  border-color: var(--text-muted);
  color: var(--text-primary);
}

.theme-option.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--bg-hover);
}

.theme-icon {
  font-size: 1rem;
}
</style>
