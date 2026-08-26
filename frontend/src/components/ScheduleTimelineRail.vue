<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { AlertTriangle, ChevronDown, ChevronRight } from '@lucide/vue'
import { formatDateShort } from '../utils/format'

export interface TimelineEntry {
  id: number | string
  label: string
  /** ISO timestamp of the upcoming run. */
  atIso: string
  /** Storage host the run writes to, or null when unknown. Counted in the summary. */
  host: string | null
  /**
   * Repository the run writes to, or null when unknown. Collisions are keyed on
   * this and not on the host: two runs sharing a host but writing to different
   * repositories do not contend for a repository lock, so warning about them
   * would be noise.
   */
  repoId: number | null
  /** Repository name, for the collision note. */
  repoName: string | null
}

const props = withDefaults(
  defineProps<{
    entries: TimelineEntry[]
    windowHours?: number
    /** Two runs on the same repository within this many minutes count as a collision. */
    collisionMinutes?: number
  }>(),
  { windowHours: 24, collisionMinutes: 30 },
)

const emit = defineEmits<{
  /** A colliding run was clicked in the expanded list. */
  select: [entry: TimelineEntry]
}>()

interface PositionedEntry extends TimelineEntry {
  ts: number
  percent: number
  colliding: boolean
}

const expanded = ref(false)

const positioned = computed<PositionedEntry[]>(() => {
  const now = Date.now()
  const windowMs = props.windowHours * 3_600_000
  const collisionMs = props.collisionMinutes * 60_000

  const withinWindow = props.entries
    .map((entry) => ({ ...entry, ts: new Date(entry.atIso).getTime() }))
    .filter((entry) => !isNaN(entry.ts) && entry.ts >= now && entry.ts <= now + windowMs)
    .sort((a, b) => a.ts - b.ts)

  return withinWindow.map((entry, index) => {
    const colliding = withinWindow.some((other, otherIndex) => {
      if (otherIndex === index) return false
      if (entry.repoId === null || entry.repoId !== other.repoId) return false
      return Math.abs(other.ts - entry.ts) <= collisionMs
    })
    return {
      ...entry,
      percent: Math.min(100, Math.max(0, ((entry.ts - now) / windowMs) * 100)),
      colliding,
    }
  })
})

/**
 * Colliding entries clustered into connected components of the same
 * pairwise same-repository/within-collisionMs relation `colliding` above uses,
 * so the note always covers exactly the entries rendered with the colliding
 * tick style. Independently bucketing each entry by its own rounded
 * distance from "now" (an earlier approach) could split two entries mere
 * seconds apart into different buckets whenever their gap straddled a
 * bucket boundary, leaving colliding ticks with no explanatory note.
 */
const collisionGroups = computed<PositionedEntry[][]>(() => {
  const collisionMs = props.collisionMinutes * 60_000
  const candidates = positioned.value.filter((entry) => entry.colliding && entry.repoId !== null)
  const visited = new Set<PositionedEntry>()
  const groups: PositionedEntry[][] = []

  for (const start of candidates) {
    if (visited.has(start)) continue
    const cluster: PositionedEntry[] = []
    const queue = [start]
    visited.add(start)
    while (queue.length > 0) {
      const current = queue.pop() as PositionedEntry
      cluster.push(current)
      for (const other of candidates) {
        if (visited.has(other)) continue
        if (other.repoId !== current.repoId) continue
        if (Math.abs(other.ts - current.ts) > collisionMs) continue
        visited.add(other)
        queue.push(other)
      }
    }
    if (cluster.length > 1) groups.push(cluster.sort((a, b) => a.ts - b.ts))
  }

  return groups
})

const summary = computed(() => {
  const hosts = new Set(
    positioned.value.map((entry) => entry.host).filter((host): host is string => host !== null),
  )
  const count = positioned.value.length
  return `${count} run${count === 1 ? '' : 's'} · ${hosts.size} host${hosts.size === 1 ? '' : 's'}`
})

/** "3 runs collide on server-daily around 26 Aug, 15:00". */
function groupTitle(group: PositionedEntry[]): string {
  const repo = group[0].repoName ?? `repository #${group[0].repoId}`
  return `${group.length} runs collide on ${repo} around ${formatDateShort(group[0].atIso)}`
}

const collisionNote = computed(() => {
  if (collisionGroups.value.length === 0) return null
  const [first, ...rest] = collisionGroups.value
  const note = groupTitle(first)
  return rest.length > 0 ? `${note} (+${rest.length} more)` : note
})
</script>

<template>
  <div
    v-if="positioned.length > 0"
    class="timeline-rail"
  >
    <div class="timeline-header">
      <span class="timeline-title">Next {{ windowHours }} hours</span>
      <span class="timeline-summary">{{ summary }}</span>
    </div>
    <div class="timeline-track">
      <span class="timeline-baseline"></span>
      <span class="timeline-now"></span>
      <span
        v-for="entry in positioned"
        :key="entry.id"
        class="timeline-tick"
        :class="{ 'timeline-tick-collision': entry.colliding }"
        :style="{ left: `${entry.percent}%` }"
        :title="`${entry.label} · ${formatDateShort(entry.atIso)}`"
      ></span>
    </div>
    <button
      v-if="collisionNote"
      type="button"
      class="timeline-note"
      :aria-expanded="expanded"
      @click="expanded = !expanded"
    >
      <AlertTriangle :size="12" />
      <span class="timeline-note-text">{{ collisionNote }}</span>
      <ChevronDown
        v-if="expanded"
        :size="12"
      />
      <ChevronRight
        v-else
        :size="12"
      />
    </button>
    <div
      v-if="expanded && collisionGroups.length > 0"
      class="timeline-collisions"
    >
      <div
        v-for="group in collisionGroups"
        :key="`${group[0].repoId}-${group[0].atIso}`"
        class="timeline-collision-group"
      >
        <span class="timeline-collision-title">{{ groupTitle(group) }}</span>
        <button
          v-for="entry in group"
          :key="entry.id"
          type="button"
          class="timeline-collision-run"
          @click="emit('select', entry)"
        >
          <span class="timeline-collision-when">{{ formatDateShort(entry.atIso) }}</span>
          <span class="timeline-collision-label">{{ entry.label }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline-rail {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-5) var(--space-6) var(--space-6);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.timeline-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
}

.timeline-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.timeline-summary {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.timeline-track {
  position: relative;
  height: 20px;
}

.timeline-baseline {
  position: absolute;
  top: 8px;
  left: 0;
  right: 0;
  height: 4px;
  border-radius: var(--radius-pill);
  background: var(--border);
}

.timeline-now {
  position: absolute;
  top: 2px;
  left: 0;
  width: 2px;
  height: 16px;
  border-radius: var(--radius-pill);
  background: var(--text-primary);
}

.timeline-tick {
  position: absolute;
  top: 4px;
  width: 3px;
  height: 12px;
  border-radius: var(--radius-pill);
  background: var(--success);
  transform: translateX(-50%);
}

.timeline-tick-collision {
  top: 2px;
  height: 16px;
  background: var(--warning);
}

/* The note opens the list of runs it is about, so it is a control, not a line
   of text: a bare warning naming a time the user then has to hunt for in the
   groups below is exactly what this replaces. */
.timeline-note {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  align-self: flex-start;
  padding: 0;
  border: 0;
  background: none;
  font-size: var(--fs-xs);
  color: var(--warning);
  cursor: pointer;
  text-align: left;
}

.timeline-note-text {
  text-decoration: underline;
  text-underline-offset: 2px;
}

.timeline-collisions {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  border-top: 1px solid var(--border);
  padding-top: var(--space-4);
}

.timeline-collision-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.timeline-collision-title {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.timeline-collision-run {
  display: flex;
  align-items: baseline;
  gap: var(--space-4);
  padding: var(--space-2) var(--space-3);
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-primary);
  font-size: var(--fs-sm);
  cursor: pointer;
  text-align: left;
  transition: background var(--duration-base);
}

.timeline-collision-run:hover {
  background: var(--bg-hover);
}

.timeline-collision-when {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.timeline-collision-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
