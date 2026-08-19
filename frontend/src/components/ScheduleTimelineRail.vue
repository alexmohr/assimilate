<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatDateShort } from '../utils/format'

export interface TimelineEntry {
  id: number | string
  label: string
  /** ISO timestamp of the upcoming run. */
  atIso: string
  /** Storage host the run writes to, or null when unknown. */
  host: string | null
}

const props = withDefaults(
  defineProps<{
    entries: TimelineEntry[]
    windowHours?: number
    /** Two runs on the same host within this many minutes count as a collision. */
    collisionMinutes?: number
  }>(),
  { windowHours: 24, collisionMinutes: 30 },
)

interface PositionedEntry extends TimelineEntry {
  ts: number
  percent: number
  colliding: boolean
}

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
      if (!entry.host || entry.host !== other.host) return false
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
 * pairwise same-host/within-collisionMs relation `colliding` above uses, so
 * the note always covers exactly the entries rendered with the colliding
 * tick style. Independently bucketing each entry by its own rounded
 * distance from "now" (an earlier approach) could split two entries mere
 * seconds apart into different buckets whenever their gap straddled a
 * bucket boundary, leaving colliding ticks with no explanatory note.
 */
const collisionGroups = computed<PositionedEntry[][]>(() => {
  const collisionMs = props.collisionMinutes * 60_000
  const candidates = positioned.value.filter((entry) => entry.colliding && entry.host)
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
        if (other.host !== current.host) continue
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

const collisionNote = computed(() => {
  if (collisionGroups.value.length === 0) return null
  const [first, ...rest] = collisionGroups.value
  const time = formatDateShort(first[0].atIso)
  const note = `${first.length} runs collide on ${first[0].host} around ${time}`
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
    <div
      v-if="collisionNote"
      class="timeline-note"
    >
      {{ collisionNote }}
    </div>
  </div>
</template>

<style scoped>
.timeline-rail {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.85rem 1.1rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.timeline-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
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

.timeline-note {
  font-size: var(--fs-xs);
  color: var(--warning);
}
</style>
