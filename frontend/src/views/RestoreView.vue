<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { ArchiveRestore, FolderSearch, SlidersHorizontal } from '@lucide/vue'
import { listRepoArchives, searchAcrossArchives } from '../api/archives'
import type { ArchiveTreeSearchOptions } from '../api/archives'
import { listRepos } from '../api/repos'
import { useAuthStore } from '../stores/auth'
import { useRestoreTree } from '../composables/useRestoreTree'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import { resolveArchiveHost } from '../utils/archiveHost'
import { collapse, selectAll, toggle } from '../utils/restoreSelection'
import BaseSegmented, { type SegmentedOption } from '../components/BaseSegmented.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import RestoreBasket from '../components/RestoreBasket.vue'
import RestoreTree from '../components/RestoreTree.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type {
  ArchiveEntryResponse as ArchiveEntry,
  CrossSearchEntryResponse,
} from '../types/generated'
import type { Repo } from '../types/repo'

type KindFilter = 'all' | 'file' | 'dir'

const SEARCH_DEBOUNCE_MS = 300
const BYTES_PER_MIB = 1024 * 1024

const route = useRoute()
const router = useRouter()
const { isAdmin } = storeToRefs(useAuthStore())

const repos = ref<Repo[]>([])
const reposLoading = ref(true)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchiveName = ref<string | null>(null)

const tree = useRestoreTree()
const selection = ref<string[]>([])

const query = ref('')
const kind = ref<KindFilter>('all')
const minSizeMiB = ref<number | null>(null)
const maxSizeMiB = ref<number | null>(null)
const modifiedFrom = ref('')
const modifiedTo = ref('')
const filtersOpen = ref(false)
const allArchives = ref(false)

const crossResults = ref<CrossSearchEntryResponse[]>([])
const crossLoading = ref(false)
const crossError = ref<string | null>(null)
const crossArchives = ref(0)

const kindOptions: SegmentedOption<KindFilter>[] = [
  { value: 'all', label: 'All' },
  { value: 'file', label: 'Files' },
  { value: 'dir', label: 'Folders' },
]

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const selectedArchive = computed<ArchiveEntry | null>(
  () => sortedArchives.value.find((a) => a.name === selectedArchiveName.value) ?? null,
)

/** The next older archive of the same host, which change badges compare against. */
const previousArchive = computed<ArchiveEntry | null>(() => {
  const archive = selectedArchive.value
  if (archive === null) return null
  const host = resolveArchiveHost(archive)
  const list = sortedArchives.value
  const index = list.findIndex((a) => a.name === archive.name)
  return list.slice(index + 1).find((a) => resolveArchiveHost(a) === host) ?? null
})

const multipleHosts = computed(
  () => new Set(archives.value.map((a) => resolveArchiveHost(a))).size > 1,
)

function archiveLabel(archive: ArchiveEntry): string {
  const date = formatDate(archive.start, archive.name)
  return multipleHosts.value ? `${date} - ${resolveArchiveHost(archive)}` : date
}

const activeFilterCount = computed(
  () =>
    Number(kind.value !== 'all') +
    Number(minSizeMiB.value !== null) +
    Number(maxSizeMiB.value !== null) +
    Number(modifiedFrom.value !== '') +
    Number(modifiedTo.value !== ''),
)

const searchOptions = computed<ArchiveTreeSearchOptions | null>(() => {
  const text = query.value.trim()
  if (text === '' && activeFilterCount.value === 0) return null
  return {
    query: text === '' ? undefined : text,
    kind: kind.value === 'all' ? undefined : kind.value,
    minSize: minSizeMiB.value === null ? undefined : Math.round(minSizeMiB.value * BYTES_PER_MIB),
    maxSize: maxSizeMiB.value === null ? undefined : Math.round(maxSizeMiB.value * BYTES_PER_MIB),
    modifiedFrom: modifiedFrom.value === '' ? undefined : modifiedFrom.value,
    modifiedTo: modifiedTo.value === '' ? undefined : modifiedTo.value,
  }
})

const highlight = computed(() => (allArchives.value ? '' : query.value.trim()))

async function loadRepos(): Promise<void> {
  reposLoading.value = true
  reposError.value = null
  try {
    repos.value = await listRepos()
    const wanted = Number(route.query.repo)
    const fromRoute = repos.value.find((r) => r.id === wanted)
    selectedRepoId.value = fromRoute?.id ?? repos.value[0]?.id ?? null
    await loadArchives(typeof route.query.archive === 'string' ? route.query.archive : null)
  } catch (e: unknown) {
    reposError.value = extractError(e)
  } finally {
    reposLoading.value = false
  }
}

async function loadArchives(preferred: string | null): Promise<void> {
  archives.value = []
  selectedArchiveName.value = null
  archivesError.value = null
  if (selectedRepoId.value === null) return
  archivesLoading.value = true
  try {
    archives.value = await listRepoArchives(selectedRepoId.value)
    const names = sortedArchives.value.map((a) => a.name)
    selectedArchiveName.value =
      preferred !== null && names.includes(preferred) ? preferred : (names[0] ?? null)
  } catch (e: unknown) {
    archivesError.value = extractError(e)
  } finally {
    archivesLoading.value = false
  }
}

function syncRoute(): void {
  const next = { ...route.query }
  if (selectedRepoId.value === null) delete next.repo
  else next.repo = String(selectedRepoId.value)
  if (selectedArchiveName.value === null) delete next.archive
  else next.archive = selectedArchiveName.value
  router.replace({ query: next }).catch(() => undefined)
}

async function onRepoChange(): Promise<void> {
  await loadArchives(null)
}

watch(selectedArchive, async (archive) => {
  selection.value = []
  syncRoute()
  const repoId = selectedRepoId.value
  const before = previousArchive.value
  await tree.open(
    archive === null || repoId === null ? null : { repoId, name: archive.name },
    before === null || repoId === null ? null : { repoId, name: before.name },
  )
  if (!allArchives.value && searchOptions.value !== null) await tree.search(searchOptions.value)
})

let searchTimer: ReturnType<typeof setTimeout> | null = null

async function runSearch(): Promise<void> {
  if (allArchives.value) {
    await tree.search(null)
    await runCrossSearch()
  } else {
    crossResults.value = []
    crossError.value = null
    await tree.search(searchOptions.value)
  }
}

watch([searchOptions, allArchives], () => {
  if (searchTimer !== null) clearTimeout(searchTimer)
  searchTimer = setTimeout(() => {
    searchTimer = null
    void runSearch()
  }, SEARCH_DEBOUNCE_MS)
})

let crossSeq = 0

async function runCrossSearch(): Promise<void> {
  const text = query.value.trim()
  const repoId = selectedRepoId.value
  const seq = ++crossSeq
  crossError.value = null
  if (text === '' || repoId === null) {
    crossResults.value = []
    return
  }
  crossLoading.value = true
  try {
    // borg's shell-style patterns match whole paths, and `*` stops at a `/`.
    const res = await searchAcrossArchives(repoId, { pattern: `**/*${text}*` })
    if (seq !== crossSeq) return
    crossResults.value = res.items
    crossArchives.value = res.total_archives_searched
  } catch (e: unknown) {
    if (seq === crossSeq) crossError.value = extractError(e)
  } finally {
    if (seq === crossSeq) crossLoading.value = false
  }
}

/** Opens the archive a cross-archive hit came from, with the tree searched down to it. */
function openHit(hit: CrossSearchEntryResponse): void {
  allArchives.value = false
  query.value = hit.path
  selectedArchiveName.value = hit.archive_name
}

async function onTick(path: string): Promise<void> {
  let result = toggle(selection.value, path, tree.childrenOf)
  if ('missing' in result) {
    await tree.ensureListings(result.missing)
    result = toggle(selection.value, path, tree.childrenOf)
  }
  if ('selection' in result) {
    selection.value = result.selection
    void tree.ensureTotals(result.selection)
  }
}

function onSelectAll(): void {
  const matched = tree.rows.value.flatMap((row) =>
    row.kind === 'entry' && row.match ? [row.path] : [],
  )
  selection.value = tree.searchActive.value
    ? collapse([...selection.value, ...matched], tree.childrenOf)
    : selectAll(tree.rootChildren.value, tree.childrenOf)
  void tree.ensureTotals(selection.value)
}

function onClear(): void {
  selection.value = []
}

function onRemove(path: string): void {
  selection.value = selection.value.filter((p) => p !== path)
}

function resetFilters(): void {
  kind.value = 'all'
  minSizeMiB.value = null
  maxSizeMiB.value = null
  modifiedFrom.value = ''
  modifiedTo.value = ''
}

onMounted(loadRepos)

onBeforeUnmount(() => {
  if (searchTimer !== null) clearTimeout(searchTimer)
  tree.dispose()
})
</script>

<template>
  <div class="restore-view">
    <div class="page-header">
      <h1 class="page-title">Restore</h1>
    </div>
    <p class="page-description">
      Pick an archive, tick the files and folders to bring back, then restore them onto the
      archive's host or download them as one tar.lz4.
    </p>

    <BaseSpinner
      v-if="reposLoading"
      size="lg"
    />
    <div
      v-else-if="reposError"
      class="error-banner"
    >
      {{ reposError }}
    </div>
    <EmptyState
      v-else-if="repos.length === 0"
      :icon="ArchiveRestore"
      title="No repositories yet"
      description="Archives appear here once a repository has been added and backed up to."
    />
    <div
      v-else
      class="page-sections"
    >
      <section
        class="panel restore-bar"
        aria-label="Archive and search"
      >
        <div class="restore-bar-row">
          <label class="restore-bar-field">
            <span class="filter-label">Repository</span>
            <select
              v-model="selectedRepoId"
              class="input select-input"
              data-testid="restore-repo-select"
              @change="onRepoChange"
            >
              <option
                v-for="repo in repos"
                :key="repo.id"
                :value="repo.id"
              >
                {{ repo.name }}
              </option>
            </select>
          </label>
          <label class="restore-bar-field">
            <span class="filter-label">Archive</span>
            <select
              v-model="selectedArchiveName"
              class="input select-input"
              data-testid="restore-archive-select"
              :disabled="archivesLoading || sortedArchives.length === 0"
            >
              <option
                v-for="archive in sortedArchives"
                :key="archive.name"
                :value="archive.name"
                :title="archive.name"
              >
                {{ archiveLabel(archive) }}
              </option>
            </select>
          </label>
          <label class="restore-bar-field restore-bar-search">
            <span class="filter-label">Search</span>
            <input
              v-model="query"
              class="input"
              type="search"
              :placeholder="allArchives ? 'File name in any archive' : 'Path contains...'"
              aria-label="Search files"
              data-testid="restore-search"
            />
          </label>
          <button
            type="button"
            class="filter-toggle"
            :class="{ active: filtersOpen }"
            :aria-expanded="filtersOpen"
            :disabled="allArchives"
            @click="filtersOpen = !filtersOpen"
          >
            <SlidersHorizontal :size="14" />
            Filters
            <span
              v-if="activeFilterCount > 0"
              class="filter-badge"
            />
          </button>
          <div class="restore-bar-toggle">
            <ToggleSwitch
              v-model="allArchives"
              label="Search all archives"
            />
            <span class="muted">All archives</span>
          </div>
        </div>

        <div
          v-if="filtersOpen && !allArchives"
          class="filters restore-filters"
        >
          <div class="filter-row">
            <div class="filter-group">
              <span class="filter-label">Type</span>
              <BaseSegmented
                v-model="kind"
                :options="kindOptions"
                label="Entry type"
              />
            </div>
            <label class="filter-group">
              <span class="filter-label">Min size (MiB)</span>
              <input
                v-model.number="minSizeMiB"
                class="input input-sm restore-number"
                type="number"
                min="0"
                step="any"
              />
            </label>
            <label class="filter-group">
              <span class="filter-label">Max size (MiB)</span>
              <input
                v-model.number="maxSizeMiB"
                class="input input-sm restore-number"
                type="number"
                min="0"
                step="any"
              />
            </label>
            <label class="filter-group">
              <span class="filter-label">Modified from</span>
              <input
                v-model="modifiedFrom"
                class="input input-sm"
                type="date"
              />
            </label>
            <label class="filter-group">
              <span class="filter-label">Modified to</span>
              <input
                v-model="modifiedTo"
                class="input input-sm"
                type="date"
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
      </section>

      <div
        v-if="archivesError"
        class="error-banner"
      >
        {{ archivesError }}
      </div>
      <BaseSpinner
        v-else-if="archivesLoading"
        size="md"
      />
      <EmptyState
        v-else-if="sortedArchives.length === 0"
        :icon="ArchiveRestore"
        title="No archives in this repository"
        description="Archives appear here once a backup has run."
      />

      <section
        v-else-if="allArchives"
        class="panel"
        aria-label="Search across archives"
        data-testid="restore-cross-results"
      >
        <div class="panel-header">
          <h2 class="panel-title">Matches in every archive</h2>
          <span
            v-if="crossResults.length > 0"
            class="row-count"
            >{{ crossResults.length }} in {{ crossArchives }} archives</span
          >
        </div>
        <BaseSpinner
          v-if="crossLoading"
          size="sm"
        />
        <div
          v-else-if="crossError"
          class="error-banner"
        >
          {{ crossError }}
        </div>
        <p
          v-else-if="query.trim() === ''"
          class="muted"
        >
          Type a file name to find the archives that hold it.
        </p>
        <EmptyState
          v-else-if="crossResults.length === 0"
          :icon="FolderSearch"
          title="No matches"
          description="No recent archive holds a path matching that name."
        />
        <ul
          v-else
          class="cross-list"
        >
          <li
            v-for="hit in crossResults"
            :key="`${hit.archive_name}:${hit.path}`"
          >
            <button
              type="button"
              class="cross-hit"
              @click="openHit(hit)"
            >
              <span class="mono cross-path">{{ hit.path }}</span>
              <span class="muted">{{ hit.archive_name }}</span>
            </button>
          </li>
        </ul>
      </section>

      <div
        v-else-if="selectedArchive"
        class="restore-layout"
      >
        <section
          class="panel panel--sectioned restore-tree-panel"
          aria-labelledby="restore-tree-title"
        >
          <div class="panel-header">
            <h2
              id="restore-tree-title"
              class="panel-title panel-title--truncate"
              :title="selectedArchive.name"
            >
              {{ selectedArchive.name }}
            </h2>
            <div class="panel-actions">
              <span
                v-if="tree.searchActive.value"
                class="row-count"
                data-testid="restore-search-count"
                >{{ tree.searchCount.value }}{{ tree.searchTruncated.value ? '+' : '' }}
                {{ tree.searchCount.value === 1 ? 'match' : 'matches' }}</span
              >
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                :disabled="tree.rows.value.length === 0"
                @click="onSelectAll"
              >
                {{ tree.searchActive.value ? 'Select matches' : 'Select all' }}
              </button>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                :disabled="selection.length === 0"
                @click="onClear"
              >
                Clear
              </button>
            </div>
          </div>
          <div class="panel-body">
            <div
              v-if="tree.error.value"
              class="error-banner"
            >
              {{ tree.error.value }}
            </div>
            <div
              v-else-if="
                tree.indexState.value === 'pending' || tree.indexState.value === 'indexing'
              "
              class="loading-row"
            >
              <BaseSpinner size="sm" />
              <span class="muted">Indexing archive contents, this only happens once...</span>
            </div>
            <div
              v-else-if="tree.searchError.value"
              class="error-banner"
            >
              {{ tree.searchError.value }}
            </div>
            <BaseSpinner
              v-else-if="tree.rootLoading.value && tree.rows.value.length === 0"
              size="sm"
            />
            <EmptyState
              v-else-if="tree.rows.value.length === 0"
              :icon="FolderSearch"
              :title="tree.searchActive.value ? 'No matches' : 'This archive is empty'"
              :description="
                tree.searchActive.value
                  ? 'Nothing in this archive matches the search and filters.'
                  : undefined
              "
            />
            <template v-else>
              <p
                v-if="tree.searchTruncated.value"
                class="field-hint"
              >
                Showing the first {{ tree.searchCount.value }} matches. Narrow the search to see the
                rest.
              </p>
              <RestoreTree
                :rows="tree.rows.value"
                :selection="selection"
                :totals="tree.totals.value"
                :change-of="tree.changeOf"
                :highlight="highlight"
                @tick="onTick"
                @expand="tree.toggleExpand"
                @more="tree.loadMore"
              />
            </template>
          </div>
        </section>

        <RestoreBasket
          v-if="selectedRepoId !== null"
          :repo-id="selectedRepoId"
          :archive="selectedArchive"
          :paths="selection"
          :entry-of="tree.entryOf"
          :totals="tree.totals.value"
          :is-admin="isAdmin"
          @remove="onRemove"
          @clear="onClear"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.restore-view {
  color: var(--text-primary);
}

.restore-bar {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.restore-bar-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-6);
}

.restore-bar-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}

.restore-bar-search {
  flex: 1;
  min-width: 12rem;
}

.restore-bar-toggle {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-height: 2.25rem;
}

.restore-filters {
  border: none;
  padding: 0;
}

.restore-number {
  width: 7rem;
}

.restore-layout {
  display: grid;
  grid-template-columns: minmax(0, 1.6fr) minmax(18rem, 1fr);
  gap: var(--space-6);
  align-items: start;
}

.restore-tree-panel {
  min-width: 0;
}

.cross-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.cross-hit {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-3) var(--space-4);
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-primary);
  text-align: left;
  font: inherit;
  cursor: pointer;
}

.cross-hit:hover {
  background: var(--bg-hover);
}

.cross-path {
  min-width: 0;
  overflow-wrap: anywhere;
}

@media (max-width: 1024px) {
  .restore-layout {
    grid-template-columns: 1fr;
  }
}
</style>
