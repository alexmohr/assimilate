<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatDateShort, formatDuration } from '../utils/format'
import { scheduleRepoRuns } from '../utils/scheduleRepos'
import {
  RUN_OUTCOME_LABELS,
  SCHEDULE_RUN_WINDOW,
  countOutcomes,
  groupScheduleRuns,
  isCompleted,
  type RunOutcome,
  type ScheduleRun,
} from '../utils/scheduleRuns'
import ScheduleRunDetail from './ScheduleRunDetail.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRepoOption } from '../types/schedule'

/**
 * A schedule's recent runs: one small pill per run, coloured by its worst
 * outcome, and below them the picked run's detail - every host's result and,
 * a click further, its output. The newest run is picked until another is.
 *
 * A run is an occurrence, not a report: one run of a schedule with three hosts
 * leaves three reports, and a strip of reports could not say whether two red
 * cells were one bad night or two.
 */
const props = defineProps<{
  /** Newest first, as every reports endpoint returns them. */
  reports: readonly ReportRow[]
  /** Every repository this schedule writes into, in write order. */
  repoOptions: readonly ScheduleRepoOption[]
  agents: ReadonlyMap<number, AgentRow>
}>()

const emit = defineEmits<{
  openReportDetail: [report: ReportRow]
  openLogs: []
}>()

/** One repository is the page's own context; naming it on every row is noise. */
const multiRepo = computed(() => props.repoOptions.length > 1)

/** Every run, so a pill on one repository's strip can always be resolved. */
const allRuns = computed(() => groupScheduleRuns(props.reports, Number.POSITIVE_INFINITY))
const runs = computed(() => allRuns.value.slice(0, SCHEDULE_RUN_WINDOW))

/**
 * One strip per repository on a schedule that writes to several: a merged
 * strip cannot say whether a run of failures is one repository having a bad
 * week or every copy of the backup going down at once. Each pill there is that
 * repository's share of a run, and picks the whole run.
 */
const repoStrips = computed(() =>
  scheduleRepoRuns(props.repoOptions, props.reports).map((entry) => ({
    repo: entry.repo,
    runs: groupScheduleRuns(entry.reports),
  })),
)

const selectedKey = ref<string | null>(null)

/** The picked run, or the newest while nothing is - or once the pick has aged out. */
const selectedRun = computed<ScheduleRun | null>(
  () => allRuns.value.find((r) => r.key === selectedKey.value) ?? allRuns.value[0] ?? null,
)

const counts = computed(() => countOutcomes(runs.value))
const completed = computed(() => runs.value.filter(isCompleted))

const headline = computed(() =>
  runs.value.length === 0
    ? 'No runs yet'
    : `${completed.value.length} of ${runs.value.length} completed`,
)

const averageText = computed<string | null>(() => {
  if (completed.value.length === 0) return null
  const total = completed.value.reduce((sum, r) => sum + r.durationSecs, 0)
  return `avg ${formatDuration(Math.round(total / completed.value.length))}`
})

interface LegendEntry {
  outcome: RunOutcome
  text: string
}

/** Cancelled runs are rare enough to be named only when there are some. */
const legend = computed<LegendEntry[]>(() => {
  const c = counts.value
  const entries: LegendEntry[] = [
    { outcome: 'success', text: `${c.success} succeeded` },
    { outcome: 'warning', text: `${c.warning} ${c.warning === 1 ? 'warning' : 'warnings'}` },
    { outcome: 'skipped', text: `${c.skipped} skipped` },
    { outcome: 'failed', text: `${c.failed} failed` },
  ]
  if (c.cancelled > 0) entries.push({ outcome: 'cancelled', text: `${c.cancelled} cancelled` })
  return entries
})

/** Oldest on the left, so the strip reads left-to-right like a timeline. */
function oldestFirst(list: readonly ScheduleRun[]): ScheduleRun[] {
  return [...list].reverse()
}

/**
 * When the oldest and the newest pill ran, under either end of the strip - or
 * just the one date, for a strip of one.
 */
function axis(newestFirst: readonly ScheduleRun[]): { first: string; last: string | null } | null {
  const newest = newestFirst[0]
  const oldest = newestFirst[newestFirst.length - 1]
  if (!newest || !oldest) return null
  return {
    first: formatDateShort(oldest.finishedAt),
    last: newest === oldest ? null : formatDateShort(newest.finishedAt),
  }
}

function pillLabel(run: ScheduleRun): string {
  return `${formatDateShort(run.finishedAt)}: ${RUN_OUTCOME_LABELS[run.status]}`
}

function hostLabel(report: ReportRow): string {
  const agent = props.agents.get(report.agent_id)
  return agent?.display_name ?? agent?.hostname ?? report.hostname ?? `#${report.agent_id}`
}

/**
 * The repository a run wrote into. A run against a repository the schedule no
 * longer writes to still reports the name it was given.
 */
function repoLabel(report: ReportRow): string | null {
  if (!multiRepo.value) return null
  const option = props.repoOptions.find((o) => o.id === report.repo_id)
  return option?.name ?? report.repo_name ?? `#${report.repo_id}`
}
</script>

<template>
  <div class="panel">
    <div class="runs-head">
      <div class="runs-headline">
        <span class="stat-label">Recent runs</span>
        <span class="stat-value stat-value--lg">{{ headline }}</span>
      </div>
      <div
        v-if="runs.length > 0"
        class="chart-legend chart-legend--start"
      >
        <span
          v-for="entry in legend"
          :key="entry.outcome"
          class="chart-legend-item"
        >
          <i
            class="chart-legend-swatch chart-legend-swatch--dot"
            :class="`run-tone--${entry.outcome}`"
            aria-hidden="true"
          />
          {{ entry.text }}
        </span>
        <span
          v-if="averageText"
          class="chart-legend-item muted"
          >{{ averageText }}</span
        >
        <button
          class="section-link"
          type="button"
          @click="emit('openLogs')"
        >
          View all in Logs
        </button>
      </div>
    </div>

    <template v-if="multiRepo">
      <div
        v-for="strip in repoStrips"
        :key="strip.repo.id"
        class="repo-strip"
      >
        <span class="group-label">{{ strip.repo.name }}</span>
        <div
          v-if="strip.runs.length > 0"
          class="run-strip"
          role="group"
          :aria-label="`${strip.repo.name}: last ${strip.runs.length} runs, oldest first`"
        >
          <button
            v-for="run in oldestFirst(strip.runs)"
            :key="run.key"
            class="run-pill"
            :class="`run-tone--${run.status}`"
            type="button"
            :title="pillLabel(run)"
            :aria-label="pillLabel(run)"
            :aria-pressed="selectedRun?.key === run.key"
            @click="selectedKey = run.key"
          />
        </div>
        <span
          v-else
          class="field-hint"
          >No runs yet.</span
        >
        <div
          v-if="axis(strip.runs)"
          class="run-axis"
        >
          <span>{{ axis(strip.runs)?.first }}</span>
          <span v-if="axis(strip.runs)?.last">{{ axis(strip.runs)?.last }}</span>
        </div>
      </div>
    </template>
    <template v-else-if="runs.length > 0">
      <div
        class="run-strip"
        role="group"
        :aria-label="`Last ${runs.length} runs, oldest first`"
      >
        <button
          v-for="run in oldestFirst(runs)"
          :key="run.key"
          class="run-pill"
          :class="`run-tone--${run.status}`"
          type="button"
          :title="pillLabel(run)"
          :aria-label="pillLabel(run)"
          :aria-pressed="selectedRun?.key === run.key"
          @click="selectedKey = run.key"
        />
      </div>
      <div
        v-if="axis(runs)"
        class="run-axis"
      >
        <span>{{ axis(runs)?.first }}</span>
        <span v-if="axis(runs)?.last">{{ axis(runs)?.last }}</span>
      </div>
    </template>

    <ScheduleRunDetail
      v-if="selectedRun"
      :key="selectedRun.key"
      :run="selectedRun"
      :host-label="hostLabel"
      :repo-label="repoLabel"
      @open-report-detail="emit('openReportDetail', $event)"
    />
  </div>
</template>

<style scoped>
.runs-head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-4) var(--space-7);
  margin-bottom: var(--space-5);
}

.runs-headline {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.repo-strip {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.repo-strip + .repo-strip {
  margin-top: var(--space-5);
}

/* Small on purpose: about the height of the agent Overview's run cells. */
.run-strip {
  display: flex;
  gap: var(--space-1);
}

.run-pill {
  flex: 1 1 0;
  min-width: 4px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: opacity var(--duration-fast);
}

.run-pill:hover {
  opacity: 0.75;
}

.run-pill[aria-pressed='true'] {
  outline: 2px solid var(--text-primary);
  outline-offset: 2px;
}

.run-axis {
  display: flex;
  justify-content: space-between;
  margin-top: var(--space-2);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.run-tone--success {
  background: var(--success);
}

.run-tone--warning {
  background: var(--warning);
}

.run-tone--failed {
  background: var(--danger);
}

.run-tone--cancelled {
  background: var(--text-muted);
}

/* A run that never started because a dependency did not answer: amber like a
   warning, but striped and pale, so it does not read as one that ran. */
.run-tone--skipped {
  background: repeating-linear-gradient(
    135deg,
    var(--warning) 0 3px,
    var(--warning-subtle) 3px 6px
  );
}
</style>
