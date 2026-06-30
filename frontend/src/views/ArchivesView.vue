<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { Folder, File, Download } from '@lucide/vue'
import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import BaseSpinner from '../components/BaseSpinner.vue'
import RestoreWizard from '../components/RestoreWizard.vue'
import ArchiveDiff from '../components/ArchiveDiff.vue'
import FileSearch from '../components/FileSearch.vue'
import BaseHostLink from '../components/BaseHostLink.vue'
import type { ContentsResponse, ContentEntryResponse } from '../types/generated'

interface RepoOption {
  id: number
  hostname: string
  target_name: string
  enabled: boolean
}

interface ArchiveEntry {
  name: string
  start: string
  hostname: string
  comment: string
  original_size: number
  deduplicated_size: number
  matched: boolean | null
  agent_hostname: string | null
}

interface BreadcrumbSegment {
  label: string
  path: string
}

const repos = ref<RepoOption[]>([])
const reposLoading = ref(false)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchive = ref<ArchiveEntry | null>(null)

const currentPath = ref('/')
const contents = ref<ContentEntryResponse[]>([])
const contentsLoading = ref(false)
const contentsError = ref<string | null>(null)
const indexing = ref(false)
const showPassphraseDialog = ref(false)

let pollTimer: ReturnType<typeof setInterval> | null = null

function stopPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling(archiveName: string, pendingPath: string): void {
  stopPolling()
  pollTimer = setInterval(async () => {
    if (selectedRepoId.value === null) return
    try {
      const res = await apiClient.get<{ status: string; error?: string }>(
        `/repos/${selectedRepoId.value}/archives/${encodeURIComponent(archiveName)}/index-status`,
      )
      if (res.data.status === 'done') {
        stopPolling()
        indexing.value = false
        await loadContents(pendingPath)
      } else if (res.data.status === 'failed') {
        stopPolling()
        indexing.value = false
        contentsError.value = res.data.error ?? 'Archive indexing failed'
      }
    } catch (e: unknown) {
      stopPolling()
      indexing.value = false
      contentsError.value = extractError(e)
    }
  }, 2000)
}

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const archiveFilters = ref({
  name: { value: '', matchMode: FilterMatchMode.CONTAINS },
  start: { value: '', matchMode: FilterMatchMode.CONTAINS },
  hostname: { value: '', matchMode: FilterMatchMode.CONTAINS },
  original_size: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

const breadcrumbs = computed<BreadcrumbSegment[]>(() => {
  const path = currentPath.value
  if (path === '/') return [{ label: '/', path: '/' }]
  const parts = path.replace(/^\//, '').split('/')
  const segments: BreadcrumbSegment[] = [{ label: '~', path: '/' }]
  let accumulated = ''
  for (const part of parts) {
    accumulated += `/${part}`
    segments.push({ label: part, path: accumulated })
  }
  return segments
})

interface DisplayEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
  displayName: string
  isDir: boolean
}

const browserEntries = computed<DisplayEntry[]>(() => {
  const currentDir = currentPath.value.replace(/^\//, '')
  const dirList = contents.value
    .filter((e) => e.type === 'd' && e.path !== currentDir)
    .sort((a, b) => a.path.localeCompare(b.path))
  const fileList = contents.value
    .filter((e) => e.type !== 'd')
    .sort((a, b) => a.path.localeCompare(b.path))

  const entries: DisplayEntry[] = []

  const currentEntry = contents.value.find((e) => e.type === 'd' && e.path === currentDir)
  if (currentEntry) {
    entries.push({
      type: currentEntry.type,
      path: currentEntry.path,
      size: Number(currentEntry.size),
      mtime: currentEntry.mtime,
      mode: currentEntry.mode,
      displayName: '.',
      isDir: true,
    })
  } else if (currentPath.value === '/') {
    entries.push({
      type: 'd',
      path: '',
      size: 0,
      mtime: '',
      mode: '',
      displayName: '.',
      isDir: true,
    })
  }

  if (currentPath.value !== '/') {
    const parentPath = currentPath.value.replace(/\/[^/]+$/, '') || '/'
    entries.push({
      type: 'd',
      path: parentPath,
      size: 0,
      mtime: '',
      mode: '',
      displayName: '..',
      isDir: true,
    })
  }

  return [
    ...entries,
    ...[...dirList, ...fileList].map((e) => ({
      type: e.type,
      path: e.path,
      size: Number(e.size),
      mtime: e.mtime,
      mode: e.mode,
      displayName: e.path.split('/').pop() ?? e.path,
      isDir: e.type === 'd',
    })),
  ]
})

const browserFilters = ref({
  displayName: { value: '', matchMode: FilterMatchMode.CONTAINS },
  size: { value: '', matchMode: FilterMatchMode.CONTAINS },
  mtime: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

async function loadRepos(): Promise<void> {
  reposLoading.value = true
  reposError.value = null
  try {
    const res = await apiClient.get<RepoOption[]>('/repos')
    repos.value = res.data
  } catch (e: unknown) {
    reposError.value = extractError(e)
  } finally {
    reposLoading.value = false
  }
}

async function onRepoChange(): Promise<void> {
  archives.value = []
  selectedArchive.value = null
  contents.value = []
  currentPath.value = '/'
  archivesError.value = null
  contentsError.value = null
  if (selectedRepoId.value === null) return
  await loadArchives()
}

async function loadArchives(): Promise<void> {
  if (selectedRepoId.value === null) return
  archivesLoading.value = true
  archivesError.value = null
  try {
    const res = await apiClient.get<ArchiveEntry[]>(`/repos/${selectedRepoId.value}/archives`)
    archives.value = res.data
  } catch (e: unknown) {
    archivesError.value = extractError(e)
  } finally {
    archivesLoading.value = false
  }
}

async function selectArchive(archive: ArchiveEntry): Promise<void> {
  stopPolling()
  indexing.value = false
  selectedArchive.value = archive
  currentPath.value = '/'
  contents.value = []
  contentsError.value = null
  await loadContents('/')
}

async function loadContents(path: string): Promise<void> {
  if (selectedRepoId.value === null || !selectedArchive.value) return
  contentsLoading.value = true
  contentsError.value = null
  const normalizedPath = path === '/' ? '/' : `/${path.replace(/^\//, '')}`
  currentPath.value = normalizedPath
  try {
    const apiPath = normalizedPath === '/' ? undefined : normalizedPath.replace(/^\//, '')
    const res = await apiClient.get<ContentsResponse>(
      `/repos/${selectedRepoId.value}/archives/${encodeURIComponent(selectedArchive.value.name)}/contents`,
      { params: apiPath ? { path: apiPath } : {} },
    )
    const { index_status, entries } = res.data
    if (index_status === 'done' || index_status === 'failed') {
      indexing.value = false
      contents.value = entries.filter((e) => e.path !== '.' && e.path !== '..')
    } else {
      indexing.value = true
      contents.value = []
      startPolling(selectedArchive.value.name, path)
    }
  } catch (e: unknown) {
    contentsError.value = extractError(e)
  } finally {
    contentsLoading.value = false
  }
}

function navigateTo(path: string): void {
  loadContents(path)
}

function entryName(entry: ContentEntryResponse): string {
  return entry.path.split('/').pop() ?? entry.path
}

function downloadEntry(entry: ContentEntryResponse): void {
  if (selectedRepoId.value === null || !selectedArchive.value) return
  const archiveName = encodeURIComponent(selectedArchive.value.name)
  const encodedPath = encodeURIComponent(entry.path)
  const isDir = entry.type === 'd'
  const url = isDir
    ? `/api/repos/${selectedRepoId.value}/archives/${archiveName}/export?path=${encodedPath}`
    : `/api/repos/${selectedRepoId.value}/archives/${archiveName}/extract?path=${encodedPath}`
  const a = document.createElement('a')
  a.href = url
  a.download = isDir ? `${entryName(entry)}.tar.lz4` : entryName(entry)
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
}

const showRestoreWizard = ref(false)
const showArchiveDiff = ref(false)

const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showPassphraseDialog, () => {
  showPassphraseDialog.value = false
})

async function revealPassphrase(): Promise<void> {
  if (selectedRepoId.value === null) return
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(
      `/repos/${selectedRepoId.value}/passphrase`,
    )
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

onMounted(loadRepos)
</script>

<template>
  <div class="archives-view">
    <div class="page-header">
      <h1 class="page-title">Archives</h1>
    </div>

    <div
      v-if="reposLoading"
      class="state-msg"
    >
      Loading repositories...
    </div>
    <div
      v-else-if="reposError"
      class="state-msg state-error"
    >
      {{ reposError }}
    </div>
    <template v-else>
      <div class="repo-selector">
        <label class="selector-label">Repository</label>
        <select
          v-model="selectedRepoId"
          class="select-input"
          @change="onRepoChange"
        >
          <option
            :value="null"
            disabled
          >
            — select a repository —
          </option>
          <option
            v-for="repo in repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.hostname }} / {{ repo.target_name }}
          </option>
        </select>
        <span
          v-if="repos.length === 0"
          class="muted-hint"
          >No repositories configured yet.</span
        >
        <button
          v-if="selectedRepoId !== null"
          class="btn btn-sm btn-ghost passphrase-btn"
          :disabled="passphraseLoading"
          @click="revealPassphrase"
        >
          {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
        </button>
      </div>

      <div
        v-if="selectedRepoId !== null"
        class="main-layout"
      >
        <!-- Archive list -->
        <div class="panel archives-panel">
          <div class="panel-header">
            <span class="panel-title">Archives</span>
            <div class="panel-actions">
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archives.length < 1"
                @click="showRestoreWizard = true"
              >
                Restore
              </button>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archives.length < 2"
                @click="showArchiveDiff = true"
              >
                Diff
              </button>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archivesLoading"
                @click="loadArchives"
              >
                {{ archivesLoading ? '...' : '&#8635;' }}
              </button>
            </div>
          </div>

          <div
            v-if="archivesLoading"
            class="state-msg"
          >
            <span class="spinner" />
            Loading archives...
          </div>
          <div
            v-else-if="archivesError"
            class="state-msg state-error"
          >
            {{ archivesError }}
          </div>
          <div
            v-else-if="archives.length === 0"
            class="state-msg"
          >
            No archives found.
          </div>
          <DataTable
            v-else
            v-model:filters="archiveFilters"
            :value="sortedArchives"
            :row-class="
              (data: ArchiveEntry) =>
                selectedArchive?.name === data.name ? 'selected clickable' : 'clickable'
            "
            filter-display="row"
            table-class="data-table"
            @row-click="(e: { data: ArchiveEntry }) => selectArchive(e.data)"
          >
            <Column
              field="name"
              header="Name"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-mono">{{ data.name }}</span>
              </template>
            </Column>
            <Column
              field="start"
              header="Date"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-date">{{ formatDate(data.start) }}</span>
              </template>
            </Column>
            <Column
              field="hostname"
              header="Host"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <BaseHostLink
                  v-if="data.matched === true && data.agent_hostname"
                  :hostname="data.agent_hostname"
                  class="host-link"
                  @click.stop
                />
                <BaseHostLink
                  v-else-if="data.matched !== true"
                  :hostname="data.hostname"
                  class="unmatched-host-link"
                  @click.stop
                />
                <span
                  v-else
                  class="td-host"
                  >{{ data.hostname }}</span
                >
              </template>
            </Column>
            <Column
              field="matched"
              header=""
              style="width: 3rem"
            >
              <template #body="{ data }">
                <span
                  v-if="data.matched === true"
                  class="match-icon match-ok"
                  title="Matched"
                  >&#10003;</span
                >
                <span
                  v-else-if="data.matched !== true"
                  class="match-icon match-warn"
                  title="Unmatched"
                  >&#9888;</span
                >
              </template>
            </Column>
            <Column
              field="original_size"
              header="Size"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-size">{{ formatBytes(data.original_size) }}</span>
              </template>
            </Column>
          </DataTable>
        </div>

        <!-- File browser -->
        <div
          v-if="selectedArchive"
          class="panel browser-panel"
        >
          <div class="panel-header">
            <span class="panel-title">Files — {{ selectedArchive.name }}</span>
          </div>

          <div class="breadcrumb">
            <button
              v-for="(seg, i) in breadcrumbs"
              :key="seg.path"
              class="crumb"
              :class="{ 'crumb-last': i === breadcrumbs.length - 1 }"
              @click="navigateTo(seg.path)"
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
            class="state-msg"
          >
            <BaseSpinner size="sm" />
            Indexing archive contents — this only happens once…
          </div>
          <div
            v-else-if="contentsError"
            class="state-msg state-error"
          >
            {{ contentsError }}
          </div>
          <div
            v-else-if="contents.length === 0"
            class="state-msg"
          >
            Empty directory.
          </div>
          <DataTable
            v-else
            v-model:filters="browserFilters"
            :value="browserEntries"
            :row-class="(data: DisplayEntry) => (data.isDir ? 'clickable' : '')"
            filter-display="row"
            table-class="data-table"
            @row-click="
              (e: { data: DisplayEntry }) =>
                e.data.isDir && e.data.displayName !== '.' && navigateTo(e.data.path)
            "
          >
            <Column
              field="displayName"
              header="Name"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter name..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-name">
                  <Folder
                    v-if="data.isDir"
                    :size="16"
                    class="entry-icon"
                  />
                  <File
                    v-else
                    :size="16"
                    class="entry-icon"
                  />
                  {{ data.displayName }}
                </span>
              </template>
            </Column>
            <Column
              field="size"
              header="Size"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter size..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-size">{{ data.isDir ? '—' : formatBytes(data.size) }}</span>
              </template>
            </Column>
            <Column
              field="mtime"
              header="Modified"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter date..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-date">{{ formatDate(data.mtime) }}</span>
              </template>
            </Column>
            <Column header="">
              <template #body="{ data }">
                <span class="td-action">
                  <button
                    class="btn btn-sm btn-ghost"
                    :title="data.isDir ? 'Download as .tar.lz4' : 'Download'"
                    @click.stop="downloadEntry(data)"
                  >
                    <Download :size="14" />
                  </button>
                </span>
              </template>
            </Column>
          </DataTable>
        </div>

        <div
          v-else
          class="panel browser-panel empty-browser"
        >
          <span class="muted">Select an archive to browse its contents.</span>
        </div>
      </div>

      <FileSearch
        :repo-id="selectedRepoId"
        :archives="archives.map((a) => ({ name: a.name }))"
      />
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
    <!-- Restore Wizard -->
    <RestoreWizard
      :open="showRestoreWizard"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showRestoreWizard = false"
    />

    <!-- Archive Diff -->
    <ArchiveDiff
      :open="showArchiveDiff"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showArchiveDiff = false"
    />
  </div>
</template>

<style scoped>
.archives-view {
  max-width: 1300px;
  color: var(--text-primary);
}

.state-msg {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 1.5rem;
  color: var(--text-muted);
  font-size: 0.875rem;
}

.state-error {
  color: var(--danger);
}

.muted {
  color: var(--text-muted);
}

.repo-selector {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.selector-label {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.select-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.55rem 0.75rem;
  font-size: 0.875rem;
  min-width: 280px;
  transition: border-color 0.15s;
}

.select-input:focus {
  outline: none;
  border-color: var(--accent);
}

.muted-hint {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.main-layout {
  display: grid;
  grid-template-columns: 480px 1fr;
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

.panel-actions {
  display: flex;
  gap: 0.25rem;
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

.host-link {
  font-size: 0.8rem;
  color: var(--accent);
  text-decoration: none;
}

.host-link:hover {
  text-decoration: underline;
}

.unmatched-host-link {
  font-size: 0.8rem;
  color: var(--warning);
  text-decoration: none;
}

.unmatched-host-link:hover {
  text-decoration: underline;
}

.match-icon {
  font-size: 0.9rem;
}

.match-ok {
  color: var(--success, #22c55e);
}

.match-warn {
  color: var(--warning);
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

.filter-input {
  width: 100%;
  background: var(--bg-input, var(--bg-card));
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.35rem 0.5rem;
  font-size: 0.78rem;
}

.filter-input:focus {
  outline: none;
  border-color: var(--accent);
}

.entry-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.breadcrumb {
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

/* Buttons */

.passphrase-btn {
  margin-left: auto;
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
</style>
