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
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'

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

const searchMode = ref<'single' | 'cross'>('cross')
const selectedArchiveName = ref<string | null>(null)
const pattern = ref('')
const maxArchives = ref(20)
const loading = ref(false)
const error = ref<string | null>(null)
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
  loading.value = true
  error.value = null
  results.value = []
  totalResults.value = 0
  hasSearched.value = true

  try {
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
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
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
      <span class="search-title">File Search</span>
    </div>

    <div class="search-controls">
      <div class="mode-toggle">
        <button
          class="mode-btn"
          :class="{ active: searchMode === 'cross' }"
          @click="searchMode = 'cross'"
        >
          All archives
        </button>
        <button
          class="mode-btn"
          :class="{ active: searchMode === 'single' }"
          @click="searchMode = 'single'"
        >
          Single archive
        </button>
      </div>

      <div
        v-if="searchMode === 'single'"
        class="archive-select-row"
      >
        <label class="field-label">Archive</label>
        <select
          v-model="selectedArchiveName"
          class="select-input"
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
          class="input input-sm"
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
        class="state-msg state-error"
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
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.search-title {
  font-size: 0.8rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.search-controls {
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.mode-toggle {
  display: flex;
  gap: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
  width: fit-content;
}

.mode-btn {
  background: var(--bg-base);
  border: none;
  padding: 0.4rem 0.85rem;
  font-size: 0.8rem;
  color: var(--text-secondary);
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s;
}

.mode-btn:not(:last-child) {
  border-right: 1px solid var(--border);
}

.mode-btn.active {
  background: var(--accent);
  color: var(--text-on-accent);
}

.archive-select-row,
.max-archives-row,
.pattern-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.field-label {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  min-width: 90px;
}

.select-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.45rem 0.65rem;
  font-size: 0.85rem;
  min-width: 220px;
}

.select-input:focus {
  outline: none;
  border-color: var(--accent);
}

.input-sm {
  width: 80px;
}

.pattern-row .input {
  flex: 1;
}

.search-results {
  padding: 0 1.25rem 1.25rem;
}

.state-msg {
  padding: 1rem 0;
  font-size: 0.875rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.cell-mono {
  font-family: var(--mono);
  font-size: 0.8rem;
}

.cell-muted {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.results-summary {
  margin-top: 0.75rem;
  font-size: 0.8rem;
  color: var(--text-muted);
}
</style>
