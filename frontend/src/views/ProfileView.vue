<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useTheme } from '../composables/useTheme'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useApiTokens } from '../composables/useApiTokens'
import { formatDate } from '../utils/format'
import { validatePassword } from '../utils/validation'
import { extractError } from '../utils/error'
import { Trash2, Monitor, Sun, Moon } from '@lucide/vue'
import ApiTokenTable from '../components/ApiTokenTable.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import type {
  SessionListResponse,
  SessionResponse,
  TotpSetupResponse,
  TotpVerifyResponse,
} from '../types/generated'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

type TabId = 'password' | 'tokens' | 'totp' | 'sessions' | 'appearance'

const authStore = useAuthStore()
const { theme, setTheme, loadFromBackend } = useTheme()
const tabs: TabOption<TabId>[] = [
  { id: 'password', label: 'Change Password' },
  { id: 'tokens', label: 'API Tokens' },
  { id: 'totp', label: 'Two-Factor Auth' },
  { id: 'sessions', label: 'Sessions' },
  { id: 'appearance', label: 'Appearance' },
]

const activeTab = ref<TabId>('password')

const newPassword = ref('')
const confirmPassword = ref('')
const passwordError = ref('')
const passwordSuccess = ref('')
const passwordSubmitting = ref(false)

const {
  tokens,
  loading: tokensLoading,
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
  openCreate: openCreateToken,
  submitCreate: submitCreateToken,
  closeCreateModal,
  openDelete: openDeleteToken,
  confirmDelete: confirmDeleteToken,
} = useApiTokens()

// TOTP setup state
const totpSetupData = ref<TotpSetupResponse | null>(null)
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
const sessions = ref<SessionResponse[]>([])
const sessionsLoading = ref(true)
const revokeSessionId = ref<string | null>(null)
const revokeSubmitting = ref(false)
const revokeError = ref('')
const sessionsError = ref('')

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

// TOTP functions
async function setupTotp(): Promise<void> {
  totpLoading.value = true
  totpError.value = ''
  try {
    const res = await apiClient.post<TotpSetupResponse>('/auth/totp/setup')
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
    await apiClient.post<TotpVerifyResponse>('/auth/totp/verify', { code: totpVerifyCode.value })
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
    await apiClient.post<TotpVerifyResponse>('/auth/totp/disable', {
      password: totpDisablePassword.value,
    })
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
    const res = await apiClient.get<SessionListResponse>('/auth/sessions')
    sessions.value = res.data.sessions
  } catch (e: unknown) {
    sessionsError.value = extractError(e, 'Failed to load sessions')
  } finally {
    sessionsLoading.value = false
  }
}

async function confirmRevokeSession(sessionId: string): Promise<void> {
  revokeSessionId.value = sessionId
  revokeError.value = ''
}

async function doRevokeSession(): Promise<void> {
  if (!revokeSessionId.value) return
  revokeSubmitting.value = true
  revokeError.value = ''
  try {
    await apiClient.delete(`/auth/sessions/${revokeSessionId.value}`)
    revokeSessionId.value = null
    await fetchSessions()
  } catch (e: unknown) {
    revokeError.value = extractError(e, 'Failed to revoke session')
  } finally {
    revokeSubmitting.value = false
  }
}

function cancelRevokeSession(): void {
  revokeSessionId.value = null
  revokeError.value = ''
}

watch(activeTab, (tab: TabId) => {
  if (tab === 'sessions') fetchSessions()
})

onMounted(async () => {
  fetchTokens()
  loadFromBackend()
  await authStore.fetchMe()
  totpEnabled.value = authStore.user?.totp_enabled ?? false
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

    <BaseTabs
      v-model="activeTab"
      :tabs="tabs"
      label="Profile sections"
    />

    <!-- Password Tab -->
    <div
      v-if="activeTab === 'password'"
      class="tab-content"
    >
      <form
        class="password-form"
        @submit.prevent="handlePasswordSubmit"
      >
        <div class="field">
          <label
            class="field-label"
            for="profile-new-pw"
            >New Password</label
          >
          <input
            id="profile-new-pw"
            v-model="newPassword"
            type="password"
            class="input"
            autocomplete="new-password"
            placeholder="Minimum 8 characters"
            :disabled="passwordSubmitting"
          />
        </div>

        <div class="field">
          <label
            class="field-label"
            for="profile-confirm-pw"
            >Confirm Password</label
          >
          <input
            id="profile-confirm-pw"
            v-model="confirmPassword"
            type="password"
            class="input"
            autocomplete="new-password"
            :disabled="passwordSubmitting"
          />
        </div>

        <div
          v-if="passwordError"
          class="form-error"
        >
          {{ passwordError }}
        </div>
        <div
          v-if="passwordSuccess"
          class="form-success"
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

      <ApiTokenTable
        v-else-if="tokens.length"
        :tokens="tokens"
        @delete="openDeleteToken"
      />

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
      <div v-if="totpShowRecoveryCodes">
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

          <div class="field">
            <label class="field-label">Verify the code from your authenticator app</label>
            <input
              v-model="totpVerifyCode"
              type="text"
              inputmode="numeric"
              maxlength="6"
              placeholder="000000"
              class="input"
              :disabled="totpVerifying"
            />
          </div>
          <div
            v-if="totpVerifyError"
            class="form-error"
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

          <div class="field">
            <label class="field-label">Enter your password to disable 2FA</label>
            <input
              v-model="totpDisablePassword"
              type="password"
              autocomplete="current-password"
              class="input"
              placeholder="Current password"
              :disabled="totpDisabling"
            />
          </div>
          <div
            v-if="totpDisableError"
            class="form-error"
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
            class="form-error"
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
        class="form-error"
      >
        {{ sessionsError }}
      </div>

      <table
        v-else-if="sessions.length"
        class="data-table"
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
                class="badge badge--success"
                >Current</span
              >
              <span
                v-else
                class="badge badge--neutral"
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
            <Monitor
              class="theme-icon"
              :size="16"
            />
            Auto
          </button>
          <button
            class="theme-option"
            :class="{ active: theme === 'light' }"
            @click="setTheme('light')"
          >
            <Sun
              class="theme-icon"
              :size="16"
            />
            Light
          </button>
          <button
            class="theme-option"
            :class="{ active: theme === 'dark' }"
            @click="setTheme('dark')"
          >
            <Moon
              class="theme-icon"
              :size="16"
            />
            Dark
          </button>
        </div>
      </div>
    </div>

    <!-- Create Token Modal -->
    <BaseModal
      :open="showCreateModal"
      :title="newTokenPlaintext ? 'Token Created' : 'Create API Token'"
      @close="closeCreateModal"
    >
      <template v-if="!newTokenPlaintext">
        <div class="field">
          <label class="field-label">Token Name</label>
          <input
            v-model="createName"
            class="input"
            placeholder="e.g. CI pipeline"
            :disabled="createSubmitting"
            @keydown.enter.prevent="submitCreateToken"
          />
        </div>
        <div
          v-if="createError"
          class="form-error"
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

      <template #footer>
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
      </template>
    </BaseModal>

    <!-- Delete Token Modal -->
    <BaseModal
      :open="showDeleteModal"
      title="Delete Token"
      @close="showDeleteModal = false"
    >
      <p>
        Delete token <strong>{{ deleteTarget?.name }}</strong
        >? Any integrations using this token will stop working.
      </p>
      <div
        v-if="deleteError"
        class="form-error"
      >
        {{ deleteError }}
      </div>

      <template #footer>
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
      </template>
    </BaseModal>

    <!-- Revoke Session Modal -->
    <BaseModal
      :open="revokeSessionId !== null"
      title="Revoke Session"
      @close="cancelRevokeSession"
    >
      <p>Revoke this session? The device will be signed out immediately.</p>
      <div
        v-if="revokeError"
        class="form-error"
      >
        {{ revokeError }}
      </div>

      <template #footer>
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
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.profile-view {
  max-width: 700px;
}

.page-subtitle {
  color: var(--text-muted);
  font-size: var(--fs-md);
  margin-bottom: 1.5rem;
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

.tokens-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1rem;
}

.tokens-desc {
  color: var(--text-muted);
  font-size: var(--fs-base);
  margin: 0;
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
  font-size: var(--fs-md);
}

.token-warning {
  color: var(--warning);
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
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
  font-size: var(--fs-md);
}

.setting-desc {
  font-size: var(--fs-sm);
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
  font-size: var(--fs-base);
  cursor: pointer;
  transition:
    border-color var(--duration-base),
    color var(--duration-base),
    background var(--duration-base);
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
  font-size: var(--fs-lg);
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
  font-size: var(--fs-md);
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
  font-size: var(--fs-base);
  margin: 0;
}

.totp-setup-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.section-title {
  font-size: var(--fs-lg);
  font-weight: 600;
  margin: 0;
}

.totp-setup-desc {
  color: var(--text-muted);
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
  color: var(--text-muted);
  text-align: center;
}

.totp-secret {
  font-family: var(--mono);
  font-size: var(--fs-xs);
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
  color: var(--text-primary);
  padding: 0.25rem 0.5rem;
  background: var(--bg-card);
  border-radius: var(--radius-sm);
}

/* Sessions Styles */
.sessions-desc {
  color: var(--text-muted);
  font-size: var(--fs-base);
  margin-bottom: 1rem;
}

.data-table tr:last-child td {
  border-bottom: none;
}

.data-table tr:hover td {
  background: var(--bg-hover);
}

.cell-type {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}
</style>
