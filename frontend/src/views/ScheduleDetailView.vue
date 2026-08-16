<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { formatDateShort } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import { useElapsedClock } from '../composables/useElapsedTimer'
import { parseLines } from '../utils/validation'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { isAgentOffline, lastSeenText } from '../utils/agent'
import { AlertTriangle, ExternalLink } from '@lucide/vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import ScheduleAdvancedTab from '../components/ScheduleAdvancedTab.vue'
import ScheduleLogsTab from '../components/ScheduleLogsTab.vue'
import ScheduleBackupsTab from '../components/ScheduleBackupsTab.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import CronBuilder from '../components/CronBuilder.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import BackupProgressCard from '../components/BackupProgressCard.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRow, ScheduleType } from '../types/schedule'
import type { ScheduleBackupSourcesResponse } from '../types/generated'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

interface ScheduleTarget {
  agent_id: number
  execution_order: number
}

const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()

// The route param is either a numeric schedule id or this sentinel for the
// "create new schedule" route.
const NEW_SCHEDULE_ROUTE_ID = 'new'

const isCreate = computed(() => props.id === NEW_SCHEDULE_ROUTE_ID)

const schedule = ref<ScheduleRow | null>(null)
const agents = ref<AgentRow[]>([])
const repos = ref<Repo[]>([])
const repo = computed(() => repos.value.find((r) => r.id === selectedRepoId.value) ?? null)
const scheduleTargets = ref<ScheduleTarget[]>([])
const health = ref<HealthSummaryResponse[]>([])
const { loading, error, run } = useAsyncAction('Failed to load schedule')
const saving = ref(false)
const saveError = ref<string | null>(null)
const saveSuccess = ref(false)
const showDeleteDialog = ref(false)
const deleteLoading = ref(false)
const runNowLoading = ref(false)
const retryingAgentId = ref<number | null>(null)
const cancelLoading = ref(false)
const backupRunning = ref(false)
const reports = ref<ReportRow[]>([])
const reportsLoading = ref(false)
const reportsError = ref<string | null>(null)
const { success: toastSuccess, error: toastError } = useToast()
const { onMessage } = useWebSocket()
const selectedAgentIds = ref<number[]>([])
const selectedRepoId = ref<number | null>(null)
const selectedType = ref<ScheduleType>('backup')
const onFailure = ref<'stop' | 'continue'>('stop')
const usePerHostPaths = ref(false)
const perHostSources = ref<Record<number, string>>({})

// The Advanced tab's per-agent overrides, grouped so the tab component takes
// one v-model instead of seven.
const agentOverrides = ref<ScheduleAgentOverrides>({
  usePerHostExcludes: false,
  perHostExcludes: {},
  usePerHostFileChangePatterns: false,
  perHostFileChangePatterns: {},
  usePerAgentCmds: false,
  perAgentPreCmds: {},
  perAgentPostCmds: {},
})

const showAgentDropdown = ref(false)
const agentDropdownRef = ref<HTMLElement | null>(null)

interface ArchiveProgressData {
  hostname: string
  nfiles: number
  originalSize: number
  currentPath: string
}
const archiveProgress = ref<ArchiveProgressData | null>(null)
const backupHostname = ref<string | null>(null)
const backupArchiveName = ref<string | null>(null)
const backupStartedAt = ref<number | null>(null)
const { now } = useElapsedClock(backupRunning)
const backupElapsedSecs = computed(() =>
  backupStartedAt.value === null
    ? 0
    : Math.max(0, Math.floor((now.value - backupStartedAt.value) / 1000)),
)

const lastSuccessfulReport = computed<ReportRow | null>(
  () =>
    reports.value.find((r) => {
      const status = normalizeBackupStatus(r.status)
      return status === 'success' || status === 'warning'
    }) ?? null,
)

const selectedBackupReport = ref<ReportRow | null>(null)

const estimatedRemainingSecs = computed<number | null>(() => {
  const ref = lastSuccessfulReport.value
  if (!ref || !archiveProgress.value || ref.original_size === 0) return null
  const fraction = archiveProgress.value.originalSize / ref.original_size
  if (fraction <= 0) return null
  const estimatedTotal = backupElapsedSecs.value / fraction
  return Math.max(0, Math.round(estimatedTotal - backupElapsedSecs.value))
})

type TabId = 'settings' | 'advanced' | 'logs' | 'backups'
const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab
    if (t === 'advanced' || t === 'logs' || t === 'backups') return t
    return 'settings'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

function goToLogs(): void {
  const id = schedule.value?.id
  router.push(
    id != null ? `/activity?category=backup&schedule_id=${id}` : '/activity?category=backup',
  )
}

const scheduleType = computed(() =>
  isCreate.value ? selectedType.value : (schedule.value?.schedule_type ?? 'backup'),
)
const isBackup = computed(() => scheduleType.value === 'backup')

const visibleTabs = computed<TabOption<TabId>[]>(() => {
  const tabs: TabOption<TabId>[] = [{ id: 'settings', label: 'Settings' }]
  if (isBackup.value) tabs.push({ id: 'advanced', label: 'Advanced' })
  if (isBackup.value && !isCreate.value) tabs.push({ id: 'backups', label: 'Backups' })
  return tabs
})

const agentMap = computed(() => {
  const m = new Map<number, AgentRow>()
  agents.value.forEach((c) => m.set(c.id, c))
  return m
})

const form = ref<ScheduleFormState>({
  name: '',
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: true,
  exclude_patterns: '',
  file_change_patterns: '',
  ignore_global_excludes: false,
  keep_hourly: 24,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 12,
  keep_yearly: 10,
  compact_enabled: true,
  rate_limit_kbps: 0,
  pre_backup_commands: '',
  post_backup_commands: '',
  backup_sources: '',
})

function agentLabel(id: number): string {
  const c = agents.value.find((x) => x.id === id)
  return c ? (c.display_name ?? c.hostname) : `#${id}`
}

const scheduleHealth = computed<HealthSummaryResponse[]>(() => {
  const scheduleId = schedule.value?.id
  if (scheduleId == null) return []
  return health.value.filter((h) => h.schedule_id === scheduleId)
})

function healthForAgent(agentId: number): HealthSummaryResponse | null {
  const hostname = agentMap.value.get(agentId)?.hostname
  if (!hostname) return null
  return scheduleHealth.value.find((h) => h.hostname === hostname) ?? null
}

function connectivityNote(agentId: number): string {
  const agent = agentMap.value.get(agentId)
  if (!agent || !isAgentOffline(agent)) return ''
  return `Agent offline (${lastSeenText(agent)})`
}

function multiSelectLabel(): string {
  if (selectedAgentIds.value.length === 0) return 'Select agents...'
  if (selectedAgentIds.value.length === 1) return agentLabel(selectedAgentIds.value[0])
  return `${selectedAgentIds.value.length} agents selected`
}

function toggleAgentSelection(id: number): void {
  if (selectedAgentIds.value.includes(id)) {
    selectedAgentIds.value = selectedAgentIds.value.filter((x) => x !== id)
  } else {
    selectedAgentIds.value = [...selectedAgentIds.value, id]
  }
}

function moveAgentUp(index: number): void {
  if (index === 0) return
  const ids = [...selectedAgentIds.value]
  ;[ids[index - 1], ids[index]] = [ids[index], ids[index - 1]]
  selectedAgentIds.value = ids
}

function moveAgentDown(index: number): void {
  if (index >= selectedAgentIds.value.length - 1) return
  const ids = [...selectedAgentIds.value]
  ;[ids[index], ids[index + 1]] = [ids[index + 1], ids[index]]
  selectedAgentIds.value = ids
}

function handleClickOutside(event: MouseEvent): void {
  if (
    showAgentDropdown.value &&
    agentDropdownRef.value &&
    !agentDropdownRef.value.contains(event.target as Node)
  ) {
    showAgentDropdown.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  loadData()
})

onBeforeUnmount(() => {
  document.removeEventListener('click', handleClickOutside)
})

function populateForm(s: ScheduleRow): void {
  form.value = {
    name: s.name,
    cron_expression: s.cron_expression,
    enabled: s.enabled,
    canary_enabled: s.canary_enabled,
    exclude_patterns: s.exclude_patterns_raw ?? '',
    file_change_patterns: s.file_change_patterns_raw ?? '',
    ignore_global_excludes: s.ignore_global_excludes,
    keep_hourly: s.keep_hourly ?? 0,
    keep_daily: s.keep_daily,
    keep_weekly: s.keep_weekly,
    keep_monthly: s.keep_monthly,
    keep_yearly: s.keep_yearly,
    compact_enabled: s.compact_enabled,
    rate_limit_kbps: s.rate_limit_kbps ?? 0,
    pre_backup_commands: s.pre_backup_commands.join('\n'),
    post_backup_commands: s.post_backup_commands.join('\n'),
    backup_sources: '',
  }
  selectedRepoId.value = s.repo_id ?? null
  onFailure.value = s.on_failure
}

function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity Check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}

async function loadData(): Promise<void> {
  await run(async () => {
    if (isCreate.value) {
      const [agentsRes, reposRes] = await Promise.all([
        apiClient.get<AgentRow[]>('/agents'),
        apiClient.get<Repo[]>('/repos'),
      ])
      agents.value = agentsRes.data
      repos.value = reposRes.data
      const queryAgentId = Number(route.query.agent_id)
      if (queryAgentId && agents.value.some((c) => c.id === queryAgentId)) {
        selectedAgentIds.value = [queryAgentId]
      }
      selectedRepoId.value = repos.value.length > 0 ? repos.value[0].id : null
    } else {
      const [schedRes, agentsRes, reposRes, targetsRes, sourcesRes, recentReportsRes, healthRes] =
        await Promise.all([
          apiClient.get<ScheduleRow>(`/schedules/${props.id}`),
          apiClient.get<AgentRow[]>('/agents'),
          apiClient.get<Repo[]>('/repos'),
          apiClient.get<ScheduleTarget[]>(`/schedules/${props.id}/targets`),
          apiClient.get<ScheduleBackupSourcesResponse>(`/schedules/${props.id}/sources`),
          apiClient.get<ReportRow[]>(`/schedules/${props.id}/reports`, { params: { limit: 20 } }),
          apiClient.get<HealthSummaryResponse[]>('/stats/health'),
        ])
      schedule.value = schedRes.data
      agents.value = agentsRes.data
      repos.value = reposRes.data
      scheduleTargets.value = targetsRes.data
      selectedRepoId.value = schedRes.data.repo_id ?? null
      reports.value = recentReportsRes.data
      health.value = healthRes.data
      const runningReport = recentReportsRes.data.find((r) => {
        const status = normalizeBackupStatus(r.status)
        return status === 'pending' || status === 'started'
      })
      backupRunning.value = runningReport !== undefined
      if (runningReport) {
        const agent = agentMap.value.get(runningReport.agent_id ?? 0)
        backupHostname.value = agent?.display_name ?? agent?.hostname ?? null
        backupStartedAt.value = new Date(runningReport.started_at).getTime()
      }
      const sorted = [...targetsRes.data].sort((a, b) => a.execution_order - b.execution_order)
      selectedAgentIds.value = sorted.map((t) => t.agent_id)
      populateForm(schedRes.data)

      const sources = sourcesRes.data
      form.value.backup_sources = (sources.backup_sources ?? []).join('\n')
      const perHost = sources.backup_sources_per_agent ?? []
      if (perHost.length > 0) {
        usePerHostPaths.value = true
        const map: Record<number, string> = {}
        for (const entry of perHost) {
          map[Number(entry.agent_id)] = entry.paths.join('\n')
        }
        perHostSources.value = map
      }
      const perHostExcludeEntries = sources.exclude_patterns_per_agent ?? []
      if (perHostExcludeEntries.length > 0) {
        agentOverrides.value.usePerHostExcludes = true
        const map: Record<number, string> = {}
        for (const entry of perHostExcludeEntries) {
          map[Number(entry.agent_id)] = entry.raw_text
        }
        agentOverrides.value.perHostExcludes = map
      }
      const perHostFileChangePatternsEntries = sources.file_change_patterns_per_agent ?? []
      if (perHostFileChangePatternsEntries.length > 0) {
        agentOverrides.value.usePerHostFileChangePatterns = true
        const map: Record<number, string> = {}
        for (const entry of perHostFileChangePatternsEntries) {
          map[Number(entry.agent_id)] = entry.raw_text
        }
        agentOverrides.value.perHostFileChangePatterns = map
      }
      const perAgentCmdEntries = sources.commands_per_agent ?? []
      if (perAgentCmdEntries.length > 0) {
        agentOverrides.value.usePerAgentCmds = true
        const preMap: Record<number, string> = {}
        const postMap: Record<number, string> = {}
        for (const entry of perAgentCmdEntries) {
          preMap[Number(entry.agent_id)] = entry.pre_backup_commands.join('\n')
          postMap[Number(entry.agent_id)] = entry.post_backup_commands.join('\n')
        }
        agentOverrides.value.perAgentPreCmds = preMap
        agentOverrides.value.perAgentPostCmds = postMap
      }
    }
  })
}

async function save(): Promise<void> {
  saving.value = true
  saveError.value = null
  saveSuccess.value = false
  try {
    const payload: Record<string, unknown> = {
      name: form.value.name,
      cron_expression: form.value.cron_expression,
      enabled: form.value.enabled,
      canary_enabled: form.value.canary_enabled,
      exclude_patterns_raw: form.value.exclude_patterns,
      file_change_patterns_raw: form.value.file_change_patterns,
      ignore_global_excludes: form.value.ignore_global_excludes,
      keep_hourly: form.value.keep_hourly,
      keep_daily: form.value.keep_daily,
      keep_weekly: form.value.keep_weekly,
      keep_monthly: form.value.keep_monthly,
      keep_yearly: form.value.keep_yearly,
      compact_enabled: form.value.compact_enabled,
      rate_limit_kbps: form.value.rate_limit_kbps,
      pre_backup_commands: parseLines(form.value.pre_backup_commands),
      post_backup_commands: parseLines(form.value.post_backup_commands),
      backup_sources: usePerHostPaths.value ? [] : parseLines(form.value.backup_sources),
    }

    if (usePerHostPaths.value) {
      const perHost: { agent_id: number; paths: string[] }[] = []
      for (const id of selectedAgentIds.value) {
        const text = perHostSources.value[id] ?? ''
        const paths = parseLines(text)
        if (paths.length > 0) {
          perHost.push({ agent_id: id, paths })
        }
      }
      payload.backup_sources_per_agent = perHost
    }

    if (agentOverrides.value.usePerHostExcludes) {
      payload.exclude_patterns_raw = ''
      const perHost: { agent_id: number; raw_text: string }[] = []
      for (const id of selectedAgentIds.value) {
        const raw_text = agentOverrides.value.perHostExcludes[id] ?? ''
        perHost.push({ agent_id: id, raw_text })
      }
      payload.exclude_patterns_per_agent = perHost
    }

    if (agentOverrides.value.usePerHostFileChangePatterns) {
      payload.file_change_patterns_raw = ''
      const perHost: { agent_id: number; raw_text: string }[] = []
      for (const id of selectedAgentIds.value) {
        const raw_text = agentOverrides.value.perHostFileChangePatterns[id] ?? ''
        perHost.push({ agent_id: id, raw_text })
      }
      payload.file_change_patterns_per_agent = perHost
    }

    if (agentOverrides.value.usePerAgentCmds) {
      payload.pre_backup_commands = []
      payload.post_backup_commands = []
      const perAgent: {
        agent_id: number
        pre_backup_commands: string[]
        post_backup_commands: string[]
      }[] = []
      for (const id of selectedAgentIds.value) {
        perAgent.push({
          agent_id: id,
          pre_backup_commands: parseLines(agentOverrides.value.perAgentPreCmds[id] ?? ''),
          post_backup_commands: parseLines(agentOverrides.value.perAgentPostCmds[id] ?? ''),
        })
      }
      payload.commands_per_agent = perAgent
    }

    if (isCreate.value) {
      if (selectedAgentIds.value.length === 0 || !selectedRepoId.value) {
        saveError.value = 'Please select at least one agent and a repository.'
        return
      }
      const res = await apiClient.post<ScheduleRow>('/schedules', {
        ...payload,
        agent_ids: selectedAgentIds.value,
        repo_id: selectedRepoId.value,
        schedule_type: selectedType.value,
        on_failure: onFailure.value,
      })
      router.push(`/schedules/${res.data.id}`)
    } else {
      const scheduleId = schedule.value?.id
      if (scheduleId == null) {
        saveError.value = 'Schedule not found'
        return
      }
      const res = await apiClient.put<ScheduleRow>(`/schedules/${scheduleId}`, {
        ...payload,
        agent_ids: selectedAgentIds.value,
        repo_id: selectedRepoId.value,
        on_failure: onFailure.value,
      })
      schedule.value = res.data
      populateForm(res.data)
      saveSuccess.value = true
      setTimeout(() => {
        saveSuccess.value = false
      }, 3000)
    }
  } catch (e: unknown) {
    saveError.value = extractError(e, 'Failed to save schedule')
  } finally {
    saving.value = false
  }
}

async function confirmDeleteSchedule(): Promise<void> {
  deleteLoading.value = true
  try {
    await apiClient.delete(`/schedules/${props.id}`)
    router.push('/schedules')
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to delete schedule')
  } finally {
    deleteLoading.value = false
    showDeleteDialog.value = false
  }
}

async function runNow(agentId?: number): Promise<void> {
  if (agentId != null) {
    retryingAgentId.value = agentId
  } else {
    runNowLoading.value = true
  }
  try {
    await apiClient.post(
      `/schedules/${props.id}/run`,
      agentId != null ? { agent_ids: [agentId] } : {},
    )
    toastSuccess(
      agentId != null
        ? `Retry started for ${agentLabel(agentId)}.`
        : `${scheduleTypeLabel(schedule.value?.schedule_type ?? 'backup')} started.`,
    )
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    if (agentId != null) {
      retryingAgentId.value = null
    } else {
      runNowLoading.value = false
    }
  }
}

async function loadReports(): Promise<void> {
  reportsLoading.value = true
  reportsError.value = null
  try {
    const res = await apiClient.get<ReportRow[]>(`/schedules/${props.id}/reports`, {
      params: { limit: 100 },
    })
    reports.value = res.data
    backupRunning.value = res.data.some((r) => {
      const status = normalizeBackupStatus(r.status)
      return status === 'pending' || status === 'started'
    })
  } catch (e: unknown) {
    reportsError.value = extractError(e, 'Failed to load reports')
  } finally {
    reportsLoading.value = false
  }
}

async function cancelBackup(): Promise<void> {
  cancelLoading.value = true
  try {
    await apiClient.post(`/schedules/${props.id}/cancel`)
    toastSuccess('Cancel request sent.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    cancelLoading.value = false
  }
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
  if (payload.schedule_id != null && payload.schedule_id !== Number(props.id)) return
  if (
    payload.schedule_id == null &&
    !(repo.value != null && payload.target_name === repo.value.name)
  )
    return
  backupRunning.value = true
  backupHostname.value = payload.hostname
  backupArchiveName.value = payload.archive_name ?? null
  archiveProgress.value = null
  backupStartedAt.value = Date.now()
})

onMessage('BackupCompleted', (payload) => {
  if (repo.value != null && payload.target_name === repo.value.name) {
    backupRunning.value = false
    backupHostname.value = null
    backupArchiveName.value = null
  }
})

onMessage('BackupLog', (payload) => {
  // Prefer schedule_id matching so progress arrives even before loadData() resolves
  // selectedRepoId; fall back to repo_id when schedule_id is absent.
  if (payload.schedule_id != null) {
    if (payload.schedule_id !== Number(props.id)) return
  } else if (selectedRepoId.value == null || payload.repo_id !== selectedRepoId.value) {
    return
  }
  const progress = parseArchiveProgress(payload.line)
  if (progress !== null) {
    archiveProgress.value = {
      hostname: payload.hostname,
      nfiles: progress.nfiles,
      originalSize: progress.original_size,
      currentPath: progress.path ?? '',
    }
  }
})

onMessage('DataChanged', () => {
  if (!isCreate.value) {
    loadReports().catch(() => undefined)
  }
})

watch(
  () => props.id,
  () => {
    selectedBackupReport.value = null
    loadData()
  },
)
watch(activeTab, (tab) => {
  if ((tab === 'logs' || tab === 'backups') && !isCreate.value) {
    loadReports().catch(() => undefined)
  }
})
</script>

<template>
  <div class="schedule-detail">
    <nav class="breadcrumb">
      <RouterLink
        to="/schedules"
        class="crumb-link"
      >
        Schedules
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">
        <template v-if="isCreate">New</template>
        <template v-else-if="schedule">{{
          schedule.name || scheduleTypeLabel(schedule.schedule_type)
        }}</template>
        <template v-else>#{{ props.id }}</template>
      </span>
    </nav>

    <div class="page-header">
      <h1 class="page-title">
        <template v-if="isCreate">New Schedule</template>
        <template v-else-if="schedule">
          {{ schedule.name || `${scheduleTypeLabel(schedule.schedule_type)} Schedule` }}
        </template>
        <template v-else>Schedule</template>
      </h1>
      <div
        v-if="!isCreate && schedule"
        class="header-actions"
      >
        <button
          v-if="backupRunning"
          class="btn btn-sm btn-danger"
          :disabled="cancelLoading"
          @click="cancelBackup"
        >
          {{ cancelLoading ? '...' : 'Cancel Backup' }}
        </button>
        <button
          v-else
          class="btn btn-sm btn-primary"
          :disabled="runNowLoading"
          @click="runNow()"
        >
          {{ runNowLoading ? '...' : 'Run Now' }}
        </button>
      </div>
    </div>

    <div
      v-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>

    <BaseSpinner
      v-if="loading && !schedule && !isCreate"
      size="lg"
    />

    <template v-if="schedule || isCreate">
      <BaseTabs
        v-model="activeTab"
        :tabs="visibleTabs"
        label="Schedule sections"
      >
        <template
          v-if="!isCreate"
          #trailing
        >
          <button
            type="button"
            class="tab tab-link"
            @click="goToLogs"
          >
            Logs
            <ExternalLink :size="12" />
          </button>
        </template>
      </BaseTabs>

      <!-- Settings Tab -->
      <div
        v-if="activeTab === 'settings'"
        class="tab-content"
      >
        <div class="form-grid">
          <!-- Schedule Name -->
          <div class="form-card">
            <h3 class="info-title">General</h3>
            <div class="field">
              <label class="field-label">Name</label>
              <input
                v-model="form.name"
                type="text"
                class="input"
                placeholder="e.g. Daily web server backup"
              />
              <span class="field-hint">Optional display name for this schedule</span>
            </div>
          </div>

          <!-- Create-only: target selection -->
          <div
            v-if="isCreate"
            class="form-card"
          >
            <h3 class="info-title">Target</h3>

            <!-- Multi-select for hosts -->
            <div class="field">
              <label class="field-label">Hosts <span class="required">*</span></label>
              <div
                ref="agentDropdownRef"
                class="multi-select-wrapper"
              >
                <button
                  type="button"
                  class="multi-select-trigger"
                  :class="{ open: showAgentDropdown }"
                  @click.stop="showAgentDropdown = !showAgentDropdown"
                >
                  <span class="multi-select-label">{{ multiSelectLabel() }}</span>
                  <span class="multi-select-arrow">{{ showAgentDropdown ? '▲' : '▼' }}</span>
                </button>
                <div
                  v-if="showAgentDropdown"
                  class="multi-select-dropdown"
                >
                  <label
                    v-for="c in agents"
                    :key="c.id"
                    class="multi-select-item"
                  >
                    <input
                      type="checkbox"
                      :checked="selectedAgentIds.includes(c.id)"
                      @change="toggleAgentSelection(c.id)"
                    />
                    <span class="multi-select-name">{{ c.display_name ?? c.hostname }}</span>
                  </label>
                </div>
              </div>
              <span class="field-hint">The agents that will execute this schedule</span>
            </div>

            <!-- On Failure -->
            <div class="field">
              <label class="field-label">On Failure</label>
              <select
                v-model="onFailure"
                class="input form-select"
              >
                <option value="stop">Stop</option>
                <option value="continue">Continue</option>
              </select>
              <span class="field-hint">
                Whether to stop or continue to the next agent when one fails.
              </span>
            </div>

            <!-- Ordering (2+ hosts) -->
            <div
              v-if="selectedAgentIds.length > 1"
              class="field"
            >
              <label class="field-label">Execution Order</label>
              <div class="order-list">
                <div
                  v-for="(agentId, idx) in selectedAgentIds"
                  :key="agentId"
                  class="order-item"
                >
                  <span class="order-index">{{ idx + 1 }}</span>
                  <span class="order-name">{{ agentLabel(agentId) }}</span>
                  <div class="order-actions">
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === 0"
                      title="Move up"
                      @click="moveAgentUp(idx)"
                    >
                      ▲
                    </button>
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === selectedAgentIds.length - 1"
                      title="Move down"
                      @click="moveAgentDown(idx)"
                    >
                      ▼
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <div class="field">
              <label class="field-label">Repository <span class="required">*</span></label>
              <select
                v-model.number="selectedRepoId"
                class="input form-select"
              >
                <option
                  :value="null"
                  disabled
                >
                  Select a repository...
                </option>
                <option
                  v-for="r in repos"
                  :key="r.id"
                  :value="r.id"
                >
                  {{ r.name }}
                </option>
              </select>
              <span class="field-hint">The borg repository to back up to</span>
            </div>
            <div class="field">
              <label class="field-label">Schedule Type</label>
              <select
                v-model="selectedType"
                class="input form-select"
              >
                <option value="backup">Backup</option>
                <option value="check">Integrity Check</option>
                <option value="verify">Verify (extract dry-run)</option>
              </select>
              <span class="field-hint">
                Backup creates archives; Check validates repo integrity; Verify tests
                extractability.
              </span>
            </div>
          </div>

          <!-- Edit-only: info card -->
          <div
            v-if="!isCreate && schedule"
            class="info-card"
          >
            <h3 class="info-title">Schedule Info</h3>
            <div class="info-row info-row-targets">
              <span class="info-label">Targets</span>
              <span
                v-if="selectedAgentIds.length === 0"
                class="info-value"
                >—</span
              >
              <div
                v-else
                class="target-health-list"
              >
                <div
                  v-for="agentId in selectedAgentIds"
                  :key="agentId"
                  class="target-health-row"
                >
                  <span class="target-health-name">{{ agentLabel(agentId) }}</span>
                  <template v-if="healthForAgent(agentId)?.is_overdue">
                    <span class="target-health-badge">
                      <AlertTriangle :size="12" />
                      Overdue
                    </span>
                    <span
                      v-if="connectivityNote(agentId)"
                      class="target-health-note"
                    >
                      {{ connectivityNote(agentId) }}
                    </span>
                    <button
                      class="btn btn-sm btn-ghost"
                      :disabled="retryingAgentId === agentId"
                      @click="runNow(agentId)"
                    >
                      {{ retryingAgentId === agentId ? '...' : 'Retry' }}
                    </button>
                  </template>
                </div>
              </div>
            </div>
            <div class="info-row">
              <span class="info-label">On Failure</span>
              <span class="info-value">
                {{ schedule.on_failure === 'continue' ? 'Continue' : 'Stop' }}
              </span>
            </div>
            <div class="info-row">
              <span class="info-label">Repository</span>
              <span class="info-value">{{
                repo?.name ??
                (schedule.repo_id != null ? `#${schedule.repo_id}` : 'No repository assigned')
              }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Type</span>
              <span class="info-value">{{ scheduleTypeLabel(schedule.schedule_type) }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Next Run</span>
              <span class="info-value">{{ formatDateShort(schedule.next_run_at) ?? 'N/A' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Last Run</span>
              <span class="info-value">{{ formatDateShort(schedule.last_run_at) ?? 'Never' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Cron (human)</span>
              <span class="info-value">{{
                cronToHuman(form.cron_expression) ?? form.cron_expression
              }}</span>
            </div>
            <BackupProgressCard
              v-if="backupRunning"
              :badge="backupHostname"
              :archive-name="backupArchiveName"
              :elapsed-secs="backupElapsedSecs"
              :estimated-remaining-secs="estimatedRemainingSecs"
              :progress="archiveProgress"
            />
          </div>

          <!-- Edit-only: target settings card -->
          <div
            v-if="!isCreate"
            class="form-card"
          >
            <h3 class="info-title">Target Settings</h3>

            <!-- Multi-select for hosts -->
            <div class="field">
              <label class="field-label">Hosts</label>
              <div
                ref="agentDropdownRef"
                class="multi-select-wrapper"
              >
                <button
                  type="button"
                  class="multi-select-trigger"
                  :class="{ open: showAgentDropdown }"
                  @click.stop="showAgentDropdown = !showAgentDropdown"
                >
                  <span class="multi-select-label">{{ multiSelectLabel() }}</span>
                  <span class="multi-select-arrow">{{ showAgentDropdown ? '▲' : '▼' }}</span>
                </button>
                <div
                  v-if="showAgentDropdown"
                  class="multi-select-dropdown"
                >
                  <label
                    v-for="c in agents"
                    :key="c.id"
                    class="multi-select-item"
                  >
                    <input
                      type="checkbox"
                      :checked="selectedAgentIds.includes(c.id)"
                      @change="toggleAgentSelection(c.id)"
                    />
                    <span class="multi-select-name">{{ c.display_name ?? c.hostname }}</span>
                  </label>
                </div>
              </div>
            </div>

            <div class="field">
              <label class="field-label">Repository</label>
              <select
                v-model.number="selectedRepoId"
                class="input form-select"
              >
                <option
                  v-for="r in repos"
                  :key="r.id"
                  :value="r.id"
                >
                  {{ r.name }}
                </option>
              </select>
            </div>

            <!-- On Failure -->
            <div class="field">
              <label class="field-label">On Failure</label>
              <select
                v-model="onFailure"
                class="input form-select"
              >
                <option value="stop">Stop</option>
                <option value="continue">Continue</option>
              </select>
            </div>

            <!-- Ordering (2+ hosts) -->
            <div
              v-if="selectedAgentIds.length > 1"
              class="field"
            >
              <label class="field-label">Execution Order</label>
              <div class="order-list">
                <div
                  v-for="(agentId, idx) in selectedAgentIds"
                  :key="agentId"
                  class="order-item"
                >
                  <span class="order-index">{{ idx + 1 }}</span>
                  <span class="order-name">{{ agentLabel(agentId) }}</span>
                  <div class="order-actions">
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === 0"
                      title="Move up"
                      @click="moveAgentUp(idx)"
                    >
                      ▲
                    </button>
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === selectedAgentIds.length - 1"
                      title="Move down"
                      @click="moveAgentDown(idx)"
                    >
                      ▼
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="form-card">
            <h3 class="info-title">Timing</h3>
            <div class="field">
              <label class="field-label">Schedule</label>
              <CronBuilder v-model="form.cron_expression" />
            </div>
            <div class="field field-inline">
              <label class="field-label">Enabled</label>
              <ToggleSwitch v-model="form.enabled" />
            </div>
          </div>

          <template v-if="isBackup">
            <div class="form-card">
              <h3 class="info-title">Backup Paths</h3>
              <div
                v-if="selectedAgentIds.length > 1"
                class="field field-inline"
              >
                <label class="field-label">Configure per agent</label>
                <ToggleSwitch v-model="usePerHostPaths" />
              </div>

              <div
                v-if="!usePerHostPaths"
                class="field"
              >
                <textarea
                  v-model="form.backup_sources"
                  class="input area-input"
                  placeholder="Directories to back up, one per line"
                  spellcheck="false"
                />
                <span class="field-hint">
                  Leave empty to use the default paths configured for this agent.
                </span>
              </div>

              <div
                v-else
                class="per-host-paths"
              >
                <div
                  v-for="agentId in selectedAgentIds"
                  :key="agentId"
                  class="per-host-entry"
                >
                  <label class="field-label">{{ agentLabel(agentId) }}</label>
                  <textarea
                    :value="perHostSources[agentId] ?? ''"
                    class="input area-input area-input-sm"
                    placeholder="Directories to back up, one per line"
                    spellcheck="false"
                    @input="
                      ($event) =>
                        (perHostSources[agentId] = ($event.target as HTMLTextAreaElement).value)
                    "
                  />
                </div>
                <span class="field-hint">
                  Leave an agent empty to use its default backup paths.
                </span>
              </div>
            </div>

            <div class="form-card">
              <h3 class="info-title">Retention</h3>
              <div class="retention-grid">
                <div class="field">
                  <label class="field-label">Hourly</label>
                  <input
                    v-model.number="form.keep_hourly"
                    type="number"
                    min="0"
                    class="input"
                  />
                </div>
                <div class="field">
                  <label class="field-label">Daily</label>
                  <input
                    v-model.number="form.keep_daily"
                    type="number"
                    min="0"
                    class="input"
                  />
                </div>
                <div class="field">
                  <label class="field-label">Weekly</label>
                  <input
                    v-model.number="form.keep_weekly"
                    type="number"
                    min="0"
                    class="input"
                  />
                </div>
                <div class="field">
                  <label class="field-label">Monthly</label>
                  <input
                    v-model.number="form.keep_monthly"
                    type="number"
                    min="0"
                    class="input"
                  />
                </div>
                <div class="field">
                  <label class="field-label">Yearly</label>
                  <input
                    v-model.number="form.keep_yearly"
                    type="number"
                    min="0"
                    class="input"
                  />
                </div>
              </div>
            </div>
          </template>
        </div>
      </div>

      <!-- Advanced Tab (backup only) -->
      <div
        v-if="activeTab === 'advanced' && isBackup"
        class="tab-content"
      >
        <ScheduleAdvancedTab
          v-model:form="form"
          v-model:overrides="agentOverrides"
          :agent-ids="selectedAgentIds"
          :agent-label="agentLabel"
        />
      </div>

      <!-- Logs Tab -->
      <div
        v-if="activeTab === 'logs'"
        class="tab-content"
      >
        <ScheduleLogsTab
          :reports="reports"
          :loading="reportsLoading"
          :error="reportsError"
          :agents="agentMap"
        />
      </div>

      <!-- Backups Tab -->
      <div
        v-if="activeTab === 'backups'"
        class="tab-content"
      >
        <ScheduleBackupsTab
          v-model:selected="selectedBackupReport"
          :reports="reports"
          :loading="reportsLoading"
          :error="reportsError"
          :agents="agentMap"
          :repo-id="schedule?.repo_id ?? null"
        />
      </div>

      <!-- Save bar -->
      <div
        v-if="activeTab !== 'logs' && activeTab !== 'backups'"
        class="save-bar"
      >
        <div
          v-if="saveError"
          class="error-inline"
        >
          {{ saveError }}
        </div>
        <span
          v-if="saveSuccess"
          class="save-success"
          >Saved</span
        >
        <button
          class="btn btn-primary"
          :disabled="saving"
          @click="save"
        >
          {{ saving ? 'Saving...' : isCreate ? 'Create Schedule' : 'Save Changes' }}
        </button>
      </div>

      <!-- Danger Zone -->
      <div
        v-if="!isCreate && activeTab === 'settings'"
        class="info-card danger-zone"
      >
        <h3 class="info-title">Danger Zone</h3>
        <div class="danger-body">
          <div class="danger-info">
            <span class="danger-heading">Delete Schedule</span>
            <span class="danger-desc">
              Permanently delete this schedule and all associated backup reports. This cannot be
              undone.
            </span>
          </div>
          <button
            class="btn btn-sm btn-danger"
            @click="showDeleteDialog = true"
          >
            Delete Schedule
          </button>
        </div>
      </div>
    </template>

    <!-- Delete Confirmation Dialog -->
    <BaseModal
      :open="showDeleteDialog"
      title="Delete Schedule"
      @close="showDeleteDialog = false"
    >
      <p>
        Are you sure you want to delete this
        <strong>{{ schedule ? scheduleTypeLabel(schedule.schedule_type) : '' }}</strong>
        schedule? All associated backup reports will also be removed.
      </p>
      <p>This action cannot be undone.</p>

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
          @click="confirmDeleteSchedule"
        >
          {{ deleteLoading ? 'Deleting...' : 'Delete Schedule' }}
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
/* Not a tab: it leaves the page. Pushed to the end and kept muted so it does
   not read as a fourth section. */
.tab-link {
  margin-left: auto;
  color: var(--text-muted);
  gap: 0.35rem;
  display: inline-flex;
  align-items: center;
}

.tab-link:hover {
  color: var(--text-primary);
}

.schedule-detail {
  color: var(--text-primary);
  max-width: 900px;
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

.error-banner {
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  color: var(--danger);
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  margin-bottom: 1rem;
  font-size: var(--fs-base);
}

.form-grid {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.info-card,
.form-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.info-title {
  margin: 0 0 1rem;
}

.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.4rem 0;
  border-bottom: 1px solid var(--border-subtle);
}

.info-row:last-child {
  border-bottom: none;
}

.info-label {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.info-value {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.info-row-targets {
  align-items: flex-start;
}

.target-health-list {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  align-items: flex-end;
}

.target-health-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.target-health-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.target-health-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--warning);
  background: var(--warning-subtle);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-sm);
}

.target-health-note {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.required {
  color: var(--danger);
}

.field-hint {
  display: block;
  margin-top: 0.25rem;
}

.input,
.form-select {
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--fs-base);
  outline: none;
  transition: border-color var(--duration-base);
  box-sizing: border-box;
}

.input:focus,
.form-select:focus {
  border-color: var(--accent);
}

.area-input {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.area-input-sm {
  min-height: 56px;
}

.per-host-paths {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.per-host-entry {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.retention-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem;
}

.save-bar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1.5rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}

.error-inline {
  font-size: var(--fs-sm);
  color: var(--danger);
}

.save-success {
  font-size: var(--fs-sm);
  color: var(--success);
  font-weight: 600;
}

/* Multi-select */
.multi-select-wrapper {
  position: relative;
}

.multi-select-trigger {
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--fs-base);
  outline: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  transition: border-color var(--duration-base);
  box-sizing: border-box;
  text-align: left;
}

.multi-select-trigger:hover,
.multi-select-trigger.open {
  border-color: var(--accent);
}

.multi-select-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.multi-select-arrow {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  flex-shrink: 0;
}

.multi-select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  padding: 0.4rem;
  z-index: 100;
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.multi-select-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--fs-base);
  color: var(--text-secondary);
  transition: background var(--duration-fast);
}

.multi-select-item:hover {
  background: var(--bg-hover);
}

.multi-select-item input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
  flex-shrink: 0;
}

.multi-select-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Segmented control */

/* Ordering list */
.order-list {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.order-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.6rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.order-index {
  font-size: var(--fs-2xs);
  font-weight: 700;
  color: var(--text-muted);
  min-width: 1.2rem;
  text-align: center;
}

.order-name {
  flex: 1;
  font-size: var(--fs-base);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.order-actions {
  display: flex;
  gap: 0.2rem;
  flex-shrink: 0;
}

.order-btn {
  padding: 0.4rem 0.6rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-lg);
  cursor: pointer;
  transition:
    background var(--duration-fast),
    color var(--duration-fast);
  line-height: 1;
}

.order-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.order-btn:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}

/* Danger zone */

/* Sits directly after the settings form here, so it needs breathing room the
   other detail views get from their surrounding grid. */
.danger-zone {
  margin-top: 2rem;
}

/* Dialog */

/* Backups tab layout */

.data-table tr.selected td {
  background: var(--accent-subtle);
  color: var(--text-primary);
}
</style>
