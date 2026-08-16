<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { Folder, FolderPlus } from '@lucide/vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import BaseModal from './BaseModal.vue'
import SshKeyDeployPanel from './SshKeyDeployPanel.vue'
import type { Repo, RepoWithStats } from '../types/repo'

/**
 * Create-or-import dialog for a repository, including the remote folder
 * browser it uses to pick a path. Lifted out of `ReposView`, which also
 * carried an unreachable "edit" mode for it - repositories are edited from
 * their detail page. See docs/contributing/ui-design-audit.md (F-24).
 */

type CompressionType = 'lz4' | 'zstd' | 'none'
type EncryptionType =
  | 'repokey'
  | 'repokey-blake2'
  | 'keyfile'
  | 'keyfile-blake2'
  | 'authenticated'
  | 'authenticated-blake2'
  | 'none'

/** Which flow the dialog is in: adopt an existing repo, or init a new one. */
export type RepoDialogMode = 'import' | 'create'

interface RepoForm {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  passphrase: string
  compression: CompressionType
  encryption: EncryptionType
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

const ROOT_PATH = '/'

const props = defineProps<{
  open: boolean
  mode: RepoDialogMode
  /** Existing repositories, used to offer their SSH targets for auto-fill. */
  repos: RepoWithStats[]
}>()

const emit = defineEmits<{
  close: []
  /** An import was accepted; the new row is optimistically added by the view. */
  imported: [repo: Repo]
  /** A repository was initialised; the caller should refetch. */
  created: []
}>()

const loading = ref(false)
const error = ref<string | null>(null)
const showDeployKey = ref(false)

const defaultRepoForm = (): RepoForm => ({
  name: '',
  repo_path: '',
  ssh_user: 'borg',
  ssh_host: '',
  ssh_port: 22,
  passphrase: '',
  compression: 'lz4',
  encryption: 'repokey-blake2',
})

const form = reactive<RepoForm>(defaultRepoForm())

const testConn = reactive<TestConnState>({ loading: false, result: null })

const browser = reactive<BrowserState>({
  path: ROOT_PATH,
  entries: [],
  loading: false,
  error: null,
  showBrowser: false,
})

const folderModal = reactive({ open: false, name: '', error: null as string | null })

const sshTargets = computed<SshTarget[]>(() => {
  const seen = new Set<string>()
  const targets: SshTarget[] = []
  for (const repo of props.repos) {
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

const sshReady = computed(() => form.ssh_host.trim().length > 0)

const formValid = computed(
  () =>
    form.name.trim().length > 0 &&
    form.ssh_host.trim().length > 0 &&
    form.repo_path.trim().length > 0 &&
    form.passphrase.length > 0,
)

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
  const pathValue = form.repo_path.trim()
  if (pathValue.endsWith('/') || pathValue === ROOT_PATH) {
    const dir = pathValue === ROOT_PATH ? ROOT_PATH : pathValue.replace(/\/+$/, '')
    if (dir !== browser.path) {
      browseDir(dir || '/')
    }
  }
}

async function fetchAutocomplete(): Promise<void> {
  if (!sshReady.value || !form.repo_path.trim()) {
    autocompleteEntries.value = []
    showAutocomplete.value = false
    return
  }
  const pathValue = form.repo_path.trim()
  const parentDir = pathValue.includes('/')
    ? pathValue.substring(0, pathValue.lastIndexOf('/')) || '/'
    : '/'
  try {
    const res = await apiClient.post<{ path: string; entries: DirEntry[]; error?: string }>(
      '/ssh/list-dir',
      {
        ssh_host: form.ssh_host.trim(),
        ssh_user: form.ssh_user.trim(),
        ssh_port: form.ssh_port,
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
  const pathValue = form.repo_path.trim()
  const parentDir = pathValue.substring(0, pathValue.lastIndexOf('/')) || ''
  form.repo_path = parentDir === ROOT_PATH ? `/${entry.name}` : `${parentDir}/${entry.name}`
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
      ssh_host: form.ssh_host.trim(),
      ssh_user: form.ssh_user.trim(),
      ssh_port: form.ssh_port,
      path: newPath,
    })
    folderModal.open = false
    await browseDir(newPath)
  } catch (e: unknown) {
    folderModal.error = extractError(e)
  }
}

function applySshTarget(event: Event): void {
  const value = (event.target as HTMLSelectElement).value
  if (!value) return
  const target = sshTargets.value.find((t) => t.label === value)
  if (target) {
    form.ssh_user = target.ssh_user
    form.ssh_host = target.ssh_host
    form.ssh_port = target.ssh_port
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
        ssh_host: form.ssh_host.trim(),
        ssh_user: form.ssh_user.trim(),
        ssh_port: form.ssh_port,
        path,
      },
    )
    if (res.data.error) {
      browser.error = res.data.error
    } else {
      browser.path = res.data.path
      browser.entries = res.data.entries.filter((e) => e.is_dir)
      form.repo_path = res.data.path
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

async function submit(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    if (props.mode === 'import') {
      const res = await apiClient.post<Repo>('/repos', {
        name: form.name.trim(),
        repo_path: form.repo_path.trim(),
        ssh_user: form.ssh_user.trim(),
        ssh_host: form.ssh_host.trim(),
        ssh_port: form.ssh_port,
        passphrase: form.passphrase,
        compression: form.compression,
      })
      emit('imported', res.data)
      emit('close')
      return
    }
    await apiClient.post('/repos/init', {
      name: form.name.trim(),
      repo_path: form.repo_path.trim(),
      ssh_user: form.ssh_user.trim(),
      ssh_host: form.ssh_host.trim(),
      ssh_port: form.ssh_port,
      passphrase: form.passphrase,
      encryption: form.encryption,
      compression: form.compression,
    })
    emit('created')
    emit('close')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
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
      ssh_host: form.ssh_host.trim(),
      ssh_user: form.ssh_user.trim(),
      ssh_port: form.ssh_port,
    })
    testConn.result = res.data
  } catch (e: unknown) {
    testConn.result = { ssh_ok: false, borg_installed: false, error: extractError(e) }
  } finally {
    testConn.loading = false
  }
}

/** Called by the view each time it opens the dialog, so no state carries over. */
function reset(): void {
  error.value = null
  showDeployKey.value = false
  testConn.result = null
  browser.path = ROOT_PATH
  browser.entries = []
  browser.error = null
  browser.showBrowser = false
  Object.assign(form, defaultRepoForm())
}

defineExpose({ reset })
</script>

<template>
  <BaseModal
    :open="open"
    size="lg"
    @close="emit('close')"
  >
    <template #header="{ titleId }">
      <h2
        :id="titleId"
        class="modal-title"
      >
        <template v-if="mode === 'create'">Create Repository</template>
        <template v-else>Import Repository</template>
      </h2>
    </template>
    <div class="form-grid">
      <!-- Name field -->
      <div class="field field-full">
        <label class="field-label">Name <span class="required">*</span></label>
        <input
          v-model="form.name"
          class="input"
          placeholder="e.g. inhouse-backups"
        />
        <span class="field-hint">A short identifier for this storage target</span>
      </div>

      <!-- SSH params -->
      <div
        v-if="sshTargets.length > 0"
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
          v-model="form.ssh_user"
          class="input mono"
          placeholder="borg"
        />
      </div>
      <div class="field">
        <label class="field-label">SSH Host <span class="required">*</span></label>
        <input
          v-model="form.ssh_host"
          class="input mono"
          placeholder="backup.example.com"
        />
      </div>
      <div class="field field-narrow">
        <label class="field-label">SSH Port</label>
        <input
          v-model.number="form.ssh_port"
          class="input"
          type="number"
          min="1"
          max="65535"
        />
      </div>

      <!-- Test & Deploy SSH Key -->
      <div class="field field-full">
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
          :ssh-host="form.ssh_host"
          :ssh-user="form.ssh_user"
          :ssh-port="form.ssh_port"
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
              v-model="form.repo_path"
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
            class="btn btn-sm btn-ghost"
            :disabled="!sshReady || browser.loading"
            @click="browseDir(form.repo_path || '/')"
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
            v-if="mode === 'create'"
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
      <div class="field field-full">
        <label class="field-label">Passphrase <span class="required">*</span></label>
        <input
          v-model="form.passphrase"
          class="input"
          type="password"
          placeholder="Repository encryption passphrase"
        />
      </div>

      <div
        v-if="mode === 'create'"
        class="field"
      >
        <label class="field-label">Encryption <span class="required">*</span></label>
        <select
          v-model="form.encryption"
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
          v-model="form.compression"
          class="input"
        >
          <option value="lz4">lz4</option>
          <option value="zstd">zstd</option>
          <option value="none">none</option>
        </select>
      </div>
    </div>

    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="emit('close')"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        :disabled="loading || !formValid"
        @click="submit"
      >
        <template v-if="loading"> Saving... </template>
        <template v-else-if="mode === 'create'"> Create Repo </template>
        <template v-else> Import Repo </template>
      </button>
    </template>
  </BaseModal>

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
        class="field-label"
        >Folder name</label
      >
      <input
        id="folder-name-input"
        v-model="folderModal.name"
        class="input form-control"
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
.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0 1rem;
}

.form-grid-below {
  margin-top: 1.25rem;
  border-top: 1px solid var(--border);
  padding-top: 1rem;
}

.field-full {
  grid-column: 1 / -1;
}

.field-narrow {
  max-width: 120px;
}

.ssh-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.deploy-result {
  font-size: var(--fs-sm);
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
  font-size: var(--fs-sm);
  font-family: var(--mono);
  color: var(--text-secondary);
  cursor: pointer;
}

.autocomplete-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
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
  font-size: var(--fs-sm);
  font-family: var(--mono);
}

.breadcrumb {
  cursor: pointer;
  color: var(--accent);
  transition: color var(--duration-base);
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

.browser-mkdir-btn {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: var(--fs-xs);
}

.browser-error {
  padding: 0.75rem;
  color: var(--danger);
  font-size: var(--fs-sm);
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
  font-size: var(--fs-sm);
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

.folder-modal-form {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.folder-modal-error {
  font-size: var(--fs-base);
  color: var(--danger);
  margin: 0;
}
</style>
