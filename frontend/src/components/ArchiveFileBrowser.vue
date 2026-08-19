<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, watch, onBeforeUnmount } from 'vue'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { Folder, File, Download, RotateCcw, Trash2 } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import {
  useArchiveBrowser,
  type ArchiveEntry,
  type ContentEntry,
  type DisplayEntry,
} from '../composables/useArchiveBrowser'

const CURRENT_DIR_MARKER = '.'

const props = withDefaults(
  defineProps<{
    repoId: number | null
    archive: ArchiveEntry | null
    isAdmin?: boolean
    // Whether `archive` already has a delete in flight - deletion is async
    // (the request just enqueues the borg job), so without this the delete
    // button stays clickable for an archive that's already being removed.
    deleting?: boolean
  }>(),
  {
    isAdmin: false,
    deleting: false,
  },
)

const emit = defineEmits<{
  'delete-archive': [archive: ArchiveEntry]
}>()

const { success: toastSuccess, error: toastError } = useToast()

const repoIdRef = computed(() => props.repoId ?? 0)
const browser = useArchiveBrowser(repoIdRef)

const breadcrumbs = browser.breadcrumbs
const contents = browser.contents
const contentsLoading = browser.contentsLoading
const contentsError = browser.contentsError
const indexing = browser.indexing
const navigateTo = browser.navigateTo
const downloadEntry = browser.downloadEntry
const browserEntries = browser.browserEntries
const browserFilters = browser.browserFilters

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
  () => props.archive,
  (archive) => {
    if (archive) {
      browser.selectArchive(archive)
    } else {
      reset()
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  browser.stopPolling()
})

async function handleRestore(entry: ContentEntry): Promise<void> {
  try {
    const restored = await browser.restoreEntry(entry)
    if (!restored) return
    toastSuccess(entry.path.length > 0 ? `Restored ${entry.path}.` : 'Restored the whole archive.')
  } catch (e: unknown) {
    toastError(extractError(e))
  }
}

function handleDeleteWholeArchive(): void {
  if (props.archive && !props.deleting) emit('delete-archive', props.archive)
}
</script>

<template>
  <div class="archive-file-browser">
    <div
      v-if="!archive"
      class="browser-placeholder"
    >
      Select an archive to browse its contents.
    </div>

    <template v-else>
      <div class="browser-header">
        <span class="browser-title">Files -- {{ archive.name }}</span>
      </div>

      <div
        v-if="archive.start"
        class="archive-meta-bar"
      >
        <span class="archive-meta-item">
          <span class="archive-meta-label">Date</span>
          <span class="archive-meta-value">{{ formatDate(archive.start) }}</span>
        </span>
        <span class="archive-meta-sep" />
        <span class="archive-meta-item">
          <span class="archive-meta-label">Original</span>
          <span class="archive-meta-value">{{ formatBytes(archive.original_size) }}</span>
        </span>
        <span class="archive-meta-sep" />
        <span class="archive-meta-item">
          <span class="archive-meta-label">Dedup</span>
          <span class="archive-meta-value">{{ formatBytes(archive.deduplicated_size) }}</span>
        </span>
      </div>

      <div class="path-crumbs">
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
        class="state-msg state-msg--inline"
      >
        <BaseSpinner size="sm" />
        Indexing archive contents -- this only happens once...
      </div>
      <div
        v-else-if="contentsError"
        class="error-banner"
      >
        {{ contentsError }}
      </div>
      <div
        v-else-if="contents.length === 0"
        class="state-msg state-msg--inline"
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
              class="input filter-input"
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
              class="input filter-input"
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
              class="input filter-input"
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
          style="width: 7rem"
        >
          <template #body="{ data }">
            <span class="td-action">
              <button
                class="btn btn-sm btn-ghost"
                :title="
                  data.isDir
                    ? data.path
                      ? 'Download as .tar.lz4'
                      : 'Download whole archive'
                    : 'Download'
                "
                @click.stop="downloadEntry(data)"
              >
                <Download :size="14" />
              </button>
              <button
                v-if="isAdmin"
                class="btn btn-sm btn-ghost"
                :title="data.path ? 'Restore to host' : 'Restore whole archive to host'"
                @click.stop="handleRestore(data)"
              >
                <RotateCcw :size="14" />
              </button>
              <button
                v-if="isAdmin && data.displayName === '.' && data.path.length === 0"
                class="btn btn-sm btn-ghost"
                :disabled="deleting"
                :title="deleting ? 'Deletion in progress' : 'Delete whole archive'"
                @click.stop="handleDeleteWholeArchive"
              >
                <BaseSpinner
                  v-if="deleting"
                  size="sm"
                />
                <Trash2
                  v-else
                  :size="14"
                />
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

.browser-placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  font-size: var(--fs-base);
  color: var(--text-muted);
}

.browser-header {
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.browser-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.archive-meta-bar {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.archive-meta-item {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}

.archive-meta-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.archive-meta-value {
  font-size: var(--fs-sm);
  color: var(--text-primary);
}

.archive-meta-sep {
  width: 1px;
  height: 1rem;
  background: var(--border);
}

:deep(.browser-table) {
  table-layout: fixed;
}

:deep(.data-table) {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-base);
}

/* No border-bottom here: the global PrimeVue passthrough config
   (primevue-pt.ts) already draws row separators via `tbody: divide-y` and
   `headerRow: border-b`. Adding a second border on th/td doubled the line. */
:deep(.data-table th) {
  text-align: left;
  padding: var(--space-4) var(--space-6);
  color: var(--text-muted);
  font-weight: 600;
  font-size: var(--fs-xs);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

:deep(.data-table td) {
  padding: var(--space-4) var(--space-6);
  color: var(--text-secondary);
}

:deep(.data-table tr.clickable) {
  cursor: pointer;
  transition: background var(--duration-fast);
}

:deep(.data-table tr.clickable:hover) {
  background: var(--bg-hover);
}

@media (max-width: 640px) {
  :deep(.browser-table th:nth-child(3)),
  :deep(.browser-table td:nth-child(3)) {
    display: none;
  }

  :deep(.browser-table th:nth-child(2)),
  :deep(.browser-table td:nth-child(2)) {
    width: 4rem;
  }

  .td-name {
    align-items: flex-start;
  }

  .name-text {
    white-space: normal;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .td-size {
    white-space: normal;
  }
}
</style>
