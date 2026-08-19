<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import { useElapsedClock } from '../composables/useElapsedTimer'
import { parseLines } from '../utils/validation'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { isAgentOffline, lastSeenText } from '../utils/agent'
import { parseArchiveProgress } from '../utils/archiveProgress'
import ScheduleHeader from '../components/ScheduleHeader.vue'
import ScheduleOverviewTab from '../components/ScheduleOverviewTab.vue'
import ScheduleSettingsTab from '../components/ScheduleSettingsTab.vue'
import ScheduleBackupsTab from '../components/ScheduleBackupsTab.vue'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import BaseSpinner from '../components/BaseSpinner.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRow, ScheduleType } from '../types/schedule'
import type { ScheduleBackupSourcesResponse } from '../types/generated'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'
import { isScheduleSettingsSection, type ScheduleSettingsSection } from '../utils/scheduleSettings'

/**
 * A schedule's detail page: a persistent header, then three tabs - Overview,
 * Backups (backup type only), Settings - following the same shape as the
 * agent detail page. Create mode has no status to show yet, so it skips
 * straight to Settings.
 */
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

// The Advanced section's per-agent overrides, grouped so the tab component
// takes one v-model instead of seven.
const agentOverrides = ref<ScheduleAgentOverrides>({
  usePerHostExcludes: false,
  perHostExcludes: {},
  usePerHostFileChangePatterns: false,
  perHostFileChangePatterns: {},
  usePerAgentCmds: false,
  perAgentPreCmds: {},
  perAgentPostCmds: {},
})

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

type TabId = 'overview' | 'backups' | 'settings'
const activeTab = computed<TabId>({
  get() {
    if (isCreate.value) return 'settings'
    const t = route.query.tab
    if (t === 'backups' && isBackup.value) return 'backups'
    if (t === 'settings') return 'settings'
    return 'overview'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const settingsSection = computed<ScheduleSettingsSection>({
  get() {
    return isScheduleSettingsSection(route.query.section) ? route.query.section : 'general'
  },
  set(val: ScheduleSettingsSection) {
    router.replace({ query: { ...route.query, section: val } })
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

/**
 * A single "Settings" tab in create mode rather than a whole Overview/Backups
 * strip - there is nothing to show status for yet, and nowhere else the
 * create form could go.
 */
const visibleTabs = computed<TabOption<TabId>[]>(() => {
  if (isCreate.value) return [{ id: 'settings', label: 'Settings' }]
  const tabs: TabOption<TabId>[] = [{ id: 'overview', label: 'Overview' }]
  // No count badge: `reports` is capped at 20 until the tab is opened, so a
  // count shown up front would silently undercount everything past the cap.
  if (isBackup.value) tabs.push({ id: 'backups', label: 'Backups' })
  tabs.push({ id: 'settings', label: 'Settings' })
  return tabs
})

const agentMap = computed(() => {
  const m = new Map<number, AgentRow>()
  agents.value.forEach((c) => m.set(c.id, c))
  return m
})

function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}

const headerTypeLabel = computed(() => scheduleTypeLabel(scheduleType.value))
const headerCronSummary = computed(
  () => cronToHuman(form.value.cron_expression) ?? form.value.cron_expression,
)
const repoName = computed(() => repo.value?.name ?? null)

// Spread rather than shared by reference: this ref is mutated in place
// elsewhere (e.g. populateForm(), the backup_sources assignment below), and
// DEFAULT_SCHEDULE_FORM_STATE is a module-level singleton every schedule page
// instance imports.
const form = ref<ScheduleFormState>({ ...DEFAULT_SCHEDULE_FORM_STATE })

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

const overdueTargetCount = computed(
  () => selectedAgentIds.value.filter((id) => healthForAgent(id)?.is_overdue).length,
)

function connectivityNote(agentId: number): string {
  const agent = agentMap.value.get(agentId)
  if (!agent || !isAgentOffline(agent)) return ''
  return `Agent offline (${lastSeenText(agent)})`
}

onMounted(() => {
  loadData()
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

onMessage('BackupStarted', (payload) => {
  if (payload.schedule_id != null && payload.schedule_id !== Number(props.id)) return
  if (
    payload.schedule_id == null &&
    !(repo.value != null && payload.target_name === repo.value.name)
  )
    return
  const agent = agents.value.find((a) => a.hostname === payload.hostname)
  backupRunning.value = true
  backupHostname.value = agent?.display_name ?? payload.hostname
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
  if (tab === 'backups' && !isCreate.value) {
    loadReports().catch(() => undefined)
  }
})
</script>

<template>
  <div class="schedule-detail">
    <nav class="detail-breadcrumb">
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
      <ScheduleHeader
        v-if="!isCreate && schedule"
        :schedule="schedule"
        :type-label="headerTypeLabel"
        :cron-summary="headerCronSummary"
        :backup-running="backupRunning"
        :run-now-loading="runNowLoading"
        :cancel-loading="cancelLoading"
        :overdue-count="overdueTargetCount"
        @run-now="runNow()"
        @cancel-backup="cancelBackup"
        @logs="goToLogs"
        @delete="showDeleteDialog = true"
      />
      <div
        v-else
        class="page-header"
      >
        <h1 class="page-title">New Schedule</h1>
      </div>

      <BaseTabs
        v-model="activeTab"
        :tabs="visibleTabs"
        label="Schedule sections"
      />

      <div class="tab-content">
        <ScheduleOverviewTab
          v-if="activeTab === 'overview' && schedule"
          :schedule="schedule"
          :repo-name="repoName"
          :cron-summary="headerCronSummary"
          :agent-ids="selectedAgentIds"
          :agent-label="agentLabel"
          :health-for-agent="healthForAgent"
          :connectivity-note="connectivityNote"
          :retrying-agent-id="retryingAgentId"
          :reports="reports"
          :agents="agentMap"
          :backup-running="backupRunning"
          :backup-hostname="backupHostname"
          :backup-archive-name="backupArchiveName"
          :backup-elapsed-secs="backupElapsedSecs"
          :estimated-remaining-secs="estimatedRemainingSecs"
          :archive-progress="archiveProgress"
          @retry="runNow($event)"
          @open-backups="activeTab = 'backups'"
        />

        <ScheduleBackupsTab
          v-else-if="activeTab === 'backups'"
          v-model:selected="selectedBackupReport"
          :reports="reports"
          :loading="reportsLoading"
          :error="reportsError"
          :agents="agentMap"
          :repo-id="schedule?.repo_id ?? null"
        />

        <ScheduleSettingsTab
          v-else-if="activeTab === 'settings'"
          v-model:section="settingsSection"
          v-model:form="form"
          v-model:overrides="agentOverrides"
          v-model:selected-agent-ids="selectedAgentIds"
          v-model:selected-repo-id="selectedRepoId"
          v-model:selected-type="selectedType"
          v-model:on-failure="onFailure"
          v-model:use-per-host-paths="usePerHostPaths"
          v-model:per-host-sources="perHostSources"
          :is-create="isCreate"
          :is-backup="isBackup"
          :agents="agents"
          :repos="repos"
          :agent-label="agentLabel"
        />
      </div>

      <!-- Save bar -->
      <div
        v-if="activeTab === 'settings'"
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
          {{ saving ? 'Saving...' : isCreate ? 'Create schedule' : 'Save changes' }}
        </button>
      </div>
    </template>

    <!-- Delete Confirmation Dialog -->
    <BaseModal
      :open="showDeleteDialog"
      title="Delete schedule"
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
          {{ deleteLoading ? 'Deleting...' : 'Delete schedule' }}
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.schedule-detail {
  color: var(--text-primary);
  max-width: 1100px;
}

.tab-content {
  margin-top: 1rem;
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
</style>
