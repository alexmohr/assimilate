<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, toRef, watch } from 'vue'
import { AlertTriangle, ChevronRight, Search } from '@lucide/vue'
import ArchiveSelectorRow from './ArchiveSelectorRow.vue'
import BaseHostLink from './BaseHostLink.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSkeleton from './BaseSkeleton.vue'
import EmptyState from './EmptyState.vue'
import { formatBytes } from '../utils/format'
import {
  ARCHIVE_SORT_OPTIONS,
  useArchiveList,
  type ArchiveGroupMode,
} from '../composables/useArchiveList'
import { archiveKey, sameArchive, type RepoArchive } from '../utils/repoArchive'

/**
 * The archive list pane: search, sort, host grouping, selection and the
 * per-archive delete control.
 *
 * The repository tab, the schedule's backups tab and the standalone archives
 * page each carried their own version of this - one grouped by host, one was a
 * four-column table, one a PrimeVue grid with four column filters - and only
 * one of them offered deletion. This is the single list all three render.
 */
const DAY_OPTION: SegmentedOption<ArchiveGroupMode> = { value: 'day', label: 'By day' }
const HOST_OPTION: SegmentedOption<ArchiveGroupMode> = { value: 'host', label: 'By host' }
const FLAT_OPTION: SegmentedOption<ArchiveGroupMode> = { value: 'flat', label: 'Flat' }

/** The repository filter's "everything" choice; a real repository id is never negative. */
const ALL_REPOS = -1

const props = withDefaults(
  defineProps<{
    archives: RepoArchive[]
    loading?: boolean
    error?: string | null
    /** Whether this user may delete, i.e. an admin on a real repository. */
    canDelete?: boolean
    /** Archives whose borg delete is already queued, by `archiveKey`. */
    deletingKeys?: readonly string[]
    /** Replaces the toolbar and rows, for a caller that filters the list to one archive. */
    hideControls?: boolean
    emptyTitle?: string
    emptyDescription?: string
  }>(),
  {
    loading: false,
    error: null,
    canDelete: false,
    deletingKeys: () => [],
    hideControls: false,
    emptyTitle: 'No archives',
    emptyDescription: 'Archives appear here once a backup has run against this repository.',
  },
)

const emit = defineEmits<{ delete: [archive: RepoArchive] }>()

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
const groupOptions = computed<readonly SegmentedOption<ArchiveGroupMode>[]>(() =>
  multiHost.value ? [DAY_OPTION, HOST_OPTION, FLAT_OPTION] : [DAY_OPTION, FLAT_OPTION],
)

/**
 * Several repositories - an agent backing up to two, a schedule with two
 * targets - are one list with each row saying which copy it is, plus this
 * filter to narrow it to one of them.
 */
const multiRepo = computed(() => repos.value.length > 1)

const repoOptions = computed<SegmentedOption<number>[]>(() => [
  { value: ALL_REPOS, label: `All · ${props.archives.length}` },
  ...repos.value.map((repo) => ({ value: repo.id, label: `${repo.name} · ${repo.count}` })),
])

const repoChoice = computed<number>({
  get: () => repoFilter.value ?? ALL_REPOS,
  set: (id) => {
    repoFilter.value = id === ALL_REPOS ? null : id
  },
})

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

/**
 * A selection made from outside the list - the schedule Overview's "browse
 * this archive" jump, a `?archive=` link - can name an archive the search or
 * the repository filter is hiding, which would leave the file tree open on an
 * archive the list says is not there. Clearing the two brings it back into
 * view; a row clicked in the list is always visible, so this never fights the
 * reader.
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
      <span class="panel-title">Archives</span>
      <span
        v-if="!loading && archives.length > 0"
        class="archive-count"
        >{{ archives.length }}</span
      >
      <span
        v-if="$slots.actions"
        class="panel-actions"
      >
        <slot name="actions" />
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
        <div class="search-input-wrap archive-search">
          <Search
            :size="14"
            class="search-icon"
          />
          <input
            v-model="filter"
            class="input search-input--icon"
            type="text"
            placeholder="Search name or host"
            aria-label="Search archives"
          />
        </div>
        <select
          v-model="sortMode"
          class="input select-input archive-sort-select"
          aria-label="Sort archives"
        >
          <option
            v-for="option in ARCHIVE_SORT_OPTIONS"
            :key="option.value"
            :value="option.value"
          >
            {{ option.label }}
          </option>
        </select>
        <BaseSegmented
          v-model="groupMode"
          class="archive-group-toggle"
          :options="groupOptions"
          label="Group archives"
        />
        <BaseSegmented
          v-if="multiRepo"
          v-model="repoChoice"
          class="archive-repo-filter"
          :options="repoOptions"
          label="Repository"
        />
      </div>

      <div
        v-if="ordered.length === 0"
        class="state-msg state-msg--inline archive-no-match"
      >
        No archives match the search.
      </div>
      <div
        v-else-if="groupMode === 'day'"
        class="archive-groups"
      >
        <section
          v-for="group in dayGroups"
          :key="group.key"
          class="archive-day-group"
          :aria-label="group.label"
        >
          <h3 class="group-label archive-day-label">{{ group.label }}</h3>
          <ArchiveSelectorRow
            v-for="archive in group.archives"
            :key="archiveKey(archive)"
            :archive="archive"
            :selected="isSelected(archive)"
            :flat="true"
            :show-host="multiHost"
            :repo-name="multiRepo ? archive.repo_name : null"
            :can-delete="canDelete"
            :deleting="isDeleting(archive)"
            @select="select(archive)"
            @delete="emit('delete', archive)"
          />
        </section>
      </div>
      <div
        v-else-if="groupMode === 'host'"
        class="archive-groups"
      >
        <div
          v-for="group in grouped"
          :key="group.hostname"
          class="archive-group"
        >
          <!--
            The header handles the click and the button beside the host name
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
          <div
            v-show="!isGroupCollapsed(group.hostname)"
            class="group-archives"
          >
            <ArchiveSelectorRow
              v-for="archive in group.archives"
              :key="archiveKey(archive)"
              :archive="archive"
              :selected="isSelected(archive)"
              :flat="false"
              :show-host="true"
              :repo-name="multiRepo ? archive.repo_name : null"
              :can-delete="canDelete"
              :deleting="isDeleting(archive)"
              @select="select(archive)"
              @delete="emit('delete', archive)"
            />
          </div>
        </div>
      </div>
      <div
        v-else
        class="archive-flat-list"
      >
        <ArchiveSelectorRow
          v-for="archive in ordered"
          :key="archiveKey(archive)"
          :archive="archive"
          :selected="isSelected(archive)"
          :flat="true"
          :show-host="multiHost"
          :repo-name="multiRepo ? archive.repo_name : null"
          :can-delete="canDelete"
          :deleting="isDeleting(archive)"
          @select="select(archive)"
          @delete="emit('delete', archive)"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.archive-count {
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  color: var(--text-secondary);
  background: var(--bg-hover);
  border-radius: var(--radius-pill);
  padding: var(--space-1) var(--space-4);
}

.archive-loading {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-5) var(--space-6);
}

.archive-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.archive-search {
  flex: 1 1 10rem;
  min-width: 8rem;
}

.archive-sort-select {
  flex: none;
}

.archive-group-toggle {
  flex: none;
}

/* A repository per option: let a long list scroll in place rather than push
   the pane wider than a phone. */
.archive-repo-filter {
  max-width: 100%;
  overflow-x: auto;
}

.archive-day-group + .archive-day-group {
  border-top: 1px solid var(--border);
}

.archive-day-label {
  margin: 0;
  padding: var(--space-5) var(--space-6) var(--space-3);
  border-bottom: 1px solid var(--border-subtle);
}

.archive-no-match {
  padding-left: var(--space-6);
}

.archive-groups,
.archive-flat-list {
  max-height: 32rem;
  overflow-y: auto;
}

.archive-group:last-child .group-archives > :last-child {
  border-bottom: none;
}

.group-header {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-4) var(--space-6);
  background: var(--bg-hover);
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
}

/* The header sits on `--bg-hover` already, so hover moves the ink rather
   than the ground - there is no step left between the two backgrounds. */
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

.group-toggle:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
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

.group-count {
  flex: none;
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  font-weight: 500;
  color: var(--text-secondary);
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  padding: 0 var(--space-4);
}

.group-size {
  flex: none;
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  font-weight: 500;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
