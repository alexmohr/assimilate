<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { useArchiveBrowser } from '../composables/useArchiveBrowser'
import { useWebSocket } from '../composables/useWebSocket'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import QuotaPanel from '../components/QuotaPanel.vue'

type TabId = 'overview' | 'archives'
type CompressionType = 'lz4' | 'zstd' | 'none'
type EncryptionType =
  | 'repokey'
  | 'repokey-blake2'
  | 'keyfile'
  | 'keyfile-blake2'
  | 'authenticated'
  | 'authenticated-blake2'
  | 'none'

interface RepoWithStats {
  id: number
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: string
  encryption: string
  enabled: boolean
  importing: boolean
  import_error: string | null
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
}

interface TagRow {
  id: number
  name: string
  color: string
  scope: string
}

interface EditForm {
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: CompressionType
  encryption: EncryptionType
  enabled: boolean
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
  repo_path: '',
  ssh_user: '',
  ssh_host: '',
  ssh_port: 22,
  compression: 'lz4',
  encryption: 'repokey',
  enabled: true,
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

// Delete
const showDeleteDialog = ref(false)
const deleteLoading = ref(false)

useEscapeKey(showDeleteDialog, () => {
  showDeleteDialog.value = false
})

// Break Lock
const showBreakLockDialog = ref(false)
const breakLockLoading = ref(false)
const breakLockError = ref<string | null>(null)
const breakLockResult = ref<string | null>(null)
const activeBackupClient = ref<string | null>(null)

useEscapeKey(showBreakLockDialog, () => {
  showBreakLockDialog.value = false
})

interface BackupStartedPayload {
  hostname: string
  target_name: string
}

const { onMessage } = useWebSocket()

onMessage<BackupStartedPayload>('BackupStarted', (payload) => {
  if (repo.value && payload.target_name === repo.value.name) {
    activeBackupClient.value = payload.hostname
  }
})

onMessage<BackupStartedPayload>('BackupCompleted', (payload) => {
  if (repo.value && payload.target_name === repo.value.name) {
    activeBackupClient.value = null
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
  breadcrumbs,
  dirs,
  files,
  loadArchives,
  selectArchive,
  navigateTo: archiveNavigateTo,
  entryName,
  downloadFile,
} = useArchiveBrowser(repoIdRef)

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

async function loadRepo(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const res = await apiClient.get<RepoWithStats>(`/repos/${repoId.value}`)
    repo.value = res.data
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
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

function startEdit(): void {
  if (!repo.value) return
  editForm.repo_path = repo.value.repo_path
  editForm.ssh_user = repo.value.ssh_user
  editForm.ssh_host = repo.value.ssh_host
  editForm.ssh_port = repo.value.ssh_port
  editForm.compression = repo.value.compression as CompressionType
  editForm.encryption = repo.value.encryption as EncryptionType
  editForm.enabled = repo.value.enabled
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
    await apiClient.put(`/repos/${repoId.value}`, {
      repo_path: editForm.repo_path.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_host: editForm.ssh_host.trim(),
      ssh_port: editForm.ssh_port,
      compression: editForm.compression,
      encryption: editForm.encryption,
      enabled: editForm.enabled,
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
    await apiClient.delete(`/repos/${repoId.value}`)
    showDeleteDialog.value = false
    router.push('/repos')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    deleteLoading.value = false
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

onMounted(async () => {
  await loadRepo()
  if (repo.value) {
    await Promise.all([loadTags(), loadArchives()])
  }
})
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
        <!-- Stats cards -->
        <div class="stats-row">
          <div class="stat-card">
            <span class="stat-card-value">{{ repo.archive_count }}</span>
            <span class="stat-card-label">Archives</span>
          </div>
          <div class="stat-card">
            <span class="stat-card-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
            <span class="stat-card-label">Deduplicated</span>
          </div>
          <div class="stat-card">
            <span class="stat-card-value">{{ formatBytes(repo.total_original_size) }}</span>
            <span class="stat-card-label">Original</span>
          </div>
          <div class="stat-card">
            <span class="stat-card-value">{{ formatLastBackup(repo.last_backup_at) }}</span>
            <span class="stat-card-label">Last Backup</span>
          </div>
          <div class="stat-card">
            <span class="stat-card-value">{{ repo.client_count }}</span>
            <span class="stat-card-label">Clients</span>
          </div>
        </div>

        <!-- Info card -->
        <div class="info-card">
          <div class="info-card-header">
            <h3 class="info-title">Repository Information</h3>
            <div class="info-header-actions">
              <template v-if="isAdmin && !isEditing">
                <button
                  class="btn btn-sm btn-ghost"
                  :disabled="passphraseLoading"
                  @click="revealPassphrase"
                >
                  {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
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
                        ? 'Importing\u2026'
                        : repo.enabled
                          ? 'Enabled'
                          : 'Disabled'
                  }}
                </span>
              </dd>
            </dl>
          </template>

          <template v-else>
            <div class="edit-form">
              <div class="form-grid">
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

        <!-- Danger zone -->
        <div
          v-if="isAdmin"
          class="info-card danger-zone"
        >
          <h3 class="info-title">Danger Zone</h3>
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
                :disabled="!!activeBackupClient || breakLockLoading"
                :title="
                  activeBackupClient
                    ? `Client '${activeBackupClient}' is currently using this repository`
                    : undefined
                "
                @click="showBreakLockDialog = true"
              >
                {{ breakLockLoading ? 'Breaking...' : 'Break Lock' }}
              </button>
              <span
                v-if="activeBackupClient"
                class="danger-hint"
              >
                Client <strong>{{ activeBackupClient }}</strong> is using this repository.
              </span>
            </div>
          </div>
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Delete Repository</span>
              <span class="danger-desc"
                >Permanently remove this repository and all associated schedules and reports.</span
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
            <table
              v-else
              class="data-table"
            >
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Date</th>
                  <th>Host</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="archive in sortedArchives"
                  :key="archive.name"
                  :class="['clickable', { selected: selectedArchive?.name === archive.name }]"
                  @click="selectArchive(archive)"
                >
                  <td class="td-mono">{{ archive.name }}</td>
                  <td class="td-date">{{ formatDate(archive.start) }}</td>
                  <td class="td-host">{{ archive.hostname }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <!-- File browser -->
          <div
            v-if="selectedArchive"
            class="panel browser-panel"
          >
            <div class="panel-header">
              <span class="panel-title">Files &mdash; {{ selectedArchive.name }}</span>
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
                  :key="entry.path"
                  class="clickable"
                  @click="archiveNavigateTo(entry.path)"
                >
                  <td class="td-name">
                    <span class="icon-dir">&#128193;</span>
                    {{ entryName(entry) }}
                  </td>
                  <td class="td-size muted">&mdash;</td>
                  <td class="td-date">{{ formatDate(entry.mtime) }}</td>
                  <td />
                </tr>
                <tr
                  v-for="entry in files"
                  :key="entry.path"
                >
                  <td class="td-name">
                    <span class="icon-file">&#128196;</span>
                    {{ entryName(entry) }}
                  </td>
                  <td class="td-size">{{ formatBytes(entry.size) }}</td>
                  <td class="td-date">{{ formatDate(entry.mtime) }}</td>
                  <td class="td-action">
                    <button
                      class="btn btn-sm btn-ghost"
                      @click="downloadFile(entry)"
                    >
                      Download
                    </button>
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
            <h2 class="dialog-title">Delete Repository</h2>
            <button
              class="close-btn"
              @click="showDeleteDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p>
              Are you sure you want to delete <strong>{{ repo?.name }}</strong
              >? This will also remove all associated schedules and reports.
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
              {{ deleteLoading ? 'Deleting...' : 'Delete' }}
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

/* Stats row */
.stats-row {
  display: flex;
  gap: 1rem;
  margin-bottom: 1.5rem;
  flex-wrap: wrap;
}

.stat-card {
  flex: 1;
  min-width: 120px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.stat-card-value {
  font-size: 1.1rem;
  font-weight: 700;
  color: var(--text-primary);
}

.stat-card-label {
  font-size: 0.7rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
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

/* Archives layout */
.archives-layout {
  display: grid;
  grid-template-columns: 380px 1fr;
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

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.data-table th {
  text-align: left;
  padding: 0.5rem 1rem;
  color: var(--text-muted);
  font-weight: 600;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid var(--border);
}

.data-table td {
  padding: 0.6rem 1rem;
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
  max-width: 180px;
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

.icon-dir,
.icon-file {
  font-size: 0.9rem;
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
@media (max-width: 768px) {
  .archives-layout {
    grid-template-columns: 1fr;
  }

  .stats-row {
    flex-direction: column;
  }

  .stat-card {
    min-width: 0;
  }
}
</style>
