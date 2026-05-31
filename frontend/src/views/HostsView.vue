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
import { Plus, SlidersHorizontal, Server, AlertCircle } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import MergeClientDialog from '../components/MergeClientDialog.vue'

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
  default_backup_paths: string[]
}

interface CreateClientResponse {
  client: ClientRow
  token: string
}

interface TagRow {
  id: number
  name: string
  color: string
  scope: string
}

interface HostTagRow {
  client_id: number
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

interface HostHealth {
  failed: number
  overdue: number
  warning: number
  total: number
}

type SortField = 'hostname' | 'status' | 'last_seen' | 'version'
type SortDir = 'asc' | 'desc'
type FilterStatus = 'all' | 'online' | 'offline'

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.user?.role === 'admin')
const clients = ref<ClientRow[]>([])
const showHidden = ref(false)
const machineScheduleCount = ref<Record<number, number>>({})
const healthByHost = ref<Record<string, HostHealth>>({})
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
const showTagDropdown = ref(false)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const allHostTags = ref<TagRow[]>([])
const hostTagsMap = ref<Record<number, { name: string; color: string }[]>>({})

const filteredClients = computed(() => {
  let list = [...clients.value]

  if (filterStatus.value === 'online') {
    list = list.filter((m) => m.is_connected)
  } else if (filterStatus.value === 'offline') {
    list = list.filter((m) => !m.is_connected)
  }

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (m) =>
        m.hostname.toLowerCase().includes(q) ||
        (m.display_name?.toLowerCase().includes(q) ?? false) ||
        (hostTagsMap.value[m.id] ?? []).some((t) => t.name.toLowerCase().includes(q)),
    )
  }

  if (filterTagIds.value.length > 0) {
    const selectedNames = new Set(
      allHostTags.value.filter((t) => filterTagIds.value.includes(t.id)).map((t) => t.name),
    )
    list = list.filter((m) =>
      (hostTagsMap.value[m.id] ?? []).some((t) => selectedNames.has(t.name)),
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

// Adopt imported client
const showAdoptDialog = ref(false)
const adoptToken = ref<string | null>(null)
const adoptHostname = ref('')

const showDeployDialog = ref(false)
const deployTarget = ref<ClientRow | null>(null)

// Merge imported client
const showMergeDialog = ref(false)
const mergeSource = ref<ClientRow | null>(null)

useEscapeKey(showMergeDialog, () => {
  showMergeDialog.value = false
})
const deployLoading = ref(false)
const deployError = ref<string | null>(null)
const deployResult = ref<{
  success: boolean
  skipped: boolean
  token?: string
  available_version?: string
  error?: string
} | null>(null)
const deployForm = reactive({
  ssh_host: '',
  ssh_user: 'root',
  ssh_port: 22,
  ssh_password: '',
  server_url: '',
  install_path: '/usr/local/bin/assimilate-agent',
  use_sudo: false,
  sudo_password: '',
  systemd_service_content: '',
})

useEscapeKey(showAddDialog, closeAddDialog)

useEscapeKey(showDeployDialog, () => {
  showDeployDialog.value = false
})

function isOnline(client: ClientRow): boolean {
  return client.is_connected
}

function isImported(client: ClientRow): boolean {
  return client.is_imported
}

function formatLastSeen(iso: string | null): string {
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

function formatVersion(v: string | null): string {
  if (!v) return '\u2014'
  return v
}

function scheduleCount(client: ClientRow): number {
  return machineScheduleCount.value[client.id] ?? 0
}

function clientTags(client: ClientRow): { name: string; color: string }[] {
  return hostTagsMap.value[client.id] ?? []
}

function hostHealthStatus(client: ClientRow): HostHealth | null {
  return healthByHost.value[client.hostname] ?? null
}

function hostHasIssues(client: ClientRow): boolean {
  const h = hostHealthStatus(client)
  if (!h) return false
  return h.failed > 0 || h.overdue > 0
}

function toggleTagFilter(tagId: number): void {
  const idx = filterTagIds.value.indexOf(tagId)
  if (idx === -1) {
    filterTagIds.value = [...filterTagIds.value, tagId]
  } else {
    filterTagIds.value = filterTagIds.value.filter((id) => id !== tagId)
  }
}

async function loadClients(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [clientsRes, hostTagAssocRes, hostTagsRes, healthRes] = await Promise.all([
      apiClient.get<ClientRow[]>('/clients', {
        params: showHidden.value ? { include_hidden: true } : undefined,
      }),
      apiClient.get<HostTagRow[]>('/host-tags').catch(() => ({ data: [] as HostTagRow[] })),
      apiClient
        .get<TagRow[]>('/tags', { params: { scope: 'host' } })
        .catch(() => ({ data: [] as TagRow[] })),
      apiClient.get<HealthEntry[]>('/stats/health'),
    ])
    clients.value = clientsRes.data
    machineScheduleCount.value = {}

    allHostTags.value = hostTagsRes.data
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    hostTagAssocRes.data.forEach((ht) => {
      if (!tagMap[ht.client_id]) tagMap[ht.client_id] = []
      tagMap[ht.client_id].push({ name: ht.tag_name, color: ht.tag_color })
    })
    hostTagsMap.value = tagMap

    const hMap: Record<string, HostHealth> = {}
    healthRes.data.forEach((entry) => {
      if (!hMap[entry.hostname]) {
        hMap[entry.hostname] = { failed: 0, overdue: 0, warning: 0, total: 0 }
      }
      hMap[entry.hostname].total++
      if (entry.last_status === 'failed') hMap[entry.hostname].failed++
      if (entry.last_status === 'warning') hMap[entry.hostname].warning++
      if (entry.is_overdue) hMap[entry.hostname].overdue++
    })
    healthByHost.value = hMap
  } catch (e: unknown) {
    error.value = extractError(e)
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
    const res = await apiClient.post<CreateClientResponse>('/clients', {
      hostname,
      display_name: addForm.display_name.trim() || null,
    })
    clients.value.push(res.data.client)
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

function navigateToHost(client: ClientRow): void {
  router.push(`/clients/${client.hostname}`)
}

async function adoptClient(client: ClientRow): Promise<void> {
  try {
    const cleanDisplayName = client.display_name?.replace(/\s*\(imported\)$/, '').trim() || null
    await apiClient.put(`/clients/${client.hostname}`, {
      display_name: cleanDisplayName,
    })
    const res = await apiClient.post<CreateClientResponse>(
      `/clients/${client.hostname}/regenerate-token`,
    )
    const idx = clients.value.findIndex((m) => m.id === client.id)
    if (idx !== -1) {
      clients.value[idx] = {
        ...clients.value[idx],
        ...res.data.client,
        is_imported: false,
        display_name: cleanDisplayName,
      }
    }
    adoptHostname.value = client.hostname
    adoptToken.value = res.data.token
    tokenCopied.value = false
    showAdoptDialog.value = true
  } catch (e: unknown) {
    logger.error('Failed to adopt client', e)
  }
}

function defaultSystemdUnit(execPath: string): string {
  return `[Unit]
Description=Assimilate Backup Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${execPath}
Environment=BORG_SERVER_URL=<will be set automatically>
Environment=BORG_AGENT_TOKEN=<will be set automatically>
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
`
}

function openDeployDialog(client: ClientRow): void {
  deployTarget.value = client
  deployError.value = null
  deployResult.value = null
  deployForm.ssh_host = client.hostname
  deployForm.ssh_user = 'root'
  deployForm.ssh_port = 22
  deployForm.server_url = window.location.origin
  deployForm.install_path = '/usr/local/bin/assimilate-agent'
  deployForm.use_sudo = false
  deployForm.sudo_password = ''
  deployForm.systemd_service_content = defaultSystemdUnit('/usr/local/bin/assimilate-agent')
  showDeployDialog.value = true
}

function openMergeDialog(client: ClientRow): void {
  mergeSource.value = client
  showMergeDialog.value = true
}

async function unhideClient(client: ClientRow): Promise<void> {
  try {
    await apiClient.put(`/clients/${client.hostname}/unhide`)
    await loadClients()
  } catch (e: unknown) {
    logger.error('Failed to unhide client', e)
  }
}

function onMerged(): void {
  showMergeDialog.value = false
  loadClients().catch(logger.error)
}

async function submitDeploy(): Promise<void> {
  if (!deployTarget.value) return
  deployLoading.value = true
  deployError.value = null
  deployResult.value = null
  try {
    const res = await apiClient.post<{
      success: boolean
      skipped: boolean
      token?: string
      available_version?: string
      error?: string
    }>(`/clients/${deployTarget.value.hostname}/deploy`, {
      ssh_host: deployForm.ssh_host.trim(),
      ssh_user: deployForm.ssh_user.trim(),
      ssh_port: deployForm.ssh_port,
      ssh_password: deployForm.ssh_password || undefined,
      server_url: deployForm.server_url.trim(),
      install_path: deployForm.install_path.trim() || undefined,
      use_sudo: deployForm.use_sudo,
      sudo_password:
        deployForm.use_sudo && deployForm.sudo_password ? deployForm.sudo_password : undefined,
      systemd_service_content: deployForm.systemd_service_content.trim() || undefined,
    })
    deployResult.value = res.data
  } catch (e: unknown) {
    deployError.value = extractError(e)
  } finally {
    deployLoading.value = false
  }
}

onMounted(() => {
  loadClients().catch(logger.error)
  apiClient
    .get<{ agent_version: string | null }>('/system/version')
    .then((res) => {
      availableAgentVersion.value = res.data.agent_version
    })
    .catch(logger.error)
})

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('AgentConnected', () => loadClients().catch(logger.error))
onMessage('AgentDisconnected', () => loadClients().catch(logger.error))
onMessage('DataChanged', () => loadClients().catch(logger.error))

interface BackupPayload {
  hostname: string
  target_name: string
}

const activeBackupsByHost = ref<Record<string, string[]>>({})

const availableAgentVersion = ref<string | null>(null)
onMessage<BackupPayload>('BackupStarted', (payload) => {
  const list = activeBackupsByHost.value[payload.hostname] ?? []
  if (!list.includes(payload.target_name)) {
    activeBackupsByHost.value = {
      ...activeBackupsByHost.value,
      [payload.hostname]: [...list, payload.target_name],
    }
  }
})

onMessage<BackupPayload>('BackupCompleted', (payload) => {
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
  loadClients().catch(logger.error)
})

function hostActiveBackups(client: ClientRow): string[] {
  return activeBackupsByHost.value[client.hostname] ?? []
}

function deployButtonLabel(client: ClientRow): string | null {
  if (!client.agent_version) return 'Deploy'
  if (availableAgentVersion.value && client.agent_version === availableAgentVersion.value)
    return null
  return 'Upgrade'
}

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    loadClients().catch(logger.error)
  }
})

watch(showHidden, () => {
  loadClients().catch(logger.error)
})
</script>

<template>
  <div class="hosts-view">
    <div class="page-header">
      <h1 class="page-title">Clients</h1>
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
        :class="{ active: filterStatus !== 'all' || filterTagIds.length > 0 }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterStatus !== 'all' || filterTagIds.length > 0"
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
        <div
          v-if="isAdmin"
          class="hidden-toggle"
        >
          <ToggleSwitch v-model="showHidden" />
          <span class="hidden-toggle-label">Show hidden</span>
        </div>
        <div
          v-if="allHostTags.length > 0"
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
              v-for="tag in allHostTags"
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
      v-else-if="clients.length === 0"
      :icon="Server"
      title="No clients registered"
      description="Add your first client to start backing up."
      action="Add Client"
      @action="showAddDialog = true"
    />
    <div
      v-else-if="filteredClients.length === 0"
      class="state-msg"
    >
      No clients match the current filter.
    </div>

    <div
      v-else
      class="host-grid"
    >
      <div
        v-for="client in filteredClients"
        :key="client.id"
        class="host-card"
        :class="{ 'host-card-hidden': client.is_hidden }"
        @click="navigateToHost(client)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-hostname">{{ client.hostname }}</span>
            <span
              v-if="client.display_name"
              class="card-display"
              >{{ client.display_name }}</span
            >
          </div>
          <div class="card-top-badges">
            <span
              v-if="client.is_hidden"
              class="badge-hidden"
            >
              Hidden
            </span>
            <span
              v-if="isImported(client)"
              class="badge-imported"
            >
              Imported
            </span>
            <span
              class="status-badge"
              :class="isOnline(client) ? 'status-online' : 'status-offline'"
            >
              {{ isOnline(client) ? 'Online' : 'Offline' }}
            </span>
          </div>
        </div>
        <div class="card-stats">
          <div class="stat">
            <span class="stat-value">{{ scheduleCount(client) }}</span>
            <span class="stat-label">Schedules</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatLastSeen(client.last_seen_at) }}</span>
            <span class="stat-label">Last seen</span>
          </div>
          <div class="stat">
            <span class="stat-value mono">{{ formatVersion(client.agent_version) }}</span>
            <span class="stat-label">Agent</span>
          </div>
        </div>
        <div
          v-if="hostHasIssues(client)"
          class="card-health-issues"
        >
          <AlertCircle :size="12" />
          <span
            v-if="hostHealthStatus(client)!.failed > 0"
            class="issue-text issue-failed"
          >
            {{ hostHealthStatus(client)!.failed }} failed
          </span>
          <span
            v-if="hostHealthStatus(client)!.overdue > 0"
            class="issue-text issue-overdue"
          >
            {{ hostHealthStatus(client)!.overdue }} overdue
          </span>
        </div>
        <div
          v-if="hostActiveBackups(client).length > 0"
          class="card-active-backup"
        >
          <span class="active-pulse" />
          <span class="active-text"> Backing up: {{ hostActiveBackups(client).join(', ') }} </span>
        </div>
        <div
          v-if="clientTags(client).length > 0"
          class="card-tags"
        >
          <span
            v-for="tag in clientTags(client)"
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
          <template v-if="client.is_hidden">
            <button
              class="btn btn-sm btn-ghost"
              @click="unhideClient(client)"
            >
              Unhide
            </button>
          </template>
          <template v-else>
            <button
              v-if="isImported(client)"
              class="btn btn-sm btn-ghost"
              @click="openMergeDialog(client)"
            >
              Merge into...
            </button>
            <button
              v-if="isImported(client)"
              class="btn btn-sm btn-ghost"
              @click="adoptClient(client)"
            >
              Adopt
            </button>
            <button
              v-if="deployButtonLabel(client) && !isImported(client)"
              class="btn btn-sm btn-ghost"
              @click="openDeployDialog(client)"
            >
              {{ deployButtonLabel(client) }}
            </button>
          </template>
        </div>
      </div>
    </div>

    <!-- Add Client Dialog -->
    <Teleport to="body">
      <div
        v-if="showAddDialog"
        class="overlay"
        @click.self="closeAddDialog"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ newToken ? 'Client Created' : 'Add Client' }}
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

    <!-- Adopt Host Dialog -->
    <Teleport to="body">
      <div
        v-if="showAdoptDialog"
        class="overlay"
        @click.self="showAdoptDialog = false"
      >
        <div class="dialog dialog-sm">
          <div class="dialog-header">
            <h2 class="dialog-title">Host Adopted &mdash; {{ adoptHostname }}</h2>
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
    <Teleport to="body">
      <div
        v-if="showDeployDialog"
        class="overlay"
        @click.self="showDeployDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ deployTarget?.agent_version ? 'Upgrade' : 'Deploy' }} Agent &mdash;
              {{ deployTarget?.hostname }}
            </h2>
            <button
              class="close-btn"
              @click="showDeployDialog = false"
            >
              &times;
            </button>
          </div>

          <template v-if="!deployResult?.success">
            <div class="dialog-body">
              <p class="deploy-info">
                Upload and install the agent binary on the target machine via SSH. Connect as root
                or enable sudo below for non-root users.
              </p>
              <p class="deploy-note">
                This will also install and enable the <code>assimilate-agent</code> systemd service
                on the target machine. You can customize the service unit below.
              </p>
              <div class="field">
                <label class="field-label">SSH Host <span class="required">*</span></label>
                <input
                  v-model="deployForm.ssh_host"
                  class="input mono"
                  placeholder="e.g. 192.168.1.10"
                />
              </div>
              <div class="deploy-row-fields">
                <div class="field">
                  <label class="field-label">SSH User</label>
                  <input
                    v-model="deployForm.ssh_user"
                    class="input mono"
                    placeholder="root"
                  />
                </div>
                <div class="field field-narrow">
                  <label class="field-label">SSH Port</label>
                  <input
                    v-model.number="deployForm.ssh_port"
                    class="input"
                    type="number"
                    min="1"
                    max="65535"
                  />
                </div>
              </div>
              <div class="field">
                <label class="field-label">SSH Password</label>
                <input
                  v-model="deployForm.ssh_password"
                  class="input mono"
                  type="password"
                  placeholder="Leave empty to use SSH key"
                />
                <span class="field-hint"
                  >Optional — authenticate with password instead of the server's SSH key</span
                >
              </div>
              <div class="field">
                <label class="field-label">Server URL <span class="required">*</span></label>
                <input
                  v-model="deployForm.server_url"
                  class="input mono"
                  placeholder="http://your-server:8080"
                />
                <span class="field-hint">The URL the agent will connect to</span>
              </div>
              <div class="field">
                <label class="field-label">Install Path</label>
                <input
                  v-model="deployForm.install_path"
                  class="input mono"
                  placeholder="/usr/local/bin/assimilate-agent"
                />
              </div>
              <div class="field toggle-row">
                <span class="toggle-row-label">Use sudo for privileged operations</span>
                <ToggleSwitch v-model="deployForm.use_sudo" />
              </div>
              <span
                v-if="deployForm.use_sudo"
                class="field-hint"
                >Enable when connecting as a non-root user that has sudo access</span
              >
              <div
                v-if="deployForm.use_sudo"
                class="field"
              >
                <label class="field-label">Sudo Password</label>
                <input
                  v-model="deployForm.sudo_password"
                  class="input mono"
                  type="password"
                  placeholder="Leave empty if passwordless sudo is configured"
                />
              </div>
              <div class="field">
                <label class="field-label">Systemd Service Unit</label>
                <textarea
                  v-model="deployForm.systemd_service_content"
                  class="input mono service-textarea"
                  rows="12"
                  spellcheck="false"
                />
                <span class="field-hint">
                  The <code>BORG_SERVER_URL</code> and <code>BORG_AGENT_TOKEN</code> environment
                  variables will be injected automatically if not present in custom content.
                </span>
              </div>
              <div
                v-if="deployError"
                class="form-error"
              >
                {{ deployError }}
              </div>
              <div
                v-if="deployResult && !deployResult.success"
                class="form-error"
              >
                {{ deployResult.error }}
              </div>
            </div>
            <div class="dialog-footer">
              <button
                class="btn btn-ghost"
                @click="showDeployDialog = false"
              >
                Cancel
              </button>
              <button
                class="btn btn-primary"
                :disabled="deployLoading || !deployForm.ssh_host || !deployForm.server_url"
                @click="submitDeploy"
              >
                {{
                  deployLoading
                    ? 'Deploying...'
                    : deployTarget?.agent_version
                      ? 'Upgrade Agent'
                      : 'Deploy Agent'
                }}
              </button>
            </div>
          </template>

          <template v-else>
            <div class="dialog-body">
              <div class="token-notice">
                <template v-if="deployResult.skipped">
                  <p class="deploy-skipped-msg">
                    Agent is already at the latest version ({{ deployResult.available_version }}).
                    Deployment skipped.
                  </p>
                </template>
                <template v-else>
                  <p class="deploy-success-msg">Agent deployed and service started successfully.</p>
                  <p
                    v-if="deployResult.available_version"
                    class="deploy-version-info"
                  >
                    Deployed version: {{ deployResult.available_version }}
                  </p>
                  <p class="token-warning">A new agent token was generated for this deployment:</p>
                  <div class="token-box">
                    <code class="token-text">{{ deployResult.token }}</code>
                  </div>
                </template>
              </div>
            </div>
            <div class="dialog-footer">
              <button
                class="btn btn-primary"
                @click="showDeployDialog = false"
              >
                Done
              </button>
            </div>
          </template>
        </div>
      </div>
    </Teleport>

    <!-- Merge Client Dialog -->
    <Teleport to="body">
      <MergeClientDialog
        v-if="showMergeDialog && mergeSource"
        :source="mergeSource"
        :all-clients="clients"
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

.deploy-info {
  font-size: 0.85rem;
  color: var(--text-muted);
  margin-bottom: 0.5rem;
}

.deploy-note {
  font-size: 0.8rem;
  color: var(--text-muted);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem 0.75rem;
  margin-bottom: 1rem;
}

.deploy-note code {
  font-size: 0.75rem;
  background: var(--bg-card);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
}

.deploy-row-fields {
  display: flex;
  gap: 1rem;
}

.deploy-row-fields .field {
  flex: 1;
}

.field-narrow {
  max-width: 120px;
  flex: 0 0 120px;
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

.service-textarea {
  font-size: 0.75rem;
  line-height: 1.5;
  resize: vertical;
  min-height: 180px;
  white-space: pre;
  overflow-x: auto;
}

.deploy-success-msg {
  color: var(--success);
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.deploy-skipped-msg {
  color: var(--text-secondary);
  font-weight: 500;
}

.deploy-version-info {
  font-size: 0.85rem;
  color: var(--text-muted);
  font-family: var(--mono);
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
