<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import { formatBytes, formatDate } from '../utils/format'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { Folder, File, Download } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import { useArchiveBrowser, type ArchiveEntry } from '../composables/useArchiveBrowser'

interface DisplayEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
  displayName: string
  isDir: boolean
  displaySize: string
  displayMtime: string
}

const CURRENT_DIR_MARKER = '.'

const props = defineProps<{
  repoId: number | null
  archiveName: string | null
}>()

const repoIdRef = computed(() => props.repoId ?? 0)
const browser = useArchiveBrowser(repoIdRef)

const breadcrumbs = browser.breadcrumbs
const contents = browser.contents
const contentsLoading = browser.contentsLoading
const contentsError = browser.contentsError
const indexing = browser.indexing
const navigateTo = browser.navigateTo
const downloadEntry = browser.downloadEntry

const browserFilters = ref({
  displayName: { value: '', matchMode: FilterMatchMode.CONTAINS },
  displaySize: { value: '', matchMode: FilterMatchMode.CONTAINS },
  displayMtime: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

const browserEntries = computed<DisplayEntry[]>(() => [
  ...browser.dirs.value.map((d) => ({
    type: d.type,
    path: d.path,
    size: Number(d.size),
    mtime: d.mtime,
    mode: d.mode,
    displayName: d.displayName,
    isDir: true,
    displaySize: '-',
    displayMtime: '',
  })),
  ...browser.files.value.map((f) => ({
    type: f.type,
    path: f.path,
    size: Number(f.size),
    mtime: f.mtime,
    mode: f.mode,
    displayName: browser.entryName(f),
    isDir: false,
    displaySize: formatBytes(Number(f.size)),
    displayMtime: formatDate(f.mtime),
  })),
])

function archivePlaceholder(name: string): ArchiveEntry {
  return {
    name,
    start: '',
    hostname: '',
    comment: '',
    original_size: 0,
    deduplicated_size: 0,
    matched: null,
    agent_hostname: null,
  }
}

function handleRowClick(entry: DisplayEntry): void {
  if (entry.isDir && entry.displayName !== CURRENT_DIR_MARKER) {
    navigateTo(entry.path)
  }
}

function reset(): void {
  browser.stopPolling()
  browser.selectedArchive.value = null
  browser.currentPath.value = '/'
  browser.contents.value = []
  browser.contentsError.value = null
  browser.indexing.value = false
  browser.contentsLoading.value = false
}

watch(
  () => props.archiveName,
  (name) => {
    if (name) {
      browser.selectArchive(archivePlaceholder(name))
    } else {
      reset()
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  browser.stopPolling()
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
        <span class="browser-title">Files -- {{ archiveName }}</span>
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
        Indexing archive contents -- this only happens once...
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
        @row-click="(e: { data: DisplayEntry }) => handleRowClick(e.data)"
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
          field="displaySize"
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
            <span class="td-size">{{ data.isDir ? '-' : data.displaySize }}</span>
          </template>
        </Column>
        <Column
          field="displayMtime"
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
            <span class="td-date">{{ data.displayMtime }}</span>
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
