<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useWebSocket } from '../composables/useWebSocket'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import {
  listTunnels,
  createTunnel,
  updateTunnel,
  deleteTunnel,
  reconnectTunnel,
} from '../api/tunnels'
import { listAgents } from '../api/agents'
import { Plus, Trash2, Cable, RefreshCw, Server, Globe } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import { badgeClass, tunnelStatusTone, tunnelStatusLabel } from '../utils/badge'
import type {
  TunnelWithStatus,
  TunnelStatus,
  CreateTunnelRequest,
  UpdateTunnelRequest,
} from '../types/tunnel'
import BaseModal from '../components/BaseModal.vue'

interface AgentOption {
  id: number
  hostname: string
}

const tunnels = ref<TunnelWithStatus[]>([])
const { loading, error, run } = useAsyncAction()

const agents = ref<AgentOption[]>([])

const connectedCount = computed(
  (): number => tunnels.value.filter((t) => t.status === 'connected').length,
)
const reconnectingCount = computed(
  (): number => tunnels.value.filter((t) => t.status === 'reconnecting').length,
)
const attentionCount = computed(
  (): number =>
    tunnels.value.filter((t) => t.status !== 'connected' && t.status !== 'reconnecting').length,
)

const showAddDialog = ref(false)
const addForm = ref<CreateTunnelRequest>({
  agent_id: 0,
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
  await run(async () => {
    tunnels.value = await listTunnels()
  })
}

async function loadAgents(): Promise<void> {
  try {
    agents.value = await listAgents()
  } catch (e: unknown) {
    logger.error('loadAgents failed', e)
  }
}

function availableAgents(): AgentOption[] {
  const usedIds = new Set(tunnels.value.map((t) => t.agent_id))
  return agents.value.filter((c) => !usedIds.has(c.id))
}

function openAdd(): void {
  addForm.value = {
    agent_id: 0,
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
  if (!addForm.value.agent_id) {
    addError.value = 'Agent is required'
    return
  }
  addLoading.value = true
  addError.value = ''
  try {
    const created = await createTunnel(addForm.value)
    const withStatus: TunnelWithStatus = {
      ...created,
      status: 'disconnected',
      agent_hostname: agents.value.find((c) => c.id === created.agent_id)?.hostname,
    }
    tunnels.value.push(withStatus)
    showAddDialog.value = false
  } catch (e: unknown) {
    addError.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

const reconnectingId = ref<number | null>(null)

async function reconnect(tunnel: TunnelWithStatus): Promise<void> {
  reconnectingId.value = tunnel.id
  try {
    const updated = await reconnectTunnel(tunnel.id)
    const idx = tunnels.value.findIndex((t) => t.id === tunnel.id)
    if (idx !== -1) {
      tunnels.value[idx] = { ...tunnels.value[idx], ...updated }
    }
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    reconnectingId.value = null
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
  deleteHostname.value = tunnel.agent_hostname ?? String(tunnel.agent_id)
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
onMessage('TunnelStatusChanged', (data) => {
  const tunnel = tunnels.value.find((t) => t.agent_id === data.agent_id)
  if (tunnel) tunnel.status = data.status
})

onMounted(() => {
  loadTunnels().catch(logger.error)
  loadAgents().catch(logger.error)
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
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="tunnels.length === 0"
      :icon="Cable"
      title="No SSH tunnels configured"
      description="Create a tunnel to access remote hosts."
      action="New tunnel"
      @action="showAddDialog = true"
    />

    <template v-else>
      <div class="tiles">
        <div class="tile">
          <span class="stat-label">Total tunnels</span>
          <span class="stat-value--lg">{{ tunnels.length }}</span>
        </div>
        <div class="tile">
          <span class="stat-label">Connected</span>
          <span class="stat-value--lg stat-value--success">{{ connectedCount }}</span>
        </div>
        <div class="tile">
          <span class="stat-label">Reconnecting</span>
          <span class="stat-value--lg stat-value--warning">{{ reconnectingCount }}</span>
        </div>
        <div class="tile">
          <span class="stat-label">Needs attention</span>
          <span class="stat-value--lg stat-value--danger">{{ attentionCount }}</span>
        </div>
      </div>

      <div class="card-grid">
        <div
          v-for="tunnel in tunnels"
          :key="tunnel.id"
          class="entity-card"
          :class="{ 'entity-card--notable': !tunnel.enabled }"
        >
          <div class="card-top">
            <div class="card-info">
              <span class="card-name">{{ tunnel.agent_hostname ?? tunnel.agent_id }}</span>
            </div>
            <span
              class="badge"
              :class="[
                badgeClass(tunnelStatusTone(tunnel.status)),
                { 'status-cell-error': statusErrorMessage(tunnel.status) },
              ]"
              :title="statusErrorMessage(tunnel.status) ?? undefined"
              @click="statusErrorMessage(tunnel.status) ? showErrorDetail(tunnel) : undefined"
            >
              <span class="badge-dot"></span>
              {{ tunnelStatusLabel(tunnel.status) }}
            </span>
          </div>

          <div
            class="tunnel-circuit"
            :class="`tunnel-circuit--${tunnelStatusTone(tunnel.status)}`"
          >
            <div class="tunnel-node">
              <Server :size="14" />
            </div>
            <div class="tunnel-line">
              <span class="tunnel-dot"></span>
            </div>
            <div class="tunnel-port">{{ tunnel.tunnel_port }}</div>
            <div class="tunnel-line">
              <span
                class="tunnel-dot"
                style="animation-delay: -1s"
              ></span>
            </div>
            <div class="tunnel-node">
              <Globe :size="14" />
            </div>
          </div>

          <div class="card-stats">
            <div class="stat">
              <span class="stat-value">{{ tunnel.ssh_user }}@{{ tunnel.ssh_host }}</span>
              <span class="stat-label">SSH target</span>
            </div>
            <div class="stat">
              <span class="stat-value">{{ tunnel.ssh_port }}</span>
              <span class="stat-label">SSH port</span>
            </div>
          </div>

          <div class="card-actions">
            <button
              v-if="tunnel.enabled && tunnel.status !== 'connected'"
              class="btn btn-sm btn-ghost"
              :disabled="reconnectingId === tunnel.id"
              title="Reconnect tunnel"
              @click="reconnect(tunnel)"
            >
              <RefreshCw
                :size="14"
                :class="{ spinning: reconnectingId === tunnel.id }"
              />
            </button>
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
        </div>
      </div>
    </template>

    <!-- Add Tunnel Dialog -->
    <BaseModal
      :open="showAddDialog"
      title="New tunnel"
      @close="showAddDialog = false"
    >
      <div class="field">
        <label class="field-label">Agent <span class="required">*</span></label>
        <select
          v-model.number="addForm.agent_id"
          class="input"
        >
          <option
            :value="0"
            disabled
          >
            Select an agent...
          </option>
          <option
            v-for="agent in availableAgents()"
            :key="agent.id"
            :value="agent.id"
          >
            {{ agent.hostname }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="field-label">SSH host <span class="required">*</span></label>
        <input
          v-model="addForm.ssh_host"
          class="input mono"
          placeholder="e.g. 192.168.1.10"
        />
      </div>
      <div class="field-row">
        <div class="field">
          <label class="field-label">SSH user</label>
          <input
            v-model="addForm.ssh_user"
            class="input mono"
            placeholder="root"
          />
        </div>
        <div class="field field-narrow">
          <label class="field-label">SSH port</label>
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
        <label class="field-label">Tunnel port <span class="required">*</span></label>
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

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showAddDialog = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="
            addLoading || !addForm.agent_id || !addForm.ssh_host.trim() || !addForm.tunnel_port
          "
          @click="submitAdd"
        >
          {{ addLoading ? 'Creating...' : 'Create' }}
        </button>
      </template>
    </BaseModal>

    <!-- Edit Tunnel Dialog -->
    <BaseModal
      :open="showEditDialog"
      title="Edit tunnel"
      @close="showEditDialog = false"
    >
      <div class="field">
        <label class="field-label">SSH host <span class="required">*</span></label>
        <input
          v-model="editForm.ssh_host"
          class="input mono"
          placeholder="e.g. 192.168.1.10"
        />
      </div>
      <div class="field-row">
        <div class="field">
          <label class="field-label">SSH user</label>
          <input
            v-model="editForm.ssh_user"
            class="input mono"
            placeholder="root"
          />
        </div>
        <div class="field field-narrow">
          <label class="field-label">SSH port</label>
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
        <label class="field-label">Tunnel port <span class="required">*</span></label>
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

      <template #footer>
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
      </template>
    </BaseModal>

    <!-- Delete Tunnel Dialog -->
    <BaseModal
      :open="showDeleteDialog"
      title="Delete tunnel"
      size="sm"
      @close="showDeleteDialog = false"
    >
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

      <template #footer>
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
      </template>
    </BaseModal>

    <!-- Error Detail Dialog -->
    <BaseModal
      :open="expandedErrorId !== null"
      title="Tunnel error"
      size="sm"
      @close="closeErrorDetail"
    >
      <pre class="error-pre">{{ errorDetailMessage }}</pre>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="closeErrorDetail"
        >
          Close
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.tunnels-view {
  max-width: 1200px;
}

.status-cell-error {
  cursor: pointer;
}

.status-cell-error:hover {
  text-decoration: underline;
}

/* The circuit: an agent node, an animated line, the tunnel-port chip it
   binds on the agent machine, another line, and the SSH host it dials out
   to. Colour and motion follow the tunnel's own status rather than
   duplicating it as a second badge. */
.tunnel-circuit {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.tunnel-node {
  flex: none;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-hover);
  border: 1px solid var(--border);
  color: var(--text-secondary);
}

.tunnel-line {
  position: relative;
  flex: 1;
  min-width: 16px;
  height: 2px;
  background-image: linear-gradient(to right, var(--border) 50%, transparent 50%);
  background-size: 8px 2px;
  background-repeat: repeat-x;
}

.tunnel-dot {
  position: absolute;
  top: 50%;
  left: 0%;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-muted);
  transform: translate(-50%, -50%);
  opacity: 0;
}

.tunnel-port {
  flex: none;
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-secondary);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  font-weight: 600;
}

.tunnel-circuit--success .tunnel-line {
  background-image: linear-gradient(to right, var(--success) 50%, transparent 50%);
  opacity: 0.6;
}

.tunnel-circuit--success .tunnel-dot {
  background: var(--success);
  opacity: 1;
  animation: tunnel-flow 1.8s linear infinite;
}

.tunnel-circuit--success .tunnel-port {
  background: var(--success-subtle);
  color: var(--success);
}

.tunnel-circuit--warning .tunnel-line {
  background-image: linear-gradient(to right, var(--warning) 50%, transparent 50%);
  opacity: 0.6;
}

.tunnel-circuit--warning .tunnel-dot {
  background: var(--warning);
  opacity: 1;
  animation: tunnel-flow 2.6s ease-in-out infinite;
}

.tunnel-circuit--warning .tunnel-port {
  background: var(--warning-subtle);
  color: var(--warning);
}

.tunnel-circuit--danger .tunnel-line {
  background-image: linear-gradient(to right, var(--danger) 50%, transparent 50%);
  opacity: 0.6;
}

.tunnel-circuit--danger .tunnel-dot {
  left: 50%;
  background: var(--danger);
  opacity: 1;
}

.tunnel-circuit--danger .tunnel-port {
  background: var(--danger-subtle);
  color: var(--danger);
}

@keyframes tunnel-flow {
  from {
    left: 0%;
  }
  to {
    left: 100%;
  }
}

.field-checkbox {
  flex-direction: row;
  align-items: center;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  cursor: pointer;
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

.checkbox-label input[type='checkbox'] {
  width: 15px;
  height: 15px;
  cursor: pointer;
}
</style>
