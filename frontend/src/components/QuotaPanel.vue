<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import ToggleSwitch from './ToggleSwitch.vue'

type QuotaAction = 'notify_only' | 'block_backups' | 'disable_schedule'

interface QuotaData {
  warn_bytes: number
  critical_bytes: number
  warn_action: QuotaAction
  critical_action: QuotaAction
  enabled: boolean
}

type QuotaStatus = 'ok' | 'warning' | 'critical'

const QUOTA_ACTIONS: { value: QuotaAction; label: string }[] = [
  { value: 'notify_only', label: 'Notify only' },
  { value: 'block_backups', label: 'Block all backups + notify' },
  { value: 'disable_schedule', label: 'Disable schedule + notify' },
]

const props = defineProps<{ repoId: number; isAdmin: boolean; currentUsageBytes: number }>()

const quota = ref<QuotaData | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)
const isEditing = ref(false)
const editLoading = ref(false)
const editError = ref<string | null>(null)

const editForm = reactive({
  warn_gb: 0,
  critical_gb: 0,
  warn_action: 'notify_only' as QuotaAction,
  critical_action: 'notify_only' as QuotaAction,
  enabled: true,
})

const quotaStatus = computed((): QuotaStatus => {
  if (!quota.value || !quota.value.enabled) return 'ok'
  const usage = props.currentUsageBytes
  if (quota.value.critical_bytes > 0 && usage >= quota.value.critical_bytes) return 'critical'
  if (quota.value.warn_bytes > 0 && usage >= quota.value.warn_bytes) return 'warning'
  return 'ok'
})

const usagePercent = computed((): number => {
  if (!quota.value || !quota.value.enabled) return 0
  const limit = quota.value.critical_bytes || quota.value.warn_bytes
  if (limit <= 0) return 0
  return Math.min(100, (props.currentUsageBytes / limit) * 100)
})

const statusLabel = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'Warning'
  if (s === 'critical') return 'Critical'
  return 'OK'
})

const statusBadgeClass = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'badge-warn'
  if (s === 'critical') return 'badge-crit'
  return 'badge-ok'
})

const progressBarClass = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'bar-warn'
  if (s === 'critical') return 'bar-crit'
  return 'bar-ok'
})

function actionLabel(action: QuotaAction): string {
  return QUOTA_ACTIONS.find((a) => a.value === action)?.label ?? action
}

function bytesToGb(bytes: number): number {
  return Math.round((bytes / 1073741824) * 100) / 100
}

function gbToBytes(gb: number): number {
  return Math.round(gb * 1073741824)
}

async function loadQuota(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const res = await apiClient.get<QuotaData>(`/repos/${props.repoId}/quota`)
    quota.value = res.data
  } catch (e: unknown) {
    const status = (e as { response?: { status?: number } }).response?.status
    if (status === 404) {
      quota.value = null
    } else {
      error.value = extractError(e)
    }
  } finally {
    loading.value = false
  }
}

function startEdit(): void {
  if (!quota.value) return
  editForm.warn_gb = bytesToGb(quota.value.warn_bytes)
  editForm.critical_gb = bytesToGb(quota.value.critical_bytes)
  editForm.warn_action = quota.value.warn_action ?? 'notify_only'
  editForm.critical_action = quota.value.critical_action ?? 'notify_only'
  editForm.enabled = quota.value.enabled
  editError.value = null
  isEditing.value = true
}

function startNewQuota(): void {
  editForm.warn_gb = 0
  editForm.critical_gb = 0
  editForm.warn_action = 'notify_only'
  editForm.critical_action = 'notify_only'
  editForm.enabled = true
  editError.value = null
  isEditing.value = true
}

function cancelEdit(): void {
  isEditing.value = false
  editError.value = null
}

async function saveQuota(): Promise<void> {
  editLoading.value = true
  editError.value = null
  try {
    await apiClient.put(`/repos/${props.repoId}/quota`, {
      warn_bytes: gbToBytes(editForm.warn_gb),
      critical_bytes: gbToBytes(editForm.critical_gb),
      warn_action: editForm.warn_action,
      critical_action: editForm.critical_action,
      enabled: editForm.enabled,
    })
    isEditing.value = false
    await loadQuota()
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

onMounted(loadQuota)
</script>

<template>
  <div class="quota-panel info-card">
    <div class="info-card-header">
      <h3 class="info-title">Storage Quota</h3>
      <div class="info-header-actions">
        <span
          v-if="quota && quota.enabled"
          class="status-badge"
          :class="statusBadgeClass"
        >
          {{ statusLabel }}
        </span>
        <button
          v-if="isAdmin && !isEditing && quota"
          class="btn btn-sm btn-ghost"
          @click="startEdit"
        >
          Edit
        </button>
      </div>
    </div>

    <div
      v-if="loading"
      class="state-msg state-msg-sm"
    >
      Loading quota...
    </div>

    <div
      v-else-if="error"
      class="state-msg state-msg-sm state-error"
    >
      {{ error }}
    </div>

    <template v-else-if="!quota && !isEditing">
      <div class="muted">No quota configured for this repository.</div>
      <button
        v-if="isAdmin"
        class="btn btn-sm btn-ghost"
        style="margin-top: 0.75rem"
        @click="startNewQuota"
      >
        Configure Quota
      </button>
    </template>

    <template v-else-if="quota && !isEditing">
      <div
        v-if="!quota.enabled"
        class="muted"
      >
        Quota monitoring is disabled for this repository.
      </div>
      <template v-else>
        <div class="quota-usage">
          <div class="usage-labels">
            <span class="usage-current">{{ formatBytes(props.currentUsageBytes) }} used</span>
            <span class="usage-limit">
              {{ formatBytes(quota.critical_bytes || quota.warn_bytes) }} limit
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :class="progressBarClass"
              :style="{ width: usagePercent + '%' }"
            />
          </div>
        </div>
        <dl class="quota-details">
          <dt>Warning threshold</dt>
          <dd>{{ formatBytes(quota.warn_bytes) }}</dd>
          <dt>Warning action</dt>
          <dd>{{ actionLabel(quota.warn_action) }}</dd>
          <dt>Critical threshold</dt>
          <dd>{{ formatBytes(quota.critical_bytes) }}</dd>
          <dt>Critical action</dt>
          <dd>{{ actionLabel(quota.critical_action) }}</dd>
        </dl>
      </template>
    </template>

    <template v-else-if="isEditing">
      <div class="edit-form">
        <div class="form-grid">
          <div class="field">
            <label class="field-label">Warning (GB)</label>
            <input
              v-model.number="editForm.warn_gb"
              class="input"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="field">
            <label class="field-label">Warning action</label>
            <select
              v-model="editForm.warn_action"
              class="input"
            >
              <option
                v-for="a in QUOTA_ACTIONS"
                :key="a.value"
                :value="a.value"
              >
                {{ a.label }}
              </option>
            </select>
          </div>
          <div class="field">
            <label class="field-label">Critical (GB)</label>
            <input
              v-model.number="editForm.critical_gb"
              class="input"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="field">
            <label class="field-label">Critical action</label>
            <select
              v-model="editForm.critical_action"
              class="input"
            >
              <option
                v-for="a in QUOTA_ACTIONS"
                :key="a.value"
                :value="a.value"
              >
                {{ a.label }}
              </option>
            </select>
          </div>
          <div class="field field-full toggle-row">
            <span class="toggle-row-label">Enabled</span>
            <ToggleSwitch v-model="editForm.enabled" />
          </div>
        </div>
        <div
          v-if="editError"
          class="form-error"
        >
          {{ editError }}
        </div>
        <div class="edit-actions">
          <button
            class="btn btn-ghost"
            @click="cancelEdit"
          >
            Cancel
          </button>
          <button
            class="btn btn-primary"
            :disabled="editLoading"
            @click="saveQuota"
          >
            {{ editLoading ? 'Saving...' : 'Save' }}
          </button>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.quota-panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;
}

.info-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1.25rem;
}

.info-header-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.info-title {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin: 0;
}

.status-badge {
  display: inline-block;
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
}

.badge-ok {
  background: var(--success-subtle, oklch(0.95 0.05 145));
  color: var(--success);
}

.badge-warn {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-crit {
  background: var(--danger-subtle);
  color: var(--danger);
}

.quota-usage {
  margin-bottom: 1rem;
}

.usage-labels {
  display: flex;
  justify-content: space-between;
  font-size: 0.8rem;
  margin-bottom: 0.4rem;
}

.usage-current {
  color: var(--text-primary);
  font-weight: 600;
}

.usage-limit {
  color: var(--text-muted);
}

.progress-bar-track {
  height: 8px;
  background: var(--bg-input, var(--border));
  border-radius: 4px;
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  border-radius: 4px;
  transition: width 0.3s ease;
}

.bar-ok {
  background: var(--success);
}

.bar-warn {
  background: var(--warning);
}

.bar-crit {
  background: var(--danger);
}

.quota-details {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.4rem 1rem;
  margin: 0;
  font-size: 0.85rem;
}

.quota-details dt {
  color: var(--text-muted);
}

.quota-details dd {
  margin: 0;
  color: var(--text-primary);
}

.muted {
  color: var(--text-muted);
  font-size: 0.875rem;
}

.state-msg {
  text-align: center;
  padding: 1.5rem;
  color: var(--text-muted);
}

.state-msg-sm {
  padding: 1rem;
  font-size: 0.875rem;
}

.state-error {
  color: var(--danger);
}

.edit-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem 1rem;
}

.field-full {
  grid-column: 1 / -1;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--border);
}

.form-error {
  color: var(--danger);
  font-size: 0.85rem;
}
</style>
