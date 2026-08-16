<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { useElapsedClock } from '../composables/useElapsedTimer'
import { formatDate, formatDateShort, formatBytes, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import BaseSpinner from '../components/BaseSpinner.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import SshKeyDeployPanel from '../components/SshKeyDeployPanel.vue'
import BackupProgressCard from '../components/BackupProgressCard.vue'
import type { EntityIssue } from '../components/EntityStatusBadges.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRow } from '../types/schedule'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { scheduleIssuesFromEntries, type ScheduleHealthEntry } from '../utils/scheduleHealth'
import type { CreateAgentResponse } from '../types/generated'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs from '../components/BaseTabs.vue'
import { backupStatusBadgeClass } from '../utils/badge'
import { X, CalendarClock } from '@lucide/vue'
import EmptyState from '../components/EmptyState.vue'
import ScheduleCard from '../components/ScheduleCard.vue'
import EntityTags from '../components/EntityTags.vue'
import AgentDefaultsCards from '../components/AgentDefaultsCards.vue'
import AgentHostnameAliases from '../components/AgentHostnameAliases.vue'
import AgentDangerZone from '../components/AgentDangerZone.vue'

type TabId = 'overview' | 'schedules' | 'backups'

const props = defineProps<{ hostname: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab as string | undefined
    if (t === 'schedules' || t === 'backups') return t
    return 'overview'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const tabs: { id: TabId; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'schedules', label: 'Schedules' },
  { id: 'backups', label: 'Backups' },
]

const agent = ref<AgentRow | null>(null)
const repos = ref<Repo[]>([])
const schedules = ref<ScheduleRow[]>([])
const reports = ref<ReportRow[]>([])
const scheduleHealth = ref<ScheduleHealthEntry[]>([])
const { loading, error, run } = useAsyncAction()
const expandedReportId = ref<number | null>(null)

// Backup filter / sort
function isRunStatusFilter(value: unknown): value is 'success' | 'warning' | 'failed' {
  return value === 'success' || value === 'warning' || value === 'failed'
}
const filterStatus = ref<'all' | 'success' | 'warning' | 'failed'>(
  isRunStatusFilter(route.query.status) ? route.query.status : 'all',
)
const sortAscending = ref(false)

const highlightedArchiveName = computed(() => {
  const a = route.query.archive
  return typeof a === 'string' ? a : undefined
})

const pinnedStatus = computed(() => {
  const s = route.query.status
  return isRunStatusFilter(s) ? s : undefined
})
const pinnedReportId = ref<number | null>(null)
const pinnedForStatus = ref<typeof pinnedStatus.value>(undefined)

function isOverdueQuery(value: unknown): value is 'overdue' {
  return value === 'overdue'
}
const overdueHighlighted = computed(() => isOverdueQuery(route.query.health))

const filteredSortedReports = computed(() => {
  let result = reports.value
  if (filterStatus.value !== 'all') {
    result = result.filter((r) => normalizeBackupStatus(r.status) === filterStatus.value)
  }
  return [...result].sort((a, b) => {
    const diff = new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
    return sortAscending.value ? -diff : diff
  })
})

// Tags

const isAdmin = computed(() => authStore.isAdmin)
const isImported = computed(() => agent.value?.is_imported ?? false)

/** Merge back the fields the defaults panel just wrote, leaving the rest. */
function onDefaultsSaved(updated: AgentRow): void {
  agent.value = agent.value ? { ...agent.value, ...updated } : updated
}

// Merge dialog
const allAgents = ref<AgentRow[]>([])
const showMergeDialog = ref(false)

useEscapeKey(showMergeDialog, () => {
  showMergeDialog.value = false
})

// Token regen
const showTokenDialog = ref(false)
const regenToken = ref<string | null>(null)
const regenLoading = ref(false)
const regenError = ref<string | null>(null)
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

// Restart agent
const restartLoading = ref(false)
const restartError = ref<string | null>(null)

// Deploy/Upgrade agent
const availableAgentVersion = ref<string | null>(null)
const serverCommitCount = ref<number | null>(null)
const showDeployDialog = ref(false)

// Deploy SSH key
const showDeploySshKey = ref(false)

function deployButtonLabel(): string | null {
  if (!agent.value) return null
  if (!agent.value.agent_version) return 'Deploy'
  const commitCount = agent.value.agent_commit_count ?? null
  if (serverCommitCount.value !== null && commitCount !== null) {
    return commitCount >= serverCommitCount.value ? null : 'Upgrade'
  }
  if (!availableAgentVersion.value) return null
  return agent.value.agent_version === availableAgentVersion.value ? null : 'Upgrade'
}

// Hostname & display name editing
const editingIdentity = ref(false)
const identityHostname = ref('')
const identityDisplayName = ref('')
const identitySaving = ref(false)
const identityError = ref<string | null>(null)

function startEditIdentity(): void {
  if (!agent.value) return
  identityHostname.value = agent.value.hostname
  identityDisplayName.value = agent.value.display_name ?? ''
  identityError.value = null
  editingIdentity.value = true
}

function cancelEditIdentity(): void {
  editingIdentity.value = false
}

async function saveIdentity(): Promise<void> {
  if (!agent.value) return
  identitySaving.value = true
  identityError.value = null
  try {
    const oldHostname = agent.value.hostname
    const newHostname = identityHostname.value.trim()
    const hostnameChanged = newHostname !== oldHostname && newHostname.length > 0
    const res = await apiClient.put<AgentRow>(`/agents/${oldHostname}`, {
      hostname: hostnameChanged ? newHostname : undefined,
      display_name: identityDisplayName.value.trim() || null,
      default_backup_paths: agent.value.default_backup_paths,
      default_exclude_patterns: agent.value.default_exclude_patterns,
      default_pre_backup_commands: agent.value.default_pre_backup_commands,
      default_post_backup_commands: agent.value.default_post_backup_commands,
      default_file_change_patterns_raw: agent.value.default_file_change_patterns_raw,
    })
    if (hostnameChanged) {
      pendingAliasOldHostname.value = oldHostname
      pendingAliasNewHostname.value = newHostname
      showAliasConfirm.value = true
      router.replace(`/agents/${newHostname}`)
    }
    agent.value = { ...agent.value, ...res.data }
    editingIdentity.value = false
  } catch (e: unknown) {
    identityError.value = extractError(e)
  } finally {
    identitySaving.value = false
  }
}

// Hostname alias confirmation
const aliasesPanel = ref<InstanceType<typeof AgentHostnameAliases> | null>(null)
const showAliasConfirm = ref(false)
const pendingAliasOldHostname = ref('')
const pendingAliasNewHostname = ref('')

useEscapeKey(showAliasConfirm, () => {
  showAliasConfirm.value = false
})

async function confirmAddAlias(): Promise<void> {
  await apiClient.post(`/agents/${pendingAliasNewHostname.value}/hostname-patterns`, {
    pattern: pendingAliasOldHostname.value,
  })
  await aliasesPanel.value?.reload(pendingAliasNewHostname.value)
  showAliasConfirm.value = false
}

function declineAlias(): void {
  showAliasConfirm.value = false
}
async function adoptHost(): Promise<void> {
  if (!agent.value) return
  try {
    const cleanDisplayName =
      agent.value.display_name?.replace(/\s*\(imported\)$/, '').trim() || null
    await apiClient.put(`/agents/${agent.value.hostname}`, {
      display_name: cleanDisplayName,
    })
    const res = await apiClient.post<CreateAgentResponse>(
      `/agents/${agent.value.hostname}/regenerate-token`,
    )
    agent.value = {
      ...agent.value,
      ...res.data.agent,
      id: Number(res.data.agent.id),
      is_imported: false,
      display_name: cleanDisplayName,
    }
    regenToken.value = res.data.token
    tokenCopied.value = false
    showTokenDialog.value = true
  } catch (e: unknown) {
    logger.error('Failed to adopt host', e)
  }
}

function openMergeDialog(): void {
  showMergeDialog.value = true
}

function onMerged(): void {
  showMergeDialog.value = false
  router.push('/agents')
}

useEscapeKey(showTokenDialog, () => {
  showTokenDialog.value = false
})

function isOnline(agent: AgentRow): boolean {
  return agent.is_connected ?? false
}

function handleResultClick(r: ReportRow): void {
  if (normalizeBackupStatus(r.status) === 'success') {
    const query: Record<string, string> = { tab: 'archives' }
    if (r.archive_name) {
      query.archive = r.archive_name
    }
    router.push({ path: `/repos/${r.repo_id}`, query })
  } else {
    expandedReportId.value = expandedReportId.value === r.id ? null : r.id
  }
}

async function loadAgent(): Promise<void> {
  await run(async () => {
    const res = await apiClient.get<AgentRow[]>('/agents')
    allAgents.value = res.data
    agent.value = res.data.find((m) => m.hostname === props.hostname) ?? null
    if (!agent.value) {
      throw new Error(`Agent "${props.hostname}" not found`)
    }
    await loadTabData()
  })
}

async function loadTabData(): Promise<void> {
  if (!agent.value) return
  const hostname = agent.value.hostname
  try {
    const [repoRes, schedRes, reportRes, healthRes] = await Promise.all([
      apiClient.get<Repo[]>(`/agents/${hostname}/repos`),
      apiClient.get<ScheduleRow[]>('/schedules'),
      apiClient.get<ReportRow[]>(`/agents/${hostname}/reports`),
      apiClient.get<ScheduleHealthEntry[]>('/stats/health'),
    ])
    repos.value = repoRes.data
    schedules.value = schedRes.data
    reports.value = reportRes.data
    scheduleHealth.value = healthRes.data.filter((h) => h.hostname === hostname)
    const runningReports = reportRes.data.filter((r) => {
      const status = normalizeBackupStatus(r.status)
      return status === 'pending' || status === 'started'
    })
    runningReports.forEach((r) => {
      if (!r.repo_name || activeBackups.value.some((b) => b.targetName === r.repo_name)) return
      const startedAt = new Date(r.started_at).getTime()
      activeBackups.value = [
        ...activeBackups.value,
        {
          targetName: r.repo_name,
          archiveName: r.archive_name,
          startedAt,
          progress: null,
        },
      ]
    })
  } catch (e: unknown) {
    logger.error('loadTabData failed', e)
  }
}

watch(
  [reports, pinnedStatus],
  ([, status]) => {
    if (!status) {
      if (pinnedReportId.value !== null && expandedReportId.value === pinnedReportId.value) {
        expandedReportId.value = null
      }
      pinnedForStatus.value = undefined
      pinnedReportId.value = null
      return
    }
    if (pinnedForStatus.value === status && pinnedReportId.value !== null) return
    const match = [...reports.value]
      .filter((r) => normalizeBackupStatus(r.status) === status)
      .sort((a, b) => new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime())[0]
    if (!match) return
    pinnedForStatus.value = status
    pinnedReportId.value = match.id
    expandedReportId.value = match.id
    nextTick(() => {
      document
        .getElementById(`report-${match.id}`)
        ?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    })
  },
  { immediate: true },
)

watch(
  [reports, highlightedArchiveName],
  ([, archiveName]) => {
    if (!archiveName) return
    const report = reports.value.find((r) => r.archive_name === archiveName)
    if (!report) return
    expandedReportId.value = report.id
    nextTick(() => {
      document
        .getElementById(`report-${report.id}`)
        ?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    })
  },
  { immediate: true },
)

const agentSchedules = computed(() => {
  const hostname = agent.value?.hostname
  return hostname ? schedules.value.filter((s) => s.target_hostnames.includes(hostname)) : []
})

function scheduleHealthEntries(s: ScheduleRow): ScheduleHealthEntry[] {
  return scheduleHealth.value.filter((h) => h.schedule_id === s.id)
}

function scheduleIssues(s: ScheduleRow): EntityIssue[] {
  return scheduleIssuesFromEntries(scheduleHealthEntries(s), s.id, router)
}

function repoNameForSchedule(s: ScheduleRow): string {
  return (
    repos.value.find((r) => r.id === s.repo_id)?.name ??
    (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
  )
}

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
}

function goToActivityLog(): void {
  router.push(`/activity?category=backup&hostname=${encodeURIComponent(props.hostname)}`)
}

// Token regeneration
async function regenerateToken(): Promise<void> {
  regenLoading.value = true
  regenError.value = null
  regenToken.value = null
  tokenCopied.value = false
  try {
    const res = await apiClient.post<{ agent: AgentRow; token: string }>(
      `/agents/${props.hostname}/regenerate-token`,
    )
    regenToken.value = res.data.token
    agent.value = res.data.agent
    showTokenDialog.value = true
  } catch (e: unknown) {
    regenError.value = extractError(e)
    showTokenDialog.value = true
  } finally {
    regenLoading.value = false
  }
}

async function restartAgent(): Promise<void> {
  restartLoading.value = true
  restartError.value = null
  try {
    await apiClient.post(`/agents/${props.hostname}/restart`)
  } catch (e: unknown) {
    restartError.value = extractError(e)
  } finally {
    restartLoading.value = false
  }
}

watch(
  () => props.hostname,
  () => {
    loadAgent()
  },
)
onMounted(() => {
  loadAgent()
  apiClient
    .get<{ agent_version: string | null; server_commit_count: number | null }>('/system/version')
    .then((res) => {
      availableAgentVersion.value = res.data.agent_version
      serverCommitCount.value = res.data.server_commit_count ?? null
    })
    .catch(logger.error)
})

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('DataChanged', () => loadAgent().catch(logger.error))
onMessage('AgentConnected', () => loadAgent().catch(logger.error))
onMessage('AgentDisconnected', () => loadAgent().catch(logger.error))

interface ArchiveProgressData {
  nfiles: number
  originalSize: number
  currentPath: string
}

interface ActiveBackup {
  targetName: string
  archiveName: string | null
  startedAt: number
  progress: ArchiveProgressData | null
}

const activeBackups = ref<ActiveBackup[]>([])
const hasActiveBackups = computed(() => activeBackups.value.length > 0)
const { now } = useElapsedClock(hasActiveBackups)

function elapsedSecsFor(backup: ActiveBackup): number {
  return Math.max(0, Math.floor((now.value - backup.startedAt) / 1000))
}

interface BorgArchiveProgress {
  type: 'archive_progress'
  nfiles: number
  original_size: number
  path: string
}

function parseArchiveProgress(raw: string): BorgArchiveProgress | null {
  try {
    const obj = JSON.parse(raw) as Record<string, unknown>
    if (obj['type'] === 'archive_progress') return obj as unknown as BorgArchiveProgress
    return null
  } catch {
    return null
  }
}

onMessage('BackupStarted', (payload) => {
  if (payload.hostname !== props.hostname) return
  if (activeBackups.value.some((b) => b.targetName === payload.target_name)) return
  activeBackups.value = [
    ...activeBackups.value,
    {
      targetName: payload.target_name,
      archiveName: payload.archive_name ?? null,
      startedAt: Date.now(),
      progress: null,
    },
  ]
})

onMessage('BackupCompleted', (payload) => {
  if (payload.hostname === props.hostname) {
    activeBackups.value = activeBackups.value.filter((b) => b.targetName !== payload.target_name)
  }
  loadAgent().catch(logger.error)
})

onMessage('BackupLog', (payload) => {
  if (payload.hostname !== props.hostname) return
  const targetName = repos.value.find((r) => r.id === payload.repo_id)?.name
  if (!targetName) return
  const backup = activeBackups.value.find((b) => b.targetName === targetName)
  if (!backup) return
  const progress = parseArchiveProgress(payload.line)
  if (progress !== null) {
    backup.progress = {
      nfiles: progress.nfiles,
      originalSize: progress.original_size,
      currentPath: progress.path ?? '',
    }
  }
})

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    loadAgent().catch(logger.error)
  }
})
</script>

<template>
  <div class="host-detail">
    <!-- Breadcrumb -->
    <nav class="breadcrumb">
      <RouterLink
        to="/agents"
        class="crumb-link"
      >
        Agents
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ props.hostname }}</span>
    </nav>

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

    <template v-else-if="agent">
      <!-- Tab bar -->
      <BaseTabs
        v-model="activeTab"
        :tabs="tabs"
        label="Agent sections"
      />

      <!-- Overview Tab -->
      <div
        v-if="activeTab === 'overview'"
        class="tab-content"
      >
        <div class="info-card">
          <h3 class="info-title">Agent Information</h3>
          <dl class="info-grid">
            <dt>Hostname</dt>
            <dd class="mono">
              {{ agent.hostname }}
            </dd>
            <dt>Display Name</dt>
            <dd>{{ agent.display_name ?? '—' }}</dd>
            <dt>Status</dt>
            <dd>
              <span
                class="badge"
                :class="isOnline(agent) ? 'badge--success' : 'badge--neutral'"
              >
                {{ isOnline(agent) ? 'Online' : 'Offline' }}
              </span>
            </dd>
            <dt>Agent Version</dt>
            <dd class="mono">
              {{ agent.agent_version ?? '—' }}
            </dd>
            <dt>Revision</dt>
            <dd class="mono">
              {{ agent.agent_git_sha ?? '—' }}
            </dd>
            <dt>Built</dt>
            <dd class="mono">
              {{ agent.agent_build_time ?? '—' }}
            </dd>
            <dt>Created</dt>
            <dd>{{ formatDate(agent.created_at ?? null, 'Never') }}</dd>
            <dt>Last Seen</dt>
            <dd>{{ formatDate(agent.last_seen_at ?? null, 'Never') }}</dd>
            <dt>Repositories</dt>
            <dd>{{ repos.length }}</dd>
          </dl>
          <BackupProgressCard
            v-for="b in activeBackups"
            :key="b.targetName"
            :badge="b.targetName"
            :archive-name="b.archiveName"
            :elapsed-secs="elapsedSecsFor(b)"
            :estimated-remaining-secs="null"
            :progress="b.progress"
          />
          <div class="info-actions">
            <button
              v-if="isImported"
              class="btn btn-sm btn-primary"
              @click="openMergeDialog"
            >
              Merge into...
            </button>
            <button
              v-if="isImported"
              class="btn btn-sm btn-primary"
              @click="adoptHost"
            >
              Adopt
            </button>
            <button
              v-if="!isImported"
              class="btn btn-sm btn-ghost"
              @click="goToActivityLog"
            >
              Activity Log
            </button>
            <button
              v-if="!isImported"
              class="btn btn-sm btn-ghost"
              @click="startEditIdentity"
            >
              Edit
            </button>
            <button
              v-if="!isImported"
              class="btn btn-sm btn-ghost"
              :disabled="regenLoading"
              @click="regenerateToken"
            >
              {{ regenLoading ? 'Regenerating...' : 'Regenerate Token' }}
            </button>
            <button
              v-if="agent.supports_restart && !isImported"
              class="btn btn-sm btn-ghost btn-danger-text"
              :disabled="restartLoading || !isOnline(agent)"
              @click="restartAgent"
            >
              {{ restartLoading ? 'Restarting...' : 'Restart Agent' }}
            </button>
            <span
              v-else-if="isOnline(agent) && agent.restart_unavailable_reason"
              class="restart-hint"
            >
              {{ agent.restart_unavailable_reason }}
            </span>
            <button
              v-if="deployButtonLabel() && !isImported && authStore.canUpgradeAgent"
              class="btn btn-sm btn-ghost"
              @click="showDeployDialog = true"
            >
              {{ deployButtonLabel() }}
            </button>
            <button
              v-if="!isImported"
              class="btn btn-sm btn-ghost"
              @click="showDeploySshKey = true"
            >
              Deploy SSH Key
            </button>
            <div
              v-if="restartError"
              class="form-error"
            >
              {{ restartError }}
            </div>
          </div>
        </div>

        <!-- Deploy SSH Key -->
        <div
          v-if="showDeploySshKey && !isImported"
          class="info-card"
        >
          <div class="info-title-row">
            <h3 class="info-title">Deploy SSH Key</h3>
            <button
              class="btn btn-sm btn-ghost"
              aria-label="Close"
              @click="showDeploySshKey = false"
            >
              <X :size="14" />
            </button>
          </div>
          <SshKeyDeployPanel
            :ssh-host="agent.hostname"
            show-credentials
          />
        </div>

        <!-- Edit Identity -->
        <div
          v-if="editingIdentity"
          class="info-card"
        >
          <h3 class="info-title">Edit Agent Identity</h3>
          <div class="field">
            <label class="field-label">Hostname</label>
            <input
              v-model="identityHostname"
              class="input"
              placeholder="hostname"
              @keyup.enter="saveIdentity"
            />
          </div>
          <div class="field">
            <label class="field-label">Display Name</label>
            <input
              v-model="identityDisplayName"
              class="input"
              placeholder="Optional friendly name"
              @keyup.enter="saveIdentity"
            />
          </div>
          <div
            v-if="identityError"
            class="form-error"
          >
            {{ identityError }}
          </div>
          <div class="info-actions">
            <button
              class="btn btn-ghost"
              @click="cancelEditIdentity"
            >
              Cancel
            </button>
            <button
              class="btn btn-primary"
              :disabled="identitySaving"
              @click="saveIdentity"
            >
              {{ identitySaving ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </div>

        <!-- Tags -->
        <EntityTags
          v-if="isAdmin"
          scope="host"
          :entity-path="`/agents/${props.hostname}`"
        />

        <AgentDefaultsCards
          :agent="agent"
          :can-edit="!isImported"
          @saved="onDefaultsSaved"
        />

        <AgentHostnameAliases
          ref="aliasesPanel"
          :hostname="agent.hostname"
          :can-edit="!isImported"
        />

        <AgentDangerZone
          v-if="isAdmin"
          :agent="agent"
        />
      </div>

      <!-- Schedules Tab -->
      <div
        v-if="activeTab === 'schedules'"
        class="tab-content"
      >
        <div class="tab-header">
          <h3 class="tab-title">Schedules</h3>
          <RouterLink
            :to="{ name: 'schedule-create', query: { agent_id: agent?.id } }"
            class="btn btn-primary btn-sm"
          >
            + Add Schedule
          </RouterLink>
        </div>
        <EmptyState
          v-if="agentSchedules.length === 0"
          :icon="CalendarClock"
          title="No schedules yet"
          description="This agent has no backup schedules. Create one to start backing it up."
        />
        <div
          v-else
          class="schedule-grid"
        >
          <ScheduleCard
            v-for="s in agentSchedules"
            :key="s.id"
            :schedule="s"
            :issues="scheduleIssues(s)"
            :format-run="formatDateShort"
            :highlighted="overdueHighlighted && scheduleHealthEntries(s).some((h) => h.is_overdue)"
            @select="navigateToSchedule(s)"
          >
            <template #title>{{ s.name || repoNameForSchedule(s) }}</template>
          </ScheduleCard>
        </div>
      </div>

      <!-- Backups Tab -->
      <div
        v-if="activeTab === 'backups'"
        class="tab-content"
      >
        <div class="tab-header">
          <h3 class="tab-title">Backup History</h3>
          <div class="backup-controls">
            <div class="filter-group">
              <button
                v-for="s in ['all', 'success', 'warning', 'failed'] as const"
                :key="s"
                class="btn btn-sm"
                :class="filterStatus === s ? 'btn-primary' : 'btn-ghost'"
                @click="filterStatus = s"
              >
                {{ s === 'all' ? 'All' : s.charAt(0).toUpperCase() + s.slice(1) }}
              </button>
            </div>
            <button
              class="btn btn-sm btn-ghost"
              @click="sortAscending = !sortAscending"
            >
              {{ sortAscending ? '↑ Oldest' : '↓ Newest' }}
            </button>
          </div>
        </div>
        <div
          v-if="filteredSortedReports.length === 0"
          class="state-msg"
        >
          {{
            reports.length === 0
              ? 'No backup reports available.'
              : 'No backups match the current filter.'
          }}
        </div>
        <div
          v-else
          class="results-list"
        >
          <div
            v-for="r in filteredSortedReports"
            :id="`report-${r.id}`"
            :key="r.id"
            class="result-card"
            :class="[
              `result-${r.status}`,
              {
                'result-card-link': r.status === 'success',
                'result-card-highlighted':
                  r.archive_name === highlightedArchiveName || r.id === pinnedReportId,
              },
            ]"
            @click="handleResultClick(r)"
          >
            <div class="result-header">
              <span
                class="badge"
                :class="backupStatusBadgeClass(r.status)"
                >{{ r.status }}</span
              >
              <span class="result-date">{{ relativeTime(r.finished_at) }}</span>
              <span class="result-duration">{{ r.duration_secs }}s</span>
            </div>
            <div class="result-meta">
              <span class="result-repo">{{ r.repo_name }}</span>
              <RouterLink
                v-if="r.schedule_id && r.schedule_name && r.schedule_name !== r.repo_name"
                :to="`/schedules/${r.schedule_id}`"
                class="result-schedule-link"
                @click.stop
              >
                {{ r.schedule_name }}
              </RouterLink>
            </div>
            <div class="result-stats">
              <span>{{ formatBytes(r.original_size) }} original</span>
              <span>{{ formatBytes(r.deduplicated_size) }} dedup</span>
              <span>{{ r.files_processed }} files</span>
            </div>
            <template v-if="expandedReportId === r.id">
              <div
                v-if="(r.warnings ?? []).length > 0"
                class="result-warnings"
              >
                <strong class="result-section-label">Warnings</strong>
                <pre class="result-output">{{ (r.warnings ?? []).join('\n') }}</pre>
              </div>
              <div
                v-if="r.error_message && normalizeBackupStatus(r.status) !== 'warning'"
                class="result-error"
              >
                <strong class="result-section-label">Error</strong>
                <pre class="result-output">{{ r.error_message }}</pre>
              </div>
            </template>
            <span
              v-if="r.status === 'success'"
              class="result-link-hint"
              >View archives →</span
            >
            <span
              v-else-if="r.error_message || (r.warnings ?? []).length > 0"
              class="result-expand-hint"
              >{{ expandedReportId === r.id ? 'Click to collapse' : 'Click to expand' }}</span
            >
          </div>
        </div>
      </div>
    </template>

    <!-- Token Dialog -->
    <BaseModal
      :open="showTokenDialog"
      :title="regenToken ? 'New Token Generated' : 'Error'"
      @close="showTokenDialog = false"
    >
      <template v-if="regenToken">
        <p class="token-warning">Copy this token now. It will not be shown again.</p>
        <div class="token-box">
          <code class="token-text">{{ regenToken }}</code>
          <button
            class="btn btn-sm btn-ghost"
            @click="copyToClipboard(regenToken ?? '')"
          >
            {{ tokenCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </template>
      <div
        v-else-if="regenError"
        class="form-error"
      >
        {{ regenError }}
      </div>

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showTokenDialog = false"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <!-- Hostname Alias Confirmation Dialog -->
    <BaseModal
      :open="showAliasConfirm"
      title="Add Hostname Pattern?"
      @close="declineAlias"
    >
      <p>
        Hostname changed from <strong>{{ pendingAliasOldHostname }}</strong> to
        <strong>{{ pendingAliasNewHostname }}</strong
        >.
      </p>
      <p>
        Add <code>{{ pendingAliasOldHostname }}</code> as an alternative hostname pattern so
        existing archives still match?
      </p>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="declineAlias"
        >
          No
        </button>
        <button
          class="btn btn-primary"
          @click="confirmAddAlias"
        >
          Add Pattern
        </button>
      </template>
    </BaseModal>

    <!-- Merge Agent Dialog -->
    <MergeAgentDialog
      v-if="showMergeDialog && agent"
      :source="agent"
      :all-agents="allAgents"
      @merged="onMerged"
      @cancel="showMergeDialog = false"
    />

    <!-- Deploy Agent Dialog -->
    <AgentDeployDialog
      v-if="showDeployDialog && agent"
      :hostname="agent.hostname"
      :agent-version="agent.agent_version ?? null"
      :last-ssh-user="agent.last_ssh_user"
      @close="showDeployDialog = false"
      @deployed="
        () => {
          showDeployDialog = false
          loadAgent()
        }
      "
    />
  </div>
</template>

<style scoped>
.host-detail {
  max-width: 1100px;
}

.breadcrumb {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
  font-size: var(--fs-base);
}

.crumb-link {
  color: var(--accent);
  text-decoration: none;
  font-weight: 500;
}

.crumb-link:hover {
  color: var(--accent-hover);
}

.crumb-sep {
  color: var(--text-muted);
}

.crumb-current {
  color: var(--text-primary);
  font-weight: 600;
  font-family: var(--mono);
}

.state-error {
  color: var(--danger);
}

/* Tab bar */

.tab-content {
  animation: fadeIn 0.15s ease;
}

@keyframes fadeIn {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

.tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1.25rem;
}

.tab-title {
  font-size: var(--fs-lg);
  font-weight: 600;
}

/* Info card (Overview) */
.restart-hint {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  font-style: italic;
}

/* Tags */

/* Repos grid */

/* Schedule cards */
.schedule-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(320px, 100%), 1fr));
  gap: 1rem;
}

/* Results list */
.results-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.result-card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  background: var(--bg-card);
}

.result-card.result-failed {
  border-left: 3px solid var(--danger);
}

.result-card.result-warning {
  border-left: 3px solid var(--warning);
}

.result-card.result-success {
  border-left: 3px solid var(--success);
}

.result-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 0.5rem;
}

.result-date {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.result-duration {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-left: auto;
}

.result-meta {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-bottom: 0.35rem;
}

.result-schedule-link {
  color: var(--text-muted);
}

.result-schedule-link:hover {
  color: var(--accent);
  text-decoration: underline;
}

.result-stats {
  display: flex;
  gap: 1rem;
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.result-warnings,
.result-error {
  margin-top: 0.5rem;
}

.result-output {
  font-size: var(--fs-2xs);
  background: var(--bg-code);
  border-radius: var(--radius-sm);
  padding: 0.5rem;
  margin-top: 0.25rem;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 12rem;
}

.result-error .result-output {
  color: var(--danger);
}

.result-card-link {
  cursor: pointer;
}

.result-card-link:hover {
  background: var(--bg-hover);
}

.result-card:not(.result-card-link) {
  cursor: pointer;
}

.result-card:not(.result-card-link):hover {
  background: var(--bg-hover);
}

.result-link-hint {
  font-size: var(--fs-2xs);
  color: var(--accent);
  margin-top: 0.4rem;
  display: block;
}

.result-expand-hint {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  margin-top: 0.4rem;
  display: block;
}

.result-section-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  display: block;
  margin-bottom: 0.25rem;
}

.result-warnings .result-section-label {
  color: var(--warning);
}

.result-error .result-section-label {
  color: var(--danger);
}

.result-card-highlighted {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.backup-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.filter-group {
  display: flex;
  gap: 0.25rem;
}

/* Overlay & Dialog */

/* Form */

.input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.token-warning {
  color: var(--warning);
  font-size: var(--fs-base);
  font-weight: 500;
  margin-bottom: 0.75rem;
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
  font-size: var(--fs-xs);
  color: var(--success);
  word-break: break-all;
  background: transparent;
  padding: 0;
}

/* Danger zone */

.info-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.info-title-row .info-title {
  margin-bottom: 0;
}
</style>
