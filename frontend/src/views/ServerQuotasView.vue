<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { listServerQuotas, upsertServerQuota, deleteServerQuota } from '../api/serverQuotas'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { actionLabel, bytesToGb, gbToBytes } from '../utils/quota'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useMobile } from '../composables/useMobile'
import { useWebSocket } from '../composables/useWebSocket'
import BaseSpinner from '../components/BaseSpinner.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import type { QuotaAction, ServerQuotaResponse } from '../types/generated'
import BaseModal from '../components/BaseModal.vue'
import { badgeClass, thresholdTone } from '../utils/badge'

const { isMobile } = useMobile()

function statusFor(quota: ServerQuotaResponse): 'ok' | 'warning' | 'critical' {
  if (!quota.configured || !quota.enabled) return 'ok'
  const usage = quota.total_deduplicated_size
  if (quota.critical_bytes !== null && usage >= quota.critical_bytes) return 'critical'
  if (quota.warn_bytes !== null && usage >= quota.warn_bytes) return 'warning'
  return 'ok'
}

function statusLabel(quota: ServerQuotaResponse): string {
  switch (statusFor(quota)) {
    case 'ok':
      return 'OK'
    case 'warning':
      return 'Warning'
    case 'critical':
      return 'Critical'
  }
}

/** `quota.warn_bytes`/`critical_bytes` are `null` only when no quota is configured yet. */
function bytesToGbOrZero(bytes: number | null): number {
  return bytes === null ? 0 : bytesToGb(bytes)
}

const quotas = ref<ServerQuotaResponse[]>([])
const { loading, error, run } = useAsyncAction('Failed to load server quotas')

const editingHost = ref<string | null>(null)
const { loading: editLoading, error: editError, run: runEdit } = useAsyncAction()
const editForm = reactive({
  warn_gb: 0,
  critical_gb: 0,
  warn_action: 'notify_only' as QuotaAction,
  critical_action: 'notify_only' as QuotaAction,
  enabled: true,
})

const deleteLoading = ref<string | null>(null)

async function loadQuotas(): Promise<void> {
  await run(async () => {
    quotas.value = await listServerQuotas()
  })
}

function startEdit(quota: ServerQuotaResponse): void {
  editForm.warn_gb = bytesToGbOrZero(quota.warn_bytes)
  editForm.critical_gb = bytesToGbOrZero(quota.critical_bytes)
  editForm.warn_action = quota.warn_action
  editForm.critical_action = quota.critical_action
  editForm.enabled = quota.configured ? quota.enabled : true
  editError.value = null
  editingHost.value = quota.ssh_host
}

function cancelEdit(): void {
  editingHost.value = null
  editError.value = null
}

async function saveEdit(): Promise<void> {
  const host = editingHost.value
  if (!host) return
  await runEdit(async () => {
    const updated = await upsertServerQuota(host, {
      warn_bytes: gbToBytes(editForm.warn_gb),
      critical_bytes: gbToBytes(editForm.critical_gb),
      warn_action: editForm.warn_action,
      critical_action: editForm.critical_action,
      enabled: editForm.enabled,
    })
    const index = quotas.value.findIndex((q) => q.ssh_host === updated.ssh_host)
    if (index !== -1) quotas.value[index] = updated
    editingHost.value = null
  })
}

async function removeQuota(quota: ServerQuotaResponse): Promise<void> {
  deleteLoading.value = quota.ssh_host
  try {
    await deleteServerQuota(quota.ssh_host)
    await loadQuotas()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    deleteLoading.value = null
  }
}

onMounted(loadQuotas)

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => loadQuotas().catch(logger.error))
</script>

<template>
  <div class="server-quotas-page">
    <div class="page-header">
      <h1 class="page-title">Server Quotas</h1>
    </div>

    <p class="page-description">
      Set a combined storage limit across every repository that shares the same SSH host, for the
      case where multiple repositories reside on one server with a shared disk quota.
    </p>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="state-msg state-error"
    >
      {{ error }}
    </div>
    <div
      v-else-if="quotas.length === 0"
      class="state-msg"
    >
      No repositories are configured yet.
    </div>

    <div
      v-else-if="isMobile"
      class="quota-card-list"
    >
      <div
        v-for="quota in quotas"
        :key="quota.ssh_host"
        class="quota-card"
      >
        <div class="quota-card-top">
          <span class="quota-host">{{ quota.ssh_host }}</span>
          <span
            class="badge"
            :class="badgeClass(thresholdTone(statusFor(quota)))"
          >
            {{ statusLabel(quota) }}
          </span>
        </div>
        <div class="quota-card-stats">
          <div class="stat">
            <span class="stat-value">{{ quota.repo_count }}</span>
            <span class="stat-label">Repos</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatBytes(quota.total_deduplicated_size) }}</span>
            <span class="stat-label">Usage</span>
          </div>
        </div>
        <dl class="quota-card-thresholds">
          <div class="threshold-row">
            <dt>Warning</dt>
            <dd>
              <template v-if="quota.configured && quota.warn_bytes !== null">
                {{ formatBytes(quota.warn_bytes) }} &middot; {{ actionLabel(quota.warn_action) }}
              </template>
              <span
                v-else
                class="muted"
                >Not set</span
              >
            </dd>
          </div>
          <div class="threshold-row">
            <dt>Critical</dt>
            <dd>
              <template v-if="quota.configured && quota.critical_bytes !== null">
                {{ formatBytes(quota.critical_bytes) }} &middot;
                {{ actionLabel(quota.critical_action) }}
              </template>
              <span
                v-else
                class="muted"
                >Not set</span
              >
            </dd>
          </div>
        </dl>
        <div class="quota-card-actions">
          <button
            class="btn btn-sm btn-ghost"
            @click="startEdit(quota)"
          >
            {{ quota.configured ? 'Edit' : 'Configure' }}
          </button>
          <button
            v-if="quota.configured"
            class="btn btn-sm btn-ghost btn-danger-text"
            :disabled="deleteLoading === quota.ssh_host"
            @click="removeQuota(quota)"
          >
            Remove
          </button>
        </div>
      </div>
    </div>

    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table">
        <thead>
          <tr>
            <th>SSH Host</th>
            <th>Repos</th>
            <th>Usage</th>
            <th>Warning</th>
            <th>Critical</th>
            <th>Status</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="quota in quotas"
            :key="quota.ssh_host"
          >
            <td class="name-cell">{{ quota.ssh_host }}</td>
            <td>{{ quota.repo_count }}</td>
            <td>{{ formatBytes(quota.total_deduplicated_size) }}</td>
            <td>
              <template v-if="quota.configured && quota.warn_bytes !== null">
                {{ formatBytes(quota.warn_bytes) }} &middot; {{ actionLabel(quota.warn_action) }}
              </template>
              <span
                v-else
                class="muted"
                >Not set</span
              >
            </td>
            <td>
              <template v-if="quota.configured && quota.critical_bytes !== null">
                {{ formatBytes(quota.critical_bytes) }} &middot;
                {{ actionLabel(quota.critical_action) }}
              </template>
              <span
                v-else
                class="muted"
                >Not set</span
              >
            </td>
            <td>
              <span
                class="badge"
                :class="badgeClass(thresholdTone(statusFor(quota)))"
              >
                {{ statusLabel(quota) }}
              </span>
            </td>
            <td class="actions-cell">
              <button
                class="btn btn-sm btn-ghost"
                @click="startEdit(quota)"
              >
                {{ quota.configured ? 'Edit' : 'Configure' }}
              </button>
              <button
                v-if="quota.configured"
                class="btn btn-sm btn-ghost btn-danger-text"
                :disabled="deleteLoading === quota.ssh_host"
                @click="removeQuota(quota)"
              >
                Remove
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <BaseModal
      :open="editingHost !== null"
      form
      @close="cancelEdit"
      @submit="saveEdit"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Quota for {{ editingHost }}
        </h2>
      </template>
      <div class="field">
        <label
          class="field-label"
          for="warn-gb"
          >Warning threshold (GB)</label
        >
        <input
          id="warn-gb"
          v-model.number="editForm.warn_gb"
          type="number"
          class="input"
          min="0"
          step="0.1"
        />
      </div>
      <div class="field">
        <label
          class="field-label"
          for="warn-action"
          >Warning action</label
        >
        <select
          id="warn-action"
          v-model="editForm.warn_action"
          class="input"
        >
          <option value="notify_only">Notify only</option>
          <option value="block_backups">Block backups</option>
          <option value="disable_schedule">Disable schedule</option>
        </select>
      </div>
      <div class="field">
        <label
          class="field-label"
          for="critical-gb"
          >Critical threshold (GB)</label
        >
        <input
          id="critical-gb"
          v-model.number="editForm.critical_gb"
          type="number"
          class="input"
          min="0"
          step="0.1"
        />
      </div>
      <div class="field">
        <label
          class="field-label"
          for="critical-action"
          >Critical action</label
        >
        <select
          id="critical-action"
          v-model="editForm.critical_action"
          class="input"
        >
          <option value="notify_only">Notify only</option>
          <option value="block_backups">Block backups</option>
          <option value="disable_schedule">Disable schedule</option>
        </select>
      </div>
      <div class="field toggle-row">
        <span class="field-label">Enabled</span>
        <ToggleSwitch v-model="editForm.enabled" />
      </div>

      <template #footer>
        <ModalFormActions
          :submitting="editLoading"
          :error="editError"
          submit-label="Save"
          submitting-label="Saving..."
          @cancel="cancelEdit"
        />
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.server-quotas-page {
  max-width: 1000px;
}

.table-wrap {
  overflow-x: auto;
  border-radius: var(--radius);
  border: 1px solid var(--border);
}

.quota-card-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.quota-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.quota-card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.quota-host {
  font-weight: 600;
  word-break: break-all;
}

.quota-card-stats {
  display: flex;
  gap: 1.5rem;
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.stat-value {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-primary);
}

.stat-label {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.quota-card-thresholds {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  margin: 0;
}

.threshold-row {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
  font-size: var(--fs-sm);
}

.threshold-row dt {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.threshold-row dd {
  margin: 0;
  color: var(--text-primary);
}

.quota-card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.375rem;
}

.data-table {
  min-width: 640px;
}

.muted {
  color: var(--text-muted);
}

.toggle-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
}
</style>
