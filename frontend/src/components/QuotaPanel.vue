<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { actionLabel, bytesToGb, gbToBytes } from '../utils/quota'
import type { QuotaAction } from '../types/generated'
import ToggleSwitch from './ToggleSwitch.vue'
import EditFormActions from './EditFormActions.vue'

interface QuotaData {
  warn_bytes: number
  critical_bytes: number
  warn_action: QuotaAction
  critical_action: QuotaAction
  enabled: boolean
}

type QuotaStatus = 'ok' | 'warning' | 'critical'

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
  editForm.warn_action = quota.value.warn_action
  editForm.critical_action = quota.value.critical_action
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
          class="badge"
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
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
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
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
            </select>
          </div>
          <div class="field field-full toggle-row">
            <span class="toggle-row-label">Enabled</span>
            <ToggleSwitch v-model="editForm.enabled" />
          </div>
        </div>
        <EditFormActions
          :saving="editLoading"
          :error="editError"
          @cancel="cancelEdit"
          @save="saveQuota"
        />
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

.info-title {
  margin: 0;
}

.quota-usage {
  margin-bottom: 1rem;
}

.usage-labels {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-sm);
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
  background: var(--bg-input);
  border-radius: var(--radius-pill);
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  border-radius: var(--radius-pill);
  transition: width var(--duration-slow) ease;
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
  font-size: var(--fs-base);
}

.quota-details dt {
  color: var(--text-muted);
}

.quota-details dd {
  margin: 0;
  color: var(--text-primary);
}
</style>
