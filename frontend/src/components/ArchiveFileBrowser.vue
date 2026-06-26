<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { Folder, File } from '@lucide/vue'
import { apiClient } from '../api/client'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'

interface ContentEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
}

interface DisplayEntry extends ContentEntry {
  displayName: string
  isDir: boolean
}

interface BreadcrumbSegment {
  label: string
  path: string
}

interface ContentsResponse {
  index_status: 'pending' | 'indexing' | 'done' | 'failed'
  entries: ContentEntry[]
}

const props = defineProps<{
  repoId: number | null
  archiveName: string | null
}>()

const currentPath = ref('/')
const contents = ref<ContentEntry[]>([])
const contentsLoading = ref(false)
const contentsError = ref<string | null>(null)
const indexing = ref(false)

let pollTimer: ReturnType<typeof setInterval> | null = null

const browserFilters = ref({
  displayName: { value: '', matchMode: FilterMatchMode.CONTAINS },
  size: { value: '', matchMode: FilterMatchMode.CONTAINS },
  mtime: { value: '', matchMode: FilterMatchMode.CONTAINS },
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
    entries.push({ ...currentEntry, displayName: '.', isDir: true })
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
      ...e,
      displayName: e.path.split('/').pop() ?? e.path,
      isDir: e.type === 'd',
    })),
  ]
})

function stopPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

function startPolling(archiveName: string, pendingPath: string): void {
  stopPolling()
  pollTimer = setInterval(async () => {
    if (props.repoId === null) return
    try {
      const res = await apiClient.get<{ status: string; error?: string }>(
        `/repos/${props.repoId}/archives/${encodeURIComponent(archiveName)}/index-status`,
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

async function loadContents(path: string): Promise<void> {
  if (props.repoId === null || !props.archiveName) return
  contentsLoading.value = true
  contentsError.value = null
  const normalizedPath = path === '/' ? '/' : `/${path.replace(/^\//, '')}`
  currentPath.value = normalizedPath
  try {
    const apiPath = normalizedPath === '/' ? undefined : normalizedPath.replace(/^\//, '')
    const res = await apiClient.get<ContentsResponse>(
      `/repos/${props.repoId}/archives/${encodeURIComponent(props.archiveName)}/contents`,
      { params: apiPath ? { path: apiPath } : {} },
    )
    const { index_status, entries } = res.data
    if (index_status === 'done' || index_status === 'failed') {
      indexing.value = false
      contents.value = entries.filter((e) => e.path !== '.' && e.path !== '..')
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
  loadContents(path).catch(() => undefined)
}

function entryName(entry: ContentEntry): string {
  return entry.path.split('/').pop() ?? entry.path
}

function downloadEntry(entry: ContentEntry): void {
  if (props.repoId === null || !props.archiveName) return
  const archiveName = encodeURIComponent(props.archiveName)
  const encodedPath = encodeURIComponent(entry.path)
  const isDir = entry.type === 'd'
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

watch(
  () => props.archiveName,
  (name) => {
    stopPolling()
    indexing.value = false
    currentPath.value = '/'
    contents.value = []
    contentsError.value = null
    if (name && props.repoId !== null) loadContents('/').catch(() => undefined)
  },
)

onBeforeUnmount(stopPolling)
</script>

<template>
  <div
    v-if="archiveName"
    class="archive-browser-panel"
  >
    <div class="archive-browser-header">
      <span class="archive-browser-title">{{ archiveName }}</span>
    </div>

    <div class="archive-breadcrumb">
      <button
        v-for="(seg, i) in breadcrumbs"
        :key="seg.path"
        class="archive-crumb"
        :class="{ 'archive-crumb-last': i === breadcrumbs.length - 1 }"
        @click="navigateTo(seg.path)"
      >
        {{ seg.label }}
      </button>
    </div>

    <div
      v-if="contentsLoading"
      class="archive-state"
    >
      <BaseSpinner size="sm" />
    </div>
    <div
      v-else-if="indexing"
      class="archive-state"
    >
      <BaseSpinner size="sm" />
      Indexing archive — this only happens once…
    </div>
    <div
      v-else-if="contentsError"
      class="archive-state archive-state-error"
    >
      {{ contentsError }}
    </div>
    <DataTable
      v-else
      v-model:filters="browserFilters"
      :value="browserEntries"
      :row-class="(data: DisplayEntry) => (data.isDir ? 'archive-dir-row' : '')"
      filter-display="row"
      table-class="archive-browser-table"
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
            placeholder="Filter name…"
            @input="filterCallback()"
          />
        </template>
        <template #body="{ data }">
          <span class="cell-name">
            <Folder
              v-if="data.isDir"
              :size="14"
              class="entry-icon"
            />
            <File
              v-else
              :size="14"
              class="entry-icon"
            />
            <span class="cell-mono">{{ data.displayName }}</span>
          </span>
        </template>
      </Column>
      <Column
        field="size"
        header="Size"
        :sortable="true"
        :show-filter-menu="false"
      >
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
        <template #body="{ data }">
          <span class="td-date">{{ data.mtime ? formatDate(data.mtime) : '—' }}</span>
        </template>
      </Column>
      <Column header="">
        <template #body="{ data }">
          <span class="cell-action">
            <button
              v-if="data.displayName !== '.' && data.displayName !== '..'"
              class="btn btn-sm btn-ghost"
              :title="data.isDir ? 'Download as .tar.lz4' : 'Download'"
              @click.stop="downloadEntry(data)"
            >
              ↓
            </button>
          </span>
        </template>
      </Column>
    </DataTable>
  </div>

  <div
    v-else
    class="archive-browser-panel archive-browser-empty"
  >
    Select an archive to browse its contents.
  </div>
</template>

<style scoped>
.archive-browser-panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.archive-browser-header {
  padding: 0.6rem 0.875rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.archive-browser-title {
  font-family: var(--mono);
  font-size: 0.78rem;
  color: var(--text-secondary);
  font-weight: 500;
}

.archive-breadcrumb {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.1rem;
  padding: 0.4rem 0.75rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.archive-crumb {
  background: none;
  border: none;
  color: var(--accent);
  cursor: pointer;
  font-size: 0.78rem;
  font-family: var(--mono);
  padding: 0.1rem 0.3rem;
  border-radius: var(--radius-sm);
  transition:
    background 0.1s,
    color 0.1s;
}

.archive-crumb:hover {
  background: var(--accent-subtle);
}

.archive-crumb-last {
  color: var(--text-primary);
  cursor: default;
}

.archive-crumb-last:hover {
  background: none;
}

.archive-crumb:not(.archive-crumb-last)::after {
  content: ' /';
  color: var(--text-muted);
  margin-left: 0.15rem;
}

.archive-state {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 1rem 0.875rem;
  color: var(--text-muted);
  font-size: 0.875rem;
}

.archive-state-error {
  color: var(--danger);
}

.archive-browser-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 160px;
  color: var(--text-muted);
  font-size: 0.875rem;
}

.cell-name {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.cell-mono {
  font-family: var(--mono);
  font-size: 0.78rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 160px;
}

.cell-action {
  text-align: right;
  padding-right: 0.5rem;
}

.entry-icon {
  color: var(--text-muted);
  flex-shrink: 0;
}

.td-size,
.td-date {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
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

:deep(.archive-browser-table) {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

:deep(.archive-browser-table th) {
  text-align: left;
  padding: 0.5rem 1rem;
  color: var(--text-muted);
  font-weight: 600;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid var(--border);
}

:deep(.archive-browser-table td) {
  padding: 0.6rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

:deep(.archive-browser-table tr:last-child td) {
  border-bottom: none;
}

:deep(.archive-dir-row) {
  cursor: pointer;
}

:deep(.archive-dir-row:hover td) {
  background: var(--bg-hover);
}
</style>
