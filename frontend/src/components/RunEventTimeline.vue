<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, type Component } from 'vue'
import { Check, Clock, LogIn, Power, WifiOff, Zap } from '@lucide/vue'
import { formatTime } from '../utils/format'
import type { RunEventResponse, RunEventType } from '../types/generated'

/**
 * A run's power-management timeline: the reachability checks, wakes, agent
 * starts and shutdowns recorded around a backup, in order. Both source and
 * repository events share one timeline - they run concurrently and
 * independently, and interleaving them by time is what shows that.
 */
const props = defineProps<{
  events: RunEventResponse[]
  /** Label for a `target: source` row's eyebrow, e.g. the agent's hostname. */
  sourceLabel: string
  /** Label for a `target: repository` row's eyebrow, e.g. `user@host`. */
  repositoryLabel: string
}>()

const sorted = computed(() =>
  [...props.events].sort(
    (a, b) => new Date(a.occurred_at).getTime() - new Date(b.occurred_at).getTime(),
  ),
)

function eyebrowLabel(event: RunEventResponse): string {
  return event.target === 'source'
    ? `Source · ${props.sourceLabel}`
    : `Repository · ${props.repositoryLabel}`
}

const ICONS: Record<RunEventType, Component> = {
  reachability_check: WifiOff,
  wake_sent: Zap,
  host_online: Clock,
  agent_start_sent: LogIn,
  agent_connected: Check,
  agent_stop_sent: Power,
  agent_stopped: Power,
  shutdown_sent: Power,
  host_offline: Power,
}

const TONES: Partial<Record<RunEventType, 'accent' | 'success' | 'muted'>> = {
  wake_sent: 'accent',
  agent_connected: 'success',
  agent_stop_sent: 'muted',
  agent_stopped: 'muted',
  shutdown_sent: 'muted',
  host_offline: 'muted',
}
</script>

<template>
  <div class="run-timeline">
    <div
      v-for="(event, idx) in sorted"
      :key="event.id"
      class="run-timeline-row"
    >
      <span class="run-timeline-time">{{ formatTime(event.occurred_at) }}</span>
      <span class="run-timeline-rail">
        <span
          class="run-timeline-icon"
          :class="TONES[event.event_type] ? `run-timeline-icon--${TONES[event.event_type]}` : ''"
        >
          <component
            :is="ICONS[event.event_type]"
            :size="14"
          />
        </span>
        <span
          v-if="idx < sorted.length - 1"
          class="run-timeline-line"
        />
      </span>
      <div class="run-timeline-body">
        <span
          class="run-timeline-eyebrow"
          :class="`run-timeline-eyebrow--${event.target}`"
          >{{ eyebrowLabel(event) }}</span
        >
        <p class="run-timeline-msg">{{ event.message }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.run-timeline {
  display: flex;
  flex-direction: column;
}

.run-timeline-row {
  display: grid;
  grid-template-columns: 4.5rem auto 1fr;
  gap: var(--space-5);
}

.run-timeline-time {
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  padding-top: var(--space-2);
}

.run-timeline-rail {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
}

.run-timeline-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--bg-hover);
  color: var(--text-muted);
  flex-shrink: 0;
}

.run-timeline-icon--accent {
  background: var(--accent-subtle);
  color: var(--accent);
}

.run-timeline-icon--success {
  background: var(--success-subtle);
  color: var(--success);
}

.run-timeline-icon--muted {
  background: var(--bg-hover);
  color: var(--text-muted);
}

.run-timeline-line {
  flex: 1;
  width: 1px;
  min-height: var(--space-6);
  background: var(--border);
}

.run-timeline-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding-bottom: var(--space-6);
}

.run-timeline-eyebrow {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
}

.run-timeline-eyebrow--source {
  color: var(--accent);
}

.run-timeline-eyebrow--repository {
  color: var(--warning);
}

.run-timeline-msg {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  margin: 0;
}
</style>
