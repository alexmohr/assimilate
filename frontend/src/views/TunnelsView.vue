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
import { listTunnels, createTunnel } from '../api/tunnels'
import { listAgents } from '../api/agents'
import { ArrowRight, Cable, ChevronRight, Plus } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import { tunnelStatusTone, tunnelStatusLabel } from '../utils/badge'
import type { TunnelWithStatus, CreateTunnelRequest } from '../types/tunnel'
import BaseModal from '../components/BaseModal.vue'
import {
  matchesTunnelFilter,
  matchesTunnelSearch,
  sortTunnels,
  tunnelDotTone,
  tunnelErrorMessage,
  type TunnelFilter,
} from '../utils/tunnel'

interface AgentOption {
  id: number
  hostname: string
  display_name?: string | null
}

const tunnels = ref<TunnelWithStatus[]>([])
const { loading, error, run } = useAsyncAction()

const agents = ref<AgentOption[]>([])

const search = ref('')
const filter = ref<TunnelFilter>('all')

interface FilterPill {
  value: TunnelFilter
  label: string
  dot: string | null
  count: number
}

/** Every pill always: "Connected · 0" is itself an answer on this page. */
const pills = computed((): FilterPill[] => {
  const all: FilterPill[] = [
    { value: 'all', label: 'All', dot: null, count: 0 },
    { value: 'connected', label: 'Connected', dot: 'status-dot--success', count: 0 },
    { value: 'reconnecting', label: 'Reconnecting', dot: 'status-dot--warning', count: 0 },
    { value: 'attention', label: 'Needs attention', dot: 'status-dot--danger', count: 0 },
  ]
  for (const pill of all) {
    pill.count = tunnels.value.filter((t) => matchesTunnelFilter(t, pill.value)).length
  }
  return all
})

/** Filtered, searched, and problems first. */
const visibleTunnels = computed((): TunnelWithStatus[] =>
  sortTunnels(
    tunnels.value.filter(
      (t) =>
        matchesTunnelFilter(t, filter.value) &&
        matchesTunnelSearch(search.value, agentLabel(t), agentDisplayName(t), t.ssh_host),
    ),
    agentLabel,
  ),
)

function countText(count: number): string {
  return count === 1 ? '1 tunnel' : `${count} tunnels`
}

function tunnelDotClass(tunnel: TunnelWithStatus): string {
  const tone = tunnelDotTone(tunnel)
  return tone ? `status-dot--${tone}` : ''
}

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

// `GET /tunnels` returns only `agent_id` - `agent_hostname` is set locally
// only right after creating a tunnel in this session (submitAdd, below), so
// every tunnel loaded from the list needs its hostname resolved against the
// agents list already fetched by loadAgents().
function agentLabel(tunnel: TunnelWithStatus): string {
  return (
    tunnel.agent_hostname ??
    agents.value.find((a) => a.id === tunnel.agent_id)?.hostname ??
    String(tunnel.agent_id)
  )
}

/** The agent's display name, when it has one that says more than its hostname. */
function agentDisplayName(tunnel: TunnelWithStatus): string | null {
  const name = agents.value.find((a) => a.id === tunnel.agent_id)?.display_name ?? null
  return name && name !== agentLabel(tunnel) ? name : null
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

useEscapeKey(showAddDialog, () => {
  showAddDialog.value = false
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
    <p class="page-description">
      Reverse SSH tunnels that let an agent reach this server when it cannot connect to it directly.
    </p>

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

    <div
      v-else
      class="tunnels-list"
    >
      <div class="tunnels-toolbar">
        <div
          class="filter-pills"
          role="group"
          aria-label="Filter tunnels"
        >
          <button
            v-for="pill in pills"
            :key="pill.value"
            type="button"
            class="filter-pill"
            :class="{ active: filter === pill.value }"
            :aria-pressed="filter === pill.value"
            @click="filter = pill.value"
          >
            <span
              v-if="pill.dot !== null"
              class="status-dot"
              :class="pill.dot"
            />
            {{ pill.label }} · {{ pill.count }}
          </button>
        </div>
        <input
          v-model="search"
          type="search"
          class="input search-input"
          placeholder="Search by agent or host"
          aria-label="Search tunnels"
        />
      </div>

      <section class="list-group">
        <div class="list-group-header">
          <h2 class="list-group-title">Tunnels</h2>
          <span class="list-group-count">{{ countText(visibleTunnels.length) }}</span>
        </div>
        <div
          class="list-cols"
          aria-hidden="true"
        >
          <span>Agent</span>
          <span>Route</span>
          <span>Status</span>
          <span />
        </div>

        <div
          v-if="visibleTunnels.length === 0"
          class="list-row tunnels-none"
        >
          <span class="muted">No tunnel matches.</span>
        </div>

        <RouterLink
          v-for="tunnel in visibleTunnels"
          :key="tunnel.id"
          :to="`/tunnels/${tunnel.id}`"
          class="list-row tunnel-row"
          :data-tunnel-id="tunnel.id"
        >
          <div class="list-row-main">
            <span
              class="status-dot"
              :class="tunnelDotClass(tunnel)"
            />
            <div class="list-row-text">
              <span class="list-row-name">{{ agentLabel(tunnel) }}</span>
              <span
                v-if="agentDisplayName(tunnel)"
                class="list-row-sub"
              >
                {{ agentDisplayName(tunnel) }}
              </span>
            </div>
          </div>

          <div class="tunnel-route">
            <span class="list-wide-only tunnel-route-server">server</span>
            <ArrowRight
              :size="14"
              class="list-wide-only tunnel-route-arrow"
            />
            <span
              class="badge badge--neutral mono"
              data-testid="tunnel-port"
              >:{{ tunnel.tunnel_port }}</span
            >
            <ArrowRight
              :size="14"
              class="tunnel-route-arrow"
            />
            <span class="mono tunnel-target"
              >{{ tunnel.ssh_user }}@{{ tunnel.ssh_host }}:{{ tunnel.ssh_port }}</span
            >
          </div>

          <div class="list-row-text tunnel-status">
            <!-- The dot says connected; any other state is named, in its colour. -->
            <span
              v-if="tunnelStatusTone(tunnel.status) !== 'success'"
              :class="`tunnel-status-text--${tunnelStatusTone(tunnel.status)}`"
            >
              {{ tunnelStatusLabel(tunnel.status) }}
            </span>
            <span
              v-if="tunnelErrorMessage(tunnel.status)"
              class="list-row-sub"
              data-testid="tunnel-error"
            >
              {{ tunnelErrorMessage(tunnel.status) }}
            </span>
            <span
              v-if="!tunnel.enabled"
              class="list-row-sub"
            >
              Disabled
            </span>
          </div>

          <ChevronRight
            :size="16"
            class="list-row-chevron"
          />
        </RouterLink>
      </section>
    </div>

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
  </div>
</template>

<style scoped>
.tunnels-view {
  max-width: 1200px;
}

.tunnels-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.tunnels-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
}

.tunnels-list .list-group {
  --list-cols: minmax(0, 1.4fr) minmax(0, 2.4fr) minmax(0, 1.4fr) 2.5rem;
}

.tunnels-none {
  display: block;
}

.tunnel-route {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.tunnel-route-server {
  color: var(--text-secondary);
}

.tunnel-route-arrow {
  flex: none;
  color: var(--text-muted);
}

.tunnel-target {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.tunnel-status {
  align-items: flex-start;
  min-width: 0;
}

.tunnel-status .list-row-sub {
  white-space: normal;
  overflow-wrap: anywhere;
}

.tunnel-status-text--success {
  color: var(--success);
}

.tunnel-status-text--warning {
  color: var(--warning);
}

.tunnel-status-text--danger {
  color: var(--danger);
}

.tunnel-status-text--neutral {
  color: var(--text-secondary);
}

@media (max-width: 640px) {
  .search-input {
    width: 100%;
  }

  /* Line 1 is the agent with the chevron beside it, line 2 the route, line 3
     the status. */

  .tunnel-row > .list-row-main {
    padding-right: var(--space-9);
  }

  .tunnel-route,
  .tunnel-status {
    flex: 1 1 100%;
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
