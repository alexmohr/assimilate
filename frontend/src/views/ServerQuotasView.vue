<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { listServerQuotas, upsertServerQuota, deleteServerQuota } from '../api/serverQuotas'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { actionLabel, bytesToGb, gbToBytes } from '../utils/quota'
import { useAsyncAction } from '../composables/useAsyncAction'

import BaseSpinner from '../components/BaseSpinner.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type { QuotaAction, ServerQuotaResponse } from '../types/generated'

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

    <div
      v-else
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
            class="quota-status-badge"
            :class="`quota-badge-${statusFor(quota)}`"
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
  font-size: 0.9rem;
  font-weight: 600;
  color: var(--text-primary);
}

.stat-label {
  font-size: 0.7rem;
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
  font-size: 0.8125rem;
}

.threshold-row dt {
  font-size: 0.7rem;
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

.muted {
  color: var(--text-muted);
}

.modal {
  max-width: 420px;
}

.toggle-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
}
</style>
