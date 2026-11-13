<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { formatDuration, formatBytes } from '../utils/format'
import type { HookOutputEntry } from '../utils/hookOutput'

interface ArchiveProgressData {
  nfiles: number
  originalSize: number
  currentPath: string
}

const props = withDefaults(
  defineProps<{
    badge: string | null
    repoId?: number | null
    archiveName: string | null
    elapsedSecs: number
    estimatedRemainingSecs: number | null
    progress: ArchiveProgressData | null
    cancelLoading?: boolean
    /** Clamp the current-file path to two lines instead of wrapping it in full. */
    clampPath?: boolean
    /**
     * The host this run is queued for, when it is offline. A Run now or Retry
     * for a host that is not connected is held until it reconnects, so there
     * is no progress to wait for yet - saying so beats a "Backup in progress"
     * that never moves. Ignored once the run has reported progress: a backup
     * that got going and then lost its connection is not queued, and its last
     * known progress is worth more than a "starts when it reconnects".
     */
    waitingFor?: string | null
    /**
     * What the pre- and post-backup scripts have printed so far. A pre-backup
     * script runs before borg reports anything, so this is often all there is
     * to show for a while - and what explains a run that seems to hang.
     */
    hookOutput?: readonly HookOutputEntry[]
  }>(),
  { repoId: null, cancelLoading: false, clampPath: false, waitingFor: null, hookOutput: () => [] },
)

const queued = computed(() => props.waitingFor != null && props.progress === null)

const hookLog = ref<HTMLElement | null>(null)

// Follows the newest line, as a terminal does. Keyed on the newest entry, not
// the count: past the cap the count stops changing while lines keep arriving.
watch(
  () => props.hookOutput.at(-1),
  async () => {
    await nextTick()
    if (hookLog.value) hookLog.value.scrollTop = hookLog.value.scrollHeight
  },
)

const emit = defineEmits<{
  cancel: []
}>()
</script>

<template>
  <div class="live-log-card">
    <div class="live-log-header">
      <span
        class="pulse-dot"
        :class="queued ? 'pulse-dot--waiting' : 'pulse-dot--success'"
      />
      <span class="live-log-title">{{ queued ? 'Backup queued' : 'Backup in progress' }}</span>
      <div class="live-log-header-actions">
        <RouterLink
          v-if="badge && repoId !== null"
          class="live-log-host-badge"
          :to="`/repos/${repoId}`"
          >{{ badge }}</RouterLink
        >
        <span
          v-else-if="badge"
          class="live-log-host-badge"
          >{{ badge }}</span
        >
        <button
          v-if="repoId !== null"
          type="button"
          class="btn btn-sm btn-danger"
          :disabled="cancelLoading"
          @click="emit('cancel')"
        >
          {{ cancelLoading ? 'Cancelling...' : 'Cancel backup' }}
        </button>
      </div>
    </div>
    <div class="progress-body">
      <div
        v-if="queued"
        class="live-log-empty"
      >
        {{ waitingFor }} is offline. The backup starts as soon as it reconnects.
      </div>
      <div
        v-else-if="!progress && hookOutput.length === 0"
        class="live-log-empty"
      >
        Waiting for progress...
      </div>
      <template v-else-if="progress">
        <div class="live-stat-row">
          <span class="live-stat-label">Elapsed</span>
          <span class="live-stat-value">{{ formatDuration(elapsedSecs) }}</span>
        </div>
        <div
          v-if="estimatedRemainingSecs !== null"
          class="live-stat-row"
        >
          <span class="live-stat-label">Est. remaining</span>
          <span class="live-stat-value">{{ formatDuration(estimatedRemainingSecs) }}</span>
        </div>
        <div class="live-stat-row">
          <span class="live-stat-label">Files</span>
          <span class="live-stat-value">{{ (progress.nfiles ?? 0).toLocaleString() }}</span>
        </div>
        <div class="live-stat-row">
          <span class="live-stat-label">Data</span>
          <span class="live-stat-value">{{ formatBytes(progress.originalSize) }}</span>
        </div>
        <div
          v-if="archiveName"
          class="live-stat-row"
        >
          <span class="live-stat-label">Archive</span>
          <span class="live-stat-value progress-mono">{{ archiveName }}</span>
        </div>
        <div
          v-if="progress.currentPath"
          class="live-stat-row live-stat-row-wrap"
        >
          <span class="live-stat-label">Current file</span>
          <span
            class="live-stat-value progress-path"
            :class="{ 'progress-path--clamp': clampPath }"
            >{{ progress.currentPath }}</span
          >
        </div>
      </template>
      <div
        v-if="!queued && hookOutput.length > 0"
        class="hook-output"
      >
        <span class="group-label">Script output</span>
        <pre
          ref="hookLog"
          class="detail-pre hook-output-log"
          aria-label="Script output"
        ><span
          v-for="(entry, i) in hookOutput"
          :key="i"
          class="hook-output-line"
          :class="{ 'hook-output-line--stderr': entry.stream === 'stderr' }"
        >{{ entry.line }}
</span></pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.live-log-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  margin-bottom: var(--space-6);
  overflow: hidden;
}

.live-log-header {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.live-log-title {
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.live-log-header-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.live-log-host-badge {
  font-size: var(--fs-xs);
  color: var(--accent);
  font-family: var(--mono);
}

.live-log-empty {
  padding: var(--space-5) var(--space-6);
  color: var(--text-muted);
  font-style: italic;
}

/* Reserves height for every row up front so the card doesn't resize as
   fields stream in over the WS connection or a long current-file path wraps. */
.progress-body {
  padding: var(--space-4) 0;
  min-height: 13rem;
}

.live-stat-row {
  display: flex;
  gap: var(--space-6);
  padding: var(--space-2) var(--space-6);
  font-size: var(--fs-base);
}

.live-stat-label {
  color: var(--text-muted);
  min-width: 9rem;
  flex-shrink: 0;
}

.live-stat-value {
  color: var(--text-primary);
}

.live-stat-row-wrap {
  align-items: flex-start;
}

.progress-path {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  word-break: break-all;
  overflow-wrap: break-word;
  white-space: pre-wrap;
  min-width: 0;
}

/* Reserves exactly two lines for the path and ellipsizes the rest, instead
   of letting it wrap in full - used where the card's overall size must stay
   fixed (the agent overview), not where it's the only thing on the page. */
.progress-path--clamp {
  white-space: normal;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
}

.progress-mono {
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.hook-output {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-6) 0;
}

/* Scrolls inside the card, so a chatty script cannot push the page down. */
.hook-output-log {
  max-height: 12rem;
  margin: 0;
  overflow-y: auto;
}

.hook-output-line--stderr {
  color: var(--warning);
}
</style>
