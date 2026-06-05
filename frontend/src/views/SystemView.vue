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

interface ImportResult {
  hosts_created: number
  hosts_updated: number
  schedules_created: number
  warnings: string[]
}

interface SettingsResponse {
  retention_days: number
  timezone: string
}

interface VersionInfo {
  server_version: string
  server_git_sha: string
  build_timestamp: string
  agent_version: string | null
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

const versionInfo = ref<VersionInfo | null>(null)
const versionLoading = ref(true)
const versionError = ref('')

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

  try {
    const res = await apiClient.get<VersionInfo>('/system/version')
    versionInfo.value = res.data
  } catch (e: unknown) {
    versionError.value = extractError(e, 'Failed to load version info')
  } finally {
    versionLoading.value = false
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

const exporting = ref(false)
const exportError = ref('')

const importing = ref(false)
const importError = ref('')
const importResult = ref<ImportResult | null>(null)
const importFileInput = ref<HTMLInputElement | null>(null)
const importFileName = ref('')

async function exportConfig(): Promise<void> {
  exporting.value = true
  exportError.value = ''
  try {
    const res = await apiClient.get<unknown>('/config/export')
    const blob = new Blob([JSON.stringify(res.data, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    const date = new Date().toISOString().slice(0, 10)
    a.download = `assimilate-config-${date}.json`
    a.click()
    URL.revokeObjectURL(url)
  } catch (e: unknown) {
    exportError.value = extractError(e, 'Export failed')
  } finally {
    exporting.value = false
  }
}

function onImportFileChange(event: Event): void {
  const target = event.target as HTMLInputElement
  importFileName.value = target.files?.[0]?.name ?? ''
  importResult.value = null
  importError.value = ''
}

async function importConfig(): Promise<void> {
  const file = importFileInput.value?.files?.[0]
  if (!file) {
    importError.value = 'Please select a file'
    return
  }
  importing.value = true
  importError.value = ''
  importResult.value = null
  try {
    const text = await file.text()
    const payload: unknown = JSON.parse(text)
    const res = await apiClient.post<ImportResult>('/config/import', payload)
    importResult.value = res.data
    if (importFileInput.value) {
      importFileInput.value.value = ''
    }
    importFileName.value = ''
  } catch (e: unknown) {
    importError.value = extractError(e, 'Import failed')
  } finally {
    importing.value = false
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
        <h3 class="info-title">Version</h3>
      </div>

      <BaseSpinner
        v-if="versionLoading"
        size="lg"
      />
      <div
        v-else-if="versionError"
        class="state-msg error"
      >
        {{ versionError }}
      </div>
      <div
        v-else-if="versionInfo"
        class="version-grid"
      >
        <div class="version-row">
          <span class="version-label">Server</span>
          <span class="version-value mono">{{ versionInfo.server_version }}</span>
        </div>
        <div class="version-row">
          <span class="version-label">Built</span>
          <span class="version-value mono">{{ versionInfo.build_timestamp }}</span>
        </div>
        <div
          v-if="versionInfo.agent_version"
          class="version-row"
        >
          <span class="version-label">Agent</span>
          <span class="version-value mono">{{ versionInfo.agent_version }}</span>
        </div>
      </div>
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

    <div class="info-card">
      <div class="card-header">
        <h3 class="info-title">Configuration Export / Import</h3>
      </div>
      <p class="info-description">
        Export host and schedule configuration as JSON for backup or migration. Importing restores
        hosts and schedules by name; repositories must exist before importing.
      </p>

      <div class="config-io-section">
        <div class="config-io-row">
          <div class="config-io-label">Export</div>
          <div class="config-io-controls">
            <button
              class="btn btn-sm btn-ghost"
              :disabled="exporting"
              @click="exportConfig"
            >
              {{ exporting ? 'Exporting...' : 'Download JSON' }}
            </button>
            <span
              v-if="exportError"
              class="config-io-error"
            >
              {{ exportError }}
            </span>
          </div>
        </div>

        <div class="config-io-row">
          <div class="config-io-label">Import</div>
          <div class="config-io-controls">
            <label class="file-label">
              <input
                ref="importFileInput"
                type="file"
                accept=".json,application/json"
                class="file-input-hidden"
                @change="onImportFileChange"
              />
              <span class="btn btn-sm btn-ghost">Choose File</span>
              <span
                v-if="importFileName"
                class="file-name"
              >
                {{ importFileName }}
              </span>
              <span
                v-else
                class="file-name muted"
              >
                No file chosen
              </span>
            </label>
            <button
              class="btn btn-sm btn-primary"
              :disabled="importing || !importFileName"
              @click="importConfig"
            >
              {{ importing ? 'Importing...' : 'Import' }}
            </button>
          </div>
        </div>

        <div
          v-if="importError"
          class="config-io-error"
        >
          {{ importError }}
        </div>

        <div
          v-if="importResult"
          class="import-result"
        >
          <div class="import-stats">
            <span>Hosts created: {{ importResult.hosts_created }}</span>
            <span>Hosts updated: {{ importResult.hosts_updated }}</span>
            <span>Schedules created: {{ importResult.schedules_created }}</span>
          </div>
          <ul
            v-if="importResult.warnings.length"
            class="import-warnings"
          >
            <li
              v-for="(w, i) in importResult.warnings"
              :key="i"
            >
              {{ w }}
            </li>
          </ul>
        </div>
      </div>
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

.version-grid {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.version-row {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.version-label {
  flex-shrink: 0;
  width: 80px;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.version-value {
  font-size: 0.875rem;
  color: var(--text-primary);
}

.config-io-section {
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
}

.config-io-row {
  display: flex;
  align-items: flex-start;
  gap: 1rem;
}

.config-io-label {
  flex-shrink: 0;
  width: 60px;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
  padding-top: 0.375rem;
}

.config-io-controls {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex-wrap: wrap;
}

.config-io-error {
  font-size: 0.875rem;
  color: var(--danger);
}

.file-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.file-input-hidden {
  display: none;
}

.file-name {
  font-size: 0.8125rem;
  color: var(--text-secondary);
}

.file-name.muted {
  color: var(--text-muted);
}

.import-result {
  margin-top: 0.5rem;
  padding: 0.75rem 1rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
}

.import-stats {
  display: flex;
  gap: 1.25rem;
  color: var(--text-secondary);
  flex-wrap: wrap;
}

.import-warnings {
  margin: 0.5rem 0 0;
  padding-left: 1.25rem;
  color: var(--warning, #e6a817);
  font-size: 0.8125rem;
}
</style>
