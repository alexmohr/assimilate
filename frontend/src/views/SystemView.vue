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
import { formatBytes } from '../utils/format'
import BaseSpinner from '../components/BaseSpinner.vue'
import TimezoneSelect from '../components/TimezoneSelect.vue'
import type {
  ImportResultResponse,
  SettingsResponse,
  SystemResetResponse,
} from '../types/generated'

interface VersionInfo {
  server_version: string
  server_git_sha: string
  build_timestamp: string
  agent_version: string | null
}

interface DatabaseRelationSize {
  table_name: string
  table_bytes: number
  index_bytes: number
  toast_bytes: number
  total_bytes: number
}

interface DatabaseStorageResponse {
  database_bytes: number
  other_bytes: number
  relations: DatabaseRelationSize[]
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
const settingsForm = reactive({
  timezone: '',
  retention_days: 7,
  report_retention_days: 0,
  failed_report_retention_days: 365,
  system_event_retention_days: 90,
  borg_query_timeout_secs: 300,
})

const versionInfo = ref<VersionInfo | null>(null)
const versionLoading = ref(true)
const versionError = ref('')

const databaseStorage = ref<DatabaseStorageResponse | null>(null)
const databaseStorageLoading = ref(true)
const databaseStorageError = ref('')

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
    settingsForm.retention_days = Number(res.data.retention_days)
    settingsForm.report_retention_days = Number(res.data.report_retention_days)
    settingsForm.failed_report_retention_days = Number(res.data.failed_report_retention_days)
    settingsForm.system_event_retention_days = Number(res.data.system_event_retention_days)
    settingsForm.borg_query_timeout_secs = Number(res.data.borg_query_timeout_secs)
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

  await loadDatabaseStorage()
})

async function loadDatabaseStorage(): Promise<void> {
  databaseStorageLoading.value = true
  databaseStorageError.value = ''
  try {
    const res = await apiClient.get<DatabaseStorageResponse>('/system/database-storage')
    databaseStorage.value = res.data
  } catch (e: unknown) {
    databaseStorageError.value = extractError(e, 'Failed to load database storage')
  } finally {
    databaseStorageLoading.value = false
  }
}

function storagePercent(bytes: number): number {
  const total = databaseStorage.value?.database_bytes ?? 0
  return total > 0 ? (bytes / total) * 100 : 0
}

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
const importResult = ref<ImportResultResponse | null>(null)
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
    const res = await apiClient.post<ImportResultResponse>('/config/import', payload)
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
      report_retention_days: settingsForm.report_retention_days,
      failed_report_retention_days: settingsForm.failed_report_retention_days,
      system_event_retention_days: settingsForm.system_event_retention_days,
      timezone: settingsForm.timezone || undefined,
      borg_query_timeout_secs: settingsForm.borg_query_timeout_secs,
    })
    settingsForm.timezone = res.data.timezone
    settingsForm.retention_days = Number(res.data.retention_days)
    settingsForm.report_retention_days = Number(res.data.report_retention_days)
    settingsForm.failed_report_retention_days = Number(res.data.failed_report_retention_days)
    settingsForm.system_event_retention_days = Number(res.data.system_event_retention_days)
    settingsForm.borg_query_timeout_secs = Number(res.data.borg_query_timeout_secs)
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

const showResetConfirm = ref(false)
const resetting = ref(false)
const resetError = ref('')
const resetResult = ref<SystemResetResponse | null>(null)

async function resetSystem(): Promise<void> {
  resetting.value = true
  resetError.value = ''
  resetResult.value = null
  try {
    const res = await apiClient.post<SystemResetResponse>('/system/reset')
    resetResult.value = res.data
    showResetConfirm.value = false
  } catch (e: unknown) {
    resetError.value = extractError(e, 'Reset failed')
  } finally {
    resetting.value = false
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

        <form
          class="settings-form"
          @submit.prevent="saveSettings"
        >
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
                step="1"
                class="form-input retention-input"
              />
              <span class="field-hint">Number of days to keep backup job history.</span>
            </div>
          </div>

          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-report-retention"
            >
              Report Retention (days)
            </label>
            <div class="setting-input-group">
              <input
                id="settings-report-retention"
                v-model.number="settingsForm.report_retention_days"
                type="number"
                min="0"
                step="1"
                class="form-input retention-input"
              />
              <span class="field-hint"
                >Days to keep successful/archived reports. 0 = keep forever.</span
              >
            </div>
          </div>

          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-failed-retention"
            >
              Failed Report Retention (days)
            </label>
            <div class="setting-input-group">
              <input
                id="settings-failed-retention"
                v-model.number="settingsForm.failed_report_retention_days"
                type="number"
                min="0"
                step="1"
                class="form-input retention-input"
              />
              <span class="field-hint"
                >Days to keep failed/archive-less reports. 0 = keep forever.</span
              >
            </div>
          </div>

          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-event-retention"
            >
              System Event Retention (days)
            </label>
            <div class="setting-input-group">
              <input
                id="settings-event-retention"
                v-model.number="settingsForm.system_event_retention_days"
                type="number"
                min="0"
                step="1"
                class="form-input retention-input"
              />
              <span class="field-hint">Days to keep system events. 0 = keep forever.</span>
            </div>
          </div>

          <div class="setting-row">
            <label
              class="setting-label"
              for="settings-borg-timeout"
            >
              Borg Timeout
            </label>
            <div class="setting-input-group">
              <input
                id="settings-borg-timeout"
                v-model.number="settingsForm.borg_query_timeout_secs"
                type="number"
                min="1"
                step="1"
                class="form-input retention-input"
              />
              <span class="field-hint"
                >Maximum seconds to wait for a single <code>borg list</code> or
                <code>borg info</code> invocation. Increase for slow or remote repositories.</span
              >
            </div>
          </div>

          <div class="settings-actions">
            <button
              class="btn btn-primary"
              type="submit"
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
        </form>
      </template>
    </div>

    <div class="info-card">
      <div class="card-header">
        <h3 class="info-title">Database Storage</h3>
        <button
          class="btn btn-sm btn-ghost"
          :disabled="databaseStorageLoading"
          @click="loadDatabaseStorage"
        >
          {{ databaseStorageLoading ? 'Loading...' : 'Refresh' }}
        </button>
      </div>
      <p class="info-description">
        PostgreSQL allocation by application table, including table data, indexes, and TOAST data.
      </p>

      <BaseSpinner
        v-if="databaseStorageLoading"
        size="lg"
      />
      <div
        v-else-if="databaseStorageError"
        class="state-msg error"
      >
        {{ databaseStorageError }}
      </div>
      <template v-else-if="databaseStorage">
        <div class="database-total">
          <span>Total database size</span>
          <strong>{{ formatBytes(databaseStorage.database_bytes) }}</strong>
        </div>
        <div class="storage-table-wrap">
          <table class="storage-table">
            <thead>
              <tr>
                <th>Table</th>
                <th>Table data</th>
                <th>Indexes</th>
                <th>TOAST</th>
                <th>Total</th>
                <th>Share</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="relation in databaseStorage.relations"
                :key="relation.table_name"
              >
                <td class="storage-name">{{ relation.table_name }}</td>
                <td>{{ formatBytes(relation.table_bytes) }}</td>
                <td>{{ formatBytes(relation.index_bytes) }}</td>
                <td>{{ formatBytes(relation.toast_bytes) }}</td>
                <td class="storage-total">{{ formatBytes(relation.total_bytes) }}</td>
                <td class="storage-share">
                  <div class="storage-share-value">
                    {{ storagePercent(relation.total_bytes).toFixed(1) }}%
                  </div>
                  <div class="storage-bar">
                    <div
                      class="storage-bar-fill"
                      :style="{ width: `${storagePercent(relation.total_bytes)}%` }"
                    ></div>
                  </div>
                </td>
              </tr>
              <tr v-if="databaseStorage.other_bytes > 0">
                <td class="storage-name">Other PostgreSQL storage</td>
                <td colspan="3">System catalogs and database overhead</td>
                <td class="storage-total">{{ formatBytes(databaseStorage.other_bytes) }}</td>
                <td class="storage-share">
                  <div class="storage-share-value">
                    {{ storagePercent(databaseStorage.other_bytes).toFixed(1) }}%
                  </div>
                  <div class="storage-bar">
                    <div
                      class="storage-bar-fill storage-bar-fill-muted"
                      :style="{ width: `${storagePercent(databaseStorage.other_bytes)}%` }"
                    ></div>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
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
            <span>Repos created: {{ importResult.repos_created }}</span>
            <span>Repos updated: {{ importResult.repos_updated }}</span>
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

    <div class="info-card danger-zone-card">
      <div class="card-header">
        <h3 class="info-title danger-title">Danger Zone</h3>
      </div>
      <p class="info-description">
        Emergency actions to bring the system back to a safe state. Use when backups are stuck or
        the system is in an inconsistent state.
      </p>

      <div class="danger-action">
        <div class="danger-action-info">
          <div class="danger-action-name">Cancel All Running Backups</div>
          <div class="danger-action-desc">
            Cancels all running and pending backup operations and notifies connected agents to abort
            immediately. Schedules are left unchanged.
          </div>
        </div>
        <button
          class="btn btn-sm btn-danger"
          @click="showResetConfirm = true"
        >
          Reset
        </button>
      </div>

      <div
        v-if="resetResult"
        class="reset-result"
      >
        <span>Cancelled backups: {{ resetResult.cancelled_backups }}</span>
        <span>Agents notified: {{ resetResult.notified_agents }}</span>
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

    <!-- Reset Confirmation -->
    <Teleport to="body">
      <div
        v-if="showResetConfirm"
        class="overlay"
        @click.self="showResetConfirm = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Reset System State</h2>
            <button
              class="close-btn"
              @click="showResetConfirm = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="warning-text">This will immediately:</p>
            <ul class="reset-list">
              <li>Cancel all running and pending backup operations in the database</li>
              <li>Send abort signals to all currently connected agents</li>
            </ul>
            <p class="warning-text warning-bold">Schedules are left unchanged.</p>
            <div
              v-if="resetError"
              class="form-error"
            >
              {{ resetError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showResetConfirm = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="resetting"
              @click="resetSystem"
            >
              {{ resetting ? 'Resetting...' : 'Reset System' }}
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

.database-total {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
  color: var(--text-secondary);
  font-size: 0.875rem;
}

.database-total strong {
  color: var(--text-primary);
  font-size: 1.25rem;
}

.storage-table-wrap {
  overflow-x: auto;
}

.storage-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.75rem;
  white-space: nowrap;
}

.storage-table th,
.storage-table td {
  padding: 0.625rem 0.5rem;
  border-bottom: 1px solid var(--border);
  text-align: right;
}

.storage-table th {
  color: var(--text-muted);
  font-weight: 500;
}

.storage-table th:first-child,
.storage-table td:first-child {
  text-align: left;
}

.storage-table tbody tr:last-child td {
  border-bottom: 0;
}

.storage-name {
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.storage-total {
  color: var(--text-primary);
  font-weight: 600;
}

.storage-share {
  min-width: 90px;
}

.storage-share-value {
  margin-bottom: 0.25rem;
}

.storage-bar {
  height: 4px;
  overflow: hidden;
  background: var(--bg-hover);
  border-radius: 999px;
}

.storage-bar-fill {
  height: 100%;
  background: var(--primary);
  border-radius: inherit;
}

.storage-bar-fill-muted {
  background: var(--text-muted);
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

.danger-zone-card {
  border-color: var(--danger, #dc2626);
}

.danger-title {
  color: var(--danger, #dc2626);
}

.danger-action {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

.danger-action-info {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.danger-action-name {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
}

.danger-action-desc {
  font-size: 0.8125rem;
  color: var(--text-secondary);
}

.reset-result {
  display: flex;
  gap: 1.25rem;
  margin-top: 1rem;
  padding: 0.75rem 1rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 0.875rem;
  color: var(--text-secondary);
  flex-wrap: wrap;
}

.reset-list {
  margin: 0.5rem 0;
  padding-left: 1.25rem;
  font-size: 0.875rem;
  color: var(--text-primary);

  & li {
    margin-bottom: 0.25rem;
  }
}
</style>
