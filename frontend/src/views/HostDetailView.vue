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
import { formatDate, formatDateShort, formatBytes, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { cronToHuman } from '../utils/cron'
import { parseLines } from '../utils/validation'
import BaseSpinner from '../components/BaseSpinner.vue'
import MergeClientDialog from '../components/MergeClientDialog.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import SshKeyDeployPanel from '../components/SshKeyDeployPanel.vue'

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
  is_imported: boolean
  is_hidden: boolean
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
  repo_id: number | null
  name: string
  schedule_type: string
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
  execution_mode: string
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
  archive_name: string | null
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
const expandedReportId = ref<number | null>(null)

// Backup filter / sort
const filterStatus = ref<'all' | 'success' | 'warning' | 'failed'>('all')
const sortAscending = ref(false)

const highlightedArchiveName = computed(() => {
  const a = route.query.archive
  return typeof a === 'string' ? a : undefined
})

const filteredSortedReports = computed(() => {
  let result = reports.value
  if (filterStatus.value !== 'all') {
    result = result.filter((r) => r.status === filterStatus.value)
  }
  return [...result].sort((a, b) => {
    const diff = new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
    return sortAscending.value ? -diff : diff
  })
})

// Tags
const allHostTags = ref<TagRow[]>([])
const hostTagIds = ref<number[]>([])
const tagsLoading = ref(false)
const newTagName = ref('')
const newTagColor = ref('#6b7280')
const createTagLoading = ref(false)

const isAdmin = computed(() => authStore.user?.role === 'admin')
const isImported = computed(() => client.value?.is_imported ?? false)

const hostTags = computed<TagRow[]>(() =>
  allHostTags.value.filter((t) => hostTagIds.value.includes(t.id)),
)

const availableTags = computed<TagRow[]>(() =>
  allHostTags.value.filter((t) => !hostTagIds.value.includes(t.id)),
)

// Merge dialog
const allClients = ref<ClientRow[]>([])
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
const showDeployDialog = ref(false)

// Deploy SSH key
const showDeploySshKey = ref(false)

function deployButtonLabel(): string | null {
  if (!client.value) return null
  if (!client.value.agent_version) return 'Deploy'
  if (availableAgentVersion.value && client.value.agent_version === availableAgentVersion.value)
    return null
  return 'Upgrade'
}

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

// Hostname & display name editing
const editingIdentity = ref(false)
const identityHostname = ref('')
const identityDisplayName = ref('')
const identitySaving = ref(false)
const identityError = ref<string | null>(null)

function startEditIdentity(): void {
  if (!client.value) return
  identityHostname.value = client.value.hostname
  identityDisplayName.value = client.value.display_name ?? ''
  identityError.value = null
  editingIdentity.value = true
}

function cancelEditIdentity(): void {
  editingIdentity.value = false
}

async function saveIdentity(): Promise<void> {
  if (!client.value) return
  identitySaving.value = true
  identityError.value = null
  try {
    const oldHostname = client.value.hostname
    const newHostname = identityHostname.value.trim()
    const hostnameChanged = newHostname !== oldHostname && newHostname.length > 0
    const res = await apiClient.put<ClientRow>(`/clients/${oldHostname}`, {
      hostname: hostnameChanged ? newHostname : undefined,
      display_name: identityDisplayName.value.trim() || null,
      default_backup_paths: client.value.default_backup_paths,
      default_exclude_patterns: client.value.default_exclude_patterns,
    })
    if (hostnameChanged) {
      pendingAliasOldHostname.value = oldHostname
      pendingAliasNewHostname.value = newHostname
      showAliasConfirm.value = true
      router.replace(`/clients/${newHostname}`)
    }
    client.value = { ...client.value, ...res.data }
    editingIdentity.value = false
  } catch (e: unknown) {
    identityError.value = extractError(e)
  } finally {
    identitySaving.value = false
  }
}

// Hostname alias confirmation
const showAliasConfirm = ref(false)
const pendingAliasOldHostname = ref('')
const pendingAliasNewHostname = ref('')

useEscapeKey(showAliasConfirm, () => {
  showAliasConfirm.value = false
})

async function confirmAddAlias(): Promise<void> {
  await apiClient.post(`/clients/${pendingAliasNewHostname.value}/hostname-patterns`, {
    pattern: pendingAliasOldHostname.value,
  })
  await loadHostnamePatterns(pendingAliasNewHostname.value)
  showAliasConfirm.value = false
}

function declineAlias(): void {
  showAliasConfirm.value = false
}
const deleteLoading = ref(false)
const showDeleteDialog = ref(false)

useEscapeKey(showDeleteDialog, () => {
  showDeleteDialog.value = false
})

async function confirmDeleteHost(): Promise<void> {
  if (!client.value) return
  deleteLoading.value = true
  try {
    await apiClient.delete(`/clients/${client.value.hostname}`)
    router.push('/clients')
  } catch (e: unknown) {
    logger.error('Failed to delete host', e)
  } finally {
    deleteLoading.value = false
  }
}

// Hide imported client
const hideLoading = ref(false)

async function hideClient(): Promise<void> {
  if (!client.value) return
  hideLoading.value = true
  try {
    await apiClient.put(`/clients/${client.value.hostname}/hide`)
    router.push('/clients')
  } catch (e: unknown) {
    logger.error('Failed to hide client', e)
  } finally {
    hideLoading.value = false
  }
}

// Delete archives & remove imported client
const showDeleteArchivesDialog = ref(false)
const deleteArchivesLoading = ref(false)

useEscapeKey(showDeleteArchivesDialog, () => {
  showDeleteArchivesDialog.value = false
})

async function confirmDeleteArchives(): Promise<void> {
  if (!client.value) return
  deleteArchivesLoading.value = true
  try {
    await apiClient.post(`/clients/${client.value.hostname}/delete-archives`)
    router.push('/clients')
  } catch (e: unknown) {
    logger.error('Failed to delete archives', e)
  } finally {
    deleteArchivesLoading.value = false
  }
}

interface CreateClientResponse {
  client: ClientRow
  token: string
}

async function adoptHost(): Promise<void> {
  if (!client.value) return
  try {
    const cleanDisplayName =
      client.value.display_name?.replace(/\s*\(imported\)$/, '').trim() || null
    await apiClient.put(`/clients/${client.value.hostname}`, {
      display_name: cleanDisplayName,
    })
    const res = await apiClient.post<CreateClientResponse>(
      `/clients/${client.value.hostname}/regenerate-token`,
    )
    client.value = {
      ...client.value,
      ...res.data.client,
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
  router.push('/clients')
}

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

async function loadHostnamePatterns(hostname?: string): Promise<void> {
  const h = hostname ?? client.value?.hostname
  if (!h) return
  try {
    const res = await apiClient.get<HostnamePattern[]>(`/clients/${h}/hostname-patterns`)
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

function isOnline(client: ClientRow): boolean {
  return client.is_connected
}

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

async function loadClient(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const res = await apiClient.get<ClientRow[]>('/clients')
    allClients.value = res.data
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

const clientSchedules = computed(() => {
  const repoIds = new Set(repos.value.map((r) => r.id))
  return schedules.value.filter((s) => s.repo_id != null && repoIds.has(s.repo_id))
})

function repoNameForSchedule(s: ScheduleRow): string {
  return (
    repos.value.find((r) => r.id === s.repo_id)?.target_name ??
    (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
  )
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

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
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
onMounted(() => {
  loadClient()
  apiClient
    .get<{ agent_version: string | null }>('/system/version')
    .then((res) => {
      availableAgentVersion.value = res.data.agent_version
    })
    .catch(logger.error)
})

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
              v-if="client.supports_restart && !isImported"
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
            <button
              v-if="deployButtonLabel() && !isImported"
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
              @click="showDeploySshKey = false"
            >
              &times;
            </button>
          </div>
          <SshKeyDeployPanel
            :ssh-host="client.hostname"
            show-credentials
          />
        </div>

        <!-- Edit Identity -->
        <div
          v-if="editingIdentity"
          class="info-card"
        >
          <h3 class="info-title">Edit Host Identity</h3>
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
                  v-if="!isImported"
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

            <div
              v-if="!isImported"
              class="tag-add-row"
            >
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
                v-if="!isImported"
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
                v-if="!isImported"
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
              placeholder="Exclude patterns, one per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
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
            Glob patterns that match archive hostnames to this client during repository import. Only
            affects future discoveries — existing imported clients are not retroactively reassigned.
            Use "Merge into" on an imported client to move its historical archives.
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
                v-if="!isImported"
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
          <div
            v-if="!isImported"
            class="pattern-add-row"
          >
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

        <!-- Danger Zone -->
        <div
          v-if="isAdmin"
          class="info-card danger-zone"
        >
          <h3 class="info-title">Danger Zone</h3>
          <template v-if="isImported">
            <div class="danger-body">
              <div class="danger-info">
                <span class="danger-heading">Hide Client</span>
                <span class="danger-desc">
                  Hide this imported client from the default list view.
                </span>
              </div>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="hideLoading"
                @click="hideClient"
              >
                {{ hideLoading ? 'Hiding...' : 'Hide' }}
              </button>
            </div>
            <div class="danger-body danger-body-sep">
              <div class="danger-info">
                <span class="danger-heading">Delete Archives &amp; Remove</span>
                <span class="danger-desc">
                  Permanently delete all borg archives and remove this client. This is irreversible.
                </span>
              </div>
              <button
                class="btn btn-sm btn-danger"
                :disabled="deleteArchivesLoading"
                @click="showDeleteArchivesDialog = true"
              >
                {{ deleteArchivesLoading ? 'Deleting...' : 'Delete Archives & Remove' }}
              </button>
            </div>
          </template>
          <template v-else>
            <div class="danger-body">
              <div class="danger-info">
                <span class="danger-heading">Delete Host</span>
                <span class="danger-desc">
                  Permanently remove this host and all associated data. This action cannot be
                  undone.
                </span>
              </div>
              <button
                class="btn btn-sm btn-danger"
                :disabled="deleteLoading"
                @click="showDeleteDialog = true"
              >
                {{ deleteLoading ? 'Deleting...' : 'Delete Host' }}
              </button>
            </div>
          </template>
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
            :class="{ disabled: !s.enabled }"
            @click="navigateToSchedule(s)"
          >
            <div class="card-top">
              <div class="card-info">
                <span class="card-hostname">{{ s.name || repoNameForSchedule(s) }}</span>
                <span class="card-repo">
                  {{ s.execution_mode === 'sequential' ? 'Sequential' : 'Parallel' }}
                </span>
              </div>
              <div class="card-badges">
                <span
                  class="status-badge"
                  :class="s.enabled ? 'status-online' : 'status-offline'"
                >
                  {{ s.enabled ? 'Enabled' : 'Disabled' }}
                </span>
              </div>
            </div>
            <div class="card-meta">
              <span
                class="type-badge"
                :class="`type-${s.schedule_type ?? 'backup'}`"
              >
                {{ scheduleTypeLabel(s.schedule_type ?? 'backup') }}
              </span>
            </div>
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{
                  cronToHuman(s.cron_expression) ?? s.cron_expression
                }}</span>
                <span class="stat-label">Schedule</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ formatDateShort(s.next_run_at) }}</span>
                <span class="stat-label">Next run</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ formatDateShort(s.last_run_at) }}</span>
                <span class="stat-label">Last run</span>
              </div>
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
                'result-card-highlighted': r.archive_name === highlightedArchiveName,
              },
            ]"
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

    <!-- Delete Host Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showDeleteDialog"
        class="overlay"
        @click.self="showDeleteDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Host</h2>
            <button
              class="close-btn"
              @click="showDeleteDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Permanently delete <strong>{{ client?.hostname }}</strong
              >? All associated schedules and backup reports will be removed. This action cannot be
              undone.
            </p>
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
              @click="confirmDeleteHost"
            >
              {{ deleteLoading ? 'Deleting...' : 'Delete Host' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Delete Archives Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showDeleteArchivesDialog"
        class="overlay"
        @click.self="showDeleteArchivesDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Archives &amp; Remove Client</h2>
            <button
              class="close-btn"
              @click="showDeleteArchivesDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="danger-warning-text">
              This will <strong>permanently destroy all borg archives</strong> belonging to
              <strong>{{ client?.hostname }}</strong> and remove the client from the system.
            </p>
            <p class="danger-warning-text">
              This operation is <strong>irreversible</strong>. Backup data will be permanently lost
              and cannot be recovered.
            </p>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showDeleteArchivesDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="deleteArchivesLoading"
              @click="confirmDeleteArchives"
            >
              {{ deleteArchivesLoading ? 'Deleting...' : 'Delete Archives & Remove' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Hostname Alias Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showAliasConfirm"
        class="overlay"
        @click.self="declineAlias"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Add Hostname Pattern?</h2>
            <button
              class="close-btn"
              @click="declineAlias"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Hostname changed from <strong>{{ pendingAliasOldHostname }}</strong> to
              <strong>{{ pendingAliasNewHostname }}</strong
              >.
            </p>
            <p>
              Add <code>{{ pendingAliasOldHostname }}</code> as an alternative hostname pattern so
              existing archives still match?
            </p>
          </div>
          <div class="dialog-footer">
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
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Merge Client Dialog -->
    <Teleport to="body">
      <MergeClientDialog
        v-if="showMergeDialog && client"
        :source="client"
        :all-clients="allClients"
        @merged="onMerged"
        @cancel="showMergeDialog = false"
      />
    </Teleport>

    <!-- Deploy Agent Dialog -->
    <AgentDeployDialog
      v-if="showDeployDialog && client"
      :hostname="client.hostname"
      :agent-version="client.agent_version"
      @close="showDeployDialog = false"
      @deployed="
        () => {
          showDeployDialog = false
          loadClient()
        }
      "
    />
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

.repo-card-footer {
  display: flex;
  justify-content: flex-end;
  margin-top: auto;
}

/* Schedule cards */
.schedule-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(320px, 100%), 1fr));
  gap: 1rem;
}

.schedule-card {
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
  gap: 0.75rem;
}

.schedule-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.schedule-card.disabled {
  opacity: 0.5;
}

.card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.card-badges {
  display: flex;
  gap: 0.4rem;
  align-items: center;
  flex-shrink: 0;
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

.card-repo {
  font-size: 0.78rem;
  color: var(--text-muted);
  font-family: var(--mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-meta {
  display: flex;
  gap: 0.4rem;
}

.type-badge {
  display: inline-block;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.type-backup {
  background: var(--success-subtle);
  color: var(--success);
}

.type-check {
  background: var(--accent-subtle);
  color: var(--accent);
}

.type-verify {
  background: var(--warning-subtle);
  color: var(--warning);
}

.card-stats {
  display: flex;
  gap: 1.25rem;
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

/* Danger zone */
.danger-zone {
  border-color: var(--danger);
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

.danger-body-sep {
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}

.danger-warning-text {
  font-size: 0.875rem;
  color: var(--danger);
  margin-bottom: 0.5rem;
}

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
