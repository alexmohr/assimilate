<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { Search } from '@lucide/vue'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { apiClient } from '../api/client'
import { formatBytes, formatDate } from '../utils/format'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'

interface ArchiveOption {
  name: string
}

interface SearchResultItem {
  path: string
  size: number
  mtime: string
  type: string
  archive_name?: string
}

interface SingleArchiveResponse {
  items: SearchResultItem[]
  total: number
  limit: number
  offset: number
}

interface Props {
  repoId: number | null
  archives: ArchiveOption[]
}

const props = defineProps<Props>()

type SearchMode = 'single' | 'cross'

const modeOptions: SegmentedOption<SearchMode>[] = [
  { value: 'cross', label: 'All archives' },
  { value: 'single', label: 'Single archive' },
]

const searchMode = ref<SearchMode>('cross')
const selectedArchiveName = ref<string | null>(null)
const pattern = ref('')
const maxArchives = ref(20)
const { loading, error, run } = useAsyncAction()
const results = ref<SearchResultItem[]>([])
const totalResults = ref(0)
const hasSearched = ref(false)

const canSearch = computed<boolean>(() => {
  if (!pattern.value.trim()) return false
  if (searchMode.value === 'single' && !selectedArchiveName.value) return false
  return true
})

async function doSearch(): Promise<void> {
  if (!canSearch.value || props.repoId === null) return
  results.value = []
  totalResults.value = 0
  hasSearched.value = true

  await run(async () => {
    if (searchMode.value === 'single' && selectedArchiveName.value) {
      const res = await apiClient.get<SingleArchiveResponse>(
        `/repos/${props.repoId}/archives/${encodeURIComponent(selectedArchiveName.value)}/search`,
        { params: { pattern: pattern.value, limit: 100, offset: 0 } },
      )
      results.value = res.data.items
      totalResults.value = res.data.total
    } else {
      const res = await apiClient.get<SearchResultItem[]>(`/repos/${props.repoId}/search`, {
        params: { pattern: pattern.value, max_archives: maxArchives.value },
      })
      results.value = res.data
      totalResults.value = res.data.length
    }
  })
}

function handleKeydown(event: KeyboardEvent): void {
  if (event.key === 'Enter' && canSearch.value) {
    doSearch()
  }
}
</script>

<template>
  <div class="file-search">
    <div class="search-header">
      <span class="search-title">File search</span>
    </div>

    <div class="search-controls">
      <BaseSegmented
        v-model="searchMode"
        :options="modeOptions"
        label="Search scope"
      />

      <div
        v-if="searchMode === 'single'"
        class="archive-select-row"
      >
        <label class="field-label">Archive</label>
        <select
          v-model="selectedArchiveName"
          class="input select-input select-input--md"
        >
          <option
            :value="null"
            disabled
          >
            — select archive —
          </option>
          <option
            v-for="archive in archives"
            :key="archive.name"
            :value="archive.name"
          >
            {{ archive.name }}
          </option>
        </select>
      </div>

      <div
        v-if="searchMode === 'cross'"
        class="max-archives-row"
      >
        <label class="field-label">Max archives</label>
        <input
          v-model.number="maxArchives"
          type="number"
          class="input input-sm max-archives-input"
          min="1"
          max="100"
        />
      </div>

      <div class="pattern-row">
        <label class="field-label">Pattern</label>
        <input
          v-model="pattern"
          type="text"
          class="input"
          placeholder="e.g. *.sql or home/**/*.conf"
          @keydown="handleKeydown"
        />
        <button
          class="btn btn-primary btn-sm"
          :disabled="!canSearch || loading"
          @click="doSearch"
        >
          Search
        </button>
      </div>
    </div>

    <div class="search-results">
      <BaseSpinner
        v-if="loading"
        size="sm"
      />

      <div
        v-else-if="error"
        class="state-msg state-msg--inline state-error"
      >
        {{ error }}
      </div>

      <EmptyState
        v-else-if="hasSearched && results.length === 0"
        :icon="Search"
        title="No files found"
        description="Try a different glob pattern."
      />

      <DataTable
        v-else-if="results.length > 0"
        :value="results"
        :rows="100"
        striped-rows
      >
        <Column
          v-if="searchMode === 'cross'"
          field="archive_name"
          header="Archive"
        >
          <template #body="{ data }">
            <span class="cell-mono">{{ (data as SearchResultItem).archive_name }}</span>
          </template>
        </Column>
        <Column
          field="path"
          header="Path"
        >
          <template #body="{ data }">
            <span class="cell-mono">{{ (data as SearchResultItem).path }}</span>
          </template>
        </Column>
        <Column
          field="size"
          header="Size"
        >
          <template #body="{ data }">
            <span class="cell-muted">{{ formatBytes((data as SearchResultItem).size) }}</span>
          </template>
        </Column>
        <Column
          field="mtime"
          header="Modified"
        >
          <template #body="{ data }">
            <span class="cell-muted">{{ formatDate((data as SearchResultItem).mtime) }}</span>
          </template>
        </Column>
        <Column
          field="type"
          header="Type"
        >
          <template #body="{ data }">
            <span class="cell-muted">{{ (data as SearchResultItem).type }}</span>
          </template>
        </Column>
      </DataTable>

      <div
        v-if="results.length > 0"
        class="results-summary"
      >
        {{ totalResults }} result{{ totalResults === 1 ? '' : 's' }} found
      </div>
    </div>
  </div>
</template>

<style scoped>
.file-search {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.search-header {
  display: flex;
  align-items: center;
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.search-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.search-controls {
  padding: var(--space-6) var(--space-7);
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.archive-select-row,
.max-archives-row,
.pattern-row {
  display: flex;
  align-items: center;
  gap: var(--space-5);
}

.field-label {
  white-space: nowrap;
  min-width: 90px;
}

.max-archives-input {
  width: 80px;
}

.pattern-row .input {
  flex: 1;
}

.search-results {
  padding: 0 var(--space-7) var(--space-7);
}

.results-summary {
  margin-top: var(--space-5);
  font-size: var(--fs-sm);
  color: var(--text-muted);
}
</style>
