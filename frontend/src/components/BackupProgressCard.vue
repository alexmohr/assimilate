<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { formatDuration, formatBytes } from '../utils/format'

interface ArchiveProgressData {
  nfiles: number
  originalSize: number
  currentPath: string
}

withDefaults(
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
  }>(),
  { repoId: null, cancelLoading: false, clampPath: false },
)

const emit = defineEmits<{
  cancel: []
}>()
</script>

<template>
  <div class="live-log-card">
    <div class="live-log-header">
      <span class="pulse-dot pulse-dot--success" />
      <span class="live-log-title">Backup in progress</span>
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
        v-if="!progress"
        class="live-log-empty"
      >
        Waiting for progress...
      </div>
      <template v-else>
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
          <span class="live-stat-value">{{ progress.nfiles.toLocaleString() }}</span>
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
  background: var(--bg-base);
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
</style>
