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
import { useAsyncAction } from '../composables/useAsyncAction'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { Plus, Download, SlidersHorizontal, Database, Folder, FolderPlus } from '@lucide/vue'
import BaseModal from '../components/BaseModal.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SshKeyDeployPanel from '../components/SshKeyDeployPanel.vue'
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
import RepoQuotaMeter from '../components/RepoQuotaMeter.vue'
import RepoQuotaSlice from '../components/RepoQuotaSlice.vue'
import type { Repo, RepoWithStats } from '../types/repo'
import type { TagRow } from '../types/tag'
import type { ServerQuotaResponse } from '../types/generated'
import { listServerQuotas } from '../api/serverQuotas'
import {
  actionForHealth,
  actionLabel,
  computeSliceGeometry,
  quotaCeiling,
  quotaHealth,
} from '../utils/quota'

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
type SortField = 'name' | 'size' | 'last_backup' | 'quota'
type SortDir = 'asc' | 'desc'
type QuotaFilter = 'all' | 'at_risk' | 'no_quota'

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

interface HostGroupEntry {
  repo: RepoWithStats
  offsetBytes: number
  visible: boolean
  colorStep: number
}

interface HostGroup {
  sshHost: string
  entries: HostGroupEntry[]
  totalDeduplicated: number
  serverQuota: ServerQuotaResponse | null
  boxMaxBytes: number | null
  visibleCount: number
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

const ROOT_PATH = '/'

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
const { loading, error, run } = useAsyncAction()

const sortField = ref<SortField>('name')
const sortDir = ref<SortDir>('asc')
const filterText = ref('')
const filterTagIds = ref<number[]>([])
const groupByTag = ref(false)
const groupByHost = ref(false)
const quotaFilter = ref<QuotaFilter>('all')
const showTagDropdown = ref(false)
const serverQuotasByHost = ref<Record<string, ServerQuotaResponse>>({})

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

function repoOwnHealth(repo: RepoWithStats): ReturnType<typeof quotaHealth> {
  return quotaHealth(repo.quota, repo.total_deduplicated_size)
}

/** Utilization of a repo's own quota (usage / ceiling), or null when unconfigured. */
function repoQuotaUtilization(repo: RepoWithStats): number | null {
  if (!repo.quota?.enabled) return null
  const ceiling = quotaCeiling(repo.quota)
  if (ceiling === null || ceiling <= 0) return null
  return repo.total_deduplicated_size / ceiling
}

const atRiskCount = computed(
  () =>
    repos.value.filter((r) => {
      const health = repoOwnHealth(r)
      return health === 'warning' || health === 'critical'
    }).length,
)

const noQuotaCount = computed(
  () => repos.value.filter((r) => repoOwnHealth(r) === 'unconfigured').length,
)

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

  if (quotaFilter.value === 'at_risk') {
    list = list.filter((r) => {
      const health = repoOwnHealth(r)
      return health === 'warning' || health === 'critical'
    })
  } else if (quotaFilter.value === 'no_quota') {
    list = list.filter((r) => repoOwnHealth(r) === 'unconfigured')
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
      case 'quota':
        cmp = (repoQuotaUtilization(a) ?? 0) - (repoQuotaUtilization(b) ?? 0)
        break
    }
    return sortDir.value === 'desc' ? -cmp : cmp
  })

  if (sortField.value === 'quota') {
    const configured = list.filter((r) => repoQuotaUtilization(r) !== null)
    const unconfigured = list.filter((r) => repoQuotaUtilization(r) === null)
    list = [...configured, ...unconfigured]
  }

  return list
})

const hostGroups = computed<HostGroup[]>(() => {
  if (!groupByHost.value) return []

  const byHost = new Map<string, RepoWithStats[]>()
  for (const repo of repos.value) {
    const existing = byHost.get(repo.ssh_host)
    if (existing) existing.push(repo)
    else byHost.set(repo.ssh_host, [repo])
  }

  const visibleIds = new Set(filteredRepos.value.map((r) => r.id))

  return [...byHost.entries()]
    .map(([sshHost, hostRepos]) => {
      const sorted = [...hostRepos].sort(
        (a, b) => b.total_deduplicated_size - a.total_deduplicated_size,
      )
      let offset = 0
      const entries: HostGroupEntry[] = sorted.map((repo, index) => {
        const entry: HostGroupEntry = {
          repo,
          offsetBytes: offset,
          visible: visibleIds.has(repo.id),
          colorStep: index,
        }
        offset += repo.total_deduplicated_size
        return entry
      })
      const serverQuota = serverQuotasByHost.value[sshHost] ?? null
      const boxMaxBytes = serverQuota?.configured
        ? (serverQuota.critical_bytes ?? serverQuota.warn_bytes)
        : null
      return {
        sshHost,
        entries,
        totalDeduplicated: offset,
        serverQuota,
        boxMaxBytes,
        visibleCount: entries.filter((e) => e.visible).length,
      }
    })
    .sort((a, b) => a.sshHost.localeCompare(b.sshHost))
})

function hostSegGeometry(entry: HostGroupEntry, group: HostGroup): { left: number; width: number } {
  const g = computeSliceGeometry({
    offsetBytes: entry.offsetBytes,
    usageBytes: entry.repo.total_deduplicated_size,
    boxMaxBytes: group.boxMaxBytes ?? 0,
    quota: null,
  })
  return { left: g.leftPercent, width: g.fillWidthPercent }
}

function hostWarnMarkPercent(group: HostGroup): number | null {
  const warnBytes = group.serverQuota?.warn_bytes ?? null
  if (
    !group.boxMaxBytes ||
    warnBytes === null ||
    warnBytes <= 0 ||
    warnBytes >= group.boxMaxBytes
  ) {
    return null
  }
  return (warnBytes / group.boxMaxBytes) * 100
}

function hostPoolNote(group: HostGroup): string {
  const count = `${group.entries.length} repo${group.entries.length === 1 ? '' : 's'}`
  if (group.visibleCount === 0) return 'No matching repos'
  if (group.visibleCount < group.entries.length) {
    return `Showing ${group.visibleCount} of ${group.entries.length} repos`
  }
  if (!group.serverQuota?.configured) return `${count} · no host quota configured`

  const health = quotaHealth(group.serverQuota, group.totalDeduplicated)
  if (health === 'critical' || health === 'warning') {
    const action = actionForHealth(
      health,
      group.serverQuota.warn_action,
      group.serverQuota.critical_action,
    )
    const state = health === 'critical' ? 'over critical' : 'over warn threshold'
    return `${count} · ${state}${action ? ` · ${actionLabel(action)}` : ''}`
  }
  const warnBytes = group.serverQuota.warn_bytes
  if (warnBytes !== null && warnBytes > group.totalDeduplicated) {
    return `${count} · ${formatBytes(warnBytes - group.totalDeduplicated)} below warn`
  }
  return `${count} · healthy`
}

function toggleGroupByTag(): void {
  groupByTag.value = !groupByTag.value
  if (groupByTag.value) groupByHost.value = false
}

function toggleGroupByHost(): void {
  groupByHost.value = !groupByHost.value
  if (groupByHost.value) groupByTag.value = false
}

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

function repoImportPhaseVerb(repo: RepoWithStats): string {
  return (repo.import_status_message ?? '').startsWith('Indexing') ? 'Indexing' : 'Importing'
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
  if (pathValue.endsWith('/') || pathValue === ROOT_PATH) {
    const dir = pathValue === ROOT_PATH ? ROOT_PATH : pathValue.replace(/\/+$/, '')
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
  repoForm.repo_path = parentDir === ROOT_PATH ? `/${entry.name}` : `${parentDir}/${entry.name}`
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
  const newPath = browser.path === ROOT_PATH ? `/${name}` : `${browser.path}/${name}`
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
  await run(async () => {
    const [reposRes, repoTagAssocRes, repoTagsRes, serverQuotasRes] = await Promise.all([
      apiClient.get<RepoWithStats[]>('/repos/stats'),
      apiClient.get<RepoTagRow[]>('/repo-tags').catch(() => ({ data: [] as RepoTagRow[] })),
      apiClient
        .get<TagRow[]>('/tags', { params: { scope: 'repo' } })
        .catch(() => ({ data: [] as TagRow[] })),
      authStore.isAdmin
        ? listServerQuotas().catch(() => [] as ServerQuotaResponse[])
        : Promise.resolve([] as ServerQuotaResponse[]),
    ])
    repos.value = reposRes.data

    allRepoTags.value = repoTagsRes.data
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    repoTagAssocRes.data.forEach((rt) => {
      if (!tagMap[rt.repo_id]) tagMap[rt.repo_id] = []
      tagMap[rt.repo_id].push({ name: rt.tag_name, color: rt.tag_color })
    })
    repoTagsMap.value = tagMap

    const quotaMap: Record<string, ServerQuotaResponse> = {}
    serverQuotasRes.forEach((q) => {
      quotaMap[q.ssh_host] = q
    })
    serverQuotasByHost.value = quotaMap
  })
}

function navigateToRepo(repo: RepoWithStats): void {
  router.push(`/repos/${repo.id}`)
}

function navigateToRepoIssue(repo: RepoWithStats): void {
  router.push(`/repos/${repo.id}?tab=archives`)
}

function repoIssues(repo: RepoWithStats): EntityIssue[] {
  if (repo.unmatched_count <= 0) return []
  return [
    {
      key: 'unmatched',
      label: `${repo.unmatched_count} unmatched`,
      severity: 'warning',
      onClick: () => navigateToRepoIssue(repo),
    },
  ]
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
        const res = await apiClient.post<Repo>('/repos', {
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
            agent_count: 0,
            unmatched_count: 0,
            visibility: 'private',
            owner_id: null,
            sync_schedule: null,
            last_synced_at: null,
            relocation_pending: false,
            last_op_kind: null,
            last_op_at: null,
            last_op_by: null,
            current_op: null,
            quota: null,
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

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => loadRepos().catch(logger.error))

onMessage('ImportProgress', (payload) => {
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
        v-if="authStore.isAdmin"
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
        :class="{ active: filterTagIds.length > 0 || groupByTag || groupByHost }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterTagIds.length > 0 || groupByTag || groupByHost"
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
          @click="toggleGroupByTag"
        >
          Group by tag
        </button>
        <button
          class="btn btn-sm btn-ghost"
          :class="{ active: groupByHost }"
          @click="toggleGroupByHost"
        >
          Group by host
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
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'quota' }"
            @click="toggleSort('quota')"
          >
            Quota {{ sortField === 'quota' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
        </div>
      </template>
    </div>

    <div
      v-if="!isMobile || showMobileFilters"
      class="quota-filter-row"
    >
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'all' }"
        @click="quotaFilter = 'all'"
      >
        All &middot; {{ repos.length }}
      </button>
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'at_risk' }"
        @click="quotaFilter = 'at_risk'"
      >
        <span class="quota-fchip-dot quota-fchip-dot-warn"></span>
        At risk &middot; {{ atRiskCount }}
      </button>
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'no_quota' }"
        @click="quotaFilter = 'no_quota'"
      >
        <span class="quota-fchip-dot quota-fchip-dot-none"></span>
        No quota &middot; {{ noQuotaCount }}
      </button>
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
      v-else-if="filteredRepos.length === 0 && !groupByHost"
      class="state-msg"
    >
      No repositories match the current filter.
    </div>

    <div
      v-else-if="groupByHost"
      class="repo-hostgrouped"
    >
      <div
        v-for="group in hostGroups"
        :key="group.sshHost"
        class="host-group"
      >
        <div
          class="pool-header"
          :class="{ 'pool-header-empty': group.visibleCount === 0 }"
        >
          <div class="pool-top">
            <span class="pool-host">{{ group.sshHost }}</span>
            <span class="pool-total">
              {{ formatBytes(group.totalDeduplicated) }}
              <template v-if="group.boxMaxBytes"> / {{ formatBytes(group.boxMaxBytes) }}</template>
            </span>
          </div>
          <div
            v-if="group.boxMaxBytes"
            class="pool-track"
            role="img"
            :aria-label="`${formatBytes(group.totalDeduplicated)} used of ${formatBytes(group.boxMaxBytes)} across ${group.entries.length} repositories`"
          >
            <span
              v-for="entry in group.entries"
              :key="entry.repo.id"
              class="pool-seg"
              :class="[`pool-seg-step-${entry.colorStep % 2}`, { 'pool-seg-dim': !entry.visible }]"
              :style="{
                left: `${hostSegGeometry(entry, group).left}%`,
                width: `${hostSegGeometry(entry, group).width}%`,
              }"
            ></span>
            <span
              v-if="hostWarnMarkPercent(group) !== null"
              class="pool-mark"
              :style="{ left: `${hostWarnMarkPercent(group)}%` }"
            ></span>
          </div>
          <span class="pool-note">{{ hostPoolNote(group) }}</span>
        </div>

        <div
          v-if="group.visibleCount > 0"
          class="repo-grid"
        >
          <div
            v-for="entry in group.entries"
            :key="entry.repo.id"
            class="repo-card"
            :class="{
              'repo-card-notable': !entry.repo.enabled,
              'repo-card-dim': !entry.visible,
            }"
            @click="navigateToRepo(entry.repo)"
          >
            <div class="card-top">
              <div class="card-info">
                <span class="card-name">{{ entry.repo.name }}</span>
                <span class="card-ssh"
                  >{{ entry.repo.ssh_user }}@{{ entry.repo.ssh_host }}:{{
                    entry.repo.ssh_port
                  }}</span
                >
              </div>
              <div class="card-badges">
                <span
                  v-if="entry.repo.import_error || entry.repo.importing"
                  class="status-badge"
                  :class="entry.repo.import_error ? 'status-error' : 'status-importing'"
                  :title="entry.repo.import_error ?? undefined"
                >
                  {{
                    entry.repo.import_error
                      ? 'Import Failed'
                      : entry.repo.import_total > 0
                        ? `${repoImportPhaseVerb(entry.repo)} ${entry.repo.import_progress}/${entry.repo.import_total}`
                        : `${repoImportPhaseVerb(entry.repo)}…`
                  }}
                </span>
              </div>
            </div>
            <EntityStatusBadges
              :notable="!entry.repo.enabled"
              notable-label="Disabled"
              :issues="repoIssues(entry.repo)"
            />
            <div class="card-meta">
              <span class="meta-pill">{{ entry.repo.encryption }}</span>
              <span class="meta-pill">{{ entry.repo.compression }}</span>
            </div>
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{ entry.repo.archive_count }}</span>
                <span class="stat-label">Archives</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ relativeTime(entry.repo.last_backup_at ?? '') }}</span>
                <span class="stat-label">Last backup</span>
              </div>
            </div>
            <RepoQuotaSlice
              v-if="group.boxMaxBytes"
              :quota="entry.repo.quota"
              :usage-bytes="entry.repo.total_deduplicated_size"
              :offset-bytes="entry.offsetBytes"
              :box-max-bytes="group.boxMaxBytes"
              :color-step="entry.colorStep"
            />
            <RepoQuotaMeter
              v-else
              :quota="entry.repo.quota"
              :usage-bytes="entry.repo.total_deduplicated_size"
            />
          </div>
        </div>
      </div>
    </div>

    <div
      v-else-if="!groupByTag"
      class="repo-grid"
    >
      <div
        v-for="repo in filteredRepos"
        :key="repo.id"
        class="repo-card"
        :class="{ 'repo-card-notable': !repo.enabled }"
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
              v-if="repo.import_error || repo.importing"
              class="status-badge"
              :class="repo.import_error ? 'status-error' : 'status-importing'"
              :title="repo.import_error ?? undefined"
            >
              {{
                repo.import_error
                  ? 'Import Failed'
                  : repo.import_total > 0
                    ? `${repoImportPhaseVerb(repo)} ${repo.import_progress}/${repo.import_total}`
                    : `${repoImportPhaseVerb(repo)}\u2026`
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
        <EntityStatusBadges
          :notable="!repo.enabled"
          notable-label="Disabled"
          :issues="repoIssues(repo)"
        />
        <div class="card-meta">
          <span class="meta-pill">{{ repo.encryption }}</span>
          <span class="meta-pill">{{ repo.compression }}</span>
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
        <RepoQuotaMeter
          :quota="repo.quota"
          :usage-bytes="repo.total_deduplicated_size"
        />
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
            :class="{ 'repo-card-notable': !repo.enabled }"
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
                  v-if="repo.import_error || repo.importing"
                  class="status-badge"
                  :class="repo.import_error ? 'status-error' : 'status-importing'"
                  :title="repo.import_error ?? undefined"
                >
                  {{
                    repo.import_error
                      ? 'Import Failed'
                      : repo.import_total > 0
                        ? `${repoImportPhaseVerb(repo)} ${repo.import_progress}/${repo.import_total}`
                        : `${repoImportPhaseVerb(repo)}\u2026`
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
            <EntityStatusBadges
              :notable="!repo.enabled"
              notable-label="Disabled"
              :issues="repoIssues(repo)"
            />
            <div class="card-meta">
              <span class="meta-pill">{{ repo.encryption }}</span>
              <span class="meta-pill">{{ repo.compression }}</span>
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
            <RepoQuotaMeter
              :quota="repo.quota"
              :usage-bytes="repo.total_deduplicated_size"
            />
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

.repo-card-notable {
  background: var(--bg-hover);
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
  background: var(--bg-card);
  color: var(--text-muted);
  text-transform: lowercase;
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

/* Quota filter chips */
.quota-filter-row {
  display: flex;
  gap: 0.4rem;
  flex-wrap: wrap;
  margin-top: 0.5rem;
}

.quota-fchip {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.75rem;
  padding: 0.25rem 0.65rem;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-secondary);
  cursor: pointer;
}

.quota-fchip.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-subtle);
}

.quota-fchip-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.quota-fchip-dot-warn {
  background: var(--warning);
}

.quota-fchip-dot-none {
  background: var(--text-muted);
}

/* Grouped by host */
.repo-hostgrouped {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.host-group {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.repo-card-dim {
  opacity: 0.45;
}

.pool-header {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius, 0.625rem);
  padding: 0.85rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.pool-header-empty {
  opacity: 0.55;
}

.pool-top {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
}

.pool-host {
  font-family: var(--mono);
  font-size: 0.8rem;
  color: var(--text-primary);
  font-weight: 600;
}

.pool-total {
  font-family: var(--mono);
  font-size: 0.78rem;
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.pool-track {
  position: relative;
  height: 8px;
  border-radius: 4px;
  background: var(--border);
}

.pool-seg {
  position: absolute;
  top: 0;
  height: 100%;
  border-radius: 4px;
}

.pool-seg-step-0 {
  background: var(--warning);
}

.pool-seg-step-1 {
  background: color-mix(in oklab, var(--warning) 62%, var(--bg-card));
}

.pool-seg-dim {
  opacity: 0.4;
}

.pool-mark {
  position: absolute;
  top: -2px;
  bottom: -2px;
  width: 2px;
  border-radius: 1px;
  background: var(--text-secondary);
}

.pool-note {
  font-size: 0.72rem;
  color: var(--text-muted);
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
