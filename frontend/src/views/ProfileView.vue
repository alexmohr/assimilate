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

type TabId = 'password' | 'tokens' | 'totp' | 'sessions' | 'appearance'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

interface SessionInfo {
  id: string
  user_id: number
  created_at: string
  expires_at: string
  last_seen_at: string
  remember_me: boolean
  current: boolean
}

const authStore = useAuthStore()
const { theme, setTheme, loadFromBackend } = useTheme()
const activeTab = ref<TabId>('password')

const newPassword = ref('')
const confirmPassword = ref('')
const passwordError = ref('')
const passwordSuccess = ref('')
const passwordSubmitting = ref(false)

// TOTP setup state
const totpSetupData = ref<{ secret: string; qr_uri: string; recovery_codes: string[] } | null>(null)
const totpLoading = ref(false)
const totpError = ref('')
const totpVerifyCode = ref('')
const totpVerifying = ref(false)
const totpVerifyError = ref('')
const totpEnabled = ref(false)
const totpDisablePassword = ref('')
const totpDisableError = ref('')
const totpDisabling = ref(false)
const totpRecoveryCodes = ref<string[]>([])
const totpShowRecoveryCodes = ref(false)

// Sessions state
const sessions = ref<SessionInfo[]>([])
const sessionsLoading = ref(true)
const revokeSessionId = ref<string | null>(null)
const revokeSubmitting = ref(false)
const sessionsError = ref('')

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

// TOTP functions
async function setupTotp(): Promise<void> {
  totpLoading.value = true
  totpError.value = ''
  try {
    const res = await apiClient.post<{
      secret: string
      qr_uri: string
      recovery_codes: string[]
    }>('/auth/totp/setup')
    totpSetupData.value = res.data
    totpRecoveryCodes.value = res.data.recovery_codes
  } catch (e: unknown) {
    totpError.value = extractError(e, 'Failed to set up TOTP')
  } finally {
    totpLoading.value = false
  }
}

async function verifyTotpSetup(): Promise<void> {
  totpVerifying.value = true
  totpVerifyError.value = ''
  try {
    await apiClient.post('/auth/totp/verify', { code: totpVerifyCode.value })
    totpSetupData.value = null
    totpVerifyCode.value = ''
    totpEnabled.value = true
    totpShowRecoveryCodes.value = true
    await authStore.fetchMe()
  } catch (e: unknown) {
    totpVerifyError.value = extractError(e, 'Invalid code')
  } finally {
    totpVerifying.value = false
  }
}

async function disableTotp(): Promise<void> {
  totpDisabling.value = true
  totpDisableError.value = ''
  try {
    await apiClient.post('/auth/totp/disable', { password: totpDisablePassword.value })
    totpEnabled.value = false
    totpSetupData.value = null
    totpDisablePassword.value = ''
    totpShowRecoveryCodes.value = false
    await authStore.fetchMe()
  } catch (e: unknown) {
    totpDisableError.value = extractError(e, 'Failed to disable TOTP')
  } finally {
    totpDisabling.value = false
  }
}

function cancelTotpSetup(): void {
  totpSetupData.value = null
  totpVerifyCode.value = ''
  totpVerifyError.value = ''
}

// Sessions functions
async function fetchSessions(): Promise<void> {
  sessionsLoading.value = true
  sessionsError.value = ''
  try {
    const res = await apiClient.get<{ sessions: SessionInfo[] }>('/auth/sessions')
    sessions.value = res.data.sessions
  } catch (e: unknown) {
    sessionsError.value = extractError(e, 'Failed to load sessions')
  } finally {
    sessionsLoading.value = false
  }
}

async function confirmRevokeSession(sessionId: string): Promise<void> {
  revokeSessionId.value = sessionId
}

async function doRevokeSession(): Promise<void> {
  if (!revokeSessionId.value) return
  revokeSubmitting.value = true
  try {
    await apiClient.delete(`/auth/sessions/${revokeSessionId.value}`)
    revokeSessionId.value = null
    await fetchSessions()
  } finally {
    revokeSubmitting.value = false
  }
}

function cancelRevokeSession(): void {
  revokeSessionId.value = null
}

onMounted(() => {
  fetchTokens()
  loadFromBackend()
  authStore.fetchMe().then(() => {
    totpEnabled.value = authStore.user?.totp_enabled ?? false
  })
})

onMounted(() => {
  if (activeTab.value === 'sessions') {
    fetchSessions()
  }
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
        :class="{ active: activeTab === 'totp' }"
        @click="activeTab = 'totp'"
      >
        Two-Factor Auth
      </button>
      <button
        class="tab"
        :class="{ active: activeTab === 'sessions' }"
        @click="
          activeTab = 'sessions'
          fetchSessions()
        "
      >
        Sessions
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

    <!-- Two-Factor Auth Tab -->
    <div
      v-if="activeTab === 'totp'"
      class="tab-content"
    >
      <div v-if="totpShowRecoveryCodes && !totpEnabled">
        <div class="recovery-codes-section">
          <h3 class="section-title">Recovery Codes</h3>
          <p class="recovery-codes-warning">
            Save these recovery codes in a secure place. They can be used to access your account if
            you lose your authenticator device. Each code can only be used once.
          </p>
          <div class="recovery-codes-list">
            <code
              v-for="(code, i) in totpRecoveryCodes"
              :key="i"
              class="recovery-code"
              >{{ code }}</code
            >
          </div>
          <button
            class="btn btn-primary"
            @click="totpShowRecoveryCodes = false"
          >
            I have saved these codes
          </button>
        </div>
      </div>

      <div v-else-if="totpSetupData">
        <div class="totp-setup-section">
          <h3 class="section-title">Set Up Two-Factor Authentication</h3>
          <p class="totp-setup-desc">
            Scan the QR code below with your authenticator app (e.g., Google Authenticator, Authy).
          </p>
          <div class="qr-container">
            <img
              :src="totpSetupData.qr_uri"
              alt="TOTP QR Code"
              class="qr-code"
            />
          </div>
          <p class="totp-secret-text">
            Or enter this key manually:
            <code class="totp-secret">{{ totpSetupData.secret }}</code>
          </p>

          <div class="form-group">
            <label class="form-label">Verify the code from your authenticator app</label>
            <input
              v-model="totpVerifyCode"
              type="text"
              inputmode="numeric"
              maxlength="6"
              placeholder="000000"
              class="form-input"
              :disabled="totpVerifying"
            />
          </div>
          <div
            v-if="totpVerifyError"
            class="msg msg-error"
          >
            {{ totpVerifyError }}
          </div>
          <div class="totp-actions">
            <button
              class="btn btn-primary"
              :disabled="totpVerifying || totpVerifyCode.length !== 6"
              @click="verifyTotpSetup"
            >
              {{ totpVerifying ? 'Verifying...' : 'Verify & Enable' }}
            </button>
            <button
              class="btn btn-ghost"
              @click="cancelTotpSetup"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>

      <div v-else>
        <div
          v-if="totpEnabled"
          class="totp-status-section"
        >
          <div class="totp-status-badge totp-enabled">Two-factor authentication is enabled</div>

          <div class="form-group">
            <label class="form-label">Enter your password to disable 2FA</label>
            <input
              v-model="totpDisablePassword"
              type="password"
              autocomplete="current-password"
              class="form-input"
              placeholder="Current password"
              :disabled="totpDisabling"
            />
          </div>
          <div
            v-if="totpDisableError"
            class="msg msg-error"
          >
            {{ totpDisableError }}
          </div>
          <button
            class="btn btn-danger"
            :disabled="totpDisabling || !totpDisablePassword"
            @click="disableTotp"
          >
            {{ totpDisabling ? 'Disabling...' : 'Disable Two-Factor Auth' }}
          </button>
        </div>

        <div
          v-else
          class="totp-status-section"
        >
          <div class="totp-status-badge totp-disabled">
            Two-factor authentication is not enabled
          </div>
          <p class="totp-desc">
            Two-factor authentication adds an extra layer of security by requiring a code from your
            authenticator app in addition to your password when signing in.
          </p>
          <button
            class="btn btn-primary"
            :disabled="totpLoading"
            @click="setupTotp"
          >
            {{ totpLoading ? 'Setting up...' : 'Set Up Two-Factor Auth' }}
          </button>
          <div
            v-if="totpError"
            class="msg msg-error"
          >
            {{ totpError }}
          </div>
        </div>
      </div>
    </div>

    <!-- Sessions Tab -->
    <div
      v-if="activeTab === 'sessions'"
      class="tab-content"
    >
      <p class="sessions-desc">
        Active sessions for your account. You can revoke any session except your current one.
      </p>

      <BaseSpinner
        v-if="sessionsLoading"
        size="lg"
      />

      <div
        v-else-if="sessionsError"
        class="msg msg-error"
      >
        {{ sessionsError }}
      </div>

      <table
        v-else-if="sessions.length"
        class="sessions-table"
      >
        <thead>
          <tr>
            <th>Created</th>
            <th>Last Active</th>
            <th>Expires</th>
            <th>Type</th>
            <th>Status</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="session in sessions"
            :key="session.id"
          >
            <td class="cell-date">
              {{ formatDate(session.created_at) }}
            </td>
            <td class="cell-date">
              {{ formatDate(session.last_seen_at) }}
            </td>
            <td class="cell-date">
              {{ formatDate(session.expires_at) }}
            </td>
            <td class="cell-type">
              {{ session.remember_me ? 'Remember Me' : 'Session' }}
            </td>
            <td>
              <span
                v-if="session.current"
                class="badge badge-current"
                >Current</span
              >
              <span
                v-else
                class="badge badge-other"
                >Active</span
              >
            </td>
            <td>
              <button
                v-if="!session.current"
                class="btn btn-sm btn-ghost btn-danger-text"
                title="Revoke session"
                @click="confirmRevokeSession(session.id)"
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
        No active sessions.
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

    <!-- Revoke Session Modal -->
    <Teleport to="body">
      <div
        v-if="revokeSessionId"
        class="overlay"
        @click.self="cancelRevokeSession"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Revoke Session</h2>
            <button
              class="close-btn"
              @click="cancelRevokeSession"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>Revoke this session? The device will be signed out immediately.</p>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="cancelRevokeSession"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="revokeSubmitting"
              @click="doRevokeSession"
            >
              {{ revokeSubmitting ? 'Revoking...' : 'Revoke' }}
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
  flex-wrap: wrap;
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

/* TOTP Styles */
.totp-status-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.totp-status-badge {
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  font-weight: 600;
  font-size: 0.9rem;
}

.totp-enabled {
  background: var(--success-subtle);
  border: 1px solid var(--success);
  color: var(--success);
}

.totp-disabled {
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-muted);
}

.totp-desc {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin: 0;
}

.totp-setup-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.section-title {
  font-size: 1rem;
  font-weight: 600;
  margin: 0;
}

.totp-setup-desc {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin: 0;
}

.qr-container {
  display: flex;
  justify-content: center;
  padding: 1rem;
  background: white;
  border-radius: var(--radius);
  border: 1px solid var(--border);
}

.qr-code {
  width: 200px;
  height: 200px;
  image-rendering: pixelated;
}

.totp-secret-text {
  font-size: 0.8rem;
  color: var(--text-muted);
  text-align: center;
}

.totp-secret {
  font-family: var(--mono);
  font-size: 0.75rem;
  word-break: break-all;
}

.totp-actions {
  display: flex;
  gap: 0.75rem;
}

.recovery-codes-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.recovery-codes-warning {
  color: var(--warning);
  font-size: 0.85rem;
  font-weight: 500;
  margin: 0;
}

.recovery-codes-list {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.5rem;
  padding: 1rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.recovery-code {
  font-family: var(--mono);
  font-size: 0.8rem;
  color: var(--text-primary);
  padding: 0.25rem 0.5rem;
  background: var(--bg-card);
  border-radius: var(--radius-sm);
}

/* Sessions Styles */
.sessions-desc {
  color: var(--text-muted);
  font-size: 0.85rem;
  margin-bottom: 1rem;
}

.sessions-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.sessions-table th {
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

.sessions-table td {
  padding: 0.65rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.sessions-table tr:last-child td {
  border-bottom: none;
}

.sessions-table tr:hover td {
  background: var(--bg-hover);
}

.cell-type {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
}

.badge-current {
  background: var(--accent-subtle, rgba(59, 130, 246, 0.1));
  color: var(--accent);
}

.badge-other {
  background: var(--bg-card);
  color: var(--text-muted);
  border: 1px solid var(--border);
}
</style>
