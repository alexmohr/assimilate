<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { formatDateShort, formatBytes, relativeTime } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import { parseLines } from '../utils/validation'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import CronBuilder from '../components/CronBuilder.vue'
import BaseSpinner from '../components/BaseSpinner.vue'

type ScheduleType = 'backup' | 'check' | 'verify'

interface ScheduleRow {
  id: number
  repo_id: number
  schedule_type: string
  cron_expression: string
  enabled: boolean
  canary_enabled: boolean
  last_run_at: string | null
  next_run_at: string | null
  exclude_patterns: string[]
  ignore_global_excludes: boolean
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  compact_enabled: boolean
  pre_backup_commands: string
  post_backup_commands: string
  execution_mode: string
  on_failure: string
}

interface ScheduleTarget {
  client_id: number
  execution_order: number
}

interface PerHostBackupSources {
  client_id: number
  paths: string[]
}

interface ScheduleBackupSourcesResponse {
  backup_sources: string[]
  backup_sources_per_host: PerHostBackupSources[]
}

interface ClientRow {
  id: number
  hostname: string
  display_name: string | null
}

interface RepoRow {
  id: number
  name: string
  repo_path: string
}

const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()

const isCreate = computed(() => props.id === 'new')

const schedule = ref<ScheduleRow | null>(null)
const clients = ref<ClientRow[]>([])
const repos = ref<RepoRow[]>([])
const repo = ref<RepoRow | null>(null)
const scheduleTargets = ref<ScheduleTarget[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const saving = ref(false)
const saveError = ref<string | null>(null)
const saveSuccess = ref(false)
const showDeleteDialog = ref(false)
const deleteLoading = ref(false)
const refOpen = ref(false)

const selectedClientIds = ref<number[]>([])
const selectedRepoId = ref<number | null>(null)
const selectedType = ref<ScheduleType>('backup')
const executionMode = ref<'parallel' | 'sequential'>('parallel')
const onFailure = ref<'stop' | 'continue'>('stop')
const usePerHostPaths = ref(false)
const perHostSources = ref<Record<number, string>>({})

const showClientDropdown = ref(false)
const clientDropdownRef = ref<HTMLElement | null>(null)

type TabId = 'settings' | 'advanced' | 'results'
const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab as string | undefined
    if (t === 'advanced' || t === 'results') return t
    return 'settings'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const scheduleType = computed(() =>
  isCreate.value ? selectedType.value : (schedule.value?.schedule_type ?? 'backup'),
)
const isBackup = computed(() => scheduleType.value === 'backup')

const form = ref({
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: false,
  exclude_patterns: '',
  ignore_global_excludes: false,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 12,
  keep_yearly: 10,
  compact_enabled: true,
  pre_backup_commands: '',
  post_backup_commands: '',
  backup_sources: '',
})

function clientLabel(id: number): string {
  const c = clients.value.find((x) => x.id === id)
  return c ? (c.display_name ?? c.hostname) : `#${id}`
}

function multiSelectLabel(): string {
  if (selectedClientIds.value.length === 0) return 'Select hosts...'
  if (selectedClientIds.value.length === 1) return clientLabel(selectedClientIds.value[0])
  return `${selectedClientIds.value.length} hosts selected`
}

function toggleClientSelection(id: number): void {
  if (selectedClientIds.value.includes(id)) {
    selectedClientIds.value = selectedClientIds.value.filter((x) => x !== id)
  } else {
    selectedClientIds.value = [...selectedClientIds.value, id]
  }
}

function moveClientUp(index: number): void {
  if (index === 0) return
  const ids = [...selectedClientIds.value]
  ;[ids[index - 1], ids[index]] = [ids[index], ids[index - 1]]
  selectedClientIds.value = ids
}

function moveClientDown(index: number): void {
  if (index >= selectedClientIds.value.length - 1) return
  const ids = [...selectedClientIds.value]
  ;[ids[index], ids[index + 1]] = [ids[index + 1], ids[index]]
  selectedClientIds.value = ids
}

function handleClickOutside(event: MouseEvent): void {
  if (
    showClientDropdown.value &&
    clientDropdownRef.value &&
    !clientDropdownRef.value.contains(event.target as Node)
  ) {
    showClientDropdown.value = false
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
    cron_expression: s.cron_expression,
    enabled: s.enabled,
    canary_enabled: s.canary_enabled,
    exclude_patterns: s.exclude_patterns.join('\n'),
    ignore_global_excludes: s.ignore_global_excludes,
    keep_daily: s.keep_daily,
    keep_weekly: s.keep_weekly,
    keep_monthly: s.keep_monthly,
    keep_yearly: s.keep_yearly,
    compact_enabled: s.compact_enabled,
    pre_backup_commands: (JSON.parse(s.pre_backup_commands || '[]') as string[]).join('\n'),
    post_backup_commands: (JSON.parse(s.post_backup_commands || '[]') as string[]).join('\n'),
    backup_sources: '',
  }
  executionMode.value = (s.execution_mode as 'parallel' | 'sequential') ?? 'parallel'
  onFailure.value = (s.on_failure as 'stop' | 'continue') ?? 'stop'
}

interface ReportRow {
  id: number
  client_id: number
  repo_id: number
  started_at: string
  finished_at: string
  status: string
  original_size: number
  compressed_size: number
  deduplicated_size: number
  files_processed: number
  duration_secs: number
  error_message: string | null
  warnings: string[]
  borg_version: string | null
  archive_name: string | null
}

const reports = ref<ReportRow[]>([])
const reportsLoading = ref(false)
const reportsError = ref<string | null>(null)
const expandedReportId = ref<number | null>(null)

function handleResultClick(r: ReportRow): void {
  if (r.status === 'success') {
    const query: Record<string, string> = { tab: 'archives' }
    if (r.archive_name) {
      query.archive = r.archive_name
    }
    router.push({ path: `/repos/${r.repo_id}`, query })
  } else {
    expandedReportId.value = expandedReportId.value === r.id ? null : r.id
  }
}

async function fetchReports(): Promise<void> {
  if (isCreate.value) return
  reportsLoading.value = true
  reportsError.value = null
  try {
    const res = await apiClient.get<ReportRow[]>(`/schedules/${props.id}/reports?limit=20`)
    reports.value = res.data
  } catch (e: unknown) {
    reportsError.value = extractError(e, 'Failed to load reports')
  } finally {
    reportsLoading.value = false
  }
}

function scheduleTypeLabel(t: string): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity Check'
    case 'verify':
      return 'Verify (extract dry-run)'
    default:
      return t
  }
}

function targetHostnames(): string {
  return selectedClientIds.value.map(clientLabel).join(', ')
}

async function loadData(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    if (isCreate.value) {
      const [clientsRes, reposRes] = await Promise.all([
        apiClient.get<ClientRow[]>('/clients'),
        apiClient.get<RepoRow[]>('/repos'),
      ])
      clients.value = clientsRes.data
      repos.value = reposRes.data
      const queryClientId = Number(route.query.client_id)
      if (queryClientId && clients.value.some((c) => c.id === queryClientId)) {
        selectedClientIds.value = [queryClientId]
      }
      selectedRepoId.value = repos.value.length > 0 ? repos.value[0].id : null
    } else {
      const [schedRes, clientsRes, reposRes, targetsRes, sourcesRes] = await Promise.all([
        apiClient.get<ScheduleRow>(`/schedules/${props.id}`),
        apiClient.get<ClientRow[]>('/clients'),
        apiClient.get<RepoRow[]>('/repos'),
        apiClient.get<ScheduleTarget[]>(`/schedules/${props.id}/targets`),
        apiClient.get<ScheduleBackupSourcesResponse>(`/schedules/${props.id}/sources`),
      ])
      schedule.value = schedRes.data
      clients.value = clientsRes.data
      repos.value = reposRes.data
      scheduleTargets.value = targetsRes.data
      repo.value = reposRes.data.find((r) => r.id === schedRes.data.repo_id) ?? null
      const sorted = [...targetsRes.data].sort((a, b) => a.execution_order - b.execution_order)
      selectedClientIds.value = sorted.map((t) => t.client_id)
      populateForm(schedRes.data)

      const sources = sourcesRes.data
      form.value.backup_sources = sources.backup_sources.join('\n')
      if (sources.backup_sources_per_host.length > 0) {
        usePerHostPaths.value = true
        const map: Record<number, string> = {}
        for (const entry of sources.backup_sources_per_host) {
          map[entry.client_id] = entry.paths.join('\n')
        }
        perHostSources.value = map
      }
    }
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load schedule')
  } finally {
    loading.value = false
  }
}

async function save(): Promise<void> {
  saving.value = true
  saveError.value = null
  saveSuccess.value = false
  try {
    const payload: Record<string, unknown> = {
      cron_expression: form.value.cron_expression,
      enabled: form.value.enabled,
      canary_enabled: form.value.canary_enabled,
      exclude_patterns: parseLines(form.value.exclude_patterns),
      ignore_global_excludes: form.value.ignore_global_excludes,
      keep_daily: form.value.keep_daily,
      keep_weekly: form.value.keep_weekly,
      keep_monthly: form.value.keep_monthly,
      keep_yearly: form.value.keep_yearly,
      compact_enabled: form.value.compact_enabled,
      pre_backup_commands: parseLines(form.value.pre_backup_commands),
      post_backup_commands: parseLines(form.value.post_backup_commands),
      backup_sources: usePerHostPaths.value ? [] : parseLines(form.value.backup_sources),
    }

    if (usePerHostPaths.value) {
      const perHost: { client_id: number; paths: string[] }[] = []
      for (const id of selectedClientIds.value) {
        const text = perHostSources.value[id] ?? ''
        const paths = parseLines(text)
        if (paths.length > 0) {
          perHost.push({ client_id: id, paths })
        }
      }
      payload.backup_sources_per_host = perHost
    }

    if (isCreate.value) {
      if (selectedClientIds.value.length === 0 || !selectedRepoId.value) {
        saveError.value = 'Please select at least one host and a repository.'
        return
      }
      const res = await apiClient.post<ScheduleRow>('/schedules', {
        ...payload,
        client_ids: selectedClientIds.value,
        repo_id: selectedRepoId.value,
        schedule_type: selectedType.value,
        execution_mode: executionMode.value,
        on_failure: onFailure.value,
      })
      router.push(`/schedules/${res.data.id}`)
    } else {
      const res = await apiClient.put<ScheduleRow>(`/schedules/${schedule.value!.id}`, {
        ...payload,
        client_ids: selectedClientIds.value,
        execution_mode: executionMode.value,
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

watch(() => props.id, loadData)
watch(activeTab, (tab) => {
  if (tab === 'results' && reports.value.length === 0 && !reportsLoading.value) {
    fetchReports()
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
        <template v-else-if="schedule">{{ scheduleTypeLabel(schedule.schedule_type) }}</template>
        <template v-else>#{{ props.id }}</template>
      </span>
    </nav>

    <div class="page-header">
      <h1 class="page-title">
        <template v-if="isCreate">New Schedule</template>
        <template v-else-if="schedule">
          {{ scheduleTypeLabel(schedule.schedule_type) }} Schedule
        </template>
        <template v-else>Schedule</template>
      </h1>
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
      <div class="tab-bar">
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'settings' }"
          @click="activeTab = 'settings'"
        >
          Settings
        </button>
        <button
          v-if="isBackup"
          class="tab-btn"
          :class="{ active: activeTab === 'advanced' }"
          @click="activeTab = 'advanced'"
        >
          Advanced
        </button>
        <button
          v-if="!isCreate"
          class="tab-btn"
          :class="{ active: activeTab === 'results' }"
          @click="activeTab = 'results'"
        >
          Results
        </button>
      </div>

      <!-- Settings Tab -->
      <div
        v-if="activeTab === 'settings'"
        class="tab-content"
      >
        <div class="form-grid">
          <!-- Create-only: target selection -->
          <div
            v-if="isCreate"
            class="form-card"
          >
            <h3 class="info-title">Target</h3>

            <!-- Multi-select for hosts -->
            <div class="form-group">
              <label class="form-label">Hosts <span class="required">*</span></label>
              <div
                ref="clientDropdownRef"
                class="multi-select-wrapper"
              >
                <button
                  type="button"
                  class="multi-select-trigger"
                  :class="{ open: showClientDropdown }"
                  @click.stop="showClientDropdown = !showClientDropdown"
                >
                  <span class="multi-select-label">{{ multiSelectLabel() }}</span>
                  <span class="multi-select-arrow">{{ showClientDropdown ? '▲' : '▼' }}</span>
                </button>
                <div
                  v-if="showClientDropdown"
                  class="multi-select-dropdown"
                >
                  <label
                    v-for="c in clients"
                    :key="c.id"
                    class="multi-select-item"
                  >
                    <input
                      type="checkbox"
                      :checked="selectedClientIds.includes(c.id)"
                      @change="toggleClientSelection(c.id)"
                    />
                    <span class="multi-select-name">{{ c.display_name ?? c.hostname }}</span>
                  </label>
                </div>
              </div>
              <span class="field-hint">The agent clients that will execute this schedule</span>
            </div>

            <!-- Execution Mode -->
            <div class="form-group">
              <label class="form-label">Execution Mode</label>
              <div class="segmented-control">
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: executionMode === 'parallel' }"
                  @click="executionMode = 'parallel'"
                >
                  Parallel
                </button>
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: executionMode === 'sequential' }"
                  @click="executionMode = 'sequential'"
                >
                  Sequential
                </button>
              </div>
              <span class="field-hint">
                Parallel runs all hosts simultaneously; Sequential runs them one by one.
              </span>
            </div>

            <!-- On Failure (sequential only) -->
            <div
              v-if="executionMode === 'sequential'"
              class="form-group"
            >
              <label class="form-label">On Failure</label>
              <select
                v-model="onFailure"
                class="form-select"
              >
                <option value="stop">Stop</option>
                <option value="continue">Continue</option>
              </select>
              <span class="field-hint">
                Whether to stop or continue to the next host when one fails.
              </span>
            </div>

            <!-- Ordering (sequential, 2+ hosts) -->
            <div
              v-if="executionMode === 'sequential' && selectedClientIds.length > 1"
              class="form-group"
            >
              <label class="form-label">Execution Order</label>
              <div class="order-list">
                <div
                  v-for="(clientId, idx) in selectedClientIds"
                  :key="clientId"
                  class="order-item"
                >
                  <span class="order-index">{{ idx + 1 }}</span>
                  <span class="order-name">{{ clientLabel(clientId) }}</span>
                  <div class="order-actions">
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === 0"
                      title="Move up"
                      @click="moveClientUp(idx)"
                    >
                      ▲
                    </button>
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === selectedClientIds.length - 1"
                      title="Move down"
                      @click="moveClientDown(idx)"
                    >
                      ▼
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <div class="form-group">
              <label class="form-label">Repository <span class="required">*</span></label>
              <select
                v-model.number="selectedRepoId"
                class="form-select"
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
            <div class="form-group">
              <label class="form-label">Schedule Type</label>
              <select
                v-model="selectedType"
                class="form-select"
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
            <div class="info-row">
              <span class="info-label">Targets</span>
              <span class="info-value">{{ targetHostnames() || '—' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Execution Mode</span>
              <span class="info-value">
                {{ schedule.execution_mode === 'sequential' ? 'Sequential' : 'Parallel' }}
              </span>
            </div>
            <div
              v-if="schedule.execution_mode === 'sequential'"
              class="info-row"
            >
              <span class="info-label">On Failure</span>
              <span class="info-value">
                {{ schedule.on_failure === 'continue' ? 'Continue' : 'Stop' }}
              </span>
            </div>
            <div class="info-row">
              <span class="info-label">Repository</span>
              <span class="info-value">{{ repo?.name ?? `#${schedule.repo_id}` }}</span>
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
          </div>

          <!-- Edit-only: target settings card -->
          <div
            v-if="!isCreate"
            class="form-card"
          >
            <h3 class="info-title">Target Settings</h3>

            <!-- Multi-select for hosts -->
            <div class="form-group">
              <label class="form-label">Hosts</label>
              <div
                ref="clientDropdownRef"
                class="multi-select-wrapper"
              >
                <button
                  type="button"
                  class="multi-select-trigger"
                  :class="{ open: showClientDropdown }"
                  @click.stop="showClientDropdown = !showClientDropdown"
                >
                  <span class="multi-select-label">{{ multiSelectLabel() }}</span>
                  <span class="multi-select-arrow">{{ showClientDropdown ? '▲' : '▼' }}</span>
                </button>
                <div
                  v-if="showClientDropdown"
                  class="multi-select-dropdown"
                >
                  <label
                    v-for="c in clients"
                    :key="c.id"
                    class="multi-select-item"
                  >
                    <input
                      type="checkbox"
                      :checked="selectedClientIds.includes(c.id)"
                      @change="toggleClientSelection(c.id)"
                    />
                    <span class="multi-select-name">{{ c.display_name ?? c.hostname }}</span>
                  </label>
                </div>
              </div>
            </div>

            <!-- Execution Mode -->
            <div class="form-group">
              <label class="form-label">Execution Mode</label>
              <div class="segmented-control">
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: executionMode === 'parallel' }"
                  @click="executionMode = 'parallel'"
                >
                  Parallel
                </button>
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: executionMode === 'sequential' }"
                  @click="executionMode = 'sequential'"
                >
                  Sequential
                </button>
              </div>
            </div>

            <!-- On Failure (sequential only) -->
            <div
              v-if="executionMode === 'sequential'"
              class="form-group"
            >
              <label class="form-label">On Failure</label>
              <select
                v-model="onFailure"
                class="form-select"
              >
                <option value="stop">Stop</option>
                <option value="continue">Continue</option>
              </select>
            </div>

            <!-- Ordering (sequential, 2+ hosts) -->
            <div
              v-if="executionMode === 'sequential' && selectedClientIds.length > 1"
              class="form-group"
            >
              <label class="form-label">Execution Order</label>
              <div class="order-list">
                <div
                  v-for="(clientId, idx) in selectedClientIds"
                  :key="clientId"
                  class="order-item"
                >
                  <span class="order-index">{{ idx + 1 }}</span>
                  <span class="order-name">{{ clientLabel(clientId) }}</span>
                  <div class="order-actions">
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === 0"
                      title="Move up"
                      @click="moveClientUp(idx)"
                    >
                      ▲
                    </button>
                    <button
                      type="button"
                      class="order-btn"
                      :disabled="idx === selectedClientIds.length - 1"
                      title="Move down"
                      @click="moveClientDown(idx)"
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
            <div class="form-group">
              <label class="form-label">Schedule</label>
              <CronBuilder v-model="form.cron_expression" />
            </div>
            <div class="form-group form-group-inline">
              <label class="form-label">Enabled</label>
              <ToggleSwitch v-model="form.enabled" />
            </div>
          </div>

          <template v-if="isBackup">
            <div class="form-card">
              <h3 class="info-title">Backup Paths</h3>
              <div
                v-if="selectedClientIds.length > 1"
                class="form-group form-group-inline"
              >
                <label class="form-label">Configure per host</label>
                <ToggleSwitch v-model="usePerHostPaths" />
              </div>

              <div
                v-if="!usePerHostPaths"
                class="form-group"
              >
                <textarea
                  v-model="form.backup_sources"
                  class="form-input area-input"
                  placeholder="Directories to back up, one per line"
                  spellcheck="false"
                />
                <span class="field-hint">
                  Leave empty to use the default paths configured for this host.
                </span>
              </div>

              <div
                v-else
                class="per-host-paths"
              >
                <div
                  v-for="clientId in selectedClientIds"
                  :key="clientId"
                  class="per-host-entry"
                >
                  <label class="form-label">{{ clientLabel(clientId) }}</label>
                  <textarea
                    :value="perHostSources[clientId] ?? ''"
                    class="form-input area-input area-input-sm"
                    placeholder="Directories to back up, one per line"
                    spellcheck="false"
                    @input="
                      ($event) =>
                        (perHostSources[clientId] = ($event.target as HTMLTextAreaElement).value)
                    "
                  />
                </div>
                <span class="field-hint">
                  Leave a host empty to use its default backup paths.
                </span>
              </div>
            </div>

            <div class="form-card">
              <h3 class="info-title">Retention</h3>
              <div class="retention-grid">
                <div class="form-group">
                  <label class="form-label">Daily</label>
                  <input
                    v-model.number="form.keep_daily"
                    type="number"
                    min="0"
                    class="form-input"
                  />
                </div>
                <div class="form-group">
                  <label class="form-label">Weekly</label>
                  <input
                    v-model.number="form.keep_weekly"
                    type="number"
                    min="0"
                    class="form-input"
                  />
                </div>
                <div class="form-group">
                  <label class="form-label">Monthly</label>
                  <input
                    v-model.number="form.keep_monthly"
                    type="number"
                    min="0"
                    class="form-input"
                  />
                </div>
                <div class="form-group">
                  <label class="form-label">Yearly</label>
                  <input
                    v-model.number="form.keep_yearly"
                    type="number"
                    min="0"
                    class="form-input"
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
        <div class="form-grid">
          <div class="form-card">
            <h3 class="info-title">Options</h3>
            <div class="form-group form-group-inline">
              <label class="form-label">Canary Verification</label>
              <ToggleSwitch v-model="form.canary_enabled" />
            </div>
            <div class="form-group form-group-inline">
              <label class="form-label">Ignore Global Excludes</label>
              <ToggleSwitch v-model="form.ignore_global_excludes" />
            </div>
            <div class="form-group form-group-inline">
              <label class="form-label">Compact after backup</label>
              <ToggleSwitch v-model="form.compact_enabled" />
            </div>
          </div>

          <div class="form-card">
            <h3 class="info-title">Exclude Patterns</h3>
            <div class="form-group">
              <div class="form-label-row">
                <label class="form-label">Patterns</label>
                <button
                  type="button"
                  class="ref-toggle"
                  @click="refOpen = !refOpen"
                >
                  {{ refOpen ? 'Close Reference' : 'Pattern Reference' }}
                </button>
              </div>
              <textarea
                v-model="form.exclude_patterns"
                class="form-input area-input"
                placeholder="One pattern per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
                spellcheck="false"
              />
              <span class="field-hint">
                Leave empty to use only global and host-level default excludes. Lines starting with
                <code>#</code> are treated as comments.
              </span>
              <div
                v-if="refOpen"
                class="ref-panel"
              >
                <div class="ref-title">Borg Pattern Syntax</div>
                <div class="ref-section">
                  <div class="ref-section-title">Shell Patterns (default)</div>
                  <div class="ref-entry">
                    <code>*.cache</code> <span>any file ending in .cache</span>
                  </div>
                  <div class="ref-entry">
                    <code>home/*/Downloads</code> <span>Downloads in any home dir</span>
                  </div>
                </div>
                <div class="ref-section">
                  <div class="ref-section-title">Path Prefix <code>pp:</code></div>
                  <div class="ref-entry">
                    <code>pp:__pycache__</code>
                    <span>any path component named __pycache__</span>
                  </div>
                </div>
                <div class="ref-section">
                  <div class="ref-section-title">Regex <code>re:</code></div>
                  <div class="ref-entry">
                    <code>re:\.git/objects/</code> <span>regex match anywhere in path</span>
                  </div>
                </div>
                <div class="ref-section">
                  <div class="ref-section-title">Fnmatch <code>fm:</code></div>
                  <div class="ref-entry">
                    <code>fm:*.log</code> <span>fnmatch pattern (case-sensitive)</span>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="form-card">
            <h3 class="info-title">Commands</h3>
            <div class="form-group">
              <label class="form-label">Pre-backup Commands</label>
              <textarea
                v-model="form.pre_backup_commands"
                class="form-input cmd-area"
                placeholder="One command per line, e.g.&#10;docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
                spellcheck="false"
              />
            </div>
            <div class="form-group">
              <label class="form-label">Post-backup Commands</label>
              <textarea
                v-model="form.post_backup_commands"
                class="form-input cmd-area"
                placeholder="One command per line (optional)"
                spellcheck="false"
              />
            </div>
          </div>
        </div>
      </div>

      <!-- Results Tab -->
      <div
        v-if="activeTab === 'results'"
        class="tab-content"
      >
        <BaseSpinner
          v-if="reportsLoading"
          size="md"
        />
        <div
          v-else-if="reportsError"
          class="error-banner"
        >
          {{ reportsError }}
        </div>
        <div
          v-else-if="reports.length === 0"
          class="empty-state"
        >
          No backup reports yet.
        </div>
        <div
          v-else
          class="results-list"
        >
          <div
            v-for="r in reports"
            :key="r.id"
            class="result-card"
            :class="[`result-${r.status}`, { 'result-card-link': r.status === 'success' }]"
            @click="handleResultClick(r)"
          >
            <div class="result-header">
              <span class="result-status-badge">{{ r.status }}</span>
              <span class="result-date">{{ relativeTime(r.finished_at) }}</span>
              <span class="result-duration">{{ r.duration_secs }}s</span>
            </div>
            <div class="result-stats">
              <span>{{ formatBytes(r.original_size) }} original</span>
              <span>{{ formatBytes(r.deduplicated_size) }} dedup</span>
              <span>{{ r.files_processed }} files</span>
            </div>
            <template v-if="expandedReportId === r.id">
              <div
                v-if="r.warnings.length > 0"
                class="result-warnings"
              >
                <strong class="result-section-label">Warnings</strong>
                <pre class="result-output">{{ r.warnings.join('\n') }}</pre>
              </div>
              <div
                v-if="r.error_message"
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
              v-else-if="r.error_message || r.warnings.length > 0"
              class="result-expand-hint"
              >{{ expandedReportId === r.id ? 'Click to collapse' : 'Click to expand' }}</span
            >
          </div>
        </div>
      </div>

      <!-- Save bar -->
      <div
        v-if="activeTab !== 'results'"
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
    <Teleport to="body">
      <div
        v-if="showDeleteDialog"
        class="overlay"
        @click.self="showDeleteDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Schedule</h2>
            <button
              class="close-btn"
              @click="showDeleteDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Are you sure you want to delete this
              <strong>{{ schedule ? scheduleTypeLabel(schedule.schedule_type) : '' }}</strong>
              schedule? All associated backup reports will also be removed.
            </p>
            <p>This action cannot be undone.</p>
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
              @click="confirmDeleteSchedule"
            >
              {{ deleteLoading ? 'Deleting...' : 'Delete Schedule' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.schedule-detail {
  color: var(--text-primary);
  max-width: 900px;
}

.breadcrumb {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
  font-size: 0.875rem;
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

.page-title {
  font-size: 1.3rem;
  font-weight: 700;
  margin: 0 0 0.4rem;
}

.error-banner {
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  color: var(--danger);
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  margin-bottom: 1rem;
  font-size: 0.875rem;
}

.tab-bar {
  display: flex;
  gap: 0;
  border-bottom: 1px solid var(--border);
  margin-top: 1.5rem;
  margin-bottom: 1.5rem;
}

.tab-btn {
  padding: 0.6rem 1.2rem;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 0.82rem;
  font-weight: 600;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  transition:
    color 0.15s,
    border-color 0.15s;
}

.tab-btn:hover {
  color: var(--text-primary);
}

.tab-btn.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
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
  font-size: 0.8rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
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
  font-size: 0.8rem;
  color: var(--text-muted);
}

.info-value {
  font-size: 0.82rem;
  font-weight: 600;
  color: var(--text-primary);
}

.form-group {
  margin-bottom: 1rem;
}

.form-group:last-child {
  margin-bottom: 0;
}

.form-group-inline {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

.form-label {
  display: block;
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 0.35rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.form-group-inline .form-label {
  margin-bottom: 0;
}

.required {
  color: var(--danger);
}

.field-hint {
  display: block;
  font-size: 0.72rem;
  color: var(--text-muted);
  margin-top: 0.25rem;
}

.form-input,
.form-select {
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  outline: none;
  transition: border-color 0.15s;
  box-sizing: border-box;
}

.form-input:focus,
.form-select:focus {
  border-color: var(--accent);
}

.area-input {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: 0.82rem;
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

.cmd-area {
  min-height: 60px;
  resize: vertical;
  font-family: var(--mono);
  font-size: 0.82rem;
  line-height: 1.5;
}

.retention-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem;
}

.form-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.35rem;
}

.form-label-row .form-label {
  margin-bottom: 0;
}

.ref-toggle {
  padding: 0.15rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.72rem;
  font-weight: 500;
  cursor: pointer;
  transition:
    color 0.15s,
    background 0.15s;
}

.ref-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.ref-panel {
  margin-top: 0.5rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.875rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.ref-title {
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--border);
}

.ref-section {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.ref-section-title {
  font-size: 0.7rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.ref-section-title code {
  font-family: var(--mono);
  color: var(--accent);
  text-transform: none;
  letter-spacing: 0;
  background: transparent;
  padding: 0;
}

.ref-entry {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
}

.ref-entry code {
  font-family: var(--mono);
  font-size: 0.75rem;
  color: var(--text-primary);
  background: var(--bg-card);
  padding: 0.1rem 0.35rem;
  border-radius: var(--radius-sm);
}

.ref-entry span {
  font-size: 0.68rem;
  color: var(--text-muted);
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
  font-size: 0.8rem;
  color: var(--danger);
}

.save-success {
  font-size: 0.8rem;
  color: var(--success);
  font-weight: 600;
}

.empty-state {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 1rem 0;
}

.results-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
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

.result-status-badge {
  font-size: 0.7rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  padding: 0.15rem 0.4rem;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
}

.result-failed .result-status-badge {
  color: var(--danger);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

.result-warning .result-status-badge {
  color: var(--warning);
  background: color-mix(in srgb, var(--warning) 10%, transparent);
}

.result-success .result-status-badge {
  color: var(--success);
  background: color-mix(in srgb, var(--success) 10%, transparent);
}

.result-date {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.result-duration {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-left: auto;
}

.result-stats {
  display: flex;
  gap: 1rem;
  font-size: 0.75rem;
  color: var(--text-secondary);
}

.result-warnings,
.result-error {
  margin-top: 0.5rem;
}

.result-warnings summary {
  font-size: 0.75rem;
  color: var(--warning);
  cursor: pointer;
}

.result-output {
  font-size: 0.7rem;
  background: var(--bg-code, var(--bg-hover));
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
  font-size: 0.7rem;
  color: var(--accent);
  margin-top: 0.4rem;
  display: block;
}

.result-expand-hint {
  font-size: 0.7rem;
  color: var(--text-muted);
  margin-top: 0.4rem;
  display: block;
}

.result-section-label {
  font-size: 0.7rem;
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
  font-size: 0.875rem;
  outline: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  transition: border-color 0.15s;
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
  font-size: 0.65rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.multi-select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: var(--bg-elevated, var(--bg-card));
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg, var(--shadow));
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
  font-size: 0.85rem;
  color: var(--text-secondary);
  transition: background 0.1s;
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
.segmented-control {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.seg-btn {
  padding: 0.4rem 1rem;
  border: none;
  background: var(--bg-input);
  color: var(--text-muted);
  font-size: 0.82rem;
  font-weight: 500;
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s;
}

.seg-btn + .seg-btn {
  border-left: 1px solid var(--border);
}

.seg-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.seg-btn.active {
  background: var(--accent);
  color: #fff;
  font-weight: 600;
}

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
  font-size: 0.7rem;
  font-weight: 700;
  color: var(--text-muted);
  min-width: 1.2rem;
  text-align: center;
}

.order-name {
  flex: 1;
  font-size: 0.85rem;
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
  font-size: 1.1rem;
  cursor: pointer;
  transition:
    background 0.1s,
    color 0.1s;
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
.danger-zone {
  border-color: var(--danger);
  margin-top: 2rem;
}

.danger-zone .info-title {
  color: var(--danger);
}

.danger-body {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1.5rem;
}

.danger-info {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.danger-heading {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
}

.danger-desc {
  font-size: 0.8rem;
  color: var(--text-muted);
}

/* Dialog */
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
}

.dialog {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  width: min(480px, 90vw);
  box-shadow: var(--shadow-lg, var(--shadow));
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.dialog-title {
  font-size: 1rem;
  font-weight: 700;
  margin: 0;
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  line-height: 1;
}

.close-btn:hover {
  color: var(--text-primary);
}

.dialog-body {
  padding: 1.25rem;
  font-size: 0.875rem;
  color: var(--text-secondary);
  line-height: 1.6;
}

.dialog-body p {
  margin: 0 0 0.75rem;
}

.dialog-body p:last-child {
  margin-bottom: 0;
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  padding: 1rem 1.25rem;
  border-top: 1px solid var(--border);
}
</style>
