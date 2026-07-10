<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { useMobile } from '../composables/useMobile'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { Plus, SlidersHorizontal, Server, AlertCircle } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import CardError from '../components/CardError.vue'
import type { DashboardOverview } from '../types/dashboard'
import type { AgentRow } from '../types/agent'
import type { TagRow } from '../types/tag'
import type { CreateAgentResponse } from '../types/generated'

interface AgentTagRow {
  agent_id: number
  tag_name: string
  tag_color: string
}

interface HealthEntry {
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
  last_error_message: string | null
}

interface AgentHealth {
  failed: number
  overdue: number
  warning: number
  total: number
  last_error_message: string | null
}

type SortField = 'hostname' | 'status' | 'last_seen' | 'version'
type SortDir = 'asc' | 'desc'
type FilterStatus = 'all' | 'online' | 'offline'
type CoverageFilter = 'all' | 'protected' | 'unassigned' | 'never-succeeded' | 'disabled-only'

function coverageFilterFromQuery(value: unknown): CoverageFilter {
  if (value === 'protected') return 'protected'
  if (value === 'unassigned') return 'unassigned'
  if (value === 'never-succeeded') return 'never-succeeded'
  if (value === 'disabled-only') return 'disabled-only'
  return 'all'
}

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)
const agents = ref<AgentRow[]>([])
const showHidden = ref(false)
const machineScheduleCount = ref<Record<number, number>>({})
const healthByHost = ref<Record<string, AgentHealth>>({})
const activeBackupsByHost = ref<Record<string, string[]>>({})
const loading = ref(false)
const error = ref<string | null>(null)

const sortField = ref<SortField>('hostname')
const sortDir = ref<SortDir>('asc')
const filterStatus = ref<FilterStatus>(
  (route.query.status as FilterStatus) === 'online' ||
    (route.query.status as FilterStatus) === 'offline'
    ? (route.query.status as FilterStatus)
    : 'all',
)
const filterText = ref('')
const filterTagIds = ref<number[]>([])
const filterCoverage = ref<CoverageFilter>(coverageFilterFromQuery(route.query.coverage))
const coverageHostIds = ref<Record<Exclude<CoverageFilter, 'all'>, Set<number>>>({
  protected: new Set(),
  unassigned: new Set(),
  'never-succeeded': new Set(),
  'disabled-only': new Set(),
})
const showTagDropdown = ref(false)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const allAgentTags = ref<TagRow[]>([])
const agentTagsMap = ref<Record<number, { name: string; color: string }[]>>({})

const filteredAgents = computed(() => {
  let list = [...agents.value]

  if (filterStatus.value === 'online') {
    list = list.filter((m) => m.is_connected)
  } else if (filterStatus.value === 'offline') {
    list = list.filter((m) => !m.is_connected)
  }

  if (filterCoverage.value !== 'all') {
    const hostIds = coverageHostIds.value[filterCoverage.value]
    list = list.filter((agent) => hostIds.has(agent.id))
  }

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (m) =>
        m.hostname.toLowerCase().includes(q) ||
        (m.display_name?.toLowerCase().includes(q) ?? false) ||
        (agentTagsMap.value[m.id] ?? []).some((t) => t.name.toLowerCase().includes(q)),
    )
  }

  if (filterTagIds.value.length > 0) {
    const selectedNames = new Set(
      allAgentTags.value.filter((t) => filterTagIds.value.includes(t.id)).map((t) => t.name),
    )
    list = list.filter((m) =>
      (agentTagsMap.value[m.id] ?? []).some((t) => selectedNames.has(t.name)),
    )
  }

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'hostname':
        cmp = a.hostname.localeCompare(b.hostname)
        break
      case 'status':
        cmp = Number(b.is_connected) - Number(a.is_connected)
        break
      case 'last_seen':
        cmp = (a.last_seen_at ?? '').localeCompare(b.last_seen_at ?? '')
        break
      case 'version':
        cmp = (a.agent_version ?? '').localeCompare(b.agent_version ?? '')
        break
    }
    return sortDir.value === 'desc' ? -cmp : cmp
  })

  return list
})

function toggleSort(field: SortField): void {
  if (sortField.value === field) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortField.value = field
    sortDir.value = 'asc'
  }
}

const showAddDialog = ref(false)
const addForm = reactive({ hostname: '', display_name: '' })
const addLoading = ref(false)
const addError = ref<string | null>(null)
const newToken = ref<string | null>(null)
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

// Adopt imported agent
const showAdoptDialog = ref(false)
const adoptToken = ref<string | null>(null)
const adoptHostname = ref('')

const showDeployDialog = ref(false)
const deployTarget = ref<AgentRow | null>(null)

// Merge imported agent
const showMergeDialog = ref(false)
const mergeSource = ref<AgentRow | null>(null)

useEscapeKey(showMergeDialog, () => {
  showMergeDialog.value = false
})

useEscapeKey(showAddDialog, closeAddDialog)

function isOnline(agent: AgentRow): boolean {
  return agent.is_connected ?? false
}

function isImported(agent: AgentRow): boolean {
  return agent.is_imported ?? false
}

function formatLastSeen(iso: string | null | undefined): string {
  if (!iso) return 'Never'
  const ts = new Date(iso).getTime()
  if (isNaN(ts) || ts === 0) return 'Never'
  const diff = Date.now() - ts
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'Just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  const days = Math.floor(hrs / 24)
  return `${days}d ago`
}

function formatVersion(v: string | null | undefined): string {
  if (!v) return '\u2014'
  return v
}

function scheduleCount(agent: AgentRow): number {
  return machineScheduleCount.value[agent.id] ?? 0
}

function agentTags(agent: AgentRow): { name: string; color: string }[] {
  return agentTagsMap.value[agent.id] ?? []
}

function agentHealthStatus(agent: AgentRow): AgentHealth | null {
  return healthByHost.value[agent.hostname] ?? null
}

function agentHasIssues(agent: AgentRow): boolean {
  const h = agentHealthStatus(agent)
  if (!h) return false
  return h.failed > 0 || h.overdue > 0
}

function agentIssueLabel(agent: AgentRow): string {
  const h = agentHealthStatus(agent)
  if (!h) return ''
  const parts: string[] = []
  if (h.failed > 0) parts.push(`${h.failed} failed`)
  if (h.overdue > 0) parts.push(`${h.overdue} overdue`)
  return parts.join(', ')
}

function toggleTagFilter(tagId: number): void {
  const idx = filterTagIds.value.indexOf(tagId)
  if (idx === -1) {
    filterTagIds.value = [...filterTagIds.value, tagId]
  } else {
    filterTagIds.value = filterTagIds.value.filter((id) => id !== tagId)
  }
}

async function loadAgents(): Promise<void> {
  if (agents.value.length === 0) {
    loading.value = true
    error.value = null
  }
  try {
    const agentsRes = await apiClient.get<AgentRow[]>('/agents', {
      params: showHidden.value ? { include_hidden: true } : undefined,
    })
    agents.value = agentsRes.data
    error.value = null
    loading.value = false

    const emptyOverview: DashboardOverview = {
      summary: {
        protected_hosts: 0,
        eligible_hosts: 0,
        needs_attention: 0,
        running_operations: 0,
        total_storage_bytes: 0,
      },
      findings: [],
      protection: {
        protected_hosts: 0,
        eligible_hosts: 0,
        protected_agent_links: [],
        unassigned_agents: [],
        never_succeeded_targets: 0,
        never_succeeded_agents: [],
        disabled_only_agents: [],
      },
      running_operations: [],
      upcoming_schedules: [],
      repository_capacity: [],
    }
    const [agentTagAssocRes, agentTagsRes, healthRes, scheduleCountsRes, overviewRes] =
      await Promise.all([
        apiClient
          .get<AgentTagRow[]>('/agent-tags', { timeout: 8000 })
          .catch(() => ({ data: [] as AgentTagRow[] })),
        apiClient
          .get<TagRow[]>('/tags', { params: { scope: 'host' }, timeout: 8000 })
          .catch(() => ({ data: [] as TagRow[] })),
        apiClient
          .get<HealthEntry[]>('/stats/health', { timeout: 8000 })
          .catch(() => ({ data: [] as HealthEntry[] })),
        apiClient
          .get<{ agent_id: number; count: number }[]>('/stats/schedule-counts', { timeout: 8000 })
          .catch(() => ({ data: [] as { agent_id: number; count: number }[] })),
        apiClient
          .get<DashboardOverview>('/stats/dashboard-overview', { timeout: 8000 })
          .catch(() => ({ data: emptyOverview })),
      ])
    machineScheduleCount.value = {}
    scheduleCountsRes.data.forEach((entry) => {
      machineScheduleCount.value[entry.agent_id] = entry.count
    })

    allAgentTags.value = agentTagsRes.data
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    agentTagAssocRes.data.forEach((ht) => {
      if (!tagMap[ht.agent_id]) tagMap[ht.agent_id] = []
      tagMap[ht.agent_id].push({ name: ht.tag_name, color: ht.tag_color })
    })
    agentTagsMap.value = tagMap

    const hMap: Record<string, AgentHealth> = {}
    healthRes.data.forEach((entry) => {
      if (!hMap[entry.hostname]) {
        hMap[entry.hostname] = {
          failed: 0,
          overdue: 0,
          warning: 0,
          total: 0,
          last_error_message: null,
        }
      }
      hMap[entry.hostname].total++
      const status = entry.last_status !== null ? normalizeBackupStatus(entry.last_status) : null
      if (status === 'failed') {
        hMap[entry.hostname].failed++
        if (entry.last_error_message) {
          hMap[entry.hostname].last_error_message = entry.last_error_message
        }
      }
      if (status === 'warning') hMap[entry.hostname].warning++
      if (entry.is_overdue) hMap[entry.hostname].overdue++
    })
    healthByHost.value = hMap
    coverageHostIds.value = {
      protected: new Set(
        overviewRes.data.protection.protected_agent_links.map((host) => host.agent_id),
      ),
      unassigned: new Set(
        overviewRes.data.protection.unassigned_agents.map((host) => host.agent_id),
      ),
      'never-succeeded': new Set(
        overviewRes.data.protection.never_succeeded_agents.map((host) => host.agent_id),
      ),
      'disabled-only': new Set(
        overviewRes.data.protection.disabled_only_agents.map((host) => host.agent_id),
      ),
    }
    const runningMap: Record<string, string[]> = {}
    overviewRes.data.running_operations.forEach((op) => {
      const list = runningMap[op.hostname] ?? []
      if (!list.includes(op.repo_name)) list.push(op.repo_name)
      runningMap[op.hostname] = list
    })
    activeBackupsByHost.value = runningMap
  } catch (e: unknown) {
    if (agents.value.length === 0) {
      error.value = extractError(e)
    }
  } finally {
    loading.value = false
  }
}

function openAddDialog(): void {
  addForm.hostname = ''
  addForm.display_name = ''
  addError.value = null
  newToken.value = null
  tokenCopied.value = false
  showAddDialog.value = true
}

async function submitAdd(): Promise<void> {
  const hostname = addForm.hostname.replaceAll(/\s/g, '')
  if (!hostname) {
    addError.value = 'Hostname is required'
    return
  }
  addLoading.value = true
  addError.value = null
  try {
    const res = await apiClient.post<CreateAgentResponse>('/agents', {
      hostname,
      display_name: addForm.display_name.trim() || null,
    })
    agents.value.push({ ...res.data.agent, id: Number(res.data.agent.id) })
    newToken.value = res.data.token
  } catch (e: unknown) {
    addError.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

function closeAddDialog(): void {
  showAddDialog.value = false
  newToken.value = null
}

function navigateToAgent(agent: AgentRow): void {
  router.push(`/agents/${agent.hostname}`)
}

async function adoptAgent(agent: AgentRow): Promise<void> {
  try {
    const cleanDisplayName = agent.display_name?.replace(/\s*\(imported\)$/, '').trim() || null
    await apiClient.put(`/agents/${agent.hostname}`, {
      display_name: cleanDisplayName,
    })
    const res = await apiClient.post<CreateAgentResponse>(
      `/agents/${agent.hostname}/regenerate-token`,
    )
    const idx = agents.value.findIndex((m) => m.id === agent.id)
    if (idx !== -1) {
      agents.value[idx] = {
        ...agents.value[idx],
        ...res.data.agent,
        id: Number(res.data.agent.id),
        is_imported: false,
        display_name: cleanDisplayName,
      }
    }
    adoptHostname.value = agent.hostname
    adoptToken.value = res.data.token
    tokenCopied.value = false
    showAdoptDialog.value = true
  } catch (e: unknown) {
    logger.error('Failed to adopt agent', e)
  }
}

function openDeployDialog(agent: AgentRow): void {
  deployTarget.value = agent
  showDeployDialog.value = true
}

function openMergeDialog(agent: AgentRow): void {
  mergeSource.value = agent
  showMergeDialog.value = true
}

async function unhideAgent(agent: AgentRow): Promise<void> {
  try {
    await apiClient.put(`/agents/${agent.hostname}/unhide`)
    await loadAgents()
  } catch (e: unknown) {
    logger.error('Failed to unhide agent', e)
  }
}

function onMerged(): void {
  showMergeDialog.value = false
  loadAgents().catch(logger.error)
}

onMounted(() => {
  loadAgents().catch(logger.error)
  apiClient
    .get<{ agent_version: string | null; server_commit_count: number | null }>('/system/version')
    .then((res) => {
      availableAgentVersion.value = res.data.agent_version
      serverCommitCount.value = res.data.server_commit_count ?? null
    })
    .catch(logger.error)
})

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('AgentConnected', () => loadAgents().catch(logger.error))
onMessage('AgentDisconnected', () => loadAgents().catch(logger.error))
onMessage('DataChanged', () => loadAgents().catch(logger.error))

const availableAgentVersion = ref<string | null>(null)
const serverCommitCount = ref<number | null>(null)
onMessage('BackupStarted', (payload) => {
  const list = activeBackupsByHost.value[payload.hostname] ?? []
  if (!list.includes(payload.target_name)) {
    activeBackupsByHost.value = {
      ...activeBackupsByHost.value,
      [payload.hostname]: [...list, payload.target_name],
    }
  }
})

onMessage('BackupCompleted', (payload) => {
  const list = activeBackupsByHost.value[payload.hostname]
  if (list) {
    const filtered = list.filter((t) => t !== payload.target_name)
    if (filtered.length === 0) {
      const copy = { ...activeBackupsByHost.value }
      delete copy[payload.hostname]
      activeBackupsByHost.value = copy
    } else {
      activeBackupsByHost.value = { ...activeBackupsByHost.value, [payload.hostname]: filtered }
    }
  }
  loadAgents().catch(logger.error)
})

function hostActiveBackups(agent: AgentRow): string[] {
  return activeBackupsByHost.value[agent.hostname] ?? []
}

function deployButtonLabel(agent: AgentRow): string | null {
  if (!agent.agent_version) return 'Deploy'
  const commitCount = agent.agent_commit_count ?? null
  if (serverCommitCount.value !== null && commitCount !== null) {
    return commitCount >= serverCommitCount.value ? null : 'Upgrade'
  }
  if (!availableAgentVersion.value) return null
  return agent.agent_version === availableAgentVersion.value ? null : 'Upgrade'
}

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    loadAgents().catch(logger.error)
  }
})

watch(showHidden, () => {
  loadAgents().catch(logger.error)
})

watch(
  () => route.query.coverage,
  (coverage) => {
    filterCoverage.value = coverageFilterFromQuery(coverage)
  },
  { immediate: true },
)
</script>

<template>
  <div class="hosts-view">
    <div class="page-header">
      <h1 class="page-title">Agents</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openAddDialog"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <div class="toolbar">
      <input
        v-model="filterText"
        class="input search-input"
        placeholder="Filter by hostname or tag..."
      />
      <button
        v-if="isMobile"
        class="btn-filter-toggle"
        :class="{
          active: filterStatus !== 'all' || filterCoverage !== 'all' || filterTagIds.length > 0,
        }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterStatus !== 'all' || filterCoverage !== 'all' || filterTagIds.length > 0"
          class="filter-badge"
        ></span>
      </button>
      <template v-if="!isMobile || showMobileFilters">
        <select
          v-model="filterStatus"
          class="input select-input"
        >
          <option value="all">All</option>
          <option value="online">Online</option>
          <option value="offline">Offline</option>
        </select>
        <select
          v-model="filterCoverage"
          class="input select-input"
          aria-label="Coverage"
        >
          <option value="all">All coverage</option>
          <option value="protected">Protected</option>
          <option value="unassigned">Unassigned</option>
          <option value="never-succeeded">Never succeeded</option>
          <option value="disabled-only">Disabled schedules only</option>
        </select>
        <div
          v-if="isAdmin"
          class="hidden-toggle"
        >
          <ToggleSwitch v-model="showHidden" />
          <span class="hidden-toggle-label">Show hidden</span>
        </div>
        <div
          v-if="allAgentTags.length > 0"
          class="tag-filter-wrapper"
        >
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: filterTagIds.length > 0 }"
            @click="showTagDropdown = !showTagDropdown"
          >
            Tags{{ filterTagIds.length > 0 ? ` (${filterTagIds.length})` : '' }}
            <span class="dropdown-arrow">{{ showTagDropdown ? '\u25B4' : '\u25BE' }}</span>
          </button>
          <div
            v-if="showTagDropdown"
            class="tag-dropdown"
          >
            <label
              v-for="tag in allAgentTags"
              :key="tag.id"
              class="tag-dropdown-item"
            >
              <input
                type="checkbox"
                :checked="filterTagIds.includes(tag.id)"
                @change="toggleTagFilter(tag.id)"
              />
              <span
                class="tag-dot"
                :style="{ background: tag.color }"
              ></span>
              <span class="tag-dropdown-name">{{ tag.name }}</span>
            </label>
          </div>
        </div>
        <div class="sort-controls">
          <span class="sort-label">Sort:</span>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'hostname' }"
            @click="toggleSort('hostname')"
          >
            Name {{ sortField === 'hostname' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'status' }"
            @click="toggleSort('status')"
          >
            Status {{ sortField === 'status' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'last_seen' }"
            @click="toggleSort('last_seen')"
          >
            Last Seen
            {{ sortField === 'last_seen' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'version' }"
            @click="toggleSort('version')"
          >
            Version {{ sortField === 'version' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
        </div>
      </template>
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
      v-else-if="agents.length === 0"
      :icon="Server"
      title="No agents registered"
      description="Add your first agent to start backing up."
      action="Add Agent"
      @action="showAddDialog = true"
    />
    <div
      v-else-if="filteredAgents.length === 0"
      class="state-msg"
    >
      No agents match the current filter.
    </div>

    <div
      v-else
      class="host-grid"
    >
      <div
        v-for="agent in filteredAgents"
        :key="agent.id"
        class="host-card"
        :class="{ 'host-card-hidden': agent.is_hidden }"
        @click="navigateToAgent(agent)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-hostname">{{ agent.hostname }}</span>
            <span
              v-if="agent.display_name"
              class="card-display"
              >{{ agent.display_name }}</span
            >
          </div>
          <div class="card-top-badges">
            <span
              v-if="agent.is_hidden"
              class="badge-hidden"
            >
              Hidden
            </span>
            <span
              v-if="isImported(agent)"
              class="badge-imported"
            >
              Imported
            </span>
            <span
              class="status-badge"
              :class="isOnline(agent) ? 'status-online' : 'status-offline'"
            >
              {{ isOnline(agent) ? 'Online' : 'Offline' }}
            </span>
          </div>
        </div>
        <div class="card-stats">
          <div class="stat">
            <span class="stat-value">{{ scheduleCount(agent) }}</span>
            <span class="stat-label">Schedules</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatLastSeen(agent.last_seen_at) }}</span>
            <span class="stat-label">Last seen</span>
          </div>
          <div class="stat">
            <span class="stat-value mono">{{ formatVersion(agent.agent_version) }}</span>
            <span class="stat-label">Agent</span>
          </div>
        </div>
        <CardError
          v-if="agentHasIssues(agent) && agentHealthStatus(agent)!.last_error_message"
          :label="agentIssueLabel(agent)"
          :message="agentHealthStatus(agent)!.last_error_message!"
        />
        <div
          v-else-if="agentHasIssues(agent)"
          class="card-health-issues"
        >
          <AlertCircle :size="12" />
          <span
            v-if="agentHealthStatus(agent)!.failed > 0"
            class="issue-text issue-failed"
          >
            {{ agentHealthStatus(agent)!.failed }} failed
          </span>
          <span
            v-if="agentHealthStatus(agent)!.overdue > 0"
            class="issue-text issue-overdue"
          >
            {{ agentHealthStatus(agent)!.overdue }} overdue
          </span>
        </div>
        <div
          v-if="hostActiveBackups(agent).length > 0"
          class="card-active-backup"
        >
          <span class="active-pulse" />
          <span class="active-text"> Backing up: {{ hostActiveBackups(agent).join(', ') }} </span>
        </div>
        <div
          v-if="agentTags(agent).length > 0"
          class="card-tags"
        >
          <span
            v-for="tag in agentTags(agent)"
            :key="tag.name"
            class="tag-pill"
            :style="{
              background: tag.color + '22',
              color: tag.color,
              borderColor: tag.color + '44',
            }"
          >
            {{ tag.name }}
          </span>
        </div>
        <div
          class="card-actions"
          @click.stop
        >
          <template v-if="agent.is_hidden">
            <button
              class="btn btn-sm btn-ghost"
              @click="unhideAgent(agent)"
            >
              Unhide
            </button>
          </template>
          <template v-else>
            <button
              v-if="isImported(agent)"
              class="btn btn-sm btn-ghost"
              @click="openMergeDialog(agent)"
            >
              Merge into...
            </button>
            <button
              v-if="isImported(agent)"
              class="btn btn-sm btn-ghost"
              @click="adoptAgent(agent)"
            >
              Adopt
            </button>
            <button
              v-if="deployButtonLabel(agent) && !isImported(agent)"
              class="btn btn-sm btn-ghost"
              @click="openDeployDialog(agent)"
            >
              {{ deployButtonLabel(agent) }}
            </button>
          </template>
        </div>
      </div>
    </div>

    <!-- Add Agent Dialog -->
    <Teleport to="body">
      <div
        v-if="showAddDialog"
        class="overlay"
        @click.self="closeAddDialog"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ newToken ? 'Agent Created' : 'Add Agent' }}
            </h2>
            <button
              class="close-btn"
              @click="closeAddDialog"
            >
              &times;
            </button>
          </div>

          <template v-if="!newToken">
            <div class="dialog-body">
              <div class="field">
                <label class="field-label">Hostname <span class="required">*</span></label>
                <input
                  v-model="addForm.hostname"
                  class="input"
                  placeholder="e.g. workstation-01"
                  @keyup.enter="submitAdd"
                />
                <span class="field-hint"
                  >Must match the machine's actual hostname (output of <code>hostname</code>).</span
                >
              </div>
              <div class="field">
                <label class="field-label">Display Name</label>
                <input
                  v-model="addForm.display_name"
                  class="input"
                  placeholder="Optional friendly name"
                />
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
                @click="closeAddDialog"
              >
                Cancel
              </button>
              <button
                class="btn btn-primary"
                :disabled="addLoading || !addForm.hostname.trim()"
                @click="submitAdd"
              >
                {{ addLoading ? 'Creating...' : 'Create' }}
              </button>
            </div>
          </template>

          <template v-else>
            <div class="dialog-body">
              <div class="token-notice">
                <p class="token-warning">Copy this agent token now. It will not be shown again.</p>
                <div class="token-box">
                  <code class="token-text">{{ newToken }}</code>
                  <button
                    class="btn btn-sm btn-ghost"
                    @click="copyToClipboard(newToken ?? '')"
                  >
                    {{ tokenCopied ? 'Copied!' : 'Copy' }}
                  </button>
                </div>
              </div>
            </div>
            <div class="dialog-footer">
              <button
                class="btn btn-primary"
                @click="closeAddDialog"
              >
                Done
              </button>
            </div>
          </template>
        </div>
      </div>
    </Teleport>

    <!-- Adopt Agent Dialog -->
    <Teleport to="body">
      <div
        v-if="showAdoptDialog"
        class="overlay"
        @click.self="showAdoptDialog = false"
      >
        <div class="dialog dialog-sm">
          <div class="dialog-header">
            <h2 class="dialog-title">Agent Adopted &mdash; {{ adoptHostname }}</h2>
            <button
              class="close-btn"
              @click="showAdoptDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <div class="token-notice">
              <p class="token-warning">Copy this agent token now. It will not be shown again.</p>
              <div class="token-box">
                <code class="token-text">{{ adoptToken }}</code>
                <button
                  class="btn btn-sm btn-ghost"
                  @click="copyToClipboard(adoptToken ?? '')"
                >
                  {{ tokenCopied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showAdoptDialog = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Deploy Agent Dialog -->
    <AgentDeployDialog
      v-if="showDeployDialog && deployTarget"
      :hostname="deployTarget.hostname"
      :agent-version="deployTarget.agent_version ?? null"
      :last-ssh-user="deployTarget.last_ssh_user"
      @close="showDeployDialog = false"
      @deployed="
        (version) => {
          if (version && deployTarget) {
            const agent = agents.find((a) => a.hostname === deployTarget!.hostname)
            if (agent) agent.agent_version = version
          }
          showDeployDialog = false
          loadAgents()
        }
      "
    />

    <!-- Merge Agent Dialog -->
    <Teleport to="body">
      <MergeAgentDialog
        v-if="showMergeDialog && mergeSource"
        :source="mergeSource"
        :all-agents="agents"
        @merged="onMerged"
        @cancel="showMergeDialog = false"
      />
    </Teleport>
  </div>
</template>

<style scoped>
.hosts-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
  flex-wrap: wrap;
}

.search-input {
  width: 220px;
}

.select-input {
  width: auto;
  min-width: 100px;
}

.btn-filter-toggle {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.4rem 0.6rem;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  position: relative;
  transition:
    color 0.15s,
    border-color 0.15s;
}

.btn-filter-toggle:hover {
  color: var(--text-primary);
  border-color: var(--text-muted);
}

.btn-filter-toggle.active {
  border-color: var(--accent);
  color: var(--accent);
}

.filter-badge {
  position: absolute;
  top: -3px;
  right: -3px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
}

.sort-controls {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  margin-left: auto;
  overflow-x: auto;
  flex-shrink: 0;
}

.sort-label {
  font-size: 0.75rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-right: 0.25rem;
}

.sort-controls .btn.active {
  background: var(--bg-hover);
  color: var(--text-primary);
  font-weight: 600;
}

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.host-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(320px, 100%), 1fr));
  gap: 1rem;
}

.host-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  cursor: pointer;
  transition:
    box-shadow 0.15s,
    border-color 0.15s;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.host-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.card-info {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.card-hostname {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-display {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.card-stats {
  display: flex;
  gap: 1.5rem;
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.stat-value {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
}

.stat-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.25rem;
  margin-top: auto;
}

/* Tag pills */
.card-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
}

.card-health-issues {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  color: var(--danger);
  font-size: 0.75rem;
  font-weight: 500;
}

.issue-text {
  font-size: 0.72rem;
}

.issue-failed {
  color: var(--danger);
}

.issue-overdue {
  color: var(--warning);
}

.card-active-backup {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.72rem;
  color: var(--accent);
}

.active-pulse {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: pulse 1.5s ease-in-out infinite;
  flex-shrink: 0;
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

.active-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tag-pill {
  display: inline-flex;
  align-items: center;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 500;
  border: 1px solid;
}

/* Tag filter dropdown */
.tag-filter-wrapper {
  position: relative;
}

.dropdown-arrow {
  font-size: 0.65rem;
  margin-left: 0.15rem;
}

.tag-dropdown {
  position: absolute;
  top: 100%;
  left: 0;
  margin-top: 0.35rem;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  padding: 0.5rem;
  min-width: 160px;
  z-index: 50;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.tag-dropdown-item {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.4rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 0.8rem;
  color: var(--text-secondary);
  transition: background 0.1s;
}

.tag-dropdown-item:hover {
  background: var(--bg-hover);
}

.tag-dropdown-item input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
}

.tag-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tag-dropdown-name {
  white-space: nowrap;
}

/* Overlay & Dialog */

.token-notice {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.token-warning {
  color: var(--warning);
  font-size: 0.875rem;
  font-weight: 500;
}

.token-box {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem 1rem;
}

.token-text {
  flex: 1;
  font-family: var(--mono);
  font-size: 0.78rem;
  color: var(--success);
  word-break: break-all;
  background: transparent;
  padding: 0;
}

.exclude-area {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: 0.82rem;
  line-height: 1.5;
}

.card-top-badges {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
}

.badge-imported {
  display: inline-block;
  padding: 0.15rem 0.45rem;
  border-radius: 999px;
  font-size: 0.68rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  background: var(--accent-subtle);
  color: var(--accent);
  border: 1px solid var(--accent);
}

.badge-hidden {
  display: inline-block;
  padding: 0.15rem 0.45rem;
  border-radius: 999px;
  font-size: 0.68rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  background: var(--bg-hover);
  color: var(--text-muted);
  border: 1px solid var(--border);
}

.host-card-hidden {
  opacity: 0.6;
}

.hidden-toggle {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  cursor: pointer;
  font-size: 0.8rem;
  color: var(--text-secondary);
  user-select: none;
}

.hidden-toggle input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
}

.hidden-toggle-label {
  white-space: nowrap;
}
</style>
