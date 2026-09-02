<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import AgentBackupRow from './AgentBackupRow.vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ReportRow } from '../types/report'

export type BackupFilter = 'all' | 'success' | 'warning' | 'failed'

/**
 * The run log: every backup run regardless of status, one line each, with
 * expandable detail for a warned or failed one - what used to be an agent's
 * "Backups" tab, now shared with a schedule's "Logs" tab too, since neither
 * page's log view was ever agent-specific.
 *
 * `reports` is only the rows loaded so far - the server has always capped a
 * bare fetch, and `total` is how the caller says whether "Load more" has
 * anything left to fetch. The status counts above the rows are therefore
 * counts of what's loaded, not of `total`; a filter can undercount until
 * every page is in.
 */
const props = defineProps<{
  reports: readonly ReportRow[]
  total: number
  loadingMore: boolean
  filter: BackupFilter
  sortAscending: boolean
  expandedReportId: number | null
  highlightedArchiveName: string | undefined
  pinnedReportId: number | null
}>()

const emit = defineEmits<{
  'update:filter': [value: BackupFilter]
  'update:sortAscending': [value: boolean]
  toggle: [report: ReportRow]
  open: [report: ReportRow]
  loadMore: []
}>()

const hasMore = computed(() => props.reports.length < props.total)

function countOf(status: BackupFilter): number {
  if (status === 'all') return props.reports.length
  return props.reports.filter((r) => normalizeBackupStatus(r.status) === status).length
}

/**
 * Counts live in the labels so the filter doubles as a summary - the shape of
 * a month of history without having to click through all four segments.
 */
const filterOptions = computed<SegmentedOption<BackupFilter>[]>(() => [
  { value: 'all', label: `All ${countOf('all')}` },
  { value: 'success', label: `Success ${countOf('success')}` },
  { value: 'warning', label: `Warning ${countOf('warning')}` },
  { value: 'failed', label: `Failed ${countOf('failed')}` },
])

const visible = computed(() => {
  const filtered =
    props.filter === 'all'
      ? [...props.reports]
      : props.reports.filter((r) => normalizeBackupStatus(r.status) === props.filter)
  return filtered.sort((a, b) => {
    const diff = new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
    return props.sortAscending ? -diff : diff
  })
})
</script>

<template>
  <div class="backups-tab">
    <div class="toolbar backups-toolbar">
      <BaseSegmented
        :model-value="filter"
        :options="filterOptions"
        label="Filter backups by status"
        @update:model-value="emit('update:filter', $event)"
      />
      <button
        class="btn btn-sm btn-ghost backups-sort"
        type="button"
        @click="emit('update:sortAscending', !sortAscending)"
      >
        {{ sortAscending ? 'Oldest first' : 'Newest first' }}
      </button>
    </div>

    <div
      v-if="visible.length === 0"
      class="state-msg"
    >
      {{
        reports.length === 0
          ? 'No backup reports available.'
          : 'No backups match the current filter.'
      }}
    </div>
    <div
      v-else
      class="rows"
    >
      <AgentBackupRow
        v-for="r in visible"
        :key="r.id"
        :report="r"
        :expanded="expandedReportId === r.id"
        :highlighted="r.archive_name === highlightedArchiveName || r.id === pinnedReportId"
        show-detail
        @toggle="emit('toggle', r)"
        @open="emit('open', r)"
      />
    </div>

    <div
      v-if="reports.length > 0"
      class="load-more-row"
    >
      <button
        v-if="hasMore"
        class="btn btn-sm btn-ghost"
        type="button"
        :disabled="loadingMore"
        @click="emit('loadMore')"
      >
        {{ loadingMore ? 'Loading...' : `Load ${Math.min(50, total - reports.length)} more` }}
      </button>
      <span class="load-more-note">Showing {{ reports.length }} of {{ total }} runs</span>
    </div>
  </div>
</template>

<style scoped>
.backups-tab {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.backups-toolbar {
  justify-content: flex-start;
}

.backups-sort {
  margin-left: auto;
}

.load-more-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.load-more-note {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>
