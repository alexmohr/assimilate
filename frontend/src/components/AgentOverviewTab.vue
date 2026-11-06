<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { CircleAlert, CircleCheck, CircleHelp, CircleX } from '@lucide/vue'
import { formatBytes, formatDateShort, formatDuration, relativeTime } from '../utils/format'
import { byFinishedDesc, normalizeBackupStatus, filterSettledReports } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import { buildRepoOverview, type RepoOverviewState } from '../utils/repoOverview'
import BackupProgressCard from './BackupProgressCard.vue'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'
import type { ScheduleRow } from '../types/schedule'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

/**
 * The landing tab for an agent, which is what you see when a backup failed at
 * 3am. It answers one question - are my backups OK - and then shows the
 * evidence: a verdict line that is always present, the last and next run, and
 * one row per repository with its own outcome strip. Schedules and archives
 * have tabs of their own, so they are not previewed here, and every setting
 * that used to stack up here lives under Settings.
 */

/** Elapsed seconds are computed by the parent, which owns the clock. */
export interface LiveBackup {
  targetName: string
  repoId: number | null
  archiveName: string | null
  elapsedSecs: number
  progress: { nfiles: number; originalSize: number; currentPath: string } | null
}

const props = defineProps<{
  agent: AgentRow
  repos: readonly { id: number; name: string }[]
  schedules: readonly ScheduleRow[]
  health: readonly ScheduleHealthEntry[]
  reports: readonly ReportRow[]
  liveBackups: readonly LiveBackup[]
  cancellingRepoIds: readonly number[]
  repoNameFor: (schedule: ScheduleRow) => string
}>()

const emit = defineEmits<{
  /** Jump to a failed run's output on the Logs tab, which is where its error is rendered. */
  openReportDetail: [report: ReportRow]
  cancelBackup: [repoId: number]
}>()

function cancelLive(backup: LiveBackup): void {
  if (backup.repoId !== null) emit('cancelBackup', backup.repoId)
}

const settledReports = computed(() => filterSettledReports(props.reports))

const lastBackup = computed(() => [...settledReports.value].sort(byFinishedDesc)[0] ?? null)

/**
 * The soonest upcoming run across every schedule targeting this agent.
 * Disabled schedules are skipped: the server still reports a `next_run_at`
 * for them, but it will not fire.
 */
const nextRun = computed(() => {
  const upcoming = props.schedules
    .filter((s) => s.enabled && s.next_run_at !== null)
    .map((s) => ({ schedule: s, at: new Date(s.next_run_at as string).getTime() }))
    .sort((a, b) => a.at - b.at)
  return upcoming[0] ?? null
})

const repoRows = computed(() =>
  buildRepoOverview({
    repos: props.repos,
    reports: props.reports,
    schedules: props.schedules,
    health: props.health,
    runningRepoIds: props.liveBackups.flatMap((b) => (b.repoId === null ? [] : [b.repoId])),
  }),
)

interface Attention {
  key: string
  severity: 'warning' | 'danger'
  label: string
  message: string
  scheduleId: number | null
  /** Set on a failed run, so its output is one click away. */
  report: ReportRow | null
}

/**
 * Rendered only when there is something to say. An always-present "all good"
 * banner trains people to skip the place a real warning will appear.
 */
const attention = computed<Attention[]>(() => {
  const items: Attention[] = []
  if (!props.agent.is_imported && props.agent.is_connected === false) {
    items.push({
      key: 'offline',
      severity: 'warning',
      label: 'Offline',
      message: props.agent.last_seen_at
        ? `Agent last checked in ${relativeTime(props.agent.last_seen_at)}.`
        : 'Agent has never checked in.',
      scheduleId: null,
      report: null,
    })
  }
  for (const entry of props.health) {
    if (!entry.is_overdue) continue
    const schedule = props.schedules.find((s) => s.id === entry.schedule_id)
    const name = schedule?.name || entry.target_name
    items.push({
      key: `overdue-${entry.schedule_id}-${entry.repo_id}`,
      severity: 'warning',
      label: 'Overdue',
      message: entry.last_backup_at
        ? `${name} has not run since ${relativeTime(entry.last_backup_at)}.`
        : `${name} has never run.`,
      scheduleId: entry.schedule_id,
      report: null,
    })
  }
  // Per repository, not from the agent's single newest run: a failure in one
  // repository must not be hidden by a later success in another.
  for (const row of repoRows.value) {
    const last = row.lastRun
    if (row.state !== 'failed' || !last) continue
    items.push({
      key: `failed-${last.id}`,
      severity: 'danger',
      label: 'Failed',
      message: `The last backup to ${row.name} failed ${relativeTime(last.finished_at)}.`,
      scheduleId: last.schedule_id,
      report: last,
    })
  }
  return items
})

type Tone = 'success' | 'warning' | 'danger' | 'neutral'

/**
 * Always present, unlike the attention rows inside it. The page's job is to
 * answer "are my backups OK", and an answer that sometimes is not there reads
 * as an answer of its own. What still must not happen is "all good" worn by
 * an agent that has never run anything, which is why that case is neutral.
 */
const verdict = computed<{ tone: Tone; title: string; detail: string | null }>(() => {
  const items = attention.value
  if (items.length > 0) {
    const danger = items.some((i) => i.severity === 'danger')
    return {
      tone: danger ? 'danger' : 'warning',
      title: items.length === 1 ? 'Needs attention' : `${items.length} things need attention`,
      detail: null,
    }
  }
  const recent = settledReports.value.length
  if (recent === 0) {
    return { tone: 'neutral', title: 'No backups yet', detail: 'Nothing has run on this agent.' }
  }
  const sample = [...settledReports.value].sort(byFinishedDesc).slice(0, RUN_WINDOW)
  const warnings = sample.filter((r) => normalizeBackupStatus(r.status) === 'warning').length
  const failed = sample.filter((r) => normalizeBackupStatus(r.status) === 'failed').length
  const detail =
    failed > 0
      ? `${failed} of the last ${sample.length} runs failed.`
      : warnings > 0
        ? `${warnings} of the last ${sample.length} runs finished with warnings.`
        : `All ${sample.length} recent runs were clean.`
  return { tone: 'success', title: 'All backups are healthy', detail }
})

/** How many runs the verdict's summary line looks back over. */
const RUN_WINDOW = 20

const REPO_STATE_LABEL: Record<RepoOverviewState, string> = {
  running: 'Running now',
  failed: 'Last run failed',
  overdue: 'Overdue',
  warning: 'Warnings',
  success: 'Healthy',
  none: 'Never ran',
}

const REPO_STATE_TONE: Record<RepoOverviewState, Tone | 'info'> = {
  running: 'info',
  failed: 'danger',
  overdue: 'warning',
  warning: 'warning',
  success: 'success',
  none: 'neutral',
}
</script>

<template>
  <div class="overview-tab">
    <!-- A backup in flight is the most current thing on the page, so it leads. -->
    <BackupProgressCard
      v-for="b in liveBackups"
      :key="b.targetName"
      :badge="b.targetName"
      :repo-id="b.repoId"
      :archive-name="b.archiveName"
      :elapsed-secs="b.elapsedSecs"
      :estimated-remaining-secs="null"
      :progress="b.progress"
      :cancel-loading="b.repoId !== null && cancellingRepoIds.includes(b.repoId)"
      :waiting-for="agent.is_connected === false ? (agent.display_name ?? agent.hostname) : null"
      clamp-path
      @cancel="cancelLive(b)"
    />

    <div
      class="verdict"
      :class="`verdict--${verdict.tone}`"
    >
      <div class="verdict-head">
        <span class="verdict-icon">
          <CircleCheck
            v-if="verdict.tone === 'success'"
            :size="16"
            aria-hidden="true"
          />
          <CircleX
            v-else-if="verdict.tone === 'danger'"
            :size="16"
            aria-hidden="true"
          />
          <CircleAlert
            v-else-if="verdict.tone === 'warning'"
            :size="16"
            aria-hidden="true"
          />
          <CircleHelp
            v-else
            :size="16"
            aria-hidden="true"
          />
        </span>
        <div class="verdict-text">
          <span class="verdict-title">{{ verdict.title }}</span>
          <span
            v-if="verdict.detail"
            class="stat-sub"
            >{{ verdict.detail }}</span
          >
        </div>
      </div>
      <div
        v-if="attention.length > 0"
        class="attention"
      >
        <div
          v-for="item in attention"
          :key="item.key"
          class="attention-row"
        >
          <span
            class="badge"
            :class="item.severity === 'danger' ? 'badge--danger' : 'badge--warning'"
            >{{ item.label }}</span
          >
          <span class="attention-message">{{ item.message }}</span>
          <button
            v-if="item.report"
            class="btn btn-sm btn-ghost attention-action"
            type="button"
            @click="emit('openReportDetail', item.report)"
          >
            View error
          </button>
          <RouterLink
            v-if="item.scheduleId !== null"
            class="attention-link"
            :to="`/schedules/${item.scheduleId}`"
          >
            Open schedule
          </RouterLink>
        </div>
      </div>
    </div>

    <div class="facts">
      <div class="fact">
        <span class="stat-label">Last backup</span>
        <span
          v-if="lastBackup"
          class="stat-value stat-value--lg"
        >
          {{ relativeTime(lastBackup.finished_at) }}
          <span
            class="badge"
            :class="backupStatusBadgeClass(lastBackup.status)"
            >{{ normalizeBackupStatus(lastBackup.status) }}</span
          >
        </span>
        <span
          v-else
          class="stat-value stat-value--lg stat-value--empty"
          >Never</span
        >
        <span
          v-if="lastBackup"
          class="stat-sub stat-sub--truncate"
        >
          {{ lastBackup.repo_name }} &middot; {{ formatDuration(lastBackup.duration_secs) }}
        </span>
      </div>
      <div class="fact">
        <span class="stat-label">Next run</span>
        <span
          v-if="nextRun"
          class="stat-value stat-value--lg"
          >{{ formatDateShort(nextRun.schedule.next_run_at) }}</span
        >
        <span
          v-else
          class="stat-value stat-value--lg stat-value--empty"
          >None scheduled</span
        >
        <span
          v-if="nextRun"
          class="stat-sub stat-sub--truncate"
        >
          {{ nextRun.schedule.name || repoNameFor(nextRun.schedule) }}
        </span>
      </div>
    </div>

    <section v-if="repoRows.length > 0">
      <div class="section-head">
        <h2 class="section-title">Repositories</h2>
      </div>
      <div class="rows">
        <RouterLink
          v-for="row in repoRows"
          :key="row.id"
          class="repo-row"
          :to="`/repos/${row.id}`"
          :data-repo-id="row.id"
        >
          <span
            class="repo-dot"
            :class="`repo-dot--${REPO_STATE_TONE[row.state]}`"
            aria-hidden="true"
          />
          <span class="repo-name">{{ row.name }}</span>
          <span
            class="badge"
            :class="`badge--${REPO_STATE_TONE[row.state]}`"
            >{{ REPO_STATE_LABEL[row.state] }}</span
          >
          <span class="repo-meta">
            <span v-if="row.lastRun">last {{ relativeTime(row.lastRun.finished_at) }}</span>
            <span v-if="row.lastGood && row.lastRun?.id !== row.lastGood.id">
              last good {{ relativeTime(row.lastGood.finished_at) }}
            </span>
            <span v-if="row.lastRun && row.state !== 'failed'">
              {{ formatBytes(row.lastRun.deduplicated_size) }} new
            </span>
            <span v-if="row.nextRunAt">next {{ formatDateShort(row.nextRunAt) }}</span>
          </span>
        </RouterLink>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* Base .overview-tab / .attention / .section-* / .rows shapes live in
   style.css, shared with ScheduleOverviewTab. What is this page's own: the
   verdict that wraps the attention rows, the ruled facts strip, and the
   repository row. */
.verdict {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-5) var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
}

.verdict--warning {
  border-color: var(--warning);
  background: var(--warning-subtle);
}

.verdict--danger {
  border-color: var(--danger);
  background: var(--danger-subtle);
}

.verdict-head {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
}

.verdict-icon {
  display: grid;
  place-items: center;
  flex: none;
  color: var(--text-muted);
}

.verdict--success .verdict-icon {
  color: var(--success);
}

.verdict--warning .verdict-icon {
  color: var(--warning);
}

.verdict--danger .verdict-icon {
  color: var(--danger);
}

.verdict-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.verdict-title {
  font-size: var(--fs-md);
  font-weight: 650;
}

/* The attention rows sit inside the verdict, which already is the box. */
.verdict .attention {
  border: none;
  background: none;
  padding: 0;
}

.attention-link,
.attention-action {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--accent);
}

/* Both actions travel together at the right edge; each taking its own
   auto margin split the free space and left the first one mid-row. */
.attention-action ~ .attention-link {
  margin-left: 0;
}

.facts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
}

.fact {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-5) var(--space-6);
  min-width: 0;
}

.fact + .fact {
  border-left: 1px solid var(--border);
}

.stat-value--empty {
  color: var(--text-muted);
  font-weight: 500;
}

.stat-sub--truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.repo-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-3) var(--space-5);
  padding: var(--space-5) var(--space-6);
  border-top: 1px solid var(--border-subtle);
  color: inherit;
  text-decoration: none;
  font-size: var(--fs-sm);
}

.repo-row:first-child {
  border-top: none;
}

.repo-row:hover {
  background: var(--bg-hover);
}

.repo-dot {
  width: var(--space-4);
  height: var(--space-4);
  border-radius: 50%;
  background: var(--text-muted);
}

.repo-dot--success {
  background: var(--success);
}

.repo-dot--warning {
  background: var(--warning);
}

.repo-dot--danger {
  background: var(--danger);
}

.repo-dot--info {
  background: var(--info);
}

.repo-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.repo-meta {
  grid-column: 1 / -1;
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-6);
  padding-left: var(--space-7);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
