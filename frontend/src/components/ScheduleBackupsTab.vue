<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, useTemplateRef } from 'vue'
import ArchiveExplorer from './ArchiveExplorer.vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

/**
 * The archive browser for one schedule: its successful runs on the left, the
 * selected run's file tree on the right.
 *
 * The runs are mapped to archive entries and handed to the same
 * `ArchiveExplorer` the repository and archives screens render, so this tab
 * gains host grouping, search, sort and - for an admin - deletion, none of
 * which its own four-column table offered.
 */
const props = defineProps<{
  /** Every report for this schedule; only the archived ones are listed. */
  reports: ReportRow[]
  loading: boolean
  error: string | null
  agents: Map<number, AgentRow>
  repoId: number | null
  repoName?: string
  isAdmin?: boolean
  /** Re-fetches the reports, so a deleted archive leaves the list. */
  reload?: () => Promise<unknown>
}>()

/** The view owns the selection so it can clear it when the route changes. */
const selected = defineModel<ReportRow | null>('selected', { required: true })

function hostFor(report: ReportRow): string {
  const agent = props.agents.get(report.agent_id ?? 0)
  return agent?.hostname ?? report.hostname ?? ''
}

/**
 * A successful run is an archive: the repository's own archive list is built
 * from exactly these rows server-side, so mapping them here keeps the two
 * screens showing the same thing without a second fetch.
 */
const archives = computed<ArchiveEntry[]>(() => {
  const byName = new Map<string, ArchiveEntry>()
  for (const r of props.reports) {
    if (r.archive_name == null) continue
    const status = normalizeBackupStatus(r.status)
    if (status !== 'success' && status !== 'warning') continue
    // One archive, one row: a re-run that wrote to the same name leaves two
    // reports behind, and the list is keyed by archive name.
    if (byName.has(r.archive_name)) continue
    byName.set(r.archive_name, {
      name: r.archive_name,
      start: r.started_at,
      hostname: hostFor(r),
      comment: '',
      original_size: r.original_size,
      deduplicated_size: r.deduplicated_size,
      matched: true,
      agent_hostname: hostFor(r),
    })
  }
  return [...byName.values()]
})

/** The explorer selects archives; the view's selection is the report behind one. */
const selectedArchive = computed<ArchiveEntry | null>({
  get: () => {
    const r = selected.value
    if (!r || r.archive_name == null) return null
    return archives.value.find((a) => a.name === r.archive_name) ?? null
  },
  set: (archive) => {
    selected.value =
      archive === null ? null : (props.reports.find((r) => r.archive_name === archive.name) ?? null)
  },
})

const reload = (): Promise<unknown> => props.reload?.() ?? Promise.resolve()

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

/**
 * The view owns the WebSocket subscription, so the three events that clear a
 * "deleting..." marker are forwarded through here to the explorer. Without the
 * repo-idle one in particular, a delete that borg subsequently failed would
 * leave its row disabled with no way back short of a page reload.
 */
defineExpose({
  onArchiveDeleted(name: string): void {
    explorer.value?.onArchiveDeleted(name)
  },
  onDataChanged(): void {
    explorer.value?.onDataChanged()
  },
  onRepoIdle(): void {
    explorer.value?.onRepoIdle()
  },
})
</script>

<template>
  <ArchiveExplorer
    ref="explorer"
    v-model:selected="selectedArchive"
    :repo-id="repoId"
    :repo-name="repoName"
    :archives="archives"
    :loading="loading"
    :error="error"
    :is-admin="isAdmin ?? false"
    :reload="reload"
    :refresh-after-delete="reload"
    empty-title="No archives"
    empty-description="No backup archives found for this schedule."
  />
</template>
