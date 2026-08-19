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
import BaseModal from '../components/BaseModal.vue'

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
  notification_delivery_retention_days: 30,
  borg_query_timeout_secs: 300,
  session_idle_timeout_minutes: 480,
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
    settingsForm.notification_delivery_retention_days = Number(
      res.data.notification_delivery_retention_days,
    )
    settingsForm.borg_query_timeout_secs = Number(res.data.borg_query_timeout_secs)
    settingsForm.session_idle_timeout_minutes = res.data.session_idle_timeout_minutes ?? 480
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
  settingsSaved.value = false
  settingsError.value = ''
  try {
    const res = await apiClient.put<SettingsResponse>('/system/settings', {
      retention_days: settingsForm.retention_days,
      report_retention_days: settingsForm.report_retention_days,
      failed_report_retention_days: settingsForm.failed_report_retention_days,
      system_event_retention_days: settingsForm.system_event_retention_days,
      notification_delivery_retention_days: settingsForm.notification_delivery_retention_days,
      timezone: settingsForm.timezone || undefined,
      borg_query_timeout_secs: settingsForm.borg_query_timeout_secs,
      session_idle_timeout_minutes: settingsForm.session_idle_timeout_minutes,
    })
    settingsForm.timezone = res.data.timezone
    settingsForm.retention_days = Number(res.data.retention_days)
    settingsForm.report_retention_days = Number(res.data.report_retention_days)
    settingsForm.failed_report_retention_days = Number(res.data.failed_report_retention_days)
    settingsForm.system_event_retention_days = Number(res.data.system_event_retention_days)
    settingsForm.notification_delivery_retention_days = Number(
      res.data.notification_delivery_retention_days,
    )
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

    <div class="panel">
      <div class="panel-header">
        <h2 class="panel-title">Version</h2>
      </div>

      <BaseSpinner
        v-if="versionLoading"
        size="lg"
      />
      <div
        v-else-if="versionError"
        class="state-msg state-msg--inline state-error"
      >
        {{ versionError }}
      </div>
      <dl
        v-else-if="versionInfo"
        class="info-grid"
      >
        <dt>Server</dt>
        <dd class="mono">{{ versionInfo.server_version }}</dd>
        <dt>Built</dt>
        <dd class="mono">{{ versionInfo.build_timestamp }}</dd>
        <template v-if="versionInfo.agent_version">
          <dt>Agent</dt>
          <dd class="mono">{{ versionInfo.agent_version }}</dd>
        </template>
      </dl>
    </div>

    <div class="panel">
      <div class="panel-header">
        <h2 class="panel-title">SSH public key</h2>
        <button
          class="btn btn-sm btn-ghost btn-danger-text"
          @click="showRegenConfirm = true"
        >
          Regenerate
        </button>
      </div>
      <p class="pane-lede">
        Add this key to <code>~/.ssh/authorized_keys</code> on your borg repository host.
      </p>

      <BaseSpinner
        v-if="loading"
        size="lg"
      />
      <div
        v-else-if="error"
        class="state-msg state-msg--inline state-error"
      >
        {{ error }}
      </div>
      <div
        v-else
        class="token-box token-box--block"
      >
        <pre class="token-text token-text--plain">{{ publicKey }}</pre>
        <button
          class="btn btn-sm btn-ghost"
          @click="copyToClipboard(publicKey)"
        >
          {{ copied ? 'Copied!' : 'Copy' }}
        </button>
      </div>
    </div>

    <div class="panel">
      <div class="panel-header">
        <h2 class="panel-title">Settings</h2>
      </div>

      <BaseSpinner
        v-if="settingsLoading"
        size="lg"
      />
      <template v-else>
        <div
          v-if="settingsError"
          class="state-msg state-msg--inline state-error"
        >
          {{ settingsError }}
        </div>

        <form
          class="form-stack"
          @submit.prevent="saveSettings"
        >
          <div class="field">
            <label
              class="field-label"
              for="settings-timezone"
            >
              Timezone
            </label>
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

          <div class="field">
            <label
              class="field-label"
              for="settings-retention"
            >
              Retention days
            </label>
            <input
              id="settings-retention"
              v-model.number="settingsForm.retention_days"
              type="number"
              min="0"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint">Number of days to keep backup job history.</span>
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-report-retention"
            >
              Report retention (days)
            </label>
            <input
              id="settings-report-retention"
              v-model.number="settingsForm.report_retention_days"
              type="number"
              min="0"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint"
              >Days to keep successful/archived reports. 0 = keep forever.</span
            >
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-failed-retention"
            >
              Failed report retention (days)
            </label>
            <input
              id="settings-failed-retention"
              v-model.number="settingsForm.failed_report_retention_days"
              type="number"
              min="0"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint"
              >Days to keep failed/archive-less reports. 0 = keep forever.</span
            >
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-event-retention"
            >
              System event retention (days)
            </label>
            <input
              id="settings-event-retention"
              v-model.number="settingsForm.system_event_retention_days"
              type="number"
              min="0"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint">Days to keep system events. 0 = keep forever.</span>
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-notification-delivery-retention"
            >
              Notification delivery retention (days)
            </label>
            <input
              id="settings-notification-delivery-retention"
              v-model.number="settingsForm.notification_delivery_retention_days"
              type="number"
              min="0"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint"
              >Days to keep notification delivery-attempt history. 0 = keep forever.</span
            >
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-borg-timeout"
            >
              Borg timeout
            </label>
            <input
              id="settings-borg-timeout"
              v-model.number="settingsForm.borg_query_timeout_secs"
              type="number"
              min="1"
              step="1"
              class="input field-narrow"
            />
            <span class="field-hint"
              >Maximum seconds to wait for a single <code>borg list</code> or
              <code>borg info</code> invocation. Increase for slow or remote repositories.</span
            >
          </div>

          <div class="field">
            <label
              class="field-label"
              for="settings-idle-timeout"
            >
              Session idle timeout
            </label>
            <input
              id="settings-idle-timeout"
              v-model.number="settingsForm.session_idle_timeout_minutes"
              type="number"
              min="1"
              class="input field-narrow"
            />
            <span class="field-hint"
              >Minutes of inactivity before a session expires. Default: 480 (8 hours). Does not
              apply to "Remember Me" sessions.</span
            >
          </div>

          <div class="info-actions">
            <button
              class="btn btn-primary"
              type="submit"
              :disabled="settingsSaving"
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

    <div class="panel">
      <div class="panel-header">
        <h2 class="panel-title">Database storage</h2>
        <button
          class="btn btn-sm btn-ghost"
          :disabled="databaseStorageLoading"
          @click="loadDatabaseStorage"
        >
          {{ databaseStorageLoading ? 'Loading...' : 'Refresh' }}
        </button>
      </div>
      <p class="pane-lede">
        PostgreSQL allocation by application table, including table data, indexes, and TOAST data.
      </p>

      <BaseSpinner
        v-if="databaseStorageLoading"
        size="lg"
      />
      <div
        v-else-if="databaseStorageError"
        class="state-msg state-msg--inline state-error"
      >
        {{ databaseStorageError }}
      </div>
      <template v-else-if="databaseStorage">
        <div class="database-total">
          <span>Total database size</span>
          <strong>{{ formatBytes(databaseStorage.database_bytes) }}</strong>
        </div>
        <div class="table-wrap">
          <table class="data-table data-table--compact">
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
                  <div class="progress-track">
                    <div
                      class="progress-bar"
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
                  <div class="progress-track">
                    <div
                      class="progress-bar progress-bar--muted"
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

    <div class="panel">
      <div class="panel-header">
        <h2 class="panel-title">Configuration export / import</h2>
      </div>
      <p class="pane-lede">
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

    <div class="panel danger-zone">
      <div class="panel-header">
        <h2 class="panel-title">Danger zone</h2>
      </div>
      <p class="pane-lede">
        Emergency actions to bring the system back to a safe state. Use when backups are stuck or
        the system is in an inconsistent state.
      </p>

      <div class="danger-body">
        <div class="danger-info">
          <span class="danger-heading">Cancel all running backups</span>
          <span class="danger-desc">
            Cancels all running and pending backup operations and notifies connected agents to abort
            immediately. Schedules are left unchanged.
          </span>
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
    <BaseModal
      :open="showRegenConfirm"
      title="Regenerate SSH key"
      @close="showRegenConfirm = false"
    >
      <p class="warning-text">
        This will generate a new SSH keypair and invalidate the current key. All borg repository
        hosts will need to be updated with the new public key.
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

      <template #footer>
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
      </template>
    </BaseModal>

    <!-- Reset Confirmation -->
    <BaseModal
      :open="showResetConfirm"
      title="Reset system state"
      @close="showResetConfirm = false"
    >
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

      <template #footer>
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
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.page {
  max-width: 800px;
}

.warning-text {
  font-size: var(--fs-base);
  color: var(--text-secondary);
  margin-bottom: 0.75rem;
}

.warning-bold {
  font-weight: 600;
  color: var(--danger);
}


.database-total {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
  color: var(--text-secondary);
  font-size: var(--fs-base);
}

.database-total strong {
  color: var(--text-primary);
  font-size: var(--fs-lg);
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
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
  font-size: var(--fs-base);
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
  color: var(--warning);
  font-size: var(--fs-sm);
}

.reset-result {
  display: flex;
  gap: 1.25rem;
  margin-top: 1rem;
  padding: 0.75rem 1rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: var(--fs-base);
  color: var(--text-secondary);
  flex-wrap: wrap;
}

.reset-list {
  margin: 0.5rem 0;
  padding-left: 1.25rem;
  font-size: var(--fs-base);
  color: var(--text-primary);

  & li {
    margin-bottom: 0.25rem;
  }
}
</style>
