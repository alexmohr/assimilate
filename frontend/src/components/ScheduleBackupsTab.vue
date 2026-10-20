<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, useTemplateRef, watch } from 'vue'
import ArchiveExplorer from './ArchiveExplorer.vue'
import PagerLoadMore from './PagerLoadMore.vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { sameArchive, type RepoArchive } from '../utils/repoArchive'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'
import type { ScheduleRepoOption } from '../types/schedule'

/**
 * The archive browser for one schedule: its successful runs on the left, the
 * selected run's file tree on the right.
 *
 * The runs are mapped to archive entries and handed to the same
 * `ArchiveExplorer` the repository and archives screens render, so this tab
 * gains host grouping, search, sort and - for an admin - deletion, none of
 * which its own four-column table offered.
 *
 * Unlike `AgentArchivesTab`, which reads straight from a repository's real
 * (uncapped) archive list, this tab has to derive archives from report
 * history - there is no server endpoint that lists a repository's archives
 * scoped to one schedule, only reports. `reports` is therefore only the rows
 * loaded so far (the same paged, capped list the Logs tab shares), and
 * `total` is how this tab knows there may be older archives it hasn't
 * fetched yet.
 */
const props = defineProps<{
  /**
   * Which schedule these reports belong to. Only used to notice that it
   * changed: the view keeps this tab mounted across `/schedules/:id` changes
   * (the tab is gated on which tab is open, not on the route), so without it a
   * repository filter chosen for one schedule silently carries into the next.
   */
  scheduleId: string
  /** Every report loaded so far for this schedule; only the archived ones are listed. */
  reports: ReportRow[]
  /** The schedule's true report count, for the "more may exist" note below. */
  total: number
  loading: boolean
  loadingMore: boolean
  error: string | null
  agents: Map<number, AgentRow>
  /** The schedule's primary target. */
  repoId: number | null
  repoName?: string
  /**
   * Every repository the schedule writes into, in write order. A schedule
   * with more than one lists all their archives in one list, each row tagged
   * with its repository and a filter to narrow it to one.
   */
  repoOptions?: readonly ScheduleRepoOption[]
  isAdmin?: boolean
  /** Re-fetches the reports, so a deleted archive leaves the list. */
  reload?: () => Promise<unknown>
}>()

const emit = defineEmits<{ loadMore: [] }>()

const hasMoreReports = computed(() => props.reports.length < props.total)

/** The view owns the selection so it can clear it when the route changes. */
const selected = defineModel<ReportRow | null>('selected', { required: true })

/** The names to tag rows with; the primary target's falls back to the schedule's own. */
const repoNames = computed<Map<number, string>>(() => {
  const names = new Map((props.repoOptions ?? []).map((o) => [o.id, o.name] as const))
  if (props.repoId !== null && !names.has(props.repoId)) {
    names.set(props.repoId, props.repoName ?? '')
  }
  return names
})

/** Write order: the primary target's archives first, as the schedule lists them. */
const repoOrder = computed<number[]>(() => [...repoNames.value.keys()])

function hostFor(report: ReportRow): string {
  const agent = props.agents.get(report.agent_id ?? 0)
  return agent?.hostname ?? report.hostname ?? ''
}

function domainFor(report: ReportRow): string | null {
  return props.agents.get(report.agent_id ?? 0)?.domain ?? null
}

/**
 * A successful run is an archive: the repository's own archive list is built
 * from exactly these rows server-side, so mapping them here keeps the two
 * screens showing the same thing without a second fetch.
 *
 * A multi-target schedule's rows cover every repository it writes into - and
 * the same archive name exists in each of them - so every entry carries its
 * repository: a row listed under the wrong one would send its delete to a
 * repository the reader never chose.
 */
const archives = computed<RepoArchive[]>(() => {
  const seen = new Set<string>()
  const entries: RepoArchive[] = []
  for (const r of props.reports) {
    if (r.archive_name == null) continue
    const status = normalizeBackupStatus(r.status)
    if (status !== 'success' && status !== 'warning') continue
    // One archive, one row: a re-run that wrote to the same name leaves two
    // reports behind, and the list is keyed by repository and archive name.
    const key = `${r.repo_id}/${r.archive_name}`
    if (seen.has(key)) continue
    seen.add(key)
    entries.push({
      name: r.archive_name,
      start: r.started_at,
      hostname: hostFor(r),
      comment: '',
      original_size: r.original_size,
      deduplicated_size: r.deduplicated_size,
      matched: true,
      agent_hostname: hostFor(r),
      agent_domain: domainFor(r),
      repo_id: r.repo_id,
      repo_name: repoNames.value.get(r.repo_id) ?? '',
    })
  }
  const order = (repoId: number): number => {
    const index = repoOrder.value.indexOf(repoId)
    return index === -1 ? repoOrder.value.length : index
  }
  // Stable: within a repository the reports' own order stands.
  return entries.sort((a, b) => order(a.repo_id) - order(b.repo_id))
})

/** The explorer selects archives; the view's selection is the report behind one. */
const selectedArchive = computed<RepoArchive | null>({
  get: () => {
    const r = selected.value
    if (!r || r.archive_name == null) return null
    const wanted = { repo_id: r.repo_id, name: r.archive_name }
    return archives.value.find((a) => sameArchive(a, wanted)) ?? null
  },
  set: (archive) => {
    selected.value =
      archive === null
        ? null
        : (props.reports.find(
            (r) => r.archive_name === archive.name && r.repo_id === archive.repo_id,
          ) ?? null)
  },
})

const reload = (): Promise<unknown> => props.reload?.() ?? Promise.resolve()

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

// A search or repository filter typed for one schedule says nothing about the
// next one, even where both write into the same repository.
watch(
  () => props.scheduleId,
  () => {
    explorer.value?.resetList()
  },
)

/**
 * The view owns the WebSocket subscription, so the three events that clear a
 * "deleting..." marker are forwarded through here to the explorer. Without the
 * repo-idle one in particular, a delete that borg subsequently failed would
 * leave its row disabled with no way back short of a page reload.
 */
defineExpose({
  onArchiveDeleted(name: string, repoId: number): void {
    explorer.value?.onArchiveDeleted(name, repoId)
  },
  onDataChanged(): void {
    explorer.value?.onDataChanged()
  },
  onRepoIdle(repoId: number): void {
    explorer.value?.onRepoIdle(repoId)
  },
})
</script>

<template>
  <ArchiveExplorer
    ref="explorer"
    v-model:selected="selectedArchive"
    :archives="archives"
    :loading="loading"
    :error="error"
    :is-admin="isAdmin ?? false"
    :reload="reload"
    :refresh-after-delete="reload"
    empty-title="No archives"
    :empty-description="
      (repoOptions?.length ?? 0) > 1
        ? 'No backup archives in any of this schedule\'s repositories yet.'
        : 'No backup archives found for this schedule.'
    "
  />
  <PagerLoadMore
    v-if="hasMoreReports"
    :loaded="reports.length"
    :total="total"
    :loading-more="loadingMore"
    load-label="more runs"
    @load-more="emit('loadMore')"
  >
    Only this schedule's {{ reports.length }} most recent runs (of {{ total }}) have been checked
    for archives - older ones may exist.
  </PagerLoadMore>
</template>

<style scoped>
:deep(.pager-load-more) {
  margin-top: var(--space-4);
}
</style>
