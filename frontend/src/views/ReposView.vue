<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useMobile } from '../composables/useMobile'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import { formatBytes, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { Plus, Download, SlidersHorizontal, Database, Folder, FolderPlus } from '@lucide/vue'
import BaseModal from '../components/BaseModal.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SshKeyDeployPanel from '../components/SshKeyDeployPanel.vue'

type CompressionType = 'lz4' | 'zstd' | 'none'
type EncryptionType =
  | 'repokey'
  | 'repokey-blake2'
  | 'keyfile'
  | 'keyfile-blake2'
  | 'authenticated'
  | 'authenticated-blake2'
  | 'none'
type AddTab = 'import' | 'create'
type SortField = 'name' | 'size' | 'last_backup'
type SortDir = 'asc' | 'desc'

interface TagRow {
  id: number
  name: string
  color: string
  scope: string
}

interface RepoTagRow {
  repo_id: number
  tag_name: string
  tag_color: string
}

interface TagGroup {
  label: string
  color: string | null
  repos: RepoWithStats[]
}

interface RepoWithStats {
  id: number
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  ssh_host_key: string | null
  compression: string
  encryption: string
  enabled: boolean
  importing: boolean
  import_error: string | null
  import_progress: number
  import_total: number
  import_status_message: string | null
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
  unmatched_count: number
}

interface RepoForm {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  passphrase: string
  compression: CompressionType
  encryption: EncryptionType
  enabled: boolean
}

interface SshTarget {
  label: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
}

interface TestConnState {
  loading: boolean
  result: { ssh_ok: boolean; borg_installed: boolean; borg_version?: string; error?: string } | null
}

interface DirEntry {
  name: string
  is_dir: boolean
}

interface BrowserState {
  path: string
  entries: DirEntry[]
  loading: boolean
  error: string | null
  showBrowser: boolean
}

const router = useRouter()
const authStore = useAuthStore()
const repos = ref<RepoWithStats[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

const sortField = ref<SortField>('name')
const sortDir = ref<SortDir>('asc')
const filterText = ref('')
const filterTagIds = ref<number[]>([])
const groupByTag = ref(false)
const showTagDropdown = ref(false)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const allRepoTags = ref<TagRow[]>([])
const repoTagsMap = ref<Record<number, { name: string; color: string }[]>>({})

const showRepoDialog = ref(false)
const repoMode = ref<'create' | 'edit'>('create')
const addTab = ref<AddTab>('import')
const repoLoading = ref(false)
const repoError = ref<string | null>(null)
const editingRepo = ref<RepoWithStats | null>(null)
const showDeployKey = ref(false)

const testConn = reactive<TestConnState>({
  loading: false,
  result: null,
})

const browser = reactive<BrowserState>({
  path: '/',
  entries: [],
  loading: false,
  error: null,
  showBrowser: false,
})

const folderModal = reactive({
  open: false,
  name: '',
  error: null as string | null,
})

useEscapeKey(showRepoDialog, () => {
  showRepoDialog.value = false
})

const filteredRepos = computed<RepoWithStats[]>(() => {
  let list = [...repos.value]

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.ssh_host.toLowerCase().includes(q) ||
        r.repo_path.toLowerCase().includes(q),
    )
  }

  if (filterTagIds.value.length > 0) {
    const selectedNames = new Set(
      allRepoTags.value.filter((t) => filterTagIds.value.includes(t.id)).map((t) => t.name),
    )
    list = list.filter((r) =>
      (repoTagsMap.value[r.id] ?? []).some((t) => selectedNames.has(t.name)),
    )
  }

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'name':
        cmp = a.name.localeCompare(b.name)
        break
      case 'size':
        cmp = a.total_deduplicated_size - b.total_deduplicated_size
        break
      case 'last_backup':
        cmp = (a.last_backup_at ?? '').localeCompare(b.last_backup_at ?? '')
        break
    }
    return sortDir.value === 'desc' ? -cmp : cmp
  })

  return list
})

const groupedRepos = computed<TagGroup[]>(() => {
  if (!groupByTag.value) return []
  const groups: Map<string, TagGroup> = new Map()
  const untagged: RepoWithStats[] = []

  for (const repo of filteredRepos.value) {
    const tags = repoTagsMap.value[repo.id]
    if (!tags || tags.length === 0) {
      untagged.push(repo)
    } else {
      for (const tag of tags) {
        const existing = groups.get(tag.name)
        if (existing) {
          existing.repos.push(repo)
        } else {
          groups.set(tag.name, { label: tag.name, color: tag.color, repos: [repo] })
        }
      }
    }
  }

  const result = [...groups.values()].sort((a, b) => a.label.localeCompare(b.label))
  if (untagged.length > 0) {
    result.push({ label: 'Untagged', color: null, repos: untagged })
  }
  return result
})

function toggleSort(field: SortField): void {
  if (sortField.value === field) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortField.value = field
    sortDir.value = 'asc'
  }
}

function toggleTagFilter(tagId: number): void {
  const idx = filterTagIds.value.indexOf(tagId)
  if (idx === -1) {
    filterTagIds.value = [...filterTagIds.value, tagId]
  } else {
    filterTagIds.value = filterTagIds.value.filter((id) => id !== tagId)
  }
}

function repoTags(repo: RepoWithStats): { name: string; color: string }[] {
  return repoTagsMap.value[repo.id] ?? []
}

const defaultRepoForm = (): RepoForm => ({
  name: '',
  repo_path: '',
  ssh_user: 'borg',
  ssh_host: '',
  ssh_port: 22,
  passphrase: '',
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
})

const repoForm = reactive<RepoForm>(defaultRepoForm())

const sshTargets = computed<SshTarget[]>(() => {
  const seen = new Set<string>()
  const targets: SshTarget[] = []
  for (const repo of repos.value) {
    const label = `${repo.ssh_user}@${repo.ssh_host}:${repo.ssh_port}`
    if (!seen.has(label)) {
      seen.add(label)
      targets.push({
        label,
        ssh_user: repo.ssh_user,
        ssh_host: repo.ssh_host,
        ssh_port: repo.ssh_port,
      })
    }
  }
  return targets
})

const breadcrumbs = computed(() => {
  const parts = browser.path.split('/').filter(Boolean)
  const crumbs = [{ label: '/', path: '/' }]
  let acc = ''
  for (const part of parts) {
    acc += `/${part}`
    crumbs.push({ label: part, path: acc })
  }
  return crumbs
})

const sshReady = computed(() => repoForm.ssh_host.trim().length > 0)

const formValid = computed(() => {
  const hasHost = repoForm.ssh_host.trim().length > 0
  const hasPath = repoForm.repo_path.trim().length > 0
  if (repoMode.value === 'edit') return hasHost && hasPath
  const hasName = repoForm.name.trim().length > 0
  const hasPassphrase = repoForm.passphrase.length > 0
  return hasName && hasHost && hasPath && hasPassphrase
})

const autocompleteEntries = ref<DirEntry[]>([])
const showAutocomplete = ref(false)
let autocompleteTimer: ReturnType<typeof setTimeout> | null = null

function onPathInput(): void {
  if (autocompleteTimer) clearTimeout(autocompleteTimer)
  autocompleteTimer = setTimeout(() => {
    fetchAutocomplete()
    syncBrowserToPath()
  }, 300)
}

function syncBrowserToPath(): void {
  if (!browser.showBrowser || !sshReady.value) return
  const pathValue = repoForm.repo_path.trim()
  if (pathValue.endsWith('/') || pathValue === '/') {
    const dir = pathValue === '/' ? '/' : pathValue.replace(/\/+$/, '')
    if (dir !== browser.path) {
      browseDir(dir || '/')
    }
  }
}

async function fetchAutocomplete(): Promise<void> {
  if (!sshReady.value || !repoForm.repo_path.trim()) {
    autocompleteEntries.value = []
    showAutocomplete.value = false
    return
  }
  const pathValue = repoForm.repo_path.trim()
  const parentDir = pathValue.includes('/')
    ? pathValue.substring(0, pathValue.lastIndexOf('/')) || '/'
    : '/'
  try {
    const res = await apiClient.post<{ path: string; entries: DirEntry[]; error?: string }>(
      '/ssh/list-dir',
      {
        ssh_host: repoForm.ssh_host.trim(),
        ssh_user: repoForm.ssh_user.trim(),
        ssh_port: repoForm.ssh_port,
        path: parentDir,
      },
    )
    if (!res.data.error && res.data.entries) {
      const prefix = pathValue.substring(pathValue.lastIndexOf('/') + 1).toLowerCase()
      autocompleteEntries.value = res.data.entries.filter(
        (e) => e.is_dir && e.name.toLowerCase().startsWith(prefix),
      )
      showAutocomplete.value = autocompleteEntries.value.length > 0
    } else {
      autocompleteEntries.value = []
      showAutocomplete.value = false
    }
  } catch {
    autocompleteEntries.value = []
    showAutocomplete.value = false
  }
}

function selectAutocomplete(entry: DirEntry): void {
  const pathValue = repoForm.repo_path.trim()
  const parentDir = pathValue.substring(0, pathValue.lastIndexOf('/')) || ''
  repoForm.repo_path = parentDir === '/' ? `/${entry.name}` : `${parentDir}/${entry.name}`
  showAutocomplete.value = false
  autocompleteEntries.value = []
}

function hideAutocomplete(): void {
  setTimeout(() => {
    showAutocomplete.value = false
  }, 200)
}

function createFolder(): void {
  folderModal.name = ''
  folderModal.error = null
  folderModal.open = true
}

async function confirmCreateFolder(): Promise<void> {
  const name = folderModal.name.trim()
  if (!name) {
    folderModal.error = 'Folder name is required.'
    return
  }
  const newPath = browser.path === '/' ? `/${name}` : `${browser.path}/${name}`
  try {
    await apiClient.post('/ssh/mkdir', {
      ssh_host: repoForm.ssh_host.trim(),
      ssh_user: repoForm.ssh_user.trim(),
      ssh_port: repoForm.ssh_port,
      path: newPath,
    })
    folderModal.open = false
    await browseDir(newPath)
  } catch (e: unknown) {
    folderModal.error = extractError(e)
  }
}

async function loadRepos(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [reposRes, repoTagAssocRes, repoTagsRes] = await Promise.all([
      apiClient.get<RepoWithStats[]>('/repos/stats'),
      apiClient.get<RepoTagRow[]>('/repo-tags').catch(() => ({ data: [] as RepoTagRow[] })),
      apiClient
        .get<TagRow[]>('/tags', { params: { scope: 'repo' } })
        .catch(() => ({ data: [] as TagRow[] })),
    ])
    repos.value = reposRes.data

    allRepoTags.value = repoTagsRes.data
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    repoTagAssocRes.data.forEach((rt) => {
      if (!tagMap[rt.repo_id]) tagMap[rt.repo_id] = []
      tagMap[rt.repo_id].push({ name: rt.tag_name, color: rt.tag_color })
    })
    repoTagsMap.value = tagMap
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

function navigateToRepo(repo: RepoWithStats): void {
  router.push(`/repos/${repo.id}`)
}

function openCreateRepo(): void {
  repoMode.value = 'create'
  addTab.value = 'create'
  editingRepo.value = null
  repoError.value = null
  showDeployKey.value = false
  testConn.result = null
  browser.path = '/'
  browser.entries = []
  browser.error = null
  browser.showBrowser = false
  Object.assign(repoForm, defaultRepoForm())
  showRepoDialog.value = true
}

function openImportRepo(): void {
  repoMode.value = 'create'
  addTab.value = 'import'
  editingRepo.value = null
  repoError.value = null
  showDeployKey.value = false
  testConn.result = null
  browser.path = '/'
  browser.entries = []
  browser.error = null
  browser.showBrowser = false
  Object.assign(repoForm, defaultRepoForm())
  showRepoDialog.value = true
}

function applySshTarget(event: Event): void {
  const value = (event.target as HTMLSelectElement).value
  if (!value) return
  const target = sshTargets.value.find((t) => t.label === value)
  if (target) {
    repoForm.ssh_user = target.ssh_user
    repoForm.ssh_host = target.ssh_host
    repoForm.ssh_port = target.ssh_port
  }
}

async function browseDir(path: string): Promise<void> {
  if (!sshReady.value) return
  browser.loading = true
  browser.error = null
  browser.showBrowser = true
  try {
    const res = await apiClient.post<{ path: string; entries: DirEntry[]; error?: string }>(
      '/ssh/list-dir',
      {
        ssh_host: repoForm.ssh_host.trim(),
        ssh_user: repoForm.ssh_user.trim(),
        ssh_port: repoForm.ssh_port,
        path,
      },
    )
    if (res.data.error) {
      browser.error = res.data.error
    } else {
      browser.path = res.data.path
      browser.entries = res.data.entries.filter((e) => e.is_dir)
      repoForm.repo_path = res.data.path
    }
  } catch (e: unknown) {
    browser.error = extractError(e)
  } finally {
    browser.loading = false
  }
}

function navigateTo(path: string): void {
  browseDir(path)
}

function navigateUp(): void {
  const parent = browser.path.replace(/\/[^/]+\/?$/, '') || '/'
  browseDir(parent)
}

function selectDir(entry: DirEntry): void {
  if (entry.is_dir) {
    const base = browser.path.endsWith('/') ? browser.path.slice(0, -1) : browser.path
    const next = base === '' ? `/${entry.name}` : `${base}/${entry.name}`
    browseDir(next)
  }
}

async function submitRepo(): Promise<void> {
  repoLoading.value = true
  repoError.value = null
  try {
    if (repoMode.value === 'create') {
      if (addTab.value === 'import') {
        const res = await apiClient.post<{
          id: number
          name: string
          repo_path: string
          ssh_user: string
          ssh_host: string
          ssh_port: number
          compression: string
          encryption: string
          enabled: boolean
        }>('/repos', {
          name: repoForm.name.trim(),
          repo_path: repoForm.repo_path.trim(),
          ssh_user: repoForm.ssh_user.trim(),
          ssh_host: repoForm.ssh_host.trim(),
          ssh_port: repoForm.ssh_port,
          passphrase: repoForm.passphrase,
          compression: repoForm.compression,
        })
        showRepoDialog.value = false
        repos.value = [
          ...repos.value,
          {
            id: res.data.id,
            name: res.data.name,
            repo_path: res.data.repo_path,
            ssh_user: res.data.ssh_user,
            ssh_host: res.data.ssh_host,
            ssh_port: res.data.ssh_port,
            ssh_host_key: null,
            compression: res.data.compression,
            encryption: res.data.encryption,
            enabled: res.data.enabled,
            importing: true,
            import_error: null,
            import_progress: 0,
            import_total: 0,
            import_status_message: null,
            archive_count: 0,
            last_backup_at: null,
            total_original_size: 0,
            total_compressed_size: 0,
            total_deduplicated_size: 0,
            client_count: 0,
            unmatched_count: 0,
          },
        ]
        return
      } else {
        await apiClient.post('/repos/init', {
          name: repoForm.name.trim(),
          repo_path: repoForm.repo_path.trim(),
          ssh_user: repoForm.ssh_user.trim(),
          ssh_host: repoForm.ssh_host.trim(),
          ssh_port: repoForm.ssh_port,
          passphrase: repoForm.passphrase,
          encryption: repoForm.encryption,
          compression: repoForm.compression,
        })
      }
    } else if (editingRepo.value) {
      await apiClient.put(`/repos/${editingRepo.value.id}`, {
        repo_path: repoForm.repo_path.trim(),
        ssh_user: repoForm.ssh_user.trim(),
        ssh_host: repoForm.ssh_host.trim(),
        ssh_port: repoForm.ssh_port,
        compression: repoForm.compression,
        encryption: repoForm.encryption,
        enabled: repoForm.enabled,
      })
    }
    showRepoDialog.value = false
    await loadRepos()
  } catch (e: unknown) {
    repoError.value = extractError(e)
  } finally {
    repoLoading.value = false
  }
}

async function testConnection(): Promise<void> {
  testConn.loading = true
  testConn.result = null
  try {
    const res = await apiClient.post<{
      ssh_ok: boolean
      borg_installed: boolean
      borg_version?: string
      error?: string
    }>('/ssh/test-connection', {
      ssh_host: repoForm.ssh_host.trim(),
      ssh_user: repoForm.ssh_user.trim(),
      ssh_port: repoForm.ssh_port,
    })
    testConn.result = res.data
  } catch (e: unknown) {
    testConn.result = { ssh_ok: false, borg_installed: false, error: extractError(e) }
  } finally {
    testConn.loading = false
  }
}

interface ImportProgressPayload {
  repo_id: number
  progress: number
  total: number
  message: string | null
}

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => loadRepos().catch(logger.error))

onMessage<ImportProgressPayload>('ImportProgress', (payload) => {
  const repo = repos.value.find((r) => r.id === payload.repo_id)
  if (repo) {
    if (payload.progress >= 0) {
      repo.import_progress = payload.progress
      repo.import_total = payload.total
    }
    repo.import_status_message = payload.message
  }
})

onMounted(loadRepos)
</script>

<template>
  <div class="repos-view">
    <div class="page-header">
      <h1 class="page-title">Repositories</h1>
      <div
        v-if="authStore.user?.role === 'admin'"
        class="header-actions"
      >
        <button
          class="btn btn-ghost"
          @click="openImportRepo"
        >
          <Download :size="14" />
          Import
        </button>
        <button
          class="btn btn-primary"
          @click="openCreateRepo"
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
        placeholder="Filter repositories..."
      />
      <button
        v-if="isMobile"
        class="btn-filter-toggle"
        :class="{ active: filterTagIds.length > 0 || groupByTag }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterTagIds.length > 0 || groupByTag"
          class="filter-badge"
        ></span>
      </button>
      <template v-if="!isMobile || showMobileFilters">
        <div
          v-if="allRepoTags.length > 0"
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
              v-for="tag in allRepoTags"
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
        <button
          v-if="allRepoTags.length > 0"
          class="btn btn-sm btn-ghost"
          :class="{ active: groupByTag }"
          @click="groupByTag = !groupByTag"
        >
          Group by tag
        </button>
        <div class="sort-controls">
          <span class="sort-label">Sort:</span>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'name' }"
            @click="toggleSort('name')"
          >
            Name {{ sortField === 'name' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'size' }"
            @click="toggleSort('size')"
          >
            Size {{ sortField === 'size' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'last_backup' }"
            @click="toggleSort('last_backup')"
          >
            Last Backup
            {{ sortField === 'last_backup' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
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
      v-else-if="repos.length === 0"
      :icon="Database"
      title="No repositories configured"
      description="Add a repository to start managing backups."
      action="Add Repository"
      @action="showRepoDialog = true"
    />
    <div
      v-else-if="filteredRepos.length === 0"
      class="state-msg"
    >
      No repositories match the current filter.
    </div>

    <div
      v-else-if="!groupByTag"
      class="repo-grid"
    >
      <div
        v-for="repo in filteredRepos"
        :key="repo.id"
        class="repo-card"
        @click="navigateToRepo(repo)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-name">{{ repo.name }}</span>
            <span class="card-ssh"
              >{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</span
            >
          </div>
          <div class="card-badges">
            <span
              class="status-badge"
              :class="
                repo.import_error
                  ? 'status-error'
                  : repo.importing
                    ? 'status-importing'
                    : repo.enabled
                      ? 'status-online'
                      : 'status-offline'
              "
              :title="repo.import_error ?? undefined"
            >
              {{
                repo.import_error
                  ? 'Import Failed'
                  : repo.importing
                    ? repo.import_total > 0
                      ? `Importing ${repo.import_progress}/${repo.import_total}`
                      : 'Importing\u2026'
                    : repo.enabled
                      ? 'Enabled'
                      : 'Disabled'
              }}
            </span>
          </div>
        </div>
        <div
          v-if="repo.importing && repo.import_total > 0"
          class="import-progress"
        >
          <div class="import-progress-track">
            <div
              class="import-progress-bar"
              :style="{ width: `${Math.round((repo.import_progress / repo.import_total) * 100)}%` }"
            ></div>
          </div>
          <span class="import-progress-label">
            {{ Math.round((repo.import_progress / repo.import_total) * 100) }}%
          </span>
        </div>
        <p
          v-if="repo.importing && repo.import_status_message"
          class="import-status-inline"
        >
          {{ repo.import_status_message }}
        </p>
        <div class="card-meta">
          <span class="meta-pill">{{ repo.encryption }}</span>
          <span class="meta-pill">{{ repo.compression }}</span>
          <span
            v-if="repo.unmatched_count > 0"
            class="meta-pill unmatched-pill"
          >
            &#9888; {{ repo.unmatched_count }} unmatched host{{
              repo.unmatched_count === 1 ? '' : 's'
            }}
          </span>
          <span
            v-for="tag in repoTags(repo)"
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
        <div class="card-stats">
          <div class="stat">
            <span class="stat-value">{{ repo.archive_count }}</span>
            <span class="stat-label">Archives</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
            <span class="stat-label">Deduplicated</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ relativeTime(repo.last_backup_at ?? '') }}</span>
            <span class="stat-label">Last backup</span>
          </div>
        </div>
      </div>
    </div>

    <div
      v-else
      class="repo-grouped"
    >
      <div
        v-for="group in groupedRepos"
        :key="group.label"
        class="tag-group"
      >
        <div class="tag-group-header">
          <span
            v-if="group.color"
            class="tag-group-dot"
            :style="{ background: group.color }"
          ></span>
          <h3 class="tag-group-title">{{ group.label }}</h3>
          <span class="tag-group-count">{{ group.repos.length }}</span>
        </div>
        <div class="repo-grid">
          <div
            v-for="repo in group.repos"
            :key="`${group.label}-${repo.id}`"
            class="repo-card"
            @click="navigateToRepo(repo)"
          >
            <div class="card-top">
              <div class="card-info">
                <span class="card-name">{{ repo.name }}</span>
                <span class="card-ssh"
                  >{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</span
                >
              </div>
              <div class="card-badges">
                <span
                  class="status-badge"
                  :class="
                    repo.import_error
                      ? 'status-error'
                      : repo.importing
                        ? 'status-importing'
                        : repo.enabled
                          ? 'status-online'
                          : 'status-offline'
                  "
                  :title="repo.import_error ?? undefined"
                >
                  {{
                    repo.import_error
                      ? 'Import Failed'
                      : repo.importing
                        ? repo.import_total > 0
                          ? `Importing ${repo.import_progress}/${repo.import_total}`
                          : 'Importing\u2026'
                        : repo.enabled
                          ? 'Enabled'
                          : 'Disabled'
                  }}
                </span>
              </div>
            </div>
            <div
              v-if="repo.importing && repo.import_total > 0"
              class="import-progress"
            >
              <div class="import-progress-track">
                <div
                  class="import-progress-bar"
                  :style="{
                    width: `${Math.round((repo.import_progress / repo.import_total) * 100)}%`,
                  }"
                ></div>
              </div>
              <span class="import-progress-label">
                {{ Math.round((repo.import_progress / repo.import_total) * 100) }}%
              </span>
            </div>
            <p
              v-if="repo.importing && repo.import_status_message"
              class="import-status-inline"
            >
              {{ repo.import_status_message }}
            </p>
            <div class="card-meta">
              <span class="meta-pill">{{ repo.encryption }}</span>
              <span class="meta-pill">{{ repo.compression }}</span>
              <span
                v-if="repo.unmatched_count > 0"
                class="meta-pill unmatched-pill"
              >
                &#9888; {{ repo.unmatched_count }} unmatched archive{{
                  repo.unmatched_count === 1 ? '' : 's'
                }}
              </span>
              <span
                v-for="tag in repoTags(repo)"
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
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{ repo.archive_count }}</span>
                <span class="stat-label">Archives</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
                <span class="stat-label">Deduplicated</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ relativeTime(repo.last_backup_at ?? '') }}</span>
                <span class="stat-label">Last backup</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Repo Dialog -->
    <Teleport to="body">
      <div
        v-if="showRepoDialog"
        class="overlay"
        @click.self="showRepoDialog = false"
      >
        <div class="dialog dialog-lg">
          <div class="dialog-header">
            <h2 class="dialog-title">
              <template v-if="repoMode === 'edit'">Edit Repository</template>
              <template v-else-if="addTab === 'create'">Create Repository</template>
              <template v-else>Import Repository</template>
            </h2>
            <button
              class="close-btn"
              @click="showRepoDialog = false"
            >
              &times;
            </button>
          </div>

          <div class="dialog-body">
            <div class="form-grid">
              <!-- Name field -->
              <div
                v-if="repoMode === 'create'"
                class="field field-full"
              >
                <label class="field-label">Name <span class="required">*</span></label>
                <input
                  v-model="repoForm.name"
                  class="input"
                  placeholder="e.g. inhouse-backups"
                />
                <span class="field-hint">A short identifier for this storage target</span>
              </div>
              <div
                v-else
                class="field field-full"
              >
                <label class="field-label">Name</label>
                <input
                  :value="repoForm.name"
                  class="input"
                  disabled
                />
              </div>

              <!-- SSH params -->
              <div
                v-if="repoMode === 'create' && sshTargets.length > 0"
                class="field field-full"
              >
                <label class="field-label">Fill SSH from existing</label>
                <select
                  class="input"
                  @change="applySshTarget"
                >
                  <option value="">-- Select to auto-fill --</option>
                  <option
                    v-for="t in sshTargets"
                    :key="t.label"
                    :value="t.label"
                  >
                    {{ t.label }}
                  </option>
                </select>
              </div>

              <div class="field">
                <label class="field-label">SSH User</label>
                <input
                  v-model="repoForm.ssh_user"
                  class="input mono"
                  placeholder="borg"
                />
              </div>
              <div class="field">
                <label class="field-label">SSH Host <span class="required">*</span></label>
                <input
                  v-model="repoForm.ssh_host"
                  class="input mono"
                  placeholder="backup.example.com"
                />
              </div>
              <div class="field field-narrow">
                <label class="field-label">SSH Port</label>
                <input
                  v-model.number="repoForm.ssh_port"
                  class="input"
                  type="number"
                  min="1"
                  max="65535"
                />
              </div>

              <!-- Test & Deploy SSH Key (create mode) -->
              <div
                v-if="repoMode === 'create'"
                class="field field-full"
              >
                <div class="ssh-actions">
                  <button
                    class="btn btn-sm btn-ghost"
                    :disabled="testConn.loading || !sshReady"
                    @click="testConnection"
                  >
                    {{ testConn.loading ? 'Testing...' : 'Test Connection' }}
                  </button>
                  <button
                    class="btn btn-sm btn-ghost"
                    :disabled="!sshReady"
                    @click="showDeployKey = !showDeployKey"
                  >
                    {{ showDeployKey ? '\u2212 Deploy Key' : '+ Deploy Key' }}
                  </button>
                  <span
                    v-if="testConn.result"
                    class="deploy-result"
                    :class="testConn.result.ssh_ok ? 'result-ok' : 'result-warn'"
                  >
                    <template v-if="testConn.result.ssh_ok && testConn.result.borg_installed"
                      >SSH OK, borg {{ testConn.result.borg_version }}</template
                    >
                    <template v-else-if="testConn.result.ssh_ok">SSH OK, borg not found</template>
                    <template v-else>{{ testConn.result.error ?? 'Connection failed' }}</template>
                  </span>
                </div>

                <SshKeyDeployPanel
                  v-if="showDeployKey"
                  :ssh-host="repoForm.ssh_host"
                  :ssh-user="repoForm.ssh_user"
                  :ssh-port="repoForm.ssh_port"
                />
              </div>
            </div>

            <!-- Folder Browser / Repo Path -->
            <div class="browser-section">
              <div class="browser-header">
                <label class="field-label">Repo Path <span class="required">*</span></label>
                <div class="browser-path-row">
                  <div class="path-autocomplete-wrapper">
                    <input
                      v-model="repoForm.repo_path"
                      class="input mono"
                      placeholder="/backup/repos/myhost"
                      @input="onPathInput"
                      @blur="hideAutocomplete"
                    />
                    <div
                      v-if="showAutocomplete"
                      class="autocomplete-dropdown"
                    >
                      <div
                        v-for="entry in autocompleteEntries"
                        :key="entry.name"
                        class="autocomplete-item"
                        @mousedown.prevent="selectAutocomplete(entry)"
                      >
                        <Folder :size="14" />
                        <span>{{ entry.name }}</span>
                      </div>
                    </div>
                  </div>
                  <button
                    v-if="repoMode === 'create'"
                    class="btn btn-sm btn-ghost"
                    :disabled="!sshReady || browser.loading"
                    @click="browseDir(repoForm.repo_path || '/')"
                  >
                    {{ browser.loading ? 'Loading...' : 'Browse' }}
                  </button>
                </div>
              </div>

              <div
                v-if="browser.showBrowser"
                class="browser-panel"
              >
                <!-- Breadcrumbs -->
                <div class="browser-breadcrumbs">
                  <span
                    v-for="(crumb, i) in breadcrumbs"
                    :key="crumb.path"
                    class="breadcrumb"
                    :class="{ 'breadcrumb-last': i === breadcrumbs.length - 1 }"
                    @click="i < breadcrumbs.length - 1 && navigateTo(crumb.path)"
                  >
                    {{ crumb.label
                    }}<span
                      v-if="i > 0 && i < breadcrumbs.length - 1"
                      class="breadcrumb-sep"
                      >/</span
                    >
                  </span>
                  <button
                    v-if="addTab === 'create'"
                    class="btn btn-xs btn-ghost browser-mkdir-btn"
                    :disabled="!sshReady"
                    @click="createFolder"
                  >
                    <FolderPlus :size="14" />
                    New Folder
                  </button>
                </div>

                <div
                  v-if="browser.error"
                  class="browser-error"
                >
                  {{ browser.error }}
                </div>

                <div
                  v-else
                  class="browser-list"
                >
                  <!-- Parent directory -->
                  <div
                    v-if="browser.path !== '/'"
                    class="browser-entry browser-entry-dir"
                    @click="navigateUp"
                  >
                    <Folder :size="14" />
                    <span class="entry-name">..</span>
                  </div>
                  <!-- Entries (directories only) -->
                  <div
                    v-for="entry in browser.entries"
                    :key="entry.name"
                    class="browser-entry browser-entry-dir"
                    @click="selectDir(entry)"
                  >
                    <Folder :size="14" />
                    <span class="entry-name">{{ entry.name }}</span>
                  </div>
                </div>
              </div>
            </div>

            <!-- Remaining form fields -->
            <div class="form-grid form-grid-below">
              <div
                v-if="repoMode === 'create'"
                class="field field-full"
              >
                <label class="field-label">Passphrase <span class="required">*</span></label>
                <input
                  v-model="repoForm.passphrase"
                  class="input"
                  type="password"
                  placeholder="Repository encryption passphrase"
                />
              </div>

              <div
                v-if="repoMode === 'create' && addTab === 'create'"
                class="field"
              >
                <label class="field-label">Encryption <span class="required">*</span></label>
                <select
                  v-model="repoForm.encryption"
                  class="input"
                >
                  <option value="repokey">repokey</option>
                  <option value="repokey-blake2">repokey-blake2</option>
                  <option value="keyfile">keyfile</option>
                  <option value="keyfile-blake2">keyfile-blake2</option>
                  <option value="authenticated">authenticated</option>
                  <option value="authenticated-blake2">authenticated-blake2</option>
                  <option value="none">none</option>
                </select>
              </div>

              <div class="field">
                <label class="field-label">Compression</label>
                <select
                  v-model="repoForm.compression"
                  class="input"
                >
                  <option value="lz4">lz4</option>
                  <option value="zstd">zstd</option>
                  <option value="none">none</option>
                </select>
              </div>

              <div
                v-if="repoMode === 'edit'"
                class="field field-full toggle-row"
              >
                <span class="toggle-row-label">Repo enabled</span>
                <ToggleSwitch v-model="repoForm.enabled" />
              </div>
            </div>

            <div
              v-if="repoError"
              class="form-error"
            >
              {{ repoError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showRepoDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-primary"
              :disabled="repoLoading || !formValid"
              @click="submitRepo"
            >
              <template v-if="repoLoading"> Saving... </template>
              <template v-else-if="repoMode === 'edit'"> Save </template>
              <template v-else-if="addTab === 'create'"> Create Repo </template>
              <template v-else> Import Repo </template>
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>

  <BaseModal
    :open="folderModal.open"
    title="New Folder"
    size="sm"
    @close="folderModal.open = false"
  >
    <form
      class="folder-modal-form"
      @submit.prevent="confirmCreateFolder"
    >
      <label
        for="folder-name-input"
        class="form-label"
        >Folder name</label
      >
      <input
        id="folder-name-input"
        v-model="folderModal.name"
        class="form-control"
        type="text"
        placeholder="my-backups"
        autofocus
      />
      <p
        v-if="folderModal.error"
        class="folder-modal-error"
      >
        {{ folderModal.error }}
      </p>
    </form>
    <template #footer>
      <button
        class="btn btn-ghost"
        type="button"
        @click="folderModal.open = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        type="button"
        @click="confirmCreateFolder"
      >
        Create
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.repos-view {
  max-width: 1100px;
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

.repo-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 1rem;
}

.repo-card {
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

.repo-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.import-progress {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.import-progress-track {
  flex: 1;
  height: 6px;
  background: var(--border);
  border-radius: 3px;
  overflow: hidden;
}

.import-progress-bar {
  height: 100%;
  background: var(--accent);
  border-radius: 3px;
  transition: width 0.4s ease;
}

.import-progress-label {
  font-size: 0.75rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.import-status-inline {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-badges {
  display: flex;
  gap: 0.35rem;
  align-items: center;
  flex-shrink: 0;
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

.card-name {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-ssh {
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

.meta-pill {
  display: inline-block;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 500;
  background: var(--bg-hover);
  color: var(--text-muted);
  text-transform: lowercase;
}

.unmatched-pill {
  background: color-mix(in srgb, var(--warning) 15%, transparent);
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

.card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.25rem;
  margin-top: auto;
}

/* Overlay & Dialog */

.dialog-lg {
  width: 680px;
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

.input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
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

.ssh-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.deploy-result {
  font-size: 0.8rem;
  font-weight: 500;
}

.result-ok {
  color: var(--success);
}

.result-warn {
  color: var(--text-muted);
}

.browser-section {
  margin-top: 1.25rem;
  border-top: 1px solid var(--border);
  padding-top: 1rem;
}

.browser-header {
  margin-bottom: 0.75rem;
}

.browser-path-row {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.4rem;
}

.browser-path-row .path-autocomplete-wrapper {
  flex: 1;
}

.browser-path-row .path-autocomplete-wrapper .input {
  width: 100%;
}

.browser-panel {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.browser-breadcrumbs {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  padding: 0.5rem 0.75rem;
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
  font-size: 0.8rem;
  font-family: var(--mono);
}

.breadcrumb {
  cursor: pointer;
  color: var(--accent);
  transition: color 0.15s;
}

.breadcrumb:hover {
  text-decoration: underline;
}

.breadcrumb-last {
  color: var(--text-primary);
  cursor: default;
  font-weight: 600;
}

.breadcrumb-last:hover {
  text-decoration: none;
}

.breadcrumb-sep {
  color: var(--text-muted);
  margin: 0 0.15rem;
}

.browser-error {
  padding: 0.75rem;
  color: var(--danger);
  font-size: 0.82rem;
}

.browser-list {
  max-height: 200px;
  overflow-y: auto;
}

.browser-entry {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem;
  font-size: 0.82rem;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border-subtle);
  cursor: default;
}

.browser-entry:last-child {
  border-bottom: none;
}

.browser-entry-dir {
  cursor: pointer;
  color: var(--text-secondary);
}

.browser-entry-dir:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.entry-name {
  font-family: var(--mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.path-autocomplete-wrapper {
  position: relative;
  flex: 1;
}

.autocomplete-dropdown {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  z-index: 60;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  max-height: 160px;
  overflow-y: auto;
  margin-top: 2px;
}

.autocomplete-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem;
  font-size: 0.82rem;
  font-family: var(--mono);
  color: var(--text-secondary);
  cursor: pointer;
}

.autocomplete-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.browser-mkdir-btn {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.75rem;
}

.btn-xs {
  padding: 0.2rem 0.5rem;
  font-size: 0.75rem;
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

/* Tag pills on cards */
.tag-pill {
  display: inline-flex;
  align-items: center;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 500;
  border: 1px solid;
}

/* Grouped view */
.repo-grouped {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.tag-group-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.tag-group-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tag-group-title {
  font-size: 0.9rem;
  font-weight: 600;
  color: var(--text-primary);
}

.tag-group-count {
  font-size: 0.75rem;
  color: var(--text-muted);
  background: var(--bg-hover);
  padding: 0.1rem 0.4rem;
  border-radius: 999px;
}

.form-grid-below {
  margin-top: 1.25rem;
  border-top: 1px solid var(--border);
  padding-top: 1rem;
}

.folder-modal-form {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.folder-modal-error {
  font-size: 0.85rem;
  color: var(--danger);
  margin: 0;
}
</style>
