<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import ArchiveExplorer from './ArchiveExplorer.vue'
import PagerLoadMore from './PagerLoadMore.vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
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
  /** Every report loaded so far for this schedule; only the archived ones are listed. */
  reports: ReportRow[]
  /** The schedule's true report count, for the "more may exist" note below. */
  total: number
  loading: boolean
  loadingMore: boolean
  error: string | null
  agents: Map<number, AgentRow>
  /** The schedule's primary target, and the repository this tab opens on. */
  repoId: number | null
  repoName?: string
  /**
   * Every repository the schedule writes into, in write order. A schedule
   * with more than one gets the scope selector below; browsing, restoring and
   * deleting then all act against whichever it names.
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

const repoChoices = computed<readonly ScheduleRepoOption[]>(() => props.repoOptions ?? [])

/**
 * The repository picked in the scope selector, or null while the tab is still
 * showing the schedule's primary target - so a schedule whose targets change
 * under a mounted tab follows them rather than pinning a stale id.
 */
const scopedRepoId = ref<number | null>(null)

/**
 * The repository this tab browses. Everything the explorer does - listing,
 * indexing, downloading, restoring and deleting - is scoped to it, so the
 * archive list and the destructive buttons can never disagree about which
 * copy they are acting on.
 */
const activeRepoId = computed<number | null>(() => {
  const scoped = scopedRepoId.value
  if (scoped !== null && repoChoices.value.some((o) => o.id === scoped)) return scoped
  return props.repoId
})

const activeRepoName = computed<string>(() => {
  const match = repoChoices.value.find((o) => o.id === activeRepoId.value)
  return match?.name ?? props.repoName ?? ''
})

/** One repository is the schedule's own context; a selector would say nothing. */
const showRepoScope = computed(() => repoChoices.value.length > 1)

const scopeModel = computed<number | null>({
  get: () => activeRepoId.value,
  set: (repoId) => {
    if (repoId === null) return
    scopedRepoId.value = repoId
    // The selection names an archive in the repository being left behind, and
    // the pane it feeds browses `activeRepoId` - keeping it would point the
    // file tree at one repository while the list shows another.
    selected.value = null
  },
})

/**
 * The Overview's "browse this archive" jump hands this tab a report, and on a
 * multi-repository schedule that report is as likely to belong to the
 * secondary target as to the primary one. Following it is what makes the jump
 * land on the archive the reader clicked rather than on an empty pane.
 *
 * `repoChoices` is watched alongside the selection, not just read inside, and
 * that is the whole point: the targets and the reports are two independent
 * requests, so a jump can arrive while the target list is still empty. Keyed
 * on the selection alone, the callback would bail out for an unknown
 * repository and never run again - the selection does not change a second
 * time - leaving the scope pinned to the primary target for good and the pane
 * showing a repository nobody asked for.
 */
watch(
  [() => selected.value?.repo_id ?? null, repoChoices],
  ([repoId]) => {
    if (repoId === null || repoId === activeRepoId.value) return
    if (!repoChoices.value.some((o) => o.id === repoId)) return
    scopedRepoId.value = repoId
  },
  { immediate: true },
)

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
 */
const archivesByRepo = computed<Map<number, ArchiveEntry[]>>(() => {
  // The reports are the whole schedule's, so a multi-target schedule's rows
  // cover every repository it writes into - and the same archive name exists
  // in each of them. They are kept apart because this tab browses and deletes
  // against one repository: a row listed under the wrong one would send its
  // delete to a repository the reader never chose.
  const byRepo = new Map<number, Map<string, ArchiveEntry>>()
  for (const r of props.reports) {
    if (r.archive_name == null) continue
    const status = normalizeBackupStatus(r.status)
    if (status !== 'success' && status !== 'warning') continue
    const forRepo = byRepo.get(r.repo_id) ?? new Map<string, ArchiveEntry>()
    byRepo.set(r.repo_id, forRepo)
    // One archive, one row: a re-run that wrote to the same name leaves two
    // reports behind, and the list is keyed by archive name.
    if (forRepo.has(r.archive_name)) continue
    forRepo.set(r.archive_name, {
      name: r.archive_name,
      start: r.started_at,
      hostname: hostFor(r),
      comment: '',
      original_size: r.original_size,
      deduplicated_size: r.deduplicated_size,
      matched: true,
      agent_hostname: hostFor(r),
      agent_domain: domainFor(r),
    })
  }
  return new Map([...byRepo].map(([repoId, entries]) => [repoId, [...entries.values()]]))
})

const archives = computed<ArchiveEntry[]>(() =>
  activeRepoId.value === null ? [] : (archivesByRepo.value.get(activeRepoId.value) ?? []),
)

/** Beside each option, so the reader can see where the copies actually are. */
function archiveCount(repoId: number): number {
  return archivesByRepo.value.get(repoId)?.length ?? 0
}

/** The explorer selects archives; the view's selection is the report behind one. */
const selectedArchive = computed<ArchiveEntry | null>({
  get: () => {
    const r = selected.value
    if (!r || r.archive_name == null) return null
    return archives.value.find((a) => a.name === r.archive_name) ?? null
  },
  set: (archive) => {
    selected.value =
      archive === null
        ? null
        : (props.reports.find(
            (r) => r.archive_name === archive.name && r.repo_id === activeRepoId.value,
          ) ?? null)
  },
})

const reload = (): Promise<unknown> => props.reload?.() ?? Promise.resolve()

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

/**
 * A search filter or collapsed host group typed against the repository being
 * left behind would carry into the next one and silently hide its archives -
 * the list would read "No archives match the search" while the selector's own
 * label still counts them. `useArchiveList.reset()` exists for exactly this
 * ("when the repository changes"), and `RepoArchivesTab` calls it from its
 * `repoId` watcher for the same reason.
 *
 * Watching `activeRepoId` rather than resetting inside the selector's setter
 * covers both ways the scope moves - the selector and the Overview's
 * "browse this archive" jump - from one place. Resetting clears the filter and
 * collapse state only, never the selection, so the jump still lands on the
 * archive that caused it.
 */
watch(activeRepoId, () => {
  explorer.value?.resetList()
})

/**
 * The view owns the WebSocket subscription, so the three events that clear a
 * "deleting..." marker are forwarded through here to the explorer. Without the
 * repo-idle one in particular, a delete that borg subsequently failed would
 * leave its row disabled with no way back short of a page reload.
 *
 * `activeRepoId` rides along because those events carry a `repo_id` the view
 * has to match them against: the scope selector means the tab is no longer
 * necessarily browsing the schedule's primary repository, and filtering on the
 * primary one would drop every event for a secondary target - stranding a
 * failed delete's row on "Deleting..." until a page reload.
 */
defineExpose({
  activeRepoId,
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
    :repo-id="activeRepoId"
    :repo-name="activeRepoName"
    :archives="archives"
    :loading="loading"
    :error="error"
    :is-admin="isAdmin ?? false"
    :reload="reload"
    :refresh-after-delete="reload"
    empty-title="No archives"
    :empty-description="
      showRepoScope
        ? `No backup archives in ${activeRepoName} for this schedule.`
        : 'No backup archives found for this schedule.'
    "
  >
    <!--
      In the list header rather than beside the archives: the repository is
      what the whole pane is scoped to - the file tree, the restore and the
      delete included - not one more way to filter the rows under it.
    -->
    <template
      v-if="showRepoScope"
      #actions
    >
      <label
        class="group-label"
        for="schedule-repo-scope"
        >Repository</label
      >
      <select
        id="schedule-repo-scope"
        v-model="scopeModel"
        class="input select-input"
      >
        <option
          v-for="option in repoChoices"
          :key="option.id"
          :value="option.id"
        >
          {{ option.name }} ({{ archiveCount(option.id) }})
        </option>
      </select>
    </template>
  </ArchiveExplorer>
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
