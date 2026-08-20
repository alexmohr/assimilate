<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import {
  listAgents,
  createAgent,
  updateAgent,
  regenerateAgentToken,
  unhideAgent as unhideAgentRequest,
} from '../api/agents'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { useMobile } from '../composables/useMobile'
import { useListSort } from '../composables/useListSort'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { cronIntervalSecs } from '../utils/cadence'
import { Plus, SlidersHorizontal, Server } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
import AgentCoverageMeter from '../components/AgentCoverageMeter.vue'
import type { DashboardOverview } from '../types/dashboard'
import type { AgentRow } from '../types/agent'
import type { TagRow } from '../types/tag'
import BaseModal from '../components/BaseModal.vue'

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
  cron_expression?: string | null
  schedule_enabled?: boolean | null
}

interface AgentHealth {
  failed: number
  overdue: number
  warning: number
  total: number
  last_error_message: string | null
  /** Most recent completed backup across this host's schedules. */
  mostRecentBackupAt: string | null
  /** Shortest cadence among this host's enabled backup schedules, in seconds. */
  minCadenceSecs: number | null
}

type SortField = 'hostname' | 'status' | 'last_seen' | 'version'

const SORT_OPTIONS: readonly { field: SortField; label: string }[] = [
  { field: 'hostname', label: 'Name' },
  { field: 'status', label: 'Status' },
  { field: 'last_seen', label: 'Last seen' },
  { field: 'version', label: 'Version' },
]
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
const fleetScheduleCount = ref(0)
const healthByHost = ref<Record<string, AgentHealth>>({})
const loading = ref(false)
const error = ref<string | null>(null)

const {
  field: sortField,
  direction: sortDir,
  toggle: toggleSort,
  sign: sortSign,
} = useListSort<SortField>('hostname')
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
    return cmp * sortSign()
  })

  return list
})

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

function agentCoverage(agent: AgentRow): {
  lastBackupAt: string | null
  cadenceSecs: number | null
} {
  const h = agentHealthStatus(agent)
  return { lastBackupAt: h?.mostRecentBackupAt ?? null, cadenceSecs: h?.minCadenceSecs ?? null }
}

interface FleetVersionCount {
  version: string
  count: number
  current: boolean
}

const fleetSummary = computed(() => {
  const total = agents.value.length
  const online = agents.value.filter((a) => a.is_connected).length
  const totalSchedules = fleetScheduleCount.value

  const versionCounts = new Map<string, number>()
  for (const a of agents.value) {
    const v = a.agent_version ?? 'unknown'
    versionCounts.set(v, (versionCounts.get(v) ?? 0) + 1)
  }
  const versions: FleetVersionCount[] = [...versionCounts.entries()]
    .map(([version, count]) => ({
      version,
      count,
      current: availableAgentVersion.value !== null && version === availableAgentVersion.value,
    }))
    .sort((a, b) => b.count - a.count)

  return { total, online, totalSchedules, versions }
})

function agentIssues(agent: AgentRow): EntityIssue[] {
  const h = agentHealthStatus(agent)
  if (!h) return []
  const issues: EntityIssue[] = []
  if (h.failed > 0) {
    issues.push({
      key: 'failed',
      label: `${h.failed} failed`,
      severity: 'danger',
      onClick: () => navigateToAgentIssue(agent, 'failed'),
    })
  }
  if (h.overdue > 0) {
    issues.push({
      key: 'overdue',
      label: `${h.overdue} overdue`,
      severity: 'warning',
      onClick: () => navigateToAgentIssue(agent, 'overdue'),
    })
  }
  return issues
}

function navigateToAgentIssue(agent: AgentRow, kind: 'failed' | 'overdue'): void {
  const query =
    kind === 'failed'
      ? { tab: 'backups', status: 'failed' }
      : { tab: 'schedules', health: 'overdue' }
  router.push({ path: `/agents/${agent.hostname}`, query })
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
    agents.value = await listAgents(showHidden.value)
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
    const [
      agentTagAssocRes,
      agentTagsRes,
      healthRes,
      scheduleCountsRes,
      schedulesRes,
      overviewRes,
    ] = await Promise.all([
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
        .get<{ id: number }[]>('/schedules', { timeout: 8000 })
        .catch(() => ({ data: [] as { id: number }[] })),
      apiClient
        .get<DashboardOverview>('/stats/dashboard-overview', { timeout: 8000 })
        .catch(() => ({ data: emptyOverview })),
    ])
    machineScheduleCount.value = {}
    scheduleCountsRes.data.forEach((entry) => {
      machineScheduleCount.value[entry.agent_id] = entry.count
    })
    // Fleet-wide total is the distinct schedule count, not a sum of
    // per-agent counts - a schedule targeting N agents would otherwise be
    // counted N times.
    fleetScheduleCount.value = schedulesRes.data.length

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
          mostRecentBackupAt: null,
          minCadenceSecs: null,
        }
      }
      const host = hMap[entry.hostname]
      host.total++
      const status = entry.last_status !== null ? normalizeBackupStatus(entry.last_status) : null
      if (status === 'failed') {
        host.failed++
        if (entry.last_error_message) {
          host.last_error_message = entry.last_error_message
        }
      }
      if (status === 'warning') host.warning++
      if (entry.is_overdue) host.overdue++

      if (
        entry.last_backup_at &&
        (status === 'success' || status === 'warning') &&
        (!host.mostRecentBackupAt || entry.last_backup_at > host.mostRecentBackupAt)
      ) {
        host.mostRecentBackupAt = entry.last_backup_at
      }
      if (entry.schedule_enabled && entry.cron_expression) {
        const secs = cronIntervalSecs(entry.cron_expression)
        if (secs !== null) {
          host.minCadenceSecs =
            host.minCadenceSecs === null ? secs : Math.min(host.minCadenceSecs, secs)
        }
      }
    })
    healthByHost.value = hMap

    const activeMap: Record<string, string[]> = {}
    overviewRes.data.running_operations.forEach((op) => {
      const list = activeMap[op.hostname] ?? []
      if (!list.includes(op.repo_name)) list.push(op.repo_name)
      activeMap[op.hostname] = list
    })
    activeBackupsByHost.value = activeMap

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
    const res = await createAgent({
      hostname,
      display_name: addForm.display_name.trim() || null,
    })
    agents.value.push({ ...res.agent, id: Number(res.agent.id) })
    newToken.value = res.token
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
    await updateAgent(agent.hostname, {
      display_name: cleanDisplayName,
    })
    const res = await regenerateAgentToken(agent.hostname)
    const idx = agents.value.findIndex((m) => m.id === agent.id)
    if (idx !== -1) {
      agents.value[idx] = {
        ...agents.value[idx],
        ...res.agent,
        id: Number(res.agent.id),
        is_imported: false,
        display_name: cleanDisplayName,
      }
    }
    adoptHostname.value = agent.hostname
    adoptToken.value = res.token
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
    await unhideAgentRequest(agent.hostname)
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

const activeBackupsByHost = ref<Record<string, string[]>>({})

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

function hostRunningLabel(agent: AgentRow): string {
  const targets = hostActiveBackups(agent)
  return targets.length > 0 ? `Backing up: ${targets.join(', ')}` : 'Running'
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
        class="filter-toggle"
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
          class="input select-input select-input--sm"
        >
          <option value="all">All</option>
          <option value="online">Online</option>
          <option value="offline">Offline</option>
        </select>
        <select
          v-model="filterCoverage"
          class="input select-input select-input--sm"
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
        <SortControls
          :field="sortField"
          :direction="sortDir"
          :options="SORT_OPTIONS"
          @toggle="toggleSort"
        />
      </template>
    </div>

    <div
      v-if="!loading && agents.length > 0"
      class="fleet-summary"
    >
      <div class="fleet-summary-row">
        <span class="fleet-summary-title">Fleet</span>
        <span class="fleet-summary-counts">
          {{ fleetSummary.total }} agent{{ fleetSummary.total === 1 ? '' : 's' }} ·
          {{ fleetSummary.online }} online · {{ fleetSummary.totalSchedules }} schedule{{
            fleetSummary.totalSchedules === 1 ? '' : 's'
          }}
        </span>
      </div>
      <div class="fleet-version-row">
        <span
          v-for="v in fleetSummary.versions"
          :key="v.version"
          class="fleet-version-chip"
          :class="{ 'fleet-version-chip-current': v.current }"
        >
          {{ v.version }}{{ v.current ? ' (current)' : '' }}: {{ v.count }}
        </span>
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
      v-else-if="agents.length === 0"
      :icon="Server"
      title="No agents registered"
      description="Add your first agent to start backing up."
      action="New agent"
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
      class="card-grid"
    >
      <div
        v-for="agent in filteredAgents"
        :key="agent.id"
        class="entity-card"
        :class="{
          'entity-card--hidden': agent.is_hidden,
          'entity-card--notable': !isOnline(agent),
        }"
        @click="navigateToAgent(agent)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-name">{{ agent.hostname }}</span>
            <span
              v-if="agent.display_name"
              class="card-display"
              >{{ agent.display_name }}</span
            >
          </div>
          <div class="card-top-badges">
            <span
              v-if="agent.is_hidden"
              class="badge badge--neutral"
            >
              Hidden
            </span>
            <span
              v-if="isImported(agent)"
              class="badge badge--accent"
            >
              Imported
            </span>
          </div>
        </div>
        <AgentCoverageMeter
          :last-backup-at="agentCoverage(agent).lastBackupAt"
          :cadence-secs="agentCoverage(agent).cadenceSecs"
        />
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
        <EntityStatusBadges
          :notable="!isOnline(agent)"
          notable-label="Offline"
          :running="hostActiveBackups(agent).length > 0"
          :running-label="hostRunningLabel(agent)"
          :issues="agentIssues(agent)"
        />
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
              v-if="deployButtonLabel(agent) && !isImported(agent) && authStore.canUpgradeAgent"
              class="btn btn-sm btn-ghost"
              @click="openDeployDialog(agent)"
            >
              {{ deployButtonLabel(agent) }}
            </button>
          </template>
        </div>
      </div>
    </div>

    <!-- Add Agent Dialog. One dialog, two states: collect the hostname,
         then reveal the generated token once. -->
    <BaseModal
      :open="showAddDialog"
      :title="newToken ? 'Agent Created' : 'Add Agent'"
      @close="closeAddDialog"
    >
      <template v-if="!newToken">
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
          <label class="field-label">Display name</label>
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
      </template>

      <div
        v-else
        class="token-notice"
      >
        <p class="token-warning">Copy this agent token now. It will not be shown again.</p>
        <div class="token-box">
          <code class="token-text">{{ newToken }}</code>
          <button
            type="button"
            class="btn btn-sm btn-ghost"
            @click="copyToClipboard(newToken ?? '')"
          >
            {{ tokenCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </div>

      <template #footer>
        <template v-if="!newToken">
          <button
            type="button"
            class="btn btn-ghost"
            @click="closeAddDialog"
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn btn-primary"
            :disabled="addLoading || !addForm.hostname.trim()"
            @click="submitAdd"
          >
            {{ addLoading ? 'Creating...' : 'Create' }}
          </button>
        </template>
        <button
          v-else
          type="button"
          class="btn btn-primary"
          @click="closeAddDialog"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <!-- Adopt Agent Dialog -->
    <BaseModal
      :open="showAdoptDialog"
      size="sm"
      @close="showAdoptDialog = false"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Agent Adopted &mdash; {{ adoptHostname }}
        </h2>
      </template>
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

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showAdoptDialog = false"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <!-- Deploy Agent Dialog -->
    <AgentDeployDialog
      v-if="showDeployDialog && deployTarget"
      :hostname="deployTarget.hostname"
      :agent-version="deployTarget.agent_version ?? null"
      :available-version="availableAgentVersion"
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
    <MergeAgentDialog
      v-if="showMergeDialog && mergeSource"
      :source="mergeSource"
      :all-agents="agents"
      @merged="onMerged"
      @cancel="showMergeDialog = false"
    />
  </div>
</template>

<style scoped>
.hosts-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

.card-display {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

/* Tag pills */
.card-tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

/* Tag filter dropdown */

/* Overlay & Dialog */

.card-top-badges {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}

.hidden-toggle {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  cursor: pointer;
  font-size: var(--fs-sm);
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

.fleet-summary {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-5) var(--space-6) var(--space-6);
  margin-bottom: var(--space-7);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.fleet-summary-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
}

.fleet-summary-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.fleet-summary-counts {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.fleet-version-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.fleet-version-chip {
  display: inline-flex;
  align-items: center;
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  padding: var(--space-1) var(--space-4);
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  color: var(--text-secondary);
}

.fleet-version-chip-current {
  color: var(--success);
}
</style>
