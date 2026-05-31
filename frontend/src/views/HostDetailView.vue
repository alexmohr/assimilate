<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { formatDate, formatBytes, formatDuration } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { cronToHuman } from '../utils/cron'
import { parseLines } from '../utils/validation'
import BaseSpinner from '../components/BaseSpinner.vue'
import { Trash2 } from '@lucide/vue'

type TabId = 'overview' | 'schedules' | 'backups'

interface ClientRow {
  id: number
  hostname: string
  display_name: string | null
  agent_version: string | null
  agent_git_sha: string | null
  agent_build_time: string | null
  created_at: string
  last_seen_at: string | null
  is_connected: boolean
  supports_restart: boolean
  restart_unavailable_reason: string | null
  default_backup_paths: string[]
  default_exclude_patterns: string[]
}

interface RepoRow {
  id: number
  target_name: string
}

interface ScheduleRow {
  id: number
  repo_id: number
  cron_expression: string
  enabled: boolean
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
}

interface ReportRow {
  id: number
  machine_id: number
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
}

interface TagRow {
  id: number
  name: string
  color: string
  scope: string
}

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

const client = ref<ClientRow | null>(null)
const repos = ref<RepoRow[]>([])
const schedules = ref<ScheduleRow[]>([])
const reports = ref<ReportRow[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const expandedReportIds = ref<Set<number>>(new Set())

// Tags
const allHostTags = ref<TagRow[]>([])
const hostTagIds = ref<number[]>([])
const tagsLoading = ref(false)
const newTagName = ref('')
const newTagColor = ref('#6b7280')
const createTagLoading = ref(false)

const isAdmin = computed(() => authStore.user?.role === 'admin')

const hostTags = computed<TagRow[]>(() =>
  allHostTags.value.filter((t) => hostTagIds.value.includes(t.id)),
)

const availableTags = computed<TagRow[]>(() =>
  allHostTags.value.filter((t) => !hostTagIds.value.includes(t.id)),
)

// Token regen
const showTokenDialog = ref(false)
const regenToken = ref<string | null>(null)
const regenLoading = ref(false)
const regenError = ref<string | null>(null)
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

// Restart agent
const restartLoading = ref(false)
const restartError = ref<string | null>(null)

// Default backup paths
const editingPaths = ref(false)
const pathsText = ref('')
const pathsSaving = ref(false)
const pathsError = ref<string | null>(null)

function startEditPaths(): void {
  pathsText.value = (client.value?.default_backup_paths ?? []).join('\n')
  pathsError.value = null
  editingPaths.value = true
}

function cancelEditPaths(): void {
  editingPaths.value = false
}

async function savePaths(): Promise<void> {
  if (!client.value) return
  pathsSaving.value = true
  pathsError.value = null
  try {
    const res = await apiClient.put<ClientRow>(`/clients/${client.value.hostname}`, {
      display_name: client.value.display_name,
      default_backup_paths: parseLines(pathsText.value),
      default_exclude_patterns: client.value.default_exclude_patterns,
    })
    client.value = { ...client.value, ...res.data }
    editingPaths.value = false
  } catch (e: unknown) {
    pathsError.value = extractError(e)
  } finally {
    pathsSaving.value = false
  }
}

const editingExcludes = ref(false)
const excludesText = ref('')
const excludesSaving = ref(false)
const excludesError = ref<string | null>(null)

// Hostname Aliases (patterns)
interface HostnamePattern {
  id: number
  client_id: number
  pattern: string
  created_at: string
}

const hostnamePatterns = ref<HostnamePattern[]>([])
const newPattern = ref('')
const patternAddLoading = ref(false)
const patternError = ref<string | null>(null)

function startEditExcludes(): void {
  excludesText.value = (client.value?.default_exclude_patterns ?? []).join('\n')
  excludesError.value = null
  editingExcludes.value = true
}

function cancelEditExcludes(): void {
  editingExcludes.value = false
}

async function saveExcludes(): Promise<void> {
  if (!client.value) return
  excludesSaving.value = true
  excludesError.value = null
  try {
    const res = await apiClient.put<ClientRow>(`/clients/${client.value.hostname}`, {
      display_name: client.value.display_name,
      default_backup_paths: client.value.default_backup_paths,
      default_exclude_patterns: parseLines(excludesText.value),
    })
    client.value = { ...client.value, ...res.data }
    editingExcludes.value = false
  } catch (e: unknown) {
    excludesError.value = extractError(e)
  } finally {
    excludesSaving.value = false
  }
}

useEscapeKey(showTokenDialog, () => {
  showTokenDialog.value = false
})

async function loadHostnamePatterns(): Promise<void> {
  if (!client.value) return
  try {
    const res = await apiClient.get<HostnamePattern[]>(
      `/clients/${client.value.hostname}/hostname-patterns`,
    )
    hostnamePatterns.value = res.data
  } catch (e: unknown) {
    logger.error('loadHostnamePatterns failed', e)
  }
}

async function addHostnamePattern(): Promise<void> {
  if (!client.value || !newPattern.value.trim()) return
  patternAddLoading.value = true
  patternError.value = null
  try {
    const res = await apiClient.post<HostnamePattern>(
      `/clients/${client.value.hostname}/hostname-patterns`,
      { pattern: newPattern.value.trim() },
    )
    hostnamePatterns.value = [...hostnamePatterns.value, res.data]
    newPattern.value = ''
  } catch (e: unknown) {
    patternError.value = extractError(e)
  } finally {
    patternAddLoading.value = false
  }
}

async function deleteHostnamePattern(id: number): Promise<void> {
  if (!client.value) return
  try {
    await apiClient.delete(`/clients/${client.value.hostname}/hostname-patterns/${id}`)
    hostnamePatterns.value = hostnamePatterns.value.filter((p) => p.id !== id)
  } catch (e: unknown) {
    patternError.value = extractError(e)
  }
}

async function deleteSchedule(id: number): Promise<void> {
  try {
    await apiClient.delete(`/schedules/${id}`)
    schedules.value = schedules.value.filter((s) => s.id !== id)
  } catch (e: unknown) {
    error.value = extractError(e)
  }
}

function isOnline(client: ClientRow): boolean {
  return client.is_connected
}

function statusClass(status: string): string {
  const s = status.toLowerCase()
  if (s === 'success') return 'badge-success'
  if (s === 'warning') return 'badge-warning'
  return 'badge-failed'
}

function hasDetails(r: ReportRow): boolean {
  return r.warnings.length > 0 || r.error_message !== null
}

function toggleReportExpand(id: number): void {
  const next = new Set(expandedReportIds.value)
  if (next.has(id)) {
    next.delete(id)
  } else {
    next.add(id)
  }
  expandedReportIds.value = next
}

async function loadClient(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const res = await apiClient.get<ClientRow[]>('/clients')
    client.value = res.data.find((m) => m.hostname === props.hostname) ?? null
    if (!client.value) {
      error.value = `Client "${props.hostname}" not found`
      return
    }
    await Promise.all([loadTabData(), loadTags(), loadHostnamePatterns()])
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

async function loadTabData(): Promise<void> {
  if (!client.value) return
  const hostname = client.value.hostname
  try {
    const [repoRes, schedRes, reportRes] = await Promise.all([
      apiClient.get<RepoRow[]>(`/clients/${hostname}/repos`),
      apiClient.get<ScheduleRow[]>('/schedules'),
      apiClient.get<ReportRow[]>(`/clients/${hostname}/reports`),
    ])
    repos.value = repoRes.data
    schedules.value = schedRes.data
    reports.value = reportRes.data
  } catch (e: unknown) {
    logger.error('loadTabData failed', e)
  }
}

const clientSchedules = computed(() => {
  const repoIds = new Set(repos.value.map((r) => r.id))
  return schedules.value.filter((s) => repoIds.has(s.repo_id))
})

function repoNameForSchedule(s: ScheduleRow): string {
  return repos.value.find((r) => r.id === s.repo_id)?.target_name ?? `repo #${s.repo_id}`
}

// Token regeneration
async function regenerateToken(): Promise<void> {
  regenLoading.value = true
  regenError.value = null
  regenToken.value = null
  tokenCopied.value = false
  try {
    const res = await apiClient.post<{ client: ClientRow; token: string }>(
      `/clients/${props.hostname}/regenerate-token`,
    )
    regenToken.value = res.data.token
    client.value = res.data.client
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
    await apiClient.post(`/clients/${props.hostname}/restart`)
  } catch (e: unknown) {
    restartError.value = extractError(e)
  } finally {
    restartLoading.value = false
  }
}

async function loadTags(): Promise<void> {
  tagsLoading.value = true
  try {
    const [tagsRes, hostTagsRes] = await Promise.all([
      apiClient.get<TagRow[]>('/tags', { params: { scope: 'host' } }),
      apiClient.get<TagRow[]>(`/clients/${props.hostname}/tags`).catch((e: unknown) => {
        logger.error('load host tags failed', e)
        return { data: [] as TagRow[] }
      }),
    ])
    allHostTags.value = tagsRes.data
    hostTagIds.value = hostTagsRes.data.map((t) => t.id)
  } catch (e: unknown) {
    logger.error('loadTags failed', e)
  } finally {
    tagsLoading.value = false
  }
}

async function addTag(tagId: number): Promise<void> {
  const updated = [...hostTagIds.value, tagId]
  try {
    await apiClient.put(`/clients/${props.hostname}/tags`, { tag_ids: updated })
    hostTagIds.value = updated
  } catch (e: unknown) {
    logger.error('addTag failed', e)
  }
}

async function removeTag(tagId: number): Promise<void> {
  const updated = hostTagIds.value.filter((id) => id !== tagId)
  try {
    await apiClient.put(`/clients/${props.hostname}/tags`, { tag_ids: updated })
    hostTagIds.value = updated
  } catch (e: unknown) {
    logger.error('removeTag failed', e)
  }
}

async function createAndAddTag(): Promise<void> {
  if (!newTagName.value.trim()) return
  createTagLoading.value = true
  try {
    const res = await apiClient.post<TagRow>('/tags', {
      name: newTagName.value.trim(),
      color: newTagColor.value,
      scope: 'host',
    })
    allHostTags.value.push(res.data)
    await addTag(res.data.id)
    newTagName.value = ''
    newTagColor.value = '#6b7280'
  } catch (e: unknown) {
    logger.error('createAndAddTag failed', e)
  } finally {
    createTagLoading.value = false
  }
}

watch(
  () => props.hostname,
  () => {
    loadClient()
  },
)
onMounted(loadClient)

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('DataChanged', () => loadClient().catch(logger.error))
onMessage('AgentConnected', () => loadClient().catch(logger.error))
onMessage('AgentDisconnected', () => loadClient().catch(logger.error))

interface BackupPayload {
  hostname: string
  target_name: string
}

const activeBackups = ref<string[]>([])

onMessage<BackupPayload>('BackupStarted', (payload) => {
  if (payload.hostname === props.hostname && !activeBackups.value.includes(payload.target_name)) {
    activeBackups.value = [...activeBackups.value, payload.target_name]
  }
})

onMessage<BackupPayload>('BackupCompleted', (payload) => {
  if (payload.hostname === props.hostname) {
    activeBackups.value = activeBackups.value.filter((t) => t !== payload.target_name)
  }
  loadClient().catch(logger.error)
})

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    loadClient().catch(logger.error)
  }
})
</script>

<template>
  <div class="host-detail">
    <!-- Breadcrumb -->
    <nav class="breadcrumb">
      <RouterLink
        to="/clients"
        class="crumb-link"
      >
        Clients
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

    <template v-else-if="client">
      <!-- Tab bar -->
      <div class="tab-bar">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          class="tab-btn"
          :class="{ active: activeTab === tab.id }"
          @click="activeTab = tab.id"
        >
          {{ tab.label }}
        </button>
      </div>

      <!-- Overview Tab -->
      <div
        v-if="activeTab === 'overview'"
        class="tab-content"
      >
        <div class="info-card">
          <h3 class="info-title">Client Information</h3>
          <dl class="info-grid">
            <dt>Hostname</dt>
            <dd class="mono">
              {{ client.hostname }}
            </dd>
            <dt>Display Name</dt>
            <dd>{{ client.display_name ?? '—' }}</dd>
            <dt>Status</dt>
            <dd>
              <span
                class="status-badge"
                :class="isOnline(client) ? 'status-online' : 'status-offline'"
              >
                {{ isOnline(client) ? 'Online' : 'Offline' }}
              </span>
            </dd>
            <dt>Agent Version</dt>
            <dd class="mono">
              {{ client.agent_version ?? '—' }}
            </dd>
            <dt>Revision</dt>
            <dd class="mono">
              {{ client.agent_git_sha ?? '—' }}
            </dd>
            <dt>Built</dt>
            <dd class="mono">
              {{ client.agent_build_time ?? '—' }}
            </dd>
            <dt>Created</dt>
            <dd>{{ formatDate(client.created_at, 'Never') }}</dd>
            <dt>Last Seen</dt>
            <dd>{{ formatDate(client.last_seen_at, 'Never') }}</dd>
            <dt>Repositories</dt>
            <dd>{{ repos.length }}</dd>
          </dl>
          <div
            v-if="activeBackups.length > 0"
            class="active-backup-banner"
          >
            <span class="active-pulse" />
            <span class="active-backup-label">Backup in progress:</span>
            <span class="active-backup-targets">{{ activeBackups.join(', ') }}</span>
          </div>
          <div class="info-actions">
            <button
              class="btn btn-sm btn-ghost"
              :disabled="regenLoading"
              @click="regenerateToken"
            >
              {{ regenLoading ? 'Regenerating...' : 'Regenerate Token' }}
            </button>
            <button
              v-if="client.supports_restart"
              class="btn btn-sm btn-ghost btn-danger-text"
              :disabled="restartLoading || !isOnline(client)"
              @click="restartAgent"
            >
              {{ restartLoading ? 'Restarting...' : 'Restart Agent' }}
            </button>
            <span
              v-else-if="isOnline(client) && client.restart_unavailable_reason"
              class="restart-hint"
            >
              {{ client.restart_unavailable_reason }}
            </span>
            <div
              v-if="restartError"
              class="form-error"
            >
              {{ restartError }}
            </div>
          </div>
        </div>

        <!-- Tags -->
        <div
          v-if="isAdmin"
          class="info-card"
        >
          <h3 class="info-title">Tags</h3>
          <div class="tags-section">
            <div
              v-if="hostTags.length > 0"
              class="tag-list"
            >
              <span
                v-for="tag in hostTags"
                :key="tag.id"
                class="tag-pill"
                :style="{
                  background: tag.color + '22',
                  color: tag.color,
                  borderColor: tag.color + '44',
                }"
              >
                {{ tag.name }}
                <button
                  class="tag-remove"
                  @click="removeTag(tag.id)"
                >
                  &times;
                </button>
              </span>
            </div>
            <span
              v-else
              class="muted"
              >No tags assigned.</span
            >

            <div class="tag-add-row">
              <select
                v-if="availableTags.length > 0"
                class="input input-sm"
                @change="
                  (e) => {
                    const id = Number((e.target as HTMLSelectElement).value)
                    if (id) addTag(id)
                    ;(e.target as HTMLSelectElement).value = ''
                  }
                "
              >
                <option value="">Add existing tag...</option>
                <option
                  v-for="t in availableTags"
                  :key="t.id"
                  :value="t.id"
                >
                  {{ t.name }}
                </option>
              </select>
              <div class="tag-create-inline">
                <input
                  v-model="newTagName"
                  class="input input-sm"
                  placeholder="New tag name"
                />
                <input
                  v-model="newTagColor"
                  class="color-input"
                  type="color"
                />
                <button
                  class="btn btn-sm btn-ghost"
                  :disabled="!newTagName.trim() || createTagLoading"
                  @click="createAndAddTag"
                >
                  {{ createTagLoading ? '...' : '+ Create' }}
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- Default Backup Paths -->
        <div class="info-card">
          <h3 class="info-title">Default Backup Paths</h3>
          <template v-if="!editingPaths">
            <div
              v-if="client.default_backup_paths.length > 0"
              class="paths-list"
            >
              <code
                v-for="(p, idx) in client.default_backup_paths"
                :key="idx"
                class="path-item mono"
              >
                {{ p }}
              </code>
            </div>
            <span
              v-else
              class="muted"
              >No default paths configured.</span
            >
            <span class="field-hint"
              >Schedules with empty backup paths will use these defaults.</span
            >
            <div class="info-actions">
              <button
                class="btn btn-sm btn-ghost"
                @click="startEditPaths"
              >
                Edit
              </button>
            </div>
          </template>
          <template v-else>
            <textarea
              v-model="pathsText"
              class="input exclude-area"
              placeholder="Directories to back up, one per line"
              spellcheck="false"
            />
            <div
              v-if="pathsError"
              class="form-error"
            >
              {{ pathsError }}
            </div>
            <div class="info-actions">
              <button
                class="btn btn-sm btn-ghost"
                :disabled="pathsSaving"
                @click="cancelEditPaths"
              >
                Cancel
              </button>
              <button
                class="btn btn-sm btn-primary"
                :disabled="pathsSaving"
                @click="savePaths"
              >
                {{ pathsSaving ? 'Saving...' : 'Save' }}
              </button>
            </div>
          </template>
        </div>

        <!-- Default Exclude Patterns -->
        <div class="info-card">
          <h3 class="info-title">Default Exclude Patterns</h3>
          <template v-if="!editingExcludes">
            <div
              v-if="client.default_exclude_patterns.length > 0"
              class="paths-list"
            >
              <code
                v-for="(p, idx) in client.default_exclude_patterns"
                :key="idx"
                class="path-item mono"
              >
                {{ p }}
              </code>
            </div>
            <span
              v-else
              class="muted"
              >No default excludes configured.</span
            >
            <span class="field-hint"
              >Applied to all schedules on this host (unless schedule ignores them).</span
            >
            <div class="info-actions">
              <button
                class="btn btn-sm btn-ghost"
                @click="startEditExcludes"
              >
                Edit
              </button>
            </div>
          </template>
          <template v-else>
            <textarea
              v-model="excludesText"
              class="input exclude-area"
              placeholder="Exclude patterns, one per line&#10;e.g. *.cache&#10;pp:__pycache__"
              spellcheck="false"
            />
            <div
              v-if="excludesError"
              class="form-error"
            >
              {{ excludesError }}
            </div>
            <div class="info-actions">
              <button
                class="btn btn-sm btn-ghost"
                :disabled="excludesSaving"
                @click="cancelEditExcludes"
              >
                Cancel
              </button>
              <button
                class="btn btn-sm btn-primary"
                :disabled="excludesSaving"
                @click="saveExcludes"
              >
                {{ excludesSaving ? 'Saving...' : 'Save' }}
              </button>
            </div>
          </template>
        </div>

        <!-- Hostname Aliases -->
        <div class="info-card">
          <h3 class="info-title">Hostname Aliases</h3>
          <p class="field-hint">
            Glob patterns that match archive hostnames to this client during repository import.
          </p>
          <div
            v-if="hostnamePatterns.length > 0"
            class="paths-list"
          >
            <div
              v-for="p in hostnamePatterns"
              :key="p.id"
              class="pattern-row"
            >
              <code class="path-item mono">{{ p.pattern }}</code>
              <button
                class="tag-remove pattern-delete"
                title="Delete pattern"
                @click="deleteHostnamePattern(p.id)"
              >
                &times;
              </button>
            </div>
          </div>
          <span
            v-else
            class="muted"
            >No alias patterns configured.</span
          >
          <p class="field-hint">
            <code>*</code> matches any characters, <code>?</code> matches a single character.
          </p>
          <div
            v-if="patternError"
            class="form-error"
          >
            {{ patternError }}
          </div>
          <div class="pattern-add-row">
            <input
              v-model="newPattern"
              class="input input-sm"
              placeholder="e.g. myhost* or host-??"
              @keyup.enter="addHostnamePattern"
            />
            <button
              class="btn btn-sm btn-primary"
              :disabled="patternAddLoading || !newPattern.trim()"
              @click="addHostnamePattern"
            >
              {{ patternAddLoading ? 'Adding...' : 'Add Pattern' }}
            </button>
          </div>
        </div>
      </div>

      <!-- Schedules Tab -->
      <div
        v-if="activeTab === 'schedules'"
        class="tab-content"
      >
        <div class="tab-header">
          <h3 class="tab-title">Schedules</h3>
          <RouterLink
            :to="{ name: 'schedule-create', query: { client_id: client?.id } }"
            class="btn btn-primary btn-sm"
          >
            + Add Schedule
          </RouterLink>
        </div>
        <div
          v-if="clientSchedules.length === 0"
          class="state-msg"
        >
          No schedules for this client.
        </div>
        <div
          v-else
          class="schedule-grid"
        >
          <div
            v-for="s in clientSchedules"
            :key="s.id"
            class="schedule-card"
          >
            <div class="schedule-card-header">
              <span class="schedule-repo">{{ repoNameForSchedule(s) }}</span>
              <span
                class="status-badge"
                :class="s.enabled ? 'status-online' : 'status-offline'"
              >
                {{ s.enabled ? 'Active' : 'Paused' }}
              </span>
            </div>
            <div class="schedule-info">
              <div class="repo-detail">
                <span class="detail-label">Schedule</span>
                <span
                  v-if="cronToHuman(s.cron_expression)"
                  class="detail-value schedule-human"
                >
                  {{ cronToHuman(s.cron_expression) }}
                </span>
                <code class="detail-value cron-badge">{{ s.cron_expression }}</code>
              </div>
              <div class="repo-detail">
                <span class="detail-label">Next run</span>
                <span class="detail-value">{{ formatDate(s.next_run_at, 'Never') }}</span>
              </div>
              <div class="repo-detail">
                <span class="detail-label">Last run</span>
                <span class="detail-value">{{ formatDate(s.last_run_at, 'Never') }}</span>
              </div>
            </div>
            <div class="schedule-card-footer">
              <button
                class="btn btn-sm btn-ghost btn-danger-text"
                title="Delete"
                @click="deleteSchedule(s.id)"
              >
                <Trash2 :size="14" />
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- Backups Tab -->
      <div
        v-if="activeTab === 'backups'"
        class="tab-content"
      >
        <div class="tab-header">
          <h3 class="tab-title">Backup History</h3>
        </div>
        <div
          v-if="reports.length === 0"
          class="state-msg"
        >
          No backup reports available.
        </div>
        <div
          v-else
          class="table-wrap"
        >
          <table class="data-table">
            <thead>
              <tr>
                <th class="col-expand" />
                <th>Date</th>
                <th>Status</th>
                <th>Duration</th>
                <th>Original</th>
                <th>Compressed</th>
                <th>Dedup</th>
                <th>Files</th>
              </tr>
            </thead>
            <tbody>
              <template
                v-for="r in reports"
                :key="r.id"
              >
                <tr
                  :class="{
                    'row-expandable': hasDetails(r),
                    'row-expanded': expandedReportIds.has(r.id),
                  }"
                  @click="hasDetails(r) && toggleReportExpand(r.id)"
                >
                  <td class="cell-expand">
                    <span
                      v-if="hasDetails(r)"
                      class="expand-icon"
                      :class="{ open: expandedReportIds.has(r.id) }"
                      >&#9654;</span
                    >
                  </td>
                  <td class="cell-ts">
                    {{ formatDate(r.started_at, 'Never') }}
                  </td>
                  <td>
                    <span
                      class="status-badge-sm"
                      :class="statusClass(r.status)"
                      >{{ r.status }}</span
                    >
                  </td>
                  <td>{{ formatDuration(r.duration_secs) }}</td>
                  <td>{{ formatBytes(r.original_size) }}</td>
                  <td>{{ formatBytes(r.compressed_size) }}</td>
                  <td>{{ formatBytes(r.deduplicated_size) }}</td>
                  <td>{{ r.files_processed.toLocaleString() }}</td>
                </tr>
                <tr
                  v-if="expandedReportIds.has(r.id)"
                  class="detail-row"
                >
                  <td :colspan="8">
                    <div class="report-detail">
                      <div
                        v-if="r.warnings.length > 0"
                        class="detail-section"
                      >
                        <span class="detail-heading">Warnings ({{ r.warnings.length }})</span>
                        <ul class="warning-list">
                          <li
                            v-for="(w, i) in r.warnings"
                            :key="i"
                            class="warning-item"
                          >
                            {{ w }}
                          </li>
                        </ul>
                      </div>
                      <div
                        v-if="r.error_message"
                        class="detail-section"
                      >
                        <span class="detail-heading">Error Output</span>
                        <pre class="error-pre">{{ r.error_message }}</pre>
                      </div>
                    </div>
                  </td>
                </tr>
              </template>
            </tbody>
          </table>
        </div>
      </div>
    </template>

    <!-- Token Dialog -->
    <Teleport to="body">
      <div
        v-if="showTokenDialog"
        class="overlay"
        @click.self="showTokenDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ regenToken ? 'New Token Generated' : 'Error' }}
            </h2>
            <button
              class="close-btn"
              @click="showTokenDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
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
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showTokenDialog = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.host-detail {
  max-width: 1100px;
}

.active-backup-banner {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 1rem;
  padding: 0.6rem 0.75rem;
  background: var(--accent-subtle, oklch(0.95 0.03 250));
  border: 1px solid var(--accent);
  border-radius: var(--radius-sm);
  font-size: 0.8rem;
}

.active-pulse {
  width: 7px;
  height: 7px;
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

.active-backup-label {
  font-weight: 600;
  color: var(--text-primary);
}

.active-backup-targets {
  font-family: var(--mono);
  color: var(--accent);
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

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

/* Tab bar */
.tab-bar {
  display: flex;
  gap: 0.25rem;
  border-bottom: 1px solid var(--border);
  margin-bottom: 1.5rem;
}

.tab-btn {
  padding: 0.75rem 1.25rem;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-secondary);
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
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
  font-weight: 600;
}

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
  font-size: 1.1rem;
  font-weight: 600;
}

/* Info card (Overview) */
.info-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;

  & + & {
    margin-top: 0.75rem;
  }
}

.info-title {
  font-size: 0.95rem;
  font-weight: 600;
  margin-bottom: 1.25rem;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-size: 0.8rem;
}

.info-grid {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.6rem 1.5rem;
  margin: 0;
}

.info-grid dt {
  color: var(--text-muted);
  font-size: 0.85rem;
  font-weight: 500;
}

.info-grid dd {
  margin: 0;
  color: var(--text-primary);
  font-size: 0.85rem;
}

.info-actions {
  margin-top: 1.5rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.75rem;
}

.restart-hint {
  font-size: 0.78rem;
  color: var(--text-muted);
  font-style: italic;
}

/* Tags */
.muted {
  color: var(--text-muted);
  font-size: 0.85rem;
}

.tags-section {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.tag-list {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
}

.tag-pill {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.2rem 0.5rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 500;
  border: 1px solid;
}

.tag-remove {
  background: none;
  border: none;
  color: inherit;
  cursor: pointer;
  font-size: 0.9rem;
  line-height: 1;
  padding: 0;
  opacity: 0.6;
  transition: opacity 0.15s;
}

.tag-remove:hover {
  opacity: 1;
}

.tag-add-row {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
  align-items: center;
}

.tag-create-inline {
  display: flex;
  gap: 0.4rem;
  align-items: center;
}

.color-input {
  width: 28px;
  height: 28px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  background: transparent;
}

.input-sm {
  padding: 0.35rem 0.55rem;
  font-size: 0.8rem;
  width: auto;
  min-width: 140px;
}

/* Status badges */

.status-badge-sm {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: capitalize;
}

.badge-success {
  background: var(--success-subtle);
  color: var(--success);
}

.badge-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-failed {
  background: var(--danger-subtle);
  color: var(--danger);
}

/* Repos grid */
.repo-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
  gap: 1rem;
}

.repo-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
}

.repo-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.repo-name {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
}

.repo-details {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.repo-detail {
  display: flex;
  align-items: baseline;
  gap: 0.75rem;
}

.detail-label {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  flex-shrink: 0;
  min-width: 80px;
}

.detail-value {
  font-size: 0.825rem;
  color: var(--text-secondary);
}

.cron-badge {
  font-family: var(--mono);
  font-size: 0.8rem;
  padding: 0.15rem 0.4rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.schedule-human {
  font-size: 0.82rem;
  font-weight: 600;
  color: var(--text-primary);
}

.repo-card-footer {
  display: flex;
  justify-content: flex-end;
  margin-top: auto;
}

/* Schedule cards */
.schedule-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 1rem;
}

.schedule-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
}

.schedule-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.schedule-repo {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
}

.schedule-info {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.schedule-card-footer {
  display: flex;
  justify-content: flex-end;
  margin-top: auto;
}

/* Table */
.table-wrap {
  overflow-x: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.data-table th {
  text-align: left;
  padding: 0.7rem 1rem;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
}

.data-table td {
  padding: 0.65rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.data-table tr:last-child td {
  border-bottom: none;
}

.data-table tr:hover td {
  background: var(--bg-hover);
}

.cell-ts {
  white-space: nowrap;
  color: var(--text-muted);
  font-size: 0.8rem;
}

.col-expand {
  width: 2rem;
}

.cell-expand {
  width: 2rem;
  text-align: center;
  padding-left: 0.5rem;
  padding-right: 0;
}

.expand-icon {
  display: inline-block;
  font-size: 0.6rem;
  color: var(--text-muted);
  transition: transform 0.15s;
}

.expand-icon.open {
  transform: rotate(90deg);
}

.row-expandable {
  cursor: pointer;
}

.row-expanded td {
  border-bottom-color: transparent;
}

.detail-row td {
  padding: 0 1rem 0.75rem;
  background: var(--bg-card);
}

.report-detail {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  padding: 0.5rem 0 0.25rem 2rem;
}

.detail-section {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.detail-heading {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
}

.warning-list {
  margin: 0;
  padding: 0 0 0 1.25rem;
  list-style: none;
}

.warning-item {
  font-size: 0.8rem;
  font-family: var(--mono);
  color: var(--warning);
  padding: 0.15rem 0;
}

.warning-item::before {
  content: '\25CF ';
  font-size: 0.5rem;
  vertical-align: middle;
  margin-right: 0.35rem;
}

.error-pre {
  margin: 0;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: 0.78rem;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
}

/* Overlay & Dialog */

.dialog-lg {
  width: 680px;
}

/* Form */

.input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0 1rem;
}

.field-full {
  grid-column: 1 / -1;
}

.field-narrow {
  max-width: 120px;
}

.field-group {
  grid-column: 1 / -1;
  display: flex;
  gap: 1rem;
  align-items: flex-start;
}

.section-divider {
  grid-column: 1 / -1;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  border-bottom: 1px solid var(--border);
  padding-bottom: 0.3rem;
  margin-top: 0.25rem;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.token-warning {
  color: var(--warning);
  font-size: 0.875rem;
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

.paths-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin-bottom: 0.5rem;
}

.path-item {
  font-size: 0.82rem;
  padding: 0.2rem 0.5rem;
  background: var(--bg-input);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}

.cmd-area {
  min-height: 60px;
  resize: vertical;
  font-family: var(--mono);
  font-size: 0.82rem;
  line-height: 1.5;
}

.pattern-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.pattern-delete {
  font-size: 1rem;
  flex-shrink: 0;
}

.pattern-add-row {
  display: flex;
  gap: 0.5rem;
  align-items: center;
  margin-top: 0.75rem;
  flex-wrap: wrap;
}
</style>
