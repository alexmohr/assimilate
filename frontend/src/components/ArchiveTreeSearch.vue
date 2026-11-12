<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'
import { File, Folder, Search } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import BrowserRow from './BrowserRow.vue'
import BrowserRowGroup from './BrowserRowGroup.vue'
import type { ArchiveTreeSearchOptions } from '../api/archives'
import { useArchiveTreeSearch } from '../composables/useArchiveTreeSearch'
import { isDirectoryEntry, parentOf, type TickState } from '../utils/archiveSelection'
import { formatBytes, formatDate, formatDayYear } from '../utils/format'
import { matchParts } from '../utils/textMatch'
import type { ContentEntryResponse as ContentEntry } from '../types/generated'

/**
 * Searching the open archive: one box for part of a path, and three rows of
 * pills - type, size, age - for the filters, each a click rather than a
 * number or a date to type. The browser opens this from the search button in
 * its path bar; while a search is entered, its matches stand in for the
 * folder's listing, ticked and opened the same way.
 */
type KindFilter = 'all' | 'file' | 'dir'
type SizeFilter = 'any' | '10m' | '100m' | '1g'
type AgeFilter = 'any' | '7d' | '30d' | '365d'

interface Pill<V extends string> {
  value: V
  label: string
}

const MIB = 1024 * 1024
const DAY_MS = 86_400_000

const KIND_PILLS: readonly Pill<KindFilter>[] = [
  { value: 'all', label: 'Any' },
  { value: 'file', label: 'Files' },
  { value: 'dir', label: 'Folders' },
]

const SIZE_PILLS: readonly Pill<SizeFilter>[] = [
  { value: 'any', label: 'Any' },
  { value: '10m', label: 'Over 10 MB' },
  { value: '100m', label: 'Over 100 MB' },
  { value: '1g', label: 'Over 1 GB' },
]

const AGE_PILLS: readonly Pill<AgeFilter>[] = [
  { value: 'any', label: 'Any time' },
  { value: '7d', label: 'Last 7 days' },
  { value: '30d', label: 'Last 30 days' },
  { value: '365d', label: 'Last year' },
]

const MIN_SIZE: Record<SizeFilter, number | undefined> = {
  any: undefined,
  '10m': 10 * MIB,
  '100m': 100 * MIB,
  '1g': 1024 * MIB,
}

const AGE_DAYS: Record<AgeFilter, number | undefined> = {
  any: undefined,
  '7d': 7,
  '30d': 30,
  '365d': 365,
}

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
const kind = ref<KindFilter>('all')
const size = ref<SizeFilter>('any')
const age = ref<AgeFilter>('any')

const input = useTemplateRef<HTMLInputElement>('input')

const search = useArchiveTreeSearch(
  () => props.repoId,
  () => props.archiveName,
)
const { entries, truncated, loading, error, notIndexed } = search

/** `YYYY-MM-DD` of the day `days` ago. */
function daysAgo(days: number): string {
  return new Date(Date.now() - days * DAY_MS).toISOString().slice(0, 10)
}

const filtered = computed(() => kind.value !== 'all' || size.value !== 'any' || age.value !== 'any')

/** The search entered, or null while there is neither text nor a filter. */
const options = computed<ArchiveTreeSearchOptions | null>(() => {
  const text = query.value.trim()
  if (text === '' && !filtered.value) return null
  const days = AGE_DAYS[age.value]
  return {
    query: text === '' ? undefined : text,
    kind: kind.value === 'all' ? undefined : kind.value,
    minSize: MIN_SIZE[size.value],
    modifiedFrom: days === undefined ? undefined : daysAgo(days),
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

onMounted(() => input.value?.focus())

onBeforeUnmount(() => {
  cancelTimer()
  // Closing the search brings the folder's own listing back.
  if (active.value) emit('update:active', false)
})

function clearSearch(): void {
  query.value = ''
  kind.value = 'all'
  size.value = 'any'
  age.value = 'any'
}

/** Shows `entry` where it sits: a folder opened, a file in its folder. */
function reveal(entry: ContentEntry): void {
  const folder = isDirectoryEntry(entry) ? entry.path : parentOf(entry.path)
  clearSearch()
  emit('open', folder)
}

function nameOf(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

function folderLabel(entry: ContentEntry): string {
  const parent = parentOf(entry.path)
  return parent === '' ? '/' : `/${parent}`
}

function onRow(entry: ContentEntry): void {
  if (isDirectoryEntry(entry)) reveal(entry)
  else if (!props.disabled) emit('toggle', entry)
}

const countLabel = computed(() => {
  const n = entries.value.length
  const matches = `${n} ${n === 1 ? 'match' : 'matches'}`
  return truncated.value ? `The first ${matches} in this archive` : `${matches} in this archive`
})
</script>

<template>
  <div>
    <div class="tree-search-form">
      <div class="search-input-wrap">
        <Search
          :size="14"
          class="search-icon"
        />
        <input
          ref="input"
          v-model="query"
          class="input search-input--icon"
          type="search"
          placeholder="Search this archive, e.g. nginx or .pdf"
          aria-label="Search this archive"
          autocomplete="off"
          data-testid="tree-search"
        />
      </div>
      <div
        class="tree-search-filter"
        role="group"
        aria-label="Type"
      >
        <span class="tree-search-key">Type</span>
        <span class="filter-pills">
          <button
            v-for="pill in KIND_PILLS"
            :key="pill.value"
            type="button"
            class="filter-pill"
            :class="{ active: kind === pill.value }"
            :aria-pressed="kind === pill.value"
            @click="kind = pill.value"
          >
            {{ pill.label }}
          </button>
        </span>
      </div>
      <div
        class="tree-search-filter"
        role="group"
        aria-label="Size"
      >
        <span class="tree-search-key">Size</span>
        <span class="filter-pills">
          <button
            v-for="pill in SIZE_PILLS"
            :key="pill.value"
            type="button"
            class="filter-pill"
            :class="{ active: size === pill.value }"
            :aria-pressed="size === pill.value"
            data-testid="tree-search-size"
            @click="size = pill.value"
          >
            {{ pill.label }}
          </button>
        </span>
      </div>
      <div
        class="tree-search-filter"
        role="group"
        aria-label="Modified"
      >
        <span class="tree-search-key">Modified</span>
        <span class="filter-pills">
          <button
            v-for="pill in AGE_PILLS"
            :key="pill.value"
            type="button"
            class="filter-pill"
            :class="{ active: age === pill.value }"
            :aria-pressed="age === pill.value"
            data-testid="tree-search-age"
            @click="age = pill.value"
          >
            {{ pill.label }}
          </button>
        </span>
      </div>
    </div>

    <div
      v-if="active"
      data-testid="tree-search-results"
    >
      <div
        v-if="loading"
        class="loading-row"
      >
        <BaseSpinner size="sm" />
      </div>
      <div
        v-else-if="error"
        class="error-banner"
      >
        {{ error }}
      </div>
      <p
        v-else-if="notIndexed"
        class="state-msg state-msg--inline tree-search-msg"
      >
        The archive is still being indexed. Search it once indexing is done.
      </p>
      <p
        v-else-if="entries.length === 0"
        class="state-msg state-msg--inline tree-search-msg"
      >
        Nothing in this archive matches. Loosen a filter or try part of the name.
      </p>
      <template v-else>
        <BrowserRowGroup :label="countLabel" />
        <BrowserRow
          v-for="entry in entries"
          :key="entry.path"
          clickable
          :selected="tickState(entry.path) === 'checked'"
          :size="isDirectoryEntry(entry) ? null : formatBytes(Number(entry.size))"
          data-testid="search-hit"
          @click="onRow(entry)"
        >
          <template #lead>
            <input
              type="checkbox"
              class="tree-search-pick"
              :checked="tickState(entry.path) === 'checked'"
              :indeterminate="tickState(entry.path) === 'partial'"
              :disabled="disabled"
              :aria-label="`Select ${entry.path}`"
              data-testid="pick-hit"
              @click.stop
              @change="emit('toggle', entry)"
            />
            <Folder
              v-if="isDirectoryEntry(entry)"
              :size="16"
              class="tree-search-icon tree-search-icon--dir"
            />
            <File
              v-else
              :size="16"
              class="tree-search-icon"
            />
          </template>
          <template #title
            ><template
              v-for="(part, i) in matchParts(nameOf(entry.path), query)"
              :key="i"
              ><mark
                v-if="part.match"
                class="tree-search-match"
                >{{ part.text }}</mark
              ><template v-else>{{ part.text }}</template></template
            ></template
          >
          <template #facts>
            <button
              type="button"
              class="tree-search-folder"
              :title="isDirectoryEntry(entry) ? 'Open folder' : 'Show in its folder'"
              :aria-label="`Show ${entry.path} in its folder`"
              data-testid="reveal-hit"
              @click.stop="reveal(entry)"
            >
              {{ folderLabel(entry) }}
            </button>
            <template v-if="entry.mtime">
              <span aria-hidden="true">&middot;</span>
              <span :title="formatDate(entry.mtime)"
                >modified {{ formatDayYear(entry.mtime) }}</span
              >
            </template>
          </template>
        </BrowserRow>
      </template>
    </div>
  </div>
</template>

<style scoped>
.tree-search-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.tree-search-filter {
  display: flex;
  align-items: center;
  gap: var(--space-3) var(--space-5);
}

.tree-search-filter > .filter-pills {
  min-width: 0;
}

/* The label column keeps the three pill rows starting at one edge. */
.tree-search-key {
  flex: none;
  width: 4.5rem;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.tree-search-msg {
  padding: var(--space-5) var(--space-7);
}

.tree-search-pick {
  flex: none;
  width: 1rem;
  height: 1rem;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.tree-search-icon {
  flex: none;
  color: var(--text-muted);
}

.tree-search-icon--dir {
  color: var(--accent);
}

.tree-search-match {
  background: var(--warning-subtle);
  color: inherit;
  border-radius: var(--radius-sm);
}

.tree-search-folder {
  padding: 0;
  border: 0;
  background: none;
  font-family: var(--mono);
  font-size: inherit;
  color: var(--accent);
  text-align: left;
  overflow-wrap: anywhere;
  cursor: pointer;
}

.tree-search-folder:hover {
  text-decoration: underline;
}

@media (max-width: 480px) {
  .tree-search-filter {
    flex-direction: column;
    align-items: flex-start;
  }
}
</style>
