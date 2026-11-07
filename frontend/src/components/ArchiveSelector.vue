<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, toRef, watch } from 'vue'
import { AlertTriangle, Check, ChevronRight, Search } from '@lucide/vue'
import ArchiveSelectorRow from './ArchiveSelectorRow.vue'
import BaseHostLink from './BaseHostLink.vue'
import BaseSkeleton from './BaseSkeleton.vue'
import BrowserRow from './BrowserRow.vue'
import BrowserRowGroup from './BrowserRowGroup.vue'
import EmptyState from './EmptyState.vue'
import OverflowMenu from './OverflowMenu.vue'
import { formatBytes } from '../utils/format'
import {
  ARCHIVE_SORT_OPTIONS,
  useArchiveList,
  type ArchiveGroupMode,
} from '../composables/useArchiveList'
import { archiveKey, sameArchive, type RepoArchive } from '../utils/repoArchive'

/**
 * The archive list pane: one search box and the repository pills above the
 * archives, everything else - grouping, sort, refresh - in the card's menu.
 *
 * The search box looks for archives by name and, from two characters on,
 * offers a first row that searches for files of that name in every archive
 * the list holds instead. Grouping by day is the default for one host's
 * archives, by host for a repository several hosts write into.
 */
interface GroupOption {
  value: ArchiveGroupMode
  label: string
}

const DAY_OPTION: GroupOption = { value: 'day', label: 'By day' }
const HOST_OPTION: GroupOption = { value: 'host', label: 'By host' }
const FLAT_OPTION: GroupOption = { value: 'flat', label: 'Flat list' }

/** The repository filter's "everything" choice; a real repository id is never negative. */
const ALL_REPOS = -1

/** How many characters the search needs before it offers to look for files. */
const FILE_SEARCH_MIN_LENGTH = 2

/** Day headings whose archives fall within the last week, so a row can show its time alone. */
const RECENT_PERIODS: ReadonlySet<string> = new Set(['today', 'yesterday', 'week'])

const props = withDefaults(
  defineProps<{
    archives: RepoArchive[]
    loading?: boolean
    error?: string | null
    /** Archives whose borg delete is already queued, by `archiveKey`. */
    deletingKeys?: readonly string[]
    /** Replaces the toolbar and rows, for a caller that filters the list to one archive. */
    hideControls?: boolean
    /** Whether the search offers to look for files across the archives. */
    fileSearch?: boolean
    /** Whether the menu offers to fetch the list again. */
    refreshable?: boolean
    emptyTitle?: string
    emptyDescription?: string
  }>(),
  {
    loading: false,
    error: null,
    deletingKeys: () => [],
    hideControls: false,
    fileSearch: false,
    refreshable: false,
    emptyTitle: 'No archives',
    emptyDescription: 'Archives appear here once a backup has run against this repository.',
  },
)

const emit = defineEmits<{
  /** Look for files named `query` in `archives`, the ones the list is narrowed to. */
  'search-files': [query: string, archives: RepoArchive[]]
  refresh: []
}>()

const selected = defineModel<RepoArchive | null>('selected', { required: true })

const {
  filter,
  sortMode,
  repoFilter,
  repos,
  groupMode,
  multiHost,
  ordered,
  grouped,
  dayGroups,
  unmatchedCount,
  unmatchedHostnames,
  toggleGroup,
  isGroupCollapsed,
  reset,
} = useArchiveList(toRef(props, 'archives'))

/** Host groups only mean something when the list holds more than one host. */
const groupOptions = computed<readonly GroupOption[]>(() =>
  multiHost.value ? [DAY_OPTION, HOST_OPTION, FLAT_OPTION] : [DAY_OPTION, FLAT_OPTION],
)

/**
 * Several repositories - an agent backing up to two, a schedule with two
 * targets - are one list, plus these pills to narrow it to one of them.
 */
const multiRepo = computed(() => repos.value.length > 1)

const repoOptions = computed<{ value: number; label: string }[]>(() => [
  { value: ALL_REPOS, label: `All · ${props.archives.length}` },
  ...repos.value.map((repo) => ({ value: repo.id, label: `${repo.name} · ${repo.count}` })),
])

const repoChoice = computed<number>({
  get: () => repoFilter.value ?? ALL_REPOS,
  set: (id) => {
    repoFilter.value = id === ALL_REPOS ? null : id
  },
})

/** Rows name their repository only while the list shows more than one. */
const showRepo = computed(() => multiRepo.value && repoFilter.value === null)

/** The archives the repository pills leave, which a file search looks through. */
const inScope = computed<RepoArchive[]>(() =>
  repoFilter.value === null
    ? props.archives
    : props.archives.filter((a) => a.repo_id === repoFilter.value),
)

const fileQuery = computed(() => filter.value.trim())

const offerFileSearch = computed(
  () => props.fileSearch && fileQuery.value.length >= FILE_SEARCH_MIN_LENGTH,
)

const countLabel = computed(() =>
  ordered.value.length === props.archives.length
    ? `${props.archives.length}`
    : `${ordered.value.length} of ${props.archives.length}`,
)

const deleting = computed(() => new Set(props.deletingKeys))

function isDeleting(archive: RepoArchive): boolean {
  return deleting.value.has(archiveKey(archive))
}

function groupSize(archives: RepoArchive[]): string {
  return formatBytes(archives.reduce((total, a) => total + a.original_size, 0))
}

function select(archive: RepoArchive): void {
  selected.value = archive
}

function isSelected(archive: RepoArchive): boolean {
  return sameArchive(selected.value, archive)
}

function searchFiles(): void {
  emit('search-files', fileQuery.value, inScope.value)
}

/**
 * A selection made from outside the list - the schedule Overview's "browse
 * this archive" jump, a `?browse=` link, a file found by the search - can name
 * an archive the search or the repository filter is hiding, which would leave
 * the archive open on an archive the list says is not there. Clearing the two
 * brings it back into view; a row clicked in the list is always visible, so
 * this never fights the reader.
 */
watch(selected, (archive) => {
  if (archive === null || ordered.value.some((a) => sameArchive(a, archive))) return
  filter.value = ''
  repoFilter.value = null
})

/**
 * Exposed for the repository tab, which resets filter and collapse state when
 * the view switches to another repository under a mounted list.
 */
defineExpose({ reset, unmatchedCount, unmatchedHostnames })
</script>

<template>
  <div class="panel panel--sectioned archives-panel">
    <div class="panel-header">
      <span class="archives-heading">
        <span class="panel-title">Archives</span>
        <span
          v-if="!loading && archives.length > 0"
          class="muted archives-count"
          >{{ countLabel }}</span
        >
      </span>
      <span class="panel-actions">
        <slot name="actions" />
        <OverflowMenu
          v-if="!hideControls && (archives.length > 0 || refreshable)"
          v-slot="{ run }"
          label="List options"
        >
          <template v-if="archives.length > 0">
            <span class="archives-menu-label">Group</span>
            <button
              v-for="option in groupOptions"
              :key="option.value"
              class="overflow-menu-item archives-menu-choice"
              role="menuitemradio"
              type="button"
              :aria-checked="groupMode === option.value"
              @click="run(() => (groupMode = option.value))"
            >
              {{ option.label }}
              <Check
                v-if="groupMode === option.value"
                :size="14"
                class="archives-menu-check"
              />
            </button>
            <span class="archives-menu-label archives-menu-label--ruled">Sort</span>
            <button
              v-for="option in ARCHIVE_SORT_OPTIONS"
              :key="option.value"
              class="overflow-menu-item archives-menu-choice"
              role="menuitemradio"
              type="button"
              :aria-checked="sortMode === option.value"
              @click="run(() => (sortMode = option.value))"
            >
              {{ option.label }}
              <Check
                v-if="sortMode === option.value"
                :size="14"
                class="archives-menu-check"
              />
            </button>
          </template>
          <button
            v-if="refreshable"
            class="overflow-menu-item archives-menu-refresh"
            role="menuitem"
            type="button"
            :disabled="loading"
            @click="run(() => emit('refresh'))"
          >
            Refresh archives
          </button>
        </OverflowMenu>
      </span>
    </div>

    <slot name="banner" />

    <div
      v-if="loading"
      class="archive-loading"
    >
      <BaseSkeleton
        v-for="i in 4"
        :key="i"
        variant="row"
        height="3rem"
      />
    </div>
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="archives.length === 0"
      :title="emptyTitle"
      :description="emptyDescription"
    />
    <template v-else-if="!hideControls">
      <div class="archive-controls">
        <div class="search-input-wrap">
          <Search
            :size="14"
            class="search-icon"
          />
          <input
            v-model="filter"
            class="input search-input--icon"
            type="search"
            :placeholder="fileSearch ? 'Search archives or files' : 'Search archives'"
            aria-label="Search archives"
            autocomplete="off"
            @keydown.enter="offerFileSearch && ordered.length === 0 && searchFiles()"
          />
        </div>
        <div
          v-if="multiRepo"
          class="filter-pills"
          role="group"
          aria-label="Repository"
        >
          <button
            v-for="option in repoOptions"
            :key="option.value"
            type="button"
            class="filter-pill"
            :class="{ active: repoChoice === option.value }"
            :aria-pressed="repoChoice === option.value"
            @click="repoChoice = option.value"
          >
            {{ option.label }}
          </button>
        </div>
      </div>

      <div class="archive-rows">
        <BrowserRow
          v-if="offerFileSearch"
          as="button"
          :mono="false"
          class="archive-find"
          data-testid="find-files"
          @click="searchFiles"
        >
          <template #lead>
            <Search
              :size="14"
              class="archive-find-icon"
            />
          </template>
          <template #title>Find files named &ldquo;{{ fileQuery }}&rdquo;</template>
          <template #facts>
            in {{ repoFilter === null && inScope.length > 1 ? 'all ' : '' }}{{ inScope.length }}
            {{ inScope.length === 1 ? 'archive' : 'archives' }}
          </template>
          <template #end>
            <ChevronRight
              :size="14"
              class="archive-find-icon"
            />
          </template>
        </BrowserRow>

        <div
          v-if="ordered.length === 0"
          class="archive-no-match"
        >
          <span class="archive-no-match-title">No archive names match</span>
          <span
            v-if="offerFileSearch"
            class="muted"
            >Use the row above to look for files instead.</span
          >
        </div>
        <template v-else-if="groupMode === 'day'">
          <template
            v-for="group in dayGroups"
            :key="group.key"
          >
            <BrowserRowGroup :label="group.label" />
            <ArchiveSelectorRow
              v-for="archive in group.archives"
              :key="archiveKey(archive)"
              :archive="archive"
              :selected="isSelected(archive)"
              :show-host="multiHost"
              :repo-name="showRepo ? archive.repo_name : null"
              :clock="RECENT_PERIODS.has(group.key)"
              :deleting="isDeleting(archive)"
              @select="select(archive)"
            />
          </template>
        </template>
        <template v-else-if="groupMode === 'host'">
          <template
            v-for="group in grouped"
            :key="group.hostname"
          >
            <!--
              The caption handles the click and the button beside the host name
              carries the state for a screen reader: the host name is a link to
              the agent, and an anchor cannot be nested inside a button.
            -->
            <div
              class="group-header"
              :class="{ collapsed: isGroupCollapsed(group.hostname) }"
              @click="toggleGroup(group.hostname)"
            >
              <button
                class="group-toggle"
                type="button"
                :aria-expanded="!isGroupCollapsed(group.hostname)"
                :aria-label="`Toggle ${group.hostname}`"
              >
                <ChevronRight
                  class="group-chevron"
                  :size="14"
                />
              </button>
              <span class="group-host">
                <BaseHostLink
                  :hostname="group.hostname"
                  class="group-hostname"
                  :class="{ 'group-unmatched': !group.matched }"
                  @click.stop
                />
              </span>
              <AlertTriangle
                v-if="!group.matched"
                class="match-icon match-warn"
                :size="12"
              />
              <span class="group-count">{{ group.archives.length }}</span>
              <span class="group-size">{{ groupSize(group.archives) }}</span>
            </div>
            <template v-if="!isGroupCollapsed(group.hostname)">
              <ArchiveSelectorRow
                v-for="archive in group.archives"
                :key="archiveKey(archive)"
                :archive="archive"
                :selected="isSelected(archive)"
                :repo-name="showRepo ? archive.repo_name : null"
                :deleting="isDeleting(archive)"
                @select="select(archive)"
              />
            </template>
          </template>
        </template>
        <template v-else>
          <BrowserRowGroup label="All archives" />
          <ArchiveSelectorRow
            v-for="archive in ordered"
            :key="archiveKey(archive)"
            :archive="archive"
            :selected="isSelected(archive)"
            :show-host="multiHost"
            :repo-name="showRepo ? archive.repo_name : null"
            :deleting="isDeleting(archive)"
            @select="select(archive)"
          />
        </template>
      </div>
    </template>
  </div>
</template>

<style scoped>
/* The menu drops out of the card's header, so the card must not clip it. */
.archives-panel {
  overflow: visible;
}

.archives-heading {
  display: flex;
  align-items: baseline;
  gap: var(--space-4);
  min-width: 0;
}

.archives-count {
  font-size: var(--fs-sm);
  font-variant-numeric: tabular-nums;
}

.archives-menu-label {
  padding: var(--space-3) var(--space-4) var(--space-1);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

.archives-menu-label--ruled {
  margin-top: var(--space-2);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
}

.archives-menu-choice {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
}

.archives-menu-check {
  flex: none;
  color: var(--accent);
}

.archives-menu-refresh {
  margin-top: var(--space-2);
  border-top: 1px solid var(--border);
  border-radius: 0;
}

.archive-loading {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-5) var(--space-7);
}

.archive-controls {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

/* Long lists scroll inside the card beside the open archive. On a phone the
   list is the whole screen, so it just runs down the page. */
.archive-rows {
  max-height: min(40rem, 75vh);
  overflow-y: auto;
  border-radius: 0 0 var(--radius) var(--radius);
}

@media (max-width: 768px) {
  .archive-rows {
    max-height: none;
  }
}

.archive-find {
  color: var(--accent);
}

.archive-find :deep(.browser-row-title) {
  font-weight: 500;
}

.archive-find-icon {
  flex: none;
}

.archive-no-match {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-7);
  font-size: var(--fs-sm);
}

.archive-no-match-title {
  font-weight: 600;
}

.group-header {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-4) var(--space-7);
  border-top: 1px solid var(--border-subtle);
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
}

.group-header:first-child {
  border-top: none;
}

.group-header:hover .group-hostname:not(.group-unmatched) {
  color: var(--accent);
}

.group-toggle {
  display: flex;
  align-items: center;
  padding: 0;
  background: none;
  border: none;
  cursor: pointer;
  color: inherit;
  font: inherit;
}

.group-chevron {
  flex: none;
  color: var(--text-muted);
  transform: rotate(90deg);
  transition: transform var(--duration-base);
}

.group-header.collapsed .group-chevron {
  transform: rotate(0deg);
}

/* Holds the flex width, so the dead space beside a short host name toggles
   the group rather than following the link. */
.group-host {
  flex: 1;
  min-width: 0;
}

.group-hostname {
  display: inline-block;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  vertical-align: bottom;
  color: inherit;
}

.group-unmatched {
  color: var(--warning);
}

.group-count,
.group-size {
  flex: none;
  font-size: var(--fs-xs);
  font-weight: 400;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
