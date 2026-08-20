<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, toRef } from 'vue'
import { AlertTriangle, ChevronRight, Search } from '@lucide/vue'
import ArchiveSelectorRow from './ArchiveSelectorRow.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSkeleton from './BaseSkeleton.vue'
import EmptyState from './EmptyState.vue'
import { formatBytes } from '../utils/format'
import { ARCHIVE_SORT_OPTIONS, useArchiveList } from '../composables/useArchiveList'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

/**
 * The archive list pane: search, sort, host grouping, selection and the
 * per-archive delete control.
 *
 * The repository tab, the schedule's backups tab and the standalone archives
 * page each carried their own version of this - one grouped by host, one was a
 * four-column table, one a PrimeVue grid with four column filters - and only
 * one of them offered deletion. This is the single list all three render.
 */
type GroupMode = 'host' | 'flat'

const GROUP_OPTIONS: readonly SegmentedOption<GroupMode>[] = [
  { value: 'host', label: 'By host' },
  { value: 'flat', label: 'Flat' },
]

const props = withDefaults(
  defineProps<{
    archives: ArchiveEntry[]
    loading?: boolean
    error?: string | null
    /** Whether this user may delete, i.e. an admin on a real repository. */
    canDelete?: boolean
    /** Archives whose borg delete is already queued. */
    deletingNames?: readonly string[]
    /** Replaces the toolbar and rows, for a caller that filters the list to one archive. */
    hideControls?: boolean
    emptyTitle?: string
    emptyDescription?: string
  }>(),
  {
    loading: false,
    error: null,
    canDelete: false,
    deletingNames: () => [],
    hideControls: false,
    emptyTitle: 'No archives',
    emptyDescription: 'Archives appear here once a backup has run against this repository.',
  },
)

const emit = defineEmits<{ delete: [archive: ArchiveEntry] }>()

const selected = defineModel<ArchiveEntry | null>('selected', { required: true })

const {
  filter,
  sortMode,
  groupByHost,
  ordered,
  grouped,
  unmatchedCount,
  unmatchedHostnames,
  toggleGroup,
  isGroupCollapsed,
  reset,
} = useArchiveList(toRef(props, 'archives'))

const groupMode = computed<GroupMode>({
  get: () => (groupByHost.value ? 'host' : 'flat'),
  set: (mode) => {
    groupByHost.value = mode === 'host'
  },
})

const deleting = computed(() => new Set(props.deletingNames))

function isDeleting(name: string): boolean {
  return deleting.value.has(name)
}

function groupSize(archives: ArchiveEntry[]): string {
  return formatBytes(archives.reduce((total, a) => total + a.original_size, 0))
}

function select(archive: ArchiveEntry): void {
  selected.value = archive
}

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
          :options="GROUP_OPTIONS"
          label="Group archives"
        />
      </div>

      <div
        v-if="ordered.length === 0"
        class="state-msg state-msg--inline archive-no-match"
      >
        No archives match the search.
      </div>
      <div
        v-else-if="groupByHost"
        class="archive-groups"
      >
        <div
          v-for="group in grouped"
          :key="group.hostname"
          class="archive-group"
        >
          <button
            class="group-header"
            type="button"
            :class="{ collapsed: isGroupCollapsed(group.hostname) }"
            :aria-expanded="!isGroupCollapsed(group.hostname)"
            @click="toggleGroup(group.hostname)"
          >
            <ChevronRight
              class="group-chevron"
              :size="14"
            />
            <span
              class="group-hostname"
              :class="{ 'group-unmatched': !group.matched }"
              >{{ group.hostname }}</span
            >
            <AlertTriangle
              v-if="!group.matched"
              class="match-icon match-warn"
              :size="12"
            />
            <span class="group-count">{{ group.archives.length }}</span>
            <span class="group-size">{{ groupSize(group.archives) }}</span>
          </button>
          <div
            v-show="!isGroupCollapsed(group.hostname)"
            class="group-archives"
          >
            <ArchiveSelectorRow
              v-for="archive in group.archives"
              :key="archive.name"
              :archive="archive"
              :selected="selected?.name === archive.name"
              :flat="false"
              :can-delete="canDelete"
              :deleting="isDeleting(archive.name)"
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
          :key="archive.name"
          :archive="archive"
          :selected="selected?.name === archive.name"
          :flat="true"
          :can-delete="canDelete"
          :deleting="isDeleting(archive.name)"
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
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-4) var(--space-6);
  background: var(--bg-hover);
  border: none;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  font: inherit;
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

.group-chevron {
  flex: none;
  color: var(--text-muted);
  transform: rotate(90deg);
  transition: transform var(--duration-base);
}

.group-header.collapsed .group-chevron {
  transform: rotate(0deg);
}

.group-hostname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
