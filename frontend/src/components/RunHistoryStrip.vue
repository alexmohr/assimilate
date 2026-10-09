<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { formatDateShort, formatDuration } from '../utils/format'

export interface RunHistoryEntry {
  id: number | string
  startedAt: string
  durationSecs: number
  status: string
  /**
   * Correlates the reports of one schedule firing - a multi-agent schedule
   * writes one report per target under the same run id. Entries sharing it
   * are drawn as a single bar; an entry without one is a run of its own.
   */
  runId?: string | null
  /** Which agent the report belongs to, shown in its segment's tooltip. */
  hostname?: string
  /**
   * The repository the report wrote into, shown beside the hostname - a
   * multi-repository schedule writes one report per agent *and* repository,
   * so the hostname alone repeats across that agent's segments.
   */
  targetName?: string
}

type RunTone = 'success' | 'warning' | 'danger' | 'accent' | 'neutral'

/** One bar: every report of a single schedule firing, oldest first. */
interface RunGroup {
  key: string
  startedAt: string
  entries: RunHistoryEntry[]
}

const props = withDefaults(
  defineProps<{
    runs: RunHistoryEntry[]
    /** How many of the most recent runs to draw. */
    maxBars?: number
  }>(),
  { maxBars: 10 },
)

const MIN_BAR_HEIGHT_PERCENT = 25
// A multi-target bar splits its height evenly between its segments, so its
// floor grows with the segment count - otherwise a short (or still running)
// run of several targets would squash each segment to a sliver.
const MIN_SEGMENT_HEIGHT_PERCENT = 15

const visible = computed<RunGroup[]>(() => {
  const groups = new Map<string, RunGroup>()
  for (const entry of props.runs) {
    const key = entry.runId ?? String(entry.id)
    const group = groups.get(key)
    if (group) {
      group.entries.push(entry)
      if (entry.startedAt.localeCompare(group.startedAt) < 0) group.startedAt = entry.startedAt
    } else {
      groups.set(key, { key, startedAt: entry.startedAt, entries: [entry] })
    }
  }
  for (const group of groups.values()) {
    group.entries.sort((a, b) => a.startedAt.localeCompare(b.startedAt))
  }
  return [...groups.values()]
    .sort((a, b) => a.startedAt.localeCompare(b.startedAt))
    .slice(-props.maxBars)
})

function tone(run: RunHistoryEntry): RunTone {
  const status = normalizeBackupStatus(run.status)
  if (status === 'success') return 'success'
  if (status === 'warning') return 'warning'
  if (status === 'started' || status === 'pending') return 'accent'
  if (status === 'cancelled') return 'neutral'
  // A skipped run never started - drawn like a cancelled one, not as a failure.
  if (status === 'skipped') return 'neutral'
  return 'danger'
}

// A run's overall tone is its most severe target's: one failed agent makes
// the whole firing a failure, an agent still going keeps it running - even
// once an earlier target has finished with a warning, since targets complete
// one at a time and the run isn't over until the last one is.
const TONE_SEVERITY: readonly RunTone[] = ['danger', 'accent', 'warning', 'neutral', 'success']

function groupTone(group: RunGroup): RunTone {
  const tones = new Set(group.entries.map(tone))
  return TONE_SEVERITY.find((t) => tones.has(t)) ?? 'success'
}

function isCompleted(t: RunTone): boolean {
  return t === 'success' || t === 'warning'
}

// Targets of a schedule run one after another, so a firing's duration is the
// sum of its completed targets' durations. An in-progress target still
// carries durationSecs: 0 and a failed/cancelled one rarely says anything
// about how long a backup takes, so neither counts.
function completedDuration(group: RunGroup): number | null {
  const completed = group.entries.filter((e) => isCompleted(tone(e)))
  if (completed.length === 0) return null
  return completed.reduce((sum, e) => sum + e.durationSecs, 0)
}

// Height encodes duration for a completed run, scaled against the longest
// completed run in the strip. A failed run is drawn at full height instead
// of its own (usually short) elapsed time - a duration-proportional bar
// would draw the most important event in the row as the shortest bar.
const maxCompletedDuration = computed(() => {
  const durations = visible.value
    .filter((g) => isCompleted(groupTone(g)))
    .map((g) => completedDuration(g) ?? 0)
  return Math.max(1, ...durations)
})

function heightPercent(group: RunGroup): number {
  if (groupTone(group) === 'danger') return 100
  const floor = Math.max(MIN_BAR_HEIGHT_PERCENT, group.entries.length * MIN_SEGMENT_HEIGHT_PERCENT)
  const raw = ((completedDuration(group) ?? 0) / maxCompletedDuration.value) * 100
  return Math.min(100, Math.max(floor, raw))
}

const TONE_LABELS: Record<RunTone, string> = {
  success: 'Succeeded',
  warning: 'Warning',
  accent: 'Running',
  danger: 'Failed',
  neutral: 'Cancelled',
}

// A segment is one target of the firing - an agent writing into one
// repository - so a schedule with several repositories has more segments than
// agents. Say both when they differ rather than calling every target an agent.
function targetSummary(group: RunGroup): string {
  const targets = group.entries.length
  const agents = new Set(group.entries.flatMap((e) => (e.hostname ? [e.hostname] : []))).size
  if (agents === targets) return `${targets} agents`
  if (agents === 0) return `${targets} targets`
  return `${agents} agent${agents === 1 ? '' : 's'}, ${targets} targets`
}

function barTitle(group: RunGroup): string {
  const duration = completedDuration(group) ?? 0
  const head = `${formatDateShort(group.startedAt)} · ${TONE_LABELS[groupTone(group)]} · ${formatDuration(duration)}`
  if (group.entries.length === 1) return head
  return `${head} · ${targetSummary(group)}`
}

function segmentTitle(entry: RunHistoryEntry): string {
  const target = [entry.hostname, entry.targetName].filter(Boolean).join(' / ')
  const who = target ? `${target} · ` : ''
  return `${who}${formatDateShort(entry.startedAt)} · ${TONE_LABELS[tone(entry)]} · ${formatDuration(entry.durationSecs)}`
}

const caption = computed(() => {
  const count = visible.value.length
  if (count === 0) return 'No runs yet'
  const plural = count === 1 ? '' : 's'

  const failedCount = visible.value.filter((g) => groupTone(g) === 'danger').length
  if (failedCount > 0) {
    return `${count} run${plural} · ${failedCount} failed`
  }

  // Only completed runs have a meaningful duration - an in-progress run
  // (accent tone) still carries durationSecs: 0, which would otherwise pull
  // the low end of the range down to 0s while it's still running.
  const durations = visible.value
    .filter((g) => isCompleted(groupTone(g)))
    .map((g) => completedDuration(g) ?? 0)
  if (durations.length === 0) return `${count} run${plural}`
  const min = Math.min(...durations)
  const max = Math.max(...durations)
  const range = min === max ? formatDuration(min) : `${formatDuration(min)}-${formatDuration(max)}`
  return `${count} run${plural} · ${range}`
})
</script>

<template>
  <div
    class="run-history"
    role="img"
    :aria-label="caption"
  >
    <div class="run-history-bars">
      <template v-if="visible.length === 0">
        <span
          v-for="i in maxBars"
          :key="i"
          class="run-bar run-bar-empty"
        ></span>
      </template>
      <span
        v-for="group in visible"
        :key="group.key"
        class="run-bar"
        :class="`run-bar-${groupTone(group)}`"
        :style="{ height: `${heightPercent(group)}%` }"
        :title="barTitle(group)"
        :data-run-id="group.key"
      >
        <span
          v-for="entry in group.entries"
          :key="entry.id"
          class="run-bar-segment"
          :class="`run-bar-segment-${tone(entry)}`"
          :title="segmentTitle(entry)"
          :data-entry-id="entry.id"
        ></span>
      </span>
    </div>
    <span class="run-history-caption">{{ caption }}</span>
  </div>
</template>

<style scoped>
.run-history {
  display: flex;
  align-items: flex-end;
  gap: var(--space-4);
  height: 26px;
}

.run-history-bars {
  display: flex;
  align-items: flex-end;
  gap: 3px;
  height: 100%;
  flex-shrink: 0;
}

.run-bar {
  /* Segments stack bottom-up in the order the targets ran, each taking an
     equal share of the bar's height. */
  display: flex;
  flex-direction: column-reverse;
  gap: 1px;
  width: 7px;
  border-radius: var(--radius-sm);
  overflow: hidden;
  flex-shrink: 0;
}

.run-bar-segment {
  flex: 1 1 0;
  min-height: 0;
}

.run-bar-success .run-bar-segment {
  opacity: 0.55;
}

.run-bar-success:last-child .run-bar-segment {
  opacity: 1;
}

.run-bar-segment-success {
  background: var(--success);
}

.run-bar-segment-warning {
  background: var(--warning);
}

.run-bar-segment-danger {
  background: var(--danger);
}

.run-bar-segment-accent {
  background: var(--accent);
}

.run-bar-segment-neutral {
  background: var(--text-muted);
}

.run-bar-empty {
  height: 6px;
  background: var(--border);
}

.run-history-caption {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  white-space: nowrap;
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
