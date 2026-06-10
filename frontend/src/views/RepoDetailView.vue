<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import {
  useArchiveBrowser,
  type ArchiveEntry,
  type ContentEntry,
} from '../composables/useArchiveBrowser'
import { useWebSocket } from '../composables/useWebSocket'
import { useToast } from '../composables/useToast'
import { formatBytes, formatDate } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { Folder, File, Download, RotateCcw, Trash2 } from '@lucide/vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import QuotaPanel from '../components/QuotaPanel.vue'
import BaseModal from '../components/BaseModal.vue'

type TabId = 'overview' | 'archives'
type ArchiveSortMode =
  | 'date-desc'
  | 'date-asc'
  | 'size-desc'
  | 'size-asc'
  | 'dedup-desc'
  | 'dedup-asc'
type CompressionType = 'lz4' | 'zstd' | 'zlib' | 'none'
type EncryptionType =
  | 'repokey'
  | 'repokey-blake2'
  | 'keyfile'
  | 'keyfile-blake2'
  | 'authenticated'
  | 'authenticated-blake2'
  | 'none'

type RepoOpKind = 'agent_backup' | 'server_sync' | 'break_lock' | 'delete_archive'

interface ActiveRepoOp {
  kind: RepoOpKind
  actor: string
  started_at: string
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
  sync_schedule: string | null
  last_synced_at: string | null
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
  relocation_pending: boolean
  last_op_kind: string | null
  last_op_at: string | null
  last_op_by: string | null
  current_op: ActiveRepoOp | null
}

interface TagRow {
  id: number
  name: string
  color: string
  scope: string
}

interface EditForm {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: CompressionType
  encryption: EncryptionType
  enabled: boolean
  sync_schedule: string | null
}

const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const repoId = computed(() => Number(props.id))
const repoIdRef = computed(() => repoId.value)

const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab as string | undefined
    if (t === 'archives') return t
    return 'overview'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const tabs: { id: TabId; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'archives', label: 'Archives' },
]

const repo = ref<RepoWithStats | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)

// Edit
const isEditing = ref(false)
const editLoading = ref(false)
const editError = ref<string | null>(null)
const editForm = reactive<EditForm>({
  name: '',
  repo_path: '',
  ssh_user: '',
  ssh_host: '',
  ssh_port: 22,
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
  sync_schedule: '0 0,12 * * *',
})

// Passphrase
const showPassphraseDialog = ref(false)
const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showPassphraseDialog, () => {
  showPassphraseDialog.value = false
})

// Tags
const allTags = ref<TagRow[]>([])
const repoTagIds = ref<number[]>([])
const tagsLoading = ref(false)
const newTagName = ref('')
const newTagColor = ref('#6b7280')
const createTagLoading = ref(false)

// Delete (destroy from disk)
const showDeleteDialog = ref(false)
const deleteLoading = ref(false)

// Remove (DB only)
const showRemoveDialog = ref(false)
const removeLoading = ref(false)

useEscapeKey(showDeleteDialog, () => {
  showDeleteDialog.value = false
})

useEscapeKey(showRemoveDialog, () => {
  showRemoveDialog.value = false
})

// Break Lock
const showBreakLockDialog = ref(false)
const breakLockLoading = ref(false)
const breakLockError = ref<string | null>(null)
const breakLockResult = ref<string | null>(null)
const currentOp = ref<ActiveRepoOp | null>(null)

// Borg console
interface BorgExecResult {
  stdout: string
  stderr: string
  exit_code: number
}
const borgConsoleCommand = ref('')
const borgConsoleLoading = ref(false)
const borgConsoleError = ref<string | null>(null)
const borgConsoleResult = ref<BorgExecResult | null>(null)

// Re-scan
const rescanLoading = ref(false)
const syncLoading = ref(false)
const resetImportLoading = ref(false)
const { success: toastSuccess, error: toastError } = useToast()

interface RescanResult {
  matched: number
  remaining_unmatched: number
}

useEscapeKey(showBreakLockDialog, () => {
  showBreakLockDialog.value = false
})

// Confirm Relocation
const showConfirmRelocationDialog = ref(false)
const confirmRelocationLoading = ref(false)
const confirmRelocationError = ref<string | null>(null)
const confirmRelocationResult = ref<string | null>(null)

useEscapeKey(showConfirmRelocationDialog, () => {
  showConfirmRelocationDialog.value = false
})

// SSH Host Key
const showAcceptHostKeyDialog = ref(false)
const hostKeyCheckLoading = ref(false)
const hostKeyMismatch = ref(false)
const acceptHostKeyLoading = ref(false)
const acceptHostKeyError = ref<string | null>(null)
const expectedHostKey = ref<string | null>(null)

useEscapeKey(showAcceptHostKeyDialog, () => {
  showAcceptHostKeyDialog.value = false
})

async function doConfirmRelocation(): Promise<void> {
  confirmRelocationLoading.value = true
  confirmRelocationError.value = null
  confirmRelocationResult.value = null
  try {
    const res = await apiClient.post<{ message: string }>(
      `/repos/${repoId.value}/confirm-relocation`,
    )
    confirmRelocationResult.value = res.data.message
    if (repo.value) {
      repo.value.relocation_pending = true
    }
  } catch (e: unknown) {
    confirmRelocationError.value = extractError(e)
  } finally {
    confirmRelocationLoading.value = false
  }
}

async function checkHostKeyMismatch(): Promise<void> {
  hostKeyCheckLoading.value = true
  expectedHostKey.value = null
  hostKeyMismatch.value = false
  try {
    const res = await apiClient.post<{ ssh_host_key: string }>(
      `/repos/${repoId.value}/ssh-host-key/scan`,
    )
    const sshHostKey = res.data.ssh_host_key
    if (repo.value?.ssh_host_key !== sshHostKey) {
      expectedHostKey.value = sshHostKey
      hostKeyMismatch.value = true
    }
  } catch (e: unknown) {
    logger.debug('host key scan failed', e)
  } finally {
    hostKeyCheckLoading.value = false
  }
}

async function acceptHostKey(): Promise<void> {
  if (!expectedHostKey.value) return
  acceptHostKeyLoading.value = true
  acceptHostKeyError.value = null
  try {
    await apiClient.post(`/repos/${repoId.value}/ssh-host-key`, {
      ssh_host_key: expectedHostKey.value,
    })
    showAcceptHostKeyDialog.value = false
    await loadRepo()
    await checkHostKeyMismatch()
    toastSuccess('SSH host key accepted.')
  } catch (e: unknown) {
    acceptHostKeyError.value = extractError(e)
  } finally {
    acceptHostKeyLoading.value = false
  }
}

interface RepoOpChangedPayload {
  repo_id: number
  op: ActiveRepoOp | null
}

interface ImportProgressPayload {
  repo_id: number
  progress: number
  total: number
  message: string | null
}

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => {
  refreshRepo().catch(logger.error)
  loadArchives().catch(logger.error)
})

onMessage<ImportProgressPayload>('ImportProgress', (payload) => {
  if (repo.value && repo.value.id === payload.repo_id) {
    if (payload.progress >= 0) {
      repo.value.import_progress = payload.progress
      repo.value.import_total = payload.total
    }
    repo.value.import_status_message = payload.message
  }
})

onMessage<RepoOpChangedPayload>('RepoOpChanged', (payload) => {
  if (repo.value && payload.repo_id === repo.value.id) {
    currentOp.value = payload.op
  }
})

// Archive browser
const {
  sortedArchives,
  archivesLoading,
  archivesError,
  selectedArchive,
  contentsLoading,
  contentsError,
  indexing,
  breadcrumbs,
  dirs,
  files,
  loadArchives,
  selectArchive,
  navigateTo: archiveNavigateTo,
  entryName,
  downloadEntry,
  restoreEntry,
  deleteArchiveByName,
} = useArchiveBrowser(repoIdRef)

const archivePendingDeletion = ref<ArchiveEntry | null>(null)
const archiveDeleteLoading = ref(false)

function requestArchiveDeletion(archive: ArchiveEntry): void {
  archivePendingDeletion.value = archive
}

function closeArchiveDeleteDialog(): void {
  if (!archiveDeleteLoading.value) {
    archivePendingDeletion.value = null
  }
}

async function restoreArchiveEntry(entry: ContentEntry): Promise<void> {
  try {
    const restored = await restoreEntry(entry)
    if (!restored) return
    toastSuccess(entry.path.length > 0 ? `Restored ${entry.path}.` : 'Restored the whole archive.')
  } catch (e: unknown) {
    toastError(extractError(e))
  }
}

async function confirmArchiveDeletion(): Promise<void> {
  const archive = archivePendingDeletion.value
  if (!archive) return
  archiveDeleteLoading.value = true
  try {
    await deleteArchiveByName(archive)
    archivePendingDeletion.value = null
    await refreshRepo()
    toastSuccess('Archive deletion started. It will disappear once borg finishes.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    archiveDeleteLoading.value = false
  }
}

const unmatchedCount = computed(() => sortedArchives.value.filter((a) => a.matched !== true).length)

const unmatchedHostnames = computed(() => [
  ...new Set(sortedArchives.value.filter((a) => a.matched !== true).map((a) => a.hostname)),
])

const archiveFilter = ref('')
const collapsedGroups = ref<Set<string>>(new Set())
const groupArchivesByHost = ref(true)
const archiveSortMode = ref<ArchiveSortMode>('date-desc')

const archiveSortModeOptions: { value: ArchiveSortMode; label: string }[] = [
  { value: 'date-desc', label: 'Date newest first' },
  { value: 'date-asc', label: 'Date oldest first' },
  { value: 'size-desc', label: 'Size largest first' },
  { value: 'size-asc', label: 'Size smallest first' },
  { value: 'dedup-desc', label: 'Dedup largest first' },
  { value: 'dedup-asc', label: 'Dedup smallest first' },
]

interface ArchiveGroup {
  hostname: string
  matched: boolean
  clientHostname: string | null
  archives: ArchiveEntry[]
}

const filteredArchives = computed<ArchiveEntry[]>(() => {
  const filter = archiveFilter.value.toLowerCase()
  return filter
    ? sortedArchives.value.filter(
        (a) => a.name.toLowerCase().includes(filter) || a.hostname.toLowerCase().includes(filter),
      )
    : sortedArchives.value
})

const orderedArchives = computed<ArchiveEntry[]>(() => {
  const compareByDate = (left: ArchiveEntry, right: ArchiveEntry): number =>
    left.start.localeCompare(right.start)
  const compareBySize = (left: ArchiveEntry, right: ArchiveEntry): number =>
    left.original_size - right.original_size
  const compareByDedup = (left: ArchiveEntry, right: ArchiveEntry): number =>
    left.deduplicated_size - right.deduplicated_size

  switch (archiveSortMode.value) {
    case 'date-desc':
      return [...filteredArchives.value].sort((a, b) => compareByDate(b, a))
    case 'date-asc':
      return [...filteredArchives.value].sort(compareByDate)
    case 'size-desc':
      return [...filteredArchives.value].sort((a, b) => compareBySize(b, a))
    case 'size-asc':
      return [...filteredArchives.value].sort(compareBySize)
    case 'dedup-desc':
      return [...filteredArchives.value].sort((a, b) => compareByDedup(b, a))
    case 'dedup-asc':
      return [...filteredArchives.value].sort(compareByDedup)
    default:
      return filteredArchives.value
  }
})

const groupedArchives = computed<ArchiveGroup[]>(() => {
  const groups = new Map<string, ArchiveGroup>()
  for (const archive of orderedArchives.value) {
    const isMatched = archive.matched === true
    const key = isMatched ? (archive.client_hostname ?? archive.hostname) : archive.hostname
    if (!groups.has(key)) {
      groups.set(key, {
        hostname: key,
        matched: isMatched,
        clientHostname: isMatched ? archive.client_hostname : null,
        archives: [],
      })
    }
    groups.get(key)!.archives.push(archive)
  }
  return [...groups.values()].sort((a, b) => a.hostname.localeCompare(b.hostname))
})

function toggleGroup(hostname: string): void {
  if (collapsedGroups.value.has(hostname)) {
    collapsedGroups.value.delete(hostname)
  } else {
    collapsedGroups.value.add(hostname)
  }
}

function isGroupCollapsed(hostname: string): boolean {
  return collapsedGroups.value.has(hostname)
}

const isAdmin = computed(() => authStore.user?.role === 'admin')

const repoTags = computed<TagRow[]>(() =>
  allTags.value.filter((t) => repoTagIds.value.includes(t.id)),
)

const availableTags = computed<TagRow[]>(() =>
  allTags.value.filter((t) => !repoTagIds.value.includes(t.id)),
)

function formatLastBackup(iso: string | null): string {
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

function repoOpLabel(op: ActiveRepoOp): string {
  switch (op.kind) {
    case 'agent_backup':
      return `Agent backup in progress by ${op.actor}`
    case 'server_sync':
      return 'Server sync in progress'
    case 'break_lock':
      return 'Break-lock in progress'
    case 'delete_archive':
      return `Deleting archive (started by ${op.actor})`
  }
}

function lastOpLabel(kind: string | null): string {
  switch (kind) {
    case 'agent_backup':
      return 'Agent backup'
    case 'server_sync':
      return 'Server sync'
    case 'break_lock':
      return 'Break lock'
    case 'delete_archive':
      return 'Delete archive'
    default:
      return kind ?? 'Unknown'
  }
}

async function loadRepo(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const res = await apiClient.get<RepoWithStats>(`/repos/${repoId.value}`)
    repo.value = res.data
    currentOp.value = res.data.current_op ?? null
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

async function refreshRepo(): Promise<void> {
  try {
    const res = await apiClient.get<RepoWithStats>(`/repos/${repoId.value}`)
    repo.value = res.data
    currentOp.value = res.data.current_op ?? null
  } catch (e: unknown) {
    logger.error('background repo refresh failed', e)
  }
}

async function loadTags(): Promise<void> {
  tagsLoading.value = true
  try {
    const [tagsRes, repoTagsRes] = await Promise.all([
      apiClient.get<TagRow[]>('/tags', { params: { scope: 'repo' } }),
      apiClient.get<TagRow[]>(`/repos/${repoId.value}/tags`).catch((e: unknown) => {
        logger.error('load repo tags failed', e)
        return { data: [] as TagRow[] }
      }),
    ])
    allTags.value = tagsRes.data
    repoTagIds.value = repoTagsRes.data.map((t) => t.id)
  } catch (e: unknown) {
    logger.error('loadTags failed', e)
  } finally {
    tagsLoading.value = false
  }
}

const VALID_COMPRESSION_BASES: CompressionType[] = ['lz4', 'zstd', 'zlib', 'none']

function normalizeCompression(raw: string): CompressionType {
  const base = raw.split(',')[0] as CompressionType
  return VALID_COMPRESSION_BASES.includes(base) ? base : 'lz4'
}

function startEdit(): void {
  if (!repo.value) return
  editForm.name = repo.value.name
  editForm.repo_path = repo.value.repo_path
  editForm.ssh_user = repo.value.ssh_user
  editForm.ssh_host = repo.value.ssh_host
  editForm.ssh_port = repo.value.ssh_port
  editForm.compression = normalizeCompression(repo.value.compression)
  editForm.encryption = repo.value.encryption as EncryptionType
  editForm.enabled = repo.value.enabled
  editForm.sync_schedule = repo.value.sync_schedule
  editError.value = null
  isEditing.value = true
}

function cancelEdit(): void {
  isEditing.value = false
  editError.value = null
}

async function saveEdit(): Promise<void> {
  editLoading.value = true
  editError.value = null
  try {
    const connRes = await apiClient.post<{
      ssh_ok: boolean
      borg_installed: boolean
      error?: string
    }>('/ssh/test-connection', {
      ssh_host: editForm.ssh_host.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_port: editForm.ssh_port,
    })
    if (!connRes.data.ssh_ok) {
      editError.value = connRes.data.error ?? 'Cannot reach repository host — changes not saved'
      return
    }
    await apiClient.put(`/repos/${repoId.value}`, {
      name: editForm.name.trim(),
      repo_path: editForm.repo_path.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_host: editForm.ssh_host.trim(),
      ssh_port: editForm.ssh_port,
      compression: editForm.compression,
      encryption: editForm.encryption,
      enabled: editForm.enabled,
      sync_schedule: editForm.sync_schedule,
    })
    isEditing.value = false
    await loadRepo()
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

async function revealPassphrase(): Promise<void> {
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(`/repos/${repoId.value}/passphrase`)
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

async function addTag(tagId: number): Promise<void> {
  const updated = [...repoTagIds.value, tagId]
  try {
    await apiClient.put(`/repos/${repoId.value}/tags`, { tag_ids: updated })
    repoTagIds.value = updated
  } catch (e: unknown) {
    logger.error('addTag failed', e)
  }
}

async function removeTag(tagId: number): Promise<void> {
  const updated = repoTagIds.value.filter((id) => id !== tagId)
  try {
    await apiClient.put(`/repos/${repoId.value}/tags`, { tag_ids: updated })
    repoTagIds.value = updated
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
      scope: 'repo',
    })
    allTags.value.push(res.data)
    await addTag(res.data.id)
    newTagName.value = ''
    newTagColor.value = '#6b7280'
  } catch (e: unknown) {
    logger.error('createAndAddTag failed', e)
  } finally {
    createTagLoading.value = false
  }
}

async function confirmDelete(): Promise<void> {
  deleteLoading.value = true
  try {
    await apiClient.post(`/repos/${repoId.value}/destroy`)
    showDeleteDialog.value = false
    router.push('/repos')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    deleteLoading.value = false
  }
}

async function confirmRemove(): Promise<void> {
  removeLoading.value = true
  try {
    await apiClient.delete(`/repos/${repoId.value}`)
    showRemoveDialog.value = false
    router.push('/repos')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    removeLoading.value = false
  }
}

async function confirmBreakLock(): Promise<void> {
  breakLockLoading.value = true
  breakLockError.value = null
  breakLockResult.value = null
  try {
    const res = await apiClient.post<{ message: string; borg_output: string }>(
      `/repos/${repoId.value}/break-lock`,
    )
    breakLockResult.value = res.data.message
  } catch (e: unknown) {
    breakLockError.value = extractError(e)
  } finally {
    breakLockLoading.value = false
  }
}

async function runBorgCommand(): Promise<void> {
  const trimmed = borgConsoleCommand.value.trim()
  if (!trimmed) return
  borgConsoleLoading.value = true
  borgConsoleError.value = null
  borgConsoleResult.value = null
  try {
    const args = trimmed.split(/\s+/).filter((s) => s.length > 0)
    const res = await apiClient.post<BorgExecResult>(`/repos/${repoId.value}/exec`, { args })
    borgConsoleResult.value = res.data
  } catch (e: unknown) {
    borgConsoleError.value = extractError(e)
  } finally {
    borgConsoleLoading.value = false
  }
}

watch(
  () => props.id,
  async () => {
    repo.value = null
    error.value = null
    allTags.value = []
    repoTagIds.value = []
    archiveFilter.value = ''
    collapsedGroups.value = new Set()
    selectedArchive.value = null
    await loadRepo()
    if (repo.value) {
      await Promise.all([loadTags(), loadArchives(), checkHostKeyMismatch()])
      await selectArchiveFromQuery()
    }
  },
)

onMounted(async () => {
  await loadRepo()
  if (repo.value) {
    await Promise.all([loadTags(), loadArchives(), checkHostKeyMismatch()])
    await selectArchiveFromQuery()
  }
})

async function selectArchiveFromQuery(): Promise<void> {
  const archiveQuery = route.query.archive as string | undefined
  if (archiveQuery && activeTab.value === 'archives') {
    const match = sortedArchives.value.find((a) => a.name === archiveQuery)
    if (match) {
      await selectArchive(match)
    }
  }
}

watch(
  () => route.query.archive,
  async () => {
    if (sortedArchives.value.length > 0) {
      await selectArchiveFromQuery()
    }
  },
)

async function rescanArchives(): Promise<void> {
  rescanLoading.value = true
  try {
    const res = await apiClient.post<RescanResult>(`/repos/${repoId.value}/rescan`)
    toastSuccess(
      `Matched ${res.data.matched} archives. ${res.data.remaining_unmatched} remaining unmatched.`,
    )
    await loadArchives()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    rescanLoading.value = false
  }
}

async function syncRepo(): Promise<void> {
  syncLoading.value = true
  try {
    await apiClient.post(`/repos/${repoId.value}/sync`)
    toastSuccess('Full resync started.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    syncLoading.value = false
  }
}

async function resetImport(): Promise<void> {
  resetImportLoading.value = true
  try {
    await apiClient.post(`/repos/${repoId.value}/reset-import`)
    toastSuccess('Import state reset.')
    await loadRepo()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    resetImportLoading.value = false
  }
}
</script>

<template>
  <div class="repo-detail">
    <nav class="breadcrumb-nav">
      <RouterLink
        to="/repos"
        class="crumb-link"
      >
        Repositories
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ repo?.name ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !repo"
      class="state-msg state-error"
    >
      {{ error }}
    </div>

    <template v-else-if="repo">
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
        <!-- Info card -->
        <div class="info-card">
          <div class="info-card-header">
            <h3 class="info-title">Repository Information</h3>
            <div class="info-header-actions">
              <template v-if="isAdmin && !isEditing">
                <button
                  v-if="!repo.importing"
                  class="btn btn-sm btn-ghost"
                  :disabled="syncLoading"
                  @click="syncRepo"
                >
                  {{ syncLoading ? 'Syncing...' : 'Full Resync' }}
                </button>
                <button
                  v-if="repo.importing || repo.import_error"
                  class="btn btn-sm btn-ghost btn-danger-text"
                  :disabled="resetImportLoading"
                  @click="resetImport"
                >
                  {{ resetImportLoading ? 'Resetting...' : 'Cancel Import' }}
                </button>
                <button
                  class="btn btn-sm btn-ghost"
                  :disabled="passphraseLoading"
                  @click="revealPassphrase"
                >
                  {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
                </button>
                <button
                  v-if="hostKeyMismatch"
                  class="btn btn-sm btn-ghost btn-warning-text"
                  :disabled="hostKeyCheckLoading"
                  @click="showAcceptHostKeyDialog = true"
                >
                  {{ hostKeyCheckLoading ? 'Checking...' : 'Accept SSH Key' }}
                </button>
                <button
                  class="btn btn-sm btn-ghost"
                  @click="startEdit"
                >
                  Edit
                </button>
              </template>
            </div>
          </div>

          <template v-if="!isEditing">
            <dl class="info-grid">
              <dt>Name</dt>
              <dd class="mono">{{ repo.name }}</dd>
              <dt>SSH Target</dt>
              <dd class="mono">{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</dd>
              <dt>SSH Host Key</dt>
              <dd class="mono ssh-host-key">
                {{ repo.ssh_host_key ?? 'Not set' }}
              </dd>
              <dt>Repo Path</dt>
              <dd class="mono">{{ repo.repo_path }}</dd>
              <dt>Compression</dt>
              <dd>{{ repo.compression }}</dd>
              <dt>Encryption</dt>
              <dd>{{ repo.encryption }}</dd>
              <dt>Status</dt>
              <dd>
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
                  class="import-status-msg"
                >
                  {{ repo.import_status_message }}
                </p>
              </dd>
              <dt>Archives</dt>
              <dd>{{ repo.archive_count }}</dd>
              <dt>Original Size</dt>
              <dd>{{ formatBytes(repo.total_original_size) }}</dd>
              <dt>Compressed</dt>
              <dd>{{ formatBytes(repo.total_compressed_size) }}</dd>
              <dt>Deduplicated</dt>
              <dd>{{ formatBytes(repo.total_deduplicated_size) }}</dd>
              <dt>Last Backup</dt>
              <dd>{{ formatLastBackup(repo.last_backup_at) }}</dd>
              <dt>Disk Sync</dt>
              <dd>
                <template v-if="repo.sync_schedule">
                  {{ cronToHuman(repo.sync_schedule) ?? repo.sync_schedule }}
                </template>
                <template v-else>Disabled</template>
              </dd>
              <dt>Last Synced</dt>
              <dd>{{ repo.last_synced_at ? formatDate(repo.last_synced_at) : 'Never' }}</dd>
              <dt>Last Operation</dt>
              <dd>
                <template v-if="repo.last_op_kind">
                  {{ lastOpLabel(repo.last_op_kind) }}
                  <template v-if="repo.last_op_by && repo.last_op_by !== 'server'">
                    by {{ repo.last_op_by }}
                  </template>
                  <template v-if="repo.last_op_at">
                    — {{ formatLastBackup(repo.last_op_at) }}
                  </template>
                </template>
                <template v-else>Never</template>
              </dd>
              <template v-if="currentOp">
                <dt>Current Operation</dt>
                <dd class="current-op-running">{{ repoOpLabel(currentOp) }}</dd>
              </template>
              <dt>Clients</dt>
              <dd>{{ repo.client_count }}</dd>
            </dl>
          </template>

          <template v-else>
            <div class="edit-form">
              <div class="form-grid">
                <div class="field field-full">
                  <label class="field-label">Name</label>
                  <input
                    v-model="editForm.name"
                    class="input"
                    placeholder="e.g. Web Server Backup"
                  />
                </div>
                <div class="field">
                  <label class="field-label">SSH User</label>
                  <input
                    v-model="editForm.ssh_user"
                    class="input mono"
                  />
                </div>
                <div class="field">
                  <label class="field-label">SSH Host</label>
                  <input
                    v-model="editForm.ssh_host"
                    class="input mono"
                  />
                </div>
                <div class="field field-narrow">
                  <label class="field-label">SSH Port</label>
                  <input
                    v-model.number="editForm.ssh_port"
                    class="input"
                    type="number"
                    min="1"
                    max="65535"
                  />
                </div>
                <div class="field field-full">
                  <label class="field-label">Repo Path</label>
                  <input
                    v-model="editForm.repo_path"
                    class="input mono"
                  />
                </div>
                <div class="field">
                  <label class="field-label">Compression</label>
                  <select
                    v-model="editForm.compression"
                    class="input"
                  >
                    <option value="lz4">lz4</option>
                    <option value="zstd">zstd</option>
                    <option value="zlib">zlib</option>
                    <option value="none">none</option>
                  </select>
                </div>
                <div class="field">
                  <label class="field-label">Encryption</label>
                  <select
                    v-model="editForm.encryption"
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
                <div class="field field-full toggle-row">
                  <span class="toggle-row-label">Enabled</span>
                  <ToggleSwitch v-model="editForm.enabled" />
                </div>
                <div class="field field-full toggle-row">
                  <span class="toggle-row-label">Disk Sync</span>
                  <ToggleSwitch
                    :model-value="editForm.sync_schedule !== null"
                    @update:model-value="editForm.sync_schedule = $event ? '0 0,12 * * *' : null"
                  />
                </div>
                <div
                  v-if="editForm.sync_schedule !== null"
                  class="field field-full"
                >
                  <label class="field-label">Sync Schedule (cron)</label>
                  <input
                    v-model="editForm.sync_schedule"
                    class="input mono"
                    placeholder="0 0,12 * * *"
                  />
                  <span class="field-hint">Cron expression for automatic disk sync</span>
                </div>
              </div>
              <div
                v-if="editError"
                class="form-error"
              >
                {{ editError }}
              </div>
              <div class="edit-actions">
                <button
                  class="btn btn-ghost"
                  @click="cancelEdit"
                >
                  Cancel
                </button>
                <button
                  class="btn btn-primary"
                  :disabled="editLoading"
                  @click="saveEdit"
                >
                  {{ editLoading ? 'Saving...' : 'Save Changes' }}
                </button>
              </div>
            </div>
          </template>
        </div>

        <!-- Tags -->
        <QuotaPanel
          :repo-id="repoId"
          :is-admin="isAdmin"
          :current-usage-bytes="repo.total_deduplicated_size"
        />

        <!-- Tags -->
        <div
          v-if="isAdmin"
          class="info-card"
        >
          <h3 class="info-title">Tags</h3>
          <div class="tags-section">
            <div
              v-if="repoTags.length > 0"
              class="tag-list"
            >
              <span
                v-for="tag in repoTags"
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

        <!-- Borg Console -->
        <div
          v-if="isAdmin"
          class="info-card"
        >
          <h3 class="info-title">Borg Console</h3>
          <p class="console-desc">
            Execute borg commands directly against this repository. The repository URL and
            passphrase are injected automatically. Use <code class="console-code">::archive</code>
            notation to reference a specific archive.
          </p>
          <div class="console-input-row">
            <span class="console-prefix">borg</span>
            <input
              v-model="borgConsoleCommand"
              class="input console-input"
              placeholder="info"
              :disabled="borgConsoleLoading"
              @keydown.enter="runBorgCommand"
            />
            <button
              class="btn btn-sm btn-primary"
              :disabled="borgConsoleLoading || !borgConsoleCommand.trim()"
              @click="runBorgCommand"
            >
              {{ borgConsoleLoading ? 'Running…' : 'Run' }}
            </button>
          </div>
          <div class="console-hints">
            <span class="console-hint-label">Commands:</span>
            <code
              v-for="cmd in [
                'info',
                'list',
                'check',
                'compact',
                'prune',
                'delete',
                'diff',
                'rename',
                'recreate',
              ]"
              :key="cmd"
              class="console-hint-cmd"
              @click="borgConsoleCommand = cmd"
              >{{ cmd }}</code
            >
          </div>
          <div
            v-if="borgConsoleError"
            class="console-error"
          >
            {{ borgConsoleError }}
          </div>
          <div
            v-if="borgConsoleResult"
            class="console-output"
          >
            <div class="console-output-header">
              <span class="console-output-label">Output</span>
              <span
                :class="{
                  'exit-ok': borgConsoleResult.exit_code === 0,
                  'exit-warn': borgConsoleResult.exit_code === 1,
                  'exit-err': borgConsoleResult.exit_code > 1 || borgConsoleResult.exit_code < 0,
                }"
                >exit {{ borgConsoleResult.exit_code }}</span
              >
            </div>
            <pre
              v-if="borgConsoleResult.stdout"
              class="console-pre"
              >{{ borgConsoleResult.stdout }}</pre
            >
            <pre
              v-if="borgConsoleResult.stderr"
              class="console-pre console-pre-stderr"
              >{{ borgConsoleResult.stderr }}</pre
            >
            <span
              v-if="!borgConsoleResult.stdout && !borgConsoleResult.stderr"
              class="console-empty"
              >(no output)</span
            >
          </div>
        </div>

        <!-- Danger zone -->
        <div
          v-if="isAdmin"
          class="info-card danger-zone"
        >
          <h3 class="info-title">Danger Zone</h3>
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Confirm Repository Relocation</span>
              <span class="danger-desc">
                Allow the next backup to accept this repository at its current location. Use this
                when borg reports the repository was previously at a different path. The flag is
                cleared automatically after the backup succeeds.
              </span>
            </div>
            <div class="danger-action-wrap">
              <button
                class="btn btn-sm btn-danger"
                :disabled="confirmRelocationLoading"
                @click="showConfirmRelocationDialog = true"
              >
                {{ confirmRelocationLoading ? 'Confirming...' : 'Confirm Relocation' }}
              </button>
              <span
                v-if="repo?.relocation_pending"
                class="danger-hint"
              >
                Relocation already pending — will apply on the next backup run.
              </span>
            </div>
          </div>
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Break Repository Lock</span>
              <span class="danger-desc">
                Remove a stale lock from the repository. Using this while a backup is in progress
                will corrupt the repository.
              </span>
            </div>
            <div class="danger-action-wrap">
              <button
                class="btn btn-sm btn-danger"
                :disabled="!!currentOp || breakLockLoading"
                :title="currentOp ? repoOpLabel(currentOp) : undefined"
                @click="showBreakLockDialog = true"
              >
                {{ breakLockLoading ? 'Breaking...' : 'Break Lock' }}
              </button>
              <span
                v-if="currentOp"
                class="danger-hint"
              >
                {{ repoOpLabel(currentOp) }}
              </span>
            </div>
          </div>
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Remove Repository</span>
              <span class="danger-desc"
                >Remove this repository from the UI and database. All associated schedules will be
                <strong>disabled</strong> and their repository link removed — they must be fixed
                manually. Reports will be deleted. The repository data on disk is NOT touched.</span
              >
            </div>
            <button
              class="btn btn-sm btn-danger"
              @click="showRemoveDialog = true"
            >
              Remove Repository
            </button>
          </div>
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Delete Repository</span>
              <span class="danger-desc"
                >PERMANENTLY DESTROY this repository from disk (rm -rf via SSH). This is
                irreversible and all backup data will be lost forever.</span
              >
            </div>
            <button
              class="btn btn-sm btn-danger"
              @click="showDeleteDialog = true"
            >
              Delete Repository
            </button>
          </div>
        </div>
      </div>

      <!-- Archives Tab -->
      <div
        v-if="activeTab === 'archives'"
        class="tab-content"
      >
        <!-- Unmatched banner -->
        <div
          v-if="!archivesLoading && unmatchedCount > 0"
          class="unmatched-banner"
        >
          <div class="unmatched-banner-text">
            <span>
              {{ unmatchedCount }} archive{{ unmatchedCount === 1 ? '' : 's' }} from
              {{ unmatchedHostnames.length }} unresolved hostname{{
                unmatchedHostnames.length === 1 ? '' : 's'
              }}:
              <code
                v-for="h in unmatchedHostnames"
                :key="h"
                class="unmatched-hostname"
                >{{ h }}</code
              >
            </span>
            <span class="unmatched-hint">
              Hostnames are read from borg archive metadata, not derived from the archive name.
              Configure hostname patterns on your hosts to match, then re-scan.
            </span>
          </div>
          <button
            class="btn btn-sm btn-primary"
            :disabled="rescanLoading"
            @click="rescanArchives"
          >
            {{ rescanLoading ? 'Scanning...' : 'Re-scan' }}
          </button>
        </div>

        <div class="archives-layout">
          <!-- Archive list -->
          <div class="panel archives-panel">
            <div class="panel-header">
              <span class="panel-title">Archives</span>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archivesLoading"
                @click="loadArchives"
              >
                {{ archivesLoading ? '...' : '&#8635;' }}
              </button>
            </div>

            <div
              v-if="archivesLoading"
              class="state-msg state-msg-sm"
            >
              <span class="spinner" />
              Loading archives...
            </div>
            <div
              v-else-if="archivesError"
              class="state-msg state-msg-sm state-error"
            >
              {{ archivesError }}
            </div>
            <div
              v-else-if="sortedArchives.length === 0"
              class="state-msg state-msg-sm"
            >
              No archives found.
            </div>
            <template v-else>
              <div class="archive-controls">
                <input
                  v-model="archiveFilter"
                  class="filter-input"
                  type="text"
                  placeholder="Filter archives..."
                />
                <select
                  v-model="archiveSortMode"
                  class="select-input archive-sort-select"
                >
                  <option
                    v-for="option in archiveSortModeOptions"
                    :key="option.value"
                    :value="option.value"
                  >
                    {{ option.label }}
                  </option>
                </select>
                <button
                  class="btn btn-sm btn-ghost archive-group-toggle"
                  :class="{ active: groupArchivesByHost }"
                  @click="groupArchivesByHost = !groupArchivesByHost"
                >
                  {{ groupArchivesByHost ? 'Grouped by host' : 'Flat list' }}
                </button>
              </div>
              <div
                v-if="orderedArchives.length === 0"
                class="state-msg state-msg-sm"
              >
                No matching archives.
              </div>
              <div
                v-else-if="groupArchivesByHost"
                class="archive-groups"
              >
                <div
                  v-for="group in groupedArchives"
                  :key="group.hostname"
                  class="archive-group"
                >
                  <button
                    class="group-header"
                    :class="{ collapsed: isGroupCollapsed(group.hostname) }"
                    @click="toggleGroup(group.hostname)"
                  >
                    <span class="group-chevron">&#9656;</span>
                    <RouterLink
                      v-if="group.matched && group.clientHostname"
                      :to="{ name: 'client-detail', params: { hostname: group.clientHostname } }"
                      class="host-link group-hostname"
                      @click.stop
                    >
                      {{ group.hostname }}
                    </RouterLink>
                    <RouterLink
                      v-else
                      :to="{ name: 'client-detail', params: { hostname: group.hostname } }"
                      class="host-link group-hostname group-unmatched"
                      @click.stop
                    >
                      {{ group.hostname }}
                    </RouterLink>
                    <span
                      v-if="!group.matched"
                      class="match-icon match-warn"
                      title="Unmatched"
                      >&#9888;</span
                    >
                    <span class="group-count">{{ group.archives.length }}</span>
                  </button>
                  <div
                    v-show="!isGroupCollapsed(group.hostname)"
                    class="group-archives"
                  >
                    <div
                      v-for="archive in group.archives"
                      :key="archive.name"
                      class="archive-row"
                      :class="{ selected: selectedArchive?.name === archive.name }"
                      @click="selectArchive(archive)"
                    >
                      <span class="archive-date">{{ formatDate(archive.start) }}</span>
                      <span class="archive-name">{{ archive.name }}</span>
                      <button
                        v-if="isAdmin"
                        class="btn btn-sm btn-ghost archive-row-delete"
                        title="Delete archive"
                        @click.stop="requestArchiveDeletion(archive)"
                      >
                        <Trash2 :size="12" />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
              <div
                v-else
                class="archive-flat-list"
              >
                <div
                  v-for="archive in orderedArchives"
                  :key="archive.name"
                  class="archive-row archive-row-detailed"
                  :class="{ selected: selectedArchive?.name === archive.name }"
                  @click="selectArchive(archive)"
                >
                  <span class="archive-name">{{ archive.name }}</span>
                  <span class="archive-host">{{
                    archive.client_hostname ?? archive.hostname
                  }}</span>
                  <span class="archive-date">{{ formatDate(archive.start) }}</span>
                  <span class="archive-size">{{ formatBytes(archive.original_size) }}</span>
                  <span class="archive-size">{{ formatBytes(archive.deduplicated_size) }}</span>
                  <button
                    v-if="isAdmin"
                    class="btn btn-sm btn-ghost archive-row-delete"
                    title="Delete archive"
                    @click.stop="requestArchiveDeletion(archive)"
                  >
                    <Trash2 :size="12" />
                  </button>
                </div>
              </div>
            </template>
          </div>

          <!-- File browser -->
          <div
            v-if="selectedArchive"
            class="panel browser-panel"
          >
            <div class="panel-header">
              <span class="panel-title">Files &mdash; {{ selectedArchive.name }}</span>
            </div>

            <div class="archive-meta-bar">
              <span class="archive-meta-item">
                <span class="archive-meta-label">Date</span>
                <span class="archive-meta-value">{{ formatDate(selectedArchive.start) }}</span>
              </span>
              <span class="archive-meta-sep" />
              <span class="archive-meta-item">
                <span class="archive-meta-label">Original</span>
                <span class="archive-meta-value">{{
                  formatBytes(selectedArchive.original_size)
                }}</span>
              </span>
              <span class="archive-meta-sep" />
              <span class="archive-meta-item">
                <span class="archive-meta-label">Dedup</span>
                <span class="archive-meta-value">{{
                  formatBytes(selectedArchive.deduplicated_size)
                }}</span>
              </span>
            </div>

            <div class="archive-breadcrumb">
              <button
                v-for="(seg, i) in breadcrumbs"
                :key="seg.path"
                class="crumb"
                :class="{ 'crumb-last': i === breadcrumbs.length - 1 }"
                @click="archiveNavigateTo(seg.path)"
              >
                {{ seg.label }}
              </button>
            </div>

            <BaseSpinner
              v-if="contentsLoading"
              size="sm"
            />
            <div
              v-else-if="indexing"
              class="state-msg state-msg-sm"
            >
              <BaseSpinner size="sm" />
              Indexing archive contents — this only happens once…
            </div>
            <div
              v-else-if="contentsError"
              class="state-msg state-msg-sm state-error"
            >
              {{ contentsError }}
            </div>
            <div
              v-else-if="dirs.length === 0 && files.length === 0"
              class="state-msg state-msg-sm"
            >
              Empty directory.
            </div>
            <table
              v-else
              class="data-table"
            >
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Size</th>
                  <th>Modified</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="entry in dirs"
                  :key="entry.displayName + entry.path"
                  :class="{ clickable: entry.displayName !== '.' }"
                  @click="entry.displayName !== '.' && archiveNavigateTo(entry.path)"
                >
                  <td class="td-name">
                    <Folder
                      :size="16"
                      class="entry-icon"
                    />
                    {{ entry.displayName }}
                  </td>
                  <td class="td-size muted">&mdash;</td>
                  <td class="td-date">{{ formatDate(entry.mtime) }}</td>
                  <td class="td-action">
                    <span class="entry-actions">
                      <button
                        class="btn btn-sm btn-ghost"
                        :title="entry.path ? 'Download as .tar.lz4' : 'Download whole archive'"
                        @click.stop="downloadEntry(entry)"
                      >
                        <Download :size="14" />
                      </button>
                      <button
                        v-if="isAdmin"
                        class="btn btn-sm btn-ghost"
                        :title="entry.path ? 'Restore to host' : 'Restore whole archive to host'"
                        @click.stop="restoreArchiveEntry(entry)"
                      >
                        <RotateCcw :size="14" />
                      </button>
                      <button
                        v-if="isAdmin && entry.path.length === 0 && selectedArchive"
                        class="btn btn-sm btn-ghost"
                        title="Delete whole archive"
                        @click.stop="requestArchiveDeletion(selectedArchive)"
                      >
                        <Trash2 :size="14" />
                      </button>
                    </span>
                  </td>
                </tr>
                <tr
                  v-for="entry in files"
                  :key="entry.path"
                >
                  <td class="td-name">
                    <File
                      :size="16"
                      class="entry-icon"
                    />
                    {{ entryName(entry) }}
                  </td>
                  <td class="td-size">{{ formatBytes(entry.size) }}</td>
                  <td class="td-date">{{ formatDate(entry.mtime) }}</td>
                  <td class="td-action">
                    <span class="entry-actions">
                      <button
                        class="btn btn-sm btn-ghost"
                        title="Download"
                        @click.stop="downloadEntry(entry)"
                      >
                        <Download :size="14" />
                      </button>
                      <button
                        v-if="isAdmin"
                        class="btn btn-sm btn-ghost"
                        title="Restore to host"
                        @click.stop="restoreArchiveEntry(entry)"
                      >
                        <RotateCcw :size="14" />
                      </button>
                    </span>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <div
            v-else
            class="panel browser-panel empty-browser"
          >
            <span class="muted">Select an archive to browse its contents.</span>
          </div>
        </div>
      </div>
    </template>

    <BaseModal
      :open="archivePendingDeletion !== null"
      title="Delete Archive"
      size="sm"
      @close="closeArchiveDeleteDialog"
    >
      <div class="archive-delete-message">
        <div class="archive-delete-icon">
          <Trash2 :size="22" />
        </div>
        <div>
          <p>
            Permanently delete <strong>{{ archivePendingDeletion?.name }}</strong> from
            <strong>{{ repo?.name }}</strong
            >?
          </p>
          <p class="muted">This archive and its stored backup data cannot be recovered.</p>
        </div>
      </div>
      <template #footer>
        <button
          class="btn btn-ghost"
          :disabled="archiveDeleteLoading"
          @click="closeArchiveDeleteDialog"
        >
          Cancel
        </button>
        <button
          class="btn btn-danger"
          :disabled="archiveDeleteLoading"
          @click="confirmArchiveDeletion"
        >
          {{ archiveDeleteLoading ? 'Deleting...' : 'Delete Archive' }}
        </button>
      </template>
    </BaseModal>

    <!-- Passphrase Dialog -->
    <Teleport to="body">
      <div
        v-if="showPassphraseDialog"
        class="overlay"
        @click.self="showPassphraseDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ passphrase ? 'Repository Passphrase' : 'Error' }}
            </h2>
            <button
              class="close-btn"
              @click="showPassphraseDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <template v-if="passphrase">
              <p class="passphrase-warning">Keep this passphrase secure. Do not share it.</p>
              <div class="passphrase-box">
                <code class="passphrase-text">{{ passphrase }}</code>
                <button
                  class="btn btn-sm btn-ghost"
                  @click="passphrase && copyToClipboard(passphrase)"
                >
                  {{ passphraseCopied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
            </template>
            <div
              v-else-if="passphraseError"
              class="form-error"
            >
              {{ passphraseError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showPassphraseDialog = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Delete Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showDeleteDialog"
        class="overlay"
        @click.self="showDeleteDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">⚠️ DESTROY Repository From Disk</h2>
            <button
              class="close-btn"
              @click="showDeleteDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p style="color: var(--danger); font-weight: 600">
              This will PERMANENTLY DELETE all data for
              <strong>{{ repo?.name }}</strong> from the remote filesystem. This action is
              irreversible. All backup archives will be lost forever.
            </p>
            <p>
              The repository at <code>{{ repo?.repo_path }}</code> on
              <code>{{ repo?.ssh_host }}</code> will be removed using <code>rm -rf</code>.
            </p>
            <p>
              All associated schedules will be <strong>disabled</strong> and their repository link
              removed. They will need to be reassigned or deleted manually.
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
              @click="confirmDelete"
            >
              {{ deleteLoading ? 'Destroying...' : 'Destroy Forever' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Remove (DB only) Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showRemoveDialog"
        class="overlay"
        @click.self="showRemoveDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Remove Repository</h2>
            <button
              class="close-btn"
              @click="showRemoveDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Are you sure you want to remove <strong>{{ repo?.name }}</strong> from the database?
            </p>
            <p>
              All associated schedules will be <strong>disabled</strong> and their repository link
              removed. They will need to be reassigned or deleted manually. Reports will be deleted.
            </p>
            <p>The repository data on disk will NOT be touched.</p>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showRemoveDialog = false"
            >
              Cancel
            </button>
            <button
              class="btn btn-danger"
              :disabled="removeLoading"
              @click="confirmRemove"
            >
              {{ removeLoading ? 'Removing...' : 'Remove' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Confirm Relocation Dialog -->
    <Teleport to="body">
      <div
        v-if="showConfirmRelocationDialog"
        class="overlay"
        @click.self="showConfirmRelocationDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Confirm Repository Relocation</h2>
            <button
              class="close-btn"
              @click="showConfirmRelocationDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="break-lock-warning">
              This sets <code>BORG_RELOCATED_REPO_ACCESS_IS_OK=yes</code> for the next backup run,
              allowing borg to accept the repository at its new location. Only confirm if you
              intentionally moved or re-pathed the repository.
            </p>
            <div
              v-if="confirmRelocationResult"
              class="break-lock-success"
            >
              {{ confirmRelocationResult }}
            </div>
            <div
              v-if="confirmRelocationError"
              class="form-error"
            >
              {{ confirmRelocationError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showConfirmRelocationDialog = false"
            >
              {{ confirmRelocationResult ? 'Close' : 'Cancel' }}
            </button>
            <button
              v-if="!confirmRelocationResult"
              class="btn btn-danger"
              :disabled="confirmRelocationLoading"
              @click="doConfirmRelocation"
            >
              {{ confirmRelocationLoading ? 'Confirming...' : 'Yes, Confirm Relocation' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- SSH Host Key Dialog -->
    <Teleport to="body">
      <div
        v-if="showAcceptHostKeyDialog"
        class="overlay"
        @click.self="showAcceptHostKeyDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Accept SSH Host Key</h2>
            <button
              class="close-btn"
              @click="showAcceptHostKeyDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="break-lock-warning">
              A different SSH host key was detected for <code>{{ repo?.ssh_host }}</code
              >. Verify the key below before accepting it.
            </p>
            <div
              v-if="expectedHostKey"
              class="ssh-key-box mono"
            >
              {{ expectedHostKey }}
            </div>
            <div
              v-if="acceptHostKeyError"
              class="form-error"
            >
              {{ acceptHostKeyError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showAcceptHostKeyDialog = false"
            >
              Cancel
            </button>
            <button
              v-if="expectedHostKey"
              class="btn btn-primary"
              :disabled="acceptHostKeyLoading"
              @click="acceptHostKey"
            >
              {{ acceptHostKeyLoading ? 'Accepting...' : 'Accept Key' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Break Lock Confirmation Dialog -->
    <Teleport to="body">
      <div
        v-if="showBreakLockDialog"
        class="overlay"
        @click.self="showBreakLockDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Break Repository Lock</h2>
            <button
              class="close-btn"
              @click="showBreakLockDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="break-lock-warning">
              This will forcibly remove the lock from the repository. Only use this if you are
              certain no backup is currently running. Breaking a lock during an active backup
              <strong>will corrupt the repository</strong>.
            </p>
            <div
              v-if="breakLockResult"
              class="break-lock-success"
            >
              {{ breakLockResult }}
            </div>
            <div
              v-if="breakLockError"
              class="form-error"
            >
              {{ breakLockError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-ghost"
              @click="showBreakLockDialog = false"
            >
              {{ breakLockResult ? 'Close' : 'Cancel' }}
            </button>
            <button
              v-if="!breakLockResult"
              class="btn btn-danger"
              :disabled="breakLockLoading"
              @click="confirmBreakLock"
            >
              {{ breakLockLoading ? 'Breaking Lock...' : 'Yes, Break Lock' }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.repo-detail {
  max-width: 1200px;
}

.import-progress {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.4rem;
}

.import-progress-track {
  flex: 1;
  height: 4px;
  background: var(--border);
  border-radius: 2px;
  overflow: hidden;
}

.import-progress-bar {
  height: 100%;
  background: var(--accent);
  border-radius: 2px;
  transition: width 0.4s ease;
}

.import-progress-label {
  font-size: 0.75rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.import-status-msg {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin: 0.4rem 0 0;
  word-break: break-word;
}

/* Breadcrumb nav */
.breadcrumb-nav {
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

/* States */
.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-msg-sm {
  padding: 1.5rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  text-align: left;
  font-size: 0.875rem;
}

.state-error {
  color: var(--danger);
}

.muted {
  color: var(--text-muted);
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

/* Info card */
.info-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;

  & + & {
    margin-top: 0.75rem;
  }
}

.info-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1.25rem;
}

.info-header-actions {
  display: flex;
  gap: 0.5rem;
}

.info-title {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-bottom: 1.25rem;
}

.info-card-header .info-title {
  margin-bottom: 0;
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

/* Edit form */
.edit-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
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

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--border);
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  justify-content: space-between;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

/* Tags */
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

/* Borg console */
.console-desc {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin-bottom: 0.75rem;
  line-height: 1.5;
}

.console-code {
  font-family: var(--mono);
  font-size: 0.78rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.05rem 0.3rem;
}

.console-input-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.console-prefix {
  font-family: var(--mono);
  font-size: 0.85rem;
  color: var(--text-muted);
  flex-shrink: 0;
}

.console-input {
  flex: 1;
  font-family: var(--mono);
  font-size: 0.85rem;
}

.console-hints {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.35rem;
  margin-top: 0.5rem;
}

.console-hint-label {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.console-hint-cmd {
  font-family: var(--mono);
  font-size: 0.75rem;
  padding: 0.1rem 0.4rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--accent);
  transition: background 0.1s;
}

.console-hint-cmd:hover {
  background: var(--accent-subtle);
}

.console-error {
  margin-top: 0.75rem;
  padding: 0.6rem 0.75rem;
  background: var(--danger-subtle, oklch(0.97 0.04 25));
  border: 1px solid var(--danger);
  border-radius: var(--radius-sm);
  font-size: 0.82rem;
  color: var(--danger);
}

.console-output {
  margin-top: 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.console-output-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.4rem 0.75rem;
  background: var(--bg-input);
  border-bottom: 1px solid var(--border);
  font-size: 0.78rem;
}

.console-output-label {
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-size: 0.7rem;
}

.exit-ok {
  color: var(--success);
  font-family: var(--mono);
  font-size: 0.78rem;
}

.exit-warn {
  color: var(--warning);
  font-family: var(--mono);
  font-size: 0.78rem;
}

.exit-err {
  color: var(--danger);
  font-family: var(--mono);
  font-size: 0.78rem;
}

.console-pre {
  margin: 0;
  padding: 0.75rem;
  font-family: var(--mono);
  font-size: 0.78rem;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-primary);
  background: var(--bg-base);
  max-height: 400px;
  overflow-y: auto;
}

.console-pre-stderr {
  color: var(--warning);
  border-top: 1px solid var(--border);
}

.console-empty {
  display: block;
  padding: 0.75rem;
  font-size: 0.82rem;
  color: var(--text-muted);
  font-style: italic;
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

.danger-body + .danger-body {
  margin-top: 1.25rem;
  padding-top: 1.25rem;
  border-top: 1px solid var(--border);
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

.danger-action-wrap {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 0.35rem;
  flex-shrink: 0;
}

.danger-hint {
  font-size: 0.7rem;
  color: var(--warning);
  text-align: right;
  max-width: 180px;
}

.ssh-host-key {
  word-break: break-all;
}

.break-lock-warning {
  color: var(--danger);
  font-size: 0.875rem;
  line-height: 1.5;
}

.break-lock-success {
  margin-top: 0.75rem;
  padding: 0.75rem;
  background: var(--success-subtle, oklch(0.95 0.05 145));
  border-radius: var(--radius-sm);
  font-size: 0.85rem;
  color: var(--success);
}

.current-op-running {
  color: var(--warning, oklch(0.7 0.15 80));
  font-weight: 500;
}

.ssh-key-box {
  margin-top: 0.75rem;
  padding: 0.85rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-muted);
  font-size: 0.8rem;
  line-height: 1.5;
  word-break: break-all;
}

/* Archives layout */
.archives-layout {
  display: grid;
  grid-template-columns: 1fr 1.2fr;
  gap: 1rem;
  align-items: start;
}

.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.panel-title {
  font-size: 0.8rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.archive-controls {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border);
}

.archive-controls .filter-input {
  flex: 1;
}

.archive-sort-select {
  min-width: 13rem;
}

.archive-group-toggle {
  flex-shrink: 0;
  white-space: nowrap;
}

.archive-filter {
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border);
}

.archive-filter .filter-input {
  width: 100%;
}

.archive-groups,
.archive-flat-list {
  max-height: 500px;
  overflow-y: auto;
}

.archive-group {
  border-bottom: 1px solid var(--border-subtle);
}

.archive-group:last-child {
  border-bottom: none;
}

.group-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-subtle);
  border: none;
  cursor: pointer;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
  transition: background 0.1s;
}

.group-header:hover {
  background: var(--bg-hover);
}

.group-chevron {
  display: inline-block;
  font-size: 1rem;
  transition: transform 0.15s;
  transform: rotate(90deg);
}

.group-header.collapsed .group-chevron {
  transform: rotate(0deg);
}

.group-hostname {
  flex: 1;
}

.group-unmatched {
  color: var(--warning);
}

.group-count {
  font-size: 0.7rem;
  color: var(--text-muted);
  background: var(--bg-card);
  border-radius: 9999px;
  padding: 0.1rem 0.5rem;
  min-width: 1.4rem;
  text-align: center;
}

.group-archives {
  display: flex;
  flex-direction: column;
}

.archive-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem 0.4rem 1.5rem;
  border: none;
  background: none;
  cursor: pointer;
  text-align: left;
  transition: background 0.1s;
  border-bottom: 1px solid var(--border-subtle);
}

.archive-row:last-child {
  border-bottom: none;
}

.archive-row:hover {
  background: var(--bg-hover);
}

.archive-row.selected {
  background: var(--accent-subtle);
}

.archive-date {
  font-size: 0.75rem;
  color: var(--text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}

.archive-name {
  font-family: var(--mono);
  font-size: 0.75rem;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-row-detailed {
  display: grid;
  grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr) auto auto auto auto;
  gap: 0.75rem;
  padding-left: 0.75rem;
}

.archive-row-delete {
  margin-left: auto;
  opacity: 0;
  transition: opacity 0.1s;
  flex-shrink: 0;
}

.archive-row:hover .archive-row-delete,
.archive-row.selected .archive-row-delete {
  opacity: 1;
}

.archive-host,
.archive-size {
  font-size: 0.75rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.archive-host {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.data-table {
  min-width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.data-table th {
  text-align: left;
  padding: 0.5rem 0.75rem;
  color: var(--text-muted);
  font-weight: 600;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid var(--border);
}

.data-table td {
  padding: 0.6rem 0.75rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.data-table tr:last-child td {
  border-bottom: none;
}

.data-table tr.clickable {
  cursor: pointer;
  transition: background 0.1s;
}

.data-table tr.clickable:hover {
  background: var(--bg-hover);
}

.data-table tr.selected td {
  background: var(--accent-subtle);
  color: var(--text-primary);
}

.td-mono {
  font-family: var(--mono);
  font-size: 0.8rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.td-date {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.td-host {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.td-name {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-family: var(--mono);
  font-size: 0.82rem;
}

.td-size {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.td-action {
  text-align: right;
}

.entry-actions {
  display: inline-flex;
  gap: 0.25rem;
}

.entry-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.archive-meta-bar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.archive-meta-item {
  display: flex;
  align-items: baseline;
  gap: 0.35rem;
}

.archive-meta-label {
  font-size: 0.72rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.archive-meta-value {
  font-size: 0.82rem;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

.archive-meta-sep {
  width: 1px;
  height: 0.9rem;
  background: var(--border);
  flex-shrink: 0;
}

.archive-breadcrumb {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.1rem;
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.crumb {
  background: none;
  border: none;
  color: var(--accent);
  cursor: pointer;
  font-size: 0.82rem;
  font-family: var(--mono);
  padding: 0.15rem 0.3rem;
  border-radius: var(--radius-sm);
  transition:
    background 0.1s,
    color 0.1s;
}

.crumb:hover {
  background: var(--accent-subtle);
  color: var(--accent-hover);
}

.crumb-last {
  color: var(--text-primary);
  cursor: default;
}

.crumb-last:hover {
  background: none;
  color: var(--text-primary);
}

.crumb:not(.crumb-last)::after {
  content: ' /';
  color: var(--text-muted);
  margin-left: 0.2rem;
}

.empty-browser {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  font-size: 0.875rem;
}

.spinner {
  display: inline-block;
  width: 1rem;
  height: 1rem;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Input */

.input:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.input-sm {
  padding: 0.35rem 0.55rem;
  font-size: 0.8rem;
  width: auto;
  min-width: 140px;
}

.passphrase-warning {
  color: var(--warning);
  font-size: 0.875rem;
  font-weight: 500;
  margin-bottom: 0.75rem;
}

.passphrase-box {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem 1rem;
}

.passphrase-text {
  flex: 1;
  font-family: var(--mono);
  font-size: 0.82rem;
  color: var(--text-primary);
  word-break: break-all;
  background: transparent;
  padding: 0;
}

/* Responsive */
@media (max-width: 1100px) {
  .archives-layout {
    grid-template-columns: 1fr;
  }
}

/* Unmatched banner */
.unmatched-banner {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
  padding: 0.75rem 1rem;
  background: var(--warning-subtle, oklch(0.97 0.04 80));
  border: 1px solid var(--warning);
  border-radius: var(--radius);
  font-size: 0.875rem;
}

.unmatched-banner-text {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  color: var(--text-primary);
}

.unmatched-hostnames {
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.unmatched-hostname {
  display: inline-block;
  margin: 0 0.25rem;
  padding: 0.1rem 0.4rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: 0.75rem;
}

.unmatched-hint {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.td-match {
  width: 2.5rem;
  min-width: 2.5rem;
  text-align: center;
  font-size: 0.9rem;
}

.th-match {
  width: 2.5rem;
}

.match-ok {
  color: var(--success);
}

.match-warn {
  color: var(--warning);
}

.host-link {
  color: var(--accent);
  text-decoration: none;
}

.host-link:hover {
  text-decoration: underline;
}

.unmatched-host-link {
  color: var(--warning);
  text-decoration: underline;
  text-decoration-color: transparent;
  transition: text-decoration-color 0.15s;
}

.unmatched-host-link:hover {
  text-decoration-color: var(--warning);
}

.archive-delete-message {
  display: flex;
  gap: 1rem;
  align-items: flex-start;
}

.archive-delete-message p {
  margin: 0 0 0.5rem;
  line-height: 1.5;
}

.archive-delete-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2.75rem;
  height: 2.75rem;
  flex: 0 0 auto;
  border-radius: 50%;
  color: var(--danger);
  background: var(--danger-subtle);
}
</style>
