<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useWebSocket } from '../composables/useWebSocket'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { listTunnels, createTunnel, updateTunnel, deleteTunnel } from '../api/tunnels'
import { apiClient } from '../api/client'
import { Plus, Trash2, Cable } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import type {
  TunnelWithStatus,
  TunnelStatus,
  CreateTunnelRequest,
  UpdateTunnelRequest,
} from '../types/tunnel'

interface ClientOption {
  id: number
  hostname: string
}

const tunnels = ref<TunnelWithStatus[]>([])
const loading = ref(false)
const error = ref('')

const clients = ref<ClientOption[]>([])

const showAddDialog = ref(false)
const addForm = ref<CreateTunnelRequest>({
  client_id: 0,
  ssh_host: '',
  ssh_user: 'root',
  ssh_port: 22,
  tunnel_port: 0,
  enabled: true,
})
const addError = ref('')
const addLoading = ref(false)

const showEditDialog = ref(false)
const editId = ref<number | null>(null)
const editForm = ref<UpdateTunnelRequest>({
  ssh_host: '',
  ssh_user: '',
  ssh_port: 22,
  tunnel_port: 0,
  enabled: true,
})
const editError = ref('')
const editLoading = ref(false)

const showDeleteDialog = ref(false)
const deleteId = ref<number | null>(null)
const deleteHostname = ref('')
const deleteLoading = ref(false)
const deleteError = ref('')

const expandedErrorId = ref<number | null>(null)
const errorDetailMessage = ref('')
const showErrorDialog = computed(() => expandedErrorId.value !== null)

function showErrorDetail(tunnel: TunnelWithStatus): void {
  const msg = statusErrorMessage(tunnel.status)
  if (msg) {
    errorDetailMessage.value = msg
    expandedErrorId.value = tunnel.id
  }
}

function closeErrorDetail(): void {
  expandedErrorId.value = null
  errorDetailMessage.value = ''
}

async function loadTunnels(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    tunnels.value = await listTunnels()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

async function loadClients(): Promise<void> {
  try {
    const res = await apiClient.get<ClientOption[]>('/clients')
    clients.value = res.data
  } catch (e: unknown) {
    logger.error('loadClients failed', e)
  }
}

function availableClients(): ClientOption[] {
  const usedIds = new Set(tunnels.value.map((t) => t.client_id))
  return clients.value.filter((c) => !usedIds.has(c.id))
}

function openAdd(): void {
  addForm.value = {
    client_id: 0,
    ssh_host: '',
    ssh_user: 'root',
    ssh_port: 22,
    tunnel_port: 0,
    enabled: true,
  }
  addError.value = ''
  showAddDialog.value = true
}

async function submitAdd(): Promise<void> {
  if (!addForm.value.ssh_host.trim()) {
    addError.value = 'SSH host is required'
    return
  }
  if (!addForm.value.client_id) {
    addError.value = 'Client is required'
    return
  }
  addLoading.value = true
  addError.value = ''
  try {
    const created = await createTunnel(addForm.value)
    const withStatus: TunnelWithStatus = {
      ...created,
      status: 'disconnected',
      client_hostname: clients.value.find((c) => c.id === created.client_id)?.hostname,
    }
    tunnels.value.push(withStatus)
    showAddDialog.value = false
  } catch (e: unknown) {
    addError.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

function openEdit(tunnel: TunnelWithStatus): void {
  editId.value = tunnel.id
  editForm.value = {
    ssh_host: tunnel.ssh_host,
    ssh_user: tunnel.ssh_user,
    ssh_port: tunnel.ssh_port,
    tunnel_port: tunnel.tunnel_port,
    enabled: tunnel.enabled,
  }
  editError.value = ''
  showEditDialog.value = true
}

async function submitEdit(): Promise<void> {
  if (editId.value === null) return
  editLoading.value = true
  editError.value = ''
  try {
    const updated = await updateTunnel(editId.value, editForm.value)
    const idx = tunnels.value.findIndex((t) => t.id === editId.value)
    if (idx !== -1) {
      tunnels.value[idx] = { ...tunnels.value[idx], ...updated }
    }
    showEditDialog.value = false
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

function openDelete(tunnel: TunnelWithStatus): void {
  deleteId.value = tunnel.id
  deleteHostname.value = tunnel.client_hostname ?? String(tunnel.client_id)
  deleteError.value = ''
  showDeleteDialog.value = true
}

async function confirmDelete(): Promise<void> {
  if (deleteId.value === null) return
  deleteLoading.value = true
  deleteError.value = ''
  try {
    await deleteTunnel(deleteId.value)
    tunnels.value = tunnels.value.filter((t) => t.id !== deleteId.value)
    showDeleteDialog.value = false
  } catch (e: unknown) {
    deleteError.value = extractError(e)
  } finally {
    deleteLoading.value = false
  }
}

function statusLabel(status: TunnelStatus): string {
  if (status === 'connected') return 'Connected'
  if (status === 'disconnected') return 'Disconnected'
  if (status === 'reconnecting') return 'Reconnecting'
  return 'Error'
}

function statusClass(status: TunnelStatus): string {
  if (status === 'connected') return 'status-connected'
  if (status === 'disconnected') return 'status-disconnected'
  if (status === 'reconnecting') return 'status-reconnecting'
  return 'status-error'
}

function statusErrorMessage(status: TunnelStatus): string | null {
  if (typeof status === 'object' && 'error' in status) return status.error.message
  return null
}

useEscapeKey(showAddDialog, () => {
  showAddDialog.value = false
})
useEscapeKey(showEditDialog, () => {
  showEditDialog.value = false
})
useEscapeKey(showDeleteDialog, () => {
  showDeleteDialog.value = false
})
useEscapeKey(showErrorDialog, () => {
  closeErrorDetail()
})

const { onMessage } = useWebSocket()
onMessage(
  'TunnelStatusChanged',
  (data: { client_id: number; hostname: string; status: TunnelStatus }) => {
    const tunnel = tunnels.value.find((t) => t.client_id === data.client_id)
    if (tunnel) tunnel.status = data.status
  },
)

onMounted(() => {
  loadTunnels().catch(logger.error)
  loadClients().catch(logger.error)
})
</script>

<template>
  <div class="tunnels-view">
    <div class="page-header">
      <h1 class="page-title">Tunnels</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openAdd"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

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
    <EmptyState
      v-else-if="tunnels.length === 0"
      :icon="Cable"
      title="No SSH tunnels configured"
      description="Create a tunnel to access remote hosts."
      action="Add Tunnel"
      @action="showAddDialog = true"
    />

    <div
      v-else
      class="table-wrapper"
    >
      <table class="tunnels-table">
        <thead>
          <tr>
            <th>Client</th>
            <th>SSH Host</th>
            <th>SSH User</th>
            <th>SSH Port</th>
            <th>Tunnel Port</th>
            <th>Status</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="tunnel in tunnels"
            :key="tunnel.id"
          >
            <td class="mono">{{ tunnel.client_hostname ?? tunnel.client_id }}</td>
            <td class="mono">{{ tunnel.ssh_host }}</td>
            <td class="mono">{{ tunnel.ssh_user }}</td>
            <td class="mono">{{ tunnel.ssh_port }}</td>
            <td class="mono">{{ tunnel.tunnel_port }}</td>
            <td>
              <div
                class="status-cell"
                :class="{ 'status-cell-error': statusErrorMessage(tunnel.status) }"
                :title="statusErrorMessage(tunnel.status) ?? undefined"
                @click="statusErrorMessage(tunnel.status) ? showErrorDetail(tunnel) : undefined"
              >
                <span
                  class="status-dot"
                  :class="statusClass(tunnel.status)"
                />
                <span class="status-text">{{ statusLabel(tunnel.status) }}</span>
              </div>
            </td>
            <td>
              <div class="row-actions">
                <button
                  class="btn btn-sm btn-ghost"
                  @click="openEdit(tunnel)"
                >
                  Edit
                </button>
                <button
                  class="btn btn-sm btn-ghost btn-danger-text"
                  @click="openDelete(tunnel)"
                >
                  <Trash2 :size="14" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Add Tunnel Dialog -->
    <Teleport to="body">
      <div
        v-if="showAddDialog"
        class="overlay"
        @click.self="showAddDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Add Tunnel</h2>
            <button
              class="close-btn"
              @click="showAddDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <div class="field">
              <label class="field-label">Client <span class="required">*</span></label>
              <select
                v-model.number="addForm.client_id"
                class="input"
              >
                <option
                  :value="0"
                  disabled
                >
                  Select a client...
                </option>
                <option
                  v-for="client in availableClients()"
                  :key="client.id"
                  :value="client.id"
                >
                  {{ client.hostname }}
                </option>
              </select>
            </div>
            <div class="field">
              <label class="field-label">SSH Host <span class="required">*</span></label>
              <input
                v-model="addForm.ssh_host"
                class="input mono"
                placeholder="e.g. 192.168.1.10"
              />
            </div>
            <div class="field-row">
              <div class="field">
                <label class="field-label">SSH User</label>
                <input
                  v-model="addForm.ssh_user"
                  class="input mono"
                  placeholder="root"
                />
              </div>
              <div class="field field-narrow">
                <label class="field-label">SSH Port</label>
                <input
                  v-model.number="addForm.ssh_port"
                  class="input"
                  type="number"
                  min="1"
                  max="65535"
                />
              </div>
            </div>
            <div class="field">
              <label class="field-label">Tunnel Port <span class="required">*</span></label>
              <input
                v-model.number="addForm.tunnel_port"
                class="input"
                type="number"
                min="1"
                max="65535"
                placeholder="e.g. 2222"
              />
              <span class="field-hint">Local port forwarded through the tunnel</span>
            </div>
            <div class="field field-checkbox">
              <label class="checkbox-label">
                <input
                  v-model="addForm.enabled"
                  type="checkbox"
                />
                <span>Enable tunnel immediately</span>
              </label>
            </div>
            <div
              v-if="addError"
              class="form-error"
            >
              {{ addError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showAddDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-primary"
              :disabled="
                addLoading || !addForm.client_id || !addForm.ssh_host.trim() || !addForm.tunnel_port
              "
              @click="submitAdd"
            >
              {{ addLoading ? 'Creating...' : 'Create' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Edit Tunnel Dialog -->
    <Teleport to="body">
      <div
        v-if="showEditDialog"
        class="overlay"
        @click.self="showEditDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Edit Tunnel</h2>
            <button
              class="close-btn"
              @click="showEditDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <div class="field">
              <label class="field-label">SSH Host <span class="required">*</span></label>
              <input
                v-model="editForm.ssh_host"
                class="input mono"
                placeholder="e.g. 192.168.1.10"
              />
            </div>
            <div class="field-row">
              <div class="field">
                <label class="field-label">SSH User</label>
                <input
                  v-model="editForm.ssh_user"
                  class="input mono"
                  placeholder="root"
                />
              </div>
              <div class="field field-narrow">
                <label class="field-label">SSH Port</label>
                <input
                  v-model.number="editForm.ssh_port"
                  class="input"
                  type="number"
                  min="1"
                  max="65535"
                />
              </div>
            </div>
            <div class="field">
              <label class="field-label">Tunnel Port <span class="required">*</span></label>
              <input
                v-model.number="editForm.tunnel_port"
                class="input"
                type="number"
                min="1"
                max="65535"
              />
            </div>
            <div class="field field-checkbox">
              <label class="checkbox-label">
                <input
                  v-model="editForm.enabled"
                  type="checkbox"
                />
                <span>Enabled</span>
              </label>
            </div>
            <div
              v-if="editError"
              class="form-error"
            >
              {{ editError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showEditDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-primary"
              :disabled="editLoading || !editForm.ssh_host?.trim() || !editForm.tunnel_port"
              @click="submitEdit"
            >
              {{ editLoading ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Delete Tunnel Dialog -->
    <Teleport to="body">
      <div
        v-if="showDeleteDialog"
        class="overlay"
        @click.self="showDeleteDialog = false"
      >
        <div class="dialog dialog-sm">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Tunnel</h2>
            <button
              class="close-btn"
              @click="showDeleteDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="confirm-text">
              Delete tunnel for <strong>{{ deleteHostname }}</strong
              >?
            </p>
            <div
              v-if="deleteError"
              class="form-error"
            >
              {{ deleteError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showDeleteDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="deleteLoading"
              @click="confirmDelete"
            >
              {{ deleteLoading ? 'Deleting...' : 'Delete' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Error Detail Dialog -->
    <Teleport to="body">
      <div
        v-if="expandedErrorId !== null"
        class="overlay"
        @click.self="closeErrorDetail"
      >
        <div class="dialog dialog-sm">
          <div class="dialog-header">
            <h2 class="dialog-title">Tunnel Error</h2>
            <button
              class="close-btn"
              @click="closeErrorDetail"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <pre class="error-pre">{{ errorDetailMessage }}</pre>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="closeErrorDetail"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.tunnels-view {
  max-width: 1100px;
}

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.table-wrapper {
  overflow-x: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.tunnels-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
}

.tunnels-table th {
  padding: 0.75rem 1rem;
  text-align: left;
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
  white-space: nowrap;
}

.tunnels-table td {
  padding: 0.75rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
  vertical-align: middle;
}

.tunnels-table tbody tr:last-child td {
  border-bottom: none;
}

.tunnels-table tbody tr:hover td {
  background: var(--bg-hover);
}

.status-cell {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.status-connected {
  background: var(--success);
}

.status-disconnected {
  background: var(--text-muted);
}

.status-reconnecting {
  background: var(--warning);
  animation: pulse 1.4s ease-in-out infinite;
}

.status-error {
  background: var(--danger);
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.status-text {
  font-size: 0.82rem;
  color: var(--text-secondary);
}

.status-cell-error {
  cursor: pointer;
}

.status-cell-error:hover .status-text {
  text-decoration: underline;
}

.error-pre {
  font-family: var(--mono);
  font-size: 0.82rem;
  color: var(--danger);
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  border-radius: var(--radius-sm);
  padding: 0.75rem 1rem;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}

.row-actions {
  display: flex;
  gap: 0.25rem;
}

.field-row {
  display: flex;
  gap: 1rem;
  margin-bottom: 1rem;
}

.field-row .field {
  flex: 1;
  margin-bottom: 0;
}

.field-narrow {
  max-width: 120px;
  flex: 0 0 120px !important;
}

.field-checkbox {
  flex-direction: row;
  align-items: center;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.checkbox-label input[type='checkbox'] {
  width: 15px;
  height: 15px;
  cursor: pointer;
}
</style>
