<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { Gauge, Plus, Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'

type QuotaAction = 'notify_only' | 'block_backups' | 'disable_schedule'

interface ServerQuota {
  ssh_host: string
  warn_bytes: number | null
  critical_bytes: number | null
  warn_action: QuotaAction
  critical_action: QuotaAction
  enabled: boolean
  updated_at: string
}

const QUOTA_ACTIONS: { value: QuotaAction; label: string }[] = [
  { value: 'notify_only', label: 'Notify only' },
  { value: 'block_backups', label: 'Block all backups + notify' },
  { value: 'disable_schedule', label: 'Disable schedule + notify' },
]

const quotas = ref<ServerQuota[]>([])
const hosts = ref<string[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

const showAddDialog = ref(false)
const addForm = reactive({
  ssh_host: '',
  warn_gb: 0,
  critical_gb: 0,
  warn_action: 'notify_only' as QuotaAction,
  critical_action: 'notify_only' as QuotaAction,
  enabled: true,
})
const addError = ref<string | null>(null)
const addLoading = ref(false)

const editingHost = ref<string | null>(null)
const editForm = reactive({
  warn_gb: 0,
  critical_gb: 0,
  warn_action: 'notify_only' as QuotaAction,
  critical_action: 'notify_only' as QuotaAction,
  enabled: true,
})
const editError = ref<string | null>(null)
const editLoading = ref(false)

const deleteConfirm = ref<string | null>(null)
const deleteLoading = ref(false)

const availableHosts = computed((): string[] => {
  const configured = new Set(quotas.value.map((q) => q.ssh_host))
  return hosts.value.filter((h) => !configured.has(h))
})

function bytesToGb(bytes: number | null): number {
  if (!bytes) return 0
  return Math.round((bytes / 1073741824) * 100) / 100
}

function gbToBytes(gb: number): number | null {
  if (gb <= 0) return null
  return Math.round(gb * 1073741824)
}

function actionLabel(action: QuotaAction): string {
  return QUOTA_ACTIONS.find((a) => a.value === action)?.label ?? action
}

async function loadData(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [quotasRes, hostsRes] = await Promise.all([
      apiClient.get<ServerQuota[]>('/server-quotas'),
      apiClient.get<string[]>('/server-quotas/hosts'),
    ])
    quotas.value = quotasRes.data
    hosts.value = hostsRes.data
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

function openAddDialog(): void {
  addForm.ssh_host = availableHosts.value[0] ?? ''
  addForm.warn_gb = 0
  addForm.critical_gb = 0
  addForm.warn_action = 'notify_only'
  addForm.critical_action = 'notify_only'
  addForm.enabled = true
  addError.value = null
  showAddDialog.value = true
}

function cancelAdd(): void {
  showAddDialog.value = false
  addError.value = null
}

async function submitAdd(): Promise<void> {
  addLoading.value = true
  addError.value = null
  try {
    await apiClient.put(`/server-quotas/${encodeURIComponent(addForm.ssh_host)}`, {
      warn_bytes: gbToBytes(addForm.warn_gb),
      critical_bytes: gbToBytes(addForm.critical_gb),
      warn_action: addForm.warn_action,
      critical_action: addForm.critical_action,
      enabled: addForm.enabled,
    })
    showAddDialog.value = false
    await loadData()
  } catch (e: unknown) {
    addError.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

function startEdit(quota: ServerQuota): void {
  editingHost.value = quota.ssh_host
  editForm.warn_gb = bytesToGb(quota.warn_bytes)
  editForm.critical_gb = bytesToGb(quota.critical_bytes)
  editForm.warn_action = quota.warn_action ?? 'notify_only'
  editForm.critical_action = quota.critical_action ?? 'notify_only'
  editForm.enabled = quota.enabled
  editError.value = null
}

function cancelEdit(): void {
  editingHost.value = null
  editError.value = null
}

async function submitEdit(): Promise<void> {
  if (!editingHost.value) return
  editLoading.value = true
  editError.value = null
  try {
    await apiClient.put(`/server-quotas/${encodeURIComponent(editingHost.value)}`, {
      warn_bytes: gbToBytes(editForm.warn_gb),
      critical_bytes: gbToBytes(editForm.critical_gb),
      warn_action: editForm.warn_action,
      critical_action: editForm.critical_action,
      enabled: editForm.enabled,
    })
    editingHost.value = null
    await loadData()
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

function confirmDelete(host: string): void {
  deleteConfirm.value = host
}

async function submitDelete(): Promise<void> {
  if (!deleteConfirm.value) return
  deleteLoading.value = true
  try {
    await apiClient.delete(`/server-quotas/${encodeURIComponent(deleteConfirm.value)}`)
    deleteConfirm.value = null
    await loadData()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    deleteLoading.value = false
  }
}

onMounted(loadData)
</script>

<template>
  <div class="view-root">
    <div class="view-header">
      <div class="view-header-left">
        <Gauge
          :size="22"
          class="view-header-icon"
        />
        <h1 class="view-title">Server Quotas</h1>
      </div>
      <button
        class="btn btn-primary"
        :disabled="availableHosts.length === 0 && !loading"
        @click="openAddDialog"
      >
        <Plus :size="16" />
        Add Quota
      </button>
    </div>

    <p class="view-description">
      Set storage limits across all repositories sharing the same backup server. When the combined
      deduplicated size of all repositories on a server exceeds a threshold, the configured action
      is triggered.
    </p>

    <BaseSpinner v-if="loading" />

    <div
      v-else-if="error"
      class="state-error"
    >
      {{ error }}
    </div>

    <EmptyState
      v-else-if="quotas.length === 0"
      title="No server quotas configured"
      description="Add a quota to monitor total storage usage across repositories on a shared backup server."
    />

    <div
      v-else
      class="quota-list"
    >
      <div
        v-for="quota in quotas"
        :key="quota.ssh_host"
        class="quota-card"
      >
        <template v-if="editingHost === quota.ssh_host">
          <div class="quota-card-header">
            <span class="host-name">{{ quota.ssh_host }}</span>
          </div>
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
                @click="submitEdit"
              >
                {{ editLoading ? 'Saving…' : 'Save' }}
              </button>
            </div>
          </div>
        </template>

        <template v-else>
          <div class="quota-card-header">
            <div class="host-info">
              <span class="host-name">{{ quota.ssh_host }}</span>
              <span
                v-if="!quota.enabled"
                class="badge badge-muted"
              >
                Disabled
              </span>
            </div>
            <div class="card-actions">
              <button
                class="btn btn-sm btn-ghost"
                @click="startEdit(quota)"
              >
                Edit
              </button>
              <button
                class="btn btn-sm btn-ghost btn-danger"
                @click="confirmDelete(quota.ssh_host)"
              >
                <Trash2 :size="14" />
              </button>
            </div>
          </div>
          <dl class="quota-details">
            <dt>Warning</dt>
            <dd>
              {{ quota.warn_bytes ? formatBytes(quota.warn_bytes) : '—' }}
              <span
                v-if="quota.warn_bytes"
                class="action-tag"
              >
                {{ actionLabel(quota.warn_action) }}
              </span>
            </dd>
            <dt>Critical</dt>
            <dd>
              {{ quota.critical_bytes ? formatBytes(quota.critical_bytes) : '—' }}
              <span
                v-if="quota.critical_bytes"
                class="action-tag"
              >
                {{ actionLabel(quota.critical_action) }}
              </span>
            </dd>
          </dl>
        </template>
      </div>
    </div>

    <!-- Add dialog -->
    <div
      v-if="showAddDialog"
      class="dialog-overlay"
      @click.self="cancelAdd"
    >
      <div class="dialog">
        <h2 class="dialog-title">Add Server Quota</h2>
        <div class="form-grid">
          <div class="field field-full">
            <label class="field-label">Server (SSH host)</label>
            <select
              v-model="addForm.ssh_host"
              class="input"
            >
              <option
                v-for="host in availableHosts"
                :key="host"
                :value="host"
              >
                {{ host }}
              </option>
            </select>
            <p
              v-if="availableHosts.length === 0"
              class="field-hint"
            >
              All known servers already have quotas configured.
            </p>
          </div>
          <div class="field">
            <label class="field-label">Warning (GB)</label>
            <input
              v-model.number="addForm.warn_gb"
              class="input"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="field">
            <label class="field-label">Warning action</label>
            <select
              v-model="addForm.warn_action"
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
              v-model.number="addForm.critical_gb"
              class="input"
              type="number"
              min="0"
              step="0.1"
            />
          </div>
          <div class="field">
            <label class="field-label">Critical action</label>
            <select
              v-model="addForm.critical_action"
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
            <ToggleSwitch v-model="addForm.enabled" />
          </div>
        </div>
        <div
          v-if="addError"
          class="form-error"
        >
          {{ addError }}
        </div>
        <div class="dialog-actions">
          <button
            class="btn btn-ghost"
            @click="cancelAdd"
          >
            Cancel
          </button>
          <button
            class="btn btn-primary"
            :disabled="addLoading || !addForm.ssh_host"
            @click="submitAdd"
          >
            {{ addLoading ? 'Saving…' : 'Add Quota' }}
          </button>
        </div>
      </div>
    </div>

    <!-- Delete confirm dialog -->
    <div
      v-if="deleteConfirm"
      class="dialog-overlay"
      @click.self="deleteConfirm = null"
    >
      <div class="dialog">
        <h2 class="dialog-title">Delete Server Quota</h2>
        <p class="dialog-body">
          Remove the quota for <strong>{{ deleteConfirm }}</strong
          >? This cannot be undone.
        </p>
        <div class="dialog-actions">
          <button
            class="btn btn-ghost"
            @click="deleteConfirm = null"
          >
            Cancel
          </button>
          <button
            class="btn btn-danger"
            :disabled="deleteLoading"
            @click="submitDelete"
          >
            {{ deleteLoading ? 'Deleting…' : 'Delete' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.view-root {
  max-width: 800px;
  margin: 0 auto;
  padding: 2rem 1.5rem;
}

.view-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.view-header-left {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.view-header-icon {
  color: var(--text-muted);
}

.view-title {
  font-size: 1.4rem;
  font-weight: 700;
  margin: 0;
}

.view-description {
  color: var(--text-muted);
  font-size: 0.875rem;
  margin: 0 0 1.5rem;
}

.quota-list {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.quota-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.quota-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.75rem;
}

.host-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.host-name {
  font-weight: 600;
  font-size: 0.95rem;
}

.badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  font-size: 0.72rem;
  font-weight: 600;
}

.badge-muted {
  background: var(--bg-input, var(--border));
  color: var(--text-muted);
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.quota-details {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.35rem 1rem;
  margin: 0;
  font-size: 0.85rem;
}

.quota-details dt {
  color: var(--text-muted);
}

.quota-details dd {
  margin: 0;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.action-tag {
  font-size: 0.72rem;
  color: var(--text-muted);
  background: var(--bg-input, var(--border));
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
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

.field-hint {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin: 0.25rem 0 0;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
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

.state-error {
  color: var(--danger);
  font-size: 0.875rem;
  padding: 1rem;
}

.form-error {
  color: var(--danger);
  font-size: 0.85rem;
}

.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;
  width: 100%;
  max-width: 480px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
}

.dialog-title {
  font-size: 1.1rem;
  font-weight: 600;
  margin: 0 0 1.25rem;
}

.dialog-body {
  font-size: 0.9rem;
  color: var(--text-secondary);
  margin: 0 0 1.25rem;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1.25rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}

.btn-danger {
  background: var(--danger);
  color: #fff;
  border: none;
}

.btn-danger:hover:not(:disabled) {
  opacity: 0.9;
}
</style>
