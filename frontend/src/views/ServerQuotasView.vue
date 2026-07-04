<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { listServerQuotas, upsertServerQuota, deleteServerQuota } from '../api/serverQuotas'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from '../components/BaseSpinner.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type { QuotaAction, ServerQuotaResponse } from '../types/generated'

const QUOTA_ACTION_LABELS: Record<QuotaAction, string> = {
  notify_only: 'Notify only',
  block_backups: 'Block backups',
  disable_schedule: 'Disable schedule',
}

function actionLabel(action: QuotaAction): string {
  return QUOTA_ACTION_LABELS[action]
}

function statusFor(quota: ServerQuotaResponse): 'ok' | 'warning' | 'critical' {
  if (!quota.configured || !quota.enabled) return 'ok'
  const usage = quota.total_deduplicated_size
  if (quota.critical_bytes !== null && usage >= quota.critical_bytes) return 'critical'
  if (quota.warn_bytes !== null && usage >= quota.warn_bytes) return 'warning'
  return 'ok'
}

function bytesToGb(bytes: number | null): number {
  if (bytes === null) return 0
  return Math.round((bytes / 1073741824) * 100) / 100
}

function gbToBytes(gb: number): number | null {
  if (gb <= 0) return null
  return Math.round(gb * 1073741824)
}

const quotas = ref<ServerQuotaResponse[]>([])
const { loading, error, run } = useAsyncAction('Failed to load server quotas')

const editingHost = ref<string | null>(null)
const editError = ref<string | null>(null)
const editLoading = ref(false)
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
  editForm.warn_gb = bytesToGb(quota.warn_bytes)
  editForm.critical_gb = bytesToGb(quota.critical_bytes)
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
  if (!editingHost.value) return
  editLoading.value = true
  editError.value = null
  try {
    const updated = await upsertServerQuota(editingHost.value, {
      warn_bytes: gbToBytes(editForm.warn_gb),
      critical_bytes: gbToBytes(editForm.critical_gb),
      warn_action: editForm.warn_action,
      critical_action: editForm.critical_action,
      enabled: editForm.enabled,
    })
    const index = quotas.value.findIndex((q) => q.ssh_host === updated.ssh_host)
    if (index !== -1) quotas.value[index] = updated
    editingHost.value = null
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
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

    <table
      v-else
      class="data-table"
    >
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
              class="status-badge"
              :class="`badge-${statusFor(quota)}`"
            >
              {{
                statusFor(quota) === 'ok'
                  ? 'OK'
                  : statusFor(quota) === 'warning'
                    ? 'Warning'
                    : 'Critical'
              }}
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

    <div
      v-if="editingHost"
      class="overlay"
      @click.self="cancelEdit"
    >
      <div class="modal">
        <h2>Quota for {{ editingHost }}</h2>
        <form
          class="modal-form"
          @submit.prevent="saveEdit"
        >
          <div class="form-group">
            <label for="warn-gb">Warning threshold (GB)</label>
            <input
              id="warn-gb"
              v-model.number="editForm.warn_gb"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="form-group">
            <label for="warn-action">Warning action</label>
            <select
              id="warn-action"
              v-model="editForm.warn_action"
            >
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
            </select>
          </div>
          <div class="form-group">
            <label for="critical-gb">Critical threshold (GB)</label>
            <input
              id="critical-gb"
              v-model.number="editForm.critical_gb"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="form-group">
            <label for="critical-action">Critical action</label>
            <select
              id="critical-action"
              v-model="editForm.critical_action"
            >
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
            </select>
          </div>
          <div class="form-group toggle-row">
            <span>Enabled</span>
            <ToggleSwitch v-model="editForm.enabled" />
          </div>
          <div
            v-if="editError"
            class="modal-error"
          >
            {{ editError }}
          </div>
          <div class="modal-actions">
            <button
              type="button"
              class="btn btn-ghost"
              @click="cancelEdit"
            >
              Cancel
            </button>
            <button
              type="submit"
              class="btn btn-primary"
              :disabled="editLoading"
            >
              {{ editLoading ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.server-quotas-page {
  max-width: 1000px;
}

.page-description {
  font-size: 0.875rem;
  line-height: 1.5;
  color: var(--text-secondary);
  margin-bottom: 1.5rem;
}

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
}

.data-table th {
  text-align: left;
  padding: 0.625rem 0.75rem;
  font-weight: 600;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
}

.data-table td {
  padding: 0.625rem 0.75rem;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-primary);
}

.name-cell {
  font-weight: 600;
}

.muted {
  color: var(--text-muted);
}

.actions-cell {
  display: flex;
  gap: 0.375rem;
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

.badge-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-critical {
  background: var(--danger-subtle);
  color: var(--danger);
}

.modal {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;
  width: 100%;
  max-width: 420px;
  box-shadow: var(--shadow-lg);
}

.modal h2 {
  font-size: 1.05rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0 0 1rem;
}

.modal-form {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.form-group label {
  font-size: 0.8125rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.form-group input,
.form-group select {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.form-group input:focus,
.form-group select:focus {
  outline: none;
  border-color: var(--accent);
}

.toggle-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
}

.modal-error {
  font-size: 0.8125rem;
  color: var(--danger);
  padding: 0.5rem 0.75rem;
  background: var(--danger-subtle);
  border-radius: var(--radius-sm);
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.5rem;
}
</style>
