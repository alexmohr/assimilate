<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { Folder, File, Download } from '@lucide/vue'
import { apiClient } from '../api/client'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'
import type { ContentsResponse, ContentEntryResponse } from '../types/generated'

const DIRECTORY_ENTRY_TYPE = 'd'
const ROOT_PATH = '/'
const CURRENT_DIR_MARKER = '.'
const PARENT_DIR_MARKER = '..'

type ArchiveIndexStatus = 'pending' | 'indexing' | 'done' | 'failed'

function normalizeIndexStatus(status: string): ArchiveIndexStatus {
  if (status === 'done') return 'done'
  if (status === 'failed') return 'failed'
  if (status === 'indexing') return 'indexing'
  return 'pending'
}

interface BreadcrumbSegment {
  label: string
  path: string
}

interface DisplayEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
  displayName: string
  isDir: boolean
}

const props = defineProps<{
  repoId: number | null
  archiveName: string | null
}>()

const currentPath = ref(ROOT_PATH)
const contents = ref<ContentEntryResponse[]>([])
const contentsLoading = ref(false)
const contentsError = ref<string | null>(null)
const indexing = ref(false)

let pollTimer: ReturnType<typeof setInterval> | null = null

function stopPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling(archiveName: string, pendingPath: string): void {
  if (props.repoId === null) return
  stopPolling()
  pollTimer = setInterval(async () => {
    if (props.repoId === null) return
    try {
      const res = await apiClient.get<{ status: string; error?: string }>(
        `/repos/${props.repoId}/archives/${encodeURIComponent(archiveName)}/index-status`,
      )
      const status = normalizeIndexStatus(res.data.status)
      if (status === 'done') {
        stopPolling()
        indexing.value = false
        await loadContents(pendingPath)
      } else if (status === 'failed') {
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

const breadcrumbs = computed<BreadcrumbSegment[]>(() => {
  const path = currentPath.value
  if (path === ROOT_PATH) return [{ label: '/', path: ROOT_PATH }]
  const parts = path.replace(/^\//, '').split('/')
  const segments: BreadcrumbSegment[] = [{ label: '~', path: '/' }]
  let accumulated = ''
  for (const part of parts) {
    accumulated += `/${part}`
    segments.push({ label: part, path: accumulated })
  }
  return segments
})

const browserFilters = ref({
  displayName: { value: '', matchMode: FilterMatchMode.CONTAINS },
  size: { value: '', matchMode: FilterMatchMode.CONTAINS },
  mtime: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

const browserEntries = computed<DisplayEntry[]>(() => {
  const currentDir = currentPath.value.replace(/^\//, '')
  const dirList = contents.value
    .filter((e) => e.type === DIRECTORY_ENTRY_TYPE && e.path !== currentDir)
    .sort((a, b) => a.path.localeCompare(b.path))
  const fileList = contents.value
    .filter((e) => e.type !== DIRECTORY_ENTRY_TYPE)
    .sort((a, b) => a.path.localeCompare(b.path))

  const entries: DisplayEntry[] = []

  const currentEntry = contents.value.find(
    (e) => e.type === DIRECTORY_ENTRY_TYPE && e.path === currentDir,
  )
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
  } else if (currentPath.value === ROOT_PATH) {
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

  if (currentPath.value !== ROOT_PATH) {
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
      isDir: e.type === DIRECTORY_ENTRY_TYPE,
    })),
  ]
})

async function loadContents(path: string): Promise<void> {
  if (props.repoId === null || props.archiveName === null) return
  contentsLoading.value = true
  contentsError.value = null
  const normalizedPath = path === ROOT_PATH ? ROOT_PATH : `/${path.replace(/^\//, '')}`
  currentPath.value = normalizedPath
  try {
    const apiPath = normalizedPath === ROOT_PATH ? undefined : normalizedPath.replace(/^\//, '')
    const res = await apiClient.get<ContentsResponse>(
      `/repos/${props.repoId}/archives/${encodeURIComponent(props.archiveName)}/contents`,
      { params: apiPath ? { path: apiPath } : {} },
    )
    const { index_status, entries } = res.data
    const status = normalizeIndexStatus(index_status)
    if (status === 'done' || status === 'failed') {
      indexing.value = false
      contents.value = entries.filter(
        (e) => e.path !== CURRENT_DIR_MARKER && e.path !== PARENT_DIR_MARKER,
      )
    } else {
      indexing.value = true
      contents.value = []
      startPolling(props.archiveName, path)
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
  if (props.repoId === null || props.archiveName === null) return
  const archiveName = encodeURIComponent(props.archiveName)
  const encodedPath = encodeURIComponent(entry.path)
  const isDir = entry.type === DIRECTORY_ENTRY_TYPE
  const url = isDir
    ? `/api/repos/${props.repoId}/archives/${archiveName}/export?path=${encodedPath}`
    : `/api/repos/${props.repoId}/archives/${archiveName}/extract?path=${encodedPath}`
  const a = document.createElement('a')
  a.href = url
  a.download = isDir ? `${entryName(entry)}.tar.lz4` : entryName(entry)
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
}

function reset(): void {
  stopPolling()
  indexing.value = false
  currentPath.value = ROOT_PATH
  contents.value = []
  contentsError.value = null
  contentsLoading.value = false
}

watch(
  () => props.archiveName,
  (newName) => {
    reset()
    if (newName) {
      loadContents(ROOT_PATH)
    }
  },
)

onBeforeUnmount(() => {
  stopPolling()
})
</script>

<template>
  <div class="archive-file-browser">
    <div
      v-if="!archiveName"
      class="empty-state"
    >
      Select an archive to browse its contents.
    </div>

    <template v-else>
      <div class="browser-header">
        <span class="browser-title">Files — {{ archiveName }}</span>
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
        table-class="data-table browser-table"
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
            <span
              class="td-name"
              :title="data.displayName"
            >
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
              <span class="name-text">{{ data.displayName }}</span>
            </span>
          </template>
        </Column>
        <Column
          field="size"
          header="Size"
          :sortable="true"
          :show-filter-menu="false"
          style="width: 6rem"
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
          style="width: 10rem"
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
        <Column
          header=""
          style="width: 3rem"
        >
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
    </template>
  </div>
</template>

<style scoped>
.archive-file-browser {
  color: var(--text-primary);
}

.empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  font-size: 0.875rem;
  color: var(--text-muted);
}

.browser-header {
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.browser-title {
  font-size: 0.8rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
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

.browser-table {
  table-layout: fixed;
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

.td-name {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
  font-family: var(--mono);
  font-size: 0.82rem;
}

.name-text {
  overflow: hidden;
  min-width: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.entry-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.td-size {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.td-date {
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
</style>
