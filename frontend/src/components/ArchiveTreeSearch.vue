<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { File, Folder, FolderOpen, Search, SlidersHorizontal } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import type { ArchiveTreeSearchOptions } from '../api/archives'
import { useArchiveTreeSearch } from '../composables/useArchiveTreeSearch'
import { isDirectoryEntry, parentOf, type TickState } from '../utils/archiveSelection'
import { formatBytes, formatDate } from '../utils/format'
import { matchParts } from '../utils/textMatch'
import type { ContentEntryResponse as ContentEntry } from '../types/generated'

/** Which entries the search keeps: anything, only files, or only folders. */
type KindFilter = 'all' | 'file' | 'dir'

const KIND_OPTIONS: readonly SegmentedOption<KindFilter>[] = [
  { value: 'all', label: 'Any' },
  { value: 'file', label: 'Files' },
  { value: 'dir', label: 'Folders' },
]

const BYTES_PER_MIB = 1024 * 1024

/** How long typing pauses before the search runs. */
const SEARCH_DEBOUNCE_MS = 300

const props = defineProps<{
  repoId: number | null
  archiveName: string | null
  /** How the browser's selection ticks `path`. */
  tickState: (path: string) => TickState
  disabled?: boolean
}>()

const emit = defineEmits<{
  /** The checkbox of a match was clicked. */
  toggle: [entry: ContentEntry]
  /** Show the folder `path` (an archive path, `''` for the root) in the browser. */
  open: [path: string]
  /** Whether a search is entered, so the browser shows its results instead of the folder. */
  'update:active': [active: boolean]
}>()

const query = ref('')
const filtersOpen = ref(false)
const kind = ref<KindFilter>('all')
const minSizeMiB = ref<number | null>(null)
const maxSizeMiB = ref<number | null>(null)
const modifiedFrom = ref('')
const modifiedTo = ref('')

const search = useArchiveTreeSearch(
  () => props.repoId,
  () => props.archiveName,
)
const { entries, truncated, loading, error, notIndexed } = search

/** A number input left empty reads back as `''`, not `null`. */
function sizeOf(mib: number | null | ''): number | undefined {
  return typeof mib === 'number' ? Math.round(mib * BYTES_PER_MIB) : undefined
}

const activeFilterCount = computed(
  () =>
    Number(kind.value !== 'all') +
    Number(sizeOf(minSizeMiB.value) !== undefined) +
    Number(sizeOf(maxSizeMiB.value) !== undefined) +
    Number(modifiedFrom.value !== '') +
    Number(modifiedTo.value !== ''),
)

/** The search entered, or null while there is neither text nor a filter. */
const options = computed<ArchiveTreeSearchOptions | null>(() => {
  const text = query.value.trim()
  if (text === '' && activeFilterCount.value === 0) return null
  return {
    query: text === '' ? undefined : text,
    kind: kind.value === 'all' ? undefined : kind.value,
    minSize: sizeOf(minSizeMiB.value),
    maxSize: sizeOf(maxSizeMiB.value),
    modifiedFrom: modifiedFrom.value === '' ? undefined : modifiedFrom.value,
    modifiedTo: modifiedTo.value === '' ? undefined : modifiedTo.value,
  }
})

const active = computed(() => options.value !== null)
watch(active, (value) => emit('update:active', value))

let timer: ReturnType<typeof setTimeout> | null = null

function cancelTimer(): void {
  if (timer !== null) clearTimeout(timer)
  timer = null
}

watch(options, (next) => {
  cancelTimer()
  if (next === null) {
    void search.search(null)
    return
  }
  timer = setTimeout(() => {
    timer = null
    void search.search(next)
  }, SEARCH_DEBOUNCE_MS)
})

// Another archive starts with an empty search.
watch(
  () => `${props.repoId}/${props.archiveName}`,
  () => clearSearch(),
)

onBeforeUnmount(cancelTimer)

function resetFilters(): void {
  kind.value = 'all'
  minSizeMiB.value = null
  maxSizeMiB.value = null
  modifiedFrom.value = ''
  modifiedTo.value = ''
}

function clearSearch(): void {
  query.value = ''
  resetFilters()
}

/** Shows `entry` where it sits: a folder opened, a file in its folder. */
function reveal(entry: ContentEntry): void {
  const folder = isDirectoryEntry(entry) ? entry.path : parentOf(entry.path)
  clearSearch()
  emit('open', folder)
}
</script>

<template>
  <div>
    <div class="tree-search-bar">
      <div class="search-input-wrap tree-search-field">
        <Search
          :size="14"
          class="search-icon"
        />
        <input
          v-model="query"
          class="input search-input--icon"
          type="search"
          placeholder="Search this archive"
          aria-label="Search this archive"
          data-testid="tree-search"
        />
      </div>
      <button
        type="button"
        class="filter-toggle"
        :class="{ active: filtersOpen }"
        :aria-expanded="filtersOpen"
        data-testid="tree-search-filters"
        @click="filtersOpen = !filtersOpen"
      >
        <SlidersHorizontal :size="14" />
        Filters
        <span
          v-if="activeFilterCount > 0"
          class="filter-badge"
        />
      </button>
    </div>

    <div
      v-if="filtersOpen"
      class="filters tree-search-filters"
    >
      <div class="filter-row">
        <div class="filter-group">
          <span class="filter-label">Type</span>
          <BaseSegmented
            v-model="kind"
            :options="KIND_OPTIONS"
            label="Entry type"
          />
        </div>
        <label class="filter-group">
          <span class="filter-label">Min size (MiB)</span>
          <input
            v-model.number="minSizeMiB"
            class="input tree-search-number"
            type="number"
            min="0"
            step="any"
            data-testid="tree-search-min-size"
          />
        </label>
        <label class="filter-group">
          <span class="filter-label">Max size (MiB)</span>
          <input
            v-model.number="maxSizeMiB"
            class="input tree-search-number"
            type="number"
            min="0"
            step="any"
            data-testid="tree-search-max-size"
          />
        </label>
        <label class="filter-group">
          <span class="filter-label">Modified from</span>
          <input
            v-model="modifiedFrom"
            class="input"
            type="date"
            data-testid="tree-search-from"
          />
        </label>
        <label class="filter-group">
          <span class="filter-label">Modified to</span>
          <input
            v-model="modifiedTo"
            class="input"
            type="date"
            data-testid="tree-search-to"
          />
        </label>
        <button
          type="button"
          class="btn btn-sm btn-ghost"
          :disabled="activeFilterCount === 0"
          @click="resetFilters"
        >
          Reset filters
        </button>
      </div>
    </div>

    <div
      v-if="active"
      class="tree-search-results"
      data-testid="tree-search-results"
    >
      <BaseSpinner
        v-if="loading"
        size="sm"
      />
      <div
        v-else-if="error"
        class="error-banner"
      >
        {{ error }}
      </div>
      <p
        v-else-if="notIndexed"
        class="state-msg state-msg--inline"
      >
        The archive is still being indexed. Search it once indexing is done.
      </p>
      <p
        v-else-if="entries.length === 0"
        class="state-msg state-msg--inline"
      >
        Nothing in this archive matches.
      </p>
      <template v-else>
        <div class="table-wrap">
          <table class="data-table browser-table">
            <thead>
              <tr>
                <th class="tree-search-pick-col" />
                <th>Path</th>
                <th class="tree-search-size-col">Size</th>
                <th class="tree-search-date-col">Modified</th>
                <th class="tree-search-action-col" />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="entry in entries"
                :key="entry.path"
                data-testid="search-hit"
              >
                <td>
                  <input
                    type="checkbox"
                    class="tree-search-pick"
                    :checked="tickState(entry.path) === 'checked'"
                    :indeterminate="tickState(entry.path) === 'partial'"
                    :disabled="disabled"
                    :aria-label="`Select ${entry.path}`"
                    data-testid="pick-hit"
                    @change="emit('toggle', entry)"
                  />
                </td>
                <td>
                  <span
                    class="td-name"
                    :title="entry.path"
                  >
                    <Folder
                      v-if="isDirectoryEntry(entry)"
                      :size="16"
                      class="entry-icon"
                    />
                    <File
                      v-else
                      :size="16"
                      class="entry-icon"
                    />
                    <span class="name-text"
                      ><template
                        v-for="(part, i) in matchParts(entry.path, query)"
                        :key="i"
                        ><mark
                          v-if="part.match"
                          class="tree-search-match"
                          >{{ part.text }}</mark
                        ><template v-else>{{ part.text }}</template></template
                      ></span
                    >
                  </span>
                </td>
                <td class="td-size">
                  {{ isDirectoryEntry(entry) ? '-' : formatBytes(Number(entry.size)) }}
                </td>
                <td class="td-date">{{ entry.mtime ? formatDate(entry.mtime) : '' }}</td>
                <td>
                  <button
                    type="button"
                    class="btn btn-sm btn-ghost"
                    :title="isDirectoryEntry(entry) ? 'Open folder' : 'Show in its folder'"
                    :aria-label="`Show ${entry.path} in its folder`"
                    data-testid="reveal-hit"
                    @click="reveal(entry)"
                  >
                    <FolderOpen :size="14" />
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p
          v-if="truncated"
          class="muted tree-search-more"
        >
          Showing the first {{ entries.length }} matches. Narrow the search to see the rest.
        </p>
      </template>
    </div>
  </div>
</template>

<style scoped>
.tree-search-bar {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.tree-search-field {
  flex: 1;
  min-width: 0;
}

.tree-search-filters {
  margin: var(--space-4) var(--space-6);
}

.tree-search-number {
  width: 7rem;
}

.tree-search-results {
  padding-bottom: var(--space-4);
}

.tree-search-pick-col {
  width: 2.5rem;
}

.tree-search-size-col {
  width: 6rem;
}

.tree-search-date-col {
  width: 10rem;
}

.tree-search-action-col {
  width: 4rem;
}

.tree-search-pick {
  width: 1rem;
  height: 1rem;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.tree-search-match {
  background: var(--warning-subtle);
  color: inherit;
  border-radius: var(--radius-sm);
}

.tree-search-more {
  padding: var(--space-4) var(--space-6);
  font-size: var(--fs-sm);
}
</style>
