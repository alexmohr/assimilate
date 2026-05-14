<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useClipboard } from '../composables/useClipboard'
import { useTimezone } from '../composables/useTimezone'
import { extractError } from '../utils/error'
import BaseSpinner from '../components/BaseSpinner.vue'
import TimezoneSelect from '../components/TimezoneSelect.vue'

interface SettingsResponse {
  retention_days: number
  timezone: string
}

const publicKey = ref('')
const loading = ref(true)
const error = ref('')
const { copied, copy: copyToClipboard } = useClipboard()
const { setTimezone } = useTimezone()
const regenerating = ref(false)
const showRegenConfirm = ref(false)
const regenError = ref('')

const settingsLoading = ref(true)
const settingsError = ref('')
const settingsSaving = ref(false)
const settingsSaved = ref(false)
const settingsForm = reactive({ timezone: '', retention_days: 7 })

onMounted(async () => {
  try {
    const res = await apiClient.get<{ public_key: string }>('/system/ssh-public-key')
    publicKey.value = res.data.public_key
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load SSH public key')
  } finally {
    loading.value = false
  }

  try {
    const res = await apiClient.get<SettingsResponse>('/system/settings')
    settingsForm.timezone = res.data.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone
    settingsForm.retention_days = res.data.retention_days
  } catch (e: unknown) {
    settingsError.value = extractError(e, 'Failed to load settings')
  } finally {
    settingsLoading.value = false
  }
})

async function regenerateKey(): Promise<void> {
  regenerating.value = true
  regenError.value = ''
  try {
    const res = await apiClient.post<{ public_key: string }>('/system/ssh-regenerate-key')
    publicKey.value = res.data.public_key
    showRegenConfirm.value = false
  } catch (e: unknown) {
    regenError.value = extractError(e, 'Failed to regenerate key')
  } finally {
    regenerating.value = false
  }
}

async function saveSettings(): Promise<void> {
  settingsSaving.value = true
  settingsError.value = ''
  settingsSaved.value = false
  try {
    const res = await apiClient.put<SettingsResponse>('/system/settings', {
      retention_days: settingsForm.retention_days,
      timezone: settingsForm.timezone || undefined,
    })
    settingsForm.timezone = res.data.timezone
    settingsForm.retention_days = res.data.retention_days
    setTimezone(res.data.timezone || undefined)
    settingsSaved.value = true
    setTimeout(() => {
      settingsSaved.value = false
    }, 2000)
  } catch (e: unknown) {
    settingsError.value = extractError(e, 'Failed to save settings')
  } finally {
    settingsSaving.value = false
  }
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">System</h1>
    </div>

    <div class="info-card">
      <div class="card-header">
        <h3 class="info-title">SSH Public Key</h3>
        <button
          class="btn btn-sm btn-ghost btn-danger-text"
          @click="showRegenConfirm = true"
        >
          Regenerate
        </button>
      </div>
      <p class="info-description">
        Add this key to <code>~/.ssh/authorized_keys</code> on your borg repository host.
      </p>

      <BaseSpinner
        v-if="loading"
        size="lg"
      />
      <div
        v-else-if="error"
        class="state-msg error"
      >
        {{ error }}
      </div>
      <div
        v-else
        class="key-box"
      >
        <pre class="key-text">{{ publicKey }}</pre>
        <button
          class="btn btn-sm btn-ghost"
          @click="copyToClipboard(publicKey)"
        >
          {{ copied ? 'Copied!' : 'Copy' }}
        </button>
      </div>
    </div>

    <div class="info-card settings-card">
      <div class="card-header">
        <h3 class="info-title">Settings</h3>
      </div>

      <BaseSpinner
        v-if="settingsLoading"
        size="lg"
      />
      <template v-else>
        <div
          v-if="settingsError"
          class="state-msg error"
        >
          {{ settingsError }}
        </div>

        <div class="settings-form">
          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-timezone"
            >
              Timezone
            </label>
            <div class="setting-input-group">
              <TimezoneSelect
                id="settings-timezone"
                v-model="settingsForm.timezone"
                placeholder="e.g. Europe/Berlin"
              />
              <span class="field-hint"
                >IANA timezone for schedule evaluation and time display. Leave empty to use the
                server's local timezone.</span
              >
            </div>
          </div>

          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-retention"
            >
              Retention Days
            </label>
            <div class="setting-input-group">
              <input
                id="settings-retention"
                v-model.number="settingsForm.retention_days"
                type="number"
                min="0"
                class="form-input retention-input"
              />
              <span class="field-hint">Number of days to keep backup job history.</span>
            </div>
          </div>

          <div class="settings-actions">
            <button
              class="btn btn-primary"
              :disabled="settingsSaving"
              @click="saveSettings"
            >
              {{ settingsSaving ? 'Saving...' : 'Save' }}
            </button>
            <span
              v-if="settingsSaved"
              class="save-success"
            >
              Settings saved
            </span>
          </div>
        </div>
      </template>
    </div>

    <!-- Regenerate Confirmation -->
    <Teleport to="body">
      <div
        v-if="showRegenConfirm"
        class="overlay"
        @click.self="showRegenConfirm = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Regenerate SSH Key</h2>
            <button
              class="close-btn"
              @click="showRegenConfirm = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="warning-text">
              This will generate a new SSH keypair and invalidate the current key. All borg
              repository hosts will need to be updated with the new public key.
            </p>
            <p class="warning-text warning-bold">
              Existing SSH connections using the old key will stop working immediately.
            </p>
            <div
              v-if="regenError"
              class="form-error"
            >
              {{ regenError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showRegenConfirm = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="regenerating"
              @click="regenerateKey"
            >
              {{ regenerating ? 'Regenerating...' : 'Regenerate Key' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.page {
  max-width: 800px;
}

.info-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;

  & + & {
    margin-top: 0.75rem;
  }
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.info-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
}

.info-description {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin-bottom: 1rem;
}

.info-description code {
  font-family: var(--font-mono);
  font-size: 0.8125rem;
  background: var(--bg-hover);
  padding: 0.125rem 0.375rem;
  border-radius: var(--radius-sm);
}

.state-msg {
  font-size: 0.875rem;
  color: var(--text-muted);
}

.state-msg.error {
  color: var(--danger);
}

.key-box {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 1rem;
}

.key-text {
  flex: 1;
  font-family: var(--font-mono);
  font-size: 0.75rem;
  line-height: 1.5;
  color: var(--text-primary);
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0;
}

.warning-text {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin-bottom: 0.75rem;
}

.warning-bold {
  font-weight: 600;
  color: var(--danger);
}

.settings-card {
  margin-top: 1.5rem;
}

.settings-form {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.setting-row {
  display: flex;
  align-items: flex-start;
  gap: 1rem;
}

.setting-label {
  flex-shrink: 0;
  width: 120px;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
  padding-top: 0.5rem;
}

.setting-input-group {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.setting-input-group .form-input {
  width: 100%;
  max-width: 300px;
}

.retention-input {
  max-width: 120px !important;
}

.field-hint {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.settings-actions {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding-top: 0.5rem;
}

.save-success {
  font-size: 0.875rem;
  color: var(--success);
  font-weight: 500;
}
</style>
